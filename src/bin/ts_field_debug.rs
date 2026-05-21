use tree_sitter::Parser;

fn main() {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    parser.set_language(&language).unwrap();

    let code = r#"import { Foo } from 'bar';"#;
    let tree = parser.parse(code, None).unwrap();
    let root = tree.root_node();
    
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "import_statement" {
            println!("import_statement found");
            
            // Try source field
            if let Some(src) = child.child_by_field_name("source") {
                println!("source field: {:?}", src.utf8_text(code.as_bytes()).ok());
            } else {
                println!("NO source field!");
            }
            
            // Try module field  
            if let Some(mod_) = child.child_by_field_name("module") {
                println!("module field: {:?}", mod_.utf8_text(code.as_bytes()).ok());
            } else {
                println!("NO module field!");
            }
            
            // Find string node manually
            let mut s_cursor = child.walk();
            for s_child in child.children(&mut s_cursor) {
                if s_child.kind() == "string" {
                    println!("string node: {:?}", s_child.utf8_text(code.as_bytes()).ok());
                }
            }
            
            // Check named_imports
            let mut ni_cursor = child.walk();
            for ni_child in child.children(&mut ni_cursor) {
                if ni_child.kind() == "import_clause" {
                    println!("import_clause found");
                    let mut ic_cursor = ni_child.walk();
                    for ic in ni_child.children(&mut ic_cursor) {
                        println!("  ic child: {}", ic.kind());
                        if ic.kind() == "named_imports" {
                            let mut ni2 = ic.walk();
                            for ns in ic.children(&mut ni2) {
                                println!("    ns: {} => {:?}", ns.kind(), ns.utf8_text(code.as_bytes()).ok());
                            }
                        }
                    }
                }
            }
        }
    }
}
