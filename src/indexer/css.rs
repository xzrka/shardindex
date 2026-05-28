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
            string_literals: Vec::new(),
        };

        Self::walk_node(&root, source_bytes, &mut result, None);
        Ok(result)
    }

    fn find_child<'a>(node: &'a Node, kind: &str) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        node.children(&mut cursor).find(|n| n.kind() == kind)
    }

    fn walk_node(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<String>) {
        match node.kind() {
            "rule_set" => Self::extract_rule_set(node, source, result),
            "keyframes_rule" => Self::extract_keyframes(node, source, result),
            "import_statement" => Self::extract_import(node, source, result),
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match child.kind() {
                "rule_set" | "keyframes_rule" => Self::find_child(&child, "selectors")
                    .and_then(|s| s.child(0))
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                _ => parent.clone(),
            };
            Self::walk_node(&child, source, result, new_parent);
        }
    }

    fn extract_rule_set(node: &Node, source: &[u8], result: &mut ParseResult) {
        let selector = Self::find_child(node, "selectors")
            .and_then(|s| s.utf8_text(source).ok())
            .map(|s| s.to_string().trim().to_string());
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
        let name = Self::find_child(node, "keyframes_name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Function,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("keyframes".to_string()),
            docstring: None,
            parent: None,
        });
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
        if let Some(import_path) = Self::find_child(node, "string")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.trim_matches('"').trim_matches('\'').to_string())
        {
            result.imports.push((import_path, String::new(), String::new()));
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
