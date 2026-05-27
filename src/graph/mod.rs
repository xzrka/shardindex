/// 심볼 그래프 랭킹 — PageRank + degree centrality
///
/// 참조 그래프(reference 테이블)를 기반으로 심볼의 중요도를 계산.
/// Phase 3 (Graph Ranking) 구현.
///
/// ## 알고리즘
///
/// 1. **Degree Centrality**: in_degree (callee로서 참조 횟수) + out_degree (caller로서 참조 횟수)
/// 2. **PageRank**: 반복적 PageRank (damping factor=0.85, max_iter=100, tolerance=1e-6)
/// 3. **Combined Score**: 가중 합산 (PageRank × 0.7 + normalized_degree × 0.3)

use std::collections::HashMap;

use crate::database::IndexDb;

// ─── DOT graph output ───

/// 심볼별 호출 그래프 (DOT 형식)
pub fn symbol_dot(db: &IndexDb, symbol: &str) -> Result<String, anyhow::Error> {
    let neighbors = db.neighbors(symbol)?;

    let mut dot = "digraph shardindex {\n".to_string();
    dot.push_str("  rankdir=LR;\n");
    dot.push_str("  node [shape=box, style=filled, fillcolor=lightblue];\n\n");

    // 노드
    dot.push_str(&format!(
        "  \"{}\" [fillcolor=gold, label=\"{}\\n(target)\"];\n",
        sanitize(symbol),
        symbol
    ));

    let mut edges = Vec::new();
    for ref_rec in &neighbors {
        let caller = ref_rec.caller_symbol.as_deref().unwrap_or("?");
        edges.push((caller, &ref_rec.callee_symbol, &ref_rec.ref_kind));

        // 중복 노드 방지
        if caller != symbol {
            dot.push_str(&format!("  \"{}\" [label=\"{}\"];\n", sanitize(caller), caller));
        }
        if ref_rec.callee_symbol != symbol {
            dot.push_str(&format!(
                "  \"{}\" [label=\"{}\"];\n",
                sanitize(&ref_rec.callee_symbol),
                ref_rec.callee_symbol
            ));
        }
    }

    // 에지
    dot.push_str("\n  // Edges\n");
    for (caller, callee, kind) in &edges {
        let style = match kind.as_str() {
            "call" => "color=green",
            "import" => "color=blue",
            "inherit" => "color=red, style=dashed",
            _ => "color=gray",
        };
        dot.push_str(&format!(
            "  \"{}\" -> \"{}\" [label=\"{}\", {}];\n",
            sanitize(caller),
            sanitize(callee),
            kind,
            style
        ));
    }

    dot.push_str("}\n");
    Ok(dot)
}

/// 전체 그래프
pub fn full_dot(db: &IndexDb) -> Result<String, anyhow::Error> {
    let mut dot = "digraph shardindex {\n".to_string();
    dot.push_str("  rankdir=LR;\n");
    dot.push_str("  node [shape=box, style=filled, fillcolor=lightblue];\n\n");

    let (files, symbols, refs) = db.stats()?;

    // 요약 노드
    dot.push_str(&format!(
        "  \"project\" [shape=ellipse, label=\"Project\\n{} files, {} symbols, {} refs\", fillcolor=lightyellow];\n",
        files, symbols, refs
    ));

    // 샘플 심볼 (최대 50개)
    for file_hash in &db.all_file_hashes()? {
        if let Ok(syms) = db.file_symbols(&file_hash.path) {
            for sym in &syms {
                let label = format!(
                    "{}\\n[{}] L{}",
                    sym.name, sym.kind, sym.start_line
                );
                dot.push_str(&format!(
                    "  \"{}::{}\" [label=\"{}\"];\n",
                    sanitize(&file_hash.path),
                    sanitize(&sym.name),
                    label
                ));
            }
        }
    }

    dot.push_str("}\n");
    Ok(dot)
}

fn sanitize(s: &str) -> String {
    s.replace('"', "\\\"")
      .replace('\n', "\\n")
      .replace(' ', "_")
}

// ─── PageRank Algorithm ───

/// PageRank 계산 결과
#[derive(Debug, Clone)]
pub struct PageRankResult {
    /// 심볼명 → PageRank 스코어
    pub scores: HashMap<String, f64>,
    /// 총 반복 횟수
    pub iterations: usize,
    /// 최종 convergence delta
    pub final_delta: f64,
}

/// PageRank 파라미터
#[derive(Debug, Clone)]
pub struct PageRankConfig {
    /// Damping factor (일반적으로 0.85)
    pub damping: f64,
    /// 최대 반복 횟수
    pub max_iterations: usize,
    /// Convergence tolerance
    pub tolerance: f64,
}

impl Default for PageRankConfig {
    fn default() -> Self {
        Self {
            damping: 0.85,
            max_iterations: 100,
            tolerance: 1e-6,
        }
    }
}

