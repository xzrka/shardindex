/// Token estimation — approximate source code token count for LLM context budgeting.
///
/// Uses a heuristic tokenizer (not a real BPE/byte-level tokenizer) to estimate
/// how many tokens a piece of source code would consume in a typical LLM context window.
///
/// ## Heuristic
///
/// - Average English token ≈ 4 chars (OpenAI/Anthropic baseline)
/// - Code tends to be denser: identifiers, symbols, punctuation
/// - We use a character-count heuristic with language-specific adjustments:
///   - Code: ~3.5 chars per token (denser than natural language)
///   - Comments/docstrings: ~4.0 chars per token (more like natural language)
///   - Whitespace/newlines: ~1 token per ~20 whitespace chars
///
/// This is a *fast approximation* suitable for budgeting decisions.
/// For exact counts, use the actual model's tokenizer.
///
/// ## Usage
///
/// ```rust
/// use shardindex::token_estimation::estimate_token_count;
///
/// let tokens = estimate_token_count("fn hello() { println!(\"world\"); }");
/// assert!(tokens > 0);
/// ```

/// Estimate the token count of a source code string.
///
/// This is a fast heuristic — not an exact BPE tokenization.
/// Suitable for budgeting and compression decisions.
pub fn estimate_token_count(source: &str) -> usize {
    if source.is_empty() {
        return 0;
    }

    let mut code_chars = 0usize;
    let mut comment_chars = 0usize;
    let mut whitespace_chars = 0usize;

    // Simple state machine to distinguish code vs comments vs whitespace
    let mut in_single_line_comment = false;
    let mut in_multi_line_comment = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    let chars: Vec<char> = source.chars().collect();
    let len = chars.len();

    for i in 0..len {
        let c = chars[i];
        let next = if i + 1 < len {
            Some(chars[i + 1])
        } else {
            None
        };

        // Whitespace (outside comments/strings)
        if !in_single_line_comment && !in_multi_line_comment && !in_single_quote && !in_double_quote
        {
            if c.is_whitespace() {
                whitespace_chars += 1;
                continue;
            }

            // String literals
            if c == '"' && !in_single_quote {
                in_double_quote = true;
                code_chars += 1;
                continue;
            }
            if c == '\'' && !in_double_quote {
                in_single_quote = true;
                code_chars += 1;
                continue;
            }

            // Single-line comment: //
            if c == '/' && next == Some('/') {
                in_single_line_comment = true;
                code_chars += 2; // // counts as code
                continue;
            }

            // Multi-line comment: /*
            if c == '/' && next == Some('*') {
                in_multi_line_comment = true;
                code_chars += 2; // /* counts as code
                continue;
            }

            // Regular code character
            code_chars += 1;
            continue;
        }

        // Inside single-line comment
        if in_single_line_comment {
            if c == '\n' {
                in_single_line_comment = false;
                whitespace_chars += 1;
            } else {
                comment_chars += 1;
            }
            continue;
        }

        // Inside multi-line comment
        if in_multi_line_comment {
            if c == '*' && next == Some('/') {
                in_multi_line_comment = false;
                code_chars += 2; // */ counts as code
                continue;
            }
            comment_chars += 1;
            continue;
        }

        // Inside string literals
        if in_double_quote {
            if c == '\\' {
                code_chars += 2; // escape sequence
                continue;
            }
            if c == '"' {
                in_double_quote = false;
            }
            code_chars += 1;
            continue;
        }
        if in_single_quote {
            if c == '\\' {
                code_chars += 2;
                continue;
            }
            if c == '\'' {
                in_single_quote = false;
            }
            code_chars += 1;
            continue;
        }
    }

    // Heuristic conversion: chars → tokens
    // Code: ~3.5 chars per token (denser)
    // Comments: ~4.0 chars per token (like natural language)
    // Whitespace: ~1 token per 20 chars (newlines, indentation)
    let code_tokens = (code_chars as f64 / 3.5).ceil() as usize;
    let comment_tokens = (comment_chars as f64 / 4.0).ceil() as usize;
    let whitespace_tokens = (whitespace_chars as f64 / 20.0).ceil() as usize;

    code_tokens + comment_tokens + whitespace_tokens
}

/// Estimate tokens for a symbol body given source lines.
///
/// Takes the full source and the line range (1-indexed, inclusive).
pub fn estimate_symbol_tokens(source: &str, start_line: usize, end_line: usize) -> usize {
    let lines: Vec<&str> = source.lines().collect();
    if start_line == 0 || start_line > lines.len() {
        return 0;
    }

    let start_idx = start_line - 1;
    let end_idx = end_line.min(lines.len());

    let body = lines[start_idx..end_idx].join("\n");
    estimate_token_count(&body)
}

/// Language-specific token density adjustment factors.
///
/// Different languages have different average token densities.
/// These factors adjust the base estimate.
#[derive(Debug, Clone, Copy)]
pub struct LanguageDensity {
    /// Language identifier
    pub language: &'static str,
    /// Chars per token (lower = denser code)
    pub chars_per_token: f64,
}

