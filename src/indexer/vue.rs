use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct VueParser;

impl VueParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_vue_updated::language();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-vue-updated")?;

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

    fn walk_node(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<String>,
    ) {
        match node.kind() {
            "script" => {
                // Extract <script> block content - delegate to JS/TS parser
                Self::extract_script_block(node, source, result);
            }
            "template" => {
                Self::extract_template(node, source, result);
            }
            "style" => {
                Self::extract_style_block(node, source, result);
            }
            "custom_block" => {
                Self::extract_custom_block(node, source, result);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_node(&child, source, result, parent.clone());
        }
    }

    fn extract_script_block(node: &Node, source: &[u8], result: &mut ParseResult) {
        // Check for lang attribute
        let lang = Self::get_attribute(node, source, "lang");
        let is_module = Self::get_attribute(node, source, "type")
            .map(|t| t.contains("module"))
            .unwrap_or(false);

        let block_type = match (&lang, is_module) {
            (Some(l), _) => format!("script({})", l),
            (_, true) => "script(module)".to_string(),
            _ => "script".to_string(),
        };

        result.symbols.push(ParsedSymbol {
            name: block_type,
            kind: SymbolKind::Module,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("<script>")),
            docstring: None,
            parent: None,
        });

        // Extract import statements from script content
        let text = node.utf8_text(source).unwrap_or("");
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("import ") {
                result.references.push(ParsedReference {
                    caller_symbol: None,
                    callee_symbol: trimmed.to_string(),
                    ref_kind: "import".to_string(),
                    line: node.start_position().row + 1,
                });
            }
        }
    }

    fn extract_template(node: &Node, source: &[u8], result: &mut ParseResult) {
        result.symbols.push(ParsedSymbol {
            name: "template".to_string(),
            kind: SymbolKind::Module,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some("<template>".to_string()),
            docstring: None,
            parent: None,
        });

        // Extract v-bind and dynamic imports
        let text = node.utf8_text(source).unwrap_or("");
        for (line_num, line) in text.lines().enumerate() {
            if line.contains("v-bind:require") || line.contains("@") {
                result.references.push(ParsedReference {
                    caller_symbol: Some("template".to_string()),
                    callee_symbol: line.trim().to_string(),
                    ref_kind: "template_ref".to_string(),
                    line: node.start_position().row + 1 + line_num,
                });
            }
        }
    }

    fn extract_style_block(node: &Node, source: &[u8], result: &mut ParseResult) {
        let lang = Self::get_attribute(node, source, "lang");
        let scoped = Self::get_attribute(node, source, "scoped").is_some();

        let block_type = match (&lang, scoped) {
            (Some(l), true) => format!("style({},scoped)", l),
            (Some(l), false) => format!("style({})", l),
            (_, true) => "style(scoped)".to_string(),
            _ => "style".to_string(),
        };

        result.symbols.push(ParsedSymbol {
            name: block_type,
            kind: SymbolKind::Module,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("<style>")),
            docstring: None,
            parent: None,
        });
    }

    fn extract_custom_block(node: &Node, source: &[u8], result: &mut ParseResult) {
        let tag = Self::get_block_tag(node, source);
        let name = tag.unwrap_or_else(|| "custom".to_string());

        result.symbols.push(ParsedSymbol {
            name: name.clone(),
            kind: SymbolKind::Module,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("<{}>", name)),
            docstring: None,
            parent: None,
        });
    }

    fn get_attribute(node: &Node, source: &[u8], attr_name: &str) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "attribute" {
                let mut ac = child.walk();
                for attr_child in child.children(&mut ac) {
                    if let Ok(text) = attr_child.utf8_text(source) {
                        if text == attr_name {
                            // Find the value
                            let mut vc = child.walk();
                            for val in child.children(&mut vc) {
                                if val.kind() == "attribute_value" {
                                    return val.utf8_text(source).ok().map(|s| s.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn get_block_tag(node: &Node, source: &[u8]) -> Option<String> {
        let text = node.utf8_text(source).ok()?;
        if let Some(start) = text.find('<') {
            if let Some(end) = text[start..].find('>') {
                let tag = text[start + 1..start + end].trim();
                if !tag.is_empty() {
                    return Some(tag.to_string());
                }
            }
        }
        None
    }
}

impl SourceCodeParser for VueParser {
    fn language(&self) -> &str {
        "vue"
    }

    fn file_extensions(&self) -> &[&str] {
        &["vue"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        self.do_parse(source)
    }
}
