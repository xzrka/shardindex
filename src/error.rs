//! Phase 11.1 — Error Taxonomy
//!
//! Complete error types for ShardIndex per masterplan §11.1.
//! Each variant maps to a specific error code and agent action.
//!
//! | Error Code               | Meaning                          | Agent Action                            |
//! |--------------------------|----------------------------------|-----------------------------------------|
//! | `StaleIndex`             | File hash mismatch               | Auto-retry 2×, then filesystem fallback |
//! | `SymbolNotFound`         | Symbol not in index              | `search()` fallback, then filesystem    |
//! | `ParserError`            | File unparseable                 | Report to user, mark file as `corrupted`|
//! | `TokenBudgetExceeded`    | Symbol too large for budget      | Request compression upgrade             |
//! | `RefIntegrityViolation`  | `edit_plan` detected breakage    | Block edit, show impact                 |
//! | `CircularDependency`     | Cycle in impact graph            | Warn user, truncate at cycle point      |
//! | `CrossLanguageGap`       | Ref crosses unsupported language | Return raw string ref with warning      |

use std::fmt;

// ---------------------------------------------------------------------------
// ShardError — domain-specific error enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ShardError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<String>,
}

impl ShardError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// Return the JSON-RPC error code for this error.
    pub fn jsonrpc_code(&self) -> i64 {
        match self.code {
            ErrorCode::StaleIndex => -32001,
            ErrorCode::SymbolNotFound => -32002,
            ErrorCode::ParserError => -32003,
            ErrorCode::TokenBudgetExceeded => -32004,
            ErrorCode::RefIntegrityViolation => -32005,
            ErrorCode::CircularDependency => -32006,
            ErrorCode::CrossLanguageGap => -32007,
            ErrorCode::DatabaseError => -32008,
            ErrorCode::IoError => -32009,
            ErrorCode::ConfigError => -32010,
            ErrorCode::IndexNotInitialized => -32011,
        }
    }

    /// Human-readable suggestion for the agent on what to do next.
    pub fn agent_action(&self) -> &str {
        match self.code {
            ErrorCode::StaleIndex => "Auto-retry 2x, then use filesystem fallback",
            ErrorCode::SymbolNotFound => {
                "Try search() with similar names, then filesystem fallback"
            }
            ErrorCode::ParserError => "Report to user, mark file as corrupted",
            ErrorCode::TokenBudgetExceeded => "Request higher compression level or increase budget",
            ErrorCode::RefIntegrityViolation => "Block edit, show impact analysis to user",
            ErrorCode::CircularDependency => "Warn user, truncate graph at cycle point",
            ErrorCode::CrossLanguageGap => "Return raw string reference with warning",
            ErrorCode::DatabaseError => "Check SQLite WAL mode, retry transaction",
            ErrorCode::IoError => "Check file permissions and disk space",
            ErrorCode::ConfigError => "Validate config file syntax and required fields",
            ErrorCode::IndexNotInitialized => "Run shardindex init -p <project_path> first",
        }
    }
}

impl fmt::Display for ShardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)?;
        if let Some(ref details) = self.details {
            write!(f, " — {}", details)?;
        }
        Ok(())
    }
}

impl std::error::Error for ShardError {}

// ---------------------------------------------------------------------------
// ErrorCode — machine-readable error identifiers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// File hash mismatch — index is stale
    StaleIndex,
    /// Symbol not found in index
    SymbolNotFound,
    /// File could not be parsed
    ParserError,
    /// Symbol body exceeds token budget
    TokenBudgetExceeded,
    /// Reference integrity check failed during edit_plan
    RefIntegrityViolation,
    /// Cycle detected in impact graph
    CircularDependency,
    /// Reference crosses language boundary with no resolver
    CrossLanguageGap,
    /// SQLite / database error
    DatabaseError,
    /// Filesystem I/O error
    IoError,
    /// Configuration parse/validation error
    ConfigError,
    /// Index has not been initialized for this project
    IndexNotInitialized,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Use SCREAMING_SNAKE_CASE for error codes (matches JSON serialization)
        match self {
            ErrorCode::StaleIndex => write!(f, "STALE_INDEX"),
            ErrorCode::SymbolNotFound => write!(f, "SYMBOL_NOT_FOUND"),
            ErrorCode::ParserError => write!(f, "PARSER_ERROR"),
            ErrorCode::TokenBudgetExceeded => write!(f, "TOKEN_BUDGET_EXCEEDED"),
            ErrorCode::RefIntegrityViolation => write!(f, "REF_INTEGRITY_VIOLATION"),
            ErrorCode::CircularDependency => write!(f, "CIRCULAR_DEPENDENCY"),
            ErrorCode::CrossLanguageGap => write!(f, "CROSS_LANGUAGE_GAP"),
            ErrorCode::DatabaseError => write!(f, "DATABASE_ERROR"),
            ErrorCode::IoError => write!(f, "IO_ERROR"),
            ErrorCode::ConfigError => write!(f, "CONFIG_ERROR"),
            ErrorCode::IndexNotInitialized => write!(f, "INDEX_NOT_INITIALIZED"),
        }
    }
}