/// 참조 그래프에서 PageRank 계산
///
/// # 알고리즘
///
/// PR(A) = (1-d)/N + d * (Σ(PR(Ti) / L(Ti)) + dangling_sum/N)
///
/// - N: 총 심볼 수
/// - d: damping factor
/// - Ti: A를 참조하는 심볼들 (in-links)
/// - L(Ti): Ti가 참조하는 심볼 수 (out_degree)
/// - dangling_sum: out_degree=0인 노드들의 rank 합
///
/// dangling node의 rank는 그래프 전체에 균등 분배됨.
pub fn compute_pagerank(edges: &[(String, String)], config: &PageRankConfig) -> PageRankResult {
    let PageRankConfig {
        damping,
        max_iterations,
        tolerance,
    } = config;

    // 모든 노드 수집 (caller + callee) — HashSet으로 중복 방지
    let mut node_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (src, tgt) in edges {
        node_set.insert(src.clone());
        node_set.insert(tgt.clone());
    }
    let mut nodes: Vec<String> = node_set.into_iter().collect();
    nodes.sort(); // 결정적 순서 보장

    let n = nodes.len();
    if n == 0 {
        return PageRankResult {
            scores: HashMap::new(),
            iterations: 0,
            final_delta: 0.0,
        };
    }

    // 노드명 → 인덱스 매핑
    let node_idx: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), i))
        .collect();

    // 초기 rank: 균등 분배
    let mut ranks = vec![1.0 / n as f64; n];

    // out_degree 계산
    let mut out_deg = vec![0usize; n];
    for (src, _tgt) in edges {
        if let Some(&si) = node_idx.get(src) {
            out_deg[si] += 1;
        }
    }

    // in-links: target → [source_index]
    let mut in_links: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (src, tgt) in edges {
        if let (Some(&si), Some(&ti)) = (node_idx.get(src), node_idx.get(tgt)) {
            in_links[ti].push(si);
        }
    }

    // Iterative PageRank
    let mut iteration = 0;
    let mut delta = 1.0;

    while delta > *tolerance && iteration < *max_iterations {
        let mut new_ranks = vec![0.0_f64; n];

        // Dangling nodes 처리: out_degree=0인 노드의 rank 합
        let dangling_sum: f64 = (0..n)
            .filter(|&i| out_deg[i] == 0)
            .map(|i| ranks[i])
            .sum();

        for i in 0..n {
            // Teleport + dangling 분배
            let mut rank = (1.0 - damping) / n as f64 + damping * dangling_sum / n as f64;

            // in-links에서 rank 전달
            for &src_idx in &in_links[i] {
                let src_out = out_deg[src_idx];
                if src_out > 0 {
                    rank += damping * ranks[src_idx] / src_out as f64;
                }
            }

            new_ranks[i] = rank;
        }

        // Convergence check (L1 norm)
        delta = (0..n).map(|i| (new_ranks[i] - ranks[i]).abs()).sum::<f64>();
        ranks = new_ranks;
        iteration += 1;
    }

    // 결과 매핑
    let mut scores = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        scores.insert(node.clone(), ranks[i]);
    }

    PageRankResult {
        scores,
        iterations: iteration,
        final_delta: delta,
    }
}

// ─── Combined Ranking ───

/// 심볼 랭킹 계산 (degree + PageRank 결합)
///
/// 1. DB에서 edges, degrees 추출
/// 2. PageRank 계산
/// 3. 정규화 후 결합 스코어 계산
/// 4. DB에 저장
pub fn compute_and_store_ranks(db: &IndexDb, config: Option<&PageRankConfig>) -> anyhow::Result<()> {
    let config = config.cloned().unwrap_or_default();

    // 기존 랭킹 초기화
    db.clear_ranks()?;

    // Degree centrality 계산 (SQLite에서 직접)
    let degrees = db.compute_degrees()?;

    // Graph edges 추출
    let edges = db.graph_edges()?;

    // PageRank 계산
    let pagerank = compute_pagerank(&edges, &config);

    tracing::info!(
        "PageRank computed: {} symbols, {} iterations, delta={:.10}",
        pagerank.scores.len(),
        pagerank.iterations,
        pagerank.final_delta
    );

    // Max PageRank (정규화용)
    let max_pr = pagerank
        .scores
        .values()
        .fold(0.0_f64, |a, &b| if b > a { b } else { a });
    let max_pr = max_pr.max(1e-10); // division by zero 방지

    // Max degree (정규화용)
    let max_degree = degrees
        .iter()
        .map(|(_, ind, outd)| ind.max(outd))
        .fold(0_i64, |a, &b| if b > a { b } else { a });
    let max_degree = max_degree.max(1);

    // 결합 스코어 계산 + 저장
    use chrono::Utc;
    let computed_at = Utc::now().to_rfc3339();

    for (name, in_deg, out_deg) in &degrees {
        let pr = pagerank.scores.get(name).copied().unwrap_or(0.0);

        // 정규화: PageRank [0,1] + degree [0,1]
        let norm_pr = pr / max_pr;
        let norm_degree = (*in_deg as f64 + *out_deg as f64) / (2.0 * max_degree as f64);

        // 결합: PageRank 70%, degree 30%
        let combined = norm_pr * 0.7 + norm_degree * 0.3;

        let rank = crate::database::SymbolRank {
            symbol_name: name.clone(),
            page_rank: combined,
            in_degree: *in_deg,
            out_degree: *out_deg,
            computed_at: computed_at.clone(),
        };

        db.upsert_rank(&rank)?;
    }

    tracing::info!("Ranking stored: {} symbols", degrees.len());
    Ok(())
}

// ─── Edit Plan Analysis ───

/// Proposed change type for edit_plan
#[derive(Debug, Clone)]
pub enum EditChangeType {
    Rename,
    AddParam,
    RemoveParam,
    ChangeReturn,
}

/// A single proposed change
#[derive(Debug, Clone)]
pub struct EditChange {
    pub change_type: EditChangeType,
    pub details: serde_json::Map<String, serde_json::Value>,
}

/// Impact analysis result for edit_plan
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EditPlanResult {
    pub affected_symbols: Vec<String>,
    pub files_to_update: Vec<String>,
    pub breaking_changes: Vec<BreakingChange>,
    pub safe_to_proceed: bool,
    pub estimated_tokens: u32,
}

/// A detected breaking change
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BreakingChange {
    pub symbol: String,
    pub file_path: String,
    pub change_type: String,
    pub description: String,
}

