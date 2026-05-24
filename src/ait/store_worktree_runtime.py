from __future__ import annotations

import re
from datetime import datetime, timedelta, timezone
from typing import Any, Optional

from ait_protocol.common import normalize_optional_text, read_json

from . import local_content, local_control
from .repo_paths import RepoContext
from .store_repo_config import (
    _load_worktree_config,
    _save_worktree_config,
    _set_worktree_materialized_snapshot,
    load_config,
    save_config,
)

WORKTREE_CREATION_KINDS = frozenset(
    {
        "task_auto_created",
        "manual_add",
        "bootstrap_helper",
        "land_helper",
        "scratch",
    }
)
WORKTREE_CLEANUP_POLICIES = frozenset(
    {
        "manual_only",
        "after_remote_land",
        "after_task_complete",
        "after_idle",
        "never",
    }
)
WORKTREE_CLEANUP_CLASSES = frozenset(
    {
        "protected",
        "safe_auto_remove",
        "safe_cleanup_candidate",
        "stale",
    }
)
LINE_CLEANUP_CLASSES = frozenset({"protected", "safe_cleanup_candidate"})
DEFAULT_WORKTREE_CREATION_KIND = "manual_add"
DEFAULT_WORKTREE_CLEANUP_POLICY = "manual_only"
DEFAULT_WORKTREE_CLEANUP_OLDER_THAN = "7d"
DEFAULT_LINE_CLEANUP_OLDER_THAN = "7d"
_WORKTREE_OLDER_THAN_RE = re.compile(r"^(?P<count>\d+)\s*(?P<unit>[dhm])$", re.IGNORECASE)
_DEFAULT_CLEANUP_POLICY_BY_CREATION_KIND = {
    "task_auto_created": "after_remote_land",
    "manual_add": "manual_only",
    "bootstrap_helper": "after_idle",
    "land_helper": "after_idle",
    "scratch": "after_idle",
}

__all__ = [
    "DEFAULT_LINE_CLEANUP_OLDER_THAN",
    "DEFAULT_WORKTREE_CLEANUP_OLDER_THAN",
    "DEFAULT_WORKTREE_CLEANUP_POLICY",
    "DEFAULT_WORKTREE_CREATION_KIND",
    "LINE_CLEANUP_CLASSES",
    "WORKTREE_CLEANUP_CLASSES",
    "WORKTREE_CLEANUP_POLICIES",
    "WORKTREE_CREATION_KINDS",
    "_coerce_datetime",
    "_default_cleanup_policy_for_creation_kind",
    "_normalize_older_than",
    "_normalize_worktree_cleanup_policy",
    "_normalize_worktree_creation_kind",
    "_set_current_line",
    "archive_line",
    "create_line",
    "create_snapshot",
    "current_line",
    "get_remote",
    "list_lines",
    "set_line_head",
    "switch_line",
    "workspace_status",
]


def _coerce_datetime(value: datetime | str | None) -> datetime:
    if value is None:
        return datetime(1970, 1, 1, tzinfo=timezone.utc)
    if isinstance(value, datetime):
        return value if value.tzinfo else value.replace(tzinfo=timezone.utc)
    text = str(value).strip()
    if not text:
        return datetime(1970, 1, 1, tzinfo=timezone.utc)
    normalized = text[:-1] + "+00:00" if text.endswith("Z") else text
    parsed = datetime.fromisoformat(normalized)
    return parsed if parsed.tzinfo else parsed.replace(tzinfo=timezone.utc)


def _normalize_worktree_creation_kind(value: Any, *, default: str | None = None) -> str | None:
    text = normalize_optional_text(value)
    if text is None:
        return default
    lowered = text.lower()
    if lowered not in WORKTREE_CREATION_KINDS:
        raise ValueError(f"Unsupported worktree creation_kind: {text}")
    return lowered


def _normalize_worktree_cleanup_policy(value: Any, *, default: str | None = None) -> str | None:
    text = normalize_optional_text(value)
    if text is None:
        return default
    lowered = text.lower()
    if lowered not in WORKTREE_CLEANUP_POLICIES:
        raise ValueError(f"Unsupported worktree cleanup_policy: {text}")
    return lowered


def _default_cleanup_policy_for_creation_kind(creation_kind: str | None) -> str:
    normalized = _normalize_worktree_creation_kind(creation_kind, default=DEFAULT_WORKTREE_CREATION_KIND)
    assert normalized is not None
    return _DEFAULT_CLEANUP_POLICY_BY_CREATION_KIND.get(normalized, DEFAULT_WORKTREE_CLEANUP_POLICY)


def _normalize_older_than(value: str | None) -> tuple[timedelta, str]:
    text = normalize_optional_text(value) or DEFAULT_WORKTREE_CLEANUP_OLDER_THAN
    match = _WORKTREE_OLDER_THAN_RE.fullmatch(text)
    if match is None:
        raise ValueError("`--older-than` must look like `7d`, `12h`, or `30m`.")
    count = int(match.group("count"))
    unit = match.group("unit").lower()
    delta = {
        "d": timedelta(days=count),
        "h": timedelta(hours=count),
        "m": timedelta(minutes=count),
    }[unit]
    return delta, f"{count}{unit}"


