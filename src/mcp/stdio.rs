//! Stdio MCP server — proper MCP protocol over stdin/stdout
//!
//! Implements the Model Context Protocol (MCP) for Hermes Agent integration:
//! - initialize / initialized handshake
//! - tools/list — discover available tools
//! - tools/call — invoke tools with arguments
//!
//! Protocol: line-delimited JSON-RPC 2.0 over stdin/stdout.

use crate::agent_cache::AgentCache;
use crate::database::IndexDb;
use crate::format::toon;
use crate::search::{self, SearchConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;

// ─────────────────────────────────────────────────────────────────────
// MCP Protocol types
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct McpRequest {
    jsonrpc: String,
    method: String,
    params: serde_json::Value,
    id: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct McpError {
    code: i32,
    message: String,
}

#[derive(Debug, Serialize)]
struct McpResponse {
    jsonrpc: String,
    result: Option<serde_json::Value>,
    error: Option<McpError>,
    id: Option<serde_json::Value>,
}

impl McpResponse {
    fn ok(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Build an MCP response with Smart YAML content when format is requested.
    /// Returns either a standard JSON result or a text content with Smart YAML.
    fn ok_with_format(
        id: Option<serde_json::Value>,
        result: &serde_json::Value,
        format_hint: Option<&str>,
    ) -> Self {
        let content_format = format_hint.unwrap_or("json");

        match content_format {
            "toon" | "toon-compact" => {
                let is_compact = content_format == "toon-compact";
                let toon_text = toon::to_toon(result, is_compact, false);
                Self {
                    jsonrpc: "2.0".into(),
                    result: Some(serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": toon_text
                        }],
                        "format": content_format
                    })),
                    error: None,
                    id,
                }
            }
            _ => Self::ok(id, result.clone()),
        }
    }

    fn err(id: Option<serde_json::Value>, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(McpError { code, message: message.into() }),
            id,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Server state
// ─────────────────────────────────────────────────────────────────────

pub struct StdioMcpServer {
    db: IndexDb,
    cache: AgentCache,
    /// Preferred output format negotiated during initialize.
    preferred_format: std::sync::Mutex<Option<String>>,
}

/// Start stdio MCP server. Blocks until stdin closes.
pub fn run(db_path: &str, cache_ttl: u64) -> anyhow::Result<()> {
    let db = IndexDb::open(db_path)?;
    let cache_db = IndexDb::open(db_path)?;
    let cache = AgentCache::new(cache_db, cache_ttl);

    let server = StdioMcpServer {
        db,
        cache,
        preferred_format: std::sync::Mutex::new(None),
    };

    let stdin = io::stdin();
    let stdout = io::stdout();
    let reader = BufReader::new(stdin);
    let mut writer = BufWriter::new(stdout);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break, // stdin closed
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let request: McpRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = McpResponse::err(None, -32700, &format!("Parse error: {}", e));
                let _ = writeln!(writer, "{}", serde_json::to_string(&resp).unwrap());
                let _ = writer.flush();
                continue;
            }
        };

        if let Some(response) = server.handle_request(request) {
            let _ = writeln!(writer, "{}", serde_json::to_string(&response).unwrap());
            let _ = writer.flush();
        }
    }

    Ok(())
}

impl StdioMcpServer {
    fn handle_request(&self, req: McpRequest) -> Option<McpResponse> {
        match req.method.as_str() {
            "initialize" => Some(self.handle_initialize(req.params, req.id)),
            "notifications/initialized" => {
                // Notification — no response per MCP spec
                None
            }
            "tools/list" => Some(self.handle_tools_list(req.params, req.id)),
            "tools/call" => Some(self.handle_tools_call(req.params, req.id)),
            "ping" => Some(McpResponse::ok(req.id, serde_json::json!({}))),
            "logging/message" => {
                // Notification — ignore
                None
            }
            unknown => Some(McpResponse::err(req.id, -32601, &format!("Method not found: {}", unknown))),
        }
    }

