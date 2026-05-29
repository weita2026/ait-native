from __future__ import annotations

from typing import Any

from ..repo_paths import RepoContext
from ..store_local_changes import (
    get_local_change,
)
from ..store import list_worktrees as local_list_worktrees
from .task_tracking_bindings import _task_worktree_repo_ctx
from .task_worktree_resolution import _find_bound_task_worktree
from .workflow_mode_config import _normalize_text_value


def _preferred_bound_worktree(rows: list[dict[str, Any]]) -> dict[str, Any] | None:
    if not rows:
        return None
    rows.sort(
        key=lambda row: (
            bool(row.get("auto_created_for_task")),
            str(row.get("created_at") or ""),
        ),
        reverse=True,
    )
    return rows[0]


def _bound_task_worktree_for_local_change(
    ctx: RepoContext,
    *,
    task_id: str | None = None,
    change_id: str | None = None,
) -> dict[str, Any] | None:
    repo_ctx = _task_worktree_repo_ctx(ctx)
    resolved_change_id = _normalize_text_value(change_id)
    if resolved_change_id is not None:
        bound_change_rows = [
            row
            for row in local_list_worktrees(repo_ctx)
            if _normalize_text_value(row.get("bound_change_id")) == resolved_change_id
        ]
        preferred_change_row = _preferred_bound_worktree(bound_change_rows)
        if preferred_change_row is not None:
            return preferred_change_row
    resolved_task_id = _normalize_text_value(task_id)
    if resolved_task_id is None and resolved_change_id is not None:
        try:
            resolved_task_id = _normalize_text_value(get_local_change(ctx, resolved_change_id).get("task_id"))
        except KeyError:
            resolved_task_id = None
    if resolved_task_id is None:
        return None
    return _find_bound_task_worktree(repo_ctx, resolved_task_id)


def _bound_task_worktree_retarget_state(
    ctx: RepoContext,
    *,
    task_id: str | None = None,
    change_id: str | None = None,
) -> dict[str, Any] | None:
    worktree = _bound_task_worktree_for_local_change(ctx, task_id=task_id, change_id=change_id)
    if not isinstance(worktree, dict):
        return None
    retarget = worktree.get("retarget") if isinstance(worktree.get("retarget"), dict) else {}
    if not retarget:
        return None
    payload = dict(retarget)
    payload.setdefault("worktree_name", _normalize_text_value(worktree.get("name")))
    payload.setdefault(
        "current_line",
        _normalize_text_value(worktree.get("current_line"))
        or _normalize_text_value(worktree.get("registered_line_name")),
    )
    return payload


def _bound_task_worktree_retarget_error(
    retarget: dict[str, Any] | None,
    *,
    operation: str,
) -> str | None:
    if not isinstance(retarget, dict):
        return None
    target_base_line = _normalize_text_value(retarget.get("target_base_line")) or "main"
    worktree_name = _normalize_text_value(retarget.get("worktree_name")) or "bound worktree"
    rebase_state = str(retarget.get("rebase_state") or "idle")
    if rebase_state == "conflicted":
        sample = ", ".join(str(path).strip() for path in (retarget.get("rebase_conflict_paths") or [])[:5] if str(path).strip())
        sample = sample or "resolve conflicts first"
        return (
            f"Bound worktree `{worktree_name}` has a conflicted rebase in progress: {sample}. "
            f"Run `ait worktree rebase --continue` or `ait worktree rebase --abort` before {operation}."
        )
    if bool(retarget.get("needs_retarget")):
        fork_snapshot_id = _normalize_text_value(retarget.get("fork_snapshot_id")) or "unknown"
        target_base_snapshot_id = _normalize_text_value(retarget.get("target_base_snapshot_id")) or "unknown"
        return (
            f"Bound worktree `{worktree_name}` still forks from `{fork_snapshot_id}` while "
            f"`{target_base_line}` now points at `{target_base_snapshot_id}`. "
            f"Run `ait worktree rebase --onto {target_base_line}` before {operation}."
        )
    return None


def _require_fresh_bound_task_worktree(
    ctx: RepoContext,
    *,
    task_id: str | None = None,
    change_id: str | None = None,
    operation: str,
) -> None:
    retarget = _bound_task_worktree_retarget_state(ctx, task_id=task_id, change_id=change_id)
    message = _bound_task_worktree_retarget_error(retarget, operation=operation)
    if message:
        raise ValueError(message)
