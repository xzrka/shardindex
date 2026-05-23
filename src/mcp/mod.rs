/// MCP (Model Context Protocol) 서버 — JSON-RPC over HTTP
///
/// 노출 API: read, neighbors, impact, search, graph, stats, edit_plan, verify

pub mod stdio;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tracing::info;

use crate::database::IndexDb;
use crate::agent_cache::AgentCache;

/// MCP 서버 상태 (공유) — rusqlite Connection은 !Send이므로 Mutex로 감쌈
#[derive(Clone)]
pub struct ServerState {
    pub db: Arc<Mutex<IndexDb>>,
    pub cache: Arc<AgentCache>,
}

/// JSON-RPC 요청
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
    pub id: Option<serde_json::Value>,
}

/// JSON-RPC 응답
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
    pub id: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: Some(result),
            error: None,
            id,
        }
    }

    pub fn error(id: Option<serde_json::Value>, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(serde_json::json!({
                "code": code,
                "message": message
            })),
            id,
        }
    }
}

/// ─── Helper ───

fn get_id(params: &serde_json::Value) -> Option<serde_json::Value> {
    params.get("id").cloned()
}

/// ─── MCP 메서드 핸들러 ───

/// read — 파일의 심볼 목록 조회
pub async fn handle_read(
    params: serde_json::Value,
    state: ServerState,
) -> JsonRpcResponse {
    let file_path = params.get("file").and_then(|v| v.as_str()).unwrap_or("");
    if file_path.is_empty() {
        return JsonRpcResponse::error(get_id(&params), -32601, "Missing 'file' parameter");
    }

    // Cache check
    if let Some(cached) = state.cache.get("read", &params) {
        return JsonRpcResponse::success(
            get_id(&params),
            serde_json::from_str(&cached).unwrap_or_else(|_| {
                serde_json::json!({"cache_error": true, "raw": cached})
            }),
        );
    }

    let db = state.db.lock().unwrap();
    match db.file_symbols(file_path) {
        Ok(symbols) => {
            let result = serde_json::json!({
                "file": file_path,
                "symbols": symbols,
                "count": symbols.len()
            });
            let result_str = serde_json::to_string(&result).unwrap_or_default();
            // Cache the result (best-effort)
            let _ = state.cache.set("read", &params, &result_str, None);
            JsonRpcResponse::success(get_id(&params), result)
        }
        Err(e) => JsonRpcResponse::error(
            get_id(&params),
            -32603,
            &format!("Database error: {}", e),
        ),
    }
}

/// neighbors — 심볼의 직접 참조 (caller/callee)
pub async fn handle_neighbors(
    params: serde_json::Value,
    state: ServerState,
) -> JsonRpcResponse {
    let symbol = params.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    if symbol.is_empty() {
        return JsonRpcResponse::error(get_id(&params), -32601, "Missing 'symbol' parameter");
    }

    // Cache check
    if let Some(cached) = state.cache.get("neighbors", &params) {
        return JsonRpcResponse::success(
            get_id(&params),
            serde_json::from_str(&cached).unwrap_or_else(|_| {
                serde_json::json!({"cache_error": true, "raw": cached})
            }),
        );
    }

    let db = state.db.lock().unwrap();
    match db.neighbors(symbol) {
        Ok(refs) => {
            let result = serde_json::json!({
                "symbol": symbol,
                "neighbors": refs,
                "count": refs.len()
            });
            let result_str = serde_json::to_string(&result).unwrap_or_default();
            let _ = state.cache.set("neighbors", &params, &result_str, None);
            JsonRpcResponse::success(get_id(&params), result)
        }
        Err(e) => JsonRpcResponse::error(
            get_id(&params),
            -32603,
            &format!("Database error: {}", e),
        ),
    }
}

/// impact — 심볼 영향도 분석 (수정 시 영향받는 범위)
pub async fn handle_impact(
    params: serde_json::Value,
    state: ServerState,
) -> JsonRpcResponse {
    let symbol = params.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    if symbol.is_empty() {
        return JsonRpcResponse::error(get_id(&params), -32601, "Missing 'symbol' parameter");
    }

    // Cache check
    if let Some(cached) = state.cache.get("impact", &params) {
        return JsonRpcResponse::success(
            get_id(&params),
            serde_json::from_str(&cached).unwrap_or_else(|_| {
                serde_json::json!({"cache_error": true, "raw": cached})
            }),
        );
    }

    let db = state.db.lock().unwrap();
    match db.impact(symbol) {
        Ok((callers, refs)) => {
            let result = serde_json::json!({
                "symbol": symbol,
                "impacted_symbols": callers,
                "references": refs,
                "impacted_count": callers.len()
            });
            let result_str = serde_json::to_string(&result).unwrap_or_default();
            let _ = state.cache.set("impact", &params, &result_str, None);
            JsonRpcResponse::success(get_id(&params), result)
        }
        Err(e) => JsonRpcResponse::error(
            get_id(&params),
            -32603,
            &format!("Database error: {}", e),
        ),
    }
}

