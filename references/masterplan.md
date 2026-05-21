# ShardIndex — Implementation Masterplan for Qwen3.6 27B Local Agent

> **Version:** 1.0  
> **Target Model:** Qwen3.6 27B (140K Context Window) via Ollama / LM Studio  
> **Target Repo Scale:** 20K–200K LOC (Phase 1–2), 500K+ LOC (Phase 4)  
> **Primary Language:** Rust (daemon + parser backend), SQLite (metadata graph)  
> **Protocol:** MCP (Model Context Protocol) / JSON-RPC 2.0

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [High-Level Architecture](#2-high-level-architecture)
3. [SQLite Schema Specification](#3-sqlite-schema-specification)
4. [Core API Specification (MCP/JSON-RPC)](#4-core-api-specification-mcpjson-rpc)
5. [Agent Skill Integration Protocol](#5-agent-skill-integration-protocol)
6. [File Integrity & Blake3 Verification System](#6-file-integrity--blake3-verification-system)
7. [Incremental Indexing Engine](#7-incremental-indexing-engine)
8. [Parser Abstraction Layer (LanguageBackend)](#8-parser-abstraction-layer-languagebackend)
9. [Refactoring-Specialized APIs](#9-refactoring-specialized-apis)
10. [Token Budget & Semantic Compression](#10-token-budget--semantic-compression)
11. [Error Handling & Fallback Strategy](#11-error-handling--fallback-strategy)
12. [Performance Targets & Benchmarks](#12-performance-targets--benchmarks)
13. [Implementation Roadmap](#13-implementation-roadmap)
14. [Qwen3.6 27B Specific Optimization Notes](#14-qwen36-27b-specific-optimization-notes)

---

## 1. Executive Summary

ShardIndex is a **semantic retrieval middleware** that sits between a codebase and an LLM coding agent. It transforms file-level, grep-based workflows into **symbol-level, graph-aware, token-budgeted** interactions.

### Key Metrics

| Metric | Baseline (Naive) | With ShardIndex | Improvement |
|---|---|---|---|
| Tokens per query (200K LOC) | ~23,000 | ~4,600 | **80% reduction** |
| Query latency | 5–15s (file I/O) | <10ms (graph lookup) | **1000x faster** |
| Incremental reindex | Full reparse (minutes) | Single file (10–100ms) | **99% faster** |
| Refactoring safety | Manual grep + hope | Automated impact graph + integrity check | **Deterministic** |

### Why Qwen3.6 27B Needs This

A 140K context window is generous, but **precision beats volume**:
- Loading 20 files × 500 lines = 30K tokens of noise
- Loading 8 symbols × 150 lines (compressed) = 2K tokens of signal
- Local 27B models reason better with **dense, relevant context** than with **sparse, massive context**

ShardIndex enables the agent to treat the codebase as a **queryable semantic database**, not a filesystem.

---

## 2. High-Level Architecture

```text
┌─────────────────────────────────────────────────────────────────────┐
│                        Source Repository                             │
│                    (Python / TypeScript / Rust / Go)                │
└───────────────────────────────┬─────────────────────────────────────┘
                                │ File System Events (notify crate)
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        Integrity Guard Layer                         │
│  • Blake3 hash verification (before every API read)                  │
│  • Auto-dirty on mismatch                                            │
│  • Synchronous update after agent edits                              │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        Dirty Queue Manager                           │
│  • Priority queue (file change frequency, repo size)                 │
│  • Debounced batching (50ms window)                                  │
│  • Crash-recovery journal                                            │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        Parser Backend Pool                           │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌────────────┐  │
│  │ tree-sitter  │ │ tree-sitter  │ │ tree-sitter  │ │ Go native  │  │
│  │ python       │ │ typescript   │ │ rust         │ │ bridge     │  │
│  └──────────────┘ └──────────────┘ └──────────────┘ └────────────┘  │
│                    (LanguageBackend trait abstraction)                │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
                    ┌───────────┴───────────┐
                    ▼                       ▼
          Symbol Extraction         Reference Extraction
          (AST → symbols)            (AST → caller/callee graph)
                    └───────────┬───────────┘
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        SQLite Metadata Graph                         │
│  • files, symbols, refs, checksums, dirty_queue, versions            │
│  • Agent query cache (agent_cache)                                   │
│  • Manual override registry (overrides)                             │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        MCP / JSON-RPC API Server                     │
│  • Unix socket / TCP (localhost only)                                │
│  • Streaming responses for large graphs                              │
│  • Token-budget negotiation                                          │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        LLM Coding Agent (Qwen3.6 27B)                │
│  • System prompt with embedded ShardIndex skill protocol             │
│  • Automatic intent → API mapping                                    │
│  • Fallback to filesystem on integrity failure                       │
└─────────────────────────────────────────────────────────────────────┘
```

### Data Flow: Read Operation

```text
Agent requests: read("auth.login")
         │
         ▼
┌────────────────────┐
│ 1. Hash Check      │  ← checksums table vs live Blake3 hash
│    (0.1ms)         │
└────────┬───────────┘
         │ Valid?
    Yes ─┼─ No
         ▼         ▼
    ┌────────┐  ┌─────────────┐
    │ Return │  │ Auto-dirty  │
    │ cached │  │ Re-parse    │
    │ symbol │  │ (50ms)      │
    │ (0.5ms)│  │ Retry       │
    └────────┘  └─────────────┘
```

---

## 3. SQLite Schema Specification

### Design Principles

1. **Metadata only in SQLite** — AST blobs, file contents stored in `.shardindex/shards/`
2. **Blake3 everywhere** — File integrity, symbol body deduplication, query cache keys
3. **Incremental-friendly** — Every table supports single-row UPSERT; no table locks during reads
4. **Agent-cache-aware** — Query results cached with TTL to reduce duplicate graph traversal

### 3.1 Table: `files`

Tracks every tracked file in the repository.

| Column | Type | Constraints | Description |
|---|---|---|---|
| `id` | `INTEGER` | PK, AUTOINCREMENT | Internal surrogate key |
| `path` | `TEXT` | UNIQUE, NOT NULL | Relative repo path (`src/auth/login.py`) |
| `abs_path` | `TEXT` | NOT NULL | Absolute path (for watcher) |
| `size_bytes` | `INTEGER` | NOT NULL | File size for change detection |
| `mtime_ns` | `INTEGER` | NOT NULL | Modification time (nanoseconds) |
| `blake3_hash` | `TEXT` | NOT NULL, INDEXED | 64-char hex Blake3 hash of full content |
| `language` | `TEXT` | NOT NULL | Detected language (`python`, `typescript`, `rust`, `go`) |
| `indexed_at` | `INTEGER` | NOT NULL | Unix timestamp (ms) |
| `status` | `TEXT` | NOT NULL, DEFAULT `'valid'` | `valid`, `dirty`, `parsing`, `corrupted`, `deleted` |
| `parser_version` | `TEXT` | NOT NULL | Parser backend version (for invalidation) |
| `symbol_count` | `INTEGER` | DEFAULT 0 | Cached count for stats |
| `line_count` | `INTEGER` | DEFAULT 0 | Cached count for stats |

**Indexes:**
```sql
CREATE UNIQUE INDEX idx_files_path ON files(path);
CREATE INDEX idx_files_status ON files(status);
CREATE INDEX idx_files_language ON files(language);
CREATE INDEX idx_files_blake3 ON files(blake3_hash);
```

### 3.2 Table: `symbols`

Every extractable symbol (function, class, method, trait, interface, enum, etc.).

| Column | Type | Constraints | Description |
|---|---|---|---|
| `id` | `INTEGER` | PK, AUTOINCREMENT | Internal ID |
| `file_id` | `INTEGER` | FK → files.id, NOT NULL, INDEXED | Source file |
| `name` | `TEXT` | NOT NULL | Short name (`login`) |
| `qualified_name` | `TEXT` | NOT NULL, INDEXED | Fully qualified (`auth.login`, `src.auth.login`) |
| `kind` | `TEXT` | NOT NULL | `function`, `class`, `method`, `trait`, `interface`, `enum`, `struct`, `impl`, `module`, `variable`, `const` |
| `line_start` | `INTEGER` | NOT NULL | 1-based |
| `line_end` | `INTEGER` | NOT NULL | Inclusive |
| `col_start` | `INTEGER` | NOT NULL | 0-based byte offset |
| `col_end` | `INTEGER` | NOT NULL | 0-based byte offset |
| `signature` | `TEXT` | | Function signature / class header (compressed) |
| `signature_hash` | `TEXT` | INDEXED | Blake3 of signature (for change detection) |
| `body_hash` | `TEXT` | NOT NULL | Blake3 of full symbol body |
| `shard_path` | `TEXT` | | Path to `.shardindex/shards/{file_id}/{symbol_id}.bin` |
| `compressed_body` | `BLOB` | | LZ4-compressed semantic body (optional inline) |
| `docstring` | `TEXT` | | Extracted docstring / rustdoc / JSDoc |
| `token_count` | `INTEGER` | DEFAULT 0 | Estimated tokens (for budget planning) |
| `is_public` | `INTEGER` | DEFAULT 1 | 1 = exported, 0 = private |
| `is_test` | `INTEGER` | DEFAULT 0 | 1 = test symbol |
| `status` | `TEXT` | DEFAULT `'valid'` | `valid`, `stale`, `deleted` |
| `extracted_at` | `INTEGER` | NOT NULL | Unix timestamp (ms) |

**Indexes:**
```sql
CREATE UNIQUE INDEX idx_symbols_qualified ON symbols(qualified_name);
CREATE INDEX idx_symbols_file ON symbols(file_id);
CREATE INDEX idx_symbols_kind ON symbols(kind);
CREATE INDEX idx_symbols_name ON symbols(name);
CREATE INDEX idx_symbols_public ON symbols(is_public) WHERE is_public = 1;
CREATE INDEX idx_symbols_status ON symbols(status);
```

### 3.3 Table: `refs`

Reference graph edges. Every caller→callee relationship.

| Column | Type | Constraints | Description |
|---|---|---|---|
| `id` | `INTEGER` | PK, AUTOINCREMENT | Edge ID |
| `caller_symbol_id` | `INTEGER` | FK → symbols.id, NOT NULL, INDEXED | Calling symbol |
| `callee_symbol_id` | `INTEGER` | FK → symbols.id, INDEXED | Target symbol (NULL if unresolved) |
| `callee_name` | `TEXT` | | Raw callee name (for unresolved dynamic refs) |
| `file_id` | `INTEGER` | FK → files.id, NOT NULL | Location file |
| `line` | `INTEGER` | NOT NULL | Call site line |
| `column` | `INTEGER` | NOT NULL | Call site column |
| `kind` | `TEXT` | NOT NULL | `direct_call`, `method_call`, `static_call`, `dynamic_call`, `import`, `inheritance`, `composition`, `callback`, `string_ref`, `eval_ref` |
| `confidence` | `REAL` | NOT NULL, DEFAULT 1.0 | 0.0–1.0. Dynamic languages: <0.9 |
| `is_dynamic` | `INTEGER` | DEFAULT 0 | 1 = runtime-resolved (getattr, eval, importlib) |
| `context` | `TEXT` | | Surrounding code snippet (±2 lines) |
| `extracted_at` | `INTEGER` | NOT NULL | Unix timestamp (ms) |
| `is_deleted` | `INTEGER` | DEFAULT 0 | Soft delete for incremental updates |

**Indexes:**
```sql
CREATE INDEX idx_refs_caller ON refs(caller_symbol_id) WHERE is_deleted = 0;
CREATE INDEX idx_refs_callee ON refs(callee_symbol_id) WHERE is_deleted = 0;
CREATE INDEX idx_refs_file ON refs(file_id) WHERE is_deleted = 0;
CREATE INDEX idx_refs_confidence ON refs(confidence) WHERE is_deleted = 0;
CREATE INDEX idx_refs_kind ON refs(kind) WHERE is_deleted = 0;
CREATE INDEX idx_refs_dynamic ON refs(is_dynamic, confidence) WHERE is_dynamic = 1 AND is_deleted = 0;
```

### 3.4 Table: `checksums`

Blake3 integrity ledger. Dual-verification protocol.

| Column | Type | Constraints | Description |
|---|---|---|---|
| `id` | `INTEGER` | PK, AUTOINCREMENT | |
| `file_id` | `INTEGER` | FK → files.id, UNIQUE | |
| `blake3_hash` | `TEXT` | NOT NULL | Last known good hash |
| `computed_at` | `INTEGER` | NOT NULL | When hash was computed |
| `verified_at` | `INTEGER` | NOT NULL | Last verification timestamp |
| `verify_count` | `INTEGER` | DEFAULT 0 | Number of API-triggered verifications |
| `mismatch_count` | `INTEGER` | DEFAULT 0 | Number of mismatches detected |
| `last_mismatch_at` | `INTEGER` | | Timestamp of last mismatch |
| `status` | `TEXT` | DEFAULT `'synced'` | `synced`, `stale`, `recovering`, `corrupted` |

**Indexes:**
```sql
CREATE UNIQUE INDEX idx_checksums_file ON checksums(file_id);
CREATE INDEX idx_checksums_status ON checksums(status);
```

### 3.5 Table: `dirty_queue`

Pending reindex queue. Priority-based with crash recovery.

| Column | Type | Constraints | Description |
|---|---|---|---|
| `id` | `INTEGER` | PK, AUTOINCREMENT | |
| `file_id` | `INTEGER` | FK → files.id, NOT NULL | |
| `reason` | `TEXT` | NOT NULL | `file_modified`, `hash_mismatch`, `parser_upgrade`, `manual_trigger`, `dependency_changed` |
| `priority` | `INTEGER` | DEFAULT 5 | 1 = critical (agent-edited), 10 = low (bulk) |
| `enqueued_at` | `INTEGER` | NOT NULL | Unix timestamp (ms) |
| `processed_at` | `INTEGER` | | NULL = pending |
| `retry_count` | `INTEGER` | DEFAULT 0 | |
| `error_log` | `TEXT` | | Last error if parsing failed |
| `status` | `TEXT` | DEFAULT `'pending'` | `pending`, `processing`, `done`, `failed` |

**Indexes:**
```sql
CREATE INDEX idx_dirty_priority ON dirty_queue(priority, enqueued_at) WHERE status = 'pending';
CREATE INDEX idx_dirty_file ON dirty_queue(file_id);
```

### 3.6 Table: `versions`

Schema migration tracking.

| Column | Type | Constraints | Description |
|---|---|---|---|
| `id` | `INTEGER` | PK | |
| `schema_version` | `INTEGER` | UNIQUE, NOT NULL | Monotonic integer |
| `migration_name` | `TEXT` | NOT NULL | |
| `applied_at` | `INTEGER` | NOT NULL | |
| `checksum` | `TEXT` | | Blake3 of migration script |

### 3.7 Table: `overrides`

Manual reference overrides for dynamic/static analysis gaps.

| Column | Type | Constraints | Description |
|---|---|---|---|
| `id` | `INTEGER` | PK | |
| `pattern_type` | `TEXT` | NOT NULL | `symbol_name`, `file_path_regex`, `qualified_name` |
| `pattern` | `TEXT` | NOT NULL | Match pattern |
| `target_symbol` | `TEXT` | NOT NULL | Resolved qualified name |
| `ref_kind` | `TEXT` | DEFAULT `'direct_call'` | |
| `confidence_override` | `REAL` | DEFAULT 1.0 | |
| `notes` | `TEXT` | | Human-readable reason |
| `created_at` | `INTEGER` | NOT NULL | |

### 3.8 Table: `agent_cache`

Query result cache to avoid re-traversing identical graphs.

| Column | Type | Constraints | Description |
|---|---|---|---|
| `id` | `INTEGER` | PK | |
| `query_hash` | `TEXT` | UNIQUE, NOT NULL | Blake3 of serialized query params |
| `api_method` | `TEXT` | NOT NULL | `impact`, `neighbors`, `search` |
| `result_json` | `BLOB` | NOT NULL | LZ4-compressed JSON result |
| `hit_count` | `INTEGER` | DEFAULT 1 | |
| `created_at` | `INTEGER` | NOT NULL | |
| `last_accessed` | `INTEGER` | NOT NULL | |
| `ttl_ms` | `INTEGER` | DEFAULT 300000 | 5 minutes default |
| `file_hashes_at_creation` | `TEXT` | | JSON array of [file_id, blake3] used to build result |

**Indexes:**
```sql
CREATE UNIQUE INDEX idx_cache_query ON agent_cache(query_hash);
CREATE INDEX idx_cache_accessed ON agent_cache(last_accessed);
```

### 3.9 Views

```sql
-- Active symbol graph (excludes deleted/stale)
CREATE VIEW v_active_refs AS
SELECT r.*, cs.qualified_name as caller_name, ce.qualified_name as callee_name
FROM refs r
LEFT JOIN symbols cs ON r.caller_symbol_id = cs.id
LEFT JOIN symbols ce ON r.callee_symbol_id = ce.id
WHERE r.is_deleted = 0 AND cs.status = 'valid' AND (ce.status = 'valid' OR ce.status IS NULL);

-- Public API surface
CREATE VIEW v_public_api AS
SELECT s.*, f.path, f.language
FROM symbols s
JOIN files f ON s.file_id = f.id
WHERE s.is_public = 1 AND s.status = 'valid' AND f.status = 'valid';

-- Dirty files with priority
CREATE VIEW v_dirty_priority AS
SELECT f.path, d.reason, d.priority, d.enqueued_at, d.retry_count
FROM dirty_queue d
JOIN files f ON d.file_id = f.id
WHERE d.status = 'pending'
ORDER BY d.priority ASC, d.enqueued_at ASC;
```

---

## 4. Core API Specification (MCP/JSON-RPC)

### 4.1 Transport

- **Default:** Unix domain socket at `.shardindex/daemon.sock`
- **Fallback:** TCP `localhost:57689` (configurable)
- **Protocol:** JSON-RPC 2.0 with batching support
- **Encoding:** UTF-8 JSON, responses may be LZ4-compressed for large graphs

### 4.2 Standard Methods

#### `initialize`

Handshake. Agent declares capabilities and token budget.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "agent_name": "qwen3.6-27b",
    "context_window": 140000,
    "preferred_token_budget": 8000,
    "supported_compression": ["signature_only", "critical_branches", "full_body"],
    "auto_verify_hash": true
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "daemon_version": "0.1.0",
    "indexed_languages": ["python", "typescript"],
    "total_symbols": 12450,
    "total_refs": 89300,
    "status": "ready",
    "compression_modes": {
      "signature_only": "~50 tokens/symbol",
      "critical_branches": "~150 tokens/symbol",
      "full_body": "~400 tokens/symbol"
    }
  }
}
```

---

#### `impact`

Determine all symbols/files affected by modifying a target symbol. **Use FIRST before any edit.**

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "impact",
  "params": {
    "symbol": "auth.login",
    "depth": 2,
    "direction": "both",
    "include_tests": false,
    "min_confidence": 0.7,
    "token_budget": 2000
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "target": "auth.login",
    "affected_symbols": [
      {
        "qualified_name": "session.create",
        "file": "src/session/manager.py",
        "kind": "function",
        "relationship": "callee",
        "confidence": 0.95,
        "path": ["auth.login → session.create"],
        "estimated_tokens": 180
      },
      {
        "qualified_name": "api.users.login_handler",
        "file": "src/api/users.py",
        "kind": "function",
        "relationship": "caller",
        "confidence": 0.98,
        "path": ["api.users.login_handler → auth.login"],
        "estimated_tokens": 220
      },
      {
        "qualified_name": "middleware.auth_check",
        "file": "src/middleware/auth.py",
        "kind": "function",
        "relationship": "sibling_caller",
        "confidence": 0.82,
        "path": ["middleware.auth_check → auth.validate_token → auth.login"],
        "estimated_tokens": 150
      }
    ],
    "total_estimated_tokens": 1550,
    "files_to_read": ["src/session/manager.py", "src/api/users.py", "src/middleware/auth.py"],
    "warnings": [
      "Dynamic reference detected: getattr(auth, 'login') in src/utils/dispatch.py (confidence: 0.45)"
    ]
  }
}
```

---

#### `read`

Read a specific symbol with semantic compression. Returns structured slices, not raw file content.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "read",
  "params": {
    "symbol": "auth.login",
    "compression": "critical_branches",
    "token_budget": 800,
    "include_refs": true,
    "include_docstring": true
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "symbol": {
      "qualified_name": "auth.login",
      "kind": "function",
      "file": "src/auth/login.py",
      "lines": [42, 89],
      "signature": "def login(username: str, password: str, mfa_token: str | None = None) → Session:",
      "docstring": "Authenticate user and create session. Raises AuthError on failure.",
      "is_public": true
    },
    "compressed_body": {
      "critical_branches": [
        "if not user or not verify_password(password, user.hash): raise AuthError('Invalid credentials')",
        "if user.mfa_enabled and not verify_mfa(mfa_token, user.mfa_secret): raise AuthError('MFA failed')",
        "if user.is_locked: raise AuthError('Account locked')"
      ],
      "side_effects": [
        "db.session.add(AuditLog('login_attempt', user_id=user.id))",
        "redis.incr(f'login_attempts:{user.id}')",
        "session = session.create(user.id, ttl=3600)"
      ],
      "key_assignments": [
        "session_token = generate_jwt(session.id, roles=user.roles)"
      ],
      "return_statement": "return Session(token=session_token, expires=session.expires)"
    },
    "refs": {
      "calls": ["verify_password", "verify_mfa", "session.create", "generate_jwt"],
      "called_by": ["api.users.login_handler", "cli.admin_login"]
    },
    "estimated_tokens": 340,
    "hash_verified": true,
    "index_fresh": true
  }
}
```

---

#### `neighbors`

Explore caller/callee graph around a symbol. Use to understand data flow without reading full files.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "neighbors",
  "params": {
    "symbol": "auth.login",
    "depth": 1,
    "direction": "both",
    "max_results": 20,
    "min_confidence": 0.8
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": {
    "center": "auth.login",
    "callers": [
      {"symbol": "api.users.login_handler", "confidence": 0.98, "file": "src/api/users.py", "line": 34},
      {"symbol": "cli.admin_login", "confidence": 0.99, "file": "src/cli/admin.py", "line": 12}
    ],
    "callees": [
      {"symbol": "verify_password", "confidence": 0.95, "file": "src/auth/crypto.py", "line": 8},
      {"symbol": "session.create", "confidence": 0.93, "file": "src/session/manager.py", "line": 56},
      {"symbol": "generate_jwt", "confidence": 0.97, "file": "src/auth/jwt.py", "line": 22}
    ],
    "total_edges": 5,
    "graph_token_estimate": 450
  }
}
```

---

#### `search`

Semantic/natural language search across codebase. Use when user mentions concepts without specific symbol names.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "search",
  "params": {
    "query": "email validation before user registration",
    "limit": 5,
    "language_filter": ["python"],
    "kind_filter": ["function", "method"],
    "min_confidence": 0.6
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "result": {
    "results": [
      {
        "qualified_name": "validators.email.validate_format",
        "file": "src/validators/email.py",
        "kind": "function",
        "score": 0.94,
        "snippet": "def validate_format(email: str) → bool: ...",
        "context": "Called by api.users.register before database insert"
      },
      {
        "qualified_name": "api.users.register",
        "file": "src/api/users.py",
        "kind": "function",
        "score": 0.89,
        "snippet": "def register(payload: RegisterPayload) → User: ...",
        "context": "Validates email via validators.email.validate_format"
      }
    ],
    "total_matches": 12,
    "query_expansion": ["email", "validation", "registration", "verify", "format"]
  }
}
```

---

#### `edit_plan`

Submit an edit plan for validation before applying. ShardIndex checks reference integrity and predicts breakage.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "edit_plan",
  "params": {
    "plan_id": "refactor-auth-001",
    "operations": [
      {
        "type": "rename",
        "symbol": "auth.login",
        "new_name": "auth.authenticate",
        "update_refs": true
      },
      {
        "type": "extract",
        "source_symbol": "auth.login",
        "new_symbol": "auth.verify_mfa_step",
        "lines": [55, 67],
        "source_file": "src/auth/login.py"
      }
    ],
    "expected_new_refs": [
      {"caller": "auth.authenticate", "callee": "auth.verify_mfa_step"}
    ],
    "expected_deleted_refs": [
      {"caller": "auth.login", "callee": "verify_mfa"}
    ]
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "result": {
    "valid": false,
    "errors": [
      {
        "severity": "error",
        "message": "Rename 'auth.login' → 'auth.authenticate' breaks 3 external callers not in search scope",
        "affected": ["mobile_app.v2.auth", "legacy.sso.handler", "tests.integration.login"],
        "suggestion": "Use cross_module_move with update_all_refs=true, or add manual overrides"
      },
      {
        "severity": "warning",
        "message": "Extracted symbol 'auth.verify_mfa_step' references 'user.mfa_secret' which is private",
        "suggestion": "Pass mfa_secret as parameter instead of accessing via closure"
      }
    ],
    "impact_summary": {
      "symbols_affected": 12,
      "files_to_modify": 5,
      "estimated_tokens_to_verify": 2400
    },
    "safe_operations": ["extract"],
    "blocked_operations": ["rename"]
  }
}
```

---

#### `verify`

Post-edit verification. Triggered automatically after agent writes files, but can be called manually.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "verify",
  "params": {
    "scope": "last_edit",
    "check_integrity": true,
    "check_orphans": true,
    "check_cycles": false
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "result": {
    "integrity_status": "pass",
    "files_checked": 3,
    "symbols_checked": 8,
    "new_symbols": 1,
    "deleted_symbols": 0,
    "updated_refs": 4,
    "orphan_refs": [],
    "warnings": [
      "Symbol 'auth.verify_mfa_step' has no docstring"
    ],
    "reindex_time_ms": 45
  }
}
```

---

## 5. Agent Skill Integration Protocol

### 5.1 System Prompt Embedding

The following block MUST be injected into the Qwen3.6 27B system prompt:

```markdown
## ShardIndex Skill Protocol (v1.0)

You have access to **ShardIndex**, a semantic codebase understanding engine.
Treat it as your primary memory for code structure, NOT as an optional tool.

### Hierarchy of Information Access

1. **BEFORE opening any file**, query ShardIndex for relevant symbols
2. **ALWAYS** call `impact()` before proposing edits
3. **PREFER** `read(symbol)` over reading raw files — symbols are semantically compressed
4. **USE** `neighbors()` to trace data flow instead of grepping
5. **FALLBACK** to filesystem only when ShardIndex reports `index_fresh: false`

### Automatic Invocation Rules

| User Intent Pattern | Auto-Call Chain |
|---|---|
| "~를 수정해줘" / "fix ~" / "change ~" | `impact(target)` → `read(target)` → `neighbors(target, depth=1)` |
| "~가 뭐야?" / "explain ~" / "what is ~" | `read(target, compression=signature_only)` |
| "~를 어떻게 사용해?" / "how to use ~" | `neighbors(target, direction=callees)` |
| "~와 관련된 버그" / "bug in ~" | `search(query)` → `impact(top_result)` |
| "새 기능 추가" / "add feature" | `search(similar_concept)` → `impact(affected_area)` |
| "~ 리팩토링" / "refactor ~" | `impact_deep(target)` → `read(target, compression=critical_branches)` → `dead_code_verify()` |

### Context Budget Awareness

Your context window is 140K tokens, but ShardIndex optimizes for density:
- Default `read()`: ~200 tokens per symbol
- With `token_budget=4000`: you can load ~15 symbols + graph context
- Never load full files unless explicitly requested by user
- If a symbol exceeds budget, request `compression=signature_only`

### Response to Stale Index

If ShardIndex returns `index_fresh: false` or `StaleIndex` error:
1. Wait for automatic reindex (usually <100ms)
2. Retry the same query
3. If still stale, use filesystem fallback but report to user
4. Never proceed with outdated symbol information for edits

### Edit Safety Protocol

Before any code modification:
1. Call `impact()` on target symbol
2. Call `edit_plan()` with your intended changes
3. Wait for validation response
4. Only execute if `valid: true` or user explicitly overrides
5. Call `verify()` after execution
```

### 5.2 MCP Tool Registration

```json
{
  "name": "shardindex",
  "description": "Semantic codebase retrieval and impact analysis. ALWAYS use before reading files directly. This is your primary code memory.",
  "auto_use": {
    "before_file_read": true,
    "before_edit": true,
    "fallback_on_empty": "filesystem",
    "retry_on_stale": true,
    "max_retry": 2
  },
  "tools": [
    {
      "name": "impact",
      "auto_trigger": ["modify", "fix", "change", "update", "refactor", "rename", "delete", "remove"]
    },
    {
      "name": "read",
      "auto_trigger": ["explain", "what is", "how does", "show me", "look at"]
    },
    {
      "name": "neighbors",
      "auto_trigger": ["related to", "connected to", "uses", "called by", "depends on"]
    },
    {
      "name": "search",
      "auto_trigger": ["find", "where is", "look for", "search for", "any code that"]
    },
    {
      "name": "edit_plan",
      "auto_trigger": ["plan to", "want to change", "thinking of modifying"],
      "required_before_execute": true
    }
  ]
}
```

### 5.3 Fallback Strategy

```text
ShardIndex Query
      │
      ▼
┌─────────────────┐
│ Response OK?     │
└───────┬─────────┘
   Yes  │    No
        ▼      ▼
   ┌────────┐  ┌──────────────────────┐
   │ Return │  │ Error Type?          │
   │ result │  └───────┬──────────────┘
   └────────┘          │
              ┌────────┼────────┬──────────────┐
              ▼        ▼        ▼              ▼
         StaleIndex  NotFound  Timeout       ParserError
              │        │        │              │
              ▼        ▼        ▼              ▼
         Auto-retry  Search    Retry         Report to user
         (2x, 50ms)  fallback  (1x)          + skip file
              │        │        │              │
              ▼        ▼        ▼              ▼
         Still fail?  Result?  Still fail?    Filesystem
              │        │        │              fallback
              ▼        ▼        ▼              + warn user
         Filesystem    Use      Filesystem
         fallback      result   fallback
         + warn user            + warn user
```

---

## 6. File Integrity & Blake3 Verification System

### 6.1 Threat Model

| Threat | Impact | Detection |
|---|---|---|
| Developer edits file outside agent (vim, IDE) | Agent uses stale symbol info | Blake3 hash mismatch on API read |
| Git checkout / branch switch | Mass index invalidation | Batch hash scan on daemon wake |
| Build script generates code (protobuf, ORM) | Generated code missing from index | Watcher detects new files |
| Agent edits file, index update lags | Agent doesn't see its own changes | Synchronous hash update post-edit |
| Disk corruption / bit rot | Wrong AST stored | Hash mismatch + parser error |

### 6.2 Dual-Verification Protocol

Every API call that reads symbol data MUST verify file integrity:

**Phase A: Fast Check (Lazy Verification)**
```rust
fn verify_lazy(file_id: FileId) -> IntegrityResult {
    let stored = db.checksums.get(file_id)?;
    let current = blake3::hash(fs::read(file_path)?);

    if stored.hash == current {
        db.checksums.update_verified_at(file_id);
        return IntegrityResult::Valid;
    }

    // Mismatch detected
    db.dirty_queue.enqueue(file_id, reason::HashMismatch, priority=1);
    db.checksums.record_mismatch(file_id, current);

    IntegrityResult::Stale { 
        file_id, 
        recommendation: "Auto-reindexing queued. Use filesystem fallback or wait 50ms." 
    }
}
```

**Phase B: Synchronous Recovery (Agent-Edited Files)**
```rust
fn verify_sync(file_id: FileId) -> IntegrityResult {
    // Called immediately after agent writes a file
    let current = blake3::hash(fs::read(file_path)?);

    // Force parse NOW, not queue
    let symbols = parser.parse(file_path)?;
    db.transaction(|tx| {
        tx.symbols.mark_stale_by_file(file_id);
        tx.symbols.insert_batch(symbols)?;
        tx.refs.update_for_file(file_id)?;
        tx.checksums.update(file_id, current)?;
        tx.files.update_indexed_at(file_id)?;
    })?;

    IntegrityResult::Valid
}
```

### 6.3 Blake3 Configuration

- **Hash length:** 256-bit (32 bytes, 64-char hex)
- **Chunk size:** Full file (Blake3 parallelizes internally)
- **Performance target:** 1GB/s+ on modern SSD
- **Storage:** Hex string in SQLite (64 chars) — negligible overhead

### 6.4 Emergency Protocol: Mass Invalidation

```rust
fn handle_git_switch_or_bulk_change() {
    // Detected via mtime bulk change or .git/index modification
    let all_files = db.files.get_all();
    let batch_size = 100;

    for batch in all_files.chunks(batch_size) {
        let hashes: Vec<_> = batch.par_iter()
            .map(|f| (f.id, blake3::hash(&fs::read(&f.abs_path))))
            .collect();

        for (file_id, current_hash) in hashes {
            let stored = db.checksums.get(file_id);
            if stored.map(|s| s.hash) != Some(current_hash.clone()) {
                db.dirty_queue.enqueue(file_id, reason::BulkChange, priority=5);
            }
        }
    }

    // Background worker processes queue
    daemon.spawn_background_indexer();
}
```

---

## 7. Incremental Indexing Engine

### 7.1 State Machine

```text
                    ┌─────────┐
         ┌─────────│  Idle   │◄────────┐
         │         └────┬────┘         │
         │              │ File event   │
         │              ▼              │
         │         ┌─────────┐        │
         │         │ Dirty   │        │
         │         └────┬────┘        │
         │              │ Debounce    │
         │              ▼ (50ms)      │
         │         ┌─────────┐        │
         │    ┌────│ Parsing │────┐   │
         │    │    └────┬────┘    │   │
         │    │         │         │   │
         │    │    ┌────┴────┐    │   │
         │    ▼    ▼         ▼    ▼   │
         │ Success          Failure    │
         │    │              │         │
         │    ▼              ▼         │
         │ ┌────────┐    ┌─────────┐   │
         │ │Persist │    │Recover  │───┘
         │ └────┬───┘    └─────────┘
         │      │
         │      ▼
         │ ┌──────────┐
         │ │UpdateRefs│
         └─┤  Graph   │
           └──────────┘
```

### 7.2 Incremental Update Rules

When file `F` changes:

1. **Parse `F`** → new symbols `S_new`, new refs `R_new`
2. **Soft-delete** old symbols in `F`: `UPDATE symbols SET status='stale' WHERE file_id=F`
3. **Soft-delete** old refs from/to symbols in `F`: `UPDATE refs SET is_deleted=1 WHERE file_id=F OR caller_symbol_id IN (old_symbols)`
4. **Insert** `S_new` and `R_new`
5. **Update** `files.blake3_hash`, `indexed_at`, `status='valid'`
6. **Remove** `F` from `dirty_queue`

**Critical:** Never re-parse unchanged files. Never full-table scan.

### 7.3 Performance Targets

| Operation | Target | Max Acceptable |
|---|---|---|
| Single-file incremental reindex | 10–50ms | 100ms |
| 10-file batch (debounced) | 100–300ms | 500ms |
| Cold index (200K LOC) | 10–30s | 120s |
| Hash verification per API call | <1ms | 5ms |
| Symbol lookup by qualified name | <1ms | 3ms |
| Impact graph traversal (depth=2) | <5ms | 20ms |

---

## 8. Parser Abstraction Layer (LanguageBackend)

### 8.1 Trait Definition

```rust
pub trait LanguageBackend: Send + Sync {
    /// Unique identifier for this backend
    fn name(&self) -> &'static str;

    /// File extensions this backend handles
    fn extensions(&self) -> &[&'static str];

    /// Parse a file into symbols
    fn parse_symbols(&self, source: &str, file_id: FileId) -> Result<Vec<Symbol>>;

    /// Extract reference edges from a file
    fn extract_refs(&self, source: &str, file_id: FileId, local_symbols: &[Symbol]) -> Result<Vec<Ref>>;

    /// Slice a specific symbol for semantic compression
    fn slice_symbol(&self, source: &str, symbol: &Symbol, mode: CompressionMode) -> Result<SymbolSlice>;

    /// Estimate token count for a code snippet
    fn estimate_tokens(&self, snippet: &str) -> usize;

    /// Detect if a reference is dynamic (runtime-resolved)
    fn is_dynamic_ref(&self, node: &Self::AstNode) -> bool;
}

pub enum CompressionMode {
    SignatureOnly,      // ~50 tokens
    CriticalBranches,   // ~150 tokens  
    FullBody,           // ~400 tokens
    TokenBudgeted(u32), // Adaptive
}
```

### 8.2 Backend Registry

| Language | Backend | Crate | Status |
|---|---|---|---|
| Python | tree-sitter-python | `tree-sitter-python` | Phase 1 |
| TypeScript | tree-sitter-typescript | `tree-sitter-typescript` | Phase 1 |
| JavaScript | tree-sitter-javascript | `tree-sitter-javascript` | Phase 2 |
| Rust | tree-sitter-rust | `tree-sitter-rust` | Phase 2 |
| Go | tree-sitter-go | `tree-sitter-go` | Phase 3 |
| Java | tree-sitter-java | `tree-sitter-java` | Phase 3 |

### 8.3 Cross-Language References

```rust
// Example: Python Pydantic model ↔ TypeScript API client
pub struct CrossLanguageResolver {
    /// Map of shared interface names (e.g., API schemas)
    symbol_aliases: HashMap<String, Vec<SymbolId>>,
}

// When Python defines `class User(BaseModel)`
// and TypeScript has `interface User { ... }`
// Create a weak ref edge with kind: `cross_language_schema`
```

---

## 9. Refactoring-Specialized APIs

### 9.1 `impact_deep`

Extended impact analysis with transitive dependency tracing and risk scoring.

**Use case:** "이 심볼을 고치면 테스트/운영/레거시 어디까지 터지나?"

**Request:**
```json
{
  "method": "impact_deep",
  "params": {
    "symbol": "payments.process_refund",
    "depth": 3,
    "include_tests": true,
    "include_dynamic": true,
    "risk_analysis": true,
    "token_budget": 3000
  }
}
```

**Response:**
```json
{
  "result": {
    "target": "payments.process_refund",
    "layers": [
      {
        "depth": 1,
        "symbols": ["orders.cancel_order", "admin.refund_manual"],
        "confidence": 0.95,
        "risk": "low"
      },
      {
        "depth": 2,
        "symbols": ["webhooks.handlers.payment_refunded", "notifications.email_refund"],
        "confidence": 0.82,
        "risk": "medium"
      },
      {
        "depth": 3,
        "symbols": ["analytics.track_revenue", "reports.monthly_summary"],
        "confidence": 0.65,
        "risk": "high"
      }
    ],
    "critical_paths": [
      "payments.process_refund → webhooks.handlers.payment_refunded → analytics.track_revenue"
    ],
    "test_coverage_gaps": [
      "analytics.track_revenue has 0 direct tests"
    ],
    "dynamic_refs_at_risk": [
      {"expr": "getattr(payments, 'process_' + action)", "confidence": 0.4, "file": "src/utils/dispatch.py"}
    ],
    "recommendation": "Modify with caution. Add tests for depth-3 symbols before refactoring."
  }
}
```

### 9.2 `dead_code_verify`

Multi-stage verification before deleting a symbol.

**Request:**
```json
{
  "method": "dead_code_verify",
  "params": {
    "symbol": "utils.legacy_hash_password",
    "stages": ["static_refs", "dynamic_refs", "string_refs", "git_history", "test_refs"],
    "min_confidence_for_deletion": 0.95
  }
}
```

**Response:**
```json
{
  "result": {
    "safe_to_delete": false,
    "stages": {
      "static_refs": {"status": "pass", "callers": []},
      "dynamic_refs": {"status": "fail", "matches": ["getattr(auth, 'legacy_hash')"]},
      "string_refs": {"status": "warn", "matches": ["logger.info('Using legacy_hash_password')"]},
      "git_history": {"status": "warn", "last_commit": "2024-01-15", "commit_message": "DEPRECATED: use hash_password_v2"},
      "test_refs": {"status": "pass", "tests": []}
    },
    "blockers": [
      "Dynamic reference in src/auth/fallback.py (confidence: 0.6)",
      "String reference in logging may indicate runtime conditional logic"
    ],
    "suggestion": "Do not delete. Mark as deprecated and monitor for 1 release cycle."
  }
}
```

### 9.3 `cross_module_move`

Safe symbol relocation across module boundaries with automatic ref updating.

**Request:**
```json
{
  "method": "cross_module_move",
  "params": {
    "symbol": "auth.login",
    "target_module": "services.authentication",
    "update_imports": true,
    "update_string_refs": false,
    "dry_run": true
  }
}
```

**Response:**
```json
{
  "result": {
    "dry_run": true,
    "files_to_modify": [
      {"path": "src/auth/login.py", "action": "delete_symbol", "symbol": "auth.login"},
      {"path": "src/services/authentication.py", "action": "insert_symbol", "symbol": "services.authentication.login"},
      {"path": "src/api/users.py", "action": "update_import", "from": "auth.login", "to": "services.authentication.login"},
      {"path": "src/tests/test_auth.py", "action": "update_import", "from": "auth.login", "to": "services.authentication.login"}
    ],
    "unresolved_refs": [
      {"file": "src/legacy/sso.py", "type": "string_import", "value": "from auth import login as old_login"}
    ],
    "estimated_tokens": 1200,
    "safe_to_execute": false,
    "reason": "2 unresolved string imports require manual review"
  }
}
```

### 9.4 `signature_migration_check`

Check if changing a function signature breaks callers.

**Request:**
```json
{
  "method": "signature_migration_check",
  "params": {
    "symbol": "session.create",
    "new_signature": "def create(user_id: int, ttl: int = 3600, device_id: str | None = None) → Session:",
    "check_call_sites": true
  }
}
```

**Response:**
```json
{
  "result": {
    "compatible": false,
    "breaking_callers": [
      {
        "symbol": "auth.login",
        "call_site": "session.create(user.id)",
        "issue": "Missing required positional: device_id is optional (OK), but check default behavior"
      },
      {
        "symbol": "api.mobile.login",
        "call_site": "session.create(user_id, 7200, 'mobile')",
        "issue": "Positional args still match, but type of 3rd arg changed from int to str | None (was probably wrong before)"
      }
    ],
    "safe_callers": 8,
    "breaking_callers": 2,
    "suggestion": "Add device_id as keyword-only with default None to maintain backward compatibility"
  }
}
```

---

## 10. Token Budget & Semantic Compression

### 10.1 Compression Pipeline

```text
Raw Symbol Body (1000 tokens)
         │
         ▼
┌─────────────────────┐
│ 1. Signature Only   │ → 50 tokens
│    (def + params + return) │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│ 2. Critical Branches │ → 150 tokens
│    (if/else, loops,  │
│     error branches)   │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│ 3. Side Effects      │ → +50 tokens
│    (DB, network,     │
│     mutation calls)   │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│ 4. Full Body         │ → 400 tokens
│    (complete impl)   │
└─────────────────────┘
```

### 10.2 Token Budget Negotiation

```json
// Agent declares budget per request
{
  "method": "read",
  "params": {
    "symbol": "auth.login",
    "token_budget": 600,
    "budget_strategy": "prefer_signature_then_critical"
  }
}

// ShardIndex responds with what it could fit
{
  "result": {
    "compression_used": "critical_branches",
    "estimated_tokens": 340,
    "budget_remaining": 260,
    "suggestion": "Use remaining budget for neighbors() to see callers"
  }
}
```

### 10.3 Qwen3.6 27B Context Strategy

With 140K context:
- **System prompt + ShardIndex skill:** ~2K tokens (fixed)
- **Per-query working memory:** ~8K tokens (recommended)
- **Conversation history:** ~30K tokens (rolling)
- **Reserve for reasoning:** ~100K tokens

ShardIndex should aim to fit within the **8K working memory** per turn, leaving headroom for the model's reasoning.

---

## 11. Error Handling & Fallback Strategy

### 11.1 Error Taxonomy

| Error Code | Meaning | Agent Action |
|---|---|---|
| `StaleIndex` | File hash mismatch | Auto-retry 2×, then filesystem fallback |
| `SymbolNotFound` | Symbol not in index | `search()` fallback, then filesystem |
| `ParserError` | File unparseable | Report to user, mark file as `corrupted` |
| `TokenBudgetExceeded` | Symbol too large for budget | Request compression upgrade |
| `RefIntegrityViolation` | `edit_plan` detected breakage | Block edit, show impact |
| `CircularDependency` | Cycle in impact graph | Warn user, truncate at cycle point |
| `CrossLanguageGap` | Ref crosses unsupported language | Return raw string ref with warning |

### 11.2 Filesystem Fallback Protocol

When ShardIndex fails:

```text
1. Attempt grep/ripgrep for symbol name in repo
2. Read top 3 matching files (limited to 200 lines each)
3. Inject warning: "ShardIndex unavailable. Using filesystem fallback. Results may be incomplete."
4. After filesystem read, enqueue file for indexing
5. On next query, ShardIndex should be ready
```

---

## 12. Performance Targets & Benchmarks

### 12.1 Benchmark Suite

```rust
// tests/benchmarks.rs
#[bench]
fn bench_cold_index_200k_python(b: &mut Bencher) {
    // Target: < 30s
}

#[bench]
fn bench_incremental_single_file(b: &mut Bencher) {
    // Target: < 50ms
}

#[bench]
fn bench_impact_depth_2(b: &mut Bencher) {
    // Target: < 5ms
}

#[bench]
fn bench_hash_verify(b: &mut Bencher) {
    // Target: < 1ms
}

#[bench]
fn bench_search_semantic(b: &mut Bencher) {
    // Target: < 10ms
}
```

### 12.2 Scale Targets

| Repository Size | Cold Index | Incremental | Query Latency | Memory (Daemon) |
|---|---|---|---|---|
| 20K LOC | < 5s | < 20ms | < 3ms | ~50MB |
| 50K LOC | < 10s | < 30ms | < 5ms | ~120MB |
| 100K LOC | < 20s | < 50ms | < 8ms | ~250MB |
| 200K LOC | < 30s | < 100ms | < 10ms | ~500MB |
| 500K LOC | < 90s | < 200ms | < 20ms | ~1.2GB |

---

## 13. Implementation Roadmap

### Phase 1 — MVP (Weeks 1–4)

**Goal:** Single language (Python), basic impact analysis, MCP API

| Week | Task | Deliverable |
|---|---|---|
| 1 | SQLite schema + migrations | `schema.sql`, `migrate.rs` |
| 1 | Blake3 hash watcher + checksums | `integrity.rs` |
| 2 | tree-sitter-python integration | `python_backend.rs` |
| 2 | Symbol extraction + storage | `symbols.rs`, `shard_writer.rs` |
| 3 | Reference extraction (direct calls) | `refs.rs` |
| 3 | Incremental update engine | `incremental.rs`, `dirty_queue.rs` |
| 4 | MCP API server (impact, read, neighbors) | `api/mcp.rs` |
| 4 | Agent skill prompt template | `prompts/shardindex_skill_v1.md` |
| 4 | CLI: `shardindex init`, `shardindex daemon` | `cli.rs` |

**Exit Criteria:**
- `shardindex init .` works on Python repo
- `impact("module.function")` returns callers/callees in <10ms
- Agent can query via MCP and get compressed symbol data

### Phase 2 — Robustness (Weeks 5–8)

**Goal:** Multi-file watch, crash recovery, confidence scoring, TypeScript support

| Week | Task | Deliverable |
|---|---|---|
| 5 | Background daemon + state machine | `daemon.rs`, `state.rs` |
| 5 | Crash recovery journal | `recovery.rs` |
| 6 | Confidence scoring for dynamic refs | `confidence.rs` |
| 6 | tree-sitter-typescript backend | `typescript_backend.rs` |
| 7 | Cross-language refs (Python↔TS schemas) | `cross_lang.rs` |
| 7 | `edit_plan` + `verify` APIs | `api/edit.rs` |
| 8 | Agent cache layer | `agent_cache.rs` |
| 8 | Performance benchmark suite | `benches/` |

**Exit Criteria:**
- 200K LOC repo cold index in <30s
- Single-file edit → reindex in <100ms
- `edit_plan` catches 90% of breaking changes

### Phase 3 — Multi-Language (Weeks 9–12)

**Goal:** Rust, Go, JavaScript support, advanced graph queries

| Week | Task |
|---|---|
| 9 | tree-sitter-rust backend |
| 10 | Go native parser bridge |
| 10 | tree-sitter-javascript backend |
| 11 | Graph ranking (PageRank-style symbol importance) |
| 11 | Advanced search (fuzzy + semantic hybrid) |
| 12 | Override registry UI / CLI |

### Phase 4 — Semantic Compression (Weeks 13–16)

**Goal:** Token-budgeted retrieval, adaptive slicing, production optimization

| Week | Task |
|---|---|
| 13 | Token estimation per symbol |
| 13 | Adaptive compression pipeline |
| 14 | `TokenBudgeted` compression mode |
| 14 | Semantic summarization (key logic extraction) |
| 15 | Graph ranking integration with retrieval |
| 15 | Local LLM-specific optimizations (Qwen, Llama, Mistral) |
| 16 | Production telemetry + cost analytics |

---

## 14. Qwen3.6 27B Specific Optimization Notes

### 14.1 Model Characteristics

- **Context:** 140K tokens (generous, but precision still wins)
- **Architecture:** Dense transformer, 27B parameters
- **Local deployment:** Ollama / LM Studio / vLLM
- **Typical throughput:** 20–60 tok/s (depending on quantization)

### 14.2 ShardIndex Optimizations for Qwen3.6 27B

1. **Dense Context Preference**
   - Qwen3.6 excels at reasoning over dense, structured context
   - ShardIndex's `critical_branches` compression aligns perfectly
   - Avoid dumping 50 files; instead provide 10 symbols with graph relationships

2. **System Prompt Budget**
   - Reserve 2K tokens for ShardIndex skill protocol
   - Keep protocol concise but complete (this document's Section 5.1)

3. **Streaming API Responses**
   - Qwen3.6 via Ollama supports streaming
   - ShardIndex should stream large graph responses in chunks
   - Agent can start reasoning while graph loads

4. **Quantization Awareness**
   - If running Q4_K_M, model may miss subtle references
   - ShardIndex's explicit `refs` list compensates for reduced model recall
   - Always include `confidence` scores so agent knows which refs to trust

5. **Local Latency Budget**
   - Ollama on consumer GPU (RTX 4090): ~30 tok/s
   - 8K token response = ~270s generation time
   - ShardIndex query latency (<10ms) is negligible vs generation time
   - **Optimize for token reduction, not query latency** — every 1K tokens saved = 30s faster response

### 14.3 Recommended Agent Configuration

```yaml
# ollama_modelfile snippet
SYSTEM """
You are an expert software engineer with access to ShardIndex, a semantic codebase engine.

## ShardIndex Access Rules
- ALWAYS query ShardIndex before reading files
- Use impact() before edits
- Use read(symbol) instead of read_file(path)
- Use neighbors() for data flow tracing
- Fallback to filesystem only on StaleIndex error

## Context Strategy
- Target 4K-8K tokens of working context per turn
- Prefer symbol-level over file-level
- Trust confidence > 0.9 refs implicitly
- Verify confidence < 0.7 refs manually
"""

PARAMETER temperature 0.2
PARAMETER num_ctx 140000
```

---

## Appendix A: Directory Structure

```text
.shardindex/
├── config.toml              # Daemon configuration
├── daemon.sock              # Unix socket (runtime)
├── sqlite/
│   └── main.db              # Metadata graph (this document's schema)
├── shards/
│   ├── 0001/
│   │   ├── 00042.bin        # Symbol body (LZ4 compressed)
│   │   └── 00043.bin
│   └── 0002/
│       └── ...
├── journals/
│   └── recovery.wal         # Crash recovery journal
├── overrides.yml            # Manual reference overrides
└── logs/
    └── daemon.log
```

## Appendix B: Quick Reference Card

| API | When to Use | Budget Impact |
|---|---|---|
| `impact()` | Before ANY edit | ~200-500 tokens |
| `read()` | Understanding implementation | ~100-400 tokens |
| `neighbors()` | Tracing data flow | ~300-800 tokens |
| `search()` | Finding concepts by name | ~100-300 tokens |
| `edit_plan()` | Validating refactor safety | ~200-500 tokens |
| `verify()` | Post-edit check | ~100 tokens |
| `impact_deep()` | Complex refactoring | ~500-1500 tokens |
| `dead_code_verify()` | Before deletion | ~200-600 tokens |

---

*End of Document*
