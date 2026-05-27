/// Background daemon — state machine for incremental indexing
///
/// Implements the Idle → Dirty → Parsing → Persist → UpdateRefs state
/// machine described in masterplan §7.1.  Coordinates file watcher,
/// dirty queue processing, and crash recovery.
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::database::IndexDb;
use crate::indexer::{Language, ProjectIndexer};

/// Daemon state machine states (masterplan §7.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    /// Idle — no pending work, index is clean
    Idle,
    /// Dirty — file events received, waiting for debounce window
    Dirty,
    /// Parsing — actively parsing changed files
    Parsing,
    /// Persist — writing parsed results to database
    Persist,
    /// UpdateRefs — updating reference graph after persist
    UpdateRefs,
    /// Recover — handling a parsing/persist failure
    Recover,
    /// Shutdown — daemon is shutting down gracefully
    Shutdown,
}

impl std::fmt::Display for DaemonState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonState::Idle => write!(f, "Idle"),
            DaemonState::Dirty => write!(f, "Dirty"),
            DaemonState::Parsing => write!(f, "Parsing"),
            DaemonState::Persist => write!(f, "Persist"),
            DaemonState::UpdateRefs => write!(f, "UpdateRefs"),
            DaemonState::Recover => write!(f, "Recover"),
            DaemonState::Shutdown => write!(f, "Shutdown"),
        }
    }
}

/// A dirty file event — queued for processing
#[derive(Debug, Clone)]
pub struct DirtyEvent {
    pub path: PathBuf,
    pub event_time: Instant,
    pub event_type: DirtyEventType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyEventType {
    Created,
    Modified,
    Removed,
}

/// Shared daemon state
#[derive(Debug)]
pub struct DaemonSharedState {
    /// Current state of the state machine
    state: DaemonState,
    /// Timestamp of last state transition
    last_transition: Instant,
    /// Number of files processed since last idle
    files_processed: usize,
    /// Number of errors encountered
    error_count: usize,
    /// Total symbols indexed
    total_symbols: usize,
    /// Total references indexed
    total_refs: usize,
    /// Current batch of dirty files
    dirty_queue: VecDeque<DirtyEvent>,
}

impl DaemonSharedState {
    pub fn new() -> Self {
        Self {
            state: DaemonState::Idle,
            last_transition: Instant::now(),
            files_processed: 0,
            error_count: 0,
            total_symbols: 0,
            total_refs: 0,
            dirty_queue: VecDeque::new(),
        }
    }

    pub fn state(&self) -> DaemonState {
        self.state
    }

    pub fn set_state(&mut self, new_state: DaemonState) {
        if self.state != new_state {
            debug!("State transition: {} → {}", self.state, new_state);
            self.state = new_state;
            self.last_transition = Instant::now();
        }
    }

    pub fn enqueue_dirty(&mut self, event: DirtyEvent) {
        self.dirty_queue.push_back(event);
        if self.state == DaemonState::Idle {
            self.set_state(DaemonState::Dirty);
        }
    }

    pub fn drain_dirty(&mut self) -> Vec<DirtyEvent> {
        let events = self.dirty_queue.drain(..).collect();
        if self.state == DaemonState::Dirty {
            self.set_state(DaemonState::Parsing);
        }
        events
    }
}

/// Background daemon — orchestrates the state machine loop
pub struct Daemon {
    root: PathBuf,
    config: Config,
    shared: Arc<Mutex<DaemonSharedState>>,
    /// Signal to stop the daemon
    shutdown_tx: Option<std::sync::mpsc::Sender<()>>,
    /// Handle to the background task
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Daemon {
    /// Create a new daemon
    pub fn new(root: PathBuf, config: Config) -> Self {
        Self {
            root,
            config,
            shared: Arc::new(Mutex::new(DaemonSharedState::new())),
            shutdown_tx: None,
            handle: None,
        }
    }

    /// Shared state reference (for health checks, status endpoints)
    pub fn shared_state(&self) -> Arc<Mutex<DaemonSharedState>> {
        self.shared.clone()
    }

    /// Start the daemon state machine loop
    pub fn start(&mut self) -> Result<()> {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        self.shutdown_tx = Some(tx);

        let root = self.root.clone();
        let config = self.config.clone();
        let shared = self.shared.clone();
        let debounce_ms = config.watcher.debounce_ms;
        let max_batch = config.watcher.max_batch_size;

        let handle = std::thread::spawn(move || {
            daemon_loop(root, config, shared, rx, debounce_ms, max_batch);
        });

        self.handle = Some(handle);
        info!(
            "Daemon started (debounce={}ms, batch={})",
            debounce_ms, max_batch
        );
        Ok(())
    }

    /// Add a dirty file event from external sources
    pub fn add_dirty_event(&self, path: PathBuf, event_type: DirtyEventType) {
        let event = DirtyEvent {
            path,
            event_time: Instant::now(),
            event_type,
        };

        let mut state = self.shared.lock().unwrap();
        state.enqueue_dirty(event);
    }

    /// Get current state
    pub fn state(&self) -> DaemonState {
        self.shared.lock().unwrap().state()
    }

    /// Graceful shutdown — signal stop and wait for current batch
    pub fn stop(&mut self) -> Result<()> {
        {
            let mut state = self.shared.lock().unwrap();
            state.set_state(DaemonState::Shutdown);
        }

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|e| anyhow::anyhow!("Daemon join error: {:?}", e))?;
        }

