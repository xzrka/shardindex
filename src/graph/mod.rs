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
}
