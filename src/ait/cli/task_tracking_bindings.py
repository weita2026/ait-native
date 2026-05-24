from __future__ import annotations

from typing import Any

import typer

from ..repo_paths import RepoContext
from ..store import (
    load_config,
    save_config,
    touch_worktree_usage as local_touch_worktree_usage,
)
from .runtime_defaults import _normalize_text_value
from .workflow_mode_config import _normalize_task_tracking_mode


TRACKED_SESSION_SCOPES = frozenset({"local", "remote"})
TRACKED_SESSION_CONFIG_KEYS = (
    "tracked_task_id",
    "tracked_session_id",
    "tracked_session_scope",
    "tracked_session_remote",
)


def _task_tracking_mode(ctx: RepoContext | None) -> str | None:
    if ctx is None:
        return None
    try:
        return _normalize_task_tracking_mode(load_config(ctx).get("task_tracking"))
    except typer.BadParameter:
        return None


def _task_tracking_enabled(ctx: RepoContext | None) -> bool:
    return _task_tracking_mode(ctx) == "on"


def _task_tracking_hard_disabled(ctx: RepoContext | None) -> bool:
    return _task_tracking_mode(ctx) == "off"


def _tracked_session_binding(ctx: RepoContext) -> dict[str, Any] | None:
    cfg = load_config(ctx)
    session_id = _normalize_text_value(cfg.get("tracked_session_id"))
    if session_id is None:
        return None
    scope = _normalize_text_value(cfg.get("tracked_session_scope")) or "remote"
    if scope not in TRACKED_SESSION_SCOPES:
        return None
    return {
        "task_id": _normalize_text_value(cfg.get("tracked_task_id")),
        "session_id": session_id,
        "scope": scope,
        "remote_name": _normalize_text_value(cfg.get("tracked_session_remote")),
    }


def _set_tracked_session_binding(
    ctx: RepoContext,
    *,
    task_id: str,
    session_id: str,
    scope: str,
    remote_name: str | None = None,
) -> dict[str, Any]:
    if scope not in TRACKED_SESSION_SCOPES:
        raise ValueError(f"Unsupported tracked session scope: {scope}")
    cfg = load_config(ctx)
    cfg["tracked_task_id"] = task_id
    cfg["tracked_session_id"] = session_id
    cfg["tracked_session_scope"] = scope
    if scope == "remote":
        cfg["tracked_session_remote"] = _normalize_text_value(remote_name)
    else:
        cfg.pop("tracked_session_remote", None)
    save_config(ctx, cfg)
    binding = _tracked_session_binding(ctx)
    assert binding is not None
    return binding


def _clear_tracked_session_binding(ctx: RepoContext) -> None:
    cfg = load_config(ctx)
    changed = False
    for key in TRACKED_SESSION_CONFIG_KEYS:
        if key in cfg:
            changed = True
            cfg.pop(key, None)
    if changed:
        save_config(ctx, cfg)


def _clear_tracked_session_binding_if_matches(
    ctx: RepoContext,
    *,
    task_id: str | None = None,
    session_id: str | None = None,
) -> None:
    binding = _tracked_session_binding(ctx)
    if binding is None:
        return
    if task_id is not None and binding.get("task_id") != task_id:
        return
    if session_id is not None and binding.get("session_id") != session_id:
        return
    _clear_tracked_session_binding(ctx)


def _task_worktree_repo_ctx(ctx: RepoContext) -> RepoContext:
    return RepoContext.discover(ctx.repo_root) if ctx.is_worktree else ctx


def _touch_worktree_usage_safely(ctx: RepoContext, name: str | None = None) -> dict[str, Any] | None:
    try:
        return local_touch_worktree_usage(ctx, name=name)
    except ValueError:
        return None


def _default_line_name(ctx: RepoContext) -> str:
    return _normalize_text_value(load_config(ctx).get("default_line")) or "main"
