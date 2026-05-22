# ShardIndex — Remaining Work Plan

> **Version:** 1.0
> **Date:** 2026-05-22
> **Status:** Phase 2a (18-language parser) complete, Phase 2b-4 partially complete
> **Current State:** `cargo check` ✓ 0 errors, `cargo test` ✓ 140 passed

---

## Executive Summary

**Already implemented (verified by code audit):**
- ✅ Sprint A: Daemon event loop (`daemon_loop()`, `process_batch()`, graceful shutdown)
- ✅ Sprint A: Crash recovery journal (`RecoveryJournal`, `RecoveryEngine`, `recover()`, `start_with_recovery()`)
- ✅ Sprint A: Agent cache CRUD + TTL eviction (`get_cached`, `set_cached`, `invalidate_cached`, `purge_expired`, `cache_stats`)
- ✅ Sprint B: PageRank persistence (`compute_and_store_ranks`, `upsert_rank`, `symbol_rank` table in schema v2)
- ✅ Sprint B: PageRank in advanced_search (`compute_combined_score` with page_rank weighting)

**Remaining work (6 tasks):**
1. **Sprint A:** `edit_plan` + `verify` MCP endpoints
2. **Sprint B:** Override registry CRUD + CLI commands
3. **Sprint B:** CLI polish — auto-detect language, verify cmd, `--json` flag
4. **Sprint C:** `compression.rs` — signature_only, critical_branches, full_body
5. **Sprint C:** Token estimation in parsers + DB storage
6. **Sprint C:** Token budget negotiation in `read()` MCP handler

---

## Current Codebase Reference

### Source Files (relevant to remaining work)

| File | Lines | Purpose |
|------|-------|---------|
| `src/mcp/mod.rs` | ~500 | MCP JSON-RPC + REST handlers |
| `src/database/mod.rs` | ~888 | IndexDb CRUD, checksums, dirty queue, agent cache, ranks |
| `src/database/schema.rs` | ~400 | Schema v2, migrations, views |
| `src/graph/mod.rs` | ~2000 | Graph traversal, DOT output, PageRank |
| `src/search.rs` | ~400 | Fuzzy search, Levenshtein, scoring |
| `src/cli/mod.rs` | ~100 | Clap CLI definitions |
| `src/indexer/mod.rs` | ~409 | Language enum, ProjectIndexer |
| `src/indexer/tests.rs` | ~952 | Multi-language parser tests |
| `src/integrity.rs` | ~400 | Blake3 integrity guard |
| `src/config.rs` | ~400 | Config structs, defaults |
| `src/main.rs` | ~454 | CLI command routing |

### Database Schema (v2)

**Existing tables:**
- `files` — path, language, blake3_hash, status, indexed_at
- `symbol` — name, kind, file_id, start_line, end_line, signature, is_public, docstring
- `reference` — caller_symbol_id, callee_symbol_id, ref_kind, line
- `file_imports` — file_id, imported_file_path
- `symbol_rank` — symbol_name, page_rank, in_degree, out_degree
- `checksums` — file_path, blake3_hash, verified_at, mismatch_count
- `dirty_queue` — file_path, reason, priority, enqueued_at, retry_count, status
- `agent_cache` — query_key, result_json, ttl_seconds, created_at, expires_at
- `versions` — schema version tracking

**Views:**
- `v_public_api` — public symbols with file info
- `v_dirty_priority` — dirty queue ordered by priority

**Missing tables (needed for remaining work):**
- `overrides` — manual reference overrides
- Symbol `estimated_tokens` column (alter `symbol` table)

---

## Task 1: edit_plan + verify MCP Endpoints

**Goal:** Add two MCP methods for refactoring safety:
- `edit_plan` — Pre-edit validation: analyze impact of proposed changes
- `verify` — Post-edit verification: confirm integrity after changes

### edit_plan

**Spec (from masterplan §9.1):**

Request:
```json
{
  "method": "edit_plan",
  "params": {
    "symbol": "payments.process_refund",
    "proposed_changes": [
      {"type": "rename", "from": "process_refund", "to": "handle_refund"},
      {"type": "add_param", "param": "reason: str"}
    ],
    "depth": 2,
    "token_budget": 2000
  }
}
```

Response:
```json
{
  "result": {
    "affected_symbols": [...],
    "files_to_update": [...],
    "breaking_changes": [...],
    "safe_to_proceed": true,
    "estimated_tokens": 1200
  }
}
```

### verify

**Spec (from masterplan §11):**

