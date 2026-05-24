from __future__ import annotations

from typing import Any

from ..snapshot_diff import snapshot_diff as build_snapshot_diff
from ..store import (
    RepoContext,
    ensure_main_seed_mirror as local_ensure_main_seed_mirror,
    get_line,
    restore_workspace as local_restore_workspace,
    workspace_status as local_workspace_status,
)
from .line_transport_helpers import _pull_line
from .plan_sync_scope import _preserve_workspace_paths_for_plan_sync
from .task_tracking_bindings import _default_line_name, _task_worktree_repo_ctx
from .workspace_command_locking import _run_locked_workspace_command


def _restore_repo_root_after_land(
    ctx: RepoContext,
    *,
    target_line: str,
    previous_head_snapshot_id: str | None = None,
) -> dict[str, Any]:
    repo_ctx = _task_worktree_repo_ctx(ctx)
    default_line = _default_line_name(repo_ctx)
    payload: dict[str, Any] = {
        "default_line": default_line,
        "line": target_line,
        "workspace_root": str(repo_ctx.root.resolve()),
        "previous_head_snapshot_id": previous_head_snapshot_id,
    }
    if target_line != default_line:
        payload["status"] = "skipped"
        payload["reason"] = "target_not_default_line"
        return payload
    target_line_row = get_line(repo_ctx, target_line)
    target_snapshot_id = str(target_line_row.get("head_snapshot_id") or "").strip() or None
    payload["target_snapshot_id"] = target_snapshot_id
    landed_diff_paths: set[str] = set()
    if previous_head_snapshot_id and target_snapshot_id:
        try:
            diff = build_snapshot_diff(repo_ctx, previous_head_snapshot_id, target_snapshot_id)
            landed_diff_paths = {
                str(row.get("path") or "").strip()
                for row in diff.get("files") or []
                if str(row.get("path") or "").strip()
            }
        except Exception as exc:  # pragma: no cover - defensive metadata aid only.
            payload["landed_diff_error"] = str(exc)
    payload["landed_diff_paths"] = sorted(landed_diff_paths)
    initial_status = local_workspace_status(repo_ctx)
    initial_changed_paths = {str(path) for path in initial_status.get("changed_paths") or []}
    unrelated_paths = sorted(initial_changed_paths - landed_diff_paths)
    payload["initial_changed_paths"] = sorted(initial_changed_paths)
    payload["unrelated_paths"] = unrelated_paths
    try:
        with _preserve_workspace_paths_for_plan_sync(
            repo_ctx,
            paths=set(unrelated_paths),
            lock_reason="land auto sync preserve unrelated repo-root paths",
        ) as preserved:
            restored = _run_locked_workspace_command(
                repo_ctx,
                "land auto sync restore",
                lambda: local_restore_workspace(
                    repo_ctx,
                    line_name=target_line,
                    force=True,
                    dry_run=False,
                    switch_current_line=True,
                ),
            )
            payload["preserved_unrelated_paths"] = list(preserved.get("paths") or [])
    except Exception as exc:  # pragma: no cover - defensive; remote land already succeeded.
        payload["status"] = "failed"
        payload["detail"] = str(exc)
        return payload
    final_status = local_workspace_status(repo_ctx)
    payload.update(restored)
    payload["remaining_paths"] = sorted(str(path) for path in final_status.get("changed_paths") or [])
    payload["status"] = "restored"
    payload["main_seed_sync"] = local_ensure_main_seed_mirror(repo_ctx, line_name=target_line)
    return payload


def _attach_local_land_sync(ctx: RepoContext, remote_name: str | None, land_result: dict[str, Any]) -> dict[str, Any]:
    if land_result.get("status") != "succeeded":
        return land_result
    result = land_result.get("result") if isinstance(land_result.get("result"), dict) else {}
    target_line = str(result.get("target_line") or "").strip()
    landed_snapshot_id = str(result.get("landed_snapshot_id") or "").strip()
    if not target_line or not landed_snapshot_id:
        return land_result
    try:
        repo_ctx = _task_worktree_repo_ctx(ctx)
        previous_line_row = get_line(repo_ctx, target_line)
        previous_head_snapshot_id = str(previous_line_row.get("head_snapshot_id") or "").strip() or None
        sync = _pull_line(repo_ctx, remote_name, target_line)
        sync["landed_snapshot_id"] = landed_snapshot_id
        workspace_restore = _restore_repo_root_after_land(
            ctx,
            target_line=target_line,
            previous_head_snapshot_id=previous_head_snapshot_id,
        )
        sync["workspace_restore"] = workspace_restore
        sync["status"] = "failed" if workspace_restore.get("status") == "failed" else "synced"
        if workspace_restore.get("status") == "failed":
            sync["error"] = str(workspace_restore.get("detail") or "repo root restore failed")
    except Exception as exc:  # pragma: no cover - defensive; the remote land already succeeded.
        sync = {
            "status": "failed",
            "line": target_line,
            "landed_snapshot_id": landed_snapshot_id,
            "error": str(exc),
        }
    land_result["local_sync"] = sync
    return land_result
