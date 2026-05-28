use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct GraphqlParser;

impl GraphqlParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_graphql::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-graphql")?;

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
            "object_type_definition"
            | "interface_type_definition"
            | "input_object_type_definition"
            | "enum_type_definition"
            | "union_type_definition" => {
                Self::extract_type(node, source, result);
            }
            "scalar_type_definition" => {
                Self::extract_scalar(node, source, result);
            }
            "directive_definition" => {
                Self::extract_directive(node, source, result);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match child.kind() {
                "object_type_definition"
                | "interface_type_definition"
                | "input_object_type_definition"
                | "enum_type_definition"
                | "union_type_definition" => Self::find_child(&child, "name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                _ => parent.clone(),
            };
            Self::walk_node(&child, source, result, new_parent);
        }
    }

    fn extract_type(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::find_child(node, "name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        let name_for_parent = name.clone();
        let kind = node.kind();
        let (symbol_kind, sig_prefix) = match kind {
            "object_type_definition" => (SymbolKind::Class, "type"),
            "interface_type_definition" => (SymbolKind::Interface, "interface"),
            "input_object_type_definition" => (SymbolKind::Struct, "input"),
            "enum_type_definition" => (SymbolKind::Enum, "enum"),
            "union_type_definition" => (SymbolKind::TypeAlias, "union"),
            _ => (SymbolKind::Class, "type"),
        };

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "field_definition" {
                Self::extract_field(&child, source, result, Some(&name_for_parent));
            }
            if child.kind() == "enum_value" {
                if let Some(val_name) = Self::find_child(&child, "name")
                    .and_then(|n| n.utf8_text(source).ok())
                {
                    result.symbols.push(ParsedSymbol {
                        name: val_name.to_string(),
                        kind: SymbolKind::EnumMember,
                        start_line: child.start_position().row + 1,
                        end_line: child.end_position().row + 1,
                        start_col: child.start_position().column,
                        end_col: child.end_position().column,
                        signature: Some(val_name.to_string()),
                        docstring: None,
                        parent: Some(name.clone()),
                    });
                }
            }
        }

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: symbol_kind,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("{} {}", sig_prefix, name)),
            docstring: None,
            parent: None,
        });
    }

    fn extract_scalar(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::find_child(node, "name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::TypeAlias,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("scalar".to_string()),
            docstring: None,
            parent: None,
        });
    }

    fn extract_directive(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = Self::find_child(node, "name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        let name_for_sig = name.clone();
        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Decorator,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("@{}", name_for_sig)),
            docstring: None,
            parent: None,
        });
    }

    fn extract_field(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = Self::find_child(node, "name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        let type_text = Self::find_child(node, "type")
            .and_then(|n| n.utf8_text(source).ok())
            .unwrap_or("");

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Field,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!(": {}", type_text)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }
}

impl SourceCodeParser for GraphqlParser {
    fn language(&self) -> &str {
        "graphql"
    }

    fn file_extensions(&self) -> &[&str] {
        &["graphql", "gql"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        self.do_parse(source)
    }
}
