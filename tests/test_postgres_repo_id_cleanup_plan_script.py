from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_repo_id_cleanup_plan.py"
SPEC = importlib.util.spec_from_file_location("postgres_repo_id_cleanup_plan", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_cleanup_plan_renders_gates_and_drop_commands() -> None:
    payload = plan_script.build_repo_id_cleanup_plan(rollback_database="rollback_db", active_database="active_db")

    assert payload["rollback_database"] == "rollback_db"
    assert payload["active_database"] == "active_db"
    assert any("row-count validation" in gate for gate in payload["gates"])
    assert any("dropdb --if-exists rollback_db" in command for command in payload["cleanup_commands"])
    assert any("pg_database" in query for query in payload["verification_queries"])
    assert any("immutable" in note or "last step" in note for note in payload["notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "repo-id-cleanup-plan.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["rollback_database"] == "ait_native_old"
