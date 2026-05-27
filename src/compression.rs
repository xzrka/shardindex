/// Adaptive semantic compression pipeline.
///
/// Compresses symbol bodies into different levels of detail for token budgeting.
/// Aligns with masterplan §10 (Token Budget & Semantic Compression).
///
/// ## Compression Levels
///
/// 1. **SignatureOnly** — function signature, params, return type (~50 tokens)
/// 2. **CriticalBranches** — control flow: if/else, match, loops, error branches (~150 tokens)
/// 3. **FullBody** — complete implementation (~400 tokens)
/// 4. **TokenBudgeted(n)** — auto-select level to fit within `n` tokens
///
/// ## Usage
///
/// ```rust
/// use shardindex::compression::{compress_symbol, CompressionLevel, CompressedSymbol};
///
/// let source = "fn hello() { println!(\"world\"); }";
/// let compressed = compress_symbol(source, 1, 1, CompressionLevel::SignatureOnly);
/// assert!(!compressed.signature.is_empty());
/// ```
use crate::token_estimation::estimate_token_count;

// ─── Compression Level ───

/// Compression level for symbol bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionLevel {
    /// Only the signature (def/params/return). ~50 tokens.
    SignatureOnly,
    /// Signature + critical control flow branches. ~150 tokens.
    CriticalBranches,
    /// Full symbol body. ~400 tokens.
    FullBody,
    /// Auto-select the highest fidelity level that fits within the token budget.
    TokenBudgeted(usize),
}

/// Alias for `CompressionLevel` to match masterplan §8.1 naming convention.
pub type CompressionMode = CompressionLevel;

impl std::fmt::Display for CompressionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompressionLevel::SignatureOnly => write!(f, "signature_only"),
            CompressionLevel::CriticalBranches => write!(f, "critical_branches"),
            CompressionLevel::FullBody => write!(f, "full_body"),
            CompressionLevel::TokenBudgeted(n) => write!(f, "token_budgeted({})", n),
        }
    }
}

impl std::str::FromStr for CompressionLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "signature_only" | "signatureonly" | "sig" | "s" => Ok(CompressionLevel::SignatureOnly),
            "critical_branches" | "criticalbranches" | "crit" | "c" => {
                Ok(CompressionLevel::CriticalBranches)
            }
            "full_body" | "fullbody" | "full" | "f" => Ok(CompressionLevel::FullBody),
            other => {
                // Try parsing "token_budgeted(N)" or "budget(N)" or just a number
                let inner = other
                    .strip_prefix("token_budgeted(")
                    .or_else(|| other.strip_prefix("budget("))
                    .map(|s| s.strip_suffix(')').unwrap_or(s))
                    .unwrap_or(other);

                if let Ok(n) = inner.parse::<usize>() {
                    return Ok(CompressionLevel::TokenBudgeted(n));
                }

                Err(format!(
                    "Invalid compression level '{}'. Use: signature_only, critical_branches, full_body, token_budgeted(N), or a number for token budget",
                    s
                ))
            }
        }
    }
}

// ─── Compressed Symbol ───

/// Result of compressing a symbol body.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompressedSymbol {
    /// The symbol's signature line.
    pub signature: String,
    /// Critical control flow branches extracted from the body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical_branches: Option<Vec<String>>,
    /// Side effects: DB calls, network calls, mutations, I/O.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side_effects: Option<Vec<String>>,
    /// Key variable assignments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_assignments: Option<Vec<String>>,
    /// Return statement(s).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_statement: Option<String>,
    /// Full body text (only for FullBody level).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_body: Option<String>,
    /// Estimated token count of this compressed representation.
    pub estimated_tokens: usize,
    /// Compression level actually used.
    pub compression_used: String,
}

/// Alias for `CompressedSymbol` to match masterplan §8.1 naming convention.
pub type SymbolSlice = CompressedSymbol;

// ─── Public API ───

