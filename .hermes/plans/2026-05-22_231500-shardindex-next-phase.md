# ShardIndex — Next Phase Implementation Plan

> **Created:** 2026-05-22  
> **Based on:** Masterplan v1.2, current codebase state  
> **Scope:** Phase 2 completion → Phase 3 infrastructure → Phase 4 preparation

---

## Goal

Complete Phase 2 (robustness features currently missing), solidify Phase 3 (graph ranking integration, CLI polish), and prepare the foundation for Phase 4 (semantic compression).

---

## Current Context & Assumptions

### What Exists (13,338 lines total)

**Core infrastructure — DONE:**
- SQLite schema v2 with 7 tables + 3 views (`files`, `symbols`, `refs`, `checksums`, `dirty_queue`, `versions`, `overrides`, `agent_cache`)
- Blake3 integrity guard with lazy/sync verification
- Dirty queue DB layer with priority-based processing
- 18-language tree-sitter parsers (Python, JS, TS, Rust, Go, Ruby, Java, PHP, Julia, Lua, Swift, Zig, Scala, Elixir, Dart, Haskell, C, C++)
- Fuzzy search with Levenshtein + identifier tokenization + PageRank-aware scoring (140 tests, all passing)
- MCP/JSON-RPC API server with `read`, `neighbors`, `impact`, `search`, `stats` endpoints
- CLI: `init`, `daemon`, `reindex`, `stats`, `search`, `neighbors`, `impact`, `graph`, `rank`
- File watcher (`notify` crate) with ignore patterns
- Daemon shared state with dirty event queue
- PageRank computation (`graph/mod.rs`)
- Crash recovery journal stubs (`recovery.rs` — 636 lines)

**Build health:**
- `cargo check`: 0 errors, 31 warnings (dead code)
- `cargo test`: 140 passed, 0 failed

### What's Missing or Incomplete

**Phase 2 gaps:**
1. Background daemon loop — `Daemon::start()` exists but the actual watch→parse→index event loop is not wired up end-to-end
2. Crash recovery journal — struct exists but no actual WAL write/read cycle
3. Confidence scoring for dynamic refs — not implemented
4. `edit_plan` + `verify` MCP endpoints — not implemented
5. Agent cache layer — schema table exists (`agent_cache`) but no CRUD logic beyond schema tests

**Phase 3 gaps:**
6. PageRank integration with search results — `compute_and_store_ranks()` exists but doesn't persist to DB
7. Override registry CLI — not implemented
8. Cross-language references — not implemented

**Phase 4 (not started):**
9. Token estimation per symbol
10. Compression pipeline (signature_only, critical_branches, full_body modes)

---

## Proposed Approach: Three Sprints

### Sprint A — Daemon & Integrity Loop (Phase 2 completion)

Wire up the daemon's actual runtime: file watch → dirty queue → parse → DB upsert → hash verification cycle. This is the critical path that makes ShardIndex self-maintaining.

**Steps:**

1. **Daemon event loop** (`src/daemon.rs`)
   - Implement `Daemon::run_watch_loop()` — async loop that drains dirty events, debounces (50ms window), dispatches to parser
   - Wire `FileWatcher` → `Daemon::add_dirty_event()` → parse → `IndexDb::upsert_*`
   - Add `Daemon::process_file(path)` that: parses with correct `LanguageBackend`, upserts file hash, upserts symbols, upserts refs, updates checksums
   - Add graceful shutdown via `tokio::signal`

2. **Crash recovery journal** (`src/recovery.rs`)
   - Implement `RecoveryJournal::begin_transaction(tx_id)` → append to `.shardindex/journals/recovery.wal`
   - `RecoveryJournal::commit(tx_id)` → mark complete
   - `RecoveryJournal::recover()` → on daemon start, replay uncommitted transactions
   - Format: `tx_id|file_path|operation|timestamp` per line, Blake3 checksum per batch

3. **Agent cache layer** (`src/database/mod.rs`)
   - Implement CRUD for `agent_cache` table: `cache_query()`, `get_cached_query()`, `evict_expired()`
   - TTL-based eviction (default 5min)
   - Hash-based invalidation: if any `file_hashes_at_creation` changed, evict dependent caches

4. **`edit_plan` + `verify` MCP endpoints** (`src/mcp/mod.rs`)
   - `handle_edit_plan()` — validate rename/extract operations against ref graph, return impact summary
   - `handle_verify()` — post-edit integrity check: re-hash files, re-parse symbols, detect orphan refs

**Files to modify:**
- `src/daemon.rs` — main changes
- `src/recovery.rs` — implement journal I/O
- `src/database/mod.rs` — agent_cache CRUD
- `src/mcp/mod.rs` — new handlers
- `src/indexer/mod.rs` — expose `parse_file(path)` that auto-detects language

**Tests:**
- `test_daemon_watch_loop` — simulate file change → verify DB update
- `test_recovery_replay` — write journal → crash → recover → verify consistency
- `test_cache_ttl_eviction` — insert cache → wait → verify eviction
- `test_edit_plan_validation` — rename symbol → verify ref graph consistency check
- `test_verify_post_edit` — edit file → verify → check integrity status

---

### Sprint B — Search Integration & CLI Polish (Phase 3 completion)

Connect PageRank to search, add override registry, and make the CLI production-ready.

**Steps:**

5. **PageRank persistence** (`src/graph/mod.rs` + `src/database/schema.rs`)
   - Add `pagerank` column to `symbols` table (migration v3)
   - `compute_and_store_ranks()` → persist scores to DB
   - `advanced_search()` → incorporate PageRank into combined score

