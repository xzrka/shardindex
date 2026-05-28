use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct CSharpParser;

impl CSharpParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-c-sharp")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-c-sharp")?;

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
            "class_declaration" => {
                Self::extract_class(node, source, result, parent.as_deref());
            }
            "struct_declaration" => {
                Self::extract_struct(node, source, result, parent.as_deref());
            }
            "interface_declaration" => {
                Self::extract_interface(node, source, result, parent.as_deref());
            }
            "enum_declaration" => {
                Self::extract_enum(node, source, result, parent.as_deref());
            }
            "method_declaration" => {
                Self::extract_method(node, source, result, parent.as_deref());
            }
            "constructor_declaration" => {
                Self::extract_constructor(node, source, result, parent.as_deref());
            }
            "property_declaration" => {
                Self::extract_property(node, source, result, parent.as_deref());
            }
            "field_declaration" => {
                Self::extract_field(node, source, result, parent.as_deref());
            }
            "delegate_declaration" => {
                Self::extract_delegate(node, source, result, parent.as_deref());
            }
            "namespace_declaration" | "file_scoped_namespace_declaration" => {
                Self::extract_namespace(node, source, result, parent.as_deref());
            }
            "using_directive" => {
                Self::extract_using(node, source, result);
            }
            _ => {}
        }

        // Extract call references
        Self::extract_calls(node, source, result, current_function.as_deref());

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match child.kind() {
                "class_declaration" | "struct_declaration" | "interface_declaration" | "enum_declaration" => child
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

    fn extract_class(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        // BUG-011 fix: prevent class from having itself as parent
        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("class {{}}")),
            docstring: None,
            parent: effective_parent,
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

        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Struct,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("struct {{}}")),
            docstring: None,
            parent: effective_parent,
        });
    }

    fn extract_interface(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Interface,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("interface {{}}")),
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
            .child_by_field_name("returns")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let sig = if let Some(rt) = return_type {
            format!("{} {}{}", rt, name, params)
        } else {
            format!("{}{}", name, params)
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

    fn extract_constructor(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
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
            name,
            kind: SymbolKind::Method,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("constructor({})", params)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_property(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        let prop_type = node
            .child_by_field_name("type")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let sig = if let Some(t) = prop_type {
            format!("{} {}", t, name)
        } else {
            name.clone()
        };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Field,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(sig),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_field(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        // field_declaration has no named fields, so we iterate children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "declaration" {
                // Find the declarator
                let mut decl_cursor = child.walk();
                for decl_child in child.children(&mut decl_cursor) {
                    if decl_child.kind() == "variable_declarator" {
                        let name = decl_child
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(source).ok())
                            .map(|s| s.to_string());
                        if let Some(name) = name {
                            result.symbols.push(ParsedSymbol {
                                name,
                                kind: SymbolKind::Variable,
                                start_line: decl_child.start_position().row + 1,
                                end_line: decl_child.end_position().row + 1,
                                start_col: decl_child.start_position().column,
                                end_col: decl_child.end_position().column,
                                signature: None,
                                docstring: None,
                                parent: parent.map(|s| s.to_string()),
                            });
                        }
                    }
                }
            }
        }
    }

    fn extract_delegate(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        let ret_type = node
            .child_by_field_name("type")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let params = node
            .child_by_field_name("parameters")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "()".to_string());

        let sig = if let Some(rt) = ret_type {
            format!("delegate {}{}{}", rt, name, params)
        } else {
            format!("delegate {}{}", name, params)
        };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Function,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(sig),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_namespace(node: &Node, source: &[u8], result: &mut ParseResult, _parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        if let Some(name) = name {
            result.symbols.push(ParsedSymbol {
                name: name.clone(),
                kind: SymbolKind::Module,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: Some(format!("namespace {}", name)),
                docstring: None,
                parent: None,
            });
        }
    }

    fn extract_using(node: &Node, source: &[u8], result: &mut ParseResult) {
        let imported_name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok());
        if let Some(name) = imported_name {
            let cleaned = name.trim_end_matches('.').to_string();
            result.imports.push((cleaned.clone(), cleaned.clone(), "using".to_string()));
            result.references.push(ParsedReference {
                caller_symbol: None,
                callee_symbol: cleaned,
                ref_kind: "using".to_string(),
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult, caller: Option<&str>) {
        if node.kind() == "invocation_expression" {
            let method_name = node.child_by_field_name("name");
            if let Some(callee) = method_name.and_then(|n| n.utf8_text(source).ok()) {
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

impl SourceCodeParser for CSharpParser {
    fn language(&self) -> &str {
        "csharp"
    }

    fn file_extensions(&self) -> &[&str] {
        &["cs"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
