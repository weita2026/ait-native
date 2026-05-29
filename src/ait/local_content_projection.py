from __future__ import annotations

from contextlib import contextmanager
from contextvars import ContextVar
from pathlib import Path
from typing import Any, Iterable

from .local_content_workspace import (
    WorkspaceIgnoreRule,
    _load_workspace_ignore_rules,
    _parse_workspace_ignore_rules,
    _workspace_path_is_ignored,
)
from .repo_paths import RepoContext

_TASK_WORKTREE_MARKDOWN_BASE_RULES = _parse_workspace_ignore_rules(
    """
/docs/*.md
/docs/**/*.md
""".strip()
)
_LINEAGE_ONLY_MARKDOWN_ALLOWLIST: ContextVar[frozenset[str]] = ContextVar(
    "lineage_only_markdown_allowlist",
    default=frozenset(),
)


def _normalize_markdown_artifact_path(path_value: str | Path) -> str:
    return Path(str(path_value).replace("\\", "/")).as_posix().strip("/")


def _is_markdown_artifact_path(path_value: str | Path) -> bool:
    path = _normalize_markdown_artifact_path(path_value)
    return bool(path) and Path(path).suffix.lower() == ".md"


def _is_line_materialized_markdown_artifact_path(path_value: str | Path) -> bool:
    # Markdown planning artifacts stay on the plan-lineage side only and do not
    # participate in line snapshots or worktree materialization.
    return False


def _is_lineage_only_markdown_artifact_path(path_value: str | Path) -> bool:
    return _is_markdown_artifact_path(path_value) and not _is_line_materialized_markdown_artifact_path(path_value)


def _is_root_lineage_only_sprint_task_graph_path(path_value: str | Path) -> bool:
    path = _normalize_markdown_artifact_path(path_value)
    return path.startswith("docs/sprints/") and path.endswith(".task_graph.json")


def _task_worktree_sprint_markdown_projection_rules(ctx: RepoContext) -> tuple[WorkspaceIgnoreRule, ...]:
    if not ctx.is_worktree:
        return ()
    return _TASK_WORKTREE_MARKDOWN_BASE_RULES


def _merge_workspace_ignore_rules(
    base_rules: tuple[WorkspaceIgnoreRule, ...] | None,
    extra_rules: tuple[WorkspaceIgnoreRule, ...] | None,
) -> tuple[WorkspaceIgnoreRule, ...]:
    base = tuple(base_rules or ())
    extra = tuple(extra_rules or ())
    if not extra:
        return base
    if not base:
        return extra
    return base + extra


def _effective_workspace_ignore_rules(
    ctx: RepoContext,
    ignore_rules: tuple[WorkspaceIgnoreRule, ...] | None = None,
) -> tuple[WorkspaceIgnoreRule, ...]:
    base_rules = _load_workspace_ignore_rules(ctx.root) if ignore_rules is None else tuple(ignore_rules)
    return _merge_workspace_ignore_rules(base_rules, _task_worktree_sprint_markdown_projection_rules(ctx))


def _path_is_projected_out_for_task_worktree(ctx: RepoContext, rel_path: str | Path) -> bool:
    projection_rules = _task_worktree_sprint_markdown_projection_rules(ctx)
    if not projection_rules:
        return False
    return _workspace_path_is_ignored(Path(rel_path), projection_rules)


def _path_is_projected_out_for_workspace(ctx: RepoContext, rel_path: str | Path) -> bool:
    normalized = _normalize_markdown_artifact_path(rel_path)
    if _path_is_projected_out_for_task_worktree(ctx, normalized):
        return True
    if normalized in _LINEAGE_ONLY_MARKDOWN_ALLOWLIST.get():
        return False
    if _is_lineage_only_markdown_artifact_path(normalized):
        return True
    return (not ctx.is_worktree) and _is_root_lineage_only_sprint_task_graph_path(normalized)


@contextmanager
def allow_lineage_only_markdown_paths(paths: Iterable[str | Path]):
    normalized_paths = {
        normalized
        for path in paths
        if (normalized := _normalize_markdown_artifact_path(path))
    }
    if not normalized_paths:
        yield
        return
    current = _LINEAGE_ONLY_MARKDOWN_ALLOWLIST.get()
    token = _LINEAGE_ONLY_MARKDOWN_ALLOWLIST.set(frozenset(set(current) | normalized_paths))
    try:
        yield
    finally:
        _LINEAGE_ONLY_MARKDOWN_ALLOWLIST.reset(token)


def _filter_snapshot_file_map_for_workspace(
    ctx: RepoContext,
    snapshot_files: dict[str, dict[str, Any]],
) -> dict[str, dict[str, Any]]:
    if not snapshot_files:
        return {}
    return {
        path: entry
        for path, entry in snapshot_files.items()
        if not _path_is_projected_out_for_workspace(ctx, path)
    }


def _filter_workspace_state_for_workspace(
    ctx: RepoContext,
    workspace_files: dict[str, dict[str, Any]],
) -> dict[str, dict[str, Any]]:
    if not workspace_files:
        return {}
    return {
        path: entry
        for path, entry in workspace_files.items()
        if not _path_is_projected_out_for_workspace(ctx, path)
    }
