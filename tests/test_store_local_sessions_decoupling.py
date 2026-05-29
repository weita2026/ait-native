from __future__ import annotations

from pathlib import Path

from ait import store
from ait import store_local_sessions
from ait import store_local_workflow


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

STORE_LOCAL_SESSION_EXPORTS = (
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

STORE_LOCAL_SESSION_CALLERS = (
    "src/ait/aitk_export.py",
    "src/ait/task_tokens.py",
    "src/ait/cli/session_runtime_helpers.py",
    "src/ait/cli/task_close_tracking.py",
    "src/ait/cli/task_dag_runtime_helpers.py",
    "src/ait/cli/task_dag_telegram_watch.py",
    "src/ait/cli/workflow_land_task_dag.py",
)


def test_store_local_session_helpers_match_facades() -> None:
    for name in STORE_LOCAL_SESSION_EXPORTS:
        assert getattr(store_local_sessions, name) is getattr(store, name), name
        assert getattr(store_local_sessions, name) is getattr(store_local_workflow, name), name


def test_store_local_session_helpers_are_extracted_from_facades() -> None:
    store_text = (WORKSPACE_ROOT / "src/ait/store.py").read_text(encoding="utf-8")
    workflow_text = (WORKSPACE_ROOT / "src/ait/store_local_workflow.py").read_text(encoding="utf-8")
    sessions_text = (WORKSPACE_ROOT / "src/ait/store_local_sessions.py").read_text(encoding="utf-8")

    assert "from .store_local_sessions import (" in store_text
    assert "from .store_local_sessions import (" in workflow_text
    assert "def create_local_session(" not in workflow_text
    assert "def list_local_sessions(" not in workflow_text
    assert "def get_local_session(" not in workflow_text
    assert "def append_local_session_event(" not in workflow_text
    assert "def list_local_session_events(" not in workflow_text
    assert "def create_local_checkpoint(" not in workflow_text
    assert "def list_local_checkpoints(" not in workflow_text
    assert "def get_local_checkpoint(" not in workflow_text
    assert "def resume_local_session(" not in workflow_text
    assert "def close_local_session(" not in workflow_text
    assert "from .store import (" not in sessions_text


def test_store_local_session_callers_use_narrow_seam() -> None:
    for relative_path in STORE_LOCAL_SESSION_CALLERS:
        text = (WORKSPACE_ROOT / relative_path).read_text(encoding="utf-8")
        assert "store_local_sessions import" in text, relative_path