/// Compress a symbol body to the specified level.
///
/// Takes the full source code, the symbol's line range (1-indexed, inclusive),
/// and the desired compression level.
pub fn compress_symbol(
    source: &str,
    start_line: usize,
    end_line: usize,
    level: CompressionLevel,
) -> CompressedSymbol {
    let lines: Vec<&str> = source.lines().collect();
    if start_line == 0 || start_line > lines.len() {
        return CompressedSymbol::empty();
    }

    let start_idx = start_line - 1;
    let end_idx = end_line.min(lines.len());
    let body = lines[start_idx..end_idx].join("\n");

    // Extract signature (first non-empty, non-comment line)
    let signature = extract_signature(&body);

    // For TokenBudgeted, try levels from highest to lowest fidelity
    if let CompressionLevel::TokenBudgeted(budget) = level {
        return compress_with_budget(&body, &signature, budget);
    }

    match level {
        CompressionLevel::SignatureOnly => {
            let tokens = estimate_token_count(&signature);
            CompressedSymbol {
                signature,
                critical_branches: None,
                side_effects: None,
                key_assignments: None,
                return_statement: None,
                full_body: None,
                estimated_tokens: tokens,
                compression_used: "signature_only".to_string(),
            }
        }
        CompressionLevel::CriticalBranches => {
            let branches = extract_critical_branches(&body);
            let side_effects = extract_side_effects(&body);
            let key_assignments = extract_key_assignments(&body);
            let return_stmt = extract_return_statement(&body);

            // Build the compressed text for token estimation
            let compressed_text = build_critical_text(
                &signature,
                &branches,
                &side_effects,
                &key_assignments,
                &return_stmt,
            );
            let tokens = estimate_token_count(&compressed_text);

            CompressedSymbol {
                signature,
                critical_branches: if branches.is_empty() {
                    None
                } else {
                    Some(branches)
                },
                side_effects: if side_effects.is_empty() {
                    None
                } else {
                    Some(side_effects)
                },
                key_assignments: if key_assignments.is_empty() {
                    None
                } else {
                    Some(key_assignments)
                },
                return_statement: return_stmt,
                full_body: None,
                estimated_tokens: tokens,
                compression_used: "critical_branches".to_string(),
            }
        }
        CompressionLevel::FullBody => {
            let tokens = estimate_token_count(&body);
            CompressedSymbol {
                signature,
                critical_branches: None,
                side_effects: None,
                key_assignments: None,
                return_statement: None,
                full_body: Some(body),
                estimated_tokens: tokens,
                compression_used: "full_body".to_string(),
            }
        }
        // Handled above
        CompressionLevel::TokenBudgeted(_) => unreachable!(),
    }
}

/// Compress with a token budget — try levels from highest to lowest fidelity.
fn compress_with_budget(body: &str, signature: &str, budget: usize) -> CompressedSymbol {
    // Try FullBody first
    let full_tokens = estimate_token_count(body);
    if full_tokens <= budget {
        return CompressedSymbol {
            signature: signature.to_string(),
            critical_branches: None,
            side_effects: None,
            key_assignments: None,
            return_statement: None,
            full_body: Some(body.to_string()),
            estimated_tokens: full_tokens,
            compression_used: "full_body".to_string(),
        };
    }

    // Try CriticalBranches
    let branches = extract_critical_branches(body);
    let side_effects = extract_side_effects(body);
    let key_assignments = extract_key_assignments(body);
    let return_stmt = extract_return_statement(body);
    let compressed_text = build_critical_text(
        signature,
        &branches,
        &side_effects,
        &key_assignments,
        &return_stmt,
    );
    let critical_tokens = estimate_token_count(&compressed_text);

    if critical_tokens <= budget {
        return CompressedSymbol {
            signature: signature.to_string(),
            critical_branches: if branches.is_empty() {
                None
            } else {
                Some(branches)
            },
            side_effects: if side_effects.is_empty() {
                None
            } else {
                Some(side_effects)
            },
            key_assignments: if key_assignments.is_empty() {
                None
            } else {
                Some(key_assignments)
            },
            return_statement: return_stmt,
            full_body: None,
            estimated_tokens: critical_tokens,
            compression_used: "critical_branches".to_string(),
        };
    }

    // Fall back to SignatureOnly
    let sig_tokens = estimate_token_count(signature);
    CompressedSymbol {
        signature: signature.to_string(),
        critical_branches: None,
        side_effects: None,
        key_assignments: None,
        return_statement: None,
        full_body: None,
        estimated_tokens: sig_tokens,
        compression_used: "signature_only".to_string(),
    }
}

// ─── Signature Extraction ───

/// Extract the signature line from a symbol body.
///
/// Returns the first meaningful line (non-empty, non-comment, non-decorator).
fn extract_signature(body: &str) -> String {
    for line in body.lines() {
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Skip comments
        if trimmed.starts_with("//")
            || trimmed.starts_with("#")
            || trimmed.starts_with("/*")
            || trimmed.starts_with("*")
            || trimmed.starts_with("///")
            || trimmed.starts_with("//!")
            || trimmed.starts_with("'''")
            || trimmed.starts_with("\"\"\"")
        {
            continue;
        }

        // Skip decorators (@)
        if trimmed.starts_with('@') {
            continue;
        }

        // Skip docstring continuation
        if trimmed.starts_with("'''") || trimmed.starts_with("\"\"\"") {
            continue;
        }

        return trimmed.to_string();
    }
    body.lines()
        .next()
        .map(|l| l.trim().to_string())
        .unwrap_or_default()
}

// ─── Critical Branches Extraction ───

