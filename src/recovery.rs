/// Crash recovery journal — WAL-based recovery for the daemon
///
/// Maintains a write-ahead log of indexing operations so that
/// if the daemon crashes mid-batch, it can replay or compensate
/// on restart.  The journal lives in .shardindex/journal.jsonl
/// and is flushed to disk before each DB commit.
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::daemon::{Daemon, DirtyEvent, DirtyEventType};
use crate::database::IndexDb;

/// A single journal entry — one indexing operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Monotonic sequence number
    pub seq: u64,
    /// Timestamp (ms since epoch)
    pub timestamp: i64,
    /// File path (relative to project root)
    pub file_path: String,
    /// Operation type
    #[serde(rename = "op")]
    pub operation: JournalOp,
    /// Status: pending → committed → cleaned
    pub status: JournalStatus,
    /// Blake3 hash of file at time of indexing
    pub file_hash: Option<String>,
}

/// Journal operation types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JournalOp {
    /// File was queued for indexing
    Enqueue,
    /// File was parsed and DB was committed
    Commit,
    /// File was soft-deleted from index
    SoftDelete,
    /// File was removed from index
    Remove,
}

impl std::fmt::Display for JournalOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JournalOp::Enqueue => write!(f, "enqueue"),
            JournalOp::Commit => write!(f, "commit"),
            JournalOp::SoftDelete => write!(f, "soft_delete"),
            JournalOp::Remove => write!(f, "remove"),
        }
    }
}

/// Journal entry status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JournalStatus {
    /// Written to journal, not yet committed to DB
    Pending,
    /// Committed to DB
    Committed,
    /// Cleaned up (old committed entry)
    Cleaned,
}

/// Write-ahead journal for crash recovery
pub struct RecoveryJournal {
    journal_path: PathBuf,
    lock: Mutex<()>,
    /// Sequence counter for journal entries
    seq: Mutex<u64>,
}

impl RecoveryJournal {
    /// Create or open the journal at the given project root
    pub fn new(root: &Path) -> Self {
        let journal_dir = root.join(".shardindex");
        let journal_path = journal_dir.join("journal.jsonl");
        Self {
            journal_path,
            lock: Mutex::new(()),
            seq: Mutex::new(0),
        }
    }

    /// Create or open the journal at a specific path
    pub fn new_at(path: PathBuf) -> Self {
        Self {
            journal_path: path,
            lock: Mutex::new(()),
            seq: Mutex::new(0),
        }
    }

    /// Ensure the journal file exists and load the sequence counter
    fn ensure(&self) -> Result<()> {
        if let Some(parent) = self.journal_path.parent() {
            std::fs::create_dir_all(parent).context("Create journal directory")?;
        }

        // Load last sequence number
        if self.journal_path.exists() {
            let file = File::open(&self.journal_path).context("Open journal file")?;
            let reader = BufReader::new(file);

            let mut max_seq: u64 = 0;
            for line in reader.lines() {
                let line = line.context("Read journal line")?;
                if let Ok(entry) = serde_json::from_str::<JournalEntry>(&line) {
                    if entry.seq > max_seq {
                        max_seq = entry.seq;
                    }
                }
            }

            let mut seq = self.seq.lock().unwrap();
            *seq = max_seq + 1;
        }

        Ok(())
    }

    /// Append a journal entry and fsync
    pub fn append(&self, entry: JournalEntry) -> Result<()> {
        let _lock = self.lock.lock().unwrap();

        if !self.journal_path.exists() {
            self.ensure()?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_path)
            .context("Open journal for append")?;

        {
            let mut handle = std::io::BufWriter::new(file);
            let line = serde_json::to_string(&entry)?;
            writeln!(handle, "{}", line)?;
            handle.flush().context("Flush journal")?;
        }

        // fsync for crash safety
        let file = OpenOptions::new()
            .read(true)
            .open(&self.journal_path)
            .context("Reopen journal for fsync")?;
        file.sync_all().context("fsync journal")?;

        Ok(())
    }

