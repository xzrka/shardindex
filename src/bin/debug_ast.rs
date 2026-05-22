// Debug: print AST node kinds for failing test cases
use tree_sitter::Parser;

fn dump_tree(parser: &mut Parser, lang: tree_sitter::Language, source: &str) {
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(source, None).unwrap();
    fn walk(node: tree_sitter::Node, src: &[u8], depth: usize) {
        let indent = "  ".repeat(depth);
        let kind = node.kind();
        let txt = node
            .utf8_text(src)
            .unwrap_or("")
            .chars()
            .take(60)
            .collect::<String>();
        let is_named = node.is_named();
        let field_str = format_field(node);
        println!("{}{kind:?} {txt:?} (named={}){}", indent, is_named, field_str);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, src, depth + 1);
        }
    }
    walk(tree.root_node(), source.as_bytes(), 0);
}

fn format_field(node: tree_sitter::Node) -> String {
    let mut parts = Vec::new();
    // Use cursor to find field names by matching grammar field IDs
    // tree-sitter 0.25 removed field()/field_name() from Node
    // We'll try to detect common fields by child position
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            // Check known field positions for each grammar
            parts.push(format!("{}:{:?}", child.kind(), child.start_position()));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" [{}]", parts.join(", "))
    }
}

fn main() {
    // 1. C struct
    println!("========== C: struct ==========");
    {
        let mut p = Parser::new();
        dump_tree(
            &mut p,
            tree_sitter_c::LANGUAGE.into(),
            "struct User { int x; };",
        );
    }

    // 2. C++ class
    println!("\n========== C++: class ==========");
    {
        let mut p = Parser::new();
        dump_tree(
            &mut p,
            tree_sitter_cpp::LANGUAGE.into(),
            "class User : public QObject { int x; };",
        );
    }

    // 3. Dart function
    println!("\n========== Dart: function ==========");
    {
        let mut p = Parser::new();
        dump_tree(
            &mut p,
            tree_sitter_dart::LANGUAGE.into(),
            "void myFunction(int x) { return x; }",
        );
    }

    // 4. Elixir module
    println!("\n========== Elixir: module ==========");
    {
        let mut p = Parser::new();
        dump_tree(
            &mut p,
            tree_sitter_elixir::LANGUAGE.into(),
            "defmodule MyApp do end",
        );
    }

    // 5. Elixir function
    println!("\n========== Elixir: function ==========");
    {
        let mut p = Parser::new();
        dump_tree(
            &mut p,
            tree_sitter_elixir::LANGUAGE.into(),
            "def my_func(x) do x end",
        );
    }

    // 6. Java import
    println!("\n========== Java: import ==========");
    {
        let mut p = Parser::new();
        dump_tree(
            &mut p,
            tree_sitter_java::LANGUAGE.into(),
            "import com.example.MyApp;",
        );
    }

    // 7. Julia module
    println!("\n========== Julia: module ==========");
    {
        let mut p = Parser::new();
        dump_tree(
            &mut p,
            tree_sitter_julia::LANGUAGE.into(),
            "module MyModule end",
        );
    }

    // 8. Julia function
    println!("\n========== Julia: function ==========");
    {
        let mut p = Parser::new();
        dump_tree(
            &mut p,
            tree_sitter_julia::LANGUAGE.into(),
            "function my_func(x)\n  return x\nend",
        );
    }

    // 9. Lua require
    println!("\n========== Lua: require ==========");
    {
        let mut p = Parser::new();
        dump_tree(
            &mut p,
            tree_sitter_lua::LANGUAGE.into(),
            "local mod = require(\"mylib\")",
        );
    }

    // 10. Ruby require
    println!("\n========== Ruby: require ==========");
    {
        let mut p = Parser::new();
        dump_tree(
            &mut p,
            tree_sitter_ruby::LANGUAGE.into(),
            "require \"mylib\"",
        );
    }

    // 11. Zig struct
    println!("\n========== Zig: struct ==========");
    {
        let mut p = Parser::new();
        dump_tree(
            &mut p,
            tree_sitter_zig::LANGUAGE.into(),
            "const User = struct { name: []const u8 };",
        );
    }
}