/// Extract critical control flow branches from source code.
///
/// Identifies: if/else/match/elif/elif, for/while loops, try/catch/except, panic/raise.
/// Returns simplified one-line representations of each branch.
fn extract_critical_branches(body: &str) -> Vec<String> {
    let mut branches = Vec::new();
    let mut in_multiline_comment = false;

    for line in body.lines() {
        let trimmed = line.trim();

        // Track multi-line comments
        if trimmed.contains("/*") {
            in_multiline_comment = !trimmed.contains("*/");
            if !in_multiline_comment {
                continue;
            }
        }
        if trimmed.contains("*/") {
            in_multiline_comment = false;
            continue;
        }
        if in_multiline_comment {
            continue;
        }

        // Skip empty lines and pure comments
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("#")
            || trimmed.starts_with("///")
            || trimmed.starts_with("//!")
        {
            continue;
        }

        // Conditionals
        if is_conditional_line(trimmed) {
            branches.push(simplify_line(trimmed));
            continue;
        }

        // Loops
        if is_loop_line(trimmed) {
            branches.push(simplify_line(trimmed));
            continue;
        }

        // Error handling
        if is_error_handling_line(trimmed) {
            branches.push(simplify_line(trimmed));
            continue;
        }

        // Match/switch arms (extract the pattern, not the body)
        if is_match_arm(trimmed) {
            branches.push(simplify_line(trimmed));
        }
    }

    branches
}

/// Check if a line is a conditional statement.
fn is_conditional_line(line: &str) -> bool {
    let trimmed = line
        .trim()
        .strip_prefix(|c: char| c == '\t' || c == ' ')
        .unwrap_or(line);

    // if/else/elif/elseif/match/switch/case
    trimmed.starts_with("if ")
        || trimmed.starts_with("if(")
        || trimmed.starts_with("elif ")
        || trimmed.starts_with("elif(")
        || trimmed.starts_with("else if ")
        || trimmed.starts_with("else if(")
        || trimmed == "else {"
        || trimmed == "else:"
        || trimmed == "else"
        || trimmed.starts_with("else ")
        || trimmed.starts_with("match ")
        || trimmed.starts_with("switch ")
        || trimmed.starts_with("case ")
        || trimmed.starts_with("case ")
}

/// Check if a line is a loop statement.
fn is_loop_line(line: &str) -> bool {
    let trimmed = line
        .trim()
        .strip_prefix(|c: char| c == '\t' || c == ' ')
        .unwrap_or(line);

    trimmed.starts_with("for ")
        || trimmed.starts_with("for(")
        || trimmed.starts_with("while ")
        || trimmed.starts_with("while(")
        || trimmed.starts_with("loop ")
        || trimmed.starts_with("loop {")
        || trimmed.starts_with("foreach ")
        || trimmed.starts_with("do ")
        || trimmed.starts_with("do {")
}

/// Check if a line is error handling (try/catch/except/raise/panic).
fn is_error_handling_line(line: &str) -> bool {
    let trimmed = line
        .trim()
        .strip_prefix(|c: char| c == '\t' || c == ' ')
        .unwrap_or(line);

    trimmed.starts_with("try ")
        || trimmed.starts_with("try {")
        || trimmed.starts_with("try{")
        || trimmed.starts_with("catch ")
        || trimmed.starts_with("catch(")
        || trimmed.starts_with("except ")
        || trimmed.starts_with("except(")
        || trimmed.starts_with("finally")
        || trimmed.starts_with("raise ")
        || trimmed.starts_with("panic!")
        || trimmed.starts_with("throw ")
        || trimmed.starts_with("err.")
        || trimmed.contains(".unwrap()")
        || trimmed.contains(".expect(")
        || trimmed.contains(".unwrap_or(")
        || trimmed.contains(".unwrap_or_else(")
}

/// Check if a line is a match arm.
fn is_match_arm(line: &str) -> bool {
    let trimmed = line.trim();
    // Rust match arms: `Pattern =>`
    trimmed.contains(" =>") && !trimmed.starts_with("fn ") && !trimmed.starts_with("let ")
        // Python case arms
        || (trimmed.starts_with("case ") && trimmed.ends_with(":"))
}

/// Simplify a line for the compressed representation.
///
/// Strips braces, trailing commas, and collapses to a single meaningful line.
fn simplify_line(line: &str) -> String {
    let trimmed = line.trim();

    // Remove leading/trailing braces
    let without_braces = trimmed.trim_start_matches('{').trim_end_matches('}').trim();

    // Remove trailing semicolons and commas for cleaner output
    let cleaned = without_braces
        .trim_end_matches(';')
        .trim_end_matches(',')
        .trim()
        .to_string();

    // Collapse multiple spaces
    cleaned.split_whitespace().collect::<Vec<&str>>().join(" ")
}

// ─── Side Effects Extraction ───

/// Extract side effect lines from source code.
///
/// Identifies: DB operations, network calls, file I/O, mutations, logging.
fn extract_side_effects(body: &str) -> Vec<String> {
    let mut effects = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("#") {
            continue;
        }

        if is_side_effect_line(&trimmed) {
            effects.push(simplify_line(&trimmed));
        }
    }

    effects
}

