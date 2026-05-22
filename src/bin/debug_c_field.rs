use tree_sitter::Parser;

fn main() {
    let mut parser = Parser::new();
    let language = tree_sitter_c::LANGUAGE.into();
    parser.set_language(&language).unwrap();
    
    let code = "struct User { char *name; int age; };";
    let tree = parser.parse(code, None).unwrap();
    let root = tree.root_node();
    
    // Find struct_specifier
    for child in root.named_children(&mut tree.walk()) {
        if child.kind() == "struct_specifier" {
            println!("struct_specifier children:");
            for gc in child.named_children(&mut tree.walk()) {
                println!("  kind={:?} text={:?}", gc.kind(), gc.utf8_text(code.as_bytes()).unwrap_or(""));
            }
            println!("child_by_field_name('body'): {:?}", child.child_by_field_name("body").map(|n| n.kind()));
            println!("child_by_field_name('type_identifier'): {:?}", child.child_by_field_name("type_identifier").map(|n| n.kind()));
            println!("child_by_field_name('field_declaration_list'): {:?}", child.child_by_field_name("field_declaration_list").map(|n| n.kind()));
            
            // Try all possible field names from grammar
            for field_name in &["body", "type_identifier", "declarator", "type", "field_declaration_list"] {
                if let Some(n) = child.child_by_field_name(field_name) {
                    println!("  FIELD '{}' -> kind={:?} text={:?}", field_name, n.kind(), n.utf8_text(code.as_bytes()).unwrap_or(""));
                }
            }
        }
    }
}
