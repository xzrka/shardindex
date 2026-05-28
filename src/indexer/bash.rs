use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct BashParser;

impl BashParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_bash::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-bash")?;

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
            "function_definition" => {
                Self::extract_function(node, source, result);
            }
            "command" => {
                Self::extract_command_ref(node, source, result);
            }
            "command_name" => {
                Self::extract_command_name(node, source, result);
            }
            "source" => {
                Self::extract_source(node, source, result);
            }
            "variable_assignment" => {
                Self::extract_variable(node, source, result);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_node(&child, source, result, parent.clone());
        }
    }

    fn extract_function(node: &Node, source: &[u8], result: &mut ParseResult) {
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
            signature: Some(format!("function {}", name)),
            docstring: None,
            parent: None,
        });
    }

    fn extract_command_ref(node: &Node, source: &[u8], result: &mut ParseResult) {
        if let Ok(text) = node.utf8_text(source) {
            let trimmed = text.trim();
            if !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !trimmed.starts_with('(')
                && !trimmed.starts_with(')')
            {
                result.references.push(ParsedReference {
                    caller_symbol: None,
                    callee_symbol: trimmed.to_string(),
                    ref_kind: "command".to_string(),
                    line: node.start_position().row + 1,
                });
            }
        }
    }

    fn extract_command_name(node: &Node, source: &[u8], result: &mut ParseResult) {
        if let Ok(name) = node.utf8_text(source) {
            result.references.push(ParsedReference {
                caller_symbol: None,
                callee_symbol: name.trim().to_string(),
                ref_kind: "command".to_string(),
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_source(node: &Node, source: &[u8], result: &mut ParseResult) {
        if let Ok(text) = node.utf8_text(source) {
            result.imports.push((
                text.trim().to_string(),
                "source".to_string(),
                "shell_source".to_string(),
            ));
        }
    }

    fn extract_variable(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Variable,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("export {}", name)),
            docstring: None,
            parent: None,
        });
    }
}

impl SourceCodeParser for BashParser {
    fn language(&self) -> &str {
        "bash"
    }

    fn file_extensions(&self) -> &[&str] {
        &["sh", "bash", "zsh"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        self.do_parse(source)
    }
}
