from __future__ import annotations

import importlib.util
import sqlite3
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT_PATH = ROOT / "scripts" / "postgres_resolve_unmatched_task_test_cases.py"
SPEC = importlib.util.spec_from_file_location("postgres_resolve_unmatched_task_test_cases", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
resolve_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = resolve_script
SPEC.loader.exec_module(resolve_script)


def _candidate(
    *,
    task_id: str = "T-1",
    change_id: str = "C-1",
    patchset_id: str = "P-1",
) -> resolve_script.CandidatePatchset:
    return resolve_script.CandidatePatchset(
        task_id=task_id,
        change_id=change_id,
        patchset_id=patchset_id,
        base_snapshot_id="SNP-OLD",
        revision_snapshot_id="SNP-NEW",
    )


def test_choose_task_for_test_case_uses_function_diff_for_single_candidate(monkeypatch) -> None:
    monkeypatch.setattr(
        resolve_script,
        "_task_ids_that_change_function",
        lambda **_kwargs: ({"T-1"}, {"T-1": ["P-1"]}),
    )

    chosen_task_id, patchset_ids, change_ids, reason = resolve_script._choose_task_for_test_case(
        ctx=None,
        content_conn=sqlite3.connect(":memory:"),
        text_cache={},
        candidate_patchsets=[_candidate()],
        candidate_task_ids=["T-1"],
        test_file_path="tests/test_alpha.py",
        function_name="test_same_name",
        function_count=15,
    )

    assert chosen_task_id == "T-1"
    assert patchset_ids == ["P-1"]
    assert change_ids == ["C-1"]
    assert reason == "single_candidate_task_and_function_diff"


def test_choose_task_for_test_case_falls_back_to_unique_function_name_for_single_candidate(monkeypatch) -> None:
    monkeypatch.setattr(
        resolve_script,
        "_task_ids_that_change_function",
        lambda **_kwargs: (set(), {}),
    )

    chosen_task_id, patchset_ids, change_ids, reason = resolve_script._choose_task_for_test_case(
        ctx=None,
        content_conn=sqlite3.connect(":memory:"),
        text_cache={},
        candidate_patchsets=[_candidate()],
        candidate_task_ids=["T-1"],
        test_file_path="tests/test_alpha.py",
        function_name="test_unique",
        function_count=1,
    )

    assert chosen_task_id == "T-1"
    assert patchset_ids == ["P-1"]
    assert change_ids == ["C-1"]
    assert reason == "single_candidate_task_and_unique_function_name"


def test_choose_task_for_test_case_keeps_single_candidate_unresolved_without_proof(monkeypatch) -> None:
    monkeypatch.setattr(
        resolve_script,
        "_task_ids_that_change_function",
        lambda **_kwargs: (set(), {}),
    )

    chosen_task_id, patchset_ids, change_ids, reason = resolve_script._choose_task_for_test_case(
        ctx=None,
        content_conn=sqlite3.connect(":memory:"),
        text_cache={},
        candidate_patchsets=[_candidate()],
        candidate_task_ids=["T-1"],
        test_file_path="tests/test_alpha.py",
        function_name="test_same_name",
        function_count=15,
    )

    assert chosen_task_id is None
    assert patchset_ids == []
    assert change_ids == []
    assert reason is None
