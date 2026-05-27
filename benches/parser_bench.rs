use criterion::{Criterion, black_box, criterion_group, criterion_main};
use shardindex::indexer::{
    GoParser, JavaScriptParser, PythonParser, RustParser, SourceCodeParser, TypeScriptParser,
};

fn benchmark_python_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/python");

    let small_fn = r#"
def add(a, b):
    return a + b
"#;

    let medium_fn = r#"
def calculate_fibonacci(n):
    if n <= 0:
        return []
    elif n == 1:
        return [0]

    fib_sequence = [0, 1]
    for i in range(2, n):
        next_val = fib_sequence[i-1] + fib_sequence[i-2]
        fib_sequence.append(next_val)

    return fib_sequence
"#;

    let large_fn = r#"
def process_data(data):
    results = []
    for item in data:
        if item.get('type') == 'numeric':
            value = float(item['value'])
            if value > 0:
                results.append({
                    'original': item,
                    'processed': value * 2,
                    'category': 'positive'
                })
            else:
                results.append({
                    'original': item,
                    'processed': abs(value),
                    'category': 'negative'
                })
        elif item.get('type') == 'string':
            results.append({
                'original': item,
                'processed': item['value'].upper(),
                'category': 'text'
            })
    return results
"#;

    let class_def = r#"
class DataProcessor:
    def __init__(self, config):
        self.config = config
        self.results = []

    def process(self, data):
        for item in data:
            result = self.transform(item)
            self.results.append(result)
        return self.results

    def transform(self, item):
        return {
            'id': item.get('id'),
            'value': item.get('value', 0) * self.config.get('multiplier', 1),
            'status': 'processed'
        }
"#;

    group.bench_function("small_function", |b| {
        b.iter(|| {
            let mut parser = PythonParser::new().unwrap();
            parser.parse(black_box(small_fn))
        })
    });

    group.bench_function("medium_function", |b| {
        b.iter(|| {
            let mut parser = PythonParser::new().unwrap();
            parser.parse(black_box(medium_fn))
        })
    });

    group.bench_function("large_function", |b| {
        b.iter(|| {
            let mut parser = PythonParser::new().unwrap();
            parser.parse(black_box(large_fn))
        })
    });

    group.bench_function("class_definition", |b| {
        b.iter(|| {
            let mut parser = PythonParser::new().unwrap();
            parser.parse(black_box(class_def))
        })
    });

    group.finish();
}

fn benchmark_javascript_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/javascript");

    let js_fn = r#"
function calculateFactorial(n) {
    if (n <= 1) return 1;
    let result = 1;
    for (let i = 2; i <= n; i++) {
        result *= i;
    }
    return result;
}
"#;

    let js_class = r#"
class EventEmitter {
    constructor() {
        this.listeners = new Map();
    }

    on(event, callback) {
        if (!this.listeners.has(event)) {
            this.listeners.set(event, []);
        }
        this.listeners.get(event).push(callback);
    }

    emit(event, ...args) {
        const callbacks = this.listeners.get(event) || [];
        for (const callback of callbacks) {
            callback(...args);
        }
    }
}
"#;

    group.bench_function("function", |b| {
        b.iter(|| {
            let mut parser = JavaScriptParser::new().unwrap();
            parser.parse(black_box(js_fn))
        })
    });

    group.bench_function("class", |b| {
        b.iter(|| {
            let mut parser = JavaScriptParser::new().unwrap();
            parser.parse(black_box(js_class))
        })
    });

    group.finish();
}

fn benchmark_rust_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/rust");

    let rust_fn = r#"
fn fibonacci(n: u32) -> Vec<u32> {
    if n == 0 { return vec![]; }
    if n == 1 { return vec![0]; }

    let mut seq = vec![0, 1];
    for i in 2..n {
        let next = seq[i as usize - 1] + seq[i as usize - 2];
        seq.push(next);
    }
    seq
}
"#;

    let rust_struct = r#"
struct User {
    id: u64,
    name: String,
    email: String,
    active: bool,
}

