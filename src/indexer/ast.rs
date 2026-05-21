/// AST parser module — tree-sitter backends for multiple languages
///
/// Extract symbols (functions, classes, variables, imports, exports) and references (calls, imports, inheritance)
/// from source code. Supports Python, JavaScript, Rust, TypeScript, and Go.

use anyhow::Context;
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Shared types (language-agnostic)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
/// Symbol kind
#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Function,
    Class,
    Variable,
    Method,
    Import,
    Export,
    Decorator,
    Module,
    Enum,
    TypeAlias,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Class => "class",
            SymbolKind::Variable => "variable",
            SymbolKind::Method => "method",
            SymbolKind::Import => "import",
            SymbolKind::Export => "export",
            SymbolKind::Decorator => "decorator",
            SymbolKind::Module => "module",
            SymbolKind::Enum => "enum",
            SymbolKind::TypeAlias => "type_alias",
        }
    }
}

/// Extracted symbol
#[derive(Debug, Clone)]
pub struct ParsedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: usize,
    pub end_line: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub parent: Option<String>,
}

/// Extracted reference
#[derive(Debug, Clone)]
pub struct ParsedReference {
    pub caller_symbol: Option<String>,
    pub callee_symbol: String,
    pub ref_kind: String,
    pub line: usize,
}

/// File parse result
#[derive(Debug, Clone)]
pub struct ParseResult {
    pub symbols: Vec<ParsedSymbol>,
    pub references: Vec<ParsedReference>,
    pub imports: Vec<(String, String, String)>,
}

// ---------------------------------------------------------------------------
// Parser trait
// ---------------------------------------------------------------------------

/// Language-agnostic source code parser
pub trait SourceCodeParser {
    /// Language identifier (e.g. "python", "javascript")
    fn language(&self) -> &str;

    /// File extensions this parser handles (e.g. ["py"])
    fn file_extensions(&self) -> &[&str];

    /// Parse source code and extract symbols, references, imports
    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error>;
}

// ---------------------------------------------------------------------------
// Python backend
// ---------------------------------------------------------------------------

mod python {
    use super::*;

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

            let tree = parser.parse(source, None).context("tree-sitter parse failed")?;
            let root = tree.root_node();
            let source_bytes = source.as_bytes();
            let mut result = ParseResult {
                symbols: Vec::new(),
                references: Vec::new(),
                imports: Vec::new(),
            };

