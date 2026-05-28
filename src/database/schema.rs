use anyhow::Context;
/// SQLite 스키마 + 마이그레이션 버전 관리
///
/// 마스터플랜 v1.1 스펙에 따른 테이블 구조:
/// - files (file_hash → 확장)
/// - symbols (qualified_name, is_public, is_test, hashes)
/// - refs (confidence, is_dynamic, context, soft delete)
/// - checksums (전용 Blake3 무결성 레저)
/// - dirty_queue (우선순위 기반 재인덱싱 큐)
/// - versions (마이그레이션 추적)
/// - project, file_imports, symbol_rank (기존 유지)
use rusqlite::Connection;

/// 현재 스키마 버전 (모노토닉 증가)
pub const CURRENT_SCHEMA_VERSION: i32 = 5;

/// Migration 001: Initial schema (기존 스키마 유지)
const MIGRATION_001_INITIAL: &str = r#"
-- 프로젝트 설정
CREATE TABLE IF NOT EXISTS project (
    id            INTEGER PRIMARY KEY DEFAULT 1,
    root_path     TEXT NOT NULL UNIQUE,
    language      TEXT NOT NULL DEFAULT 'python',
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ'))
);

-- 파일 해시 + 상태 (Blake3 무결성 검증)
CREATE TABLE IF NOT EXISTS file_hash (
    path          TEXT PRIMARY KEY,
    hash          TEXT NOT NULL,
    size          INTEGER NOT NULL,
    modified      TEXT NOT NULL,
    indexed_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ'))
);

-- 심볼 (함수, 클래스, 변수 등)
CREATE TABLE IF NOT EXISTS symbol (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path     TEXT NOT NULL REFERENCES file_hash(path) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL,
    start_line    INTEGER NOT NULL,
    end_line      INTEGER NOT NULL,
    start_col     INTEGER NOT NULL DEFAULT 0,
    end_col       INTEGER NOT NULL DEFAULT 0,
    signature     TEXT,
    docstring     TEXT,
    parent_symbol TEXT,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ'))
);

-- 참조 그래프 (caller → callee)
CREATE TABLE IF NOT EXISTS reference (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    caller_file   TEXT NOT NULL REFERENCES file_hash(path),
    callee_file   TEXT NOT NULL REFERENCES file_hash(path),
    caller_symbol TEXT,
    callee_symbol TEXT NOT NULL,
    ref_kind      TEXT NOT NULL DEFAULT 'call',
    line          INTEGER NOT NULL,
    UNIQUE(caller_file, callee_file, caller_symbol, callee_symbol, ref_kind, line)
);

-- import 그래프
CREATE TABLE IF NOT EXISTS file_imports (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    importer      TEXT NOT NULL REFERENCES file_hash(path),
    imported      TEXT NOT NULL REFERENCES file_hash(path),
    module_name   TEXT NOT NULL,
    import_kind   TEXT NOT NULL DEFAULT 'import',
    UNIQUE(importer, imported, module_name, import_kind)
);

-- 검색 성능 인덱스
CREATE INDEX IF NOT EXISTS idx_symbol_name ON symbol(name);
CREATE INDEX IF NOT EXISTS idx_symbol_file ON symbol(file_path);
CREATE INDEX IF NOT EXISTS idx_symbol_kind ON symbol(kind);
CREATE INDEX IF NOT EXISTS idx_ref_callee ON reference(callee_symbol);
CREATE INDEX IF NOT EXISTS idx_ref_caller ON reference(caller_file);
CREATE INDEX IF NOT EXISTS idx_import_imported ON file_imports(imported);

-- 그래프 랭킹
CREATE TABLE IF NOT EXISTS symbol_rank (
    symbol_name   TEXT PRIMARY KEY,
    page_rank     REAL NOT NULL DEFAULT 0.0,
    in_degree     INTEGER NOT NULL DEFAULT 0,
    out_degree    INTEGER NOT NULL DEFAULT 0,
    computed_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ'))
);

