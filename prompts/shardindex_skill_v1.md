---
name: shardindex-agent
description: "Use ShardIndex MCP tools for semantic code search, impact analysis, and symbol graph queries."
version: "1.0.0"
---

# ShardIndex Agent Skill

## When to Use

Use ShardIndex MCP tools whenever you need deep codebase understanding:
- **Finding symbol definitions** — "Where is `X` defined?"
- **Impact analysis** — "If I change `X`, what breaks?"
- **Call graph navigation** — "Who calls `X`? What does `X` call?"
- **Symbol search** — "Find all functions matching pattern `*auth*`"
- **Codebase stats** — "How many files/symbols/references in the index?"

## Prerequisites

ShardIndex daemon must be running:
```bash
cd /home/kali/shardindex
./target/debug/shardindex daemon -p /path/to/project --db /path/to/project/.shardindex.db --listen 127.0.0.1:3000
```

Or use CLI directly (no daemon needed):
```bash
./target/debug/shardindex search <query> --db /path/to/project/.shardindex.db
./target/debug/shardindex neighbors <symbol> --db /path/to/project/.shardindex.db
./target/debug/shardindex impact <symbol> --db /path/to/project/.shardindex.db
```

## MCP Methods

### `read` — Read a symbol definition

Returns full source text and doc comment for a symbol.

```json
{ "method": "read", "params": { "symbol": "MyFunction" } }
```

Use when you need the actual implementation of a function, struct, class, etc.

### `neighbors` — Call graph neighbors

Returns callers and callees within `depth` hops (default: 1).

```json
{ "method": "neighbors", "params": { "symbol": "MyFunction", "depth": 2 } }
```

Use when tracing data flow, understanding dependencies, or planning refactors.

### `impact` — Impact analysis

Analyzes what symbols/files are affected by a proposed edit.

```json
{ "method": "impact", "params": { "symbol": "MyFunction" } }
```

Use before making changes to understand ripple effects.

### `search` — Full-text symbol search

Searches symbol names, signatures, and doc comments.

```json
{ "method": "search", "params": { "query": "authenticate", "limit": 10 } }
```

Use to find symbols by name pattern or description.

### `stats` — Index statistics

Returns file count, symbol count, reference count, and language breakdown.

```json
{ "method": "stats", "params": {} }
```

Use to understand the scope of the indexed codebase.

## Workflow Patterns

### Pattern 1: Understanding a function before editing

1. `read(symbol="TargetFunc")` — Get the definition
2. `neighbors(symbol="TargetFunc", depth=1)` — See who calls it and what it calls
3. `impact(symbol="TargetFunc")` — Understand ripple effects

### Pattern 2: Finding where to make a change

1. `search(query="keyword", limit=5)` — Find candidate symbols
2. `read(symbol="Candidate")` — Inspect each
3. `neighbors(symbol="Candidate", depth=1)` — Check coupling

### Pattern 3: Refactoring planning

1. `stats()` — Understand codebase scale
2. `search(query="old_name", limit=20)` — Find all instances
3. For each: `impact(symbol=...)` — Assess change cost

## Tips

- Always check `impact` before suggesting code changes to symbols with many dependents
- Use `neighbors` with `depth=2` for broader context, `depth=1` for focused view
- Cache is enabled by default (5min TTL) — repeated queries are fast
- 18 languages supported: Python, JavaScript, Rust, TypeScript, Go, Ruby, Java, PHP, Julia, Lua, Swift, Zig, Scala, Elixir, Dart, Haskell, C, C++
