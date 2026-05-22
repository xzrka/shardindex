# ShardIndex 작업 완료 보고서

**작성일:** 2026-05-23
**모델:** Qwen3.6-27B-Q4_K_M.gguf
**세션:** Telegram DM (thread: 1851094)

---

## 완료된 작업

### 1. Agent Skill Prompt Template ✅
- **파일:** `prompts/shardindex_skill_v1.md`
- **내용:** MCP 메서드 (`read`, `neighbors`, `impact`, `search`, `stats`) 사용 가이드, 워크플로우 패턴 3가지, 18언어 지원 안내
- **라인:** 3,418 bytes

### 2. Confidence Scoring for Dynamic References ✅
- **스키마 v4:** `reference` 테이블에 `confidence REAL`, `is_dynamic INTEGER` 컬럼 추가
- **migration 004:** `ALTER TABLE reference ADD COLUMN confidence/is_dynamic`
- **ParsedReference:** `confidence()` 메서드 — `call/import/inherit`=1.0, `dynamic_dispatch`=0.7, `string_ref`=0.3
- **ParsedReference:** `is_dynamic()` 메서드 — 런타임 리졸브 참조 감지
- **IndexDb::insert_reference:** confidence + is_dynamic 포함 INSERT
- **IndexDb::neighbors:** COALESCE(confidence, 1.0) fallback
- **ProjectIndexer:** 인덱싱 시 `ref_rec.confidence()` 자동 계산 후 저장

### 3. Agent Cache Layer ✅ (이전 세션 완료)
- **파일:** `src/agent_cache.rs` (536 라인, 27 테스트)
- **기능:** TTL 기반 캐시, blake3 해시 무결성, 파일 변경 시 invalidate
- **MCP 통합:** `ServerState`에 `AgentCache` 주입, 5개 핸들러 모두 read-through 패턴

---

## 미완료 작업 (Remaining)

| # | 작업 | 상태 | 비고 |
|---|------|------|------|
| 4 | Performance benchmark suite (`benches/`) | 미시작 | criterion 기반 |
| 5 | Background daemon state machine 완료 | 미완료 | `src/daemon/` state machine incomplete |
| 6 | Override registry UI/CLI | 부분완료 | CLI `override add/remove/list` 완료, UI 미구현 |

---

## 빌드 상태

```
cargo check: 0 errors, 36 warnings (dead code, unused mut)
cargo test:  166 passed, 0 failed
total 소스:  14,736 lines (18 language parsers + DB + MCP + indexer)
```

## 스키마 버전

- `CURRENT_SCHEMA_VERSION = 4` (v1 → v2 → v3 → v4)
- 테이블: `project`, `files`, `checksums`, `dirty_queue`, `versions`, `symbols`, `references`, `symbol_rank`, `agent_cache`, `overrides`

## git 변경사항 (커밋 전)

```
M .hermes.md
M src/database/mod.rs      (confidence/is_dynamic on ReferenceRecord + insert/neighbors)
M src/database/schema.rs   (migration v4, CURRENT_SCHEMA_VERSION=4)
M src/indexer/mod.rs       (confidence() 호출, is_dynamic 계산)
M src/indexer/types.rs     (ParsedReference::confidence(), ::is_dynamic())
?? prompts/                (shardindex_skill_v1.md)
```