CREATE INDEX IF NOT EXISTS idx_symbol_rank_pr ON symbol_rank(page_rank DESC);
CREATE INDEX IF NOT EXISTS idx_symbol_rank_in ON symbol_rank(in_degree DESC);
"#;

/// Migration 002: Masterplan v1.1 alignment
///
/// 마스터플랜 스펙에 맞춰 확장:
/// 1. files 테이블 (새 스펙 — file_hash 데이터를 마이그레이션)
/// 2. symbols 확장 (qualified_name, is_public, is_test, token_count, body_hash, signature_hash, status)
/// 3. refs 테이블 (새 스펙 — reference 데이터를 마이그레이션)
/// 4. checksums 테이블 (전용 Blake3 무결성 레저)
/// 5. dirty_queue 테이블 (우선순위 재인덱싱 큐)
/// 6. versions 테이블 (마이그레이션 추적 — 자체 부트스트랩)
/// 7. agent_cache 테이블 (쿼리 결과 캐시)
const MIGRATION_002_MASTERPLAN: &str = r#"
-- ============================================================
-- 6. versions 테이블 (마이그레이션 추적)
-- 자체 부트스트랩: 이 테이블은 먼저 생성되어야 함
-- ============================================================
CREATE TABLE IF NOT EXISTS versions (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version    INTEGER UNIQUE NOT NULL,
    migration_name    TEXT NOT NULL,
    applied_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ')),
    checksum          TEXT
);

