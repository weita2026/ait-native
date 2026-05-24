from __future__ import annotations

import json

from ait_protocol.common import (
    LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX,
    generate_namespaced_workflow_id,
    lane_from_risk,
    normalize_optional_text,
    utc_now,
    workflow_origin_namespace_prefix,
)

from . import local_content, local_control
from .repo_paths import RepoContext
from .store_repo_config import (
    _load_worktree_config,
    effective_id_namespace_prefix,
    load_config,
)
from .store_local_workflow_runtime import current_line, resolve_local_task_plan_linkage
from .store_local_views import (
    _local_change_view,
    _local_plan_revision_view,
    _local_plan_view,
    _local_release_view,
)

def create_local_task(
    ctx: RepoContext,
    title: str,
    intent: str,
    risk_tier: str,
    *,
    plan_id: str | None = None,
    origin_plan_revision_id: str | None = None,
    plan_item_ref: str | None = None,
) -> dict:
    repo_name = load_config(ctx).get("repo_name") or ctx.root.name
    resolved_plan_id, resolved_revision_id, resolved_plan_item_ref = resolve_local_task_plan_linkage(
        ctx,
        plan_id=plan_id,
        origin_plan_revision_id=origin_plan_revision_id,
        plan_item_ref=plan_item_ref,
    )
    planning_state = "planned" if resolved_plan_id is not None else "unplanned"
    plan_linked_at = utc_now() if resolved_plan_id is not None else None
    identity = local_control.allocate_workflow_task_identity(
        ctx,
        repo_name,
        workflow_origin_namespace_prefix(LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX, effective_id_namespace_prefix(ctx)),
    )
    row = local_control.create_workflow_task(
        ctx,
        identity["task_id"],
        repo_name,
        title,
        intent,
        risk_tier,
        task_seq=identity["task_seq"],
        identity_source=local_control.LOCAL_IDENTITY_SOURCE_SEQUENCE,
        planning_state=planning_state,
        plan_id=resolved_plan_id,
        origin_plan_revision_id=resolved_revision_id,
        plan_item_ref=resolved_plan_item_ref,
        plan_linked_at=plan_linked_at,
    )
    local_control.record_event(
        ctx,
        "task.local_created",
        "task",
        row["task_id"],
        {
            "task_id": row["task_id"],
            "repo_name": repo_name,
            "title": title,
            "planning_state": row["planning_state"],
            "plan_id": row.get("plan_id"),
            "origin_plan_revision_id": row.get("origin_plan_revision_id"),
            "plan_item_ref": row.get("plan_item_ref"),
            "plan_linked_at": row.get("plan_linked_at"),
            "publication_state": row["publication_state"],
        },
    )
    return row


def create_local_plan(
    ctx: RepoContext,
    title: str,
    artifact_path: str,
    artifact_selector: str | None,
    artifact_heading: str,
    items: list[dict],
    *,
    artifact_blob_id: str | None = None,
    summary: str | None = None,
    status: str = "draft",
    source_kind: str = "manual_edit",
    source_session_id: str | None = None,
) -> dict:
    repo_name = load_config(ctx).get("repo_name") or ctx.root.name
    actor_identity = _local_actor_identity(ctx)
    row = local_control.create_workflow_plan(
        ctx,
        generate_namespaced_workflow_id("PL", effective_id_namespace_prefix(ctx)),
        generate_namespaced_workflow_id("PR", effective_id_namespace_prefix(ctx)),
        repo_name,
        title,
        artifact_path,
        artifact_selector,
        artifact_heading,
        items,
        artifact_blob_id=artifact_blob_id,
        summary=summary,
        status=status,
        source_kind=source_kind,
        source_session_id=source_session_id,
        created_by=actor_identity,
        actor_type="human",
    )
    payload = _local_plan_view(ctx, row)
    local_control.record_event(
        ctx,
        "plan.local_created",
        "plan",
        payload["plan_id"],
        {
            "plan_id": payload["plan_id"],
            "repo_name": repo_name,
            "title": title,
            "status": payload["status"],
            "publication_state": payload["publication_state"],
            "plan_revision_id": payload["head_revision"]["plan_revision_id"] if payload.get("head_revision") else None,
        },
    )
    return payload


