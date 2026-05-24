from __future__ import annotations

from pathlib import Path

from ait import store
from ait import store_worktree_runtime


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

STORE_WORKTREE_RUNTIME_EXPORTS = (
    "WORKTREE_CREATION_KINDS",
    "WORKTREE_CLEANUP_POLICIES",
    "WORKTREE_CLEANUP_CLASSES",
    "LINE_CLEANUP_CLASSES",
    "DEFAULT_WORKTREE_CREATION_KIND",
    "DEFAULT_WORKTREE_CLEANUP_POLICY",
    "DEFAULT_WORKTREE_CLEANUP_OLDER_THAN",
    "DEFAULT_LINE_CLEANUP_OLDER_THAN",
    "_coerce_datetime",
    "_normalize_worktree_creation_kind",
    "_normalize_worktree_cleanup_policy",
    "_default_cleanup_policy_for_creation_kind",
    "_normalize_older_than",
    "get_remote",
    "list_lines",
    "current_line",
    "create_line",
    "archive_line",
    "_set_current_line",
    "switch_line",
    "create_snapshot",
    "workspace_status",
    "set_line_head",
)

WORKTREE_RUNTIME_SEAM_FILES = (
    "src/ait/store_worktree_bindings.py",
    "src/ait/store_worktree_rebase.py",
    "src/ait/store_worktree_state.py",
    "src/ait/store_worktree_views.py",
    "src/ait/store_worktrees.py",
    "src/ait/store_line_cleanup.py",
)


def test_store_worktree_runtime_helpers_match_store_facade() -> None:
    for name in STORE_WORKTREE_RUNTIME_EXPORTS:
        assert getattr(store_worktree_runtime, name) is getattr(store, name), name


def test_store_worktree_modules_use_narrow_runtime_seam() -> None:
    for relative_path in WORKTREE_RUNTIME_SEAM_FILES:
        text = (WORKSPACE_ROOT / relative_path).read_text(encoding="utf-8")
        assert "from .store import (" not in text, relative_path

    worktree_text = (WORKSPACE_ROOT / "src/ait/store_worktrees.py").read_text(encoding="utf-8")
    state_text = (WORKSPACE_ROOT / "src/ait/store_worktree_state.py").read_text(encoding="utf-8")
    views_text = (WORKSPACE_ROOT / "src/ait/store_worktree_views.py").read_text(encoding="utf-8")
    rebase_text = (WORKSPACE_ROOT / "src/ait/store_worktree_rebase.py").read_text(encoding="utf-8")
    cleanup_text = (WORKSPACE_ROOT / "src/ait/store_line_cleanup.py").read_text(encoding="utf-8")
    bindings_text = (WORKSPACE_ROOT / "src/ait/store_worktree_bindings.py").read_text(encoding="utf-8")

    assert "from .store_worktree_runtime import (" in worktree_text
    assert "from .store_worktree_runtime import (" in state_text
    assert "from .store_worktree_runtime import (" in views_text
    assert "from .store_worktree_runtime import (" in rebase_text
    assert "from .store_worktree_runtime import (" in cleanup_text
    assert "from ait_protocol.common import normalize_optional_text" in bindings_text
    assert "from .repo_paths import RepoContext" in bindings_text