-- ============================================================
-- 1. files 테이블 (마스터플랜 스펙)
-- file_hash 데이터를 마이그레이션하여 기존 데이터를 유지
-- ============================================================
CREATE TABLE IF NOT EXISTS files (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    path              TEXT UNIQUE NOT NULL,
    abs_path          TEXT NOT NULL,
    size_bytes        INTEGER NOT NULL,
    mtime_ns          INTEGER NOT NULL DEFAULT 0,
    blake3_hash       TEXT NOT NULL,
    language          TEXT NOT NULL DEFAULT 'unknown',
    indexed_at        INTEGER NOT NULL,
    status            TEXT NOT NULL DEFAULT 'valid',
    parser_version    TEXT NOT NULL DEFAULT '0.1.0',
    symbol_count      INTEGER NOT NULL DEFAULT 0,
    line_count        INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_files_status ON files(status);
CREATE INDEX IF NOT EXISTS idx_files_language ON files(language);
CREATE INDEX IF NOT EXISTS idx_files_blake3 ON files(blake3_hash);

-- file_hash → files 마이그레이션 (기존 데이터 유지)
INSERT OR IGNORE INTO files (path, abs_path, size_bytes, blake3_hash, indexed_at)
    SELECT path, path, size, hash, 
           STRFTIME('%s', indexed_at) * 1000 + CAST(SUBSTR(indexed_at, INSTR(indexed_at, ':')-2, 2) AS INTEGER) * 1000000
    FROM file_hash
    WHERE path NOT IN (SELECT path FROM files);

-- ============================================================
-- 2. symbols 확장 (qualified_name, is_public, is_test, hashes, status)
-- ============================================================
ALTER TABLE symbol ADD COLUMN qualified_name TEXT DEFAULT '';
ALTER TABLE symbol ADD COLUMN signature_hash TEXT DEFAULT '';
ALTER TABLE symbol ADD COLUMN body_hash TEXT DEFAULT '';
ALTER TABLE symbol ADD COLUMN token_count INTEGER DEFAULT 0;
ALTER TABLE symbol ADD COLUMN is_public INTEGER DEFAULT 1;
ALTER TABLE symbol ADD COLUMN is_test INTEGER DEFAULT 0;
ALTER TABLE symbol ADD COLUMN status TEXT DEFAULT 'valid';
ALTER TABLE symbol ADD COLUMN extracted_at INTEGER NOT NULL DEFAULT (STRFTIME('%s', 'now') * 1000);

-- qualified_name 자동 채움: parent_symbol.name 포맷
UPDATE symbol
SET qualified_name = COALESCE(parent_symbol || '.' || name, name)
WHERE qualified_name = '';

CREATE INDEX IF NOT EXISTS idx_symbols_qualified ON symbol(qualified_name);
CREATE INDEX IF NOT EXISTS idx_symbols_public ON symbol(is_public) WHERE is_public = 1;
CREATE INDEX IF NOT EXISTS idx_symbols_status ON symbol(status);

-- ============================================================
-- 3. refs 테이블 (새 스펙 — reference 데이터를 마이그레이션)
-- ============================================================
CREATE TABLE IF NOT EXISTS refs (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    caller_symbol_id      INTEGER NOT NULL REFERENCES symbol(id),
    callee_symbol_id      INTEGER REFERENCES symbol(id),
    callee_name           TEXT,
    file_id               INTEGER NOT NULL REFERENCES files(id),
    line                  INTEGER NOT NULL,
    column                INTEGER NOT NULL DEFAULT 0,
    kind                  TEXT NOT NULL,
    confidence            REAL NOT NULL DEFAULT 1.0,
    is_dynamic            INTEGER NOT NULL DEFAULT 0,
    context               TEXT,
    extracted_at          INTEGER NOT NULL DEFAULT (STRFTIME('%s', 'now') * 1000),
    is_deleted            INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_refs_caller ON refs(caller_symbol_id) WHERE is_deleted = 0;
CREATE INDEX IF NOT EXISTS idx_refs_callee ON refs(callee_symbol_id) WHERE is_deleted = 0;
CREATE INDEX IF NOT EXISTS idx_refs_file ON refs(file_id) WHERE is_deleted = 0;
CREATE INDEX IF NOT EXISTS idx_refs_kind ON refs(kind) WHERE is_deleted = 0;
CREATE INDEX IF NOT EXISTS idx_refs_dynamic ON refs(is_dynamic, confidence) WHERE is_dynamic = 1 AND is_deleted = 0;

-- reference → refs 마이그레이션 (최대 1000개까지)
INSERT OR IGNORE INTO refs (caller_symbol_id, callee_name, file_id, line, kind, confidence, extracted_at)
    SELECT
        (SELECT id FROM symbol WHERE name = reference.caller_symbol AND file_path = reference.caller_file LIMIT 1),
        reference.callee_symbol,
        (SELECT id FROM files WHERE path = reference.caller_file LIMIT 1),
        reference.line,
        reference.ref_kind,
        1.0,
        STRFTIME('%s', 'now') * 1000
    FROM reference
    WHERE reference.caller_file IN (SELECT path FROM files)
    LIMIT 1000;

-- ============================================================
-- 4. checksums 테이블 (Blake3 무결성 레저)
-- ============================================================
CREATE TABLE IF NOT EXISTS checksums (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id           INTEGER UNIQUE NOT NULL REFERENCES files(id),
    blake3_hash       TEXT NOT NULL,
    computed_at       INTEGER NOT NULL DEFAULT (STRFTIME('%s', 'now') * 1000),
    verified_at       INTEGER NOT NULL DEFAULT (STRFTIME('%s', 'now') * 1000),
    verify_count      INTEGER NOT NULL DEFAULT 0,
    mismatch_count    INTEGER NOT NULL DEFAULT 0,
    last_mismatch_at  INTEGER,
    status            TEXT NOT NULL DEFAULT 'synced'
);

CREATE INDEX IF NOT EXISTS idx_checksums_status ON checksums(status);

-- file_hash → checksums 마이그레이션
INSERT OR IGNORE INTO checksums (file_id, blake3_hash, computed_at, verified_at)
    SELECT f.id, f.blake3_hash, f.indexed_at, f.indexed_at
    FROM files f
    WHERE f.id NOT IN (SELECT file_id FROM checksums);

-- ============================================================
-- 5. dirty_queue 테이블 (우선순위 재인덱싱 큐)
-- ============================================================
CREATE TABLE IF NOT EXISTS dirty_queue (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id         INTEGER NOT NULL REFERENCES files(id),
    reason          TEXT NOT NULL,
    priority        INTEGER NOT NULL DEFAULT 5,
    enqueued_at     INTEGER NOT NULL DEFAULT (STRFTIME('%s', 'now') * 1000),
    processed_at    INTEGER,
    retry_count     INTEGER NOT NULL DEFAULT 0,
    error_log       TEXT,
    status          TEXT NOT NULL DEFAULT 'pending'
);

CREATE INDEX IF NOT EXISTS idx_dirty_priority ON dirty_queue(priority, enqueued_at) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_dirty_file ON dirty_queue(file_id);

-- ============================================================
-- 8. agent_cache 테이블 (쿼리 결과 캐시)
-- ============================================================
CREATE TABLE IF NOT EXISTS agent_cache (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    query_key       TEXT UNIQUE NOT NULL,
    query_hash      TEXT NOT NULL,
    result_json     TEXT NOT NULL,
    created_at      INTEGER NOT NULL DEFAULT (STRFTIME('%s', 'now') * 1000),
    expires_at      INTEGER NOT NULL,
    hit_count       INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_agent_cache_hash ON agent_cache(query_hash);
CREATE INDEX IF NOT EXISTS idx_agent_cache_expires ON agent_cache(expires_at);
"#;

/// Migration 003: Override registry — manual reference overrides for dynamic refs
const MIGRATION_003_OVERRIDES: &str = r#"
CREATE TABLE IF NOT EXISTS overrides (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    caller_symbol   TEXT NOT NULL,
    callee_symbol   TEXT NOT NULL,
    ref_kind        TEXT NOT NULL DEFAULT 'override',
    confidence      REAL NOT NULL DEFAULT 0.9,
    reason          TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_overrides_caller ON overrides(caller_symbol);
CREATE INDEX IF NOT EXISTS idx_overrides_callee ON overrides(callee_symbol);
"#;

/// Migration 004: Confidence scoring for references
///
/// Add confidence and is_dynamic columns to the legacy reference table
/// so AST-extracted references carry dynamic confidence scores.
const MIGRATION_004_CONFIDENCE: &str = r#"
ALTER TABLE reference ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0;
ALTER TABLE reference ADD COLUMN is_dynamic INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_ref_confidence ON reference(confidence) WHERE confidence < 1.0;
"#;

/// Migration 005: String literals + potential string refs (Cross-ref Engine)
///
/// Two new tables for string-based dynamic reference detection:
/// 1. string_literals — collected string literals from AST parsing
/// 2. potential_string_refs — cross-matched string literal → symbol pairs with confidence
const MIGRATION_005_STRING_REFS: &str = r#"
-- ============================================================
-- 9. string_literals 테이블 (문자열 리터럴 수집)
-- ============================================================
CREATE TABLE IF NOT EXISTS string_literals (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path       TEXT NOT NULL REFERENCES file_hash(path) ON DELETE CASCADE,
    line_number     INTEGER NOT NULL,
    col_start       INTEGER NOT NULL,
    string_value    TEXT NOT NULL,
    is_symbol_like  INTEGER NOT NULL DEFAULT 0,
    context         TEXT,           -- "function_arg" | "sequence_element" | "assignment_rhs" | "kwarg" | "unknown"
    parent_fn       TEXT            -- enclosing function name (false positive 필터용)
);

CREATE INDEX IF NOT EXISTS idx_sl_file      ON string_literals(file_path);
CREATE INDEX IF NOT EXISTS idx_sl_value     ON string_literals(string_value);
CREATE INDEX IF NOT EXISTS idx_sl_sym_like  ON string_literals(is_symbol_like, string_value);

-- ============================================================
-- 10. potential_string_refs 테이블 (문자열 → 심볼 매칭 결과)
-- ============================================================
CREATE TABLE IF NOT EXISTS potential_string_refs (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    literal_id          INTEGER NOT NULL REFERENCES string_literals(id) ON DELETE CASCADE,
    target_symbol_id    INTEGER NOT NULL REFERENCES symbol(id) ON DELETE CASCADE,
    confidence          REAL NOT NULL,
    match_type          TEXT NOT NULL,  -- "exact_fq" | "module_scope" | "import_scope" | "method_ref"
    UNIQUE(literal_id, target_symbol_id)
);

CREATE INDEX IF NOT EXISTS idx_psr_target   ON potential_string_refs(target_symbol_id);
CREATE INDEX IF NOT EXISTS idx_psr_conf     ON potential_string_refs(confidence);
"#;

/// 마이그레이션 정의: (버전, 이름, SQL)
const MIGRATIONS: &[(i32, &str, &str)] = &[
    (1, "initial", MIGRATION_001_INITIAL),
    (2, "masterplan-v1.1", MIGRATION_002_MASTERPLAN),
    (3, "override-registry", MIGRATION_003_OVERRIDES),
    (4, "confidence-scoring", MIGRATION_004_CONFIDENCE),
    (5, "string-refs", MIGRATION_005_STRING_REFS),
];

/// 현재 스키마 버전 조회 (versions 테이블에서)
fn get_schema_version(conn: &Connection) -> Result<i32, anyhow::Error> {
    conn.query_row(
        "SELECT COALESCE(MAX(schema_version), 0) FROM versions",
        [],
        |r| r.get(0),
    )
    .context("schema version query failed")
}

/// 마이그레이션 적용 여부 확인 후 버전 기록
fn record_migration(conn: &Connection, version: i32, name: &str) -> Result<(), anyhow::Error> {
    conn.execute(
        "INSERT OR IGNORE INTO versions (schema_version, migration_name) VALUES (?1, ?2)",
        rusqlite::params![version, name],
    )?;
    Ok(())
}

/// 스키마 초기화 + 마이그레이션
///
/// 1. versions 테이블을 먼저 생성 (부트스트랩)
/// 2. 현재 버전 확인
/// 3. 누락된 마이그레이션 순차 적용
pub fn init_db(conn: &Connection) -> Result<(), anyhow::Error> {
    // 부트스트랩: versions 테이블 먼저 생성
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS versions (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            schema_version    INTEGER UNIQUE NOT NULL,
            migration_name    TEXT NOT NULL,
            applied_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ')),
            checksum          TEXT
        );
        "#,
    )?;

    let current_version = get_schema_version(conn)?;

    for &(version, name, sql) in MIGRATIONS {
        if version > current_version {
            tracing::info!("Applying migration: v{} — {}", version, name);

            conn.execute_batch(sql)
                .with_context(|| format!("migration v{} ({}) failed", version, name))?;

            // versions 테이블이 이미 만들어졌으므로 기록
            if version >= 2 {
                record_migration(conn, version, name)?;
            }

            // v1은 초기 마이그레이션 — versions 테이블이 없었을 수 있으므로 조건부
            if version == 1 {
                // versions 테이블이 방금 생성됨
                record_migration(conn, version, name)?;
            }
        }
    }

    let final_version = get_schema_version(conn)?;
    tracing::info!(
        "Schema at version {} (target: {})",
        final_version,
        CURRENT_SCHEMA_VERSION
    );

    Ok(())
}

/// 스키마 버전 조회 (퍼블릭)
pub fn schema_version(conn: &Connection) -> Result<i32, anyhow::Error> {
    get_schema_version(conn)
}

/// 스키마가 최신인지 확인
pub fn is_schema_up_to_date(conn: &Connection) -> Result<bool, anyhow::Error> {
    let version = get_schema_version(conn)?;
    Ok(version >= CURRENT_SCHEMA_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_creates_all_tables() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        // 모든 테이블 존재 확인
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(tables.contains(&"files".to_string()));
        assert!(tables.contains(&"symbol".to_string()));
        assert!(tables.contains(&"reference".to_string()));
        assert!(tables.contains(&"refs".to_string()));
        assert!(tables.contains(&"checksums".to_string()));
        assert!(tables.contains(&"dirty_queue".to_string()));
        assert!(tables.contains(&"versions".to_string()));
        assert!(tables.contains(&"agent_cache".to_string()));
        assert!(tables.contains(&"symbol_rank".to_string()));
        assert!(tables.contains(&"file_imports".to_string()));
        assert!(tables.contains(&"project".to_string()));
        assert!(tables.contains(&"file_hash".to_string()));
        assert!(tables.contains(&"string_literals".to_string()));
        assert!(tables.contains(&"potential_string_refs".to_string()));
    }

    #[test]
    fn test_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        assert!(is_schema_up_to_date(&conn).unwrap());
        assert_eq!(schema_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_idempotent_init() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        init_db(&conn).unwrap(); // 두 번째 호출도 에러 없이 통과
        assert_eq!(schema_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_files_table_has_correct_columns() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        // files 테이블에 마스터플랜 스펙 컬럼 확인
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(files)")
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(columns.contains(&"id".to_string()));
        assert!(columns.contains(&"path".to_string()));
        assert!(columns.contains(&"abs_path".to_string()));
        assert!(columns.contains(&"blake3_hash".to_string()));
        assert!(columns.contains(&"language".to_string()));
        assert!(columns.contains(&"status".to_string()));
        assert!(columns.contains(&"parser_version".to_string()));
        assert!(columns.contains(&"symbol_count".to_string()));
        assert!(columns.contains(&"line_count".to_string()));
    }

    #[test]
    fn test_symbol_table_has_extended_columns() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(symbol)")
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(columns.contains(&"qualified_name".to_string()));
        assert!(columns.contains(&"signature_hash".to_string()));
        assert!(columns.contains(&"body_hash".to_string()));
        assert!(columns.contains(&"token_count".to_string()));
        assert!(columns.contains(&"is_public".to_string()));
        assert!(columns.contains(&"is_test".to_string()));
        assert!(columns.contains(&"status".to_string()));
        assert!(columns.contains(&"extracted_at".to_string()));
    }

    #[test]
    fn test_refs_table_masterplan_spec() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(refs)")
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(columns.contains(&"caller_symbol_id".to_string()));
        assert!(columns.contains(&"callee_symbol_id".to_string()));
        assert!(columns.contains(&"confidence".to_string()));
        assert!(columns.contains(&"is_dynamic".to_string()));
        assert!(columns.contains(&"context".to_string()));
        assert!(columns.contains(&"is_deleted".to_string()));
    }

    #[test]
    fn test_dirty_queue_table() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(dirty_queue)")
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(columns.contains(&"file_id".to_string()));
        assert!(columns.contains(&"reason".to_string()));
        assert!(columns.contains(&"priority".to_string()));
        assert!(columns.contains(&"retry_count".to_string()));
        assert!(columns.contains(&"error_log".to_string()));
        assert!(columns.contains(&"status".to_string()));
    }

    #[test]
    fn test_checksums_table() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(checksums)")
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(columns.contains(&"file_id".to_string()));
        assert!(columns.contains(&"blake3_hash".to_string()));
        assert!(columns.contains(&"verify_count".to_string()));
        assert!(columns.contains(&"mismatch_count".to_string()));
        assert!(columns.contains(&"status".to_string()));
    }

    #[test]
    fn test_agent_cache_table() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(agent_cache)")
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(columns.contains(&"id".to_string()));
        assert!(columns.contains(&"query_key".to_string()));
        assert!(columns.contains(&"query_hash".to_string()));
        assert!(columns.contains(&"result_json".to_string()));
        assert!(columns.contains(&"created_at".to_string()));
        assert!(columns.contains(&"expires_at".to_string()));
        assert!(columns.contains(&"hit_count".to_string()));
    }

    #[test]
    fn test_agent_cache_crud() {
        use super::super::IndexDb;

        let db = IndexDb::open_in_memory().unwrap();

        // 1. set_cached → get_cached (기본 round-trip)
        db.set_cached("test:query:1", r#"{"symbols": ["foo", "bar"]}"#, 3600)
            .unwrap();
        let cached = db.get_cached("test:query:1").expect("캐시가 존재해야 함");
        assert_eq!(cached.query_key, "test:query:1");
        assert_eq!(cached.result_json, r#"{"symbols": ["foo", "bar"]}"#);
        assert_eq!(cached.hit_count, 1); // get_cached() 호출로 +1

        // 2. hit_count 증가 확인 (두 번째 조회)
        let cached2 = db.get_cached("test:query:1").expect("캐시가 존재해야 함");
        assert_eq!(cached2.hit_count, 2);

        // 3. set_cached UPSERT → hit_count 리셋
        db.set_cached("test:query:1", r#"{"symbols": ["baz"]}"#, 3600)
            .unwrap();
        let cached3 = db.get_cached("test:query:1").expect("캐시가 존재해야 함");
        assert_eq!(cached3.hit_count, 1); // UPSERT 후 리셋됨
        assert_eq!(cached3.result_json, r#"{"symbols": ["baz"]}"#);

        // 4. all_cached
        db.set_cached("test:query:2", r#"{"count": 42}"#, 3600)
            .unwrap();
        let all = db.all_cached().unwrap();
        assert!(all.len() >= 2);

        // 5. cache_stats
        let (total, active, expired) = db.cache_stats().unwrap();
        assert!(total >= 2);
        assert!(active >= 2);
        assert_eq!(expired, 0);

        // 6. invalidate_cached
        db.invalidate_cached("test:query:1").unwrap();
        assert!(db.get_cached("test:query:1").is_none());
        let (total2, active2, _) = db.cache_stats().unwrap();
        assert_eq!(total2, total - 1);
        assert_eq!(active2, active - 1);

        // 7. purge_expired (TTL 0초로 설정 → 즉시 만료)
        db.set_cached("test:expired", r#"{}"#, 0).unwrap();
        // 짧은 딜레이 후 purge
        std::thread::sleep(std::time::Duration::from_millis(10));
        let deleted = db.purge_expired().unwrap();
        assert!(deleted >= 1);
    }

    #[test]
    fn test_agent_cache_ttl_expiry() {
        use super::super::IndexDb;

        let db = IndexDb::open_in_memory().unwrap();

        // TTL 0초 → 즉시 만료됨
        db.set_cached("ttl:test", r#"{"data": true}"#, 0).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));

        // 만료된 캐시는 get_cached()에서 None 반환
        assert!(db.get_cached("ttl:test").is_none());

        // all_cached()에서도 제외
        let all = db.all_cached().unwrap();
        for entry in &all {
            assert_ne!(entry.query_key, "ttl:test");
        }
    }

    #[test]
    fn test_agent_cache_hash_consistency() {
        use super::super::IndexDb;

        let db = IndexDb::open_in_memory().unwrap();

        db.set_cached("hash:test", r#"{}"#, 3600).unwrap();
        let cached = db.get_cached("hash:test").unwrap();

        // query_hash는 blake3(query_key)와 일치해야 함
        let expected_hash = blake3::hash("hash:test".as_bytes()).to_hex().to_string();
        assert_eq!(cached.query_hash, expected_hash);
    }
}