/// Analyze the impact of proposed edits on a symbol.
///
/// 1. Look up target symbol in DB
/// 2. Run impact analysis to get all callers
/// 3. For `rename`: identify all files referencing the old name
/// 4. For `add_param/remove_param`: identify callers needing updates
/// 5. Return affected files + breaking changes
pub fn analyze_edit_plan(
    db: &IndexDb,
    symbol: &str,
    changes: &[EditChange],
    _depth: u8,
) -> anyhow::Result<EditPlanResult> {
    // Get impact: all callers of this symbol
    let (callers, _refs) = db.impact(symbol)?;

    // Get neighbors for callee info
    let neighbors = db.neighbors(symbol)?;

    // Collect affected symbols (callers + the target itself)
    let mut affected_symbols: Vec<String> = Vec::new();
    affected_symbols.push(symbol.to_string());
    for caller in &callers {
        if !affected_symbols.contains(&caller.name) {
            affected_symbols.push(caller.name.clone());
        }
    }

    // Collect files that need updating (caller symbols already have file_path)
    let mut files_to_update: Vec<String> = Vec::new();
    for caller in &callers {
        if !files_to_update.contains(&caller.file_path) {
            files_to_update.push(caller.file_path.clone());
        }
    }

    // Analyze breaking changes per change type
    let mut breaking_changes: Vec<BreakingChange> = Vec::new();

    for change in changes {
        match &change.change_type {
            EditChangeType::Rename => {
                let from = change
                    .details
                    .get("from")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let to = change
                    .details
                    .get("to")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if !from.is_empty() && !to.is_empty() {
                    for caller in &callers {
                        breaking_changes.push(BreakingChange {
                            symbol: caller.name.clone(),
                            file_path: caller.file_path.clone(),
                            change_type: "rename".to_string(),
                            description: format!(
                                "Update reference from '{}' to '{}'",
                                from, to
                            ),
                        });
                    }
                }
            }
            EditChangeType::AddParam => {
                let param = change
                    .details
                    .get("param")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                for caller in &callers {
                    breaking_changes.push(BreakingChange {
                        symbol: caller.name.clone(),
                        file_path: caller.file_path.clone(),
                        change_type: "add_param".to_string(),
                        description: format!(
                            "Caller must add parameter '{}' to call site",
                            param
                        ),
                    });
                }
            }
            EditChangeType::RemoveParam => {
                let param = change
                    .details
                    .get("param")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                for caller in &callers {
                    breaking_changes.push(BreakingChange {
                        symbol: caller.name.clone(),
                        file_path: caller.file_path.clone(),
                        change_type: "remove_param".to_string(),
                        description: format!(
                            "Caller must remove parameter '{}' from call site",
                            param
                        ),
                    });
                }
            }
            EditChangeType::ChangeReturn => {
                let new_return = change
                    .details
                    .get("new_return")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                for caller in &callers {
                    breaking_changes.push(BreakingChange {
                        symbol: caller.name.clone(),
                        file_path: caller.file_path.clone(),
                        change_type: "change_return".to_string(),
                        description: format!(
                            "Return type changed — caller may need to adapt to '{}'",
                            new_return
                        ),
                    });
                }
            }
        }
    }

    // Also include callee references (symbols this target calls) — they may be affected by signature changes
    for neighbor in &neighbors {
        let callee = neighbor.callee_symbol.as_str();
        if !affected_symbols.contains(&callee.to_string()) {
            affected_symbols.push(callee.to_string());
        }
    }

    // Estimate tokens: ~50 tokens per affected symbol for ref updates
    let estimated_tokens = (affected_symbols.len() * 50) as u32;

    // Safe to proceed if no breaking changes or only rename changes
    let safe_to_proceed = breaking_changes.is_empty()
        || breaking_changes.iter().all(|bc| bc.change_type == "rename");

    Ok(EditPlanResult {
        affected_symbols,
        files_to_update,
        breaking_changes,
        safe_to_proceed,
        estimated_tokens,
    })
}

// ─── Phase 9: Refactoring-Specialized APIs ───

/// Impact layer at a specific depth (for impact_deep)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImpactLayer {
    pub depth: u8,
    pub symbols: Vec<String>,
    pub confidence: f64,
    pub risk: String,
}

/// Dynamic reference at risk
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DynamicRefAtRisk {
    pub expr: String,
    pub confidence: f64,
    pub file: String,
}

/// Deep impact analysis result (transitive dependency tracing + risk scoring)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImpactDeepResult {
    pub target: String,
    pub layers: Vec<ImpactLayer>,
    pub critical_paths: Vec<String>,
    pub test_coverage_gaps: Vec<String>,
    pub dynamic_refs_at_risk: Vec<DynamicRefAtRisk>,
    pub recommendation: String,
}

/// Dead code verification stage result
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeadCodeStage {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callers: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tests: Option<Vec<String>>,
}

/// Dead code verification result (multi-stage)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeadCodeVerifyResult {
    pub safe_to_delete: bool,
    pub stages: std::collections::HashMap<String, DeadCodeStage>,
    pub blockers: Vec<String>,
    pub suggestion: String,
}

/// File modification plan for cross-module move
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileModification {
    pub path: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// Unresolved reference for cross-module move
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UnresolvedRef {
    pub file: String,
    #[serde(rename = "type")]
    pub ref_type: String,
    pub value: String,
}

/// Cross-module move result
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CrossModuleMoveResult {
    pub dry_run: bool,
    pub files_to_modify: Vec<FileModification>,
    pub unresolved_refs: Vec<UnresolvedRef>,
    pub estimated_tokens: u32,
    pub safe_to_execute: bool,
    pub reason: String,
}

/// Breaking caller for signature migration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BreakingCaller {
    pub symbol: String,
    pub call_site: String,
    pub issue: String,
}

/// Signature migration check result
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SignatureMigrationResult {
    pub compatible: bool,
    pub breaking_callers: Vec<BreakingCaller>,
    pub safe_callers: usize,
    pub suggestion: String,
}

// ─── 9.1 impact_deep ───

