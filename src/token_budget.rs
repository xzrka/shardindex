/// Token budget enforcement for MCP responses.
///
/// Wraps MCP tool results with token budget awareness:
/// - Tracks budget consumed and remaining
/// - Auto-downgrades response detail when budget exceeded
/// - Strips fields from JSON results to fit within budget
///
/// Aligns with masterplan §10 (Token Budget & Semantic Compression)
/// and NEXT_TODO Phase 4-3.
use serde::{Deserialize, Serialize};

use crate::token_estimation::estimate_token_count;

// ─── Token Budgeted Response Wrapper ───

/// Response wrapper that includes token budget metadata.
///
/// Every MCP tool response can be wrapped in this structure when
/// `token_budget` is specified by the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudgetedResponse {
    /// The actual tool result data.
    pub result: serde_json::Value,
    /// Token budget requested by the client (in tokens).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_requested: Option<usize>,
    /// Estimated tokens consumed by this response.
    pub tokens_used: usize,
    /// Remaining token budget (budget_requested - tokens_used).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_remaining: Option<usize>,
    /// Whether the response was truncated/compressed to fit the budget.
    pub truncated: bool,
    /// Compression level actually applied (if auto-downgraded).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_applied: Option<String>,
}

impl TokenBudgetedResponse {
    /// Create a new budgeted response. Returns `None` if no budget was specified.
    pub fn new(result: serde_json::Value, budget_requested: Option<usize>) -> Option<Self> {
        let budget_requested = match budget_requested {
            Some(b) if b > 0 => b,
            _ => return None,
        };

        // Serialize result to estimate its token count
        let result_json = serde_json::to_string(&result).unwrap_or_default();
        let tokens_used = estimate_token_count(&result_json);
        let truncated = tokens_used > budget_requested;
        let budget_remaining = if truncated {
            Some(0)
        } else {
            Some(budget_requested.saturating_sub(tokens_used))
        };

        Some(Self {
            result,
            budget_requested: Some(budget_requested),
            tokens_used,
            budget_remaining,
            truncated,
            compression_applied: None,
        })
    }

    /// Mark the response as truncated with a specific compression level applied.
    pub fn with_compression(mut self, level: &str) -> Self {
        self.truncated = true;
        self.compression_applied = Some(level.to_string());
        self.budget_remaining = Some(0);
        self
    }
}

// ─── Budget Enforcement ───

/// Strategy for reducing JSON response size when budget is exceeded.
/// Applied in order of priority (most aggressive first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStrategy {
    /// Strip docstrings from search results
    StripDocstrings,
    /// Strip signatures from search results
    StripSignatures,
    /// Limit result count
    TruncateResults,
    /// Remove nested detail fields
    RemoveDetails,
}

impl BudgetStrategy {
    /// Apply budget reduction strategy to a JSON value.
    /// Returns the reduced JSON and estimated tokens saved.
    pub fn apply(&self, json: &serde_json::Value) -> serde_json::Value {
        match self {
            BudgetStrategy::StripDocstrings => strip_docstrings(json),
            BudgetStrategy::StripSignatures => strip_signatures(json),
            BudgetStrategy::TruncateResults => truncate_results(json, 10),
            BudgetStrategy::RemoveDetails => remove_details(json),
        }
    }

    /// All strategies in order of application (least to most aggressive).
    pub fn all_strategies() -> &'static [BudgetStrategy] {
        &[
            BudgetStrategy::StripDocstrings,
            BudgetStrategy::StripSignatures,
            BudgetStrategy::RemoveDetails,
            BudgetStrategy::TruncateResults,
        ]
    }
}

/// Strip `docstring` fields from all objects in the JSON.
fn strip_docstrings(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                if k == "docstring" {
                    continue;
                }
                new_map.insert(k.clone(), strip_docstrings(v));
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(strip_docstrings).collect())
        }
        other => other.clone(),
    }
}

/// Strip `signature` fields from all objects in the JSON.
fn strip_signatures(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                if k == "signature" {
                    continue;
                }
                new_map.insert(k.clone(), strip_signatures(v));
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(strip_signatures).collect())
        }
        other => other.clone(),
    }
}