    /// Read all entries from the journal
    pub fn read_all(&self) -> Result<Vec<JournalEntry>> {
        if !self.journal_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.journal_path).context("Open journal file")?;
        let reader = BufReader::new(file);

        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line.context("Read journal line")?;
            if line.is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<JournalEntry>(&line) {
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    /// Truncate the journal after successful recovery
    pub fn truncate(&self) -> Result<()> {
        let _lock = self.lock.lock().unwrap();

        if self.journal_path.exists() {
            std::fs::write(&self.journal_path, "").context("Truncate journal file")?;
        }

        let mut seq = self.seq.lock().unwrap();
        *seq = 0;

        Ok(())
    }

    /// Rotate: archive current journal and start fresh
    pub fn rotate(&self) -> Result<PathBuf> {
        let _lock = self.lock.lock().unwrap();

        let archive = self.journal_path.with_extension("jsonl.archive");

        if self.journal_path.exists() {
            std::fs::rename(&self.journal_path, &archive).context("Rotate journal")?;
        }

        let mut seq = self.seq.lock().unwrap();
        *seq = 0;

        Ok(archive)
    }

    /// Get journal file size in bytes
    pub fn size_bytes(&self) -> u64 {
        std::fs::metadata(&self.journal_path)
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Next sequence number
    fn next_seq(&self) -> u64 {
        let mut seq = self.seq.lock().unwrap();
        let val = *seq;
        *seq += 1;
        val
    }
}

/// Recovery orchestrator — reads journal and replays/compensates
pub struct RecoveryEngine;

impl RecoveryEngine {
    /// Scan the journal for uncommitted operations and recover
    ///
    /// Recovery logic:
    /// 1. Read all journal entries
    /// 2. Find entries with status=Pending
    /// 3. For each pending entry:
    ///    - If file exists and hash matches → re-index
    ///    - If file exists but hash differs → mark for re-index
    ///    - If file doesn't exist → skip
    /// 4. Clean up recovered entries
    pub fn recover(root: &Path, config: &Config) -> Result<RecoveryReport> {
        let journal = RecoveryJournal::new(root);

        let entries = journal.read_all()?;
        if entries.is_empty() {
            return Ok(RecoveryReport {
                total_entries: 0,
                recovered: 0,
                skipped: 0,
                errors: 0,
            });
        }

        let total_entries = entries.len();

        info!("Recovery: scanning {} journal entries", total_entries);

        let mut recovered = 0;
        let mut skipped = 0;
        let mut errors = 0;

        // Filter pending entries
        let pending: Vec<_> = entries
            .into_iter()
            .filter(|e| e.status == JournalStatus::Pending)
            .collect();

        if pending.is_empty() {
            info!("Recovery: no pending entries, journal is clean");
            return Ok(RecoveryReport {
                total_entries,
                recovered: 0,
                skipped: 0,
                errors: 0,
            });
        }

        info!("Recovery: {} pending entries to process", pending.len());

        for entry in pending {
            let file_path = root.join(&entry.file_path);

            // Verify file hash
            let current_hash = if file_path.exists() {
                let content = std::fs::read(&file_path)
                    .with_context(|| format!("Read file: {}", entry.file_path))?;
                Some(blake3::hash(&content).to_hex().to_string())
            } else {
                None
            };

            match (current_hash, entry.file_hash) {
                // File exists and hash matches → DB should already have it
                (Some(current), Some(stored)) if current == stored => {
                    debug!(
                        "Recovery: {} — hash matches, already indexed",
                        entry.file_path
                    );
                    skipped += 1;
                }
                // File exists but hash changed → re-index
                (Some(_), Some(_)) | (Some(_), None) => {
                    info!(
                        "Recovery: {} — re-indexing (hash mismatch or missing)",
                        entry.file_path
                    );
                    match Self::reindex_file(root, config, &entry.file_path) {
                        Ok(_) => recovered += 1,
                        Err(e) => {
                            warn!("Recovery: {} — re-index failed: {}", entry.file_path, e);
                            errors += 1;
                        }
                    }
                }
                // File doesn't exist → skip
                (None, _) => {
                    debug!("Recovery: {} — file removed, skipping", entry.file_path);
                    skipped += 1;
                }
            }
        }

        // Truncate journal after recovery
        journal.truncate()?;

        info!(
            "Recovery complete: {} recovered, {} skipped, {} errors",
            recovered, skipped, errors
        );

        Ok(RecoveryReport {
            total_entries,
            recovered,
            skipped,
            errors,
        })
    }

    /// Re-index a single file during recovery
    fn reindex_file(root: &Path, config: &Config, file_path: &str) -> Result<()> {
        use crate::indexer::{Language, ProjectIndexer};

        let abs_path = root.join(file_path);
        if !abs_path.exists() {
            return Ok(());
        }

        let lang = match Language::from_extension(&file_path) {
            Some(lang) => lang,
            None => return Ok(()),
        };

        let db = IndexDb::open(&config.db_path)?;
        let mut indexer = ProjectIndexer::new(db, root.to_path_buf(), lang)?;
        let _ = indexer.index_file(&abs_path);

        Ok(())
    }

    /// Start the daemon with journal entries queued for recovery
    pub fn start_with_recovery(root: &Path, config: &Config) -> Result<(Daemon, RecoveryReport)> {
        let report = Self::recover(root, config)?;

        let mut daemon = Daemon::new(root.to_path_buf(), config.clone());
        daemon.start()?;

        Ok((daemon, report))
    }
}

/// Report from a recovery operation
#[derive(Debug)]
pub struct RecoveryReport {
    pub total_entries: usize,
    pub recovered: usize,
    pub skipped: usize,
    pub errors: usize,
}

/// Create a journal entry for an operation
pub fn make_journal_entry(
    seq: u64,
    file_path: &str,
    operation: JournalOp,
    status: JournalStatus,
    file_hash: Option<String>,
) -> JournalEntry {
    JournalEntry {
        seq,
        timestamp: chrono::Utc::now().timestamp_millis(),
        file_path: file_path.to_string(),
        operation,
        status,
        file_hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_journal_append_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let journal = RecoveryJournal::new_at(dir.path().join("journal.jsonl"));

        let entry = JournalEntry {
            seq: 1,
            timestamp: 1000,
            file_path: "src/test.py".into(),
            operation: JournalOp::Enqueue,
            status: JournalStatus::Pending,
            file_hash: Some("abc123".into()),
        };

        journal.append(entry.clone()).unwrap();

        let entries = journal.read_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].seq, 1);
        assert_eq!(entries[0].file_path, "src/test.py");
    }