/// search — 심볼명 검색 (advanced: kind_filter, language_filter, fuzzy, min_score)
pub async fn handle_search(
    params: serde_json::Value,
    state: ServerState,
) -> JsonRpcResponse {
    let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
    if query.is_empty() {
        return JsonRpcResponse::error(get_id(&params), -32601, "Missing 'query' parameter");
    }

    // Cache check
    if let Some(cached) = state.cache.get("search", &params) {
        return JsonRpcResponse::success(
            get_id(&params),
            serde_json::from_str(&cached).unwrap_or_else(|_| {
                serde_json::json!({"cache_error": true, "raw": cached})
            }),
        );
    }

    let kind_filter = params.get("kind").and_then(|v| v.as_str()).map(String::from);
    let language_filter = params.get("language").and_then(|v| v.as_str()).map(String::from);
    let min_score = params.get("min_score").and_then(|v| v.as_f64()).unwrap_or(0.1);
    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
    let use_like = params.get("use_like").and_then(|v| v.as_bool()).unwrap_or(false);

    let db = state.db.lock().unwrap();

    // 파일 확장자 매핑 (language → extension)
    let extension_filter = language_filter.as_ref().map(|lang| {
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

    if use_like {
        // 빠른 LIKE 기반 검색 (fuzzy 비활성화)
        let candidates = db
            .search_symbol_ranked(query)
            .unwrap_or_else(|_| Vec::new());

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

        let result = serde_json::json!({
            "query": query,
            "results": results,
            "count": results.len(),
            "mode": "like"
        });
        let result_str = serde_json::to_string(&result).unwrap_or_default();
        let _ = state.cache.set("search", &params, &result_str, None);
        return JsonRpcResponse::success(get_id(&params), result);
    }

    // advanced fuzzy search
    let search_config = crate::search::SearchConfig {
        kind_filter: kind_filter.clone(),
        language_filter: language_filter.clone(),
        min_score,
        limit,
        ..Default::default()
    };

    match crate::search::advanced_search(&db, query, extension_filter.as_deref(), &search_config) {
        Ok(results) => {
            let result = serde_json::json!({
                "query": query,
                "results": results,
                "count": results.len(),
                "filters": {
                    "kind": kind_filter,
                    "language": language_filter,
                    "min_score": min_score
                },
                "mode": "fuzzy"
            });
            let result_str = serde_json::to_string(&result).unwrap_or_default();
            let _ = state.cache.set("search", &params, &result_str, None);
            JsonRpcResponse::success(get_id(&params), result)
        }
        Err(e) => JsonRpcResponse::error(
            get_id(&params),
            -32603,
            &format!("Search error: {}", e),
        ),
    }
}

/// stats — 인덱스 통계
pub async fn handle_stats(
    params: serde_json::Value,
    state: ServerState,
) -> JsonRpcResponse {
    // Cache check — stats is static so always cacheable
    if let Some(cached) = state.cache.get("stats", &params) {
        return JsonRpcResponse::success(
            get_id(&params),
            serde_json::from_str(&cached).unwrap_or_else(|_| {
                serde_json::json!({"cache_error": true, "raw": cached})
            }),
        );
    }

    let db = state.db.lock().unwrap();
    match db.stats() {
        Ok((files, symbols, refs)) => {
            let result = serde_json::json!({
                "files": files,
                "symbols": symbols,
                "references": refs
            });
            let result_str = serde_json::to_string(&result).unwrap_or_default();
            let _ = state.cache.set("stats", &params, &result_str, None);
            JsonRpcResponse::success(get_id(&params), result)
        }
        Err(e) => JsonRpcResponse::error(
            get_id(&params),
            -32603,
            &format!("Database error: {}", e),
        ),
    }
}

/// edit_plan — 수정 전 영향도 분석
///
/// params: { symbol, changes: [{ type, ... }], depth }
/// - changes[].type ∈ { rename, add_param, remove_param, change_return }
/// - rename: { from, to }
/// - add_param: { param, default_value? }
/// - remove_param: { param }
/// - change_return: { new_return }
pub async fn handle_edit_plan(
    params: serde_json::Value,
    state: ServerState,
) -> JsonRpcResponse {
    let symbol = params.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    if symbol.is_empty() {
        return JsonRpcResponse::error(get_id(&params), -32601, "Missing 'symbol' parameter");
    }

    let empty: Vec<serde_json::Value> = vec![];
    let changes_raw = params.get("changes").and_then(|v| v.as_array()).unwrap_or(&empty);
    let depth: u8 = params.get("depth").and_then(|v| v.as_u64()).unwrap_or(1) as u8;

    // Parse changes
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
        let details = ch.get("details")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        changes.push(crate::graph::EditChange { change_type, details });
    }

    let db = state.db.lock().unwrap();
    match crate::graph::analyze_edit_plan(&db, symbol, &changes, depth) {
        Ok(result) => JsonRpcResponse::success(
            get_id(&params),
            serde_json::to_value(result).unwrap_or_else(|_| {
                serde_json::json!({ "error": "Failed to serialize edit plan result" })
            }),
        ),
        Err(e) => JsonRpcResponse::error(
            get_id(&params),
            -32603,
            &format!("Edit plan error: {}", e),
        ),
    }
}

/// verify — 파일 무결성 검증 (BLAKE3 checksum)
///
/// params: { file_path }
/// Returns: { file_path, stored_hash, disk_hash, status }
pub async fn handle_verify(
    params: serde_json::Value,
    state: ServerState,
) -> JsonRpcResponse {
    let file_path = params.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
    if file_path.is_empty() {
        return JsonRpcResponse::error(get_id(&params), -32601, "Missing 'file_path' parameter");
    }

    let db = state.db.lock().unwrap();

    // Get stored checksum
    let stored_hash = db.get_checksum(file_path).unwrap_or_default();

    // Try to read from disk — try absolute and relative paths
    use std::path::Path;
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

    JsonRpcResponse::success(
        get_id(&params),
        serde_json::json!({
            "file_path": file_path,
            "stored_hash": stored_hash,
            "disk_hash": disk_hash,
            "status": status
        }),
    )
}

/// ─── HTTP REST 핸들러 ───

pub async fn rest_stats(
    State(state): State<ServerState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let db = state.db.lock().unwrap();
    match db.stats() {
        Ok((files, symbols, refs)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "files": files,
                "symbols": symbols,
                "references": refs
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

pub async fn rest_search(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(state): State<ServerState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let symbol = params.get("symbol").cloned().unwrap_or_default();
    if symbol.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'symbol' parameter"})),
        );
    }

    let kind_filter = params.get("kind").cloned();
    let language_filter = params.get("language").cloned();
    let min_score: f64 = params
        .get("min_score")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.1);
    let limit: usize = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let use_fuzzy: bool = params.get("fuzzy").map(|v| v == "true").unwrap_or(true);

    let db = state.db.lock().unwrap();

    // language → extension
    let ext_lang = language_filter.clone();
    let extension_filter = ext_lang.as_ref().map(|lang| {
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

    if !use_fuzzy {
        // LIKE 모드
        let results = db
            .search_symbol_ranked(&symbol)
            .unwrap_or_else(|_| Vec::new());

        let json_results: Vec<serde_json::Value> = results
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

        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "query": &symbol,
                "results": json_results,
                "count": json_results.len(),
                "mode": "like"
            })),
        );
    }

    // Fuzzy 모드
    let config = crate::search::SearchConfig {
        kind_filter,
        language_filter,
        min_score,
        limit,
        ..Default::default()
    };

    match crate::search::advanced_search(
        &db,
        &symbol,
        extension_filter.as_deref(),
        &config,
    ) {
        Ok(results) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "query": &symbol,
                "results": results,
                "count": results.len(),
                "mode": "fuzzy"
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

pub async fn rest_neighbors(
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(state): State<ServerState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let symbol = query.get("symbol").cloned().unwrap_or_default();
    if symbol.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'symbol' parameter"})),
        );
    }
    let db = state.db.lock().unwrap();
    match db.neighbors(&symbol) {
        Ok(refs) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "symbol": &symbol,
                "neighbors": refs,
                "count": refs.len()
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

/// ─── JSON-RPC 핸들러 ───

pub async fn jsonrpc_handler(
    State(state): State<ServerState>,
    Json(req): Json<JsonRpcRequest>,
) -> (StatusCode, Json<JsonRpcResponse>) {
    let response = match req.method.as_str() {
        "shardindex.read" => handle_read(req.params, state).await,
        "shardindex.neighbors" => handle_neighbors(req.params, state).await,
        "shardindex.impact" => handle_impact(req.params, state).await,
        "shardindex.search" => handle_search(req.params, state).await,
        "shardindex.stats" => handle_stats(req.params, state).await,
        "shardindex.edit_plan" => handle_edit_plan(req.params, state).await,
        "shardindex.verify" => handle_verify(req.params, state).await,
        unknown => JsonRpcResponse::error(
            req.id.clone(),
            -32601,
            &format!("Method not found: {}", unknown),
        ),
    };

    (StatusCode::OK, Json(response))
}

pub async fn health_handler() -> (StatusCode, &'static str) {
    (StatusCode::OK, "OK")
}

/// MCP 서버 라우터 생성 (REST + JSON-RPC)
pub fn create_router(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/stats", get(rest_stats))
        .route("/search", get(rest_search))
        .route("/neighbors", get(rest_neighbors))
        .route("/jsonrpc", post(jsonrpc_handler))
        .with_state(state)
}

/// MCP 서버 시작
pub async fn serve(state: ServerState, addr: &str) {
    let app = create_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await.expect("Bind failed");
    info!("MCP server listening on {}", addr);
    axum::serve(listener, app).await.expect("Server error");
}
