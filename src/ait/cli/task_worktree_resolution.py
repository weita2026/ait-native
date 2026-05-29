from __future__ import annotations

import re
from typing import Any, Mapping

from ait_protocol.common import read_json

from ..remote_client import RemoteError, get_change as remote_get_change
from ..repo_paths import RepoContext
from ..store_local_changes import (
    get_local_change,
)
from ..store_local_tasks import (
    get_local_task,
)
from ..store import (
    create_line,
    get_line,
    get_worktree as local_get_worktree,
    list_worktrees as local_list_worktrees,
    load_config,
)
from ..store_remotes import (
    get_remote,
)
from .task_tracking_bindings import _task_worktree_repo_ctx
from .workflow_mode_config import _normalize_text_value


_LEGACY_TASK_EXECUTION_NAMESPACE_PREFIX = "task"


def _task_identity_slug(task_id: str) -> str:
    task_slug = re.sub(r"[^a-z0-9]+", "-", task_id.lower()).strip("-")
    if not task_slug:
        raise ValueError("Task id is required to derive a task worktree name.")
    return task_slug


def _task_bound_worktree_name(task_id: str, title: str | None = None) -> str:
    del title  # retained for call-site compatibility while extraction stays behavior-neutral
    return _task_identity_slug(task_id)


def _legacy_task_bound_worktree_name(task_id: str) -> str:
    return f"{_LEGACY_TASK_EXECUTION_NAMESPACE_PREFIX}-{_task_bound_worktree_name(task_id)}"


def _resolve_task_bound_worktree_name(ctx: RepoContext, task_id: str, title: str | None = None) -> str:
    base_name = _task_bound_worktree_name(task_id, title)
    suffix = 1
    while True:
        candidate_name = base_name if suffix == 1 else f"{base_name}-{suffix}"
        try:
            local_get_worktree(ctx, candidate_name)
        except KeyError:
            return candidate_name
        suffix += 1


def _task_feature_line_name(task_id: str) -> str:
    return f"feature/{_task_bound_worktree_name(task_id)}"


def _legacy_task_feature_line_name(task_id: str) -> str:
    return f"feature/{_legacy_task_bound_worktree_name(task_id)}"


def _task_feature_line_candidates(task_id: str) -> list[str]:
    candidates = [
        _task_feature_line_name(task_id),
        _legacy_task_feature_line_name(task_id),
    ]
    return list(dict.fromkeys(candidates))


def _change_bootstrap_lineage(
    change_row: Mapping[str, Any] | None,
    *,
    fallback_base_line_name: str,
) -> tuple[str, str | None]:
    resolved_base_line_name = str(fallback_base_line_name or "").strip() or fallback_base_line_name
    if not isinstance(change_row, Mapping):
        return resolved_base_line_name, None
    change_id = _normalize_text_value(change_row.get("change_id")) or "unknown change"
    change_base_line = (
        _normalize_text_value(change_row.get("base_line"))
        or _normalize_text_value(change_row.get("forked_from_line"))
    )
    if (
        change_base_line is not None
        and resolved_base_line_name
        and change_base_line != resolved_base_line_name
    ):
        raise ValueError(
            f"Bound change `{change_id}` forks from `{change_base_line}`, not `{resolved_base_line_name}`."
        )
    fork_snapshot_id = _normalize_text_value(change_row.get("fork_snapshot_id"))
    if fork_snapshot_id is None:
        raise ValueError(
            f"Bound change `{change_id}` is missing fork_snapshot_id lineage, so task worktree bootstrap cannot safely continue."
        )
    return change_base_line or resolved_base_line_name, fork_snapshot_id


def _ensure_task_feature_line(
    ctx: RepoContext,
    *,
    task_id: str,
    base_line_name: str,
    base_snapshot_id: str | None = None,
) -> dict[str, Any]:
    feature_line = None
    for feature_line_name in _task_feature_line_candidates(task_id):
        try:
            feature_line = get_line(ctx, feature_line_name)
            break
        except KeyError:
            continue
    if feature_line is None:
        resolved_base_snapshot_id = _normalize_text_value(base_snapshot_id)
        if resolved_base_snapshot_id is None:
            base_line = get_line(ctx, base_line_name)
            resolved_base_snapshot_id = _normalize_text_value(base_line.get("head_snapshot_id"))
        feature_line = create_line(ctx, _task_feature_line_name(task_id), resolved_base_snapshot_id)
    feature_line_name = str(feature_line.get("line_name") or "")
    if str(feature_line.get("status") or "active").strip() == "archived":
        raise ValueError(f"Feature line {feature_line_name} is archived and cannot be reused for task {task_id}.")
    return feature_line