    fn handle_initialize(
        &self,
        params: serde_json::Value,
        id: Option<serde_json::Value>,
    ) -> McpResponse {
        // Negotiate preferred output format from client capabilities
        let preferred_format = params
            .get("capabilities")
            .and_then(|c| c.get("preferredFormat"))
            .and_then(|v| v.as_str())
            .map(String::from);

        if let Some(ref fmt) = preferred_format {
            let mut pf = self.preferred_format.lock().unwrap();
            *pf = Some(fmt.clone());
        }

        McpResponse::ok(
            id,
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    },
                    "experimental": {
                        "toon": {
                            "supported": true,
                            "description": "TOON output format for LLM-friendly responses"
                        }
                    }
                },
                "serverInfo": {
                    "name": "shardindex",
                    "version": "0.1.0"
                }
            }),
        )
    }

    fn handle_tools_list(
        &self,
        _params: serde_json::Value,
        id: Option<serde_json::Value>,
    ) -> McpResponse {
        let tools = serde_json::json!([
            {
                "name": "stats",
                "description": "Show index statistics (files, symbols, references count).",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "search",
                "description": "Search symbols by name with fuzzy matching and PageRank scoring. Supports kind filter, language filter, min_score, limit, and use_like mode.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query string"
                        },
                        "kind": {
                            "type": "string",
                            "description": "Symbol kind filter (e.g., function, class, method)"
                        },
                        "language": {
                            "type": "string",
                            "description": "Language filter (e.g., python, javascript, rust)"
                        },
                        "min_score": {
                            "type": "number",
                            "description": "Minimum fuzzy score (0.0-1.0, default: 0.1)"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum results (default: 50)"
                        },
                        "use_like": {
                            "type": "boolean",
                            "description": "Use fast LIKE search instead of fuzzy matching"
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "read",
                "description": "List all symbols in a file. Returns symbol names, kinds, line ranges, and signatures.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file": {
                            "type": "string",
                            "description": "File path to read symbols from"
                        }
                    },
                    "required": ["file"]
                }
            },
            {
                "name": "neighbors",
                "description": "Show direct references (callers and callees) for a symbol.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "Symbol name to find neighbors for"
                        }
                    },
                    "required": ["symbol"]
                }
            },
            {
                "name": "impact",
                "description": "Impact analysis — find all symbols affected by changing a given symbol. Returns impacted callers and references.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "Symbol name to analyze impact for"
                        }
                    },
                    "required": ["symbol"]
                }
            },
            {
                "name": "edit_plan",
                "description": "Pre-edit impact analysis. Analyze the effect of proposed changes (rename, add_param, remove_param, change_return) on dependent symbols.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "Symbol name to analyze"
                        },
                        "changes": {
                            "type": "array",
                            "description": "List of changes, each with type and details",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "type": {
                                        "type": "string",
                                        "enum": ["rename", "add_param", "remove_param", "change_return"]
                                    },
                                    "details": {
                                        "type": "object"
                                    }
                                }
                            }
                        },
                        "depth": {
                            "type": "integer",
                            "description": "Analysis depth (default: 1)"
                        }
                    },
                    "required": ["symbol"]
                }
            },
            {
                "name": "verify",
                "description": "Verify file integrity using BLAKE3 checksum. Compares stored hash with disk hash.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "File path to verify"
                        }
                    },
                    "required": ["file_path"]
                }
            }
        ]);
        McpResponse::ok(id, serde_json::json!({ "tools": tools }))
    }

    fn handle_tools_call(
        &self,
        params: serde_json::Value,
        id: Option<serde_json::Value>,
    ) -> McpResponse {
        let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let empty_obj = serde_json::json!({});
        let arguments = params.get("arguments").unwrap_or(&empty_obj);

        // Get format preference: per-call argument > session-level > default (json)
        let format_hint = arguments
            .get("format")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                let pf = self.preferred_format.lock().unwrap();
                pf.clone()
            });

        match tool_name {
            "stats" => self.tool_stats(arguments, id, format_hint.as_deref()),
            "search" => self.tool_search(arguments, id, format_hint.as_deref()),
            "read" => self.tool_read(arguments, id, format_hint.as_deref()),
            "neighbors" => self.tool_neighbors(arguments, id, format_hint.as_deref()),
            "impact" => self.tool_impact(arguments, id, format_hint.as_deref()),
            "edit_plan" => self.tool_edit_plan(arguments, id, format_hint.as_deref()),
            "verify" => self.tool_verify(arguments, id, format_hint.as_deref()),
            _ => McpResponse::err(
                id,
                -32601,
                &format!("Unknown tool: {}", tool_name),
            ),
        }
    }

    // ────────────────────────────────────────────────────────────────
    // Tool implementations
    // ────────────────────────────────────────────────────────────────

    fn tool_stats(
        &self,
        _args: &serde_json::Value,
        id: Option<serde_json::Value>,
        format_hint: Option<&str>,
    ) -> McpResponse {
        let result = match self.db.stats() {
            Ok((files, symbols, refs)) => serde_json::json!({
                "files": files,
                "symbols": symbols,
                "references": refs
            }),
            Err(e) => return McpResponse::err(id, -1, &format!("Database error: {}", e)),
        };
        McpResponse::ok_with_format(id, &result, format_hint)
    }

    fn tool_search(
        &self,
        args: &serde_json::Value,
        id: Option<serde_json::Value>,
        format_hint: Option<&str>,
    ) -> McpResponse {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        if query.is_empty() {
            return McpResponse::err(id, -1, "Missing 'query' parameter");
        }

        let kind_filter = args.get("kind").and_then(|v| v.as_str()).map(String::from);
        let language_filter = args.get("language").and_then(|v| v.as_str()).map(String::from);
        let min_score = args.get("min_score").and_then(|v| v.as_f64()).unwrap_or(0.1);
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
        let use_like = args.get("use_like").and_then(|v| v.as_bool()).unwrap_or(false);

        let extension_filter: Option<String> = language_filter.as_ref().map(|lang| {
            match lang.to_lowercase().as_str() {
                "python" => "py".to_string(),
                "javascript" | "js" => "js".to_string(),
                "typescript" | "ts" => "ts".to_string(),
                "rust" | "rs" => "rs".to_string(),
                "go" => "go".to_string(),
                "ruby" | "rb" => "rb".to_string(),
                "java" => "java".to_string(),
                "php" => "php".to_string(),
                "julia" | "jl" => "jl".to_string(),
                "lua" => "lua".to_string(),
                "swift" => "swift".to_string(),
                "zig" => "zig".to_string(),
                "scala" => "scala".to_string(),
                "elixir" | "ex" => "ex".to_string(),
                "dart" => "dart".to_string(),
                "haskell" | "hs" => "hs".to_string(),
                "c" => "c".to_string(),
                "cpp" | "c++" => "cpp".to_string(),
                _ => lang.clone(),
            }
        });

        if use_like {
            let candidates = self.db.search_symbol_ranked(query).unwrap_or_default();
            let results: Vec<serde_json::Value> = candidates
                .iter()
                .map(|(sym, rank)| {
                    serde_json::json!({
                        "name": sym.name,
                        "kind": sym.kind,
                        "file_path": sym.file_path,
                        "start_line": sym.start_line,
                        "end_line": sym.end_line,
                        "signature": sym.signature,
                        "page_rank": rank
                    })
                })
                .collect();

            return McpResponse::ok_with_format(
                id,
                &serde_json::json!({"query": query, "results": results, "count": results.len(), "mode": "like"}),
                format_hint,
            );
        }

        let config = SearchConfig {
            kind_filter,
            language_filter,
            min_score,
            limit,
            fuzzy_weight: 0.5,
            rank_weight: 0.3,
            kind_weight: 0.2,
            use_like: false,
        };

        match search::advanced_search(&self.db, query, extension_filter.as_deref(), &config) {
            Ok(results) => {
                let result_json = serde_json::json!({"query": query, "results": results, "count": results.len(), "mode": "fuzzy"});
                McpResponse::ok_with_format(id, &result_json, format_hint)
            }
            Err(e) => McpResponse::err(id, -1, &format!("Search error: {}", e)),
        }
    }

    fn tool_read(
        &self,
        args: &serde_json::Value,
        id: Option<serde_json::Value>,
        format_hint: Option<&str>,
    ) -> McpResponse {
        let file_path = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
        if file_path.is_empty() {
            return McpResponse::err(id, -1, "Missing 'file' parameter");
        }

        match self.db.file_symbols(file_path) {
            Ok(symbols) => {
                let result_json = serde_json::json!({"file": file_path, "symbols": symbols, "count": symbols.len()});
                McpResponse::ok_with_format(id, &result_json, format_hint)
            }
            Err(e) => McpResponse::err(id, -1, &format!("Database error: {}", e)),
        }
    }

    fn tool_neighbors(
        &self,
        args: &serde_json::Value,
        id: Option<serde_json::Value>,
        format_hint: Option<&str>,
    ) -> McpResponse {
        let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
        if symbol.is_empty() {
            return McpResponse::err(id, -1, "Missing 'symbol' parameter");
        }

        match self.db.neighbors(symbol) {
            Ok(refs) => {
                let result = serde_json::json!({
                    "symbol": symbol,
                    "neighbors": refs,
                    "count": refs.len()
                });
                McpResponse::ok_with_format(id, &result, format_hint)
            }
            Err(e) => McpResponse::err(id, -1, &format!("Database error: {}", e)),
        }
    }

    fn tool_impact(
        &self,
        args: &serde_json::Value,
        id: Option<serde_json::Value>,
        format_hint: Option<&str>,
    ) -> McpResponse {
        let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
        if symbol.is_empty() {
            return McpResponse::err(id, -1, "Missing 'symbol' parameter");
        }

        match self.db.impact(symbol) {
            Ok((callers, refs)) => {
                let result = serde_json::json!({
                    "symbol": symbol,
                    "impacted_symbols": callers,
                    "references": refs,
                    "impacted_count": callers.len()
                });
                McpResponse::ok_with_format(id, &result, format_hint)
            }
            Err(e) => McpResponse::err(id, -1, &format!("Database error: {}", e)),
        }
    }

 fn tool_edit_plan(
       &self,
       args: &serde_json::Value,
       id: Option<serde_json::Value>,
       format_hint: Option<&str>,
   ) -> McpResponse {
        let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
        if symbol.is_empty() {
            return McpResponse::err(id, -1, "Missing 'symbol' parameter");
        }

        let empty: Vec<serde_json::Value> = vec![];
        let changes_raw = args.get("changes").and_then(|v| v.as_array()).unwrap_or(&empty);
        let depth: u8 = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(1) as u8;

        let mut changes = Vec::new();
        for ch in changes_raw {
            let change_type_str = ch.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let change_type = match change_type_str {
                "rename" => crate::graph::EditChangeType::Rename,
                "add_param" => crate::graph::EditChangeType::AddParam,
                "remove_param" => crate::graph::EditChangeType::RemoveParam,
                "change_return" => crate::graph::EditChangeType::ChangeReturn,
                _ => continue,
            };
            let details = ch
                .get("details")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            changes.push(crate::graph::EditChange { change_type, details });
        }

        match crate::graph::analyze_edit_plan(&self.db, symbol, &changes, depth) {
            Ok(result) => {
                let json_result = serde_json::to_value(result).unwrap_or_else(|_| {
                    serde_json::json!({ "error": "Failed to serialize edit plan result" })
                });
                McpResponse::ok_with_format(id, &json_result, format_hint)
            }
            Err(e) => McpResponse::err(id, -1, &format!("Edit plan error: {}", e)),
        }
    }

    fn tool_verify(
        &self,
        args: &serde_json::Value,
        id: Option<serde_json::Value>,
        format_hint: Option<&str>,
    ) -> McpResponse {
        let file_path = args.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
        if file_path.is_empty() {
            return McpResponse::err(id, -1, "Missing 'file_path' parameter");
        }

        let stored_hash = self.db.get_checksum(file_path).unwrap_or_default();

        let disk_path = Path::new(file_path);
        let disk_hash = if disk_path.exists() {
            crate::integrity::IntegrityGuard::compute_file_hash(disk_path).ok()
        } else {
            None
        };

        let status = match (&stored_hash, &disk_hash) {
            (Some(sh), Some(dh)) => {
                if sh == dh { "clean" } else { "dirty" }
            }
            (None, Some(_)) => "unknown",
            (_, None) => "missing",
        };

        McpResponse::ok_with_format(
            id,
            &serde_json::json!({
                "file_path": file_path,
                "stored_hash": stored_hash,
                "disk_hash": disk_hash,
                "status": status
            }),
            format_hint,
        )
    }
}
