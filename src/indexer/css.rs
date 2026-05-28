use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct CssParser;

impl CssParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_css::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-css")?;

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

    fn walk_node(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<String>,
    ) {
        match node.kind() {
            "rule" => {
                Self::extract_rule(node, source, result);
            }
            "keyframes_rule" => {
                Self::extract_keyframes(node, source, result);
            }
            "import_statement" => {
                Self::extract_import(node, source, result);
            }
            "custom_token" => {
                Self::extract_custom_property(node, source, result, parent.as_deref());
            }
            "at_rule" => {
                Self::extract_at_rule(node, source, result);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = if child.kind() == "rule" {
                child
                    .child_by_field_name("prelude")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string())
            } else {
                parent.clone()
            };
            Self::walk_node(&child, source, result, new_parent);
        }
    }

    fn extract_rule(node: &Node, source: &[u8], result: &mut ParseResult) {
        let selector = node
            .child_by_field_name("prelude")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(selector) = selector else { return };

        result.symbols.push(ParsedSymbol {
            name: selector.clone(),
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(selector),
            docstring: None,
            parent: None,
        });
    }

    fn extract_keyframes(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Function,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("@keyframes {}", name)),
            docstring: None,
            parent: None,
        });
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
        if let Ok(text) = node.utf8_text(source) {
            result.imports.push((
                text.trim().to_string(),
                "import".to_string(),
                "css_import".to_string(),
            ));
        }
    }

    fn extract_custom_property(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        if let Ok(name) = node.utf8_text(source) {
            if name.starts_with("--") {
                result.symbols.push(ParsedSymbol {
                    name: name.trim().to_string(),
                    kind: SymbolKind::Constant,
                    start_line: node.start_position().row + 1,
                    end_line: node.end_position().row + 1,
                    start_col: node.start_position().column,
                    end_col: node.end_position().column,
                    signature: None,
                    docstring: None,
                    parent: parent.map(|s| s.to_string()),
                });
            }
        }
    }

    fn extract_at_rule(node: &Node, source: &[u8], result: &mut ParseResult) {
        if let Ok(text) = node.utf8_text(source) {
            let trimmed = text.trim().to_string();
            if trimmed.starts_with("@") && !trimmed.starts_with("@keyframes") {
                result.symbols.push(ParsedSymbol {
                    name: trimmed.clone(),
                    kind: SymbolKind::Module,
                    start_line: node.start_position().row + 1,
                    end_line: node.end_position().row + 1,
                    start_col: node.start_position().column,
                    end_col: node.end_position().column,
                    signature: Some(trimmed),
                    docstring: None,
                    parent: None,
                });
            }
        }
    }
}

impl SourceCodeParser for CssParser {
    fn language(&self) -> &str {
        "css"
    }

    fn file_extensions(&self) -> &[&str] {
        &["css", "scss", "sass"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        self.do_parse(source)
    }
}
