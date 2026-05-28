#!/usr/bin/env python3
"""
edit_plan 최종 벤치마크 - 10회 실행 + 실제 코드 수정 + 테스트 비교
"""

import subprocess
import json
import os
import shutil
import time
import re

FASTAPI = "/tmp/fastapi"
SI = "/home/kali/shardindex/target/release/shardindex"
RESULTS = "/tmp/benchmark/edit_plan_complete_results.json"

SCENARIOS = [
    {
        "num": 1, "name": "generate_unique_id → generate_route_id",
        "symbol": "generate_unique_id", "change": "rename",
        "file": "fastapi/utils.py",
        "edit": {"old": "def generate_unique_id(route:", "new": "def generate_route_id(route:"},
        "test": "tests/test_generate_unique_id_function.py"
    },
    {
        "num": 2, "name": "APIRoute → HttpRoute",
        "symbol": "APIRoute", "change": "rename",
        "file": "fastapi/routing.py",
        "edit": {"old": "class APIRoute(routing.Route):", "new": "class HttpRoute(routing.Route):"},
        "test": "tests/test_custom_route_class.py"
    },
    {
        "num": 3, "name": "generate_unique_id 반환 타입 변경",
        "symbol": "generate_unique_id", "change": "change_return",
        "file": "fastapi/utils.py",
        "edit": {"old": "def generate_unique_id(route: \"APIRoute\") -> str:",
                 "new": "def generate_unique_id(route: \"APIRoute\") -> str | None:"},
        "test": "tests/test_generate_unique_id_function.py"
    },
    {
        "num": 4, "name": "Depends 함수 → DependsX",
        "symbol": "Depends", "change": "rename",
        "file": "fastapi/param_functions.py",
        "edit": {"old": "def Depends(  # noqa: N802", "new": "def DependsX(  # noqa: N802"},
        "test": "tests/test_dependency_cache.py"
    },
    {
        "num": 5, "name": "Query 클래스 → QueryX",
        "symbol": "Query", "change": "rename",
        "file": "fastapi/params.py",
        "edit": {"old": "class Query(Param):  # type: ignore[misc]", "new": "class QueryX(Param):  # type: ignore[misc]"},
        "test": "tests/test_query.py"
    },
    {
        "num": 6, "name": "Body 클래스 → BodyX",
        "symbol": "Body", "change": "rename",
        "file": "fastapi/params.py",
        "edit": {"old": "class Body(FieldInfo):  # type: ignore[misc]", "new": "class BodyX(FieldInfo):  # type: ignore[misc]"},
        "test": "tests/test_union_body.py"
    },
    {
        "num": 7, "name": "Header 클래스 → HeaderX",
        "symbol": "Header", "change": "rename",
        "file": "fastapi/params.py",
        "edit": {"old": "class Header(Param):  # type: ignore[misc]", "new": "class HeaderX(Param):  # type: ignore[misc]"},
        "test": "tests/test_security_api_key_header.py"
    },
    {
        "num": 8, "name": "Path 클래스 → PathX",
        "symbol": "Path", "change": "rename",
        "file": "fastapi/params.py",
        "edit": {"old": "class Path(Param):  # type: ignore[misc]", "new": "class PathX(Param):  # type: ignore[misc]"},
        "test": "tests/test_path.py"
    },
    {
        "num": 9, "name": "Cookie 클래스 → CookieX",
        "symbol": "Cookie", "change": "rename",
        "file": "fastapi/params.py",
        "edit": {"old": "class Cookie(Param):  # type: ignore[misc]", "new": "class CookieX(Param):  # type: ignore[misc]"},
        "test": "tests/test_repeated_cookie_headers.py"
    },
    {
        "num": 10, "name": "Form 클래스 → FormX",
        "symbol": "Form", "change": "rename",
        "file": "fastapi/params.py",
        "edit": {"old": "class Form(Body):  # type: ignore[misc]", "new": "class FormX(Body):  # type: ignore[misc]"},
        "test": "tests/test_form_default.py"
    },
]

def run_cmd(cmd, cwd=FASTAPI, timeout=60):
    start = time.time()
    r = subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    elapsed = (time.time() - start) * 1000
    return r.stdout, r.stderr, r.returncode, elapsed

def parse_pytest(output):
    passed = failed = errors = 0
    m = re.search(r'(\d+) passed', output)
    if m: passed = int(m.group(1))
    m = re.search(r'(\d+) failed', output)
    if m: failed = int(m.group(1))
    m = re.search(r'(\d+) error', output)
    if m: errors = int(m.group(1))
    return passed, failed, errors

def get_impact(symbol):
    stdout, stderr, code, ms = run_cmd(f'{SI} impact "{symbol}" 2>/dev/null')
    output = stdout + stderr
    callers = 0
    refs = 0
    m = re.search(r'(\d+) callers', output)
    if m: callers = int(m.group(1))
    m = re.search(r'(\d+) refs', output)
    if m: refs = int(m.group(1))
    return callers, refs, ms

def get_neighbors(symbol):
    stdout, stderr, code, ms = run_cmd(f'{SI} neighbors "{symbol}" 2>/dev/null')
    count = stdout.count('→') + stderr.count('→')
    return count, ms

