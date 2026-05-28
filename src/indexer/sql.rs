use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct SqlParser;

impl SqlParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_sequel::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-sequel")?;

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

    fn find_child<'a>(node: &'a Node, kind: &str) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        node.children(&mut cursor).find(|n| n.kind() == kind)
    }

    fn walk_node(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<String>) {
        match node.kind() {
            "create_table" => Self::extract_create_table(node, source, result),
            "create_function" => Self::extract_create_function(node, source, result),
            "create_procedure" => Self::extract_create_procedure(node, source, result),
            "create_view" => Self::extract_create_view(node, source, result),
            "create_index" => Self::extract_create_index(node, source, result),
            "create_trigger" => Self::extract_create_trigger(node, source, result),
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match child.kind() {
                "create_table" | "create_function" | "create_procedure"
                | "create_view" | "create_index" | "create_trigger" => {
                    Self::get_object_name(&child, source)
                }
                _ => parent.clone(),
            };
            Self::walk_node(&child, source, result, new_parent);
        }
    }

    fn extract_create_table(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::get_object_name(node, source);
        let Some(name) = name else { return };

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "column_definition" {
                if let Some(col_name) = Self::find_child(&child, "identifier")
                    .and_then(|n| n.utf8_text(source).ok())
                {
                    let col_type = child
                        .child(1)
                        .and_then(|n| n.utf8_text(source).ok())
                        .unwrap_or("unknown");

                    result.symbols.push(ParsedSymbol {
                        name: col_name.to_string(),
                        kind: SymbolKind::Field,
                        start_line: child.start_position().row + 1,
                        end_line: child.end_position().row + 1,
                        start_col: child.start_position().column,
                        end_col: child.end_position().column,
                        signature: Some(format!("{} {}", col_name, col_type)),
                        docstring: None,
                        parent: Some(name.clone()),
                    });
                }
            }
        }

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Struct,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("CREATE TABLE {}", name)),
            docstring: None,
            parent: None,
        });
    }

    fn extract_create_function(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::get_object_name(node, source);
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Function,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("CREATE FUNCTION".to_string()),
            docstring: None,
            parent: None,
        });
    }

    fn extract_create_procedure(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::get_object_name(node, source);
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Function,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("CREATE PROCEDURE".to_string()),
            docstring: None,
            parent: None,
        });
    }

    fn extract_create_view(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::get_object_name(node, source);
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::TypeAlias,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("CREATE VIEW".to_string()),
            docstring: None,
            parent: None,
        });
    }

    fn extract_create_index(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::get_object_name(node, source);
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Variable,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("CREATE INDEX".to_string()),
            docstring: None,
            parent: None,
        });
    }

    fn extract_create_trigger(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::get_object_name(node, source);
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Function,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("CREATE TRIGGER".to_string()),
            docstring: None,
            parent: None,
        });
    }

    fn get_object_name(node: &Node, source: &[u8]) -> Option<String> {
        if let Some(name_node) = Self::find_child(node, "name") {
            return name_node.utf8_text(source).ok().map(|s| s.to_string());
        }
        // Fallback: find object_reference > identifier
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "object_reference" {
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "identifier" {
                        return inner.utf8_text(source).ok().map(|s| s.to_string());
                    }
                }
            }
        }
        None
    }
}

impl SourceCodeParser for SqlParser {
    fn language(&self) -> &str {
        "sql"
    }

    fn file_extensions(&self) -> &[&str] {
        &["sql"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        self.do_parse(source)
    }
}
