from __future__ import annotations

from pathlib import Path

import pytest

from ait import local_content
from ait import local_control
from ait import store
from ait import store_worktree_cleanup
from ait import store_worktree_state
from ait import store_worktree_filesystem
from ait import store_worktree_lifecycle
from ait import store_worktree_rebase
from ait import store_worktree_restore
from ait import store_worktrees

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_store_worktree_helpers_match_store_facade() -> None:
    assert store_worktrees.add_worktree is store.add_worktree
    assert store_worktrees.list_worktrees is store.list_worktrees
    assert store_worktrees.worktree_doctor is store.worktree_doctor
    assert store_worktrees.promote_worktree is store.promote_worktree
    assert store_worktrees.bind_worktree is store.bind_worktree
    assert store_worktrees.prune_stale_worktrees is store.prune_stale_worktrees
    assert store_worktrees.preview_worktree_rebase is store.preview_worktree_rebase
    assert store_worktrees.rebase_worktree is store.rebase_worktree
    assert store_worktrees.continue_worktree_rebase is store.continue_worktree_rebase
    assert store_worktrees.abort_worktree_rebase is store.abort_worktree_rebase
    assert store_worktrees.restore_owned_head is store.restore_owned_head
    assert store_worktrees.sync_worktree is store.sync_worktree
    assert store_worktrees.sync_all_worktrees is store.sync_all_worktrees
    assert store_worktrees.remove_worktree is store.remove_worktree
    assert store_worktrees.remove_worktrees is store.remove_worktrees
    assert store_worktrees._create_directory_link is store_worktree_filesystem._create_directory_link
    assert store_worktrees._remove_tree_force is store_worktree_filesystem._remove_tree_force
    assert store_worktrees._copy_seed_tree is store_worktree_filesystem._copy_seed_tree


def test_store_worktree_lifecycle_helpers_match_lifecycle_module() -> None:
    assert store_worktrees.add_worktree is store_worktree_lifecycle.add_worktree
    assert store_worktrees.bind_worktree is store_worktree_lifecycle.bind_worktree
    assert store_worktrees.promote_worktree is store_worktree_lifecycle.promote_worktree
    assert store_worktrees.prune_stale_worktrees is store_worktree_lifecycle.prune_stale_worktrees


def test_store_worktree_rebase_helpers_match_rebase_module() -> None:
    assert store_worktrees.preview_worktree_rebase is store_worktree_rebase.preview_worktree_rebase
    assert store_worktrees.rebase_worktree is store_worktree_rebase.rebase_worktree
    assert store_worktrees.continue_worktree_rebase is store_worktree_rebase.continue_worktree_rebase
    assert store_worktrees.abort_worktree_rebase is store_worktree_rebase.abort_worktree_rebase


def test_store_worktree_restore_helpers_match_restore_module() -> None:
    assert store_worktrees.recreate_worktree is store_worktree_restore.recreate_worktree
    assert store_worktrees.restore_owned_head is store_worktree_restore.restore_owned_head
    assert store_worktrees.sync_worktree is store_worktree_restore.sync_worktree
    assert store_worktrees.sync_all_worktrees is store_worktree_restore.sync_all_worktrees


def test_store_worktree_cleanup_helpers_match_cleanup_module() -> None:
    assert store_worktrees.list_worktree_cleanup_candidates is store_worktree_cleanup.list_worktree_cleanup_candidates
    assert store_worktrees.cleanup_worktrees is store_worktree_cleanup.cleanup_worktrees
    assert store_worktrees.touch_worktree_usage is store_worktree_cleanup.touch_worktree_usage
    assert store_worktrees.remove_worktree is store_worktree_cleanup.remove_worktree
    assert store_worktrees.remove_worktrees is store_worktree_cleanup.remove_worktrees
    assert store_worktrees._update_worktree_registration is store_worktree_cleanup._update_worktree_registration


