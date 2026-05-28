use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct PythonParser;

impl PythonParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        // Validate tree-sitter-python loads
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-python")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-python")?;

        let tree = parser
            .parse(source, None)
            .context("tree-sitter parse failed")?;
        let root = tree.root_node();
        let source_bytes = source.as_bytes();
        let mut result = ParseResult {
            symbols: Vec::new(),
            references: Vec::new(),
            imports: Vec::new(),
            string_literals: Vec::new(),
        };

        Self::walk_node(&root, source_bytes, &mut result, None, None);
        Ok(result)
    }

    /// Walk the AST, tracking current class parent and current function context.
    ///
    /// - `parent`: enclosing class name (for method detection + qualified_name)
    /// - `current_function`: enclosing function/method name (for caller_symbol on calls)
    fn walk_node(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<String>,
        current_function: Option<String>,
    ) {
        let kind = node.kind();

        match kind {
            "function_definition" => {
                Self::extract_function(node, source, result, parent.as_deref());
            }
            "class_definition" => {
                Self::extract_class(node, source, result, parent.as_deref());
            }
            "import_statement" | "import_from_statement" => {
                Self::extract_import(node, source, result);
            }
            "expression_statement" => {
                Self::extract_assignment(node, source, result, parent.as_deref());
            }
            _ => {}
        }

        // Extract call references with caller_symbol context
        Self::extract_calls(node, source, result, current_function.as_deref());

        // Extract string literals (Cross-ref Engine)
        Self::extract_string_literals(node, source, result, current_function.as_deref());

        // Recurse into children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = if child.kind() == "class_definition" {
                child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string())
            } else {
                parent.clone()
            };

            // Update current_function if entering a function definition
            let new_function = if child.kind() == "function_definition" {
                child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string())
            } else {
                current_function.clone()
            };

            Self::walk_node(&child, source, result, new_parent, new_function);
        }
    }

    fn extract_function(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let Some(name) = name else {
            return;
        };

        let signature = node
            .child_by_field_name("parameters")
            .map(|p| format!("def {}({})", name, p.utf8_text(source).unwrap_or("")))
            .or(Some(format!("def {}", name)));

        let docstring = Self::extract_docstring(node, source);

        result.symbols.push(ParsedSymbol {
            name,
            kind: if parent.is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            },
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature,
            docstring,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_class(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let Some(name) = name else {
            return;
        };

        let bases = Self::extract_class_bases(node, source);
        let signature = if bases.is_empty() {
            format!("class {}", name)
        } else {
            format!("class {}({})", name, bases.join(", "))
        };

        // Inheritance references
        for base in &bases {
            result.references.push(ParsedReference {
                caller_symbol: Some(name.clone()),
                callee_symbol: base.clone(),
                ref_kind: "inherit".to_string(),
                line: node.start_position().row + 1,
            });
        }

        // BUG-011 fix: parent must not equal the class's own name.
        // When walk_node recurses into a class_definition, it passes
        // the class name as parent. extract_class should treat that as
        // "no enclosing class" rather than creating "User.User".
        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(signature),
            docstring: Self::extract_docstring(node, source),
            parent: effective_parent,
        });
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
        let import_kind = if node.kind() == "import_from_statement" {
            "from_import"
        } else {
            "import"
        };

        let module_node = if node.kind() == "import_from_statement" {
            node.child_by_field_name("module_name")
        } else {
            node.child_by_field_name("name")
        };

        if let Some(module) = module_node.and_then(|n| n.utf8_text(source).ok()) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "dotted_name" || child.kind() == "alias" {
                    if let Some(child_name) = child.utf8_text(source).ok() {
                        result.imports.push((
                            module.to_string(),
                            child_name.to_string(),
                            import_kind.to_string(),
                        ));
                        result.references.push(ParsedReference {
                            caller_symbol: None,
                            callee_symbol: child_name.to_string(),
                            ref_kind: "import".to_string(),
                            line: node.start_position().row + 1,
                        });
                    }
                }
            }

            result.symbols.push(ParsedSymbol {
                name: module.to_string(),
                kind: SymbolKind::Import,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: None,
                docstring: None,
                parent: None,
            });
        }
    }

    fn extract_assignment(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent_context: Option<&str>,
    ) {
        if parent_context.is_some() {
            return;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "assignment" {
                if let Some(left) = child.child_by_field_name("left") {
                    if let Some(name) = left.utf8_text(source).ok() {
                        result.symbols.push(ParsedSymbol {
                            name: name.to_string(),
                            kind: SymbolKind::Variable,
                            start_line: child.start_position().row + 1,
                            end_line: child.end_position().row + 1,
                            start_col: child.start_position().column,
                            end_col: child.end_position().column,
                            signature: None,
                            docstring: None,
                            parent: None,
                        });
                    }
                }
                break;
            }
        }
    }

    fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult, caller: Option<&str>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "call" {
                if let Some(func) = child.child_by_field_name("function") {
                    let callee = func.utf8_text(source).unwrap_or("").to_string();
                    if !callee.is_empty() {
                        result.references.push(ParsedReference {
                            caller_symbol: caller.map(|s| s.to_string()),
                            callee_symbol: callee,
                            ref_kind: "call".to_string(),
                            line: child.start_position().row + 1,
                        });
                    }
                }
            }
        }
    }

    fn extract_docstring(node: &Node, source: &[u8]) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" {
                let mut block_cursor = child.walk();
                for stmt in child.children(&mut block_cursor) {
                    if stmt.kind() == "expression_statement" {
                        let mut stmt_cursor = stmt.walk();
                        for expr in stmt.children(&mut stmt_cursor) {
                            if expr.kind() == "string" {
                                if let Ok(text) = expr.utf8_text(source) {
                                    let cleaned = text.trim_matches(|c| c == '\'' || c == '"');
                                    return Some(
                                        cleaned
                                            .lines()
                                            .next()
                                            .map(|l| l.to_string())
                                            .unwrap_or_default(),
                                    );
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
        None
    }

    fn extract_class_bases(node: &Node, source: &[u8]) -> Vec<String> {
        let arg_list = node.child_by_field_name("superclasses");
        let Some(args) = arg_list else {
            return Vec::new();
        };

        let mut bases = Vec::new();
        let mut cursor = args.walk();
        for child in args.children(&mut cursor) {
            if let Ok(text) = child.utf8_text(source) {
                bases.push(text.to_string());
            }
        }
        bases
    }

    // ─── String literal extraction (Cross-ref Engine) ───

    /// 문자열 리터럴 추출 (AST walk 중에 호출)
    fn extract_string_literals(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent_fn: Option<&str>,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "string" {
                if let Ok(raw) = child.utf8_text(source) {
                    // f-string, b-string, r-string 제외
                    if Self::is_noise_string(raw) {
                        continue;
                    }

                    // docstring 위치인지 확인
                    if Self::is_docstring_position(node, &child) {
                        continue;
                    }

                    let inner = Self::strip_quotes(raw);
                    let is_sym_like = Self::is_symbol_like_path(&inner);

                    let context = Self::infer_string_context(&child);

                    result.string_literals.push(ParsedStringLiteral {
                        value: inner.to_string(),
                        line: child.start_position().row + 1,
                        col: child.start_position().column,
                        is_symbol_like: is_sym_like,
                        context,
                        parent_fn: parent_fn.map(|s| s.to_string()),
                    });
                }
            }
        }
    }

    /// f-string, b-string, r-string, raw bytes 제외
    fn is_noise_string(raw: &str) -> bool {
        let prefix: String = raw.chars().take(3).filter(|c| *c != '"' && *c != '\'').collect();
        prefix.starts_with('f') || prefix.starts_with('F')
            || prefix.starts_with("b'") || prefix.starts_with("b\"")
            || prefix.starts_with("B'") || prefix.starts_with("B\"")
            || prefix.starts_with('b') || prefix.starts_with('B')
    }

    /// 인용부호 제거
    fn strip_quotes(s: &str) -> &str {
        let trimmed = s.trim();
        // Triple quotes first
        if trimmed.starts_with("\"\"\"") && trimmed.ends_with("\"\"\"") {
            &trimmed[3..trimmed.len()-3]
        } else if trimmed.starts_with("'''") && trimmed.ends_with("'''") {
            &trimmed[3..trimmed.len()-3]
        } else if trimmed.starts_with('\"') && trimmed.ends_with('\"') {
            &trimmed[1..trimmed.len()-1]
        } else if trimmed.starts_with('\'') && trimmed.ends_with('\'') {
            &trimmed[1..trimmed.len()-1]
        } else {
            trimmed
        }
    }

    /// 심볼 경로 후보인지 판단
    /// "sentry.models.user.User" → true
    /// "hello world" → false (공백)
    /// "http://example.com" → false (슬래시)
    /// "1.0.2" → false (버전 문자열)
    /// "User" (대문자 시작) → true (클래스명 후보)
    fn is_symbol_like_path(s: &str) -> bool {
        // 공백, 슬래시, 하이픈, 콜론 → 즉시 false
        if s.chars().any(|c| matches!(c, ' ' | '/' | '-' | ':')) {
            return false;
        }
        // 버전 문자열 패턴: "1.0.2", "v1.2"
        if let Some(first) = s.chars().next() {
            if first.is_ascii_digit() || first == 'v' || first == 'V' {
                // 숫자 시작 + 점이 있으면 버전 문자열
                if s.contains('.') {
                    return false;
                }
            }
        }
        // 점으로 구분된 유효한 식별자들
        let segs: Vec<&str> = s.split('.').collect();
        if segs.len() >= 2 {
            return segs.iter().all(|seg| Self::is_valid_identifier(seg));
        }
        // 단일 식별자: 대문자 시작이면 클래스명 후보
        if let Some(first) = s.chars().next() {
            if first.is_uppercase() && Self::is_valid_identifier(s) {
                return true;
            }
        }
        false
    }

    fn is_valid_identifier(s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        s.chars().all(|c| c.is_alphanumeric() || c == '_')
    }

    /// 문자열의 AST 컨텍스트 추론
    fn infer_string_context(node: &Node) -> String {
        match node.parent().map(|n| n.kind()) {
            Some("argument_list")    => "function_arg".to_string(),
            Some("list")            => "sequence_element".to_string(),
            Some("tuple")           => "sequence_element".to_string(),
            Some("assignment")      => "assignment_rhs".to_string(),
            Some("keyword_argument")=> "kwarg".to_string(),
            _                       => "unknown".to_string(),
        }
    }

    /// docstring 위치인지 확인
    fn is_docstring_position(parent: &Node, string_node: &Node) -> bool {
        // 함수/클래스의 첫 번째 statement가 expression_statement이고
        // 그 안에 string이 있으면 docstring
        if parent.kind() != "expression_statement" {
            return false;
        }
        let first_child = parent.child(0);
        first_child.map_or(false, |c| {
            c.start_position() == string_node.start_position()
        })
    }
}

impl SourceCodeParser for PythonParser {
   fn language(&self) -> &str {
       "python"
   }

   fn file_extensions(&self) -> &[&str] {
       &["py"]
   }

   fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
       Self::do_parse(self, source)
   }
}

#[cfg(test)]
mod tests {
   use super::*;

   // ─── is_symbol_like_path tests ───

   #[test]
   fn test_symbol_like_dotted_path() {
       assert!(PythonParser::is_symbol_like_path("sentry.models.user.User"));
       assert!(PythonParser::is_symbol_like_path("django.contrib.auth.models.User"));
       assert!(PythonParser::is_symbol_like_path("my_module.MyClass"));
   }

   #[test]
   fn test_symbol_like_single_class() {
       assert!(PythonParser::is_symbol_like_path("User"));
       assert!(PythonParser::is_symbol_like_path("MyClass"));
       assert!(PythonParser::is_symbol_like_path("ForeignKey"));
   }

   #[test]
   fn test_symbol_like_reject_whitespace() {
       assert!(!PythonParser::is_symbol_like_path("hello world"));
       assert!(!PythonParser::is_symbol_like_path("user not found"));
   }

   #[test]
   fn test_symbol_like_reject_slash() {
       assert!(!PythonParser::is_symbol_like_path("http://example.com"));
       assert!(!PythonParser::is_symbol_like_path("/usr/local/bin"));
   }

   #[test]
   fn test_symbol_like_reject_version() {
       assert!(!PythonParser::is_symbol_like_path("1.0.2"));
       assert!(!PythonParser::is_symbol_like_path("v1.2"));
       assert!(!PythonParser::is_symbol_like_path("V2.0"));
   }

   #[test]
   fn test_symbol_like_reject_lowercase_single() {
       assert!(!PythonParser::is_symbol_like_path("hello"));
       assert!(!PythonParser::is_symbol_like_path("user"));
       assert!(!PythonParser::is_symbol_like_path("logger"));
   }

   #[test]
   fn test_symbol_like_reject_colon() {
       assert!(!PythonParser::is_symbol_like_path("postgres://localhost"));
       assert!(!PythonParser::is_symbol_like_path("http:8080"));
   }

   #[test]
   fn test_symbol_like_reject_hyphen() {
       assert!(!PythonParser::is_symbol_like_path("my-package"));
       assert!(!PythonParser::is_symbol_like_path("user-profile"));
   }

   #[test]
   fn test_valid_identifier() {
       assert!(PythonParser::is_valid_identifier("User"));
       assert!(PythonParser::is_valid_identifier("my_class"));
       assert!(PythonParser::is_valid_identifier("User123"));
       assert!(!PythonParser::is_valid_identifier(""));
       assert!(!PythonParser::is_valid_identifier("my-class"));
       assert!(!PythonParser::is_valid_identifier("my class"));
   }

   // ─── strip_quotes tests ───

   #[test]
   fn test_strip_single_quotes() {
       assert_eq!(PythonParser::strip_quotes("'hello'"), "hello");
       assert_eq!(PythonParser::strip_quotes("\"hello\""), "hello");
   }

   #[test]
   fn test_strip_triple_quotes() {
       assert_eq!(PythonParser::strip_quotes("\"\"\"hello\"\"\""), "hello");
       assert_eq!(PythonParser::strip_quotes("'''hello'''"), "hello");
   }

   // ─── is_noise_string tests ───

   #[test]
   fn test_noise_fstring() {
       assert!(PythonParser::is_noise_string("f'hello {name}'"));
       assert!(PythonParser::is_noise_string("F\"hello {name}\""));
   }

   #[test]
   fn test_noise_bstring() {
       assert!(PythonParser::is_noise_string("b'hello'"));
       assert!(PythonParser::is_noise_string("B\"hello\""));
   }

   #[test]
   fn test_not_noise_regular() {
       assert!(!PythonParser::is_noise_string("'hello'"));
       assert!(!PythonParser::is_noise_string("\"hello\""));
   }

   // ─── Integration: parse string literals ───

   #[test]
   fn test_parse_string_literals() {
       let mut parser = PythonParser::new().unwrap();
       let code = r#"
INSTALLED_APPS = [
   "sentry.models.User",
   "User",
   "hello world",
   "http://example.com",
   "1.0.2",
]

def foo():
   x = ForeignKey("sentry.User")
"#;
       let result = parser.parse(code).unwrap();

       // 심볼 유사 문자열들
       let sym_like: Vec<_> = result
           .string_literals
           .iter()
           .filter(|l| l.is_symbol_like)
           .map(|l| l.value.as_str())
           .collect();

       assert!(sym_like.contains(&"sentry.models.User"));
       assert!(sym_like.contains(&"User"));
       assert!(sym_like.contains(&"sentry.User"));

       // 제외된 문자열들
       let all_values: Vec<_> = result
           .string_literals
           .iter()
           .filter(|l| !l.is_symbol_like)
           .map(|l| l.value.as_str())
           .collect();

       assert!(all_values.contains(&"hello world"));
       assert!(all_values.contains(&"http://example.com"));
       assert!(all_values.contains(&"1.0.2"));
   }
}
