# ShardIndex — Next Tasks

Generated: 2026-05-26
Updated: 2026-05-27 (Phase 11 error handling + fallback protocol completed)
Based on: `references/masterplan.md` v1.3 vs current implementation gap analysis

## Current State

- **Branch:** master
- **Build:** `cargo check` 0 errors
- **Tests:** 545/545 passing (263 unit lib + 263 unit bin + 17 integration + 2 doctest)
- **Schema:** v4 (4 migrations)
- **Languages:** 19 (18 tree-sitter + Markdown)

---

## Phase 4 — Semantic Compression ✅ COMPLETE

All 4 sub-tasks done. Token estimation, adaptive compression, TokenBudgeted MCP responses, and integration tests are fully implemented and tested.

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

### 4-3. TokenBudgeted MCP responses ✅ DONE

- [x] Create `src/token_budget.rs` (503 lines)
  - `TokenBudget` struct with `budget_requested`, `tokens_used`, `budget_remaining`, `compression_applied`
  - 4-stage compression strategy:
    1. `StripDocstrings` → 2. `StripSignatures` → 3. `RemoveDetails` → 4. `TruncateResults`
  - `enforce_budget(response, budget) -> TokenBudgetedResponse` — iterative compression with re-estimation
  - `ok_with_budget()` helper method on MCP responses
  - `truncate_results()` with count field fix (bug: was re-inserting original count)
- [x] Wire into `src/mcp/stdio.rs` — 6 tool handlers support `token_budget` param:
  - `stats`, `search`, `read`, `neighbors`, `impact`, `edit_plan`
  - Auto-enforce: budget exceeded → `enforce_budget()` → strip docstrings → strip signatures → remove details → truncate
  - Response metadata: `budget_requested`, `tokens_used`, `compression_applied`
- [x] Wire into `src/mcp/mod.rs` — HTTP JSON-RPC handlers support `token_budget` param:
  - `handle_read`, `handle_neighbors`, `handle_impact`, `handle_search`
  - `get_token_budget()` — extract optional budget from params
  - `apply_budget()` — enforce + wrap in `TokenBudgetedResponse` when truncated
  - `budgeted_success()` — budget-aware response builder
  - Within budget: attach `tokens_used` + `budget_remaining` metadata
  - Over budget: 4-stage compression → `compression_applied` info included
- [x] `src/main.rs` — `mod token_budget;` added (bin target)
- [x] 14 new token_budget tests (545 total tests: 263 lib + 263 bin + 17 integration + 2 doctest)

### 4-4. Integration tests ✅ DONE

- [x] `tests/integration_test.rs` — 17 integration tests
- [x] Token budget enforcement tests (reduces response, preserves within budget)
- [x] Compression pipeline E2E tests (monotonic reduction across 4 budgets)
- [x] MCP response token count verification (file_symbols, search, impact, neighbors, stats)
- [x] Compression preserves essential fields (name, kind survive all strategies)
- [x] Strategy order verification (docstrings → signatures → details → truncate)
- [x] TokenBudgetedResponse wrapper tests (within/exceeded budget)
- [x] TruncateResults count field consistency
- [x] 545 total tests (263 unit + 17 integration + 2 doctest)

---

## Phase 8 — LanguageBackend Trait Completion ✅ COMPLETE

The `SourceCodeParser` trait is now fully aligned with masterplan §8.1.

- [x] Add to `SourceCodeParser` trait:
  - [x] `slice_symbol(&self, source, symbol, mode) -> Result<SymbolSlice>`
  - [x] `estimate_tokens(&self, snippet: &str) -> usize`
  - [x] `is_dynamic_ref(&self, ref_kind: &str) -> bool` — default impl checks dynamic_dispatch/virtual_call/string_ref
- [x] `CompressionMode` type alias → `CompressionLevel` (masterplan §8.1 naming)
- [x] `SymbolSlice` type alias → `CompressedSymbol` (masterplan §8.1 naming)
- [x] 6 new unit tests for `is_dynamic_ref()` (static kinds, dynamic kinds, trait dispatch, multi-parser)
- [x] 2 new unit tests for type aliases (`CompressionMode`, `SymbolSlice`)
- [x] 256 unit + 17 integration = 273 tests, all passing

---

## Phase 9 — Refactoring-Specialized APIs ✅ COMPLETE

Advanced APIs for safe refactoring workflows. All 4 APIs implemented with MCP handlers, CLI commands, and unit tests.

### 9-1. impact_deep ✅ DONE

- [x] Implement `impact_deep` in `src/graph/mod.rs`
  - Multi-depth transitive dependency tracing (BFS with visited set)
  - Risk scoring per depth layer (`low`, `medium`, `high`, `critical`)
  - `include_tests`, `include_dynamic` flags
  - `test_coverage_gaps`, `critical_paths`, `dynamic_refs_at_risk`
  - `recommendation` string based on analysis
