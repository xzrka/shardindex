#!/usr/bin/env python3
"""
edit_plan benchmark: 10회 실행 + 실제 테스트 비교

각 시나리오:
1. edit_plan으로 영향 분석 예측
2. 실제 코드 수정
3. pytest로 테스트 실행 (수정 전 vs 수정 후)
4. 결과 비교: edit_plan 예측 vs 실제 테스트 실패
"""

import json
import subprocess
import shutil
import os
import time
from dataclasses import dataclass, field
from typing import Optional

FASTAPI_DIR = "/tmp/fastapi"
SHARDINDEX = "/home/kali/shardindex/target/release/shardindex"
RESULTS_FILE = "/tmp/benchmark/edit_plan_results.json"
BASELINE_RESULTS = "/tmp/benchmark/baseline_test_results.txt"

@dataclass
class Scenario:
    num: int
    name: str
    change_type: str  # rename, add_param, remove_param, change_return
    symbol: str
    new_signature: Optional[str] = None  # for signature_migration_check
    changes: list = field(default_factory=list)  # for edit_plan
    file_path: str = ""
    description: str = ""
    test_subset: str = ""  # pytest test file/pattern to run

@dataclass
class Result:
    scenario: str
    edit_plan_impact: list = field(default_factory=list)
    edit_plan_impact_count: int = 0
    edit_plan_time_ms: float = 0
    actual_test_failures: int = 0
    baseline_test_pass: int = 0
    modified_test_pass: int = 0
    prediction_accuracy: str = ""
    notes: str = ""

def run_cmd(cmd, cwd=FASTAPI_DIR, timeout=120):
    """Run command and return (stdout, stderr, exit_code, elapsed_ms)"""
    start = time.time()
    result = subprocess.run(
        cmd, shell=True, cwd=cwd, capture_output=True, text=True, timeout=timeout
    )
    elapsed = (time.time() - start) * 1000
    return result.stdout, result.stderr, result.returncode, elapsed

def run_shardindex_cli(args):
    """Run shardindex CLI command"""
    cmd = f"{SHARDINDEX} {args}"
    return run_cmd(cmd, cwd=FASTAPI_DIR)

def run_shardindex_mcp(method, params):
    """Run shardindex MCP tool via CLI"""
    # Use CLI equivalent for edit_plan
    if method == "edit_plan":
        symbol = params.get("symbol", "")
        changes_json = json.dumps(params.get("changes", []))
        cmd = f'{SHARDINDEX} edit-plan --symbol "{symbol}" --changes "{changes_json}"'
        stdout, stderr, code, elapsed = run_cmd(cmd, cwd=FASTAPI_DIR)
        try:
            return json.loads(stdout) if stdout.strip() else {}, elapsed
        except json.JSONDecodeError:
            return {"raw": stdout.strip(), "error": stderr.strip()}, elapsed
    return {}, 0

def run_pytest(test_pattern, timeout=120):
    """Run pytest and return (passed, failed, errors, elapsed_ms)"""
    cmd = f"python -m pytest {test_pattern} --tb=no -q --no-header 2>&1"
    stdout, stderr, code, elapsed = run_cmd(cmd, timeout=timeout)
    output = stdout + stderr
    
    # Parse pytest output
    passed = 0
    failed = 0
    errors = 0
    for line in output.split('\n'):
        if ' passed' in line:
            parts = line.strip().split()
            for i, p in enumerate(parts):
                if 'passed' in parts[i+1] if i+1 < len(parts) else '':
                    try:
                        passed = int(p)
                    except:
                        pass
        if ' failed' in line:
            parts = line.strip().split()
            for i, p in enumerate(parts):
                if i+1 < len(parts) and 'failed' in parts[i+1]:
                    try:
                        failed = int(p)
                    except:
                        pass
        if ' error' in line:
            parts = line.strip().split()
            for i, p in enumerate(parts):
                if i+1 < len(parts) and 'error' in parts[i+1]:
                    try:
                        errors = int(p)
                    except:
                        pass
    
    # Try to parse summary line
    import re
    summary = re.search(r'(\d+) passed.*?(\d+) failed?', output)
    if summary:
        passed = int(summary.group(1))
        failed = int(summary.group(2)) if summary.group(2) else 0
    
    summary2 = re.search(r'(\d+) passed.*?(\d+) errors?', output)
    if summary2:
        errors = int(summary2.group(2)) if summary2.group(2) else 0
    
    return passed, failed, errors, elapsed

