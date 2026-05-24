from __future__ import annotations

from pathlib import Path

from ait import store
from ait import store_local_workflow


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

STORE_LOCAL_WORKFLOW_EXPORTS = (
    "create_local_task",
    "create_local_plan",
    "list_local_plans",
    "get_local_plan",
    "list_local_plan_revisions",
    "get_local_plan_revision",
    "revise_local_plan",
    "close_local_plan",
    "mark_local_plan_published",
    "create_local_release",
    "list_local_releases",
    "get_local_release",
    "update_local_release",
    "list_local_tasks",
    "get_local_task",
    "close_local_task",
    "mark_local_task_published",
    "create_local_change",
    "list_local_changes",
    "get_local_change",
    "close_local_change",
    "land_local_change",
    "mark_local_change_published",
    "_local_actor_identity",
    "create_local_session",
    "list_local_sessions",
    "get_local_session",
    "append_local_session_event",
    "list_local_session_events",
    "create_local_checkpoint",
    "list_local_checkpoints",
    "get_local_checkpoint",
    "resume_local_session",
    "close_local_session",
)


def test_store_local_workflow_helpers_match_store_facade() -> None:
    for name in STORE_LOCAL_WORKFLOW_EXPORTS:
        assert getattr(store_local_workflow, name) is getattr(store, name), name


def test_store_local_workflow_uses_narrow_runtime_seam() -> None:
    text = (WORKSPACE_ROOT / "src/ait/store_local_workflow.py").read_text(encoding="utf-8")
    assert "from .store import (" not in text
    assert "from .store_local_workflow_runtime import current_line, resolve_local_task_plan_linkage" in text
