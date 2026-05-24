from __future__ import annotations

from rich import print as rprint
from rich.table import Table

from ...local_control import restart_workflow_task as restart_local_task
from ...remote_client import restart_task as remote_restart_task
from ..plan_markdown_authoring import _guard_markdown_task_dispatch
from ..workflow_authoring import (
    _create_change_record,
    _create_task_record,
    _preflight_change_base_line,
    _workflow_uses_local_scope,
)
from ..workflow_identity_helpers import (
    _aligned_remote_publish_identity_request,
    _require_remote_workflow_identity_family,
)
from ..task_close_tracking import (
    _SESSION_STATUS_PRIORITY,
    _append_task_retrospective_event,
    _auto_track_created_task,
    _build_task_retrospective,
    _close_task_review_session,
    _create_task_retrospective_checkpoint,
    _ensure_task_review_session_active,
    _finalize_task_close_tracking,
    _latest_task_session_row,
    _list_task_review_session_events,
    _maybe_attach_task_tracking,
    _resolve_task_review_session,
    _task_improvement_plan,
    _task_tracking_session_metadata,
    _task_tracking_session_title,
    _trim_current_task_close_event,
)
from ...task_statuses import (
    TASK_STATUS_ABANDONED,
    TASK_STATUS_LATER_PROMOTION_EXCLUDED,
    is_task_abandoned_status,
    is_task_later_promotion_excluded_status,
)
from ...task_tokens import local_task_tokens_report, remote_task_tokens_report
from ..shared import export_app_namespace
from ..shared import (
    COMPLETED_LOCAL_BATCH_PROMOTION_GUIDANCE as _COMPLETED_LOCAL_BATCH_PROMOTION_GUIDANCE,
    LOCAL_SCOPE_OVERRIDE_HELP as _LOCAL_SCOPE_OVERRIDE_HELP,
    REMOTE_SCOPE_OVERRIDE_HELP as _REMOTE_SCOPE_OVERRIDE_HELP,
    REMOTE_TARGET_DEFAULT_HELP as _REMOTE_TARGET_DEFAULT_HELP,
)

export_app_namespace(globals())

