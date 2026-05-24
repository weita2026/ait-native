from __future__ import annotations

from typing import Any, Callable

from ait_protocol.common import AuthorMode

from .. import local_control
from ..remote_client import (
    RemoteError,
    create_change as remote_create_change,
    create_plan as remote_create_plan,
    create_task as remote_create_task,
    get_change as remote_get_change,
    get_plan as remote_get_plan,
    get_task as remote_get_task,
    list_plan_revisions as remote_list_plan_revisions,
    revise_plan as remote_revise_plan,
)
from ..store import (
    RepoContext,
    get_local_plan,
    get_local_plan_revision,
    list_local_plan_revisions,
    mark_local_plan_published,
    mark_local_task_published,
    workspace_status as local_workspace_status,
)
from .plan_sync_adoption import _plan_revision_artifact_body, _plan_revisions_ascending
from .plan_task_linkage import _normalize_plan_task_linkage, _published_local_task_plan_linkage
from .remote_repository_defaults import _remote_tuple
from .runtime_defaults import _normalize_text_value
from .workflow_identity_helpers import (
    _aligned_remote_publish_identity_request,
    _is_remote_publish_identity_conflict,
    _require_remote_workflow_identity_family,
)
from .workflow_land_snapshot_replay import (
    _workflow_land_batch_ensure_remote_patchset_for_landed_change,
    _workflow_land_batch_ensure_remote_target_line_base,
)
from .workflow_land_state import _workflow_land_payload, _workflow_land_step
from .workflow_land_task_dag import _workflow_land_batch_ensure_remote_task_dag_session
from .workflow_land_views import _workflow_land_completed_local_route_metadata
from .workflow_mode_config import _effective_id_namespace_prefix