            Self::walk_node(&root, source_bytes, &mut result, None);
            Ok(result)
        }

        fn walk_node(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            parent: Option<String>,
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

            // Extract call references
            Self::extract_calls(node, source, result);

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
                Self::walk_node(&child, source, result, new_parent);
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

        fn extract_class(
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

            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Class,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(signature),
                docstring: Self::extract_docstring(node, source),
                parent: parent.map(|s| s.to_string()),
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

        fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "call" {
                    if let Some(func) = child.child_by_field_name("function") {
                        let callee = func.utf8_text(source).unwrap_or("").to_string();
                        if !callee.is_empty() {
                            result.references.push(ParsedReference {
                                caller_symbol: None,
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
                                        let cleaned =
                                            text.trim_matches(|c| c == '\'' || c == '"');
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
}

// ---------------------------------------------------------------------------
// JavaScript Parser
// ---------------------------------------------------------------------------
//
// AST structure (verified with tree-sitter-javascript v0.23.1):
//
// function_declaration
//   async           <-- token child (presence = async function)
//   function        <-- keyword
//   identifier      <-- function name (field: "name")
//   formal_parameters
//   statement_block
//
// class_declaration
//   class           <-- keyword
//   identifier      <-- class name (field: "name")
//   class_heritage  <-- "extends Parent" (field: "superClass")
//     extends
//     identifier    <-- parent class name
//   class_body
//     method_definition
//       property_identifier  <-- method name (field: "name")
//       formal_parameters
//       statement_block
//
// import_statement
//   import
//   import_clause
//     identifier          <-- default import name
//     named_imports       <-- { a, b }
//       import_specifier
//         identifier
//   from
//   string                  <-- module name (field: "source")
//
// export_statement
//   export
//   default                 <-- token (presence = default export)
//   identifier              <-- exported name
//   export_clause
//     export_specifier
//       identifier
//       as
//       identifier          <-- alias

mod javascript {
    use super::*;

    pub struct JavaScriptParser;

    impl JavaScriptParser {
        pub fn new() -> Result<Self, anyhow::Error> {
            let mut parser = tree_sitter::Parser::new();
            let language: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
            parser
                .set_language(&language)
                .context("Failed to load tree-sitter-javascript")?;
            Ok(Self)
        }

        fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
            let mut parser = tree_sitter::Parser::new();
            let language: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
            parser
                .set_language(&language)
                .context("Failed to load tree-sitter-javascript")?;

            let tree = parser.parse(source, None).context("tree-sitter parse failed")?;
            let root = tree.root_node();
            let source_bytes = source.as_bytes();
            let mut result = ParseResult {
                symbols: Vec::new(),
                references: Vec::new(),
                imports: Vec::new(),
            };

            Self::walk_node(&root, source_bytes, &mut result, None);
            Ok(result)
        }

        fn walk_node(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            parent: Option<String>,
        ) {
            let kind = node.kind();

            match kind {
                "function_declaration" => {
                    let is_async = Self::has_child_token(node, "async");
                    Self::extract_function(node, source, result, parent.as_deref(), is_async);
                }
                "class_declaration" => {
                    Self::extract_class(node, source, result, parent.as_deref());
                }
                "import_statement" => {
                    Self::extract_import(node, source, result);
                }
                "export_statement" => {
                    Self::extract_export(node, source, result);
                }
                "lexical_declaration" => {
                    Self::extract_lexical_declaration(node, source, result, parent.as_deref());
                }
                "variable_declaration" => {
                    Self::extract_variable_declaration(node, source, result, parent.as_deref());
                }
                _ => {}
            }

            // Extract call references
            Self::extract_calls(node, source, result);

            // Recurse into children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let new_parent = if child.kind() == "class_declaration" {
                    child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string())
                } else {
                    parent.clone()
                };
                Self::walk_node(&child, source, result, new_parent);
            }
        }

        /// Check if a node has a direct child token with the given kind
        fn has_child_token(node: &Node, token_kind: &str) -> bool {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == token_kind {
                    return true;
                }
            }
            false
        }

        fn extract_function(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            parent: Option<&str>,
            is_async: bool,
        ) {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());

            let Some(name) = name else {
                return;
            };

            let params = node
                .child_by_field_name("parameters")
                .and_then(|p| p.utf8_text(source).ok())
                .map(|s| s.to_string())
                .unwrap_or_default();

            let prefix = if is_async { "async " } else { "" };
            let signature = Some(format!("{}function {}({})", prefix, name, params));

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
                docstring: Self::extract_j_sdoc(node, source),
                parent: parent.map(|s| s.to_string()),
            });
        }

         fn extract_method(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            parent: Option<&str>,
        ) {
            // method_definition: try "name" field first, fall back to property_identifier
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()))
                .or_else(|| {
                    // Fallback: find property_identifier as direct child
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "property_identifier" {
                            return child.utf8_text(source).ok().map(|s| s.to_string());
                        }
                    }
                    None
                });

            let Some(name) = name else {
                return;
            };

            let params = node
                .child_by_field_name("parameters")
                .and_then(|p| p.utf8_text(source).ok())
                .map(|s| s.to_string())
                .unwrap_or_default();

            let is_async = Self::has_child_token(node, "async");
            let prefix = if is_async { "async " } else { "" };
            let signature = Some(format!("{}{}({})", prefix, name, params));

            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Method,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature,
                docstring: Self::extract_j_sdoc(node, source),
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_class(
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

            // Extract extends: find class_heritage as direct child of class_declaration
            let mut bases = Vec::new();
            let mut heritage_cursor = node.walk();
            for child in node.children(&mut heritage_cursor) {
                if child.kind() == "class_heritage" {
                    let mut ch_cursor = child.walk();
                    for heritage_child in child.children(&mut ch_cursor) {
                        if heritage_child.kind() == "identifier" {
                            if let Ok(base) = heritage_child.utf8_text(source) {
                                bases.push(base.to_string());
                                result.references.push(ParsedReference {
                                    caller_symbol: Some(name.clone()),
                                    callee_symbol: base.to_string(),
                                    ref_kind: "inherit".to_string(),
                                    line: node.start_position().row + 1,
                                });
                            }
                        }
                    }
                }
            }

            let signature = if bases.is_empty() {
                format!("class {}", name)
            } else {
                format!("class {} extends {}", name, bases.join(", "))
            };

            result.symbols.push(ParsedSymbol {
                name: name.clone(),
                kind: SymbolKind::Class,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(signature),
                docstring: Self::extract_j_sdoc(node, source),
                parent: parent.map(|s| s.to_string()),
            });

            // Extract methods from class_body
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "class_body" {
                    let mut body_cursor = child.walk();
                    for member in child.children(&mut body_cursor) {
                        if member.kind() == "method_definition" {
                            Self::extract_method(&member, source, result, Some(&name));
                        }
                    }
                }
            }
        }

        fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
            let module = Self::find_string_child(node, source);

            // Find import_clause as a direct child of import_statement
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "import_clause" {
                    let mut ic_cursor = child.walk();
                    for ic_child in child.children(&mut ic_cursor) {
                        if ic_child.kind() == "named_imports" {
                            let mut ni_cursor = ic_child.walk();
                            for spec in ic_child.children(&mut ni_cursor) {
                                if spec.kind() == "import_specifier" {
                                    let mut spec_cursor = spec.walk();
                                    for spec_child in spec.children(&mut spec_cursor) {
                                        if spec_child.kind() == "identifier" {
                                            if let Ok(name) = spec_child.utf8_text(source) {
                                                if let Some(ref mod_name) = module {
                                                    result.imports.push((
                                                        mod_name.clone(),
                                                        name.to_string(),
                                                        "from_import".to_string(),
                                                    ));
                                                    result.references.push(ParsedReference {
                                                        caller_symbol: None,
                                                        callee_symbol: name.to_string(),
                                                        ref_kind: "import".to_string(),
                                                        line: node.start_position().row + 1,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if ic_child.kind() == "identifier" {
                            if let Ok(name) = ic_child.utf8_text(source) {
                                if let Some(ref mod_name) = module {
                                    result.imports.push((
                                        mod_name.clone(),
                                        name.to_string(),
                                        "default_import".to_string(),
                                    ));
                                    result.references.push(ParsedReference {
                                        caller_symbol: None,
                                        callee_symbol: name.to_string(),
                                        ref_kind: "import".to_string(),
                                        line: node.start_position().row + 1,
                                    });
                                }
                            }
                        }
                    }
                }
            }

            if let Some(ref mod_name) = module {
                result.symbols.push(ParsedSymbol {
                    name: mod_name.clone(),
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

        fn find_string_child(node: &Node, source: &[u8]) -> Option<String> {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "string" {
                    if let Ok(text) = child.utf8_text(source) {
                        let trimmed = text
                            .trim_matches(|c| c == '\'' || c == '"' || c == '`')
                            .to_string();
                        return Some(trimmed);
                    }
                }
            }
            None
        }

        fn extract_export(node: &Node, source: &[u8], result: &mut ParseResult) {
            let is_default = Self::has_child_token(node, "default");

            if is_default {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        if let Ok(name) = child.utf8_text(source) {
                            result.symbols.push(ParsedSymbol {
                                name: name.to_string(),
                                kind: SymbolKind::Export,
                                start_line: node.start_position().row + 1,
                                end_line: node.end_position().row + 1,
                                start_col: node.start_position().column,
                                end_col: node.end_position().column,
                                signature: None,
                                docstring: None,
                                parent: None,
                            });
                        }
                        break;
                    }
                }
            }

            // Find export_clause as a direct child of export_statement
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "export_clause" {
                    let mut ec_cursor = child.walk();
                    for specifier in child.children(&mut ec_cursor) {
                        if specifier.kind() == "export_specifier" {
                            let mut spec_cursor = specifier.walk();
                            for spec_child in specifier.children(&mut spec_cursor) {
                                if spec_child.kind() == "identifier" {
                                    if let Ok(name) = spec_child.utf8_text(source) {
                                        result.symbols.push(ParsedSymbol {
                                            name: name.to_string(),
                                            kind: SymbolKind::Export,
                                            start_line: node.start_position().row + 1,
                                            end_line: node.end_position().row + 1,
                                            start_col: node.start_position().column,
                                            end_col: node.end_position().column,
                                            signature: None,
                                            docstring: None,
                                            parent: None,
                                        });
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        fn extract_lexical_declaration(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            parent: Option<&str>,
        ) {
            if parent.is_some() {
                return;
            }

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Ok(name) = name_node.utf8_text(source) {
                            // Check if value is an arrow function
                            let value = child.child_by_field_name("value");
                            let signature = value
                                .and_then(|v| v.utf8_text(source).ok())
                                .map(|v| format!("const {} = {}", name, v))
                                .or(Some(format!("const {}", name)));

                            result.symbols.push(ParsedSymbol {
                                name: name.to_string(),
                                kind: SymbolKind::Variable,
                                start_line: child.start_position().row + 1,
                                end_line: child.end_position().row + 1,
                                start_col: child.start_position().column,
                                end_col: child.end_position().column,
                                signature: signature.clone(),
                                docstring: None,
                                parent: None,
                            });

                            // If it's an arrow function, also extract as function
                            if let Some(ref val_str) = signature {
                                if val_str.contains("=>") {
                                    result.symbols.push(ParsedSymbol {
                                        name: format!("{} (arrow)", name),
                                        kind: SymbolKind::Function,
                                        start_line: child.start_position().row + 1,
                                        end_line: child.end_position().row + 1,
                                        start_col: child.start_position().column,
                                        end_col: child.end_position().column,
                                        signature: signature.clone(),
                                        docstring: None,
                                        parent: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        fn extract_variable_declaration(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            parent: Option<&str>,
        ) {
            if parent.is_some() {
                return;
            }

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Ok(name) = name_node.utf8_text(source) {
                            let value = child.child_by_field_name("value");
                            let signature = value
                                .and_then(|v| v.utf8_text(source).ok())
                                .map(|v| format!("var {} = {}", name, v))
                                .or(Some(format!("var {}", name)));

                            result.symbols.push(ParsedSymbol {
                                name: name.to_string(),
                                kind: SymbolKind::Variable,
                                start_line: child.start_position().row + 1,
                                end_line: child.end_position().row + 1,
                                start_col: child.start_position().column,
                                end_col: child.end_position().column,
                                signature,
                                docstring: None,
                                parent: None,
                            });
                        }
                    }
                }
            }
        }

        fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "call_expression" {
                    if let Some(callee) = child.child_by_field_name("function") {
                        let callee_text = callee.utf8_text(source).unwrap_or("").to_string();
                        if !callee_text.is_empty() {
                            result.references.push(ParsedReference {
                                caller_symbol: None,
                                callee_symbol: callee_text,
                                ref_kind: "call".to_string(),
                                line: child.start_position().row + 1,
                            });
                        }
                    }
                }
            }
        }

        fn extract_j_sdoc(node: &Node, source: &[u8]) -> Option<String> {
            // Look for comment_line or comment nodes before the declaration
            // tree-sitter doesn't attach comments to the AST by default,
            // so we check the raw source for preceding comments
            let start_row = node.start_position().row;
            let lines: Vec<&str> = std::str::from_utf8(source)
                .unwrap_or("")
                .split('\n')
                .collect();

            let mut i = start_row.saturating_sub(1);
            let mut comment_lines = Vec::new();

            while i < lines.len() {
                let line = lines[i].trim();
                if line.starts_with("//") {
                    let text = line.strip_prefix("//").unwrap_or(line).trim();
                    comment_lines.insert(0, text);
                    i = i.saturating_sub(1);
                } else if line.starts_with("/*") {
                    // Multi-line comment - collect until */
                    let mut full = String::new();
                    let mut j = i;
                    while j < lines.len() {
                        full.push_str(lines[j]);
                        full.push('\n');
                        if lines[j].contains("*/") {
                            break;
                        }
                        j += 1;
                    }
                    let cleaned = full.trim_matches(|c| c == '/' || c == '*').trim();
                    let first_line = cleaned.lines().next().map(|l| l.to_string());
                    return first_line;
                } else {
                    break;
                }
            }

            if comment_lines.is_empty() {
                None
            } else {
                Some(comment_lines.join(" ").trim().to_string())
            }
        }
    }

    impl SourceCodeParser for JavaScriptParser {
        fn language(&self) -> &str {
            "javascript"
        }

        fn file_extensions(&self) -> &[&str] {
            &["js", "jsx", "mjs", "cjs"]
        }

        fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
            Self::do_parse(self, source)
        }
    }
}

// Re-export language-specific parsers
pub use javascript::JavaScriptParser;
pub use python::PythonParser;
pub use r#rust::RustParser;
pub use r#typecript::TypeScriptParser;
pub use r#go::GoParser;

// ---------------------------------------------------------------------------
// Rust Parser
// ---------------------------------------------------------------------------
//
// AST structure (tree-sitter-rust v0.24):
//
// function_item
//   fn
//   field: "name" -> identifier
//   generic_parameters
//   parameters
//   field: "return_type" -> primitive_type / path_type
//   field: "body" -> block
//
// struct_item
//   struct
//   field: "name" -> type_identifier
//   generic_parameters
//   field: "body" -> field_declaration_list
//
// enum_item
//   enum
//   field: "name" -> type_identifier
//   generic_parameters
//   field: "body" -> enum_variant_list
//     enum_variant
//       field: "name" -> field_identifier
//
// trait_item
//   trait
//   field: "name" -> type_identifier
//   generic_parameters
//   field: "body" -> declaration_list
//
// impl_item
//   impl
//   generic_parameters
//   field: "trait" -> path_type
//   field: "type" -> path_type
//   field: "body" -> declaration_list
//
// use_declaration
//   use
//   use_tree
//   mutable_specifier
//
// module_ -> path_type, call_expression

mod r#rust {
    use super::*;

    pub struct RustParser;

    impl RustParser {
        pub fn new() -> Result<Self, anyhow::Error> {
            let mut parser = tree_sitter::Parser::new();
            let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
            parser
                .set_language(&language)
                .context("Failed to load tree-sitter-rust")?;
            Ok(Self)
        }

        fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
            let mut parser = tree_sitter::Parser::new();
            let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
            parser
                .set_language(&language)
                .context("Failed to load tree-sitter-rust")?;

            let tree = parser.parse(source, None).context("tree-sitter parse failed")?;
            let root = tree.root_node();
            let source_bytes = source.as_bytes();
            let mut result = ParseResult {
                symbols: Vec::new(),
                references: Vec::new(),
                imports: Vec::new(),
            };

            Self::walk_node(&root, source_bytes, &mut result, None);
            Ok(result)
        }

        fn walk_node(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            parent: Option<String>,
        ) {
            let kind = node.kind();

            match kind {
                "function_item" => {
                    Self::extract_function(node, source, result, parent.as_deref());
                }
                "struct_item" => {
                    Self::extract_struct(node, source, result, parent.as_deref());
                }
                "enum_item" => {
                    Self::extract_enum(node, source, result, parent.as_deref());
                }
                "trait_item" => {
                    Self::extract_trait(node, source, result, parent.as_deref());
                }
                "impl_item" => {
                    Self::extract_impl(node, source, result, parent.as_deref());
                }
                "use_declaration" | "use" => {
                    Self::extract_import(node, source, result);
                }
                "type_alias" => {
                    Self::extract_type_alias(node, source, result, parent.as_deref());
                }
                _ => {}
            }

            // Extract call references
            Self::extract_calls(node, source, result);

            // Recurse into children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let new_parent = match child.kind() {
                    "impl_item" => {
                        child
                            .child_by_field_name("type")
                            .and_then(|n| n.utf8_text(source).ok())
                            .map(|s| s.to_string())
                    }
                    "trait_item" => {
                        child
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(source).ok())
                            .map(|s| s.to_string())
                    }
                    _ => parent.clone(),
                };
                Self::walk_node(&child, source, result, new_parent);
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

            // Build signature
            let params = node
                .child_by_field_name("parameters")
                .and_then(|p| p.utf8_text(source).ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "()".to_string());

            let return_type = node
                .child_by_field_name("return_type")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());

            let sig = if let Some(rt) = return_type {
                format!("fn {}{} -> {}", name, params, rt)
            } else {
                format!("fn {}{}", name, params)
            };

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
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_struct(
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

            let sig = format!("struct {}", name);
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Class, // Reuse Class for struct
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_enum(
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

            // Extract variants as sub-symbols
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "enum_variant_list" {
                    let mut variant_cursor = child.walk();
                    for variant in child.children(&mut variant_cursor) {
                        if variant.kind() == "enum_variant" {
                            if let Some(variant_name) = variant
                                .child_by_field_name("name")
                                .and_then(|n| n.utf8_text(source).ok())
                            {
                                result.symbols.push(ParsedSymbol {
                                    name: format!("{}::{}", name, variant_name),
                                    kind: SymbolKind::Variable,
                                    start_line: variant.start_position().row + 1,
                                    end_line: variant.end_position().row + 1,
                                    start_col: variant.start_position().column,
                                    end_col: variant.end_position().column,
                                    signature: Some(format!("enum variant {}", variant_name)),
                                    docstring: None,
                                    parent: Some(name.clone()),
                                });
                            }
                        }
                    }
                }
            }

            let sig = format!("enum {}", name);
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Enum,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_trait(
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

            let sig = format!("trait {}", name);
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Class, // Reuse Class for trait
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_impl(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            _parent: Option<&str>,
        ) {
            // Get impl target type
            let target = node
                .child_by_field_name("type")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());

            let Some(target) = target else {
                return;
            };

            // If impl for a trait, record inherit reference
            if let Some(trait_node) = node.child_by_field_name("trait") {
                if let Ok(trait_name) = trait_node.utf8_text(source) {
                    result.references.push(ParsedReference {
                        caller_symbol: Some(target.clone()),
                        callee_symbol: trait_name.to_string(),
                        ref_kind: "implement".to_string(),
                        line: node.start_position().row + 1,
                    });
                }
            }
        }

        fn extract_type_alias(
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

            let alias_type = node
                .child_by_field_name("type")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());

            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Variable,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: alias_type.map(|t| format!("type {}", t)),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

         fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                // Use declaration has either scoped_identifier or scoped_use_list
                let text = if child.kind() == "scoped_identifier" {
                    child.utf8_text(source).ok()
                } else if child.kind() == "scoped_use_list" {
                    // e.g. std::io::{Read, Write} — extract the base path
                    child.utf8_text(source).ok()
                } else {
                    None
                };

                if let Some(raw) = text {
                    let cleaned = raw.trim_start_matches("pub ").trim();
                    if !cleaned.is_empty() {
                        if cleaned.contains('{') {
                            if let Some(braces) = cleaned.split_once('{') {
                                let base = braces.0.trim();
                                let items: Vec<&str> = braces
                                    .1
                                    .trim_end_matches('}')
                                    .split(',')
                                    .map(|s| s.trim())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                                let full = format!("{}::{}", base, items.join("::"));
                                result.imports.push((full.clone(), full.clone(), "use".to_string()));
                                result.references.push(ParsedReference {
                                    caller_symbol: None,
                                    callee_symbol: full,
                                    ref_kind: "import".to_string(),
                                    line: node.start_position().row + 1,
                                });
                            }
                        } else {
                            result.imports.push((
                                cleaned.to_string(),
                                cleaned.to_string(),
                                "use".to_string(),
                            ));
                            result.references.push(ParsedReference {
                                caller_symbol: None,
                                callee_symbol: cleaned.to_string(),
                                ref_kind: "import".to_string(),
                                line: node.start_position().row + 1,
                            });
                        }
                    }
                }
            }
        }

        fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult) {
            if node.kind() == "call_expression" {
                if let Some(func) = node.child_by_field_name("function") {
                    let callee = func.utf8_text(source).unwrap_or("").to_string();
                    if !callee.is_empty() {
                        result.references.push(ParsedReference {
                            caller_symbol: None,
                            callee_symbol: callee,
                            ref_kind: "call".to_string(),
                            line: node.start_position().row + 1,
                        });
                    }
                }
            }

            // Recurse for nested calls
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "call_expression" {
                    Self::extract_calls(&child, source, result);
                }
            }
        }
    }

    impl SourceCodeParser for RustParser {
        fn language(&self) -> &str {
            "rust"
        }

        fn file_extensions(&self) -> &[&str] {
            &["rs"]
        }

        fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
            Self::do_parse(self, source)
        }
    }
}

// ---------------------------------------------------------------------------
// TypeScript Parser
// ---------------------------------------------------------------------------
//
// AST structure (tree-sitter-typescript v0.23):
// Uses TypeScript grammar which extends JavaScript.
// Key nodes:
//
// function_declaration -> same as JS
// class_declaration -> same as JS
// method_definition -> same as JS
//
// Additional TS-specific:
// type_alias -> type Identifier = ...
// interface_declaration -> interface Identifier ...
// module_declaration -> namespace Identifier { ... }
// type_annotation -> : Type
// type_parameter -> <T>
// import_statement -> same as JS
// export_statement -> same as JS

mod r#typecript {
    use super::*;

    pub struct TypeScriptParser;

    impl TypeScriptParser {
        pub fn new() -> Result<Self, anyhow::Error> {
            let mut parser = tree_sitter::Parser::new();
            let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
            parser
                .set_language(&language)
                .context("Failed to load tree-sitter-typescript")?;
            Ok(Self)
        }

        fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
            let mut parser = tree_sitter::Parser::new();
            let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
            parser
                .set_language(&language)
                .context("Failed to load tree-sitter-typescript")?;

            let tree = parser.parse(source, None).context("tree-sitter parse failed")?;
            let root = tree.root_node();
            let source_bytes = source.as_bytes();
            let mut result = ParseResult {
                symbols: Vec::new(),
                references: Vec::new(),
                imports: Vec::new(),
            };

            Self::walk_node(&root, source_bytes, &mut result, None);
            Ok(result)
        }

        fn walk_node(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            parent: Option<String>,
        ) {
            let kind = node.kind();

            match kind {
                "function_declaration" => {
                    Self::extract_function(node, source, result, parent.as_deref());
                }
                "class_declaration" => {
                    Self::extract_class(node, source, result, parent.as_deref());
                }
                "method_definition" => {
                    // Only extract if not already inside a class parent context
                    Self::extract_method(node, source, result, parent.as_deref());
                }
                "import_statement" => {
                    Self::extract_import(node, source, result);
                }
                "export_statement" => {
                    Self::extract_export(node, source, result, parent.as_deref());
                }
                "type_alias_declaration" | "type_annotation" => {
                    // type aliases
                    if kind == "type_alias_declaration" {
                        Self::extract_type_alias(node, source, result, parent.as_deref());
                    }
                }
                "interface_declaration" => {
                    Self::extract_interface(node, source, result, parent.as_deref());
                }
                "enum_declaration" => {
                    Self::extract_enum(node, source, result, parent.as_deref());
                }
                "module_declaration" => {
                    Self::extract_module(node, source, result, parent.as_deref());
                }
                "variable_declaration" => {
                    // Top-level variables (const, let, var)
                    if parent.is_none() {
                        Self::extract_variable(node, source, result);
                    }
                }
                _ => {}
            }

            // Extract call references
            Self::extract_calls(node, source, result);

            // Recurse into children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let new_parent = if child.kind() == "class_declaration" {
                    child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string())
                } else {
                    parent.clone()
                };
                Self::walk_node(&child, source, result, new_parent);
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

            let params = node
                .child_by_field_name("parameters")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "()".to_string());

            let is_async = node
                .child_by_field_name("async")
                .is_some()
                .then_some("async ")
                .unwrap_or("");

            let sig = format!("{}function {}({})", is_async, name, params);
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Function,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_class(
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

            // Get superclass
            let superclass = node
                .child_by_field_name("superClass")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());

            let signature = if let Some(sc) = &superclass {
                format!("class {} extends {}", name, sc)
            } else {
                format!("class {}", name)
            };

            // Inheritance reference
            if let Some(sc) = superclass {
                result.references.push(ParsedReference {
                    caller_symbol: Some(name.clone()),
                    callee_symbol: sc,
                    ref_kind: "inherit".to_string(),
                    line: node.start_position().row + 1,
                });
            }

            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Class,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(signature),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_method(
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

            let params = node
                .child_by_field_name("parameters")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "()".to_string());

            let sig = format!("{}({})", name, params);
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Method,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
            let module_node = node.child_by_field_name("source");
            let Some(module) = module_node.and_then(|n| n.utf8_text(source).ok()) else {
                return;
            };

            let cleaned = module.trim_matches('"').trim_matches('\'').to_string();

            // Extract imported names
            let mut names = Vec::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "import_clause" {
                    let mut ic_cursor = child.walk();
                    for ic_child in child.children(&mut ic_cursor) {
                        if ic_child.kind() == "identifier" {
                            if let Ok(n) = ic_child.utf8_text(source) {
                                names.push(n.to_string());
                            }
                        }
                        if ic_child.kind() == "named_imports" {
                            let mut ni_cursor = ic_child.walk();
                            for ns in ic_child.children(&mut ni_cursor) {
                                if ns.kind() == "import_specifier" {
                                    // import_specifier has identifier child
                                    let mut spec_cursor = ns.walk();
                                    for spec_child in ns.children(&mut spec_cursor) {
                                        if spec_child.kind() == "identifier" {
                                            if let Ok(name) = spec_child.utf8_text(source) {
                                                names.push(name.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            for name in &names {
                result.imports.push((
                    cleaned.clone(),
                    name.clone(),
                    "import".to_string(),
                ));
                result.references.push(ParsedReference {
                    caller_symbol: None,
                    callee_symbol: name.clone(),
                    ref_kind: "import".to_string(),
                    line: node.start_position().row + 1,
                });
            }

            result.symbols.push(ParsedSymbol {
                name: cleaned.clone(),
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

        fn extract_export(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            _parent: Option<&str>,
        ) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "export_clause" {
                    let mut ec_cursor = child.walk();
                    for ec_child in child.children(&mut ec_cursor) {
                        if ec_child.kind() == "export_specifier" {
                            if let Ok(name) = ec_child.utf8_text(source) {
                                result.symbols.push(ParsedSymbol {
                                    name: name.to_string(),
                                    kind: SymbolKind::Export,
                                    start_line: ec_child.start_position().row + 1,
                                    end_line: ec_child.end_position().row + 1,
                                    start_col: ec_child.start_position().column,
                                    end_col: ec_child.end_position().column,
                                    signature: None,
                                    docstring: None,
                                    parent: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        fn extract_type_alias(
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

            let sig = format!("type {}", name);
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Class, // Reuse Class for type alias
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_enum(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            parent: Option<&str>,
        ) {
            let name = node
                .child_by_field_name("name")
                .or_else(|| node.child(1))
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());

            let Some(name) = name else {
                return;
            };

            // Extract enum members as sub-symbols
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "enum_body" {
                    let mut body_cursor = child.walk();
                    for member in child.children(&mut body_cursor) {
                        if member.kind() == "property_identifier" {
                            if let Some(member_name) = member.utf8_text(source).ok() {
                                result.symbols.push(ParsedSymbol {
                                    name: format!("{}::{}", name, member_name),
                                    kind: SymbolKind::Variable,
                                    start_line: member.start_position().row + 1,
                                    end_line: member.end_position().row + 1,
                                    start_col: member.start_position().column,
                                    end_col: member.end_position().column,
                                    signature: Some(format!("enum member {}", member_name)),
                                    docstring: None,
                                    parent: Some(name.clone()),
                                });
                            }
                        }
                    }
                }
            }

            let sig = format!("enum {}", name);
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Enum,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_interface(
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

            let sig = format!("interface {}", name);
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Class, // Reuse Class for interface
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_module(
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

            let sig = format!("namespace {}", name);
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Module,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_variable(node: &Node, source: &[u8], result: &mut ParseResult) {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());

            let Some(name) = name else {
                return;
            };

            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Variable,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: None,
                docstring: None,
                parent: None,
            });
        }

        fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "call_expression" {
                    if let Some(func) = child.child_by_field_name("function") {
                        let callee = func.utf8_text(source).unwrap_or("").to_string();
                        if !callee.is_empty() {
                            result.references.push(ParsedReference {
                                caller_symbol: None,
                                callee_symbol: callee,
                                ref_kind: "call".to_string(),
                                line: child.start_position().row + 1,
                            });
                        }
                    }
                }
                // Recurse
                Self::extract_calls(&child, source, result);
            }
        }
    }

    impl SourceCodeParser for TypeScriptParser {
        fn language(&self) -> &str {
            "typescript"
        }

        fn file_extensions(&self) -> &[&str] {
            &["ts", "tsx", "mts", "cts"]
        }

        fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
            Self::do_parse(self, source)
        }
    }
}

// ---------------------------------------------------------------------------
// Go Parser
// ---------------------------------------------------------------------------
//
// AST structure (tree-sitter-go v0.23):
//
// function_declaration
//   func
//   field: "name" -> field_identifier
//   field: "parameters" -> parameter_list
//   field: "result" -> parameter_list / type_identifier
//   field: "body" -> block
//
// type_spec
//   type
//   field: "name" -> type_identifier
//   field: "type" -> struct_type / interface_type / etc.
//
// struct_type
//   struct
//   field_list
//     field
//       field_identifier
//       type_identifier / basic_type
//
// interface_type
//   interface
//   field_list
//     field
//       field_identifier
//       parameter_type
//         parameter_list
//
// import_declaration
//   import
//   import_specifier
//     import_path (string)
//
// call_expression
//   field: "function" -> identifier / selector_expression
//   field: "arguments" -> arguments

mod r#go {
    use super::*;

    pub struct GoParser;

    impl GoParser {
        pub fn new() -> Result<Self, anyhow::Error> {
            let mut parser = tree_sitter::Parser::new();
            let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
            parser
                .set_language(&language)
                .context("Failed to load tree-sitter-go")?;
            Ok(Self)
        }

        fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
            let mut parser = tree_sitter::Parser::new();
            let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
            parser
                .set_language(&language)
                .context("Failed to load tree-sitter-go")?;

            let tree = parser.parse(source, None).context("tree-sitter parse failed")?;
            let root = tree.root_node();
            let source_bytes = source.as_bytes();
            let mut result = ParseResult {
                symbols: Vec::new(),
                references: Vec::new(),
                imports: Vec::new(),
            };

            Self::walk_node(&root, source_bytes, &mut result, None);
            Ok(result)
        }

        fn walk_node(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
            parent: Option<String>,
        ) {
            let kind = node.kind();

            match kind {
                "function_declaration" => {
                    Self::extract_function(node, source, result, parent.as_deref());
                }
                "type_spec" => {
                    Self::extract_type(node, source, result, parent.as_deref());
                }
                "import_declaration" => {
                    Self::extract_import(node, source, result);
                }
                "var_declaration" => {
                    if parent.is_none() {
                        Self::extract_var(node, source, result);
                    }
                }
                "method_declaration" => {
                    Self::extract_method(node, source, result);
                }
                _ => {}
            }

            // Extract call references
            Self::extract_calls(node, source, result);

            // Recurse into children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                Self::walk_node(&child, source, result, parent.clone());
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

            let params = node
                .child_by_field_name("parameters")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "()".to_string());

            let return_type = node
                .child_by_field_name("result")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());

            let sig = if let Some(rt) = return_type {
                format!("func {}{} {}", name, params, rt)
            } else {
                format!("func {}{}", name, params)
            };

            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Function,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_type(
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

            // Determine type kind
            let type_kind = node
                .child_by_field_name("type")
                .map(|n| n.kind())
                .unwrap_or("");

            let kind = match type_kind {
                "struct_type" | "interface_type" => SymbolKind::Class,
                _ => SymbolKind::Variable, // type alias, etc.
            };

            let sig = format!("type {}", name);
            result.symbols.push(ParsedSymbol {
                name,
                kind,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }

        fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
            // Helper: extract module path from an import_spec node
            let extract_spec = |spec: &Node| -> Option<String> {
                // import_spec has interpreted_string_literal child (NOT a field)
                let mut spec_cursor = spec.walk();
                for child in spec.children(&mut spec_cursor) {
                    if child.kind() == "interpreted_string_literal" {
                        if let Ok(text) = child.utf8_text(source) {
                            let cleaned = text.trim_matches('"').to_string();
                            if !cleaned.is_empty() {
                                return Some(cleaned);
                            }
                        }
                    }
                }
                None
            };

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                // Single import: import "fmt"
                if child.kind() == "import_spec" {
                    if let Some(cleaned) = extract_spec(&child) {
                        result.imports.push((
                            cleaned.clone(),
                            cleaned.clone(),
                            "import".to_string(),
                        ));
                        result.references.push(ParsedReference {
                            caller_symbol: None,
                            callee_symbol: cleaned,
                            ref_kind: "import".to_string(),
                            line: node.start_position().row + 1,
                        });
                    }
                }
                // Grouped imports: import ( "a" "b" )
                if child.kind() == "import_spec_list" {
                    let mut list_cursor = child.walk();
                    for spec in child.children(&mut list_cursor) {
                        if spec.kind() == "import_spec" {
                            if let Some(cleaned) = extract_spec(&spec) {
                                result.imports.push((
                                    cleaned.clone(),
                                    cleaned.clone(),
                                    "import".to_string(),
                                ));
                                result.references.push(ParsedReference {
                                    caller_symbol: None,
                                    callee_symbol: cleaned,
                                    ref_kind: "import".to_string(),
                                    line: node.start_position().row + 1,
                                });
                            }
                        }
                    }
                }
            }
        }

        fn extract_var(node: &Node, source: &[u8], result: &mut ParseResult) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "var_spec" {
                    let mut spec_cursor = child.walk();
                    for spec_child in child.children(&mut spec_cursor) {
                        if spec_child.kind() == "identifier" {
                            if let Ok(name) = spec_child.utf8_text(source) {
                                result.symbols.push(ParsedSymbol {
                                    name: name.to_string(),
                                    kind: SymbolKind::Variable,
                                    start_line: spec_child.start_position().row + 1,
                                    end_line: spec_child.end_position().row + 1,
                                    start_col: spec_child.start_position().column,
                                    end_col: spec_child.end_position().column,
                                    signature: None,
                                    docstring: None,
                                    parent: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        fn extract_method(
            node: &Node,
            source: &[u8],
            result: &mut ParseResult,
        ) {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());

            let Some(name) = name else {
                return;
            };

            // Get receiver type for parent
            let receiver = node
                .child_by_field_name("receiver")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| {
                    let s = s.trim_start_matches('(').trim_end_matches(')');
                    let type_part = s.split_whitespace().nth(1).unwrap_or(s);
                    type_part.trim_start_matches('*').trim_start_matches('&').to_string()
                });

            let params = node
                .child_by_field_name("parameters")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "()".to_string());

            let sig = format!(
                "({}) {}({})",
                receiver.as_deref().unwrap_or(""),
                name,
                params
            );
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Method,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(sig),
                docstring: None,
                parent: receiver,
            });
        }

        fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult) {
            if node.kind() == "call_expression" {
                if let Some(func) = node.child_by_field_name("function") {
                    let callee = func.utf8_text(source).unwrap_or("").to_string();
                    if !callee.is_empty() {
                        result.references.push(ParsedReference {
                            caller_symbol: None,
                            callee_symbol: callee,
                            ref_kind: "call".to_string(),
                            line: node.start_position().row + 1,
                        });
                    }
                }
            }

            // Recurse
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "call_expression" {
                    Self::extract_calls(&child, source, result);
                }
            }
        }
    }

    impl SourceCodeParser for GoParser {
        fn language(&self) -> &str {
            "go"
        }

        fn file_extensions(&self) -> &[&str] {
            &["go"]
        }

        fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
            Self::do_parse(self, source)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_parse_py(code: &str) -> ParseResult {
        let mut parser = PythonParser::new().unwrap();
        parser.parse(code).unwrap()
    }

    fn test_parse_js(code: &str) -> ParseResult {
        let mut parser = JavaScriptParser::new().unwrap();
        parser.parse(code).unwrap()
    }

    // ---- Python tests ----

    #[test]
    fn test_parse_function() {
        let code = r#"
def hello(name: str) -> str:
    """Say hello."""
    return f"Hello, {name}!"
"#;
        let result = test_parse_py(code);
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "hello");
        assert_eq!(result.symbols[0].kind, SymbolKind::Function);
        assert!(result.symbols[0].docstring.is_some());
    }

    #[test]
    fn test_parse_class() {
        let code = r#"
class Animal:
    pass

class Dog(Animal):
    def bark(self):
        pass
"#;
        let result = test_parse_py(code);
        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(classes.len(), 2);
        assert_eq!(classes[0].name, "Animal");
        assert_eq!(classes[1].name, "Dog");
    }

    #[test]
    fn test_parse_imports() {
        let code = r#"
import os
from sys import path
"#;
        let result = test_parse_py(code);
        assert!(!result.imports.is_empty());
    }

    // ---- JavaScript tests ----

    #[test]
    fn test_js_function_declaration() {
        let code = r#"
function greet(name) {
    return `Hello, ${name}!`;
}

async function fetchData(url) {
    return fetch(url);
}
"#;
        let result = test_parse_js(code);
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 2);
        assert_eq!(funcs[0].name, "greet");
        assert_eq!(funcs[1].name, "fetchData");
        assert!(funcs[1].signature.as_ref().unwrap().contains("async"));
    }

    #[test]
    fn test_js_class_and_methods() {
        let code = r#"
class Animal {
    constructor(name) {
        this.name = name;
    }

    speak() {
        return "sound";
    }
}

class Dog extends Animal {
    bark() {
        return "woof";
    }
}
"#;
        let result = test_parse_js(code);
        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();

        assert_eq!(classes.len(), 2);
        assert_eq!(classes[0].name, "Animal");
        assert_eq!(classes[1].name, "Dog");

        // Dog extends Animal
        let inherits: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.ref_kind == "inherit")
            .collect();
        assert_eq!(inherits.len(), 1);
        assert_eq!(inherits[0].callee_symbol, "Animal");

        // Methods: constructor, speak (Animal) + bark (Dog)
        let method_names: Vec<_> = methods.iter().map(|m| m.name.as_str()).collect();
        assert!(methods.len() >= 3);
        assert!(method_names.contains(&"constructor"));
        assert!(method_names.contains(&"speak"));
        assert!(method_names.contains(&"bark"));
    }

    #[test]
    fn test_js_imports() {
        let code = r#"
import React from 'react';
import { useState, useEffect } from 'react';
"#;
        let result = test_parse_js(code);
        assert!(!result.imports.is_empty());
        // Should have React (default) + useState, useEffect (named)
        assert!(result.imports.len() >= 2);

        let import_symbols: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Import)
            .collect();
        assert!(!import_symbols.is_empty());
    }

    #[test]
    fn test_js_arrow_function() {
        let code = r#"
const add = (a, b) => a + b;
const multiply = (x, y) => { return x * y; };
"#;
        let result = test_parse_js(code);
        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].name, "add");
        assert_eq!(vars[1].name, "multiply");
    }

    #[test]
    fn test_js_exports() {
        let code = r#"
function greet() {}
export default greet;
export { greet };
"#;
        let result = test_parse_js(code);
        let exports: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Export)
            .collect();
        assert!(!exports.is_empty());
    }

    #[test]
    fn test_js_call_references() {
        let code = r#"
const result = fetch(url);
response.json();
"#;
        let result = test_parse_js(code);
        let calls: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.ref_kind == "call")
            .collect();
        assert!(!calls.is_empty());
    }

    // ---- Trait tests ----

    #[test]
    fn test_parser_trait() {
        let py: Box<dyn SourceCodeParser> = Box::new(PythonParser::new().unwrap());
        assert_eq!(py.language(), "python");
        assert!(py.file_extensions().contains(&"py"));

        let js: Box<dyn SourceCodeParser> = Box::new(JavaScriptParser::new().unwrap());
        assert_eq!(js.language(), "javascript");
        assert!(js.file_extensions().contains(&"js"));

        let rust_p: Box<dyn SourceCodeParser> = Box::new(RustParser::new().unwrap());
        assert_eq!(rust_p.language(), "rust");
        assert!(rust_p.file_extensions().contains(&"rs"));

        let ts: Box<dyn SourceCodeParser> = Box::new(TypeScriptParser::new().unwrap());
        assert_eq!(ts.language(), "typescript");
        assert!(ts.file_extensions().contains(&"ts"));

        let go: Box<dyn SourceCodeParser> = Box::new(GoParser::new().unwrap());
        assert_eq!(go.language(), "go");
        assert!(go.file_extensions().contains(&"go"));
    }

    #[test]
    fn test_rust_function() {
        let mut parser = RustParser::new().unwrap();
        let code = r#"
fn hello(name: &str) -> String {
    format!("Hello, {}!", name)
}
"#;
        let result = parser.parse(code).unwrap();
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "hello");
        assert_eq!(result.symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_rust_struct() {
        let mut parser = RustParser::new().unwrap();
        let code = r#"
struct User {
    name: String,
    age: u32,
}
"#;
        let result = parser.parse(code).unwrap();
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "User");
    }

    #[test]
    fn test_rust_enum() {
        let mut parser = RustParser::new().unwrap();
        let code = r#"
enum Status {
    Ok,
    Err(String),
}
"#;
        let result = parser.parse(code).unwrap();
        // enum itself + 2 variants
        assert!(result.symbols.len() >= 3);
        assert!(result.symbols.iter().any(|s| s.name == "Status"));
    }

    #[test]
    fn test_rust_trait_and_impl() {
        let mut parser = RustParser::new().unwrap();
        let code = r#"
trait Drawable {
    fn draw(&self);
}

struct Circle;

impl Drawable for Circle {}
"#;
        let result = parser.parse(code).unwrap();
        assert!(result.symbols.iter().any(|s| s.name == "Drawable"));
        assert!(result.symbols.iter().any(|s| s.name == "Circle"));
    }

    #[test]
    fn test_rust_use_import() {
        let mut parser = RustParser::new().unwrap();
        let code = r#"
use std::collections::HashMap;
use std::io;
"#;
        let result = parser.parse(code).unwrap();
        assert!(!result.imports.is_empty());
    }

    #[test]
    fn test_ts_class() {
        let mut parser = TypeScriptParser::new().unwrap();
        let code = r#"
class Animal {
    constructor(public name: string) {}
    speak(): void {
        console.log(this.name);
    }
}
"#;
        let result = parser.parse(code).unwrap();
        assert!(result.symbols.iter().any(|s| s.name == "Animal"));
    }

    #[test]
    fn test_ts_interface() {
        let mut parser = TypeScriptParser::new().unwrap();
        let code = r#"
interface Config {
    host: string;
    port: number;
}
"#;
        let result = parser.parse(code).unwrap();
        assert!(result.symbols.iter().any(|s| s.name == "Config"));
    }

    #[test]
    fn test_ts_type_alias() {
        let mut parser = TypeScriptParser::new().unwrap();
        let code = r#"
type Result<T> = { ok: true; value: T } | { ok: false; error: string };
"#;
        let result = parser.parse(code).unwrap();
        assert!(result.symbols.iter().any(|s| s.name == "Result"));
    }

    #[test]
    fn test_ts_import() {
        let mut parser = TypeScriptParser::new().unwrap();
        let code = r#"
import { Foo, Bar } from 'bar';
import React from 'react';
import * as utils from '@utils';
"#;
        let result = parser.parse(code).unwrap();
        assert!(!result.imports.is_empty());
        assert!(result.imports.iter().any(|(m, _, _)| m == "bar"));
        assert!(result.imports.iter().any(|(m, _, _)| m == "react"));
    }

    #[test]
    fn test_ts_export() {
        let mut parser = TypeScriptParser::new().unwrap();
        let code = r#"
export { Config };
export default function main() {}
"#;
        let result = parser.parse(code).unwrap();
        let exports: Vec<&str> = result.symbols.iter()
            .filter(|s| s.kind == SymbolKind::Export)
            .map(|s| s.name.as_str())
            .collect();
        assert!(exports.contains(&"Config"));
    }

    #[test]
    fn test_ts_function() {
        let mut parser = TypeScriptParser::new().unwrap();
        let code = r#"
function greet(name: string): void {}
async function fetchData(): Promise<string> { return ""; }
"#;
        let result = parser.parse(code).unwrap();
        assert!(result.symbols.iter().any(|s| s.name == "greet"));
        assert!(result.symbols.iter().any(|s| s.name == "fetchData"));
    }

    #[test]
    fn test_ts_enum() {
        let mut parser = TypeScriptParser::new().unwrap();
        let code = r#"
enum Direction { Up, Down, Left, Right }
"#;
        let result = parser.parse(code).unwrap();
        let enum_sym = result
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Enum)
            .expect("enum symbol not found");
        assert_eq!(enum_sym.name, "Direction");
        assert_eq!(enum_sym.kind, SymbolKind::Enum);
    }

    #[test]
    fn test_ts_generic_type() {
        let mut parser = TypeScriptParser::new().unwrap();
        let code = r#"
type Result<T> = { ok: true; value: T } | { ok: false; error: string };
"#;
        let result = parser.parse(code).unwrap();
        let sym = result.symbols.iter().find(|s| s.name == "Result").unwrap();
        assert!(sym.signature.as_deref() == Some("type Result"));
    }

    #[test]
    fn test_go_function() {
        let mut parser = GoParser::new().unwrap();
        let code = r#"
package main

func Hello(name string) string {
    return "Hello, " + name
}
"#;
        let result = parser.parse(code).unwrap();
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "Hello");
    }

    #[test]
    fn test_go_struct() {
        let mut parser = GoParser::new().unwrap();
        let code = r#"
package main

type User struct {
    Name string
    Age  int
}
"#;
        let result = parser.parse(code).unwrap();
        assert!(result.symbols.iter().any(|s| s.name == "User"));
    }

    #[test]
    fn test_go_method() {
        let mut parser = GoParser::new().unwrap();
        let code = r#"
package main

type Counter struct {
    Value int
}

func (c *Counter) Increment() {
    c.Value++
}
"#;
        let result = parser.parse(code).unwrap();
        let method = result.symbols.iter().find(|s| s.name == "Increment").unwrap();
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.parent.as_deref(), Some("Counter"));
    }

    #[test]
    fn test_go_import() {
        let mut parser = GoParser::new().unwrap();
        let code = r#"
package main

import (
    "fmt"
    "net/http"
)

func main() {
    fmt.Println(http.StatusOK)
}
"#;
        let result = parser.parse(code).unwrap();
        assert!(!result.imports.is_empty());
        assert!(result.imports.iter().any(|(m, _, _)| m == "fmt"));
    }
}
