//! ShardIndex — Semantic code graph index
//!
//! AST-powered middleware for AI agents. Exposes code structure via MCP/JSON-RPC.
//!
//! # Usage
//!
//! ```bash
//! shardindex init .                    # Index a Python project
//! shardindex init . -l javascript      # Index a JS project
//! shardindex daemon                    # Start MCP server + file watcher
//! shardindex search my_function        # Search symbols
//! shardindex impact my_function        # Impact analysis
//! shardindex rank                      # Compute symbol ranking
mod agent_cache;
mod cli;
mod config;
mod daemon;
mod database;
mod format;
mod graph;
mod indexer;
mod integrity;
mod mcp;
mod recovery;
mod search;
mod token_estimation;
mod watcher;

use std::sync::{Arc, Mutex};

use clap::Parser;
use cli::{Cli, Commands, OutputFormat, OverrideSubcommand};
use database::IndexDb;
use graph::PageRankConfig;
use indexer::{IndexSummary, Language, ProjectIndexer};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("shardindex=debug,tower_http=info")
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init {
            path,
            language,
            db,
        } => cmd_init(&path, &language, &db)?,
        Commands::Daemon {
            path,
            db,
            listen,
            poll_interval,
            language,
        } => cmd_daemon(&path, &db, &listen, poll_interval, &language).await?,
        Commands::Reindex { path, language, db } => cmd_reindex(&path, &language, &db)?,
        Commands::Stats { db, format } => cmd_stats(&db, format)?,
        Commands::Search { query, db, kind, language, min_score, limit, like, format } => {
            cmd_search(&db, &query, kind, language, min_score, limit, like, format)?
        }
        Commands::Neighbors { symbol, db, format } => cmd_neighbors(&db, &symbol, format)?,
        Commands::Impact { symbol, db, format } => cmd_impact(&db, &symbol, format)?,
        Commands::Graph {
            symbol,
            db,
            output,
        } => cmd_graph(&db, symbol.as_deref(), output.as_deref())?,
        Commands::Rank {
            db,
            damping,
            max_iter,
            tolerance,
            top,
            format,
        } => cmd_rank(&db, damping, max_iter, tolerance, top, format)?,
        Commands::Override { command, db } => cmd_override(&db, command)?,
        Commands::Verify { symbols, db, language } => cmd_verify(&db, &symbols, language.as_deref())?,
        Commands::McpServer { db, cache_ttl } => {
            mcp::stdio::run(&db, cache_ttl)?;
        }
    }

    Ok(())
}

fn parse_language(lang: &str) -> anyhow::Result<Option<Language>> {
    match lang.to_lowercase().as_str() {
        "auto" => Ok(None),
        "python" | "py" => Ok(Some(Language::Python)),
        "javascript" | "js" => Ok(Some(Language::JavaScript)),
        "typescript" | "ts" => Ok(Some(Language::TypeScript)),
        "rust" | "rs" => Ok(Some(Language::Rust)),
        "go" => Ok(Some(Language::Go)),
        "ruby" | "rb" => Ok(Some(Language::Ruby)),
        "java" => Ok(Some(Language::Java)),
        "php" => Ok(Some(Language::Php)),
        "julia" | "jl" => Ok(Some(Language::Julia)),
        "lua" => Ok(Some(Language::Lua)),
        "swift" => Ok(Some(Language::Swift)),
        "zig" => Ok(Some(Language::Zig)),
        "scala" => Ok(Some(Language::Scala)),
        "elixir" | "ex" | "exs" => Ok(Some(Language::Elixir)),
        "dart" => Ok(Some(Language::Dart)),
        "haskell" | "hs" => Ok(Some(Language::Haskell)),
        "c" => Ok(Some(Language::C)),
        "cpp" | "c++" | "cc" | "cxx" => Ok(Some(Language::Cpp)),
        _ => anyhow::bail!(
            "Unsupported language '{}'. Supported: auto, python, javascript, typescript, rust, go, ruby, java, php, julia, lua, swift, zig, scala, elixir, dart, haskell, c, cpp",
            lang
        ),
    }
}

