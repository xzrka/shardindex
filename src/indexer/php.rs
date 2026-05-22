use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct PhpParser;

impl PhpParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_php::LANGUAGE_PHP.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-php")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_php::LANGUAGE_PHP.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-php")?;

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
            "class_declaration" | "enum_declaration" => {
                Self::extract_class(node, source, result, parent.as_deref());
            }
            "method_declaration" => {
                Self::extract_method(node, source, result, parent.as_deref());
            }
            "use_group" | "use_clause" => {
                Self::extract_use(node, source, result);
            }
            "property_element" => {
                Self::extract_property(node, source, result, parent.as_deref());
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match kind {
                "class_declaration" | "enum_declaration" => node
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
            signature: Some(format!("function {}{}", name, params)),
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

        let ext = node.child_by_field_name("extends")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());

        let is_enum = node.kind() == "enum_declaration";
        let kind = if is_enum { SymbolKind::Enum } else { SymbolKind::Class };
        let sig = if let Some(base) = &ext {
            format!("class {} extends {}", name, base)
        } else if is_enum {
            format!("enum {{}}")
        } else {
            format!("class {}", name)
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

        if let Some(base) = ext {
            result.references.push(ParsedReference {
                caller_symbol: Some(name),
                callee_symbol: base,
                ref_kind: "inherit".to_string(),
                line: node.start_position().row + 1,
            });
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

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Method,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("function {}{}", name, params)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_use(node: &Node, source: &[u8], result: &mut ParseResult) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "use_declaration" {
                let name = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string());
                if let Some(n) = name {
                    let cleaned = n.replace("\\", "/");
                    result.imports.push((cleaned.clone(), cleaned.clone(), "use".to_string()));
                    result.references.push(ParsedReference {
                        caller_symbol: None,
                        callee_symbol: cleaned,
                        ref_kind: "import".to_string(),
                        line: child.start_position().row + 1,
                    });
                }
            }
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
}

impl SourceCodeParser for PhpParser {
    fn language(&self) -> &str {
        "php"
    }

    fn file_extensions(&self) -> &[&str] {
        &["php"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