- [x] MCP handler: `handle_impact_deep` → `shardindex.impact_deep`
- [x] CLI: `shardindex impact_deep <symbol> --depth --include-tests --include-dynamic`
- [x] Response types: `ImpactDeepResult`, `ImpactLayer`, `DynamicRefAtRisk`

### 9-2. dead_code_verify ✅ DONE

- [x] Implement multi-stage dead code verification
  - Stages: static_refs, dynamic_refs, string_refs, git_history, test_refs
  - Return `safe_to_delete` + blockers list
  - `DeadCodeVerifyResult` with `HashMap<String, DeadCodeStage>` stages
  - `suggestion` string with actionable advice
- [x] MCP handler: `handle_dead_code_verify` → `shardindex.dead_code_verify`
- [x] CLI: `shardindex dead_code_verify <symbol> --stages`

### 9-3. cross_module_move ✅ DONE

- [x] Safe symbol relocation across modules
  - Auto-update imports and references
  - Dry-run mode with file modification plan
  - Unresolved reference detection
  - `CrossModuleMoveResult` with `FileModification`, `UnresolvedRef`
  - `estimated_tokens` for change scope
- [x] MCP handler: `handle_cross_module_move` → `shardindex.cross_module_move`
- [x] CLI: `shardindex cross_module_move <symbol> <target_module> --update-imports --dry-run`

### 9-4. signature_migration_check ✅ DONE

- [x] Check if signature change breaks callers
  - Analyze call sites for positional/keyword arg compatibility
  - Return `compatible` + `breaking_callers` list
  - `SignatureMigrationResult` with `BreakingCaller` details
  - Helper functions: `count_params`, `count_required_params`, `extract_return_type`, `return_type_changed`
  - UTF-8 safe arrow handling (`→` vs `->`)
- [x] MCP handler: `handle_signature_migration_check` → `shardindex.signature_migration_check`
- [x] CLI: `shardindex signature_migration_check <symbol> <new_signature>`
- [x] 5 unit tests for helper functions (count_params, count_required_params, extract_return_type, return_type_changed)

### Infrastructure
- [x] 4 MCP JSON-RPC routes registered in router
- [x] 4 CLI subcommands in `src/cli/mod.rs` + handlers in `src/main.rs`
- [x] All response types derive `Serialize`, `Deserialize`, `Clone`, `Debug`
- [x] 545 total tests (263 lib + 263 bin + 17 integration + 2 doctest), all passing

---

## Phase 11 — Error Handling ✅ COMPLETE

### 11-1. Error Taxonomy ✅ DONE

- [x] Create `src/error.rs` (354 lines)
  - `ShardError` struct with `code`, `message`, `details`
  - `ErrorCode` enum: 11 variants (StaleIndex, SymbolNotFound, ParserError, TokenBudgetExceeded, RefIntegrityViolation, CircularDependency, CrossLanguageGap, DatabaseError, IoError, ConfigError, IndexNotInitialized)
  - JSON-RPC error codes (-32001 ~ -32011)
  - `agent_action()` — human-readable suggestion per error type
  - `ShardResult<T>` type alias
  - `From<anyhow::Error>` — heuristic classification (order matters!)
  - `From<std::io::Error>`, `From<rusqlite::Error>`
  - `serde::Serialize` for MCP responses
- [x] 12 unit tests (error_codes_unique, error_display, agent_action, serialization, clone, anyhow_conversion x4, io_conversion, sqlite_type_check, jsonrpc_negative)

### 11-2. Filesystem Fallback Protocol ✅ DONE

- [x] Create `src/fallback.rs` (400 lines)
  - `FallbackResult` — structured output with `success`, `warning`, `matches`, `source_tag`
  - `FallbackMatch` — file, line, content, context_before/after, estimated_tokens
  - `FallbackConfig` — max_files(3), max_lines(200), context_lines(2), prefer_ripgrep
  - `filesystem_fallback(repo_root, symbol, config)` — public API
  - ripgrep search with glob exclusions (node_modules, .git, target, etc.)
  - grep fallback with 20 language file extensions
  - Context reading with configurable lines before/after match
  - Warning injection: "ShardIndex unavailable. Using filesystem fallback."
- [x] 8 unit tests (not_found, finds_symbol, max_files, invalid_repo, serialization, match_context, parse_grep, config_defaults)

### Infrastructure
- [x] `thiserror = "2"` added to Cargo.toml
- [x] `error` + `fallback` modules registered in `src/lib.rs`
- [x] 565 total tests (283 lib + 263 bin + 17 integration + 2 doctest), all passing

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

1. ~~**Phase 4** — Semantic Compression (all 4 sub-tasks)**~~ ✅ COMPLETE
2. ~~**Phase 8** — Complete LanguageBackend trait (is_dynamic_ref, types)~~ ✅ COMPLETE
3. ~~**Phase 9** — Refactoring APIs (impact_deep, dead_code_verify, etc.)~~ ✅ COMPLETE
4. ~~**Phase 11** — Error handling / fallback~~ ✅ COMPLETE
5. **Phase 12** — Benchmarks ← NEXT
