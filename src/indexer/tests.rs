#[cfg(test)]
use super::*;

fn test_parse_py(code: &str) -> ParseResult {
    let mut parser = PythonParser::new().unwrap();
    parser.parse(code).unwrap()
}

fn test_parse_js(code: &str) -> ParseResult {
    let mut parser = JavaScriptParser::new().unwrap();
    parser.parse(code).unwrap()
}

// ---- Python tests ----

#[test]
fn test_parse_function() {
    let code = r#"
def hello(name: str) -> str:
    """Say hello."""
    return f"Hello, {name}!"
"#;
    let result = test_parse_py(code);
    assert_eq!(result.symbols.len(), 1);
    assert_eq!(result.symbols[0].name, "hello");
    assert_eq!(result.symbols[0].kind, SymbolKind::Function);
    assert!(result.symbols[0].docstring.is_some());
}

#[test]
fn test_parse_class() {
    let code = r#"
class Animal:
    pass

class Dog(Animal):
    def bark(self):
        pass
"#;
    let result = test_parse_py(code);
    let classes: Vec<_> = result
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Class)
        .collect();
    assert_eq!(classes.len(), 2);
    assert_eq!(classes[0].name, "Animal");
    assert_eq!(classes[1].name, "Dog");
}

#[test]
fn test_parse_imports() {
    let code = r#"
import os
from sys import path
"#;
    let result = test_parse_py(code);
    assert!(!result.imports.is_empty());
}

// ---- JavaScript tests ----

#[test]
fn test_js_function_declaration() {
    let code = r#"
function greet(name) {
return `Hello, ${name}!`;
}

async function fetchData(url) {
return fetch(url);
}
"#;
    let result = test_parse_js(code);
    let funcs: Vec<_> = result
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .collect();
    assert_eq!(funcs.len(), 2);
    assert_eq!(funcs[0].name, "greet");
    assert_eq!(funcs[1].name, "fetchData");
    assert!(funcs[1].signature.as_ref().unwrap().contains("async"));
}

#[test]
fn test_js_class_and_methods() {
    let code = r#"
class Animal {
constructor(name) {
    this.name = name;
}

speak() {
    return "sound";
}
}

class Dog extends Animal {
bark() {
    return "woof";
}
}
"#;
    let result = test_parse_js(code);
    let classes: Vec<_> = result
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Class)
        .collect();
    let methods: Vec<_> = result
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .collect();

    assert_eq!(classes.len(), 2);
    assert_eq!(classes[0].name, "Animal");
    assert_eq!(classes[1].name, "Dog");

    // Dog extends Animal
    let inherits: Vec<_> = result
        .references
        .iter()
        .filter(|r| r.ref_kind == "inherit")
        .collect();
    assert_eq!(inherits.len(), 1);
    assert_eq!(inherits[0].callee_symbol, "Animal");

    // Methods: constructor, speak (Animal) + bark (Dog)
    let method_names: Vec<_> = methods.iter().map(|m| m.name.as_str()).collect();
    assert!(methods.len() >= 3);
    assert!(method_names.contains(&"constructor"));
    assert!(method_names.contains(&"speak"));
    assert!(method_names.contains(&"bark"));
}

#[test]
fn test_js_imports() {
    let code = r#"
import React from 'react';
import { useState, useEffect } from 'react';
"#;
    let result = test_parse_js(code);
    assert!(!result.imports.is_empty());
    // Should have React (default) + useState, useEffect (named)
    assert!(result.imports.len() >= 2);

    let import_symbols: Vec<_> = result
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Import)
        .collect();
    assert!(!import_symbols.is_empty());
}

#[test]
fn test_js_arrow_function() {
    let code = r#"
const add = (a, b) => a + b;
const multiply = (x, y) => { return x * y; };
"#;
    let result = test_parse_js(code);
    let vars: Vec<_> = result
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .collect();
    assert_eq!(vars.len(), 2);
    assert_eq!(vars[0].name, "add");
    assert_eq!(vars[1].name, "multiply");
}

#[test]
fn test_js_exports() {
    let code = r#"
function greet() {}
export default greet;
export { greet };
"#;
    let result = test_parse_js(code);
    let exports: Vec<_> = result
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Export)
        .collect();
    assert!(!exports.is_empty());
}

#[test]
fn test_js_call_references() {
    let code = r#"
const result = fetch(url);
response.json();
"#;
    let result = test_parse_js(code);
    let calls: Vec<_> = result
        .references
        .iter()
        .filter(|r| r.ref_kind == "call")
        .collect();
    assert!(!calls.is_empty());
}

