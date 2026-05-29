from __future__ import annotations

from pathlib import Path

from ait import store
from ait import store_local_tasks
from ait import store_local_workflow


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

STORE_LOCAL_TASK_EXPORTS = (
    "create_local_task",
    "list_local_tasks",
    "get_local_task",
    "close_local_task",
    "mark_local_task_published",
)

STORE_LOCAL_TASK_CALLERS = (
    "src/ait/aitk_export.py",
    "src/ait/release_ops.py",
    "src/ait/task_tokens.py",
    "src/ait/cli/workflow_authoring.py",
    "src/ait/cli/task_worktree_runtime.py",
    "src/ait/cli/queue_summary_helpers.py",
    "src/ait/cli/task_dag_runtime_helpers.py",
    "src/ait/cli/task_dag_telegram_watch.py",
    "src/ait/cli/task_worktree_resolution.py",
    "src/ait/cli/workflow_boundary_sessions.py",
    "src/ait/cli/workflow_land_selection.py",
    "src/ait/cli/workflow_land_completed_local.py",
)


def test_store_local_task_helpers_match_facades() -> None:
    for name in STORE_LOCAL_TASK_EXPORTS:
        assert getattr(store_local_tasks, name) is getattr(store, name), name
        assert getattr(store_local_tasks, name) is getattr(store_local_workflow, name), name


def test_store_local_task_helpers_are_extracted_from_facades() -> None:
    store_text = (WORKSPACE_ROOT / "src/ait/store.py").read_text(encoding="utf-8")
    workflow_text = (WORKSPACE_ROOT / "src/ait/store_local_workflow.py").read_text(encoding="utf-8")
    tasks_text = (WORKSPACE_ROOT / "src/ait/store_local_tasks.py").read_text(encoding="utf-8")

    assert "from .store_local_tasks import (" in store_text
    assert "from .store_local_tasks import (" in workflow_text
    assert "def create_local_task(" not in workflow_text
    assert "def list_local_tasks(" not in workflow_text
    assert "def get_local_task(" not in workflow_text
    assert "def close_local_task(" not in workflow_text
    assert "def mark_local_task_published(" not in workflow_text
    assert "from .store import (" not in tasks_text


def test_store_local_task_callers_use_narrow_seam() -> None:
    for relative_path in STORE_LOCAL_TASK_CALLERS:
        text = (WORKSPACE_ROOT / relative_path).read_text(encoding="utf-8")
        assert "store_local_tasks import" in text, relative_path
