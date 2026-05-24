from __future__ import annotations

import sqlite3

from scripts import export_task_test_case_backfill_json as export_script


def _make_task(task_id: str = "T-1") -> export_script.TaskRow:
    return export_script.TaskRow(
        task_id=task_id,
        repo_name="ait",
        title="Example task",
        intent="Verify reviewer test backfill counts.",
        risk_tier="medium",
        status="completed",
        publication_state="published",
        published_task_id=task_id,
        plan_id=None,
        origin_plan_revision_id=None,
        plan_item_ref=None,
        created_at="2026-05-13T00:00:00+00:00",
        updated_at="2026-05-13T00:00:00+00:00",
    )


def _make_case(test_case_id: str, path: str, function_name: str, line: int) -> dict[str, object]:
    return {
        "repo_id": "REPO-AIT-LOCAL",
        "test_case_id": test_case_id,
        "pytest_node_id": f"{path}::{function_name}",
        "test_file_path": path,
        "class_name": None,
        "function_name": function_name,
        "description": f"Exercise {function_name}.",
        "source_line": line,
    }


def test_task_payload_counts_match_materialized_test_cases(monkeypatch) -> None:
    conn = sqlite3.connect(":memory:")
    monkeypatch.setattr(
        export_script,
        "_changed_test_files",
        lambda *_args, **_kwargs: ["tests/test_alpha.py", "tests/test_beta.py"],
    )
    payload = export_script._task_payload(
        _make_task(),
        changes=[
            {
                "change_id": "C-1",
                "status": "landed",
                "fork_snapshot_id": "SNP-OLD",
                "landed_snapshot_id": "SNP-NEW",
            }
        ],
        inventory_by_file={
            "tests/test_alpha.py": [_make_case("TC-1", "tests/test_alpha.py", "test_alpha", 10)],
            "tests/test_beta.py": [_make_case("TC-2", "tests/test_beta.py", "test_beta", 20)],
        },
        content_conn=conn,
        snapshot_cache={},
    )
    reviewer_test = payload["reviewer_test_backfill"]
    assert reviewer_test["test_case_count"] == 2
    assert reviewer_test["materialized_test_case_count"] == 2
    assert reviewer_test["counts_match"] is True
    assert len(payload["test_cases"]) == 2
    conn.close()


def test_task_payload_flags_mismatch_when_flattened_test_cases_drop_duplicates(monkeypatch) -> None:
    conn = sqlite3.connect(":memory:")
    monkeypatch.setattr(
        export_script,
        "_changed_test_files",
        lambda *_args, **_kwargs: ["tests/test_alpha.py", "tests/test_beta.py"],
    )
    duplicate_case_id = "TC-DUP"
    payload = export_script._task_payload(
        _make_task("T-2"),
        changes=[
            {
                "change_id": "C-2",
                "status": "landed",
                "fork_snapshot_id": "SNP-OLD",
                "landed_snapshot_id": "SNP-NEW",
            }
        ],
        inventory_by_file={
            "tests/test_alpha.py": [_make_case(duplicate_case_id, "tests/test_alpha.py", "test_alpha", 10)],
            "tests/test_beta.py": [_make_case(duplicate_case_id, "tests/test_beta.py", "test_beta", 20)],
        },
        content_conn=conn,
        snapshot_cache={},
    )
    reviewer_test = payload["reviewer_test_backfill"]
    assert reviewer_test["test_case_count"] == 2
    assert reviewer_test["materialized_test_case_count"] == 1
    assert reviewer_test["counts_match"] is False
    assert len(payload["test_cases"]) == 1
    conn.close()