// ---------------------------------------------------------------------------
// Result alias
// ---------------------------------------------------------------------------

pub type ShardResult<T> = Result<T, ShardError>;

// ---------------------------------------------------------------------------
// Conversion from anyhow::Error (for gradual migration)
// ---------------------------------------------------------------------------

impl From<anyhow::Error> for ShardError {
    fn from(err: anyhow::Error) -> Self {
        let msg = err.to_string();
        // Heuristic: try to classify anyhow errors into domain errors
        // Order matters: more specific patterns first to avoid false positives
        if msg.contains("database") || msg.contains("sqlite") || msg.contains("SQL") {
            ShardError::new(ErrorCode::DatabaseError, msg)
        } else if msg.contains("hash mismatch") || msg.contains("stale") {
            ShardError::new(ErrorCode::StaleIndex, msg)
        } else if msg.contains("parse") || msg.contains("unparseable") {
            ShardError::new(ErrorCode::ParserError, msg)
        } else if msg.contains("budget") || msg.contains("token limit") {
            ShardError::new(ErrorCode::TokenBudgetExceeded, msg)
        } else if msg.contains("no such symbol") || msg.contains("not found") {
            ShardError::new(ErrorCode::SymbolNotFound, msg)
        } else if msg.contains("io") || msg.contains("file") || msg.contains("permission") {
            ShardError::new(ErrorCode::IoError, msg)
        } else {
            ShardError::new(ErrorCode::DatabaseError, msg)
        }
    }
}

// ---------------------------------------------------------------------------
// Conversion from stdio errors
// ---------------------------------------------------------------------------

impl From<std::io::Error> for ShardError {
    fn from(err: std::io::Error) -> Self {
        ShardError::new(ErrorCode::IoError, err.to_string())
    }
}

// ---------------------------------------------------------------------------
// Conversion from rusqlite errors
// ---------------------------------------------------------------------------

impl From<rusqlite::Error> for ShardError {
    fn from(err: rusqlite::Error) -> Self {
        ShardError::new(ErrorCode::DatabaseError, err.to_string())
    }
}

impl From<rusqlite::types::FromSqlError> for ShardError {
    fn from(err: rusqlite::types::FromSqlError) -> Self {
        ShardError::new(ErrorCode::DatabaseError, err.to_string())
    }
}

// ---------------------------------------------------------------------------
// JSON serialization for MCP responses
// ---------------------------------------------------------------------------

