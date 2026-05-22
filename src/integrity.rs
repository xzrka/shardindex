/// Blake3 Integrity Guard Layer
///
/// 파일 무결성 검증 + 자동 dirty 큐 등록
///
/// ## 기능
/// 1. `verify_file(path)` — 디스크 파일의 Blake3 해시를 계산하여 DB checksum과 비교
/// 2. `verify_all()` — 모든 인덱싱된 파일의 무결성 일괄 검증
/// 3. `add_dirty(file_path, reason, priority)` — hash mismatch 시 dirty_queue에 등록
/// 4. `process_dirty(db, root, language)` — dirty 큐에서 대기 중인 파일들을 재인덱싱
///
/// ## 워크플로우
/// ```
/// 파일 변경 감지 (watcher)
///     → Blake3 hash 재계산
///     → DB checksum과 비교
///     → mismatch 시 dirty_queue INSERT (reason="hash_changed", priority=1)
///     → process_dirty() 호출 → 재인덱싱 → checksums UPDATE
/// ```

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::Context;
use tracing::{debug, info, warn};

use crate::database::IndexDb;
use crate::indexer::{Language, ProjectIndexer};

/// Integrity verification result for a single file
#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub file_path: String,
    pub status: VerifyStatus,
    pub stored_hash: Option<String>,
    pub disk_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VerifyStatus {
    /// Hash matches — file is clean
    Clean,
    /// Hash mismatch — file needs re-indexing
    Dirty,
    /// File not found on disk
    Missing,
    /// File has no stored checksum in DB
    Unknown,
}

/// Blake3 integrity guard
pub struct IntegrityGuard;

impl IntegrityGuard {
    /// Compute Blake3 hash of a file on disk
    pub fn compute_file_hash(path: &Path) -> anyhow::Result<String> {
        let content =
            fs::read(path).with_context(|| format!("Read file for hashing: {}", path.display()))?;
        Ok(blake3::hash(&content).to_hex().to_string())
    }

    /// Compute Blake3 hash from content bytes
    pub fn compute_content_hash(content: &[u8]) -> String {
        blake3::hash(content).to_hex().to_string()
    }

    /// Verify a single file against its stored checksum
    pub fn verify_file(db: &IndexDb, root: &Path, file_path: &str) -> anyhow::Result<VerifyResult> {
        // Look up stored checksum
        let stored = db.get_checksum(file_path)?;

        let disk_path = root.join(file_path);
        let disk_hash = if disk_path.exists() {
            Self::compute_file_hash(&disk_path).ok()
        } else {
            None
        };

        let stored_for_result = stored.clone();
        let status = match (stored, disk_hash.as_ref()) {
            (Some(stored_hash), Some(dh)) => {
                if stored_hash == *dh {
                    VerifyStatus::Clean
                } else {
                    VerifyStatus::Dirty
                }
            }
            (None, Some(_)) => VerifyStatus::Unknown,
            (_, None) => VerifyStatus::Missing,
        };

        Ok(VerifyResult {
            file_path: file_path.to_string(),
            status,
            stored_hash: stored_for_result,
            disk_hash,
        })
    }

    /// Verify all indexed files and return dirty ones
    pub fn verify_all(db: &IndexDb, root: &Path) -> anyhow::Result<Vec<VerifyResult>> {
        let checksums = db.all_checksums()?;
        let mut results = Vec::new();

        for cs in checksums {
            let result = Self::verify_file(db, root, &cs.file_path)?;
            results.push(result);
        }

        Ok(results)
    }

    /// Add a dirty entry to the queue
    pub fn add_dirty(
        db: &IndexDb,
        file_path: &str,
        reason: &str,
        priority: i32,
    ) -> anyhow::Result<()> {
        db.insert_dirty(file_path, reason, priority)?;
        debug!(
            "Dirty queue: {} (reason={}, priority={})",
            file_path, reason, priority
        );
        Ok(())
    }

    /// Check if a file is dirty and re-index if needed
    pub fn check_and_process(
        db: &IndexDb,
        root: &Path,
        file_path: &str,
        language: Language,
    ) -> anyhow::Result<bool> {
        let result = Self::verify_file(db, root, file_path)?;

        match result.status {
            VerifyStatus::Dirty => {
                info!(
                    "Integrity check: {} is dirty, adding to dirty queue",
                    file_path
                );
                Self::add_dirty(db, file_path, "hash_mismatch", 1)?;
                Self::process_single(db, root, file_path, language)?;
                Ok(true)
            }
            VerifyStatus::Unknown => {
                info!(
                    "Integrity check: {} unknown (no checksum), adding to dirty queue",
                    file_path
                );
                Self::add_dirty(db, file_path, "no_checksum", 0)?;
                Self::process_single(db, root, file_path, language)?;
                Ok(true)
            }
            VerifyStatus::Missing => {
                warn!(
                    "Integrity check: {} missing on disk, marking dirty",
                    file_path
                );
                Self::add_dirty(db, file_path, "file_missing", -1)?;
                Ok(false)
            }
            VerifyStatus::Clean => {
                db.touch_checksum_verified(file_path)?;
                Ok(false)
            }
        }
    }

