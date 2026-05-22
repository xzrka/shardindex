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
//! ```

mod cli;
mod database;
mod graph;
mod indexer;
mod mcp;
mod watcher;

use std::sync::{Arc, Mutex};

use clap::Parser;
use cli::{Cli, Commands};
use database::IndexDb;
use graph::PageRankConfig;
use indexer::{Language, ProjectIndexer};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
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
        Commands::Stats { db } => cmd_stats(&db)?,
        Commands::Search { query, db } => cmd_search(&db, &query)?,
        Commands::Neighbors { symbol, db } => cmd_neighbors(&db, &symbol)?,
        Commands::Impact { symbol, db } => cmd_impact(&db, &symbol)?,
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
        } => cmd_rank(&db, damping, max_iter, tolerance, top)?,
    }

    Ok(())
}

fn parse_language(lang: &str) -> anyhow::Result<Language> {
    match lang.to_lowercase().as_str() {
        "python" | "py" => Ok(Language::Python),
        "javascript" | "js" => Ok(Language::JavaScript),
        _ => anyhow::bail!(
            "Unsupported language '{}'. Supported: python, javascript",
            lang
        ),
    }
}

fn cmd_init(root: &str, language: &str, db_path: &str) -> anyhow::Result<()> {
    let root_path = std::fs::canonicalize(root)?;
    let lang = parse_language(language)?;
    info!(
        "Initializing ShardIndex for {} ({})",
        root_path.display(),
        lang.as_str()
    );

    let db = IndexDb::open(db_path)?;
    let mut indexer = ProjectIndexer::new(db, root_path, lang)?;
    let (files, symbols, refs) = indexer.index_all()?;

    println!("\n✅ ShardIndex initialized");
    println!("   Files:      {}", files);
    println!("   Symbols:    {}", symbols);
    println!("   References: {}", refs);
    println!("   Language:   {}", lang.as_str());
    println!("   Database:   {}", db_path);

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
    info!(
        "Starting ShardIndex daemon at {} (watch: {}, lang: {})",
        listen,
        if poll_interval == 0 {
            "event-driven (notify)".to_string()
        } else {
            format!("event-driven + {}s polling fallback", poll_interval)
        },
        lang.as_str()
    );

    // 초기 인덱싱
    let db = IndexDb::open(db_path)?;
    {
        let mut indexer = ProjectIndexer::new(db, root_path.clone(), lang)?;
        let (files, symbols, refs) = indexer.index_all()?;
        info!(
            "Initial index: {} files, {} symbols, {} refs",
            files, symbols, refs
        );
    }

    // DB를 MCP 서버와 watcher가 공유 (WAL mode이므로 동시 읽기/쓰기 가능)
    let db = IndexDb::open(db_path)?;
    let state = mcp::ServerState {
        db: Arc::new(Mutex::new(db)),
    };

    // ── Event-driven file watcher (notify crate) ──
    let (notify_watcher, debouncer_handle) =
        watcher::start_watcher(&root_path, db_path, lang)?;

    // ── Optional polling fallback (for systems without inotify) ──
    if poll_interval > 0 {
        let root_poll = root_path.clone();
        let db_path_poll = db_path.to_string();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(poll_interval));
            loop {
                interval.tick().await;
                if let Err(e) = watcher::poll_fallback(&db_path_poll, &root_poll, lang) {
                    tracing::warn!("Poll fallback error: {}", e);
                }
            }
        });
    }

    // ── MCP 서버 실행 (keep watcher alive for the lifetime of the daemon) ──
    // NOTE: notify_watcher and debouncer_handle must stay alive.
    // We drop them after serve() returns (graceful shutdown).
    mcp::serve(state, listen).await;

    // Graceful shutdown
    drop(notify_watcher);
    debouncer_handle.abort();

    Ok(())
}

fn cmd_reindex(root: &str, language: &str, db_path: &str) -> anyhow::Result<()> {
    let root_path = std::fs::canonicalize(root)?;
    let lang = parse_language(language)?;
    let db = IndexDb::open(db_path)?;
    let mut indexer = ProjectIndexer::new(db, root_path, lang)?;
    let (files, symbols, refs) = indexer.index_all()?;

    println!(
        "Re-indexed: {} files, {} symbols, {} refs",
        files, symbols, refs
    );
    Ok(())
}

fn cmd_stats(db_path: &str) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;
    let (files, symbols, refs) = db.stats()?;

    println!("📊 ShardIndex Statistics");
    println!("   Files:      {}", files);
    println!("   Symbols:    {}", symbols);
    println!("   References: {}", refs);
    Ok(())
}

fn cmd_search(db_path: &str, query: &str) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;
    let results = db.search_symbol_ranked(query)?;

    println!(
        "🔍 Search '{}' — {} results",
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
    Ok(())
}

fn cmd_neighbors(db_path: &str, symbol: &str) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;
    let neighbors = db.neighbors(symbol)?;

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
    Ok(())
}

fn cmd_impact(db_path: &str, symbol: &str) -> anyhow::Result<()> {
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
) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;

    let config = PageRankConfig {
        damping,
        max_iterations: max_iter,
        tolerance,
    };

    println!("📊 Computing symbol ranking...");
    println!(
        "   Config: damping={}, max_iter={}, tolerance={}\n",
        damping, max_iter, tolerance
    );

    graph::compute_and_store_ranks(&db, Some(&config))?;

    // Top-N 출력
    let ranked = db.ranked_symbols(top)?;

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

    Ok(())
}