// ---- Trait tests ----

#[test]
fn test_parser_trait() {
    let py: Box<dyn SourceCodeParser> = Box::new(PythonParser::new().unwrap());
    assert_eq!(py.language(), "python");
    assert!(py.file_extensions().contains(&"py"));

    let js: Box<dyn SourceCodeParser> = Box::new(JavaScriptParser::new().unwrap());
    assert_eq!(js.language(), "javascript");
    assert!(js.file_extensions().contains(&"js"));

    let rust_p: Box<dyn SourceCodeParser> = Box::new(RustParser::new().unwrap());
    assert_eq!(rust_p.language(), "rust");
    assert!(rust_p.file_extensions().contains(&"rs"));

    let ts: Box<dyn SourceCodeParser> = Box::new(TypeScriptParser::new().unwrap());
    assert_eq!(ts.language(), "typescript");
    assert!(ts.file_extensions().contains(&"ts"));

    let go: Box<dyn SourceCodeParser> = Box::new(GoParser::new().unwrap());
    assert_eq!(go.language(), "go");
    assert!(go.file_extensions().contains(&"go"));
}

#[test]
fn test_rust_function() {
    let mut parser = RustParser::new().unwrap();
    let code = r#"
fn hello(name: &str) -> String {
format!("Hello, {}!", name)
}
"#;
    let result = parser.parse(code).unwrap();
    assert_eq!(result.symbols.len(), 1);
    assert_eq!(result.symbols[0].name, "hello");
    assert_eq!(result.symbols[0].kind, SymbolKind::Function);
}

#[test]
fn test_rust_struct() {
    let mut parser = RustParser::new().unwrap();
    let code = r#"
struct User {
name: String,
age: u32,
}
"#;
    let result = parser.parse(code).unwrap();
    assert_eq!(result.symbols.len(), 1);
    assert_eq!(result.symbols[0].name, "User");
}

#[test]
fn test_rust_enum() {
    let mut parser = RustParser::new().unwrap();
    let code = r#"
enum Status {
Ok,
Err(String),
}
"#;
    let result = parser.parse(code).unwrap();
    // enum itself + 2 variants
    assert!(result.symbols.len() >= 3);
    assert!(result.symbols.iter().any(|s| s.name == "Status"));
}

#[test]
fn test_rust_trait_and_impl() {
    let mut parser = RustParser::new().unwrap();
    let code = r#"
trait Drawable {
fn draw(&self);
}

struct Circle;

impl Drawable for Circle {}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "Drawable"));
    assert!(result.symbols.iter().any(|s| s.name == "Circle"));
}

#[test]
fn test_rust_use_import() {
    let mut parser = RustParser::new().unwrap();
    let code = r#"
use std::collections::HashMap;
use std::io;
"#;
    let result = parser.parse(code).unwrap();
    assert!(!result.imports.is_empty());
}

#[test]
fn test_ts_class() {
    let mut parser = TypeScriptParser::new().unwrap();
    let code = r#"
class Animal {
constructor(public name: string) {}
speak(): void {
    console.log(this.name);
}
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "Animal"));
}

#[test]
fn test_ts_interface() {
    let mut parser = TypeScriptParser::new().unwrap();
    let code = r#"
interface Config {
host: string;
port: number;
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "Config"));
}

#[test]
fn test_ts_type_alias() {
    let mut parser = TypeScriptParser::new().unwrap();
    let code = r#"
type Result<T> = { ok: true; value: T } | { ok: false; error: string };
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "Result"));
}

#[test]
fn test_ts_import() {
    let mut parser = TypeScriptParser::new().unwrap();
    let code = r#"
import { Foo, Bar } from 'bar';
import React from 'react';
import * as utils from '@utils';
"#;
    let result = parser.parse(code).unwrap();
    assert!(!result.imports.is_empty());
    assert!(result.imports.iter().any(|(m, _, _)| m == "bar"));
    assert!(result.imports.iter().any(|(m, _, _)| m == "react"));
}

#[test]
fn test_ts_export() {
    let mut parser = TypeScriptParser::new().unwrap();
    let code = r#"
export { Config };
export default function main() {}
"#;
    let result = parser.parse(code).unwrap();
    let exports: Vec<&str> = result.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Export)
        .map(|s| s.name.as_str())
        .collect();
    assert!(exports.contains(&"Config"));
}