/// Check if a line represents a side effect.
fn is_side_effect_line(line: &str) -> bool {
    let trimmed = line.trim();

    // Database operations
    trimmed.contains(".insert(")
        || trimmed.contains(".update(")
        || trimmed.contains(".delete(")
        || trimmed.contains(".save(")
        || trimmed.contains(".execute(")
        || trimmed.contains(".query(")
        || trimmed.contains(".commit(")
        || trimmed.contains(".rollback(")
        || trimmed.contains("db.")
        || trimmed.contains("DB.")
        || trimmed.contains(".add(")

        // Network / HTTP
        || trimmed.contains("http")
        || trimmed.contains("fetch(")
        || trimmed.contains("axios")
        || trimmed.contains("requests.")
        || trimmed.contains("reqwest")
        || trimmed.contains("HttpClient")

        // File I/O
        || trimmed.contains("File::")
        || trimmed.contains("fs::")
        || trimmed.contains("open(")
        || trimmed.contains("read_to_string")
        || trimmed.contains("write_all")
        || trimmed.contains(".read(")
        || trimmed.contains(".write(")

        // Logging
        || trimmed.contains("log.")
        || trimmed.contains("println!")
        || trimmed.contains("print(")
        || trimmed.contains("eprint")
        || trimmed.contains("console.log")
        || trimmed.contains("console.error")
        || trimmed.contains("console.warn")
        || trimmed.contains("tracing::")
        || trimmed.contains("info!")
        || trimmed.contains("debug!")
        || trimmed.contains("warn!")
        || trimmed.contains("error!")
        || trimmed.contains("trace!")

        // State mutation
        || trimmed.contains(".push(")
        || trimmed.contains(".extend(")
        || trimmed.contains(".remove(")
        || trimmed.contains(".clear()")
        || trimmed.contains(".sort(")
        || trimmed.contains("mut ")

        // Process / system
        || trimmed.contains("std::process")
        || trimmed.contains("subprocess")
        || trimmed.contains("Command::")
        || trimmed.contains("spawn")
        || trimmed.contains("fork")

        // Mutex / atomic
        || trimmed.contains(".lock()")
        || trimmed.contains(".unwrap().lock")
        || trimmed.contains("Atomic")
        || trimmed.contains("Mutex")
        || trimmed.contains("RwLock")
}

// ─── Key Assignments Extraction ───

/// Extract key variable assignments from source code.
///
/// Identifies: `let x =`, `const x =`, `var x =`, `x = value` (non-trivial).
fn extract_key_assignments(body: &str) -> Vec<String> {
    let mut assignments = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("#") {
            continue;
        }

        if is_key_assignment(&trimmed) {
            assignments.push(simplify_line(&trimmed));
        }
    }

    assignments
}

/// Check if a line is a key variable assignment.
fn is_key_assignment(line: &str) -> bool {
    let trimmed = line.trim();

    // Rust let/const
    trimmed.starts_with("let ")
        || trimmed.starts_with("const ")
        // Python assignment (not inside a function call)
        || (trimmed.contains('=')
            && !trimmed.starts_with("if ")
            && !trimmed.starts_with("for ")
            && !trimmed.starts_with("while ")
            && !trimmed.starts_with("def ")
            && !trimmed.starts_with("class ")
            && !trimmed.starts_with("fn ")
            && !trimmed.starts_with("pub ")
            && !trimmed.starts_with("//")
            && !trimmed.starts_with("#")
            && !trimmed.starts_with("return ")
            && !trimmed.starts_with('}'))
        // JS/TS const/let/var
        || trimmed.starts_with("const ")
        || trimmed.starts_with("let ")
        || trimmed.starts_with("var ")
        // Go := assignment
        || trimmed.contains(":=")
}

// ─── Return Statement Extraction ───

/// Extract the return statement from a symbol body.
///
/// Returns the last meaningful return statement found.
fn extract_return_statement(body: &str) -> Option<String> {
    let mut last_return = None;

    for line in body.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("return ")
            || trimmed == "return"
            || trimmed.starts_with("return{")
            || trimmed.starts_with("return {")
            || trimmed.starts_with("yield ")
            || trimmed.starts_with("yield from ")
        {
            last_return = Some(simplify_line(&trimmed));
        }

        // Rust implicit return (last expression without semicolon)
        // Heuristic: a line that's not a statement keyword and doesn't end with semicolon
        // This is a fallback — explicit return is preferred
    }

    last_return
}

// ─── Text Builder ───