fn cmd_init(root: &str, language: &str, db_path: &str) -> anyhow::Result<()> {
    let root_path = std::fs::canonicalize(root)?;
    let lang = parse_language(language)?;

    let db = IndexDb::open(db_path)?;

    if let Some(l) = lang {
        // ── Single-language mode ──
        info!(
            "Initializing ShardIndex for {} ({})",
            root_path.display(),
            l.as_str()
        );

        let mut indexer = ProjectIndexer::new(db, root_path, l)?;
        let (files, symbols, refs) = indexer.index_all()?;

        println!("\n✅ ShardIndex initialized");
        println!("   Files:      {}", files);
        println!("   Symbols:    {}", symbols);
        println!("   References: {}", refs);
        println!("   Language:   {}", l.as_str());
        println!("   Database:   {}", db_path);
    } else {
        // ── Multi-language (auto) mode ──
        // Use a dummy language to create the indexer — index_all_multi ignores self.language
        let dummy_lang = Language::Python;
        info!(
            "Initializing ShardIndex for {} (auto-detect all languages)",
            root_path.display()
        );

        let mut indexer = ProjectIndexer::new(db, root_path, dummy_lang)?;
        let summary: IndexSummary = indexer.index_all_multi()?;

        println!("\n✅ ShardIndex initialized (multi-language)");
        println!("   Total files:      {}", summary.total_files);
        println!("   Total symbols:    {}", summary.total_symbols);
        println!("   Total references: {}", summary.total_refs);
        println!("   Languages found:  {}", summary.languages.len());
        println!();

        for ls in &summary.languages {
            println!(
                "   └─ {:<14} {} files, {} symbols, {} refs",
                ls.language, ls.files, ls.symbols, ls.refs
            );
        }
        println!();
        println!("   Database:   {}", db_path);
    }

    Ok(())
}

async fn cmd_daemon(
    root: &str,
    db_path: &str,
    listen: &str,
    poll_interval: u64,
    language: &str,
) -> anyhow::Result<()> {
    let root_path = std::fs::canonicalize(root)?;
    let lang = parse_language(language)?;
    let lang_label = lang
        .as_ref()
        .map(|l| l.as_str().to_string())
        .unwrap_or_else(|| "auto (multi-language)".to_string());

    info!(
        "Starting ShardIndex daemon at {} (watch: {}, lang: {})",
        listen,
        if poll_interval == 0 {
            "event-driven (notify)".to_string()
        } else {
            format!("event-driven + {}s polling fallback", poll_interval)
        },
        lang_label
    );

    // 초기 인덱싱
    let db = IndexDb::open(db_path)?;
    {
        if let Some(l) = lang {
            let mut indexer = ProjectIndexer::new(db.clone(), root_path.clone(), l)?;
            let (files, symbols, refs) = indexer.index_all()?;
            info!(
                "Initial index: {} files, {} symbols, {} refs",
                files, symbols, refs
            );
        } else {
            // Multi-language auto-detect
            let dummy_lang = Language::Python;
            let mut indexer = ProjectIndexer::new(db.clone(), root_path.clone(), dummy_lang)?;
            let summary = indexer.index_all_multi()?;
            info!(
                "Initial multi-lang index: {} files, {} symbols, {} refs across {} languages",
                summary.total_files, summary.total_symbols, summary.total_refs, summary.languages.len()
            );
        }
    }

    // ── Load config ──
    let config = config::load_config(&root_path).unwrap_or_default();

    // DB를 MCP 서버와 watcher가 공유 (WAL mode이므로 동시 읽기/쓰기 가능)
    let db = IndexDb::open(db_path)?;
    // AgentCache owns its own DB handle (separate connection)
    let cache_db = IndexDb::open(db_path)?;
    let cache = agent_cache::AgentCache::new(cache_db, 300); // 5min default TTL
    let state = mcp::ServerState {
        db: Arc::new(Mutex::new(db)),
        cache: Arc::new(cache),
    };

    // ── Multi-language event-driven file watcher ──
    let (file_watcher, debouncer_handle) =
        watcher::FileWatcher::new(root_path.clone(), config.clone()).start()?;

    // ── Optional polling fallback (for systems without inotify) ──
    if poll_interval > 0 {
        let root_poll = root_path.clone();
        let db_path_poll = db_path.to_string();
        let config_poll = config.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(poll_interval));
            loop {
                interval.tick().await;
                if let Err(e) = watcher::poll_fallback(&db_path_poll, &root_poll, &config_poll) {
                    tracing::warn!("Poll fallback error: {}", e);
                }
            }
        });
    }

    // ── MCP 서버 실행 (keep watcher alive for the lifetime of the daemon) ──
    // NOTE: file_watcher and debouncer_handle must stay alive.
    // We drop them after serve() returns (graceful shutdown).
    mcp::serve(state, listen).await;

    // Graceful shutdown
    drop(file_watcher);
    debouncer_handle.abort();

    Ok(())
}

