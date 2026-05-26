use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct DartParser;

impl DartParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_dart::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-dart")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_dart::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-dart")?;

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
            "class_declaration" => {
                Self::extract_class(node, source, result, parent.as_deref());
            }
            "method_declaration" => {
                Self::extract_method(node, source, result, parent.as_deref());
            }
            "method_invocation" => {
                Self::extract_call(node, source, result);
            }
            "import_directive" => {
                Self::extract_import(node, source, result);
            }
            "enum_declaration" => {
                Self::extract_enum(node, source, result, parent.as_deref());
            }
            "mixin_declaration" => {
                Self::extract_mixin(node, source, result, parent.as_deref());
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match kind {
                "class_declaration" | "enum_declaration" | "mixin_declaration" => node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                _ => parent.clone(),
            };
            Self::walk_node(&child, source, result, new_parent);
        }
    }

    fn extract_function(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        // Dart: function name is inside function_signature -> child_by_field_name("name")
        let name = node
            .child_by_field_name("signature")
            .and_then(|sig| sig.child_by_field_name("name"))
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        let params = node
            .child_by_field_name("signature")
            .and_then(|sig| sig.child_by_field_name("parameters"))
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
            signature: Some(format!("{}{}", name, params)),
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

        // BUG-011 fix: prevent class from having itself as parent
        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        result.symbols.push(ParsedSymbol {
            name,
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

    fn extract_mixin(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        // BUG-011 fix: prevent mixin from having itself as parent
        let effective_parent = parent.filter(|p| *p != name).map(|s| s.to_string());

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("mixin {{}}")),
            docstring: None,
            parent: effective_parent,
        });
    }

    fn extract_method(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        // Dart method_declaration:
        //   signature -> method_signature -> function_signature -> name (identifier)
        //   signature -> method_signature -> function_signature -> parameters
        let signature = node.child_by_field_name("signature");
        // method_signature contains function_signature as first child
        let function_sig = signature.and_then(|sig| sig.child(0));
        let name = function_sig
            .and_then(|fsig| fsig.child_by_field_name("name"))
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        let params = function_sig
            .and_then(|fsig| fsig.child_by_field_name("parameters"))
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
            signature: Some(format!("{}{}", name, params)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
        let uri = node
            .child_by_field_name("uri")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.trim_matches('"').to_string());
        if let Some(name) = uri {
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
        // Dart method_invocation: the method name is in the 'method' or 'name' field.
        // tree-sitter-dart uses 'method' for chained calls, but some versions use 'name'.
        // Fallback: use the first named child if field lookup fails.
        let callee = node
            .child_by_field_name("method")
            .or_else(|| node.child_by_field_name("name"))
            .or_else(|| node.named_child(0))
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

impl SourceCodeParser for DartParser {
    fn language(&self) -> &str {
        "dart"
    }

    fn file_extensions(&self) -> &[&str] {
        &["dart"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
