# ShardIndex — Next Implementation Plan

> **Created:** 2026-05-28
> **Updated:** 2026-05-28 (C# added)
> **Target:** New language parsers (7 languages)
> **Status:** ✅ Complete — 7 languages + C# (26 total)

---

## Overview

Add tree-sitter parsers for 7 new languages to expand ShardIndex multi-language coverage from 12 to 19 languages.

### Priority Order

| # | Language | 난이도 | 핵심 가치 | 비고 |
|---|----------|--------|-----------|------|
| 1 | JSX/TSX | 낮음 | React 생태계 완전 커버 | 기존 JS/TS 파서로 거의 공짜 |
| 2 | Kotlin | 중간 | 안드로이드+서버+KMP | tree-sitter-kotlin-ng 사용 |
| 3 | GraphQL SDL | 낮음 | 스키마 변경 영향도 분석 | tree-sitter-graphql 사용 |
| 4 | SQL | 낮음 | 마이그레이션 Breaking Change | 테이블/뷰/프로시저만 (스코프 축소) |
| 5 | CSS/SCSS | 낮음 | 데드코드 검출 정도 | 2차 구현으로 강등 |
| 6 | Vue SFC | 높음 | Vue 생태계 | 사용자 수요 검증 후 결정 |
| 7 | Bash | 낮음 | CI/CD 분석 | DevOps 타겟 명확해지면 |

### Implementation Status

| # | Language | Status | Commit |
|---|----------|--------|--------|
| 1 | JSX/TSX | ✅ 기존 JS/TS 파서로 자동 지원 | — |
| 8 | C# | ✅ tree-sitter-c-shsharp | 83b9fce |
| 2 | Kotlin | ✅ tree-sitter-kotlin-ng | a17a2a1 |
| 3 | GraphQL SDL | ✅ tree-sitter-graphql | a17a2a1 |
| 4 | SQL | ✅ tree-sitter-sequel | a17a2a1 |
| 5 | CSS/SCSS | ✅ tree-sitter-css (nesting, scss_embedded) | a17a2a1 |
| 6 | Vue SFC | ✅ tree-sitter-vue-updated (extract_script_content) | a17a2a1 |
| 7 | Bash | ✅ tree-sitter-bash | a17a2a1 |

---

## Language-by-Language Plan

### 1. JSX/TSX (Low effort — verify existing support)

**Current state:**
- `Language::from_extension()` already maps `jsx` → `JavaScript`, `tsx` → `TypeScript`
- `tree-sitter-javascript` 0.23 handles JSX syntax natively
- `tree-sitter-typescript` 0.23 handles TSX syntax natively

**Action items:**
- [ ] Add JSX/TSX test cases to `tests.rs` to verify parsing works
- [ ] Test: function components, class components, hooks, JSX expressions
- [ ] If parsing gaps found, consider upgrading to `tree-sitter-jsx` or `tree-sitter-tsx`

**Estimated effort:** 30 min (test-only)

---

### 2. Kotlin (Medium effort)

**Crate:** `tree-sitter-kotlin-ng` 1.1.0 (actively maintained, better than `tree-sitter-kotlin` 0.3.8)

**File extensions:** `.kt`, `.kts`

**Language enum:** `Language::Kotlin`

**Parser file:** `src/indexer/kotlin.rs`

**Symbols to extract:**
- `class_declaration` → SymbolKind::Class
- `function_declaration` → SymbolKind::Function
- `property` (top-level) → SymbolKind::Variable
- `companion_object` → SymbolKind::Class (nested)
- `type_alias` → SymbolKind::TypeAlias
- `import_list` / `import_declaration` → imports
- `constructor_declaration` → methods within class

**AST structure (tree-sitter-kotlin-ng):**
```
class_declaration
  modifiers (optional)
  name (field)
  type_parameters (optional)
  primary_constructor (optional)
  delegation_providers (optional) — for inheritance refs
  type_constraints (optional)
  body (field) — class_body
    class_member_list
      function_declaration
        name (field)
        parameters (field)
        body (field)
      property
        name (field)
        type (field)
        initializer (field)
```

**Implementation steps:**
1. Add `tree-sitter-kotlin-ng = "1.1.0"` to `Cargo.toml`
2. Create `src/indexer/kotlin.rs` following existing parser pattern
3. Register in `mod.rs`: `mod kotlin;` + `pub use kotlin::KotlinParser;`
4. Add `Language::Kotlin` variant + extension mapping + `create_parser()`
5. Add tests to `tests.rs`

**Estimated effort:** 1–2 hours

---

### 3. SQL (Medium effort)

**Crate:** `tree-sitter-sequel` 0.3.11 (most mature, supports multiple dialects)
- Alternative: `devgen-tree-sitter-sql` 0.21.0 (newer, may have better tree-sitter 0.25 compat)

**File extensions:** `.sql`

**Language enum:** `Language::Sql`

**Parser file:** `src/indexer/sql.rs`

**Symbols to extract:**
- `create_table_statement` → SymbolKind::Class (table as "class")
- `create_function_statement` → SymbolKind::Function
- `create_procedure_statement` → SymbolKind::Function
- `create_view_statement` → SymbolKind::TypeAlias
- `create_trigger_statement` → SymbolKind::Function
- `table_factor` / `joined_table` → references (table refs)
- `column_definition` → SymbolKind::Variable (within table)

**AST structure (tree-sitter-sequel):**
```
create_table_statement
  table_name: object_reference
    identifier
  column_list
    column_definition
      column_name: identifier
      data_type: ...
      constraint: ...
```

**Implementation steps:**
1. Add `tree-sitter-sequel = "0.3.11"` to `Cargo.toml`
2. Create `src/indexer/sql.rs`
3. Register in `mod.rs`
4. Add `Language::Sql` variant + extension mapping + `create_parser()`
5. Add tests to `tests.rs`

**Estimated effort:** 1–2 hours

---

### 4. GraphQL SDL (Medium effort)

**Crate:** `tree-sitter-graphql` 0.1.0

**File extensions:** `.graphql`, `.gql`

**Language enum:** `Language::Graphql`

**Parser file:** `src/indexer/graphql.rs`

**Symbols to extract:**
- `type_definition` → SymbolKind::Class
- `interface_definition` → SymbolKind::Class (or new kind if needed)
- `enum_type_definition` → SymbolKind::Enum
- `input_type_definition` → SymbolKind::Class
- `schema_definition` → SymbolKind::Module
- `scalar_type_definition` → SymbolKind::TypeAlias
- `field_definition` → SymbolKind::Method (within type)
- `implements_interfaces` → references (inheritance)

**AST structure (tree-sitter-graphql):**
```
type_definition
  name: name
  implements_interfaces (optional)
    named_type
      name: name
  fields_definition
    field_definition
      name: name
      type: ...
```

**Implementation steps:**
1. Add `tree-sitter-graphql = "0.1.0"` to `Cargo.toml`
2. Create `src/indexer/graphql.rs`
3. Register in `mod.rs`
4. Add `Language::Graphql` variant + extension mapping + `create_parser()`
5. Add tests to `tests.rs`

**Estimated effort:** 1 hour

---

### 5. Vue SFC (High effort — hybrid parser)

**Crate:** `tree-sitter-vue-updated` 0.1.0 (or `tree-sitter-vue-next` 0.1.0)

**File extensions:** `.vue`

**Language enum:** `Language::Vue`

**Parser file:** `src/indexer/vue.rs`

**Challenge:** Vue SFC has 3 blocks (`<template>`, `<script>`, `<style>`). Need to:
1. Parse the SFC wrapper with tree-sitter-vue
2. Delegate `<script>` block to TypeScript or JavaScript parser
3. Delegate `<style>` block to CSS parser (when available)
4. Extract `<template>` block structure (component references)

**Symbols to extract:**
- `script` block → delegate to TS/JS parser for components, functions, methods
- `template` block → extract component references as symbols
- `style` block → extract CSS rules (when CSS parser available)
- Component name from `<script>` export → SymbolKind::Class

**AST structure (tree-sitter-vue):**
```
document
  script
    script_content
      (JavaScript/TypeScript AST here)
  template
    start_tag
    (HTML AST here)
    end_tag
  style
    style_content
      (CSS AST here)
```

**Implementation approach:**
- Phase A: Basic Vue parser that extracts `<script>` content and delegates to TS/JS
- Phase B: Add template component reference extraction
- Phase C: Add style block parsing (depends on CSS parser)

**Implementation steps:**
1. Add `tree-sitter-vue-updated = "0.1.0"` to `Cargo.toml`
2. Create `src/indexer/vue.rs` with hybrid parsing
3. Register in `mod.rs`
4. Add `Language::Vue` variant + extension mapping + `create_parser()`
5. Add tests to `tests.rs`

**Estimated effort:** 2–3 hours

---

### 6. CSS/SCSS (Medium effort)

**Crate:** `tree-sitter-css` 0.25.0 (handles CSS + SCSS natively)

**File extensions:** `.css`, `.scss`, `.sass`

**Language enum:** `Language::Css`

**Parser file:** `src/indexer/css.rs`

**Symbols to extract:**
- `rule_set` → SymbolKind::Section (CSS rule as "section")
- `keyframe_block` → SymbolKind::Section
- `media_feature` / `media_query` → SymbolKind::Section
- `custom_property` → SymbolKind::Variable (CSS variables: `--name`)
- `keyframes` → SymbolKind::Function (`@keyframes name`)
- `mixin` (SCSS) → SymbolKind::Function
- `function` (SCSS) → SymbolKind::Function
- `at_rule` → SymbolKind::Section

**AST structure (tree-sitter-css):**
```
stylesheet
  rule_set
    selector_list
      generic_selector
        simple_selector
          type_selector: identifier
    declaration_block
      declaration
        property: property_name
        value: ...
  at_rule
    at_keyword: @keyframes
    name: identifier
    block
      keyframe_block
```

**Implementation steps:**
1. Add `tree-sitter-css = "0.25.0"` to `Cargo.toml`
2. Create `src/indexer/css.rs`
3. Register in `mod.rs`
4. Add `Language::Css` variant + extension mapping + `create_parser()`
5. Add tests to `tests.rs`

**Estimated effort:** 1–2 hours

---

### 7. Bash (Medium effort)

**Crate:** `tree-sitter-bash` 0.25.1

**File extensions:** `.sh`, `.bash`, `.zsh`

**Language enum:** `Language::Bash`

**Parser file:** `src/indexer/bash.rs`

**Symbols to extract:**
- `function_definition` → SymbolKind::Function
- `variable_assignment` (global) → SymbolKind::Variable
- `case_item` → SymbolKind::Section (case branches)
- `heredoc` → SymbolKind::CodeBlock
- Command calls → references

**AST structure (tree-sitter-bash):**
```
function_definition
  name: word
  body: brace_group
    command
      name: word
variable_assignment
  name: word
  value: ...
```

**Implementation steps:**
1. Add `tree-sitter-bash = "0.25.1"` to `Cargo.toml`
2. Create `src/indexer/bash.rs`
3. Register in `mod.rs`
4. Add `Language::Bash` variant + extension mapping + `create_parser()`
5. Add tests to `tests.rs`

**Estimated effort:** 1 hour

---

## Implementation Order

| Phase | Language | Effort | Dependencies | Notes |
|-------|----------|--------|-------------|-------|
| 1 | JSX/TSX (tests) | 30m | None | ✅ 기존 JS/TS 파서 검증 완료 |
| 2 | Kotlin | 1–2h | None | ✅ tree-sitter-kotlin-ng |
| 3 | GraphQL SDL | 1h | None | ✅ tree-sitter-graphql |
| 4 | SQL | 1–2h | None | ✅ 테이블/뷰/프로시저만 |
| 5 | CSS/SCSS | 1–2h | None | ✅ nesting, scss_embedded 지원 |
| 6 | Vue SFC | 2–3h | CSS parser | ✅ extract_script_content 도입 |
| 7 | Bash | 1h | None | ✅ DevOps 타겟 |

**Total estimated effort:** 8–12 hours
**Actual effort:** ~10 hours

---

## Shared Changes (all languages)

Each language requires these modifications:

### 1. `Cargo.toml` — Add dependency
```toml
tree-sitter-{lang} = "x.y.z"
```

### 2. `src/indexer/mod.rs` — Register language
```rust
mod {lang};
pub use {lang}::{Lang}Parser;

// Language enum variant
Language::{Lang},

// from_extension mapping
"ext" => Some(Language::{Lang}),

// as_str mapping
Language::{Lang} => "lang",

// extensions mapping
Language::{Lang} => &["ext"],

// create_parser mapping
Language::{Lang} => Ok(Box::new({Lang}Parser::new()?)),

// all_extensions mapping
("ext", Language::{Lang}),
```

### 3. `src/indexer/{lang}.rs` — Parser implementation
Follow the established pattern:
```rust
use crate::indexer::types::*;
use anyhow::Context;
use tree_sitter::Node;

pub struct {Lang}Parser;

impl {Lang}Parser {
    pub fn new() -> Result<Self, anyhow::Error> { ... }
    fn do_parse(&self, source: &str) -> Result<ParseResult, anyhow::Error> { ... }
    fn walk_node(...) { ... }
    // extraction helpers...
}

impl SourceCodeParser for {Lang}Parser {
    fn language(&self) -> &str { "lang" }
    fn file_extensions(&self) -> &[&str] { &["ext"] }
    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error> { ... }
}
```

### 4. `src/indexer/tests.rs` — Add tests
```rust
#[test]
fn test_{lang}_function() { ... }
#[test]
fn test_{lang}_class() { ... }
#[test]
fn test_{lang}_import() { ... }
```

---

## Potential Issues

1. **tree-sitter version compatibility** — Some language crates may use older tree-sitter APIs. If `cargo check` fails with ABI issues, try newer versions or pin to compatible versions.

2. **tree-sitter-graphql 0.1.0** — Very early version. May have incomplete grammar or API issues. Monitor for `tree-sitter-graphql` updates.

3. **Vue SFC delegation** — The Vue parser needs to extract script content and re-parse it with the TS/JS parser. This requires careful handling of line number offsets.

4. **CSS/SCSS SymbolKind** — CSS doesn't have traditional "functions" or "classes". Using `SymbolKind::Section` for rule sets is a pragmatic choice, but may feel semantically odd. Consider adding CSS-specific symbol kinds later.

5. **SQL dialects** — tree-sitter-sequel supports multiple SQL dialects. Start with standard SQL, add dialect-specific features later.

---

## Quality Gates

For each new language parser:
- [x] `cargo check` passes
- [x] `cargo test` passes (605 tests — ≥3 test cases per language)
- [x] Parser extracts at least: functions, types/classes, imports
- [x] `Language::create_parser()` returns `Ok`
- [x] `Language::from_extension()` maps all extensions correctly
- [x] `all_extensions()` includes all new extensions
- [x] Masterplan updated with new language count
