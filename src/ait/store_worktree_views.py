from __future__ import annotations

from pathlib import Path
from typing import Any

from ait_protocol.common import normalize_optional_text, read_json, utc_now

from . import local_content
from .repo_paths import (
    APP_DIR,
    RepoContext,
    WORKTREE_CONFIG_NAME,
)
from .store_worktree_runtime import (
    _coerce_datetime,
    current_line,
    workspace_status,
)
from .store_repo_config import _load_worktree_config
from .store_worktree_filesystem import _path_exists_or_directory_link
from .store_worktree_layout import _ensure_worktree_runtime_layout
from .store_worktree_metadata import (
    _WORKTREE_STATUS_CACHE_KEY,
    _build_worktree_status_cache_payload,
    _load_worktree_metadata,
    _save_worktree_metadata,
    _worktree_local_config_payload,
    _worktree_metadata_path,
    _worktree_status_cache,
)
from .store_worktree_state import (
    _build_worktree_summary_shared_state,
    _line_head_snapshot_id,
    _normalize_worktree_name,
    _repo_worktree_ctx,
    _worktree_cleanup_decision,
    _worktree_metadata_with_defaults,
    _worktree_retarget_summary,
    _workflow_statuses_for_worktree,
)


def _resolve_worktree_name(ctx: RepoContext, name: str | None = None) -> str:
    if name is not None:
        return _normalize_worktree_name(name)
    cfg = _load_worktree_config(ctx)
    worktree_name = cfg.get("worktree_name")
    if worktree_name:
        return _normalize_worktree_name(str(worktree_name))
    raise KeyError("Worktree name is required outside a worktree context.")


def _write_worktree_metadata_fields(
    ctx: RepoContext,
    worktree_name: str,
    *,
    clear_keys: tuple[str, ...] = (),
    **updates: Any,
) -> None:
    payload = _load_worktree_metadata(ctx, worktree_name)
    for key in clear_keys:
        payload.pop(str(key), None)
    for key, value in updates.items():
        payload[key] = value
    _save_worktree_metadata(ctx, worktree_name, payload)


def _maybe_discover_worktree(worktree_path: Path) -> RepoContext | None:
    marker_path = worktree_path / WORKTREE_CONFIG_NAME
    ait_link = worktree_path / APP_DIR
    if not worktree_path.is_dir() or not marker_path.exists() or not _path_exists_or_directory_link(ait_link):
        return None
    try:
        return RepoContext.discover(worktree_path)
    except FileNotFoundError:
        return None


