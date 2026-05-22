/// Configuration management — .shardindex/config.toml
///
/// Supports env var overrides and CLI defaults.  Config is loaded
/// once at startup and cloned for daemon/MCP components.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Full ShardIndex configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Database path (default: .shardindex.db)
    #[serde(default = "default_db_path")]
    pub db_path: String,

    /// Daemon settings
    #[serde(default)]
    pub daemon: DaemonConfig,

    /// Watcher settings
    #[serde(default)]
    pub watcher: WatcherConfig,

    /// Indexing settings
    #[serde(default)]
    pub indexing: IndexingConfig,

    /// Search settings
    #[serde(default)]
    pub search: SearchConfig,

    /// MCP server settings
    #[serde(default)]
    pub mcp: McpConfig,

    /// Logging settings
    #[serde(default)]
    pub logging: LoggingConfig,
}

/// Daemon lifecycle configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Listen address for MCP server
    #[serde(default = "default_listen")]
    pub listen: String,

    /// Poll interval in seconds (fallback for systems without inotify; 0 = disabled)
    #[serde(default = "default_poll_interval")]
    pub poll_interval: u64,

    /// Grace period in milliseconds before force shutdown
    #[serde(default = "default_grace_period_ms")]
    pub grace_period_ms: u64,

    /// Auto-recover from crash journal on startup
    #[serde(default = "default_crash_recovery")]
    pub crash_recovery: bool,
}

/// File watcher configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatcherConfig {
    /// Debounce window in milliseconds (editor save coalesce)
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,

    /// Max files to process in a single debounce batch
    #[serde(default = "default_max_batch")]
    pub max_batch_size: usize,

    /// Whether to use event-driven watcher (notify) vs polling
    #[serde(default = "default_event_driven")]
    pub event_driven: bool,

    /// Directories to ignore (in addition to defaults)
    #[serde(default)]
    pub ignore_dirs: Vec<String>,
}

/// Indexing engine configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingConfig {
    /// Whether to use incremental indexing (vs full re-index)
    #[serde(default = "default_incremental")]
    pub incremental: bool,

    /// Max concurrent file parsers
    #[serde(default = "default_max_workers")]
    pub max_workers: usize,

    /// Whether to verify file integrity on every API read
    #[serde(default = "default_integrity_verify")]
    pub integrity_verify: bool,

    /// Soft-delete stale symbols instead of hard delete
    #[serde(default = "default_soft_delete")]
    pub soft_delete: bool,

    /// Batch size for dirty queue processing
    #[serde(default = "default_dirty_batch")]
    pub dirty_batch_size: usize,
}

/// Search configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Default minimum fuzzy score
    #[serde(default = "default_min_score")]
    pub min_score: f64,

    /// Default maximum result count
    #[serde(default = "default_search_limit")]
    pub default_limit: usize,

    /// Whether to include PageRank in search results
    #[serde(default = "default_page_rank")]
    pub page_rank_enabled: bool,
}

/// MCP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    /// Maximum concurrent connections
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,

    /// Request timeout in seconds
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,

    /// Whether to expose REST endpoints (in addition to JSON-RPC)
    #[serde(default = "default_rest_enabled")]
    pub rest_enabled: bool,
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level: trace, debug, info, warn, error
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Log format: json, compact, pretty
    #[serde(default = "default_log_format")]
    pub format: String,
}

// ─── Defaults ───

fn default_db_path() -> String {
    ".shardindex.db".into()
}

fn default_listen() -> String {
    "127.0.0.1:3999".into()
}

fn default_poll_interval() -> u64 {
    0 // disabled, event-driven by default
}

fn default_grace_period_ms() -> u64 {
    5000
}

fn default_crash_recovery() -> bool {
    true
}

fn default_debounce_ms() -> u64 {
    200
}

fn default_max_batch() -> usize {
    50
}

fn default_event_driven() -> bool {
    true
}

fn default_incremental() -> bool {
    true
}

fn default_max_workers() -> usize {
    4
}

fn default_integrity_verify() -> bool {
    true
}

fn default_soft_delete() -> bool {
    true
}

fn default_dirty_batch() -> usize {
    100
}

fn default_min_score() -> f64 {
    0.1
}

fn default_search_limit() -> usize {
    50
}

fn default_page_rank() -> bool {
    true
}

fn default_max_connections() -> usize {
    100
}

fn default_request_timeout() -> u64 {
    30
}

fn default_rest_enabled() -> bool {
    true
}

fn default_log_level() -> String {
    "info".into()
}

fn default_log_format() -> String {
    "compact".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            daemon: Default::default(),
            watcher: Default::default(),
            indexing: Default::default(),
            search: Default::default(),
            mcp: Default::default(),
            logging: Default::default(),
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            poll_interval: default_poll_interval(),
            grace_period_ms: default_grace_period_ms(),
            crash_recovery: default_crash_recovery(),
        }
    }
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            debounce_ms: default_debounce_ms(),
            max_batch_size: default_max_batch(),
            event_driven: default_event_driven(),
            ignore_dirs: Vec::new(),
        }
    }
}

