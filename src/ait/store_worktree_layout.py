from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from ait_protocol.common import normalize_optional_text, utc_now, write_json

from . import local_content, local_control
from .repo_paths import APP_DIR, RepoContext, WORKTREE_CONFIG_NAME
from .store_repo_config import _load_worktree_config, _set_worktree_materialized_snapshot
from .store_worktree_filesystem import (
    _create_directory_link,
    _make_tree_readonly,
    _path_exists_or_directory_link,
    _remove_tree_force,
)
from .store_worktree_metadata import (
    _configured_task_worktree_policy,
    _default_line_name,
    _is_seed_state_aligned,
    _main_seed_config_payload,
    _main_seed_state,
    _main_seed_worktree_name,
)
from .store_worktree_state import _repo_worktree_ctx
from .task_worktree_layout import (
    resolve_main_seed_mirror_location,
    resolve_managed_worktree_location,
)


def _prune_empty_worktree_dirs(root: Path, path: Path) -> None:
    cur = path.parent
    while cur != root and cur.exists():
        try:
            cur.rmdir()
        except OSError:
            break
        cur = cur.parent


def _link_shared_runtime_dir(repo_root: Path, target_path: Path, dirname: str) -> None:
    source_path = (repo_root / dirname).resolve()
    link_path = target_path / dirname
    if _path_exists_or_directory_link(link_path) or not source_path.is_dir():
        return
    try:
        _create_directory_link(link_path, source_path)
    except OSError:
        return


def _ensure_worktree_runtime_layout(repo_root: Path, target_path: Path) -> None:
    if target_path.is_dir():
        _link_shared_runtime_dir(repo_root, target_path, ".venv")


def _materialize_worktree_alias(target_path: Path, alias_path: Path) -> None:
    if alias_path == target_path:
        raise ValueError("Worktree alias path must differ from the canonical worktree path.")
    if _path_exists_or_directory_link(alias_path):
        raise ValueError(f"Worktree alias path is already in use: {alias_path}")
    alias_path.parent.mkdir(parents=True, exist_ok=True)
    _create_directory_link(alias_path, target_path)


def _materialize_worktree_runtime_layout(
    ctx: RepoContext,
    *,
    worktree_name: str,
    target_path: Path,
    line_name: str,
    created_at: str,
    extra_worktree_config: dict[str, Any] | None = None,
) -> RepoContext:
    shared_ait_dir = ctx.ait_dir.resolve()
    target_path.mkdir(parents=True, exist_ok=True)
    ait_link = target_path / APP_DIR
    if _path_exists_or_directory_link(ait_link):
        raise ValueError(f"Worktree path already contains {APP_DIR}: {target_path}")
    _create_directory_link(ait_link, shared_ait_dir)
    _ensure_worktree_runtime_layout(ctx.repo_root, target_path)
    config_payload = {
        "worktree_name": worktree_name,
        "current_line": line_name,
        "repo_root": str(ctx.repo_root),
        "workspace_root": str(target_path),
        "created_at": created_at,
    }
    if extra_worktree_config:
        config_payload.update(dict(extra_worktree_config))
    write_json(target_path / WORKTREE_CONFIG_NAME, config_payload)
    return RepoContext.discover(target_path)


def _main_seed_location(ctx: RepoContext, *, line_name: str) -> dict[str, Any] | None:
    repo_ctx = _repo_worktree_ctx(ctx)
    policy = _configured_task_worktree_policy(repo_ctx)
    return resolve_main_seed_mirror_location(
        repo_ctx,
        seed_name=_main_seed_worktree_name(line_name),
        root_mode=policy["root_mode"],
        ephemeral_root=policy["ephemeral_root"],
    )


