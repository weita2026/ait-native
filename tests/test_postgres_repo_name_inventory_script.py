from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_repo_name_inventory.py"
SPEC = importlib.util.spec_from_file_location("postgres_repo_name_inventory", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
inventory_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = inventory_script
SPEC.loader.exec_module(inventory_script)


def test_summarize_inventory_groups_tables_into_waves() -> None:
    summary = inventory_script.summarize_inventory(
        [
            {
                "table_schema": "public",
                "table_name": "repositories",
                "column_name": "repo_name",
                "data_type": "text",
            },
            {
                "table_schema": "public",
                "table_name": "tasks",
                "column_name": "repo_name",
                "data_type": "text",
            },
        ],
        [
            {
                "table_schema": "public",
                "table_name": "tasks",
                "constraint_name": "tasks_repo_name_fkey",
                "definition": "FOREIGN KEY (repo_name) REFERENCES repositories(repo_name)",
            },
            {
                "table_schema": "public",
                "table_name": "custom_queue_projection",
                "constraint_name": "custom_queue_projection_repo_name_key",
                "definition": "UNIQUE (repo_name)",
            },
        ],
        [
            {
                "schemaname": "public",
                "tablename": "plan_revisions",
                "indexname": "plan_revisions_repo_name_idx",
                "indexdef": "CREATE INDEX plan_revisions_repo_name_idx ON plan_revisions (repo_name)",
            }
        ],
    )

    table_summary = summary["table_summary"]
    assert [row["table_ref"] for row in table_summary] == [
        "public.repositories",
        "public.tasks",
        "public.plan_revisions",
        "public.custom_queue_projection",
    ]
    assert table_summary[0]["wave"] == "wave_1_repository_foundation"
    assert table_summary[1]["wave"] == "wave_3_workflow_roots"
    assert table_summary[2]["wave"] == "wave_4_workflow_children"
    assert table_summary[3]["wave"] is None
    assert summary["wave_groups"][0]["tables"] == ["public.repositories"]
    assert summary["wave_groups"][2]["tables"] == ["public.tasks"]
    assert summary["wave_groups"][3]["tables"] == ["public.plan_revisions"]
    assert summary["unclassified_tables"] == ["public.custom_queue_projection"]


def test_main_writes_json_report(monkeypatch, tmp_path: Path) -> None:
    payload = {
        "generated_at": "2026-05-11T04:00:00+00:00",
        "repo_name_columns": [],
        "repo_name_constraints": [],
        "repo_name_indexes": [],
        "table_summary": [
            {
                "table_schema": "public",
                "table_name": "repositories",
                "table_ref": "public.repositories",
                "wave": "wave_1_repository_foundation",
                "wave_label": "Wave 1: repository foundation rebuild",
                "repo_name_columns": ["repo_name"],
                "repo_name_constraints": [],
                "repo_name_indexes": [],
                "dependency_count": 1,
            }
        ],
        "wave_groups": [
            {
                "wave": "wave_1_repository_foundation",
                "label": "Wave 1: repository foundation rebuild",
                "tables": ["public.repositories"],
            }
        ],
        "unclassified_tables": [],
    }
    monkeypatch.setattr(inventory_script, "capture_live_inventory", lambda dsn: payload)

    output_path = tmp_path / "inventory.json"
    exit_code = inventory_script.main(
        [
            "--dsn",
            "postgresql://ait:secret@db.example:5432/ait_native",
            "--json",
            "--output",
            str(output_path),
        ]
    )

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["wave_groups"][0]["tables"] == ["public.repositories"]
    assert written["table_summary"][0]["repo_name_columns"] == ["repo_name"]
