from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_repository_foundation_indexes_plan.py"
SPEC = importlib.util.spec_from_file_location("postgres_repository_foundation_indexes_plan", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_plan_renders_foundation_index_and_compatibility_sql() -> None:
    payload = plan_script.build_foundation_indexes_plan(
        active_dsn_env="ACTIVE_DSN",
        schema="cutover",
        repositories_table="repos",
        memberships_table="memberships",
    )

    assert payload["repositories_relation"] == '"cutover"."repos"'
    assert payload["repository_group_memberships_relation"] == '"cutover"."memberships"'
    assert any("add column if not exists id_namespace_prefix" in row for row in payload["compatibility_column_sql"])
    assert any("update \"cutover\".\"memberships\" as m" in row for row in payload["compatibility_column_sql"])
    assert any(plan_script.REPOSITORY_NAMESPACE_PREFIX_UNIQUE_INDEX in row for row in payload["index_sql"])
    assert any(plan_script.REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_ID_INDEX in row for row in payload["index_sql"])
    assert any("id_namespace_prefix" in row for row in payload["preflight_queries"])
    assert any("repo_id is distinct from r.repo_id" in row for row in payload["validation_queries"])
    assert any("repository_group_memberships" in row for row in payload["follow_up_notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "foundation-indexes-plan.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["active_dsn_env"] == "AIT_NATIVE_SERVER_POSTGRES_DSN"
    assert written["repositories_relation"] == '"public"."repositories"'
    assert written["repository_group_memberships_relation"] == '"public"."repository_group_memberships"'
