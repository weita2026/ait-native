from __future__ import annotations

from ait_protocol.common import (
    generate_namespaced_workflow_id,
)

from . import local_control
from .repo_paths import RepoContext
from .store_local_changes import (
    close_local_change,
    create_local_change,
    get_local_change,
    land_local_change,
    list_local_changes,
    mark_local_change_published,
)
from .store_local_tasks import (
    close_local_task,
    create_local_task,
    get_local_task,
    list_local_tasks,
    mark_local_task_published,
)
from .store_repo_config import (
    effective_id_namespace_prefix,
    load_config,
)
from .store_local_sessions import (
    _local_actor_identity,
    append_local_session_event,
    close_local_session,
    create_local_checkpoint,
    create_local_session,
    get_local_checkpoint,
    get_local_session,
    list_local_checkpoints,
    list_local_session_events,
    list_local_sessions,
    resume_local_session,
)
from .store_local_releases import (
    create_local_release,
    get_local_release,
    list_local_releases,
    update_local_release,
)
from .store_local_workflow_runtime import resolve_local_task_plan_linkage
from .store_local_views import (
    _local_plan_revision_view,
    _local_plan_view,
)


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
