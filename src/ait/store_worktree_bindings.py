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
    fork_snapshot_id: str | None = None,
    forked_from_line: str | None = None,
    target_base_line: str | None = None,
) -> None:
    requested_task_id = canonical_bound_task_id(ctx, task_id)
    change_task_id = bound_task_id_for_change(ctx, change_id)
    if requested_task_id is not None and change_task_id is not None and requested_task_id != change_task_id:
        raise ValueError(
            f"Local change `{change_id}` belongs to task `{change_task_id}`, not `{task_id}`."
        )
    repo_ctx = _repo_worktree_ctx(ctx)
    resolved_change_id = normalize_optional_text(change_id)
    if resolved_change_id is not None:
        try:
            change = local_control.get_workflow_change(repo_ctx, resolved_change_id)
        except KeyError:
            change = None
        if isinstance(change, dict):
            expected_fork_snapshot_id = normalize_optional_text(change.get("fork_snapshot_id"))
            expected_forked_from_line = normalize_optional_text(change.get("forked_from_line")) or normalize_optional_text(
                change.get("base_line")
            )
            expected_target_base_line = normalize_optional_text(change.get("base_line")) or expected_forked_from_line
            candidate_fork_snapshot_id = (
                normalize_optional_text(fork_snapshot_id)
                if fork_snapshot_id is not None
                else normalize_optional_text(metadata.get("fork_snapshot_id"))
            )
            candidate_forked_from_line = (
                normalize_optional_text(forked_from_line)
                if forked_from_line is not None
                else normalize_optional_text(metadata.get("forked_from_line"))
            )
            candidate_target_base_line = (
                normalize_optional_text(target_base_line)
                if target_base_line is not None
                else normalize_optional_text(metadata.get("target_base_line")) or candidate_forked_from_line
            )

            if (
                expected_fork_snapshot_id is not None
                and candidate_fork_snapshot_id is not None
                and candidate_fork_snapshot_id != expected_fork_snapshot_id
            ):
                raise ValueError(
                    f"Worktree `{worktree_name}` cannot bind change `{resolved_change_id}` with fork snapshot "
                    f"`{candidate_fork_snapshot_id}` because the local change forks from `{expected_fork_snapshot_id}`."
                )
            if (
                expected_forked_from_line is not None
                and candidate_forked_from_line is not None
                and candidate_forked_from_line != expected_forked_from_line
            ):
                raise ValueError(
                    f"Worktree `{worktree_name}` cannot bind change `{resolved_change_id}` from line "
                    f"`{candidate_forked_from_line}` because the local change forks from `{expected_forked_from_line}`."
                )
            if (
                expected_target_base_line is not None
                and candidate_target_base_line is not None
                and candidate_target_base_line != expected_target_base_line
            ):
                raise ValueError(
                    f"Worktree `{worktree_name}` cannot target base line `{candidate_target_base_line}` for change "
                    f"`{resolved_change_id}` because the local change targets `{expected_target_base_line}`."
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
