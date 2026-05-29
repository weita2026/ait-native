from __future__ import annotations

from pathlib import Path
from typing import Iterable

from . import local_content, local_content_snapshots, local_control
from .repo_paths import RepoContext
from .store_repo_config import _set_worktree_materialized_snapshot, load_config
from .store_worktree_runtime import _set_current_line, current_line


def restore_workspace(
    ctx: RepoContext,
    *,
    snapshot_id: str | None = None,
    line_name: str | None = None,
    force: bool = False,
    dry_run: bool = False,
    switch_current_line: bool = True,
) -> dict:
    if snapshot_id is not None and line_name is not None:
        raise ValueError("Choose either snapshot_id or line_name, not both.")

    current_line_name = current_line(ctx)
    current_line_row = local_content.get_line(ctx, current_line_name)
    baseline_snapshot_id = current_line_row.get("head_snapshot_id")

    target_line_name = line_name or current_line_name
    target_snapshot_id = snapshot_id
    if target_snapshot_id is None:
        target_line_row = local_content.get_line(ctx, target_line_name)
        target_snapshot_id = target_line_row.get("head_snapshot_id")

    result = local_content_snapshots.restore_workspace(
        ctx,
        target_snapshot_id,
        baseline_snapshot_id=baseline_snapshot_id,
        force=force,
        dry_run=dry_run,
    )
    result.update(
        {
            "repo_name": load_config(ctx).get("repo_name") or ctx.root.name,
            "current_line_before": current_line_name,
            "current_line": current_line_name,
            "line_name": target_line_name,
            "line_head_snapshot_id": target_snapshot_id,
        }
    )
    if not dry_run:
        _set_worktree_materialized_snapshot(ctx, target_snapshot_id)
    if line_name is not None and switch_current_line and not dry_run:
        _set_current_line(ctx, target_line_name)
        result["current_line"] = target_line_name
    elif line_name is not None and switch_current_line:
        result["current_line"] = target_line_name
    if not dry_run:
        local_control.record_event(
            ctx,
            "workspace.restored",
            "workspace",
            target_snapshot_id or "empty",
            {
                "target_snapshot_id": target_snapshot_id,
                "baseline_snapshot_id": baseline_snapshot_id,
                "current_line_before": current_line_name,
                "current_line_after": result["current_line"],
                "line_name": target_line_name,
                "write_count": result["plan"]["write_count"],
                "remove_count": result["plan"]["remove_count"],
                "force": force,
            },
        )
    return result


def restore_workspace_paths(
    ctx: RepoContext,
    paths: Iterable[str | Path],
    *,
    snapshot_id: str | None = None,
    line_name: str | None = None,
    force: bool = False,
    dry_run: bool = False,
) -> dict:
    if snapshot_id is not None and line_name is not None:
        raise ValueError("Choose either snapshot_id or line_name, not both.")

    current_line_name = current_line(ctx)
    current_line_row = local_content.get_line(ctx, current_line_name)
    baseline_snapshot_id = current_line_row.get("head_snapshot_id")

    target_line_name = line_name or current_line_name
    target_snapshot_id = snapshot_id
    if target_snapshot_id is None:
        target_line_row = local_content.get_line(ctx, target_line_name)
        target_snapshot_id = target_line_row.get("head_snapshot_id")
    if target_snapshot_id is None:
        raise ValueError(f"Line {target_line_name} has no head snapshot to restore selected paths from.")

    result = local_content_snapshots.restore_workspace_paths(
        ctx,
        target_snapshot_id,
        paths,
        baseline_snapshot_id=baseline_snapshot_id,
        force=force,
        dry_run=dry_run,
    )
    result.update(
        {
            "repo_name": load_config(ctx).get("repo_name") or ctx.root.name,
            "current_line": current_line_name,
            "line_name": target_line_name,
            "line_head_snapshot_id": target_snapshot_id,
        }
    )
    if not dry_run:
        local_control.record_event(
            ctx,
            "workspace.paths_restored",
            "workspace",
            target_snapshot_id,
            {
                "target_snapshot_id": target_snapshot_id,
                "baseline_snapshot_id": baseline_snapshot_id,
                "current_line": current_line_name,
                "line_name": target_line_name,
                "write_count": result["plan"]["write_count"],
                "remove_count": result["plan"]["remove_count"],
                "requested_paths": result["plan"]["requested_paths"],
                "force": force,
            },
        )
    return result
