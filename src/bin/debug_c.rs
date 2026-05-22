use tree_sitter::Parser;

fn main() {
    let mut parser = Parser::new();
    let language = tree_sitter_c::LANGUAGE.into();
    parser.set_language(&language).unwrap();
    
    let code = r#"
struct User {
    char *name;
    int age;
};
"#;
    let tree = parser.parse(code, None).unwrap();
    walk(tree.root_node(), code.as_bytes(), 0);
}

fn walk(node: tree_sitter::Node, src: &[u8], depth: usize) {
    let indent = "  ".repeat(depth);
    let kind = node.kind();
    let txt = node.utf8_text(src).unwrap_or("").chars().take(40).collect::<String>();
    let named = node.is_named();
    println!("{}{:?} {:?} (named={})", indent, kind, txt, named);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, src, depth + 1);
    }
}
