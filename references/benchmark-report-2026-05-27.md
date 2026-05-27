# ShardIndex Benchmark Report — 2026-05-27

## 환경 정보

- **OS**: Linux 7.0.0-15-generic (Kali)
- **Rust**: 1.95.0
- **Cargo**: release profile [optimized]
- **Binary**: target/release/shardindex (40MB)
- **프로젝트**: shardindex 자체 (45 Rust + 11 Markdown 파일)
- **인덱스**: 56 files, 1,342 symbols, 8,053 references

---

## 단위 테스트

| 테스트 스위트 | 통과 | 실패 | 무시 |
|--------------|------|------|------|
| lib (unittests) | 263 | 0 | 0 |
| bin (unittests) | 263 | 0 | 0 |
| integration | 17 | 0 | 0 |
| doc-tests | 2 | 0 | 1 |
| **총계** | **545** | **0** | **2** |

---

## 데이터베이스 벤치마크 (criterion 0.5)

| 벤치마크 | 시간 | 샘플 | 아웃라이어 |
|---------|------|------|----------|
| database/insert/single_symbol | 9.49 us | 540k | 11% |
| database/insert/batch_symbols_100 | 1.16 ms | 5,050 | 10% |
| database/search/search_by_pattern | 29.46 us | 182k | 11% |
| database/search/search_ranked | 38.84 us | 131k | 4% |
| database/neighbors/neighbors_single | 10.02 us | 520k | 7% |
| database/cache/cache_set | 10.70 us | 449k | 4% |
| database/cache/cache_get | 10.26 us | 495k | 2% |

---

## 파서 벤치마크 (criterion 0.5)

### Python

| 벤치마크 | 시간 | 샘플 | 아웃라이어 |
|---------|------|------|----------|
| parser/python/small_function | 11.26 us | 460k | 5% |
| parser/python/medium_function | 73.82 us | 71k | 3% |
| parser/python/large_function | 174.70 us | 30k | 4% |
| parser/python/class_definition | 131.72 us | 40k | 4% |

### JavaScript

| 벤치마크 | 시간 | 샘플 | 아웃라이어 |
|---------|------|------|----------|
| parser/javascript/function | 40.21 us | 131k | 11% |
| parser/javascript/class | 115.77 us | 45k | 8% |

### Rust

| 벤치마크 | 시간 | 샘플 | 아웃라이어 |
|---------|------|------|----------|
| parser/rust/function | 96.82 us | 56k | 13% |
| parser/rust/struct_with_impl | 82.84 us | 61k | 8% |

### TypeScript

| 벤치마크 | 시간 | 샘플 | 아웃라이어 |
|---------|------|------|----------|
| parser/typescript/interface_and_class | 180.79 us | 30k | 11% |

### Go

| 벤치마크 | 시간 | 샘플 | 아웃라이어 |
|---------|------|------|----------|
| parser/go/function | 94.01 us | 56k | 14% |

### 언어별 비교 (comparison)

| 벤치마크 | 시간 | 샘플 | 아웃라이어 |
|---------|------|------|----------|
| parser/comparison/python | 33.36 us | 152k | 8% |
| parser/comparison/javascript | 36.47 us | 136k | 4% |
| parser/comparison/rust | 48.70 us | 106k | 10% |
| parser/comparison/typescript | 84.06 us | 61k | 12% |
| parser/comparison/go | 45.12 us | 116k | 14% |

---

## TOON 포맷 벤치마크

### JSON vs TOON 크기 비교 (바이트)

| 명령어 | JSON | TOON | 감소율 |
|-------|------|------|-------|
| read single (critical_branches) | 2,069 | 1,776 | 14.2% |
| search 20 results | 8,298 | 4,719 | 43.1% |
| neighbors (19 refs) | 2,570 | 1,020 | 60.3% |
| impact (19 symbols) | 5,074 | 1,912 | 62.3% |
| rank top 20 | 3,463 | 1,388 | 59.9% |
| search 10 results | 4,020 | 2,249 | 44.1% |
| read single (signature_only) | 588 | 503 | 14.5% |

### TOON 포맷 테스트 시나리오 (34개)

