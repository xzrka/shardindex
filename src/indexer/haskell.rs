use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct HaskellParser;

impl HaskellParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_haskell::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-haskell")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_haskell::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-haskell")?;

        let tree = parser
            .parse(source, None)
            .context("tree-sitter parse failed")?;
        let root = tree.root_node();
        let source_bytes = source.as_bytes();
        let mut result = ParseResult {
            symbols: Vec::new(),
            references: Vec::new(),
            imports: Vec::new(),
            string_literals: Vec::new(),
        };

        Self::walk_node(&root, source_bytes, &mut result, None);
        Ok(result)
    }

    fn walk_node(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<String>) {
        let kind = node.kind();

        match kind {
            "function" => {
                Self::extract_function(node, source, result, parent.as_deref());
            }
            "data_type" => {
                Self::extract_type(node, source, result, parent.as_deref());
            }
            "type_synonym" => {
                Self::extract_type_alias(node, source, result, parent.as_deref());
            }
            "import" => {
                Self::extract_import(node, source, result);
            }
            "import_list" => {
                Self::extract_import_list(node, source, result);
            }
            "apply" => {
                Self::extract_call(node, source, result);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match kind {
                "data_type" | "type_synonym" => node
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
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        let arguments = node
            .child_by_field_name("arguments")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let sig = if let Some(args) = arguments {
            format!("{} {}", name, args)
        } else {
            name.clone()
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

    fn extract_type(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            return;
        };

        let constructors = node
            .child_by_field_name("constructors")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let sig = if let Some(ctors) = constructors {
            format!("data {} = {}", name, ctors)
        } else {
            format!("data {}", name)
        };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(sig),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
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

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::TypeAlias,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("type {}", name)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
        let module_name = node
            .child_by_field_name("module")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        if let Some(name) = module_name {
            result
                .imports
                .push((name.clone(), name.clone(), "import".to_string()));
            result.references.push(ParsedReference {
                caller_symbol: None,
                callee_symbol: name,
                ref_kind: "import".to_string(),
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_import_list(node: &Node, source: &[u8], result: &mut ParseResult) {
        let module_name = node
            .child_by_field_name("module")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        if let Some(name) = module_name {
            result
                .imports
                .push((name.clone(), name.clone(), "import".to_string()));
            result.references.push(ParsedReference {
                caller_symbol: None,
                callee_symbol: name,
                ref_kind: "import".to_string(),
                line: node.start_position().row + 1,
            });
        }
    }

    fn extract_call(node: &Node, source: &[u8], result: &mut ParseResult) {
        // Haskell apply: first named child is the function being applied
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

impl SourceCodeParser for HaskellParser {
    fn language(&self) -> &str {
        "haskell"
    }

    fn file_extensions(&self) -> &[&str] {
        &["hs", "lhs"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
