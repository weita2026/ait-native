from __future__ import annotations

import hashlib
from typing import Any

from ..repo_paths import RepoContext
from .workflow_mode_config import _normalize_text_value


def _artifact_blob_id(markdown: str) -> str:
    return f"BLB-{hashlib.sha256(markdown.encode('utf-8')).hexdigest()[:20]}"


def _artifact_candidates_open(candidates: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [row for row in candidates if not _plan_status_is_historical(row.get("status"))]


def _plan_status_is_historical(status: Any) -> bool:
    return str(status or "").strip().lower() in {"archived", "superseded"}


def _plan_artifact_identity_key(
    artifact_path: str,
    artifact_selector: str | None,
) -> tuple[str, str | None]:
    return str(artifact_path), _normalize_text_value(artifact_selector)


def _plan_artifact_identity_label(
    artifact_path: str,
    artifact_selector: str | None,
) -> str:
    normalized_selector = _normalize_text_value(artifact_selector)
    if normalized_selector is None:
        return str(artifact_path)
    return f"{artifact_path} [{normalized_selector}]"


def _select_sync_existing_plan(artifact_label: str, candidates: list[dict[str, Any]]) -> dict[str, Any] | None:
    current_candidates = [row for row in candidates if not _plan_status_is_historical(row.get("status"))]
    if len(current_candidates) > 1:
        raise ValueError(
            f"Multiple current plans already track {artifact_label}: "
            f"{', '.join(str(row.get('plan_id') or '') for row in current_candidates)}"
        )
    return current_candidates[0] if current_candidates else None


def _index_plans_by_artifact_path(plans: list[dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    indexed: dict[str, list[dict[str, Any]]] = {}
    for plan in plans:
        head_revision = plan.get("head_revision") or {}
        artifact_path = _normalize_text_value(head_revision.get("artifact_path"))
        if artifact_path is None:
            continue
        indexed.setdefault(artifact_path, []).append(plan)
    return indexed


def _index_plans_by_artifact_identity(plans: list[dict[str, Any]]) -> dict[tuple[str, str | None], list[dict[str, Any]]]:
    indexed: dict[tuple[str, str | None], list[dict[str, Any]]] = {}
    for plan in plans:
        head_revision = plan.get("head_revision") or {}
        artifact_path = _normalize_text_value(head_revision.get("artifact_path"))
        if artifact_path is None:
            continue
        key = _plan_artifact_identity_key(
            artifact_path,
            _normalize_text_value(head_revision.get("artifact_selector")),
        )
        indexed.setdefault(key, []).append(plan)
    return indexed


def _plan_head_value(plan: dict[str, Any], key: str) -> str | None:
    head_revision = plan.get("head_revision") if isinstance(plan.get("head_revision"), dict) else {}
    return _normalize_text_value(head_revision.get(key))


def _artifact_path_exists(ctx: RepoContext, artifact_path: str | None) -> bool:
    normalized = _normalize_text_value(artifact_path)
    if normalized is None:
        return False
    return (ctx.root / normalized).exists()


def _open_plans_matching_selector(plans: list[dict[str, Any]], selector: str) -> list[dict[str, Any]]:
    return [
        row
        for row in plans
        if not _plan_status_is_historical(row.get("status")) and _plan_head_value(row, "artifact_selector") == selector
    ]


def _open_generic_plans_matching_blob_id(plans: list[dict[str, Any]], blob_id: str) -> list[dict[str, Any]]:
    return [
        row
        for row in plans
        if not _plan_status_is_historical(row.get("status"))
        and _plan_head_value(row, "artifact_selector") is None
        and _plan_head_value(row, "artifact_blob_id") == blob_id
    ]


def _continuity_conflict_due_to_existing_source_path(
    *,
    plan_id: str,
    previous_artifact_path: str,
    artifact_path: str,
    artifact_selector: str | None,
) -> ValueError:
    selector_detail = f" [{artifact_selector}]" if artifact_selector is not None else ""
    return ValueError(
        f"Tracked plan {plan_id} still points at {previous_artifact_path}{selector_detail}; "
        f"rename/move continuity to {artifact_path} is only allowed after the previously tracked Markdown path disappears."
    )


def _select_sync_existing_plan_with_continuity(
    ctx: RepoContext,
    artifact: dict[str, Any],
    *,
    indexed_plans_by_identity: dict[tuple[str, str | None], list[dict[str, Any]]],
    plans: list[dict[str, Any]],
) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
    artifact_path = str(artifact["artifact_path"])
    artifact_selector = _normalize_text_value(artifact.get("artifact_selector"))
    artifact_key = _plan_artifact_identity_key(artifact_path, artifact_selector)
    artifact_label = _plan_artifact_identity_label(artifact_path, artifact_selector)
    direct_match = _select_sync_existing_plan(artifact_label, indexed_plans_by_identity.get(artifact_key, []))
    if direct_match is not None:
        return direct_match, None

    if artifact_selector is not None:
        selector_matches = _open_plans_matching_selector(plans, artifact_selector)
        if not selector_matches:
            return None, None
        if len(selector_matches) > 1:
            raise ValueError(
                f"Multiple open plans already expose selector {artifact_selector}: "
                + ", ".join(str(row.get("plan_id") or "") for row in selector_matches)
            )
        matched_plan = selector_matches[0]
        previous_artifact_path = _plan_head_value(matched_plan, "artifact_path")
        if previous_artifact_path is None:
            return matched_plan, None
        if _artifact_path_exists(ctx, previous_artifact_path):
            raise _continuity_conflict_due_to_existing_source_path(
                plan_id=str(matched_plan.get("plan_id") or ""),
                previous_artifact_path=previous_artifact_path,
                artifact_path=artifact_path,
                artifact_selector=artifact_selector,
            )
        return matched_plan, {
            "match_kind": "artifact_selector_move",
            "previous_artifact_path": previous_artifact_path,
            "new_artifact_path": artifact_path,
            "artifact_selector": artifact_selector,
        }

    artifact_blob_id = _normalize_text_value(artifact.get("artifact_blob_id"))
    if artifact_blob_id is None:
        return None, None
    blob_matches = _open_generic_plans_matching_blob_id(plans, artifact_blob_id)
    if not blob_matches:
        return None, None
    if len(blob_matches) > 1:
        raise ValueError(
            f"Multiple open generic Markdown plans share exact blob {artifact_blob_id}; "
            f"rename/move continuity for {artifact_path} is ambiguous."
        )
    matched_plan = blob_matches[0]
    previous_artifact_path = _plan_head_value(matched_plan, "artifact_path")
    if previous_artifact_path is None:
        return matched_plan, None
    if _artifact_path_exists(ctx, previous_artifact_path):
        raise _continuity_conflict_due_to_existing_source_path(
            plan_id=str(matched_plan.get("plan_id") or ""),
            previous_artifact_path=previous_artifact_path,
            artifact_path=artifact_path,
            artifact_selector=None,
        )
    return matched_plan, {
        "match_kind": "exact_blob_move",
        "previous_artifact_path": previous_artifact_path,
        "new_artifact_path": artifact_path,
        "artifact_blob_id": artifact_blob_id,
    }


def _local_plan_fully_published(plan: dict[str, Any]) -> bool:
    head_revision = plan.get("head_revision") if isinstance(plan.get("head_revision"), dict) else {}
    return (
        plan.get("publication_state") == "published"
        and head_revision.get("publication_state") == "published"
        and _normalize_text_value(plan.get("published_head_revision_id")) is not None
    )


def _plan_heads_equivalent(left: dict[str, Any], right: dict[str, Any]) -> bool:
    left_head = left.get("head_revision") if isinstance(left.get("head_revision"), dict) else {}
    right_head = right.get("head_revision") if isinstance(right.get("head_revision"), dict) else {}
    return (
        _normalize_text_value(left.get("title")) == _normalize_text_value(right.get("title"))
        and _normalize_text_value(left_head.get("artifact_path")) == _normalize_text_value(right_head.get("artifact_path"))
        and _normalize_text_value(left_head.get("artifact_selector")) == _normalize_text_value(right_head.get("artifact_selector"))
        and _normalize_text_value(left_head.get("artifact_heading")) == _normalize_text_value(right_head.get("artifact_heading"))
        and _normalize_text_value(left_head.get("artifact_blob_id")) == _normalize_text_value(right_head.get("artifact_blob_id"))
        and list(left_head.get("items") or []) == list(right_head.get("items") or [])
    )


def _select_existing_plan_for_artifact(artifact: dict[str, Any], plans: list[dict[str, Any]]) -> dict[str, Any]:
    artifact_path = str(artifact["artifact_path"])
    artifact_selector = _normalize_text_value(str(artifact.get("artifact_selector") or ""))
    path_matches = [
        row
        for row in plans
        if str((row.get("head_revision") or {}).get("artifact_path") or "") == artifact_path
    ]
    selector_matches = (
        [
            row
            for row in path_matches
            if str((row.get("head_revision") or {}).get("artifact_selector") or "") == artifact_selector
        ]
        if artifact_selector is not None
        else path_matches
    )
    open_matches = [row for row in selector_matches if not _plan_status_is_historical(row.get("status"))]
    if len(open_matches) > 1:
        raise ValueError(
            "Multiple open plans match "
            f"{artifact_path}"
            f"{f' [{artifact_selector}]' if artifact_selector else ''}: "
            + ", ".join(str(row.get("plan_id") or "") for row in open_matches)
        )
    if open_matches:
        return open_matches[0]

    if selector_matches:
        raise ValueError(
            "Tracked plan history exists for "
            f"{artifact_path}"
            f"{f' [{artifact_selector}]' if artifact_selector else ''}. "
            "Use the explicit `<plan-id>` form to inspect the historical record."
        )

    known_selectors = sorted(
        {
            str((row.get("head_revision") or {}).get("artifact_selector") or "")
            for row in path_matches
            if str((row.get("head_revision") or {}).get("artifact_selector") or "")
        }
    )
    if known_selectors:
        raise ValueError(
            f"No tracked plan currently matches {artifact_path}"
            f"{f' [{artifact_selector}]' if artifact_selector else ''}. "
            f"Known tracked refs for this file: {', '.join(known_selectors)}"
        )
    raise ValueError(
        f"No tracked plan currently matches {artifact_path}. "
        f"Run `ait plan sync {artifact_path}` first."
    )


def _plan_matches_sync_artifact(
    plan: dict[str, Any],
    artifact: dict[str, Any],
    *,
    require_title_match: bool = True,
) -> bool:
    head_revision = plan.get("head_revision") or {}
    head_selector = _normalize_text_value(head_revision.get("artifact_selector"))
    artifact_selector = _normalize_text_value(artifact.get("artifact_selector"))
    artifact_body = artifact.get("artifact_body")
    expected_blob_id = None
    if isinstance(artifact_body, str):
        expected_blob_id = _artifact_blob_id(artifact_body)
    return (
        ((str(plan.get("title") or "") == str(artifact["artifact_heading"])) if require_title_match else True)
        and str(head_revision.get("artifact_path") or "") == str(artifact["artifact_path"])
        and head_selector == artifact_selector
        and str(head_revision.get("artifact_heading") or "") == str(artifact["artifact_heading"])
        and list(head_revision.get("items") or []) == list(artifact["items"] or [])
        and (expected_blob_id is None or str(head_revision.get("artifact_blob_id") or "") == expected_blob_id)
    )
