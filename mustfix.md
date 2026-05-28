# ShardIndex — Must Fix (필수 구현 목록)

> **생성일:** 2026-05-29
> **상태:** Cross-ref Engine — 문자열 기반 동적 참조 탐지
> **테스트:** 526 passed (263 lib + 263 bin + 17 integ + 2 doctest)
> **스키마:** v4 (4 migrations)
> **언어:** 26개 (Python, JS, TS, Rust, Go, Ruby, Java, PHP, Julia, Lua, Swift, Zig, Scala, Elixir, Dart, Haskell, C, C++, Markdown, SQL, GraphQL, Vue, CSS, Bash, Kotlin, C#)

---

## 🔴 Critical — Cross-ref Engine: 문자열 기반 동적 참조 탐지

### 문제 정의

현재 ShardIndex는 AST 기반 정적 참조만 추출함. 문자열로 표현된 동적 참조는 **전부 누락**:

- Django ORM: `FlexibleForeignKey("sentry.User")`, `ForeignKey("app.Model")`
- mock.patch: `@patch("sentry.models.User.save")`
- `__all__` export: `["User", "Project", "Team"]`
- Python late import: `importlib.import_module("sentry.auth")`
- Django settings: `AUTH_USER_MODEL = "users.CustomUser"`
- URL routing: `path("admin/", admin.site.urls)` + string-based view names

**결과:** impact recall ~50% → 85%+ 목표

---

### 아키텍처 개요

```
┌─────────────────────────────────────────────────────────────┐
│                    AST 파싱 단계                              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐   │
│  │ symbol       │  │ reference    │  │ string_literal ◀─│ NEW │
│  │ extraction   │  │ extraction   │  │ extraction       │   │
│  └──────────────┘  └──────────────┘  └──────────────────┘   │
└───────────────────────┬─────────────────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────────────────┐
│                    DB 스토리지                                │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐   │
│  │ symbol       │  │ reference    │  │ string_literals ◀│ NEW │
│  │ (기존)       │  │ (기존)       │  │ potential_str_refs│ NEW │
│  └──────────────┘  └──────────────┘  └──────────────────┘   │
└───────────────────────┬─────────────────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────────────────┐
│                 교차 매칭 엔진 (NEW)                          │
│  string_literals × symbols → potential_string_refs          │
│  confidence scoring: exact_fq(0.65) > import_scope(0.50)    │
│                     > module_scope(0.45) > unknown(0.30)    │
└───────────────────────┬─────────────────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────────────────┐
│                 쿼리 레이어 통합                               │
│  shardindex impact X --with-string-refs                     │
│  shardindex impact X --with-string-refs --min-confidence N  │
│  MCP: impact(with_string_refs: bool, min_confidence: f64)   │
└─────────────────────────────────────────────────────────────┘
```

---

### Phase 1: DB 스키마 v5 — 문자열 리터럴 + 잠재 참조 테이블

**파일:** `src/database/schema.rs`

**작업:**

1. `CURRENT_SCHEMA_VERSION` → 5
2. `MIGRATION_005_STRING_REFS` 추가
3. `MIGRATIONS` 배열에 등록

**스키마:**

```sql
-- ============================================================
-- 9. string_literals 테이블 (문자열 리터럴 수집)
-- ============================================================
CREATE TABLE IF NOT EXISTS string_literals (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path       TEXT NOT NULL REFERENCES file_hash(path) ON DELETE CASCADE,
    line_number     INTEGER NOT NULL,
    col_start       INTEGER NOT NULL,
    string_value    TEXT NOT NULL,
    is_symbol_like  INTEGER NOT NULL DEFAULT 0,
    context         TEXT,           -- "function_arg" | "sequence_element" | "assignment_rhs" | "kwarg" | "unknown"
    parent_fn       TEXT            -- enclosing function name (false positive 필터용)
);

CREATE INDEX IF NOT EXISTS idx_sl_file      ON string_literals(file_path);
CREATE INDEX IF NOT EXISTS idx_sl_value     ON string_literals(string_value);
CREATE INDEX IF NOT EXISTS idx_sl_sym_like  ON string_literals(is_symbol_like, string_value);

-- ============================================================
-- 10. potential_string_refs 테이블 (문자열 → 심볼 매칭 결과)
-- ============================================================
CREATE TABLE IF NOT EXISTS potential_string_refs (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    literal_id          INTEGER NOT NULL REFERENCES string_literals(id) ON DELETE CASCADE,
    target_symbol_id    INTEGER NOT NULL REFERENCES symbol(id) ON DELETE CASCADE,
    confidence          REAL NOT NULL,
    match_type          TEXT NOT NULL,  -- "exact_fq" | "module_scope" | "import_scope" | "method_ref"
    UNIQUE(literal_id, target_symbol_id)
);

CREATE INDEX IF NOT EXISTS idx_psr_target   ON potential_string_refs(target_symbol_id);
CREATE INDEX IF NOT EXISTS idx_psr_conf     ON potential_string_refs(confidence);
```

**검증:** `cargo test test_init_creates_all_tables` — string_literals, potential_string_refs 포함 확인

---

### Phase 2: AST 파싱 — 문자열 리터럴 수집

**파일:** `src/indexer/types.rs`, `src/indexer/python.rs` (→ 이후 다른 언어 확장)

**작업:**

#### 2-1. types.rs — ParseResult 확장

```rust
/// Extracted string literal (candidate for symbol reference)
#[derive(Debug, Clone)]
pub struct ParsedStringLiteral {
    pub value: String,
    pub line: usize,
    pub col: usize,
    pub is_symbol_like: bool,
    pub context: String,       // "function_arg" | "sequence_element" | "assignment_rhs" | "kwarg" | "unknown"
    pub parent_fn: Option<String>,  // enclosing function name
}

/// File parse result
#[derive(Debug, Clone)]
pub struct ParseResult {
    pub symbols: Vec<ParsedSymbol>,
    pub references: Vec<ParsedReference>,
    pub imports: Vec<(String, String, String)>,
    pub string_literals: Vec<ParsedStringLiteral>,  // NEW
}
```

#### 2-2. python.rs — 문자열 추출 로직 추가

**핵심 함수:**

```rust
// PythonParser impl 내부에 추가

/// 문자열 리터럴 추출 (AST walk 중에 호출)
fn extract_string_literals(
    node: &Node,
    source: &[u8],
    result: &mut ParseResult,
    parent_fn: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            if let Ok(raw) = child.utf8_text(source) {
                // f-string, b-string, r-string 제외
                if is_noise_string(raw) {
                    continue;
                }

                let inner = strip_quotes(raw);
                let is_sym_like = is_symbol_like_path(&inner);

                // docstring 위치인지 확인 (함수/클래스 첫 expression_statement)
                if is_docstring_position(node, &child) {
                    continue;
                }

                let context = infer_string_context(&child);

                result.string_literals.push(ParsedStringLiteral {
                    value: inner.to_string(),
                    line: child.start_position().row + 1,
                    col: child.start_position().column,
                    is_symbol_like: is_sym_like,
                    context,
                    parent_fn: parent_fn.map(|s| s.to_string()),
                });
            }
        }
    }
}

/// f-string, b-string, r-string, raw bytes 제외
fn is_noise_string(raw: &str) -> bool {
    let first_chars: String = raw.chars().take(2).filter(|c| *c != '"').collect();
    first_chars.starts_with('f') || first_chars.starts_with('F')
        || first_chars.starts_with('b') || first_chars.starts_with('B')
        || first_chars.starts_with('r') || first_chars.starts_with('R')
}

/// 인용부호 제거
fn strip_quotes(s: &str) -> &str {
    s.trim_matches(|c| c == '"' || c == '\'')
     .trim_start_matches(|c| c == 'r' || c == 'R')  // raw string prefix
}

/// 심볼 경로 후보인지 판단
/// "sentry.models.user.User" → true
/// "hello world" → false (공백)
/// "http://example.com" → false (슬래시)
/// "1.0.2" → false (버전 문자열)
/// "User" (대문자 시작) → true (클래스명 후보)
fn is_symbol_like_path(s: &str) -> bool {
    // 공백, 슬래시, 하이픈, 콜론 → 즉시 false
    if s.chars().any(|c| matches!(c, ' ' | '/' | '-' | ':')) {
        return false;
    }
    // 버전 문자열 패턴: "1.0.2", "v1.2"
    if let Some(first) = s.chars().next() {
        if first.is_ascii_digit() || first == 'v' || first == 'V' {
            // 숫자 시작 + 점이 있으면 버전 문자열
            if s.contains('.') {
                return false;
            }
        }
    }
    // 점으로 구분된 유효한 식별자들
    let segs: Vec<&str> = s.split('.').collect();
    if segs.len() >= 2 {
        return segs.iter().all(|seg| is_valid_identifier(seg));
    }
    // 단일 식별자: 대문자 시작이면 클래스명 후보
    if let Some(first) = s.chars().next() {
        if first.is_uppercase() && is_valid_identifier(s) {
            return true;
        }
    }
    false
}

fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// 문자열의 AST 컨텍스트 추론
fn infer_string_context(node: &Node) -> String {
    match node.parent().map(|n| n.kind()) {
        Some("argument_list")    => "function_arg".to_string(),
        Some("list")            => "sequence_element".to_string(),
        Some("tuple")           => "sequence_element".to_string(),
        Some("assignment")      => "assignment_rhs".to_string(),
        Some("keyword_argument")=> "kwarg".to_string(),
        _                       => "unknown".to_string(),
    }
}

/// docstring 위치인지 확인
fn is_docstring_position(parent: &Node, string_node: &Node) -> bool {
    // 함수/클래스의 첫 번째 statement가 expression_statement이고
    // 그 안에 string이 있으면 docstring
    if parent.kind() != "expression_statement" {
        return false;
    }
    let first_child = parent.child(0);
    first_child.map_or(false, |c| {
        c.start_position() == string_node.start_position()
    })
}
```

#### 2-3. walk_node 통합

```rust
// walk_node 내에서 string 노드 처리 추가
fn walk_node(
    node: &Node,
    source: &[u8],
    result: &mut ParseResult,
    parent: Option<String>,
    current_function: Option<String>,
) {
    // ... 기존 match 블록 ...

    // 문자열 리터럴 추출 (NEW)
    Self::extract_string_literals(node, source, result, current_function.as_deref());

    // ... 기존 재귀 ...
}
```

#### 2-4. indexer/mod.rs — DB 저장

```rust
// ProjectIndexer::index_file() 내에서 ParseResult 처리 후:

// Store string literals (NEW)
for lit in &result.string_literals {
    self.db.insert_string_literal(
        &relative,
        lit.line,
        lit.col,
        &lit.value,
        lit.is_symbol_like,
        &lit.context,
        lit.parent_fn.as_deref(),
    )?;
}
```

**파일 수정:**
- `src/indexer/types.rs` — `ParsedStringLiteral` 추가, `ParseResult` 확장
- `src/indexer/python.rs` — `extract_string_literals()` + helper 함수들
- `src/indexer/mod.rs` — `index_file()` 내에서 string_literal DB 저장
- `src/database/mod.rs` — `insert_string_literal()`, `remove_file_string_literals()`

---

### Phase 3: 교차 매칭 엔진

**파일:** `src/database/mod.rs`, `src/graph/mod.rs` (또는 새 파일 `src/cross_ref.rs`)

**작업:**

#### 3-1. DB 레이어

```rust
// src/database/mod.rs — IndexDb impl에 추가

pub fn insert_string_literal(
    &self,
    file_path: &str,
    line: usize,
    col: usize,
    value: &str,
    is_symbol_like: bool,
    context: &str,
    parent_fn: Option<&str>,
) -> Result<(), anyhow::Error> { ... }

pub fn remove_file_string_literals(&self, file_path: &str) -> Result<(), anyhow::Error> { ... }

pub fn get_symbol_like_literals(&self) -> Result<Vec<StringLiteralRecord>, anyhow::Error> { ... }

pub fn insert_potential_string_ref(
    &self,
    literal_id: i64,
    target_symbol_id: i64,
    confidence: f64,
    match_type: &str,
) -> Result<(), anyhow::Error> { ... }

pub fn get_potential_refs_for_symbol(
    &self,
    symbol_name: &str,
    min_confidence: f64,
) -> Result<Vec<PotentialStringRefRecord>, anyhow::Error> { ... }
```

#### 3-2. 매칭 엔진

```rust
// src/graph/mod.rs 또는 src/cross_ref.rs

/// 교차 매칭 실행 — 인덱싱 완료 후 또는 수동 트리거
pub fn cross_reference_strings(db: &IndexDb) -> Result<usize, anyhow::Error> {
    let candidates = db.get_symbol_like_literals()?;
    let mut matched = 0;

    for lit in &candidates {
        // noise context 필터 (false positive 감소)
        if is_noise_context(lit.parent_fn.as_deref()) {
            continue;
        }

        let symbol_matches = find_matching_symbols(db, &lit.value, &lit.file_path)?;

        for sm in symbol_matches {
            let conf = calculate_confidence(&lit.context, &sm.match_type);
            db.insert_potential_string_ref(lit.literal_id, sm.symbol_id, conf, &sm.match_type)?;
            matched += 1;
        }
    }

    tracing::info!("Cross-ref matching: {} potential refs from {} candidates", matched, candidates.len());
    Ok(matched)
}

/// 심볼 매칭 (4가지 전략)
fn find_matching_symbols(
    db: &IndexDb,
    value: &str,
    src_file: &str,
) -> Result<Vec<SymbolMatch>, anyhow::Error> {
    let mut out = Vec::new();

    // 1. Fully qualified: "sentry.models.user.User"
    if let Some(sym) = db.find_symbol_by_qualified_name(value)? {
        out.push(SymbolMatch { symbol_id: sym.id, match_type: "exact_fq" });
    }

    // 2. Module + short name: "sentry.User" → module=sentry, name=User
    if let Some((module, name)) = value.rsplit_once('.') {
        if let Ok(syms) = db.find_symbols_by_module_and_name(module, name) {
            for sym in syms {
                out.push(SymbolMatch { symbol_id: sym.id, match_type: "module_scope" });
            }
        }
    }

    // 3. mock.patch 스타일: "sentry.models.User.save" → User.save 메서드
    let segs: Vec<&str> = value.split('.').collect();
    if segs.len() >= 3 {
        let method = segs.last().unwrap();
        let class_path = segs[..segs.len()-1].join(".");
        if let Ok(syms) = db.find_symbol_and_method(&class_path, method) {
            for sym in syms {
                out.push(SymbolMatch { symbol_id: sym.id, match_type: "method_ref" });
            }
        }
    }

    // 4. Same-file import scope: "User" → 이 파일에서 import된 User
    if !value.contains('.') {
        if let Ok(syms) = db.find_imported_in_file(src_file, value) {
            for sym in syms {
                out.push(SymbolMatch { symbol_id: sym.id, match_type: "import_scope" });
            }
        }
    }

    Ok(out)
}

/// Confidence 계산
fn calculate_confidence(context: &str, match_type: &str) -> f64 {
    let base = match match_type {
        "exact_fq"     => 0.65,
        "method_ref"   => 0.60,
        "module_scope" => 0.45,
        "import_scope" => 0.50,
        _              => 0.30,
    };
    let boost = match context {
        "function_arg"     => 0.10,
        "sequence_element" => 0.05,
        "kwarg"            => 0.08,
        _                  => 0.0,
    };
    (base + boost).min(0.75)
}

/// False positive 필터
fn is_noise_context(parent_fn: Option<&str>) -> bool {
    const NOISE_PATTERNS: &[&str] = &[
        "getLogger", "logger.info", "logger.debug", "logger.error",
        "logger.warning", "print", "format", "Exception",
        "pytest.mark", "re.compile", "logging",
    ];
    match parent_fn {
        Some(fn_name) => NOISE_PATTERNS.iter().any(|p| fn_name.ends_with(p)),
        None => false,
    }
}
```

#### 3-3. 인덱싱 파이프라인 연동

```rust
// src/indexer/mod.rs — ProjectIndexer::index_all() 완료 후

pub fn index_all(&mut self) -> Result<(usize, usize, usize), anyhow::Error> {
    // ... 기존 인덱싱 ...

    // Cross-ref matching (NEW — 인덱싱 완료 후 실행)
    let string_refs = cross_reference_strings(&self.db)?;
    tracing::info!("String refs matched: {}", string_refs);

    Ok((files.len(), symbols, refs + string_refs))
}
```

---

### Phase 4: 쿼리 레이어 통합

**파일:** `src/graph/mod.rs`, `src/database/mod.rs`, `src/mcp/stdio.rs`, `src/cli/mod.rs`

#### 4-1. CLI 확장

```bash
# 기존 (정적 참조만)
shardindex impact sentry.User

# 신규 (string refs 포함)
shardindex impact sentry.User --with-string-refs
shardindex impact sentry.User --with-string-refs --min-confidence 0.4
```

**출력 예시:**

```
Impact: sentry.models.user.User
  Static refs (confidence 1.0):   277 symbols
  String refs (confidence 0.4+):  109 symbols  [--with-string-refs]
    ├─ sentry/models/project.py:42  FlexibleForeignKey("sentry.User")  [0.75]
    ├─ sentry/models/team.py:18     ForeignKey("sentry.User")           [0.75]
    └─ tests/test_user.py:91        mock.patch("sentry.models.User.save")[0.60]
  Total affected:                  386 symbols
```

#### 4-2. MCP 도구 확장

```rust
// src/mcp/stdio.rs — impact 핸들러에 파라미터 추가

// 기존:
// { "symbol": "X" }

// 신규:
// { "symbol": "X", "with_string_refs": true, "min_confidence": 0.4 }
```

#### 4-3. DB 쿼리 통합

```rust
// src/database/mod.rs — IndexDb::impact() 확장

pub fn impact_with_string_refs(
    &self,
    symbol_name: &str,
    min_confidence: f64,
) -> Result<(Vec<SymbolRecord>, Vec<ReferenceRecord>, Vec<PotentialStringRefRecord>), anyhow::Error> {
    let (callers, refs) = self.impact(symbol_name)?;
    let string_refs = self.get_potential_refs_for_symbol(symbol_name, min_confidence)?;
    Ok((callers, refs, string_refs))
}
```

---

### Phase 5: 증분 업데이트 연동

**파일:** `src/indexer/mod.rs`, `src/database/mod.rs`

**기존 인프라 활용:** dirty_queue + file watcher 이미 구현됨

**수정 사항:**

```rust
// 파일 변경 감지 → 재인덱싱 시:
// 1. 기존: remove_file_symbols() → symbols + references 삭제
// 2. 추가: remove_file_string_literals() → string_literals + potential_string_refs 삭제
// 3. 추가: 해당 파일의 string_literals 재수집 + 재매칭

pub fn remove_file_symbols(&self, path: &str) -> Result<(), anyhow::Error> {
    // ... 기존 ...
    // string_literals + potential_string_refs도 함께 삭제 (NEW)
    self.conn.execute(
        "DELETE FROM potential_string_refs WHERE literal_id IN (SELECT id FROM string_literals WHERE file_path = ?1)",
        params![path],
    )?;
    self.conn.execute(
        "DELETE FROM string_literals WHERE file_path = ?1",
        params![path],
    )?;
    Ok(())
}
```

**성능 고려사항:**
- 전체 재매칭은 무거우므로, dirty 파일의 string_literals만 선택적으로 재처리
- `cross_reference_strings()` → `cross_reference_file(file_path)`로 파일 단위 버전 추가

---

## 📊 전체 일정

| Phase | 작업 | 예상 | 파일 |
|-------|------|------|------|
| 1 | DB 스키마 v5 | 0.5일 | `src/database/schema.rs` |
| 2 | AST 문자열 수집 | 1일 | `src/indexer/types.rs`, `src/indexer/python.rs`, `src/indexer/mod.rs` |
| 3 | 교차 매칭 엔진 | 1일 | `src/database/mod.rs`, `src/graph/mod.rs` (또는 `src/cross_ref.rs`) |
| 4 | 쿼리 레이어 | 0.5일 | `src/graph/mod.rs`, `src/mcp/stdio.rs`, `src/cli/mod.rs` |
| 5 | 증분 업데이트 | 0.5일 | `src/indexer/mod.rs`, `src/database/mod.rs` |
| — | Sentry 벤치마크 재검증 | 0.5일 | — |
| **합계** | | **4일** | |

---

## 📈 기대 결과 (Sentry 기준)

| 지표 | 현재 | 구현 후 |
|------|------|---------|
| Django ORM 문자열 탐지 | 0% | ~85% |
| mock.patch 탐지 | 0% | ~80% |
| `__all__` export 탐지 | 0% | ~90% |
| 전체 impact recall | ~50% | ~88% |
| false positive | 낮음 | confidence로 분리 관리 |

---

## 🧪 테스트 계획

### Unit Tests

```rust
// src/database/schema.rs
#[test]
fn test_string_literals_table() { ... }
#[test]
fn test_potential_string_refs_table() { ... }

// src/indexer/tests.rs
#[test]
fn test_python_string_literal_extraction() { ... }
#[test]
fn test_is_symbol_like_path() { ... }
#[test]
fn test_is_noise_string() { ... }
#[test]
fn test_infer_string_context() { ... }

// src/graph/mod.rs (또는 src/cross_ref.rs)
#[test]
fn test_cross_reference_strings() { ... }
#[test]
fn test_calculate_confidence() { ... }
#[test]
fn test_is_noise_context() { ... }
```

### Integration Tests

```rust
// tests/integration/cross_ref.rs
#[test]
fn test_django_foreign_key_detection() { ... }
#[test]
fn test_mock_patch_detection() { ... }
#[test]
fn test_all_export_detection() { ... }
#[test]
fn test_impact_with_string_refs() { ... }
```

---

## ⚠️ 주의사항

1. **성능:** string_literals 테이블은 파일당 수백 개 레코드 발생 가능 → 인덱스 필수
2. **false positive:** confidence threshold는 0.4가 기본값이지만, 사용자는 `--min-confidence`로 조정 가능해야 함
3. **언어 지원:** Phase 2는 Python으로 시작 → TypeScript, JavaScript, Go 등 이후 확장
4. **DB 마이그레이션:** 기존 DB를 가진 사용자는 자동 마이그레이션 (execute_batch 안전)
5. **테스트 커버리지:** string literal 추출 → 매칭 → 쿼리 전链路 테스트 필요
6. **기존 ref_kind와의 관계:** `reference` 테이블에는 이미 `string_ref` kind가 있음 — 새로운 `potential_string_refs` 테이블과 명확히 구분 (기존은 AST에서 직접 추출한 것, 신규는 교차 매칭 결과)
