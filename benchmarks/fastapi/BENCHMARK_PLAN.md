# ShardIndex Benchmark Plan — FastAPI Codebase

## Goal

**Prove ShardIndex efficiency** by comparing traditional grep-based development vs. ShardIndex-assisted development on the FastAPI codebase (1,118 Python files, ~107K LOC).

## Target: FastAPI

- **Repo:** `https://github.com/tiangolo/fastapi`
- **Files:** 1,118 Python files
- **Lines:** ~107,574
- **Indexed:** ShardIndex SQLite graph (symbols, references, PageRank)

---

## Benchmark Scenarios

Each scenario is run twice: **(A) Traditional grep approach** and **(B) ShardIndex approach**.

### Scenario 1: Symbol Search & Understanding

**Task:** Find all usages of `request` parameter in `fastapi.routing.APIRoute`

| Step | Traditional (grep) | ShardIndex |
|------|-------------------|------------|
| 1. Find symbol | `grep -rn "APIRoute" --include="*.py"` | `shardindex search "APIRoute"` |
| 2. Find usages | `grep -rn "request" --include="*.py"` (manual filter) | `shardindex neighbors "routing.APIRoute"` |
| 3. Read context | `sed -n 'X,Yp' file.py` for each match | `shardindex read "routing.APIRoute" --compression critical_branches` |
| 4. Impact check | Manual grep chain | `shardindex impact "routing.APIRoute"` |

**Metrics:**
- Token count per step (LLM input tokens)
- Number of commands/tool calls
- Time to find all relevant references
- Accuracy: % of relevant references found vs. false positives

---

### Scenario 2: Adding a New Parameter to Existing Function

**Task:** Add `dependencies: list[Depends] = []` to `fastapi.API.__init__`

| Step | Traditional (grep) | ShardIndex |
|------|-------------------|------------|
| 1. Find signature | `grep -n "def __init__" fastapi/api.py` | `shardindex search "API.__init__"` |
| 2. Find callers | `grep -rn "API(" --include="*.py"` | `shardindex neighbors "fastapi.API"` |
| 3. Check impact | Manual review of each caller | `shardindex impact "fastapi.API"` |
| 4. Validate change | Read each file manually | `shardindex edit_plan "fastapi.API" --add-param` |
| 5. Verify | `grep -rn "API(" --include="*.py"` again | `shardindex verify fastapi/api.py` |

**Metrics:**
- Token count for reading all affected files
- Number of missed callers (accuracy)
- Time from start to verified change
- Edit safety: did we break any caller?

---

### Scenario 3: Refactoring — Extract Method

**Task:** Extract dependency resolution logic from `fastapi.routing.solve_dependencies` into a separate utility function

| Step | Traditional (grep) | ShardIndex |
|------|-------------------|------------|
| 1. Find target | `grep -n "solve_dependencies" --include="*.py"` | `shardindex search "solve_dependencies"` |
| 2. Understand scope | Read full function (200+ lines) | `shardindex read "routing.solve_dependencies" --token-budget 4000` |
| 3. Find callers | `grep -rn "solve_dependencies" --include="*.py"` | `shardindex neighbors "routing.solve_dependencies"` |
| 4. Transitive impact | Manual trace through callers | `shardindex impact_deep "routing.solve_dependencies" --depth 3` |
| 5. Plan changes | Mental model + notes | `shardindex edit_plan "routing.solve_dependencies"` |

**Metrics:**
- Context tokens: full file vs. compressed symbol
- Missed transitive dependencies
- Planning time
- Refactoring accuracy (no broken references)

---

### Scenario 4: Dead Code Detection

**Task:** Identify unused imports and functions in `fastapi/params.py`

| Step | Traditional (grep) | ShardIndex |
|------|-------------------|------------|
| 1. List symbols | `grep -n "^def \|^class \|^import" fastapi/params.py` | `shardindex list_file_symbols "fastapi/params.py"` |
| 2. Check usage | `grep -rn "symbol_name" --include="*.py"` per symbol | `shardindex dead_code_verify "params.SomeSymbol"` |
| 3. Verify | Manual cross-check | Multi-stage verification (static + dynamic + string + git + test refs) |