def list_local_plans(ctx: RepoContext) -> list[dict]:
    return local_control.list_workflow_plans(ctx)


def get_local_plan(ctx: RepoContext, plan_id: str) -> dict:
    return _local_plan_view(ctx, local_control.get_workflow_plan(ctx, plan_id))


def list_local_plan_revisions(ctx: RepoContext, plan_id: str) -> list[dict]:
    return [_local_plan_revision_view(row) or {} for row in local_control.list_workflow_plan_revisions(ctx, plan_id)]


def get_local_plan_revision(ctx: RepoContext, plan_id: str, plan_revision_id: str) -> dict:
    return _local_plan_revision_view(local_control.get_workflow_plan_revision(ctx, plan_id, plan_revision_id)) or {}


def revise_local_plan(
    ctx: RepoContext,
    plan_id: str,
    artifact_path: str,
    artifact_selector: str | None,
    artifact_heading: str,
    items: list[dict],
    *,
    artifact_blob_id: str | None = None,
    title: str | None = None,
    summary: str | None = None,
    source_kind: str = "manual_edit",
    source_session_id: str | None = None,
) -> dict:
    actor_identity = _local_actor_identity(ctx)
    row = local_control.revise_workflow_plan(
        ctx,
        plan_id,
        generate_namespaced_workflow_id("PR", effective_id_namespace_prefix(ctx)),
        artifact_path,
        artifact_selector,
        artifact_heading,
        items,
        artifact_blob_id=artifact_blob_id,
        title=title,
        summary=summary,
        source_kind=source_kind,
        source_session_id=source_session_id,
        created_by=actor_identity,
        actor_type="human",
    )
    payload = _local_plan_view(ctx, row)
    local_control.record_event(
        ctx,
        "plan.local_revised",
        "plan",
        plan_id,
        {
            "plan_id": plan_id,
            "plan_revision_id": payload["head_revision"]["plan_revision_id"] if payload.get("head_revision") else None,
            "revision_number": payload["head_revision"]["revision_number"] if payload.get("head_revision") else None,
            "title": payload["title"],
        },
    )
    return payload


def close_local_plan(ctx: RepoContext, plan_id: str, status: str) -> dict:
    row = _local_plan_view(ctx, local_control.close_workflow_plan(ctx, plan_id, status))
    local_control.record_event(
        ctx,
        "plan.closed",
        "plan",
        plan_id,
        {"plan_id": plan_id, "status": row["status"], "publication_state": row["publication_state"]},
    )
    return row


def mark_local_plan_published(
    ctx: RepoContext,
    plan_id: str,
    *,
    remote_name: str | None,
    published_plan_id: str,
    published_head_revision_id: str | None,
    revision_mappings: list[tuple[str, str]],
) -> dict:
    row = _local_plan_view(
        ctx,
        local_control.mark_workflow_plan_published(
            ctx,
            plan_id,
            remote_name=remote_name,
            published_plan_id=published_plan_id,
            published_head_revision_id=published_head_revision_id,
            revision_mappings=revision_mappings,
        ),
    )
    local_control.record_event(
        ctx,
        "plan.published",
        "plan",
        plan_id,
        {
            "plan_id": plan_id,
            "publication_state": row["publication_state"],
            "published_at": row["published_at"],
            "published_head_revision_id": row.get("published_head_revision_id"),
            "published_revision_count": len(revision_mappings),
        },
    )
    return row


_LOCAL_RELEASE_UNSET = object()


def create_local_release(
    ctx: RepoContext,
    version: str,
    line_name: str,
    snapshot_id: str,
    manifest_hash: str,
    profile: str,
    *,
    package_name: str | None = None,
    package_version: str | None = None,
    package_requires_python: str | None = None,
    status: str = "candidate",
    checks: list[dict] | None = None,
    artifacts: list[dict] | None = None,
    formula: dict | None = None,
    metadata: dict | None = None,
) -> dict:
    repo_name = load_config(ctx).get("repo_name") or ctx.root.name
    row = local_control.create_workflow_release(
        ctx,
        generate_namespaced_workflow_id("R", effective_id_namespace_prefix(ctx)),
        repo_name,
        version,
        line_name,
        snapshot_id,
        manifest_hash,
        profile,
        package_name=package_name,
        package_version=package_version,
        package_requires_python=package_requires_python,
        status=status,
        checks=checks,
        artifacts=artifacts,
        formula=formula,
        metadata=metadata,
    )
    payload = _local_release_view(row)
    assert payload is not None
    local_control.record_event(
        ctx,
        "release.local_created",
        "release",
        payload["release_id"],
        {
            "release_id": payload["release_id"],
            "repo_name": repo_name,
            "version": version,
            "line": line_name,
            "snapshot_id": snapshot_id,
            "profile": profile,
            "status": payload["status"],
        },
    )
    return payload


