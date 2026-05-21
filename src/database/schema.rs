/// SQLite 스키마 — 심볼 그래프 메타데이터
///
/// Phase 1: Python 전용 (tree-sitter-python)
/// Phase 2+: JavaScript, TypeScript 추가

pub const SCHEMA_SQL: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

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
    path          TEXT PRIMARY KEY,        -- 프로젝트 루트 상대 경로
    hash          TEXT NOT NULL,           -- Blake3 hex digest (64 char)
    size          INTEGER NOT NULL,        -- 파일 바이트 크기
    modified      TEXT NOT NULL,           -- 마지막 수정 ISO 8601
    indexed_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ'))
);

-- 심볼 (함수, 클래스, 변수 등)
CREATE TABLE IF NOT EXISTS symbol (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path     TEXT NOT NULL REFERENCES file_hash(path) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL,           -- 'function', 'class', 'variable', 'method', 'import', 'decorator'
    start_line    INTEGER NOT NULL,        -- 1-based
    end_line      INTEGER NOT NULL,        -- 1-based
    start_col     INTEGER NOT NULL DEFAULT 0,
    end_col       INTEGER NOT NULL DEFAULT 0,
    signature     TEXT,                    -- 함수 시그니처 (예: 'def foo(x: int) -> str:')
    docstring     TEXT,                   -- 첫 번째 줄 docstring
    parent_symbol TEXT,                   -- 부모 심볼 이름 (예: 클래스 속 메서드)
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ'))
);

-- 참조 그래프 (caller → callee)
CREATE TABLE IF NOT EXISTS reference (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    caller_file   TEXT NOT NULL REFERENCES file_hash(path),
    callee_file   TEXT NOT NULL REFERENCES file_hash(path),
    caller_symbol TEXT,                   -- 호출하는 심볼 (선택적)
    callee_symbol TEXT NOT NULL,          -- 호출받는 심볼
    ref_kind      TEXT NOT NULL DEFAULT 'call',  -- 'call', 'import', 'inherit', 'define'
    line          INTEGER NOT NULL,       -- 호출 라인 (1-based)
    UNIQUE(caller_file, callee_file, caller_symbol, callee_symbol, ref_kind, line)
);

-- import 그래프
CREATE TABLE IF NOT EXISTS file_imports (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    importer      TEXT NOT NULL REFERENCES file_hash(path),
    imported      TEXT NOT NULL REFERENCES file_hash(path),
    module_name   TEXT NOT NULL,          -- 'os', 'sys', 'my_module'
    import_kind   TEXT NOT NULL DEFAULT 'import',  -- 'import', 'from_import', 'relative'
    UNIQUE(importer, imported, module_name, import_kind)
);

-- 검색 성능 인덱스
CREATE INDEX IF NOT EXISTS idx_symbol_name ON symbol(name);
CREATE INDEX IF NOT EXISTS idx_symbol_file ON symbol(file_path);
CREATE INDEX IF NOT EXISTS idx_symbol_kind ON symbol(kind);
CREATE INDEX IF NOT EXISTS idx_ref_callee ON reference(callee_symbol);
CREATE INDEX IF NOT EXISTS idx_ref_caller ON reference(caller_file);
CREATE INDEX IF NOT EXISTS idx_import_imported ON file_imports(imported);
"#;

pub fn init_db(conn: &rusqlite::Connection) -> Result<(), anyhow::Error> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}
