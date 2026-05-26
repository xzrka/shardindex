use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct SwiftParser;

impl SwiftParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_swift::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-swift")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_swift::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-swift")?;

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
            "class_declaration" | "struct_declaration" | "enum_declaration" | "protocol_declaration" => {
                Self::extract_type(node, source, result, parent.as_deref());
            }
            "property_declaration" => {
                Self::extract_property(node, source, result, parent.as_deref());
            }
            "import" => {
                Self::extract_import(node, source, result);
            }
            "call_suffix" => {
                Self::extract_call(node, source, result);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match kind {
                "class_declaration" | "struct_declaration" | "enum_declaration" => node
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
            .child_by_field_name("signature")
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
            signature: Some(format!("func {}{}", name, params)),
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

        let kind = match node.kind() {
            "class_declaration" => SymbolKind::Class,
            "struct_declaration" => SymbolKind::Class,
            "enum_declaration" => SymbolKind::Enum,
            "protocol_declaration" => SymbolKind::Class,
            _ => SymbolKind::Class,
        };

        let inherits_from = node
            .child_by_field_name("inherits_from")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let sig = if let Some(base) = &inherits_from {
            format!("{} {} : {}", node.kind().replace("_declaration", ""), name, base)
        } else {
            format!("{} {}", node.kind().replace("_declaration", ""), name)
        };

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(sig),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });

        if let Some(base) = inherits_from {
            result.references.push(ParsedReference {
                caller_symbol: Some(name),
                callee_symbol: base,
                ref_kind: "inherit".to_string(),
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_property(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        if let Some(name) = name {
            result.symbols.push(ParsedSymbol {
                name,
                kind: SymbolKind::Variable,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_col: node.start_position().column,
                end_col: node.end_position().column,
                signature: None,
                docstring: None,
                parent: parent.map(|s| s.to_string()),
            });
        }
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
        let module_name = node
            .child_by_field_name("path")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        if let Some(name) = module_name {
            result.imports.push((
                name.clone(),
                name.clone(),
                "import".to_string(),
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
        // Swift tree-sitter 0.7.2: call_suffix has no 'function' field.
        // The callee is the first named child (simple_identifier or member_access).
        let callee_node = node.named_child(0);
        let callee = if let Some(cn) = callee_node {
            match cn.kind() {
                "simple_identifier" => cn.utf8_text(source).ok().map(|s| s.to_string()),
                "member_access" => {
                    // member_access: base . identifier — get the identifier part
                    cn.child_by_field_name("detail")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string())
                }
                _ => cn.utf8_text(source).ok().map(|s| s.to_string()),
            }
        } else {
            None
        };

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

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "call_suffix" {
                Self::extract_call(&child, source, result);
            }
        }
    }
}

impl SourceCodeParser for SwiftParser {
    fn language(&self) -> &str {
        "swift"
    }

    fn file_extensions(&self) -> &[&str] {
        &["swift"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
