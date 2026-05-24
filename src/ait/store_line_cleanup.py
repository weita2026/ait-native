from __future__ import annotations

from typing import Any

from ait_protocol.common import normalize_optional_text, utc_now

from . import local_control
from .repo_paths import RepoContext
from .store_worktree_runtime import (
    DEFAULT_LINE_CLEANUP_OLDER_THAN,
    _coerce_datetime,
    _normalize_older_than,
    archive_line,
    current_line,
    list_lines,
)


def _normalize_line_cleanup_kind(value: str | None) -> str | None:
    text = normalize_optional_text(value)
    if text is None:
        return None
    lowered = text.lower().replace("-", "_")
    if lowered not in {"review_base", "review", "wip"}:
        raise ValueError("`--kind` must be one of `review_base`, `review`, or `wip`.")
    return lowered


def _line_cleanup_profile(line_name: str) -> tuple[str, str, str]:
    if line_name.startswith("review-base/"):
        return ("review_base", "after_idle", "review-base line idle past threshold")
    if line_name.startswith("review/"):
        return ("review", "after_idle", "review line idle past threshold")
    if line_name.startswith("wip/"):
        return ("wip", "after_idle", "wip line idle past threshold")
    return ("manual", "manual_only", "manual line")


def _line_usage_summary(ctx: RepoContext, line_name: str) -> dict[str, Any]:
    return _line_usage_summary_from_indexes(ctx, line_name, _build_line_usage_indexes(ctx))


def _empty_line_usage_summary() -> dict[str, Any]:
    return {
        "worktree_count": 0,
        "worktree_names": [],
        "active_session_count": 0,
        "active_session_ids": [],
        "active_change_count": 0,
        "active_change_ids": [],
    }


def _build_line_usage_indexes(
    ctx: RepoContext,
    *,
    worktree_rows: list[dict[str, Any]] | None = None,
) -> dict[str, dict[str, Any]]:
    active_session_rows = list(local_control.list_workflow_sessions(ctx, status="active"))
    change_rows = list(local_control.list_workflow_changes(ctx))
    worktree_index: dict[str, set[str]] = {}
    if worktree_rows is None:
        from .store_worktree_state import (
            _cache_worktree_summary_workflow_rows,
            _discard_worktree_summary_workflow_rows,
        )
        from .store_worktrees import list_worktrees

        _cache_worktree_summary_workflow_rows(
            ctx,
            active_sessions=active_session_rows,
            changes=change_rows,
        )
        try:
            worktree_rows = list_worktrees(ctx, refresh_status=False)
        finally:
            _discard_worktree_summary_workflow_rows(ctx)
    for row in worktree_rows:
        if str(row.get("workspace_status") or "") in {"missing", "detached"}:
            continue
        worktree_name = str(row.get("name") or "").strip()
        for candidate_line in {
            str(row.get("current_line") or "").strip(),
            str(row.get("registered_line_name") or "").strip(),
        }:
            if candidate_line:
                worktree_index.setdefault(candidate_line, set()).add(worktree_name)

    session_index: dict[str, set[str]] = {}
    for row in active_session_rows:
        line_name = str(row.get("line_name") or "").strip()
        session_id = str(row.get("session_id") or "").strip()
        if line_name and session_id:
            session_index.setdefault(line_name, set()).add(session_id)

    change_index: dict[str, set[str]] = {}
    for row in change_rows:
        if str(row.get("status") or "") in {"archived", "landed"}:
            continue
        line_name = str(row.get("base_line") or "").strip()
        change_id = str(row.get("change_id") or "").strip()
        if line_name and change_id:
            change_index.setdefault(line_name, set()).add(change_id)

    return {
        "worktrees": {key: sorted(values) for key, values in worktree_index.items()},
        "sessions": {key: sorted(values) for key, values in session_index.items()},
        "changes": {key: sorted(values) for key, values in change_index.items()},
    }


def _line_usage_summary_from_indexes(
    ctx: RepoContext,
    line_name: str,
    indexes: dict[str, dict[str, Any]] | None,
) -> dict[str, Any]:
    if not isinstance(indexes, dict):
        return _empty_line_usage_summary()
    worktree_names = list((indexes.get("worktrees") or {}).get(line_name) or [])
    active_session_ids = list((indexes.get("sessions") or {}).get(line_name) or [])
    active_change_ids = list((indexes.get("changes") or {}).get(line_name) or [])
    return {
        "worktree_count": len(worktree_names),
        "worktree_names": worktree_names,
        "active_session_count": len(active_session_ids),
        "active_session_ids": active_session_ids,
        "active_change_count": len(active_change_ids),
        "active_change_ids": active_change_ids,
    }