    /// Process a single dirty file: re-index and update checksum
    fn process_single(
        db: &IndexDb,
        root: &Path,
        file_path: &str,
        language: Language,
    ) -> anyhow::Result<()> {
        let disk_path = root.join(file_path);
        if !disk_path.exists() {
            warn!("File missing, skipping re-index: {}", file_path);
            return Ok(());
        }

        let content =
            fs::read_to_string(&disk_path).with_context(|| format!("Read file: {}", file_path))?;
        let new_hash = Self::compute_content_hash(content.as_bytes());
        let size = content.len() as u64;

        let mut indexer = ProjectIndexer::new(db.clone(), root.to_path_buf(), language)?;
        let (syms, refs) = indexer.index_file(&disk_path)?;

        // Update checksum in DB
        db.upsert_checksum(file_path, &new_hash, size)?;

        // Remove from dirty queue
        db.clear_dirty(file_path)?;

        info!(
            "Processed dirty file: {} ({} symbols, {} refs, hash={})",
            file_path,
            syms,
            refs,
            &new_hash[..8]
        );

        Ok(())
    }

    /// Process all dirty queue entries
    pub fn process_dirty_queue(
        db: &IndexDb,
        root: &Path,
        language: Language,
    ) -> anyhow::Result<usize> {
        let dirty_entries = db.dirty_queue_entries()?;
        if dirty_entries.is_empty() {
            return Ok(0);
        }

        info!(
            "Processing dirty queue: {} entries",
            dirty_entries.len()
        );

        let mut processed = 0;
        let mut failed: HashSet<String> = HashSet::new();

        for entry in &dirty_entries {
            let disk_path = root.join(&entry.file_path);

            if !disk_path.exists() {
                // Remove missing files from index
                db.remove_file(&entry.file_path)?;
                db.clear_dirty(&entry.file_path)?;
                processed += 1;
                continue;
            }

            match Self::check_and_process(db, root, &entry.file_path, language) {
                Ok(_) => {
                    processed += 1;
                }
                Err(e) => {
                    warn!(
                        "Failed to process dirty file {}: {}",
                        entry.file_path, e
                    );
                    failed.insert(entry.file_path.clone());
                }
            }
        }

        if !failed.is_empty() {
            warn!(
                "Dirty queue: {} files processed, {} failed",
                processed,
                failed.len()
            );
        }

        Ok(processed)
    }

    /// Scan for new files that aren't in the checksum table yet
    pub fn scan_new_files(
        db: &IndexDb,
        root: &Path,
        language: Language,
    ) -> anyhow::Result<Vec<String>> {
        let extensions: Vec<String> =
            language.extensions().iter().map(|s| s.to_string()).collect();
        let existing_files: HashSet<String> = db
            .all_checksums()?
            .iter()
            .map(|cs| cs.file_path.clone())
            .collect();

        let mut new_files = Vec::new();

        for entry in walkdir::WalkDir::new(root).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let ext = match path.extension().and_then(|e| e.to_str()) {
                Some(e) => e.to_string(),
                None => continue,
            };

            if !extensions.contains(&ext) {
                continue;
            }

            if Self::is_ignored(path, root) {
                continue;
            }

            let relative = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            if !existing_files.contains(&relative) {
                new_files.push(relative);
            }
        }

        Ok(new_files)
    }

    fn is_ignored(path: &Path, root: &Path) -> bool {
        let skip = [
            ".git", "__pycache__", ".venv", "venv", "node_modules", ".mypy_cache",
            ".tox", ".eggs", "target", "dist", "build", ".next", ".nuxt", ".shardindex",
        ];

        let relative = match path.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => path,
        };

        for component in relative.components() {
            let name = component.as_os_str().to_string_lossy();
            if skip.contains(&name.as_ref()) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_db() -> (IndexDb, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test_integrity.db");
        let db = IndexDb::open(db_path.to_str().unwrap()).unwrap();
        (db, dir)
    }

    #[test]
    fn test_compute_hash() {
        let hash1 = IntegrityGuard::compute_content_hash(b"hello world");
        let hash2 = IntegrityGuard::compute_content_hash(b"hello world");
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // Blake3 hex = 64 chars
    }

    #[test]
    fn test_compute_hash_different_content() {
        let hash1 = IntegrityGuard::compute_content_hash(b"hello");
        let hash2 = IntegrityGuard::compute_content_hash(b"world");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_verify_file_clean() {
        let (db, dir) = test_db();
        let test_file = dir.path().join("test.py");
        fs::write(&test_file, "def foo():\n    pass\n").unwrap();

        let hash = IntegrityGuard::compute_file_hash(&test_file).unwrap();
        db.upsert_checksum("test.py", &hash, 22).unwrap();

        let result = IntegrityGuard::verify_file(&db, dir.path(), "test.py").unwrap();
        assert_eq!(result.status, VerifyStatus::Clean);
    }

    #[test]
    fn test_verify_file_dirty() {
        let (db, dir) = test_db();
        let test_file = dir.path().join("test.py");
        fs::write(&test_file, "def foo():\n    pass\n").unwrap();

        // Store a different hash
        db.upsert_checksum("test.py", "0000000000000000000000000000000000000000000000000000000000000000", 22)
            .unwrap();

        let result = IntegrityGuard::verify_file(&db, dir.path(), "test.py").unwrap();
        assert_eq!(result.status, VerifyStatus::Dirty);
    }

    #[test]
    fn test_verify_file_missing() {
        let (db, _dir) = test_db();
        db.upsert_checksum("nonexistent.py", "abc123", 100).unwrap();

        let result =
            IntegrityGuard::verify_file(&db, Path::new("."), "nonexistent.py").unwrap();
        assert_eq!(result.status, VerifyStatus::Missing);
    }

    #[test]
    fn test_add_and_clear_dirty() {
        let (db, _dir) = test_db();
        db.insert_dirty("test.py", "hash_changed", 1).unwrap();

        let entries = db.dirty_queue_entries().unwrap();
        assert!(!entries.is_empty());
        assert_eq!(entries[0].file_path, "test.py");

        db.clear_dirty("test.py").unwrap();
        let entries = db.dirty_queue_entries().unwrap();
        assert!(entries.is_empty());
    }
}
