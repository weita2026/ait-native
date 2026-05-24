from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_repo_id_smoke_checks.py"
SPEC = importlib.util.spec_from_file_location("postgres_repo_id_smoke_checks", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_checks_renders_repo_aware_operator_queries() -> None:
    payload = plan_script.build_repo_id_smoke_checks(schema="control")

    checks = {entry["surface"]: entry for entry in payload["checks"]}
    assert "repository + line directory" in checks and "workflow queue" in checks
    assert "control.repositories" in checks["repository + line directory"]["query"]
    assert "control.snapshots" in checks["snapshot history"]["query"]
    assert "control.jobs" in checks["session + job surfaces"]["query"]
    assert any("rollback" in note for note in payload["notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "repo-id-smoke-checks.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["schema"] == "public"
    assert len(written["checks"]) == 5
