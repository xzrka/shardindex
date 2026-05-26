//! Smart YAML — LLM-optimized output format
//!
//! Converts structured data (serde_json::Value) to a compact, token-efficient
//! YAML-like format optimized for LLM context windows. Adapted from smart-yaml.

use serde_json::Value;

// =============================================================================
// Public API
// =============================================================================

/// Convert any JSON value to Smart YAML format.
pub fn to_smart_yaml(value: &Value, compact: bool, sort: bool) -> String {
    match value {
        Value::Object(map) => {
            if is_impact_response(map) {
                format_impact_response(map, compact, sort)
            } else if is_symbol_detail(map) {
                format_symbol_detail(map, compact)
            } else if is_neighbors_response(map) {
                format_neighbors_response(map, compact)
            } else {
                format_generic_object(map, 0, compact, sort)
            }
        }
        Value::Array(arr) => {
            if is_symbol_array(arr) {
                format_symbol_table(arr, compact)
            } else {
                format_generic_array(arr, 0, compact)
            }
        }
        other => format_value(other),
    }
}

/// Convert JSON to standard YAML (via serde_yaml).
pub fn to_standard_yaml(value: &Value) -> anyhow::Result<String> {
    Ok(serde_yaml::to_string(value)?)
}

// =============================================================================
// Response type detection
// =============================================================================

/// impact() response detection
fn is_impact_response(map: &serde_json::Map<String, Value>) -> bool {
    map.contains_key("target")
        || map.contains_key("impacted_symbols")
        || map.contains_key("impacted_count")
        || (map.contains_key("result")
            && map
                .get("result")
                .and_then(|v| v.get("target"))
                .is_some())
}

/// Symbol detail detection
fn is_symbol_detail(map: &serde_json::Map<String, Value>) -> bool {
    map.contains_key("qualified_name")
        && (map.contains_key("signature") || map.contains_key("kind"))
}

/// neighbors response detection
fn is_neighbors_response(map: &serde_json::Map<String, Value>) -> bool {
    map.contains_key("center") && map.contains_key("callers") && map.contains_key("callees")
}

/// Symbol array detection
fn is_symbol_array(arr: &[Value]) -> bool {
    arr.first()
        .and_then(|v| v.as_object())
        .map(|m| m.contains_key("qualified_name") || m.contains_key("name"))
        .unwrap_or(false)
}

// =============================================================================
// Response-specific formatters
// =============================================================================

/// impact() response formatting
fn format_impact_response(
    map: &serde_json::Map<String, Value>,
    compact: bool,
    _sort: bool,
) -> String {
    let mut lines = vec![];

    let target = extract_target(map);
    lines.push(format!("▶ {}", target));

    if !compact {
        lines.push(String::new());
    }

    let symbols = extract_impacted_symbols(map);
    if !symbols.is_empty() {
        if !compact {
            lines.push("dependencies:".to_string());
        }

        for sym in &symbols {
            let line = format_symbol_line(sym, compact);
            lines.push(line);
        }
    }

    let count = symbols.len();
    if !compact {
        lines.push(String::new());
        lines.push(format!("total: {} symbols", count));
    }

    lines.join("\n")
}

/// neighbors() response formatting
fn format_neighbors_response(
    map: &serde_json::Map<String, Value>,
    compact: bool,
) -> String {
    let mut lines = vec![];

    let center = map
        .get("center")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    lines.push(format!("▶ {}", center));

    if !compact {
        lines.push(String::new());
    }

    // callers
    if let Some(callers) = map.get("callers").and_then(|v| v.as_array()) {
        if !callers.is_empty() && !compact {
            lines.push("callers:".to_string());
        }
        for caller in callers {
            if let Some(obj) = caller.as_object() {
                let name = obj
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .or_else(|| obj.get("qualified_name").and_then(|v| v.as_str()))
                    .unwrap_or("?");
                let file = obj.get("file").and_then(|v| v.as_str()).unwrap_or("?");
                let line = obj.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
                let conf = obj.get("confidence").and_then(|v| v.as_f64()).unwrap_or(1.0);

                if compact {
                    lines.push(format!("  ← {} @ {}:{}", name, file, line));
                } else {
                    let conf_str = if conf >= 0.9 {
                        String::new()
                    } else {
                        format!("[{:.0}]", conf * 100.0)
                    };
                    lines.push(format!(
                        "  ← {}{} @ {}:{}",
                        name, conf_str, file, line
                    ));
                }
            }
        }
    }

    // callees
    if let Some(callees) = map.get("callees").and_then(|v| v.as_array()) {
        if !callees.is_empty() && !compact {
            lines.push("callees:".to_string());
        }
        for callee in callees {
            if let Some(obj) = callee.as_object() {
                let name = obj
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .or_else(|| obj.get("qualified_name").and_then(|v| v.as_str()))
                    .unwrap_or("?");
                let file = obj.get("file").and_then(|v| v.as_str()).unwrap_or("?");
                let line = obj.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
                let conf = obj.get("confidence").and_then(|v| v.as_f64()).unwrap_or(1.0);

                if compact {
                    lines.push(format!("  → {} @ {}:{}", name, file, line));
                } else {
                    let conf_str = if conf >= 0.9 {
                        String::new()
                    } else {
                        format!("[{:.0}]", conf * 100.0)
                    };
                    lines.push(format!(
                        "  → {}{} @ {}:{}",
                        name, conf_str, file, line
                    ));
                }
            }
        }
    }

    lines.join("\n")
}