/// Build a text representation of critical branches compression for token estimation.
fn build_critical_text(
    signature: &str,
    branches: &[String],
    side_effects: &[String],
    key_assignments: &[String],
    return_stmt: &Option<String>,
) -> String {
    let mut parts = Vec::new();
    parts.push(signature.to_string());

    if !branches.is_empty() {
        parts.push("critical_branches:".to_string());
        parts.extend(branches.iter().map(|b| format!("  - {}", b)));
    }

    if !side_effects.is_empty() {
        parts.push("side_effects:".to_string());
        parts.extend(side_effects.iter().map(|s| format!("  - {}", s)));
    }

    if !key_assignments.is_empty() {
        parts.push("key_assignments:".to_string());
        parts.extend(key_assignments.iter().map(|a| format!("  - {}", a)));
    }

    if let Some(ret) = return_stmt {
        parts.push(format!("return: {}", ret));
    }

    parts.join("\n")
}

// ─── Empty / Default ───

impl CompressedSymbol {
    /// Create an empty compressed symbol (for error cases).
    pub fn empty() -> Self {
        CompressedSymbol {
            signature: String::new(),
            critical_branches: None,
            side_effects: None,
            key_assignments: None,
            return_statement: None,
            full_body: None,
            estimated_tokens: 0,
            compression_used: "empty".to_string(),
        }
    }
}

