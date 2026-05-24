from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_repository_group_memberships_rebuild_plan.py"
SPEC = importlib.util.spec_from_file_location("postgres_repository_group_memberships_rebuild_plan", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_plan_renders_membership_rebuild_and_validation_flow() -> None:
    payload = plan_script.build_repository_group_memberships_rebuild_plan(
        rollback_dsn_env="ROLLBACK_DSN",
        active_dsn_env="ACTIVE_DSN",
        source_schema="rollback_schema",
        target_schema="active_schema",
        source_table="memberships_old",
        target_table="memberships_new",
        repositories_table="repos",
        repository_groups_table="groups",
        export_path="memberships.csv",
    )

    assert payload["source_memberships_relation"] == '"rollback_schema"."memberships_old"'
    assert payload["target_memberships_relation"] == '"active_schema"."memberships_new"'
    assert payload["target_repositories_relation"] == '"active_schema"."repos"'
    assert payload["target_repository_groups_relation"] == '"active_schema"."groups"'
    assert "repo_id text primary key" in payload["create_table_sql"]
    assert "repo_name text not null unique" in payload["create_table_sql"]
    assert "references \"active_schema\".\"repos\"(repo_id)" in payload["create_table_sql"]
    assert 'psql "$ROLLBACK_DSN"' in payload["export_command"]
    assert "coalesce(m.repo_id, r.repo_id) as repo_id" in payload["export_query"]
    assert 'psql "$ACTIVE_DSN"' in payload["import_command"]
    assert any(plan_script.REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_ID_INDEX in row for row in payload["post_create_index_sql"])
    assert any("g.group_id is null" in row for row in payload["validation_queries"])
    assert any("compatibility" in row for row in payload["follow_up_notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "repository-group-memberships-plan.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["export_path"] == "repository_group_memberships_wave1.csv"
    assert written["target_memberships_relation"] == '"public"."repository_group_memberships"'