/// Extended impact analysis with transitive dependency tracing and risk scoring.
///
/// BFS-based multi-depth impact propagation. Each layer represents one hop in the
/// call graph. Confidence decreases with depth. Risk is computed based on the number
/// of affected symbols and the presence of dynamic references.
pub fn impact_deep(
    db: &crate::database::IndexDb,
    symbol: &str,
    depth: u8,
    include_tests: bool,
    include_dynamic: bool,
    _risk_analysis: bool,
    _token_budget: Option<u32>,
) -> anyhow::Result<ImpactDeepResult> {
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut layers: Vec<ImpactLayer> = Vec::new();
    let mut critical_paths: Vec<String> = Vec::new();
    let mut dynamic_refs_at_risk: Vec<DynamicRefAtRisk> = Vec::new();

    // BFS from target symbol
    let mut current_callers: Vec<String> = Vec::new();
    current_callers.push(symbol.to_string());
    visited.insert(symbol.to_string());

    for d in 1..=depth {
        let mut layer_symbols: Vec<String> = Vec::new();

        for caller_name in &current_callers {
            let (callers, refs) = db.impact(caller_name)?;

            for caller in &callers {
                // Skip test files if not included
                if !include_tests && (caller.file_path.contains("test") || caller.file_path.contains("_spec")) {
                    continue;
                }

                if !visited.contains(&caller.name) {
                    visited.insert(caller.name.clone());
                    layer_symbols.push(caller.name.clone());
                }
            }

            // Collect dynamic references at risk
            if include_dynamic {
                for ref_rec in &refs {
                    if ref_rec.is_dynamic && !visited.contains(&ref_rec.callee_symbol) {
                        dynamic_refs_at_risk.push(DynamicRefAtRisk {
                            expr: format!("{}.{}", ref_rec.caller_symbol.as_deref().unwrap_or("?"), ref_rec.callee_symbol),
                            confidence: ref_rec.confidence,
                            file: ref_rec.caller_file.clone(),
                        });
                    }
                }
            }
        }

        // Confidence decreases with depth
        let confidence = match d {
            1 => 0.95,
            2 => 0.82,
            _ => 0.95_f64.max(0.3 - (d as f64) * 0.15),
        };

        // Risk based on layer size and depth
        let risk = if d <= 1 {
            "low".to_string()
        } else if d <= 2 {
            "medium".to_string()
        } else {
            "high".to_string()
        };

        if !layer_symbols.is_empty() {
            layers.push(ImpactLayer {
                depth: d,
                symbols: layer_symbols.clone(),
                confidence,
                risk,
            });
        }

        current_callers = layer_symbols;
        if current_callers.is_empty() {
            break;
        }
    }

    // Build critical paths (longest chain from target through layers)
    if layers.len() >= 2 {
        let mut path = format!("→ {}", symbol);
        for layer in &layers {
            if let Some(first) = layer.symbols.first() {
                path.push_str(" → ");
                path.push_str(first);
            }
        }
        critical_paths.push(path);
    }

    // Test coverage gaps: symbols in depth-2+ that have no test callers
    let mut test_coverage_gaps: Vec<String> = Vec::new();
    for layer in &layers {
        if layer.depth >= 2 {
            for sym_name in &layer.symbols {
                let (test_callers, _) = db.impact(sym_name).unwrap_or_else(|_| (Vec::new(), Vec::new()));
                let has_tests = test_callers.iter().any(|c| {
                    c.file_path.contains("test") || c.file_path.contains("_spec")
                });
                if !has_tests {
                    test_coverage_gaps.push(format!(
                        "{} has 0 direct tests",
                        sym_name
                    ));
                }
            }
        }
    }

    // Generate recommendation
    let recommendation = if layers.is_empty() {
        "No transitive dependencies found. Safe to modify.".to_string()
    } else if layers.iter().any(|l| l.risk == "high") {
        "Modify with caution. Add tests for depth-3+ symbols before refactoring.".to_string()
    } else if !test_coverage_gaps.is_empty() {
        format!(
            "Proceed with care. {} symbols lack test coverage.",
            test_coverage_gaps.len()
        )
    } else if include_dynamic && !dynamic_refs_at_risk.is_empty() {
        format!(
            "Watch for {} dynamic references that may break at runtime.",
            dynamic_refs_at_risk.len()
        )
    } else {
        "Low risk. Direct dependencies are well-contained.".to_string()
    };

    Ok(ImpactDeepResult {
        target: symbol.to_string(),
        layers,
        critical_paths,
        test_coverage_gaps,
        dynamic_refs_at_risk,
        recommendation,
    })
}

// ─── 9.2 dead_code_verify ───

