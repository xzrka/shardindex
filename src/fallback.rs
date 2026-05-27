//! Phase 11.2 — Filesystem Fallback Protocol
//!
//! When ShardIndex fails (stale index, symbol not found, parser error),
//! this module provides a grep/ripgrep-based fallback that:
//!
//! 1. Attempts grep/ripgrep for symbol name in repo
//! 2. Reads top 3 matching files (limited to 200 lines each)
//! 3. Injects warning: "ShardIndex unavailable. Using filesystem fallback."
//! 4. After filesystem read, returns results with fallback metadata
//!
//! Per masterplan §11.2.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, ShardError, ShardResult};

// ---------------------------------------------------------------------------
// FallbackResult — structured output from filesystem fallback
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackResult {
    /// The original symbol name that was searched
    pub symbol: String,
    /// Whether fallback succeeded
    pub success: bool,
    /// Warning message injected into the response
    pub warning: String,
    /// Matching files from grep/ripgrep
    pub matches: Vec<FallbackMatch>,
    /// Files enqueued for indexing (after fallback read)
    pub enqueued_for_indexing: Vec<String>,
    /// Source: "filesystem_fallback" to distinguish from ShardIndex results
    #[serde(rename = "source")]
    pub source_tag: String,
}

impl FallbackResult {
    pub fn success(symbol: &str, matches: Vec<FallbackMatch>) -> Self {
        Self {
            symbol: symbol.to_string(),
            success: true,
            warning:
                "ShardIndex unavailable. Using filesystem fallback. Results may be incomplete."
                    .to_string(),
            matches,
            enqueued_for_indexing: Vec::new(),
            source_tag: "filesystem_fallback".to_string(),
        }
    }

