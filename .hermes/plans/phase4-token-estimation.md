# Phase 4: Token Estimation + Adaptive Compression

## 상태
- Phase 4-1: 진행 중 (in_progress)
- Phase 4-2 ~ 4-4: 대기 중 (pending)

## 배경
- Smart YAML 통합 완료 (commit `0a8baae`)
- `symbol` 테이블에 `token_count` 컬럼 존재 (migration 002) — 아직 채워지지 않음
- 모든 테스트 172개 통과, 빌드 클린

## Phase 4-1: Token estimation per symbol

### 목표
심볼의 소스 코드로부터 토큰 수를 추정하여 DB에 저장. LLM context window 관리의 기초.

### 구현 계획

#### 1. `src/token_estimation.rs` 모듈 생성
```
pub fn estimate_token_count(source: &str) -> u32
```
- BPE-style heuristic: 평균 1 토큰 ≈ 4 문자 (GPT-4/CL100k 기준)
- 보정:
  - 공백/주석 제거 후 계산
  - 특수 문자열 (이모티콘, 유니코드)는 +1 토큰
  - 숫자/식별자는 토큰 경계로 분리

#### 2. 인덱싱 파이프라인에 통합
- `src/indexer/mod.rs` — `index_file()` 내 심볼 삽입 시:
  - `content`에서 심볼 범위의 코드를 추출 (`start_line` ~ `end_line`)
  - `estimate_token_count()` 호출 → `token_count` 계산
  - DB insert 시 `token_count` 컬럼에 저장

- `src/database/mod.rs` — `insert_symbol()` 수정:
  - SQL에 `token_count` 컬럼 추가
  - `SymbolRecord` struct에 `token_count: u32` 필드 추가

#### 3. `SymbolRecord` struct 확장
```rust
pub struct SymbolRecord {
    // ... 기존 필드 ...
    pub token_count: u32,  // NEW
}
```

#### 4. DB 스키마
- `token_count` 컬럼 이미 존재 (migration 002)
- 추가 migration 불필요

#### 5. 검색 결과에 토큰 정보 포함
- `src/search.rs` — `SearchResultJson` 에 `token_count` 필드 추가
- `src/format/smart_yaml.rs` — Smart YAML 출력에 토큰 정보 포함

#### 6. CLI 명령어
- `shardindex stats --format smart-yaml` 에 총 토큰 수 추가
- `shardindex read <symbol>` 에 개별 심볼 토큰 수 표시

### 테스트
- `estimate_token_count()` 단위 테스트 (여러 언어/코드 패턴)
- 인덱싱 후 `token_count`가 DB에 저장되는지 확인
- Smart YAML 출력에 토큰 정보가 포함되는지 확인

## Phase 4-2: Adaptive compression pipeline

### 목표
LLM context window 제약에 맞춰 심볼 정보를 압축하는 3-tier 시스템:
- `signature_only`: 시그니처만 (최대 압축)
- `critical_branches`: 시그니처 + 핵심 코드 경로
- `full_body`: 전체 코드 (최소 압축)

### 구현 계획
1. `src/compression.rs` 모듈 생성
2. `CompressionLevel` enum + `compress_symbol()` 함수
3. 심볼의 코드 본문에서 "핵심 경로" 추출 (조건문, 반환문, 예외처리)

## Phase 4-3: TokenBudgeted MCP responses

### 목표
MCP 응답에 토큰 예산을 적용하여 LLM context window를 초과하지 않도록 함.

### 구현 계획
1. MCP tool handlers 에 `token_budget` 파라미터 추가
2. 예산 초과 시 자동 압축 레벨 다운그레이드
3. `TokenBudgeted` response wrapper

## Phase 4-4: Integration tests

### 목표
- 토큰 예산 강제 적용 테스트
- 압축 파이프라인 E2E 테스트
- MCP 응답의 토큰 수 검증

## 관련 파일
- `src/token_estimation.rs` — NEW (Phase 4-1)
- `src/database/mod.rs` — 수정 (SymbolRecord + insert_symbol)
- `src/indexer/mod.rs` — 수정 (index_file token_count 계산)
- `src/search.rs` — 수정 (SearchResultJson.token_count)
- `src/format/smart_yaml.rs` — 수정 (토큰 정보 출력)
- `src/compression.rs` — NEW (Phase 4-2)
- `src/mcp/stdio.rs` — 수정 (Phase 4-3)
- `src/cli/mod.rs` — 수정 (CLI flags)
- `tests/` — 테스트 추가

## 현재 진행 상황 (중단 시점)
- 코드베이스 분석 완료
- `src/database/mod.rs` (1088줄) — IndexDb 구조 파악 완료
- `src/search.rs` (516줄) — 검색 엔진 구조 파악 완료
- `src/indexer/mod.rs` (554줄) — index_file() 파이프라인 파악 완료
- `src/indexer/types.rs` (120줄) — ParsedSymbol 구조 파악 완료
- `src/graph/mod.rs` (738줄) — edit_plan 에 이미 `estimated_tokens` 필드 존재
- 다음 단계: `src/token_estimation.rs` 모듈 생성 시작