/// Truncate array results to a maximum count.
fn truncate_results(value: &serde_json::Value, max_count: usize) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            let mut count_updated = false;

            for (k, v) in map {
                // Skip 'count' key — it's handled alongside 'results' below
                if k == "count" {
                    continue;
                }

                let reduced = truncate_results(v, max_count);
                if k == "results"
                    || k == "symbols"
                    || k == "neighbors"
                    || k == "impacted_symbols"
                    || k == "references"
                {
                    if let serde_json::Value::Array(arr) = &reduced {
                        if arr.len() > max_count {
                            new_map.insert(
                                k.clone(),
                                serde_json::Value::Array(
                                    arr[..max_count].iter().cloned().collect(),
                                ),
                            );
                            // Update count field if present
                            if let Some(count_val) = map.get("count") {
                                if count_val.as_u64().map_or(false, |c| c > max_count as u64) {
                                    new_map
                                        .insert("count".to_string(), serde_json::json!(max_count));
                                    count_updated = true;
                                }
                            }
                            continue;
                        }
                    }
                }
                new_map.insert(k.clone(), reduced);
            }

            // If 'count' existed but wasn't updated (no truncation happened),
            // preserve the original count value
            if !count_updated {
                if let Some(count_val) = map.get("count") {
                    new_map.insert("count".to_string(), count_val.clone());
                }
            }

            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| truncate_results(v, max_count)).collect())
        }
        other => other.clone(),
    }
}

/// Remove nested detail fields (full_body, context, path, etc.) from JSON.
fn remove_details(value: &serde_json::Value) -> serde_json::Value {
    let detail_fields = [
        "full_body",
        "context",
        "path",
        "error_log",
        "notes",
        "details",
    ];

    match value {
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                if detail_fields.contains(&k.as_str()) {
                    continue;
                }
                new_map.insert(k.clone(), remove_details(v));
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(remove_details).collect())
        }
        other => other.clone(),
    }
}

/// Enforce token budget on a JSON response.
///
/// Applies strategies in order until the response fits within the budget.
/// Returns the (possibly reduced) JSON, whether it was truncated, and the
/// compression strategy that was applied.
pub fn enforce_budget(
    json: &serde_json::Value,
    budget: usize,
) -> (serde_json::Value, bool, Option<String>) {
    let initial_tokens = estimate_json_tokens(json);
    if initial_tokens <= budget {
        return (json.clone(), false, None);
    }

    let mut current = json.clone();
    let mut was_truncated = false;
    let mut last_strategy = String::new();

    for strategy in BudgetStrategy::all_strategies() {
        current = strategy.apply(&current);
        last_strategy = format!("{:?}", strategy);
        was_truncated = true;

        let current_tokens = estimate_json_tokens(&current);
        if current_tokens <= budget {
            break;
        }
    }

    (current, was_truncated, Some(last_strategy))
}

/// Estimate the token count of a serialized JSON value.
pub fn estimate_json_tokens(value: &serde_json::Value) -> usize {
    let json_str = serde_json::to_string(value).unwrap_or_default();
    estimate_token_count(&json_str)
}

