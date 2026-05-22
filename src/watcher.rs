/// Multi-language event-driven file watcher.
///
/// Automatically detects file language from extension and routes
/// changes to the daemon's dirty queue.  Supports configurable
/// debounce window, batch size, and ignored directories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use notify::{
    event::ModifyKind, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::daemon::{Daemon, DirtyEventType};
use crate::database::IndexDb;
use crate::indexer::{Language, ProjectIndexer};

/// Debounce state for a single file path.
#[derive(Debug)]
struct DebounceEntry {
    last_event: Instant,
}

/// Shared debounce map — protected by a Mutex.
type DebounceMap = Arc<Mutex<HashMap<PathBuf, DebounceEntry>>>;

/// Multi-language file watcher — detects language from file extension
pub struct FileWatcher {
    root: PathBuf,
    config: Config,
    watcher: Option<RecommendedWatcher>,
    daemon: Option<Daemon>,
    /// Custom ignored directories (merged with defaults)
    ignore_dirs: Vec<String>,
}

impl FileWatcher {
    /// Create a new multi-language watcher
    pub fn new(root: PathBuf, config: Config) -> Self {
        Self {
            root,
            config,
            watcher: None,
            daemon: None,
            ignore_dirs: Vec::new(),
        }
    }

    /// Set custom ignored directories (in addition to defaults)
    pub fn with_ignore_dirs(mut self, dirs: Vec<String>) -> Self {
        self.ignore_dirs = dirs;
        self
    }

    /// Start the watcher and daemon
    pub fn start(mut self) -> anyhow::Result<(Self, tokio::task::JoinHandle<()>)> {
        let debounce_ms = self.config.watcher.debounce_ms;
        let debounce: DebounceMap = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = std::sync::mpsc::channel::<PathBuf>();
        let tx: Arc<Mutex<std::sync::mpsc::Sender<PathBuf>>> = Arc::new(Mutex::new(tx));

        // Start daemon
        let mut daemon = Daemon::new(self.root.clone(), self.config.clone());
        daemon.start()?;
        self.daemon = Some(daemon);
        let daemon_ref = self.daemon.as_ref().unwrap();

        // Clone for the event handler
        let root_watch = self.root.clone();
        let debounce_watch = debounce.clone();
        let tx_watch = tx.clone();
        let ignore_dirs = self.ignore_dirs.clone();
        let daemon_shared = daemon_ref.shared_state();

        // --- Event handler closure (runs in the notify thread) ---
        let event_handler =
            move |event_result: Result<Event, notify::Error>| {
                match event_result {
                    Ok(event) => handle_event_multi(
                        &root_watch,
                        &debounce_watch,
                        &tx_watch,
                        &daemon_shared,
                        event,
                        &ignore_dirs,
                    ),
                    Err(e) => warn!("Watch error: {}", e),
                }
            };

        let mut watcher = notify::recommended_watcher(event_handler)
            .context("Failed to create file watcher")?;

        watcher
            .watch(&self.root, RecursiveMode::Recursive)
            .context(format!("Failed to watch directory {}", self.root.display()))?;

        info!(
            "Multi-language file watcher started for {} (debounce={}ms)",
            self.root.display(),
            debounce_ms
        );

        // --- Debouncer background task ---
        let debounce_loop = debounce.clone();
        let db_path = self.config.db_path.clone();
        let root_loop = self.root.clone();
        let daemon_shared_loop = daemon_ref.shared_state();
        let config_loop = self.config.clone();
        let ignore_dirs_loop = self.ignore_dirs.clone();

        let handle = tokio::spawn(async move {
            debouncer_loop_multi(
                debounce_loop,
                db_path,
                root_loop,
                rx,
                daemon_shared_loop,
                config_loop,
                ignore_dirs_loop,
            )
            .await;
        });

        self.watcher = Some(watcher);
        Ok((self, handle))
    }

    /// Stop the watcher and daemon
    pub fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(mut daemon) = self.daemon.take() {
            daemon.stop()?;
        }
        // Dropping the watcher stops the notify thread
        self.watcher = None;
        info!("File watcher stopped");
        Ok(())
    }

    /// Get daemon state reference
    pub fn daemon_state(&self) -> Option<crate::daemon::DaemonState> {
        self.daemon.as_ref().map(|d| d.state())
    }
}

