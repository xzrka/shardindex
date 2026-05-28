mod r#c;
mod bash;
mod cpp;
mod csharp;
mod css;
mod dart;
mod elixir;
mod graphql;
mod r#go;
mod haskell;
mod java;
mod javascript;
mod julia;
mod kotlin;
mod lua;
mod markdown;
mod php;
mod python;
mod ruby;
mod r#rust;
mod scala;
mod sql;
mod swift;
#[cfg(test)]
mod tests;
mod r#typescript;
mod vue;
/// Indexer engine — file scan → Blake3 hash → AST parsing → DB storage
///
/// Incremental indexing: only reparse changed files. Supports multiple languages.
// Module declarations — each language parser in its own file
pub mod types;
mod zig;

// Re-export types for convenience and test access
#[allow(unused_imports)]
pub use types::{ParseResult, ParsedReference, ParsedSymbol, SourceCodeParser, SymbolKind};
// Re-export language parsers
pub use bash::BashParser;
pub use r#c::CParser;
pub use cpp::CppParser;
pub use csharp::CSharpParser;
pub use css::CssParser;
pub use dart::DartParser;
pub use elixir::ElixirParser;
pub use graphql::GraphqlParser;
pub use r#go::GoParser;
pub use haskell::HaskellParser;
pub use java::JavaParser;
pub use javascript::JavaScriptParser;
pub use julia::JuliaParser;
pub use kotlin::KotlinParser;
pub use lua::LuaParser;
pub use markdown::MarkdownParser;
pub use php::PhpParser;
pub use python::PythonParser;
pub use ruby::RubyParser;
pub use r#rust::RustParser;
pub use scala::ScalaParser;
pub use sql::SqlParser;
pub use swift::SwiftParser;
pub use r#typescript::TypeScriptParser;
pub use vue::VueParser;
pub use zig::ZigParser;

use anyhow::Context;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use crate::database::{IndexDb, SymbolRecord};
use crate::token_estimation::{estimate_symbol_tokens, estimate_token_count};

/// Supported language parsers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Python,
    JavaScript,
    Rust,
    TypeScript,
    Go,
    Ruby,
    Java,
    Php,
    Julia,
    Lua,
    Swift,
    Zig,
    Scala,
    Elixir,
    Dart,
    Haskell,
    C,
    Cpp,
    Markdown,
    Sql,
    Graphql,
    Vue,
    Css,
    Bash,
    Kotlin,
    CSharp,
}