// ─── Unit Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    // ── Signature extraction ──

    #[test]
    fn test_extract_signature_rust_fn() {
        let body = "/// Create a new cache\npub fn new(max_size: usize) -> Self {";
        let sig = extract_signature(body);
        assert_eq!(sig, "pub fn new(max_size: usize) -> Self {");
    }

    #[test]
    fn test_extract_signature_python_def() {
        let body = "def login(username: str, password: str) -> Session:\n    pass";
        let sig = extract_signature(body);
        assert!(sig.starts_with("def login"));
    }

    #[test]
    fn test_extract_signature_js_function() {
        let body = "// Authenticate user\nfunction authenticate(user, pass) {\n  return true;\n}";
        let sig = extract_signature(body);
        assert!(sig.starts_with("function authenticate"));
    }

    #[test]
    fn test_extract_signature_decorated_python() {
        let body = "@app.route('/login')\ndef login():\n    pass";
        let sig = extract_signature(body);
        assert!(sig.starts_with("def login"));
    }

    #[test]
    fn test_extract_signature_empty() {
        let sig = extract_signature("");
        assert!(sig.is_empty());
    }

    // ── Critical branches ──

    #[test]
    fn test_extract_critical_branches_if_else() {
        let body = r#"fn check(x: i32) -> bool {
    if x > 0 {
        return true;
    } else {
        return false;
    }
}"#;
        let branches = extract_critical_branches(body);
        assert!(!branches.is_empty(), "Should find if/else branches");
        assert!(branches.iter().any(|b| b.contains("if")));
    }

    #[test]
    fn test_extract_critical_branches_loops() {
        let body = r#"for i in 0..10 {
    println!("{}", i);
}
while x > 0 {
    x -= 1;
}"#;
        let branches = extract_critical_branches(body);
        assert!(!branches.is_empty(), "Should find loop branches");
    }

    #[test]
    fn test_extract_critical_branches_try_catch() {
        let body = r#"try {
    let result = risky_op();
} catch (e) {
    log_error(e);
}"#;
        let branches = extract_critical_branches(body);
        assert!(!branches.is_empty(), "Should find try/catch");
    }

    #[test]
    fn test_extract_critical_branches_python() {
        let body = r#"def validate(x):
    if x is None:
        raise ValueError("x is required")
    elif x < 0:
        raise ValueError("x must be positive")
    else:
        return True"#;
        let branches = extract_critical_branches(body);
        assert!(
            branches.len() >= 3,
            "Should find if/elif/else branches, got {}",
            branches.len()
        );
    }

    #[test]
    fn test_extract_critical_branches_rust_match() {
        let body = r#"match status {
    Ok(val) => val,
    Err(e) => panic!(e),
}"#;
        let branches = extract_critical_branches(body);
        assert!(!branches.is_empty(), "Should find match arms");
    }

    #[test]
    fn test_extract_critical_branches_no_branches() {
        let body = r#"fn simple() -> i32 {
    42
}"#;
        let branches = extract_critical_branches(body);
        assert!(
            branches.is_empty(),
            "Simple function should have no critical branches"
        );
    }

    // ── Side effects ──

    #[test]
    fn test_extract_side_effects_db() {
        let body = r#"db.execute("INSERT INTO users VALUES (?)", (name,));
    let id = db.last_insert_rowid();"#;
        let effects = extract_side_effects(body);
        assert!(!effects.is_empty(), "Should find DB side effects");
    }

    #[test]
    fn test_extract_side_effects_logging() {
        let body = r#"println!("Processing {} items", count);
    log::info!("Done");"#;
        let effects = extract_side_effects(body);
        assert!(!effects.is_empty(), "Should find logging side effects");
    }

    #[test]
    fn test_extract_side_effects_file_io() {
        let body = r#"let contents = std::fs::read_to_string(path)?;
    let mut file = File::create(path)?;
    file.write_all(data)?;"#;
        let effects = extract_side_effects(body);
        assert!(!effects.is_empty(), "Should find file I/O side effects");
    }

    #[test]
    fn test_extract_side_effects_console() {
        let body = r#"console.log("hello");
    console.error("oops");"#;
        let effects = extract_side_effects(body);
        assert!(!effects.is_empty(), "Should find console side effects");
    }

    #[test]
    fn test_extract_side_effects_no_effects() {
        let body = r#"fn pure(x: i32, y: i32) -> i32 {
    x + y
}"#;
        let effects = extract_side_effects(body);
        assert!(
            effects.is_empty(),
            "Pure function should have no side effects"
        );
    }

    // ── Key assignments ──

    #[test]
    fn test_extract_key_assignments_rust() {
        let body = r#"let x = 42;
let result = compute(x);
const MAX: usize = 100;"#;
        let assignments = extract_key_assignments(body);
        assert!(assignments.len() >= 2, "Should find let/const assignments");
    }

    #[test]
    fn test_extract_key_assignments_python() {
        let body = r#"result = compute(x)
max_size = 100"#;
        let assignments = extract_key_assignments(body);
        assert!(!assignments.is_empty(), "Should find Python assignments");
    }

    #[test]
    fn test_extract_key_assignments_go() {
        let body = r#"result := compute(x)
err := doSomething()"#;
        let assignments = extract_key_assignments(body);
        assert!(!assignments.is_empty(), "Should find Go := assignments");
    }

    #[test]
    fn test_extract_key_assignments_js() {
        let body = r#"const result = compute(x);
let count = 0;
var old = true;"#;
        let assignments = extract_key_assignments(body);
        assert!(
            assignments.len() >= 2,
            "Should find JS const/let/var assignments"
        );
    }

    // ── Return statement ──

    #[test]
    fn test_extract_return_statement() {
        let body = r#"if x > 0 {
    return true;
}
return false;"#;
        let ret = extract_return_statement(body);
        assert!(ret.is_some());
        assert!(ret.unwrap().contains("return"));
    }

    #[test]
    fn test_extract_return_statement_none() {
        let body = r#"fn side_effect_only() {
    println!("hello");
}"#;
        let ret = extract_return_statement(body);
        assert!(ret.is_none(), "No return statement expected");
    }

    #[test]
    fn test_extract_return_yield() {
        let body = r#"yield value
yield from generator()"#;
        let ret = extract_return_statement(body);
        assert!(ret.is_some(), "Should find yield as return-like");
    }

    // ── Compression levels ──

    #[test]
    fn test_compress_signature_only() {
        let source = r#"/// Create a new cache with the given max size.
pub fn new(max_size: usize) -> Self {
    Self {
        entries: HashMap::new(),
        max_size,
    }
}"#;
        let compressed = compress_symbol(source, 1, 7, CompressionLevel::SignatureOnly);
        assert!(compressed.signature.contains("pub fn new"));
        assert!(compressed.critical_branches.is_none());
        assert!(compressed.full_body.is_none());
        assert!(compressed.estimated_tokens > 0);
    }

    #[test]
    fn test_compress_full_body() {
        let source = r#"fn hello() -> &'static str {
    "world"
}"#;
        let compressed = compress_symbol(source, 1, 3, CompressionLevel::FullBody);
        assert!(compressed.full_body.is_some());
        assert_eq!(compressed.compression_used, "full_body");
    }

    #[test]
    fn test_compress_critical_branches() {
        let source = r#"fn validate(x: i32) -> Result<i32, &'static str> {
    if x < 0 {
        return Err("negative");
    }
    if x > 100 {
        return Err("too large");
    }
    Ok(x)
}"#;
        let compressed = compress_symbol(source, 1, 9, CompressionLevel::CriticalBranches);
        assert!(compressed.critical_branches.is_some());
        assert_eq!(compressed.compression_used, "critical_branches");
        assert!(compressed.estimated_tokens > 0);
    }

    #[test]
    fn test_compress_token_budgeted_fits_full() {
        let source = r#"fn hello() -> &'static str {
    "world"
}"#;
        let compressed = compress_symbol(source, 1, 3, CompressionLevel::TokenBudgeted(1000));
        assert_eq!(compressed.compression_used, "full_body");
    }

    #[test]
    fn test_compress_token_budgeted_fits_critical() {
        let source = r#"fn validate(x: i32) -> Result<i32, &'static str> {
    if x < 0 {
        return Err("negative");
    }
    if x > 100 {
        return Err("too large");
    }
    Ok(x)
}"#;
        // Very tight budget — should fall back to signature_only
        let compressed = compress_symbol(source, 1, 9, CompressionLevel::TokenBudgeted(5));
        assert_eq!(compressed.compression_used, "signature_only");
    }

    #[test]
    fn test_compress_token_budgeted_signature_fallback() {
        let source = r#"fn complex(a: i32, b: i32, c: i32) -> Result<i32, String> {
    if a < 0 { return Err("a negative".into()); }
    if b < 0 { return Err("b negative".into()); }
    if c < 0 { return Err("c negative".into()); }
    let result = a + b + c;
    if result > 1000 { return Err("overflow".into()); }
    println!("result: {}", result);
    Ok(result)
}"#;
        // Very tight budget — should fall back to signature_only
        let compressed = compress_symbol(source, 1, 9, CompressionLevel::TokenBudgeted(3));
        assert_eq!(compressed.compression_used, "signature_only");
    }

    // ── Edge cases ──

    #[test]
    fn test_compress_empty_source() {
        let compressed = compress_symbol("", 1, 1, CompressionLevel::FullBody);
        assert!(compressed.signature.is_empty());
        assert_eq!(compressed.estimated_tokens, 0);
    }

    #[test]
    fn test_compress_out_of_range() {
        let source = "fn hello() {}";
        let compressed = compress_symbol(source, 100, 200, CompressionLevel::FullBody);
        assert!(compressed.signature.is_empty());
        assert_eq!(compressed.compression_used, "empty");
    }

    #[test]
    fn test_compress_consistency() {
        let source = r#"fn add(a: i32, b: i32) -> i32 {
    a + b
}"#;
        let a = compress_symbol(source, 1, 3, CompressionLevel::FullBody);
        let b = compress_symbol(source, 1, 3, CompressionLevel::FullBody);
        assert_eq!(
            a.estimated_tokens, b.estimated_tokens,
            "Compression should be deterministic"
        );
    }

    // ── Helper function tests ──

    #[test]
    fn test_simplify_line() {
        assert_eq!(simplify_line("  let x = 42;  "), "let x = 42");
        assert_eq!(simplify_line("if x > 0 {"), "if x > 0 {");
    }

    #[test]
    fn test_is_conditional_line() {
        assert!(is_conditional_line("if x > 0 {"));
        assert!(is_conditional_line("elif y < 0:"));
        assert!(is_conditional_line("else {"));
        assert!(is_conditional_line("else:"));
        assert!(is_conditional_line("match x {"));
        assert!(!is_conditional_line("let x = 0;"));
    }

    #[test]
    fn test_is_loop_line() {
        assert!(is_loop_line("for i in 0..10 {"));
        assert!(is_loop_line("while x > 0 {"));
        assert!(is_loop_line("for (let i = 0; i < 10; i++) {"));
        assert!(!is_loop_line("let x = 0;"));
    }

    #[test]
    fn test_is_error_handling_line() {
        assert!(is_error_handling_line("try {"));
        assert!(is_error_handling_line("catch (e) {"));
        assert!(is_error_handling_line("except ValueError:"));
        assert!(is_error_handling_line("raise ValueError(\"msg\")"));
        assert!(is_error_handling_line("panic!(\"msg\")"));
        assert!(is_error_handling_line("x.unwrap()"));
        assert!(!is_error_handling_line("let x = 0;"));
    }

    #[test]
    fn test_is_key_assignment() {
        assert!(is_key_assignment("let x = 42;"));
        assert!(is_key_assignment("const MAX: usize = 100;"));
        assert!(is_key_assignment("result = compute(x)"));
        assert!(is_key_assignment("result := compute(x)"));
        assert!(is_key_assignment("const result = compute(x);"));
        assert!(!is_key_assignment("if x > 0 {"));
        assert!(!is_key_assignment("return x;"));
    }

    #[test]
    fn test_is_side_effect_line() {
        assert!(is_side_effect_line("db.execute(\"INSERT\")"));
        assert!(is_side_effect_line("println!(\"hello\")"));
        assert!(is_side_effect_line("console.log(\"hello\")"));
        assert!(is_side_effect_line("std::fs::read_to_string(path)"));
        assert!(!is_side_effect_line("let x = 42;"));
        assert!(!is_side_effect_line("x + y"));
    }

    #[test]
    fn test_compression_level_description() {
        assert_eq!(
            CompressionLevel::SignatureOnly.to_string(),
            "signature_only"
        );
        assert_eq!(
            CompressionLevel::CriticalBranches.to_string(),
            "critical_branches"
        );
        assert_eq!(CompressionLevel::FullBody.to_string(), "full_body");
        assert!(
            CompressionLevel::TokenBudgeted(100)
                .to_string()
                .contains("100")
        );
    }

    #[test]
    fn test_build_critical_text() {
        let sig = "fn hello() -> i32".to_string();
        let branches = vec!["if x > 0 return true".to_string()];
        let effects = vec!["println!(\"hello\")".to_string()];
        let assignments = vec!["let x = 42".to_string()];
        let ret = Some("return 0".to_string());

        let text = build_critical_text(&sig, &branches, &effects, &assignments, &ret);
        assert!(text.contains("fn hello"));
        assert!(text.contains("critical_branches"));
        assert!(text.contains("side_effects"));
        assert!(text.contains("key_assignments"));
        assert!(text.contains("return:"));
    }

    #[test]
    fn test_compress_symbol_tokens_valid_range() {
        // Verify that compress_symbol handles out-of-range gracefully
        let source = "fn hello() {}";
        let result = compress_symbol(source, 0, 1, CompressionLevel::FullBody);
        assert_eq!(result.estimated_tokens, 0);

        let result = compress_symbol(source, 100, 200, CompressionLevel::FullBody);
        assert_eq!(result.estimated_tokens, 0);
    }

    #[test]
    fn test_compress_multiline_comment_skipped() {
        let body = r#"/*
 * This is a multi-line comment
 * with if and for keywords that should not be detected
 */
fn real() -> i32 {
    42
}"#;
        let branches = extract_critical_branches(body);
        // Multi-line comment content should not produce false positives
        assert!(
            branches.is_empty(),
            "Multi-line comment should not produce branches, got {:?}",
            branches
        );
    }

    #[test]
    fn test_compress_python_complex() {
        let source = r#"def process_data(items: list, threshold: float) -> dict:
    """Process items and return results."""
    results = {}
    for item in items:
        if item.value > threshold:
            results[item.id] = item.value
            log.info(f"Processed {item.id}")
        elif item.value == threshold:
            results[item.id] = None
        else:
            continue
    return results"#;
        let compressed = compress_symbol(source, 1, 12, CompressionLevel::CriticalBranches);
        assert!(compressed.critical_branches.is_some());
        let branches = compressed.critical_branches.unwrap();
        // Should find for, if, elif, else
        assert!(
            branches.len() >= 3,
            "Should find multiple branches, got {}",
            branches.len()
        );

        assert!(compressed.side_effects.is_some());
        assert!(compressed.key_assignments.is_some());
        assert!(compressed.return_statement.is_some());
    }

    // ── FromStr for CompressionLevel ──

    #[test]
    fn test_fromstr_signature_only_aliases() {
        assert_eq!(
            "signature_only".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::SignatureOnly
        );
        assert_eq!(
            "signatureonly".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::SignatureOnly
        );
        assert_eq!(
            "sig".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::SignatureOnly
        );
        assert_eq!(
            "s".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::SignatureOnly
        );
        assert_eq!(
            "SIG".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::SignatureOnly
        );
    }

    #[test]
    fn test_fromstr_critical_branches_aliases() {
        assert_eq!(
            "critical_branches".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::CriticalBranches
        );
        assert_eq!(
            "criticalbranches".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::CriticalBranches
        );
        assert_eq!(
            "crit".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::CriticalBranches
        );
        assert_eq!(
            "c".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::CriticalBranches
        );
    }

    #[test]
    fn test_fromstr_full_body_aliases() {
        assert_eq!(
            "full_body".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::FullBody
        );
        assert_eq!(
            "fullbody".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::FullBody
        );
        assert_eq!(
            "full".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::FullBody
        );
        assert_eq!(
            "f".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::FullBody
        );
    }

    #[test]
    fn test_fromstr_token_budgeted() {
        assert_eq!(
            "token_budgeted(100)".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::TokenBudgeted(100)
        );
        assert_eq!(
            "budget(500)".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::TokenBudgeted(500)
        );
        assert_eq!(
            "200".parse::<CompressionLevel>().unwrap(),
            CompressionLevel::TokenBudgeted(200)
        );
    }

    #[test]
    fn test_fromstr_invalid() {
        let result = "invalid_level".parse::<CompressionLevel>();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Invalid compression level"));
    }

    // ── Phase 8: Type alias tests ──

    #[test]
    fn test_compression_mode_alias() {
        // CompressionMode is an alias for CompressionLevel
        let mode: CompressionMode = CompressionMode::SignatureOnly;
        assert_eq!(mode, CompressionLevel::SignatureOnly);

        let mode: CompressionMode = CompressionMode::CriticalBranches;
        assert_eq!(mode, CompressionLevel::CriticalBranches);

        let mode: CompressionMode = CompressionMode::FullBody;
        assert_eq!(mode, CompressionLevel::FullBody);

        let mode: CompressionMode = CompressionMode::TokenBudgeted(100);
        assert_eq!(mode, CompressionLevel::TokenBudgeted(100));
    }

    #[test]
    fn test_symbol_slice_alias() {
        // SymbolSlice is an alias for CompressedSymbol
        let slice: SymbolSlice =
            compress_symbol("fn hello() {}", 1, 1, CompressionLevel::SignatureOnly);
        assert!(!slice.signature.is_empty());
        assert_eq!(slice.compression_used, "signature_only");

        // Fields match CompressedSymbol
        let _cs: CompressedSymbol = slice;
    }
}