def list_local_releases(ctx: RepoContext) -> list[dict]:
    return [_local_release_view(row) or {} for row in local_control.list_workflow_releases(ctx)]


def get_local_release(ctx: RepoContext, release_id: str) -> dict:
    return _local_release_view(local_control.get_workflow_release(ctx, release_id)) or {}


def update_local_release(
    ctx: RepoContext,
    release_id: str,
    *,
    status: str | None = None,
    checks: list[dict] | object = _LOCAL_RELEASE_UNSET,
    artifacts: list[dict] | object = _LOCAL_RELEASE_UNSET,
    formula: dict | object = _LOCAL_RELEASE_UNSET,
    metadata: dict | object = _LOCAL_RELEASE_UNSET,
    event_type: str = "release.updated",
) -> dict:
    row = _local_release_view(
        local_control.update_workflow_release(
            ctx,
            release_id,
            status=status,
            checks=checks if checks is not _LOCAL_RELEASE_UNSET else local_control._UNSET,
            artifacts=artifacts if artifacts is not _LOCAL_RELEASE_UNSET else local_control._UNSET,
            formula=formula if formula is not _LOCAL_RELEASE_UNSET else local_control._UNSET,
            metadata=metadata if metadata is not _LOCAL_RELEASE_UNSET else local_control._UNSET,
        )
    )
    assert row is not None
    local_control.record_event(
        ctx,
        event_type,
        "release",
        release_id,
        {
            "release_id": release_id,
            "status": row["status"],
            "check_count": len(row.get("checks") or []),
            "artifact_count": len(row.get("artifacts") or []),
            "has_formula": bool(row.get("formula")),
        },
    )
    return row


def list_local_tasks(ctx: RepoContext) -> list[dict]:
    return local_control.list_workflow_tasks(ctx)


def get_local_task(ctx: RepoContext, task_id: str) -> dict:
    return local_control.get_workflow_task(ctx, task_id)


def close_local_task(ctx: RepoContext, task_id: str, status: str) -> dict:
    task = local_control.get_workflow_task(ctx, task_id)
    if task["publication_state"] == "published":
        raise ValueError(f"Local task {task_id} has already been published; close the remote task instead.")
    row = local_control.close_workflow_task(ctx, task_id, status)
    local_control.record_event(
        ctx,
        "task.closed",
        "task",
        task_id,
        {"task_id": task_id, "status": row["status"], "publication_state": row["publication_state"]},
    )
    return row


def mark_local_task_published(
    ctx: RepoContext,
    task_id: str,
    *,
    remote_name: str | None = None,
    published_task_id: str | None = None,
) -> dict:
    row = local_control.mark_workflow_task_published(
        ctx,
        task_id,
        remote_name=remote_name,
        published_task_id=published_task_id,
    )
    local_control.record_event(
        ctx,
        "task.published",
        "task",
        task_id,
        {
            "task_id": task_id,
            "publication_state": row["publication_state"],
            "published_remote_name": row["published_remote_name"],
            "published_task_id": row["published_task_id"],
            "published_at": row["published_at"],
        },
    )
    return row