def _worktree_summary(
    ctx: RepoContext,
    payload: dict,
    *,
    refresh_status: bool = True,
    persist_status_cache: bool = False,
    shared_state=None,
) -> dict:
    metadata = _worktree_metadata_with_defaults(payload)
    path_value = metadata.get("path")
    worktree_path = Path(path_value).expanduser().resolve() if path_value else None
    if worktree_path is not None and refresh_status:
        _ensure_worktree_runtime_layout(ctx.repo_root, worktree_path)
    worktree_layout_present = bool(
        worktree_path
        and worktree_path.is_dir()
        and (worktree_path / WORKTREE_CONFIG_NAME).exists()
        and _path_exists_or_directory_link(worktree_path / APP_DIR)
    )
    worktree_ctx = _maybe_discover_worktree(worktree_path) if refresh_status and worktree_path is not None else None
    current_line_name = metadata.get("line_name")
    cached_status = _worktree_status_cache(metadata)
    head_snapshot_id = None
    status_label = "missing"
    status_source = "verified"
    status_checked_at: str | None = None
    clean = None
    changed_count = None
    modified_paths: list[str] = []
    missing_paths: list[str] = []
    untracked_paths: list[str] = []

    if worktree_ctx is not None:
        current_line_name = current_line(worktree_ctx)
        status = workspace_status(worktree_ctx)
        status_label = "clean" if status["clean"] else "dirty"
        status_source = "verified"
        status_checked_at = utc_now()
        clean = status["clean"]
        changed_count = status["changed_count"]
        modified_paths = status["modified_paths"]
        missing_paths = status["missing_paths"]
        untracked_paths = status["untracked_paths"]
        try:
            head_snapshot_id = local_content.get_line(worktree_ctx, current_line_name).get("head_snapshot_id")
        except KeyError:
            head_snapshot_id = None
    elif worktree_layout_present:
        worktree_local_cfg = _worktree_local_config_payload(worktree_path)
        current_line_name = normalize_optional_text(worktree_local_cfg.get("current_line")) or current_line_name
        if current_line_name is not None:
            repo_ctx = shared_state.repo_ctx if shared_state is not None else _repo_worktree_ctx(ctx)
            line_head_cache = shared_state.line_head_snapshot_ids if shared_state is not None else None
            head_snapshot_id = _line_head_snapshot_id(repo_ctx, str(current_line_name), line_head_cache=line_head_cache)
        if (
            cached_status is not None
            and cached_status.get("current_line") == normalize_optional_text(current_line_name)
            and cached_status.get("head_snapshot_id") == normalize_optional_text(head_snapshot_id)
        ):
            status_label = str(cached_status.get("workspace_status") or "unknown")
            status_source = "cached"
            status_checked_at = normalize_optional_text(cached_status.get("status_checked_at"))
            clean = cached_status.get("clean")
            changed_count = cached_status.get("changed_count")
            modified_paths = list(cached_status.get("modified_paths") or [])
            missing_paths = list(cached_status.get("missing_paths") or [])
            untracked_paths = list(cached_status.get("untracked_paths") or [])
        else:
            status_label = "unknown"
            status_source = "unverified"
    elif worktree_path is not None and worktree_path.is_dir():
        status_label = "detached"
    elif current_line_name is not None:
        repo_ctx = shared_state.repo_ctx if shared_state is not None else _repo_worktree_ctx(ctx)
        line_head_cache = shared_state.line_head_snapshot_ids if shared_state is not None else None
        head_snapshot_id = _line_head_snapshot_id(repo_ctx, str(current_line_name), line_head_cache=line_head_cache)

    is_current = worktree_path.resolve() == ctx.root.resolve() if worktree_path and worktree_path.exists() else False
    decision = _worktree_cleanup_decision(
        ctx,
        {
            **metadata,
            "path": str(worktree_path) if worktree_path is not None else None,
            "clean": clean,
        },
        status_label=status_label,
        is_current=is_current,
        workflow_status_resolver=_workflow_statuses_for_worktree,
        shared_state=shared_state,
    )
    retarget = _worktree_retarget_summary(
        ctx,
        metadata,
        current_line_name=normalize_optional_text(current_line_name),
        head_snapshot_id=normalize_optional_text(head_snapshot_id),
        shared_state=shared_state,
    )

    if persist_status_cache and metadata.get("name"):
        worktree_name = _normalize_worktree_name(str(metadata.get("name") or ""))
        if status_source == "verified" and status_label in {"clean", "dirty", "missing", "detached"}:
            _write_worktree_metadata_fields(
                ctx,
                worktree_name,
                workspace_status_cache=_build_worktree_status_cache_payload(
                    workspace_status_value=status_label,
                    clean=clean,
                    changed_count=changed_count,
                    modified_paths=modified_paths,
                    missing_paths=missing_paths,
                    untracked_paths=untracked_paths,
                    current_line_name=normalize_optional_text(current_line_name),
                    head_snapshot_id=normalize_optional_text(head_snapshot_id),
                    status_checked_at=status_checked_at,
                ),
            )
        elif status_source == "unverified":
            _write_worktree_metadata_fields(
                ctx,
                worktree_name,
                clear_keys=(_WORKTREE_STATUS_CACHE_KEY,),
            )

    return {
        "name": metadata.get("name"),
        "path": str(worktree_path) if worktree_path is not None else None,
        "alias_path": normalize_optional_text(metadata.get("alias_path")),
        "open_path": normalize_optional_text(metadata.get("alias_path")) or (str(worktree_path) if worktree_path is not None else None),
        "venv_path": str(worktree_path / ".venv") if worktree_path is not None and (worktree_path / ".venv").exists() else None,
        "repo_root": metadata.get("repo_root") or str(ctx.repo_root),
        "registered_line_name": metadata.get("line_name"),
        "current_line": current_line_name,
        "head_snapshot_id": head_snapshot_id,
        "created_at": metadata.get("created_at"),
        "exists": bool(worktree_path) and worktree_path.is_dir(),
        "is_current": is_current,
        "workspace_status": status_label,
        "status_source": status_source,
        "status_checked_at": status_checked_at,
        "clean": clean,
        "changed_count": changed_count,
        "modified_paths": modified_paths,
        "missing_paths": missing_paths,
        "untracked_paths": untracked_paths,
        "bound_task_id": metadata.get("bound_task_id"),
        "bound_change_id": metadata.get("bound_change_id"),
        "auto_created_for_task": bool(metadata.get("auto_created_for_task", False)),
        "creation_kind": decision["creation_kind"],
        "cleanup_policy": decision["cleanup_policy"],
        "root_source": normalize_optional_text(metadata.get("root_source")),
        "last_used_at": decision["last_used_at"],
        "cleanup_class": decision["cleanup_class"],
        "cleanup_candidate": decision["cleanup_candidate"],
        "cleanup_reason": decision["cleanup_reason"],
        "protected_reason": decision["protected_reason"],
        "manual_review_candidate": decision["manual_review_candidate"],
        "manual_review_reason": decision["manual_review_reason"],
        "binding_summary": decision["binding_summary"],
        "cleanup": {
            "class": decision["cleanup_class"],
            "candidate": decision["cleanup_candidate"],
            "reason": decision["cleanup_reason"],
            "protected_reason": decision["protected_reason"],
            "manual_review_candidate": decision["manual_review_candidate"],
            "manual_review_reason": decision["manual_review_reason"],
            "older_than": decision["older_than"],
        },
        "fork_snapshot_id": retarget["fork_snapshot_id"],
        "forked_from_line": retarget["forked_from_line"],
        "target_base_line": retarget["target_base_line"],
        "target_base_snapshot_id": retarget["target_base_snapshot_id"],
        "needs_retarget": retarget["needs_retarget"],
        "feature_ahead_count": retarget["feature_ahead_count"],
        "base_behind_count": retarget["base_behind_count"],
        "rebase_state": retarget["rebase_state"],
        "rebase_started_at": retarget["rebase_started_at"],
        "rebase_original_head_snapshot_id": retarget["rebase_original_head_snapshot_id"],
        "rebase_onto_snapshot_id": retarget["rebase_onto_snapshot_id"],
        "rebase_conflict_paths": retarget["rebase_conflict_paths"],
        "last_retargeted_at": retarget["last_retargeted_at"],
        "retarget": retarget,
    }


