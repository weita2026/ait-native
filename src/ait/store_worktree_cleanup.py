from __future__ import annotations

from pathlib import Path
from typing import Any

from ait_protocol.common import normalize_optional_text, read_json, utc_now, write_json

from . import local_control
from .repo_paths import APP_DIR, RepoContext, WORKTREE_CONFIG_NAME
from .store_worktree_filesystem import (
    _path_exists_or_directory_link,
    _remove_path_entry,
    _remove_tree_entry,
)
from .store_worktree_metadata import (
    _load_worktree_metadata,
    _save_worktree_metadata,
    _worktree_metadata_path,
)
from .store_worktree_runtime import (
    _coerce_datetime,
    _normalize_older_than,
    _normalize_worktree_cleanup_policy,
    current_line,
    workspace_status,
)
from .store_worktree_state import (
    _active_root_worktree_binding_name,
    _normalize_worktree_name,
    _repo_worktree_ctx,
    _worktree_cleanup_decision,
    _workflow_statuses_for_worktree,
)
from .store_worktree_views import (
    _maybe_discover_worktree,
    _resolve_worktree_name,
    get_worktree,
    list_worktrees,
)

__all__ = [
    "_touch_worktree_metadata",
    "_update_worktree_registration",
    "cleanup_worktrees",
    "list_worktree_cleanup_candidates",
    "remove_worktree",
    "remove_worktrees",
    "touch_worktree_usage",
]


def _cleanup_workflow_status_resolver():
    try:
        from . import store_worktrees as _store_worktrees
    except Exception:
        return _workflow_statuses_for_worktree
    return getattr(_store_worktrees, "_workflow_statuses_for_worktree", _workflow_statuses_for_worktree)


def _touch_worktree_metadata(ctx: RepoContext, worktree_name: str, *, timestamp: str | None = None) -> dict[str, Any]:
    repo_ctx = _repo_worktree_ctx(ctx)
    _update_worktree_registration(repo_ctx, worktree_name, last_used_at=timestamp or utc_now())
    return get_worktree(repo_ctx, worktree_name)


def _update_worktree_registration(ctx: RepoContext, worktree_name: str, **updates: Any) -> None:
    try:
        payload = _load_worktree_metadata(ctx, worktree_name)
    except KeyError:
        return
    for key, value in updates.items():
        if value is not None:
            payload[key] = value
    _save_worktree_metadata(ctx, worktree_name, payload)


def list_worktree_cleanup_candidates(
    ctx: RepoContext,
    *,
    older_than: str | None = None,
    cleanup_policy: str | None = None,
    include_protected: bool = False,
    allow_manual_only: bool = False,
) -> dict:
    normalized_policy = _normalize_worktree_cleanup_policy(cleanup_policy, default=None)
    _, older_than_label = _normalize_older_than(older_than)
    rows = list_worktrees(ctx)
    candidates: list[dict[str, Any]] = []
    protected: list[dict[str, Any]] = []
    stale_rows: list[dict[str, Any]] = []
    inspected_count = 0
    protected_count = 0

    for row in rows:
        if normalized_policy is not None and str(row.get("cleanup_policy") or "") != normalized_policy:
            continue
        inspected_count += 1
        status_label = str(row.get("workspace_status") or "missing").strip() or "missing"
        decision = _worktree_cleanup_decision(
            ctx,
            row,
            status_label=status_label,
            is_current=bool(row.get("is_current")),
            older_than=older_than_label,
            allow_manual_only=allow_manual_only,
            workflow_status_resolver=_cleanup_workflow_status_resolver(),
        )
        enriched = dict(row)
        enriched.update(
            {
                "creation_kind": decision["creation_kind"],
                "cleanup_policy": decision["cleanup_policy"],
                "last_used_at": decision["last_used_at"],
                "cleanup_class": decision["cleanup_class"],
                "cleanup_candidate": decision["cleanup_candidate"],
                "cleanup_reason": decision["cleanup_reason"],
                "protected_reason": decision["protected_reason"],
                "manual_review_candidate": decision["manual_review_candidate"],
                "manual_review_reason": decision["manual_review_reason"],
                "force_remove_dirty": bool(decision.get("force_remove_dirty")),
                "binding_summary": decision["binding_summary"],
                "cleanup": {
                    "class": decision["cleanup_class"],
                    "candidate": decision["cleanup_candidate"],
                    "reason": decision["cleanup_reason"],
                    "protected_reason": decision["protected_reason"],
                    "manual_review_candidate": decision["manual_review_candidate"],
                    "manual_review_reason": decision["manual_review_reason"],
                    "force_remove_dirty": bool(decision.get("force_remove_dirty")),
                    "older_than": decision["older_than"],
                },
            }
        )
        cleanup_class = str(decision.get("cleanup_class") or "")
        if cleanup_class == "stale":
            stale_rows.append(enriched)
        elif cleanup_class in {"safe_auto_remove", "safe_cleanup_candidate"} and bool(decision.get("cleanup_candidate")):
            candidates.append(enriched)
        elif cleanup_class == "protected":
            protected_count += 1
            if include_protected:
                protected.append(enriched)

    candidates.sort(
        key=lambda row: (
            0 if str(row.get("cleanup_class") or "") == "safe_auto_remove" else 1,
            _coerce_datetime(normalize_optional_text(row.get("last_used_at"))),
            str(row.get("name") or ""),
        )
    )
    protected.sort(
        key=lambda row: (
            str(row.get("protected_reason") or ""),
            str(row.get("name") or ""),
        )
    )
    stale_rows.sort(key=lambda row: (str(row.get("workspace_status") or ""), str(row.get("name") or "")))

    return {
        "older_than": older_than_label,
        "cleanup_policy": normalized_policy,
        "include_protected": include_protected,
        "allow_manual_only": allow_manual_only,
        "inspected_count": inspected_count,
        "candidate_count": len(candidates),
        "protected_count": protected_count,
        "stale_count": len(stale_rows),
        "candidates": candidates,
        "protected": protected if include_protected else [],
        "stale_rows": stale_rows,
    }