def test_store_worktree_lifecycle_is_extracted_from_worktree_facade() -> None:
    worktree_text = (WORKSPACE_ROOT / "src/ait/store_worktrees.py").read_text(encoding="utf-8")
    lifecycle_text = (WORKSPACE_ROOT / "src/ait/store_worktree_lifecycle.py").read_text(encoding="utf-8")

    assert "from .store_worktree_lifecycle import (" in worktree_text
    assert "def add_worktree(" not in worktree_text
    assert "def bind_worktree(" not in worktree_text
    assert "def promote_worktree(" not in worktree_text
    assert "def prune_stale_worktrees(" not in worktree_text
    assert "from .store import (" not in lifecycle_text


def test_store_worktree_cleanup_is_extracted_from_worktree_facade() -> None:
    worktree_text = (WORKSPACE_ROOT / "src/ait/store_worktrees.py").read_text(encoding="utf-8")
    cleanup_text = (WORKSPACE_ROOT / "src/ait/store_worktree_cleanup.py").read_text(encoding="utf-8")

    assert "from .store_worktree_cleanup import (" in worktree_text
    assert "def list_worktree_cleanup_candidates(" not in worktree_text
    assert "def cleanup_worktrees(" not in worktree_text
    assert "def touch_worktree_usage(" not in worktree_text
    assert "def remove_worktree(" not in worktree_text
    assert "def remove_worktrees(" not in worktree_text
    assert "from .store import (" not in cleanup_text


def test_store_worktree_restore_is_extracted_from_worktree_facade() -> None:
    worktree_text = (WORKSPACE_ROOT / "src/ait/store_worktrees.py").read_text(encoding="utf-8")
    restore_text = (WORKSPACE_ROOT / "src/ait/store_worktree_restore.py").read_text(encoding="utf-8")

    assert "from .store_worktree_restore import (" in worktree_text
    assert "def recreate_worktree(" not in worktree_text
    assert "def restore_owned_head(" not in worktree_text
    assert "def sync_worktree(" not in worktree_text
    assert "def sync_all_worktrees(" not in worktree_text
    assert "from .store import (" not in restore_text


def test_list_worktrees_hoists_shared_indexes_for_cached_status_reads(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")
    local_content.create_snapshot(ctx, "repo", "main", "seed")
    store.create_line(ctx, "feature/a")
    store.create_line(ctx, "feature/b")
    worktree_root = tmp_path / "worktrees"
    store.add_worktree(
        ctx,
        "wt-a",
        line_name="feature/a",
        path=str(worktree_root / "wt-a"),
        alias_path=str(repo / ".ait" / "worktree-links" / "wt-a"),
    )
    store.add_worktree(
        ctx,
        "wt-b",
        line_name="feature/b",
        path=str(worktree_root / "wt-b"),
        alias_path=str(repo / ".ait" / "worktree-links" / "wt-b"),
    )

    counts = {"sessions": 0, "tasks": 0, "changes": 0, "chains": 0}
    original_list_sessions = local_control.list_workflow_sessions
    original_list_tasks = local_control.list_workflow_tasks
    original_list_changes = local_control.list_workflow_changes
    original_collect_snapshot_chain = local_content.collect_snapshot_chain

    def counted_list_sessions(*args, **kwargs):
        counts["sessions"] += 1
        return original_list_sessions(*args, **kwargs)

    def counted_list_tasks(*args, **kwargs):
        counts["tasks"] += 1
        return original_list_tasks(*args, **kwargs)

    def counted_list_changes(*args, **kwargs):
        counts["changes"] += 1
        return original_list_changes(*args, **kwargs)

    def counted_collect_snapshot_chain(*args, **kwargs):
        counts["chains"] += 1
        return original_collect_snapshot_chain(*args, **kwargs)

    monkeypatch.setattr(local_control, "list_workflow_sessions", counted_list_sessions)
    monkeypatch.setattr(local_control, "list_workflow_tasks", counted_list_tasks)
    monkeypatch.setattr(local_control, "list_workflow_changes", counted_list_changes)
    monkeypatch.setattr(store_worktree_state.local_content, "collect_snapshot_chain", counted_collect_snapshot_chain)

    rows = store_worktrees.list_worktrees(ctx, refresh_status=False)

    assert {row["name"] for row in rows} == {"wt-a", "wt-b"}
    assert counts["sessions"] == 1
    assert counts["tasks"] == 1
    assert counts["changes"] == 1
    assert counts["chains"] <= 2
