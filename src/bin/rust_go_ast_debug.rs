// Debug script to see Rust use + Go import AST nodes
use tree_sitter::Parser;

fn print_tree(node: &tree_sitter::Node, source: &str, indent: usize) {
    let prefix = "  ".repeat(indent);
    let text = node.utf8_text(source.as_bytes()).unwrap_or("");
    let display = if text.len() > 60 { &text[..60] } else { text };
    println!("{}{} [{}-{}] => {:?}", prefix, node.kind(), node.start_position().row, node.end_position().row, display.replace('\n', "\\n"));
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        print_tree(&child, source, indent + 1);
    }
}

fn main() {
    // --- Rust use ---
    println!("=== RUST USE ===");
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    parser.set_language(&language).unwrap();

    let code = r#"
use std::collections::HashMap;
use std::io;
use std::io::{Read, Write};
"#;
    let tree = parser.parse(code, None).unwrap();
    print_tree(&tree.root_node(), code, 0);

    // --- Go import ---
    println!("\n=== GO IMPORT ===");
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
    parser.set_language(&language).unwrap();

    let code = r#"
package main

import (
    "fmt"
    "net/http"
)

func main() {
    fmt.Println(http.StatusOK)
}
"#;
    let tree = parser.parse(code, None).unwrap();
    print_tree(&tree.root_node(), code, 0);
}
