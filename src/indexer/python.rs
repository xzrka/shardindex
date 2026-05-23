  use crate::indexer::types::*;
use tree_sitter::Node;
use anyhow::Context;

  pub struct PythonParser;

  impl PythonParser {
      pub fn new() -> Result<Self, anyhow::Error> {
          // Validate tree-sitter-python loads
          let mut parser = tree_sitter::Parser::new();
          let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
          parser
              .set_language(&language)
              .context("Failed to load tree-sitter-python")?;
          Ok(Self)
      }

      fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
          let mut parser = tree_sitter::Parser::new();
          let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
          parser
              .set_language(&language)
              .context("Failed to load tree-sitter-python")?;

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

      /// Walk the AST, tracking current class parent and current function context.
      ///
      /// - `parent`: enclosing class name (for method detection + qualified_name)
      /// - `current_function`: enclosing function/method name (for caller_symbol on calls)
      fn walk_node(
          node: &Node,
          source: &[u8],
          result: &mut ParseResult,
          parent: Option<String>,
          current_function: Option<String>,
      ) {
          let kind = node.kind();

          match kind {
              "function_definition" => {
                  Self::extract_function(node, source, result, parent.as_deref());
              }
              "class_definition" => {
                  Self::extract_class(node, source, result, parent.as_deref());
              }
              "import_statement" | "import_from_statement" => {
                  Self::extract_import(node, source, result);
              }
              "expression_statement" => {
                  Self::extract_assignment(node, source, result, parent.as_deref());
              }
              _ => {}
          }

          // Extract call references with caller_symbol context
          Self::extract_calls(node, source, result, current_function.as_deref());

          // Recurse into children
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              let new_parent = if child.kind() == "class_definition" {
                  child
                      .child_by_field_name("name")
                      .and_then(|n| n.utf8_text(source).ok())
                      .map(|s| s.to_string())
              } else {
                  parent.clone()
              };

              // Update current_function if entering a function definition
              let new_function = if child.kind() == "function_definition" {
                  child
                      .child_by_field_name("name")
                      .and_then(|n| n.utf8_text(source).ok())
                      .map(|s| s.to_string())
              } else {
                  current_function.clone()
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

          let signature = node
              .child_by_field_name("parameters")
              .map(|p| format!("def {}({})", name, p.utf8_text(source).unwrap_or("")))
              .or(Some(format!("def {}", name)));

          let docstring = Self::extract_docstring(node, source);

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
              docstring,
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

          let bases = Self::extract_class_bases(node, source);
          let signature = if bases.is_empty() {
              format!("class {}", name)
          } else {
              format!("class {}({})", name, bases.join(", "))
          };

          // Inheritance references
          for base in &bases {
              result.references.push(ParsedReference {
                  caller_symbol: Some(name.clone()),
                  callee_symbol: base.clone(),
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
              docstring: Self::extract_docstring(node, source),
              parent: parent.map(|s| s.to_string()),
          });
      }

      fn extract_import(node: &Node, source: &[u8], result: &mut ParseResult) {
          let import_kind = if node.kind() == "import_from_statement" {
              "from_import"
          } else {
              "import"
          };

          let module_node = if node.kind() == "import_from_statement" {
              node.child_by_field_name("module_name")
          } else {
              node.child_by_field_name("name")
          };

          if let Some(module) = module_node.and_then(|n| n.utf8_text(source).ok()) {
              let mut cursor = node.walk();
              for child in node.children(&mut cursor) {
                  if child.kind() == "dotted_name" || child.kind() == "alias" {
                      if let Some(child_name) = child.utf8_text(source).ok() {
                          result.imports.push((
                              module.to_string(),
                              child_name.to_string(),
                              import_kind.to_string(),
                          ));
                          result.references.push(ParsedReference {
                              caller_symbol: None,
                              callee_symbol: child_name.to_string(),
                              ref_kind: "import".to_string(),
                              line: node.start_position().row + 1,
                          });
                      }
                  }
              }

              result.symbols.push(ParsedSymbol {
                  name: module.to_string(),
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

      fn extract_assignment(
          node: &Node,
          source: &[u8],
          result: &mut ParseResult,
          parent_context: Option<&str>,
      ) {
          if parent_context.is_some() {
              return;
          }

          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "assignment" {
                  if let Some(left) = child.child_by_field_name("left") {
                      if let Some(name) = left.utf8_text(source).ok() {
                          result.symbols.push(ParsedSymbol {
                              name: name.to_string(),
                              kind: SymbolKind::Variable,
                              start_line: child.start_position().row + 1,
                              end_line: child.end_position().row + 1,
                              start_col: child.start_position().column,
                              end_col: child.end_position().column,
                              signature: None,
                              docstring: None,
                              parent: None,
                          });
                      }
                  }
                  break;
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
            if child.kind() == "call" {
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
        }
    }

      fn extract_docstring(node: &Node, source: &[u8]) -> Option<String> {
          let mut cursor = node.walk();
          for child in node.children(&mut cursor) {
              if child.kind() == "block" {
                  let mut block_cursor = child.walk();
                  for stmt in child.children(&mut block_cursor) {
                      if stmt.kind() == "expression_statement" {
                          let mut stmt_cursor = stmt.walk();
                          for expr in stmt.children(&mut stmt_cursor) {
                              if expr.kind() == "string" {
                                  if let Ok(text) = expr.utf8_text(source) {
                                      let cleaned =
                                          text.trim_matches(|c| c == '\'' || c == '"');
                                      return Some(
                                          cleaned
                                              .lines()
                                              .next()
                                              .map(|l| l.to_string())
                                              .unwrap_or_default(),
                                      );
                                  }
                              }
                          }
                          break;
                      }
                  }
              }
          }
          None
      }

      fn extract_class_bases(node: &Node, source: &[u8]) -> Vec<String> {
          let arg_list = node.child_by_field_name("superclasses");
          let Some(args) = arg_list else {
              return Vec::new();
          };

          let mut bases = Vec::new();
          let mut cursor = args.walk();
          for child in args.children(&mut cursor) {
              if let Ok(text) = child.utf8_text(source) {
                  bases.push(text.to_string());
              }
          }
          bases
      }
  }

  impl SourceCodeParser for PythonParser {
      fn language(&self) -> &str {
          "python"
      }

      fn file_extensions(&self) -> &[&str] {
          &["py"]
      }

      fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
          Self::do_parse(self, source)
      }
  }