6. **Override registry** (`src/database/mod.rs` + `src/cli/mod.rs`)
   - CRUD for `overrides` table: `add_override()`, `get_overrides()`, `apply_overrides_to_refs()`
   - CLI commands: `shardindex override add --pattern "..." --target "..."`
   - Apply overrides during search/neighbors/impact queries

7. **CLI polish**
   - `shardindex init` → actually walk directory, detect all 18 languages, parse and index all files
   - `shardindex daemon` → start MCP server + file watcher + recovery journal
   - Add `shardindex verify` CLI command
   - Add `--json` flag for machine-readable output on all commands
   - Multi-language support: `init` and `daemon` auto-detect all languages, no need for `--language`

**Files to modify:**
- `src/database/schema.rs` — migration v3 (pagerank column)
- `src/graph/mod.rs` — persist ranks
- `src/search.rs` — integrate PageRank into scoring
- `src/database/mod.rs` — overrides CRUD
- `src/cli/mod.rs` — new commands, multi-language
- `src/main.rs` — wire CLI → implementation

**Tests:**
- `test_pagerank_persistence` — compute → store → reload → verify scores match
- `test_search_with_pagerank` — high-rank symbols score higher
- `test_override_application` — add override → verify ref graph includes it
- `test_init_multilanguage` — init on mixed repo → verify all languages indexed

---

### Sprint C — Semantic Compression Foundation (Phase 4 start)

Implement token estimation and the compression pipeline. This is the core differentiator — ShardIndex returns semantically compressed code, not raw files.

**Steps:**

8. **Token estimation** (`src/indexer/types.rs` + parser backends)
   - Add `token_count` to `ParsedSymbol` — simple heuristic: `chars / 4` (4 chars/token avg)
   - Store in `symbols.token_count` on insert
   - `read()` API returns `estimated_tokens` per symbol

9. **Compression pipeline** (`src/compression.rs` — new file)
   - `compress_signature_only(symbol)` → extract signature line(s)
   - `compress_critical_branches(symbol)` → extract control flow nodes (if/else/match/loop) from AST
   - `compress_full_body(symbol)` → full source with comments stripped
   - Each mode returns compressed text + token estimate

10. **Token budget negotiation** (`src/mcp/mod.rs`)
    - `read()` accepts `token_budget` parameter
    - Auto-select compression level to fit budget
    - Return `budget_remaining` + `suggestion` in response

11. **Update `read()` handler** (`src/mcp/mod.rs`)
    - Replace raw file content with compressed symbol body
    - Include refs (calls + called_by)
    - Include hash verification status

**Files to modify:**
- `src/compression.rs` — new file
- `src/indexer/types.rs` — add token_count
- `src/database/schema.rs` — ensure `token_count` column
- `src/mcp/mod.rs` — update `handle_read()`, add budget logic
- `src/database/mod.rs` — update `insert_symbol()` to store token_count

**Tests:**
- `test_token_estimation_accuracy` — compare estimated vs actual tokens on known files
- `test_compression_signature_only` — verify output is just signature
- `test_compression_critical_branches` — verify control flow extracted
- `test_budget_negotiation` — request budget=200 → verify compression level adapts
- `test_read_with_compression` — end-to-end: parse → compress → serve via MCP

---

## Files Summary

| File | Sprint | Change Type |
|------|--------|-------------|
| `src/daemon.rs` | A | Major — event loop, file processing |
| `src/recovery.rs` | A | Major — journal I/O, replay |
| `src/database/mod.rs` | A, B, C | Incremental — cache, overrides, token_count |
| `src/mcp/mod.rs` | A, C | Major — edit_plan, verify, compression |
| `src/indexer/mod.rs` | A, C | Minor — parse_file(), token_count |
| `src/compression.rs` | C | New file |
| `src/graph/mod.rs` | B | Medium — persist ranks |
| `src/search.rs` | B | Minor — PageRank integration |
| `src/database/schema.rs` | B | Minor — migration v3 |
| `src/cli/mod.rs` | B | Medium — new commands |
| `src/main.rs` | B | Minor — wire commands |
| `src/indexer/types.rs` | C | Minor — token_count field |

---

## Risks & Tradeoffs

1. **Daemon complexity** — The watch→parse→index loop is the most error-prone part. Recommend starting with a synchronous `process_file()` that works standalone, then wrapping it in the async loop.

2. **tree-sitter 0.25 API** — All 18 parsers already use the 0.25 API. Any compression logic that walks AST nodes must use `named_child()` / `child_by_field_name()` — not the old `field()` API.

3. **Token estimation accuracy** — `chars / 4` is a rough heuristic. Consider using `tiktoken-rs` or `tokenizers` crate for precision, but this adds a dependency. The heuristic is good enough for Phase 4 MVP.

4. **Compression correctness** — `critical_branches` extraction requires walking AST control flow nodes per-language. This is language-specific logic. Start with Python/Rust/JS as proof-of-concept, then generalize.

5. **SQLite WAL mode** — Multiple daemon threads may write concurrently. Ensure WAL mode is enabled and use proper transaction boundaries.

---

## Verification Criteria

After all three sprints:
- `cargo check`: 0 errors
- `cargo test`: 140+ tests, all passing
- `shardindex init /some/repo` → indexes all files, all 18 languages detected
- `shardindex daemon` → starts server, watches files, auto-reindexes on changes
- MCP `read()` returns compressed symbol with refs and token budget
- MCP `edit_plan()` validates ref integrity before allowing edits
- Crash recovery: kill daemon → restart → no data loss
