# ShardIndex

**Semantic code graph index — AST-powered middleware for AI coding agents.**

ShardIndex sits between a codebase and an LLM agent, transforming file-level, grep-based workflows into **symbol-level, graph-aware, token-budgeted** interactions. It parses source code using tree-sitter, builds a SQLite-backed metadata graph of symbols and references, and exposes an MCP (Model Context Protocol) server for AI agents to query.

## Key Features

- **26-language support** — Python, JavaScript/JSX, TypeScript/TSX, Rust, Go, Ruby, Java, PHP, Julia, Lua, Swift, Zig, Scala, Elixir, Dart, Haskell, C, C++, Markdown, SQL, GraphQL, Vue, CSS/SCSS, Bash, Kotlin, C#
- **Symbol-level indexing** — Extract functions, classes, methods, traits, interfaces, enums, structs, etc. with full signatures and docstrings
- **Reference graph** — Caller/callee relationships with confidence scoring for dynamic references
- **Impact analysis** — Determine all symbols affected before making edits (shallow and deep/transitive)
- **Semantic compression** — Token-budgeted output with critical branches, side effects, and key assignments
- **TOON format** — Token-Oriented Object Notation reduces LLM context usage by 14–62% compared to JSON
- **Blake3 integrity** — File hash verification with auto-dirty detection and reindexing
- **MCP server** — JSON-RPC 2.0 stdio transport for direct integration with AI agents (Hermes Agent, Claude, etc.)
- **Incremental indexing** — File watcher-based reindexing via dirty queue with priority scheduling
- **Agent cache** — TTL-based query result cache with hash invalidation
- **PageRank ranking** — Symbol importance scoring based on reference graph centrality

## Architecture

```
Source Code (26 languages)
        │
        ▼
┌──────────────────┐
│  tree-sitter     │  AST parsing → symbol & reference extraction
│  parsers         │
└────────┬─────────┘
         ▼
┌──────────────────┐
│  Integrity Guard  │  Blake3 hash verification, auto-dirty on mismatch
└────────┬─────────┘
         ▼
┌──────────────────┐
│  SQLite Graph DB  │  files, symbols, references, checksums, cache, overrides
│  (Schema v4)      │
└────────┬─────────┘
         ▼
┌──────────────────┐
│  MCP Server       │  JSON-RPC 2.0 over stdio (or HTTP)
│  (JSON-RPC 2.0)   │
└────────┬─────────┘
         ▼
   AI Agent (Qwen, Claude, etc.)
```

## Quick Start

### Build

```bash
cargo build --release
```

Release binary: ~40MB.

### Initialize an index

```bash
# Auto-detect all supported languages
./target/release/shardindex init -p /path/to/project

# Specific language
./target/release/shardindex init -p /path/to/project -l python

# Custom DB path
./target/release/shardindex init -p /path/to/project --db ./my_index.db
```

### Query (CLI)

```bash
# Search symbols (fuzzy matching + PageRank scoring)
shardindex search "auth login" --limit 10

# Show neighbors (callers/callees) of a symbol
shardindex neighbors auth.login

# Impact analysis — what breaks if I change this symbol?
shardindex impact auth.login

# Deep impact (transitive dependencies, depth=3)
shardindex impact-deep auth.login

# Read a symbol with semantic compression
shardindex read auth.login --compression critical_branches

# Symbol ranking (PageRank)
shardindex rank --top 10

# Index statistics
shardindex stats

# DOT graph output
shardindex graph auth.login --output graph.dot
```

### Output Formats

All query commands support `--format` flag:

- `text` — Human-readable (default)
- `json` — Standard JSON
- `toon` — TOON format (LLM-optimized, 14–62% smaller than JSON)

### MCP Server

Start the MCP stdio server for AI agent integration:

```bash
shardindex mcp-server --db .shardindex.db --cache-ttl 300
```

Configure in your AI agent's MCP settings. Supported tools:

| Tool | Description |
|------|-------------|
| `stats` | Index statistics (files, symbols, references) |
| `search` | Fuzzy symbol search with PageRank scoring |
| `list_file_symbols` | List all symbols in a file |
| `neighbors` | Show caller/callee graph for a symbol |
| `impact` | Impact analysis — symbols affected by changes |
| `edit_plan` | Pre-edit impact validation |
| `verify` | Blake3 file integrity verification |
| `impact_deep` | Deep transitive impact analysis (BFS, configurable depth) |
| `dead_code_verify` | Multi-stage dead code detection (static/dynamic/string/git/test refs) |
| `cross_module_move` | Cross-module move analysis with import update plan |
| `signature_migration_check` | Signature compatibility check — detect breaking changes |

### System Prompt Integration

Embed ShardIndex into your AI agent's system prompt for automatic codebase awareness. See `prompts/shardindex_skill_v1.md` for the full skill template.

#### Agent Skill Protocol

Inject this block into your agent's system prompt when ShardIndex is available:

```markdown
## ShardIndex Skill Protocol

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
| "fix ~" / "change ~" | `impact(target)` → `read(target)` → `neighbors(target, depth=1)` |
| "explain ~" / "what is ~" | `read(target, compression=signature_only)` |
| "how to use ~" | `neighbors(target, direction=callees)` |
| "bug in ~" | `search(query)` → `impact(top_result)` |
| "refactor ~" | `impact_deep(target)` → `read(target, compression=critical_branches)` → `dead_code_verify()` |

### Context Budget Awareness

- Default `read()`: ~200 tokens per symbol
- With `token_budget=4000`: you can load ~15 symbols + graph context
- Never load full files unless explicitly requested by user
- If a symbol exceeds budget, request `compression=signature_only`

### Edit Safety Protocol

Before any code modification:
1. Call `impact()` on target symbol
2. Call `edit_plan()` with your intended changes
3. Wait for validation response
4. Only execute if `valid: true` or user explicitly overrides
5. Call `verify()` after execution
```

