from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Callable

from ait_protocol.common import generate_namespaced_workflow_id

from .. import local_control
from ..local_content_pack_runtime import _read_blob_bytes, ensure_blob_bytes
from ..repo_paths import RepoContext
from ..store import (
    close_local_plan,
    effective_id_namespace_prefix as repo_id_namespace_prefix,
    get_local_plan,
    list_local_plan_revisions,
    mark_local_plan_published,
)
from .plan_sync_matching import (
    _local_plan_fully_published,
    _plan_artifact_identity_key,
    _plan_heads_equivalent,
    _select_sync_existing_plan_with_continuity,
)
from .workflow_mode_config import _normalize_text_value


def _remote_plan_summary_to_plan(row: dict[str, Any]) -> dict[str, Any]:
    plan = dict(row)
    items_json = row.get("head_revision_items_json")
    items: list[dict[str, Any]] = []
    if isinstance(items_json, str) and items_json.strip():
        try:
            parsed_items = json.loads(items_json)
        except json.JSONDecodeError:
            parsed_items = []
        if isinstance(parsed_items, list):
            items = [item for item in parsed_items if isinstance(item, dict)]
    plan["head_revision"] = {
        "plan_revision_id": _normalize_text_value(row.get("head_revision_id")),
        "revision_number": row.get("head_revision_number"),
        "artifact_path": _normalize_text_value(row.get("head_artifact_path")),
        "artifact_selector": _normalize_text_value(row.get("head_artifact_selector")),
        "artifact_heading": _normalize_text_value(row.get("head_artifact_heading")),
        "artifact_blob_id": _normalize_text_value(row.get("head_artifact_blob_id")),
        "items": items,
        "summary": _normalize_text_value(row.get("head_revision_summary")),
        "created_at": _normalize_text_value(row.get("head_revision_created_at")),
    }
    return plan


