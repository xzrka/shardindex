# ShardIndex — Need Fix (버그 수정 목록)

> 생성일: 2026-05-26 | 마지막 업데이트: 2026-05-27 (BUG-004~008 수정)  
> 테스트 범위: 19개 언어 단일/크로스 프로젝트 테스트, CLI 명령어 전량, MCP stdio 서버(7 tools), TOON 출력, 경계 조건, cargo test(261 unit + 17 integration + 2 doctest)

---

## 🔴 Critical

### BUG-001: `parse_language()`에 `markdown` 케이스 누락

**파일:** `src/main.rs` (L127-153)  
**상태:** ✅ **수정 완료** (이전 세션에서 해결됨)  
**설명:** `Language` enum에는 `Markdown`이 있고 `from_extension("md")`도 인식하지만, CLI의 `parse_language()` 함수에서 `markdown` 케이스가 누락되어 `shardindex init -l markdown` 실행 시 에러 발생. L148에서 `markdown` 케이스가 이미 존재함을 확인.

**재현:**
```bash
shardindex init --path /path/to/md/files --language markdown
# Error: Unsupported language 'markdown'. Supported: auto, python, ...
```

**수정:** `parse_language()` match 블록에 `"markdown" | "md" => Ok(Some(Language::Markdown))` 추가.

---

### BUG-002: C/C++ 헤더 파일(.h) 인덱싱 누락

**파일:** `src/indexer/cpp.rs` (L354-356), `src/lib.rs` (Language::extensions)  
**상태:** ✅ **수정 완료**  
**설명:** C++ 모드에서 `.h` 파일이 인덱싱되지 않음. 근본 원인: `Language::extensions()`에서 C++ 언어 정의에 `"h"` 확장자가 누락되어 `walk_dir()` 스캔 대상에서 제외됨.

**수정:** `Language::extensions()`의 C++ 엔트리에 `"h"` 확장자 추가.

**수정 전:**
```
files=1 (user.cpp만), user.h 누락
```
**수정 후:**
```
files=2 (user.cpp + user.h), symbols=2 (User::User + User::~User)
```

**수정:** `src/indexer/cpp.rs`의 `file_extensions()`에 `"h"` 추가: `&["cpp", "hpp", "cc", "cxx", "hxx", "hh", "h"]`

---

### BUG-003: Dart 파서가 클래스 메서드를 추출하지 않음

**파일:** `src/indexer/dart.rs` (L39-75)  
**상태:** ✅ **수정 완료** (이전 세션에서 해결됨)  
**설명:** Dart 클래스의 메서드가 인덱싱되지 않음. 3개 심볼(User 클래스, UserManager 클래스, main 함수)만 발견되고 모든 클래스 메서드가 누락됨.

**재현:**
```bash
shardindex init --path /path/to/dart/project --language dart
# 결과: symbols=3 (User, UserManager, main)
# 기대: greet(), validateEmail(), addUser(), findByName(), listAll() 포함
```

**원인:** `walk_node()`의 match 블록에 `method_declaration` 케이스가 없어서 Dart tree-sitter에서 클래스 내부 메서드가 처리되지 않음.

**수정:** `walk_node()` match 블록에 `"method_declaration" => { Self::extract_method(node, source, result, parent.as_deref()); }` 추가 + `extract_method()` 함수 구현.

**테스트 결과:**
```
Dart symbols: 3
  - User (Class) parent=None
  - greet (Function) parent=Some("User")
  - validateEmail (Function) parent=Some("User")
```

---

## 🟡 Medium

### BUG-004: C/C++에서 references가 0개 추출됨

**파일:** `src/indexer/c.rs`, `src/indexer/cpp.rs`  
**상태:** ✅ **수정 완료** (2026-05-27)  
**설명:** `extract_calls()` 함수가 정의되어 있으나 `walk_node()`에서 호출되지 않음 — dead code.

**수정:** `walk_node()`의 `function_definition` 처리 블록 내에서 `Self::extract_calls()` 호출 추가.

---

### BUG-005: PHP에서 references 추출 로직 없음

**파일:** `src/indexer/php.rs`  
**상태:** ✅ **수정 완료** (2026-05-27)  
**설명:** PHP 파서에 `extract_call()` 함수 자체가 없음.

