use crate::indexer::types::*;
use tree_sitter::Node;
use anyhow::Context;

pub struct RubyParser;

impl RubyParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_ruby::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-ruby")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_ruby::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-ruby")?;

        let tree = parser.parse(source, None).context("tree-sitter parse failed")?;
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
        let kind = node.kind();

        match kind {
            "method" => {
                Self::extract_method(node, source, result, parent.as_deref());
            }
            "class" | "singleton_class" => {
                Self::extract_class(node, source, result, parent.as_deref());
            }
            "module" => {
                Self::extract_module(node, source, result, parent.as_deref());
            }
            "require" | "require_relative" => {
                Self::extract_require(node, source, result);
            }
            "call" => {
                Self::extract_call(node, source, result);
            }
            _ => {}
        }

        // Recurse into children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match kind {
                "class" | "singleton_class" => {
                    node.child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string())
                }
                "module" => {
                    node.child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string())
                }
                _ => parent.clone(),
            };
            Self::walk_node(&child, source, result, new_parent);
        }
    }

    fn extract_method(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name_node = node.child_by_field_name("name");
        let Some(name) = name_node.and_then(|n| n.utf8_text(source).ok()) else {
            return;
        };
        let name = name.to_string();

        let params = node
            .child_by_field_name("parameters")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let sig = if params.is_empty() {
            format!("def {}", name)
        } else {
            format!("def {}({})", name, params)
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

    fn extract_class(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name_node = node.child_by_field_name("name");
        let Some(name) = name_node.and_then(|n| n.utf8_text(source).ok()) else {
            return;
        };
        let name = name.to_string();

        let superclass = node
            .child_by_field_name("superclass")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let sig = if let Some(base) = &superclass {
            format!("class {} < {}", name, base)
        } else {
            format!("class {}", name)
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

        if let Some(base) = superclass {
            result.references.push(ParsedReference {
                caller_symbol: Some(name),
                callee_symbol: base,
                ref_kind: "inherit".to_string(),
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_module(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name_node = node.child_by_field_name("name");
        let Some(name) = name_node.and_then(|n| n.utf8_text(source).ok()) else {
            return;
        };
        let name = name.to_string();

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Module,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("module {}", name)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_require(node: &Node, source: &[u8], result: &mut ParseResult) {
        let arg = node.child_by_field_name("argument");
        if let Some(path) = arg.and_then(|n| n.utf8_text(source).ok()) {
            let cleaned = path.trim_matches('"').to_string();
            if !cleaned.is_empty() {
                result.imports.push((
                    cleaned.clone(),
                    cleaned.clone(),
                    "require".to_string(),
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

    fn extract_call(node: &Node, source: &[u8], result: &mut ParseResult) {
        let method_name = node.child_by_field_name("method_name");
        if let Some(name) = method_name.and_then(|n| n.utf8_text(source).ok()) {
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

impl SourceCodeParser for RubyParser {
    fn language(&self) -> &str {
        "ruby"
    }

    fn file_extensions(&self) -> &[&str] {
        &["rb"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