def get_remote(ctx: RepoContext, name: Optional[str] = None) -> dict:
    if name is None:
        name = load_config(ctx).get("default_remote")
    if not name:
        raise KeyError("No remote configured. Run `ait remote add ... --default` first.")
    return local_control.get_remote(ctx, name)


def list_lines(ctx: RepoContext) -> list[dict]:
    return local_content.list_lines(ctx)


def current_line(ctx: RepoContext) -> str:
    if ctx.worktree_config_path is not None:
        cfg = _load_worktree_config(ctx)
        if cfg.get("current_line"):
            return cfg["current_line"]
    cfg = read_json(ctx.config_path, default={}) or {}
    if isinstance(cfg, dict) and cfg.get("current_line"):
        return cfg["current_line"]
    return local_control.get_meta(ctx, "current_line") or "main"


def create_line(ctx: RepoContext, name: str, from_snapshot: Optional[str] = None) -> dict:
    if from_snapshot is None:
        current = current_line(ctx)
        try:
            row = local_content.get_line(ctx, current)
            from_snapshot = row.get("head_snapshot_id")
        except KeyError:
            from_snapshot = None
    row = local_content.create_line(ctx, name, from_snapshot)
    local_control.record_event(
        ctx,
        "line.created",
        "line",
        name,
        {"line_name": name, "head_snapshot_id": from_snapshot},
    )
    return row


def archive_line(ctx: RepoContext, name: str) -> dict:
    default_line = local_control.get_meta(ctx, "default_line") or "main"
    current = current_line(ctx)
    if name == default_line:
        raise ValueError(f"Default line {name} cannot be archived")
    if name == current:
        raise ValueError(f"Current line {name} cannot be archived; switch to another line first")
    row = local_content.archive_line(ctx, name)
    local_control.record_event(
        ctx,
        "line.archived",
        "line",
        name,
        {"line_name": name, "status": row["status"], "archived_at": row.get("archived_at")},
    )
    return row


def _set_current_line(ctx: RepoContext, name: str) -> dict:
    row = local_content.get_line(ctx, name)
    if ctx.worktree_config_path is not None:
        from .store_worktree_state import _normalize_worktree_name
        from .store_worktree_cleanup import _update_worktree_registration

        cfg = _load_worktree_config(ctx)
        cfg["current_line"] = name
        cfg.setdefault("repo_root", str(ctx.repo_root))
        cfg.setdefault("workspace_root", str(ctx.root))
        _save_worktree_config(ctx, cfg)
        worktree_name = cfg.get("worktree_name")
        if worktree_name:
            _update_worktree_registration(
                ctx,
                _normalize_worktree_name(str(worktree_name)),
                line_name=name,
                path=str(ctx.root.resolve()),
                repo_root=str(ctx.repo_root),
            )
        return row
    local_control.set_meta(ctx, "current_line", name)
    cfg = load_config(ctx)
    cfg["current_line"] = name
    save_config(ctx, cfg)
    return row


def switch_line(ctx: RepoContext, name: str) -> dict:
    return _set_current_line(ctx, name)


def _matching_snapshot_session(
    ctx: RepoContext,
    *,
    task_id: str | None,
    change_id: str | None,
    worktree_name: str | None,
    line_name: str | None,
) -> dict[str, Any] | None:
    sessions = local_control.list_workflow_sessions(ctx)
    best_session: dict[str, Any] | None = None
    best_score: tuple[int, int, int, int] | None = None
    best_updated_at: str | None = None
    for session in sessions:
        status = str(session.get("status") or "").strip()
        if status not in {"active", "paused"}:
            continue
        score = (
            0 if worktree_name and str(session.get("worktree_name") or "").strip() == worktree_name else 1,
            0 if change_id and str(session.get("change_id") or "").strip() == change_id else 1,
            0 if task_id and str(session.get("task_id") or "").strip() == task_id else 1,
            0 if line_name and str(session.get("line_name") or "").strip() == line_name else 1,
        )
        if all(part == 1 for part in score):
            continue
        updated_at = str(session.get("updated_at") or "")
        if best_score is None or score < best_score or (score == best_score and updated_at > (best_updated_at or "")):
            best_session = session
            best_score = score
            best_updated_at = updated_at
    return best_session


