use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct JuliaParser;

impl JuliaParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_julia::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-julia")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_julia::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-julia")?;

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
            "function_definition" => {
                Self::extract_function(node, source, result, parent.as_deref());
            }
            "abstract_type_definition" | "struct_definition" => {
                Self::extract_type(node, source, result, parent.as_deref());
            }
            "macro_definition" => {
                Self::extract_macro(node, source, result, parent.as_deref());
            }
            "module_definition" => {
                Self::extract_module(node, source, result, parent.as_deref());
            }
            "import" | "using" => {
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
                "abstract_type_definition" | "struct_definition" => node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                "module_definition" => node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                _ => parent.clone(),
            };
            Self::walk_node(&child, source, result, new_parent);
        }
    }

    fn extract_function(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        // Julia: function name is positional: named_child(0) = signature -> [typed_]expression -> call_expression -> identifier
        let signature = node.named_child(0);
        let name = signature
            .and_then(|sig| {
                let first = sig.named_child(0)?; // typed_expression or call_expression
                // If typed_expression, drill down to call_expression
                let ce = if first.kind() == "call_expression" {
                    first
                } else {
                    first.named_child(0)? // typed_expression -> call_expression
                };
                ce.named_child(0) // call_expression -> identifier
            })
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        let args = signature
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
            signature: Some(format!("function {}{}", name, args)),
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

        let is_abstract = node.kind() == "abstract_type_definition";
        let supertype = node
            .child_by_field_name("supertype")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let sig = if let Some(base) = &supertype {
            if is_abstract {
                format!("abstract type {} <: {}", name, base)
            } else {
                format!("struct {} <: {}", name, base)
            }
        } else if is_abstract {
            format!("abstract type {}", name)
        } else {
            format!("struct {}", name)
        };

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(sig),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });

        if let Some(base) = supertype {
            result.references.push(ParsedReference {
                caller_symbol: Some(name),
                callee_symbol: base,
                ref_kind: "inherit".to_string(),
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_macro(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

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

    fn extract_module(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Module,
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
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "dotted_name" {
                if let Ok(name) = child.utf8_text(source) {
                    let cleaned = name.to_string();
                    result
                        .imports
                        .push((cleaned.clone(), cleaned.clone(), "import".to_string()));
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

    fn extract_call(node: &Node, source: &[u8], result: &mut ParseResult) {
        // Julia call_expression: first named child is the function name (identifier or dotted_name)
        let func = node.named_child(0);
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
}

impl SourceCodeParser for JuliaParser {
    fn language(&self) -> &str {
        "julia"
    }

    fn file_extensions(&self) -> &[&str] {
        &["jl"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