/// Symbol detail formatting
fn format_symbol_detail(map: &serde_json::Map<String, Value>, compact: bool) -> String {
    let mut lines = vec![];

    let name = map
        .get("qualified_name")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let file = map.get("file").and_then(|v| v.as_str()).unwrap_or("?");
    let line = map
        .get("line_start")
        .or_else(|| map.get("line"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    lines.push(format!("▶ {} @ {}:{}", name, file, line));

    if let Some(sig) = map.get("signature").and_then(|v| v.as_str()) {
        if !sig.is_empty() && !compact {
            lines.push(format!("  sig: {}", sig));
        }
    }

    if let Some(doc) = map.get("docstring").and_then(|v| v.as_str()) {
        if !doc.is_empty() && !compact {
            let first_line = doc.lines().next().unwrap_or("");
            lines.push(format!("  doc: {}", first_line));
        }
    }

    if let Some(refs) = map.get("refs").and_then(|v| v.as_object()) {
        if let Some(calls) = refs.get("calls").and_then(|v| v.as_array()) {
            if !calls.is_empty() {
                if !compact {
                    lines.push("  calls:".to_string());
                }
                for call in calls {
                    if let Some(s) = call.as_str() {
                        lines.push(format!("    → {}", s));
                    }
                }
            }
        }
        if let Some(called_by) = refs.get("called_by").and_then(|v| v.as_array()) {
            if !called_by.is_empty() {
                if !compact {
                    lines.push("  called_by:".to_string());
                }
                for caller in called_by {
                    if let Some(s) = caller.as_str() {
                        lines.push(format!("    ← {}", s));
                    }
                }
            }
        }
    }

    lines.join("\n")
}

/// Symbol table formatting (array)
fn format_symbol_table(arr: &[Value], compact: bool) -> String {
    let mut lines = vec![];

    if compact {
        for item in arr {
            if let Some(obj) = item.as_object() {
                let name = obj
                    .get("qualified_name")
                    .or_else(|| obj.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let file = obj.get("file").and_then(|v| v.as_str()).unwrap_or("?");
                let line = obj.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
                lines.push(format!("{} @ {}:{}", name, file, line));
            }
        }
    } else {
        lines.push("name                    file        kind      line".to_string());
        lines.push("─".repeat(55));

        for item in arr {
            if let Some(obj) = item.as_object() {
                let name = obj
                    .get("qualified_name")
                    .or_else(|| obj.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let file = obj.get("file").and_then(|v| v.as_str()).unwrap_or("?");
                let kind = obj.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                let line = obj.get("line").and_then(|v| v.as_u64()).unwrap_or(0);

                lines.push(format!(
                    "{:<24} {:<12} {:<10} {}",
                    name, file, kind, line
                ));
            }
        }
    }

    lines.join("\n")
}

// =============================================================================
// Generic formatters
// =============================================================================

fn format_generic_object(
    map: &serde_json::Map<String, Value>,
    indent: usize,
    compact: bool,
    sort: bool,
) -> String {
    let spaces = "  ".repeat(indent);
    let mut lines = vec![];

    let mut entries: Vec<_> = map.iter().collect();
    if sort {
        entries.sort_by(|a, b| a.0.cmp(b.0));
    }

    for (key, val) in entries {
        if should_skip_key(key, indent) {
            continue;
        }
        if val.is_null() {
            continue;
        }

        let smart_key = optimize_key(key);

        match val {
            Value::Object(nested) => {
                if !compact || indent == 0 {
                    lines.push(format!("{}{}:", spaces, smart_key));
                    lines.push(format_generic_object(nested, indent + 1, compact, sort));
                }
            }
            Value::Array(arr) => {
                if is_simple_array(arr) {
                    lines.push(format!("{}{}:", spaces, smart_key));
                    for item in arr {
                        lines.push(format!("{}  - {}", spaces, format_value(item)));
                    }
                } else if !compact {
                    lines.push(format!("{}{}:", spaces, smart_key));
                    for (i, item) in arr.iter().enumerate() {
                        lines.push(format!("{}  [{}]:", spaces, i));
                        if let Some(obj) = item.as_object() {
                            lines.push(format_generic_object(
                                obj,
                                indent + 2,
                                compact,
                                sort,
                            ));
                        }
                    }
                }
            }
            _ => {
                lines.push(format!(
                    "{}{}: {}",
                    spaces,
                    smart_key,
                    format_value(val)
                ));
            }
        }
    }

    lines.join("\n")
}

fn format_generic_array(arr: &[Value], indent: usize, _compact: bool) -> String {
    let spaces = "  ".repeat(indent);
    arr.iter()
        .map(|v| format!("{}  - {}", spaces, format_value(v)))
        .collect::<Vec<_>>()
        .join("\n")
}

// =============================================================================
// Utility functions
// =============================================================================

fn extract_target(map: &serde_json::Map<String, Value>) -> String {
    map.get("target")
        .and_then(|v| v.as_str())
        .or_else(|| {
            map.get("result")
                .and_then(|v| v.get("target"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| map.get("center").and_then(|v| v.as_str()))
        .unwrap_or("?")
        .to_string()
}

fn extract_impacted_symbols(
    map: &serde_json::Map<String, Value>,
) -> Vec<&serde_json::Map<String, Value>> {
    let mut symbols = vec![];

    if let Some(arr) = map.get("impacted_symbols").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(obj) = item.as_object() {
                symbols.push(obj);
            }
        }
    }

    if symbols.is_empty() {
        if let Some(arr) = map
            .get("result")
            .and_then(|v| v.get("impacted_symbols"))
            .and_then(|v| v.as_array())
        {
            for item in arr {
                if let Some(obj) = item.as_object() {
                    symbols.push(obj);
                }
            }
        }
    }

    if symbols.is_empty() {
        if let Some(layers) = map
            .get("result")
            .and_then(|v| v.get("layers"))
            .and_then(|v| v.as_array())
        {
            for layer in layers {
                if let Some(arr) = layer.get("symbols").and_then(|v| v.as_array()) {
                    for item in arr {
                        if let Some(obj) = item.as_object() {
                            symbols.push(obj);
                        }
                    }
                }
            }
        }
    }

    symbols
}

fn format_symbol_line(sym: &serde_json::Map<String, Value>, compact: bool) -> String {
    let name = sym
        .get("qualified_name")
        .or_else(|| sym.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let file = sym.get("file").and_then(|v| v.as_str()).unwrap_or("?");
    let line = sym.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
    let rel = sym.get("relationship").and_then(|v| v.as_str()).unwrap_or("");
    let conf = sym.get("confidence").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let kind = sym.get("kind").and_then(|v| v.as_str()).unwrap_or("");

    let arrow = match rel {
        "caller" => "←",
        "callee" => "→",
        _ => "•",
    };

    let conf_str = if conf >= 0.9 {
        String::new()
    } else {
        format!("[{:.0}]", conf * 100.0)
    };

    if compact {
        format!("  {} {} @ {}:{}{}", arrow, name, file, line, conf_str)
    } else {
        format!(
            "  {} {} ({}) @ {}:{}{}",
            arrow, name, kind, file, line, conf_str
        )
    }
}

fn should_skip_key(key: &str, indent: usize) -> bool {
    let skip_root = ["jsonrpc", "id"];

    if indent == 0 && skip_root.contains(&key) {
        return true;
    }
    false
}

fn optimize_key(key: &str) -> &str {
    match key {
        "qualified_name" => "name",
        "impacted_symbols" => "symbols",
        "total_estimated_tokens" => "tokens",
        "relationship" => "rel",
        "confidence" => "conf",
        other => other,
    }
}

fn format_value(val: &Value) -> String {
    match val {
        Value::String(s) => {
            let needs_quote = s.is_empty()
                || s.contains(':')
                || s.contains('#')
                || s.starts_with(' ')
                || s.starts_with('-')
                || s.contains('\n')
                || s.parse::<f64>().is_ok()
                || ["true", "false", "null", "yes", "no", "on", "off"]
                    .contains(&s.as_str());

            if needs_quote {
                format!("'{}'", s.replace('\'', "''"))
            } else {
                s.to_string()
            }
        }
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "".to_string(),
        _ => String::new(),
    }
}

fn is_simple_array(arr: &[Value]) -> bool {
    arr.iter().all(|v| !v.is_object() && !v.is_array())
}
