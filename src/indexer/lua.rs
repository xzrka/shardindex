use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct LuaParser;

impl LuaParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_lua::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-lua")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_lua::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-lua")?;

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
            "function_declaration" => {
                Self::extract_function(node, source, result, parent.as_deref());
            }
            "local_function" => {
                Self::extract_local_function(node, source, result, parent.as_deref());
            }
            "require_call" => {
                Self::extract_require(node, source, result);
            }
            "assignment_statement" => {
                Self::extract_assignment(node, source, result, parent.as_deref());
            }
            _ => {}
        }

        // Extract call references
        Self::extract_calls(node, source, result);

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_node(&child, source, result, parent.clone());
        }
    }

    fn extract_function(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
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

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Function,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("function {}{}", name, params)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_local_function(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
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

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Function,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("local function {}{}", name, params)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_require(node: &Node, source: &[u8], result: &mut ParseResult) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "string" {
                if let Ok(text) = child.utf8_text(source) {
                    let cleaned = text.trim_matches('"').trim_matches('\'').to_string();
                    if !cleaned.is_empty() {
                        result.imports.push((
                            cleaned.clone(),
                            cleaned.clone(),
                            "require".to_string(),
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

    fn extract_assignment(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" && child.is_named() {
                if let Ok(name) = child.utf8_text(source) {
                    // Only top-level assignments
                    if node.parent().map_or(false, |p| p.kind() == "chunk" || p.kind() == "do_block") {
                        result.symbols.push(ParsedSymbol {
                            name: name.to_string(),
                            kind: SymbolKind::Variable,
                            start_line: child.start_position().row + 1,
                            end_line: child.end_position().row + 1,
                            start_col: child.start_position().column,
                            end_col: child.end_position().column,
                            signature: None,
                            docstring: None,
                            parent: parent.map(|s| s.to_string()),
                        });
                    }
                }
            }
        }
    }

    fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult) {
        if node.kind() == "function_call" {
            let func = node.child_by_field_name("function");
            if let Some(name) = func.and_then(|n| n.utf8_text(source).ok()) {
                let callee = name.to_string();
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
            if child.kind() == "function_call" {
                Self::extract_calls(&child, source, result);
            }
        }
    }
}

impl SourceCodeParser for LuaParser {
    fn language(&self) -> &str {
        "lua"
    }

    fn file_extensions(&self) -> &[&str] {
        &["lua"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