def cleanup_worktrees(
    ctx: RepoContext,
    *,
    older_than: str | None = None,
    cleanup_policy: str | None = None,
    allow_manual_only: bool = False,
    limit: int | None = None,
    dry_run: bool = False,
) -> dict[str, Any]:
    candidate_payload = list_worktree_cleanup_candidates(
        ctx,
        older_than=older_than,
        cleanup_policy=cleanup_policy,
        include_protected=False,
        allow_manual_only=allow_manual_only,
    )
    candidates = list(candidate_payload.get("candidates") or [])
    selected = candidates[: max(limit or 0, 0)] if limit is not None and limit >= 0 else candidates
    planned_rows = [
        {
            "name": str(row.get("name") or ""),
            "path": str(row.get("path") or ""),
            "current_line": str(row.get("current_line") or row.get("registered_line_name") or ""),
            "cleanup_class": str(row.get("cleanup_class") or ""),
            "cleanup_policy": str(row.get("cleanup_policy") or ""),
            "cleanup_reason": str(row.get("cleanup_reason") or ""),
            "deleted_path": True,
            "force": bool(row.get("force_remove_dirty")),
        }
        for row in selected
    ]
    if dry_run:
        return {
            "dry_run": True,
            "older_than": candidate_payload.get("older_than"),
            "cleanup_policy": candidate_payload.get("cleanup_policy"),
            "allow_manual_only": allow_manual_only,
            "candidate_count": candidate_payload.get("candidate_count", len(candidates)),
            "planned_count": len(planned_rows),
            "planned_rows": planned_rows,
            "removed_count": 0,
            "removed_rows": [],
        }
    if selected:
        removed_rows = [
            remove_worktree(
                ctx,
                str(row.get("name") or ""),
                delete_path=True,
                force=bool(row.get("force_remove_dirty")),
            )
            for row in selected
        ]
    else:
        removed_rows = []
    return {
        "dry_run": False,
        "older_than": candidate_payload.get("older_than"),
        "cleanup_policy": candidate_payload.get("cleanup_policy"),
        "allow_manual_only": allow_manual_only,
        "candidate_count": candidate_payload.get("candidate_count", len(candidates)),
        "planned_count": len(planned_rows),
        "planned_rows": planned_rows,
        "removed_count": len(removed_rows),
        "removed_rows": removed_rows,
    }


def touch_worktree_usage(ctx: RepoContext, name: str | None = None) -> dict[str, Any] | None:
    repo_ctx = _repo_worktree_ctx(ctx)
    resolved_name = normalize_optional_text(name)
    if resolved_name is None:
        if ctx.is_worktree:
            try:
                resolved_name = _resolve_worktree_name(ctx)
            except KeyError:
                return None
        else:
            resolved_name = _active_root_worktree_binding_name(repo_ctx)
            if resolved_name is None:
                return None
    try:
        return _touch_worktree_metadata(repo_ctx, _normalize_worktree_name(resolved_name))
    except (KeyError, ValueError):
        return None


