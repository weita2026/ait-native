from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_workflow_roots_rebuild_plan.py"
SPEC = importlib.util.spec_from_file_location("postgres_workflow_roots_rebuild_plan", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_plan_renders_root_table_exports_and_repo_id_backfill() -> None:
    payload = plan_script.build_workflow_roots_rebuild_plan(
        rollback_dsn_env="ROLLBACK_DSN",
        active_dsn_env="ACTIVE_DSN",
        source_schema="rollback_schema",
        target_schema="active_schema",
        repositories_table="repos",
        export_dir="root_exports",
    )

    tables = {entry["table"]: entry for entry in payload["tables"]}
    assert set(tables) == {"plans", "tasks", "changes", "releases", "sessions", "planning_sessions", "stacks", "role_bindings", "jobs", "authority_maps"}
    assert tables["plans"]["source_relation"] == '"rollback_schema"."plans"'
    assert tables["tasks"]["target_relation"] == '"active_schema"."tasks"'
    assert 'psql "$ROLLBACK_DSN"' in tables["changes"]["export_command"]
    assert "coalesce(t.repo_id, r.repo_id) as repo_id" in tables["releases"]["export_query"]
    assert any("root-constraints helper" in step for step in tables["sessions"]["rebuild_checklist"])
    assert any("repo_name, t.repo_id as table_repo_id" in row for row in tables["authority_maps"]["validation_queries"])
    assert any("root-constraints helper" in note or "workflow-root" in note for note in payload["follow_up_notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "workflow-roots-plan.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["export_dir"] == "workflow_roots_wave3"
    assert len(written["tables"]) == 10
