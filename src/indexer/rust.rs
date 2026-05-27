use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct RustParser;

impl RustParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-rust")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-rust")?;

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

        Self::walk_node(&root, source_bytes, &mut result, None, None);
        Ok(result)
    }

    fn walk_node(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<String>,
        current_function: Option<String>,
    ) {
        let kind = node.kind();

        match kind {
            "function_item" => {
                Self::extract_function(node, source, result, parent.as_deref());
            }
            "struct_item" => {
                Self::extract_struct(node, source, result, parent.as_deref());
            }
            "enum_item" => {
                Self::extract_enum(node, source, result, parent.as_deref());
            }
            "trait_item" => {
                Self::extract_trait(node, source, result, parent.as_deref());
            }
            "impl_item" => {
                Self::extract_impl(node, source, result, parent.as_deref());
            }
            "use_declaration" | "use" => {
                Self::extract_import(node, source, result);
            }
            "type_alias" => {
                Self::extract_type_alias(node, source, result, parent.as_deref());
            }
            _ => {}
        }

        // Extract call references
        Self::extract_calls(node, source, result, current_function.as_deref());

        // Recurse into children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match child.kind() {
                "impl_item" => child
                    .child_by_field_name("type")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                "trait_item" => child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                _ => parent.clone(),
            };
            let new_function = if child.kind() == "function_item" {
                child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string())
            } else {
                current_function.clone()
            };
            Self::walk_node(&child, source, result, new_parent, new_function);
        }
    }

    fn extract_function(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let Some(name) = name else {
            return;
        };

        // Build signature
        let params = node
            .child_by_field_name("parameters")
            .and_then(|p| p.utf8_text(source).ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "()".to_string());

        let return_type = node
            .child_by_field_name("return_type")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let sig = if let Some(rt) = return_type {
            format!("fn {}{} -> {}", name, params, rt)
        } else {
            format!("fn {}{}", name, params)
        };

        result.symbols.push(ParsedSymbol {
            name,
            kind: if parent.is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            },
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(sig),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_struct(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let Some(name) = name else {
            return;
        };

        // BUG-011 fix: prevent struct from having itself as parent
        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        let sig = format!("struct {}", name);
        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Class, // Reuse Class for struct
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(sig),
            docstring: None,
            parent: effective_parent,
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

        // Extract variants as sub-symbols
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "enum_variant_list" {
                let mut variant_cursor = child.walk();
                for variant in child.children(&mut variant_cursor) {
                    if variant.kind() == "enum_variant" {
                        if let Some(variant_name) = variant
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(source).ok())
                        {
                            result.symbols.push(ParsedSymbol {
                                name: variant_name.to_string(),
                                kind: SymbolKind::Variable,
                                start_line: variant.start_position().row + 1,
                                end_line: variant.end_position().row + 1,
                                start_col: variant.start_position().column,
                                end_col: variant.end_position().column,
                                signature: Some(format!("enum variant {}", variant_name)),
                                docstring: None,
                                parent: Some(name.clone()),
                            });
                        }
                    }
                }
            }
        }

        // BUG-011 fix: prevent enum from having itself as parent
        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        let sig = format!("enum {}", name);
        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Enum,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(sig),
            docstring: None,
            parent: effective_parent,
        });
    }

    fn extract_trait(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let Some(name) = name else {
            return;
        };

        // BUG-011 fix: prevent trait from having itself as parent
        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        let sig = format!("trait {}", name);
        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Class, // Reuse Class for trait
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(sig),
            docstring: None,
            parent: effective_parent,
        });
    }

    fn extract_impl(node: &Node, source: &[u8], result: &mut ParseResult, _parent: Option<&str>) {
        // Get impl target type
        let target = node
            .child_by_field_name("type")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let Some(target) = target else {
            return;
        };

        // If impl for a trait, record inherit reference
        if let Some(trait_node) = node.child_by_field_name("trait") {
            if let Ok(trait_name) = trait_node.utf8_text(source) {
                result.references.push(ParsedReference {
                    caller_symbol: Some(target.clone()),
                    callee_symbol: trait_name.to_string(),
                    ref_kind: "implement".to_string(),
                    line: node.start_position().row + 1,
                });
            }
        }
    }

    fn extract_type_alias(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let Some(name) = name else {
            return;
        };

        let alias_type = node
            .child_by_field_name("type")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        // BUG-011 fix: prevent type_alias from having itself as parent
        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Variable,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: alias_type.map(|t| format!("type {}", t)),
            docstring: None,
            parent: effective_parent,
        });
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            // Use declaration has either scoped_identifier or scoped_use_list
            let text = if child.kind() == "scoped_identifier" {
                child.utf8_text(source).ok()
            } else if child.kind() == "scoped_use_list" {
                // e.g. std::io::{Read, Write} — extract the base path
                child.utf8_text(source).ok()
            } else {
                None
            };

            if let Some(raw) = text {
                let cleaned = raw.trim_start_matches("pub ").trim();
                if !cleaned.is_empty() {
                    if cleaned.contains('{') {
                        if let Some(braces) = cleaned.split_once('{') {
                            let base = braces.0.trim();
                            let items: Vec<&str> = braces
                                .1
                                .trim_end_matches('}')
                                .split(',')
                                .map(|s| s.trim())
                                .filter(|s| !s.is_empty())
                                .collect();
                            let full = format!("{}::{}", base, items.join("::"));
                            result
                                .imports
                                .push((full.clone(), full.clone(), "use".to_string()));
                            result.references.push(ParsedReference {
                                caller_symbol: None,
                                callee_symbol: full,
                                ref_kind: "import".to_string(),
                                line: node.start_position().row + 1,
                            });
                        }
                    } else {
                        result.imports.push((
                            cleaned.to_string(),
                            cleaned.to_string(),
                            "use".to_string(),
                        ));
                        result.references.push(ParsedReference {
                            caller_symbol: None,
                            callee_symbol: cleaned.to_string(),
                            ref_kind: "import".to_string(),
                            line: node.start_position().row + 1,
                        });
                    }
                }
            }
        }
    }

    fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult, caller: Option<&str>) {
        if node.kind() == "call_expression" {
            if let Some(func) = node.child_by_field_name("function") {
                let callee = func.utf8_text(source).unwrap_or("").to_string();
                if !callee.is_empty() {
                    result.references.push(ParsedReference {
                        caller_symbol: caller.map(|s| s.to_string()),
                        callee_symbol: callee,
                        ref_kind: "call".to_string(),
                        line: node.start_position().row + 1,
                    });
                }
            }
        }

        // Recurse for nested calls
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "call_expression" {
                Self::extract_calls(&child, source, result, caller);
            }
        }
    }
}

impl SourceCodeParser for RustParser {
    fn language(&self) -> &str {
        "rust"
    }

    fn file_extensions(&self) -> &[&str] {
        &["rs"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
