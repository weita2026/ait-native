from __future__ import annotations

from pathlib import Path

from ait.repo_paths import RepoContext


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_decoupling_plan_routes_latest_shipped_and_current_wave_entries() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)

    assert (
        "| Latest shipped internal hotspot wave | "
        "[Directory Decoupling Compact DAG Wave 22]"
        "(./sprints/directory_decoupling_compact_dag_wave_22.md) |"
    ) in plan_text
    assert (
        "| Latest internal hotspot DAG artifact | "
        "[Directory Decoupling Compact DAG Wave 23]"
        "(./sprints/directory_decoupling_compact_dag_wave_23.md) |"
    ) in plan_text
    assert "./sprints/directory_decoupling_compact_dag_wave_22.md" in plan_text
    assert "./sprints/directory_decoupling_compact_dag_wave_23.md" in plan_text
    assert (
        "| Post-wave-10 docs/guard reconciliation | "
        "[Reconcile Decoupling Docs Routing After Wave 10 Land]"
        "(./sprints/directory_decoupling_docs_routing_reconciliation.md) |"
    ) in plan_text


def test_decoupling_plan_marks_wave_10_follow_up_complete() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)

    assert "- [x] Dispatch the wave-10 compact follow-up batch" in plan_text
    assert "./sprints/directory_decoupling_compact_dag_wave_10.md" in plan_text
    assert "tests/test_directory_decoupling_wave_10_validation.py" in plan_text
