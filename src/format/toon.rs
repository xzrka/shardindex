//! TOON format encoder — LLM-optimized output via toon-format crate.
//!
//! Token-Oriented Object Notation (TOON) is a compact, human-readable format
//! designed for passing structured data to LLMs with significantly reduced
//! token usage. Arrays of uniform objects become tabular rows with declared
//! length and field list, eliminating repeated keys.

use serde_json::Value;

/// Encode a JSON value to TOON format.
///
/// The `compact` and `sort` parameters are kept for API compatibility but
/// TOON always produces deterministic output regardless.
pub fn to_toon(value: &Value, _compact: bool, _sort: bool) -> String {
    toon_format::encode_default(value).unwrap_or_else(|e| {
        format!(
            "# TOON encode error: {}\n{}",
            e,
            serde_json::to_string(value).unwrap_or_default()
        )
    })
}
