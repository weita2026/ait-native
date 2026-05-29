from __future__ import annotations

from ait_protocol.common import generate_namespaced_workflow_id

from . import local_control
from .repo_paths import RepoContext
from .store_local_workflow_runtime import _local_actor_identity, current_line
from .store_repo_config import (
    _load_worktree_config,
    effective_id_namespace_prefix,
    load_config,
)

__all__ = [
    "_local_actor_identity",
    "create_local_session",
    "list_local_sessions",
    "get_local_session",
    "append_local_session_event",
    "list_local_session_events",
    "create_local_checkpoint",
    "list_local_checkpoints",
    "get_local_checkpoint",
    "resume_local_session",
    "close_local_session",
]


def create_local_session(
    ctx: RepoContext,
    session_kind: str,
    *,
    task_id: str | None = None,
    change_id: str | None = None,
    title: str | None = None,
    line_name: str | None = None,
    worktree_name: str | None = None,
    model_name: str | None = None,
    metadata: dict | None = None,
) -> dict:
    repo_name = load_config(ctx).get("repo_name") or ctx.root.name
    resolved_line = line_name or current_line(ctx)
    resolved_worktree_name = worktree_name
    if resolved_worktree_name is None and ctx.is_worktree:
        resolved_worktree_name = _load_worktree_config(ctx).get("worktree_name")
    actor_identity = _local_actor_identity(ctx)
    row = local_control.create_workflow_session(
        ctx,
        generate_namespaced_workflow_id("S", effective_id_namespace_prefix(ctx)),
        repo_name,
        session_kind,
        task_id=task_id,
        change_id=change_id,
        title=title,
        line_name=resolved_line,
        worktree_name=resolved_worktree_name,
        model_name=model_name,
        actor_identity=actor_identity,
        actor_type="human",
        metadata=metadata,
    )
    local_control.record_event(
        ctx,
        "session.local_created",
        "session",
        row["session_id"],
        {
            "session_id": row["session_id"],
            "repo_name": repo_name,
            "session_kind": row["session_kind"],
            "task_id": row.get("task_id"),
            "change_id": row.get("change_id"),
            "line_name": row.get("line_name"),
        },
    )
    return row


def list_local_sessions(ctx: RepoContext, *, status: str | None = None) -> list[dict]:
    return local_control.list_workflow_sessions(ctx, status=status)


def get_local_session(ctx: RepoContext, session_id: str) -> dict:
    return local_control.get_workflow_session(ctx, session_id)


def append_local_session_event(
    ctx: RepoContext,
    session_id: str,
    event_type: str,
    payload: dict | None = None,
    *,
    actor_identity: str | None = None,
    actor_type: str = "human",
) -> dict:
    row = local_control.append_workflow_session_event(
        ctx,
        session_id,
        event_type,
        payload,
        actor_identity=actor_identity or _local_actor_identity(ctx),
        actor_type=actor_type,
    )
    local_control.record_event(
        ctx,
        "session.event_appended",
        "session",
        session_id,
        {"session_id": session_id, "sequence": row["sequence"], "event_type": row["event_type"]},
    )
    return row


def list_local_session_events(
    ctx: RepoContext,
    session_id: str,
    *,
    after_sequence: int = 0,
    limit: int = 200,
) -> list[dict]:
    return local_control.list_workflow_session_events(ctx, session_id, after_sequence=after_sequence, limit=limit)


def create_local_checkpoint(
    ctx: RepoContext,
    session_id: str,
    summary: str,
    *,
    snapshot_id: str | None = None,
    resume_payload: dict | None = None,
    based_on_sequence: int | None = None,
) -> dict:
    row = local_control.create_workflow_checkpoint(
        ctx,
        generate_namespaced_workflow_id("K", effective_id_namespace_prefix(ctx)),
        session_id,
        summary,
        snapshot_id=snapshot_id,
        resume_payload=resume_payload,
        based_on_sequence=based_on_sequence,
    )
    local_control.record_event(
        ctx,
        "checkpoint.created",
        "checkpoint",
        row["checkpoint_id"],
        {
            "checkpoint_id": row["checkpoint_id"],
            "session_id": session_id,
            "based_on_sequence": row["based_on_sequence"],
            "snapshot_id": row.get("snapshot_id"),
        },
    )
    return row


def list_local_checkpoints(ctx: RepoContext, session_id: str) -> list[dict]:
    return local_control.list_workflow_checkpoints(ctx, session_id)


def get_local_checkpoint(ctx: RepoContext, checkpoint_id: str) -> dict:
    return local_control.get_workflow_checkpoint(ctx, checkpoint_id)


def resume_local_session(
    ctx: RepoContext,
    session_id: str,
    *,
    after_sequence: int | None = None,
    limit: int = 200,
) -> dict:
    row = local_control.resume_workflow_session(ctx, session_id, after_sequence=after_sequence, limit=limit)
    local_control.record_event(
        ctx,
        "session.resumed",
        "session",
        session_id,
        {
            "session_id": session_id,
            "resume_after_sequence": row["resume_after_sequence"],
            "latest_checkpoint_id": (row.get("latest_checkpoint") or {}).get("checkpoint_id"),
            "pending_event_count": len(row.get("pending_events") or []),
        },
    )
    return row


def close_local_session(ctx: RepoContext, session_id: str, status: str) -> dict:
    row = local_control.close_workflow_session(ctx, session_id, status)
    local_control.record_event(
        ctx,
        "session.closed",
        "session",
        session_id,
        {"session_id": session_id, "status": row["status"]},
    )
    return row