impl Language {
    /// Detect language from file path or extension string.
    /// Accepts full paths ("src/file.py"), filenames ("file.py"), or bare extensions ("py").
    #[allow(dead_code)]
    pub fn from_extension(path: &str) -> Option<Self> {
        let ext = path.rsplit('.').next().unwrap_or(path);
        match ext {
            "py" => Some(Language::Python),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "rs" => Some(Language::Rust),
            "ts" | "tsx" | "mts" | "cts" => Some(Language::TypeScript),
            "go" => Some(Language::Go),
            "rb" | "gemspec" => Some(Language::Ruby),
            "java" => Some(Language::Java),
            "php" => Some(Language::Php),
            "jl" => Some(Language::Julia),
            "lua" => Some(Language::Lua),
            "swift" => Some(Language::Swift),
            "zig" => Some(Language::Zig),
            "scala" => Some(Language::Scala),
            "ex" | "exs" => Some(Language::Elixir),
            "dart" => Some(Language::Dart),
            "hs" | "lhs" => Some(Language::Haskell),
            "c" | "h" => Some(Language::C),
            "cpp" | "hpp" | "cc" | "cxx" | "hxx" | "hh" => Some(Language::Cpp),
            "md" | "markdown" | "mdown" | "mkd" => Some(Language::Markdown),
            "sql" => Some(Language::Sql),
            "graphql" | "gql" => Some(Language::Graphql),
            "vue" => Some(Language::Vue),
            "css" | "scss" | "sass" => Some(Language::Css),
            "sh" | "bash" | "zsh" => Some(Language::Bash),
            "kt" | "kts" => Some(Language::Kotlin),
            "cs" => Some(Language::CSharp),
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
            Language::Ruby => "ruby",
            Language::Java => "java",
            Language::Php => "php",
            Language::Julia => "julia",
            Language::Lua => "lua",
            Language::Swift => "swift",
            Language::Zig => "zig",
            Language::Scala => "scala",
            Language::Elixir => "elixir",
            Language::Dart => "dart",
            Language::Haskell => "haskell",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Markdown => "markdown",
            Language::Sql => "sql",
            Language::Graphql => "graphql",
            Language::Vue => "vue",
            Language::Css => "css",
            Language::Bash => "bash",
            Language::Kotlin => "kotlin",
            Language::CSharp => "csharp",
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
            Language::Ruby => &["rb", "gemspec"],
            Language::Java => &["java"],
            Language::Php => &["php"],
            Language::Julia => &["jl"],
            Language::Lua => &["lua"],
            Language::Swift => &["swift"],
            Language::Zig => &["zig"],
            Language::Scala => &["scala"],
            Language::Elixir => &["ex", "exs"],
            Language::Dart => &["dart"],
            Language::Haskell => &["hs", "lhs"],
            Language::C => &["c", "h"],
            Language::Cpp => &["cpp", "hpp", "cc", "cxx", "hxx", "hh", "h"],
            Language::Markdown => &["md", "markdown", "mdown", "mkd"],
            Language::Sql => &["sql"],
            Language::Graphql => &["graphql", "gql"],
            Language::Vue => &["vue"],
            Language::Css => &["css", "scss", "sass"],
            Language::Bash => &["sh", "bash", "zsh"],
            Language::Kotlin => &["kt", "kts"],
            Language::CSharp => &["cs"],
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
            Language::Ruby => Ok(Box::new(RubyParser::new()?)),
            Language::Java => Ok(Box::new(JavaParser::new()?)),
            Language::Php => Ok(Box::new(PhpParser::new()?)),
            Language::Julia => Ok(Box::new(JuliaParser::new()?)),
            Language::Lua => Ok(Box::new(LuaParser::new()?)),
            Language::Swift => Ok(Box::new(SwiftParser::new()?)),
            Language::Zig => Ok(Box::new(ZigParser::new()?)),
            Language::Scala => Ok(Box::new(ScalaParser::new()?)),
            Language::Elixir => Ok(Box::new(ElixirParser::new()?)),
            Language::Dart => Ok(Box::new(DartParser::new()?)),
            Language::Haskell => Ok(Box::new(HaskellParser::new()?)),
            Language::C => Ok(Box::new(CParser::new()?)),
            Language::Cpp => Ok(Box::new(CppParser::new()?)),
            Language::Markdown => Ok(Box::new(MarkdownParser::new()?)),
            Language::Sql => Ok(Box::new(SqlParser::new()?)),
            Language::Graphql => Ok(Box::new(GraphqlParser::new()?)),
            Language::Vue => Ok(Box::new(VueParser::new()?)),
            Language::Css => Ok(Box::new(CssParser::new()?)),
            Language::Bash => Ok(Box::new(BashParser::new()?)),
            Language::Kotlin => Ok(Box::new(KotlinParser::new()?)),
            Language::CSharp => Ok(Box::new(CSharpParser::new()?)),
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
            ("rb", Language::Ruby),
            ("gemspec", Language::Ruby),
            ("java", Language::Java),
            ("php", Language::Php),
            ("jl", Language::Julia),
            ("lua", Language::Lua),
            ("swift", Language::Swift),
            ("zig", Language::Zig),
            ("scala", Language::Scala),
            ("ex", Language::Elixir),
            ("exs", Language::Elixir),
            ("dart", Language::Dart),
            ("hs", Language::Haskell),
            ("lhs", Language::Haskell),
            ("c", Language::C),
            ("h", Language::C),
            ("cpp", Language::Cpp),
            ("hpp", Language::Cpp),
            ("cc", Language::Cpp),
            ("cxx", Language::Cpp),
            ("hxx", Language::Cpp),
            ("hh", Language::Cpp),
            ("md", Language::Markdown),
            ("markdown", Language::Markdown),
            ("mdown", Language::Markdown),
            ("mkd", Language::Markdown),
            ("sql", Language::Sql),
            ("graphql", Language::Graphql),
            ("gql", Language::Graphql),
            ("vue", Language::Vue),
            ("css", Language::Css),
            ("scss", Language::Css),
            ("sass", Language::Css),
            ("sh", Language::Bash),
            ("bash", Language::Bash),
            ("zsh", Language::Bash),
            ("kt", Language::Kotlin),
            ("kts", Language::Kotlin),
            ("cs", Language::CSharp),
        ]
    }
}

/// Per-language indexing result
#[derive(Debug)]
pub struct IndexSummary {
    pub total_files: usize,
    pub total_symbols: usize,
    pub total_refs: usize,
    pub languages: Vec<LanguageSummary>,
}

#[derive(Debug)]
pub struct LanguageSummary {
    pub language: String,
    pub files: usize,
    pub symbols: usize,
    pub refs: usize,
}

impl IndexSummary {
    pub fn new() -> Self {
        Self {
            total_files: 0,
            total_symbols: 0,
            total_refs: 0,
            languages: Vec::new(),
        }
    }

    pub fn add_language(&mut self, language: String, files: usize, symbols: usize, refs: usize) {
        self.languages.push(LanguageSummary {
            language,
            files,
            symbols,
            refs,
        });
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
        Ok(Self {
            db,
            root,
            language,
            parser,
        })
    }

    /// Full project indexing (initial) — single language
    pub fn index_all(&mut self) -> Result<(usize, usize, usize), anyhow::Error> {
        info!(
            "Indexing project at {} ({})",
            self.root.display(),
            self.language.as_str()
        );

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
            files.len(),
            symbols,
            refs
        );

