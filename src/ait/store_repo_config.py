from __future__ import annotations

import os
from contextlib import contextmanager
from typing import Any, Callable, Iterator

from ait_protocol.common import (
    DEFAULT_ID_NAMESPACE_PREFIX,
    normalize_id_namespace_prefix,
    normalize_policy,
    parse_policy_yaml,
    policy_profile,
    policy_to_yaml,
    read_json,
    write_json,
)

from .repo_paths import RepoContext

try:  # pragma: no cover - platform-specific import
    import fcntl  # type: ignore[attr-defined]
except ImportError:  # pragma: no cover - Windows
    fcntl = None

try:  # pragma: no cover - platform-specific import
    import msvcrt  # type: ignore[attr-defined]
except ImportError:  # pragma: no cover - Unix
    msvcrt = None

WORKTREE_LOCAL_KEYS = {
    "current_line",
    "worktree_name",
    "repo_root",
    "workspace_root",
    "created_at",
    "materialized_snapshot_id",
    "tracked_task_id",
    "tracked_session_id",
    "tracked_session_scope",
    "tracked_session_remote",
}


def _repo_config_lock_path(ctx: RepoContext) -> Any:
    return ctx.workspace_dir / "locks" / "repo-config.lock"


def _acquire_repo_config_lock(handle) -> None:
    if fcntl is not None:
        fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
        return
    if msvcrt is not None:
        handle.seek(0, os.SEEK_END)
        if handle.tell() == 0:
            handle.write(b"\0")
            handle.flush()
        handle.seek(0)
        msvcrt.locking(handle.fileno(), msvcrt.LK_LOCK, 1)
        return
    raise RuntimeError("Repo config locking is not supported on this platform.")


def _release_repo_config_lock(handle) -> None:
    if fcntl is not None:
        fcntl.flock(handle.fileno(), fcntl.LOCK_UN)
        return
    if msvcrt is not None:
        handle.seek(0)
        msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
        return


@contextmanager
def _repo_config_lock(ctx: RepoContext) -> Iterator[None]:
    lock_path = _repo_config_lock_path(ctx)
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+b") as handle:
        _acquire_repo_config_lock(handle)
        try:
            yield
        finally:
            _release_repo_config_lock(handle)


def _configured_id_namespace_prefix(config: dict[str, object]) -> str | None:
    if "id_namespace_prefix" not in config:
        return None
    return normalize_id_namespace_prefix(config.get("id_namespace_prefix"), default=DEFAULT_ID_NAMESPACE_PREFIX)


def effective_id_namespace_prefix(ctx: RepoContext) -> str:
    config = load_config(ctx)
    configured = _configured_id_namespace_prefix(config)
    if configured is not None:
        return configured
    return DEFAULT_ID_NAMESPACE_PREFIX


def load_config(ctx: RepoContext) -> dict:
    config = read_json(ctx.config_path, default={}) or {}
    if not isinstance(config, dict):
        config = {}
    if ctx.worktree_config_path is None or not ctx.worktree_config_path.exists():
        return config
    overlay = read_json(ctx.worktree_config_path, default={}) or {}
    if not isinstance(overlay, dict):
        return config
    merged = dict(config)
    for key, value in overlay.items():
        if value is not None:
            merged[key] = value
    return merged


def save_config(ctx: RepoContext, config: dict) -> None:
    if ctx.worktree_config_path is None:
        write_json(ctx.config_path, config)
        return
    shared = dict(config)
    for key in WORKTREE_LOCAL_KEYS:
        shared.pop(key, None)
    existing_shared = read_json(ctx.config_path, default={}) or {}
    if isinstance(existing_shared, dict) and "current_line" in existing_shared:
        shared["current_line"] = existing_shared["current_line"]
    write_json(ctx.config_path, shared)


def update_config(ctx: RepoContext, updater: Callable[[dict], None]) -> dict:
    with _repo_config_lock(ctx):
        config = load_config(ctx)
        updater(config)
        save_config(ctx, config)
        return config


def _load_worktree_config(ctx: RepoContext) -> dict:
    if ctx.worktree_config_path is None:
        return {}
    payload = read_json(ctx.worktree_config_path, default={}) or {}
    return payload if isinstance(payload, dict) else {}


def _save_worktree_config(ctx: RepoContext, config: dict) -> None:
    if ctx.worktree_config_path is None:
        raise ValueError("Worktree config requested outside a worktree context.")
    write_json(ctx.worktree_config_path, config)


def _worktree_materialized_snapshot_id(ctx: RepoContext) -> str | None:
    cfg = _load_worktree_config(ctx)
    value = cfg.get("materialized_snapshot_id")
    return str(value) if value else None


def _set_worktree_materialized_snapshot(ctx: RepoContext, snapshot_id: str | None) -> None:
    if ctx.worktree_config_path is None:
        return
    cfg = _load_worktree_config(ctx)
    cfg["materialized_snapshot_id"] = snapshot_id
    cfg.setdefault("repo_root", str(ctx.repo_root))
    cfg.setdefault("workspace_root", str(ctx.root))
    _save_worktree_config(ctx, cfg)


def load_policy(ctx: RepoContext) -> dict:
    if not ctx.policy_path.exists():
        return policy_profile("prototype")
    return parse_policy_yaml(ctx.policy_path.read_text(encoding="utf-8"))


def save_policy(ctx: RepoContext, policy: dict) -> dict:
    normalized = normalize_policy(policy)
    ctx.policy_path.write_text(policy_to_yaml(normalized), encoding="utf-8")
    return normalized