Request:
```json
{
  "method": "verify",
  "params": {
    "files": ["src/payments/mod.rs"],
    "check_integrity": true,
    "check_refs": true
  }
}
```

Response:
```json
{
  "result": {
    "verified": true,
    "integrity_ok": true,
    "refs_valid": true,
    "issues": []
  }
}
```

### Implementation

**Files to modify:**
- `src/mcp/mod.rs` — Add `handle_edit_plan()` and `handle_verify()` handlers
- `src/mcp/mod.rs` — Register in `jsonrpc_handler` match and `create_router`
- `src/graph/mod.rs` — Add `analyze_edit_plan()` function (impact + breaking change detection)
- `src/integrity.rs` — Add `verify_post_edit()` function

**New structs (in `src/mcp/mod.rs`):**
```rust
#[derive(Debug, Deserialize)]
struct EditPlanParams {
    symbol: String,
    proposed_changes: Vec<EditChange>,
    depth: u8,
    token_budget: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct EditChange {
    #[serde(rename = "type")]
    change_type: String, // "rename", "add_param", "remove_param", "change_return"
    #[serde(flatten)]
    details: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct VerifyParams {
    files: Option<Vec<String>>,
    symbols: Option<Vec<String>>,
    check_integrity: bool,
    check_refs: bool,
}
```

**edit_plan logic:**
1. Look up target symbol in DB
2. Run `impact_ranked(symbol)` to get all callers
3. For each `rename` change: identify all files that reference the old name
4. For each `add_param/remove_param`: identify callers that need updating
5. Return list of affected files + breaking changes assessment

**verify logic:**
1. For each file in params: re-hash with blake3
2. Compare with stored checksum → mark dirty if mismatch
3. Check reference integrity: verify all caller→callee pairs still exist in AST
4. Return verification report

**Tests:**
- Test `edit_plan` with rename scenario
- Test `edit_plan` with add_param scenario
- Test `verify` with clean file
- Test `verify` with modified file (hash mismatch)

---

## Task 2: Override Registry CRUD + CLI Commands

**Goal:** Manual reference override system for cases where static analysis misses dynamic refs.

### Spec (from masterplan)

**Override table schema:**
```sql
CREATE TABLE overrides (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    caller_symbol TEXT NOT NULL,
    callee_symbol TEXT NOT NULL,
    ref_kind TEXT NOT NULL DEFAULT 'override',
    confidence REAL NOT NULL DEFAULT 0.9,
    reason TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_overrides_caller ON overrides(caller_symbol);
CREATE INDEX idx_overrides_callee ON overrides(callee_symbol);
```

### Implementation

**Files to create/modify:**
- `src/database/mod.rs` — Add `insert_override()`, `get_overrides()`, `remove_override()`, `list_overrides()`
- `src/database/schema.rs` — Schema v3 migration with `overrides` table
- `src/cli/mod.rs` — Add `override add`, `override list`, `override remove` subcommands
- `src/main.rs` — Route override commands

**Override API:**
```rust
// In IndexDb
pub fn insert_override(&self, caller: &str, callee: &str, kind: &str, confidence: f64, reason: &str) -> Result<()>
pub fn remove_override(&self, id: i64) -> Result<()>
pub fn list_overrides(&self) -> Result<Vec<OverrideRecord>>
pub fn overrides_for_symbol(&self, symbol: &str) -> Result<Vec<OverrideRecord>>
```

**CLI commands:**
```
shardindex override add --caller "utils.dispatch" --callee "payments.process_refund" --kind "dynamic" --reason "getattr()"
shardindex override list
shardindex override remove --id 1
```

**Integration with graph/search:**
- `graph_edges()` should UNION `reference` + `overrides` tables
- `neighbors()` and `impact()` should include override refs
- `search_symbol_ranked()` should factor in override refs

**Schema migration:**
- Bump `CURRENT_SCHEMA_VERSION` from 2 → 3
- Add migration entry for `overrides` table creation

**Tests:**
- Override CRUD lifecycle
- Override refs appear in graph traversal
- Override refs appear in impact analysis

---

## Task 3: CLI Polish

### 3.1 Auto-detect Language in `init`

**Current behavior:** User must specify `-l python` explicitly.

**New behavior:** If language not specified, scan project files to detect:
- `Cargo.toml` → Rust
- `package.json` + `.ts` files → TypeScript
- `package.json` + `.js` files → JavaScript
- `pyproject.toml` / `requirements.txt` / `setup.py` → Python
- `go.mod` → Go
- `Gemfile` → Ruby
- `*.go` files → Go
- File extension majority vote as fallback