    #[test]
    fn test_journal_multiple_entries() {
        let dir = tempfile::tempdir().unwrap();
        let journal = RecoveryJournal::new_at(dir.path().join("journal.jsonl"));

        for i in 0..5 {
            let entry = JournalEntry {
                seq: i,
                timestamp: 1000 + i as i64,
                file_path: format!("src/test{}.py", i),
                operation: JournalOp::Enqueue,
                status: JournalStatus::Pending,
                file_hash: None,
            };
            journal.append(entry).unwrap();
        }

        let entries = journal.read_all().unwrap();
        assert_eq!(entries.len(), 5);
    }

    #[test]
    fn test_journal_truncate() {
        let dir = tempfile::tempdir().unwrap();
        let journal = RecoveryJournal::new_at(dir.path().join("journal.jsonl"));

        journal
            .append(JournalEntry {
                seq: 1,
                timestamp: 1000,
                file_path: "test.py".into(),
                operation: JournalOp::Enqueue,
                status: JournalStatus::Pending,
                file_hash: None,
            })
            .unwrap();

        journal.truncate().unwrap();

        let entries = journal.read_all().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_journal_empty() {
        let dir = tempfile::tempdir().unwrap();
        let journal = RecoveryJournal::new_at(dir.path().join("journal.jsonl"));

        let entries = journal.read_all().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_journal_rotate() {
        let dir = tempfile::tempdir().unwrap();
        let journal = RecoveryJournal::new_at(dir.path().join("journal.jsonl"));

        journal
            .append(JournalEntry {
                seq: 1,
                timestamp: 1000,
                file_path: "test.py".into(),
                operation: JournalOp::Enqueue,
                status: JournalStatus::Pending,
                file_hash: None,
            })
            .unwrap();

        let archive = journal.rotate().unwrap();
        assert!(archive.exists());

        let entries = journal.read_all().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_journal_op_display() {
        assert_eq!(format!("{}", JournalOp::Enqueue), "enqueue");
        assert_eq!(format!("{}", JournalOp::Commit), "commit");
        assert_eq!(format!("{}", JournalOp::SoftDelete), "soft_delete");
        assert_eq!(format!("{}", JournalOp::Remove), "remove");
    }

    #[test]
    fn test_recovery_no_journal() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            db_path: dir.path().join("test.db").to_string_lossy().to_string(),
            ..Default::default()
        };

        let report = RecoveryEngine::recover(dir.path(), &config).unwrap();
        assert_eq!(report.total_entries, 0);
        assert_eq!(report.recovered, 0);
    }

    #[test]
    fn test_recovery_pending_resolved_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            db_path: dir.path().join("test.db").to_string_lossy().to_string(),
            ..Default::default()
        };

