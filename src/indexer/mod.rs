/// Indexer engine — file scan → Blake3 hash → AST parsing → DB storage
///
/// Incremental indexing: only reparse changed files. Supports multiple languages.

pub mod ast;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Context;
use tracing::{info, debug, warn};

use crate::database::{IndexDb, SymbolRecord};
use ast::{GoParser, JavaScriptParser, PythonParser, RustParser, SourceCodeParser, TypeScriptParser};

/// Supported language parsers
#[derive(Debug, Clone, Copy)]
pub enum Language {
    Python,
    JavaScript,
    Rust,
    TypeScript,
    Go,
}

impl Language {
    /// Detect language from file extension
    #[allow(dead_code)]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "py" => Some(Language::Python),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "rs" => Some(Language::Rust),
            "ts" | "tsx" | "mts" | "cts" => Some(Language::TypeScript),
            "go" => Some(Language::Go),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::JavaScript => "javascript",
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::Go => "go",
        }
    }

    /// File extensions for this language
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            Language::Python => &["py"],
            Language::JavaScript => &["js", "jsx", "mjs", "cjs"],
            Language::Rust => &["rs"],
            Language::TypeScript => &["ts", "tsx", "mts", "cts"],
            Language::Go => &["go"],
        }
    }

    /// Create a parser for this language
    pub fn create_parser(&self) -> Result<Box<dyn SourceCodeParser>, anyhow::Error> {
        match self {
            Language::Python => Ok(Box::new(PythonParser::new()?)),
            Language::JavaScript => Ok(Box::new(JavaScriptParser::new()?)),
            Language::Rust => Ok(Box::new(RustParser::new()?)),
            Language::TypeScript => Ok(Box::new(TypeScriptParser::new()?)),
            Language::Go => Ok(Box::new(GoParser::new()?)),
        }
    }
    
    /// All supported extensions (for auto-detection in multi-lang mode)
    #[allow(dead_code)]
    pub fn all_extensions() -> &'static [(&'static str, Language)] {
        &[
            ("py", Language::Python),
            ("js", Language::JavaScript),
            ("jsx", Language::JavaScript),
            ("mjs", Language::JavaScript),
            ("cjs", Language::JavaScript),
            ("rs", Language::Rust),
            ("ts", Language::TypeScript),
            ("tsx", Language::TypeScript),
            ("mts", Language::TypeScript),
            ("cts", Language::TypeScript),
            ("go", Language::Go),
        ]
    }
}

/// Project indexer
pub struct ProjectIndexer {
    db: IndexDb,
    root: PathBuf,
    language: Language,
    parser: Box<dyn SourceCodeParser>,
}

impl ProjectIndexer {
    /// Create a new indexer
    pub fn new(db: IndexDb, root: PathBuf, language: Language) -> Result<Self, anyhow::Error> {
        let parser = language.create_parser()?;
        Ok(Self { db, root, language, parser })
    }

    /// Full project indexing (initial)
    pub fn index_all(&mut self) -> Result<(usize, usize, usize), anyhow::Error> {
        info!("Indexing project at {} ({})", self.root.display(), self.language.as_str());

        let files = self.scan_files()?;
        let mut symbols = 0;
        let mut refs = 0;

        for file in &files {
            match self.index_file(file) {
                Ok((s, r)) => {
                    symbols += s;
                    refs += r;
                }
                Err(e) => {
                    warn!("Failed to index {}: {}", file.display(), e);
                }
            }
        }

        // Clean up deleted files from DB
        self.clean_deleted_files(&files)?;

        info!(
            "Indexing complete: {} files, {} symbols, {} references",
            files.len(), symbols, refs
        );

        Ok((files.len(), symbols, refs))
    }

    /// Index a single file (change detection + reparse)
    pub fn index_file(&mut self, path: &Path) -> Result<(usize, usize), anyhow::Error> {
        let relative = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Compute Blake3 hash
        let content = fs::read_to_string(path)
            .context(format!("Read file: {}", path.display()))?;
        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        let size = content.len() as u64;

        // File modification time
        let modified = fs::metadata(path)?
            .modified()
            .ok()
            .map(|t| {
                use chrono::{DateTime, Utc};
                DateTime::<Utc>::from(t).to_rfc3339()
            })
            .unwrap_or_default();

        // Compare with previous hash
        if let Some(old_hash) = self.db.get_file_hash(&relative) {
            if old_hash == hash {
                debug!("{} unchanged, skipping", relative);
                return Ok((0, 0));
            }
            debug!(
                "{} changed ({:?} -> {:?}), reindexing",
                relative, &old_hash[..8], &hash[..8]
            );
        }

        // Store hash
        self.db.upsert_file(&relative, &hash, size, &modified)?;

        // Remove old symbols/refs for this file before re-indexing
        self.db.remove_file_symbols(&relative)?;

        // AST parsing
        let result = self.parser.parse(&content)?;

        // Store symbols
        let mut symbol_count = 0;
        for sym in &result.symbols {
            let _id = self.db.insert_symbol(&SymbolRecord {
                id: 0,
                file_path: relative.clone(),
                name: sym.name.clone(),
                kind: sym.kind.as_str().to_string(),
                start_line: sym.start_line,
                end_line: sym.end_line,
                start_col: sym.start_col,
                end_col: sym.end_col,
                signature: sym.signature.clone(),
                docstring: sym.docstring.clone(),
                parent_symbol: sym.parent.clone(),
            })?;
            symbol_count += 1;
        }

        // Store references
        let mut ref_count = 0;
        for ref_rec in &result.references {
            self.db.insert_reference(&crate::database::ReferenceRecord {
                id: 0,
                caller_file: relative.clone(),
                callee_file: relative.clone(), // same-file reference (default)
                caller_symbol: ref_rec.caller_symbol.clone(),
                callee_symbol: ref_rec.callee_symbol.clone(),
                ref_kind: ref_rec.ref_kind.clone(),
                line: ref_rec.line,
            })?;
            ref_count += 1;
        }

        debug!("{}: {} symbols, {} refs", relative, symbol_count, ref_count);
        Ok((symbol_count, ref_count))
    }

    /// Clean up deleted files
    fn clean_deleted_files(&self, existing: &[PathBuf]) -> Result<(), anyhow::Error> {
        let existing_set: HashSet<String> = existing
            .iter()
            .filter_map(|p| {
                p.strip_prefix(&self.root)
                    .ok()
                    .map(|r| r.to_string_lossy().to_string())
            })
            .collect();

        let db_files = self.db.all_file_hashes()?;
        for file in &db_files {
            if !existing_set.contains(&file.path) {
                debug!("Removed stale file: {}", file.path);
                self.db.remove_file(&file.path)?;
            }
        }
        Ok(())
    }

    /// Scan source files matching this language's extensions
    fn scan_files(&self) -> Result<Vec<PathBuf>, anyhow::Error> {
        let mut files = Vec::new();
        self.walk_dir(&self.root, &mut files)?;
        Ok(files)
    }

    fn walk_dir(&self, dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), anyhow::Error> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Skip directories
                let skip = [
                    ".git", "__pycache__", ".venv", "venv", "node_modules", ".mypy_cache",
                    ".tox", ".eggs", "dist", "build", ".next", ".nuxt",
                ];
                if path
                    .file_name()
                    .map_or(false, |n| skip.contains(&n.to_string_lossy().as_ref()))
                {
                    continue;
                }
                self.walk_dir(&path, files)?;
            } else {
                // Check extension
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if self.language.extensions().contains(&ext) {
                        files.push(path);
                    }
                }
            }
        }
        Ok(())
    }
}
