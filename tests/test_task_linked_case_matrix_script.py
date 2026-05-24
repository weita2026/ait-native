from __future__ import annotations

import importlib.util
import json
import sqlite3
import sys
from pathlib import Path
from types import SimpleNamespace


ROOT = Path(__file__).resolve().parent.parent
SCRIPT_PATH = ROOT / "scripts" / "task_linked_case_matrix.py"
SPEC = importlib.util.spec_from_file_location("task_linked_case_matrix", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
matrix_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = matrix_script
SPEC.loader.exec_module(matrix_script)


class _FakeCursor:
    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False

    def execute(self, *_args, **_kwargs):
        return None

    def fetchall(self):
        return []


class _FakeConnection:
    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False

    def cursor(self):
        return _FakeCursor()

    def close(self):
        return None


def test_discover_repo_name_reads_local_ait_config(tmp_path: Path) -> None:
    ait_dir = tmp_path / ".ait"
    ait_dir.mkdir()
    (ait_dir / "config.json").write_text(json.dumps({"repo_name": "demo"}), encoding="utf-8")

    assert matrix_script._discover_repo_name(tmp_path) == "demo"


def test_optional_text_keeps_none_as_none() -> None:
    assert matrix_script._optional_text(None) is None
    assert matrix_script._optional_text("  ait  ") == "ait"


def test_run_task_linked_case_matrix_executes_pytest_per_task(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setattr(matrix_script, "_load_psycopg", lambda: SimpleNamespace(connect=lambda *_args, **_kwargs: _FakeConnection(), rows=SimpleNamespace(dict_row=object())))
    monkeypatch.setattr(matrix_script, "_resolve_repo_id", lambda *_args, **_kwargs: "REPO-1")
    monkeypatch.setattr(
        matrix_script,
        "_load_task_matrix_rows",
        lambda *_args, **_kwargs: [
            {
                "task_id": "T-1",
                "task_title": "Alpha",
                "pytest_node_ids": ["tests/test_alpha.py::test_one", "tests/test_alpha.py::test_two"],
            },
            {
                "task_id": "T-2",
                "task_title": "Beta",
                "pytest_node_ids": ["tests/test_beta.py::test_three"],
            },
        ],
    )

    calls: list[list[str]] = []

    def _fake_run(cmd, **_kwargs):
        calls.append(list(cmd))
        return SimpleNamespace(returncode=0, stdout="ok\n", stderr="")

    monkeypatch.setattr(matrix_script.subprocess, "run", _fake_run)

    payload = matrix_script.run_task_linked_case_matrix(
        dsn="postgresql://example",
        repo_root=tmp_path,
        control_schema="ait_native_control",
        repo_id=None,
        repo_name="ait",
        task_ids=None,
        max_tasks=None,
        fail_fast=False,
    )

    assert payload["status"] == "pass"
    assert payload["selected_task_count"] == 2
    assert payload["executed_task_count"] == 2
    assert payload["total_pytest_node_count"] == 3
    assert [row["task_id"] for row in payload["task_results"]] == ["T-1", "T-2"]
    assert calls == [
        [sys.executable, "-m", "pytest", "-q", "tests/test_alpha.py::test_one", "tests/test_alpha.py::test_two"],
        [sys.executable, "-m", "pytest", "-q", "tests/test_beta.py::test_three"],
    ]


def test_run_task_linked_case_matrix_fail_fast_stops_after_first_failure(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setattr(matrix_script, "_load_psycopg", lambda: SimpleNamespace(connect=lambda *_args, **_kwargs: _FakeConnection(), rows=SimpleNamespace(dict_row=object())))
    monkeypatch.setattr(matrix_script, "_resolve_repo_id", lambda *_args, **_kwargs: "REPO-1")
    monkeypatch.setattr(
        matrix_script,
        "_load_task_matrix_rows",
        lambda *_args, **_kwargs: [
            {"task_id": "T-1", "task_title": "Alpha", "pytest_node_ids": ["tests/test_alpha.py::test_one"]},
            {"task_id": "T-2", "task_title": "Beta", "pytest_node_ids": ["tests/test_beta.py::test_two"]},
        ],
    )

    def _fake_run(_cmd, **_kwargs):
        return SimpleNamespace(returncode=1, stdout="boom\n", stderr="trace\n")

    monkeypatch.setattr(matrix_script.subprocess, "run", _fake_run)

    payload = matrix_script.run_task_linked_case_matrix(
        dsn="postgresql://example",
        repo_root=tmp_path,
        control_schema="ait_native_control",
        repo_id=None,
        repo_name="ait",
        task_ids=None,
        max_tasks=None,
        fail_fast=True,
    )

    assert payload["status"] == "fail"
    assert payload["executed_task_count"] == 1
    assert payload["failed_task_ids"] == ["T-1"]


def test_build_markdown_summarizes_task_results() -> None:
    markdown = matrix_script._build_markdown(
        {
            "repo_name": "ait",
            "repo_id": "REPO-1",
            "selected_task_count": 1,
            "executed_task_count": 1,
            "passed_task_count": 1,
            "failed_task_count": 0,
            "total_pytest_node_count": 2,
            "status": "pass",
            "task_results": [
                {
                    "task_id": "T-1",
                    "task_title": "Alpha",
                    "status": "pass",
                    "pytest_node_count": 2,
                    "exit_code": 0,
                    "duration_seconds": 0.25,
                }
            ],
        }
    )

    assert "# Task Linked Case Matrix (ait)" in markdown
    assert "- `T-1` Alpha: `pass` nodes=2 exit=0 duration=0.25s" in markdown
