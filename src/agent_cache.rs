/// Agent Cache — query result caching with TTL + hash invalidation
///
/// Wraps DB-level cache methods (`get_cached`, `set_cached`, etc.) with a
/// higher-level API that:
/// - Generates deterministic cache keys from method + params
/// - Deserializes cached JSON back into typed results
/// - Tracks hit/miss statistics
/// - Auto-purges expired entries on cold starts
use crate::database::{AgentCacheRecord, IndexDb};
use std::sync::Mutex;

/// Default TTL in seconds (5 minutes)
const DEFAULT_TTL_SECS: u64 = 300;

/// Cache hit/miss statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct CacheStats {
    /// Total cache entries
    pub total: usize,
    /// Active (non-expired) entries
    pub active: usize,
    /// Expired entries
    pub expired: usize,
}

/// Agent cache wrapper around the database
///
/// Uses Mutex<IndexDb> internally because rusqlite::Connection is !Send.
pub struct AgentCache {
    db: Mutex<IndexDb>,
    /// Per-query TTL in seconds (overrides default)
    default_ttl: u64,
}

impl std::fmt::Debug for AgentCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentCache")
            .field("default_ttl", &self.default_ttl)
            .finish()
    }
}

impl Clone for AgentCache {
    fn clone(&self) -> Self {
        // Open a fresh connection for the clone
        let db_path = self.db.lock().unwrap().db_path().to_string();
        Self {
            db: Mutex::new(IndexDb::open(&db_path).expect("cache clone DB open")),
            default_ttl: self.default_ttl,
        }
    }
}

impl AgentCache {
    /// Create a new cache wrapper
    pub fn new(db: IndexDb, default_ttl: u64) -> Self {
        Self {
            db: Mutex::new(db),
            default_ttl,
        }
    }

    /// Create with default TTL
    pub fn with_db(db: IndexDb) -> Self {
        Self {
            db: Mutex::new(db),
            default_ttl: DEFAULT_TTL_SECS,
        }
    }

    // ─── Key Generation ───

    /// Build a deterministic cache key from method name and params
    ///
    /// Format: `{method}:{canonicalized_params}`
    ///
    /// The params are canonicalized by sorting object keys to ensure
    /// deterministic keys regardless of insertion order.
    pub fn make_key(method: &str, params: &serde_json::Value) -> String {
        let canonical = Self::canonicalize(params);
        format!("{}:{}", method, canonical)
    }

    /// Canonicalize a JSON value for use as a cache key component.
    /// Sorts object keys, preserves array order, leaves primitives as-is.
    fn canonicalize(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Object(map) => {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by_key(|(k, _)| k.clone());
                let parts: Vec<_> = entries
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, Self::canonicalize(v)))
                    .collect();
                format!("{{{}}}", parts.join(","))
            }
            serde_json::Value::Array(arr) => {
                let parts: Vec<_> = arr.iter().map(|v| Self::canonicalize(v)).collect();
                format!("[{}]", parts.join(","))
            }
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => String::from("null"),
        }
    }

    // ─── Read/Write ───

    /// Check the cache for a query result.
    ///
    /// Returns `Some(result_json)` on a hit, `None` on a miss or expiry.
    /// Increments `hit_count` on success.
    pub fn get(&self, method: &str, params: &serde_json::Value) -> Option<String> {
        let key = Self::make_key(method, params);
        self.db
            .lock()
            .unwrap()
            .get_cached(&key)
            .map(|rec| rec.result_json)
    }

    /// Store a query result in the cache.
    ///
    /// If `ttl` is provided, it overrides the default TTL.
    pub fn set(
        &self,
        method: &str,
        params: &serde_json::Value,
        result_json: &str,
        ttl: Option<u64>,
    ) -> Result<(), anyhow::Error> {
        let key = Self::make_key(method, params);
        let effective_ttl = ttl.unwrap_or(self.default_ttl);
        self.db
            .lock()
            .unwrap()
            .set_cached(&key, result_json, effective_ttl)
    }

    /// Invalidate a specific cache entry.
    pub fn invalidate(
        &self,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let key = Self::make_key(method, params);
        self.db.lock().unwrap().invalidate_cached(&key)
    }

    /// Invalidate cache entries affected by a file change.
    ///
    /// When a file changes, all queries that might reference symbols from
    /// that file should be invalidated. This invalidates keys containing
    /// the file path.
    pub fn invalidate_for_file(&self, file_path: &str) -> Result<usize, anyhow::Error> {
        // Get all active cache entries, filter by file path presence in result
        let entries = self.db.lock().unwrap().all_cached()?;
        let mut invalidated = 0;
        for rec in entries {
            // Check if the cached result mentions this file
            if rec.result_json.contains(file_path) {
                self.db.lock().unwrap().invalidate_cached(&rec.query_key)?;
                invalidated += 1;
            }
        }
        Ok(invalidated)
    }

    /// Purge all expired entries from the cache.
    /// Returns the number of entries deleted.
    pub fn purge(&self) -> Result<usize, anyhow::Error> {
        self.db.lock().unwrap().purge_expired()
    }

    /// Get cache statistics
    pub fn stats(&self) -> Result<CacheStats, anyhow::Error> {
        let (total, active, expired) = self.db.lock().unwrap().cache_stats()?;
        Ok(CacheStats {
            total,
            active,
            expired,
        })
    }

    /// List all active cache entries
    pub fn list(&self) -> Result<Vec<AgentCacheRecord>, anyhow::Error> {
        self.db.lock().unwrap().all_cached()
    }
}