    pub fn not_found(symbol: &str) -> Self {
        Self {
            symbol: symbol.to_string(),
            success: false,
            warning: format!(
                "ShardIndex unavailable. Filesystem fallback found no matches for '{}'.",
                symbol
            ),
            matches: Vec::new(),
            enqueued_for_indexing: Vec::new(),
            source_tag: "filesystem_fallback".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// FallbackMatch — single grep match with context
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackMatch {
    /// Relative file path
    pub file: String,
    /// Absolute file path
    pub abs_path: String,
    /// Matching line number
    pub line: usize,
    /// Matching line content
    pub content: String,
    /// Context lines before the match (up to 2)
    pub context_before: Vec<String>,
    /// Context lines after the match (up to 2)
    pub context_after: Vec<String>,
    /// Estimated token count for this match
    pub estimated_tokens: usize,
}

// ---------------------------------------------------------------------------
// FallbackConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FallbackConfig {
    /// Maximum number of matching files to return
    pub max_files: usize,
    /// Maximum lines per file to read
    pub max_lines_per_file: usize,
    /// Context lines around each match
    pub context_lines: usize,
    /// Use ripgrep if available, fall back to grep
    pub prefer_ripgrep: bool,
}

impl Default for FallbackConfig {
    fn default() -> Self {
        Self {
            max_files: 3,
            max_lines_per_file: 200,
            context_lines: 2,
            prefer_ripgrep: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Execute filesystem fallback for a symbol search.
///
/// # Arguments
/// * `repo_root` - Repository root directory
/// * `symbol` - Symbol name to search for
/// * `config` - Fallback configuration
///
/// # Returns
/// `FallbackResult` with matches, warnings, and metadata
pub fn filesystem_fallback(
    repo_root: &Path,
    symbol: &str,
    config: &FallbackConfig,
) -> ShardResult<FallbackResult> {
    if !repo_root.exists() {
        return Err(ShardError::new(
            ErrorCode::IoError,
            format!("Repository root does not exist: {}", repo_root.display()),
        ));
    }

    // Step 1: Search with ripgrep or grep
    let raw_matches = search_symbol(repo_root, symbol, config)?;

    if raw_matches.is_empty() {
        return Ok(FallbackResult::not_found(symbol));
    }

    // Step 2: Read context for each match
    let matches = read_match_context(repo_root, &raw_matches, config)?;

    Ok(FallbackResult::success(symbol, matches))
}

/// Search for a symbol name in the repository using ripgrep or grep.
fn search_symbol(
    repo_root: &Path,
    symbol: &str,
    config: &FallbackConfig,
) -> ShardResult<Vec<RawMatch>> {
    let mut matches = Vec::new();

    // Try ripgrep first if preferred
    if config.prefer_ripgrep {
        if let Ok(rg_matches) = search_with_ripgrep(repo_root, symbol, config) {
            matches.extend(rg_matches);
        }
    }

    // Fall back to grep if ripgrep failed or wasn't preferred
    if matches.is_empty() {
        if let Ok(grep_matches) = search_with_grep(repo_root, symbol, config) {
            matches.extend(grep_matches);
        }
    }

    // Limit to max_files
    let mut seen_files = std::collections::HashSet::new();
    matches.retain(|m| seen_files.insert(m.file_path.clone()));
    matches.truncate(config.max_files);

    Ok(matches)
}

/// Search using ripgrep (rg).
fn search_with_ripgrep(
    repo_root: &Path,
    symbol: &str,
    config: &FallbackConfig,
) -> ShardResult<Vec<RawMatch>> {
    let output = std::process::Command::new("rg")
        .args([
            "--no-heading",
            "--with-filename",
            "--line-number",
            "--color",
            "never",
            "--max-count",
            &config.max_files.to_string(),
            "--glob",
            "!*.git",
            "--glob",
            "!node_modules",
            "--glob",
            "!*.shardindex",
            "--glob",
            "!*.pyc",
            "--glob",
            "!*.o",
            "--glob",
            "!*.so",
            "--glob",
            "!*.dll",
            "--glob",
            "!*.dylib",
            "--glob",
            "!*.egg-info",
            "--glob",
            "!__pycache__",
            "--glob",
            "!dist",
            "--glob",
            "!build",
            "--glob",
            "!target",
            "--glob",
            "!vendor",
            "--glob",
            "!.venv",
            "--glob",
            "!venv",
            "-F", // Fixed string (not regex) for symbol search
            symbol,
        ])
        .current_dir(repo_root)
        .output()
        .map_err(|e| {
            ShardError::new(
                ErrorCode::IoError,
                format!("ripgrep not found or failed: {}", e),
            )
        })?;

    if !output.status.success() {
        return Ok(Vec::new()); // ripgrep returned no results or error
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_grep_output(&stdout, repo_root)
}

/// Search using grep.
fn search_with_grep(
    repo_root: &Path,
    symbol: &str,
    _config: &FallbackConfig,
) -> ShardResult<Vec<RawMatch>> {
    let output = std::process::Command::new("grep")
        .args([
            "-r",
            "-n",
            "-F", // Fixed string
            "--include=*.rs",
            "--include=*.py",
            "--include=*.ts",
            "--include=*.js",
            "--include=*.go",
            "--include=*.java",
            "--include=*.c",
            "--include=*.cpp",
            "--include=*.h",
            "--include=*.rb",
            "--include=*.php",
            "--include=*.lua",
            "--include=*.swift",
            "--include=*.scala",
            "--include=*.kt",
            "--include=*.dart",
            "--include=*.hs",
            "--include=*.ex",
            "--include=*.exs",
            "--include=*.zig",
            "--include=*.md",
            "--exclude-dir=.git",
            "--exclude-dir=node_modules",
            "--exclude-dir=.shardindex",
            "--exclude-dir=__pycache__",
            "--exclude-dir=target",
            "--exclude-dir=vendor",
            "--exclude-dir=dist",
            "--exclude-dir=build",
            symbol,
        ])
        .current_dir(repo_root)
        .output()
        .map_err(|e| ShardError::new(ErrorCode::IoError, format!("grep failed: {}", e)))?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_grep_output(&stdout, repo_root)
}

/// Parse grep/ripgrep output into RawMatch structs.
/// Format: `file_path:line_number:content`
fn parse_grep_output(output: &str, repo_root: &Path) -> ShardResult<Vec<RawMatch>> {
    let mut matches = Vec::new();

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }

        // Split on first two colons: file:line:content
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 3 {
            continue;
        }

        let file_path = parts[0].to_string();
        let line_num: usize = parts[1].parse().unwrap_or(0);
        let content = parts[2].to_string();

        let abs_path = repo_root.join(&file_path);
        if !abs_path.exists() {
            continue;
        }

        matches.push(RawMatch {
            file_path,
            abs_path,
            line: line_num,
            content,
        });
    }

    Ok(matches)
}

/// Read context lines around each match.
fn read_match_context(
    repo_root: &Path,
    raw_matches: &[RawMatch],
    config: &FallbackConfig,
) -> ShardResult<Vec<FallbackMatch>> {
    let mut matches = Vec::new();

    for raw in raw_matches {
        let file_path = repo_root.join(&raw.file_path);
        let content = std::fs::read_to_string(&file_path).map_err(|e| {
            ShardError::new(
                ErrorCode::IoError,
                format!("Failed to read {}: {}", raw.file_path, e),
            )
        })?;

        let lines: Vec<&str> = content.lines().collect();
        let match_idx = raw.line.saturating_sub(1); // 0-indexed

        // Context before: up to context_lines before the match
        let before_start = match_idx.saturating_sub(config.context_lines);
        let context_before: Vec<String> = lines[before_start..match_idx]
            .iter()
            .map(|l| l.to_string())
            .collect();

        // Context after: up to context_lines after the match
        let after_end = std::cmp::min(match_idx + 1 + config.context_lines, lines.len());
        let context_after: Vec<String> = lines[match_idx + 1..after_end]
            .iter()
            .map(|l| l.to_string())
            .collect();

        // Estimated tokens for the context window
        let window_start = before_start;
        let window_end = after_end;
        let estimated_tokens = (lines[window_start..window_end].join("\n").len() / 4).max(1);

        matches.push(FallbackMatch {
            file: raw.file_path.clone(),
            abs_path: raw.abs_path.display().to_string(),
            line: raw.line,
            content: raw.content.clone(),
            context_before,
            context_after,
            estimated_tokens,
        });
    }

    Ok(matches)
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RawMatch {
    file_path: String,
    abs_path: PathBuf,
    line: usize,
    content: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_repo() -> TempDir {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // Create directories first
        fs::create_dir_all(root.join("src/auth")).expect("create src/auth");
        fs::create_dir_all(root.join("src/api")).expect("create src/api");
        fs::create_dir_all(root.join("src/core")).expect("create src/core");

        // Create a Python file with a function
        fs::write(
            root.join("src/auth/login.py"),
            r#"def login(user_id, password):
    """Authenticate user."""
    user = get_user(user_id)
    if verify_password(user, password):
        return create_session(user)
    return None

def get_user(user_id):
    """Fetch user from database."""
    pass
"#,
        )
        .expect("write file");

        // Create a TypeScript file
        fs::create_dir_all(root.join("src/api")).expect("create dir");
        fs::write(
            root.join("src/api/users.ts"),
            r#"import { login } from '../auth/login';

export async function handleLogin(req: Request): Promise<Response> {
    const { user_id, password } = req.body;
    const session = await login(user_id, password);
    if (!session) {
        throw new Error('Authentication failed');
    }
    return Response.json({ success: true });
}
"#,
        )
        .expect("write file");

        // Create a Rust file
        fs::create_dir_all(root.join("src/core")).expect("create dir");
        fs::write(
            root.join("src/core/session.rs"),
            r#"pub fn create_session(user: &User) -> Session {
    Session {
        id: generate_id(),
        user_id: user.id,
        created_at: chrono::Utc::now(),
    }
}

pub struct Session {
    pub id: String,
    pub user_id: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
"#,
        )
        .expect("write file");

        dir
    }

    #[test]
    fn test_fallback_not_found() {
        let dir = create_test_repo();
        let result = filesystem_fallback(
            dir.path(),
            "nonexistent_symbol_xyz",
            &FallbackConfig::default(),
        );

        assert!(result.is_ok());
        let fb = result.unwrap();
        assert!(!fb.success);
        assert_eq!(fb.matches.len(), 0);
        assert_eq!(fb.source_tag, "filesystem_fallback");
        assert!(fb.warning.contains("nonexistent_symbol_xyz"));
    }

    #[test]
    fn test_fallback_finds_symbol() {
        let dir = create_test_repo();
        let result = filesystem_fallback(dir.path(), "login", &FallbackConfig::default());

        assert!(result.is_ok());
        let fb = result.unwrap();
        assert!(fb.success);
        assert!(!fb.matches.is_empty());
        assert_eq!(fb.source_tag, "filesystem_fallback");
        assert!(fb.warning.contains("filesystem fallback"));
    }

    #[test]
    fn test_fallback_max_files() {
        let dir = create_test_repo();
        let config = FallbackConfig {
            max_files: 1,
            ..Default::default()
        };
        let result = filesystem_fallback(dir.path(), "login", &config);

        assert!(result.is_ok());
        let fb = result.unwrap();
        assert!(fb.matches.len() <= 1);
    }

    #[test]
    fn test_fallback_invalid_repo() {
        let result = filesystem_fallback(
            Path::new("/nonexistent/path"),
            "anything",
            &FallbackConfig::default(),
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, ErrorCode::IoError);
    }

    #[test]
    fn test_fallback_result_serialization() {
        let dir = create_test_repo();
        let result = filesystem_fallback(dir.path(), "login", &FallbackConfig::default());

        assert!(result.is_ok());
        let fb = result.unwrap();

        // Verify it serializes to JSON
        let json = serde_json::to_string(&fb).expect("serialize fallback result");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse json");

        assert!(parsed["success"].as_bool().unwrap());
        assert_eq!(parsed["source"], "filesystem_fallback");
        assert!(parsed["warning"].is_string());
        assert!(parsed["matches"].is_array());
    }

    #[test]
    fn test_fallback_match_has_context() {
        let dir = create_test_repo();
        let config = FallbackConfig {
            context_lines: 3,
            ..Default::default()
        };
        let result = filesystem_fallback(dir.path(), "login", &config);

        assert!(result.is_ok());
        let fb = result.unwrap();

        for match_item in &fb.matches {
            // Each match should have context
            assert!(match_item.context_before.len() <= 3);
            assert!(match_item.context_after.len() <= 3);
            assert!(match_item.estimated_tokens > 0);
        }
    }

    #[test]
    fn test_parse_grep_output() {
        let repo_root = Path::new("/tmp/test");
        let output = "src/auth/login.py:1:def login(user_id, password):\nsrc/api/users.ts:1:import { login }";

        // Note: This test will skip files that don't exist
        let matches = parse_grep_output(output, repo_root).unwrap();
        assert!(matches.is_empty()); // Files don't exist, so all filtered out
    }

    #[test]
    fn test_fallback_config_defaults() {
        let config = FallbackConfig::default();
        assert_eq!(config.max_files, 3);
        assert_eq!(config.max_lines_per_file, 200);
        assert_eq!(config.context_lines, 2);
        assert!(config.prefer_ripgrep);
    }
}
