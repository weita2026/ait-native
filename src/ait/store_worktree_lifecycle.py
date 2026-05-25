from __future__ import annotations

from pathlib import Path
from typing import Any

from ait_protocol.common import normalize_optional_text, read_json, utc_now, write_json

from . import local_content, local_control
from .repo_paths import APP_DIR, RepoContext, WORKTREE_CONFIG_NAME
from .store_repo_config import _set_worktree_materialized_snapshot
from .store_worktree_bindings import (
    guard_worktree_binding_task_lineage as _guard_worktree_binding_task_lineage,
)
from .store_worktree_cleanup import (
    _touch_worktree_metadata,
    _update_worktree_registration,
)
from .store_worktree_filesystem import (
    _copy_seed_tree,
    _make_tree_writable,
    _path_exists_or_directory_link,
    _remove_tree_force,
)
from .store_worktree_layout import (
    _managed_worktree_location_from_defaults,
    _materialize_worktree_alias,
    _materialize_worktree_runtime_layout,
    ensure_main_seed_mirror,
)
from .store_worktree_metadata import (
    _WORKTREE_STATUS_CACHE_KEY,
    _build_worktree_status_cache_payload,
    _default_line_name,
    _load_worktree_metadata,
    _worktree_metadata_path,
)
from .store_worktree_runtime import (
    DEFAULT_WORKTREE_CREATION_KIND,
    _default_cleanup_policy_for_creation_kind,
    _normalize_worktree_cleanup_policy,
    _normalize_worktree_creation_kind,
    create_line,
    current_line,
    switch_line,
)
from .store_worktree_state import (
    _normalize_worktree_name,
    _repo_worktree_ctx,
    _worktree_metadata_with_defaults,
)
from .store_worktree_views import (
    _maybe_discover_worktree,
    _resolve_worktree_name,
    get_worktree,
    list_worktrees,
    worktree_doctor,
)

__all__ = [
    'add_worktree',
    'bind_worktree',
    'promote_worktree',
    'prune_stale_worktrees',
]

_MAIN_SEED_COPY_EXCLUDE_NAMES = frozenset({APP_DIR, WORKTREE_CONFIG_NAME, '.venv'})


def _seed_fast_path_candidate(
    ctx: RepoContext,
    *,
    target_snapshot_id: str | None,
    root_source: str | None,
) -> dict[str, Any] | None:
    repo_ctx = _repo_worktree_ctx(ctx)
    if normalize_optional_text(target_snapshot_id) is None:
        return None
    resolved_root_source = normalize_optional_text(root_source)
    if resolved_root_source in {None, 'repo_internal_fallback'}:
        return None
    default_line = _default_line_name(repo_ctx)
    try:
        default_line_row = local_content.get_line(repo_ctx, default_line)
    except KeyError:
        return None
    default_snapshot_id = normalize_optional_text(default_line_row.get('head_snapshot_id'))
    if target_snapshot_id != default_snapshot_id:
        return None
    main_seed = ensure_main_seed_mirror(repo_ctx, line_name=default_line)
    return {
        'default_line': default_line,
        'default_snapshot_id': default_snapshot_id,
        'seed': main_seed,
    }