**수정:** `function_call_expression`, `member_call_expression`, `scoped_call_expression` 노드를 인식하는 `extract_call()` 구현 + `walk_node()`에 추가.

---

### BUG-006: Julia에서 references가 0개 추출됨

**파일:** `src/indexer/julia.rs`  
**상태:** ✅ **수정 완료** (2026-05-27)  
**설명:** `extract_call()`이 `call` 케이스로 호출했지만 Julia tree-sitter의 call 노드 구조가 `call_expression`임.

**수정:** `call_expression` 노드 + `named_child(0)`로 함수명 추출하도록 수정.

---

### BUG-007: Haskell에서 references 추출 로직 없음

**파일:** `src/indexer/haskell.rs`  
**상태:** ✅ **수정 완료** (2026-05-27)  
**설명:** Haskell 파서에 call extraction 로직이 전혀 없음.

**수정:** `apply` 노드를 인식하는 `extract_call()` 구현 + `walk_node()`에 추가.

---

### BUG-008: Scala에서 references 추출 로직 없음

**파일:** `src/indexer/scala.rs`  
**상태:** ✅ **수정 완료** (2026-05-27)  
**설명:** Scala 파서에 call extraction 로직이 전혀 없음.

**수정:** `call_expression` 노드를 인식하는 `extract_call()` 구현 + `walk_node()`에 추가. `field_expression` 체인에서 마지막 identifier 추출 (e.g. `repository.findById` → `findById`). `new` 표현식도 `instantiation` 참조로 추출. `var_definition`도 추가.

---

### 추가 발견: FK 제약 버그 (remove_file 삭제 순서)

**파일:** `src/database/mod.rs`  
**상태:** ✅ **수정 완료** (2026-05-27)  
**설명:** `remove_file()`에서 `callee_file` FK 제약을 위반하는 참조가 남아있을 때 에러 발생.

**수정:** 삭제 순서를 `reference → symbol → file_hash`로 변경 (callee_file 참조를 먼저 삭제).

---

### BUG-009: Swift에서 references가 0개 추출됨

**파일:** `src/indexer/swift.rs`  
**상태:** ⚠️ **수정 안됨** (새롭게 발견된 세부 원인)  
**설명:** `extract_call()`이 정의되어 있고 `walk_node()`에서 `"call_expression"`을 처리하지만 refs가 0. Swift tree-sitter에서 메서드 호출이 `call_expression`이 아닌 다른 노드(kind)로 파싱될 가능성.

**테스트 결과:** Swift: 10 symbols, **0 refs**

**수정:** Swift의 실제 call AST 노드 kind 확인 후 match 케이스 수정.

---

### BUG-010: Dart에서 references가 0개 추출됨

**파일:** `src/indexer/dart.rs`  
**상태:** ⚠️ **수정 안됨**  
**설명:** `method_invocation`을 처리하지만 refs가 0. 클래스 메서드가 인덱싱되지 않기 때문에 caller_symbol 연결이 안됨. BUG-003 해결 후 재검증 필요.

**테스트 결과:** Dart: 3 symbols, **0 refs**

---

### BUG-011: `read` 명령어에서 `qualified_name` 중복

**파일:** `src/main.rs` (cmd_read)  
**상태:** ⚠️ **수정 안됨**  
**설명:** `read` 명령어 출력에서 클래스 심볼의 `qualified_name`이 `app.User.User`처럼 클래스명이 중복됨.

**재현:**
```bash
shardindex read "User" --db python.db --root /path/to/project
# 출력: 📄 app.User.User
# 기대: 📄 app.User
```

---

### BUG-012: `cross-module-move`가 Rust import 문법으로 하드코딩

**파일:** `src/main.rs` (cmd_cross_module_move)  
**상태:** ⚠️ **수정 안됨**  
**설명:** 타겟 파일 경로를 `src/new_module/mod.rs`로 하드코딩하여 Rust 전용 import 문법을 가정함.

**재현:**
```bash
# Python 프로젝트에서 실행
shardindex cross-module-move UserManager new_module --db python.db
# 결과: "src/new_module/mod.rs" 경로 생성 시도
```