/// Multi-stage verification before deleting a symbol.
///
/// Stages: static_refs, dynamic_refs, string_refs, git_history, test_refs
pub fn dead_code_verify(
    db: &crate::database::IndexDb,
    symbol: &str,
    stages: &[&str],
    _min_confidence_for_deletion: f64,
) -> anyhow::Result<DeadCodeVerifyResult> {
    let mut result_stages: std::collections::HashMap<String, DeadCodeStage> =
        std::collections::HashMap::new();
    let mut blockers: Vec<String> = Vec::new();
    let mut is_safe = true;

    // Stage 1: static_refs — check for static callers in DB
    if stages.is_empty() || stages.contains(&"static_refs") {
        let (callers, _) = db.impact(symbol).unwrap_or_else(|_| (Vec::new(), Vec::new()));
        let caller_names: Vec<String> = callers.iter().map(|c| c.name.clone()).collect();

        if caller_names.is_empty() {
            result_stages.insert(
                "static_refs".to_string(),
                DeadCodeStage {
                    status: "pass".to_string(),
                    callers: Some(Vec::new()),
                    matches: None,
                    last_commit: None,
                    commit_message: None,
                    tests: None,
                },
            );
        } else {
            is_safe = false;
            blockers.push(format!(
                "{} static callers still reference this symbol",
                caller_names.len()
            ));
            result_stages.insert(
                "static_refs".to_string(),
                DeadCodeStage {
                    status: "fail".to_string(),
                    callers: Some(caller_names),
                    matches: None,
                    last_commit: None,
                    commit_message: None,
                    tests: None,
                },
            );
        }
    }

    // Stage 2: dynamic_refs — check for dynamic references
    if stages.contains(&"dynamic_refs") {
        let neighbors = db.neighbors(symbol).unwrap_or_else(|_| Vec::new());
        let dynamic_matches: Vec<String> = neighbors
            .iter()
            .filter(|r| r.is_dynamic)
            .map(|r| format!(
                "{} (confidence: {:.1})",
                r.caller_file, r.confidence
            ))
            .collect();

        if dynamic_matches.is_empty() {
            result_stages.insert(
                "dynamic_refs".to_string(),
                DeadCodeStage {
                    status: "pass".to_string(),
                    callers: None,
                    matches: Some(Vec::new()),
                    last_commit: None,
                    commit_message: None,
                    tests: None,
                },
            );
        } else {
            is_safe = false;
            for m in &dynamic_matches {
                blockers.push(format!("Dynamic reference: {}", m));
            }
            result_stages.insert(
                "dynamic_refs".to_string(),
                DeadCodeStage {
                    status: "fail".to_string(),
                    callers: None,
                    matches: Some(dynamic_matches),
                    last_commit: None,
                    commit_message: None,
                    tests: None,
                },
            );
        }
    }

    // Stage 3: string_refs — grep for string references to the symbol name
    if stages.contains(&"string_refs") {
        // Search for the symbol name in all indexed files (as string literals)
        let all_symbols = db.search_symbol(symbol).unwrap_or_else(|_| Vec::new());
        let string_matches: Vec<String> = all_symbols
            .iter()
            .filter(|s| s.name != symbol)
            .map(|s| format!("{} in {}", s.name, s.file_path))
            .take(10)
            .collect();

        if string_matches.is_empty() {
            result_stages.insert(
                "string_refs".to_string(),
                DeadCodeStage {
                    status: "pass".to_string(),
                    callers: None,
                    matches: Some(Vec::new()),
                    last_commit: None,
                    commit_message: None,
                    tests: None,
                },
            );
        } else {
            // Warning only — string refs may be logging/comments
            result_stages.insert(
                "string_refs".to_string(),
                DeadCodeStage {
                    status: "warn".to_string(),
                    callers: None,
                    matches: Some(string_matches),
                    last_commit: None,
                    commit_message: None,
                    tests: None,
                },
            );
        }
    }

    // Stage 4: git_history — check last commit info (best-effort)
    if stages.contains(&"git_history") {
        // Try to get git log for files containing this symbol
        let file_symbols = db.search_symbol(symbol).unwrap_or_else(|_| Vec::new());
        if let Some(sym) = file_symbols.first() {
            let output = std::process::Command::new("git")
                .args(&["log", "-1", "--format=%ai|%s", "--", &sym.file_path])
                .output()
                .ok();

            if let Some(output) = output {
                let log_line = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if let Some(pipe) = log_line.find('|') {
                    let commit_date = &log_line[..pipe];
                    let commit_msg = &log_line[pipe + 1..];

                    result_stages.insert(
                        "git_history".to_string(),
                        DeadCodeStage {
                            status: if commit_msg.to_lowercase().contains("deprecat") {
                                "warn".to_string()
                            } else {
                                "info".to_string()
                            },
                            callers: None,
                            matches: None,
                            last_commit: Some(commit_date.to_string()),
                            commit_message: Some(commit_msg.to_string()),
                            tests: None,
                        },
                    );
                }
            }
        }
    }

    // Stage 5: test_refs — check if any tests reference this symbol
    if stages.contains(&"test_refs") {
        let (callers, _) = db.impact(symbol).unwrap_or_else(|_| (Vec::new(), Vec::new()));
        let test_refs: Vec<String> = callers
            .iter()
            .filter(|c| c.file_path.contains("test") || c.file_path.contains("_spec"))
            .map(|c| c.name.clone())
            .collect();

        if test_refs.is_empty() {
            result_stages.insert(
                "test_refs".to_string(),
                DeadCodeStage {
                    status: "pass".to_string(),
                    callers: None,
                    matches: None,
                    last_commit: None,
                    commit_message: None,
                    tests: Some(Vec::new()),
                },
            );
        } else {
            // Tests still reference it — could be intentional
            result_stages.insert(
                "test_refs".to_string(),
                DeadCodeStage {
                    status: "warn".to_string(),
                    callers: None,
                    matches: None,
                    last_commit: None,
                    commit_message: None,
                    tests: Some(test_refs),
                },
            );
        }
    }

    // Generate suggestion
    let suggestion = if !is_safe {
        "Do not delete. Mark as deprecated and monitor for 1 release cycle.".to_string()
    } else if blockers.is_empty() {
        "Safe to delete. No references found.".to_string()
    } else {
        "Review blockers before deletion.".to_string()
    };

    Ok(DeadCodeVerifyResult {
        safe_to_delete: is_safe && blockers.is_empty(),
        stages: result_stages,
        blockers,
        suggestion,
    })
}

// ─── 9.3 cross_module_move ───

/// Generate a language-appropriate target file path for a module move.
///
/// Uses the detected project language to determine the correct file extension
/// and path convention. Falls back to the source file's extension if language
/// detection fails.
fn resolve_target_file_path(
    db: &crate::database::IndexDb,
    target_module: &str,
    source_file: &str,
) -> String {
    // Detect project language
    let language = db.detect_project_language()
        .or_else(|| db.detect_language_from_path(source_file))
        .unwrap_or_else(|| "python".to_string());

    // Determine file extension from language
    let ext = match language.as_str() {
        "python" => "py",
        "javascript" => "js",
        "typescript" => "ts",
        "rust" => "rs",
        "go" => "go",
        "ruby" => "rb",
        "java" => "java",
        "php" => "php",
        "julia" => "jl",
        "lua" => "lua",
        "swift" => "swift",
        "zig" => "zig",
        "scala" => "scala",
        "elixir" => "ex",
        "dart" => "dart",
        "haskell" => "hs",
        "c" => "c",
        "cpp" => "cpp",
        "markdown" => "md",
        _ => "py", // fallback
    };

    // Language-specific path conventions
    match language.as_str() {
        // Rust: src/module/mod.rs
        "rust" => format!("src/{}/mod.rs", target_module.replace('.', "/")),
        // Go: module/file.go (lowercase module name)
        "go" => format!("{}/{}.go", target_module.replace('.', "/"), target_module.split('.').last().unwrap_or(target_module)),
        // Java: module/file.java (package-style path)
        "java" => format!("{}/{}.java", target_module.replace('.', "/"), target_module.split('.').last().unwrap_or(target_module)),
        // C/C++: include/module/file.h or src/module/file.cpp
        "c" => format!("include/{}.h", target_module.replace('.', "/")),
        "cpp" => format!("src/{}/{}.cpp", target_module.replace('.', "/"), target_module.split('.').last().unwrap_or(target_module)),
        // Python/JS/TS/Ruby/PHP/Julia/Lua/Swift/Zig/Scala/Elixir/Dart/Haskell: module/file.ext
        _ => format!("{}/{}.{}", target_module.replace('.', "/"), target_module.split('.').last().unwrap_or(target_module), ext),
    }
}

