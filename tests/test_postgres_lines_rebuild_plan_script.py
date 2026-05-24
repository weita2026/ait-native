from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_lines_rebuild_plan.py"
SPEC = importlib.util.spec_from_file_location("postgres_lines_rebuild_plan", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_plan_renders_lines_rebuild_and_validation_flow() -> None:
    payload = plan_script.build_lines_rebuild_plan(
        rollback_dsn_env="ROLLBACK_DSN",
        active_dsn_env="ACTIVE_DSN",
        source_schema="rollback_schema",
        target_schema="active_schema",
        source_table="lines_old",
        target_table="lines_new",
        repositories_table="repos",
        export_path="lines.csv",
    )

    assert payload["source_lines_relation"] == '"rollback_schema"."lines_old"'
    assert payload["target_lines_relation"] == '"active_schema"."lines_new"'
    assert payload["target_repositories_relation"] == '"active_schema"."repos"'
    assert "primary key (repo_id, line_name)" in payload["create_table_sql"]
    assert "references \"active_schema\".\"repos\"(repo_id)" in payload["create_table_sql"]
    assert "references \"active_schema\".\"repos\"(repo_name)" in payload["create_table_sql"]
    assert 'psql "$ROLLBACK_DSN"' in payload["export_command"]
    assert "coalesce(l.repo_id, r.repo_id) as repo_id" in payload["export_query"]
    assert 'psql "$ACTIVE_DSN"' in payload["import_command"]
    assert any(plan_script.LINES_REPO_COMPAT_INDEX in row for row in payload["post_create_index_sql"])
    assert any(plan_script.LINES_REPO_ID_INDEX in row for row in payload["post_create_index_sql"])
    assert any("line_repo_id" in row for row in payload["validation_queries"])
    assert any("ref-path compatibility" in row for row in payload["follow_up_notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "lines-plan.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["export_path"] == "lines_wave2.csv"
    assert written["target_lines_relation"] == '"public"."lines"'