def add_worktree(
    ctx: RepoContext,
    name: str,
    *,
    line_name: str | None = None,
    path: str | None = None,
    alias_path: str | None = None,
    creation_kind: str | None = None,
    cleanup_policy: str | None = None,
    root_source: str | None = None,
) -> dict[str, Any]:
    ctx.worktree_registry_dir.mkdir(parents=True, exist_ok=True)
    ctx.task_worktree_dir.mkdir(parents=True, exist_ok=True)
    worktree_name = _normalize_worktree_name(name)
    effective_line_name = line_name or current_line(ctx)
    line_row = local_content.get_line(ctx, effective_line_name)
    target_snapshot_id = normalize_optional_text(line_row.get('head_snapshot_id'))
    managed_location = _managed_worktree_location_from_defaults(ctx, worktree_name=worktree_name) if path is None else None
    target_path = (Path(path).expanduser() if path else Path(str(managed_location['target_path']))).resolve()
    resolved_alias_path = (
        Path(alias_path).expanduser().resolve()
        if alias_path
        else (
            Path(str(managed_location['alias_path'])).resolve()
            if managed_location is not None and managed_location.get('alias_path') is not None
            else None
        )
    )
    resolved_root_source = normalize_optional_text(root_source) or (
        normalize_optional_text(managed_location.get('root_source')) if managed_location is not None else None
    )
    metadata_path = _worktree_metadata_path(ctx, worktree_name)
    if metadata_path.exists():
        raise ValueError(f'Worktree already exists: {worktree_name}')
    for registered in sorted(ctx.worktree_registry_dir.glob('*.json')):
        payload = read_json(registered, default={}) or {}
        if not isinstance(payload, dict) or not payload.get('path'):
            continue
        registered_path = Path(payload['path']).expanduser().resolve()
        if registered_path == target_path:
            raise ValueError(f"Worktree path is already registered to {payload.get('name')}: {registered_path}")
        registered_alias_path = normalize_optional_text(payload.get('alias_path'))
        if registered_alias_path is not None and resolved_alias_path is not None:
            if Path(registered_alias_path).expanduser().resolve() == resolved_alias_path:
                raise ValueError(
                    f"Worktree alias path is already registered to {payload.get('name')}: {resolved_alias_path}"
                )
    shared_ait_dir = ctx.ait_dir.resolve()
    if target_path.exists():
        if not target_path.is_dir():
            raise ValueError(f'Worktree path exists and is not a directory: {target_path}')
        if any(target_path.iterdir()):
            raise ValueError(f'Worktree path must be empty: {target_path}')
    workspace_root = ctx.root.resolve()
    try:
        target_path.relative_to(workspace_root)
    except ValueError:
        pass
    else:
        try:
            target_path.relative_to(shared_ait_dir)
        except ValueError as exc:
            raise ValueError(
                'Worktree path cannot be nested inside the active workspace unless it lives under the shared .ait directory.'
            ) from exc
    if resolved_alias_path is not None:
        if resolved_alias_path == target_path:
            raise ValueError('Worktree alias path must differ from the canonical worktree path.')
        if _path_exists_or_directory_link(resolved_alias_path):
            raise ValueError(f'Worktree alias path is already in use: {resolved_alias_path}')

    created_at = utc_now()
    main_seed: dict[str, Any] | None = None
    materialization_source = 'snapshot_restore'
    copy_strategy: str | None = None
    seed_candidate = _seed_fast_path_candidate(
        ctx,
        target_snapshot_id=target_snapshot_id,
        root_source=resolved_root_source,
    )
    worktree_ctx: RepoContext | None = None
    if seed_candidate is not None:
        main_seed = dict(seed_candidate['seed'])
        if str(main_seed.get('status') or '').strip() in {'aligned', 'refreshed'}:
            try:
                copy_strategy = _copy_seed_tree(
                    Path(str(main_seed['path'])).resolve(),
                    target_path,
                    exclude_names=_MAIN_SEED_COPY_EXCLUDE_NAMES,
                )
                _make_tree_writable(target_path)
                worktree_ctx = _materialize_worktree_runtime_layout(
                    ctx,
                    worktree_name=worktree_name,
                    target_path=target_path,
                    line_name=effective_line_name,
                    created_at=created_at,
                )
                materialization_source = 'main_seed_mirror'
            except Exception as exc:
                if _path_exists_or_directory_link(target_path):
                    _remove_tree_force(target_path)
                main_seed = {
                    **main_seed,
                    'status': 'failed',
                    'fallback_used': True,
                    'fallback_reason': str(exc),
                }
    if worktree_ctx is None:
        worktree_ctx = _materialize_worktree_runtime_layout(
            ctx,
            worktree_name=worktree_name,
            target_path=target_path,
            line_name=effective_line_name,
            created_at=created_at,
        )
        if target_snapshot_id is not None:
            local_content.restore_workspace(
                worktree_ctx,
                target_snapshot_id,
                baseline_snapshot_id=None,
                force=True,
                dry_run=False,
            )
    _set_worktree_materialized_snapshot(worktree_ctx, target_snapshot_id)
    if resolved_alias_path is not None:
        _materialize_worktree_alias(target_path, resolved_alias_path)

    resolved_creation_kind = _normalize_worktree_creation_kind(
        creation_kind,
        default=DEFAULT_WORKTREE_CREATION_KIND,
    )
    assert resolved_creation_kind is not None
    resolved_cleanup_policy = _normalize_worktree_cleanup_policy(
        cleanup_policy,
        default=_default_cleanup_policy_for_creation_kind(resolved_creation_kind),
    )
    assert resolved_cleanup_policy is not None
    metadata = {
        'name': worktree_name,
        'path': str(target_path),
        'alias_path': str(resolved_alias_path) if resolved_alias_path is not None else None,
        'line_name': effective_line_name,
        'repo_root': str(ctx.repo_root),
        'created_at': created_at,
        'creation_kind': resolved_creation_kind,
        'cleanup_policy': resolved_cleanup_policy,
        'root_source': resolved_root_source,
        'last_used_at': created_at,
        _WORKTREE_STATUS_CACHE_KEY: _build_worktree_status_cache_payload(
            workspace_status_value='clean',
            clean=True,
            changed_count=0,
            modified_paths=[],
            missing_paths=[],
            untracked_paths=[],
            current_line_name=effective_line_name,
            head_snapshot_id=normalize_optional_text(target_snapshot_id),
            status_checked_at=created_at,
        ),
    }
    write_json(metadata_path, metadata)
    local_control.record_event(
        ctx,
        'worktree.created',
        'worktree',
        worktree_name,
        {
            'name': worktree_name,
            'path': str(target_path),
            'alias_path': str(resolved_alias_path) if resolved_alias_path is not None else None,
            'line_name': effective_line_name,
            'materialization_source': materialization_source,
        },
    )
    return {
        **get_worktree(ctx, worktree_name),
        'line_name': effective_line_name,
        'head_snapshot_id': target_snapshot_id,
        'repo_root': str(ctx.repo_root),
        'created_at': created_at,
        'materialization_source': materialization_source,
        'copy_strategy': copy_strategy,
        'main_seed': main_seed,
    }