def _published_local_task_plan_linkage_for_remote(
    ctx: RepoContext,
    task: dict[str, Any],
    *,
    remote_name: str,
) -> tuple[str | None, str | None, str | None]:
    try:
        published_plan_id, published_revision_id, published_plan_item_ref = _published_local_task_plan_linkage(ctx, task)
        if published_plan_id is None:
            return None, None, None
        remote_row, repo_name = _remote_tuple(ctx, remote_name)
        remote_get_plan(remote_row["url"], published_plan_id)
        return published_plan_id, published_revision_id, published_plan_item_ref
    except (KeyError, RemoteError, ValueError):
        pass

    resolved_plan_id, resolved_revision_id, resolved_plan_item_ref = _normalize_plan_task_linkage(
        ctx,
        plan_id=_normalize_text_value(task.get("plan_id")),
        plan_revision_id=_normalize_text_value(task.get("origin_plan_revision_id")),
        plan_item_ref=_normalize_text_value(task.get("plan_item_ref")),
    )
    if resolved_plan_id is None and resolved_revision_id is None:
        return None, None, None
    if resolved_plan_id is None:
        assert resolved_revision_id is not None
        revision_row = local_control.get_workflow_plan_revision_by_id(ctx, resolved_revision_id)
        resolved_plan_id = _normalize_text_value(revision_row.get("plan_id"))
    if resolved_plan_id is None:
        raise ValueError(f"Local task {task['task_id']} is missing `plan_id` metadata.")

    local_plan = get_local_plan(ctx, resolved_plan_id)
    if resolved_revision_id is None:
        resolved_revision_id = _normalize_text_value(local_plan.get("head_revision_id"))
    if resolved_revision_id is None:
        raise ValueError(
            f"Local task {task['task_id']} is linked to local plan {resolved_plan_id} without a stored revision id."
        )

    local_revisions = _plan_revisions_ascending(list_local_plan_revisions(ctx, resolved_plan_id))
    target_index = next(
        (
            index
            for index, row in enumerate(local_revisions)
            if str(row.get("plan_revision_id") or "").strip() == resolved_revision_id
        ),
        None,
    )
    if target_index is None:
        raise ValueError(
            f"Local task {task['task_id']} references unknown local plan revision {resolved_revision_id} on plan {resolved_plan_id}."
        )
    target_revisions = local_revisions[: target_index + 1]

    remote_row, repo_name = _remote_tuple(ctx, remote_name)
    try:
        remote_plan = remote_get_plan(remote_row["url"], resolved_plan_id)
        remote_revisions = _plan_revisions_ascending(remote_list_plan_revisions(remote_row["url"], resolved_plan_id))
        remote_revision_ids = {
            _normalize_text_value(row.get("plan_revision_id"))
            for row in remote_revisions
            if _normalize_text_value(row.get("plan_revision_id")) is not None
        }
    except (KeyError, RemoteError, ValueError):
        remote_plan = None
        remote_revision_ids = set()

    revision_mappings: list[tuple[str, str]] = []
    revisions_to_publish: list[dict[str, Any]] = []
    publish_gap_open = remote_plan is None
    for revision in target_revisions:
        local_revision_id = str(revision["plan_revision_id"])
        published_revision_id = _normalize_text_value(revision.get("published_plan_revision_id"))
        if not publish_gap_open and published_revision_id is not None and published_revision_id in remote_revision_ids:
            revision_mappings.append((local_revision_id, published_revision_id))
            continue
        publish_gap_open = True
        revisions_to_publish.append(revision)

    if remote_plan is None:
        if not revisions_to_publish:
            revisions_to_publish = target_revisions[:1]
        seed_revision = revisions_to_publish.pop(0)
        remote_plan = remote_create_plan(
            remote_row["url"],
            repo_name,
            seed_revision["title_snapshot"],
            seed_revision.get("artifact_path"),
            seed_revision.get("artifact_selector"),
            seed_revision.get("artifact_heading"),
            seed_revision.get("items") or [],
            summary=seed_revision.get("summary"),
            status=local_plan["status"],
            plan_id=resolved_plan_id,
            source_kind=seed_revision.get("source_kind") or "manual_edit",
            source_session_id=seed_revision.get("source_session_id"),
            artifact_body=_plan_revision_artifact_body(ctx, seed_revision),
        )
        remote_head_revision_id = _normalize_text_value(
            (remote_plan.get("head_revision") or {}).get("plan_revision_id") or remote_plan.get("head_revision_id")
        )
        if remote_head_revision_id is None:
            raise ValueError(f"Remote plan {resolved_plan_id} create response did not include a head revision id.")
        revision_mappings.append((str(seed_revision["plan_revision_id"]), remote_head_revision_id))

    for revision in revisions_to_publish:
        expected_head_revision_id = _normalize_text_value(
            (remote_plan.get("head_revision") or {}).get("plan_revision_id") or remote_plan.get("head_revision_id")
        )
        remote_plan = remote_revise_plan(
            remote_row["url"],
            resolved_plan_id,
            revision.get("artifact_path"),
            revision.get("artifact_selector"),
            revision.get("artifact_heading"),
            revision.get("items") or [],
            title=revision.get("title_snapshot"),
            summary=revision.get("summary"),
            source_kind=revision.get("source_kind") or "manual_edit",
            source_session_id=revision.get("source_session_id"),
            artifact_body=_plan_revision_artifact_body(ctx, revision),
            expected_head_revision_id=expected_head_revision_id,
        )
        remote_revision_id = _normalize_text_value(
            (remote_plan.get("head_revision") or {}).get("plan_revision_id") or remote_plan.get("head_revision_id")
        )
        if remote_revision_id is None:
            raise ValueError(
                f"Remote plan {resolved_plan_id} revise response did not include a head revision id for local revision {revision['plan_revision_id']}."
            )
        revision_mappings.append((str(revision["plan_revision_id"]), remote_revision_id))

    remote_head_revision_id = _normalize_text_value(
        (remote_plan.get("head_revision") or {}).get("plan_revision_id") or remote_plan.get("head_revision_id")
    )
    mark_local_plan_published(
        ctx,
        resolved_plan_id,
        remote_name=remote_name or _normalize_text_value(remote_row.get("name")),
        published_plan_id=resolved_plan_id,
        published_head_revision_id=remote_head_revision_id,
        revision_mappings=revision_mappings,
    )
    published_target_revision = get_local_plan_revision(ctx, resolved_plan_id, resolved_revision_id)
    published_revision_id = _normalize_text_value(published_target_revision.get("published_plan_revision_id"))
    if published_revision_id is None:
        raise ValueError(
            f"Failed to publish local task {task['task_id']} origin revision {resolved_revision_id} to `{remote_name}`."
        )
    return resolved_plan_id, published_revision_id, resolved_plan_item_ref


