from __future__ import annotations

import time
from pathlib import Path
from typing import Any, Iterable, Optional

from ait_protocol.common import normalize_optional_text

from . import local_content, local_content_snapshots, local_control
from .command_profiling import _command_profile_elapsed_ms, _command_profile_record_phase
from .repo_paths import RepoContext
from .store_line_cleanup import list_line_cleanup_candidates
from .store_repo_config import _load_worktree_config, load_config
from .store_worktree_runtime import DEFAULT_LINE_CLEANUP_OLDER_THAN, current_line, set_line_head


def get_line(ctx: RepoContext, name: Optional[str] = None) -> dict:
    return local_content.get_line(ctx, name or current_line(ctx))


def iter_workspace_files(root: Path) -> Iterable[Path]:
    return local_content.iter_workspace_files(root)


def snapshot_exists(ctx: RepoContext, snapshot_id: str) -> bool:
    return local_content_snapshots.snapshot_exists(ctx, snapshot_id)


def list_snapshots(ctx: RepoContext) -> list[dict]:
    return local_content_snapshots.list_snapshots(ctx)


def get_snapshot(ctx: RepoContext, snapshot_id: str) -> dict:
    return local_content_snapshots.get_snapshot(ctx, snapshot_id)


def move_ref(ctx: RepoContext, line_name: str, snapshot_id: str) -> dict:
    if not snapshot_exists(ctx, snapshot_id):
        raise KeyError(f"Unknown snapshot: {snapshot_id}")
    return set_line_head(ctx, line_name, snapshot_id)


def ref_history(
    ctx: RepoContext,
    name: str | None = None,
    *,
    limit: int = 20,
) -> dict[str, Any]:
    resolved_name = str(name or f"lines/{current_line(ctx)}").strip()
    if not resolved_name.startswith("lines/"):
        raise ValueError("Only lines/* refs are supported in this starter.")
    if limit < 1:
        raise ValueError("--limit must be at least 1")
    line_name = resolved_name.split("/", 1)[1]
    line_row = get_line(ctx, line_name)
    current_target_snapshot_id = normalize_optional_text(line_row.get("head_snapshot_id"))

    snapshots: list[dict[str, Any]] = []
    if current_target_snapshot_id is not None:
        chain = collect_snapshot_chain(ctx, current_target_snapshot_id)
        for position_from_head, ancestor_snapshot_id in enumerate(reversed(chain)):
            if position_from_head >= limit:
                break
            snapshot = get_snapshot(ctx, ancestor_snapshot_id)
            snapshots.append(
                {
                    "snapshot_id": snapshot["snapshot_id"],
                    "parent_snapshot_id": snapshot.get("parent_snapshot_id"),
                    "created_at": snapshot.get("created_at"),
                    "message": snapshot.get("message"),
                    "file_count": snapshot.get("file_count"),
                    "position_from_head": position_from_head,
                    "is_current_target": ancestor_snapshot_id == current_target_snapshot_id,
                }
            )

    move_events = []
    for row in local_control.list_line_events(ctx, line_name, limit=limit):
        payload = row.get("payload") if isinstance(row.get("payload"), dict) else {}
        move_events.append(
            {
                "event_id": row.get("event_id"),
                "event_type": row.get("event_type"),
                "created_at": row.get("created_at"),
                "name": resolved_name,
                "line_name": line_name,
                "target_snapshot_id": normalize_optional_text(payload.get("head_snapshot_id")),
                "previous_target_snapshot_id": normalize_optional_text(payload.get("previous_head_snapshot_id")),
            }
        )

    return {
        "repo_name": load_config(ctx).get("repo_name") or ctx.root.name,
        "name": resolved_name,
        "line_name": line_name,
        "line_status": line_row.get("status") or "active",
        "current_target_snapshot_id": current_target_snapshot_id,
        "limit": limit,
        "snapshot_count": len(snapshots),
        "move_event_count": len(move_events),
        "snapshots": snapshots,
        "move_events": move_events,
    }


def repo_status(ctx: RepoContext) -> dict:
    from .store_worktrees import list_worktrees, worktree_doctor_from_rows

    repo_name = local_control.get_meta(ctx, "repo_name") or ctx.root.name
    current = current_line(ctx)
    remote_count = len(local_control.list_remotes(ctx))
    local_content_started = time.perf_counter_ns()
    data = local_content.repo_status(ctx, repo_name, current, remote_count)
    _command_profile_record_phase(
        tracker=None,
        name="local_content.repo_status",
        value={
            "total": _command_profile_elapsed_ms(local_content_started),
            "workspace_delta": data.get("phase_timings_ms"),
        },
    )
    worktree_doctor_started = time.perf_counter_ns()
    worktree_rows = list_worktrees(ctx, refresh_status=False)
    worktree_hygiene = worktree_doctor_from_rows(worktree_rows)
    _command_profile_record_phase(
        tracker=None,
        name="worktree_doctor",
        value=_command_profile_elapsed_ms(worktree_doctor_started),
    )
    line_cleanup_started = time.perf_counter_ns()
    line_hygiene = list_line_cleanup_candidates(
        ctx,
        older_than=DEFAULT_LINE_CLEANUP_OLDER_THAN,
        include_protected=False,
        worktree_rows=worktree_rows,
    )
    _command_profile_record_phase(
        tracker=None,
        name="list_line_cleanup_candidates",
        value=_command_profile_elapsed_ms(line_cleanup_started),
    )
    data["is_worktree"] = ctx.is_worktree
    data["worktree_name"] = _load_worktree_config(ctx).get("worktree_name") if ctx.is_worktree else None
    data["worktree_hygiene"] = {
        "total_count": worktree_hygiene.get("total_count", 0),
        "stale_count": worktree_hygiene.get("stale_count", 0),
        "cleanup_candidate_count": int(worktree_hygiene.get("safe_auto_remove_count", 0))
        + int(worktree_hygiene.get("safe_cleanup_candidate_count", 0)),
        "manual_review_candidate_count": worktree_hygiene.get("manual_review_candidate_count", 0),
        "protected_count": worktree_hygiene.get("protected_count", 0),
    }
    data["line_hygiene"] = {
        "older_than": line_hygiene.get("older_than"),
        "candidate_count": line_hygiene.get("candidate_count", 0),
        "protected_count": line_hygiene.get("protected_count", 0),
        "inspected_count": line_hygiene.get("inspected_count", 0),
    }
    return data


def collect_snapshot_chain(ctx: RepoContext, snapshot_id: str) -> list[str]:
    return local_content.collect_snapshot_chain(ctx, snapshot_id)


def ensure_snapshot_chain(ctx: RepoContext, bundles: list[dict]) -> list[dict]:
    return local_content.ensure_snapshot_chain(ctx, bundles)