def prune_stale_worktrees(ctx: RepoContext, *, dry_run: bool = False) -> dict[str, Any]:
    report = worktree_doctor(ctx)
    pruned_rows: list[dict[str, Any]] = []
    for row in report['stale_rows']:
        name = row.get('name')
        if not name:
            continue
        metadata_path = _worktree_metadata_path(ctx, _normalize_worktree_name(str(name)))
        pruned_rows.append(
            {
                'name': row.get('name'),
                'path': row.get('path'),
                'workspace_status': row.get('workspace_status'),
                'deleted_metadata': metadata_path.exists(),
            }
        )
        if dry_run:
            continue
        if metadata_path.exists():
            metadata_path.unlink()
        local_control.record_event(
            ctx,
            'worktree.pruned',
            'worktree',
            str(name),
            {
                'name': row.get('name'),
                'path': row.get('path'),
                'workspace_status': row.get('workspace_status'),
            },
        )
    remaining_rows = report['rows'] if dry_run else list_worktrees(ctx)
    return {
        'dry_run': dry_run,
        'stale_count_before': report['stale_count'],
        'pruned_count': len(pruned_rows),
        'pruned_rows': pruned_rows,
        'remaining_count': len(remaining_rows),
        'remaining_rows': remaining_rows,
    }


def promote_worktree(
    ctx: RepoContext,
    name: str | None = None,
    *,
    line_name: str,
) -> dict[str, Any]:
    worktree_name = _resolve_worktree_name(ctx, name)
    payload = _load_worktree_metadata(ctx, worktree_name)
    if str(_worktree_metadata_with_defaults(payload).get('rebase_state') or 'idle') == 'conflicted':
        raise ValueError(
            f'Worktree {worktree_name} is in a conflicted rebase. Use `ait worktree rebase --continue` or `--abort`.'
        )
    worktree_path = Path(payload['path']).expanduser().resolve()
    worktree_ctx = _maybe_discover_worktree(worktree_path)
    if worktree_ctx is None:
        raise ValueError(f'Worktree is missing or detached: {worktree_name}')

    previous_line_name = current_line(worktree_ctx)
    if previous_line_name == line_name:
        raise ValueError(f'Worktree {worktree_name} already uses line {line_name}.')
    try:
        local_content.get_line(worktree_ctx, line_name)
    except KeyError:
        pass
    else:
        raise ValueError(f'Line already exists: {line_name}')

    previous_line_row = local_content.get_line(worktree_ctx, previous_line_name)
    previous_head_snapshot_id = previous_line_row.get('head_snapshot_id')
    created_line = create_line(worktree_ctx, line_name, previous_head_snapshot_id)
    switch_line(worktree_ctx, line_name)

    local_control.record_event(
        ctx,
        'worktree.promoted',
        'worktree',
        worktree_name,
        {
            'name': worktree_name,
            'path': str(worktree_path),
            'previous_line_name': previous_line_name,
            'line_name': line_name,
            'head_snapshot_id': previous_head_snapshot_id,
        },
    )
    summary = get_worktree(ctx, worktree_name)
    return {
        **summary,
        'previous_line_name': previous_line_name,
        'line_name': line_name,
        'created_line': created_line,
    }


