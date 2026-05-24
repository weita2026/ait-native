from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_content_exceptions_audit.py"
SPEC = importlib.util.spec_from_file_location("postgres_content_exceptions_audit", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_audit_renders_repo_neutral_dependency_checks() -> None:
    payload = plan_script.build_content_exceptions_audit(
        rollback_dsn_env="ROLLBACK_DSN",
        active_dsn_env="ACTIVE_DSN",
        schema="audit_schema",
        blobs_table="blobs_live",
        trees_table="trees_live",
        tree_entries_table="tree_entries_live",
        tree_packs_table="tree_packs_live",
        snapshots_table="snapshots_live",
        packs_table="packs_live",
    )

    assert payload["schema"] == "audit_schema"
    assert payload["repo_neutral_tables"] == plan_script.REPO_NEUTRAL_TABLES
    assert "content-addressed storage" in payload["table_roles"]["blobs"]
    assert any('"audit_schema"."snapshots_live"' in row for row in payload["active_dependency_queries"])
    assert any('"audit_schema"."packs_live"' in row for row in payload["active_dependency_queries"])
    assert any("tree_entries" in row for row in payload["active_dependency_queries"])
    assert any("stay in place" in row for row in payload["decision_notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "content-exceptions-audit.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["repo_neutral_tables"] == ["blobs", "trees", "tree_entries", "tree_packs"]
