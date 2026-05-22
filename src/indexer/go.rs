  use crate::indexer::types::*;
use tree_sitter::Node;
use anyhow::Context;

  pub struct GoParser;

  impl GoParser {
      pub fn new() -> Result<Self, anyhow::Error> {
          let mut parser = tree_sitter::Parser::new();
          let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
          parser
              .set_language(&language)
              .context("Failed to load tree-sitter-go")?;
          Ok(Self)
      }

      fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
          let mut parser = tree_sitter::Parser::new();
          let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
          parser
              .set_language(&language)
              .context("Failed to load tree-sitter-go")?;

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
              "function_declaration" => {
                  Self::extract_function(node, source, result, parent.as_deref());
              }
              "type_spec" => {
                  Self::extract_type(node, source, result, parent.as_deref());
              }
              "import_declaration" => {
                  Self::extract_import(node, source, result);
              }
              "var_declaration" => {
                  if parent.is_none() {
                      Self::extract_var(node, source, result);
                  }
              }
              "method_declaration" => {
                  Self::extract_method(node, source, result);
              }
              _ => {}
          }

          // Extract call references
          Self::extract_calls(node, source, result);

          // Recurse into children
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              Self::walk_node(&child, source, result, parent.clone());
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

          let params = node
              .child_by_field_name("parameters")
              .and_then(|n| n.utf8_text(source).ok())
              .map(|s| s.to_string())
              .unwrap_or_else(|| "()".to_string());

          let return_type = node
              .child_by_field_name("result")
              .and_then(|n| n.utf8_text(source).ok())
              .map(|s| s.to_string());

          let sig = if let Some(rt) = return_type {
              format!("func {}{} {}", name, params, rt)
          } else {
              format!("func {}{}", name, params)
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

      fn extract_type(
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

          // Determine type kind
          let type_kind = node
              .child_by_field_name("type")
              .map(|n| n.kind())
              .unwrap_or("");

          let kind = match type_kind {
              "struct_type" | "interface_type" => SymbolKind::Class,
              _ => SymbolKind::Variable, // type alias, etc.
          };

          let sig = format!("type {}", name);
          result.symbols.push(ParsedSymbol {
              name,
              kind,
              start_line: node.start_position().row + 1,
              end_line: node.end_position().row + 1,
              start_col: node.start_position().column,
              end_col: node.end_position().column,
              signature: Some(sig),
              docstring: None,
              parent: parent.map(|s| s.to_string()),
          });
      }

      fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
          // Helper: extract module path from an import_spec node
          let extract_spec = |spec: &Node| -> Option<String> {
              // import_spec has interpreted_string_literal child (NOT a field)
              let mut spec_cursor = spec.walk();
              for child in spec.children(&mut spec_cursor) {
                  if child.kind() == "interpreted_string_literal" {
                      if let Ok(text) = child.utf8_text(source) {
                          let cleaned = text.trim_matches('"').to_string();
                          if !cleaned.is_empty() {
                              return Some(cleaned);
                          }
                      }
                  }
              }
              None
          };

          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              // Single import: import "fmt"
              if child.kind() == "import_spec" {
                  if let Some(cleaned) = extract_spec(&child) {
                      result.imports.push((
                          cleaned.clone(),
                          cleaned.clone(),
                          "import".to_string(),
                      ));
                      result.references.push(ParsedReference {
                          caller_symbol: None,
                          callee_symbol: cleaned,
                          ref_kind: "import".to_string(),
                          line: node.start_position().row + 1,
                      });
                  }
              }
              // Grouped imports: import ( "a" "b" )
              if child.kind() == "import_spec_list" {
                  let mut list_cursor = child.walk();
                  for spec in child.children(&mut list_cursor) {
                      if spec.kind() == "import_spec" {
                          if let Some(cleaned) = extract_spec(&spec) {
                              result.imports.push((
                                  cleaned.clone(),
                                  cleaned.clone(),
                                  "import".to_string(),
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
              }
          }
      }

      fn extract_var(node: &Node, source: &[u8], result: &mut ParseResult) {
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "var_spec" {
                  let mut spec_cursor = child.walk();
                  for spec_child in child.children(&mut spec_cursor) {
                      if spec_child.kind() == "identifier" {
                          if let Ok(name) = spec_child.utf8_text(source) {
                              result.symbols.push(ParsedSymbol {
                                  name: name.to_string(),
                                  kind: SymbolKind::Variable,
                                  start_line: spec_child.start_position().row + 1,
                                  end_line: spec_child.end_position().row + 1,
                                  start_col: spec_child.start_position().column,
                                  end_col: spec_child.end_position().column,
                                  signature: None,
                                  docstring: None,
                                  parent: None,
                              });
                          }
                      }
                  }
              }
          }
      }

      fn extract_method(
          node: &Node,
          source: &[u8],
          result: &mut ParseResult,
      ) {
          let name = node
              .child_by_field_name("name")
              .and_then(|n| n.utf8_text(source).ok())
              .map(|s| s.to_string());

          let Some(name) = name else {
              return;
          };

          // Get receiver type for parent
          let receiver = node
              .child_by_field_name("receiver")
              .and_then(|n| n.utf8_text(source).ok())
              .map(|s| {
                  let s = s.trim_start_matches('(').trim_end_matches(')');
                  let type_part = s.split_whitespace().nth(1).unwrap_or(s);
                  type_part.trim_start_matches('*').trim_start_matches('&').to_string()
              });

          let params = node
              .child_by_field_name("parameters")
              .and_then(|n| n.utf8_text(source).ok())
              .map(|s| s.to_string())
              .unwrap_or_else(|| "()".to_string());

          let sig = format!(
              "({}) {}({})",
              receiver.as_deref().unwrap_or(""),
              name,
              params
          );
          result.symbols.push(ParsedSymbol {
              name,
              kind: SymbolKind::Method,
              start_line: node.start_position().row + 1,
              end_line: node.end_position().row + 1,
              start_col: node.start_position().column,
              end_col: node.end_position().column,
              signature: Some(sig),
              docstring: None,
              parent: receiver,
          });
      }

      fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult) {
          if node.kind() == "call_expression" {
              if let Some(func) = node.child_by_field_name("function") {
                  let callee = func.utf8_text(source).unwrap_or("").to_string();
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

          // Recurse
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "call_expression" {
                  Self::extract_calls(&child, source, result);
              }
          }
      }
  }

  impl SourceCodeParser for GoParser {
      fn language(&self) -> &str {
          "go"
      }

      fn file_extensions(&self) -> &[&str] {
          &["go"]
      }

      fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
          Self::do_parse(self, source)
      }
  }

