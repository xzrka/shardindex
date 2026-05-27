# ShardIndex — 남은 개발 내용 (Next TODO)

> **생성일:** 2026-05-27
> **상태:** 마스터플랜 v1.3 기준, 실제 코드베이스와 대조
> **테스트:** 526 passed (263 lib + 263 bin + 17 integ + 2 doctest), 0 failed
> **코드:** ~20,129 lines, 40 modules

---

## ✅ 완료된 항목 요약

### Phase 1 — MVP (완료)
- SQLite schema v3 (3 migrations)
- Blake3 integrity guard + checksums
- 18-language tree-sitter parser (Python, JS, TS, Rust, Go, Ruby, Java, PHP, Julia, Lua, Swift, Zig, Scala, Elixir, Dart, Haskell, C, C++)
- Symbol extraction + storage (indexer module)
- Reference extraction with confidence scoring (indexer/types.rs)
- Dirty queue manager + incremental reindex
- MCP API server (impact, read, neighbors, search, stats, edit_plan, verify)
- STDIO MCP transport (mcp/stdio.rs)
- CLI: init, index, search, read, neighbors, impact, override, verify, daemon
- Multi-language indexer (auto-detect)

### Phase 2 — Robustness (완료)
- Background daemon + state machine (daemon.rs — 562 lines)
- Crash recovery journal (recovery.rs — 636 lines)
- Configuration system (config.rs — 475 lines)
- Agent cache layer (agent_cache.rs — 536 lines, 26 tests)
- Advanced search with fuzzy + PageRank (search.rs — 522 lines, 26 tests)
- Edit Plan API (graph/mod.rs — analyze_edit_plan)
- Multi-language watcher (watcher.rs — 522 lines)
- Performance benchmarks (benches/db_bench.rs, benches/parser_bench.rs)

### Phase 3 — Multi-Language (대부분 완료)
- Graph ranking (PageRank) — graph/mod.rs에 compute_pagerank, compute_and_store_ranks 구현
- Advanced search — 완료
- Override registry CLI — 완료

### 추가 구현 (마스터플랜 외)
- Semantic compression (compression.rs — 1,217 lines, CompressionLevel enum, critical_branches, side_effects, key_assignments)
- Token budget enforcement (token_budget.rs — 503 lines, BudgetStrategy, enforce_budget)
- Token estimation (token_estimation.rs — 355 lines, LanguageDensity)
- Deep impact analysis (graph/mod.rs — impact_deep, 다단계 전달 의존성 추적)
- Dead code verification (graph/mod.rs — dead_code_verify)
- Cross-module move analysis (graph/mod.rs — cross_module_move)
- Signature migration check (graph/mod.rs — signature_migration_check)
- TOON format output (format/toon.rs)
- Markdown parser (indexer/markdown.rs)

---

## ⏳ 남은 개발 내용

### Phase 3 — 미완료 항목

#### 3-1. Override Registry UI
- **상태:** DB layer + CLI 완료, UI 미구현
- **내용:**
  - Web UI 또는 TUI로 override registry 시각화
  - Manual reference override의 CRUD를 GUI에서 관리
  - Override 패턴의 유효성 검증 UI
- **우선순위:** 낮음 (CLI로 대체 가능)

#### 3-2. Graph Ranking — Search 통합
- **상태:** PageRank 계산은 완료, search 결과에 PageRank weight 적용은 부분적
- **내용:**
  - search.rs의 `compute_combined_score()`에 PageRank score를 실제 DB에서 읽어와 적용
  - 현재는 `graph_edges()`가 adj list만 반환 — 실제 symbol_rank 테이블과 연동
  - Search 결과 정렬 시 PageRank 기반 importance 반영
- **우선순위:** 중간

### Phase 4 — Semantic Compression (대부분 구현됨, 개선 필요)

#### 4-1. Adaptive Compression Pipeline
- **상태:** compression.rs에 3-level compression (signature_only, critical_branches, full_body) 구현됨
- **남은 작업:**
  - Token budget에 따라 **자동** compression level 선택 로직
  - 현재는 `compress_symbol()`이 수동 호출 — MCP handler에서 budget 기반 자동 적용 필요
  - Compression 결과의 token_count 검증 루프 (budget 초과 시 재압축)
- **우선순위:** 높음

#### 4-2. Semantic Summarization
- **상태:** 미구현
- **내용:**
  - LLM 기반 symbol body 요약 (key logic extraction)
  - 현재 critical_branches는 키워드 기반 (if/for/try/match) — 의미론적 요약 아님
  - Local LLM (Qwen3.6)을 사용하여 symbol body를 2-3문장으로 요약하는 pipeline
  - 요약 결과를 DB에 캐싱 (agent_cache 또는 별도 테이블)
- **우선순위:** 중간