def _run_task_start_command(
    *,
    title: str,
    intent: str,
    risk: str,
    plan: Optional[str],
    revision: Optional[str],
    plan_item_ref: Optional[str],
    local: bool,
    remote: Optional[str],
    json_output: bool,
    command_name: str,
    source_surface: str,
    create_initial_change: bool,
    change_title: Optional[str] = None,
    base_line: Optional[str] = None,
):
    if not create_initial_change:
        if change_title is not None:
            raise typer.BadParameter("`--change-title` cannot be used together with `--task-only`.")
        if base_line is not None:
            raise typer.BadParameter("`--base-line` cannot be used together with `--task-only`.")
    ctx = _ctx()
    resolved_change_title: str | None = None
    if create_initial_change:
        resolved_change_title = (change_title or title).strip()
        if not resolved_change_title:
            raise typer.BadParameter("The initial change title cannot be empty.")
    worktree: dict[str, Any] | None = None
    resolved_base_line = base_line or _default_line_name(ctx)
    source_workspace_status = _task_auto_worktree_source_status(ctx)
    try:
        _guard_active_root_worktree(ctx, command_name)
        use_local = _workflow_uses_local_scope(ctx, kind="task", local=local, remote_name=remote)
        if create_initial_change:
            _preflight_change_base_line(ctx, local=use_local, remote_name=remote, base_line=resolved_base_line)
        _guard_markdown_task_dispatch(
            ctx,
            plan_id=plan,
            plan_revision_id=revision,
            plan_item_ref=plan_item_ref,
            command_name=command_name,
        )
        data = _create_task_record(
            ctx,
            title=title,
            intent=intent,
            risk=risk,
            local=use_local,
            remote_name=remote,
            plan_id=plan,
            plan_revision_id=revision,
            plan_item_ref=plan_item_ref,
            source_surface=source_surface,
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    change: dict[str, Any] | None = None
    if create_initial_change:
        try:
            change = _create_change_record(
                ctx,
                task_id=str(data["task_id"]),
                title=resolved_change_title,
                base_line=resolved_base_line,
                risk=risk,
                local=use_local,
                remote_name=remote,
            )
        except (KeyError, RemoteError, ValueError) as exc:
            raise click.ClickException(
                f"Task {data['task_id']} was created, but the initial change could not be created: {exc}"
            ) from exc
    try:
        worktree = _maybe_auto_create_task_worktree(
            ctx,
            task_id=str(data["task_id"]),
            title=str(data["title"]),
            base_line_name=resolved_base_line,
            change_id=str(change["change_id"]) if change is not None else None,
        )
    except (KeyError, ValueError, WorkspaceCommandBusyError, IsADirectoryError) as exc:
        if change is not None:
            raise click.ClickException(
                f"Task {data['task_id']} and change {change['change_id']} were created, but the bound worktree could not be created: {exc}"
            ) from exc
        raise click.ClickException(
            f"Task {data['task_id']} was created, but the bound worktree could not be created: {exc}"
        ) from exc
    payload = dict(data)
    if change is not None:
        payload["change"] = change
    if worktree is not None:
        payload["worktree"] = worktree
    payload = _maybe_attach_task_tracking(
        ctx,
        payload,
        local=use_local,
        remote_name=None if use_local else remote,
        worktree=worktree,
    )
    payload = _attach_task_worktree_guidance(
        ctx,
        payload,
        worktree=worktree,
        source_workspace_status=source_workspace_status,
    )
    _emit_task_creation_payload(payload, json_output=json_output)


@task_app.command(
    "start",
    help=(
        "Start a task and optionally open its first change. "
        "In repositories configured with `workflow_mode=solo_remote`, omitting "
        "`--local` and `--remote` usually starts remote-backed lineage."
    ),
)
def task_start(
    title: str = typer.Option(..., "--title"),
    intent: str = typer.Option(..., "--intent"),
    task_only: bool = typer.Option(
        False,
        "--task-only",
        help="Start only the task and defer opening the first change.",
    ),
    change_title: Optional[str] = typer.Option(
        None,
        "--change-title",
        help="Override the initial change title. Defaults to the task title unless `--task-only` is set.",
    ),
    base_line: Optional[str] = typer.Option(
        None,
        "--base-line",
        help="Base line for the initial change. Defaults to the repository default line unless `--task-only` is set.",
    ),
    risk: str = typer.Option("medium", "--risk"),
    plan: Optional[str] = typer.Option(None, "--plan"),
    revision: Optional[str] = typer.Option(None, "--revision"),
    plan_item_ref: Optional[str] = typer.Option(None, "--plan-item-ref"),
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    json_output: bool = typer.Option(False, "--json"),
):
    _run_task_start_command(
        title=title,
        intent=intent,
        risk=risk,
        plan=plan,
        revision=revision,
        plan_item_ref=plan_item_ref,
        local=local,
        remote=remote,
        json_output=json_output,
        command_name="task start",
        source_surface="cli.task.start",
        create_initial_change=not task_only,
        change_title=change_title,
        base_line=base_line,
    )


@task_app.command("list", help="List task records in the effective local or remote workflow scope.", short_help="List task records in effective scope.")
def task_list(
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    json_output: bool = typer.Option(False, "--json"),
):
    """List task records in the effective local or remote workflow scope."""
    ctx = _ctx()
    try:
        use_local = _workflow_uses_local_scope(ctx, kind="task", local=local, remote_name=remote)
        if use_local:
            rows = list_local_tasks(ctx)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            rows = remote_list_tasks(remote_row["url"], repo_name)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    table = Table(title="ait native tasks")
    table.add_column("task_id")
    table.add_column("title")
    table.add_column("risk")
    table.add_column("status")
    if use_local:
        table.add_column("publication")
    for row in rows:
        cells = [row["task_id"], row["title"], row["risk_tier"], row["status"]]
        if use_local:
            cells.append(row["publication_state"])
        table.add_row(*cells)
    rprint(table)


@task_app.command("show", help="Inspect one task record in the effective local or remote workflow scope.", short_help="Inspect one task record.")
def task_show(
    task_id: str,
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    repo: Optional[str] = typer.Option(None, "--repo", help="Resolve the task ref within this remote repository when using repo-scoped workflow identity."),
    json_output: bool = typer.Option(False, "--json"),
):
    """Inspect one task record in the effective local or remote workflow scope."""
    ctx = _ctx()
    use_local = False
    selected_remote_name: str | None = None
    try:
        use_local = _workflow_uses_local_scope(ctx, kind="task", local=local, remote_name=remote)
        if use_local:
            data = get_local_task(ctx, task_id)
        else:
            remote_row, _ = _remote_tuple(ctx, remote)
            selected_remote_name = str(remote_row.get("name") or remote or "selected remote")
            data = remote_get_task(remote_row["url"], task_id, repo_name=repo)
    except (KeyError, RemoteError, ValueError) as exc:
        if not use_local:
            raise typer.BadParameter(
                _workflow_show_scope_error_message(
                    ctx,
                    kind="task",
                    workflow_id=task_id,
                    remote_name=selected_remote_name or remote or "selected remote",
                    exc=exc,
                )
            ) from exc
        raise typer.BadParameter(str(exc)) from exc
    data = _attach_task_show_worktree_state(ctx, data)
    _emit(data, json_output)


@task_app.command(
    "tokens",
    help="Summarize one task's assistant token usage from workflow sessions and session events.",
    short_help="Summarize one task token usage.",
)
def task_tokens_cmd(
    task_id: str,
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    repo: Optional[str] = typer.Option(
        None,
        "--repo",
        help="Resolve the task ref within this remote repository when using repo-scoped workflow identity.",
    ),
    by: Optional[str] = typer.Option(
        None,
        "--by",
        help="Optional breakdown table: change, worktree, session, or model.",
    ),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    normalized_by = str(by or "").strip().lower() or None
    if normalized_by not in {None, "change", "worktree", "session", "model"}:
        raise typer.BadParameter("`--by` must be one of: change, worktree, session, model.")
    use_local = False
    selected_remote_name: str | None = None
    try:
        use_local = _workflow_uses_local_scope(ctx, kind="task", local=local, remote_name=remote)
        if use_local:
            data = local_task_tokens_report(ctx, task_id)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            selected_remote_name = str(remote_row.get("name") or remote or "selected remote")
            data = remote_task_tokens_report(remote_row["url"], repo or repo_name, task_id)
    except (KeyError, RemoteError, ValueError) as exc:
        if not use_local:
            raise typer.BadParameter(
                _workflow_show_scope_error_message(
                    ctx,
                    kind="task",
                    workflow_id=task_id,
                    remote_name=selected_remote_name or remote or "selected remote",
                    exc=exc,
                )
            ) from exc
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_task_tokens_report(task_id, data, breakdown=normalized_by)


def _task_show_worktree_advisory(task: dict[str, Any], worktree: dict[str, Any]) -> dict[str, Any] | None:
    retarget = worktree.get("retarget") if isinstance(worktree.get("retarget"), dict) else {}
    if not retarget:
        return None
    target_base_line = (
        _normalize_text_value(retarget.get("target_base_line"))
        or _normalize_text_value(worktree.get("target_base_line"))
        or "main"
    )
    rebase_state = str(retarget.get("rebase_state") or worktree.get("rebase_state") or "idle")
    conflict_paths = [str(path).strip() for path in (retarget.get("rebase_conflict_paths") or []) if str(path).strip()]
    fork_snapshot_id = _normalize_text_value(retarget.get("fork_snapshot_id"))
    target_base_snapshot_id = _normalize_text_value(retarget.get("target_base_snapshot_id"))
    advisory: dict[str, Any] = {
        "worktree_name": _normalize_text_value(worktree.get("name")),
        "current_line": _normalize_text_value(worktree.get("current_line"))
        or _normalize_text_value(worktree.get("registered_line_name")),
        "target_base_line": target_base_line,
        "rebase_state": rebase_state,
        "needs_retarget": bool(retarget.get("needs_retarget") or worktree.get("needs_retarget")),
        "fork_snapshot_id": fork_snapshot_id,
        "target_base_snapshot_id": target_base_snapshot_id,
    }
    if rebase_state == "conflicted":
        sample = ", ".join(conflict_paths[:5]) or "resolve conflicts first"
        advisory.update(
            {
                "code": "conflicted_rebase",
                "status": "blocked",
                "summary": "Bound worktree rebase is paused on conflicts.",
                "detail": f"The bound worktree is paused on conflicted rebase paths: {sample}.",
                "command": "ait worktree rebase --continue",
                "abort_command": "ait worktree rebase --abort",
                "rebase_conflict_paths": conflict_paths,
            }
        )
        return advisory
    if advisory["needs_retarget"]:
        detail = (
            f"The bound worktree still forks from `{fork_snapshot_id or 'unknown'}` while "
            f"`{target_base_line}` now points at `{target_base_snapshot_id or 'unknown'}`."
        )
        advisory.update(
            {
                "code": "needs_retarget",
                "status": "stale",
                "summary": "Bound worktree needs rebase onto the current target base line.",
                "detail": detail,
                "command": f"ait worktree rebase --onto {target_base_line}",
            }
        )
        return advisory
    advisory.update(
        {
            "code": "current",
            "status": "current",
            "summary": "Bound worktree already matches the current target base line.",
            "detail": (
                f"The bound worktree already tracks `{target_base_line}` at "
                f"`{target_base_snapshot_id or 'unknown'}`."
            ),
            "command": None,
        }
    )
    return advisory


def _attach_task_show_worktree_state(ctx: RepoContext, data: dict[str, Any]) -> dict[str, Any]:
    task_id = _normalize_text_value((data or {}).get("task_id"))
    if task_id is None:
        return data
    repo_ctx = _task_worktree_repo_ctx(ctx)
    worktree = _find_bound_task_worktree(repo_ctx, task_id)
    if not isinstance(worktree, dict):
        return data
    enriched = dict(data)
    worktree_payload = _task_worktree_output(worktree)
    enriched["worktree"] = worktree_payload
    advisory = _task_show_worktree_advisory(enriched, worktree_payload)
    if advisory is not None:
        enriched["worktree_advisory"] = advisory
    return enriched


def _render_task_tokens_report(task_id: str, data: dict[str, Any], *, breakdown: str | None) -> None:
    summary_data = dict(data.get("summary") or {})
    task = dict(data.get("task") or {})
    scope = dict(data.get("scope") or {})
    summary = Table(title=f"task tokens {task_id}")
    summary.add_column("field")
    summary.add_column("value")
    summary.add_row("title", str(task.get("title") or ""))
    summary.add_row("status", str(task.get("status") or ""))
    summary.add_row("scope", f"{scope.get('mode') or 'unknown'} ({scope.get('repo_name') or ''})".strip())
    summary.add_row("prompt tokens", str(summary_data.get("prompt_tokens") or 0))
    summary.add_row("completion tokens", str(summary_data.get("completion_tokens") or 0))
    summary.add_row("total tokens", str(summary_data.get("total_tokens") or 0))
    summary.add_row("cached input tokens", str(summary_data.get("cached_input_tokens") or 0))
    summary.add_row("reasoning output tokens", str(summary_data.get("reasoning_output_tokens") or 0))
    summary.add_row("sessions", str(summary_data.get("session_count") or 0))
    summary.add_row("sessions with usage", str(summary_data.get("sessions_with_usage_count") or 0))
    summary.add_row("assistant replies", str(summary_data.get("assistant_reply_count") or 0))
    summary.add_row("metered replies", str(summary_data.get("metered_reply_count") or 0))
    summary.add_row("usage.last replies", str(summary_data.get("usage_last_reply_count") or 0))
    summary.add_row("direct usage replies", str(summary_data.get("direct_usage_reply_count") or 0))
    summary.add_row("payload usage replies", str(summary_data.get("payload_usage_reply_count") or 0))
    summary.add_row("missing usage replies", str(summary_data.get("missing_usage_reply_count") or 0))
    summary.add_row("models", ", ".join(summary_data.get("models") or []) or "(none)")
    rprint(summary)
    if breakdown is None:
        return
    if breakdown == "change":
        _render_task_tokens_rollup_table(
            title="change breakdown",
            rows=list(data.get("changes") or []),
            label_field="change_id",
        )
        return
    if breakdown == "worktree":
        _render_task_tokens_rollup_table(
            title="worktree breakdown",
            rows=list(data.get("worktrees") or []),
            label_field="worktree_name",
        )
        return
    if breakdown == "model":
        _render_task_tokens_rollup_table(
            title="model breakdown",
            rows=list(data.get("models") or []),
            label_field="model_name",
        )
        return
    _render_task_token_session_table(list(data.get("sessions") or []))


def _render_task_tokens_rollup_table(*, title: str, rows: list[dict[str, Any]], label_field: str) -> None:
    table = Table(title=title)
    table.add_column(label_field)
    table.add_column("sessions")
    table.add_column("assistant replies")
    table.add_column("metered replies")
    table.add_column("prompt")
    table.add_column("completion")
    table.add_column("total")
    for row in rows:
        table.add_row(
            str(row.get(label_field) or ""),
            str(row.get("session_count") or 0),
            str(row.get("assistant_reply_count") or 0),
            str(row.get("metered_reply_count") or 0),
            str(row.get("prompt_tokens") or 0),
            str(row.get("completion_tokens") or 0),
            str(row.get("total_tokens") or 0),
        )
    rprint(table)


def _render_task_token_session_table(rows: list[dict[str, Any]]) -> None:
    table = Table(title="session breakdown")
    table.add_column("session_id")
    table.add_column("kind")
    table.add_column("change")
    table.add_column("worktree")
    table.add_column("assistant replies")
    table.add_column("metered replies")
    table.add_column("prompt")
    table.add_column("completion")
    table.add_column("total")
    for row in rows:
        table.add_row(
            str(row.get("session_id") or ""),
            str(row.get("session_kind") or ""),
            str(row.get("change_id") or ""),
            str(row.get("worktree_name") or ""),
            str(row.get("assistant_reply_count") or 0),
            str(row.get("metered_reply_count") or 0),
            str(row.get("prompt_tokens") or 0),
            str(row.get("completion_tokens") or 0),
            str(row.get("total_tokens") or 0),
        )
    rprint(table)


@task_app.command(
    "audit",
    help="Summarize one task's readiness against a target line in one helper read-model view.",
    short_help="Summarize one task readiness in one helper view.",
)
def task_audit_cmd(
    task_id: str,
    target_line: str = typer.Option("main", "--target-line"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    """Summarize one task's readiness against a target line in one helper read-model view."""
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        local_task = None
        try:
            local_task = get_local_task(ctx, task_id)
        except KeyError:
            local_task = None
        local_draft_only = bool(
            local_task
            and _normalize_text_value(local_task.get("publication_state")) == "local_draft"
            and _normalize_text_value(local_task.get("published_remote_name")) is None
            and _normalize_text_value(local_task.get("published_task_id")) is None
        )
        if local_draft_only:
            data = _infer_local_task_audit(
                ctx,
                remote_row["url"],
                task_id,
                target_line=target_line,
                prefer_local=True,
            )
            data = dict(data)
            data["audit_source"] = {
                "mode": "local_draft",
                "detail": "Local draft task audit used local workflow records directly because this task has not been published to the remote workflow yet.",
                "remote_task_missing": True,
            }
        else:
            try:
                data = remote_read_task_audit(remote_row["url"], task_id, repo_name=repo_name, target_line=target_line)
            except RemoteError as exc:
                if not _remote_task_missing(remote_row["url"], task_id, repo_name=repo_name):
                    raise
                data = _infer_local_task_audit(ctx, remote_row["url"], task_id, target_line=target_line)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    summary = Table(title=f"task audit {task_id}")
    summary.add_column("field")
    summary.add_column("value")
    if data.get("audit_source"):
        source = data["audit_source"]
        summary.add_row("source", f"{source.get('mode')} - {source.get('detail')}")
    summary.add_row("title", str(data["task"]["title"]))
    summary.add_row("status", str(data["task"]["status"]))
    summary.add_row("workflow", f"{data['workflow']['state']} - {data['workflow']['reason']}")
    summary.add_row("verdict", str(data["summary"]["verdict"]))
    summary.add_row(
        "target",
        f"{data['target']['line_name']} ({data['target']['head_snapshot_id'] or 'no head snapshot'})",
    )
    summary.add_row("open changes", str(data["summary"]["open_change_count"]))
    summary.add_row("landed changes", str(data["summary"]["landed_change_count"]))
    summary.add_row("effective on target", str(data["summary"]["effective_on_target_change_count"]))
    summary.add_row("stale workflow", "yes" if data["summary"]["stale_workflow_records"] else "no")
    summary.add_row(
        "recommended action",
        f"{data['recommended_action']['label']} - {data['recommended_action']['detail']}",
    )
    rprint(summary)

    changes = Table(title="linked changes")
    changes.add_column("change_id")
    changes.add_column("status")
    changes.add_column("target_state")
    show_line_column = any(row.get("preferred_line") for row in data["changes"])
    if show_line_column:
        changes.add_column("line")
    changes.add_column("patchset")
    changes.add_column("revision")
    for row in data["changes"]:
        display_patchset = row.get("display_patchset") or {}
        preferred_line = row.get("preferred_line") or {}
        cells = [
            row["change"]["change_id"],
            row["change"]["status"],
            row["target_state"],
        ]
        if show_line_column:
            cells.append(preferred_line.get("line_name") or "")
        cells.extend(
            [
                display_patchset.get("patchset_id") or "",
                display_patchset.get("revision_snapshot_id") or "",
            ]
        )
        changes.add_row(*cells)
    rprint(changes)


def _remote_error_status_code(exc: RemoteError) -> int | None:
    message = str(exc)
    marker = " failed: "
    if marker not in message:
        return None
    status_text = message.split(marker, 1)[1].split(" ", 1)[0]
    return int(status_text) if status_text.isdigit() else None


def _local_workflow_record_exists(ctx: RepoContext, *, kind: str, workflow_id: str) -> bool:
    try:
        if kind == "task":
            get_local_task(ctx, workflow_id)
            return True
        if kind == "change":
            get_local_change(ctx, workflow_id)
            return True
    except KeyError:
        return False
    raise ValueError(f"Unsupported workflow scope kind: {kind}")


def _workflow_show_scope_error_message(
    ctx: RepoContext,
    *,
    kind: str,
    workflow_id: str,
    remote_name: str,
    exc: KeyError | RemoteError | ValueError,
) -> str:
    if not isinstance(exc, RemoteError) or _remote_error_status_code(exc) != 404:
        return str(exc)
    kind_label = "Task" if kind == "task" else "Change"
    if _local_workflow_record_exists(ctx, kind=kind, workflow_id=workflow_id):
        return (
            f"{kind_label} {workflow_id} was not found on remote {remote_name}, "
            "but a local workflow record exists. Retry with --local if you meant "
            "the local record, or publish it before using remote workflow commands."
        )
    return (
        f"{kind_label} {workflow_id} was not found on remote {remote_name}. "
        "If you meant a local workflow record, retry with --local."
    )


def _remote_task_missing(base_url: str, task_id: str, *, repo_name: str | None = None) -> bool:
    try:
        remote_get_task(base_url, task_id, repo_name=repo_name)
    except RemoteError as exc:
        return _remote_error_status_code(exc) == 404 or "Unknown task" in str(exc)
    return False


def _task_audit_id_tokens(workflow_id: str | None) -> list[str]:
    text = str(workflow_id or "").strip().lower()
    if not text:
        return []
    tokens = [text]
    if "-" in text:
        prefix, suffix = text.split("-", 1)
        if len(suffix) > 8:
            tokens.append(f"{prefix}-{suffix[:8]}")
    return list(dict.fromkeys(tokens))


_TASK_AUDIT_LINE_REASON_PRIORITY = {
    "change_session": 0,
    "task_session": 1,
    "change_id": 2,
    "task_id": 3,
}


def _task_audit_line_rank(reasons: list[str]) -> int:
    if not reasons:
        return 99
    return min(_TASK_AUDIT_LINE_REASON_PRIORITY.get(reason, 99) for reason in reasons)


def _remote_snapshot_ancestry(base_url: str, repo_name: str, snapshot_id: str | None) -> set[str]:
    ancestry: set[str] = set()
    current = snapshot_id
    while current:
        if current in ancestry:
            raise ValueError(f"Cycle detected in remote snapshot ancestry at {current}")
        ancestry.add(current)
        bundle = get_remote_snapshot(base_url, repo_name, current, include_content=False)
        current = bundle.get("parent_snapshot_id")
    return ancestry


def _task_audit_target_info(
    ctx: RepoContext,
    base_url: str,
    repo_name: str,
    target_line: str,
    *,
    prefer_local: bool = False,
) -> tuple[dict[str, Any], set[str]]:
    if not prefer_local:
        try:
            line = get_remote_line(base_url, repo_name, target_line)
            head_snapshot_id = line.get("head_snapshot_id")
            ancestry = _remote_snapshot_ancestry(base_url, repo_name, head_snapshot_id) if head_snapshot_id else set()
            return {
                "line_name": target_line,
                "head_snapshot_id": head_snapshot_id,
                "ancestor_snapshot_count": len(ancestry),
                "source": "remote",
            }, ancestry
        except RemoteError:
            pass
    line = get_line(ctx, target_line)
    head_snapshot_id = line.get("head_snapshot_id")
    ancestry = set(collect_snapshot_chain(ctx, head_snapshot_id)) if head_snapshot_id else set()
    return {
        "line_name": target_line,
        "head_snapshot_id": head_snapshot_id,
        "ancestor_snapshot_count": len(ancestry),
        "source": "local",
    }, ancestry


def _task_audit_candidate_lines(
    lines: list[dict[str, Any]],
    *,
    target_line: str,
    task_tokens: list[str],
    change_tokens: list[str],
    task_session_lines: set[str],
    change_session_lines: set[str],
) -> list[dict[str, Any]]:
    candidates: list[dict[str, Any]] = []
    for line in lines:
        line_name = line["line_name"]
        if line_name == target_line:
            continue
        lowered = line_name.lower()
        reasons: list[str] = []
        if line_name in change_session_lines:
            reasons.append("change_session")
        if line_name in task_session_lines:
            reasons.append("task_session")
        if any(token in lowered for token in change_tokens):
            reasons.append("change_id")
        if any(token in lowered for token in task_tokens):
            reasons.append("task_id")
        if not reasons:
            continue
        candidate = dict(line)
        candidate["match_reasons"] = reasons
        candidates.append(candidate)
    candidates.sort(key=lambda row: row.get("updated_at") or "", reverse=True)
    candidates.sort(key=lambda row: _task_audit_line_rank(row.get("match_reasons") or []))
    return candidates


def _infer_local_task_audit(
    ctx: RepoContext,
    base_url: str,
    task_id: str,
    *,
    target_line: str = "main",
    prefer_local: bool = False,
) -> dict[str, Any]:
    task = get_local_task(ctx, task_id)
    repo_name = task["repo_name"]
    if prefer_local:
        config = load_config(ctx)
        repository = {
            "repo_name": repo_name,
            "default_line": config.get("default_line") or "main",
        }
    else:
        try:
            repository = remote_get_repository(base_url, repo_name)
        except RemoteError:
            config = load_config(ctx)
            repository = {
                "repo_name": repo_name,
                "default_line": config.get("default_line") or "main",
            }
    target, target_ancestry = _task_audit_target_info(
        ctx,
        base_url,
        repo_name,
        target_line,
        prefer_local=prefer_local,
    )
    changes = [row for row in list_local_changes(ctx) if row["task_id"] == task_id]
    changes.sort(key=lambda item: item["created_at"])
    all_lines = list_lines(ctx)
    sessions = list_local_sessions(ctx)
    task_tokens = _task_audit_id_tokens(task_id)
    task_session_lines = {row["line_name"] for row in sessions if row.get("task_id") == task_id and row.get("line_name")}

    change_rows: list[dict[str, Any]] = []
    for change in changes:
        change_session_lines = {
            row["line_name"]
            for row in sessions
            if row.get("change_id") == change["change_id"] and row.get("line_name")
        }
        candidates = _task_audit_candidate_lines(
            all_lines,
            target_line=target_line,
            task_tokens=task_tokens,
            change_tokens=_task_audit_id_tokens(change["change_id"]),
            task_session_lines=task_session_lines,
            change_session_lines=change_session_lines,
        )
        preferred_line = candidates[0] if candidates else None
        preferred_snapshot_id = preferred_line.get("head_snapshot_id") if preferred_line else None
        on_target_candidate_count = sum(
            1 for row in candidates if row.get("head_snapshot_id") and row["head_snapshot_id"] in target_ancestry
        )
        effective_on_target = bool(preferred_snapshot_id and preferred_snapshot_id in target_ancestry)
        stale_workflow_record = effective_on_target and change["status"] not in {"landed", "archived"}
        missing_remote_record = True

        if change["status"] == "archived":
            target_state = "archived"
            target_reason = "This change is archived and no longer blocks task completion."
        elif effective_on_target:
            target_state = "merged_on_target_missing_remote"
            target_reason = (
                f"The preferred inferred line head is already reachable from {target_line}, "
                "but the remote workflow record is missing."
            )
        elif on_target_candidate_count > 0:
            target_state = "ambiguous_line_candidates"
            target_reason = (
                f"{on_target_candidate_count} lower-confidence candidate line(s) appear on {target_line}, "
                "but the preferred inferred line does not."
            )
        elif preferred_line is None:
            target_state = "no_line_evidence"
            target_reason = "No local line could be linked to this change strongly enough to infer target-line reachability."
        elif not preferred_snapshot_id:
            target_state = "line_missing_head"
            target_reason = "The preferred inferred line does not currently have a head snapshot."
        else:
            target_state = "not_on_target"
            target_reason = f"The preferred inferred line head is not reachable from {target_line}."

        change_rows.append(
            {
                "change": change,
                "current_patchset": None,
                "selected_patchset": None,
                "display_patchset": None
                if preferred_snapshot_id is None
                else {
                    "patchset_id": None,
                    "revision_snapshot_id": preferred_snapshot_id,
                },
                "landing_summary": None,
                "effective_on_target": effective_on_target,
                "stale_workflow_record": stale_workflow_record,
                "missing_remote_record": missing_remote_record,
                "target_state": target_state,
                "target_reason": target_reason,
                "preferred_line": None
                if preferred_line is None
                else {
                    "line_name": preferred_line["line_name"],
                    "head_snapshot_id": preferred_line.get("head_snapshot_id"),
                    "status": preferred_line.get("status") or "active",
                    "match_reasons": preferred_line.get("match_reasons") or [],
                },
                "candidate_lines": [
                    {
                        "line_name": row["line_name"],
                        "head_snapshot_id": row.get("head_snapshot_id"),
                        "status": row.get("status") or "active",
                        "match_reasons": row.get("match_reasons") or [],
                    }
                    for row in candidates
                ],
            }
        )

    open_change_count = sum(1 for row in changes if row["status"] not in {"landed", "archived"})
    landed_change_count = sum(1 for row in changes if row["status"] == "landed")
    effective_on_target_count = sum(1 for row in change_rows if row["effective_on_target"])
    open_on_target_count = sum(
        1
        for row in change_rows
        if row["effective_on_target"] and row["change"]["status"] not in {"landed", "archived"}
    )
    stale_workflow_count = sum(1 for row in change_rows if row["stale_workflow_record"])
    ambiguous_line_count = sum(1 for row in change_rows if row["target_state"] == "ambiguous_line_candidates")
    line_evidence_count = sum(1 for row in change_rows if row["preferred_line"] is not None)
    effectively_complete_on_target = bool(change_rows) and all(
        row["change"]["status"] == "archived" or row["effective_on_target"] for row in change_rows
    ) and any(row["effective_on_target"] for row in change_rows)

    if task["status"] == "completed":
        verdict = "task_completed"
        workflow = {"state": "task_completed", "reason": "The local task is already completed."}
        recommended_action = {
            "code": "none",
            "label": "No action required",
            "detail": "The local task is already completed.",
            "change_id": None,
        }
    elif is_task_abandoned_status(task["status"]):
        verdict = "task_abandoned"
        workflow = {"state": "task_abandoned", "reason": "The local task is already abandoned."}
        recommended_action = {
            "code": "none",
            "label": "No action required",
            "detail": "The local task is already abandoned.",
            "change_id": None,
        }
    elif is_task_later_promotion_excluded_status(task["status"]):
        verdict = "task_later_promotion_excluded"
        workflow = {
            "state": "task_later_promotion_excluded",
            "reason": "The local task has already been excluded from later promotion.",
        }
        recommended_action = {
            "code": "none",
            "label": "No action required",
            "detail": "The local task has already been excluded from later promotion.",
            "change_id": None,
        }
    elif not change_rows:
        verdict = "no_changes"
        workflow = {"state": "planning", "reason": "No linked local changes exist yet."}
        recommended_action = {
            "code": "create_change",
            "label": "Create a change",
            "detail": "This task has no linked local changes yet.",
            "change_id": None,
        }
    elif effectively_complete_on_target:
        verdict = "workflow_missing_on_target"
        workflow = {
            "state": "workflow_missing_on_target",
            "reason": f"Linked changes already appear on {target_line}, but the remote task record is missing.",
        }
        recommended_action = {
            "code": "reconcile_workflow_records",
            "label": "Reconcile workflow records",
            "detail": f"The remote task record is missing, but local line evidence indicates this task is already absorbed into {target_line}.",
            "change_id": None,
        }
    elif ambiguous_line_count > 0:
        verdict = "needs_line_inspection"
        workflow = {
            "state": "needs_line_inspection",
            "reason": "Task audit found multiple candidate lines and could not safely infer closure from the preferred line alone.",
        }
        recommended_action = {
            "code": "inspect_candidate_lines",
            "label": "Inspect candidate lines",
            "detail": "Review the inferred local line candidates before deciding whether to reconcile or continue the task.",
            "change_id": None,
        }
    elif open_on_target_count > 0:
        verdict = "partially_on_target"
        workflow = {
            "state": "partially_on_target",
            "reason": f"Some inferred local line heads already appear on {target_line}, while other linked changes still need work.",
        }
        recommended_action = {
            "code": "inspect_stale_workflow",
            "label": "Inspect stale workflow state",
            "detail": "At least one inferred local line head is already on target, but the task is not fully absorbed.",
            "change_id": None,
        }
    else:
        verdict = "not_landed_on_target"
        workflow = {
            "state": "in_progress",
            "reason": f"No inferred local line head for this task is reachable from {target_line}.",
        }
        open_change = next((row["change"]["change_id"] for row in change_rows if row["change"]["status"] not in {"landed", "archived"}), None)
        recommended_action = {
            "code": "continue_task_work",
            "label": "Continue task work",
            "detail": "Keep working on the task and publish or repair workflow records once reviewable work exists.",
            "change_id": open_change,
        }

    updated_candidates = [task["updated_at"], *[change["updated_at"] for change in changes]]
    return {
        "task": task,
        "repository": repository,
        "workflow": workflow,
        "queue_workflow": workflow,
        "next_action": recommended_action,
        "recommended_action": recommended_action,
        "audit_source": {
            "mode": "local_fallback",
            "detail": "Remote task audit could not load the task, so local workflow records and line ancestry were used as read-only evidence.",
            "remote_task_missing": True,
        },
        "target": target,
        "summary": {
            "change_count": len(change_rows),
            "open_change_count": open_change_count,
            "landed_change_count": landed_change_count,
            "patchset_count": 0,
            "effective_on_target_change_count": effective_on_target_count,
            "open_on_target_change_count": open_on_target_count,
            "stale_workflow_change_count": stale_workflow_count,
            "ready_to_complete": False,
            "effectively_complete_on_target": effectively_complete_on_target,
            "stale_workflow_records": stale_workflow_count > 0,
            "missing_remote_change_count": sum(1 for row in change_rows if row["missing_remote_record"]),
            "line_evidence_change_count": line_evidence_count,
            "ambiguous_line_change_count": ambiguous_line_count,
            "verdict": verdict,
        },
        "changes": change_rows,
    }


@task_app.command("backfill-sessions", help="Create missing remote task_run sessions for existing tasks.", short_help="Backfill missing task sessions.")
def task_backfill_sessions(
    task_id: Optional[str] = typer.Option(None, "--task", help="Limit the backfill to one remote task id or repo-scoped task ref."),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_TARGET_DEFAULT_HELP),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        payload = remote_backfill_task_tracking_sessions(
            remote_row["url"],
            repo_name,
            task_id=task_id,
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(payload, json_output)


def _task_close_action(task_id: str, status: str, local: bool, remote: Optional[str], json_output: bool) -> None:
    ctx = _ctx()
    review_session = None
    bound_worktree_cleanup: dict[str, Any] | None = None
    try:
        use_local = _workflow_uses_local_scope(ctx, kind="task", local=local, remote_name=remote)
        if _task_tracking_enabled(ctx):
            review_session = _resolve_task_review_session(ctx, task_id, local=use_local, remote_name=remote)
            if review_session is None:
                raise ValueError(
                    f"Task tracking is enabled, but task {task_id} has no tracked session to review before close."
                )
        if use_local:
            data = close_local_task(ctx, task_id, status)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            data = remote_close_task(remote_row["url"], task_id, status, repo_name=repo_name)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if review_session is not None:
        try:
            tracking = _finalize_task_close_tracking(ctx, review_session, task_id=task_id, task_status=status)
        except (KeyError, RemoteError, ValueError) as exc:
            raise click.ClickException(
                f"Task {task_id} was closed as {status}, but the tracked session close-out failed: {exc}"
            ) from exc
        data = dict(data)
        data["tracking"] = tracking
        data["retrospective"] = tracking["retrospective"]
        data["improvement_plan"] = tracking["improvement_plan"]
    if status == "completed":
        try:
            bound_worktree_cleanup = _maybe_auto_remove_bound_worktree_after_task_complete(
                ctx,
                task_id=task_id,
                task_status=str(data.get("status") or status),
            )
        except (KeyError, ValueError, WorkspaceCommandBusyError) as exc:
            raise click.ClickException(
                f"Task {task_id} was closed as {status}, but bound worktree cleanup failed: {exc}"
            ) from exc
    if bound_worktree_cleanup is not None:
        data = dict(data)
        data["bound_worktree_cleanup"] = bound_worktree_cleanup
    _emit(data, json_output)


def _resolve_task_canceled_status(*, abandoned: bool, exclude_later_promotion: bool) -> str:
    if abandoned and exclude_later_promotion:
        raise ValueError("Use either --abandoned or --exclude-later-promotion, not both.")
    if exclude_later_promotion:
        return TASK_STATUS_LATER_PROMOTION_EXCLUDED
    return TASK_STATUS_ABANDONED


@task_app.command(
    "canceled",
    help=(
        "Close a task explicitly as abandoned work, or reclassify an unpublished completed local slice "
        "so it stops participating in later promotion."
    ),
    short_help="Close as abandoned or excluded.",
)
def task_canceled(
    task_id: str,
    abandoned: bool = typer.Option(False, "--abandoned", help="Close the task as abandoned work. This is the default when no explicit close mode is selected."),
    exclude_later_promotion: bool = typer.Option(False, "--exclude-later-promotion", help="Reclassify an unpublished completed local landed slice so repo-wide later promotion will skip it."),
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    json_output: bool = typer.Option(False, "--json"),
):
    status = _resolve_task_canceled_status(
        abandoned=abandoned,
        exclude_later_promotion=exclude_later_promotion,
    )
    if status == TASK_STATUS_LATER_PROMOTION_EXCLUDED and not local:
        raise typer.BadParameter("--exclude-later-promotion only applies to local unpublished task lineage.")
    _task_close_action(task_id, status, local, remote, json_output)


@task_app.command(
    "restart",
    help=(
        "Restore mistakenly canceled task lineage back to an active state. "
        "When the task has exactly one archived change and no still-open changes, restart also reopens that "
        "archived change so land/workflow can continue on the same lineage."
    ),
    short_help="Restart canceled task lineage.",
)
def task_restart(
    task_id: str,
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        use_local = _workflow_uses_local_scope(ctx, kind="task", local=local, remote_name=remote)
        if use_local:
            data = restart_local_task(ctx, task_id)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            data = remote_restart_task(remote_row["url"], task_id, repo_name=repo_name)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    payload = _maybe_attach_task_tracking(
        ctx,
        dict(data),
        local=use_local,
        remote_name=None if use_local else remote,
        worktree=None,
    )
    _emit(payload, json_output)


@task_app.command("complete", help="Mark a task complete after landed work or intentional local-only finish.", short_help="Mark a task complete.")
def task_complete(
    task_id: str,
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    json_output: bool = typer.Option(False, "--json"),
):
    _task_close_action(task_id, "completed", local, remote, json_output)


@task_app.command(
    "publish",
    help=(
        "Promote a local draft task into shared remote workflow state. "
        "This is the explicit local→remote exception path for in-progress local drafts, "
        "not the normal start for new `solo_remote` work or for already-landed `solo_local` slices."
    ),
)
def task_publish(
    task_id: str,
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_TARGET_DEFAULT_HELP),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        local_task = get_local_task(ctx, task_id)
        local_changes = [row for row in list_local_changes(ctx) if str(row.get("task_id") or "") == task_id]
        landed_local_changes = [row for row in local_changes if str(row.get("status") or "") == "landed"]
        if str(local_task.get("status") or "") == "completed" and landed_local_changes:
            landed_ids = [str(row.get("change_id") or "") for row in landed_local_changes if row.get("change_id")]
            landed_preview = ", ".join(landed_ids[:3])
            overflow = f" (+{len(landed_ids) - 3} more)" if len(landed_ids) > 3 else ""
            detail = f" landed local change(s) {landed_preview}{overflow}" if landed_preview else " landed local change lineage"
            raise ValueError(f"Local task {task_id} already has{detail}. {_COMPLETED_LOCAL_BATCH_PROMOTION_GUIDANCE}")
        remote_row, repo_name = _remote_tuple(ctx, remote)
        if local_task["repo_name"] != repo_name:
            raise KeyError(f"Local task {task_id} belongs to repository {local_task['repo_name']}, not {repo_name}")
        published_plan_id, published_revision_id, published_plan_item_ref = _published_local_task_plan_linkage(ctx, local_task)
        namespace_prefix = _effective_id_namespace_prefix(ctx)["value"]
        requested_task_id = _aligned_remote_publish_identity_request(
            remote_row["url"],
            repo_name,
            local_task,
            entity_type="task",
            namespace_prefix=namespace_prefix,
        )
        remote_task = remote_create_task(
            remote_row["url"],
            repo_name,
            local_task["title"],
            local_task["intent"],
            local_task["risk_tier"],
            task_id=requested_task_id,
            plan_id=published_plan_id,
            origin_plan_revision_id=published_revision_id,
            plan_item_ref=published_plan_item_ref,
        )
        published_task_id = _require_remote_workflow_identity_family(
            "task",
            remote_task,
            namespace_prefix=namespace_prefix,
            requested_id=requested_task_id,
        )
        published_task_id = (
            _normalize_text_value(remote_task.get("published_task_id"))
            or published_task_id
            or task_id
        )
        data = mark_local_task_published(
            ctx,
            task_id,
            remote_name=_normalize_text_value(remote_row.get("name")) or remote,
            published_task_id=published_task_id,
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)
