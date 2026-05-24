from __future__ import annotations

from typing import Any, Optional

from ..remote_client import (
    RemoteError,
    create_plan as remote_create_plan,
    get_plan as remote_get_plan,
    list_plan_revisions as remote_list_plan_revisions,
    revise_plan as remote_revise_plan,
    update_plan_status as remote_update_plan_status,
)
from ..repo_paths import RepoContext
from ..store import get_local_plan, list_local_plan_revisions, mark_local_plan_published
from .plan_sync_adoption import (
    _plan_publish_revision_metadata_matches,
    _plan_revision_artifact_body,
    _plan_revisions_ascending,
)
from .plan_sync_matching import _local_plan_fully_published
from .remote_repository_defaults import _sync_remote_repository_defaults
from .workflow_mode_config import _normalize_text_value


def _select_source_session_id(value: str | None, fallback: str | None) -> str | None:
    return _normalize_text_value(value) or _normalize_text_value(fallback)


def _require_remote_plan_identity(requested_id: str, remote_data: dict[str, Any]) -> None:
    remote_id = remote_data.get("plan_id")
    if remote_id != requested_id:
        raise ValueError(
            f"Remote server returned plan_id {remote_id!r} while publishing local plan {requested_id}. "
            "Upgrade ait-server before publishing local short sequence IDs."
        )


def _map_equivalent_remote_plan_revision_suffix(
    local_revisions: list[dict[str, Any]],
    remote_revisions: list[dict[str, Any]],
    *,
    expected_remote_head: str | None,
) -> tuple[list[tuple[str, str]], list[dict[str, Any]]] | None:
    if not local_revisions:
        return [], []
    if expected_remote_head is None:
        return None
    remote_head_index = next(
        (index for index, revision in enumerate(remote_revisions) if revision.get("plan_revision_id") == expected_remote_head),
        None,
    )
    if remote_head_index is None:
        return None
    remote_suffix = remote_revisions[remote_head_index + 1 :]
    mappings: list[tuple[str, str]] = []
    next_local_index = 0
    last_matched_local_index = -1
    for remote_revision in remote_suffix:
        matched_local_index = next(
            (
                index
                for index in range(next_local_index, len(local_revisions))
                if _plan_publish_revision_metadata_matches(local_revisions[index], remote_revision)
            ),
            None,
        )
        if matched_local_index is None:
            return None
        remote_revision_id = _normalize_text_value(remote_revision.get("plan_revision_id"))
        if remote_revision_id is None:
            return None
        local_revision = local_revisions[matched_local_index]
        mappings.append((str(local_revision["plan_revision_id"]), remote_revision_id))
        next_local_index = matched_local_index + 1
        last_matched_local_index = matched_local_index
    remaining_local_revisions = local_revisions[last_matched_local_index + 1 :] if last_matched_local_index >= 0 else local_revisions
    return mappings, remaining_local_revisions


def _select_plan_divergent_retry_publish_target(
    local_revisions: list[dict[str, Any]],
    remote_revisions: list[dict[str, Any]],
    *,
    actual_remote_head: str | None,
) -> tuple[list[tuple[str, str]], list[dict[str, Any]], dict[str, Any]]:
    if not local_revisions:
        raise ValueError("Reconcile requires at least one local plan revision")
    local_head = local_revisions[-1]
    local_head_revision_id = _normalize_text_value(local_head.get("plan_revision_id"))
    if local_head_revision_id is None:
        raise ValueError("Local plan head is missing a revision id")
    remote_head = next(
        (revision for revision in remote_revisions if revision.get("plan_revision_id") == actual_remote_head),
        None,
    )
    if remote_head is not None and _plan_publish_revision_metadata_matches(local_head, remote_head):
        return (
            [(local_head_revision_id, str(actual_remote_head))],
            [],
            {
                "mode": "mapped_head",
                "local_head_revision_id": local_head_revision_id,
                "remote_head_revision_id": actual_remote_head,
            },
        )
    return (
        [],
        [local_head],
        {
            "mode": "publish_head",
            "local_head_revision_id": local_head_revision_id,
            "remote_head_revision_id": actual_remote_head,
        },
    )