impl User {
    fn new(id: u64, name: &str, email: &str) -> Self {
        Self {
            id,
            name: name.to_string(),
            email: email.to_string(),
            active: true,
        }
    }

    fn deactivate(&mut self) {
        self.active = false;
    }
}
"#;

    group.bench_function("function", |b| {
        b.iter(|| {
            let mut parser = RustParser::new().unwrap();
            parser.parse(black_box(rust_fn))
        })
    });

    group.bench_function("struct_with_impl", |b| {
        b.iter(|| {
            let mut parser = RustParser::new().unwrap();
            parser.parse(black_box(rust_struct))
        })
    });

    group.finish();
}

fn benchmark_typescript_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/typescript");

    let ts_interface = r#"
interface User {
    id: number;
    name: string;
    email: string;
    isActive: boolean;
}

class UserService {
    private users: Map<number, User>;

    constructor() {
        this.users = new Map();
    }

    addUser(user: User): void {
        this.users.set(user.id, user);
    }

    getUser(id: number): User | undefined {
        return this.users.get(id);
    }
}
"#;

    group.bench_function("interface_and_class", |b| {
        b.iter(|| {
            let mut parser = TypeScriptParser::new().unwrap();
            parser.parse(black_box(ts_interface))
        })
    });

    group.finish();
}

fn benchmark_go_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/go");

    let go_fn = r#"
package main

func fibonacci(n int) []int {
    if n <= 0 {
        return []int{}
    }
    if n == 1 {
        return []int{0}
    }

    seq := []int{0, 1}
    for i := 2; i < n; i++ {
        next := seq[i-1] + seq[i-2]
        seq = append(seq, next)
    }
    return seq
}
"#;

    group.bench_function("function", |b| {
        b.iter(|| {
            let mut parser = GoParser::new().unwrap();
            parser.parse(black_box(go_fn))
        })
    });

    group.finish();
}

fn benchmark_all_parsers(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/comparison");

    let python_code = r#"
def process(items):
    results = []
    for item in items:
        if item > 0:
            results.append(item * 2)
    return results
"#;

    let js_code = r#"
function process(items) {
    const results = [];
    for (const item of items) {
        if (item > 0) {
            results.push(item * 2);
        }
    }
    return results;
}
"#;

    let rust_code = r#"
fn process(items: &[i32]) -> Vec<i32> {
    let mut results = Vec::new();
    for &item in items {
        if item > 0 {
            results.push(item * 2);
        }
    }
    results
}
"#;

    let ts_code = r#"
function process(items: number[]): number[] {
    const results: number[] = [];
    for (const item of items) {
        if (item > 0) {
            results.push(item * 2);
        }
    }
    return results;
}
"#;

    let go_code = r#"
func process(items []int) []int {
    results := []int{}
    for _, item := range items {
        if item > 0 {
            results = append(results, item*2)
        }
    }
    return results
}
"#;

    group.bench_function("python", |b| {
        b.iter(|| {
            let mut parser = PythonParser::new().unwrap();
            parser.parse(black_box(python_code))
        })
    });

    group.bench_function("javascript", |b| {
        b.iter(|| {
            let mut parser = JavaScriptParser::new().unwrap();
            parser.parse(black_box(js_code))
        })
    });

    group.bench_function("rust", |b| {
        b.iter(|| {
            let mut parser = RustParser::new().unwrap();
            parser.parse(black_box(rust_code))
        })
    });

    group.bench_function("typescript", |b| {
        b.iter(|| {
            let mut parser = TypeScriptParser::new().unwrap();
            parser.parse(black_box(ts_code))
        })
    });

    group.bench_function("go", |b| {
        b.iter(|| {
            let mut parser = GoParser::new().unwrap();
            parser.parse(black_box(go_code))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_python_parser,
    benchmark_javascript_parser,
    benchmark_rust_parser,
    benchmark_typescript_parser,
    benchmark_go_parser,
    benchmark_all_parsers,
);
criterion_main!(benches);
