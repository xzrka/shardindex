use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct ZigParser;

impl ZigParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_zig::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-zig")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_zig::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-zig")?;

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
            "struct_declaration" | "opaque_declaration" => {
                Self::extract_type(node, source, result, parent.as_deref());
            }
            "union_declaration" | "enum_declaration" => {
                Self::extract_enum(node, source, result, parent.as_deref());
            }
            "using_namespace_declaration" => {
                Self::extract_import(node, source, result);
            }
            "call_expression" => {
                Self::extract_call(node, source, result);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match kind {
                "struct_declaration" | "opaque_declaration" | "union_declaration" | "enum_declaration" => node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                _ => parent.clone(),
            };
            Self::walk_node(&child, source, result, new_parent);
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
            signature: Some(format!("fn {}{}", name, params)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_type(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
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
            signature: Some(format!("{} {{}}", node.kind().replace("_declaration", ""))),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_enum(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        let kind = if node.kind() == "enum_declaration" {
            SymbolKind::Enum
        } else {
            SymbolKind::TypeAlias
        };

        result.symbols.push(ParsedSymbol {
            name,
            kind,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: None,
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = node
            .child_by_field_name("identifier")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        if let Some(name) = name {
            result.imports.push((
                name.clone(),
                name.clone(),
                "usingnamespace".to_string(),
            ));
            result.references.push(ParsedReference {
                caller_symbol: None,
                callee_symbol: name,
                ref_kind: "import".to_string(),
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_call(node: &Node, source: &[u8], result: &mut ParseResult) {
        let callee = node
            .child_by_field_name("function")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        if let Some(callee) = callee {
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
}

impl SourceCodeParser for ZigParser {
    fn language(&self) -> &str {
        "zig"
    }

    fn file_extensions(&self) -> &[&str] {
        &["zig"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