        info!("Daemon stopped");
        Ok(())
    }
}

/// Main daemon loop — runs in a background thread
fn daemon_loop(
    root: PathBuf,
    config: Config,
    shared: Arc<Mutex<DaemonSharedState>>,
    mut shutdown_rx: std::sync::mpsc::Receiver<()>,
    debounce_ms: u64,
    max_batch: usize,
) {
    let check_interval = Duration::from_millis(debounce_ms.max(50));

    // Set up a separate thread for shutdown signaling that doesn't block the loop
    loop {
        // Check shutdown signal (non-blocking)
        if shutdown_rx.try_recv().is_ok() {
            let mut state = shared.lock().unwrap();
            state.set_state(DaemonState::Shutdown);
            info!("Daemon shutdown signal received, finishing current batch");

            // Process any remaining dirty files before exiting
            process_remaining_batch(&root, &config, &shared);
            break;
        }

        // State machine tick
        let state = {
            let guard = shared.lock().unwrap();
            guard.state()
        };

        match state {
            DaemonState::Shutdown => break,
            DaemonState::Dirty => {
                // Wait for debounce window
                std::thread::sleep(check_interval);
                // Fall through to process
                process_batch(&root, &config, &shared, max_batch);
            }
            DaemonState::Parsing | DaemonState::Persist | DaemonState::UpdateRefs => {
                // These states are set/cleared within process_batch
                std::thread::sleep(check_interval);
            }
            DaemonState::Idle => {
                // Nothing to do, sleep and check again
                std::thread::sleep(check_interval);
            }
            DaemonState::Recover => {
                // Recovery: brief pause then back to idle
                std::thread::sleep(Duration::from_millis(100));
                let mut state = shared.lock().unwrap();
                state.set_state(DaemonState::Idle);
            }
        }
    }

    debug!("Daemon loop exited");
}

/// Process a batch of dirty files through the state machine
fn process_batch(
    root: &Path,
    config: &Config,
    shared: &Arc<Mutex<DaemonSharedState>>,
    max_batch: usize,
) {
    // Drain dirty queue
    let events = {
        let mut state = shared.lock().unwrap();
        // Transition: Dirty → Parsing
        state.set_state(DaemonState::Parsing);
        state.drain_dirty()
    };

    if events.is_empty() {
        let mut state = shared.lock().unwrap();
        state.set_state(DaemonState::Idle);
        return;
    }

    let events: Vec<_> = events.into_iter().take(max_batch).collect();

    let mut symbols_total: usize = 0;
    let mut refs_total: usize = 0;
    let mut errors: usize = 0;

    for event in &events {
        let result = process_single_file(root, config, &event);
        match result {
            Ok((syms, refs)) => {
                symbols_total += syms;
                refs_total += refs;
            }
            Err(e) => {
                errors += 1;
                warn!("Failed to process {}: {}", event.path.display(), e);
            }
        }
    }

    // Update shared state with results
    {
        let mut state = shared.lock().unwrap();
        state.files_processed += events.len();
        state.total_symbols += symbols_total;
        state.total_refs += refs_total;

        if errors > 0 {
            state.error_count += errors;
            state.set_state(DaemonState::Recover);
        } else {
            // Transition: Persist → UpdateRefs → Idle
            state.set_state(DaemonState::Persist);
            state.set_state(DaemonState::UpdateRefs);
            state.set_state(DaemonState::Idle);
        }
    }

    if symbols_total > 0 || refs_total > 0 {
        debug!(
            "Batch processed: {} files, {} symbols, {} refs, {} errors",
            events.len(),
            symbols_total,
            refs_total,
            errors
        );
    }
}

/// Process remaining dirty files before shutdown
fn process_remaining_batch(root: &Path, config: &Config, shared: &Arc<Mutex<DaemonSharedState>>) {
    let events = {
        let mut state = shared.lock().unwrap();
        let events = state.drain_dirty();
        if events.is_empty() {
            return;
        }
        events
    };

    info!(
        "Processing {} remaining files before shutdown",
        events.len()
    );

    for event in &events {
        let _ = process_single_file(root, config, event);
    }

    // Force flush SQLite WAL
    if let Ok(db) = IndexDb::open(&config.db_path) {
        let _ = db.flush_wal();
    }
}

