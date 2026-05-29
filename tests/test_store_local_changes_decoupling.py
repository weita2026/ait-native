from __future__ import annotations

from pathlib import Path

from ait import store
from ait import store_local_changes
from ait import store_local_workflow


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

STORE_LOCAL_CHANGE_EXPORTS = (
    "create_local_change",
    "list_local_changes",
    "get_local_change",
    "close_local_change",
    "land_local_change",
    "mark_local_change_published",
)

STORE_LOCAL_CHANGE_CALLERS = (
    "src/ait/aitk_export.py",
    "src/ait/release_ops.py",
    "src/ait/cli/workflow_authoring.py",
    "src/ait/cli/queue_summary_helpers.py",
    "src/ait/cli/task_dag_runtime_helpers.py",
    "src/ait/cli/task_dag_telegram_watch.py",
    "src/ait/cli/task_worktree_resolution.py",
    "src/ait/cli/task_worktree_runtime.py",
    "src/ait/cli/workflow_boundary_sessions.py",
    "src/ait/cli/workflow_land_selection.py",
    "src/ait/cli/local_promotion_stale_base_guard.py",
)


def test_store_local_change_helpers_match_facades() -> None:
    for name in STORE_LOCAL_CHANGE_EXPORTS:
        assert getattr(store_local_changes, name) is getattr(store, name), name
        assert getattr(store_local_changes, name) is getattr(store_local_workflow, name), name


def test_store_local_change_helpers_are_extracted_from_facades() -> None:
    store_text = (WORKSPACE_ROOT / "src/ait/store.py").read_text(encoding="utf-8")
    workflow_text = (WORKSPACE_ROOT / "src/ait/store_local_workflow.py").read_text(encoding="utf-8")
    changes_text = (WORKSPACE_ROOT / "src/ait/store_local_changes.py").read_text(encoding="utf-8")

    assert "from .store_local_changes import (" in store_text
    assert "from .store_local_changes import (" in workflow_text
    assert "def create_local_change(" not in workflow_text
    assert "def list_local_changes(" not in workflow_text
    assert "def get_local_change(" not in workflow_text
    assert "def close_local_change(" not in workflow_text
    assert "def land_local_change(" not in workflow_text
    assert "def mark_local_change_published(" not in workflow_text
    assert "from .store import (" not in changes_text


def test_store_local_change_callers_use_narrow_seam() -> None:
    for relative_path in STORE_LOCAL_CHANGE_CALLERS:
        text = (WORKSPACE_ROOT / relative_path).read_text(encoding="utf-8")
        assert "store_local_changes import" in text, relative_path