def bind_worktree(
    ctx: RepoContext,
    name: str | None = None,
    *,
    task_id: str | None = None,
    change_id: str | None = None,
    auto_created_for_task: bool | None = None,
    fork_snapshot_id: str | None = None,
    forked_from_line: str | None = None,
    target_base_line: str | None = None,
) -> dict[str, Any]:
    worktree_name = _resolve_worktree_name(ctx, name)
    metadata = _load_worktree_metadata(ctx, worktree_name)
    _guard_worktree_binding_task_lineage(
        ctx,
        worktree_name=worktree_name,
        metadata=metadata,
        task_id=task_id,
        change_id=change_id,
    )
    updates: dict[str, object] = {}
    if task_id is not None:
        updates['bound_task_id'] = str(task_id).strip() or None
    if change_id is not None:
        updates['bound_change_id'] = str(change_id).strip() or None
    if auto_created_for_task is not None:
        updates['auto_created_for_task'] = bool(auto_created_for_task)
        if auto_created_for_task:
            updates['creation_kind'] = 'task_auto_created'
            updates['cleanup_policy'] = 'after_remote_land'
    if fork_snapshot_id is not None:
        updates['fork_snapshot_id'] = normalize_optional_text(fork_snapshot_id)
    if forked_from_line is not None:
        updates['forked_from_line'] = normalize_optional_text(forked_from_line)
    if target_base_line is not None:
        updates['target_base_line'] = normalize_optional_text(target_base_line)
    if updates:
        _update_worktree_registration(ctx, worktree_name, **updates)
        local_control.record_event(
            ctx,
            'worktree.bound',
            'worktree',
            worktree_name,
            {
                'name': worktree_name,
                'task_id': updates.get('bound_task_id'),
                'change_id': updates.get('bound_change_id'),
                'auto_created_for_task': updates.get('auto_created_for_task'),
                'fork_snapshot_id': updates.get('fork_snapshot_id'),
                'forked_from_line': updates.get('forked_from_line'),
                'target_base_line': updates.get('target_base_line'),
            },
        )
    return _touch_worktree_metadata(ctx, worktree_name)
