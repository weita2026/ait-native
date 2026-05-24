from __future__ import annotations

import shlex
from pathlib import Path
from typing import Any

from rich import print as rprint

from ..repo_paths import RepoContext
from ..store import workspace_status as local_workspace_status
from .workflow_mode_config import _normalize_text_value
from .worktree_views import _worktree_shell_command


def _task_worktree_output(data: dict[str, Any]) -> dict[str, Any]:
    payload = dict(data)
    path_value = str(payload.get("open_path") or payload.get("alias_path") or payload.get("path") or "").strip()
    if path_value:
        payload["open_path"] = path_value
        payload["cd_command"] = f"cd {shlex.quote(path_value)}"
        payload["shell_command"] = _worktree_shell_command(path_value, payload)
    return payload


def _task_worktree_guidance(
    ctx: RepoContext,
    worktree: dict[str, Any] | None,
    *,
    source_workspace_status: dict[str, Any] | None = None,
) -> dict[str, Any] | None:
    if worktree is None:
        return None
    path_value = (
        _normalize_text_value(worktree.get("open_path"))
        or _normalize_text_value(worktree.get("alias_path"))
        or _normalize_text_value(worktree.get("path"))
        or _normalize_text_value(worktree.get("workspace_root"))
    )
    if path_value is None:
        return None
    target_path = Path(path_value).expanduser()
    target_workspace_root = str(target_path if target_path.is_absolute() else (ctx.root / target_path).resolve())
    current_workspace_root = str(ctx.root.resolve())
    guidance: dict[str, Any] = {
        "switch_required": current_workspace_root != target_workspace_root,
        "current_workspace_root": current_workspace_root,
        "target_workspace_root": target_workspace_root,
        "cd_command": _normalize_text_value(worktree.get("cd_command")) or f"cd {shlex.quote(target_workspace_root)}",
        "shell_command": _normalize_text_value(worktree.get("shell_command"))
        or _worktree_shell_command(target_workspace_root, worktree),
        "message": (
            "Bound worktree created. Your current shell has not been switched automatically."
            if current_workspace_root != target_workspace_root
            else "Bound worktree created for this task."
        ),
    }
    if current_workspace_root != target_workspace_root:
        guidance["root_guard_warning"] = (
            "Future stateful `ait` commands from the repo root will be guarded until you continue in the bound worktree "
            "or remove that worktree."
        )
    if source_workspace_status is None or bool(source_workspace_status.get("clean")):
        return guidance
    changed_paths = [str(item).strip() for item in source_workspace_status.get("changed_paths") or [] if str(item).strip()]
    sample = changed_paths[:5]
    remaining = max(len(changed_paths) - len(sample), 0)
    sample_summary = ", ".join(sample)
    if remaining:
        sample_summary = f"{sample_summary}, +{remaining} more" if sample_summary else f"+{remaining} more"
    guidance["source_workspace_changed_count"] = int(source_workspace_status.get("changed_count") or len(changed_paths))
    guidance["source_workspace_changed_paths"] = changed_paths
    if sample_summary:
        guidance["source_workspace_summary"] = (
            f"Current workspace had {guidance['source_workspace_changed_count']} changed path(s): {sample_summary}"
        )
    guidance["dirty_source_warning"] = (
        "Existing workspace changes were not copied into the new task worktree. "
        "Reconcile or discard them in the source workspace before continuing formal task work."
    )
    return guidance


def _task_auto_worktree_source_status(ctx: RepoContext) -> dict[str, Any] | None:
    return local_workspace_status(ctx)


def _attach_task_worktree_guidance(
    ctx: RepoContext,
    payload: dict[str, Any],
    *,
    worktree: dict[str, Any] | None,
    source_workspace_status: dict[str, Any] | None = None,
) -> dict[str, Any]:
    guidance = _task_worktree_guidance(ctx, worktree, source_workspace_status=source_workspace_status)
    if guidance is None:
        return payload
    enriched = dict(payload)
    enriched["worktree_guidance"] = guidance
    return enriched


def _render_task_worktree_guidance(guidance: dict[str, Any] | None) -> None:
    if not isinstance(guidance, dict):
        return
    message = _normalize_text_value(guidance.get("message"))
    target_workspace_root = _normalize_text_value(guidance.get("target_workspace_root"))
    cd_command = _normalize_text_value(guidance.get("cd_command"))
    dirty_summary = _normalize_text_value(guidance.get("source_workspace_summary"))
    dirty_warning = _normalize_text_value(guidance.get("dirty_source_warning"))
    root_guard_warning = _normalize_text_value(guidance.get("root_guard_warning"))
    if message:
        rprint(f"[yellow]{message}[/yellow]")
    if target_workspace_root:
        rprint(f"[cyan]task worktree:[/cyan] {target_workspace_root}")
    if cd_command:
        rprint(f"[green]Continue in the task worktree with:[/green] [bold]{cd_command}[/bold]")
    if root_guard_warning:
        rprint(f"[yellow]{root_guard_warning}[/yellow]")
    if dirty_summary:
        rprint(f"[yellow]{dirty_summary}[/yellow]")
    if dirty_warning:
        rprint(f"[yellow]{dirty_warning}[/yellow]")
