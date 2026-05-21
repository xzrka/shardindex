/// MCP (Model Context Protocol) 서버 — JSON-RPC over HTTP
///
/// 노출 API: read, neighbors, impact, search, graph, stats, init

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

/// MCP 서버 상태 (공유) — rusqlite Connection은 !Send이므로 Mutex로 감쌈
#[derive(Clone)]
pub struct ServerState {
    pub db: Arc<Mutex<IndexDb>>,
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

    let db = state.db.lock().unwrap();
    match db.file_symbols(file_path) {
        Ok(symbols) => JsonRpcResponse::success(
            get_id(&params),
            serde_json::json!({
                "file": file_path,
                "symbols": symbols,
                "count": symbols.len()
            }),
        ),
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

    let db = state.db.lock().unwrap();
    match db.neighbors(symbol) {
        Ok(refs) => JsonRpcResponse::success(
            get_id(&params),
            serde_json::json!({
                "symbol": symbol,
                "neighbors": refs,
                "count": refs.len()
            }),
        ),
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

    let db = state.db.lock().unwrap();
    match db.impact(symbol) {
        Ok((callers, refs)) => JsonRpcResponse::success(
            get_id(&params),
            serde_json::json!({
                "symbol": symbol,
                "impacted_symbols": callers,
                "references": refs,
                "impacted_count": callers.len()
            }),
        ),
        Err(e) => JsonRpcResponse::error(
            get_id(&params),
            -32603,
            &format!("Database error: {}", e),
        ),
    }
}

/// search — 심볼명 검색
pub async fn handle_search(
    params: serde_json::Value,
    state: ServerState,
) -> JsonRpcResponse {
    let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
    if query.is_empty() {
        return JsonRpcResponse::error(get_id(&params), -32601, "Missing 'query' parameter");
    }

    let db = state.db.lock().unwrap();
    match db.search_symbol(query) {
        Ok(symbols) => JsonRpcResponse::success(
            get_id(&params),
            serde_json::json!({
                "query": query,
                "results": symbols,
                "count": symbols.len()
            }),
        ),
        Err(e) => JsonRpcResponse::error(
            get_id(&params),
            -32603,
            &format!("Database error: {}", e),
        ),
    }
}

/// stats — 인덱스 통계
pub async fn handle_stats(
    params: serde_json::Value,
    state: ServerState,
) -> JsonRpcResponse {
    let db = state.db.lock().unwrap();
    match db.stats() {
        Ok((files, symbols, refs)) => JsonRpcResponse::success(
            get_id(&params),
            serde_json::json!({
                "files": files,
                "symbols": symbols,
                "references": refs
            }),
        ),
        Err(e) => JsonRpcResponse::error(
            get_id(&params),
            -32603,
            &format!("Database error: {}", e),
        ),
    }
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
    match db.search_symbol(&symbol) {
        Ok(symbols) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "query": &symbol,
                "results": symbols,
                "count": symbols.len()
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
