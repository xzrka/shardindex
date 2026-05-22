/// CLI — shardindex 명령어 (clap derive)
use clap::{Parser, Subcommand};

/// ShardIndex — Semantic code graph index
#[derive(Parser, Debug)]
#[command(name = "shardindex", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize index for a project
    Init {
        /// Project root directory (default: current directory)
        #[arg(short, long, default_value = ".")]
        path: String,

        /// Language (default: python)
        #[arg(short, long, default_value = "python")]
        language: String,

        /// Database path (default: .shardindex.db)
        #[arg(long, default_value = ".shardindex.db")]
        db: String,
    },

    /// Start MCP daemon (file watch + JSON-RPC server)
    Daemon {
        /// Project root directory
        #[arg(short, long, default_value = ".")]
        path: String,

        /// Database path
        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Language (default: python)
        #[arg(short, long, default_value = "python")]
        language: String,

        /// Listen address
        #[arg(long, default_value = "127.0.0.1:3999")]
        listen: String,

        /// Poll interval in seconds (fallback for systems without inotify)
        #[arg(long, default_value_t = 2)]
        poll_interval: u64,
    },

    /// Re-index all files
    Reindex {
        #[arg(short, long, default_value = ".")]
        path: String,

        /// Language (default: python)
        #[arg(short, long, default_value = "python")]
        language: String,

        #[arg(long, default_value = ".shardindex.db")]
        db: String,
    },

    /// Show index statistics
    Stats {
        #[arg(long, default_value = ".shardindex.db")]
        db: String,
    },

    /// Search symbols
    Search {
        /// Search query
        query: String,

        #[arg(long, default_value = ".shardindex.db")]
        db: String,
    },

    /// Show neighbors of a symbol
    Neighbors {
        /// Symbol name
        symbol: String,

        #[arg(long, default_value = ".shardindex.db")]
        db: String,
    },

    /// Show impact analysis for a symbol
    Impact {
        /// Symbol name
        symbol: String,

        #[arg(long, default_value = ".shardindex.db")]
        db: String,
    },

    /// Generate DOT graph
    Graph {
        /// Symbol name (optional — full graph if omitted)
        symbol: Option<String>,

        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Output file (default: stdout)
        #[arg(long)]
        output: Option<String>,
    },

    /// Compute symbol ranking (PageRank + degree centrality)
    Rank {
        /// Database path
        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Damping factor (default: 0.85)
        #[arg(long, default_value_t = 0.85)]
        damping: f64,

        /// Maximum iterations (default: 100)
        #[arg(long, default_value_t = 100)]
        max_iter: usize,

        /// Convergence tolerance (default: 1e-6)
        #[arg(long, default_value_t = 1e-6)]
        tolerance: f64,

        /// Show top N ranked symbols (default: 20)
        #[arg(short, long, default_value_t = 20)]
        top: usize,
    },
}