fn cmd_reindex(root: &str, language: &str, db_path: &str) -> anyhow::Result<()> {
    let root_path = std::fs::canonicalize(root)?;
    let lang = parse_language(language)?;
    let db = IndexDb::open(db_path)?;

    if let Some(l) = lang {
        let mut indexer = ProjectIndexer::new(db, root_path, l)?;
        let (files, symbols, refs) = indexer.index_all()?;
        println!(
            "Re-indexed: {} files, {} symbols, {} refs",
            files, symbols, refs
        );
    } else {
        let dummy_lang = Language::Python;
        let mut indexer = ProjectIndexer::new(db, root_path, dummy_lang)?;
        let summary = indexer.index_all_multi()?;
        println!(
            "Re-indexed (multi-language): {} files, {} symbols, {} refs across {} languages",
            summary.total_files, summary.total_symbols, summary.total_refs, summary.languages.len()
        );
    }
    Ok(())
}

fn cmd_stats(db_path: &str, output_format: OutputFormat) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;
    let (files, symbols, refs) = db.stats()?;

    if output_format == OutputFormat::Text {
        println!("📊 ShardIndex Statistics");
        println!("   Files:      {}", files);
        println!("   Symbols:    {}", symbols);
        println!("   References: {}", refs);
    } else {
        let json = serde_json::json!({
            "files": files,
            "symbols": symbols,
            "references": refs
        });
        let output = match output_format {
            OutputFormat::Json => serde_json::to_string_pretty(&json)?,
            OutputFormat::Toon => format::toon::to_toon(&json, false, true),
            OutputFormat::Text => unreachable!(),
        };
        println!("{}", output);
    }
    Ok(())
}

fn cmd_search(
    db_path: &str,
    query: &str,
    kind: Option<String>,
    language: Option<String>,
    min_score: f64,
    limit: usize,
    like_mode: bool,
    output_format: OutputFormat,
) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;

    // language → file extension
    let extension_filter = language.as_ref().map(|lang| {
        match lang.to_lowercase().as_str() {
            "python" => "py",
            "javascript" | "js" => "js",
            "typescript" | "ts" => "ts",
            "rust" | "rs" => "rs",
            "go" => "go",
            "ruby" | "rb" => "rb",
            "java" => "java",
            "php" => "php",
            "julia" | "jl" => "jl",
            "lua" => "lua",
            "swift" => "swift",
            "zig" => "zig",
            "scala" => "scala",
            "elixir" | "ex" => "ex",
            "dart" => "dart",
            "haskell" | "hs" => "hs",
            "c" => "c",
            "cpp" | "c++" => "cpp",
            _ => lang.as_str(),
        }
    });

    if like_mode {
        let results = db.search_symbol_ranked(query)?;

        if output_format == OutputFormat::Text {
            println!(
                "🔍 LIKE Search '{}' — {} results",
                query,
                results.len()
            );
            for (sym, rank) in &results {
                let rank_str = match rank {
                    Some(r) => format!(" [PR: {:.4}]", r),
                    None => String::from(""),
                };
                println!(
                    "  {}:{} {} [{}]{}{}",
                    sym.file_path,
                    sym.start_line,
                    sym.name,
                    sym.kind,
                    rank_str,
                    sym.signature.as_deref().unwrap_or("")
                );
            }
        } else {
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|(sym, rank)| {
                    serde_json::json!({
                        "name": sym.name,
                        "qualified_name": sym.name,
                        "file": sym.file_path,
                        "line": sym.start_line,
                        "kind": sym.kind,
                        "signature": sym.signature,
                        "page_rank": rank
                    })
                })
                .collect();
            let json = serde_json::json!({
                "query": query,
                "mode": "like",
                "count": items.len(),
                "results": items
            });
            print_formatted(&json, output_format);
        }
    } else {
        let config = search::SearchConfig {
            kind_filter: kind.clone(),
            language_filter: language.clone(),
            min_score,
            limit,
            ..Default::default()
        };

        let results =
            search::advanced_search(&db, query, extension_filter.as_deref(), &config)?;

        if output_format == OutputFormat::Text {
            println!(
                "🔍 Fuzzy Search '{}' (min_score={}, limit={}) — {} results",
                query,
                min_score,
                limit,
                results.len()
            );
            for result in &results {
                let rank_str = match result.page_rank {
                    Some(r) => format!(" [PR: {:.4}]", r),
                    None => String::from(""),
                };
                println!(
                    "  {}:{} {} [{}] score={:.3} fuzzy={:.3}{}{}",
                    result.file_path,
                    result.start_line,
                    result.name,
                    result.kind,
                    result.score,
                    result.fuzzy_score,
                    rank_str,
                    result.signature.as_deref().unwrap_or("")
                );
            }
        } else {
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "name": r.name,
                        "qualified_name": r.name,
                        "file": r.file_path,
                        "line": r.start_line,
                        "kind": r.kind,
                        "signature": r.signature,
                        "score": r.score,
                        "fuzzy_score": r.fuzzy_score,
                        "page_rank": r.page_rank
                    })
                })
                .collect();
            let json = serde_json::json!({
                "query": query,
                "mode": "fuzzy",
                "min_score": min_score,
                "count": items.len(),
                "results": items
            });
            print_formatted(&json, output_format);
        }
    }

    Ok(())
}