def _line_cleanup_decision(
    ctx: RepoContext,
    row: dict[str, Any],
    *,
    older_than: str | None = None,
    cleanup_kind: str | None = None,
    usage_indexes: dict[str, dict[str, Any]] | None = None,
    current_line_name: str | None = None,
    default_line_name: str | None = None,
    reference_now: str | None = None,
) -> dict[str, Any]:
    line_name = str(row.get("line_name") or "").strip()
    kind, cleanup_policy, cleanup_reason = _line_cleanup_profile(line_name)
    normalized_kind = _normalize_line_cleanup_kind(cleanup_kind)
    older_than_delta, older_than_label = _normalize_older_than(older_than or DEFAULT_LINE_CLEANUP_OLDER_THAN)
    usage = _line_usage_summary_from_indexes(ctx, line_name, usage_indexes)
    current = current_line_name or current_line(ctx)
    default_line = default_line_name or local_control.get_meta(ctx, "default_line") or "main"
    current_time = reference_now or utc_now()
    updated_at = normalize_optional_text(row.get("updated_at")) or normalize_optional_text(row.get("created_at")) or current_time
    idle_long_enough = (_coerce_datetime(current_time) - _coerce_datetime(updated_at)) >= older_than_delta
    protected_reason: str | None = None
    cleanup_candidate = False
    cleanup_class = "protected"

    if normalized_kind is not None and kind != normalized_kind:
        protected_reason = f"line kind {kind} does not match requested cleanup kind {normalized_kind}"
    elif str(row.get("status") or "active") == "archived":
        protected_reason = "line is already archived"
    elif line_name == default_line:
        protected_reason = "default line"
    elif line_name == current:
        protected_reason = "current line"
    elif usage["worktree_count"]:
        protected_reason = "line is still used by a worktree"
    elif usage["active_session_count"]:
        protected_reason = "line is still used by an active session"
    elif usage["active_change_count"]:
        protected_reason = "line is still used by an active local change"
    elif cleanup_policy == "manual_only":
        protected_reason = "line lifecycle is manual_only"
    elif not idle_long_enough:
        protected_reason = f"idle threshold {older_than_label} not reached"
    else:
        cleanup_class = "safe_cleanup_candidate"
        cleanup_candidate = True

    return {
        "line_name": line_name,
        "lifecycle_kind": kind,
        "cleanup_policy": cleanup_policy,
        "cleanup_class": cleanup_class,
        "cleanup_candidate": cleanup_candidate,
        "cleanup_reason": cleanup_reason if cleanup_candidate else None,
        "protected_reason": protected_reason,
        "last_activity_at": updated_at,
        "older_than": older_than_label,
        "usage": usage,
    }


def list_line_cleanup_candidates(
    ctx: RepoContext,
    *,
    older_than: str | None = None,
    include_protected: bool = False,
    cleanup_kind: str | None = None,
    worktree_rows: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    normalized_kind = _normalize_line_cleanup_kind(cleanup_kind)
    rows = list_lines(ctx)
    usage_indexes = _build_line_usage_indexes(ctx, worktree_rows=worktree_rows)
    current_line_name = current_line(ctx)
    default_line_name = normalize_optional_text(local_control.get_meta(ctx, "default_line")) or "main"
    reference_now = utc_now()
    candidates: list[dict[str, Any]] = []
    protected: list[dict[str, Any]] = []
    inspected_count = 0
    protected_count = 0
    for row in rows:
        decision = _line_cleanup_decision(
            ctx,
            row,
            older_than=older_than,
            cleanup_kind=normalized_kind,
            usage_indexes=usage_indexes,
            current_line_name=current_line_name,
            default_line_name=default_line_name,
            reference_now=reference_now,
        )
        enriched = dict(row)
        enriched.update(decision)
        inspected_count += 1
        if decision["cleanup_candidate"]:
            candidates.append(enriched)
        else:
            protected_count += 1
            if include_protected:
                protected.append(enriched)
    candidates.sort(key=lambda item: (str(item.get("last_activity_at") or ""), str(item.get("line_name") or "")))
    protected.sort(key=lambda item: (str(item.get("line_name") or ""), str(item.get("protected_reason") or "")))
    return {
        "older_than": _normalize_older_than(older_than or DEFAULT_LINE_CLEANUP_OLDER_THAN)[1],
        "cleanup_kind": normalized_kind,
        "include_protected": include_protected,
        "inspected_count": inspected_count,
        "candidate_count": len(candidates),
        "protected_count": protected_count,
        "candidates": candidates,
        "protected": protected if include_protected else [],
    }


def cleanup_lines(
    ctx: RepoContext,
    *,
    older_than: str | None = None,
    cleanup_kind: str | None = None,
    limit: int | None = None,
    dry_run: bool = False,
) -> dict[str, Any]:
    candidate_payload = list_line_cleanup_candidates(
        ctx,
        older_than=older_than,
        include_protected=False,
        cleanup_kind=cleanup_kind,
    )
    candidates = list(candidate_payload.get("candidates") or [])
    selected = candidates[: max(limit or 0, 0)] if limit is not None and limit >= 0 else candidates
    planned_rows = [
        {
            "line_name": str(row.get("line_name") or ""),
            "cleanup_policy": str(row.get("cleanup_policy") or ""),
            "cleanup_reason": str(row.get("cleanup_reason") or ""),
        }
        for row in selected
    ]
    if dry_run:
        return {
            "dry_run": True,
            "older_than": candidate_payload.get("older_than"),
            "cleanup_kind": candidate_payload.get("cleanup_kind"),
            "candidate_count": candidate_payload.get("candidate_count", len(candidates)),
            "planned_count": len(planned_rows),
            "planned_rows": planned_rows,
            "archived_count": 0,
            "archived_rows": [],
        }
    archived_rows = [archive_line(ctx, str(row.get("line_name") or "")) for row in selected]
    return {
        "dry_run": False,
        "older_than": candidate_payload.get("older_than"),
        "cleanup_kind": candidate_payload.get("cleanup_kind"),
        "candidate_count": candidate_payload.get("candidate_count", len(candidates)),
        "planned_count": len(planned_rows),
        "planned_rows": planned_rows,
        "archived_count": len(archived_rows),
        "archived_rows": archived_rows,
    }


__all__ = [
    "_build_line_usage_indexes",
    "_empty_line_usage_summary",
    "_line_cleanup_decision",
    "_line_cleanup_profile",
    "_line_usage_summary",
    "_line_usage_summary_from_indexes",
    "_normalize_line_cleanup_kind",
    "cleanup_lines",
    "list_line_cleanup_candidates",
]