def _find_bound_task_worktree(
    ctx: RepoContext,
    task_id: str,
    *,
    auto_created_only: bool | None = None,
) -> dict[str, Any] | None:
    normalized_task_id = str(task_id or "").strip()
    if not normalized_task_id:
        return None
    candidate_task_ids = {normalized_task_id}
    try:
        canonical_task_id = str(get_local_task(ctx, normalized_task_id).get("task_id") or "").strip()
    except KeyError:
        canonical_task_id = ""
    if canonical_task_id:
        candidate_task_ids.add(canonical_task_id)
    candidate_names: list[str] = []
    for metadata_path in sorted(ctx.worktree_registry_dir.glob("*.json")):
        payload = read_json(metadata_path, default={}) or {}
        if not isinstance(payload, dict):
            continue
        if str(payload.get("bound_task_id") or "").strip() not in candidate_task_ids:
            continue
        if (
            auto_created_only is not None
            and bool(payload.get("auto_created_for_task")) is not auto_created_only
        ):
            continue
        worktree_name = _normalize_text_value(payload.get("name")) or metadata_path.stem
        if worktree_name:
            candidate_names.append(worktree_name)
    candidate_names = list(dict.fromkeys(candidate_names))
    rows: list[dict[str, Any]] = []
    for worktree_name in candidate_names:
        try:
            rows.append(local_get_worktree(ctx, worktree_name, refresh_status=False))
        except KeyError:
            continue
    if not rows:
        rows = [
            row
            for row in local_list_worktrees(ctx, refresh_status=False)
            if str(row.get("bound_task_id") or "").strip() in candidate_task_ids
            and (
                auto_created_only is None
                or bool(row.get("auto_created_for_task")) is auto_created_only
            )
        ]
    if not rows:
        return None
    rows.sort(
        key=lambda row: (
            bool(row.get("auto_created_for_task")),
            str(row.get("created_at") or ""),
        ),
        reverse=True,
    )
    return rows[0]


def _find_auto_created_task_worktree(ctx: RepoContext, task_id: str) -> dict[str, Any] | None:
    return _find_bound_task_worktree(ctx, task_id, auto_created_only=True)


def _session_bound_worktree(
    ctx: RepoContext,
    *,
    local: bool,
    remote_name: str | None,
    task_id: str | None = None,
    change_id: str | None = None,
    worktree_name: str | None = None,
) -> dict[str, Any] | None:
    repo_ctx = _task_worktree_repo_ctx(ctx)
    resolved_worktree_name = _normalize_text_value(worktree_name)
    if resolved_worktree_name is None:
        resolved_worktree_name = _normalize_text_value(load_config(ctx).get("worktree_name"))
    if resolved_worktree_name is not None:
        try:
            return local_get_worktree(repo_ctx, resolved_worktree_name)
        except KeyError as exc:
            raise ValueError(f"Unknown worktree: {resolved_worktree_name}") from exc

    resolved_task_id = _normalize_text_value(task_id)
    resolved_change_id = _normalize_text_value(change_id)
    if resolved_task_id is None and resolved_change_id is not None:
        try:
            if local:
                resolved_task_id = _normalize_text_value(get_local_change(ctx, resolved_change_id).get("task_id"))
            else:
                remote_row = get_remote(ctx, remote_name)
                repo_name = _normalize_text_value(remote_row.get("repo_name")) or str(load_config(ctx).get("repo_name") or "")
                resolved_task_id = _normalize_text_value(
                    remote_get_change(remote_row["url"], resolved_change_id, repo_name=repo_name).get("task_id")
                )
        except (KeyError, RemoteError, ValueError):
            resolved_task_id = None
    if resolved_task_id is None:
        return None
    return _find_bound_task_worktree(repo_ctx, resolved_task_id)