fn cmd_neighbors(db_path: &str, symbol: &str, output_format: OutputFormat) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;
    let neighbors = db.neighbors(symbol)?;

    if output_format == OutputFormat::Text {
        println!(
            "🔗 Neighbors of '{}' — {} refs",
            symbol,
            neighbors.len()
        );
        for ref_rec in &neighbors {
            println!(
                "  {}:{} {} → {} [{}]",
                ref_rec.caller_file,
                ref_rec.line,
                ref_rec.caller_symbol.as_deref().unwrap_or("?"),
                ref_rec.callee_symbol,
                ref_rec.ref_kind
            );
        }
    } else {
        let callers: Vec<serde_json::Value> = neighbors
            .iter()
            .filter(|r| r.callee_symbol == symbol)
            .map(|r| {
                serde_json::json!({
                    "symbol": r.caller_symbol.as_deref().unwrap_or("?"),
                    "file": r.caller_file,
                    "line": r.line,
                    "confidence": r.confidence
                })
            })
            .collect();
        let callees: Vec<serde_json::Value> = neighbors
            .iter()
            .filter(|r| r.caller_symbol.as_deref() == Some(&symbol))
            .map(|r| {
                serde_json::json!({
                    "symbol": r.callee_symbol,
                    "file": r.caller_file,
                    "line": r.line,
                    "confidence": r.confidence
                })
            })
            .collect();
        let json = serde_json::json!({
            "center": symbol,
            "callers": callers,
            "callees": callees
        });
        print_formatted(&json, output_format);
    }
    Ok(())
}

fn cmd_impact(db_path: &str, symbol: &str, output_format: OutputFormat) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;
    let (callers, refs) = db.impact_ranked(symbol)?;

    // 심볼 자체의 랭킹
    let own_rank = db.symbol_rank(symbol);
    let own_rank_str = match &own_rank {
        Some(r) => format!(
            " [PR: {:.4}  in:{}  out:{}]",
            r.page_rank, r.in_degree, r.out_degree
        ),
        None => String::from(" (no rank computed — run 'rank' first)"),
    };

    if output_format == OutputFormat::Text {
        println!(
            "💥 Impact analysis for '{}'{} — {} callers, {} refs",
            symbol, own_rank_str, callers.len(), refs.len()
        );

        if !callers.is_empty() {
            println!("\n  Impacted callers (sorted by PageRank):");
            for (sym, rank) in &callers {
                let rank_str = match rank {
                    Some(r) => format!(" [PR: {:.4}]", r),
                    None => String::from(""),
                };
                println!(
                    "    {}:{} {} [{}]{}",
                    sym.file_path, sym.start_line, sym.name, sym.kind, rank_str
                );
            }
        }
    } else {
        let items: Vec<serde_json::Value> = callers
            .iter()
            .map(|(sym, rank)| {
                serde_json::json!({
                    "qualified_name": sym.name,
                    "name": sym.name,
                    "file": sym.file_path,
                    "line": sym.start_line,
                    "kind": sym.kind,
                    "relationship": "caller",
                    "confidence": 0.95,
                    "page_rank": rank
                })
            })
            .collect();
        let json = serde_json::json!({
            "target": symbol,
            "impacted_symbols": items,
            "impacted_count": items.len()
        });
        print_formatted(&json, output_format);
    }

    Ok(())
}

fn cmd_graph(
    db_path: &str,
    symbol: Option<&str>,
    output: Option<&str>,
) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;
    let dot = if let Some(sym) = symbol {
        graph::symbol_dot(&db, sym)?
    } else {
        graph::full_dot(&db)?
    };

    if let Some(path) = output {
        std::fs::write(path, &dot)?;
        println!("Graph written to {}", path);
    } else {
        println!("{}", dot);
    }

    Ok(())
}