def _refresh_main_seed_mirror(
    ctx: RepoContext,
    *,
    line_name: str,
    target_snapshot_id: str,
    seed_location: dict[str, Any],
) -> dict[str, Any]:
    repo_ctx = _repo_worktree_ctx(ctx)
    seed_name = _main_seed_worktree_name(line_name)
    seed_path = Path(seed_location["target_path"]).resolve()
    refreshed_at = utc_now()
    staging_name = f".{seed_name}.tmp-{os.getpid()}-{refreshed_at.replace(':', '').replace('-', '')}"
    staging_path = (seed_path.parent / staging_name).resolve()
    try:
        if _path_exists_or_directory_link(staging_path):
            _remove_tree_force(staging_path)
        staging_path.parent.mkdir(parents=True, exist_ok=True)
        seed_ctx = _materialize_worktree_runtime_layout(
            repo_ctx,
            worktree_name=seed_name,
            target_path=staging_path,
            line_name=line_name,
            created_at=refreshed_at,
            extra_worktree_config=_main_seed_config_payload(
                seed_name=seed_name,
                line_name=line_name,
                created_at=refreshed_at,
                seed_snapshot_id=target_snapshot_id,
                root_mode=seed_location.get("root_mode"),
                root_source=seed_location.get("root_source"),
            ),
        )
        local_content.restore_workspace(
            seed_ctx,
            target_snapshot_id,
            baseline_snapshot_id=None,
            force=True,
            dry_run=False,
        )
        _set_worktree_materialized_snapshot(seed_ctx, target_snapshot_id)
        seed_cfg = _load_worktree_config(seed_ctx)
        seed_cfg["seed_snapshot_id"] = target_snapshot_id
        seed_cfg["seed_refreshed_at"] = refreshed_at
        write_json(staging_path / WORKTREE_CONFIG_NAME, seed_cfg)
        _make_tree_readonly(staging_path)
        if _path_exists_or_directory_link(seed_path):
            _remove_tree_force(seed_path)
        staging_path.rename(seed_path)
        local_control.record_event(
            repo_ctx,
            "worktree.main_seed_refreshed",
            "worktree",
            seed_name,
            {
                "name": seed_name,
                "path": str(seed_path),
                "line_name": line_name,
                "seed_snapshot_id": target_snapshot_id,
                "root_source": seed_location.get("root_source"),
            },
        )
        return {
            "status": "refreshed",
            "name": seed_name,
            "path": str(seed_path),
            "line_name": line_name,
            "seed_snapshot_id": target_snapshot_id,
            "root_mode": seed_location.get("root_mode"),
            "root_source": seed_location.get("root_source"),
            "seed_refreshed_at": refreshed_at,
        }
    except Exception as exc:
        if _path_exists_or_directory_link(staging_path):
            _remove_tree_force(staging_path)
        return {
            "status": "failed",
            "name": seed_name,
            "path": str(seed_path),
            "line_name": line_name,
            "seed_snapshot_id": target_snapshot_id,
            "root_mode": seed_location.get("root_mode"),
            "root_source": seed_location.get("root_source"),
            "error": str(exc),
        }


def ensure_main_seed_mirror(
    ctx: RepoContext,
    *,
    force_refresh: bool = False,
    line_name: str | None = None,
) -> dict[str, Any]:
    repo_ctx = _repo_worktree_ctx(ctx)
    default_line = _default_line_name(repo_ctx)
    effective_line_name = normalize_optional_text(line_name) or default_line
    if effective_line_name != default_line:
        return {
            "status": "skipped",
            "reason": "target_not_default_line",
            "default_line": default_line,
            "line_name": effective_line_name,
        }
    try:
        line_row = local_content.get_line(repo_ctx, effective_line_name)
    except KeyError:
        return {
            "status": "skipped",
            "reason": "line_missing",
            "default_line": default_line,
            "line_name": effective_line_name,
        }
    target_snapshot_id = normalize_optional_text(line_row.get("head_snapshot_id"))
    if target_snapshot_id is None:
        return {
            "status": "skipped",
            "reason": "line_head_missing",
            "default_line": default_line,
            "line_name": effective_line_name,
        }
    seed_location = _main_seed_location(repo_ctx, line_name=effective_line_name)
    if seed_location is None:
        return {
            "status": "skipped",
            "reason": "managed_ephemeral_root_unavailable",
            "default_line": default_line,
            "line_name": effective_line_name,
            "seed_snapshot_id": target_snapshot_id,
        }
    seed_path = Path(seed_location["target_path"]).resolve()
    seed_state = _main_seed_state(seed_path)
    if not force_refresh and _is_seed_state_aligned(seed_state, line_name=effective_line_name, snapshot_id=target_snapshot_id):
        return {
            "status": "aligned",
            "name": seed_state.get("worktree_name") or _main_seed_worktree_name(effective_line_name),
            "path": str(seed_path),
            "line_name": effective_line_name,
            "seed_snapshot_id": target_snapshot_id,
            "root_mode": seed_location.get("root_mode"),
            "root_source": seed_location.get("root_source"),
            "seed_refreshed_at": seed_state.get("seed_refreshed_at"),
        }
    refresh_result = _refresh_main_seed_mirror(
        repo_ctx,
        line_name=effective_line_name,
        target_snapshot_id=target_snapshot_id,
        seed_location=seed_location,
    )
    refresh_result.setdefault("default_line", default_line)
    return refresh_result


def _managed_worktree_location_from_defaults(ctx: RepoContext, *, worktree_name: str) -> dict[str, Any]:
    policy = _configured_task_worktree_policy(ctx)
    return resolve_managed_worktree_location(
        ctx,
        worktree_name=worktree_name,
        root_mode=policy["root_mode"],
        ephemeral_root=policy["ephemeral_root"],
        alias_root=policy["alias_root"],
    )