#[test]
fn test_ts_function() {
    let mut parser = TypeScriptParser::new().unwrap();
    let code = r#"
function greet(name: string): void {}
async function fetchData(): Promise<string> { return ""; }
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "greet"));
    assert!(result.symbols.iter().any(|s| s.name == "fetchData"));
}

#[test]
fn test_ts_enum() {
    let mut parser = TypeScriptParser::new().unwrap();
    let code = r#"
enum Direction { Up, Down, Left, Right }
"#;
    let result = parser.parse(code).unwrap();
    let enum_sym = result
        .symbols
        .iter()
        .find(|s| s.kind == SymbolKind::Enum)
        .expect("enum symbol not found");
    assert_eq!(enum_sym.name, "Direction");
    assert_eq!(enum_sym.kind, SymbolKind::Enum);
}

#[test]
fn test_ts_generic_type() {
    let mut parser = TypeScriptParser::new().unwrap();
    let code = r#"
type Result<T> = { ok: true; value: T } | { ok: false; error: string };
"#;
    let result = parser.parse(code).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "Result").unwrap();
    assert!(sym.signature.as_deref() == Some("type Result"));
}

#[test]
fn test_go_function() {
    let mut parser = GoParser::new().unwrap();
    let code = r#"
package main

func Hello(name string) string {
return "Hello, " + name
}
"#;
    let result = parser.parse(code).unwrap();
    assert_eq!(result.symbols.len(), 1);
    assert_eq!(result.symbols[0].name, "Hello");
}

#[test]
fn test_go_struct() {
    let mut parser = GoParser::new().unwrap();
    let code = r#"
package main

type User struct {
Name string
Age  int
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "User"));
}

#[test]
fn test_go_method() {
    let mut parser = GoParser::new().unwrap();
    let code = r#"
package main

type Counter struct {
Value int
}

func (c *Counter) Increment() {
c.Value++
}
"#;
    let result = parser.parse(code).unwrap();
    let method = result.symbols.iter().find(|s| s.name == "Increment").unwrap();
    assert_eq!(method.kind, SymbolKind::Method);
    assert_eq!(method.parent.as_deref(), Some("Counter"));
}

#[test]
fn test_go_import() {
    let mut parser = GoParser::new().unwrap();
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
    let result = parser.parse(code).unwrap();
    assert!(!result.imports.is_empty());
    assert!(result.imports.iter().any(|(m, _, _)| m == "fmt"));
}

// ---- Ruby tests ----

#[test]
fn test_ruby_method() {
    let mut parser = RubyParser::new().unwrap();
    let code = r#"
def greet(name)
  "Hello, #{name}!"
end

def add(a, b)
  a + b
end
"#;
    let result = parser.parse(code).unwrap();
    let funcs: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Method).collect();
    assert_eq!(funcs.len(), 2);
    assert_eq!(funcs[0].name, "greet");
    assert_eq!(funcs[1].name, "add");
}

#[test]
fn test_ruby_class() {
    let mut parser = RubyParser::new().unwrap();
    let code = r#"
class Animal
  def speak
    "sound"
  end
end

class Dog < Animal
  def bark
    "woof"
  end
end
"#;
    let result = parser.parse(code).unwrap();
    let classes: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Class).collect();
    assert_eq!(classes.len(), 2);
    assert_eq!(classes[0].name, "Animal");
    assert_eq!(classes[1].name, "Dog");
}

#[test]
fn test_ruby_require() {
    let mut parser = RubyParser::new().unwrap();
    let code = r#"
require 'json'
require_relative 'utils'
"#;
    let result = parser.parse(code).unwrap();
    assert!(!result.imports.is_empty());
}

// ---- Java tests ----

#[test]
fn test_java_class() {
    let mut parser = JavaParser::new().unwrap();
    let code = r#"
public class User {
    private String name;
    private int age;
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "User" && s.kind == SymbolKind::Class));
}

#[test]
fn test_java_method() {
    let mut parser = JavaParser::new().unwrap();
    let code = r#"
public class Greeter {
    public String greet(String name) {
        return "Hello, " + name;
    }
}
"#;
    let result = parser.parse(code).unwrap();
    let method = result.symbols.iter().find(|s| s.name == "greet").unwrap();
    assert_eq!(method.kind, SymbolKind::Method);
    assert_eq!(method.parent.as_deref(), Some("Greeter"));
}

#[test]
fn test_java_import() {
    let mut parser = JavaParser::new().unwrap();
    let code = r#"
import java.util.List;
import java.util.Map;
"#;
    let result = parser.parse(code).unwrap();
    assert!(!result.imports.is_empty());
}

