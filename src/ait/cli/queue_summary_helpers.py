from __future__ import annotations

import sys
from concurrent.futures import ThreadPoolExecutor
from typing import Any, Optional

from ..remote_client import (
    RemoteError,
    list_changes as remote_list_changes,
    read_queue_summary_bundle as remote_read_queue_summary_bundle,
    read_reviewer_inbox as remote_read_reviewer_inbox,
    read_task_queue as remote_read_task_queue,
)
from ..store import (
    RepoContext,
    list_local_changes,
    list_local_tasks,
    list_remotes,
    load_config,
    worktree_doctor as local_worktree_doctor,
    workspace_status as local_workspace_status,
)
from .queue_views import (
    _queue_actionable_local_changes,
    _queue_actionable_local_tasks,
    _queue_change_inventory,
    _queue_local_summary,
)
from .remote_repository_defaults import _remote_tuple


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)


def _queue_summary_bundle_missing(exc: RemoteError) -> bool:
    text = str(exc)
    return "/v1/native/read/queue-summary" in text and " 404 " in text


def _queue_remote_section(
    ctx: RepoContext,
    remote_name: Optional[str],
    status: str,
    include_all_changes: bool,
) -> dict[str, Any]:
    load_config_fn = _app_override("load_config", load_config)
    list_remotes_fn = _app_override("list_remotes", list_remotes)
    remote_tuple_fn = _app_override("_remote_tuple", _remote_tuple)
    read_queue_summary_bundle_fn = _app_override("remote_read_queue_summary_bundle", remote_read_queue_summary_bundle)
    read_task_queue_fn = _app_override("remote_read_task_queue", remote_read_task_queue)
    read_reviewer_inbox_fn = _app_override("remote_read_reviewer_inbox", remote_read_reviewer_inbox)
    list_changes_fn = _app_override("remote_list_changes", remote_list_changes)
    queue_change_inventory_fn = _app_override("_queue_change_inventory", _queue_change_inventory)

    cfg = load_config_fn(ctx)
    available_remotes = [str(row.get("name") or "") for row in list_remotes_fn(ctx) if row.get("name")]
    remote_section: dict[str, Any] = {
        "configured": False,
        "remote_name": None,
        "repo_name": cfg.get("repo_name") or ctx.root.name,
        "url": None,
        "status_filter": status,
        "available_remotes": available_remotes,
        "task_queue": None,
        "reviewer_inbox": None,
        "changes": None,
        "error": None,
    }
    should_attempt_remote = remote_name is not None or bool(cfg.get("default_remote"))
    if not should_attempt_remote:
        if available_remotes:
            remote_section["error"] = "No default remote configured. Set one first, or pass --remote <name> for this queue read."
        return remote_section
    remote_row, repo_name = remote_tuple_fn(ctx, remote_name)
    remote_section["configured"] = True
    remote_section["remote_name"] = remote_row.get("name")
    remote_section["repo_name"] = remote_row.get("repo_name") or repo_name
    remote_section["url"] = remote_row.get("url")
    try:
        try:
            bundle = read_queue_summary_bundle_fn(remote_row["url"], repo_name, status=status)
        except RemoteError as exc:
            if not _queue_summary_bundle_missing(exc):
                raise
            with ThreadPoolExecutor(max_workers=2, thread_name_prefix="ait-queue-summary") as executor:
                task_queue_future = executor.submit(read_task_queue_fn, remote_row["url"], repo_name, status=status)
                reviewer_inbox_future = executor.submit(read_reviewer_inbox_fn, remote_row["url"], repo_name)
                remote_section["task_queue"] = task_queue_future.result()
                remote_section["reviewer_inbox"] = reviewer_inbox_future.result()
        else:
            remote_section["task_queue"] = bundle.get("task_queue") if isinstance(bundle, dict) else None
            remote_section["reviewer_inbox"] = bundle.get("reviewer_inbox") if isinstance(bundle, dict) else None
        if include_all_changes:
            task_items = remote_section["task_queue"].get("items", []) if isinstance(remote_section["task_queue"], dict) else []
            review_items = (
                remote_section["reviewer_inbox"].get("items", [])
                if isinstance(remote_section["reviewer_inbox"], dict)
                else []
            )
            change_rows = list_changes_fn(remote_row["url"], repo_name)
            remote_section["changes"] = queue_change_inventory_fn(change_rows, task_items, review_items)
    except RemoteError as exc:
        remote_section["error"] = str(exc)
    return remote_section


