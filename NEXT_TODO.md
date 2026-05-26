# ShardIndex — Next Tasks

Generated: 2026-05-26
Based on: `references/masterplan.md` v1.3 vs current implementation gap analysis

## Current State

- **Branch:** master, commit `b8ec17d` (Phase 4-2: Wire LanguageBackend trait + CLI read --compression flag)
- **Build:** `cargo check` 0 errors, 44 warnings (existing unused code)
- **Tests:** 236/236 passing
- **Schema:** v4 (4 migrations)
- **Lines:** ~17,765 total across 41 source files
- **Languages:** 19 (18 tree-sitter + Markdown)

---

## Phase 4 — Semantic Compression (HIGH PRIORITY)

The masterplan Phase 4 is the next major milestone. Token estimation and
adaptive compression are the foundation for all budgeted retrieval features.

### 4-1. Token estimation per symbol ✅ DONE

**Plan:** `.hermes/plans/phase4-token-estimation.md`

- [x] Create `src/token_estimation.rs`
  - `pub fn estimate_token_count(source: &str) -> usize`
  - BPE-style heuristic (~3.5 chars/token, adjusted for whitespace/comments/unicode)
  - `LanguageDensity` with 9 language-specific adjustments
- [x] Integrate into indexing pipeline (`src/indexer/mod.rs`)
  - Extract symbol body (start_line..end_line) from source
  - Call `estimate_symbol_tokens()` and store in DB
- [x] Update `src/database/mod.rs`
  - `insert_symbol()` writes `token_count` (column already exists from migration 002)
  - `SymbolRecord` struct adds `token_count: usize`
  - All DB queries (file_symbols, search_symbol, impact, ranked variants) include `token_count`
- [x] Include token info in search results (`src/search.rs`)
  - `SearchResultJson` adds `token_count` field
- [x] Unit tests for `estimate_token_count()` across languages/patterns (16 tests)

### 4-2. Adaptive compression pipeline ✅ DONE

- [x] Create `src/compression.rs`
  - `CompressionLevel` enum: `SignatureOnly`, `CriticalBranches`, `FullBody`, `TokenBudgeted(u32)`
  - `compress_symbol(source, symbol, level) -> CompressedSymbol`
  - Extract critical branches (if/else, loops, error handling, return statements)
  - Extract side effects (DB calls, network calls, mutations)
- [x] Wire into `LanguageBackend` trait
  - Add `slice_symbol()` method (masterplan §8.1)
  - Add `estimate_tokens()` method (masterplan §8.1)
- [x] CLI: `read <symbol> --compression=critical_branches`
- [x] `CompressionLevel::from_str()` with aliases (sig/s, crit/c, full/f, token_budgeted/budget, raw number)
- [x] 5 FromStr unit tests (48 total compression tests)

### 4-3. TokenBudgeted MCP responses

- [ ] Add `token_budget` parameter to MCP tool handlers
- [ ] Auto-downgrade compression level when budget exceeded
- [ ] `TokenBudgeted` response wrapper with `budget_remaining` field
- [ ] MCP `read` handler respects `token_budget` param

### 4-4. Integration tests

- [ ] Token budget enforcement tests
- [ ] Compression pipeline E2E tests
- [ ] MCP response token count verification

---

## Phase 8 — LanguageBackend Trait Completion

The `Parser` trait now has `slice_symbol()` and `estimate_tokens()` with default
implementations. May still need `is_dynamic_ref()` and explicit types.

- [ ] Add to `Parser` trait:
  - [x] `slice_symbol(&self, source, symbol, mode) -> Result<SymbolSlice>`
  - [x] `estimate_tokens(&self, snippet: &str) -> usize`
  - [ ] `is_dynamic_ref(&self, node) -> bool`
- [ ] Define `CompressionMode` enum (masterplan §8.1) — check if `CompressionLevel` covers this
- [ ] Define `SymbolSlice` struct with fields:
  - `signature`, `critical_branches`, `side_effects`, `key_assignments`, `return_statement`

---

## Phase 9 — Refactoring-Specialized APIs

Advanced APIs for safe refactoring workflows.

### 9-1. impact_deep

- [ ] Implement `impact_deep` in `src/graph/mod.rs`
  - Multi-depth transitive dependency tracing
  - Risk scoring per depth layer
  - `include_tests`, `include_dynamic` flags
- [ ] Expose via MCP + CLI

### 9-2. dead_code_verify

- [ ] Implement multi-stage dead code verification
  - Stages: static_refs, dynamic_refs, string_refs, git_history, test_refs
  - Return `safe_to_delete` + blockers list

### 9-3. cross_module_move

- [ ] Safe symbol relocation across modules
  - Auto-update imports and references
  - Dry-run mode with file modification plan
  - Unresolved reference detection

### 9-4. signature_migration_check

- [ ] Check if signature change breaks callers
  - Analyze call sites for positional/keyword arg compatibility
  - Return `compatible` + `breaking_callers` list

---

## Phase 11 — Error Handling

- [ ] Define complete error taxonomy:
  - `StaleIndex`, `SymbolNotFound`, `ParserError`, `TokenBudgetExceeded`
  - `RefIntegrityViolation`, `CircularDependency`, `CrossLanguageGap`
- [ ] Implement filesystem fallback protocol (masterplan §11.2)
  - grep/ripgrep fallback when ShardIndex fails
  - Auto-enqueue fallback files for indexing

---

## Phase 12 — Performance Benchmarks

- [ ] Create `benches/benchmarks.rs`
  - `bench_cold_index_200k_python` (target: <30s)
  - `bench_incremental_single_file` (target: <50ms)
  - `bench_impact_depth_2` (target: <5ms)
  - `bench_hash_verify` (target: <1ms)
  - `bench_search_semantic` (target: <10ms)

---

## Cross-cutting / Cleanup

### Cross-language references

- [ ] Implement `CrossLanguageResolver` (masterplan §8.3)
  - Detect shared interface names across languages
  - Create weak ref edges with `cross_language_schema` kind

### Agent skill protocol

- [ ] Complete system prompt per masterplan §5.1
  - Full ShardIndex Skill Protocol with auto-invocation rules
  - Context budget awareness section
  - Response to stale index protocol

### TypeScript file naming

- [ ] Rename `src/indexer/typecript.rs` → `src/indexer/typescript.rs`
  - Fix typo in filename (currently `typecript` instead of `typescript`)
  - Update `src/indexer/mod.rs` module declaration

### Override UI

- [ ] Optional: Simple web UI or TUI for managing reference overrides

---

## Recommended Order

1. ~~**Phase 4-1** — Token estimation (foundation for everything else)**~~ ✅
2. ~~**Phase 4-2** — Compression pipeline**~~ ✅
3. **Phase 4-3** — TokenBudgeted MCP responses ← NEXT
4. **Phase 4-4** — Integration tests
5. **Phase 8** — Complete LanguageBackend trait (is_dynamic_ref, types)
6. **Phase 9** — Refactoring APIs (impact_deep, dead_code_verify, etc.)
7. **Phase 11** — Error handling / fallback
8. **Phase 12** — Benchmarks
