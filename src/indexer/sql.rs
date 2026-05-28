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
            .context("Failed to load tree-sitter-sql")?;

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
            "create_table" => {
                Self::extract_create_table(node, source, result);
            }
            "create_view" => {
                Self::extract_create_view(node, source, result);
            }
            "create_function" => {
                Self::extract_create_function(node, source, result);
            }
            "create_procedure" => {
                Self::extract_create_procedure(node, source, result);
            }
            "create_index" => {
                Self::extract_create_index(node, source, result);
            }
            "table_ref" => {
                Self::extract_table_ref(node, source, result);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_node(&child, source, result, parent.clone());
        }
    }

    fn extract_name(node: &Node, source: &[u8]) -> Option<String> {
        node.child_by_field_name("name")
            .or_else(|| {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "table_name" || child.kind() == "qualified_name" {
                        return Some(child);
                    }
                }
                None
            })
            .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()))
    }

    fn extract_create_table(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::extract_name(node, source);
        let Some(name) = name else { return };

        let signature = Some(format!("CREATE TABLE {}", name));
        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature,
            docstring: None,
            parent: None,
        });

        // Extract columns as fields
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "column_definition" {
                if let Some(col_node) = child.child_by_field_name("name") {
                    if let Ok(col_name) = col_node.utf8_text(source) {
                        result.symbols.push(ParsedSymbol {
                            name: col_name.to_string(),
                            kind: SymbolKind::Field,
                            start_line: child.start_position().row + 1,
                            end_line: child.end_position().row + 1,
                            start_col: child.start_position().column,
                            end_col: child.end_position().column,
                            signature: Some(format!("column {}", col_name)),
                            docstring: None,
                            parent: Some(name.clone()),
                        });
                    }
                }
            }
        }
    }

    fn extract_create_view(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::extract_name(node, source);
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("CREATE VIEW {}", name)),
            docstring: None,
            parent: None,
        });
    }

    fn extract_create_function(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::extract_name(node, source);
        let Some(name) = name else { return };

        let params = node
            .child_by_field_name("parameters")
            .and_then(|p| p.utf8_text(source).ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Function,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("CREATE FUNCTION {}({})", name, params)),
            docstring: None,
            parent: None,
        });
    }

    fn extract_create_procedure(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::extract_name(node, source);
        let Some(name) = name else { return };

        let params = node
            .child_by_field_name("parameters")
            .and_then(|p| p.utf8_text(source).ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Function,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("CREATE PROCEDURE {}({})", name, params)),
            docstring: None,
            parent: None,
        });
    }

    fn extract_create_index(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::extract_name(node, source);
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::TypeAlias,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("CREATE INDEX {}", name)),
            docstring: None,
            parent: None,
        });
    }

    fn extract_table_ref(node: &Node, source: &[u8], result: &mut ParseResult) {
        if let Ok(name) = node.utf8_text(source) {
            result.references.push(ParsedReference {
                caller_symbol: None,
                callee_symbol: name.to_string(),
                ref_kind: "table_ref".to_string(),
                line: node.start_position().row + 1,
            });
        }
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