/// Safe symbol relocation across module boundaries with automatic ref updating.
///
/// Analyzes all references and import statements, generates a plan of file
/// modifications needed to move the symbol to a new module.
pub fn cross_module_move(
    db: &crate::database::IndexDb,
    symbol: &str,
    target_module: &str,
    update_imports: bool,
    _update_string_refs: bool,
    dry_run: bool,
) -> anyhow::Result<CrossModuleMoveResult> {
    let mut files_to_modify: Vec<FileModification> = Vec::new();
    let mut unresolved_refs: Vec<UnresolvedRef> = Vec::new();

    // Find the symbol in DB
    let search_results = db.search_symbol(symbol).unwrap_or_else(|_| Vec::new());
    if search_results.is_empty() {
        return Err(anyhow::anyhow!("Symbol '{}' not found in index", symbol));
    }

    let target_sym = &search_results[0];
    let new_symbol_name = format!("{}.{}", target_module, target_sym.name);

    // Source file: delete_symbol action
    files_to_modify.push(FileModification {
        path: target_sym.file_path.clone(),
        action: "delete_symbol".to_string(),
        symbol: Some(symbol.to_string()),
        from: None,
        to: None,
    });

    // Target file: insert_symbol action (language-aware path)
    let target_file = resolve_target_file_path(db, target_module, &target_sym.file_path);
    files_to_modify.push(FileModification {
        path: target_file.clone(),
        action: "insert_symbol".to_string(),
        symbol: Some(new_symbol_name.clone()),
        from: None,
        to: None,
    });

    // Get all callers and generate import update actions
    let (callers, _) = db.impact(symbol).unwrap_or_else(|_| (Vec::new(), Vec::new()));

    if update_imports {
        for caller in &callers {
            files_to_modify.push(FileModification {
                path: caller.file_path.clone(),
                action: "update_import".to_string(),
                symbol: None,
                from: Some(symbol.to_string()),
                to: Some(new_symbol_name.clone()),
            });
        }
    } else {
        // If not auto-updating, mark callers as unresolved
        for caller in &callers {
            unresolved_refs.push(UnresolvedRef {
                file: caller.file_path.clone(),
                ref_type: "static_import".to_string(),
                value: format!("import {} from caller {}", symbol, caller.name),
            });
        }
    }

    // Check for dynamic/string references that can't be auto-updated
    let neighbors = db.neighbors(symbol).unwrap_or_else(|_| Vec::new());
    for ref_rec in &neighbors {
        if ref_rec.is_dynamic {
            unresolved_refs.push(UnresolvedRef {
                file: ref_rec.caller_file.clone(),
                ref_type: "dynamic_ref".to_string(),
                value: format!("Dynamic reference with confidence {:.1}", ref_rec.confidence),
            });
        }
    }

    let estimated_tokens = (files_to_modify.len() * 50 + unresolved_refs.len() * 30) as u32;
    let safe_to_execute = unresolved_refs.is_empty();
    let reason = if unresolved_refs.is_empty() {
        "All references can be automatically updated.".to_string()
    } else {
        format!(
            "{} unresolved references require manual review",
            unresolved_refs.len()
        )
    };

    Ok(CrossModuleMoveResult {
        dry_run,
        files_to_modify,
        unresolved_refs,
        estimated_tokens,
        safe_to_execute,
        reason,
    })
}

// ─── 9.4 signature_migration_check ───

/// Check if changing a function signature breaks callers.
///
/// Compares the current signature with the proposed new signature and identifies
/// callers that may need updates.
pub fn signature_migration_check(
    db: &crate::database::IndexDb,
    symbol: &str,
    new_signature: &str,
    _check_call_sites: bool,
) -> anyhow::Result<SignatureMigrationResult> {
    let mut breaking_callers: Vec<BreakingCaller> = Vec::new();
    let mut safe_count: usize = 0;

    // Get current symbol info
    let search_results = db.search_symbol(symbol).unwrap_or_else(|_| Vec::new());
    if search_results.is_empty() {
        return Err(anyhow::anyhow!("Symbol '{}' not found in index", symbol));
    }

    let current_sym = &search_results[0];
    let current_sig = current_sym.signature.as_deref().unwrap_or("");

    // Get all callers
    let (callers, _) = db.impact(symbol).unwrap_or_else(|_| (Vec::new(), Vec::new()));

    // Parse parameter counts from signatures (simplified heuristic)
    let old_params = count_params(current_sig);
    let new_params = count_params(new_signature);
    let old_required = count_required_params(current_sig);
    let new_required = count_required_params(new_signature);

    // Check for breaking changes
    let param_increase = new_required > old_required;
    let return_changed = return_type_changed(current_sig, new_signature);

    for caller in &callers {
        let call_site = format!("{}()", caller.name);

        if param_increase {
            breaking_callers.push(BreakingCaller {
                symbol: caller.name.clone(),
                call_site,
                issue: format!(
                    "New signature requires {} params, old required {} — caller may need update",
                    new_required, old_required
                ),
            });
        } else if return_changed {
            breaking_callers.push(BreakingCaller {
                symbol: caller.name.clone(),
                call_site,
                issue: "Return type changed — caller may need to adapt".to_string(),
            });
        } else {
            safe_count += 1;
        }
    }

    let compatible = breaking_callers.is_empty();

    let suggestion = if param_increase && !return_changed {
        format!(
            "Add new parameters as optional (with defaults) to maintain backward compatibility. {} callers affected.",
            breaking_callers.len()
        )
    } else if return_changed {
        format!(
            "Consider keeping the old return type or providing a wrapper. {} callers affected.",
            breaking_callers.len()
        )
    } else if compatible {
        "Signature change is backward compatible.".to_string()
    } else {
        "Review breaking changes carefully.".to_string()
    };

    Ok(SignatureMigrationResult {
        compatible,
        breaking_callers,
        safe_callers: safe_count,
        suggestion,
    })
}