impl serde::Serialize for ShardError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("code", &self.code)?;
        map.serialize_entry("message", &self.message)?;
        if let Some(ref details) = self.details {
            map.serialize_entry("details", details)?;
        }
        map.serialize_entry("agent_action", self.agent_action())?;
        map.end()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes_unique() {
        let codes = [
            ErrorCode::StaleIndex,
            ErrorCode::SymbolNotFound,
            ErrorCode::ParserError,
            ErrorCode::TokenBudgetExceeded,
            ErrorCode::RefIntegrityViolation,
            ErrorCode::CircularDependency,
            ErrorCode::CrossLanguageGap,
            ErrorCode::DatabaseError,
            ErrorCode::IoError,
            ErrorCode::ConfigError,
            ErrorCode::IndexNotInitialized,
        ];
        // Verify all JSON-RPC codes are unique
        let rpc_codes: Vec<i64> = codes
            .iter()
            .map(|c| ShardError::new(*c, "test").jsonrpc_code())
            .collect();
        for i in 0..rpc_codes.len() {
            for j in (i + 1)..rpc_codes.len() {
                assert_ne!(rpc_codes[i], rpc_codes[j], "Duplicate JSON-RPC code");
            }
        }
    }

    #[test]
    fn test_error_display() {
        let err = ShardError::new(ErrorCode::SymbolNotFound, "auth.login")
            .with_details("Searched 1,234 symbols, no match");
        let msg = err.to_string();
        assert!(msg.contains("SYMBOL_NOT_FOUND"));
        assert!(msg.contains("auth.login"));
        assert!(msg.contains("Searched 1,234 symbols"));
    }

    #[test]
    fn test_agent_action() {
        let err = ShardError::new(ErrorCode::StaleIndex, "hash mismatch");
        assert!(err.agent_action().contains("retry"));
        assert!(err.agent_action().contains("filesystem"));

        let err = ShardError::new(ErrorCode::TokenBudgetExceeded, "too large");
        assert!(
            err.agent_action().contains("compression") || err.agent_action().contains("budget")
        );
    }

    #[test]
    fn test_error_serialization() {
        let err = ShardError::new(ErrorCode::SymbolNotFound, "missing_func")
            .with_details("checked 3 files");
        let json = serde_json::to_string(&err).expect("serialize");
        let val: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(val["code"], "SYMBOL_NOT_FOUND");
        assert_eq!(val["message"], "missing_func");
        assert_eq!(val["details"], "checked 3 files");
        assert!(val["agent_action"].is_string());
    }

    #[test]
    fn test_error_clone() {
        let err = ShardError::new(ErrorCode::ParserError, "broken file").with_details("line 42");
        let cloned = err.clone();
        assert_eq!(err.code, cloned.code);
        assert_eq!(err.message, cloned.message);
    }

    #[test]
    fn test_anyhow_conversion_symbol_not_found() {
        let anyhow_err = anyhow::anyhow!("no such symbol: foo.bar");
        let shard_err: ShardError = anyhow_err.into();
        assert_eq!(shard_err.code, ErrorCode::SymbolNotFound);
    }

    #[test]
    fn test_anyhow_conversion_stale_index() {
        let anyhow_err = anyhow::anyhow!("hash mismatch: file is stale");
        let shard_err: ShardError = anyhow_err.into();
        assert_eq!(shard_err.code, ErrorCode::StaleIndex);
    }

    #[test]
    fn test_anyhow_conversion_parser_error() {
        let anyhow_err = anyhow::anyhow!("parse error at line 10");
        let shard_err: ShardError = anyhow_err.into();
        assert_eq!(shard_err.code, ErrorCode::ParserError);
    }

    #[test]
    fn test_anyhow_conversion_database() {
        let anyhow_err = anyhow::anyhow!("SQL error: table not found");
        let shard_err: ShardError = anyhow_err.into();
        assert_eq!(shard_err.code, ErrorCode::DatabaseError);
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file gone");
        let shard_err: ShardError = io_err.into();
        assert_eq!(shard_err.code, ErrorCode::IoError);
    }

    #[test]
    fn test_from_sqlite_error_type() {
        // Verify the From<rusqlite::Error> impl compiles (type check only)
        // rusqlite 0.35 API changes make it hard to construct errors in tests,
        // but the impl is verified at compile time.
        fn assert_from_sqlite<T: From<rusqlite::Error>>() {}
        assert_from_sqlite::<ShardError>();
    }

    #[test]
    fn test_jsonrpc_codes_negative() {
        // All JSON-RPC error codes should be negative (per spec)
        for code in [
            ErrorCode::StaleIndex,
            ErrorCode::SymbolNotFound,
            ErrorCode::ParserError,
            ErrorCode::TokenBudgetExceeded,
            ErrorCode::RefIntegrityViolation,
            ErrorCode::CircularDependency,
            ErrorCode::CrossLanguageGap,
            ErrorCode::DatabaseError,
            ErrorCode::IoError,
            ErrorCode::ConfigError,
            ErrorCode::IndexNotInitialized,
        ] {
            let rpc = ShardError::new(code, "test").jsonrpc_code();
            assert!(
                rpc < 0,
                "JSON-RPC code {:?} = {} should be negative",
                code,
                rpc
            );
        }
    }
}