fn cmd_rank(
    db_path: &str,
    damping: f64,
    max_iter: usize,
    tolerance: f64,
    top: usize,
    output_format: OutputFormat,
) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;

    let config = PageRankConfig {
        damping,
        max_iterations: max_iter,
        tolerance,
    };

    if output_format == OutputFormat::Text {
        println!("📊 Computing symbol ranking...");
        println!(
            "   Config: damping={}, max_iter={}, tolerance={}\n",
            damping, max_iter, tolerance
        );
    }

    graph::compute_and_store_ranks(&db, Some(&config))?;

    // Top-N 출력
    let ranked = db.ranked_symbols(top)?;

    if output_format == OutputFormat::Text {
        println!("🏆 Top {} Ranked Symbols ({} total)", top, ranked.len());
        println!();

        for (i, rank) in ranked.iter().enumerate() {
            println!(
                "  {}. {}  [PR: {:.6}  in: {}  out: {}]",
                i + 1,
                rank.symbol_name,
                rank.page_rank,
                rank.in_degree,
                rank.out_degree
            );
        }
    } else {
        let items: Vec<serde_json::Value> = ranked
            .iter()
            .enumerate()
            .map(|(i, rank)| {
                serde_json::json!({
                    "rank": i + 1,
                    "symbol": rank.symbol_name,
                    "page_rank": rank.page_rank,
                    "in_degree": rank.in_degree,
                    "out_degree": rank.out_degree
                })
            })
            .collect();
        let json = serde_json::json!({
            "top": top,
            "total": items.len(),
            "rankings": items
        });
        print_formatted(&json, output_format);
    }

    Ok(())
}

/// Helper: format JSON value as either JSON or Smart YAML
fn print_formatted(json: &serde_json::Value, output_format: OutputFormat) {
    let output = match output_format {
        OutputFormat::Json => serde_json::to_string_pretty(json).unwrap_or_default(),
        OutputFormat::Toon => format::toon::to_toon(json, false, true),
        OutputFormat::Text => unreachable!(),
    };
    println!("{}", output);
}

fn cmd_override(db_path: &str, command: OverrideSubcommand) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;

    match command {
        OverrideSubcommand::Add {
            caller,
            callee,
            kind,
            confidence,
            reason,
        } => {
            let id = db.insert_override(
                &caller,
                &callee,
                &kind,
                confidence,
                reason.as_deref().unwrap_or(""),
            )?;
            println!("✅ Override added (id={})", id);
        }
        OverrideSubcommand::Remove { id } => {
            db.remove_override(id)?;
            println!("🗑️  Override {} removed", id);
        }
        OverrideSubcommand::List => {
            let overrides = db.list_overrides()?;
            if overrides.is_empty() {
                println!("No overrides registered.");
            } else {
                println!("📋 Overrides ({} total):", overrides.len());
                for ov in &overrides {
                    println!(
                        "  [{}] {} → {} [{}] conf={:.2} created={}",
                        ov.id,
                        ov.caller_symbol,
                        ov.callee_symbol,
                        ov.ref_kind,
                        ov.confidence,
                        ov.created_at
                    );
                }
            }
        }
        OverrideSubcommand::ForSymbol { symbol } => {
            let overrides = db.overrides_for_symbol(&symbol)?;
            println!("Overrides for '{}':", symbol);
            for ov in &overrides {
                println!(
                    "  [{}] {} → {} [{}] conf={:.2}",
                    ov.id,
                    ov.caller_symbol,
                    ov.callee_symbol,
                    ov.ref_kind,
                    ov.confidence
                );
            }
        }
    }

    Ok(())
}

fn cmd_verify(
    db_path: &str,
    symbols: &[String],
    language: Option<&str>,
) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;

    println!("🔍 Verifying override coverage for {} symbols...", symbols.len());

    let mut unresolved = Vec::new();
    for sym in symbols {
        let overrides = db.overrides_for_symbol(sym)?;
        if overrides.is_empty() {
            unresolved.push(sym);
            println!("  ❌ {} — NO overrides", sym);
        } else {
            println!("  ✅ {} — {} override(s)", sym, overrides.len());
        }
    }

    if unresolved.is_empty() {
        println!("\n🎉 All symbols have overrides!");
    } else {
        println!(
            "\n⚠️  {} symbols unresolved: {}",
            unresolved.len(),
            unresolved.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        );
    }

    Ok(())
}
