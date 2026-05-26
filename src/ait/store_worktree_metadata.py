from __future__ import annotations

from pathlib import Path
from typing import Any

from ait_protocol.common import normalize_optional_text, read_json, write_json, utc_now

from .repo_paths import RepoContext, WORKTREE_CONFIG_NAME
from .store_repo_config import load_config
from .store_worktree_state import _normalize_worktree_name
from .task_worktree_layout import (
    DEFAULT_TASK_WORKTREE_ALIAS_ROOT,
    normalize_task_worktree_main_seed_ram_max_bytes,
    normalize_task_worktree_memory_root,
)

_WORKTREE_STATUS_CACHE_KEY = "workspace_status_cache"
_INTERNAL_WORKTREE_ROLE_MAIN_SEED = "main_seed_mirror"


def _default_line_name(ctx: RepoContext) -> str:
    return normalize_optional_text(load_config(ctx).get("default_line")) or "main"


def _configured_task_worktree_policy(ctx: RepoContext) -> dict[str, Any]:
    cfg = load_config(ctx)
    raw = cfg.get("task_worktree")
    if not isinstance(raw, dict):
        raw = {}
    return {
        "ephemeral_root": normalize_optional_text(raw.get("ephemeral_root")),
        "alias_root": normalize_optional_text(raw.get("alias_root")) or DEFAULT_TASK_WORKTREE_ALIAS_ROOT,
        "memory_root": normalize_task_worktree_memory_root(raw.get("memory_root")),
        "main_seed_ram_max_bytes": normalize_task_worktree_main_seed_ram_max_bytes(
            raw.get("main_seed_ram_max_bytes")
        ),
    }


def _main_seed_worktree_name(line_name: str) -> str:
    normalized = _normalize_worktree_name(f"{line_name}-seed")
    return normalized or "main-seed"


def _main_seed_config_payload(
    *,
    seed_name: str,
    line_name: str,
    created_at: str,
    seed_snapshot_id: str | None,
    root_source: str | None,
) -> dict[str, Any]:
    return {
        "worktree_name": seed_name,
        "current_line": line_name,
        "created_at": created_at,
        "internal_role": _INTERNAL_WORKTREE_ROLE_MAIN_SEED,
        "seed_line_name": line_name,
        "seed_snapshot_id": seed_snapshot_id,
        "seed_refreshed_at": created_at,
        "root_source": normalize_optional_text(root_source),
    }


def _worktree_metadata_path(ctx: RepoContext, name: str) -> Path:
    return ctx.worktree_registry_dir / f"{name}.json"


def _load_worktree_metadata(ctx: RepoContext, name: str) -> dict:
    payload = read_json(_worktree_metadata_path(ctx, name), default={}) or {}
    if not isinstance(payload, dict) or not payload:
        raise KeyError(f"Unknown worktree: {name}")
    return payload


def _save_worktree_metadata(ctx: RepoContext, name: str, payload: dict) -> None:
    write_json(_worktree_metadata_path(ctx, name), payload)


def _main_seed_local_config(seed_path: Path) -> dict[str, Any]:
    payload = read_json(seed_path / WORKTREE_CONFIG_NAME, default={}) or {}
    return payload if isinstance(payload, dict) else {}


def _main_seed_state(seed_path: Path) -> dict[str, Any]:
    payload = _main_seed_local_config(seed_path) if seed_path.is_dir() else {}
    return {
        "path": str(seed_path),
        "exists": seed_path.is_dir(),
        "internal_role": normalize_optional_text(payload.get("internal_role")),
        "seed_line_name": normalize_optional_text(payload.get("seed_line_name")),
        "seed_snapshot_id": normalize_optional_text(payload.get("seed_snapshot_id"))
        or normalize_optional_text(payload.get("materialized_snapshot_id")),
        "worktree_name": normalize_optional_text(payload.get("worktree_name")),
        "current_line": normalize_optional_text(payload.get("current_line")),
        "root_source": normalize_optional_text(payload.get("root_source")),
        "seed_refreshed_at": normalize_optional_text(payload.get("seed_refreshed_at")),
    }


def _is_seed_state_aligned(
    seed_state: dict[str, Any],
    *,
    line_name: str,
    snapshot_id: str | None,
) -> bool:
    return (
        bool(seed_state.get("exists"))
        and seed_state.get("internal_role") == _INTERNAL_WORKTREE_ROLE_MAIN_SEED
        and seed_state.get("seed_line_name") == line_name
        and seed_state.get("seed_snapshot_id") == normalize_optional_text(snapshot_id)
    )


def _worktree_local_config_payload(worktree_path: Path | None) -> dict[str, Any]:
    if worktree_path is None:
        return {}
    payload = read_json(worktree_path / WORKTREE_CONFIG_NAME, default={}) or {}
    return payload if isinstance(payload, dict) else {}


def _worktree_status_cache(metadata: dict[str, Any]) -> dict[str, Any] | None:
    payload = metadata.get(_WORKTREE_STATUS_CACHE_KEY)
    if not isinstance(payload, dict):
        return None
    workspace_status_value = normalize_optional_text(payload.get("workspace_status")) or "unknown"
    if workspace_status_value not in {"clean", "dirty", "missing", "detached", "unknown"}:
        workspace_status_value = "unknown"
    clean = payload.get("clean")
    if not isinstance(clean, bool):
        clean = True if workspace_status_value == "clean" else False if workspace_status_value == "dirty" else None
    changed_count = payload.get("changed_count")
    if changed_count is not None:
        changed_count = int(changed_count)

    def _normalize_paths(value: Any) -> list[str]:
        if not isinstance(value, list):
            return []
        return [str(item) for item in value if str(item).strip()]

    return {
        "workspace_status": workspace_status_value,
        "clean": clean,
        "changed_count": changed_count,
        "modified_paths": _normalize_paths(payload.get("modified_paths")),
        "missing_paths": _normalize_paths(payload.get("missing_paths")),
        "untracked_paths": _normalize_paths(payload.get("untracked_paths")),
        "current_line": normalize_optional_text(payload.get("current_line")),
        "head_snapshot_id": normalize_optional_text(payload.get("head_snapshot_id")),
        "status_checked_at": normalize_optional_text(payload.get("status_checked_at")),
    }


def _build_worktree_status_cache_payload(
    *,
    workspace_status_value: str,
    clean: bool | None,
    changed_count: int | None,
    modified_paths: list[str],
    missing_paths: list[str],
    untracked_paths: list[str],
    current_line_name: str | None,
    head_snapshot_id: str | None,
    status_checked_at: str | None,
) -> dict[str, Any]:
    return {
        "workspace_status": workspace_status_value,
        "clean": clean,
        "changed_count": changed_count,
        "modified_paths": list(modified_paths),
        "missing_paths": list(missing_paths),
        "untracked_paths": list(untracked_paths),
        "current_line": current_line_name,
        "head_snapshot_id": head_snapshot_id,
        "status_checked_at": status_checked_at or utc_now(),
    }