def main():
    results = []

    print("=" * 70)
    print("edit_plan 최종 벤치마크 - FastAPI v0.136.3")
    print("=" * 70)

    for sc in SCENARIOS:
        num = sc["num"]
        symbol = sc["symbol"]
        filepath = os.path.join(FASTAPI, sc["file"])
        backup = filepath + ".bak"

        print(f"\n[{num}/10] {sc['name']}")

        r = {"num": num, "name": sc["name"], "symbol": symbol, "change": sc["change"]}

        # 1. Impact analysis
        impact_start = time.time()
        callers, refs, impact_ms = get_impact(symbol)
        r["impact_callers"] = callers
        r["impact_refs"] = refs
        r["impact_total"] = callers + refs
        r["impact_ms"] = round(impact_ms, 1)

        # 2. Neighbors
        neighbors, neighbors_ms = get_neighbors(symbol)
        r["neighbors"] = neighbors

        # 3. grep 실제 참조
        grep_out, _, _, _ = run_cmd(f'grep -rn "{symbol}" --include="*.py" fastapi/ | wc -l')
        grep_count = int(grep_out.strip())
        r["grep_fastapi"] = grep_count

        # 4. Baseline test
        test_out, _, _, test_ms = run_cmd(
            f'python -m pytest {sc["test"]} -q --tb=no --no-header 2>&1', timeout=60
        )
        bp, bf, be = parse_pytest(test_out)
        r["baseline_pass"] = bp
        r["baseline_fail"] = bf
        r["baseline_errors"] = be
        r["baseline_ms"] = round(test_ms, 1)
        r["baseline_total"] = bp + bf + be

        if bp == 0 and bf == 0 and be == 0:
            print(f"  ⊘ 테스트 실행 불가: {sc['test']}")
            r["edit_applied"] = False
            r["prediction"] = "⊘ 테스트 불가"
            results.append(r)
            continue

        # 5. Apply change
        shutil.copy2(filepath, backup)
        try:
            with open(filepath, 'r') as f:
                content = f.read()

            old = sc["edit"]["old"]
            new = sc["edit"]["new"]

            if old in content:
                content = content.replace(old, new, 1)
                with open(filepath, 'w') as f:
                    f.write(content)
                r["edit_applied"] = True
            else:
                r["edit_applied"] = False
                r["edit_error"] = f"not found: {old[:60]}"
                r["prediction"] = "⊘ 수정 불가"
                results.append(r)
                continue

            # 6. Modified test
            mod_out, _, _, mod_ms = run_cmd(
                f'python -m pytest {sc["test"]} -q --tb=no --no-header 2>&1', timeout=60
            )
            mp, mf, me = parse_pytest(mod_out)
            r["modified_pass"] = mp
            r["modified_fail"] = mf
            r["modified_errors"] = me
            r["modified_ms"] = round(mod_ms, 1)
            r["test_regression"] = (mf + me) - (bf + be)
            print(f"  impact: {callers+refs}개 ({impact_ms:.0f}ms)")
            print(f"  baseline: {bp}p/{bf}f/{be}e → modified: {mp}p/{mf}f/{me}e")
            print(f"  회귀: +{r['test_regression']}")

            # 7. Prediction accuracy
            if r["impact_total"] > 0 and r["test_regression"] > 0:
                r["prediction"] = "✅ True Positive"
            elif r["impact_total"] == 0 and r["test_regression"] <= 0:
                r["prediction"] = "✅ True Negative"
            elif r["impact_total"] > 0 and r["test_regression"] <= 0:
                r["prediction"] = "⚠ False Positive"
            elif r["impact_total"] == 0 and r["test_regression"] > 0:
                r["prediction"] = "❌ False Negative"
            else:
                r["prediction"] = "❓ 불명확"
            print(f"  결과: {r['prediction']}")

        except Exception as e:
            r["error"] = str(e)[:200]
        finally:
            shutil.move(backup, filepath)

        results.append(r)

    # Save
    with open(RESULTS, 'w') as f:
        json.dump(results, f, indent=2, ensure_ascii=False)

    # Summary
    print(f"\n{'='*70}")
    print("최종 결과 요약")
    print(f"{'='*70}")

    applied = [r for r in results if r.get("edit_applied")]
    tp = sum(1 for r in applied if "True Positive" in r.get("prediction", ""))
    tn = sum(1 for r in applied if "True Negative" in r.get("prediction", ""))
    fp = sum(1 for r in applied if "False Positive" in r.get("prediction", ""))
    fn = sum(1 for r in applied if "False Negative" in r.get("prediction", ""))
    avg_impact = sum(r["impact_ms"] for r in results) / len(results) if results else 0
    total_regression = sum(r.get("test_regression", 0) for r in applied)

    total_impact = sum(r["impact_total"] for r in results)
    total_grep = sum(r["grep_fastapi"] for r in results)
    coverage = (total_impact / total_grep * 100) if total_grep > 0 else 100

    print(f"\n수정 성공: {len(applied)}/10")
    print(f"✅ True Positive: {tp}")
    print(f"✅ True Negative: {tn}")
    print(f"⚠ False Positive: {fp}")
    print(f"❌ False Negative: {fn}")
    print(f"총 테스트 회귀: {total_regression}개")
    print(f"평균 impact 시간: {avg_impact:.0f}ms")
    print(f"impact 참조: {total_impact}개 / grep 참조: {total_grep}개 = {coverage:.1f}%")
    print(f"\n상세: {RESULTS}")

    for r in results:
        print(f"  [{r['num']}] {r.get('prediction', '⊘')} | "
              f"impact={r['impact_total']}개, "
              f"grep={r['grep_fastapi']}개, "
              f"baseline={r.get('baseline_pass',0)}p/{r.get('baseline_fail',0)}f → "
              f"modified={r.get('modified_pass',0)}p/{r.get('modified_fail',0)}f, "
              f"회귀=+{r.get('test_regression',0)}")

if __name__ == "__main__":
    main()