impl Default for IndexingConfig {
    fn default() -> Self {
        Self {
            incremental: default_incremental(),
            max_workers: default_max_workers(),
            integrity_verify: default_integrity_verify(),
            soft_delete: default_soft_delete(),
            dirty_batch_size: default_dirty_batch(),
        }
    }
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            min_score: default_min_score(),
            default_limit: default_search_limit(),
            page_rank_enabled: default_page_rank(),
        }
    }
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            max_connections: default_max_connections(),
            request_timeout: default_request_timeout(),
            rest_enabled: default_rest_enabled(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

// ─── Config Loading ───

/// Default config directory relative to project root
pub fn default_config_dir(root: &Path) -> PathBuf {
    root.join(".shardindex")
}

/// Default config file path
pub fn default_config_file(root: &Path) -> PathBuf {
    default_config_dir(root).join("config.toml")
}

/// Load configuration from file, merging with defaults.
///
/// Precedence: file values > env vars > defaults.
/// Missing file → defaults used (no error).
pub fn load_config(root: &Path) -> Result<Config> {
    let config_path = default_config_file(root);

    if !config_path.exists() {
        return Ok(Config::default());
    }

    let content = std::fs::read_to_string(&config_path)
        .context(format!("Read config file: {}", config_path.display()))?;

    let config: Config = toml::from_str(&content)
        .context(format!("Parse config: {}", config_path.display()))?;

    Ok(config)
}

/// Generate a default config file at the given path
pub fn generate_default_config(root: &Path) -> Result<PathBuf> {
    let config = Config::default();
    let config_path = default_config_file(root);

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).context("Create config directory")?;
    }

    let content = toml::to_string_pretty(&config)?;
    std::fs::write(&config_path, content).context("Write config file")?;

    Ok(config_path)
}

/// Build a tracing subscriber filter string from config
pub fn tracing_filter(config: &Config) -> String {
    format!("shardindex={},tower_http=info", config.logging.level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.db_path, ".shardindex.db");
        assert_eq!(config.daemon.listen, "127.0.0.1:3999");
        assert_eq!(config.watcher.debounce_ms, 200);
        assert_eq!(config.indexing.max_workers, 4);
        assert_eq!(config.search.min_score, 0.1);
        assert!(config.watcher.event_driven);
        assert!(config.indexing.incremental);
    }

    #[test]
    fn test_load_missing_config_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let config = load_config(dir.path()).unwrap();
        assert_eq!(config.db_path, ".shardindex.db");
    }

    #[test]
    fn test_generate_and_load_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = generate_default_config(dir.path()).unwrap();
        assert!(path.exists());

        let config = load_config(dir.path()).unwrap();
        assert_eq!(config.db_path, ".shardindex.db");
        assert_eq!(config.daemon.listen, "127.0.0.1:3999");
    }

    #[test]
    fn test_load_custom_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = default_config_dir(dir.path());
        std::fs::create_dir_all(&config_dir).unwrap();

        let custom = r#"
db_path = "custom.db"

[daemon]
listen = "0.0.0.0:9999"
poll_interval = 5
grace_period_ms = 3000

[watcher]
debounce_ms = 500
max_batch_size = 25

[indexing]
max_workers = 8
integrity_verify = false

[search]
min_score = 0.5
default_limit = 100

[mcp]
max_connections = 50
request_timeout = 60

[logging]
level = "debug"
format = "json"
"#;

        std::fs::write(default_config_file(dir.path()), custom).unwrap();
        let config = load_config(dir.path()).unwrap();

        assert_eq!(config.db_path, "custom.db");
        assert_eq!(config.daemon.listen, "0.0.0.0:9999");
        assert_eq!(config.daemon.poll_interval, 5);
        assert_eq!(config.daemon.grace_period_ms, 3000);
        assert_eq!(config.watcher.debounce_ms, 500);
        assert_eq!(config.watcher.max_batch_size, 25);
        assert_eq!(config.indexing.max_workers, 8);
        assert!(!config.indexing.integrity_verify);
        assert_eq!(config.search.min_score, 0.5);
        assert_eq!(config.search.default_limit, 100);
        assert_eq!(config.mcp.max_connections, 50);
        assert_eq!(config.mcp.request_timeout, 60);
        assert_eq!(config.logging.level, "debug");
        assert_eq!(config.logging.format, "json");
    }

    #[test]
    fn test_partial_config_merges_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = default_config_dir(dir.path());
        std::fs::create_dir_all(&config_dir).unwrap();

        let partial = r#"
[watcher]
debounce_ms = 100
"#;

        std::fs::write(default_config_file(dir.path()), partial).unwrap();
        let config = load_config(dir.path()).unwrap();

        // Custom value
        assert_eq!(config.watcher.debounce_ms, 100);
        // Default values preserved
        assert_eq!(config.db_path, ".shardindex.db");
        assert_eq!(config.daemon.listen, "127.0.0.1:3999");
        assert_eq!(config.indexing.max_workers, 4);
    }
}