/// Process a notify event with multi-language detection
fn handle_event_multi(
    root: &Path,
    debounce: &DebounceMap,
    tx: &Arc<Mutex<std::sync::mpsc::Sender<PathBuf>>>,
    daemon_shared: &Arc<Mutex<crate::daemon::DaemonSharedState>>,
    event: Event,
    extra_ignore_dirs: &[String],
) {
    // Determine event type
    let event_type = match event.kind {
        EventKind::Modify(ModifyKind::Name(_))
        | EventKind::Modify(ModifyKind::Data(_))
        | EventKind::Modify(ModifyKind::Any) => DirtyEventType::Modified,
        EventKind::Create(_) => DirtyEventType::Created,
        EventKind::Remove(_) => DirtyEventType::Removed,
        _ => return,
    };

    let now = Instant::now();

    for path in event.paths {
        // Skip files that don't have a supported language
        if path.extension().map_or(true, |e| {
            let ext_str = e.to_str().unwrap_or("");
            Language::from_extension(&format!(".{}", ext_str)).is_none()
        }) {
            continue;
        }

        // Skip ignored directories
        if path_is_ignored(&path, root, extra_ignore_dirs) {
            continue;
        }

        // Enqueue directly to daemon's dirty queue
        let mut state = daemon_shared.lock().unwrap();
        state.enqueue_dirty(crate::daemon::DirtyEvent {
            path: path.clone(),
            event_time: now,
            event_type,
        });

        // Also apply debounce for channel forwarding
        let mut map = debounce.lock().unwrap();
        let prev = map.insert(path.clone(), DebounceEntry { last_event: now });

        let should_process = if let Some(entry) = prev {
            now.duration_since(entry.last_event).as_millis() > debounce_ms_as_u128()
        } else {
            false
        };

        if should_process {
            drop(map);
            if let Ok(sender) = tx.lock() {
                let _ = sender.send(path);
            }
        }
    }
}

/// Default debounce window in milliseconds
const fn debounce_ms_as_u128() -> u128 {
    200
}

/// Multi-language debouncer loop
async fn debouncer_loop_multi(
    debounce: DebounceMap,
    db_path: String,
    root: PathBuf,
    rx: std::sync::mpsc::Receiver<PathBuf>,
    daemon_shared: Arc<Mutex<crate::daemon::DaemonSharedState>>,
    config: Config,
    extra_ignore_dirs: Vec<String>,
) {
    let debounce_ms = config.watcher.debounce_ms;
    let max_batch = config.watcher.max_batch_size;

    loop {
        tokio::time::sleep(Duration::from_millis(debounce_ms.max(50))).await;

        let now = Instant::now();
        let mut to_process = Vec::new();

        // Clean up debounce map — entries older than debounce window
        {
            let mut map = debounce.lock().unwrap();
            map.retain(|path, entry| {
                if now.duration_since(entry.last_event) >= Duration::from_millis(debounce_ms) {
                    to_process.push(path.clone());
                    false // remove
                } else {
                    true // keep
                }
            });
        }

        // Drain channel
        while let Ok(path) = rx.try_recv() {
            to_process.push(path);
        }

        // Also drain daemon dirty queue
        let daemon_events = {
            let mut state = daemon_shared.lock().unwrap();
            state.drain_dirty()
        };

        // Merge daemon events into to_process
        for evt in daemon_events {
            if !to_process.contains(&evt.path) {
                to_process.push(evt.path);
            }
        }

        if to_process.is_empty() {
            continue;
        }

        // Deduplicate and batch
        to_process.sort();
        to_process.dedup();

        let batch: Vec<_> = to_process.into_iter().take(max_batch).collect();

        debug!(
            "Multi-watcher debouncer: {} file(s) ready for re-index (batch_max={})",
            batch.len(),
            max_batch
        );

        for path in batch {
            reindex_auto_detect(&db_path, &root, &path, &extra_ignore_dirs);
        }
    }
}

/// Re-index a file with auto-detected language
fn reindex_auto_detect(
    db_path: &str,
    root: &Path,
    file_path: &Path,
    _extra_ignore_dirs: &[String],
) {
    let relative = file_path
        .strip_prefix(root)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();

    // Auto-detect language from extension
    let language = match Language::from_extension(&file_path.to_string_lossy()) {
        Some(lang) => lang,
        None => {
            debug!("Skipping unsupported file: {}", relative);
            return;
        }
    };

    let start = Instant::now();

    match IndexDb::open(db_path) {
        Ok(db) => match ProjectIndexer::new(db, root.to_path_buf(), language) {
            Ok(mut indexer) => match indexer.index_file(file_path) {
                Ok((syms, refs)) => {
                    let elapsed = start.elapsed();
                    debug!(
                        "Auto-reindexed {}: {} symbols, {} refs [{}] in {:.1}ms",
                        relative,
                        syms,
                        refs,
                        language.as_str(),
                        elapsed.as_secs_f64() * 1000.0
                    );
                }
                Err(e) => warn!("Re-index error for {}: {}", relative, e),
            },
            Err(e) => warn!("Indexer creation error for {}: {}", relative, e),
        },
        Err(e) => warn!("DB open error: {}", e),
    }
}

