from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_workflow_root_constraints_plan.py"
SPEC = importlib.util.spec_from_file_location("postgres_workflow_root_constraints_plan", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_plan_renders_repo_id_constraint_steps() -> None:
    payload = plan_script.build_workflow_root_constraints_plan(schema="control")

    steps = {entry["table"]: entry for entry in payload["steps"]}
    assert "tasks" in steps and "authority_maps" in steps
    assert any("idx_tasks_repo_id_created" in row for row in steps["tasks"]["statements"])
    assert any("releases_repo_id_version_key" in row for row in steps["releases"]["statements"])
    assert any("idx_jobs_repo_id_state" in row for row in steps["jobs"]["statements"])
    assert any("idx_authority_maps_repo_id_unique" in row for row in steps["authority_maps"]["statements"])
    assert all("schemaname = 'control'" in row for row in steps["plans"]["validation_queries"] if "schemaname" in row)
    assert any("validation bundle" in note for note in payload["notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "workflow-root-constraints-plan.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["schema"] == "public"
    assert len(written["steps"]) == 10
