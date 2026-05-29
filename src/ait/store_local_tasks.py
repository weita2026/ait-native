from __future__ import annotations

from ait_protocol.common import (
    LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX,
    utc_now,
    workflow_origin_namespace_prefix,
)

from . import local_control
from .repo_paths import RepoContext
from .store_local_workflow_runtime import resolve_local_task_plan_linkage
from .store_repo_config import effective_id_namespace_prefix, load_config


def _ensure_local_control_runtime_schema(ctx: RepoContext) -> None:
    conn = local_control._connect_control(ctx)
    try:
        local_control._ensure_schema(conn)
        conn.commit()
    finally:
        conn.close()


def _guard_reused_plan_item_ref(
    ctx: RepoContext,
    *,
    repo_name: str,
    plan_id: str | None,
    plan_item_ref: str | None,
) -> None:
    if plan_id is None or plan_item_ref is None:
        return
    linked_task = local_control.find_latest_workflow_task_for_plan_item(
        ctx,
        repo_name=repo_name,
        plan_id=plan_id,
        plan_item_ref=plan_item_ref,
    )
    if linked_task is None:
        return
    linked_task_id = str(linked_task.get("task_id") or "").strip() or "unknown"
    linked_status = str(linked_task.get("status") or "unknown").strip() or "unknown"
    linked_revision_id = str(linked_task.get("origin_plan_revision_id") or "").strip()
    revision_note = f" from revision {linked_revision_id}" if linked_revision_id else ""
    raise ValueError(
        f"Plan item ref {plan_item_ref!r} on plan {plan_id}{revision_note} is already linked to task "
        f"{linked_task_id} (status: {linked_status}). Use `ait plan inspect <plan-id>` or "
        "`ait plan candidates` to confirm the next taskable ref, or open a new plan item ref / revision "
        "instead of binding a new task to an older dispatched ref."
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
    _ensure_local_control_runtime_schema(ctx)
    _guard_reused_plan_item_ref(
        ctx,
        repo_name=repo_name,
        plan_id=resolved_plan_id,
        plan_item_ref=resolved_plan_item_ref,
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
