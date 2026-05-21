/// Event-driven file watcher using the `notify` crate.
///
/// Replaces the polling-based approach with inotify-backed file change
/// detection.  Debounces rapid bursts of events (e.g. editor saves that
/// fire multiple write events) so that each file is re-indexed at most
/// once per debounce window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use notify::{
    event::ModifyKind, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use tracing::{debug, info, warn};

use crate::database::IndexDb;
use crate::indexer::{Language, ProjectIndexer};

/// How long to wait after the last event for a given file before
/// triggering a re-index.  Editors often fire 2–5 events per save
/// (write, close, chmod, …); 200 ms is enough to coalesce them.
const DEBOUNCE_MS: u64 = 200;

/// Debounce state for a single file path.
#[derive(Debug)]
struct DebounceEntry {
    last_event: Instant,
}

/// Shared debounce map — protected by a Mutex.
type DebounceMap = Arc<Mutex<HashMap<PathBuf, DebounceEntry>>>;

/// Shared sender for debounced file paths.
type FileSender = Arc<Mutex<mpsc::Sender<PathBuf>>>;

/// Start the event-driven file watcher.
///
/// Returns a `(RecommendedWatcher, JoinHandle)` tuple.  Drop the
/// watcher or abort the handle to stop watching.
pub fn start_watcher(
    root: &Path,
    db_path: &str,
    language: Language,
) -> anyhow::Result<(RecommendedWatcher, tokio::task::JoinHandle<()>)> {
    let debounce: DebounceMap = Arc::new(Mutex::new(HashMap::new()));
    let (tx, rx) = mpsc::channel::<PathBuf>();
    let tx: FileSender = Arc::new(Mutex::new(tx));

    // Clone for the event handler
    let root_watch = root.to_path_buf();
    let debounce_watch = debounce.clone();
    let tx_watch = tx.clone();
    let lang_watch = language;

    // --- Event handler closure (runs in the notify thread) ---
    let event_handler = move |event_result: Result<Event, notify::Error>| {
        match event_result {
            Ok(event) => handle_event(&root_watch, &debounce_watch, &tx_watch, event, lang_watch),
            Err(e) => warn!("Watch error: {}", e),
        }
    };

    let mut watcher = notify::recommended_watcher(event_handler)
        .context("Failed to create file watcher")?;

    watcher
        .watch(root, RecursiveMode::Recursive)
        .context(format!("Failed to watch directory {}", root.display()))?;

    info!("File watcher started for {} (recursive)", root.display());

    // --- Debouncer background task ---
    let debounce_loop = debounce.clone();
    let db_path_loop = db_path.to_string();
    let root_loop = root.to_path_buf();
    let lang_loop = language;

    let handle = tokio::spawn(async move {
        debouncer_loop(debounce_loop, db_path_loop, root_loop, rx, lang_loop).await;
    });

    Ok((watcher, handle))
}

/// Process a single notify event: filter, debounce, and forward ready
/// paths to the debouncer task.
fn handle_event(
    root: &Path,
    debounce: &DebounceMap,
    tx: &FileSender,
    event: Event,
    language: Language,
) {
    // Only care about modifications and creates
    let is_relevant = match event.kind {
        EventKind::Modify(ModifyKind::Name(_))
        | EventKind::Modify(ModifyKind::Data(_))
        | EventKind::Modify(ModifyKind::Any) => true,
        EventKind::Create(_) => true,
        EventKind::Remove(_) => true,
        _ => false,
    };

    if !is_relevant {
        return;
    }

    let now = Instant::now();
    let extensions = language.extensions();

    for path in event.paths {
        // Skip files that don't match this language's extensions
        if path.extension().map_or(true, |e| !extensions.contains(&e.to_str().unwrap_or(""))) {
            continue;
        }

        // Skip ignored directories
        if path_is_ignored(&path, root) {
            continue;
        }

        let mut map = debounce.lock().unwrap();
        let prev = map.insert(path.clone(), DebounceEntry { last_event: now });

        // Check if this path already passed the debounce window
        let should_process = if let Some(entry) = prev {
            // Only process if the gap since the PREVIOUS event was
            // larger than DEBOUNCE_MS (meaning we already forwarded
            // this path earlier).  For the very first event we wait.
            now.duration_since(entry.last_event).as_millis() > DEBOUNCE_MS as u128
        } else {
            false
        };

        if should_process {
            drop(map);
            // Best-effort send
            if let Ok(sender) = tx.lock() {
                let _ = sender.send(path);
            }
        }
    }
}