/// Count parameters in a signature string (heuristic)
fn count_params(signature: &str) -> usize {
    if let Some(start) = signature.find('(') {
        if let Some(end) = signature.rfind(')') {
            if start < end {
                let params_str = &signature[start + 1..end];
                if params_str.trim().is_empty() {
                    return 0;
                }
                return params_str.split(',').count();
            }
        }
    }
    0
}

/// Count required (non-optional) parameters
fn count_required_params(signature: &str) -> usize {
    if let Some(start) = signature.find('(') {
        if let Some(end) = signature.rfind(')') {
            if start < end {
                let params_str = &signature[start + 1..end];
                if params_str.trim().is_empty() {
                    return 0;
                }
                return params_str
                    .split(',')
                    .filter(|p| {
                        let p = p.trim();
                        !p.contains('=') && !p.contains(": Optional") && !p.contains(": Option<")
                    })
                    .count();
            }
        }
    }
    0
}

/// Check if return type changed between signatures
fn return_type_changed(old_sig: &str, new_sig: &str) -> bool {
    let old_return = extract_return_type(old_sig);
    let new_return = extract_return_type(new_sig);
    old_return != new_return && (!old_return.is_empty() || !new_return.is_empty())
}

/// Extract return type from signature (heuristic)
fn extract_return_type(signature: &str) -> String {
    if let Some(pos) = signature.find("->") {
        signature[pos + 2..].trim().to_string()
    } else if let Some(arrow) = signature.find('→') {
        // '→' is a 3-byte UTF-8 char; skip past it properly
        let skip = '→'.len_utf8();
        signature[arrow + skip..].trim().to_string()
    } else {
        String::new()
    }
}

