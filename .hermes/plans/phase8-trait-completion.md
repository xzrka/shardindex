# Phase 8 — LanguageBackend Trait Completion

**Goal:** Complete `SourceCodeParser` trait per masterplan §8.1

## Current State

- `SourceCodeParser` trait in `src/indexer/types.rs`
- Has: `language()`, `file_extensions()`, `parse()`, `slice_symbol()`, `estimate_tokens()`
- Missing: `is_dynamic_ref()`
- `CompressionMode` enum: NOT needed — `CompressionLevel` already covers it
- `SymbolSlice` struct: NOT needed — `CompressedSymbol` already has all fields

## Tasks

### 8-1. Add `is_dynamic_ref()` to `SourceCodeParser` trait

**File:** `src/indexer/types.rs`

Add to trait:
```rust
/// Detect if a reference node represents a dynamic (runtime-resolved) reference.
///
/// Default implementation returns `false` (static reference).
/// Language-specific parsers can override for AST-aware detection.
///
/// Aligns with masterplan §8.1 `LanguageBackend::is_dynamic_ref()`.
fn is_dynamic_ref(&self, _ref_kind: &str) -> bool {
    matches!(_ref_kind, "dynamic_dispatch" | "virtual_call" | "string_ref")
}
```

**Why `&str` instead of `&Self::AstNode`?**
- Masterplan uses `&Self::AstNode` but tree-sitter `Node` is not generic over the parser
- Using `ref_kind: &str` is practical and aligns with existing `ParsedReference::is_dynamic()` logic
- Language parsers can override with AST-node-based detection if needed

### 8-2. Add `CompressionMode` as type alias

**File:** `src/compression.rs`

```rust
/// Alias for `CompressionLevel` to match masterplan §8.1 naming.
pub type CompressionMode = CompressionLevel;
```

### 8-3. Add `SymbolSlice` as type alias

**File:** `src/compression.rs`

```rust
/// Alias for `CompressedSymbol` to match masterplan §8.1 naming.
pub type SymbolSlice = CompressedSymbol;
```

### 8-4. Update `slice_symbol()` return type

**File:** `src/indexer/types.rs`

Change return type from `CompressedSymbol` to `SymbolSlice` (alias).

### 8-5. Add unit tests

- Test `is_dynamic_ref()` default behavior (static vs dynamic ref kinds)
- Test `CompressionMode` alias compiles and matches `CompressionLevel`
- Test `SymbolSlice` alias compiles and matches `CompressedSymbol`

## Files Modified

1. `src/indexer/types.rs` — add `is_dynamic_ref()` to trait, update `slice_symbol()` return type
2. `src/compression.rs` — add `CompressionMode` and `SymbolSlice` type aliases
3. `src/lib.rs` — re-export aliases if needed

## Verification

- `cargo check` — 0 errors
- `cargo test` — all existing tests pass + new tests pass
- `cargo test is_dynamic` — new tests pass