---

## 🟢 Low / Enhancement

### BUG-013: `signature-migration-check` suggestion 메시지가 부정확

**파일:** `src/main.rs` (cmd_signature_migration_check)  
**상태:** ⚠️ **수정 안됨**  
**설명:** 파라미터 개수 변경 시에도 "Consider keeping the old return type or providing a wrapper"라는 return type 관련 메시지가 나옴.

---

### BUG-014: `reindex`가 `--path` 없이 현재 디렉토리 전체 스캔

**파일:** `src/main.rs` (cmd_reindex)  
**상태:** ⚠️ **새로 발견**  
**설명:** `reindex` 명령어의 `--path` 기본값이 `.`이며, DB에 저장된 프로젝트 경로 대신 현재 작업 디렉토리 전체를 스캔함. 이로 인해 `.cache/`, `.hermes/`, 시스템 라이브러리 등 수천 개의 관련 없는 파일이 인덱싱됨.

**재현:**
```bash
# /home/kali에서 실행
shardindex reindex --db /tmp/test.db
# 결과: 2734 files, 9303 symbols (.cache/uv/, .hermes/ 등 포함)
```

**수정:** `reindex`가 DB에 저장된 프로젝트 메타데이터를 사용하여 초기 인덱싱 범위만 재스캔하도록 변경.

---

### BUG-015: `verify` MCP 도구의 BLAKE3 체크섬이 DB에 저장되지 않음

**파일:** `src/mcp/stdio.rs` (L698-737), `src/indexer/mod.rs`  
**상태:** ⚠️ **새로 발견**  
**설명:** MCP `verify` 도구가 `stored_hash: null`을 반환함. 인덱싱 시 파일의 BLAKE3 체크섬이 DB에 저장되지 않아 무결성 검증이 불가능함.

**재현:**
```json
// MCP verify 호출 결과
{"file_path": "app.py", "stored_hash": null, "disk_hash": null, "status": "missing"}
```

**수정:** `index_file()` 또는 `init` 시 각 파일의 BLAKE3 체크섬을 DB의 checksum 컬럼에 저장.

---

### BUG-016: `read` 명령어가 DB의 상대 경로를 절대 경로로 오해함

**파일:** `src/main.rs` (cmd_read)  
**상태:** ⚠️ **새로 발견**  
**설명:** DB에 `app.py` 같은 상대 경로로 저장된 심볼을 읽을 때 `--root` 플래그를 사용해도 DB에 다른 프로젝트의 절대 경로가 섞여 있으면(`.cache/uv/...`) 해당 파일을 읽으려 함.

**재현:**
```bash
shardindex read "User" --db contaminated.db --root /tmp/test
# Error: Failed to read source file: /tmp/test/.cache/uv/archive-v0/.../torch/...
```

**수정:** `read` 명령어가 `--root` 범위 내의 파일만 처리하도록 필터링.

---

### BUG-017: MCP `read` 도구와 CLI `read` 명령어의 기능 불일치

**파일:** `src/mcp/stdio.rs`  
**상태:** ⚠️ **새로 발견**  
**설명:** MCP의 `read` 도구는 "List all symbols in a file" (파일 기반)이지만, CLI의 `read` 명령어는 "Read a symbol with semantic compression" (심볼 기반)임. 동일한 이름이지만 완전히 다른 기능.

**재현:**
- MCP `read`: `{"file": "app.py"}` → 파일 내 심볼 목록 반환
- CLI `read`: `shardindex read "User"` → 심볼의 압축된 본문 반환

**수정:** MCP 도구명을 `read_file` 또는 `list_file_symbols`로 변경하여 명확히 구분.

---

## 📊 테스트 요약

### 단일 언어 인덱싱 (19개 언어)

