use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct ElixirParser;

impl ElixirParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_elixir::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-elixir")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_elixir::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-elixir")?;

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
            "call" => {
                if let Some(text) = node.child_by_field_name("function")
                    .and_then(|n| n.utf8_text(source).ok())
                {
                    match text {
                        "def" | "defp" => {
                            Self::extract_function(node, source, result, parent.as_deref(), text);
                        }
                        "defmacro" | "defmacrop" => {
                            Self::extract_macro(node, source, result, parent.as_deref());
                        }
                        "defimpl" => {
                            Self::extract_impl(node, source, result, parent.as_deref());
                        }
                        "defmodule" | "defmodulep" => {
                            Self::extract_module(node, source, result, parent.as_deref());
                        }
                        "defprotocol" => {
                            Self::extract_protocol(node, source, result, parent.as_deref());
                        }
                        "defdelegate" | "defoverridable" => {}
                        "import" | "use" | "alias" | "require" => {
                            Self::extract_import(node, source, result, text);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match kind {
                "call" => {
                    if let Some(text) = node.child_by_field_name("function")
                        .and_then(|n| n.utf8_text(source).ok())
                    {
                        match text {
                            "defmodule" | "defmodulep" | "defimpl" | "defprotocol" => {
                                node.child_by_field_name("arguments")
                                    .and_then(|args| args.named_child(0))
                                    .and_then(|n| n.utf8_text(source).ok())
                                    .map(|s| s.to_string())
                            }
                            _ => parent.clone(),
                        }
                    } else {
                        parent.clone()
                    }
                }
                _ => parent.clone(),
            };
            Self::walk_node(&child, source, result, new_parent);
        }
    }

    fn extract_function(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>, keyword: &str) {
        let args = node.child_by_field_name("arguments");
        let first_arg = args.and_then(|a| a.named_child(0));
        let Some(name) = first_arg.and_then(|n| n.utf8_text(source).ok()) else {
            return;
        };
        let name = name.to_string();

        let params = args
            .and_then(|a| a.utf8_text(source).ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Function,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("{} {}", keyword, params)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_macro(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let args = node.child_by_field_name("arguments");
        let first_arg = args.and_then(|a| a.named_child(0));
        let Some(name) = first_arg.and_then(|n| n.utf8_text(source).ok()) else {
            return;
        };
        let name = name.to_string();

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Decorator,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: None,
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_impl(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let args = node.child_by_field_name("arguments");
        let first_arg = args.and_then(|a| a.named_child(0));
        let Some(name) = first_arg.and_then(|n| n.utf8_text(source).ok()) else {
            return;
        };
        let name = name.to_string();

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("defimpl {{}}")),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_module(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let args = node.child_by_field_name("arguments");
        let first_arg = args.and_then(|a| a.named_child(0));
        let Some(name) = first_arg.and_then(|n| n.utf8_text(source).ok()) else {
            return;
        };
        let name = name.to_string();

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Module,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("defmodule".to_string()),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_protocol(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let args = node.child_by_field_name("arguments");
        let first_arg = args.and_then(|a| a.named_child(0));
        let Some(name) = first_arg.and_then(|n| n.utf8_text(source).ok()) else {
            return;
        };
        let name = name.to_string();

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("defprotocol".to_string()),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult, keyword: &str) {
        let args = node.child_by_field_name("arguments");
        let first_arg = args.and_then(|a| a.named_child(0));
        if let Some(name) = first_arg.and_then(|n| n.utf8_text(source).ok()) {
            let cleaned = name.to_string();
            result.imports.push((
                cleaned.clone(),
                cleaned.clone(),
                keyword.to_string(),
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

impl SourceCodeParser for ElixirParser {
    fn language(&self) -> &str {
        "elixir"
    }

    fn file_extensions(&self) -> &[&str] {
        &["ex", "exs"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
