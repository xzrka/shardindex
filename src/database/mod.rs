pub mod schema;
pub use schema::init_db;

use anyhow::Context;
/// SQLite 데이터베이스 연결 관리
///
/// 파일 레벨 CRUD — 인덱싱 상태 조회
use rusqlite::params;
use rusqlite::{Connection, params_from_iter};

#[derive(Debug, Clone, serde::Serialize)]
pub struct FileRecord {
    pub path: String,
    pub hash: String,
    pub size: u64,
    pub modified: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SymbolRecord {
    pub id: i64,
    pub file_path: String,
    pub name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub parent_symbol: Option<String>,
    pub qualified_name: String,
    pub token_count: usize,
}

impl SymbolRecord {
    /// Build qualified name from file path, symbol name, and optional parent.
    pub fn build_qualified_name(file_path: &str, name: &str, parent: &Option<String>) -> String {
        let module = Self::module_name_from_path(file_path);
        match parent {
            Some(p) => format!("{}.{}.{}", module, p, name),
            None => format!("{}.{}", module, name),
        }
    }

    /// Extract module name from file path.
    pub fn module_name_from_path(file_path: &str) -> String {
        let path = std::path::Path::new(file_path);
        let stem = path
            .components()
            .filter_map(|comp| {
                if comp.as_os_str() == "src" || comp.as_os_str() == "lib" {
                    None
                } else {
                    Some(comp)
                }
            })
            .collect::<std::path::PathBuf>();
        let mut parts: Vec<String> = stem
            .components()
            .filter_map(|comp| comp.as_os_str().to_str().map(|s| s.to_string()))
            .collect();
        if let Some(last) = parts.last_mut() {
            if let Some(dot) = last.rfind('.') {
                *last = last[..dot].to_string();
            }
        }
        parts.retain(|p| p != "__init__");
        if parts.is_empty() {
            "root".to_string()
        } else {
            parts.join(".")
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReferenceRecord {
    pub id: i64,
    pub caller_file: String,
    pub callee_file: String,
    pub caller_symbol: Option<String>,
    pub callee_symbol: String,
    pub ref_kind: String,
    pub line: usize,
    pub confidence: f64,
    pub is_dynamic: bool,
}

/// 심볼 랭킹 스코어 (PageRank + degree centrality)
#[derive(Debug, Clone, serde::Serialize)]
pub struct SymbolRank {
    pub symbol_name: String,
    pub page_rank: f64,
    pub in_degree: i64,
    pub out_degree: i64,
    pub computed_at: String,
}

/// Blake3 checksum 레저 레코드
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChecksumRecord {
    pub id: i64,
    pub file_id: i64,
    pub file_path: String,
    pub blake3_hash: String,
    pub computed_at: i64,
    pub verified_at: i64,
    pub verify_count: i32,
    pub mismatch_count: i32,
    pub status: String,
}

/// Dirty queue 엔트리
#[derive(Debug, Clone, serde::Serialize)]
pub struct DirtyQueueRecord {
    pub id: i64,
    pub file_id: i64,
    pub file_path: String,
    pub reason: String,
    pub priority: i32,
    pub enqueued_at: i64,
    pub processed_at: Option<i64>,
    pub retry_count: i32,
    pub error_log: Option<String>,
    pub status: String,
}

/// Agent cache 엔트리 (MCP 쿼리 결과 TTL 캐시)
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentCacheRecord {
    pub id: i64,
    pub query_key: String,
    pub query_hash: String,
    pub result_json: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub hit_count: i32,
}

pub struct IndexDb {
    pub conn: Connection,
    db_path: String,
}

/// Override record — manual reference override for dynamic refs
#[derive(Debug, Clone)]
pub struct OverrideRecord {
    pub id: i64,
    pub caller_symbol: String,
    pub callee_symbol: String,
    pub ref_kind: String,
    pub confidence: f64,
    pub reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl std::fmt::Debug for IndexDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexDb")
            .field("db_path", &self.db_path)
            .finish()
    }
}

impl Clone for IndexDb {
    fn clone(&self) -> Self {
        let conn = Connection::open(&self.db_path).unwrap_or_else(|_| {
            Connection::open_in_memory().expect("fallback memory DB should always work")
        });
        Self {
            conn,
            db_path: self.db_path.clone(),
        }
    }
}

impl IndexDb {
    /// Return the database file path
    pub fn db_path(&self) -> &str {
        &self.db_path
    }

    /// 새 데이터베이스 연결 생성 + 스키마 초기화
    pub fn open(db_path: &str) -> Result<Self, anyhow::Error> {
        let conn = Connection::open(db_path).context(format!("DB open failed: {}", db_path))?;
        init_db(&conn)?;
        Ok(Self {
            conn,
            db_path: db_path.to_string(),
        })
    }

    /// 인메모리 DB 생성 (테스트용)
    pub fn open_in_memory() -> Result<Self, anyhow::Error> {
        let conn = Connection::open_in_memory()?;
        init_db(&conn)?;
        Ok(Self {
            conn,
            db_path: ":memory:".to_string(),
        })
    }

    // ─── Project ───

    /// 프로젝트 메타데이터 저장 (UPSERT)
    pub fn upsert_project(&self, root_path: &str, language: &str) -> Result<(), anyhow::Error> {
        self.conn.execute(
            r#"INSERT INTO project (root_path, language)
               VALUES (?1, ?2)
               ON CONFLICT(id) DO UPDATE SET
                 root_path = excluded.root_path,
                 language = excluded.language,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ')"#,
            params![root_path, language],
        )?;
        Ok(())
    }

    /// 프로젝트 root_path 조회
    pub fn get_project_root(&self) -> Option<String> {
        self.conn
            .query_row("SELECT root_path FROM project WHERE id = 1", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
    }

    // ─── File Hash ───

    /// 파일 해시 저장 또는 업데이트
    pub fn upsert_file(
        &self,
        path: &str,
        hash: &str,
        size: u64,
        modified: &str,
    ) -> Result<(), anyhow::Error> {
        self.conn.execute(
            r#"INSERT INTO file_hash (path, hash, size, modified)
               VALUES (?1, ?2, ?3, ?4)
               ON CONFLICT(path) DO UPDATE SET hash=?2, size=?3, modified=?4, indexed_at=strftime('%Y-%m-%dT%H:%M:%fZ')"#,
            params![path, hash, size, modified],
        )?;
        Ok(())
    }

    /// 파일 해시 조회
    pub fn get_file_hash(&self, path: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT hash FROM file_hash WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .ok()
    }

    /// 변경된 파일 목록 조회 (인메모리 해시 맵과 비교용)
    pub fn all_file_hashes(&self) -> Result<Vec<FileRecord>, anyhow::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, hash, size, modified FROM file_hash ORDER BY path")?;
        let records = stmt.query_map(params![], |row| {
            Ok(FileRecord {
                path: row.get(0)?,
                hash: row.get(1)?,
                size: row.get(2)?,
                modified: row.get(3)?,
            })
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// 삭제된 파일 정리 (hash + symbols + references)
    pub fn remove_file(&self, path: &str) -> Result<(), anyhow::Error> {
        self.conn.execute(
            "DELETE FROM reference WHERE caller_file = ?1 OR callee_file = ?1",
            params![path],
        )?;
        self.conn
            .execute("DELETE FROM symbol WHERE file_path = ?1", params![path])?;
        self.conn
            .execute("DELETE FROM file_hash WHERE path = ?1", params![path])?;
        Ok(())
    }

    /// 파일의 심볼만 삭제 (재인덱싱 전 정리)
    pub fn remove_file_symbols(&self, path: &str) -> Result<(), anyhow::Error> {
       self.conn
           .execute("DELETE FROM symbol WHERE file_path = ?1", params![path])?;
       self.conn.execute(
           "DELETE FROM reference WHERE caller_file = ?1",
           params![path],
       )?;
       // string_literals + potential_string_refs도 함께 삭제 (Cross-ref Engine)
       self.remove_file_string_literals(path)?;
       Ok(())
    }

    // ─── Symbols ───

    /// 심볼 삽입
    pub fn insert_symbol(&self, rec: &SymbolRecord) -> Result<i64, anyhow::Error> {
        let _id = self.conn.last_insert_rowid();
        self.conn.execute(
            r#"INSERT INTO symbol (file_path, name, kind, start_line, end_line, start_col, end_col, signature, docstring, parent_symbol, qualified_name, token_count)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
            params![
                rec.file_path, rec.name, rec.kind,
                rec.start_line, rec.end_line, rec.start_col, rec.end_col,
                rec.signature, rec.docstring, rec.parent_symbol, rec.qualified_name, rec.token_count
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 파일의 모든 심볼 조회
    pub fn file_symbols(&self, file_path: &str) -> Result<Vec<SymbolRecord>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, name, kind, start_line, end_line, start_col, end_col, signature, docstring, parent_symbol, qualified_name, COALESCE(token_count, 0)
             FROM symbol WHERE file_path = ?1 ORDER BY start_line"
        )?;
        let records = stmt.query_map(params![file_path], |row| {
            Ok(SymbolRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                start_col: row.get(6)?,
                end_col: row.get(7)?,
                signature: row.get(8)?,
                docstring: row.get(9)?,
                parent_symbol: row.get::<_, Option<String>>(10)?,
                qualified_name: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
                token_count: row.get::<_, usize>(12)?,
            })
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// 심볼명 검색
    pub fn search_symbol(&self, pattern: &str) -> Result<Vec<SymbolRecord>, anyhow::Error> {
        let search = format!("%{}%", pattern);
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, name, kind, start_line, end_line, start_col, end_col, signature, docstring, parent_symbol, qualified_name, COALESCE(token_count, 0)
             FROM symbol WHERE name LIKE ?1 OR qualified_name LIKE ?1 ORDER BY kind, name LIMIT 50"
        )?;
        let records = stmt.query_map(params![search], |row| {
            Ok(SymbolRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                start_col: row.get(6)?,
                end_col: row.get(7)?,
                signature: row.get(8)?,
                docstring: row.get(9)?,
                parent_symbol: row.get::<_, Option<String>>(10)?,
                qualified_name: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
                token_count: row.get::<_, usize>(12)?,
            })
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    // ─── References ───

    /// 참조 삽입 (confidence + is_dynamic 포함)
    pub fn insert_reference(&self, rec: &ReferenceRecord) -> Result<i64, anyhow::Error> {
        self.conn.execute(
            r#"INSERT OR IGNORE INTO reference (caller_file, callee_file, caller_symbol, callee_symbol, ref_kind, line, confidence, is_dynamic)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            params![
                rec.caller_file, rec.callee_file,
                rec.caller_symbol, rec.callee_symbol,
                rec.ref_kind, rec.line,
                rec.confidence, if rec.is_dynamic { 1 } else { 0 }
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 심볼의 neighbors (직접 참조하는/참조받는 심볼)
    pub fn neighbors(&self, symbol_name: &str) -> Result<Vec<ReferenceRecord>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, caller_file, callee_file, caller_symbol, callee_symbol, ref_kind, line,
                      COALESCE(confidence, 1.0), COALESCE(is_dynamic, 0)
               FROM reference
               WHERE caller_symbol = ?1 OR callee_symbol = ?1
               ORDER BY ref_kind, line
               LIMIT 100"#,
        )?;
        let records = stmt.query_map(params![symbol_name], |row| {
            Ok(ReferenceRecord {
                id: row.get(0)?,
                caller_file: row.get(1)?,
                callee_file: row.get(2)?,
                caller_symbol: row.get(3)?,
                callee_symbol: row.get(4)?,
                ref_kind: row.get(5)?,
                line: row.get(6)?,
                confidence: row.get(7)?,
                is_dynamic: row.get::<_, i32>(8)? == 1,
            })
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// 심볼 영향도 분석 — 이 심볼을 사용하는 모든 파일/심볼
    pub fn impact(
        &self,
        symbol_name: &str,
    ) -> Result<(Vec<SymbolRecord>, Vec<ReferenceRecord>), anyhow::Error> {
        let (filter_col, filter_arg) = if symbol_name.contains('.') {
            ("qualified_name", symbol_name.to_string())
        } else {
            ("name", symbol_name.to_string())
        };
        let resolved_name: String = self
            .conn
            .query_row(
                &format!("SELECT name FROM symbol WHERE {} = ?1 LIMIT 1", filter_col),
                params![filter_arg],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| symbol_name.to_string());

        let callers: Vec<SymbolRecord> = {
            let mut stmt = self.conn.prepare(
                r#"SELECT id, file_path, name, kind, start_line, end_line, start_col, end_col, signature, docstring, parent_symbol, qualified_name, COALESCE(token_count, 0)
                   FROM symbol WHERE name IN (SELECT DISTINCT caller_symbol FROM reference WHERE callee_symbol = ?1)
                   LIMIT 50"#
            )?;
            stmt.query_map(params![resolved_name], |row| {
                Ok(SymbolRecord {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    start_line: row.get(4)?,
                    end_line: row.get(5)?,
                    start_col: row.get(6)?,
                    end_col: row.get(7)?,
                    signature: row.get::<_, Option<String>>(8)?,
                    docstring: row.get::<_, Option<String>>(9)?,
                    parent_symbol: row.get::<_, Option<String>>(10)?,
                    qualified_name: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
                    token_count: row.get::<_, usize>(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
        };

        let all_refs = self.neighbors(&resolved_name)?;
        Ok((callers, all_refs))
    }

    /// 심볼명 검색 (랭킹 기반 정렬)
    pub fn search_symbol_ranked(
        &self,
        pattern: &str,
    ) -> Result<Vec<(SymbolRecord, Option<f64>)>, anyhow::Error> {
        let search = format!("%{}%", pattern);
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.file_path, s.name, s.kind, s.start_line, s.end_line, s.start_col, s.end_col,
                    s.signature, s.docstring, s.parent_symbol, s.qualified_name, COALESCE(s.token_count, 0), sr.page_rank
             FROM symbol s
             LEFT JOIN symbol_rank sr ON s.name = sr.symbol_name
             WHERE s.name LIKE ?1 OR s.qualified_name LIKE ?1
             ORDER BY sr.page_rank DESC NULLS LAST, s.kind, s.name
             LIMIT 50"
        )?;
        let records = stmt.query_map(params![search], |row| {
            Ok((
                SymbolRecord {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    start_line: row.get(4)?,
                    end_line: row.get(5)?,
                    start_col: row.get(6)?,
                    end_col: row.get(7)?,
                    signature: row.get(8)?,
                    docstring: row.get(9)?,
                    parent_symbol: row.get::<_, Option<String>>(10)?,
                    qualified_name: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
                    token_count: row.get::<_, usize>(12)?,
                },
                row.get::<_, Option<f64>>(13)?,
            ))
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// 심볼 영향도 분석 + 랭킹 정보 포함
    pub fn impact_ranked(
        &self,
        symbol_name: &str,
    ) -> Result<(Vec<(SymbolRecord, Option<f64>)>, Vec<ReferenceRecord>), anyhow::Error> {
        let (filter_col, filter_arg) = if symbol_name.contains('.') {
            ("qualified_name", symbol_name.to_string())
        } else {
            ("name", symbol_name.to_string())
        };
        let resolved_name: String = self
            .conn
            .query_row(
                &format!("SELECT name FROM symbol WHERE {} = ?1 LIMIT 1", filter_col),
                params![filter_arg],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| symbol_name.to_string());

        // 직접 호출하는 심볼들 (랭킹 JOIN)
        let mut stmt = self.conn.prepare(
            r#"SELECT s.id, s.file_path, s.name, s.kind, s.start_line, s.end_line,
                      s.start_col, s.end_col, s.signature, s.docstring, s.parent_symbol, s.qualified_name,
                      COALESCE(s.token_count, 0), sr.page_rank
               FROM symbol s
               LEFT JOIN symbol_rank sr ON s.name = sr.symbol_name
               WHERE s.name IN (
                   SELECT DISTINCT caller_symbol FROM reference WHERE callee_symbol = ?1
               )
               ORDER BY sr.page_rank DESC NULLS LAST
               LIMIT 50"#,
        )?;
        let callers = stmt
            .query_map(params![resolved_name], |row| {
                Ok((
                    SymbolRecord {
                        id: row.get(0)?,
                        file_path: row.get(1)?,
                        name: row.get(2)?,
                        kind: row.get(3)?,
                        start_line: row.get(4)?,
                        end_line: row.get(5)?,
                        start_col: row.get(6)?,
                        end_col: row.get(7)?,
                        signature: row.get::<_, Option<String>>(8)?,
                        docstring: row.get::<_, Option<String>>(9)?,
                        parent_symbol: row.get::<_, Option<String>>(10)?,
                        qualified_name: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
                        token_count: row.get::<_, usize>(12)?,
                    },
                    row.get::<_, Option<f64>>(13)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let all_refs = self.neighbors(&resolved_name)?;
        Ok((callers, all_refs))
    }

    /// 통계
    pub fn stats(&self) -> Result<(usize, usize, usize), anyhow::Error> {
        let files: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM file_hash", [], |r| r.get(0))
            .unwrap_or(0);
        let symbols: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbol", [], |r| r.get(0))
            .unwrap_or(0);
        let refs: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM reference", [], |r| r.get(0))
            .unwrap_or(0);
        Ok((files, symbols, refs))
    }

    /// 프로젝트의 주요 언어 감지 (files 테이블에서 majority language)
    ///
    /// `project` 테이블의 language를 먼저 확인하고, 없으면 `files` 테이블에서
    /// 가장 많은 언어를 반환합니다. 둘 다 없으면 `None`을 반환합니다.
    pub fn detect_project_language(&self) -> Option<String> {
        // 1. project 테이블에서 language 확인
        if let Ok(lang) =
            self.conn
                .query_row("SELECT language FROM project WHERE id = 1", [], |r| {
                    r.get::<_, String>(0)
                })
        {
            if !lang.is_empty() && lang != "unknown" {
                return Some(lang);
            }
        }

        // 2. files 테이블에서 majority language
        if let Ok(lang) = self.conn.query_row(
            "SELECT language FROM files
             WHERE language != 'unknown' AND language != ''
             GROUP BY language
             ORDER BY COUNT(*) DESC
             LIMIT 1",
            [],
            |r| r.get::<_, String>(0),
        ) {
            return Some(lang);
        }

        None
    }

    /// 파일 경로에서 언어 감지 (확장자 기반)
    pub fn detect_language_from_path(&self, path: &str) -> Option<String> {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())?;
        match ext {
            "py" => Some("python".to_string()),
            "js" => Some("javascript".to_string()),
            "ts" | "tsx" => Some("typescript".to_string()),
            "rs" => Some("rust".to_string()),
            "go" => Some("go".to_string()),
            "rb" => Some("ruby".to_string()),
            "java" => Some("java".to_string()),
            "php" => Some("php".to_string()),
            "jl" => Some("julia".to_string()),
            "lua" => Some("lua".to_string()),
            "swift" => Some("swift".to_string()),
            "zig" => Some("zig".to_string()),
            "scala" => Some("scala".to_string()),
            "ex" | "exs" => Some("elixir".to_string()),
            "dart" => Some("dart".to_string()),
            "hs" => Some("haskell".to_string()),
            "c" => Some("c".to_string()),
            "cpp" | "cc" | "cxx" | "h" | "hpp" | "hxx" | "hh" => Some("cpp".to_string()),
            "md" => Some("markdown".to_string()),
            _ => None,
        }
    }

    // ─── Symbol Ranking ───

    /// 랭킹 데이터 전체 조회 (page_rank 내림차순)
    pub fn ranked_symbols(&self, limit: usize) -> Result<Vec<SymbolRank>, anyhow::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT symbol_name, page_rank, in_degree, out_degree, computed_at FROM symbol_rank ORDER BY page_rank DESC LIMIT ?1")?;
        let records = stmt.query_map(params![limit], |row| {
            Ok(SymbolRank {
                symbol_name: row.get(0)?,
                page_rank: row.get(1)?,
                in_degree: row.get(2)?,
                out_degree: row.get(3)?,
                computed_at: row.get(4)?,
            })
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// 심볼 이름으로 랭킹 조회
    pub fn symbol_rank(&self, symbol_name: &str) -> Option<SymbolRank> {
        self.conn
            .query_row(
                "SELECT symbol_name, page_rank, in_degree, out_degree, computed_at FROM symbol_rank WHERE symbol_name = ?1",
                params![symbol_name],
                |row| Ok(SymbolRank {
                    symbol_name: row.get(0)?,
                    page_rank: row.get(1)?,
                    in_degree: row.get(2)?,
                    out_degree: row.get(3)?,
                    computed_at: row.get(4)?,
                }),
            )
            .ok()
    }

    /// 랭킹 스코어 저장 (UPSERT)
    pub fn upsert_rank(&self, rank: &SymbolRank) -> Result<(), anyhow::Error> {
        self.conn.execute(
            r#"INSERT INTO symbol_rank (symbol_name, page_rank, in_degree, out_degree, computed_at)
               VALUES (?1, ?2, ?3, ?4, ?5)
               ON CONFLICT(symbol_name) DO UPDATE SET
                 page_rank = excluded.page_rank,
                 in_degree = excluded.in_degree,
                 out_degree = excluded.out_degree,
                 computed_at = excluded.computed_at"#,
            params![
                rank.symbol_name,
                rank.page_rank,
                rank.in_degree,
                rank.out_degree,
                rank.computed_at,
            ],
        )?;
        Ok(())
    }

    /// 랭킹 테이블 전체 삭제 (재인dex 후)
    pub fn clear_ranks(&self) -> Result<(), anyhow::Error> {
        self.conn.execute("DELETE FROM symbol_rank", [])?;
        Ok(())
    }

    /// degree centrality 계산 (reference 테이블에서)
    pub fn compute_degrees(&self) -> Result<Vec<(String, i64, i64)>, anyhow::Error> {
        // in_degree: callee로서 참조되는 횟수
        // out_degree: caller로서 참조하는 횟수
        let mut stmt = self.conn.prepare(
            r#"SELECT
                 s.name,
                 COALESCE(ind.in_deg, 0) AS in_degree,
                 COALESCE(outd.out_deg, 0) AS out_degree
               FROM symbol s
               LEFT JOIN (
                 SELECT callee_symbol, COUNT(*) AS in_deg
                 FROM reference
                 GROUP BY callee_symbol
               ) ind ON s.name = ind.callee_symbol
               LEFT JOIN (
                 SELECT caller_symbol, COUNT(*) AS out_deg
                 FROM reference
                 WHERE caller_symbol IS NOT NULL
                 GROUP BY caller_symbol
               ) outd ON s.name = outd.caller_symbol
               ORDER BY in_degree DESC, out_degree DESC"#,
        )?;
        let records = stmt.query_map(params![], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// PageRank 계산에 필요한 인접 리스트: (source, target) edges
    pub fn graph_edges(&self) -> Result<Vec<(String, String)>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            r#"SELECT caller_symbol, callee_symbol
               FROM reference
               WHERE caller_symbol IS NOT NULL
               AND caller_symbol != ''"#,
        )?;
        let records = stmt.query_map(params![], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    // ─── Checksums (Blake3 Integrity) ───

    /// 파일의 checksum 조회 (blake3_hash)
    pub fn get_checksum(&self, file_path: &str) -> Result<Option<String>, anyhow::Error> {
        let hash = self
            .conn
            .query_row(
                "SELECT c.blake3_hash FROM checksums c
                 JOIN files f ON c.file_id = f.id
                 WHERE f.path = ?1",
                params![file_path],
                |row| row.get::<_, String>(0),
            )
            .ok();
        Ok(hash)
    }

    /// 모든 checksum 레코드 조회 (file_path 포함)
    pub fn all_checksums(&self) -> Result<Vec<ChecksumRecord>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.file_id, f.path, c.blake3_hash, c.computed_at, c.verified_at,
                    c.verify_count, c.mismatch_count, c.status
             FROM checksums c
             JOIN files f ON c.file_id = f.id
             ORDER BY c.id",
        )?;
        let records = stmt.query_map(params![], |row| {
            Ok(ChecksumRecord {
                id: row.get(0)?,
                file_id: row.get(1)?,
                file_path: row.get(2)?,
                blake3_hash: row.get(3)?,
                computed_at: row.get(4)?,
                verified_at: row.get(5)?,
                verify_count: row.get(6)?,
                mismatch_count: row.get(7)?,
                status: row.get(8)?,
            })
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// checksum 삽입 또는 업데이트 (files + checksums 테이블 동기화)
    pub fn upsert_checksum(
        &self,
        file_path: &str,
        blake3_hash: &str,
        size: u64,
    ) -> Result<(), anyhow::Error> {
        let now_ms = chrono::Utc::now().timestamp_millis();

        // files 테이블에 먼저 삽입/업데이트
        self.conn.execute(
            r#"INSERT INTO files (path, abs_path, size_bytes, blake3_hash, indexed_at, status)
               VALUES (?1, ?1, ?2, ?3, ?4, 'valid')
               ON CONFLICT(path) DO UPDATE SET
                 blake3_hash = excluded.blake3_hash,
                 size_bytes = excluded.size_bytes,
                 indexed_at = excluded.indexed_at,
                 status = 'valid'"#,
            params![file_path, size, blake3_hash, now_ms],
        )?;

        let file_id: i64 = self.conn.query_row(
            "SELECT id FROM files WHERE path = ?1",
            params![file_path],
            |row| row.get(0),
        )?;

        // checksums 테이블 UPSERT
        self.conn.execute(
            r#"INSERT INTO checksums (file_id, blake3_hash, computed_at, verified_at)
               VALUES (?1, ?2, ?3, ?3)
               ON CONFLICT(file_id) DO UPDATE SET
                 blake3_hash = excluded.blake3_hash,
                 computed_at = excluded.computed_at,
                 verified_at = excluded.verified_at,
                 status = 'synced'"#,
            params![file_id, blake3_hash, now_ms],
        )?;

        Ok(())
    }

    /// checksum verified_at + verify_count 증가
    pub fn touch_checksum_verified(&self, file_path: &str) -> Result<(), anyhow::Error> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.conn.execute(
            r#"UPDATE checksums SET verified_at = ?1, verify_count = verify_count + 1
               WHERE file_id = (SELECT id FROM files WHERE path = ?2)"#,
            params![now_ms, file_path],
        )?;
        Ok(())
    }

    /// checksum mismatch 기록
    pub fn record_checksum_mismatch(&self, file_path: &str) -> Result<(), anyhow::Error> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.conn.execute(
            r#"UPDATE checksums SET mismatch_count = mismatch_count + 1,
                 last_mismatch_at = ?1, status = 'mismatch'
               WHERE file_id = (SELECT id FROM files WHERE path = ?2)"#,
            params![now_ms, file_path],
        )?;
        Ok(())
    }

    // ─── Dirty Queue ───

    /// dirty_queue에 엔트리 삽입
    pub fn insert_dirty(
        &self,
        file_path: &str,
        reason: &str,
        priority: i32,
    ) -> Result<(), anyhow::Error> {
        let now_ms = chrono::Utc::now().timestamp_millis();

        // 파일이 없으면 먼저 files 테이블에 생성
        self.conn.execute(
            r#"INSERT INTO files (path, abs_path, size_bytes, blake3_hash, indexed_at, status)
               VALUES (?1, ?1, 0, '', ?2, 'dirty')
               ON CONFLICT(path) DO UPDATE SET status = 'dirty'"#,
            params![file_path, now_ms],
        )?;

        let file_id: i64 = self.conn.query_row(
            "SELECT id FROM files WHERE path = ?1",
            params![file_path],
            |row| row.get(0),
        )?;

        // 이미 pending이면 삭제 후 재삽입 (upsert)
        self.conn.execute(
            "DELETE FROM dirty_queue WHERE file_id = ?1 AND status = 'pending'",
            params![file_id],
        )?;

        self.conn.execute(
            r#"INSERT INTO dirty_queue (file_id, reason, priority, enqueued_at, status)
               VALUES (?1, ?2, ?3, ?4, 'pending')"#,
            params![file_id, reason, priority, now_ms],
        )?;

        Ok(())
    }

    /// pending dirty queue 엔트리 조회 (우선순위 정렬)
    pub fn dirty_queue_entries(&self) -> Result<Vec<DirtyQueueRecord>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT dq.id, dq.file_id, f.path, dq.reason, dq.priority,
                    dq.enqueued_at, dq.processed_at, dq.retry_count, dq.error_log, dq.status
             FROM dirty_queue dq
             JOIN files f ON dq.file_id = f.id
             WHERE dq.status = 'pending'
             ORDER BY dq.priority DESC, dq.enqueued_at ASC",
        )?;
        let records = stmt.query_map(params![], |row| {
            Ok(DirtyQueueRecord {
                id: row.get(0)?,
                file_id: row.get(1)?,
                file_path: row.get(2)?,
                reason: row.get(3)?,
                priority: row.get(4)?,
                enqueued_at: row.get(5)?,
                processed_at: row.get(6)?,
                retry_count: row.get(7)?,
                error_log: row.get(8)?,
                status: row.get(9)?,
            })
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// dirty 큐에서 파일 제거 (처리 완료)
    pub fn clear_dirty(&self, file_path: &str) -> Result<(), anyhow::Error> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.conn.execute(
            r#"UPDATE dirty_queue SET status = 'processed', processed_at = ?1
               WHERE file_id = (SELECT id FROM files WHERE path = ?2)"#,
            params![now_ms, file_path],
        )?;
        Ok(())
    }

    /// 파일 상태 업데이트
    pub fn update_file_status(&self, file_path: &str, status: &str) -> Result<(), anyhow::Error> {
        self.conn.execute(
            "UPDATE files SET status = ?1 WHERE path = ?2",
            params![status, file_path],
        )?;
        Ok(())
    }

    // ─── Agent Cache ───

    /// 캐시 조회 (TTL 체크 포함, hit_count +1)
    ///
    /// `query_key`로 캐시된 결과를 조회하고, 유효한 경우 `hit_count`를 1 증가시킵니다.
    /// 증가된 `hit_count` 값을 반환합니다.
    /// 만료된 캐시는 `None`을 반환합니다.
    pub fn get_cached(&self, query_key: &str) -> Option<AgentCacheRecord> {
        let now_ms = chrono::Utc::now().timestamp_millis();

        // 1. hit_count 증가 (TTL 체크 포함 — 만료된 항목은 업데이트하지 않음)
        if self
            .conn
            .execute(
                "UPDATE agent_cache SET hit_count = hit_count + 1 \n                 WHERE query_key = ?1 AND expires_at > ?2",
                params![query_key, now_ms],
            )
            .ok()
            .unwrap_or(0)
            == 0
        {
            return None; // 만료됨 또는 존재하지 않음
        }

        // 2. 업데이트된 값 조회
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, query_key, query_hash, result_json, created_at, expires_at, hit_count
                 FROM agent_cache
                 WHERE query_key = ?1",
            )
            .ok()?;
        stmt.query_row(params![query_key], |row| {
            Ok(AgentCacheRecord {
                id: row.get(0)?,
                query_key: row.get(1)?,
                query_hash: row.get(2)?,
                result_json: row.get(3)?,
                created_at: row.get(4)?,
                expires_at: row.get(5)?,
                hit_count: row.get(6)?,
            })
        })
        .ok()
    }

    /// 캐시 저장 (query_key → result_json, TTL 초)
    ///
    /// `query_key`가 이미 존재하면 결과를 덮어쓰고 `hit_count`를 초기화합니다.
    /// `ttl_seconds`는 캐시 유효 기간(초)입니다.
    pub fn set_cached(
        &self,
        query_key: &str,
        result_json: &str,
        ttl_seconds: u64,
    ) -> Result<(), anyhow::Error> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let expires_at = now_ms + (ttl_seconds * 1_000) as i64;
        let query_hash = blake3::hash(query_key.as_bytes()).to_hex().to_string();

        self.conn.execute(
            r#"INSERT INTO agent_cache (query_key, query_hash, result_json, created_at, expires_at)
               VALUES (?1, ?2, ?3, ?4, ?5)
               ON CONFLICT(query_key) DO UPDATE SET
                 result_json = excluded.result_json,
                 query_hash = excluded.query_hash,
                 created_at = excluded.created_at,
                 expires_at = excluded.expires_at,
                 hit_count = 0"#,
            params![query_key, query_hash, result_json, now_ms, expires_at],
        )?;
        Ok(())
    }

    /// 캐시 무효화
    ///
    /// 특정 `query_key`에 해당하는 캐시 엔트리를 삭제합니다.
    pub fn invalidate_cached(&self, query_key: &str) -> Result<(), anyhow::Error> {
        self.conn.execute(
            "DELETE FROM agent_cache WHERE query_key = ?1",
            params![query_key],
        )?;
        Ok(())
    }

    /// 만료된 캐시 전체 정리
    ///
    /// `expires_at`가 지난 모든 캐시 엔트리를 삭제하고, 삭제된 개수를 반환합니다.
    pub fn purge_expired(&self) -> Result<usize, anyhow::Error> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let deleted = self.conn.execute(
            "DELETE FROM agent_cache WHERE expires_at <= ?1",
            params![now_ms],
        )?;
        Ok(deleted)
    }

    /// 모든 활성 캐시 엔트리 조회 (hit_count 내림차순)
    ///
    /// 만료되지 않은 캐시만 반환합니다. 디버깅 및 모니터링용입니다.
    pub fn all_cached(&self) -> Result<Vec<AgentCacheRecord>, anyhow::Error> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut stmt = self.conn.prepare(
            "SELECT id, query_key, query_hash, result_json, created_at, expires_at, hit_count
             FROM agent_cache
             WHERE expires_at > ?1
             ORDER BY hit_count DESC",
        )?;
        let records = stmt.query_map(params![now_ms], |row| {
            Ok(AgentCacheRecord {
                id: row.get(0)?,
                query_key: row.get(1)?,
                query_hash: row.get(2)?,
                result_json: row.get(3)?,
                created_at: row.get(4)?,
                expires_at: row.get(5)?,
                hit_count: row.get(6)?,
            })
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// 캐시 통계 (총 엔트리 수, 활성, 만료됨)
    pub fn cache_stats(&self) -> Result<(usize, usize, usize), anyhow::Error> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let total: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM agent_cache", [], |r| r.get(0))
            .unwrap_or(0);
        let active: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM agent_cache WHERE expires_at > ?1",
                params![now_ms],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let expired = total - active;
        Ok((total, active, expired))
    }

    /// Flush SQLite WAL mode — checkpoint all WAL frames to the main database.
    /// Use before graceful shutdown to ensure durability.
    pub fn flush_wal(&self) -> Result<(), anyhow::Error> {
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .context("WAL checkpoint")?;
        Ok(())
    }

    // ================================================================
    // Override registry
    // ================================================================

    /// Insert a manual reference override
    pub fn insert_override(
        &self,
        caller: &str,
        callee: &str,
        kind: &str,
        confidence: f64,
        reason: &str,
    ) -> Result<i64, anyhow::Error> {
        let mut stmt = self
            .conn
            .prepare(
                "INSERT INTO overrides (caller_symbol, callee_symbol, ref_kind, confidence, reason) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .context("prepare insert_override")?;
        stmt.execute(rusqlite::params![caller, callee, kind, confidence, reason])
            .context("execute insert_override")?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Remove an override by ID
    pub fn remove_override(&self, id: i64) -> Result<usize, anyhow::Error> {
        self.conn
            .execute("DELETE FROM overrides WHERE id = ?1", rusqlite::params![id])
            .context("execute remove_override")
    }

    /// List all overrides
    pub fn list_overrides(&self) -> Result<Vec<OverrideRecord>, anyhow::Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, caller_symbol, callee_symbol, ref_kind, confidence, reason, \
                       created_at, updated_at FROM overrides ORDER BY id",
            )
            .context("prepare list_overrides")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(OverrideRecord {
                    id: r.get(0)?,
                    caller_symbol: r.get(1)?,
                    callee_symbol: r.get(2)?,
                    ref_kind: r.get(3)?,
                    confidence: r.get(4)?,
                    reason: r.get(5)?,
                    created_at: r.get(6)?,
                    updated_at: r.get(7)?,
                })
            })
            .context("query list_overrides")?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("collect list_overrides")
    }

    /// Get overrides for a specific symbol (as caller or callee)
    pub fn overrides_for_symbol(&self, symbol: &str) -> Result<Vec<OverrideRecord>, anyhow::Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, caller_symbol, callee_symbol, ref_kind, confidence, reason, \
                       created_at, updated_at FROM overrides \
                       WHERE caller_symbol = ?1 OR callee_symbol = ?1 ORDER BY id",
            )
            .context("prepare overrides_for_symbol")?;
        let rows = stmt
            .query_map(rusqlite::params![symbol], |r| {
                Ok(OverrideRecord {
                    id: r.get(0)?,
                    caller_symbol: r.get(1)?,
                    callee_symbol: r.get(2)?,
                    ref_kind: r.get(3)?,
                    confidence: r.get(4)?,
                    reason: r.get(5)?,
                    created_at: r.get(6)?,
                    updated_at: r.get(7)?,
                })
            })
            .context("query overrides_for_symbol")?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("collect overrides_for_symbol")
    }

    // ─── String Literals (Cross-ref Engine) ───

    /// 문자열 리터럴 삽입
    pub fn insert_string_literal(
        &self,
        file_path: &str,
        line: usize,
        col: usize,
        value: &str,
        is_symbol_like: bool,
        context: &str,
        parent_fn: Option<&str>,
    ) -> Result<i64, anyhow::Error> {
        self.conn.execute(
            r#"INSERT INTO string_literals (file_path, line_number, col_start, string_value, is_symbol_like, context, parent_fn)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![file_path, line, col, value, if is_symbol_like { 1 } else { 0 }, context, parent_fn],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 파일의 문자열 리터럴 삭제 (재인덱싱 전 정리)
    pub fn remove_file_string_literals(&self, path: &str) -> Result<(), anyhow::Error> {
        self.conn.execute(
            "DELETE FROM potential_string_refs WHERE literal_id IN (SELECT id FROM string_literals WHERE file_path = ?1)",
            params![path],
        )?;
        self.conn.execute(
            "DELETE FROM string_literals WHERE file_path = ?1",
            params![path],
        )?;
        Ok(())
    }

    /// 심볼 유사 문자열 리터럴 조회 (교차 매칭용)
    pub fn get_symbol_like_literals(&self) -> Result<Vec<(i64, String, String, i32, String, Option<String>)>, anyhow::Error> {
        // Returns: (id, file_path, string_value, line_number, context, parent_fn)
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, string_value, line_number, context, parent_fn
             FROM string_literals
             WHERE is_symbol_like = 1"
        )?;
        let rows = stmt.query_map(params![], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("query symbol-like literals")
    }

    /// 잠재 문자열 참조 삽입
    pub fn insert_potential_string_ref(
        &self,
        literal_id: i64,
        target_symbol_id: i64,
        confidence: f64,
        match_type: &str,
    ) -> Result<(), anyhow::Error> {
        self.conn.execute(
            r#"INSERT OR IGNORE INTO potential_string_refs (literal_id, target_symbol_id, confidence, match_type)
               VALUES (?1, ?2, ?3, ?4)"#,
            params![literal_id, target_symbol_id, confidence, match_type],
        )?;
        Ok(())
    }

    /// 심볼의 잠재 문자열 참조 조회
    pub fn get_potential_refs_for_symbol(
        &self,
        symbol_name: &str,
        min_confidence: f64,
    ) -> Result<Vec<(i64, String, i64, f64, String)>, anyhow::Error> {
        // Returns: (psr.id, file_path, literal_id, confidence, match_type)
        let mut stmt = self.conn.prepare(
            r#"SELECT psr.id, sl.file_path, psr.literal_id, psr.confidence, psr.match_type
               FROM potential_string_refs psr
               JOIN string_literals sl ON psr.literal_id = sl.id
               JOIN symbol s ON psr.target_symbol_id = s.id
               WHERE s.name = ?1 OR s.qualified_name = ?1
               AND psr.confidence >= ?2
               ORDER BY psr.confidence DESC"#
        )?;
        let rows = stmt.query_map(params![symbol_name, min_confidence], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("query potential string refs")
    }
}

#[cfg(test)]
mod qualified_name_tests {
    use super::*;

    #[test]
    fn test_module_name_from_path() {
        assert_eq!(SymbolRecord::module_name_from_path("auth.py"), "auth");
        assert_eq!(SymbolRecord::module_name_from_path("session.py"), "session");
        assert_eq!(SymbolRecord::module_name_from_path("src/auth.py"), "auth");
        assert_eq!(
            SymbolRecord::module_name_from_path("src/session.py"),
            "session"
        );
        assert_eq!(
            SymbolRecord::module_name_from_path("lib/session.py"),
            "session"
        );
    }

    #[test]
    fn test_build_qualified_name() {
        assert_eq!(
            SymbolRecord::build_qualified_name("auth.py", "login", &None),
            "auth.login"
        );
        assert_eq!(
            SymbolRecord::build_qualified_name(
                "session.py",
                "create",
                &Some("Session".to_string())
            ),
            "session.Session.create"
        );
    }
}