// ─── Unit Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pagerank_simple_chain() {
        // A → B → C (선형 체인)
        let edges = [
            ("A".into(), "B".into()),
            ("B".into(), "C".into()),
        ];
        let config = PageRankConfig::default();
        let result = compute_pagerank(&edges, &config);

        // B가 가장 높은 rank를 가져야 함 (A에서 incoming, C에 outgoing)
        let a = result.scores.get("A").copied().unwrap_or(0.0);
        let b = result.scores.get("B").copied().unwrap_or(0.0);
        let c = result.scores.get("C").copied().unwrap_or(0.0);

        // B > A (B는 A로부터 incoming)
        assert!(b > a, "B({}) should be > A({})", b, a);
        // C가 가장 높음 (dangling node — outgoing이 없음)
        assert!(c > b, "C({}) should be > B({})", c, b);

        // 총합은 1.0
        let total = a + b + c;
        assert!((total - 1.0).abs() < 1e-6, "PageRank sum should be 1.0, got {}", total);
    }

    #[test]
    fn test_pagerank_hub() {
        // A, B, C → D (허브)
        let edges = [
            ("A".into(), "D".into()),
            ("B".into(), "D".into()),
            ("C".into(), "D".into()),
        ];
        let config = PageRankConfig::default();
        let result = compute_pagerank(&edges, &config);

        let d = result.scores.get("D").copied().unwrap_or(0.0);
        let a = result.scores.get("A").copied().unwrap_or(0.0);

        // D가 가장 높은 rank (3개의 incoming)
        assert!(d > a, "D({}) should be > A({})", d, a);
        assert!(d > 0.25, "D should have significant rank");
    }

    #[test]
    fn test_pagerank_cycle() {
        // A → B → C → A (완전한 사이클)
        let edges = [
            ("A".into(), "B".into()),
            ("B".into(), "C".into()),
            ("C".into(), "A".into()),
        ];
        let config = PageRankConfig::default();
        let result = compute_pagerank(&edges, &config);

        // 모든 노드가 동일한 rank
        let a = result.scores.get("A").copied().unwrap_or(0.0);
        let b = result.scores.get("B").copied().unwrap_or(0.0);
        let c = result.scores.get("C").copied().unwrap_or(0.0);

        assert!((a - b).abs() < 1e-6, "Cycle: A={} B={}", a, b);
        assert!((b - c).abs() < 1e-6, "Cycle: B={} C={}", b, c);
        assert!((a - 1.0 / 3.0).abs() < 1e-4, "Each should be ~1/3");
    }

    #[test]
    fn test_pagerank_empty() {
        let edges: Vec<(String, String)> = vec![];
        let config = PageRankConfig::default();
        let result = compute_pagerank(&edges, &config);

        assert!(result.scores.is_empty());
        assert_eq!(result.iterations, 0);
    }

    #[test]
    fn test_pagerank_single_self_loop() {
        // A → A (셀프 루프)
        let edges = [
            ("A".into(), "A".into()),
        ];
        let config = PageRankConfig::default();
        let result = compute_pagerank(&edges, &config);

        let a = result.scores.get("A").copied().unwrap_or(0.0);
        assert!((a - 1.0).abs() < 1e-6, "Single node should have rank 1.0");
    }

    #[test]
    fn test_pagerank_two_disconnected() {
        // A → B, C → D (두 개의 분리된 컴포넌트)
        let edges = [
            ("A".into(), "B".into()),
            ("C".into(), "D".into()),
        ];
        let config = PageRankConfig::default();
        let result = compute_pagerank(&edges, &config);

        // 총합은 1.0
        let total: f64 = result.scores.values().sum();
        assert!((total - 1.0).abs() < 1e-6);

        // 각 컴포넌트의 dangling node (B, D)가 더 높은 rank
        let b = result.scores.get("B").copied().unwrap_or(0.0);
        let d = result.scores.get("D").copied().unwrap_or(0.0);
        let a = result.scores.get("A").copied().unwrap_or(0.0);
        let c = result.scores.get("C").copied().unwrap_or(0.0);

        assert!(b > a, "B({}) > A({})", b, a);
        assert!(d > c, "D({}) > C({})", d, c);
        assert!((b - d).abs() < 1e-6, "B and D should be equal (symmetric)");
    }

    #[test]
    fn test_pagerank_authority_vs_hub() {
        // 허브: H가 모든 것에 outgoing
        // 권위: A가 모든 것에서 incoming
        // H → A, H → B, H → C
        // A, B, C는 dangling
        let edges = [
            ("H".into(), "A".into()),
            ("H".into(), "B".into()),
            ("H".into(), "C".into()),
        ];
        let config = PageRankConfig::default();
        let result = compute_pagerank(&edges, &config);

        let h = result.scores.get("H").copied().unwrap_or(0.0);
        let a = result.scores.get("A").copied().unwrap_or(0.0);
        let b = result.scores.get("B").copied().unwrap_or(0.0);
        let c = result.scores.get("C").copied().unwrap_or(0.0);

        // A, B, C는 dangling nodes로 균등
        assert!((a - b).abs() < 1e-6);
        assert!((b - c).abs() < 1e-6);

        // dangling이 더 높지만, H도 damping만큼 유지
        assert!(a > h, "Authority A({}) > Hub H({})", a, h);
    }

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize("hello world"), "hello_world");
        assert_eq!(sanitize("has\"quote"), "has\\\"quote");
        assert_eq!(sanitize("normal"), "normal");
    }

    #[test]
    fn test_pagerank_convergence() {
        // 작은 그래프에서 빠른 수렴 확인
        // A↔B, A↔C (상호 참조 구조 — dangling node 없음)
        let edges = [
            ("A".into(), "B".into()),
            ("B".into(), "A".into()),
            ("A".into(), "C".into()),
            ("C".into(), "A".into()),
        ];
        // 1e-6 tolerance로 50회 미만에 수렴해야 함
        let config = PageRankConfig {
            tolerance: 1e-6,
            ..Default::default()
        };
        let result = compute_pagerank(&edges, &config);

        // 합리적인 횟수 내에서 수렴 (damping=0.85, tolerance=1e-6 기준)
        assert!(
            result.iterations < 100,
            "Should converge within 100 iterations at tolerance=1e-6, took {}",
            result.iterations
        );
        assert!(result.final_delta < config.tolerance);

        // 모든 노드가 rank를 가져야 함
        assert!(result.scores.len() == 3, "Should have 3 nodes");
        for (name, score) in &result.scores {
            assert!(*score > 0.0, "Node {} should have positive rank", name);
        }
    }

    #[test]
    fn test_pagerank_complex_graph() {
        // 실제 코드 그래프를 모방
        // main → auth.login, main → db.connect
        // auth.login → auth.verify, auth.login → session.create
        // db.connect → db.query
        // session.create → db.query
        let edges = [
            ("main".into(), "auth.login".into()),
            ("main".into(), "db.connect".into()),
            ("auth.login".into(), "auth.verify".into()),
            ("auth.login".into(), "session.create".into()),
            ("db.connect".into(), "db.query".into()),
            ("session.create".into(), "db.query".into()),
        ];
        let config = PageRankConfig::default();
        let result = compute_pagerank(&edges, &config);

        let main = result.scores.get("main").copied().unwrap_or(0.0);
        let db_query = result.scores.get("db.query").copied().unwrap_or(0.0);
        let auth_login = result.scores.get("auth.login").copied().unwrap_or(0.0);

        // db.query가 두 곳에서 호출됨 → 높은 authority
        assert!(
            db_query > main,
            "db.query({}) should be > main({}) — it's called by 2 nodes",
            db_query,
            main
        );
        // auth.login도 중간 허브
        assert!(
            auth_login > 0.1,
            "auth.login should have meaningful rank"
        );
    }

    // ─── Phase 9 Helper Function Tests ───

    #[test]
    fn test_count_params_zero() {
        assert_eq!(count_params("foo()"), 0);
        assert_eq!(count_params("bar"), 0);
        assert_eq!(count_params(""), 0);
    }

    #[test]
    fn test_count_params_basic() {
        assert_eq!(count_params("foo(a)"), 1);
        assert_eq!(count_params("foo(a, b)"), 2);
        assert_eq!(count_params("foo(a, b, c)"), 3);
    }

    #[test]
    fn test_count_required_params() {
        assert_eq!(count_required_params("foo(a, b=10)"), 1);
        assert_eq!(count_required_params("foo(a, b, c=None)"), 2);
        assert_eq!(count_required_params("foo(a: Optional[int])"), 0);
        assert_eq!(count_required_params("foo(a: Option<i32>)"), 0);
        assert_eq!(count_required_params("foo(a, b)"), 2);
    }

    #[test]
    fn test_extract_return_type() {
        assert_eq!(extract_return_type("foo() -> int"), "int");
        assert_eq!(extract_return_type("foo() -> Vec<String>"), "Vec<String>");
        assert_eq!(extract_return_type("foo()"), "");
        assert_eq!(extract_return_type("foo() → Result<T>"), "Result<T>");
    }

    #[test]
    fn test_return_type_changed() {
        assert!(return_type_changed("foo() -> int", "foo() -> String"));
        assert!(!return_type_changed("foo() -> int", "foo(a) -> int"));
        assert!(!return_type_changed("foo()", "bar()"));
        assert!(return_type_changed("foo() -> int", "foo()"));
    }
}