def _local_plan_publish(
    ctx: RepoContext,
    plan_id: str,
    remote_name: Optional[str],
    *,
    divergent_retry_mode: str | None = None,
    source_session_id: str | None = None,
) -> dict[str, Any]:
    local_plan = get_local_plan(ctx, plan_id)
    remote_row, repo_name = _sync_remote_repository_defaults(ctx, remote_name)
    if local_plan["repo_name"] != repo_name:
        raise KeyError(f"Local plan {plan_id} belongs to repository {local_plan['repo_name']}, not {repo_name}")

    local_revisions = _plan_revisions_ascending(list_local_plan_revisions(ctx, plan_id))
    if not local_revisions:
        raise ValueError(f"Local plan {plan_id} has no revisions to publish")

    revision_mappings: list[tuple[str, str]] = []
    remote_plan: dict[str, Any] | None = None
    published_revision_count = 0
    rebase_details: dict[str, Any] | None = None
    reconcile_details: dict[str, Any] | None = None
    published_revisions = [
        row for row in local_revisions if row.get("publication_state") == "published" and row.get("published_plan_revision_id")
    ]
    unpublished_revisions = [
        row for row in local_revisions if row.get("publication_state") != "published" or not row.get("published_plan_revision_id")
    ]

    if local_plan.get("publication_state") != "published":
        seed_revision = local_revisions[0]
        try:
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
                plan_id=plan_id,
                source_kind=seed_revision.get("source_kind") or "manual_edit",
                source_session_id=_select_source_session_id(seed_revision.get("source_session_id"), source_session_id),
                artifact_body=_plan_revision_artifact_body(ctx, seed_revision),
            )
            _require_remote_plan_identity(plan_id, remote_plan)
            seed_remote_revision_id = ((remote_plan.get("head_revision") or {}).get("plan_revision_id"))
            if seed_remote_revision_id:
                revision_mappings.append((seed_revision["plan_revision_id"], seed_remote_revision_id))
                published_revision_count += 1
            unpublished_revisions = local_revisions[1:]
        except RemoteError as exc:
            if "409" not in str(exc) and "already exists" not in str(exc):
                raise
            remote_plan = remote_get_plan(remote_row["url"], plan_id)
            remote_revisions = _plan_revisions_ascending(remote_list_plan_revisions(remote_row["url"], plan_id))
            matched_count = min(len(local_revisions), len(remote_revisions))
            for local_revision, remote_revision in zip(local_revisions[:matched_count], remote_revisions[:matched_count]):
                if not _plan_publish_revision_metadata_matches(local_revision, remote_revision):
                    raise ValueError(
                        f"Remote plan {plan_id} already exists but revision {remote_revision.get('revision_number')} "
                        "does not match local publish history."
                    ) from exc
                remote_revision_id = _normalize_text_value(remote_revision.get("plan_revision_id"))
                if remote_revision_id is not None:
                    revision_mappings.append((local_revision["plan_revision_id"], remote_revision_id))
            unpublished_revisions = local_revisions[matched_count:]
    else:
        remote_plan = remote_get_plan(remote_row["url"], plan_id)
        latest_published = published_revisions[-1] if published_revisions else None
        latest_published_number = int(latest_published.get("revision_number") or 0) if latest_published else 0
        unpublished_revisions = [
            row
            for row in local_revisions
            if int(row.get("revision_number") or 0) > latest_published_number
            and (row.get("publication_state") != "published" or not row.get("published_plan_revision_id"))
        ]
        expected_remote_head = latest_published.get("published_plan_revision_id") if latest_published else None
        actual_remote_head = remote_plan.get("head_revision_id")
        if expected_remote_head and actual_remote_head != expected_remote_head:
            remote_revisions = _plan_revisions_ascending(remote_list_plan_revisions(remote_row["url"], plan_id))
            suffix_mapping_result = _map_equivalent_remote_plan_revision_suffix(
                unpublished_revisions,
                remote_revisions,
                expected_remote_head=expected_remote_head,
            )
            if suffix_mapping_result is None:
                if divergent_retry_mode is None:
                    raise ValueError(
                        f"Remote plan {plan_id} has advanced to {actual_remote_head}; retry the shared publish with `--rebase` "
                        "or the legacy `--reconcile` retry path."
                    )
                retry_mappings, unpublished_revisions, retry_details = _select_plan_divergent_retry_publish_target(
                    local_revisions,
                    remote_revisions,
                    actual_remote_head=actual_remote_head,
                )
                revision_mappings.extend(retry_mappings)
                if divergent_retry_mode == "rebase":
                    rebase_details = retry_details
                else:
                    reconcile_details = retry_details
            else:
                suffix_mappings, unpublished_revisions = suffix_mapping_result
                revision_mappings.extend(suffix_mappings)

    for revision in unpublished_revisions:
        expected_head_revision_id = None
        if remote_plan is not None:
            expected_head_revision_id = (remote_plan.get("head_revision") or {}).get("plan_revision_id") or remote_plan.get("head_revision_id")
        remote_plan = remote_revise_plan(
            remote_row["url"],
            plan_id,
            revision.get("artifact_path"),
            revision.get("artifact_selector"),
            revision.get("artifact_heading"),
            revision.get("items") or [],
            title=revision.get("title_snapshot"),
            summary=revision.get("summary"),
            source_kind=revision.get("source_kind") or "manual_edit",
            source_session_id=_select_source_session_id(revision.get("source_session_id"), source_session_id),
            artifact_body=_plan_revision_artifact_body(ctx, revision),
            expected_head_revision_id=expected_head_revision_id,
        )
        remote_revision_id = ((remote_plan.get("head_revision") or {}).get("plan_revision_id"))
        if not remote_revision_id:
            raise ValueError(f"Remote plan {plan_id} revise response did not include a head revision id")
        revision_mappings.append((revision["plan_revision_id"], remote_revision_id))
        published_revision_count += 1

    if remote_plan is None:
        remote_plan = remote_get_plan(remote_row["url"], plan_id)
    if remote_plan.get("status") != local_plan["status"]:
        remote_plan = remote_update_plan_status(remote_row["url"], plan_id, local_plan["status"])

    remote_head_revision_id = (remote_plan.get("head_revision") or {}).get("plan_revision_id") or remote_plan.get("head_revision_id")
    plan_row = mark_local_plan_published(
        ctx,
        plan_id,
        remote_name=remote_name or remote_row.get("name"),
        published_plan_id=plan_id,
        published_head_revision_id=remote_head_revision_id,
        revision_mappings=revision_mappings,
    )
    publish_action = "published"
    if rebase_details is not None:
        publish_action = "rebased" if published_revision_count else "rebased_mapping"
    elif reconcile_details is not None:
        publish_action = "reconciled" if published_revision_count else "reconciled_mapping"
    elif published_revision_count == 0:
        publish_action = "mapped"
    return {
        "plan": plan_row,
        "publish_action": publish_action,
        "published_revision_count": published_revision_count,
        "rebased": rebase_details is not None,
        "rebase_details": rebase_details,
        "reconciled": reconcile_details is not None,
        "reconcile_details": reconcile_details,
        "published_head_revision_id": remote_head_revision_id,
    }


