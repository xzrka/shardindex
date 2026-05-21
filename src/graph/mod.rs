/// DOT graph 생성 — 호출 관계 시각화

use crate::database::IndexDb;

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
