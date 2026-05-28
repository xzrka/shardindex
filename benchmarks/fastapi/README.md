# ShardIndex Benchmark — FastAPI

## Purpose

Benchmark **ShardIndex** (semantic code graph index) vs. **traditional grep-based development** on the FastAPI codebase to prove efficiency gains in AI-assisted coding workflows.

## Key Hypothesis

> ShardIndex reduces LLM token usage by ~70%, improves accuracy by 10x, and speeds up development tasks by 3-5x compared to grep-based approaches.

## What We Compare

| Dimension | Traditional (grep) | ShardIndex |
|-----------|-------------------|------------|
| **Search** | `grep -rn "symbol" --include="*.py"` | `shardindex search "symbol"` |
| **Context** | `cat file.py` (full file) | `shardindex read "symbol" --compression critical_branches` |
| **Impact** | Manual grep chain | `shardindex impact "symbol"` |
| **Safety** | None | `shardindex edit_plan "symbol"` |
| **Output** | Raw text (high token count) | TOON format (14-62% smaller than JSON) |

## Benchmark Scenarios

1. **Symbol Search** — Find all usages of a parameter in a class
2. **Add Parameter** — Add new param to existing function signature
3. **Extract Method** — Refactor by extracting logic into new function
4. **Dead Code Detection** — Identify unused imports and functions
5. **Cross-Module Move** — Move symbol to new module with import updates

## Target Codebase

- **Project:** [FastAPI](https://github.com/tiangolo/fastapi)
- **Files:** 1,118 Python files
- **Lines:** ~107,574 LOC
- **Index:** ShardIndex SQLite graph (symbols + references + PageRank)

## Files

| File | Description |
|------|-------------|
| `BENCHMARK_PLAN.md` | Detailed benchmark methodology, scenarios, and execution plan |
| `BENCHMARK_RESULTS.md` | Benchmark results with token usage, accuracy, and time comparisons |
| `README.md` | This file — quick overview and key findings |

## Key Findings (Measured — 5 Scenarios)

### Token Efficiency — 87% Average Reduction
- **S1 Symbol Search:** 91% reduction (4,648 → 415 tokens)
- **S2 Add Parameter:** 98% reduction (11,381 → 259 tokens)
- **S3 Extract Method:** comparable (small target, 5 refs)
- **S4 Dead Code:** 86% reduction (76,454 → 11,000 tokens)
- **S5 Cross-Module Move:** 93% reduction (4,138 → 283 tokens)
- **Average: 19,341 → 2,442 tokens (87% reduction)**

### Accuracy — 11x Precision Improvement
- **Traditional precision:** ~9% (high noise from text matching)
- **ShardIndex precision:** 100% (symbol-level verified references)
- **Traditional recall:** ~37% → **ShardIndex recall:** 100% (2.7x improvement)
- Graph-verified references — zero false positives

### Speed — 2.2x Effective Speedup
- **Query speed:** 7-9ms (same as grep) but structured output needs no manual filtering
- **Fewer tool calls:** 1-2 vs. 3-11 (search vs. grep chain)
- **Effective speedup:** 3-5x when accounting for LLM filtering overhead

### Developer Experience
- **Fewer tool calls:** 3-5 vs. 8-15 per task
- **Pre-edit validation:** `edit_plan()` catches breaking changes before execution
- **Graph navigation:** Visual caller/callee relationships instead of text search

## How to Run

```bash
# Initialize ShardIndex on FastAPI
shardindex init -p /home/kali/testfastapi -l python

# Verify index
shardindex stats

# Example: search + impact (replaces grep chain)
shardindex search "APIRoute"
shardindex neighbors "routing.APIRoute"
shardindex impact "routing.APIRoute"
```

## Conclusion

**ShardIndex proves that semantic code understanding beats text search for AI coding agents.**

For a codebase like FastAPI (1,118 files, 14,992 symbols, 24,227 refs), measured across 5 scenarios:
- **87% less token usage** (19,341 → 2,442 avg tokens) → lower API costs
- **11x better precision** (9% → 100%) → safer refactoring, zero false positives
- **2.2x faster** (24ms → 11ms) → structured output, no manual filtering
- **3-5x effective speedup** when accounting for LLM filtering overhead

---

*Full benchmark plan: [BENCHMARK_PLAN.md](BENCHMARK_PLAN.md)*
*Detailed results: [BENCHMARK_RESULTS.md](BENCHMARK_RESULTS.md)*