def _publish_synced_local_plan_results(
    ctx: RepoContext,
    results: list[dict[str, Any]],
    remote_name: str | None,
    *,
    divergent_retry_mode: str | None = None,
    source_session_id: str | None = None,
) -> list[dict[str, Any]]:
    published: list[dict[str, Any]] = []
    seen_plan_ids: set[str] = set()
    for row in results:
        plan_id = _normalize_text_value(row.get("plan_id"))
        if plan_id is None or plan_id in seen_plan_ids:
            continue
        seen_plan_ids.add(plan_id)
        local_plan = get_local_plan(ctx, plan_id)
        if row.get("action") == "unchanged" and _local_plan_fully_published(local_plan) and divergent_retry_mode is None:
            continue
        publish_result = _local_plan_publish(
            ctx,
            plan_id,
            remote_name,
            divergent_retry_mode=divergent_retry_mode,
            source_session_id=source_session_id,
        )
        if publish_result.get("publish_action") == "noop":
            continue
        data = publish_result["plan"]
        head_revision = data.get("head_revision") if isinstance(data.get("head_revision"), dict) else {}
        rebase_details = publish_result.get("rebase_details") if isinstance(publish_result.get("rebase_details"), dict) else {}
        reconcile_details = publish_result.get("reconcile_details") if isinstance(publish_result.get("reconcile_details"), dict) else {}
        published.append(
            {
                "plan_id": data.get("plan_id"),
                "status": data.get("status"),
                "publication_state": data.get("publication_state"),
                "head_revision_id": data.get("head_revision_id"),
                "published_plan_id": data.get("published_plan_id"),
                "published_head_revision_id": data.get("published_head_revision_id"),
                "head_publication_state": head_revision.get("publication_state"),
                "publish_action": publish_result.get("publish_action"),
                "published_revision_count": publish_result.get("published_revision_count"),
                "rebased": bool(publish_result.get("rebased")),
                "rebase_mode": rebase_details.get("mode"),
                "rebase_remote_head_revision_id": rebase_details.get("remote_head_revision_id"),
                "rebase_local_head_revision_id": rebase_details.get("local_head_revision_id"),
                "reconciled": bool(publish_result.get("reconciled")),
                "reconcile_mode": reconcile_details.get("mode"),
                "reconcile_remote_head_revision_id": reconcile_details.get("remote_head_revision_id"),
                "reconcile_local_head_revision_id": reconcile_details.get("local_head_revision_id"),
            }
        )
    return published