def _workflow_land_batch_ensure_remote_task(
    ctx: RepoContext,
    *,
    remote_name: str,
    remote_row: dict[str, Any],
    repo_name: str,
    local_task: dict[str, Any],
) -> dict[str, Any]:
    task_id = str(local_task["task_id"])
    if str(local_task.get("publication_state") or "") == "published":
        published_remote_name = _normalize_text_value(local_task.get("published_remote_name"))
        if published_remote_name not in {None, remote_name}:
            raise ValueError(
                f"Local task {task_id} is already published to `{published_remote_name}`, not `{remote_name}`."
            )
        published_task_id = _normalize_text_value(local_task.get("published_task_id")) or task_id
        return remote_get_task(remote_row["url"], published_task_id, repo_name=repo_name)

    published_plan_id, published_revision_id, published_plan_item_ref = _published_local_task_plan_linkage_for_remote(
        ctx,
        local_task,
        remote_name=remote_name,
    )
    namespace_prefix = _effective_id_namespace_prefix(ctx)["value"]
    requested_task_id = _aligned_remote_publish_identity_request(
        remote_row["url"],
        repo_name,
        local_task,
        entity_type="task",
        namespace_prefix=namespace_prefix,
    )
    try:
        remote_task = remote_create_task(
            remote_row["url"],
            repo_name,
            str(local_task.get("title") or task_id),
            str(local_task.get("intent") or ""),
            str(local_task.get("risk_tier") or "medium"),
            task_id=requested_task_id,
            plan_id=published_plan_id,
            origin_plan_revision_id=published_revision_id,
            plan_item_ref=published_plan_item_ref,
        )
    except (RemoteError, ValueError) as exc:
        if not _is_remote_publish_identity_conflict(exc, entity_type="task", requested_id=requested_task_id):
            raise
        remote_task = remote_create_task(
            remote_row["url"],
            repo_name,
            str(local_task.get("title") or task_id),
            str(local_task.get("intent") or ""),
            str(local_task.get("risk_tier") or "medium"),
            task_id=None,
            plan_id=published_plan_id,
            origin_plan_revision_id=published_revision_id,
            plan_item_ref=published_plan_item_ref,
        )
        requested_task_id = None
    published_task_id = _require_remote_workflow_identity_family(
        "task",
        remote_task,
        namespace_prefix=namespace_prefix,
        requested_id=requested_task_id,
    )
    published_task_id = _normalize_text_value(remote_task.get("published_task_id")) or published_task_id or task_id
    mark_local_task_published(
        ctx,
        task_id,
        remote_name=_normalize_text_value(remote_row.get("name")) or remote_name,
        published_task_id=published_task_id,
    )
    return remote_task