#### 4-3. Local LLM-Specific Optimizations
- **상태:** 미구현
- **내용:**
  - Qwen3.6, Llama, Mistral 등 모델별 최적화
  - 모델의 context window에 맞춘 chunking 전략
  - Quantization level에 따른 confidence threshold 조정
  - Streaming API response 지원 (대용량 graph response를 chunking하여 스트리밍)
- **우선순위:** 중간

#### 4-4. Production Telemetry + Cost Analytics
- **상태:** 미구현
- **내용:**
  - Query latency histogram
  - Cache hit/miss ratio monitoring
  - Token budget 사용량 분석
  - Compression effectiveness metrics
  - Prometheus metrics exporter 또는 simple REST endpoint
- **우선순위:** 낮음

### Phase 5 — 미정의 (마스터플랜에 없음, 권장)

#### 5-1. Cross-Language References
- **상태:** 미구현 (masterplan Phase 2 Week 7 항목)
- **내용:**
  - Python ↔ TypeScript 간 schema/type 참조 추적
  - API boundary detection (REST endpoint ↔ frontend caller)
  - JSON schema ↔ TypeScript interface 매핑
- **우선순위:** 낮음

#### 5-2. Agent Skill Prompt Template
- **상태:** 미구현 (masterplan Phase 1 Week 4 항목)
- **내용:**
  - `prompts/shardindex_skill_v1.md` 파일 생성
  - Masterplan §5.1의 시스템 프롬프트 템플릿 구현
  - Agent가 ShardIndex를 자동으로 호출하는 규칙 정의
- **우선순위:** 중간

#### 5-3. Shard Writer (Symbol body persistence)
- **상태:** 미구현 (masterplan Phase 1 Week 2 항목)
- **내용:**
  - `.shardindex/shards/{file_id}/{symbol_id}.bin` LZ4-compressed symbol body 저장
  - 현재 DB에 metadata만 저장 — 실제 body 파일 persistence 없음
  - `shard_path` column은 schema에 정의되어 있지만 실제 사용 안함
- **우선순위:** 낮음

#### 5-4. Streaming MCP Response
- **상태:** 미구현
- **내용:**
  - 대용량 graph response를 JSON-RPC notification으로 chunking
  - Agent가 첫 chunk를 받자마자 reasoning 시작 가능
  - 현재는 전체 response를 한 번에 전송
- **우선순위:** 중간

#### 5-5. Unix Domain Socket Support
- **상태:** 미구현 (masterplan §4.1에 명시)
- **내용:**
  - 현재는 TCP (`localhost:port`) + STDIO만 지원
  - `.shardindex/daemon.sock` Unix socket 추가
  - TCP fallback 유지
- **우선순위:** 낮음

#### 5-6. LZ4 Response Compression
- **상태:** 미구현 (masterplan §4.1에 명시)
- **내용:**
  - 대용량 MCP response에 LZ4 compression 적용
  - `compression` header negotiation
- **우선순위:** 낮음

---

## 📊 우선순위 기반 작업 목록

### 🔴 높음 (즉시 구현 권장)
1. **Adaptive Compression Pipeline** — budget 기반 자동 compression level 선택 + 재압축 루프
2. **Agent Skill Prompt Template** — `prompts/shardindex_skill_v1.md` 생성
3. **Streaming MCP Response** — 대용량 graph chunking

### 🟡 중간 (다음 사이클)
4. **Graph Ranking — Search 통합** — PageRank를 search 결과에 실제 적용
5. **Semantic Summarization** — LLM 기반 symbol 요약 pipeline
6. **Local LLM-Specific Optimizations** — 모델별 chunking + streaming
7. **Cross-Language References** — Python↔TS schema 추적

### 🟢 낮음 (향후 고려)
8. **Override Registry UI** — CLI로 대체 가능
9. **Production Telemetry** — metrics exporter
10. **Shard Writer** — symbol body file persistence
11. **Unix Domain Socket** — TCP로 대체 가능
12. **LZ4 Response Compression** — 대용량 response 한정

---

## 📏 현재 상태 요약

| 항목 | 상태 |
|---|---|
| `cargo check` | ✅ 0 errors, 48 warnings |
| `cargo test` | ✅ 526 passed, 0 failed |
| Schema version | v3 (3 migrations) |
| Parser languages | 18 (all done) |
| MCP methods | 14 (read, neighbors, impact, search, stats, edit_plan, verify, impact_deep, dead_code_verify, cross_module_move, signature_migration_check, health) |
| REST endpoints | 3 (stats, search, neighbors) |
| Benchmark | ✅ 2 (db, parser) |
| Config system | ✅ TOML-based |
| Daemon | ✅ state machine + watcher |
| Recovery | ✅ WAL journal |
| Agent cache | ✅ TTL-based |
| Compression | ✅ 3-level (signature_only, critical_branches, full_body) |
| Token budget | ✅ enforcement + estimation |