def create_local_change(ctx: RepoContext, task_id: str, title: str, base_line: str, risk_tier: str) -> dict:
    repo_name = load_config(ctx).get("repo_name") or ctx.root.name
    lane = lane_from_risk(risk_tier)
    base_line_row = local_content.get_line(ctx, base_line)
    fork_snapshot_id = normalize_optional_text(base_line_row.get("head_snapshot_id"))
    identity = local_control.allocate_workflow_change_identity(
        ctx,
        repo_name,
        workflow_origin_namespace_prefix(LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX, effective_id_namespace_prefix(ctx)),
    )
    row = local_control.create_workflow_change(
        ctx,
        identity["change_id"],
        task_id,
        repo_name,
        title,
        base_line,
        risk_tier,
        lane,
        change_seq=identity["change_seq"],
        identity_source=local_control.LOCAL_IDENTITY_SOURCE_SEQUENCE,
        fork_snapshot_id=fork_snapshot_id,
        forked_from_line=base_line,
    )
    local_control.record_event(
        ctx,
        "change.local_created",
        "change",
        row["change_id"],
        {
            "change_id": row["change_id"],
            "task_id": task_id,
            "repo_name": repo_name,
            "title": title,
            "base_line": base_line,
            "fork_snapshot_id": fork_snapshot_id,
            "forked_from_line": base_line,
            "lane": lane,
            "publication_state": row["publication_state"],
        },
    )
    return _local_change_view(row)


def list_local_changes(ctx: RepoContext) -> list[dict]:
    rows = local_control.list_workflow_changes(ctx)
    return [_local_change_view(row) for row in rows]


def get_local_change(ctx: RepoContext, change_id: str) -> dict:
    return _local_change_view(local_control.get_workflow_change(ctx, change_id))


def close_local_change(ctx: RepoContext, change_id: str, status: str) -> dict:
    change = local_control.get_workflow_change(ctx, change_id)
    if change["publication_state"] == "published":
        raise ValueError(f"Local change {change_id} has already been published; close the remote change instead.")
    row = _local_change_view(local_control.close_workflow_change(ctx, change_id, status))
    local_control.record_event(
        ctx,
        "change.closed",
        "change",
        change_id,
        {"change_id": change_id, "status": row["status"], "publication_state": row["publication_state"]},
    )
    return row


def land_local_change(
    ctx: RepoContext,
    change_id: str,
    *,
    target_line: str,
    landed_snapshot_id: str,
    pre_land_target_snapshot_id: str | None = None,
) -> dict:
    change = local_control.get_workflow_change(ctx, change_id)
    if change["publication_state"] == "published":
        raise ValueError(
            f"Local change {change_id} has already been published; use `ait workflow land` for shared landing."
        )
    row = _local_change_view(
        local_control.land_workflow_change(
            ctx,
            change_id,
            target_line=target_line,
            landed_snapshot_id=landed_snapshot_id,
            pre_land_target_snapshot_id=pre_land_target_snapshot_id,
        )
    )
    local_control.record_event(
        ctx,
        "change.local_landed",
        "change",
        change_id,
        {
            "change_id": change_id,
            "task_id": row.get("task_id"),
            "status": row["status"],
            "publication_state": row["publication_state"],
            "target_line": row.get("target_line"),
            "pre_land_target_snapshot_id": row.get("pre_land_target_snapshot_id"),
            "landed_snapshot_id": row.get("landed_snapshot_id"),
        },
    )
    return row


def mark_local_change_published(
    ctx: RepoContext,
    change_id: str,
    *,
    remote_name: str | None = None,
    published_change_id: str | None = None,
    allow_landed: bool = False,
) -> dict:
    row = _local_change_view(
        local_control.mark_workflow_change_published(
            ctx,
            change_id,
            remote_name=remote_name,
            published_change_id=published_change_id,
            allow_landed=allow_landed,
        )
    )
    local_control.record_event(
        ctx,
        "change.published",
        "change",
        change_id,
        {
            "change_id": change_id,
            "publication_state": row["publication_state"],
            "published_remote_name": row["published_remote_name"],
            "published_change_id": row["published_change_id"],
            "published_at": row["published_at"],
        },
    )
    return row


def _local_actor_identity(ctx: RepoContext) -> str | None:
    config = load_config(ctx)
    return normalize_optional_text(config.get("user_email")) or normalize_optional_text(config.get("user_name"))


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


def list_local_session_events(ctx: RepoContext, session_id: str, *, after_sequence: int = 0, limit: int = 200) -> list[dict]:
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


def resume_local_session(ctx: RepoContext, session_id: str, *, after_sequence: int | None = None, limit: int = 200) -> dict:
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