def _preflight_worktree_removal(ctx: RepoContext, name: str, *, force: bool = False) -> dict[str, Any]:
    worktree_name = _resolve_worktree_name(ctx, name)
    metadata_path = _worktree_metadata_path(ctx, worktree_name)
    payload = _load_worktree_metadata(ctx, worktree_name)
    worktree_path = Path(payload["path"]).resolve()
    if ctx.root.resolve() == worktree_path:
        raise ValueError("Cannot remove the current worktree from inside itself.")
    worktree_ctx = _maybe_discover_worktree(worktree_path)
    worktree_status = workspace_status(worktree_ctx) if worktree_ctx is not None else None
    if worktree_status is not None and not worktree_status["clean"] and not force:
        sample = ", ".join(worktree_status["changed_paths"][:5])
        if len(worktree_status["changed_paths"]) > 5:
            sample += ", ..."
        raise ValueError(f"Worktree {worktree_name} has unsaved changes: {sample}. Use --force to remove it.")
    return {
        "name": worktree_name,
        "path": str(worktree_path),
        "metadata_path": str(metadata_path),
        "worktree_ctx": worktree_ctx,
        "worktree_status": worktree_status,
        "workspace_status": "clean" if worktree_status is None or worktree_status["clean"] else "dirty",
    }


def remove_worktree(ctx: RepoContext, name: str, *, delete_path: bool = False, force: bool = False) -> dict[str, Any]:
    removal = _preflight_worktree_removal(ctx, name, force=force)
    worktree_name = str(removal["name"])
    metadata_path = Path(removal["metadata_path"])
    worktree_path = Path(removal["path"])
    metadata = _load_worktree_metadata(ctx, worktree_name)
    alias_path_value = normalize_optional_text(metadata.get("alias_path"))
    alias_path = Path(alias_path_value).expanduser() if alias_path_value is not None else None
    worktree_status = removal["worktree_status"]
    marker_path = worktree_path / WORKTREE_CONFIG_NAME
    ait_link = worktree_path / APP_DIR
    if marker_path.exists():
        marker_path.unlink()
    if _path_exists_or_directory_link(ait_link):
        _remove_path_entry(ait_link)
    if delete_path and worktree_path.exists():
        for child in sorted(worktree_path.iterdir(), key=lambda item: str(item), reverse=True):
            _remove_tree_entry(child)
        if worktree_path.exists():
            worktree_path.rmdir()
    if alias_path is not None and _path_exists_or_directory_link(alias_path):
        _remove_path_entry(alias_path)
    metadata_path.unlink()
    config = read_json(ctx.config_path, default={}) or {}
    if isinstance(config, dict) and str(config.get("worktree_name") or "").strip() == worktree_name:
        config = dict(config)
        config.pop("worktree_name", None)
        write_json(ctx.config_path, config)
    local_control.record_event(
        ctx,
        "worktree.removed",
        "worktree",
        worktree_name,
        {
            "name": worktree_name,
            "path": str(worktree_path),
            "alias_path": str(alias_path) if alias_path is not None else None,
            "delete_path": delete_path,
            "force": force,
            "workspace_status": "clean" if worktree_status is None or worktree_status["clean"] else "dirty",
        },
    )
    return {
        "name": worktree_name,
        "path": str(worktree_path),
        "alias_path": str(alias_path) if alias_path is not None else None,
        "removed": True,
        "deleted_path": delete_path,
        "workspace_status": "clean" if worktree_status is None or worktree_status["clean"] else "dirty",
    }


def remove_worktrees(
    ctx: RepoContext,
    names: list[str],
    *,
    delete_path: bool = False,
    force: bool = False,
    dry_run: bool = False,
) -> dict[str, Any]:
    if not names:
        raise ValueError("Provide at least one worktree name.")
    ordered_names: list[str] = []
    seen: set[str] = set()
    for raw_name in names:
        normalized = _normalize_worktree_name(raw_name)
        if normalized in seen:
            continue
        seen.add(normalized)
        ordered_names.append(normalized)
    removals = [_preflight_worktree_removal(ctx, name, force=force) for name in ordered_names]
    planned_rows = [
        {
            "name": str(removal["name"]),
            "path": str(removal["path"]),
            "workspace_status": str(removal["workspace_status"]),
            "deleted_path": bool(delete_path),
        }
        for removal in removals
    ]
    if dry_run:
        return {
            "dry_run": True,
            "planned_count": len(planned_rows),
            "planned_rows": planned_rows,
            "removed_count": 0,
            "removed_rows": [],
        }
    removed_rows = [remove_worktree(ctx, str(removal["name"]), delete_path=delete_path, force=force) for removal in removals]
    return {
        "dry_run": False,
        "planned_count": len(planned_rows),
        "planned_rows": planned_rows,
        "removed_count": len(removed_rows),
        "removed_rows": removed_rows,
    }
