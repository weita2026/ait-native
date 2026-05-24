from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_content_history_rebuild_plan.py"
SPEC = importlib.util.spec_from_file_location("postgres_content_history_rebuild_plan", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_plan_renders_snapshots_and_packs_rebuild_flow() -> None:
    payload = plan_script.build_content_history_rebuild_plan(
        rollback_dsn_env="ROLLBACK_DSN",
        active_dsn_env="ACTIVE_DSN",
        source_schema="rollback_schema",
        target_schema="active_schema",
        snapshots_table="snapshots_old",
        packs_table="packs_old",
        repositories_table="repos",
        lines_table="lines_new",
        snapshots_export_path="snapshots.csv",
        packs_export_path="packs.csv",
    )

    snapshots = payload["snapshots"]
    packs = payload["packs"]
    assert snapshots["source_relation"] == '"rollback_schema"."snapshots_old"'
    assert snapshots["target_relation"] == '"active_schema"."snapshots_old"'
    assert snapshots["target_lines_relation"] == '"active_schema"."lines_new"'
    assert 'psql "$ROLLBACK_DSN"' in snapshots["export_command"]
    assert "coalesce(s.repo_id, r.repo_id) as repo_id" in snapshots["export_query"]
    assert any(plan_script.SNAPSHOTS_REPO_ID_CREATED_INDEX in row for row in snapshots["post_create_index_sql"])
    assert any("parent_snapshot_id" in row for row in snapshots["validation_queries"])
    assert any("line_name" in row for row in snapshots["validation_queries"])

    assert packs["source_relation"] == '"rollback_schema"."packs_old"'
    assert packs["target_relation"] == '"active_schema"."packs_old"'
    assert 'psql "$ACTIVE_DSN"' in packs["import_command"]
    assert any(plan_script.PACKS_REPO_ID_INDEX in row for row in packs["post_create_index_sql"])
    assert any("pack_path" in row for row in packs["validation_queries"])
    assert any("content-exceptions audit" in row for row in payload["follow_up_notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "content-history-plan.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["snapshots"]["export_path"] == "snapshots_wave2.csv"
    assert written["packs"]["export_path"] == "packs_wave2.csv"