        // Create a test file
        let test_file = dir.path().join("test.py");
        std::fs::write(&test_file, "def hello():\n    pass\n").unwrap();

        let hash = blake3::hash(&std::fs::read(&test_file).unwrap())
            .to_hex()
            .to_string();

        // Create journal with pending entry
        let journal = RecoveryJournal::new(dir.path());
        journal
            .append(JournalEntry {
                seq: 1,
                timestamp: 1000,
                file_path: "test.py".into(),
                operation: JournalOp::Enqueue,
                status: JournalStatus::Pending,
                file_hash: Some(hash),
            })
            .unwrap();

        let report = RecoveryEngine::recover(dir.path(), &config).unwrap();
        assert_eq!(report.total_entries, 1);
        // Hash matches → skipped (already indexed)
        assert_eq!(report.skipped, 1);
    }

    #[test]
    fn test_recovery_deleted_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            db_path: dir.path().join("test.db").to_string_lossy().to_string(),
            ..Default::default()
        };

        // Create journal for a file that doesn't exist
        let journal = RecoveryJournal::new(dir.path());
        journal
            .append(JournalEntry {
                seq: 1,
                timestamp: 1000,
                file_path: "deleted.py".into(),
                operation: JournalOp::Enqueue,
                status: JournalStatus::Pending,
                file_hash: Some("abc123".into()),
            })
            .unwrap();

        let report = RecoveryEngine::recover(dir.path(), &config).unwrap();
        assert_eq!(report.total_entries, 1);
        assert_eq!(report.skipped, 1);
    }

    #[test]
    fn test_journal_size_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let journal = RecoveryJournal::new_at(dir.path().join("journal.jsonl"));

        assert_eq!(journal.size_bytes(), 0);

        journal
            .append(JournalEntry {
                seq: 1,
                timestamp: 1000,
                file_path: "test.py".into(),
                operation: JournalOp::Enqueue,
                status: JournalStatus::Pending,
                file_hash: None,
            })
            .unwrap();

        assert!(journal.size_bytes() > 0);
    }

    #[test]
    fn test_make_journal_entry() {
        let entry = make_journal_entry(
            42,
            "src/main.rs",
            JournalOp::Commit,
            JournalStatus::Committed,
            Some("hash123".into()),
        );

        assert_eq!(entry.seq, 42);
        assert_eq!(entry.file_path, "src/main.rs");
        assert_eq!(entry.operation, JournalOp::Commit);
        assert_eq!(entry.status, JournalStatus::Committed);
        assert_eq!(entry.file_hash, Some("hash123".into()));
    }
}