def _plan_revisions_ascending(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return sorted(rows, key=lambda row: int(row.get("revision_number") or 0))


def _plan_revision_artifact_body(ctx: RepoContext, revision: dict[str, Any]) -> str | None:
    artifact_path = _normalize_text_value(revision.get("artifact_path"))
    if artifact_path is None:
        return None
    path = (ctx.root / artifact_path).resolve(strict=False)
    try:
        return path.read_text(encoding="utf-8")
    except OSError:
        return None


def _plan_publish_revision_metadata_matches(local_revision: dict[str, Any], remote_revision: dict[str, Any]) -> bool:
    return (
        _normalize_text_value(local_revision.get("artifact_path")) == _normalize_text_value(remote_revision.get("artifact_path"))
        and _normalize_text_value(local_revision.get("artifact_selector")) == _normalize_text_value(remote_revision.get("artifact_selector"))
        and _normalize_text_value(local_revision.get("artifact_heading")) == _normalize_text_value(remote_revision.get("artifact_heading"))
        and _normalize_text_value(local_revision.get("title_snapshot")) == _normalize_text_value(remote_revision.get("title_snapshot"))
        and _normalize_text_value(local_revision.get("artifact_blob_id")) == _normalize_text_value(remote_revision.get("artifact_blob_id"))
        and list(local_revision.get("items") or []) == list(remote_revision.get("items") or [])
    )


def _revision_with_materialized_artifact_body(
    ctx: RepoContext,
    revision: dict[str, Any],
) -> tuple[dict[str, Any], str | None]:
    payload = dict(revision)
    artifact_blob_id = _normalize_text_value(payload.get("artifact_blob_id"))
    artifact_body = payload.get("artifact_body")
    if isinstance(artifact_body, str):
        local_blob_id = ensure_blob_bytes(
            ctx,
            artifact_body.encode("utf-8"),
            path_hint=_normalize_text_value(payload.get("artifact_path")),
        )
        if artifact_blob_id is not None and local_blob_id != artifact_blob_id:
            raise ValueError(
                "Remote plan revision artifact body does not match its declared artifact_blob_id "
                f"for plan revision {payload.get('plan_revision_id')!r}."
            )
        payload["artifact_blob_id"] = local_blob_id
        return payload, local_blob_id
    return payload, artifact_blob_id


def _repair_local_plan_revision_blobs_from_remote(
    ctx: RepoContext,
    plan_id: str,
    remote_revisions: list[dict[str, Any]],
    *,
    load_remote_revision: Callable[[str], dict[str, Any]] | None,
) -> None:
    local_by_published_remote_revision_id = {
        _normalize_text_value(row.get("published_plan_revision_id")): row
        for row in list_local_plan_revisions(ctx, plan_id)
        if _normalize_text_value(row.get("published_plan_revision_id")) is not None
    }
    for remote_revision in remote_revisions:
        remote_revision_id = _normalize_text_value(remote_revision.get("plan_revision_id"))
        if remote_revision_id is None:
            continue
        local_revision = local_by_published_remote_revision_id.get(remote_revision_id)
        if local_revision is None:
            continue
        local_blob_id = _normalize_text_value(local_revision.get("artifact_blob_id"))
        remote_blob_id = _normalize_text_value(remote_revision.get("artifact_blob_id"))
        if local_blob_id is None or remote_blob_id is None or local_blob_id != remote_blob_id:
            continue
        try:
            _read_blob_bytes(ctx, local_blob_id)
            continue
        except KeyError:
            pass
        if load_remote_revision is None:
            continue
        revision_detail = load_remote_revision(remote_revision_id)
        _, materialized_blob_id = _revision_with_materialized_artifact_body(ctx, revision_detail)
        if materialized_blob_id != local_blob_id:
            raise ValueError(
                f"Remote plan revision {remote_revision_id} materialized blob {materialized_blob_id!r}, "
                f"but local revision {local_revision.get('plan_revision_id')!r} expects {local_blob_id!r}."
            )


def _local_published_plan_blobs_intact(ctx: RepoContext, plan_id: str) -> bool:
    published_revisions = [
        row
        for row in list_local_plan_revisions(ctx, plan_id)
        if _normalize_text_value(row.get("published_plan_revision_id")) is not None
    ]
    if not published_revisions:
        return False
    for revision in published_revisions:
        blob_id = _normalize_text_value(revision.get("artifact_blob_id"))
        if blob_id is None:
            return False
        try:
            _read_blob_bytes(ctx, blob_id)
        except KeyError:
            return False
    return True


def _adopt_remote_plan_for_local_sync(
    ctx: RepoContext,
    remote_plan: dict[str, Any],
    remote_revisions: list[dict[str, Any]],
    *,
    remote_name: str | None,
    repo_name: str,
    load_remote_revision: Callable[[str], dict[str, Any]] | None = None,
) -> dict[str, Any]:
    plan_id = str(remote_plan["plan_id"])
    try:
        return get_local_plan(ctx, plan_id)
    except KeyError:
        pass

    revisions = _plan_revisions_ascending(remote_revisions)
    if not revisions:
        raise ValueError(f"Remote plan {plan_id} has no revisions to adopt locally.")

    revision_mappings: list[tuple[str, str]] = []
    first_revision = revisions[0]
    if load_remote_revision is not None and not isinstance(first_revision.get("artifact_body"), str):
        first_revision = load_remote_revision(str(first_revision.get("plan_revision_id") or ""))
    first_revision, first_blob_id = _revision_with_materialized_artifact_body(ctx, first_revision)
    first_local_revision_id = generate_namespaced_workflow_id("PR", repo_id_namespace_prefix(ctx))
    local_control.create_workflow_plan(
        ctx,
        plan_id,
        first_local_revision_id,
        repo_name,
        _normalize_text_value(first_revision.get("title_snapshot")) or str(remote_plan.get("title") or plan_id),
        _normalize_text_value(first_revision.get("artifact_path")) or "",
        _normalize_text_value(first_revision.get("artifact_selector")),
        _normalize_text_value(first_revision.get("artifact_heading"))
        or _normalize_text_value(first_revision.get("title_snapshot"))
        or str(remote_plan.get("title") or plan_id),
        list(first_revision.get("items") or []),
        artifact_blob_id=first_blob_id,
        summary=_normalize_text_value(first_revision.get("summary")),
        status=str(remote_plan.get("status") or "draft"),
        source_kind=_normalize_text_value(first_revision.get("source_kind")) or "remote_adoption",
        source_session_id=_normalize_text_value(first_revision.get("source_session_id")),
        created_by=_normalize_text_value(first_revision.get("created_by")),
        actor_type=_normalize_text_value(first_revision.get("actor_type")) or "agent",
        publication_state="local_draft",
    )
    remote_revision_id = _normalize_text_value(first_revision.get("plan_revision_id"))
    if remote_revision_id is not None:
        revision_mappings.append((first_local_revision_id, remote_revision_id))

    for revision in revisions[1:]:
        if load_remote_revision is not None and not isinstance(revision.get("artifact_body"), str):
            revision = load_remote_revision(str(revision.get("plan_revision_id") or ""))
        revision, local_blob_id = _revision_with_materialized_artifact_body(ctx, revision)
        local_revision_id = generate_namespaced_workflow_id("PR", repo_id_namespace_prefix(ctx))
        local_control.revise_workflow_plan(
            ctx,
            plan_id,
            local_revision_id,
            _normalize_text_value(revision.get("artifact_path")) or "",
            _normalize_text_value(revision.get("artifact_selector")),
            _normalize_text_value(revision.get("artifact_heading"))
            or _normalize_text_value(revision.get("title_snapshot"))
            or str(remote_plan.get("title") or plan_id),
            list(revision.get("items") or []),
            artifact_blob_id=local_blob_id,
            title=_normalize_text_value(revision.get("title_snapshot")) or str(remote_plan.get("title") or plan_id),
            summary=_normalize_text_value(revision.get("summary")),
            source_kind=_normalize_text_value(revision.get("source_kind")) or "remote_adoption",
            source_session_id=_normalize_text_value(revision.get("source_session_id")),
            created_by=_normalize_text_value(revision.get("created_by")),
            actor_type=_normalize_text_value(revision.get("actor_type")) or "agent",
        )
        remote_revision_id = _normalize_text_value(revision.get("plan_revision_id"))
        if remote_revision_id is not None:
            revision_mappings.append((local_revision_id, remote_revision_id))

    remote_head_revision_id = (remote_plan.get("head_revision") or {}).get("plan_revision_id") or remote_plan.get("head_revision_id")
    return mark_local_plan_published(
        ctx,
        plan_id,
        remote_name=remote_name,
        published_plan_id=plan_id,
        published_head_revision_id=_normalize_text_value(remote_head_revision_id),
        revision_mappings=revision_mappings,
    )


def _resolve_local_sync_plan_candidate(
    ctx: RepoContext,
    artifact: dict[str, Any],
    *,
    local_plans: list[dict[str, Any]],
    local_indexed_plans: dict[tuple[str, str | None], list[dict[str, Any]]],
    remote_plans: list[dict[str, Any]],
    remote_indexed_plans: dict[tuple[str, str | None], list[dict[str, Any]]],
    remote_name: str | None,
    repo_name: str | None,
    load_full_remote_plan_candidates: Callable[
        [],
        tuple[list[dict[str, Any]], dict[tuple[str, str | None], list[dict[str, Any]]]],
    ]
    | None = None,
    load_remote_plan: Callable[[str], dict[str, Any]] | None = None,
    load_remote_revisions: Callable[[str], list[dict[str, Any]]] | None = None,
    load_remote_revision: Callable[[str, str], dict[str, Any]] | None = None,
) -> tuple[dict[str, Any] | None, dict[str, Any] | None, dict[str, Any] | None]:
    artifact_path = str(artifact["artifact_path"])
    artifact_selector = _normalize_text_value(artifact.get("artifact_selector"))
    artifact_key = _plan_artifact_identity_key(artifact_path, artifact_selector)
    local_plan, continuity_match = _select_sync_existing_plan_with_continuity(
        ctx,
        artifact,
        indexed_plans_by_identity=local_indexed_plans,
        plans=local_plans,
    )
    adoption: dict[str, Any] | None = None
    remote_plan: dict[str, Any] | None = None
    remote_continuity_match: dict[str, Any] | None = None
    if repo_name is not None and (local_plan is None or not _local_plan_fully_published(local_plan)):
        remote_plan, remote_continuity_match = _select_sync_existing_plan_with_continuity(
            ctx,
            artifact,
            indexed_plans_by_identity=remote_indexed_plans,
            plans=remote_plans,
        )
        if remote_plan is None and load_full_remote_plan_candidates is not None:
            full_remote_plans, full_remote_index = load_full_remote_plan_candidates()
            remote_plan, remote_continuity_match = _select_sync_existing_plan_with_continuity(
                ctx,
                artifact,
                indexed_plans_by_identity=full_remote_index,
                plans=full_remote_plans,
            )
    continuity_match = continuity_match or remote_continuity_match
    if remote_plan is None:
        return local_plan, adoption, continuity_match
    remote_plan_id = str(remote_plan["plan_id"])
    if local_plan is None:
        if repo_name is None:
            raise ValueError("Remote repository context is required to adopt a remote plan.")
        if load_remote_revisions is None:
            raise ValueError("Remote plan revision loader is required to adopt a remote plan.")
        local_plan = _adopt_remote_plan_for_local_sync(
            ctx,
            remote_plan,
            load_remote_revisions(remote_plan_id),
            remote_name=remote_name,
            repo_name=repo_name,
            load_remote_revision=(
                None
                if load_remote_revision is None
                else lambda plan_revision_id: load_remote_revision(remote_plan_id, plan_revision_id)
            ),
        )
        local_indexed_plans.setdefault(artifact_key, []).append(local_plan)
        adoption = {
            "plan_id": local_plan.get("plan_id"),
            "artifact_path": artifact_path,
            "artifact_selector": artifact_selector,
            "remote_head_revision_id": remote_plan.get("head_revision_id")
            or (remote_plan.get("head_revision") or {}).get("plan_revision_id"),
            "local_head_revision_id": local_plan.get("head_revision_id")
            or (local_plan.get("head_revision") or {}).get("plan_revision_id"),
        }
        return local_plan, adoption, continuity_match
    if str(local_plan.get("plan_id") or "") != str(remote_plan.get("plan_id") or ""):
        resolved_remote_plan = load_remote_plan(remote_plan_id) if load_remote_plan is not None else remote_plan
        if local_plan.get("publication_state") != "published" and _plan_heads_equivalent(local_plan, resolved_remote_plan):
            archived_local = close_local_plan(ctx, str(local_plan["plan_id"]), "archived")
            local_indexed_plans[artifact_key] = [
                plan
                for plan in local_indexed_plans.get(artifact_key, [])
                if str(plan.get("plan_id") or "") != str(archived_local.get("plan_id") or "")
            ]
            if repo_name is None:
                raise ValueError("Remote repository context is required to adopt a remote plan.")
            if load_remote_revisions is None:
                raise ValueError("Remote plan revision loader is required to adopt a remote plan.")
            adopted_plan = _adopt_remote_plan_for_local_sync(
                ctx,
                resolved_remote_plan,
                load_remote_revisions(remote_plan_id),
                remote_name=remote_name,
                repo_name=repo_name,
                load_remote_revision=(
                    None
                    if load_remote_revision is None
                    else lambda plan_revision_id: load_remote_revision(remote_plan_id, plan_revision_id)
                ),
            )
            local_indexed_plans.setdefault(artifact_key, []).append(adopted_plan)
            return adopted_plan, {
                "plan_id": adopted_plan.get("plan_id"),
                "artifact_path": artifact_path,
                "artifact_selector": artifact_selector,
                "replaced_local_plan_id": archived_local.get("plan_id"),
                "remote_head_revision_id": remote_plan.get("head_revision_id")
                or (remote_plan.get("head_revision") or {}).get("plan_revision_id"),
                "local_head_revision_id": adopted_plan.get("head_revision_id")
                or (adopted_plan.get("head_revision") or {}).get("plan_revision_id"),
            }, continuity_match
        raise ValueError(
            f"Remote plan {remote_plan.get('plan_id')} already tracks {artifact_path}"
            f"{f' [{artifact_selector}]' if artifact_selector else ''}; "
            f"local plan {local_plan.get('plan_id')} would publish a duplicate."
        )
    if load_remote_revisions is not None and not _local_published_plan_blobs_intact(ctx, str(local_plan["plan_id"])):
        _repair_local_plan_revision_blobs_from_remote(
            ctx,
            str(local_plan["plan_id"]),
            load_remote_revisions(remote_plan_id),
            load_remote_revision=(
                None
                if load_remote_revision is None
                else lambda plan_revision_id: load_remote_revision(remote_plan_id, plan_revision_id)
            ),
        )
    return local_plan, adoption, continuity_match
