/// CLI — shardindex 명령어 (clap derive)
use clap::{Parser, Subcommand, ValueEnum};

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

        /// Language (default: auto — auto-detect all supported languages)
        #[arg(short, long, default_value = "auto")]
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

        /// Language (default: auto — auto-detect all supported languages)
        #[arg(short, long, default_value = "auto")]
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

        /// Language (default: auto — auto-detect all supported languages)
        #[arg(short, long, default_value = "auto")]
        language: String,

        #[arg(long, default_value = ".shardindex.db")]
        db: String,
    },

    /// Show index statistics
    Stats {
        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Output format (default: text)
        #[arg(long, value_enum, default_value_t = OutputFormat::default())]
        format: OutputFormat,
    },

    /// Search symbols (fuzzy matching + PageRank scoring)
    Search {
        /// Search query
        query: String,

        /// Database path
        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Symbol kind filter (e.g., "function", "class", "method")
        #[arg(long)]
        kind: Option<String>,

        /// Language filter (e.g., "python", "javascript")
        #[arg(long)]
        language: Option<String>,

        /// Minimum fuzzy score (0.0 - 1.0, default: 0.1)
        #[arg(long, default_value_t = 0.1)]
        min_score: f64,

        /// Maximum results (default: 50)
        #[arg(short, long, default_value_t = 50)]
        limit: usize,

        /// Use fast LIKE search instead of fuzzy matching
        #[arg(long)]
        like: bool,

        /// Output format (default: text)
        #[arg(long, value_enum, default_value_t = OutputFormat::default())]
        format: OutputFormat,
    },

    /// Show neighbors of a symbol
    Neighbors {
        /// Symbol name
        symbol: String,

        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Output format (default: text)
        #[arg(long, value_enum, default_value_t = OutputFormat::default())]
        format: OutputFormat,
    },

    /// Show impact analysis for a symbol
    Impact {
        /// Symbol name
        symbol: String,

        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Output format (default: text)
        #[arg(long, value_enum, default_value_t = OutputFormat::default())]
        format: OutputFormat,
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

        /// Output format (default: text)
        #[arg(long, value_enum, default_value_t = OutputFormat::default())]
        format: OutputFormat,
    },

    /// Override registry management
    Override {
        #[command(subcommand)]
        command: OverrideSubcommand,

        /// Database path
        #[arg(long, default_value = ".shardindex.db")]
        db: String,
    },

    /// Verify override coverage for symbols
    Verify {
        /// Symbol names to check
        symbols: Vec<String>,

        /// Database path
        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Language filter
        #[arg(long)]
        language: Option<String>,
    },

    /// Start MCP stdio server (for Hermes Agent / MCP clients)
    McpServer {
        /// Database path
        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Cache TTL in seconds (default: 300)
        #[arg(long, default_value_t = 300)]
        cache_ttl: u64,
    },

    /// Read a symbol with semantic compression
    Read {
        /// Symbol name (short name or qualified name)
        symbol: String,

        /// Database path
        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Project root (for reading source files)
        #[arg(long, default_value = ".")]
        root: String,

        /// Compression level: signature_only, critical_branches, full_body, or token budget (number)
        #[arg(long, default_value = "critical_branches")]
        compression: String,

        /// Output format (default: text)
        #[arg(long, value_enum, default_value_t = OutputFormat::default())]
        format: OutputFormat,
    },

    /// Deep impact analysis (transitive dependencies)
    ImpactDeep {
        /// Symbol name
        symbol: String,

        /// Database path
        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Max depth (default: 3)
        #[arg(short, long, default_value_t = 3)]
        depth: u8,

        /// Include test files
        #[arg(long)]
        include_tests: bool,

        /// Include dynamic references
        #[arg(long)]
        include_dynamic: bool,

        /// Output format (default: json)
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
    },

    /// Dead code verification (multi-stage)
    DeadCodeVerify {
        /// Symbol name
        symbol: String,

        /// Database path
        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Stages to run (default: all)
        #[arg(long, value_delimiter = ',')]
        stages: Option<Vec<String>>,

        /// Output format (default: json)
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
    },

    /// Cross-module move analysis
    CrossModuleMove {
        /// Symbol name
        symbol: String,

        /// Target module
        target_module: String,

        /// Database path
        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Auto-update imports (default: true)
        #[arg(long, default_value_t = true)]
        update_imports: bool,

        /// Dry run (default: true)
        #[arg(long, default_value_t = true)]
        dry_run: bool,

        /// Output format (default: json)
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
    },

    /// Signature migration compatibility check
    SignatureMigrationCheck {
        /// Symbol name
        symbol: String,

        /// New signature
        new_signature: String,

        /// Database path
        #[arg(long, default_value = ".shardindex.db")]
        db: String,

        /// Output format (default: json)
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
    },
}

#[derive(Subcommand, Debug)]
pub enum OverrideSubcommand {
    /// Add an override
    Add {
        /// Caller symbol
        caller: String,
        /// Callee symbol
        callee: String,
        /// Reference kind
        #[arg(long, default_value = "override")]
        kind: String,
        /// Confidence (0.0-1.0)
        #[arg(long, default_value_t = 0.9)]
        confidence: f64,
        /// Reason
        #[arg(long)]
        reason: Option<String>,
    },
    /// Remove an override by ID
    Remove {
        /// Override ID
        id: i64,
    },
    /// List all overrides
    List,
    /// Get overrides for a symbol
    ForSymbol {
        /// Symbol name
        symbol: String,
    },
}

/// Output format for query commands
#[derive(Clone, Copy, Debug, Default, PartialEq, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable text (default)
    #[default]
    Text,
    /// JSON format
    Json,
    /// TOON — Token-Oriented Object Notation (LLM-optimized format)
    Toon,
}