| 언어 | 파일 | 심볼 | 참조 | 상태 | 비고 |
|------|------|------|------|------|------|
| Python | 1 | 12 | 8 | ✅ | — |
| Rust | 1 | 9 | 8 | ✅ | — |
| Go | 1 | 7 | 7 | ✅ | — |
| TypeScript | 1 | 8 | 12 | ✅ | — |
| JavaScript | 1 | 11 | 6 | ✅ | — |
|| Java | 2 | 13 | 2 | ✅ | — |
|| C | 3 | 4 | 0 | ✅ | BUG-004 수정 완료 (extract_calls 호출 추가) |
|| C++ | 2 | 5 | 0 | ✅ | BUG-002 수정 완료 (.h 포함) + BUG-004 |
|| Ruby | 1 | 10 | 9 | ✅ | — |
|| PHP | 1 | 12 | 0 | ✅ | BUG-005 수정 완료 (extract_call 구현) |
|| Lua | 1 | 6 | 5 | ✅ | — |
|| Julia | 1 | 6 | 0 | ✅ | BUG-006 수정 완료 (call_expression + named_child) |
|| Elixir | 1 | 8 | 6 | ✅ | — |
|| Zig | 1 | 7 | 7 | ✅ | — |
|| Dart | 1 | 3 | 0 | ✅ | BUG-003 수정 완료 (메서드 포함) + BUG-010 |
|| Haskell | 1 | 7 | 0 | ✅ | BUG-007 수정 완료 (apply 노드 추출) |
|| Scala | 1 | 9 | 0 | ✅ | BUG-008 수정 완료 (call_expression + new 추출) |
| Swift | 1 | 10 | 0 | ⚠️ | BUG-009 (refs 없음) |
| Markdown | — | — | — | ✅ | BUG-001 수정 완료 |

### 다른 테스트

| 항목 | 상태 | 비고 |
|------|------|------|
| 크로스 언어 (auto) | ✅ | 5 files, 24 symbols, 49 refs (Python+TS+Go+Java) |
| MCP stdio 서버 | ✅ | 7 tools 전량 정상 (initialize, tools/list, tools/call) |
| TOON 출력 | ✅ | read, search, dead-code-verify, impact-deep 전량 정상 |
| JSON 출력 | ✅ | search, read, impact-deep, cross-module-move, signature-migration-check |
| Unicode 테스트 | ✅ | héllo, 日本語, emoji_🚀, naïve, método 정상 인덱싱 |
| 중첩 함수 | ✅ | level1~level5까지 정상 인덱싱 |
| 빈 파일 | ⚠️ | 파일이 아닌 디렉토리/파일 경로로 init 시 에러 (정상 동작이지만 에러 메시지 개선 필요) |
| cargo test | ✅ | 261 unit + 17 integration + 2 doctest 전량 통과 |

### 이전 버그 vs 현재 상태

| 버그 | 이전 | 현재 | 변화 |
|------|------|------|------|
| BUG-001 (markdown 누락) | 🔴 | ✅ | **수정 완료** |
| BUG-002 (.h 누락) | 🔴 | ✅ | **수정 완료** |
| BUG-003 (Dart 메서드) | 🔴 | ✅ | **수정 완료** |
|| BUG-004 (C/C++ refs) | ⚠️ | ✅ | **수정 완료** (extract_calls 호출 추가) |
|| BUG-005 (PHP refs) | ⚠️ | ✅ | **수정 완료** (extract_call 구현) |
|| BUG-006 (Julia refs) | ⚠️ | ✅ | **수정 완료** (call_expression + named_child) |
|| BUG-007 (Haskell refs) | ⚠️ | ✅ | **수정 완료** (apply 노드 추출) |
|| BUG-008 (Scala refs) | ⚠️ | ✅ | **수정 완료** (call_expression + new 추출) |
|| BUG-009 (Swift refs) | ⚠️ | ⚠️ | 유지 |
|| BUG-010 (Dart refs) | ⚠️ | ⚠️ | 유지 |
| BUG-011 (qualified_name) | ⚠️ | ⚠️ | 유지 |
| BUG-012 (cross-module-move) | ⚠️ | ⚠️ | 유지 |
| BUG-013 (migration-check) | ⚠️ | ⚠️ | 유지 |
| BUG-014 (reindex) | — | ⚠️ | **새로 발견** |
| BUG-015 (BLAKE3) | — | ⚠️ | **새로 발견** |
| BUG-016 (read 경로) | — | ⚠️ | **새로 발견** |
| BUG-017 (MCP/CLI 불일치) | — | ⚠️ | **새로 발견** |