def _workflow_land_batch_ensure_remote_change(
    ctx: RepoContext,
    *,
    remote_name: str,
    remote_row: dict[str, Any],
    repo_name: str,
    remote_task_id: str,
    local_change: dict[str, Any],
    target_line: str,
    parent_snapshot_id: str,
) -> dict[str, Any]:
    change_id = str(local_change["change_id"])
    if str(local_change.get("publication_state") or "") == "published":
        published_remote_name = _normalize_text_value(local_change.get("published_remote_name"))
        if published_remote_name not in {None, remote_name}:
            raise ValueError(
                f"Local change {change_id} is already published to `{published_remote_name}`, not `{remote_name}`."
            )
        published_change_id = _normalize_text_value(local_change.get("published_change_id")) or change_id
        return remote_get_change(remote_row["url"], published_change_id, repo_name=repo_name)

    namespace_prefix = _effective_id_namespace_prefix(ctx)["value"]
    requested_change_id = _aligned_remote_publish_identity_request(
        remote_row["url"],
        repo_name,
        local_change,
        entity_type="change",
        namespace_prefix=namespace_prefix,
    )
    try:
        remote_change = remote_create_change(
            remote_row["url"],
            repo_name,
            remote_task_id,
            str(local_change.get("title") or change_id),
            target_line,
            str(local_change.get("risk_tier") or "medium"),
            change_id=requested_change_id,
            fork_snapshot_id=parent_snapshot_id,
            forked_from_line=target_line,
        )
    except (RemoteError, ValueError) as exc:
        if not _is_remote_publish_identity_conflict(exc, entity_type="change", requested_id=requested_change_id):
            raise
        remote_change = remote_create_change(
            remote_row["url"],
            repo_name,
            remote_task_id,
            str(local_change.get("title") or change_id),
            target_line,
            str(local_change.get("risk_tier") or "medium"),
            change_id=None,
            fork_snapshot_id=parent_snapshot_id,
            forked_from_line=target_line,
        )
        requested_change_id = None
    published_change_id = _require_remote_workflow_identity_family(
        "change",
        remote_change,
        namespace_prefix=namespace_prefix,
        requested_id=requested_change_id,
    )
    published_change_id = _normalize_text_value(remote_change.get("published_change_id")) or published_change_id or change_id
    local_control.mark_workflow_change_published(
        ctx,
        change_id,
        remote_name=_normalize_text_value(remote_row.get("name")) or remote_name,
        published_change_id=published_change_id,
        allow_landed=True,
    )
    return remote_change


def _workflow_land_completed_local_preview_state(
    ctx: RepoContext,
    *,
    entry: dict[str, Any],
    remote_name: str,
) -> dict[str, Any]:
    local_task = entry["task"]
    local_change = entry["change"]
    target_line = str(entry.get("target_line") or "main")
    change_id = str(local_change.get("change_id") or "")
    published_change_id = _normalize_text_value(local_change.get("published_change_id"))
    published_task_id = _normalize_text_value(local_task.get("published_task_id"))

    remote_task_id: str | None = None
    remote_change_id: str | None = None
    if str(local_change.get("publication_state") or "") == "published" and published_change_id is not None:
        state = _workflow_land_payload(
            ctx,
            change_id=published_change_id,
            patchset_id=None,
            remote_name=remote_name,
            ignore_workspace_authoring=True,
            patchset_is_authoritative=True,
        )
        remote_task_id = _normalize_text_value((state.get("task") or {}).get("task_id")) or published_task_id
        remote_change_id = _normalize_text_value((state.get("change") or {}).get("change_id")) or published_change_id
    else:
        apply_command = f"ait workflow land {change_id} --remote {remote_name} --apply"
        state = {
            "change": {
                "change_id": change_id,
                "status": str(local_change.get("status") or "landed"),
                "base_line": str(local_change.get("base_line") or target_line or "main"),
            },
            "task": {
                "task_id": str(local_task.get("task_id") or ""),
                "status": str(local_task.get("status") or "completed"),
            },
            "patchset": {},
            "workspace": local_workspace_status(ctx),
            "steps": [
                _workflow_land_step(
                    "local_landed",
                    "local change landed",
                    "done",
                    f"Local change `{change_id}` already landed at `{entry.get('landed_snapshot_id') or 'unknown'}`.",
                ),
                _workflow_land_step(
                    "remote_promote",
                    "remote later-promotion",
                    "pending",
                    f"Promote the landed local slice on `{remote_name}` against `{target_line}`.",
                    apply_command,
                ),
            ],
            "next_action": {
                "code": "remote_promote",
                "summary": "Promote the landed local slice through the native remote land path.",
                "detail": (
                    f"`{change_id}` uses the local workflow family, so `ait workflow land` "
                    "will later-promote it instead of resolving a remote sequence fallback."
                ),
                "command": apply_command,
            },
        }
    state["routing"] = _workflow_land_completed_local_route_metadata(
        local_task=local_task,
        local_change=local_change,
        remote_name=remote_name,
        target_line=target_line,
        remote_task_id=remote_task_id,
        remote_change_id=remote_change_id,
    )
    state["later_promotion"] = {
        "mode": "completed_local_single_change",
        "remote_name": remote_name,
        "target_line": target_line,
        "landed_snapshot_id": _normalize_text_value(entry.get("landed_snapshot_id")),
    }
    return state


