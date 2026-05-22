use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct ScalaParser;

impl ScalaParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-scala")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-scala")?;

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
            "class_definition" => {
                Self::extract_class(node, source, result, parent.as_deref());
            }
            "object_definition" => {
                Self::extract_object(node, source, result, parent.as_deref());
            }
            "trait_definition" => {
                Self::extract_trait(node, source, result, parent.as_deref());
            }
            "import_clause" => {
                Self::extract_import(node, source, result);
            }
            "val_definition" => {
                Self::extract_val(node, source, result, parent.as_deref());
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match kind {
                "class_definition" | "object_definition" | "trait_definition" => node
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
            .child_by_field_name("parameters")
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
            signature: Some(format!("def {}{}", name, params)),
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

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("class {{}}")),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_object(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
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
            signature: Some(format!("object {{}}")),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
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

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("trait {{}}")),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
        let importee = node
            .child_by_field_name("importee")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        if let Some(name) = importee {
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

    fn extract_val(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
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
}

impl SourceCodeParser for ScalaParser {
    fn language(&self) -> &str {
        "scala"
    }

    fn file_extensions(&self) -> &[&str] {
        &["scala"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