def backup_and_modify(scenario):
    """Backup file and apply modification"""
    filepath = os.path.join(FASTAPI_DIR, scenario.file_path)
    backup = filepath + ".benchmark_backup"
    shutil.copy2(filepath, backup)
    return backup

def restore_file(backup_path):
    """Restore file from backup"""
    shutil.move(backup_path, backup_path.replace(".benchmark_backup", ""))

def apply_rename(file_path, old_name, new_name):
    """Apply symbol rename to file"""
    filepath = os.path.join(FASTAPI_DIR, file_path)
    with open(filepath, 'r') as f:
        content = f.read()
    # Simple rename - replace function/class definition
    content = content.replace(f'def {old_name}(', f'def {new_name}(')
    content = content.replace(f'class {old_name}(', f'class {new_name}(')
    content = content.replace(f'class {old_name}:', f'class {new_name}:')
    with open(filepath, 'w') as f:
        f.write(content)

def main():
    scenarios = [
        Scenario(
            num=1, name="generate_unique_id 리네임",
            change_type="rename", symbol="generate_unique_id",
            changes=[{"type": "rename", "details": {"new_name": "generate_route_id"}}],
            file_path="fastapi/utils.py",
            description="utils.py의 generate_unique_id를 generate_route_id로 리네임",
            test_subset="tests/test_generate_unique_id_function.py -x"
        ),
        Scenario(
            num=2, name="Depends에 파라미터 추가",
            change_type="add_param", symbol="Depends",
            changes=[{"type": "add_param", "details": {"param_name": "timeout", "param_type": "float | None", "default": "None"}}],
            file_path="fastapi/param_functions.py",
            description="Depends()에 timeout 파라미터 추가",
            test_subset="tests/test_dependency_overrides.py -x"
        ),
        Scenario(
            num=3, name="generate_unique_id 파라미터 제거",
            change_type="remove_param", symbol="generate_unique_id",
            changes=[{"type": "remove_param", "details": {"param_name": "route"}}],
            file_path="fastapi/utils.py",
            description="generate_unique_id의 route 파라미터 제거 (breaking)",
            test_subset="tests/test_generate_unique_id_function.py -x"
        ),
        Scenario(
            num=4, name="generate_unique_id 반환 타입 변경",
            change_type="change_return", symbol="generate_unique_id",
            changes=[{"type": "change_return", "details": {"new_return": "str | None"}}],
            file_path="fastapi/utils.py",
            description="generate_unique_id 반환 타입 str → str | None",
            test_subset="tests/test_generate_unique_id_function.py -x"
        ),
        Scenario(
            num=5, name="APIRoute 리네임",
            change_type="rename", symbol="APIRoute",
            changes=[{"type": "rename", "details": {"new_name": "HttpRoute"}}],
            file_path="fastapi/routing.py",
            description="APIRoute → HttpRoute (major rename, 광범위 영향)",
            test_subset="tests/test_custom_route_class.py -x"
        ),
        Scenario(
            num=6, name="Query에 파라미터 추가",
            change_type="add_param", symbol="Query",
            changes=[{"type": "add_param", "details": {"param_name": "max_length", "param_type": "int | None", "default": "None"}}],
            file_path="fastapi/params.py",
            description="Query 클래스에 max_length 파라미터 추가",
            test_subset="tests/test_query_cookie_header_model_extra_params.py -x"
        ),
        Scenario(
            num=7, name="Depends use_cache 제거",
            change_type="remove_param", symbol="Depends",
            changes=[{"type": "remove_param", "details": {"param_name": "use_cache"}}],
            file_path="fastapi/param_functions.py",
            description="Depends()의 use_cache 파라미터 제거 (breaking)",
            test_subset="tests/test_dependency_cache.py -x"
        ),
        Scenario(
            num=8, name="ResponseModel 파라미터 추가",
            change_type="add_param", symbol="ResponseModel",
            changes=[{"type": "add_param", "details": {"param_name": "strict", "param_type": "bool", "default": "False"}}],
            file_path="fastapi/params.py",
            description="ResponseModel에 strict 파라미터 추가",
            test_subset="tests/test_validate_response.py -x"
        ),
        Scenario(
            num=9, name="Body 파라미터 제거",
            change_type="remove_param", symbol="Body",
            changes=[{"type": "remove_param", "details": {"param_name": "embed"}}],
            file_path="fastapi/params.py",
            description="Body()의 embed 파라미터 제거",
            test_subset="tests/test_union_body.py -x"
        ),
        Scenario(
            num=10, name="Header 반환 타입 변경",
            change_type="change_return", symbol="Header",
            changes=[{"type": "change_return", "details": {"new_return": "str | list[str] | None"}}],
            file_path="fastapi/params.py",
            description="Header 반환 타입 변경",
            test_subset="tests/test_custom_middleware_exception.py -x"
        ),
    ]

    results = []
    
    print("=" * 70)
    print("edit_plan 벤치마크 시작 (10 시나리오)")
    print("=" * 70)

    for scenario in scenarios:
        print(f"\n{'='*50}")
        print(f"[{scenario.num}/10] {scenario.name}")
        print(f"  유형: {scenario.change_type}")
        print(f"  심볼: {scenario.symbol}")
        print(f"  파일: {scenario.file_path}")
        print(f"{'='*50}")
        
        result = Result(scenario=f"{scenario.num}. {scenario.name}")
        
        # Step 1: Run edit_plan
        print(f"\n  [1/4] edit_plan 실행...")
        edit_plan_result, elapsed = run_shardindex_mcp("edit_plan", {
            "symbol": scenario.symbol,
            "changes": scenario.changes
        })
        result.edit_plan_time_ms = elapsed
        
        if isinstance(edit_plan_result, dict):
            # Parse impact
            impacted = edit_plan_result.get("impacted_symbols", [])
            if isinstance(impacted, list):
                result.edit_plan_impact_count = len(impacted)
                result.edit_plan_impact = [
                    {"symbol": s.get("name", "?"), "file": s.get("file", "?")}
                    for s in impacted[:10]  # Top 10 only
                ]
            elif "raw" in edit_plan_result:
                result.edit_plan_impact_count = -1  # Error
                result.notes = f"edit_plan error: {edit_plan_result.get('error', '')}"
                print(f"    ⚠ edit_plan 응답: {edit_plan_result.get('raw', '')[:200]}")
        else:
            result.notes = f"Unexpected edit_plan response type"
        
        print(f"    영향 심볼: {result.edit_plan_impact_count}개, 시간: {elapsed:.0f}ms")
        
        # Step 2: Run baseline test
        print(f"  [2/4] 베이스라인 테스트 실행...")
        baseline_pass, baseline_fail, baseline_err, baseline_time = run_pytest(
            scenario.test_subset, timeout=60
        )
        result.baseline_test_pass = baseline_pass
        print(f"    통과: {baseline_pass}, 실패: {baseline_fail}, 시간: {baseline_time/1000:.1f}s")
        
        # Step 3: Apply change and run test
        print(f"  [3/4] 변경 적용 + 테스트...")
        backup = backup_and_modify(scenario)
        
        try:
            if scenario.change_type == "rename":
                # Extract new name from changes
                new_name = scenario.changes[0]["details"].get("new_name", "")
                old_name = scenario.symbol
                apply_rename(scenario.file_path, old_name, new_name)
            
            mod_pass, mod_fail, mod_err, mod_time = run_pytest(
                scenario.test_subset, timeout=60
            )
            result.modified_test_pass = mod_pass
            result.actual_test_failures = mod_fail
            print(f"    통과: {mod_pass}, 실패: {mod_fail}, 시간: {mod_time/1000:.1f}s")
            
            # Step 4: Compare
            print(f"  [4/4] 결과 비교...")
            test_regression = mod_fail - baseline_fail
            
            if result.edit_plan_impact_count > 0 and test_regression > 0:
                result.prediction_accuracy = "✅ 예측 성공 (영향 예측 + 실제 실패)"
            elif result.edit_plan_impact_count == 0 and test_regression <= 0:
                result.prediction_accuracy = "✅ 예측 성공 (영향 없음 + 실패 없음)"
            elif result.edit_plan_impact_count > 0 and test_regression <= 0:
                result.prediction_accuracy = "⚠ False Positive (영향 예측했으나 실패 없음)"
            elif result.edit_plan_impact_count <= 0 and test_regression > 0:
                result.prediction_accuracy = "❌ False Negative (예측 실패 + 실제 실패)"
            else:
                result.prediction_accuracy = "❓ 불명확"
            
            result.notes += f"| 테스트 회귀: {test_regression}개"
            
        except Exception as e:
            result.notes += f"| 테스트 에러: {str(e)[:100]}"
        finally:
            restore_file(backup)
        
        results.append(result)
        
        print(f"\n  결과: {result.prediction_accuracy}")
        print(f"  edit_plan: {result.edit_plan_impact_count}개 영향, {elapsed:.0f}ms")
        print(f"  테스트: baseline={baseline_pass}pass/{baseline_fail}fail → modified={result.modified_test_pass}pass/{result.actual_test_failures}fail")

    # Save results
    results_data = []
    for r in results:
        results_data.append({
            "scenario": r.scenario,
            "edit_plan_impact_count": r.edit_plan_impact_count,
            "edit_plan_time_ms": r.edit_plan_time_ms,
            "edit_plan_impact": r.edit_plan_impact,
            "baseline_test_pass": r.baseline_test_pass,
            "modified_test_pass": r.modified_test_pass,
            "actual_test_failures": r.actual_test_failures,
            "prediction_accuracy": r.prediction_accuracy,
            "notes": r.notes
        })
    
    with open(RESULTS_FILE, 'w') as f:
        json.dump(results_data, f, indent=2, ensure_ascii=False)
    
    # Print summary
    print(f"\n\n{'='*70}")
    print("벤치마크 결과 요약")
    print(f"{'='*70}")
    
    success = sum(1 for r in results if "✅" in r.prediction_accuracy)
    false_pos = sum(1 for r in results if "False Positive" in r.prediction_accuracy)
    false_neg = sum(1 for r in results if "False Negative" in r.prediction_accuracy)
    unclear = sum(1 for r in results if "❓" in r.prediction_accuracy)
    
    avg_time = sum(r.edit_plan_time_ms for r in results) / len(results)
    
    print(f"\n총 시나리오: {len(results)}")
    print(f"✅ 예측 성공: {success}")
    print(f"⚠ False Positive: {false_pos}")
    print(f"❌ False Negative: {false_neg}")
    print(f"❓ 불명확: {unclear}")
    print(f"평균 edit_plan 시간: {avg_time:.0f}ms")
    print(f"\n상세 결과: {RESULTS_FILE}")
    
    for r in results:
        print(f"  [{r.scenario.split('.')[0]}] {r.prediction_accuracy} "
              f"(edit_plan: {r.edit_plan_impact_count}개, 테스트: {r.actual_test_failures}fail)")

if __name__ == "__main__":
    main()
