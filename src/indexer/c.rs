use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct CParser;

impl CParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_c::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-c")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_c::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-c")?;

        let tree = parser
            .parse(source, None)
            .context("tree-sitter parse failed")?;
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

    fn walk_node(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<String>) {
        let kind = node.kind();

        match kind {
            "function_definition" => {
                Self::extract_function(node, source, result, parent.as_deref());
            }
            "struct_specifier" => {
                Self::extract_struct(node, source, result, parent.as_deref());
            }
            "typedef_declaration" => {
                Self::extract_typedef(node, source, result, parent.as_deref());
            }
            "enum_specifier" => {
                Self::extract_enum(node, source, result, parent.as_deref());
            }
            "#include" => {
                Self::extract_include(node, source, result);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match kind {
                "struct_specifier" => node
                    .child_by_field_name("type_identifier")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                _ => parent.clone(),
            };
            Self::walk_node(&child, source, result, new_parent);
        }
    }

    fn extract_function(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let declarator = node.child_by_field_name("declarator");
        let Some(decl_text) = declarator.and_then(|n| n.utf8_text(source).ok()) else {
            return;
        };
        let decl_text = decl_text.to_string();

        // Extract function name from declarator
        let name = Self::extract_function_name(&decl_text);
        let Some(name) = name else {
            return;
        };

        let params = node
            .child_by_field_name("parameters")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "()".to_string());

        let return_type = node
            .child_by_field_name("type")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let sig = if let Some(rt) = return_type {
            format!("{} {}{}", rt, name, params)
        } else {
            format!("{}{}", name, params)
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

        Self::extract_calls(node, source, result);
    }

    fn extract_function_name(declarator: &str) -> Option<String> {
        let decl = declarator.trim();
        // Remove parentheses for function pointers
        let decl = decl.trim_start_matches('(').trim_end_matches(')');
        // Remove leading * for pointers
        let decl = decl.trim_start_matches('*');
        // Split on ( to get the name part
        if let Some(idx) = decl.find('(') {
            let name = decl[..idx].trim().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
        // Fallback: take first identifier-like token
        let name = decl.split(|c: char| !c.is_alphanumeric() && c != '_').find(|s| !s.is_empty())?;
        Some(name.to_string())
    }

    fn extract_struct(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        // In tree-sitter-c, struct_specifier uses "body" field or just has a field_declaration_list child
        let has_body = node.child_by_field_name("body").is_some()
            || node.child_by_field_name("field_declaration_list").is_some();
        if !has_body {
            return;
        }

        let name = node
            .child_by_field_name("type_identifier")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let Some(name) = name else {
            return;
        };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("struct".to_string()),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_typedef(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let type_identifier = node
            .child_by_field_name("type")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        if let Some(name) = type_identifier {
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::TypeAlias,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some("typedef".to_string()),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }
    }

    fn extract_enum(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("type_identifier")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        if let Some(name) = name {
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Enum,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some("enum".to_string()),
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }
    }

    fn extract_include(node: &Node, source: &[u8], result: &mut ParseResult) {
        let path_node = node.child_by_field_name("path");
        if let Some(path) = path_node.and_then(|n| n.utf8_text(source).ok()) {
            let cleaned = path.trim_matches('"').trim_matches('<').trim_matches('>').to_string();
            if !cleaned.is_empty() {
                result.imports.push((
                    cleaned.clone(),
                    cleaned.clone(),
                    "include".to_string(),
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

    fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult) {
        if node.kind() == "call_expression" {
            let function = node.child_by_field_name("function");
            if let Some(callee) = function.and_then(|n| n.utf8_text(source).ok()) {
                let callee = callee.to_string();
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

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "call_expression" {
                Self::extract_calls(&child, source, result);
            }
        }
    }
}

impl SourceCodeParser for CParser {
    fn language(&self) -> &str {
        "c"
    }

    fn file_extensions(&self) -> &[&str] {
        &["c", "h"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