def _snapshot_workflow_context(
    ctx: RepoContext,
    *,
    line_name: str,
) -> dict[str, Any] | None:
    task_id = None
    change_id = None
    worktree_name = None
    if ctx.is_worktree:
        from .store_worktrees import get_worktree

        try:
            worktree = get_worktree(ctx)
        except KeyError:
            worktree = None
        if isinstance(worktree, dict):
            task_id = normalize_optional_text(worktree.get("bound_task_id"))
            change_id = normalize_optional_text(worktree.get("bound_change_id"))
            worktree_name = normalize_optional_text(worktree.get("name")) or normalize_optional_text(
                worktree.get("worktree_name")
            )
    session = _matching_snapshot_session(
        ctx,
        task_id=task_id,
        change_id=change_id,
        worktree_name=worktree_name,
        line_name=line_name,
    )
    session_id = normalize_optional_text((session or {}).get("session_id"))
    checkpoint_id = normalize_optional_text((session or {}).get("head_checkpoint_id"))
    metadata = (session or {}).get("metadata") if isinstance((session or {}).get("metadata"), dict) else {}
    author_mode = normalize_optional_text(metadata.get("author_mode")) if isinstance(metadata, dict) else None
    model_name = normalize_optional_text((session or {}).get("model_name"))
    if not any((task_id, change_id, session_id, checkpoint_id, worktree_name, author_mode, model_name)):
        return None
    return {
        "task_id": task_id,
        "change_id": change_id,
        "session_id": session_id,
        "checkpoint_id": checkpoint_id,
        "worktree_name": worktree_name,
        "line_name": line_name,
        "author_mode": author_mode,
        "model_name": model_name,
    }


def create_snapshot(ctx: RepoContext, message: Optional[str], *, parent_snapshot_id: str | None = None) -> dict:
    repo_name = local_control.get_meta(ctx, "repo_name") or ctx.root.name
    line_name = current_line(ctx)
    row = local_content.create_snapshot(ctx, repo_name, line_name, message, parent_snapshot_id=parent_snapshot_id)
    workflow_context = _snapshot_workflow_context(ctx, line_name=line_name)
    if workflow_context is not None:
        local_control.record_workflow_snapshot_provenance(
            ctx,
            row["snapshot_id"],
            task_id=workflow_context.get("task_id"),
            change_id=workflow_context.get("change_id"),
            session_id=workflow_context.get("session_id"),
            checkpoint_id=workflow_context.get("checkpoint_id"),
            worktree_name=workflow_context.get("worktree_name"),
            line_name=workflow_context.get("line_name"),
            author_mode=workflow_context.get("author_mode"),
            model_name=workflow_context.get("model_name"),
            created_at=row.get("created_at"),
        )
    _set_worktree_materialized_snapshot(ctx, row["snapshot_id"])
    local_control.record_event(
        ctx,
        "snapshot.created",
        "snapshot",
        row["snapshot_id"],
        {
            "snapshot_id": row["snapshot_id"],
            "line_name": row["line_name"],
            "file_count": row["file_count"],
            "parent_snapshot_id": row["parent_snapshot_id"],
        },
    )
    return row


def workspace_status(
    ctx: RepoContext,
    *,
    snapshot_id: str | None = None,
    line_name: str | None = None,
) -> dict:
    if snapshot_id is not None and line_name is not None:
        raise ValueError("Choose either snapshot_id or line_name, not both.")

    current_line_name = current_line(ctx)
    baseline_source = "snapshot" if snapshot_id is not None else "line"
    baseline_line_name = None
    baseline_snapshot_id = snapshot_id
    if baseline_snapshot_id is None:
        baseline_line_name = line_name or current_line_name
        baseline_line_row = local_content.get_line(ctx, baseline_line_name)
        baseline_snapshot_id = baseline_line_row.get("head_snapshot_id")
        baseline_source = "current_line_head" if baseline_line_name == current_line_name else "line_head"

    delta = local_content.workspace_delta(ctx, baseline_snapshot_id)
    return {
        "repo_name": load_config(ctx).get("repo_name") or ctx.root.name,
        "workspace_root": str(ctx.root),
        "is_worktree": ctx.is_worktree,
        "worktree_name": _load_worktree_config(ctx).get("worktree_name") if ctx.is_worktree else None,
        "current_line": current_line_name,
        "baseline_source": baseline_source,
        "baseline_line_name": baseline_line_name,
        "baseline_snapshot_id": baseline_snapshot_id,
        "clean": delta["clean"],
        "changed_count": delta["changed_count"],
        "changed_paths": delta["changed_paths"],
        "modified_paths": delta["modified_paths"],
        "missing_paths": delta["missing_paths"],
        "untracked_paths": delta["untracked_paths"],
        "ignore_policy": delta["ignore_policy"],
        "phase_timings_ms": delta.get("phase_timings_ms"),
    }


def set_line_head(ctx: RepoContext, line_name: str, snapshot_id: Optional[str]) -> dict:
    try:
        previous_snapshot_id = normalize_optional_text(local_content.get_line(ctx, line_name).get("head_snapshot_id"))
    except KeyError:
        previous_snapshot_id = None
    row = local_content.set_line_head(ctx, line_name, snapshot_id)
    local_control.record_event(
        ctx,
        "line.moved",
        "line",
        line_name,
        {
            "line_name": line_name,
            "previous_head_snapshot_id": previous_snapshot_id,
            "head_snapshot_id": snapshot_id,
        },
    )
    return row