| # | 시나리오 | 상태 |
|---|---------|------|
| 1 | read single symbol | OK |
| 2 | search multiple results (10) | OK |
| 3 | neighbors (callers/callees) | OK |
| 4 | impact analysis | OK |
| 5 | rank (PageRank) | OK |
| 6 | impact-deep | OK |
| 7 | compression: signature_only | OK |
| 8 | compression: full_body | OK |
| 9 | cross-language: do_parse (19 parsers) | OK |
| 10 | compression: token budget (50) | OK |
| 11 | empty search result | OK |
| 12 | search with kind filter | OK |
| 13 | search with language filter | OK |
| 14 | read qualified name | OK |
| 15 | neighbors multiple refs (19 callers) | OK |
| 16 | impact with rank data (19 impacted) | OK |
| 17 | JSON vs TOON token 비교 | OK |
| 18 | MCP server tools/list | OK |
| 19 | MCP server search | OK |
| 20 | MCP server neighbors | OK |
| 21 | MCP server impact | OK |
| 22 | MCP server stats | OK |
| 23 | read nonexistent symbol (error) | OK |
| 24 | search with min-score (0.6) | OK |
| 25 | compression levels 비교 | OK |
| 26 | cross-language kind filter (method) | OK |
| 27 | MCP server list_file_symbols | OK |
| 28 | MCP server edit_plan (rename) | OK |
| 29 | MCP server verify (BLAKE3) | OK |
| 30 | rank with damping (0.9) | OK |
| 31 | search with LIKE mode | OK |
| 32 | impact-deep with depth (1) | OK |
| 33 | read markdown symbol | OK |
| 34 | JSON vs TOON 크기 벤치마크 | OK |

### TOON 포맷 특성

- **배열 데이터가 많을수록 큰 효율** (search, impact, rank)
- **반복되는 JSON 키 제거**로 토큰 절약
- **LLM 컨텍스트 윈도우 효율성 향상**
- **MCP 서버를 통한 자동 TOON 출력 지원**
- **최대 62.3% 크기 감소** (impact 분석)
- **최소 14.2% 크기 감소** (단일 심볼 읽기)

---

## PageRank 벤치마크

| 파라미터 | 값 |
|---------|-----|
| 심볼 수 | 2,713 |
| 수렴 이터레이션 | 8 |
| 델타 | 0.0000001672 |
| 저장된 순위 | 1,342 |

### Top 10 심볼 (damping=0.85)

| 순위 | 심볼 | PageRank | In-Degree | Out-Degree |
|------|------|----------|-----------|------------|
| 1 | SourceCodeParser | 0.6982 | 19 | 0 |
| 2 | estimate_token_count | 0.4375 | 20 | 12 |
| 3 | compress_symbol | 0.3977 | 15 | 27 |
| 4 | split_identifier | 0.3129 | 9 | 21 |
| 5 | extract_critical_branches | 0.3042 | 9 | 23 |
| 6 | compute_fuzzy_score | 0.2556 | 7 | 18 |
| 7 | extract_side_effects | 0.2306 | 7 | 8 |
| 8 | extract_signature | 0.2303 | 6 | 15 |
| 9 | extract_key_assignments | 0.1960 | 6 | 8 |
| 10 | walk_node | 0.1904 | 0 | 513 |

### Top 5 심볼 (damping=0.9)

| 순위 | 심볼 | PageRank | In-Degree | Out-Degree |
|------|------|----------|-----------|------------|
| 1 | SourceCodeParser | 0.6965 | 19 | 0 |
| 2 | estimate_token_count | 0.4389 | 20 | 12 |
| 3 | compress_symbol | 0.3958 | 15 | 27 |
| 4 | split_identifier | 0.3120 | 9 | 21 |
| 5 | extract_critical_branches | 0.3029 | 9 | 23 |

---

## 인덱싱 성능

| 단계 | 파일 수 | 심볼 수 | 참조 수 | 시간 |
|------|--------|--------|--------|------|
| 초기화 (Rust) | 45 | 381 | 6,882 | ~60s |
| 초기화 (Markdown) | 11 | 33 | 0 | ~1s |
| 전체 | 56 | 414 | 6,882 | ~61s |
| 재인덱싱 (변경 파일) | 11 | 928 | 8,053 | ~10s/file |

---

## 참고 사항

- 모든 벤치마크는 `cargo bench` (criterion 0.5)를 사용하여 측정
- 데이터베이스 벤치마크는 in-memory SQLite 사용
- 파서 벤치마크는 tree-sitter 0.24 사용 (0.25 stack overflow workaround)
- TOON 포맷은 `toon-format` crate v0.5.0 사용
- PageRank는 damping=0.85, tolerance=1e-6, max_iter=100 기본값 사용