// ─── Unit Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budgeted_response_no_budget() {
        let result = serde_json::json!({ "key": "value" });
        let response = TokenBudgetedResponse::new(result.clone(), None);
        assert!(response.is_none());
    }

    #[test]
    fn test_budgeted_response_within_budget() {
        let result = serde_json::json!({ "key": "value" });
        let response = TokenBudgetedResponse::new(result, Some(1000));
        assert!(response.is_some());
        let resp = response.unwrap();
        assert!(!resp.truncated);
        assert!(resp.budget_remaining.is_some());
        assert!(resp.tokens_used > 0);
    }

    #[test]
    fn test_budgeted_response_exceeded_budget() {
        // Create a large JSON response
        let result = serde_json::json!({
            "results": [
                { "name": "very_long_symbol_name_that_takes_tokens", "docstring": "This is a very long docstring that describes the symbol in great detail and takes up a lot of tokens in the response", "signature": "fn very_long_function_name(param1: String, param2: i32) -> Result<(), Error>" },
                { "name": "another_symbol", "docstring": "Another long docstring with lots of text", "signature": "fn another_fn() -> bool" },
            ]
        });
        let response = TokenBudgetedResponse::new(result, Some(2));
        assert!(response.is_some());
        let resp = response.unwrap();
        assert!(resp.truncated);
    }

    #[test]
    fn test_budgeted_response_with_compression() {
        let result = serde_json::json!({ "key": "value" });
        let response = TokenBudgetedResponse::new(result, Some(100))
            .unwrap()
            .with_compression("signature_only");
        assert!(response.truncated);
        assert_eq!(
            response.compression_applied,
            Some("signature_only".to_string())
        );
    }

    #[test]
    fn test_strip_docstrings() {
        let json = serde_json::json!({
            "name": "my_func",
            "docstring": "This is a docstring",
            "kind": "function"
        });
        let stripped = strip_docstrings(&json);
        assert_eq!(
            stripped.get("name").and_then(|v| v.as_str()),
            Some("my_func")
        );
        assert!(stripped.get("docstring").is_none());
        assert_eq!(
            stripped.get("kind").and_then(|v| v.as_str()),
            Some("function")
        );
    }

    #[test]
    fn test_strip_docstrings_in_array() {
        let json = serde_json::json!([
            { "name": "a", "docstring": "doc a" },
            { "name": "b", "docstring": "doc b" }
        ]);
        let stripped = strip_docstrings(&json);
        if let serde_json::Value::Array(arr) = stripped {
            assert_eq!(arr.len(), 2);
            for item in arr {
                assert!(item.get("docstring").is_none());
                assert!(item.get("name").is_some());
            }
        } else {
            panic!("Expected array");
        }
    }

    #[test]
    fn test_strip_signatures() {
        let json = serde_json::json!({
            "name": "my_func",
            "signature": "fn my_func() -> bool",
            "kind": "function"
        });
        let stripped = strip_signatures(&json);
        assert!(stripped.get("signature").is_none());
        assert_eq!(
            stripped.get("name").and_then(|v| v.as_str()),
            Some("my_func")
        );
    }

    #[test]
    fn test_truncate_results() {
        let items: Vec<serde_json::Value> =
            (0..20).map(|i| serde_json::json!({ "id": i })).collect();
        let json = serde_json::json!({
            "results": items,
            "count": 20
        });
        let truncated = truncate_results(&json, 5);
        if let serde_json::Value::Object(map) = truncated {
            if let serde_json::Value::Array(arr) = map.get("results").unwrap() {
                assert_eq!(arr.len(), 5);
            } else {
                panic!("Expected array for results");
            }
            assert_eq!(map.get("count").and_then(|v| v.as_u64()), Some(5));
        } else {
            panic!("Expected object");
        }
    }

    #[test]
    fn test_remove_details() {
        let json = serde_json::json!({
            "name": "my_func",
            "full_body": "fn my_func() { /* lots of code */ }",
            "context": "surrounding code",
            "kind": "function"
        });
        let cleaned = remove_details(&json);
        assert!(cleaned.get("full_body").is_none());
        assert!(cleaned.get("context").is_none());
        assert_eq!(
            cleaned.get("name").and_then(|v| v.as_str()),
            Some("my_func")
        );
    }

    #[test]
    fn test_enforce_budget_within() {
        let json = serde_json::json!({ "key": "value" });
        let (result, truncated, strategy) = enforce_budget(&json, 1000);
        assert!(!truncated);
        assert!(strategy.is_none());
        assert_eq!(result, json);
    }

    #[test]
    fn test_enforce_budget_reduces() {
        let json = serde_json::json!({
            "results": [
                {
                    "name": "symbol_one",
                    "docstring": "A very long docstring that takes up tokens",
                    "signature": "fn symbol_one(param: String) -> Result<(), Error>",
                    "full_body": "fn symbol_one(param: String) -> Result<(), Error> { /* implementation */ }",
                    "kind": "function"
                }
            ]
        });
        let (result, truncated, strategy) = enforce_budget(&json, 2);
        assert!(truncated);
        assert!(strategy.is_some());

        // Verify docstring, signature, and full_body are stripped
        if let serde_json::Value::Object(map) = result {
            if let serde_json::Value::Array(arr) = map.get("results").unwrap() {
                for item in arr {
                    assert!(
                        item.get("docstring").is_none(),
                        "docstring should be stripped"
                    );
                    assert!(
                        item.get("signature").is_none(),
                        "signature should be stripped"
                    );
                    assert!(
                        item.get("full_body").is_none(),
                        "full_body should be stripped"
                    );
                    assert!(item.get("name").is_some(), "name should remain");
                }
            }
        }
    }

    #[test]
    fn test_enforce_budget_truncation() {
        // Create a large response that needs result count truncation
        let items: Vec<serde_json::Value> = (0..100)
            .map(|i| serde_json::json!({ "id": i, "name": format!("symbol_{}", i) }))
            .collect();
        let json = serde_json::json!({
            "results": items,
            "count": 100
        });
        let (result, truncated, _strategy) = enforce_budget(&json, 5);
        assert!(truncated);

        if let serde_json::Value::Object(map) = result {
            if let serde_json::Value::Array(arr) = map.get("results").unwrap() {
                assert!(arr.len() < 100, "Results should be truncated");
            }
        }
    }

    #[test]
    fn test_estimate_json_tokens() {
        let json = serde_json::json!({ "hello": "world" });
        let tokens = estimate_json_tokens(&json);
        assert!(tokens > 0);
        assert!(tokens < 10); // Small JSON should be few tokens
    }

    #[test]
    fn test_budget_remaining_calculation() {
        let result = serde_json::json!({ "key": "value" });
        let response = TokenBudgetedResponse::new(result, Some(100)).unwrap();
        assert!(response.budget_remaining.unwrap() < 100);
        assert_eq!(response.budget_requested.unwrap(), 100);
    }
}
