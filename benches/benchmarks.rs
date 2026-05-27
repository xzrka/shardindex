// Phase 12 — Performance Benchmarks
// Masterplan §12: Performance Targets & Benchmarks
//
// Bench targets:
//   bench_cold_index_200k_python  < 30s
//   bench_incremental_single_file < 50ms
//   bench_impact_depth_2          < 5ms
//   bench_hash_verify             < 1ms
//   bench_search_semantic         < 10ms

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use shardindex::database::{IndexDb, ReferenceRecord, SymbolRecord};
use shardindex::indexer::{Language, ProjectIndexer};
use shardindex::integrity::IntegrityGuard;
use shardindex::search::{SearchConfig, advanced_search};

// ---------------------------------------------------------------------------
// Helpers — synthetic data generators
// ---------------------------------------------------------------------------

/// Generate a Python file with ~N lines of realistic-looking code.
fn generate_python_loc(lines: usize) -> String {
    let mut buf = String::with_capacity(lines * 40);
    buf.push_str("# Auto-generated test module\n");
    buf.push_str("import os, sys, json\n\n");

    for i in 0..(lines / 10) {
        buf.push_str(&format!(
            "def function_{i}(\n    arg_a,\n    arg_b,\n    arg_c=None,\n):\n"
        ));
        buf.push_str("    \"\"\"Docstring for function.\"\"\"\n");
        buf.push_str("    result = arg_a + arg_b\n");
        buf.push_str("    if arg_c is not None:\n");
        buf.push_str("        result += arg_c\n");
        buf.push_str("    return result\n\n");
    }

    // Pad remaining lines with comments
    let current = buf.lines().count();
    for _ in current..lines {
        buf.push_str("# padding line\n");
    }
    buf
}

/// Generate a single large Python file (~500 lines).
fn generate_single_python_file() -> String {
    generate_python_loc(500)
}

/// Insert a batch of symbols into an in-memory DB.
fn populate_db(db: &mut IndexDb, count: usize) {
    for i in 0..count {
        let symbol = SymbolRecord {
            id: 0,
            file_path: format!("module_{:03}.py", i % 50),
            name: format!("func_{i}"),
            kind: "function".into(),
            start_line: i * 10 + 1,
            end_line: i * 10 + 10,
            start_col: 0,
            end_col: 4,
            signature: Some(format!("def func_{i}(): pass")),
            docstring: None,
            parent_symbol: None,
            qualified_name: format!("module_{}.func_{i}", i % 50),
            token_count: 15,
        };
        db.insert_symbol(&symbol).ok();
    }
}

/// Insert references for impact analysis: chain of callers → callees.
fn populate_refs(db: &mut IndexDb, count: usize) {
    for i in 0..count.saturating_sub(1) {
        let ref_rec = ReferenceRecord {
            id: 0,
            caller_file: format!("module_{:03}.py", i % 50),
            callee_file: format!("module_{:03}.py", (i + 1) % 50),
            caller_symbol: Some(format!("func_{i}")),
            callee_symbol: format!("func_{}", i + 1),
            ref_kind: "call".into(),
            line: i * 10 + 5,
            confidence: 1.0,
            is_dynamic: false,
        };
        db.insert_reference(&ref_rec).ok();
    }
}

// ---------------------------------------------------------------------------
// Benchmark: Cold Index 200K LOC (Python)
// ---------------------------------------------------------------------------