def get_worktree(ctx: RepoContext, name: str | None = None, *, refresh_status: bool = True) -> dict:
    worktree_name = _resolve_worktree_name(ctx, name)
    payload = _load_worktree_metadata(ctx, worktree_name)
    return _worktree_summary(
        ctx,
        payload,
        refresh_status=refresh_status,
        persist_status_cache=refresh_status,
    )


def list_worktrees(ctx: RepoContext, *, refresh_status: bool = True) -> list[dict]:
    rows: list[dict] = []
    shared_state = _build_worktree_summary_shared_state(ctx)
    for path in sorted(ctx.worktree_registry_dir.glob("*.json")):
        payload = read_json(path, default={}) or {}
        if not isinstance(payload, dict):
            continue
        rows.append(
            _worktree_summary(
                ctx,
                payload,
                refresh_status=refresh_status,
                persist_status_cache=refresh_status,
                shared_state=shared_state,
            )
        )
    return rows


def worktree_doctor(ctx: RepoContext, *, refresh_status: bool = True) -> dict:
    rows = list_worktrees(ctx, refresh_status=refresh_status)
    return worktree_doctor_from_rows(rows)


def worktree_doctor_from_rows(rows: list[dict[str, Any]]) -> dict:
    counts = {
        "total_count": len(rows),
        "current_count": 0,
        "clean_count": 0,
        "dirty_count": 0,
        "missing_count": 0,
        "detached_count": 0,
        "protected_count": 0,
        "safe_auto_remove_count": 0,
        "safe_cleanup_candidate_count": 0,
        "manual_review_candidate_count": 0,
    }
    stale_rows: list[dict] = []
    cleanup_candidate_rows: list[dict] = []
    manual_review_rows: list[dict] = []
    for row in rows:
        if row.get("is_current"):
            counts["current_count"] += 1
        status = row.get("workspace_status")
        if status == "clean":
            counts["clean_count"] += 1
        elif status == "dirty":
            counts["dirty_count"] += 1
        elif status == "missing":
            counts["missing_count"] += 1
            stale_rows.append(row)
        elif status == "detached":
            counts["detached_count"] += 1
            stale_rows.append(row)
        cleanup_class = str(row.get("cleanup_class") or "")
        if cleanup_class == "protected":
            counts["protected_count"] += 1
        elif cleanup_class == "safe_auto_remove":
            counts["safe_auto_remove_count"] += 1
            cleanup_candidate_rows.append(row)
        elif cleanup_class == "safe_cleanup_candidate":
            counts["safe_cleanup_candidate_count"] += 1
            cleanup_candidate_rows.append(row)
        if row.get("manual_review_candidate"):
            counts["manual_review_candidate_count"] += 1
            manual_review_rows.append(row)
    cleanup_candidate_rows.sort(
        key=lambda row: (
            0 if str(row.get("cleanup_class") or "") == "safe_auto_remove" else 1,
            _coerce_datetime(normalize_optional_text(row.get("last_used_at"))),
            str(row.get("name") or ""),
        )
    )
    stale_rows.sort(key=lambda row: (str(row.get("workspace_status") or ""), str(row.get("name") or "")))
    manual_review_rows.sort(key=lambda row: (str(row.get("manual_review_reason") or ""), str(row.get("name") or "")))
    return {
        **counts,
        "healthy": counts["missing_count"] == 0 and counts["detached_count"] == 0,
        "stale_count": len(stale_rows),
        "stale_rows": stale_rows,
        "cleanup_candidate_rows": cleanup_candidate_rows,
        "manual_review_rows": manual_review_rows,
        "rows": rows,
    }


__all__ = [
    "_maybe_discover_worktree",
    "_resolve_worktree_name",
    "_worktree_summary",
    "_write_worktree_metadata_fields",
    "get_worktree",
    "list_worktrees",
    "worktree_doctor",
    "worktree_doctor_from_rows",
]
