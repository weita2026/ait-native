from __future__ import annotations

from pathlib import Path

from ait import store
from ait import store_remotes
from ait import store_worktree_runtime


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

STORE_REMOTE_EXPORTS = (
    "add_remote",
    "list_remotes",
    "get_remote",
)

STORE_REMOTE_CALLERS = (
    "src/ait/cli/install_command.py",
    "src/ait/cli/session_runtime_helpers.py",
    "src/ait/cli/remote_session_wrappers.py",
    "src/ait/cli/remote_repository_defaults.py",
    "src/ait/cli/queue_summary_helpers.py",
    "src/ait/cli/task_worktree_resolution.py",
)


def test_store_remote_helpers_match_store_facade() -> None:
    for name in STORE_REMOTE_EXPORTS:
        assert getattr(store_remotes, name) is getattr(store, name), name


def test_store_worktree_runtime_get_remote_matches_remote_seam() -> None:
    assert store_worktree_runtime.get_remote is store_remotes.get_remote
    assert store_worktree_runtime.get_remote is store.get_remote


def test_store_remote_helpers_are_extracted_from_store_facades() -> None:
    store_text = (WORKSPACE_ROOT / "src/ait/store.py").read_text(encoding="utf-8")
    remotes_text = (WORKSPACE_ROOT / "src/ait/store_remotes.py").read_text(encoding="utf-8")
    runtime_text = (WORKSPACE_ROOT / "src/ait/store_worktree_runtime.py").read_text(encoding="utf-8")

    assert "from .store_remotes import (" in store_text
    assert "def add_remote(" not in store_text
    assert "def list_remotes(" not in store_text
    assert "from .store_remotes import get_remote" in runtime_text
    assert "def get_remote(" not in runtime_text
    assert "from .store import (" not in remotes_text


def test_store_remote_callers_use_narrow_remote_seam() -> None:
    for relative_path in STORE_REMOTE_CALLERS:
        text = (WORKSPACE_ROOT / relative_path).read_text(encoding="utf-8")
        assert "store_remotes import" in text, relative_path