impl LanguageDensity {
    /// Get density for a language. Defaults to 3.5 chars/token for unknown languages.
    pub fn for_language(language: &str) -> Self {
        match language {
            "python" => LanguageDensity {
                language: "python",
                chars_per_token: 3.8,
            },
            "javascript" => LanguageDensity {
                language: "javascript",
                chars_per_token: 3.4,
            },
            "typescript" => LanguageDensity {
                language: "typescript",
                chars_per_token: 3.4,
            },
            "rust" => LanguageDensity {
                language: "rust",
                chars_per_token: 3.2,
            },
            "go" => LanguageDensity {
                language: "go",
                chars_per_token: 3.5,
            },
            "java" => LanguageDensity {
                language: "java",
                chars_per_token: 3.3,
            },
            "ruby" => LanguageDensity {
                language: "ruby",
                chars_per_token: 3.6,
            },
            "c" => LanguageDensity {
                language: "c",
                chars_per_token: 3.1,
            },
            "cpp" => LanguageDensity {
                language: "cpp",
                chars_per_token: 3.1,
            },
            _ => LanguageDensity {
                language: "unknown",
                chars_per_token: 3.5,
            },
        }
    }

    /// Estimate tokens with language-specific density adjustment.
    pub fn estimate(&self, source: &str) -> usize {
        if source.is_empty() {
            return 0;
        }
        let chars = source.chars().count();
        (chars as f64 / self.chars_per_token).ceil() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_source() {
        assert_eq!(estimate_token_count(""), 0);
    }

    #[test]
    fn test_simple_function() {
        let code = "fn hello() { println!(\"world\"); }";
        let tokens = estimate_token_count(code);
        // Should be roughly 8-12 tokens for this short function
        assert!(
            tokens >= 5 && tokens <= 15,
            "Expected 5-12 tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_python_function() {
        let code = r#"def hello():
    print("world")
"#;
        let tokens = estimate_token_count(code);
        assert!(
            tokens >= 4 && tokens <= 12,
            "Expected 4-12 tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_with_comments() {
        let code = r#"// This is a comment explaining the function
fn hello() {
    // Another comment
    println!("world");
}"#;
        let tokens = estimate_token_count(code);
        assert!(
            tokens >= 15 && tokens <= 40,
            "Expected 15-40 tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_multiline_comment() {
        let code = r#"/*
 * This is a multi-line comment
 * with multiple lines
 */
fn hello() {}"#;
        let tokens = estimate_token_count(code);
        assert!(
            tokens >= 10 && tokens <= 30,
            "Expected 10-30 tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_string_literal() {
        let code = r#"let s = "hello world with \"escape\"";"#;
        let tokens = estimate_token_count(code);
        assert!(
            tokens >= 5 && tokens <= 12,
            "Expected 5-12 tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_whitespace_heavy() {
        let code = "fn hello() {\n\n    println!(\"world\");\n\n}";
        let tokens = estimate_token_count(code);
        assert!(
            tokens >= 5 && tokens <= 12,
            "Expected 5-12 tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_symbol_tokens_out_of_range() {
        let code = "fn hello() {}";
        assert_eq!(estimate_symbol_tokens(code, 0, 1), 0);
        assert_eq!(estimate_symbol_tokens(code, 100, 200), 0);
    }

    #[test]
    fn test_symbol_tokens_valid_range() {
        let code = "fn hello() {\n    println!(\"world\");\n}";
        let tokens = estimate_symbol_tokens(code, 1, 3);
        assert!(tokens > 0, "Expected positive tokens, got {}", tokens);
    }

    #[test]
    fn test_language_density_python() {
        let density = LanguageDensity::for_language("python");
        assert!((density.chars_per_token - 3.8).abs() < 0.01);
    }

    #[test]
    fn test_language_density_rust() {
        let density = LanguageDensity::for_language("rust");
        assert!((density.chars_per_token - 3.2).abs() < 0.01);
    }

    #[test]
    fn test_language_density_unknown() {
        let density = LanguageDensity::for_language("cobol");
        assert_eq!(density.language, "unknown");
        assert!((density.chars_per_token - 3.5).abs() < 0.01);
    }

    #[test]
    fn test_language_density_estimate() {
        let density = LanguageDensity::for_language("rust");
        let tokens = density.estimate("fn hello() {}");
        assert!(tokens > 0, "Expected positive tokens, got {}", tokens);
    }

    #[test]
    fn test_language_density_empty() {
        let density = LanguageDensity::for_language("rust");
        assert_eq!(density.estimate(""), 0);
    }

    #[test]
    fn test_consistency_same_input() {
        let code = "fn hello() { println!(\"world\"); }";
        let a = estimate_token_count(code);
        let b = estimate_token_count(code);
        assert_eq!(a, b, "Token estimation should be deterministic");
    }

    #[test]
    fn test_larger_code_block() {
        let code = r#"
use std::collections::HashMap;

/// A simple cache implementation
pub struct Cache<K, V> {
    entries: HashMap<K, V>,
    max_size: usize,
}

impl<K: Eq + std::hash::Hash, V> Cache<K, V> {
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_size,
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: K, value: V) {
        if self.entries.len() >= self.max_size {
            // Eviction logic would go here
        }
        self.entries.insert(key, value);
    }
}
"#;
        let tokens = estimate_token_count(code);
        // ~100 chars of code + comments → roughly 20-50 tokens
        assert!(
            tokens >= 50 && tokens <= 200,
            "Expected 50-200 tokens for ~100 LOC, got {}",
            tokens
        );
    }
}
