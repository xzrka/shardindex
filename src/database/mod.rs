pub mod schema;
pub use schema::init_db;

/// SQLite 데이터베이스 연결 관리
///
/// 파일 레벨 CRUD — 인덱싱 상태 조회

use rusqlite::params;
use rusqlite::Connection;
use anyhow::Context;

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

pub struct IndexDb {
    pub conn: Connection,
}

impl IndexDb {
    /// 새 데이터베이스 연결 생성 + 스키마 초기화
    pub fn open(db_path: &str) -> Result<Self, anyhow::Error> {
        let conn = Connection::open(db_path).context(format!("DB open failed: {}", db_path))?;
        init_db(&conn)?;
        Ok(Self { conn })
    }

    // ─── File Hash ───

    /// 파일 해시 저장 또는 업데이트
    pub fn upsert_file(&self, path: &str, hash: &str, size: u64, modified: &str) -> Result<(), anyhow::Error> {
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
        self.conn.query_row(
            "SELECT hash FROM file_hash WHERE path = ?1",
            params![path],
            |row| row.get(0),
        ).ok()
    }

    /// 변경된 파일 목록 조회 (인메모리 해시 맵과 비교용)
    pub fn all_file_hashes(&self) -> Result<Vec<FileRecord>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT path, hash, size, modified FROM file_hash ORDER BY path"
        )?;
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
            "DELETE FROM file_hash WHERE path = ?1",
            params![path],
        )?;
        self.conn.execute(
            "DELETE FROM symbol WHERE file_path = ?1",
            params![path],
        )?;
        self.conn.execute(
            "DELETE FROM reference WHERE caller_file = ?1",
            params![path],
        )?;
        Ok(())
    }

    /// 파일의 심볼만 삭제 (재인덱싱 전 정리)
    pub fn remove_file_symbols(&self, path: &str) -> Result<(), anyhow::Error> {
        self.conn.execute(
            "DELETE FROM symbol WHERE file_path = ?1",
            params![path],
        )?;
        self.conn.execute(
            "DELETE FROM reference WHERE caller_file = ?1",
            params![path],
        )?;
        Ok(())
    }

    // ─── Symbols ───

    /// 심볼 삽입
    pub fn insert_symbol(&self, rec: &SymbolRecord) -> Result<i64, anyhow::Error> {
        let _id = self.conn.last_insert_rowid();
        self.conn.execute(
            r#"INSERT INTO symbol (file_path, name, kind, start_line, end_line, start_col, end_col, signature, docstring, parent_symbol)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
            params![
                rec.file_path, rec.name, rec.kind,
                rec.start_line, rec.end_line, rec.start_col, rec.end_col,
                rec.signature, rec.docstring, rec.parent_symbol
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 파일의 모든 심볼 조회
    pub fn file_symbols(&self, file_path: &str) -> Result<Vec<SymbolRecord>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, name, kind, start_line, end_line, start_col, end_col, signature, docstring, parent_symbol
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
                parent_symbol: row.get(10)?,
            })
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// 심볼명 검색
    pub fn search_symbol(&self, pattern: &str) -> Result<Vec<SymbolRecord>, anyhow::Error> {
        let search = format!("%{}%", pattern);
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, name, kind, start_line, end_line, start_col, end_col, signature, docstring, parent_symbol
             FROM symbol WHERE name LIKE ?1 ORDER BY kind, name LIMIT 50"
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
                parent_symbol: row.get(10)?,
            })
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    // ─── References ───

    /// 참조 삽입
    pub fn insert_reference(&self, rec: &ReferenceRecord) -> Result<i64, anyhow::Error> {
        self.conn.execute(
            r#"INSERT OR IGNORE INTO reference (caller_file, callee_file, caller_symbol, callee_symbol, ref_kind, line)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![
                rec.caller_file, rec.callee_file,
                rec.caller_symbol, rec.callee_symbol,
                rec.ref_kind, rec.line
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 심볼의 neighbors (직접 참조하는/참조받는 심볼)
    pub fn neighbors(&self, symbol_name: &str) -> Result<Vec<ReferenceRecord>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, caller_file, callee_file, caller_symbol, callee_symbol, ref_kind, line
               FROM reference
               WHERE caller_symbol = ?1 OR callee_symbol = ?1
               ORDER BY ref_kind, line
               LIMIT 100"#
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
            })
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// 심볼 영향도 분석 — 이 심볼을 사용하는 모든 파일/심볼
    pub fn impact(&self, symbol_name: &str) -> Result<(Vec<SymbolRecord>, Vec<ReferenceRecord>), anyhow::Error> {
        let callers: Vec<SymbolRecord> = {
            let mut stmt = self.conn.prepare(
                r#"SELECT id, file_path, name, kind, start_line, end_line, start_col, end_col, signature, docstring, parent_symbol
                   FROM symbol WHERE name IN (SELECT DISTINCT caller_symbol FROM reference WHERE callee_symbol = ?1)
                   LIMIT 50"#
            )?;
            stmt.query_map(params![symbol_name], |row| {
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
                    parent_symbol: row.get(10)?,
                })
            })?.collect::<Result<Vec<_>, _>>()?
        };

        let all_refs = self.neighbors(symbol_name)?;
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
                    s.signature, s.docstring, s.parent_symbol, sr.page_rank
             FROM symbol s
             LEFT JOIN symbol_rank sr ON s.name = sr.symbol_name
             WHERE s.name LIKE ?1
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
                    parent_symbol: row.get(10)?,
                },
                row.get::<_, Option<f64>>(11)?,
            ))
        })?;
        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// 심볼 영향도 분석 + 랭킹 정보 포함
    pub fn impact_ranked(
        &self,
        symbol_name: &str,
    ) -> Result<
        (Vec<(SymbolRecord, Option<f64>)>, Vec<ReferenceRecord>),
        anyhow::Error,
    > {
        // 직접 호출하는 심볼들 (랭킹 JOIN)
        let mut stmt = self.conn.prepare(
            r#"SELECT s.id, s.file_path, s.name, s.kind, s.start_line, s.end_line,
                      s.start_col, s.end_col, s.signature, s.docstring, s.parent_symbol,
                      sr.page_rank
               FROM symbol s
               LEFT JOIN symbol_rank sr ON s.name = sr.symbol_name
               WHERE s.name IN (
                   SELECT DISTINCT caller_symbol FROM reference WHERE callee_symbol = ?1
               )
               ORDER BY sr.page_rank DESC NULLS LAST
               LIMIT 50"#,
        )?;
        let callers = stmt
            .query_map(params![symbol_name], |row| {
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
                        parent_symbol: row.get(10)?,
                    },
                    row.get::<_, Option<f64>>(11)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let all_refs = self.neighbors(symbol_name)?;
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
}
