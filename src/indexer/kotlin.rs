use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct KotlinParser;

impl KotlinParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_kotlin_ng::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-kotlin")?;

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

    fn walk_node(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<String>,
    ) {
        match node.kind() {
            "class_declaration" => {
                Self::extract_class(node, source, result, parent.as_deref());
            }
            "interface_declaration" => {
                Self::extract_interface(node, source, result, parent.as_deref());
            }
            "fun_declaration" => {
                Self::extract_function(node, source, result, parent.as_deref());
            }
            "property_declaration" => {
                Self::extract_property(node, source, result, parent.as_deref());
            }
            "enum_class_declaration" => {
                Self::extract_enum(node, source, result, parent.as_deref());
            }
            "data_class_declaration" => {
                Self::extract_data_class(node, source, result, parent.as_deref());
            }
            "companion_object" => {
                Self::extract_companion_object(node, source, result, parent.as_deref());
            }
            "import_declaration" => {
                Self::extract_import(node, source, result);
            }
            "object_literal" => {
                Self::extract_object_literal(node, source, result, parent.as_deref());
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let new_parent = match child.kind() {
                "class_declaration" | "interface_declaration" | "enum_class_declaration"
                | "data_class_declaration" => child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.to_string()),
                _ => parent.clone(),
            };
            Self::walk_node(&child, source, result, new_parent);
        }
    }

    fn extract_class(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        // Extract superclass
        let mut bases = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "super_type_list" {
                let mut sc = child.walk();
                for st in child.children(&mut sc) {
                    if let Ok(base) = st.utf8_text(source) {
                        bases.push(base.to_string());
                        result.references.push(ParsedReference {
                            caller_symbol: Some(name.clone()),
                            callee_symbol: base.to_string(),
                            ref_kind: "inherit".to_string(),
                            line: node.start_position().row + 1,
                        });
                    }
                }
            }
        }

        let signature = if bases.is_empty() {
            format!("class {}", name)
        } else {
            format!("class {} : {}", name, bases.join(", "))
        };

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(signature),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });

        // Extract methods from class body
        let mut bc = node.walk();
        for child in node.children(&mut bc) {
            if child.kind() == "class_body" {
                let mut body_cursor = child.walk();
                for member in child.children(&mut body_cursor) {
                    match member.kind() {
                        "fun_declaration" => {
                            Self::extract_function(&member, source, result, Some(&name));
                        }
                        "property_declaration" => {
                            Self::extract_property(&member, source, result, Some(&name));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn extract_interface(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Interface,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("interface {}", name)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
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
        let Some(name) = name else { return };

        let params = node
            .child_by_field_name("parameters")
            .and_then(|p| p.utf8_text(source).ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: if parent.is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            },
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("fun {}({})", name, params)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_property(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::Field,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: None,
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_enum(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Enum,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("enum class {}", name)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_data_class(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else { return };

        let params = node
            .child_by_field_name("parameters")
            .and_then(|p| p.utf8_text(source).ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Struct,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("data class {}({})", name, params)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_companion_object(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        result.symbols.push(ParsedSymbol {
            name: "Companion".to_string(),
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("companion object".to_string()),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
        if let Ok(text) = node.utf8_text(source) {
            let trimmed = text.trim().to_string();
            if trimmed.starts_with("import ") {
                let module = trimmed.strip_prefix("import ").unwrap_or(&trimmed).to_string();
                result.imports.push((
                    module.clone(),
                    module,
                    "import".to_string(),
                ));
            }
        }
    }

    fn extract_object_literal(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        result.symbols.push(ParsedSymbol {
            name: "object".to_string(),
            kind: SymbolKind::Class,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("object".to_string()),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }
}

impl SourceCodeParser for KotlinParser {
    fn language(&self) -> &str {
        "kotlin"
    }

    fn file_extensions(&self) -> &[&str] {
        &["kt", "kts"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        self.do_parse(source)
    }
}