**Implementation:**
- Add `detect_language(root: &Path) -> Option<Language>` function in `src/indexer/mod.rs`
- Modify `cmd_init()` to call `detect_language()` when language arg is not provided
- Update `Commands::Init` in `src/cli/mod.rs` to make `--language` optional

### 3.2 Verify CLI Command

**Command:** `shardindex verify [--file <path>] [--all]`

**Implementation:**
- Add `Commands::Verify` variant in `src/cli/mod.rs`
- Add `cmd_verify()` function in `src/main.rs`
- Use existing `integrity::verify_file()` and `integrity::verify_all_files()`
- Output: pass/fail per file, summary count

### 3.3 JSON Output Flag

**Implementation:**
- Add `--json` flag to search, stats, impact, neighbors commands
- When `--json` is set, output results as JSON instead of formatted text
- Create helper structs for JSON serialization in each command module

**Files to modify:**
- `src/cli/mod.rs` — Add `#[command(global = true)] json_output: bool` or per-command `--json`
- `src/main.rs` — Conditional JSON vs text output in each command function

---

## Task 4: compression.rs — Semantic Compression Pipeline

**Goal:** Implement AST-aware code compression with 3 modes as specified in masterplan §10.

### Compression Modes

```rust
pub enum CompressionMode {
    SignatureOnly,      // ~50 tokens/symbol
    CriticalBranches,   // ~150 tokens/symbol
    FullBody,           // ~400 tokens/symbol
}
```

### Implementation

**Files to create:**
- `src/compression.rs` — Main compression module

**New module declaration:**
- Add `mod compression;` in `src/main.rs`

**Structures:**
```rust
pub struct CompressedSymbol {
    pub signature: String,
    pub docstring: Option<String>,
    pub critical_branches: Vec<String>,
    pub side_effects: Vec<String>,
    pub key_assignments: Vec<String>,
    pub return_statement: Option<String>,
    pub estimated_tokens: u32,
    pub mode: CompressionMode,
}
```

**`signature_only` mode:**
- Extract: function/method signature line, docstring
- Returns: signature + docstring only
- ~50 tokens/symbol

**`critical_branches` mode:**
- Everything from `signature_only` +
- Extract control flow: `if/elif/else`, `match/branch`, `try/catch`, `for/while` conditions
- Extract side effects: DB calls, API calls, file I/O (heuristic: method calls outside local scope)
- Extract key assignments: variable declarations with complex RHS
- Extract return statement
- ~150 tokens/symbol

**`full_body` mode:**
- Full source code of the symbol
- ~400 tokens/symbol (or actual token count)

**AST-based extraction approach:**
- Use tree-sitter AST to identify node types for each mode
- `signature_only`: Just the signature node + docstring child
- `critical_branches`: Walk AST, collect condition nodes, call nodes, assignment nodes
- `full_body`: Source slice from `start_byte` to `end_byte`

**API:**
```rust
pub fn compress_symbol(
    source: &str,
    symbol: &ParsedSymbol,
    language: Language,
    mode: CompressionMode,
) -> Result<CompressedSymbol, anyhow::Error>
```

**Tests:**
- Compress a Python function with all 3 modes
- Compress a Rust function with all 3 modes
- Verify token estimates are within reasonable bounds

---

## Task 5: Token Estimation in Parsers + DB Storage

**Goal:** Estimate token count for each symbol during indexing, store in DB.

### Implementation

**Token estimation heuristic:**
- Simple approach: `chars.len() / 4` as rough token estimate (English/code avg ~4 chars/token)
- Better approach: Use a simple tokenizer count — split on whitespace + punctuation boundaries

**DB changes:**
- Add `estimated_tokens` column to `symbol` table
- Schema v3 migration: `ALTER TABLE symbol ADD COLUMN estimated_tokens INTEGER DEFAULT 0`
- Update `SymbolRecord` struct with `estimated_tokens: u32`
- Update `insert_symbol()` to accept and store token count
- Update `search_symbol_ranked()` to return `estimated_tokens`

**Parser integration:**
- In `indexer/mod.rs` — Add `estimate_tokens(source: &str) -> u32` function
- In `ProjectIndexer::index_file()` — After parsing symbols, compute estimated_tokens for each
- Store during `insert_symbol()`

