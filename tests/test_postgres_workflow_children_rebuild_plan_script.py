from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_workflow_children_rebuild_plan.py"
SPEC = importlib.util.spec_from_file_location("postgres_workflow_children_rebuild_plan", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_plan_renders_child_table_scope_modes_and_resolution_paths() -> None:
    payload = plan_script.build_workflow_children_rebuild_plan(
        rollback_dsn_env="ROLLBACK_DSN",
        active_dsn_env="ACTIVE_DSN",
        source_schema="rollback_schema",
        target_schema="active_schema",
        export_dir="child_exports",
    )

    tables = {entry["table"]: entry for entry in payload["tables"]}
    assert "patchsets" in tables and "authority_nodes" in tables
    assert tables["patchsets"]["scope_mode"] == "explicit-repo-id"
    assert tables["authority_nodes"]["scope_mode"] == "parent-derived"
    assert tables["planning_session_events"]["source_relation"] == '"rollback_schema"."planning_session_events"'
    assert tables["reviews"]["target_relation"] == '"active_schema"."reviews"'
    assert 'psql "$ROLLBACK_DSN"' in tables["attestations"]["export_command"]
    assert "repo_id" in (tables["session_checkpoints"]["repo_id_resolution"] or "")
    assert any("payload-lineage audit" in note for note in payload["notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "workflow-children-plan.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["export_dir"] == "workflow_children_wave4"
    assert len(written["tables"]) == 16
