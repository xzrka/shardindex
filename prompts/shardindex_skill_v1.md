---
name: shardindex-agent
description: "Use ShardIndex MCP tools for semantic code search, impact analysis, and symbol graph queries."
version: "1.1.0"
---

# ShardIndex Agent Skill Protocol (v1.1)

## System Prompt Embedding

The following block MUST be injected into the agent system prompt when ShardIndex is available:

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

## MCP Tool Registration

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
      "name": "search",
      "auto_trigger": ["explain", "what is", "how does", "show me", "look at"]
    },
    {
      "name": "neighbors",
      "auto_trigger": ["related to", "connected to", "uses", "called by", "depends on"]
    },
    {
      "name": "list_file_symbols",
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

## Fallback Strategy

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

## Available MCP Tools (Stdio)

### `stats` — Index statistics
Returns file count, symbol count, reference count, and language breakdown.

```json
{ "method": "stats", "params": {} }
```

### `search` — Full-text symbol search
Searches symbol names, signatures, and doc comments with fuzzy + PageRank hybrid scoring.

```json
{ "method": "search", "params": { "query": "authenticate", "limit": 10 } }
```

### `list_file_symbols` — List symbols in a file
Returns all symbols in a file with names, kinds, line ranges, and signatures.

```json
{ "method": "list_file_symbols", "params": { "file": "path/to/file.py" } }
```

### `neighbors` — Call graph neighbors
Returns callers and callees within `depth` hops (default: 1).

```json
{ "method": "neighbors", "params": { "symbol": "MyFunction", "depth": 2 } }
```

### `impact` — Impact analysis
Analyzes what symbols/files are affected by a proposed edit.

```json
{ "method": "impact", "params": { "symbol": "MyFunction" } }
```

### `edit_plan` — Pre-edit impact analysis
Analyze planned changes (rename, add_param, remove_param, change_return) before execution.

```json
{
  "method": "edit_plan",
  "params": {
    "symbol": "MyFunction",
    "changes": [{ "type": "rename", "details": { "new_name": "NewName" } }]
  }
}
```

### `verify` — BLAKE3 file integrity verification
Verify file integrity against stored BLAKE3 hashes.

```json
{ "method": "verify", "params": { "file": "path/to/file.py" } }
```

## When to Use

Use ShardIndex MCP tools whenever you need deep codebase understanding:
- **Finding symbol definitions** — "Where is `X` defined?"
- **Impact analysis** — "If I change `X`, what breaks?"
- **Call graph navigation** — "Who calls `X`? What does `X` call?"
- **Symbol search** — "Find all functions matching pattern `*auth*`"
- **Codebase stats** — "How many files/symbols/references in the index?"
- **Refactoring planning** — "Can I safely rename `X`?"
- **File integrity** — "Has this file changed since indexing?"

## Workflow Patterns

### Pattern 1: Understanding a function before editing

1. `impact(symbol="TargetFunc")` — Understand ripple effects FIRST
2. `list_file_symbols(file="path/to/file.py")` — Get all symbols in the file
3. `neighbors(symbol="TargetFunc", depth=1)` — See who calls it and what it calls

### Pattern 2: Finding where to make a change

1. `search(query="keyword", limit=5)` — Find candidate symbols
2. `list_file_symbols(file="candidate_file.py")` — Inspect file symbols
3. `neighbors(symbol="Candidate", depth=1)` — Check coupling

### Pattern 3: Refactoring planning

1. `stats()` — Understand codebase scale
2. `search(query="old_name", limit=20)` — Find all instances
3. For each: `impact(symbol=...)` — Assess change cost
4. `edit_plan(symbol=..., changes=[...])` — Validate before executing

## Tips

- Always check `impact` before suggesting code changes to symbols with many dependents
- Use `neighbors` with `depth=2` for broader context, `depth=1` for focused view
- Cache is enabled by default (5min TTL) — repeated queries are fast
- 19 languages supported: Python, JavaScript, Rust, TypeScript, Go, Ruby, Java, PHP, Julia, Lua, Swift, Zig, Scala, Elixir, Dart, Haskell, C, C++, Markdown
- TOON format available via `--format toon` — 53.6% token saving vs JSON
- Use `token_budget` param on any MCP tool to enforce context limits
- Cross-language references (`cross_language_schema`) are stored with confidence scoring [0.1, 0.9]
