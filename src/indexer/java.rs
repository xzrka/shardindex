use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct JavaParser;

impl JavaParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-java")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-java")?;

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
            "method_declaration" => {
                Self::extract_method(node, source, result, parent.as_deref());
            }
            "class_declaration" | "interface_declaration" => {
                Self::extract_class(node, source, result, parent.as_deref());
            }
            "enum_declaration" => {
                Self::extract_enum(node, source, result, parent.as_deref());
            }
            "import_declaration" => {
                Self::extract_import(node, source, result);
            }
            "field_declaration" => {
                Self::extract_field(node, source, result, parent.as_deref());
            }
            "constructor_declaration" => {
                Self::extract_constructor(node, source, result, parent.as_deref());
            }
            _ => {}
        }

        // Extract call references
        Self::extract_calls(node, source, result, current_function.as_deref());

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match kind {
                "class_declaration" | "interface_declaration" | "enum_declaration" => node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                _ => parent.clone(),
            };
            let new_function = match child.kind() {
                "method_declaration" | "constructor_declaration" => child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                _ => current_function.clone(),
            };
            Self::walk_node(&child, source, result, new_parent, new_function);
        }
    }

    fn extract_method(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
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

        let return_type = node
            .child_by_field_name("type")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let sig = if let Some(rt) = return_type {
            format!("{} {}{}", rt, name, params)
        } else {
            format!("{}({})", name, params)
        };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Method,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(sig),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_class(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        let superclass = node
            .child_by_field_name("superclass")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let interfaces = node
            .child_by_field_name("interfaces")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let is_interface = node.kind() == "interface_declaration";
        let sig = if is_interface {
            if let Some(ifaces) = &interfaces {
                format!("interface {} extends {}", name, ifaces)
            } else {
                format!("interface {}", name)
            }
        } else {
            if let Some(base) = &superclass {
                format!("class {} extends {}", name, base)
            } else {
                format!("class {}", name)
            }
        };

        let kind = if is_interface {
            SymbolKind::Class
        } else {
            SymbolKind::Class
        };

        // BUG-011 fix: prevent class from having itself as parent
        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(sig),
            docstring: None,
            parent: effective_parent,
        });

        if let Some(base) = superclass {
            result.references.push(ParsedReference {
                caller_symbol: Some(name),
                callee_symbol: base,
                ref_kind: "inherit".to_string(),
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_enum(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        // BUG-011 fix: prevent enum from having itself as parent
        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Enum,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("enum {{}}")),
            docstring: None,
            parent: effective_parent,
        });
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
        // Java: first named child is scoped_identifier (or identifier)
        let imported_name = node.named_child(0).and_then(|n| n.utf8_text(source).ok());
        if let Some(name) = imported_name {
            let cleaned = name.trim_end_matches('.').to_string();
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

    fn extract_field(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                let name = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string());
                if let Some(name) = name {
                    result.symbols.push(ParsedSymbol {
                        name,
                        kind: SymbolKind::Variable,
                        start_line: child.start_position().row + 1,
                        end_line: child.end_position().row + 1,
                        start_col: child.start_position().column,
                        end_col: child.end_position().column,
                        signature: None,
                        docstring: None,
                        parent: parent.map(|s| s.to_string()),
                    });
                }
            }
        }
    }

    fn extract_constructor(
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

        let params = node
            .child_by_field_name("parameters")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "()".to_string());

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Method,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("{}({})", name, params)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult, caller: Option<&str>) {
        if node.kind() == "method_invocation" {
            let method_selector = node.child_by_field_name("selector");
            if let Some(callee) = method_selector.and_then(|n| n.utf8_text(source).ok()) {
                let callee = callee.to_string();
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

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::extract_calls(&child, source, result, caller);
        }
    }
}

impl SourceCodeParser for JavaParser {
    fn language(&self) -> &str {
        "java"
    }

    fn file_extensions(&self) -> &[&str] {
        &["java"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
