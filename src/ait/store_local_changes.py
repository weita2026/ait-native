from __future__ import annotations

from ait_protocol.common import (
    LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX,
    lane_from_risk,
    normalize_optional_text,
    workflow_origin_namespace_prefix,
)

from . import local_content, local_control
from .repo_paths import RepoContext
from .store_local_views import _local_change_view
from .store_repo_config import effective_id_namespace_prefix, load_config


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
