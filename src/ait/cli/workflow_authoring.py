from __future__ import annotations

from typing import Any, Optional

import typer

from ..remote_client import (
    create_change as remote_create_change,
    create_task as remote_create_task,
    get_remote_line,
)
from ..repo_paths import RepoContext
from ..store_local_changes import (
    create_local_change,
)
from ..store_local_tasks import (
    create_local_task,
    get_local_task,
)
from ..store import (
    get_line,
    load_config,
)
from .plan_task_linkage import _normalize_plan_task_linkage
from .remote_repository_defaults import _remote_tuple, _sync_remote_repository_defaults
from .workflow_mode_config import (
    _effective_change_default_scope,
    _effective_task_default_scope,
    _normalize_text_value,
)


def _validate_local_scope(local: bool, remote_name: Optional[str]) -> None:
    if local and remote_name is not None:
        raise typer.BadParameter("--local cannot be combined with --remote")


def _workflow_uses_local_scope(
    ctx: RepoContext,
    *,
    kind: str,
    local: bool,
    remote_name: Optional[str],
) -> bool:
    _validate_local_scope(local, remote_name)
    if local:
        return True
    if remote_name is not None:
        return False
    if kind == "task":
        return _effective_task_default_scope(ctx)["value"] == "local"
    if kind == "change":
        return _effective_change_default_scope(ctx)["value"] == "local"
    raise ValueError(f"Unsupported workflow scope kind: {kind}")


def _plan_uses_local_store(local: bool, remote_name: Optional[str]) -> bool:
    """Return the selected plan scope.

    Native plan authoring is local-first: omitting ``--remote`` reads/writes the
    local draft plan store. ``--local`` remains accepted as an explicit
    no-op/compatibility signal, while ``--remote NAME`` selects shared remote
    plan state.
    """
    _validate_local_scope(local, remote_name)
    return remote_name is None


def _preflight_change_base_line(
    ctx: RepoContext,
    *,
    local: bool,
    remote_name: Optional[str],
    base_line: str,
) -> None:
    _validate_local_scope(local, remote_name)
    if local:
        get_line(ctx, base_line)
        return
    remote_row, repo_name = _remote_tuple(ctx, remote_name)
    get_remote_line(remote_row["url"], repo_name, base_line)


def _create_task_record(
    ctx: RepoContext,
    *,
    title: str,
    intent: str,
    risk: str,
    local: bool,
    remote_name: Optional[str],
    plan_id: str | None = None,
    plan_revision_id: str | None = None,
    plan_item_ref: str | None = None,
    worktree: dict[str, Any] | None = None,
    source_surface: str = "cli.task.start",
) -> dict[str, Any]:
    _validate_local_scope(local, remote_name)
    resolved_plan_id, resolved_plan_revision_id, resolved_plan_item_ref = _normalize_plan_task_linkage(
        ctx,
        plan_id=plan_id,
        plan_revision_id=plan_revision_id,
        plan_item_ref=plan_item_ref,
        local=local,
        require_execution_binding=True,
        guard_dag_task_bootstrap=True,
    )
    if local:
        return create_local_task(
            ctx,
            title,
            intent,
            risk,
            plan_id=resolved_plan_id,
            origin_plan_revision_id=resolved_plan_revision_id,
            plan_item_ref=resolved_plan_item_ref,
        )
    remote_row, repo_name = _sync_remote_repository_defaults(ctx, remote_name)
    from .workflow_boundary_sessions import _remote_task_tracking_session_seed

    tracking_session = _remote_task_tracking_session_seed(
        ctx,
        title=title,
        intent=intent,
        remote_name=remote_name,
        worktree=worktree,
        source_surface=source_surface,
    )
    return remote_create_task(
        remote_row["url"],
        repo_name,
        title,
        intent,
        risk,
        plan_id=resolved_plan_id,
        origin_plan_revision_id=resolved_plan_revision_id,
        plan_item_ref=resolved_plan_item_ref,
        tracking_session=tracking_session,
    )


def _create_change_record(
    ctx: RepoContext,
    *,
    task_id: str,
    title: str,
    base_line: str,
    risk: str,
    local: bool,
    remote_name: Optional[str],
) -> dict[str, Any]:
    _validate_local_scope(local, remote_name)
    if local:
        local_task = get_local_task(ctx, task_id)
        get_line(ctx, base_line)
        repo_name = load_config(ctx).get("repo_name") or ctx.root.name
        if local_task["repo_name"] != repo_name:
            raise KeyError(f"Local task {task_id} belongs to repository {local_task['repo_name']}, not {repo_name}")
        return create_local_change(ctx, task_id, title, base_line, risk)
    remote_row, repo_name = _sync_remote_repository_defaults(ctx, remote_name)
    return remote_create_change(
        remote_row["url"],
        repo_name,
        task_id,
        title,
        base_line,
        risk,
        **_remote_change_lineage_payload(remote_row["url"], repo_name, base_line),
    )


def _remote_change_lineage_payload(base_url: str, repo_name: str, base_line: str) -> dict[str, str]:
    line_row = get_remote_line(base_url, repo_name, base_line)
    payload = {"forked_from_line": base_line}
    fork_snapshot_id = _normalize_text_value(line_row.get("head_snapshot_id"))
    if fork_snapshot_id is not None:
        payload["fork_snapshot_id"] = fork_snapshot_id
    return payload
