from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_repo_id_row_count_validation.py"
SPEC = importlib.util.spec_from_file_location("postgres_repo_id_row_count_validation", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_plan_renders_row_count_and_uniqueness_queries() -> None:
    payload = plan_script.build_repo_id_row_count_validation(schema="control")

    tables = {entry["table"]: entry for entry in payload["tables"]}
    assert "repositories" in tables and "land_requests" in tables
    assert "control.repositories" in tables["repositories"]["rollback_count_query"]
    assert any("repo_id, count(*) as duplicates" in row for row in tables["repositories"]["uniqueness_queries"])
    assert any("repo_id, line_name" in row for row in tables["lines"]["uniqueness_queries"])
    assert "null_repo_id_count" in tables["jobs"]["repo_id_null_query"]
    assert any("rollback-count query" in note for note in payload["notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "row-count-validation.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["schema"] == "public"
    assert len(written["tables"]) >= 20
