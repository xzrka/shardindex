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

    fn walk_node(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<String>,
    ) {
        match node.kind() {
            "type_definition" => {
                Self::extract_type(node, source, result, SymbolKind::Class);
            }
            "interface_definition" => {
                Self::extract_type(node, source, result, SymbolKind::Interface);
            }
            "input_type_definition" => {
                Self::extract_type(node, source, result, SymbolKind::Struct);
            }
            "enum_type_definition" => {
                Self::extract_enum(node, source, result);
            }
            "union_type_definition" => {
                Self::extract_union(node, source, result);
            }
            "scalar_type_definition" => {
                Self::extract_scalar(node, source, result);
            }
            "directive_definition" => {
                Self::extract_directive(node, source, result);
            }
            "field_definition" => {
                Self::extract_field(node, source, result, parent.as_deref());
            }
            "enum_value_definition" => {
                Self::extract_enum_value(node, source, result, parent.as_deref());
            }
            "argument" => {
                Self::extract_argument_ref(node, source, result);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match child.kind() {
                "type_definition" | "interface_definition" | "input_type_definition" | "enum_type_definition" => {
                    child.child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string())
                }
                _ => parent.clone(),
            };
            Self::walk_node(&child, source, result, new_parent);
        }
    }

    fn extract_type(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        kind: SymbolKind,
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        // Check for implements
        let mut interfaces = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "implements_interfaces" {
                let mut ic = child.walk();
                for iface in child.children(&mut ic) {
                    if let Ok(iface_name) = iface.utf8_text(source) {
                        interfaces.push(iface_name.to_string());
                        result.references.push(ParsedReference {
                            caller_symbol: Some(name.clone()),
                            callee_symbol: iface_name.to_string(),
                            ref_kind: "implements".to_string(),
                            line: node.start_position().row + 1,
                        });
                    }
                }
            }
        }

        let sig_prefix = match kind {
            SymbolKind::Class => "type",
            SymbolKind::Interface => "interface",
            SymbolKind::Struct => "input",
            _ => "type",
        };

        let signature = if interfaces.is_empty() {
            format!("{} {}", sig_prefix, name)
        } else {
            format!("{} {} implements {}", sig_prefix, name, interfaces.join(", "))
        };

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(signature),
            docstring: None,
            parent: None,
        });
    }

    fn extract_enum(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Enum,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("enum {}", name)),
            docstring: None,
            parent: None,
        });
    }

    fn extract_union(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = node
            .child_by_field_name("name")
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
            signature: Some(format!("union")),
            docstring: None,
            parent: None,
        });
    }

    fn extract_scalar(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = node
            .child_by_field_name("name")
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
            signature: Some(format!("scalar")),
            docstring: None,
            parent: None,
        });
    }

    fn extract_directive(node: &Node, source: &[u8], result: &mut ParseResult) {
        let name = node
            .child_by_field_name("name")
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
            signature: Some(format!("directive")),
            docstring: None,
            parent: None,
        });
    }

    fn extract_field(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        let ty = node
            .child_by_field_name("type")
            .and_then(|t| t.utf8_text(source).ok())
            .map(|s| s.to_string());

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Field,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: ty.map(|t| format!("{}: {}", name, t)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_enum_value(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::EnumMember,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: None,
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_argument_ref(node: &Node, source: &[u8], result: &mut ParseResult) {
        if let Ok(text) = node.utf8_text(source) {
            result.references.push(ParsedReference {
                caller_symbol: None,
                callee_symbol: text.to_string(),
                ref_kind: "argument".to_string(),
                line: node.start_position().row + 1,
            });
        }
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