**Metrics:**
- Commands per symbol checked
- False positive rate (flagged as dead but actually used)
- False negative rate (missed actual dead code)
- Total time for full file analysis

---

### Scenario 5: Cross-Module Move

**Task:** Move `fastapi.exceptions.HTTPException` to a new `fastapi/exceptions/http.py` module

| Step | Traditional (grep) | ShardIndex |
|------|-------------------|------------|
| 1. Find symbol | `grep -n "class HTTPException" --include="*.py"` | `shardindex search "HTTPException"` |
| 2. Find all imports | `grep -rn "from.*import.*HTTPException\|import.*HTTPException"` | `shardindex impact "exceptions.HTTPException"` |
| 3. Plan import updates | Manual sed/replace per file | `shardindex cross_module_move "HTTPException" --target "exceptions.http"` |
| 4. Verify | Run tests + grep again | `shardindex verify` + import validation |

**Metrics:**
- Number of import statements found vs. missed
- Token count for planning
- Time to generate complete migration plan
- Accuracy: all import paths correctly updated

---

## Measurement Methodology

### Token Counting

For each scenario, count LLM input tokens:

```
tokens = sum(len(text_embedding(tokenize(cmd_output))) for each command)
```

- **Traditional:** Every `grep` output, `cat` output, `sed` output counts as tokens
- **ShardIndex:** Every MCP tool response (TOON format) counts as tokens
- Compare: Traditional tokens vs. ShardIndex tokens → **Token reduction %**

### Accuracy Measurement

```
accuracy = true_positives / (true_positives + false_negatives)
precision = true_positives / (true_positives + false_positives)
```

- **Ground truth:** Manual verification of all references for each symbol
- **Traditional:** Count grep matches that are relevant vs. irrelevant
- **ShardIndex:** Count MCP results that are relevant vs. irrelevant

### Processing Time

```
time = sum(command_execution_time + LLM_processing_time)
```

- Measure wall clock time from task start to completion
- Include command execution, output reading, and LLM reasoning time
- Compare: Traditional time vs. ShardIndex time → **Speedup factor**

---

## Execution Plan

### Phase 1: Setup (Day 1)

- [x] Clone FastAPI repo
- [x] Build ShardIndex release binary
- [ ] Initialize ShardIndex on FastAPI
- [ ] Verify index quality (symbol count, reference count)
- [ ] Set up token counting infrastructure

### Phase 2: Benchmark Runs (Day 2-3)

- [ ] Run Scenario 1 (Symbol Search) — both approaches
- [ ] Run Scenario 2 (Add Parameter) — both approaches
- [ ] Run Scenario 3 (Extract Method) — both approaches
- [ ] Run Scenario 4 (Dead Code) — both approaches
- [ ] Run Scenario 5 (Cross-Module Move) — both approaches

### Phase 3: Analysis (Day 4)

- [ ] Aggregate token counts across all scenarios
- [ ] Calculate accuracy metrics
- [ ] Compute processing time ratios
- [ ] Generate benchmark results report

### Phase 4: Report (Day 5)

- [ ] Write detailed benchmark results MD
- [ ] Create summary README with key findings
- [ ] Identify ShardIndex advantages and limitations

---

## Expected Outcomes

| Metric | Traditional (grep) | ShardIndex | Expected Improvement |
|--------|-------------------|------------|---------------------|
| Token usage | ~5,000-10,000 per scenario | ~500-2,000 per scenario | **60-80% reduction** |
| Tool calls | 8-15 per scenario | 3-5 per scenario | **60% fewer calls** |
| False positives | 30-50% (grep noise) | <5% (graph-verified) | **10x precision** |
| Missed refs | 10-20% (dynamic refs) | <2% (multi-stage) | **5x recall** |
| Processing time | 5-15 min per scenario | 1-3 min per scenario | **3-5x faster** |

---

## Risk Factors

1. **Dynamic references:** Python's dynamic nature means some refs are invisible to static analysis → ShardIndex uses confidence scoring
2. **String-based refs:** Module names in strings, decorators, metaprogramming → covered by `dead_code_verify` string_refs stage
3. **Index freshness:** If code changes between indexing and querying → Blake3 integrity check + auto-reindex
4. **LLM model variance:** Different models have different tokenization → use consistent model across all runs