/// Background task that periodically scans the debounce map and
/// re-indexes paths whose last event is older than `DEBOUNCE_MS`.
async fn debouncer_loop(
    debounce: DebounceMap,
    db_path: String,
    root: PathBuf,
    rx: mpsc::Receiver<PathBuf>,
    language: Language,
) {
    loop {
        tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;

        // Collect paths from both the debounce map and the channel
        let now = Instant::now();
        let mut to_process = Vec::new();

        // Clean up debounce map — entries older than DEBOUNCE_MS
        {
            let mut map = debounce.lock().unwrap();
            map.retain(|path, entry| {
                if now.duration_since(entry.last_event)
                    >= Duration::from_millis(DEBOUNCE_MS)
                {
                    to_process.push(path.clone());
                    false // remove from map
                } else {
                    true // keep
                }
            });
        }

        // Also drain any paths sent directly via channel (should_process=true)
        while let Ok(path) = rx.try_recv() {
            to_process.push(path);
        }

        if to_process.is_empty() {
            continue;
        }

        // Deduplicate
        to_process.sort();
        to_process.dedup();

        debug!(
            "Debouncer: {} file(s) ready for re-index",
            to_process.len()
        );

        for path in to_process {
            reindex_single_file(&db_path, &root, &path, language);
        }
    }
}

/// Re-index a single file.  Opens a fresh DB connection, runs the
/// indexer on just that file, then drops everything.
fn reindex_single_file(db_path: &str, root: &Path, file_path: &Path, language: Language) {
    let relative = file_path
        .strip_prefix(root)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();

    match IndexDb::open(db_path) {
        Ok(db) => match ProjectIndexer::new(db, root.to_path_buf(), language) {
            Ok(mut indexer) => match indexer.index_file(file_path) {
                Ok((syms, refs)) => {
                    debug!(
                        "Re-indexed {}: {} symbols, {} refs",
                        relative, syms, refs
                    );
                }
                Err(e) => warn!("Re-index error for {}: {}", relative, e),
            },
            Err(e) => warn!("Indexer creation error: {}", e),
        },
        Err(e) => warn!("DB open error: {}", e),
    }
}

/// Async-friendly version of reindex_single_file — called from tokio task.
/// Runs on a blocking thread to avoid blocking the async runtime.
#[allow(dead_code)]
pub fn reindex_single_file_for_tokio(
    db_path: &str,
    root: &Path,
    file_path: &Path,
    language: Language,
) {
    let db_path = db_path.to_string();
    let root = root.to_path_buf();
    let file_path = file_path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        reindex_single_file(&db_path, &root, &file_path, language);
    });
}

/// Polling fallback: re-index all files.  Used when `poll_interval > 0`
/// as a safety net for systems where inotify is unavailable.
pub fn poll_fallback(db_path: &str, root: &Path, language: Language) -> anyhow::Result<()> {
    match IndexDb::open(db_path) {
        Ok(db) => {
            let mut indexer = ProjectIndexer::new(db, root.to_path_buf(), language)?;
            let (files, symbols, refs) = indexer.index_all()?;
            debug!(
                "Poll fallback: {} files, {} symbols, {} refs",
                files, symbols, refs
            );
        }
        Err(e) => warn!("Poll fallback DB error: {}", e),
    }
    Ok(())
}

/// Check if a path falls under an ignored directory.
fn path_is_ignored(path: &Path, root: &Path) -> bool {
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