fn bench_cold_index_200k_python(c: &mut Criterion) {
    let mut group = c.benchmark_group("cold_index");
    group.sample_size(10); // Reduced: 200K LOC indexing takes a while

    // Generate ~200K LOC across multiple "files"
    let total_loc = 200_000usize;
    let files_per_batch = 20;
    let loc_per_file = total_loc / files_per_batch; // 10,000 lines each

    group.bench_function("200k_loc_python", |b| {
        b.iter(|| {
            let db = IndexDb::open_in_memory().expect("in-memory db");
            let tmpdir =
                std::env::temp_dir().join(format!("shardindex_bench_{}", std::process::id()));
            std::fs::create_dir_all(&tmpdir).ok();

            let mut indexer =
                ProjectIndexer::new(db, tmpdir.clone(), Language::Python).expect("indexer");

            // Write synthetic files and index
            for f in 0..files_per_batch {
                let content = generate_python_loc(loc_per_file);
                let path = tmpdir.join(format!("module_{:03}.py", f));
                std::fs::write(&path, content).ok();
                indexer.index_file(&path).ok();
            }

            drop(indexer);
            std::fs::remove_dir_all(&tmpdir).ok();
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: Incremental Single File
// ---------------------------------------------------------------------------

fn bench_incremental_single_file(c: &mut Criterion) {
    let mut group = c.benchmark_group("incremental");

    let content = generate_single_python_file();
    let tmpdir =
        std::env::temp_dir().join(format!("shardindex_incr_bench_{}", std::process::id()));
    std::fs::create_dir_all(&tmpdir).ok();
    let file_path = tmpdir.join("target.py");
    std::fs::write(&file_path, &content).ok();

    group.bench_function("single_file_500loc", |b| {
        b.iter(|| {
            let db = IndexDb::open_in_memory().expect("in-memory db");
            let mut indexer =
                ProjectIndexer::new(db, tmpdir.clone(), Language::Python).expect("indexer");
            indexer.index_file(black_box(&file_path))
        })
    });

    std::fs::remove_dir_all(&tmpdir).ok();
    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: Impact Analysis (depth=2)
// ---------------------------------------------------------------------------

fn bench_impact_depth_2(c: &mut Criterion) {
    let mut group = c.benchmark_group("impact");

    // Pre-populate DB with 500 symbols and references
    let mut db = IndexDb::open_in_memory().expect("in-memory db");
    populate_db(&mut db, 500);
    populate_refs(&mut db, 500);

    group.bench_function("depth_2_500symbols", |b| {
        b.iter(|| {
            // Use graph::impact_deep — BFS depth=2
            shardindex::graph::impact_deep(
                &db,
                black_box("func_100"),
                2,
                false, // include_tests
                false, // include_dynamic
                false, // risk_analysis
                None,  // token_budget
            )
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: Hash Verify (Blake3)
// ---------------------------------------------------------------------------

fn bench_hash_verify(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_verify");

    let content = generate_python_loc(1000); // ~1000 lines
    let tmpdir =
        std::env::temp_dir().join(format!("shardindex_hash_bench_{}", std::process::id()));
    std::fs::create_dir_all(&tmpdir).ok();
    let file_path = tmpdir.join("hash_target.py");
    std::fs::write(&file_path, &content).ok();

    group.bench_function("blake3_1000loc", |b| {
        b.iter_custom(|iters| {
            let mut total_time = std::time::Duration::ZERO;
            for _ in 0..iters {
                // Create fresh indexer + index file for each iteration
                let db = IndexDb::open_in_memory().expect("in-memory db");
                let mut indexer =
                    ProjectIndexer::new(db, tmpdir.clone(), Language::Python).expect("indexer");
                indexer.index_file(&file_path).ok();

                // Access db through the indexer — use integrity check on the indexed file
                let start = std::time::Instant::now();
                let _ = IntegrityGuard::compute_file_hash(&file_path);
                total_time += start.elapsed();
            }
            total_time
        })
    });

    std::fs::remove_dir_all(&tmpdir).ok();
    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: Semantic Search (fuzzy + ranked)
// ---------------------------------------------------------------------------

fn bench_search_semantic(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_semantic");

    let mut db = IndexDb::open_in_memory().expect("in-memory db");
    populate_db(&mut db, 1000);

    group.bench_function("fuzzy_1000symbols", |b| {
        b.iter(|| {
            advanced_search(
                &db,
                black_box("func_100"),
                None, // extension_filter
                &SearchConfig::default(),
            )
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion groups
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_cold_index_200k_python,
    bench_incremental_single_file,
    bench_impact_depth_2,
    bench_hash_verify,
    bench_search_semantic,
);
criterion_main!(benches);
