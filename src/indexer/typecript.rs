  use crate::indexer::types::*;
use tree_sitter::Node;
use anyhow::Context;

  pub struct TypeScriptParser;

  impl TypeScriptParser {
      pub fn new() -> Result<Self, anyhow::Error> {
          let mut parser = tree_sitter::Parser::new();
          let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
          parser
              .set_language(&language)
              .context("Failed to load tree-sitter-typescript")?;
          Ok(Self)
      }

      fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
          let mut parser = tree_sitter::Parser::new();
          let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
          parser
              .set_language(&language)
              .context("Failed to load tree-sitter-typescript")?;

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
                  Self::extract_function(node, source, result, parent.as_deref());
              }
              "class_declaration" => {
                  Self::extract_class(node, source, result, parent.as_deref());
              }
              "method_definition" => {
                  // Only extract if not already inside a class parent context
                  Self::extract_method(node, source, result, parent.as_deref());
              }
              "import_statement" => {
                  Self::extract_import(node, source, result);
              }
              "export_statement" => {
                  Self::extract_export(node, source, result, parent.as_deref());
              }
              "type_alias_declaration" | "type_annotation" => {
                  // type aliases
                  if kind == "type_alias_declaration" {
                      Self::extract_type_alias(node, source, result, parent.as_deref());
                  }
              }
              "interface_declaration" => {
                  Self::extract_interface(node, source, result, parent.as_deref());
              }
              "enum_declaration" => {
                  Self::extract_enum(node, source, result, parent.as_deref());
              }
              "module_declaration" => {
                  Self::extract_module(node, source, result, parent.as_deref());
              }
              "variable_declaration" => {
                  // Top-level variables (const, let, var)
                  if parent.is_none() {
                      Self::extract_variable(node, source, result);
                  }
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
                "function_declaration" => {
                    child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string())
                }
                "method_definition" | "abstract_method_definition" => {
                    child
                        .child_by_field_name("key")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.to_string())
                }
                "arrow_function" | "generator_function" => current_function.clone(),
                _ => current_function.clone(),
            };
            Self::walk_node(&child, source, result, new_parent, new_function);
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

          let is_async = node
              .child_by_field_name("async")
              .is_some()
              .then_some("async ")
              .unwrap_or("");

          let sig = format!("{}function {}({})", is_async, name, params);
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

          // Get superclass
          let superclass = node
              .child_by_field_name("superClass")
              .and_then(|n| n.utf8_text(source).ok())
              .map(|s| s.to_string());

          let signature = if let Some(sc) = &superclass {
              format!("class {} extends {}", name, sc)
          } else {
              format!("class {}", name)
          };

          // Inheritance reference
          if let Some(sc) = superclass {
              result.references.push(ParsedReference {
                  caller_symbol: Some(name.clone()),
                  callee_symbol: sc,
                  ref_kind: "inherit".to_string(),
                  line: node.start_position().row + 1,
              });
          }

          result.symbols.push(ParsedSymbol {
              name,
              kind: SymbolKind::Class,
              start_line: node.start_position().row + 1,
              end_line: node.end_position().row + 1,
              start_col: node.start_position().column,
              end_col: node.end_position().column,
              signature: Some(signature),
              docstring: None,
              parent: parent.map(|s| s.to_string()),
          });
      }

      fn extract_method(
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

          let sig = format!("{}({})", name, params);
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

      fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
          let module_node = node.child_by_field_name("source");
          let Some(module) = module_node.and_then(|n| n.utf8_text(source).ok()) else {
              return;
          };

          let cleaned = module.trim_matches('"').trim_matches('\'').to_string();

          // Extract imported names
          let mut names = Vec::new();
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "import_clause" {
                  let mut ic_cursor = child.walk();
                  for ic_child in child.children(&mut ic_cursor) {
                      if ic_child.kind() == "identifier" {
                          if let Ok(n) = ic_child.utf8_text(source) {
                              names.push(n.to_string());
                          }
                      }
                      if ic_child.kind() == "named_imports" {
                          let mut ni_cursor = ic_child.walk();
                          for ns in ic_child.children(&mut ni_cursor) {
                              if ns.kind() == "import_specifier" {
                                  // import_specifier has identifier child
                                  let mut spec_cursor = ns.walk();
                                  for spec_child in ns.children(&mut spec_cursor) {
                                      if spec_child.kind() == "identifier" {
                                          if let Ok(name) = spec_child.utf8_text(source) {
                                              names.push(name.to_string());
                                          }
                                      }
                                  }
                              }
                          }
                      }
                  }
              }
          }

          for name in &names {
              result.imports.push((
                  cleaned.clone(),
                  name.clone(),
                  "import".to_string(),
              ));
              result.references.push(ParsedReference {
                  caller_symbol: None,
                  callee_symbol: name.clone(),
                  ref_kind: "import".to_string(),
                  line: node.start_position().row + 1,
              });
          }

          result.symbols.push(ParsedSymbol {
              name: cleaned.clone(),
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

      fn extract_export(
          node: &Node,
          source: &[u8],
          result: &mut ParseResult,
          _parent: Option<&str>,
      ) {
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "export_clause" {
                  let mut ec_cursor = child.walk();
                  for ec_child in child.children(&mut ec_cursor) {
                      if ec_child.kind() == "export_specifier" {
                          if let Ok(name) = ec_child.utf8_text(source) {
                              result.symbols.push(ParsedSymbol {
                                  name: name.to_string(),
                                  kind: SymbolKind::Export,
                                  start_line: ec_child.start_position().row + 1,
                                  end_line: ec_child.end_position().row + 1,
                                  start_col: ec_child.start_position().column,
                                  end_col: ec_child.end_position().column,
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

          let sig = format!("type {}", name);
          result.symbols.push(ParsedSymbol {
              name,
              kind: SymbolKind::Class, // Reuse Class for type alias
              start_line: node.start_position().row + 1,
              end_line: node.end_position().row + 1,
              start_col: node.start_position().column,
              end_col: node.end_position().column,
              signature: Some(sig),
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
              .or_else(|| node.child(1))
              .and_then(|n| n.utf8_text(source).ok())
              .map(|s| s.to_string());

          let Some(name) = name else {
              return;
          };

          // Extract enum members as sub-symbols
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "enum_body" {
                  let mut body_cursor = child.walk();
                  for member in child.children(&mut body_cursor) {
                      if member.kind() == "property_identifier" {
                          if let Some(member_name) = member.utf8_text(source).ok() {
                              result.symbols.push(ParsedSymbol {
                                  name: format!("{}::{}", name, member_name),
                                  kind: SymbolKind::Variable,
                                  start_line: member.start_position().row + 1,
                                  end_line: member.end_position().row + 1,
                                  start_col: member.start_position().column,
                                  end_col: member.end_position().column,
                                  signature: Some(format!("enum member {}", member_name)),
                                  docstring: None,
                                  parent: Some(name.clone()),
                              });
                          }
                      }
                  }
              }
          }

          let sig = format!("enum {}", name);
          result.symbols.push(ParsedSymbol {
              name,
              kind: SymbolKind::Enum,
              start_line: node.start_position().row + 1,
              end_line: node.end_position().row + 1,
              start_col: node.start_position().column,
              end_col: node.end_position().column,
              signature: Some(sig),
              docstring: None,
              parent: parent.map(|s| s.to_string()),
          });
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

          let Some(name) = name else {
              return;
          };

          let sig = format!("interface {}", name);
          result.symbols.push(ParsedSymbol {
              name,
              kind: SymbolKind::Class, // Reuse Class for interface
              start_line: node.start_position().row + 1,
              end_line: node.end_position().row + 1,
              start_col: node.start_position().column,
              end_col: node.end_position().column,
              signature: Some(sig),
              docstring: None,
              parent: parent.map(|s| s.to_string()),
          });
      }

      fn extract_module(
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

          let sig = format!("namespace {}", name);
          result.symbols.push(ParsedSymbol {
              name,
              kind: SymbolKind::Module,
              start_line: node.start_position().row + 1,
              end_line: node.end_position().row + 1,
              start_col: node.start_position().column,
              end_col: node.end_position().column,
              signature: Some(sig),
              docstring: None,
              parent: parent.map(|s| s.to_string()),
          });
      }

      fn extract_variable(node: &Node, source: &[u8], result: &mut ParseResult) {
          let name = node
              .child_by_field_name("name")
              .and_then(|n| n.utf8_text(source).ok())
              .map(|s| s.to_string());

          let Some(name) = name else {
              return;
          };

          result.symbols.push(ParsedSymbol {
              name,
              kind: SymbolKind::Variable,
              start_line: node.start_position().row + 1,
              end_line: node.end_position().row + 1,
              start_col: node.start_position().column,
              end_col: node.end_position().column,
              signature: None,
              docstring: None,
              parent: None,
          });
      }

   fn extract_calls(node: &Node, source: &[u8], result: &mut ParseResult, caller: Option<&str>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "call_expression" {
                if let Some(func) = child.child_by_field_name("function") {
                    let callee = func.utf8_text(source).unwrap_or("").to_string();
                    if !callee.is_empty() {
                        result.references.push(ParsedReference {
                            caller_symbol: caller.map(|s| s.to_string()),
                            callee_symbol: callee,
                            ref_kind: "call".to_string(),
                            line: child.start_position().row + 1,
                        });
                    }
                }
            }
            // Recurse
            Self::extract_calls(&child, source, result, caller);
        }
    }
  }

  impl SourceCodeParser for TypeScriptParser {
      fn language(&self) -> &str {
          "typescript"
      }

      fn file_extensions(&self) -> &[&str] {
          &["ts", "tsx", "mts", "cts"]
      }

      fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
          Self::do_parse(self, source)
      }
  }