#### Workflow Patterns

**Pattern 1: Understanding a function before editing**

1. `impact(symbol="TargetFunc")` — Understand ripple effects FIRST
2. `list_file_symbols(file="path/to/file.py")` — Get all symbols in the file
3. `neighbors(symbol="TargetFunc", depth=1)` — See who calls it and what it calls

**Pattern 2: Finding where to make a change**

1. `search(query="keyword", limit=5)` — Find candidate symbols
2. `list_file_symbols(file="candidate_file.py")` — Inspect file symbols
3. `neighbors(symbol="Candidate", depth=1)` — Check coupling

**Pattern 3: Refactoring planning**

1. `stats()` — Understand codebase scale
2. `search(query="old_name", limit=20)` — Find all instances
3. For each: `impact(symbol=...)` — Assess change cost
4. `edit_plan(symbol=..., changes=[...])` — Validate before executing

### Daemon (file watching)

```bash
shardindex daemon -p /path/to/project --listen 127.0.0.1:3999 --poll-interval 2
```

Watches for file changes and updates the index automatically.

## Performance Benchmarks

Measured on Linux (Rust 1.95.0, criterion 0.5):

### Database

| Benchmark | Time |
|-----------|------|
| Single symbol insert | 9.5 µs |
| Batch insert (100 symbols) | 1.16 ms |
| Search by pattern | 29.5 µs |
| Ranked search | 38.8 µs |
| Neighbors lookup | 10.0 µs |
| Cache set/get | ~10 µs |

### Parser

| Language | Time (per function) |
|----------|---------------------|
| Python | 11.3 µs |
| JavaScript | 40.2 µs |
| Rust | 96.8 µs |
| Go | 94.0 µs |
| TypeScript | 180.8 µs |

### TOON vs JSON Size Reduction

| Query Type | JSON | TOON | Reduction |
|------------|------|------|-----------|
| Single symbol (critical_branches) | 2,069 B | 1,776 B | 14.2% |
| Search (20 results) | 8,298 B | 4,719 B | 43.1% |
| Neighbors (19 refs) | 2,570 B | 1,020 B | 60.3% |
| Impact (19 symbols) | 5,074 B | 1,912 B | 62.3% |
| Rank (top 20) | 3,463 B | 1,388 B | 59.9% |

## Tests

```bash
cargo test
```

| Suite | Passed | Failed |
|-------|--------|--------|
| lib (unit) | 303 | 0 |
| bin (unit) | 283 | 0 |
| integration | 17 | 0 |
| doc-tests | 2 | 0 |
| **Total** | **605** | **0** |

## CLI Commands

| Command | Description |
|---------|-------------|
| `init` | Initialize index for a project |
| `daemon` | Start file watcher + JSON-RPC server |
| `reindex` | Re-index all files |
| `stats` | Show index statistics |
| `search` | Fuzzy symbol search |
| `read` | Read symbol with semantic compression |
| `neighbors` | Show caller/callee graph |
| `impact` | Impact analysis |
| `impact-deep` | Deep transitive impact analysis |
| `graph` | Generate DOT graph |
| `rank` | Symbol ranking (PageRank) |
| `override` | Manage manual reference overrides |
| `verify` | Verify file integrity (Blake3) |
| `mcp-server` | Start MCP stdio server |
| `dead-code-verify` | Dead code detection |
| `cross-module-move` | Cross-module move analysis |
| `signature-migration-check` | Signature compatibility check |

## Supported Languages

**26 languages** supported via tree-sitter parsers:

| # | Language | Extensions |
|---|----------|-----------|
| 1 | Python | `.py` |
| 2 | JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` |
| 3 | TypeScript | `.ts`, `.tsx`, `.mts`, `.cts` |
| 4 | Rust | `.rs` |
| 5 | Go | `.go` |
| 6 | Ruby | `.rb`, `.gemspec` |
| 7 | Java | `.java` |
| 8 | PHP | `.php` |
| 9 | Julia | `.jl` |
| 10 | Lua | `.lua` |
| 11 | Swift | `.swift` |
| 12 | Zig | `.zig` |
| 13 | Scala | `.scala` |
| 14 | Elixir | `.ex`, `.exs` |
| 15 | Dart | `.dart` |
| 16 | Haskell | `.hs`, `.lhs` |
| 17 | C | `.c`, `.h` |
| 18 | C++ | `.cpp`, `.hpp`, `.cc`, `.cxx` |
| 19 | Markdown | `.md` |
| 20 | SQL | `.sql` |
| 21 | GraphQL | `.graphql`, `.gql` |
| 22 | Vue | `.vue` |
| 23 | CSS/SCSS | `.css`, `.scss`, `.sass` |
| 24 | Bash | `.sh`, `.bash` |
| 25 | Kotlin | `.kt`, `.kts` |
| 26 | C# | `.cs` |

## SQLite Schema

**Version 4** — Tables: `project`, `files`, `symbols`, `references`, `checksums`, `dirty_queue`, `versions`, `symbol_rank`, `agent_cache`, `overrides`

Key indexes: qualified name (unique), file path (unique), Blake3 hash, symbol kind, reference caller/callee, dirty queue priority.

## Dependencies

- **Rust** 1.80+ (Rust 2024 edition)
- **tree-sitter** 0.25 + language grammars
- **SQLite** via rusqlite 0.35 (bundled)
- **Blake3** for file integrity
- **tokio** + **axum** for async MCP server
- **clap** for CLI
- **toon-format** for LLM-optimized output

## License

MIT
