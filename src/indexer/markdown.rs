/// Markdown parser — tree-sitter-markdown-updated backend
///
/// Extracts sections (headings), code blocks, and links from Markdown files.
/// Useful for indexing documentation, READMEs, and specification files.
///
/// AST structure (tree-sitter-markdown-updated 0.1.0):
///   atx_heading
///     atx_h1_marker (or atx_h2_marker, ...)
///     heading_content
///       text
///   fenced_code_block
///     info_string
///       text (language name)
///     code_fence_content
///       text (code lines)
///   link
///     link_text
///       text
///     link_destination
///       text (URL)

use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct MarkdownParser;

impl MarkdownParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language = tree_sitter_markdown_updated::language();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-markdown")?;

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

        // Extract links first (separate pass to avoid duplicates)
        Self::extract_links(&root, source_bytes, &mut result);

        Self::walk_node(&root, source_bytes, &mut result, None);
        Ok(result)
    }

    fn walk_node(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<String>) {
        let kind = node.kind();

        // Process block-level elements
        match kind {
            "atx_heading" => {
                Self::extract_heading(node, source, result, parent.as_deref());
                // Don't recurse into heading children
                return;
            }
            "fenced_code_block" => {
                Self::extract_code_block(node, source, result, parent.as_deref());
                // Don't recurse into code block children
                return;
            }
            _ => {}
        }

        // Links are extracted in do_parse() to avoid duplicates

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_node(&child, source, result, parent.clone());
        }
    }

    /// Extract heading — uses `heading_content` field for the actual text.
    /// Level is determined by the marker node kind (`atx_h1_marker`, `atx_h2_marker`, ...).
    fn extract_heading(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        // Get heading content via the "heading_content" field
        let content_node = node
            .child_by_field_name("heading_content")
            .or_else(|| node.child(1)) // fallback: second child after marker
            .and_then(|n| n.utf8_text(source).ok());

        let Some(raw) = content_node else {
            return;
        };

        // Strip inline formatting
        let cleaned = raw
            .replace("**", "")
            .replace("*", "")
            .replace("``", "`")
            .replace("~~", "")
            .trim()
            .to_string();

        if cleaned.is_empty() {
            return;
        }

        // Determine level from marker node kind: atx_h1_marker → 1, atx_h2_marker → 2, ...
        let level = node
            .child(0)
            .map(|m| {
                let kind = m.kind(); // "atx_h1_marker", "atx_h2_marker", ...
                kind.strip_prefix("atx_h")
                    .and_then(|s| s.strip_suffix("_marker"))
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(1)
            })
            .unwrap_or(1);

        result.symbols.push(ParsedSymbol {
            name: cleaned,
            kind: SymbolKind::Section,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(format!("h{}", level)),
            docstring: None,
            parent: parent.map(|s| s.to_string()),
        });
    }

    fn extract_code_block(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<&str>,
    ) {
        // Language from first child (info_string) - child_by_field_name doesn't work
        // with tree-sitter-markdown-updated 0.1.0, so use child(0) directly
        let language = node
            .child(0)
            .filter(|c| c.kind() == "info_string")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "text".to_string());

        // Code content from second child (code_fence_content)
        let code_content = node
            .child(1)
            .filter(|c| c.kind() == "code_fence_content")
            .and_then(|n| n.utf8_text(source).ok())
            .unwrap_or("");

        let line_count = code_content.lines().count();
        let name = format!("code_block ({} - {} lines)", language, line_count);

        result.symbols.push(ParsedSymbol {
            name,
            kind: SymbolKind::CodeBlock,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_col: node.start_position().column,
            end_col: node.end_position().column,
            signature: Some(language),
            docstring: if line_count <= 20 {
                Some(code_content.to_string())
            } else {
                Some(format!("... ({} lines, truncated) ...", line_count))
            },
            parent: parent.map(|s| s.to_string()),
        });
    }

    /// Extract links — stored as symbols only (no DB references).
    /// External URLs would violate file_hash FK constraints in the reference table.
    fn extract_links(node: &Node, source: &[u8], result: &mut ParseResult) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "link" {
                // Destination from `link_destination` child
                let destination = child
                    .child_by_field_name("link_destination")
                    .or_else(|| {
                        let mut c_cursor = child.walk();
                        for c in child.children(&mut c_cursor) {
                            if c.kind() == "link_destination" {
                                return Some(c);
                            }
                        }
                        None
                    })
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.trim().to_string());

                // Link text from `link_text` child
                let title = child
                    .child_by_field_name("link_text")
                    .or_else(|| child.child(0))
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| "link".to_string());

                if let Some(dest) = destination {
                    if !dest.is_empty() {
                        result.symbols.push(ParsedSymbol {
                            name: title,
                            kind: SymbolKind::Link,
                            start_line: child.start_position().row + 1,
                            end_line: child.end_position().row + 1,
                            start_col: child.start_position().column,
                            end_col: child.end_position().column,
                            signature: Some(dest),
                            docstring: None,
                            parent: None,
                        });
                    }
                }

                // Don't recurse into link children - they are already processed
            } else {
                Self::extract_links(&child, source, result);
            }
        }
    }
}

impl SourceCodeParser for MarkdownParser {
    fn language(&self) -> &str {
        "markdown"
    }

    fn file_extensions(&self) -> &[&str] {
        &["md", "markdown", "mdown", "mkd"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_parse(md: &str) -> ParseResult {
        let parser = MarkdownParser::new().expect("failed to create parser");
        let mut parser = parser;
        parser.parse(md).expect("parse failed")
    }

    #[test]
    fn test_parse_headings() {
        let md = r#"# Main Title
## Section One
### Sub Section
## Section Two
"#;
        let result = test_parse(md);
        let sections: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Section)
            .collect();
        assert_eq!(sections.len(), 4);
        assert_eq!(sections[0].name, "Main Title");
        assert_eq!(sections[0].signature.as_deref(), Some("h1"));
        assert_eq!(sections[1].name, "Section One");
        assert_eq!(sections[1].signature.as_deref(), Some("h2"));
        assert_eq!(sections[2].name, "Sub Section");
        assert_eq!(sections[2].signature.as_deref(), Some("h3"));
    }

    #[test]
    fn test_parse_code_blocks() {
        let md = r#"# Example
```python
def hello():
    print("world")
```

```rust
fn main() {}
```
"#;
        let result = test_parse(md);
        let blocks: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::CodeBlock)
            .collect();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].signature.as_deref(), Some("python"));
        assert_eq!(blocks[1].signature.as_deref(), Some("rust"));
    }

    #[test]
    fn test_parse_links() {
        let md = r#"Check [docs](https://example.com) and [guide](./README.md).
"#;
        let result = test_parse(md);
        let links: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Link)
            .collect();
        assert_eq!(links.len(), 2);
    }

    #[test]
    fn test_empty_md() {
        let result = test_parse("");
        assert!(result.symbols.is_empty());
    }
}
