from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_repositories_rebuild_plan.py"
SPEC = importlib.util.spec_from_file_location("postgres_repositories_rebuild_plan", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_plan_renders_repo_id_primary_key_and_copy_flow() -> None:
    payload = plan_script.build_repositories_rebuild_plan(
        rollback_dsn_env="ROLLBACK_DSN",
        active_dsn_env="ACTIVE_DSN",
        source_schema="rollback_schema",
        target_schema="active_schema",
        export_path="repos.csv",
    )

    assert payload["source_relation"] == '"rollback_schema"."repositories"'
    assert payload["target_relation"] == '"active_schema"."repositories"'
    assert "repo_id text primary key" in payload["create_table_sql"]
    assert "repo_name text not null unique" in payload["create_table_sql"]
    assert "psql \"$ROLLBACK_DSN\"" in payload["export_command"]
    assert "\\copy (select repo_id, repo_name" in payload["export_command"]
    assert "psql \"$ACTIVE_DSN\"" in payload["import_command"]
    assert any("id_namespace_prefix" in query for query in payload["validation_queries"])
    assert any("foundation-indexes" in note for note in payload["follow_up_notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "repositories-plan.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["export_path"] == "repositories_wave1.csv"
    assert written["target_relation"] == '"public"."repositories"'