**Files to modify:**
- `src/database/schema.rs` — Schema v3 migration, add `estimated_tokens` to symbol table
- `src/database/mod.rs` — Update `SymbolRecord`, `insert_symbol()`, `search_symbol_ranked()`
- `src/indexer/mod.rs` — Add `estimate_tokens()`, use in `index_file()`
- `src/indexer/tests.rs` — Tests for token estimation

---

## Task 6: Token Budget Negotiation in read() MCP Handler

**Goal:** When an agent calls `read(symbol)`, respect its token budget and adapt compression.

### Implementation

**Current `read()` handler:** Returns full symbol body.

**New `read()` handler with budget negotiation:**

```rust
pub async fn handle_read(
    db: Arc<Mutex<IndexDb>>,
    params: &serde_json::Value,
) -> JsonRpcResponse {
    let symbol_name = params.get("symbol").unwrap().as_str().unwrap();
    let mode = params.get("compression").and_then(|v| v.as_str());
    let token_budget = params.get("token_budget").and_then(|v| v.as_u64());

    let db = db.lock().unwrap();
    let symbol = db.symbol_by_name(symbol_name)?;

    // Read source and compress
    let source = std::fs::read_to_string(&symbol.file_path)?;
    let compressed = compress_symbol(&source, &symbol, mode, token_budget)?;

    if let Some(budget) = token_budget {
        if compressed.estimated_tokens > budget {
            // Downgrade compression mode until within budget
            // full_body → critical_branches → signature_only
        }
    }

    JsonRpcResponse::success(Some(params.get("id").clone()), &compressed)
}
```

**Budget negotiation logic:**
1. If `token_budget` specified, start with most compressed mode
2. Check if estimated tokens ≤ budget
3. If not, escalate compression (signature_only → critical_branches → full_body)
4. If even full_body exceeds budget, truncate and warn
5. Return `compression_used`, `estimated_tokens`, `budget_remaining`, `suggestion`

**Files to modify:**
- `src/mcp/mod.rs` — Update `handle_read()` to use compression + budget negotiation
- `src/compression.rs` — Export for MCP use

---

## Schema Migration Plan

Current schema is v2. The remaining work requires v3:

```
v2 → v3 migrations:
1. ALTER TABLE symbol ADD COLUMN estimated_tokens INTEGER DEFAULT 0
2. CREATE TABLE overrides (...) with indexes
```

---

## File Change Summary

| File | Changes |
|------|---------|
| `src/main.rs` | Add `mod compression`, add `Commands::Verify`, add `Commands::Override`, route new commands |
| `src/mcp/mod.rs` | Add `handle_edit_plan()`, `handle_verify()`, update `handle_read()` with budget negotiation |
| `src/database/mod.rs` | Add `estimated_tokens` to `SymbolRecord`, add override CRUD methods, update `insert_symbol()` |
| `src/database/schema.rs` | Schema v3 migration: `estimated_tokens` column + `overrides` table |
| `src/cli/mod.rs` | Add `Verify`, `Override` commands, make `--language` optional in `Init`, add `--json` flag |
| `src/graph/mod.rs` | Add `analyze_edit_plan()`, integrate overrides into `graph_edges()` |
| `src/integrity.rs` | Add `verify_post_edit()` |
| `src/indexer/mod.rs` | Add `estimate_tokens()`, `detect_language()`, use in `index_file()` |
| `src/compression.rs` | **NEW FILE** — `CompressedSymbol`, `compress_symbol()`, 3 compression modes |
| `src/indexer/tests.rs` | Add tests for token estimation |

---

## Execution Order (Dependencies)

```
Phase 1 — Schema v3 (foundation)
  ├─ Task 5: Token estimation (DB schema change first)
  └─ Task 2: Override registry (DB schema change)

Phase 2 — Core features (parallel)
  ├─ Task 4: Compression (no DB dependency)
  ├─ Task 1: edit_plan + verify (needs graph analysis)
  └─ Task 3: CLI polish (independent)

Phase 3 — Integration
  └─ Task 6: Token budget in read() (depends on Task 4 + Task 5)

Final
  └─ cargo check + cargo test
```

---

## Risks & Tradeoffs

1. **Schema v3 migration** — Must handle existing databases gracefully (ALTER TABLE with DEFAULT)
2. **Token estimation accuracy** — Char-based heuristic is approximate; consider adding a proper tokenizer later
3. **Compression AST traversal** — Different languages have different AST structures; may need per-language extraction logic
4. **Override table** — Could grow large in projects with many dynamic refs; consider periodic cleanup

---

*End of Plan*