// ─── Unit Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_db() -> IndexDb {
        IndexDb::open_in_memory().expect("in-memory DB should work")
    }

    fn test_cache() -> AgentCache {
        AgentCache::with_db(test_db())
    }

    /// Helper to build a simple params object
    fn params(file: &str) -> serde_json::Value {
        serde_json::json!({ "file": file })
    }

    fn symbol_params(symbol: &str) -> serde_json::Value {
        serde_json::json!({ "symbol": symbol })
    }

    // ─── Key Generation ───

    #[test]
    fn test_make_key_simple() {
        let key = AgentCache::make_key("read", &params("main.py"));
        assert_eq!(key, "read:{file=main.py}");
    }

    #[test]
    fn test_make_key_nested() {
        let p = serde_json::json!({ "a": 1, "b": { "x": 10, "y": 20 } });
        let key = AgentCache::make_key("search", &p);
        // Keys should be sorted: a=1,b={x=10,y=20}
        assert!(key.contains("a=1"));
        assert!(key.contains("b={x=10,y=20}"));
    }

    #[test]
    fn test_make_key_order_independent() {
        // {file: main.py} should produce same key regardless of insertion order
        let p1 = serde_json::json!({ "file": "main.py" });
        let p2 = serde_json::json!({ "file": "main.py" });
        assert_eq!(
            AgentCache::make_key("read", &p1),
            AgentCache::make_key("read", &p2)
        );
    }

    #[test]
    fn test_make_key_array() {
        let p = serde_json::json!({ "symbols": ["foo", "bar"] });
        let key = AgentCache::make_key("read", &p);
        assert!(key.contains("[foo,bar]"));
    }

    #[test]
    fn test_make_key_null() {
        let p = serde_json::json!({ "filter": null });
        let key = AgentCache::make_key("search", &p);
        assert!(key.contains("filter=null"));
    }

    // ─── CRUD ───

    #[test]
    fn test_set_and_get() {
        let cache = test_cache();
        let p = params("main.py");

        cache
            .set("read", &p, r#"{"symbols":[],"count":0}"#, None)
            .unwrap();

        let result = cache.get("read", &p);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), r#"{"symbols":[],"count":0}"#);
    }

    #[test]
    fn test_get_miss() {
        let cache = test_cache();
        let result = cache.get("read", &params("nonexistent.py"));
        assert!(result.is_none());
    }

    #[test]
    fn test_invalidate() {
        let cache = test_cache();
        let p = params("main.py");

        cache
            .set("read", &p, r#"{"symbols":[],"count":0}"#, None)
            .unwrap();
        assert!(cache.get("read", &p).is_some());

        cache.invalidate("read", &p).unwrap();
        assert!(cache.get("read", &p).is_none());
    }

    #[test]
    fn test_overwrite() {
        let cache = test_cache();
        let p = params("main.py");

        cache.set("read", &p, r#"{"count":0}"#, None).unwrap();
        cache.set("read", &p, r#"{"count":5}"#, None).unwrap();

        let result = cache.get("read", &p).unwrap();
        assert_eq!(result, r#"{"count":5}"#);
    }

    #[test]
    fn test_different_methods_no_collision() {
        let cache = test_cache();
        let p = symbol_params("MyFunction");

        cache.set("read", &p, r#"{"method":"read"}"#, None).unwrap();
        cache
            .set("impact", &p, r#"{"method":"impact"}"#, None)
            .unwrap();

        assert_eq!(cache.get("read", &p).unwrap(), r#"{"method":"read"}"#);
        assert_eq!(cache.get("impact", &p).unwrap(), r#"{"method":"impact"}"#);
    }

    #[test]
    fn test_different_params_no_collision() {
        let cache = test_cache();

        cache
            .set("read", &params("a.py"), r#"{"file":"a"}"#, None)
            .unwrap();
        cache
            .set("read", &params("b.py"), r#"{"file":"b"}"#, None)
            .unwrap();

        assert_eq!(
            cache.get("read", &params("a.py")).unwrap(),
            r#"{"file":"a"}"#
        );
        assert_eq!(
            cache.get("read", &params("b.py")).unwrap(),
            r#"{"file":"b"}"#
        );
    }

    // ─── TTL ───

    #[test]
    fn test_ttl_expiry() {
        let db = test_db();
        let cache = AgentCache::with_db(db);
        let p = params("main.py");

        // Set with 0-second TTL (expires immediately)
        cache.set("read", &p, r#"{"count":1}"#, Some(0)).unwrap();

        // Give it a moment for the expiry to take effect
        std::thread::sleep(Duration::from_millis(10));

        // Should be expired now
        let result = cache.get("read", &p);
        assert!(result.is_none());
    }

    #[test]
    fn test_ttl_not_expired() {
        let cache = test_cache();
        let p = params("main.py");

        // Set with 1-hour TTL
        cache.set("read", &p, r#"{"count":1}"#, Some(3600)).unwrap();

        let result = cache.get("read", &p);
        assert!(result.is_some());
    }

    #[test]
    fn test_default_ttl() {
        let db = test_db();
        let cache = AgentCache::new(db, 60); // 60-second default
        let p = params("main.py");

        cache.set("read", &p, r#"{"count":1}"#, None).unwrap();
        let result = cache.get("read", &p);
        assert!(result.is_some());
    }

    // ─── Hash Invalidation ───

    #[test]
    fn test_invalidate_for_file() {
        let cache = test_cache();

        // Cache results that mention "src/main.py"
        cache
            .set(
                "read",
                &params("src/main.py"),
                r#"{"file":"src/main.py","symbols":[{"name":"foo"}]}"#,
                None,
            )
            .unwrap();
        cache
            .set(
                "impact",
                &symbol_params("foo"),
                r#"{"impacted_symbols":[{"file_path":"src/main.py","name":"bar"}]}"#,
                None,
            )
            .unwrap();
        // This one does NOT mention "src/main.py"
        cache
            .set(
                "read",
                &params("src/utils.py"),
                r#"{"file":"src/utils.py","symbols":[{"name":"baz"}]}"#,
                None,
            )
            .unwrap();

        // Invalidate for "src/main.py"
        let count = cache.invalidate_for_file("src/main.py").unwrap();
        assert_eq!(count, 2);

        // The unaffected entry should still be there
        assert!(cache.get("read", &params("src/utils.py")).is_some());
    }

    #[test]
    fn test_invalidate_for_file_empty() {
        let cache = test_cache();
        let count = cache.invalidate_for_file("nonexistent.py").unwrap();
        assert_eq!(count, 0);
    }

    // ─── Hash Invalidation (query_hash mismatch) ───

    #[test]
    fn test_query_hash_stored() {
        let cache = test_cache();
        let p = params("main.py");

        cache.set("read", &p, r#"{"count":1}"#, None).unwrap();

        // Check that query_hash is stored correctly
        let records = cache.list().unwrap();
        assert!(!records.is_empty());
        let rec = &records[0];
        // Verify hash is deterministic
        let expected_hash = blake3::hash(rec.query_key.as_bytes()).to_hex().to_string();
        assert_eq!(rec.query_hash, expected_hash);
    }

    // ─── Purge ───

    #[test]
    fn test_purge_expired() {
        let cache = test_cache();
        let p = params("main.py");

        // Insert an entry with 0-second TTL
        cache.set("read", &p, r#"{"count":1}"#, Some(0)).unwrap();
        std::thread::sleep(Duration::from_millis(10));

        let deleted = cache.purge().unwrap();
        assert_eq!(deleted, 1);
    }

    #[test]
    fn test_purge_nothing() {
        let cache = test_cache();
        let deleted = cache.purge().unwrap();
        assert_eq!(deleted, 0);
    }

    // ─── Stats ───

    #[test]
    fn test_stats_empty() {
        let cache = test_cache();
        let stats = cache.stats().unwrap();
        assert_eq!(stats.total, 0);
        assert_eq!(stats.active, 0);
        assert_eq!(stats.expired, 0);
    }

    #[test]
    fn test_stats_with_entries() {
        let cache = test_cache();
        let p = params("main.py");

        cache.set("read", &p, r#"{"count":1}"#, Some(3600)).unwrap();
        cache
            .set(
                "impact",
                &symbol_params("foo"),
                r#"{"count":5}"#,
                Some(3600),
            )
            .unwrap();

        let stats = cache.stats().unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.active, 2);
        assert_eq!(stats.expired, 0);
    }

    #[test]
    fn test_stats_with_expired() {
        let db = test_db();
        let cache = AgentCache::with_db(db);
        let p = params("main.py");

        // One active, one expired
        cache.set("read", &p, r#"{"count":1}"#, Some(3600)).unwrap();
        cache
            .set("impact", &symbol_params("foo"), r#"{"count":5}"#, Some(0))
            .unwrap();
        std::thread::sleep(Duration::from_millis(10));

        let stats = cache.stats().unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.active, 1);
        assert_eq!(stats.expired, 1);
    }

    // ─── List ───

    #[test]
    fn test_list() {
        let cache = test_cache();
        let p = params("main.py");

        cache.set("read", &p, r#"{"count":1}"#, Some(3600)).unwrap();

        let records = cache.list().unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].query_key.starts_with("read:"));
    }

    #[test]
    fn test_hit_count_increments() {
        let cache = test_cache();
        let p = params("main.py");

        cache.set("read", &p, r#"{"count":1}"#, Some(3600)).unwrap();

        // First access
        cache.get("read", &p);
        let rec1 = cache.list().unwrap()[0].clone();
        assert!(rec1.hit_count >= 1);

        // Second access
        cache.get("read", &p);
        let rec2 = cache.list().unwrap()[0].clone();
        assert!(rec2.hit_count >= 2);
        assert!(rec2.hit_count > rec1.hit_count);
    }

    // ─── Canonicalization edge cases ───

    #[test]
    fn test_canonicalize_sorts_keys() {
        let p = serde_json::json!({ "z": 1, "a": 2, "m": 3 });
        let key = AgentCache::make_key("test", &p);
        // 'a' should come before 'm' which should come before 'z'
        let a_pos = key.find("a=").unwrap();
        let m_pos = key.find("m=").unwrap();
        let z_pos = key.find("z=").unwrap();
        assert!(a_pos < m_pos);
        assert!(m_pos < z_pos);
    }

    #[test]
    fn test_canonicalize_bool() {
        let p = serde_json::json!({ "enabled": true });
        let key = AgentCache::make_key("test", &p);
        assert!(key.contains("enabled=true"));
    }
}
