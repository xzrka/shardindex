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
            string_literals: Vec::new(),
        };

        // Walk from root node (which is "component")
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            Self::walk_component(&child, source_bytes, &mut result);
        }
        Ok(result)
    }

    fn find_child<'a>(node: &'a Node, kind: &str) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        node.children(&mut cursor).find(|n| n.kind() == kind)
    }

    fn walk_component(node: &Node, source: &[u8], result: &mut ParseResult) {
        // node itself is one of: template_element, script_element, style_element
        match node.kind() {
            "script_element" => {
                let script_content = Self::extract_script_content(node, source);
                if let Some(content) = script_content {
                    let is_ts = content.contains("interface ")
                        || content.contains(": string")
                        || content.contains(": number")
                        || content.contains(": any")
                        || content.contains(": void");

                    if is_ts {
                        if let Ok(mut ts_parser) = super::TypeScriptParser::new() {
                            if let Ok(ts_result) = ts_parser.parse(&content) {
                                let line_offset = node.start_position().row;
                                for sym in ts_result.symbols {
                                    result.symbols.push(ParsedSymbol {
                                        start_line: sym.start_line + line_offset,
                                        end_line: sym.end_line + line_offset,
                                        ..sym
                                    });
                                }
                                result.references.extend(ts_result.references);
                                result.imports.extend(ts_result.imports);
                            }
                        }
                    } else {
                        if let Ok(mut js_parser) = super::JavaScriptParser::new() {
                            if let Ok(js_result) = js_parser.parse(&content) {
                                let line_offset = node.start_position().row;
                                for sym in js_result.symbols {
                                    result.symbols.push(ParsedSymbol {
                                        start_line: sym.start_line + line_offset,
                                        end_line: sym.end_line + line_offset,
                                        ..sym
                                    });
                                }
                                result.references.extend(js_result.references);
                                result.imports.extend(js_result.imports);
                            }
                        }
                    }
                }
            }
            "template_element" => {
                let template_name = Self::extract_template_name(node, source);
                if let Some(name) = template_name {
                    result.symbols.push(ParsedSymbol {
                        name: format!("<{}>", name),
                        kind: SymbolKind::Class,
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        start_col: node.start_position().column,
                        end_col: node.end_position().column,
                        signature: Some(format!("<{}>", name)),
                        docstring: None,
                        parent: None,
                    });
                }
            }
            "style_element" => {}
            _ => {}
        }
    }

    fn extract_script_content(node: &Node, source: &[u8]) -> Option<String> {
        let mut cursor = node.walk();
        let mut text_parts = Vec::new();

        for child in node.children(&mut cursor) {
            // tree-sitter-vue-updated uses "raw_text" for script/style content
            // and "text" for template text nodes
            if child.kind() == "raw_text" || child.kind() == "text" {
                if let Ok(text) = child.utf8_text(source) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        text_parts.push(trimmed.to_string());
                    }
                }
            }
        }

        if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join("\n"))
        }
    }

    fn extract_template_name(node: &Node, source: &[u8]) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "element" {
                // tag_name is inside start_tag, not a direct child of element
                if let Some(start_tag) = Self::find_child(&child, "start_tag") {
                    return Self::find_child(&start_tag, "tag_name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string());
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
