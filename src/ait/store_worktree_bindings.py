from __future__ import annotations

from typing import Any

from ait_protocol.common import normalize_optional_text

from . import local_control
from .repo_paths import RepoContext
from .store_worktree_state import _repo_worktree_ctx


def canonical_bound_task_id(ctx: RepoContext, task_id: str | None) -> str | None:
    resolved_task_id = normalize_optional_text(task_id)
    if resolved_task_id is None:
        return None
    repo_ctx = _repo_worktree_ctx(ctx)
    try:
        task = local_control.get_workflow_task(repo_ctx, resolved_task_id)
    except KeyError:
        return resolved_task_id
    return normalize_optional_text(task.get("task_id")) or resolved_task_id


def bound_task_id_for_change(ctx: RepoContext, change_id: str | None) -> str | None:
    resolved_change_id = normalize_optional_text(change_id)
    if resolved_change_id is None:
        return None
    repo_ctx = _repo_worktree_ctx(ctx)
    try:
        change = local_control.get_workflow_change(repo_ctx, resolved_change_id)
    except KeyError:
        return None
    return canonical_bound_task_id(ctx, change.get("task_id"))


def bound_task_id_for_metadata(ctx: RepoContext, metadata: dict[str, Any]) -> str | None:
    bound_task_id = canonical_bound_task_id(ctx, metadata.get("bound_task_id"))
    if bound_task_id is not None:
        return bound_task_id
    return bound_task_id_for_change(ctx, metadata.get("bound_change_id"))


def guard_worktree_binding_task_lineage(
    ctx: RepoContext,
    *,
    worktree_name: str,
    metadata: dict[str, Any],
    task_id: str | None,
    change_id: str | None,
) -> None:
    requested_task_id = canonical_bound_task_id(ctx, task_id)
    change_task_id = bound_task_id_for_change(ctx, change_id)
    if requested_task_id is not None and change_task_id is not None and requested_task_id != change_task_id:
        raise ValueError(
            f"Local change `{change_id}` belongs to task `{change_task_id}`, not `{task_id}`."
        )
    current_task_id = bound_task_id_for_metadata(ctx, metadata)
    requested_bound_task_id = requested_task_id or change_task_id
    if current_task_id is None or requested_bound_task_id is None or current_task_id == requested_bound_task_id:
        return
    current_display = normalize_optional_text(metadata.get("bound_task_id")) or current_task_id
    requested_display = normalize_optional_text(task_id) or requested_bound_task_id
    raise ValueError(
        f"Worktree `{worktree_name}` is already bound to task `{current_display}` "
        f"and cannot be rebound to task `{requested_display}`."
    )