// ---- PHP tests ----

#[test]
fn test_php_function() {
    let mut parser = PhpParser::new().unwrap();
    let code = r#"
<?php
function greet($name) {
    return "Hello, " . $name;
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
}

#[test]
fn test_php_class() {
    let mut parser = PhpParser::new().unwrap();
    let code = r#"
<?php
class User {
    public function getName() {
        return $this->name;
    }
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "User" && s.kind == SymbolKind::Class));
}

// ---- Julia tests ----

#[test]
fn test_julia_function() {
    let mut parser = JuliaParser::new().unwrap();
    let code = r#"
function greet(name::String)::String
    "Hello, $(name)!"
end
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
}

#[test]
fn test_julia_module() {
    let mut parser = JuliaParser::new().unwrap();
    let code = r#"
module MyModule
    function foo() end
end
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "MyModule"));
}

// ---- Lua tests ----

#[test]
fn test_lua_function() {
    let mut parser = LuaParser::new().unwrap();
    let code = r#"
function greet(name)
    return "Hello, " .. name
end

local function add(a, b)
    return a + b
end
"#;
    let result = parser.parse(code).unwrap();
    let funcs: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Function).collect();
    assert_eq!(funcs.len(), 2);
    assert_eq!(funcs[0].name, "greet");
    assert_eq!(funcs[1].name, "add");
}

#[test]
fn test_lua_require() {
    let mut parser = LuaParser::new().unwrap();
    let code = r#"
local http = require("http")
local json = require("json")
"#;
    let result = parser.parse(code).unwrap();
    assert!(!result.imports.is_empty());
}

// ---- Swift tests ----

#[test]
fn test_swift_class() {
    let mut parser = SwiftParser::new().unwrap();
    let code = r#"
class User {
    var name: String
    init(name: String) {
        self.name = name
    }
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "User" && s.kind == SymbolKind::Class));
}

#[test]
fn test_swift_function() {
    let mut parser = SwiftParser::new().unwrap();
    let code = r#"
func greet(name: String) -> String {
    return "Hello, " + name
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
}

// ---- Zig tests ----

#[test]
fn test_zig_function() {
    let mut parser = ZigParser::new().unwrap();
    let code = r#"
pub fn greet(name: []const u8) []const u8 {
    return "Hello!";
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
}

#[test]
fn test_zig_struct() {
    let mut parser = ZigParser::new().unwrap();
    let code = r#"
pub const User = struct {
    name: []const u8,
    age: u32,
};
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "User"));
}

// ---- Scala tests ----

#[test]
fn test_scala_class() {
    let mut parser = ScalaParser::new().unwrap();
    let code = r#"
class User(val name: String, val age: Int)
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "User"));
}

#[test]
fn test_scala_function() {
    let mut parser = ScalaParser::new().unwrap();
    let code = r#"
def greet(name: String): String = {
    s"Hello, $name!"
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
}

// ---- Elixir tests ----

#[test]
fn test_elixir_module() {
    let mut parser = ElixirParser::new().unwrap();
    let code = r#"
defmodule MyApp.Math do
  def add(a, b) do
    a + b
  end
end
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "MyApp.Math"));
}

#[test]
fn test_elixir_function() {
    let mut parser = ElixirParser::new().unwrap();
    let code = r#"
defmodule MyApp do
  def greet(name) do
    "Hello, #{name}!"
  end
end
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
}

// ---- Dart tests ----

#[test]
fn test_dart_function() {
    let mut parser = DartParser::new().unwrap();
    let code = r#"
String greet(String name) {
  return 'Hello, $name!';
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
}

#[test]
fn test_dart_class() {
    let mut parser = DartParser::new().unwrap();
    let code = r#"
class User {
  String name;
  int age;

  User(this.name, this.age);
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "User" && s.kind == SymbolKind::Class));
}

// ---- Haskell tests ----

#[test]
fn test_haskell_function() {
    let mut parser = HaskellParser::new().unwrap();
    let code = r#"
greet :: String -> String
greet name = "Hello, " ++ name
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
}

#[test]
fn test_haskell_datatype() {
    let mut parser = HaskellParser::new().unwrap();
    let code = r#"
data User = User { name :: String, age :: Int }
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "User"));
}

// ---- C tests ----

