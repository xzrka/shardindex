use criterion::{black_box, criterion_group, criterion_main, Criterion};
use shardindex::database::{IndexDb, ReferenceRecord, SymbolRecord};

fn make_symbol(name: &str) -> SymbolRecord {
    SymbolRecord {
        id: 0,
        file_path: "test.py".to_string(),
        name: name.to_string(),
        kind: "function".to_string(),
        start_line: 1,
        end_line: 10,
        start_col: 0,
        end_col: 4,
        signature: Some(format!("def {}(): pass", name)),
        docstring: None,
        parent_symbol: None,
        qualified_name: format!("test.{}", name),
        token_count: 10,
    }
}

fn benchmark_db_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("database/insert");

    let mut db = IndexDb::open_in_memory().expect("Failed to create in-memory DB");

    group.bench_function("single_symbol", |b| {
        b.iter(|| {
            let symbol = make_symbol("test_function");
            let _ = db.insert_symbol(black_box(&symbol));
        })
    });

    group.bench_function("batch_symbols_100", |b| {
        b.iter(|| {
            for i in 0..100 {
                let symbol = make_symbol(&format!("function_{}", i));
                let _ = db.insert_symbol(black_box(&symbol));
            }
        })
    });

    group.finish();
}

fn benchmark_db_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("database/search");

    let mut db = IndexDb::open_in_memory().expect("Failed to create in-memory DB");

    // Insert test data
    for i in 0..1000 {
        let symbol = make_symbol(&format!("function_{}", i));
        let _ = db.insert_symbol(&symbol);
    }

    group.bench_function("search_by_pattern", |b| {
        b.iter(|| {
            let _ = db.search_symbol(black_box("function_500"));
        })
    });

    group.bench_function("search_ranked", |b| {
        b.iter(|| {
            let _ = db.search_symbol_ranked(black_box("function"));
        })
    });

    group.finish();
}

fn benchmark_db_neighbors(c: &mut Criterion) {
    let mut group = c.benchmark_group("database/neighbors");

    let mut db = IndexDb::open_in_memory().expect("Failed to create in-memory DB");

    // Insert symbols
    for i in 0..100 {
        let symbol = make_symbol(&format!("func_{}", i));
        let _ = db.insert_symbol(&symbol);
    }

    // Insert references
    for i in 0..99 {
        let reference = ReferenceRecord {
            id: 0,
            caller_file: "test.py".to_string(),
            callee_file: "test.py".to_string(),
            caller_symbol: Some(format!("func_{}", i)),
            callee_symbol: format!("func_{}", i + 1),
            ref_kind: "call".to_string(),
            line: 1,
            confidence: 1.0,
            is_dynamic: false,
        };
        let _ = db.insert_reference(black_box(&reference));
    }

    group.bench_function("neighbors_single", |b| {
        b.iter(|| {
            let _ = db.neighbors(black_box("func_50"));
        })
    });

    group.finish();
}

fn benchmark_cache_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("database/cache");

    let db = IndexDb::open_in_memory().expect("Failed to create in-memory DB");
    use shardindex::agent_cache::AgentCache;
    let cache = AgentCache::with_db(db);

    group.bench_function("cache_set", |b| {
        b.iter(|| {
            let _ = cache.set(
                "read",
                black_box(&serde_json::json!({"file": "test.py"})),
                black_box(r#"{"result": "test"}"#),
                None,
            );
        })
    });

    group.bench_function("cache_get", |b| {
        b.iter(|| {
            let _ = cache.get("read", black_box(&serde_json::json!({"file": "test.py"})));
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_db_insert,
    benchmark_db_search,
    benchmark_db_neighbors,
    benchmark_cache_operations,
);
criterion_main!(benches);
