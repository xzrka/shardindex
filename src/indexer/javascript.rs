  use crate::indexer::types::*;
use tree_sitter::Node;
use anyhow::Context;

  pub struct JavaScriptParser;

  impl JavaScriptParser {
      pub fn new() -> Result<Self, anyhow::Error> {
          let mut parser = tree_sitter::Parser::new();
          let language: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
          parser
              .set_language(&language)
              .context("Failed to load tree-sitter-javascript")?;
          Ok(Self)
      }

      fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
          let mut parser = tree_sitter::Parser::new();
          let language: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
          parser
              .set_language(&language)
              .context("Failed to load tree-sitter-javascript")?;

          let tree = parser.parse(source, None).context("tree-sitter parse failed")?;
          let root = tree.root_node();
          let source_bytes = source.as_bytes();
          let mut result = ParseResult {
              symbols: Vec::new(),
              references: Vec::new(),
              imports: Vec::new(),
          };

          Self::walk_node(&root, source_bytes, &mut result, None, None);
          Ok(result)
      }

    fn walk_node(
        node: &Node,
        source: &[u8],
        result: &mut ParseResult,
        parent: Option<String>,
        current_function: Option<String>,
    ) {
          let kind = node.kind();

          match kind {
              "function_declaration" => {
                  let is_async = Self::has_child_token(node, "async");
                  Self::extract_function(node, source, result, parent.as_deref(), is_async);
              }
              "class_declaration" => {
                  Self::extract_class(node, source, result, parent.as_deref());
              }
              "import_statement" => {
                  Self::extract_import(node, source, result);
              }
              "export_statement" => {
                  Self::extract_export(node, source, result);
              }
              "lexical_declaration" => {
                  Self::extract_lexical_declaration(node, source, result, parent.as_deref());
              }
              "variable_declaration" => {
                  Self::extract_variable_declaration(node, source, result, parent.as_deref());
              }
              _ => {}
          }

          // Extract call references
          Self::extract_calls(node, source, result, current_function.as_deref());

          // Recurse into children
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              let new_parent = if child.kind() == "class_declaration" {
                  child
                      .child_by_field_name("name")
                      .and_then(|n| n.utf8_text(source).ok())
                      .map(|s| s.to_string())
              } else {
                  parent.clone()
              };
              let new_function = match child.kind() {
                  "function_declaration" | "generator_function" => {
                      child
                          .child_by_field_name("id")
                          .and_then(|n| n.utf8_text(source).ok())
                          .map(|s| s.to_string())
                  }
                  "method_definition" => {
                      child
                          .child_by_field_name("key")
                          .and_then(|n| n.utf8_text(source).ok())
                          .map(|s| s.to_string())
                  }
                  "arrow_function" => {
                      // Keep the parent function name for arrow functions
                      current_function.clone()
                  }
                  _ => current_function.clone(),
              };
              Self::walk_node(&child, source, result, new_parent, new_function);
          }
      }

      /// Check if a node has a direct child token with the given kind
      fn has_child_token(node: &Node, token_kind: &str) -> bool {
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == token_kind {
                  return true;
              }
          }
          false
      }

      fn extract_function(
          node: &Node,
          source: &[u8],
          result: &mut ParseResult,
          parent: Option<&str>,
          is_async: bool,
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
              .and_then(|p| p.utf8_text(source).ok())
              .map(|s| s.to_string())
              .unwrap_or_default();

          let prefix = if is_async { "async " } else { "" };
          let signature = Some(format!("{}function {}({})", prefix, name, params));

          result.symbols.push(ParsedSymbol {
              name,
              kind: if parent.is_some() {
                  SymbolKind::Method
              } else {
                  SymbolKind::Function
              },
              start_line: node.start_position().row + 1,
              end_line: node.end_position().row + 1,
              start_col: node.start_position().column,
              end_col: node.end_position().column,
              signature,
              docstring: Self::extract_j_sdoc(node, source),
              parent: parent.map(|s| s.to_string()),
          });
      }

       fn extract_method(
          node: &Node,
          source: &[u8],
          result: &mut ParseResult,
          parent: Option<&str>,
      ) {
          // method_definition: try "name" field first, fall back to property_identifier
          let name = node
              .child_by_field_name("name")
              .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()))
              .or_else(|| {
                  // Fallback: find property_identifier as direct child
                  let mut cursor = node.walk();
                  for child in node.children(&mut cursor) {
                      if child.kind() == "property_identifier" {
                          return child.utf8_text(source).ok().map(|s| s.to_string());
                      }
                  }
                  None
              });

          let Some(name) = name else {
              return;
          };

          let params = node
              .child_by_field_name("parameters")
              .and_then(|p| p.utf8_text(source).ok())
              .map(|s| s.to_string())
              .unwrap_or_default();

          let is_async = Self::has_child_token(node, "async");
          let prefix = if is_async { "async " } else { "" };
          let signature = Some(format!("{}{}({})", prefix, name, params));

          result.symbols.push(ParsedSymbol {
              name,
              kind: SymbolKind::Method,
              start_line: node.start_position().row + 1,
              end_line: node.end_position().row + 1,
              start_col: node.start_position().column,
              end_col: node.end_position().column,
              signature,
              docstring: Self::extract_j_sdoc(node, source),
              parent: parent.map(|s| s.to_string()),
          });
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

          let Some(name) = name else {
              return;
          };

          // Extract extends: find class_heritage as direct child of class_declaration
          let mut bases = Vec::new();
          let mut heritage_cursor = node.walk();
          for child in node.children(&mut heritage_cursor) {
              if child.kind() == "class_heritage" {
                  let mut ch_cursor = child.walk();
                  for heritage_child in child.children(&mut ch_cursor) {
                      if heritage_child.kind() == "identifier" {
                          if let Ok(base) = heritage_child.utf8_text(source) {
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
          }

          let signature = if bases.is_empty() {
              format!("class {}", name)
          } else {
              format!("class {} extends {}", name, bases.join(", "))
          };

          // BUG-011 fix: prevent class from having itself as parent
          let effective_parent = parent
              .filter(|p| *p != name)
              .map(|s| s.to_string());

          result.symbols.push(ParsedSymbol {
              name: name.clone(),
              kind: SymbolKind::Class,
              start_line: node.start_position().row + 1,
              end_line: node.end_position().row + 1,
              start_col: node.start_position().column,
              end_col: node.end_position().column,
              signature: Some(signature),
              docstring: Self::extract_j_sdoc(node, source),
              parent: effective_parent,
          });

          // Extract methods from class_body
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "class_body" {
                  let mut body_cursor = child.walk();
                  for member in child.children(&mut body_cursor) {
                      if member.kind() == "method_definition" {
                          Self::extract_method(&member, source, result, Some(&name));
                      }
                  }
              }
          }
      }

      fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
          let module = Self::find_string_child(node, source);

          // Find import_clause as a direct child of import_statement
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "import_clause" {
                  let mut ic_cursor = child.walk();
                  for ic_child in child.children(&mut ic_cursor) {
                      if ic_child.kind() == "named_imports" {
                          let mut ni_cursor = ic_child.walk();
                          for spec in ic_child.children(&mut ni_cursor) {
                              if spec.kind() == "import_specifier" {
                                  let mut spec_cursor = spec.walk();
                                  for spec_child in spec.children(&mut spec_cursor) {
                                      if spec_child.kind() == "identifier" {
                                          if let Ok(name) = spec_child.utf8_text(source) {
                                              if let Some(ref mod_name) = module {
                                                  result.imports.push((
                                                      mod_name.clone(),
                                                      name.to_string(),
                                                      "from_import".to_string(),
                                                  ));
                                                  result.references.push(ParsedReference {
                                                      caller_symbol: None,
                                                      callee_symbol: name.to_string(),
                                                      ref_kind: "import".to_string(),
                                                      line: node.start_position().row + 1,
                                                  });
                                              }
                                          }
                                      }
                                  }
                              }
                          }
                      }
                      if ic_child.kind() == "identifier" {
                          if let Ok(name) = ic_child.utf8_text(source) {
                              if let Some(ref mod_name) = module {
                                  result.imports.push((
                                      mod_name.clone(),
                                      name.to_string(),
                                      "default_import".to_string(),
                                  ));
                                  result.references.push(ParsedReference {
                                      caller_symbol: None,
                                      callee_symbol: name.to_string(),
                                      ref_kind: "import".to_string(),
                                      line: node.start_position().row + 1,
                                  });
                              }
                          }
                      }
                  }
              }
          }

          if let Some(ref mod_name) = module {
              result.symbols.push(ParsedSymbol {
                  name: mod_name.clone(),
                  kind: SymbolKind::Import,
                  start_line: node.start_position().row + 1,
                  end_line: node.end_position().row + 1,
                  start_col: node.start_position().column,
                  end_col: node.end_position().column,
                  signature: None,
                  docstring: None,
                  parent: None,
              });
          }
      }

      fn find_string_child(node: &Node, source: &[u8]) -> Option<String> {
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "string" {
                  if let Ok(text) = child.utf8_text(source) {
                      let trimmed = text
                          .trim_matches(|c| c == '\'' || c == '"' || c == '`')
                          .to_string();
                      return Some(trimmed);
                  }
              }
          }
          None
      }

      fn extract_export(node: &Node, source: &[u8], result: &mut ParseResult) {
          let is_default = Self::has_child_token(node, "default");

          if is_default {
              let mut cursor = node.walk();
              for child in node.children(&mut cursor) {
                  if child.kind() == "identifier" {
                      if let Ok(name) = child.utf8_text(source) {
                          result.symbols.push(ParsedSymbol {
                              name: name.to_string(),
                              kind: SymbolKind::Export,
                              start_line: node.start_position().row + 1,
                              end_line: node.end_position().row + 1,
                              start_col: node.start_position().column,
                              end_col: node.end_position().column,
                              signature: None,
                              docstring: None,
                              parent: None,
                          });
                      }
                      break;
                  }
              }
          }

          // Find export_clause as a direct child of export_statement
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "export_clause" {
                  let mut ec_cursor = child.walk();
                  for specifier in child.children(&mut ec_cursor) {
                      if specifier.kind() == "export_specifier" {
                          let mut spec_cursor = specifier.walk();
                          for spec_child in specifier.children(&mut spec_cursor) {
                              if spec_child.kind() == "identifier" {
                                  if let Ok(name) = spec_child.utf8_text(source) {
                                      result.symbols.push(ParsedSymbol {
                                          name: name.to_string(),
                                          kind: SymbolKind::Export,
                                          start_line: node.start_position().row + 1,
                                          end_line: node.end_position().row + 1,
                                          start_col: node.start_position().column,
                                          end_col: node.end_position().column,
                                          signature: None,
                                          docstring: None,
                                          parent: None,
                                      });
                                  }
                                  break;
                              }
                          }
                      }
                  }
              }
          }
      }

      fn extract_lexical_declaration(
          node: &Node,
          source: &[u8],
          result: &mut ParseResult,
          parent: Option<&str>,
      ) {
          if parent.is_some() {
              return;
          }

          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "variable_declarator" {
                  if let Some(name_node) = child.child_by_field_name("name") {
                      if let Ok(name) = name_node.utf8_text(source) {
                          // Check if value is an arrow function
                          let value = child.child_by_field_name("value");
                          let signature = value
                              .and_then(|v| v.utf8_text(source).ok())
                              .map(|v| format!("const {} = {}", name, v))
                              .or(Some(format!("const {}", name)));

                          result.symbols.push(ParsedSymbol {
                              name: name.to_string(),
                              kind: SymbolKind::Variable,
                              start_line: child.start_position().row + 1,
                              end_line: child.end_position().row + 1,
                              start_col: child.start_position().column,
                              end_col: child.end_position().column,
                              signature: signature.clone(),
                              docstring: None,
                              parent: None,
                          });

                          // If it's an arrow function, also extract as function
                          if let Some(ref val_str) = signature {
                              if val_str.contains("=>") {
                                  result.symbols.push(ParsedSymbol {
                                      name: format!("{} (arrow)", name),
                                      kind: SymbolKind::Function,
                                      start_line: child.start_position().row + 1,
                                      end_line: child.end_position().row + 1,
                                      start_col: child.start_position().column,
                                      end_col: child.end_position().column,
                                      signature: signature.clone(),
                                      docstring: None,
                                      parent: None,
                                  });
                              }
                          }
                      }
                  }
              }
          }
      }

      fn extract_variable_declaration(
          node: &Node,
          source: &[u8],
          result: &mut ParseResult,
          parent: Option<&str>,
      ) {
          if parent.is_some() {
              return;
          }

          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "variable_declarator" {
                  if let Some(name_node) = child.child_by_field_name("name") {
                      if let Ok(name) = name_node.utf8_text(source) {
                          let value = child.child_by_field_name("value");
                          let signature = value
                              .and_then(|v| v.utf8_text(source).ok())
                              .map(|v| format!("var {} = {}", name, v))
                              .or(Some(format!("var {}", name)));

                          result.symbols.push(ParsedSymbol {
                              name: name.to_string(),
                              kind: SymbolKind::Variable,
                              start_line: child.start_position().row + 1,
                              end_line: child.end_position().row + 1,
                              start_col: child.start_position().column,
                              end_col: child.end_position().column,
                              signature,
                              docstring: None,
                              parent: None,
                          });
                      }
                  }
              }
          }
      }

      fn extract_calls(
          node: &Node,
          source: &[u8],
          result: &mut ParseResult,
          caller: Option<&str>,
      ) {
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "call_expression" {
                  if let Some(callee) = child.child_by_field_name("function") {
                      let callee_text = callee.utf8_text(source).unwrap_or("").to_string();
                      if !callee_text.is_empty() {
                          result.references.push(ParsedReference {
                              caller_symbol: caller.map(|s| s.to_string()),
                              callee_symbol: callee_text,
                              ref_kind: "call".to_string(),
                              line: child.start_position().row + 1,
                          });
                      }
                  }
              }
          }
      }

      fn extract_j_sdoc(node: &Node, source: &[u8]) -> Option<String> {
          // Look for comment_line or comment nodes before the declaration
          // tree-sitter doesn't attach comments to the AST by default,
          // so we check the raw source for preceding comments
          let start_row = node.start_position().row;
          let lines: Vec<&str> = std::str::from_utf8(source)
              .unwrap_or("")
              .split('\n')
              .collect();

          let mut i = start_row.saturating_sub(1);
          let mut comment_lines = Vec::new();

          while i < lines.len() {
              let line = lines[i].trim();
              if line.starts_with("//") {
                  let text = line.strip_prefix("//").unwrap_or(line).trim();
                  comment_lines.insert(0, text);
                  i = i.saturating_sub(1);
              } else if line.starts_with("/*") {
                  // Multi-line comment - collect until */
                  let mut full = String::new();
                  let mut j = i;
                  while j < lines.len() {
                      full.push_str(lines[j]);
                      full.push('\n');
                      if lines[j].contains("*/") {
                          break;
                      }
                      j += 1;
                  }
                  let cleaned = full.trim_matches(|c| c == '/' || c == '*').trim();
                  let first_line = cleaned.lines().next().map(|l| l.to_string());
                  return first_line;
              } else {
                  break;
              }
          }

          if comment_lines.is_empty() {
              None
          } else {
              Some(comment_lines.join(" ").trim().to_string())
          }
      }
  }

  impl SourceCodeParser for JavaScriptParser {
      fn language(&self) -> &str {
          "javascript"
      }

      fn file_extensions(&self) -> &[&str] {
          &["js", "jsx", "mjs", "cjs"]
      }

      fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
          Self::do_parse(self, source)
      }
  }