def _workflow_land_apply_completed_local_entry(
    ctx: RepoContext,
    *,
    entry: dict[str, Any],
    remote_name: str,
    remote_row: dict[str, Any],
    repo_name: str,
    summary: str | None,
    tests: str | None,
    lint: str | None,
    security: str | None,
    license: str | None,
    author_mode: AuthorMode | None,
    model: str | None,
    session: str | None,
    checkpoint: str | None,
    reviewer: str | None,
    review_message: str | None,
    target: str | None,
    mode: str,
    apply_fn: Callable[..., dict[str, Any]],
) -> tuple[dict[str, Any], dict[str, str]]:
    local_task = entry["task"]
    local_change = entry["change"]
    target_line = str(entry.get("target_line") or target or "main")
    remote_base_snapshot_id = _workflow_land_batch_ensure_remote_target_line_base(
        ctx,
        remote_name=remote_name,
        remote_row=remote_row,
        repo_name=repo_name,
        target_line=target_line,
        initial_parent_snapshot_id=str(entry["parent_snapshot_id"]),
    )
    remote_task = _workflow_land_batch_ensure_remote_task(
        ctx,
        remote_name=remote_name,
        remote_row=remote_row,
        repo_name=repo_name,
        local_task=local_task,
    )
    remote_change = _workflow_land_batch_ensure_remote_change(
        ctx,
        remote_name=remote_name,
        remote_row=remote_row,
        repo_name=repo_name,
        remote_task_id=str(remote_task["task_id"]),
        local_change=local_change,
        target_line=target_line,
        parent_snapshot_id=remote_base_snapshot_id,
    )
    remote_task_dag_session = _workflow_land_batch_ensure_remote_task_dag_session(
        ctx,
        entry=entry,
        remote_row=remote_row,
        repo_name=repo_name,
        remote_task_id=str(remote_task["task_id"]),
        remote_change_id=str(remote_change["change_id"]),
    )
    if str(remote_change.get("status") or "") != "landed":
        patchset = _workflow_land_batch_ensure_remote_patchset_for_landed_change(
            ctx,
            remote_name=remote_name,
            remote_row=remote_row,
            repo_name=repo_name,
            remote_change=remote_change,
            local_change=local_change,
            target_line=target_line,
            remote_base_snapshot_id=remote_base_snapshot_id,
            summary=summary,
            author_mode=author_mode,
        )
        patchset_id = str(patchset["patchset_id"])
    else:
        patchset_id = _normalize_text_value(remote_change.get("current_patchset_id"))
    state = apply_fn(
        ctx,
        change_id=str(remote_change["change_id"]),
        patchset_id=patchset_id,
        remote_name=remote_name,
        snapshot_message=None,
        patchset_summary=summary,
        tests=tests,
        lint=lint,
        security=security,
        license=license,
        author_mode=author_mode,
        model=model,
        session=session or _normalize_text_value((remote_task_dag_session or {}).get("session_id")),
        checkpoint=checkpoint,
        reviewer=reviewer,
        review_message=review_message,
        target=target_line,
        mode=mode,
        ignore_workspace_authoring=True,
        patchset_is_authoritative=True,
    )
    state["routing"] = _workflow_land_completed_local_route_metadata(
        local_task=local_task,
        local_change=local_change,
        remote_name=remote_name,
        target_line=target_line,
        remote_task_id=str(remote_task.get("task_id") or ""),
        remote_change_id=str(remote_change.get("change_id") or ""),
    )
    state["later_promotion"] = {
        "mode": "completed_local_single_change",
        "remote_name": remote_name,
        "target_line": target_line,
        "landed_snapshot_id": _normalize_text_value(entry.get("landed_snapshot_id")),
    }
    return state, {
        "task_id": str(local_task["task_id"]),
        "change_id": str(local_change["change_id"]),
    }
