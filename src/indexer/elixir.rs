use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct ElixirParser;

impl ElixirParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_elixir::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-elixir")?;
        Ok(Self)
    }

    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> {
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_elixir::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to load tree-sitter-elixir")?;

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
            "call" => {
                Self::extract_call(node, source, result, parent.as_deref());
            }
            "module_body" | "chunk" | "do_block" => {}
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_node(&child, source, result, parent.clone());
        }
    }

    fn extract_call(node: &Node, source: &[u8], result: &mut ParseResult, parent: Option<&str>) {
        // call = [callee identifier] [arguments] [do_block]
        let mut ci: usize = 0;

        // Find callee (first child must be identifier)
        let callee = if node
            .named_child(ci)
            .map_or(false, |n| n.kind() == "identifier")
        {
            node.named_child(ci).and_then(|n| n.utf8_text(source).ok())
        } else {
            None
        };
        if callee.is_some() {
            ci += 1;
        }

        // Find arguments
        let arguments = loop {
            match node.named_child(ci) {
                None => break None,
                Some(n) if n.kind() == "arguments" => {
                    ci += 1;
                    break n.utf8_text(source).ok();
                }
                _ => break None,
            }
        };

        let has_do_block = node
            .named_child(ci)
            .map_or(false, |n| n.kind() == "do_block");

        match callee {
            Some("defmodule") => {
                if let Some(args_node) = node.named_child(ci - 1) {
                    let mut ac = args_node.walk();
                    for ac in args_node.named_children(&mut ac) {
                        let name = Self::module_name_from_node(&ac, source);
                        if let Some(name) = name {
                            result.symbols.push(ParsedSymbol {
                                name: name.clone(),
                                kind: SymbolKind::Module,
                                start_line: node.start_position().row + 1,
                                end_line: node.end_position().row + 1,
                                start_col: node.start_position().column,
                                end_col: node.end_position().column,
                                signature: Some(format!("defmodule {}", name)),
                                docstring: None,
                                parent: parent.map(String::from),
                            });
                        }
                    }
                }
            }
            Some("def" | "defp") => {
                let visibility = if callee == Some("defp") {
                    "private"
                } else {
                    "public"
                };
                if let Some(args) = arguments {
                    // Extract function name from arguments like "greet(name)"
                    let func_name = Self::elixir_func_name(&args);
                    let func_name = match func_name {
                        Some(n) if !n.is_empty() => n,
                        _ => String::from("anonymous"),
                    };
                    result.symbols.push(ParsedSymbol {
                        name: func_name.clone(),
                        kind: SymbolKind::Function,
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        start_col: node.start_position().column,
                        end_col: node.end_position().column,
                        signature: Some(format!("{} {}", visibility, func_name)),
                        docstring: None,
                        parent: parent.map(String::from),
                    });
                } else if has_do_block {
                    result.symbols.push(ParsedSymbol {
                        name: String::from("anonymous"),
                        kind: SymbolKind::Function,
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        start_col: node.start_position().column,
                        end_col: node.end_position().column,
                        signature: Some(format!("{} anonymous/1", visibility)),
                        docstring: None,
                        parent: parent.map(String::from),
                    });
                }
            }
            Some("defdelegate") => {
                if let Some(args) = arguments {
                    if let Some(sig) = Self::elixir_signature(args) {
                        result.symbols.push(ParsedSymbol {
                            name: sig.clone(),
                            kind: SymbolKind::Function,
                            start_line: node.start_position().row + 1,
                            end_line: node.end_position().row + 1,
                            start_col: node.start_position().column,
                            end_col: node.end_position().column,
                            signature: Some(format!("delegate {}", sig)),
                            docstring: None,
                            parent: parent.map(String::from),
                        });
                    }
                }
            }
            Some("require" | "import" | "use") => {
                let import_kind = if callee == Some("require") {
                    "require"
                } else if callee == Some("import") {
                    "import"
                } else {
                    "use"
                };
                if let Some(args) = arguments {
                    let cleaned = Self::elixir_clean(args);
                    result.imports.push((
                        cleaned.clone(),
                        cleaned.clone(),
                        import_kind.to_string(),
                    ));
                    result.references.push(ParsedReference {
                        caller_symbol: None,
                        callee_symbol: cleaned,
                        ref_kind: import_kind.to_string(),
                        line: node.start_position().row + 1,
                    });
                }
            }
            Some(name) => {
                result.references.push(ParsedReference {
                    caller_symbol: parent.map(String::from),
                    callee_symbol: name.to_string(),
                    ref_kind: "call".to_string(),
                    line: node.start_position().row + 1,
                });
            }
            None => {}
        }
    }

    fn module_name_from_node(node: &Node, source: &[u8]) -> Option<String> {
        match node.kind() {
            "identifier" | "alias" => node.utf8_text(source).ok().map(|s| s.to_string()),
            "remote_call" => {
                let mut c = node.walk();
                for child in node.named_children(&mut c) {
                    if child.kind() == "identifier" {
                        return child.utf8_text(source).ok().map(|s| s.to_string());
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn elixir_func_name(args: &str) -> Option<String> {
        // args is like "greet(name)" from AST text
        // Extract function name before '('
        let paren = args.find('(')?;
        let func = &args[..paren];
        let func_clean = Self::elixir_clean(func);
        if func_clean.is_empty() {
            return None;
        }
        Some(func_clean)
    }

    fn elixir_signature(args: &str) -> Option<String> {
        // args is like "greet(name)" from AST text
        // Extract function name before '(' and count parameters
        let paren = args.find('(')?;
        let func = &args[..paren];
        let func_clean = Self::elixir_clean(func);
        if func_clean.is_empty() {
            return None;
        }
        // Count parameters inside parens
        let inner = &args[paren + 1..args.len().saturating_sub(1)];
        let params: Vec<&str> = inner.split(',').collect();
        let arity = params.len();
        Some(format!("{}/{}", func_clean, arity))
    }

    fn elixir_clean(s: &str) -> String {
        let mut out = String::new();
        for ch in s.chars() {
            if ch.is_alphanumeric()
                || ch == '_'
                || ch == '-'
                || ch == '.'
                || ch == '/'
                || ch == '@'
                || ch == '>'
                || ch == '<'
            {
                out.push(ch);
            } else if ch == ' ' {
                out.push(',');
            }
        }
        let out = out.trim_matches(',');
        out.to_string()
    }
}

impl SourceCodeParser for ElixirParser {
    fn language(&self) -> &str {
        "elixir"
    }

    fn file_extensions(&self) -> &[&str] {
        &["ex", "exs"]
    }

    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> {
        Self::do_parse(self, source)
    }
}
