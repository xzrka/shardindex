// Quick debug script to see TS AST nodes for import/export
use tree_sitter::Parser;

fn print_tree(node: &tree_sitter::Node, source: &str, indent: usize) {
    let prefix = "  ".repeat(indent);
    let text = node.utf8_text(source.as_bytes()).unwrap_or("");
    let display = if text.len() > 40 { &text[..40] } else { text };
    println!("{}{} [{}-{}] => {:?}", prefix, node.kind(), node.start_position().row, node.end_position().row, display.replace('\n', "\\n"));
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        print_tree(&child, source, indent + 1);
    }
}

fn main() {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    parser.set_language(&language).unwrap();

    let code = r#"
import { Foo, Bar } from 'bar';
import React from 'react';
import * as utils from '@utils';

export interface Config {
    host: string;
    port: number;
}

export default function main() {}
export { Config };

type Result<T> = { ok: true; value: T } | { ok: false; error: string };
enum Direction { Up, Down, Left, Right }

const x: number = 1;

function greet(name: string): void {}

class Animal {
    constructor(public name: string) {}
    speak(): void {}
}

namespace MyNS {
    export function helper() {}
}
"#;

    let tree = parser.parse(code, None).unwrap();
    print_tree(&tree.root_node(), code, 0);
}