def _queue_summary_payload(
    ctx: RepoContext,
    remote_name: Optional[str],
    status: str,
    include_all_changes: bool = False,
) -> dict[str, Any]:
    load_config_fn = _app_override("load_config", load_config)
    workspace_status_fn = _app_override("local_workspace_status", local_workspace_status)
    worktree_doctor_fn = _app_override("local_worktree_doctor", local_worktree_doctor)
    list_local_tasks_fn = _app_override("list_local_tasks", list_local_tasks)
    list_local_changes_fn = _app_override("list_local_changes", list_local_changes)
    actionable_local_tasks_fn = _app_override("_queue_actionable_local_tasks", _queue_actionable_local_tasks)
    actionable_local_changes_fn = _app_override("_queue_actionable_local_changes", _queue_actionable_local_changes)
    queue_local_summary_fn = _app_override("_queue_local_summary", _queue_local_summary)
    queue_remote_section_fn = _app_override("_queue_remote_section", _queue_remote_section)

    cfg = load_config_fn(ctx)
    repo_name = cfg.get("repo_name") or ctx.root.name
    workspace = workspace_status_fn(ctx)
    worktrees = worktree_doctor_fn(ctx, refresh_status=False)
    local_tasks = list_local_tasks_fn(ctx)
    local_changes = list_local_changes_fn(ctx)
    actionable_local_tasks = actionable_local_tasks_fn(local_tasks)
    actionable_local_changes = actionable_local_changes_fn(local_changes)
    local_summary = queue_local_summary_fn(local_tasks, local_changes)
    remote_section = queue_remote_section_fn(ctx, remote_name, status, include_all_changes)
    task_queue = remote_section.get("task_queue") if isinstance(remote_section.get("task_queue"), dict) else {}
    reviewer_inbox = (
        remote_section.get("reviewer_inbox")
        if isinstance(remote_section.get("reviewer_inbox"), dict)
        else {}
    )
    task_queue_summary = task_queue.get("summary") if isinstance(task_queue.get("summary"), dict) else {}
    remote_changes = remote_section.get("changes") if isinstance(remote_section.get("changes"), list) else []
    return {
        "repo_name": repo_name,
        "query": {
            "all_changes": include_all_changes,
            "status": status,
        },
        "remote": remote_section,
        "local": {
            "tasks": actionable_local_tasks,
            "changes": actionable_local_changes,
            "all_tasks": local_tasks,
            "all_changes": local_changes,
            "summary": local_summary,
        },
        "workspace": {
            "status": workspace,
            "worktrees": worktrees,
        },
        "summary": {
            "shared_task_count": int(task_queue.get("count") or 0),
            "attention_required_count": int(task_queue_summary.get("attention_required") or 0),
            "ready_to_land_count": int(task_queue_summary.get("ready_to_land") or 0),
            "ready_to_complete_count": int(task_queue_summary.get("ready_to_complete") or 0),
            "open_shared_change_count": len(remote_changes),
            "reviewer_inbox_count": int(reviewer_inbox.get("count") or 0),
            "local_draft_task_count": int(local_summary["draft_task_count"]),
            "local_draft_change_count": int(local_summary["draft_change_count"]),
            "workspace_dirty": bool(workspace.get("clean") is False),
            "workspace_changed_count": int(workspace.get("changed_count") or 0),
            "dirty_worktree_count": int(worktrees.get("dirty_count") or 0),
            "stale_worktree_count": int(worktrees.get("stale_count") or 0),
        },
    }