/// Process a single file through the state machine
///
/// Implements masterplan §7.2 Incremental Update Rules:
/// 1. Parse file → new symbols, new refs
/// 2. Soft-delete old symbols/refs for this file
/// 3. Insert new symbols/refs
/// 4. Update file hash and timestamp
/// 5. Remove from dirty queue
fn process_single_file(root: &Path, config: &Config, event: &DirtyEvent) -> Result<(usize, usize)> {
    let path = &event.path;

    match event.event_type {
        DirtyEventType::Removed => {
            // Handle file deletion
            let relative = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            if let Ok(db) = IndexDb::open(&config.db_path) {
                let _ = db.remove_file(&relative);
                debug!("Removed deleted file from index: {}", relative);
            }
            return Ok((0, 0));
        }
        _ => {}
    }

    // Skip if file doesn't exist (race condition with removal)
    if !path.exists() {
        return Ok((0, 0));
    }

    // Detect language from file extension
    let language = match Language::from_extension(&path.to_string_lossy()) {
        Some(lang) => lang,
        None => {
            debug!("Skipping unsupported file: {}", path.display());
            return Ok((0, 0));
        }
    };

    // Open DB and create indexer
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    let db =
        IndexDb::open(&config.db_path).context(format!("Open DB for reindex: {}", relative))?;

    let mut indexer = ProjectIndexer::new(db, root.to_path_buf(), language)
        .context(format!("Create indexer for {}", relative))?;

    // Index the file (implements §7.2 rules)
    let (symbols, refs) = indexer
        .index_file(path)
        .context(format!("Index file: {}", relative))?;

    debug!(
        "Processed {}: {} symbols, {} refs [{}]",
        relative,
        symbols,
        refs,
        language.as_str()
    );

    Ok((symbols, refs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_state_transitions() {
        let mut state = DaemonSharedState::new();

        // Initial state
        assert_eq!(state.state(), DaemonState::Idle);

        // Add dirty event → Idle → Dirty
        state.enqueue_dirty(DirtyEvent {
            path: PathBuf::from("test.py"),
            event_time: Instant::now(),
            event_type: DirtyEventType::Modified,
        });
        assert_eq!(state.state(), DaemonState::Dirty);

        // Drain → Dirty → Parsing
        let events = state.drain_dirty();
        assert_eq!(events.len(), 1);
        assert_eq!(state.state(), DaemonState::Parsing);
    }

    #[test]
    fn test_daemon_state_display() {
        assert_eq!(format!("{}", DaemonState::Idle), "Idle");
        assert_eq!(format!("{}", DaemonState::Dirty), "Dirty");
        assert_eq!(format!("{}", DaemonState::Parsing), "Parsing");
        assert_eq!(format!("{}", DaemonState::Persist), "Persist");
        assert_eq!(format!("{}", DaemonState::UpdateRefs), "UpdateRefs");
        assert_eq!(format!("{}", DaemonState::Recover), "Recover");
        assert_eq!(format!("{}", DaemonState::Shutdown), "Shutdown");
    }

    #[test]
    fn test_multiple_dirty_events_coalesce() {
        let mut state = DaemonSharedState::new();

        // Add multiple events
        for i in 0..5 {
            state.enqueue_dirty(DirtyEvent {
                path: PathBuf::from(format!("test{}.py", i)),
                event_time: Instant::now(),
                event_type: DirtyEventType::Modified,
            });
        }

        // Should still be Dirty, not transitioned
        assert_eq!(state.state(), DaemonState::Dirty);

        // Drain should collect all
        let events = state.drain_dirty();
        assert_eq!(events.len(), 5);
        assert_eq!(state.state(), DaemonState::Parsing);
    }

    #[test]
    fn test_daemon_new() {
        let config = crate::config::Config::default();
        let daemon = Daemon::new(PathBuf::from("/tmp"), config);
        assert_eq!(daemon.state(), DaemonState::Idle);
    }

    #[test]
    fn test_add_dirty_event() {
        let config = crate::config::Config::default();
        let mut daemon = Daemon::new(PathBuf::from("/tmp"), config);

        daemon.add_dirty_event(PathBuf::from("/tmp/test.py"), DirtyEventType::Modified);

        assert_eq!(daemon.state(), DaemonState::Dirty);
    }

    #[test]
    fn test_process_file_removed() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            db_path: dir.path().join("test.db").to_string_lossy().to_string(),
            ..Default::default()
        };

        let event = DirtyEvent {
            path: dir.path().join("deleted.py"),
            event_time: Instant::now(),
            event_type: DirtyEventType::Removed,
        };

        let result = process_single_file(dir.path(), &config, &event);
        // Should succeed even if file doesn't exist (graceful handling)
        assert!(result.is_ok());
    }

    #[test]
    fn test_process_unsupported_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            db_path: dir.path().join("test.db").to_string_lossy().to_string(),
            ..Default::default()
        };

        // Create a .txt file (unsupported)
        let txt_file = dir.path().join("readme.txt");
        std::fs::write(&txt_file, "Hello world").unwrap();

        let event = DirtyEvent {
            path: txt_file,
            event_time: Instant::now(),
            event_type: DirtyEventType::Modified,
        };

        let (syms, refs) = process_single_file(dir.path(), &config, &event).unwrap();
        assert_eq!(syms, 0);
        assert_eq!(refs, 0);
    }
}