        Ok((files.len(), symbols, refs))
    }

    /// Multi-language project indexing — auto-detect language per file.
    ///
    /// Walks the project root, groups files by detected language, then
    /// indexes each group with the appropriate parser.
    pub fn index_all_multi(&mut self) -> Result<IndexSummary, anyhow::Error> {
        info!("Multi-language indexing at {}", self.root.display());

        // Collect all supported files, grouped by language
        let mut lang_files: std::collections::HashMap<Language, Vec<PathBuf>> =
            std::collections::HashMap::new();

        self.walk_all_supported(&self.root, &mut lang_files)?;

        let mut summary = IndexSummary::new();

        // Collect all files before consuming lang_files
        let all_files: Vec<PathBuf> = lang_files.values().flatten().cloned().collect();

        for (lang, files) in lang_files {
            info!("Indexing {} files as {}", files.len(), lang.as_str());

            // Create a temporary indexer for this language
            let mut lang_indexer = ProjectIndexer::new(self.db.clone(), self.root.clone(), lang)?;

            let mut symbols = 0;
            let mut refs = 0;

            for file in &files {
                match lang_indexer.index_file(file) {
                    Ok((s, r)) => {
                        symbols += s;
                        refs += r;
                    }
                    Err(e) => {
                        warn!("Failed to index {}: {}", file.display(), e);
                    }
                }
            }

            summary.add_language(lang.as_str().to_string(), files.len(), symbols, refs);
            summary.total_files += files.len();
            summary.total_symbols += symbols;
            summary.total_refs += refs;
        }

        // Clean up deleted files from DB
        self.clean_deleted_files(&all_files)?;

        info!(
            "Multi-language indexing complete: {} files, {} symbols, {} references across {} languages",
            summary.total_files,
            summary.total_symbols,
            summary.total_refs,
            summary.languages.len()
        );

        Ok(summary)
    }

    /// Index a single file (change detection + reparse)
    pub fn index_file(&mut self, path: &Path) -> Result<(usize, usize), anyhow::Error> {
        let relative = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Compute Blake3 hash
        let content = fs::read_to_string(path).context(format!("Read file: {}", path.display()))?;
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
                relative,
                &old_hash[..8],
                &hash[..8]
            );
        }

        // Store hash + checksum
        self.db.upsert_file(&relative, &hash, size, &modified)?;
        self.db.upsert_checksum(&relative, &hash, size)?;

        // Remove old symbols/refs for this file before re-indexing
        self.db.remove_file_symbols(&relative)?;

        // AST parsing
        let result = self.parser.parse(&content)?;

        // Store symbols
        let mut symbol_count = 0;
        for sym in &result.symbols {
            let qualified_name =
                SymbolRecord::build_qualified_name(&relative, &sym.name, &sym.parent);
            let token_count = estimate_symbol_tokens(&content, sym.start_line, sym.end_line);
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
                qualified_name,
                token_count,
            })?;
            symbol_count += 1;
        }

        // Store references
        let mut ref_count = 0;
        for ref_rec in &result.references {
            let conf = ref_rec.confidence();
            let is_dynamic = conf < 1.0;
            self.db
                .insert_reference(&crate::database::ReferenceRecord {
                    id: 0,
                    caller_file: relative.clone(),
                    callee_file: relative.clone(),
                    caller_symbol: ref_rec.caller_symbol.clone(),
                    callee_symbol: ref_rec.callee_symbol.clone(),
                    ref_kind: ref_rec.ref_kind.clone(),
                    line: ref_rec.line,
                    confidence: conf,
                    is_dynamic,
                })?;
            ref_count += 1;
        }

        // Store string literals (Cross-ref Engine)
        for lit in &result.string_literals {
            let _id = self.db.insert_string_literal(
                &relative,
                lit.line,
                lit.col,
                &lit.value,
                lit.is_symbol_like,
                &lit.context,
                lit.parent_fn.as_deref(),
            )?;
        }

        debug!("{}: {} symbols, {} refs, {} string literals", relative, symbol_count, ref_count, result.string_literals.len());
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
                    ".git",
                    "__pycache__",
                    ".venv",
                    "venv",
                    "node_modules",
                    ".mypy_cache",
                    ".tox",
                    ".eggs",
                    "dist",
                    "build",
                    ".next",
                    ".nuxt",
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

    /// Walk all supported languages — populate lang_files map
    fn walk_all_supported(
        &self,
        dir: &Path,
        lang_files: &mut std::collections::HashMap<Language, Vec<PathBuf>>,
    ) -> Result<(), anyhow::Error> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                let skip = [
                    ".git",
                    "__pycache__",
                    ".venv",
                    "venv",
                    "node_modules",
                    ".mypy_cache",
                    ".tox",
                    ".eggs",
                    "dist",
                    "build",
                    ".next",
                    ".nuxt",
                    "target",
                    ".shardindex",
                ];
                if path
                    .file_name()
                    .map_or(false, |n| skip.contains(&n.to_string_lossy().as_ref()))
                {
                    continue;
                }
                self.walk_all_supported(&path, lang_files)?;
            } else if let Some(lang) = path
                .extension()
                .and_then(|e| e.to_str())
                .and_then(|ext| Language::from_extension(&format!(".{}", ext)))
            {
                lang_files.entry(lang).or_insert_with(Vec::new).push(path);
            }
        }
        Ok(())
    }
}