#[test]
fn test_c_function() {
    let mut parser = CParser::new().unwrap();
    let code = r#"
int greet(const char *name) {
    printf("Hello, %s!\n", name);
    return 0;
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
}

#[test]
fn test_c_struct() {
    let mut parser = CParser::new().unwrap();
    let code = r#"
struct User {
    char *name;
    int age;
};
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "User"));
}

// ---- C++ tests ----

#[test]
fn test_cpp_class() {
    let mut parser = CppParser::new().unwrap();
    let code = r#"
class User {
public:
    std::string name;
    int age;
    void greet();
};
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "User" && s.kind == SymbolKind::Class));
}

#[test]
fn test_cpp_function() {
    let mut parser = CppParser::new().unwrap();
    let code = r#"
void greet(const std::string& name) {
    std::cout << "Hello, " << name << "!" << std::endl;
}
"#;
    let result = parser.parse(code).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
}

// ---- Extended parser trait test ----

#[test]
fn test_all_parser_traits() {
    // Verify all 18 parsers implement SourceCodeParser
    let parsers: Vec<Box<dyn SourceCodeParser>> = vec![
        Box::new(PythonParser::new().unwrap()),
        Box::new(JavaScriptParser::new().unwrap()),
        Box::new(RustParser::new().unwrap()),
        Box::new(TypeScriptParser::new().unwrap()),
        Box::new(GoParser::new().unwrap()),
        Box::new(RubyParser::new().unwrap()),
        Box::new(JavaParser::new().unwrap()),
        Box::new(PhpParser::new().unwrap()),
        Box::new(JuliaParser::new().unwrap()),
        Box::new(LuaParser::new().unwrap()),
        Box::new(SwiftParser::new().unwrap()),
        Box::new(ZigParser::new().unwrap()),
        Box::new(ScalaParser::new().unwrap()),
        Box::new(ElixirParser::new().unwrap()),
        Box::new(DartParser::new().unwrap()),
        Box::new(HaskellParser::new().unwrap()),
        Box::new(CParser::new().unwrap()),
        Box::new(CppParser::new().unwrap()),
    ];

    assert_eq!(parsers.len(), 18);

    for p in &parsers {
        // Each parser should have at least one file extension
        assert!(!p.file_extensions().is_empty());
        // Each parser should report its language
        assert!(!p.language().is_empty());
    }
}

// ---- Language detection tests ----

#[test]
fn test_language_from_extension() {
    assert_eq!(Language::from_extension("file.py"), Some(Language::Python));
    assert_eq!(Language::from_extension("file.js"), Some(Language::JavaScript));
    assert_eq!(Language::from_extension("file.rs"), Some(Language::Rust));
    assert_eq!(Language::from_extension("file.ts"), Some(Language::TypeScript));
    assert_eq!(Language::from_extension("file.go"), Some(Language::Go));
    assert_eq!(Language::from_extension("file.rb"), Some(Language::Ruby));
    assert_eq!(Language::from_extension("file.java"), Some(Language::Java));
    assert_eq!(Language::from_extension("file.php"), Some(Language::Php));
    assert_eq!(Language::from_extension("file.jl"), Some(Language::Julia));
    assert_eq!(Language::from_extension("file.lua"), Some(Language::Lua));
    assert_eq!(Language::from_extension("file.swift"), Some(Language::Swift));
    assert_eq!(Language::from_extension("file.zig"), Some(Language::Zig));
    assert_eq!(Language::from_extension("file.scala"), Some(Language::Scala));
    assert_eq!(Language::from_extension("file.ex"), Some(Language::Elixir));
    assert_eq!(Language::from_extension("file.exs"), Some(Language::Elixir));
    assert_eq!(Language::from_extension("file.dart"), Some(Language::Dart));
    assert_eq!(Language::from_extension("file.hs"), Some(Language::Haskell));
    assert_eq!(Language::from_extension("file.c"), Some(Language::C));
    assert_eq!(Language::from_extension("file.cpp"), Some(Language::Cpp));
    assert_eq!(Language::from_extension("file.hpp"), Some(Language::Cpp));
    assert_eq!(Language::from_extension("file.unknown"), None);
}

#[test]
fn test_all_extensions() {
    let exts = Language::all_extensions();
    assert!(exts.len() >= 25); // At least 25 unique extensions across 18 languages
    assert!(exts.iter().any(|(ext, _)| *ext == "py"));
    assert!(exts.iter().any(|(ext, _)| *ext == "rs"));
    assert!(exts.iter().any(|(ext, _)| *ext == "zig"));
    assert!(exts.iter().any(|(ext, _)| *ext == "exs"));
}