/// Polling fallback: re-index all files with auto-detection.
pub fn poll_fallback(
    db_path: &str,
    root: &Path,
    config: &Config,
) -> anyhow::Result<()> {
    // Walk all supported files
    use walkdir::WalkDir;

    let mut total_files: usize = 0;
    let mut total_symbols: usize = 0;
    let mut total_refs: usize = 0;

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        if path_is_ignored(path, root, &config.watcher.ignore_dirs) {
            continue;
        }

        let lang = match Language::from_extension(&path.to_string_lossy()) {
            Some(lang) => lang,
            None => continue,
        };

        let db = match IndexDb::open(db_path) {
            Ok(db) => db,
            Err(_) => continue,
        };

        match ProjectIndexer::new(db, root.to_path_buf(), lang) {
            Ok(mut indexer) => match indexer.index_file(path) {
                Ok((syms, refs)) => {
                    total_files += 1;
                    total_symbols += syms;
                    total_refs += refs;
                }
                Err(e) => warn!("Poll error for {}: {}", path.display(), e),
            },
            Err(e) => warn!("Indexer error for {}: {}", path.display(), e),
        }
    }

    debug!(
        "Poll fallback: {} files, {} symbols, {} refs",
        total_files, total_symbols, total_refs
    );

    Ok(())
}

/// Check if a path falls under an ignored directory.
fn path_is_ignored(path: &Path, root: &Path, extra_ignore_dirs: &[String]) -> bool {
    let skip = [
        ".git",
        "__pycache__",
        ".venv",
        "venv",
        "node_modules",
        ".mypy_cache",
        ".tox",
        ".eggs",
        "target",
        ".shardindex",
        ".DS_Store",
        "dist",
        "build",
        "out",
        ".next",
        ".nuxt",
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
        // Check extra ignore dirs
        if extra_ignore_dirs.iter().any(|d| d == name.as_ref()) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_is_ignored_default() {
        let root = Path::new("/project");

        assert!(path_is_ignored(
            &PathBuf::from("/project/.git/config"),
            root,
            &[]
        ));
        assert!(path_is_ignored(
            &PathBuf::from("/project/node_modules/pkg/index.js"),
            root,
            &[]
        ));
        assert!(path_is_ignored(
            &PathBuf::from("/project/__pycache__/module.cpython-39.pyc"),
            root,
            &[]
        ));
        assert!(path_is_ignored(
            &PathBuf::from("/project/.shardindex/journal.jsonl"),
            root,
            &[]
        ));
        assert!(!path_is_ignored(
            &PathBuf::from("/project/src/main.rs"),
            root,
            &[]
        ));
    }

    #[test]
    fn test_path_is_ignored_custom() {
        let root = Path::new("/project");

        assert!(!path_is_ignored(
            &PathBuf::from("/project/vendor/lib.rs"),
            root,
            &[]
        ));

        assert!(path_is_ignored(
            &PathBuf::from("/project/vendor/lib.rs"),
            root,
            &["vendor".to_string()]
        ));
    }

    #[test]
    fn test_language_extension_detection() {
        // Verify all 18 languages can be detected
        assert!(Language::from_extension("test.py").is_some());
        assert!(Language::from_extension("test.js").is_some());
        assert!(Language::from_extension("test.ts").is_some());
        assert!(Language::from_extension("test.rs").is_some());
        assert!(Language::from_extension("test.go").is_some());
        assert!(Language::from_extension("test.rb").is_some());
        assert!(Language::from_extension("test.java").is_some());
        assert!(Language::from_extension("test.php").is_some());
        assert!(Language::from_extension("test.jl").is_some());
        assert!(Language::from_extension("test.lua").is_some());
        assert!(Language::from_extension("test.swift").is_some());
        assert!(Language::from_extension("test.zig").is_some());
        assert!(Language::from_extension("test.scala").is_some());
        assert!(Language::from_extension("test.ex").is_some());
        assert!(Language::from_extension("test.dart").is_some());
        assert!(Language::from_extension("test.hs").is_some());
        assert!(Language::from_extension("test.c").is_some());
        assert!(Language::from_extension("test.cpp").is_some());

        // Unsupported
        assert!(Language::from_extension("test.txt").is_none());
        assert!(Language::from_extension("test.md").is_none());
        assert!(Language::from_extension("test.bin").is_none());
    }

    #[test]
    fn test_watcher_new() {
        let config = crate::config::Config::default();
        let watcher = FileWatcher::new(PathBuf::from("/tmp"), config);
        assert!(watcher.daemon_state() == None);
    }

    #[test]
    fn test_watcher_with_ignore_dirs() {
        let config = crate::config::Config::default();
        let _watcher = FileWatcher::new(PathBuf::from("/tmp"), config)
            .with_ignore_dirs(vec!["custom_ignore".to_string()]);
    }
}
