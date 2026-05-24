from __future__ import annotations

from typing import Optional

from . import local_content, local_control
from .repo_paths import RepoContext
from .store_repo_config import _set_worktree_materialized_snapshot, _worktree_materialized_snapshot_id
from .store_worktree_runtime import current_line


def create_stash(ctx: RepoContext, message: Optional[str], *, keep_workspace: bool = False) -> dict:
    repo_name = local_control.get_meta(ctx, "repo_name") or ctx.root.name
    line_name = current_line(ctx)
    base_snapshot_id = local_content.read_ref(ctx, line_name)
    dirty = local_content.workspace_delta(ctx, base_snapshot_id)
    if dirty["clean"]:
        raise ValueError("Workspace is already clean; stash save requires local changes to park.")
    snapshot = local_content.create_snapshot(
        ctx,
        repo_name,
        line_name,
        message,
        parent_snapshot_id=base_snapshot_id,
        update_line_ref=False,
        snapshot_kind="stash",
        touch_line=False,
    )
    stash = local_content.create_stash(
        ctx,
        snapshot_id=snapshot["snapshot_id"],
        source_line_name=line_name,
        base_snapshot_id=base_snapshot_id,
        message=message,
        workspace_cleared=not keep_workspace,
    )
    if keep_workspace:
        _set_worktree_materialized_snapshot(ctx, snapshot["snapshot_id"])
    else:
        local_content.restore_workspace(
            ctx,
            base_snapshot_id,
            baseline_snapshot_id=snapshot["snapshot_id"],
            force=True,
        )
        _set_worktree_materialized_snapshot(ctx, base_snapshot_id)
    local_control.record_event(
        ctx,
        "stash.saved",
        "stash",
        stash["stash_id"],
        {
            "stash_id": stash["stash_id"],
            "snapshot_id": stash["snapshot_id"],
            "source_line_name": line_name,
            "base_snapshot_id": base_snapshot_id,
            "keep_workspace": keep_workspace,
        },
    )
    return {
        **stash,
        "current_line": line_name,
        "line_head_snapshot_id_before": base_snapshot_id,
        "line_head_snapshot_id_after": local_content.read_ref(ctx, line_name),
        "workspace_cleared": not keep_workspace,
        "dirty_workspace": dirty,
    }


def list_stashes(ctx: RepoContext) -> list[dict]:
    return local_content.list_stashes(ctx)


def get_stash(ctx: RepoContext, stash_id: str) -> dict:
    return local_content.get_stash(ctx, stash_id)


def apply_stash(ctx: RepoContext, stash_id: str, *, force: bool = False, drop: bool = False) -> dict:
    stash = local_content.get_stash(ctx, stash_id)
    line_name = current_line(ctx)
    line_head_snapshot_id = local_content.read_ref(ctx, line_name)
    local_content.restore_workspace(
        ctx,
        stash["snapshot_id"],
        baseline_snapshot_id=line_head_snapshot_id,
        force=force,
    )
    _set_worktree_materialized_snapshot(ctx, stash["snapshot_id"])
    local_control.record_event(
        ctx,
        "stash.applied",
        "stash",
        stash["stash_id"],
        {
            "stash_id": stash["stash_id"],
            "snapshot_id": stash["snapshot_id"],
            "current_line": line_name,
            "force": force,
            "drop": drop,
        },
    )
    payload = {
        **stash,
        "current_line": line_name,
        "line_head_snapshot_id_before": line_head_snapshot_id,
        "line_head_snapshot_id_after": local_content.read_ref(ctx, line_name),
        "applied": True,
        "dropped": False,
        "workspace_restored_from_stash": True,
    }
    if not drop:
        return payload
    dropped = local_content.drop_stash(ctx, stash_id)
    if dropped["snapshot_deleted"]:
        _set_worktree_materialized_snapshot(ctx, None)
    local_control.record_event(
        ctx,
        "stash.dropped",
        "stash",
        stash_id,
        {
            "stash_id": stash_id,
            "snapshot_id": stash["snapshot_id"],
            "drop_reason": "pop",
        },
    )
    payload.update(
        {
            "dropped": True,
            "snapshot_deleted": dropped["snapshot_deleted"],
        }
    )
    return payload


def drop_stash(ctx: RepoContext, stash_id: str) -> dict:
    stash = local_content.drop_stash(ctx, stash_id)
    if stash["snapshot_deleted"] and _worktree_materialized_snapshot_id(ctx) == stash["snapshot_id"]:
        _set_worktree_materialized_snapshot(ctx, None)
    local_control.record_event(
        ctx,
        "stash.dropped",
        "stash",
        stash_id,
        {
            "stash_id": stash_id,
            "snapshot_id": stash["snapshot_id"],
            "drop_reason": "explicit",
        },
    )
    return stash
