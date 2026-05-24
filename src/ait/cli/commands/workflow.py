from __future__ import annotations

from rich import print as rprint
from ait_protocol.common import (
    LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX,
    workflow_id_namespace_prefix_for_value,
)

from ..shared import export_app_namespace

export_app_namespace(globals())

@workflow_app.command(
    "guide",
    help="Show helper playbooks that collapse common inventory and landing command bursts.",
    short_help="Show workflow helper guides.",
)
def workflow_guide_cmd(
    topic: Optional[str] = typer.Argument(None),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        data = _workflow_guide_payload(topic)
    except KeyError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    rprint(_render_workflow_guide_text(data))


@workflow_app.command("help", hidden=True)
def workflow_help_cmd(
    topic: Optional[str] = typer.Argument(None),
    json_output: bool = typer.Option(False, "--json"),
):
    workflow_guide_cmd(topic=topic, json_output=json_output)

LOCAL_CHANGE_OPEN_STATUSES = {"draft", "active"}
LOCAL_CHANGE_TERMINAL_STATUSES = {"landed", "archived"}


def _snapshot_is_ancestor(ctx: RepoContext, ancestor_snapshot_id: str | None, descendant_snapshot_id: str | None) -> bool:
    resolved_ancestor = str(ancestor_snapshot_id or "").strip()
    resolved_descendant = str(descendant_snapshot_id or "").strip()
    if not resolved_ancestor or not resolved_descendant:
        return False
    if resolved_ancestor == resolved_descendant:
        return True
    return resolved_ancestor in collect_snapshot_chain(ctx, resolved_descendant)


def _workflow_land_local_stale_target_guidance(ctx: RepoContext, *, target_line: str) -> str:
    if ctx.is_worktree:
        try:
            worktree = local_get_worktree(ctx, refresh_status=False)
        except (KeyError, ValueError):
            worktree = None
        if isinstance(worktree, dict):
            current_line_name = str(worktree.get("current_line") or current_line(ctx) or "").strip()
            if current_line_name and current_line_name != target_line:
                return (
                    f" Run `ait worktree rebase --onto {target_line}` in the bound worktree and retry "
                    "`ait workflow land-local`."
                )
    return f" Rebase or retarget the current line onto `{target_line}` before retrying `ait workflow land-local`."


def _workflow_land_local_payload(
    ctx: RepoContext,
    *,
    change_id: str,
    target: str | None,
    snapshot: str | None,
) -> dict[str, Any]:
    resolved_change_id = str(change_id or "").strip()
    if not resolved_change_id:
        raise ValueError("change-id is required")
    change = get_local_change(ctx, resolved_change_id)
    change_status = str(change.get("status") or "")
    if change_status == "landed":
        raise ValueError(f"Local change {resolved_change_id} is already landed")
    if change_status in LOCAL_CHANGE_TERMINAL_STATUSES:
        raise ValueError(f"Local change {resolved_change_id} is {change_status} and cannot be landed")
    if change_status not in LOCAL_CHANGE_OPEN_STATUSES:
        raise ValueError(f"Local change {resolved_change_id} is {change_status} and cannot be landed")
    if change.get("publication_state") == "published":
        raise ValueError(
            f"Local change {resolved_change_id} has already been published; use `ait workflow land` for shared landing."
        )
    task_id = str(change.get("task_id") or "").strip()
    task = get_local_task(ctx, task_id)
    if task.get("publication_state") == "published":
        raise ValueError(f"Local task {task_id} has already been published; use `ait workflow land` for shared landing.")
    if task.get("status") not in {"active", "completed"}:
        raise ValueError(f"Local task {task_id} is {task.get('status')} and cannot be locally landed")

    status = repo_status(ctx)
    if status.get("workspace_dirty"):
        changed_count = status.get("workspace_changed_count")
        detail = f" ({changed_count} changed)" if changed_count is not None else ""
        raise ValueError(
            "Workspace is dirty"
            f"{detail}; run `ait snapshot create --message ...` before `ait workflow land-local`."
        )

    current_line_name = current_line(ctx)
    current_line_row = get_line(ctx, current_line_name)
    revision_snapshot_id = str(snapshot or current_line_row.get("head_snapshot_id") or "").strip()
    if not revision_snapshot_id:
        raise ValueError(
            f"Current line {current_line_name} has no head snapshot; pass --snapshot or create a snapshot first."
        )
    if not snapshot_exists(ctx, revision_snapshot_id):
        raise KeyError(f"Unknown snapshot: {revision_snapshot_id}")

    target_line = str(target or change.get("base_line") or "main").strip()
    if not target_line:
        target_line = "main"
    target_line_row = get_line(ctx, target_line)
    previous_target_head_snapshot_id = str(target_line_row.get("head_snapshot_id") or "").strip() or None
    if previous_target_head_snapshot_id and not _snapshot_is_ancestor(ctx, previous_target_head_snapshot_id, revision_snapshot_id):
        raise ValueError(
            f"Local land target `{target_line}` currently points at `{previous_target_head_snapshot_id}`, "
            f"but selected revision `{revision_snapshot_id}` does not descend from that head."
            f"{_workflow_land_local_stale_target_guidance(ctx, target_line=target_line)}"
        )

    open_peer_changes = [
        row
        for row in list_local_changes(ctx)
        if row.get("task_id") == task_id
        and row.get("change_id") != resolved_change_id
        and row.get("status") not in LOCAL_CHANGE_TERMINAL_STATUSES
    ]

    line = set_line_head(ctx, target_line, revision_snapshot_id)
    landed_change = land_local_change(
        ctx,
        resolved_change_id,
        target_line=target_line,
        landed_snapshot_id=revision_snapshot_id,
        pre_land_target_snapshot_id=previous_target_head_snapshot_id,
    )
    task_status_before = task.get("status")
    if open_peer_changes:
        resulting_task = task
    else:
        resulting_task = close_local_task(ctx, task_id, "completed")
    from ..task_dag_telegram_watch import trigger_local_task_dag_telegram_notifications

    telegram_graph_notifications = trigger_local_task_dag_telegram_notifications(
        ctx,
        repo_name=str(change.get("repo_name") or "").strip() or None,
        event_type="change.local_landed",
        entity_id=resolved_change_id,
    )
    repo_root_restore = _restore_repo_root_after_land(
        ctx,
        target_line=target_line,
        previous_head_snapshot_id=previous_target_head_snapshot_id,
    )
    if str(repo_root_restore.get("status") or "").strip() == "failed":
        bound_worktree_cleanup = {
            "status": "skipped",
            "reason": "repo_root_restore_failed",
            "task_id": task_id,
        }
    else:
        bound_worktree_cleanup = _auto_remove_bound_worktree_after_local_land(
            ctx,
            task_id=task_id,
            task_status=str(resulting_task.get("status") or ""),
            change_status=str(landed_change.get("status") or ""),
        )
    workspace_action = "unchanged"
    restore_status = str(repo_root_restore.get("status") or "").strip()
    if restore_status == "restored":
        workspace_action = "restored"
    elif restore_status == "failed":
        workspace_action = "failed"

    return {
        "change_id": resolved_change_id,
        "task_id": task_id,
        "target_line": target_line,
        "line_name": line.get("name") or target_line,
        "previous_target_head_snapshot_id": previous_target_head_snapshot_id,
        "landed_snapshot_id": revision_snapshot_id,
        "change_status": landed_change.get("status"),
        "task_status": resulting_task.get("status"),
        "task_status_before": task_status_before,
        "open_peer_change_count": len(open_peer_changes),
        "current_line": current_line_name,
        "workspace_action": workspace_action,
        "telegram_graph_notifications": telegram_graph_notifications,
        "repo_root_restore": repo_root_restore,
        "bound_worktree_cleanup": bound_worktree_cleanup,
    }


def _render_workflow_land_local_text(data: dict[str, Any]) -> str:
    cleanup = data.get("bound_worktree_cleanup") if isinstance(data.get("bound_worktree_cleanup"), dict) else {}
    lines = [
        f"landed local change {data.get('change_id')} onto {data.get('target_line')}",
        f"snapshot: {data.get('landed_snapshot_id')}",
        f"change: {data.get('change_status')}",
        f"task: {data.get('task_status')}",
        f"workspace: {data.get('workspace_action') or 'unchanged'}",
    ]
    if cleanup:
        lines.append(f"worktree cleanup: {cleanup.get('status')}")
    return "\n".join(lines)


@workflow_app.command("publish", hidden=True)
def workflow_publish_cmd(
    task_id: str = typer.Option(..., "--task", help="Remote task that should own the generated review change."),
    summary: str = typer.Option(..., "--summary", help="Patchset summary."),
    base_snapshot: Optional[str] = typer.Option(None, "--base-snapshot", help="Snapshot to use as the review base."),
    base_line: Optional[str] = typer.Option(None, "--base-line", help="Review-base line to create or reuse."),
    target_line: Optional[str] = typer.Option(None, "--target-line", help="Promote the final reviewable output directly against this remote target line."),
    change_title: Optional[str] = typer.Option(None, "--change-title", help="Generated change title. Defaults to the task title."),
    snapshot_message: Optional[str] = typer.Option(None, "--snapshot-message", help="Snapshot message if the workspace is dirty."),
    author_mode: AuthorMode | None = typer.Option(None, "--author-mode"),
    remote: Optional[str] = typer.Option(
        None,
        "--remote",
        help="Target remote. Single-change mode defaults to the repository default remote when omitted; batch later-promotion selectors require an explicit remote.",
    ),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "workflow publish",
            lambda: _workflow_publish_payload(
                ctx,
                task_id=task_id,
                summary=summary,
                remote_name=remote,
                base_snapshot_id=base_snapshot,
                base_line_name=base_line,
                target_line=target_line,
                change_title=change_title,
                snapshot_message=snapshot_message,
                author_mode=author_mode,
            ),
        )
    except (KeyError, RemoteError, ValueError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    patchset = data.get("patchset") if isinstance(data.get("patchset"), dict) else {}
    change = data.get("change") if isinstance(data.get("change"), dict) else {}
    rprint(
        f"Published patchset `{patchset.get('patchset_id') or 'unknown'}` "
        f"for change `{change.get('change_id') or 'unknown'}`."
    )


@workflow_app.command(
    "land-local",
    help="Run the local-only landing helper for one change onto a local target line.",
    short_help="Run the local-only landing helper.",
)
def workflow_land_local_cmd(
    change_id: str = typer.Argument(...),
    target: Optional[str] = typer.Option(None, "--target", help="Local target line to advance. Defaults to the change base line or main."),
    snapshot: Optional[str] = typer.Option(None, "--snapshot", help="Snapshot to land. Defaults to the current line head."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "workflow land-local",
            lambda: _workflow_land_local_payload(ctx, change_id=change_id, target=target, snapshot=snapshot),
        )
    except (KeyError, ValueError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    rprint(_render_workflow_land_local_text(data))


@workflow_app.command(
    "land",
    help=(
        "Show or apply the landing helper/orchestrator view for one change or "
        "batch-promote completed local work / converged graph outputs through the native remote land path."
    ),
    short_help="Show the landing helper/orchestrator.",
)
def workflow_land_cmd(
    change_id: Optional[str] = typer.Argument(None),
    all_completed_local: bool = typer.Option(
        False,
        "--all-completed-local",
        help=(
            "Promote every eligible completed local slice through the native remote land path. "
            "This is the public later-promotion surface for completed `solo_local` work."
        ),
    ),
    graph_run_session: Optional[str] = typer.Option(
        None,
        "--graph-run-session",
        help="Promote the converged candidate set owned by this task_graph_run session.",
    ),
    apply: bool = typer.Option(False, "--apply", help="Apply safe land-workflow actions sequentially until blocked or done."),
    snapshot_message: Optional[str] = typer.Option(None, "--snapshot-message", help="Snapshot message to use if --apply needs a fresh snapshot."),
    summary: Optional[str] = typer.Option(None, "--summary", help="Patchset summary to use if --apply needs to publish or refresh a patchset."),
    tests: str | None = typer.Option(None, "--tests"),
    lint: str | None = typer.Option(None, "--lint"),
    security: str | None = typer.Option(None, "--security"),
    license: str | None = typer.Option(None, "--license"),
    author_mode: AuthorMode | None = typer.Option(None, "--author-mode"),
    model: Optional[str] = typer.Option(None, "--model"),
    session: Optional[str] = typer.Option(None, "--session"),
    checkpoint: Optional[str] = typer.Option(None, "--checkpoint"),
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    review_message: Optional[str] = typer.Option(None, "--review-message"),
    target: Optional[str] = typer.Option(None, "--target"),
    mode: str = typer.Option("direct", "--mode"),
    remote: Optional[str] = typer.Option(
        None,
        "--remote",
        help="Target remote. Single-change mode defaults to the repository default remote when omitted; batch later-promotion selectors require an explicit remote.",
    ),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    resolved_change_id = str(change_id or "").strip().upper() or None
    local_change_route = False
    if not all_completed_local and graph_run_session is None and resolved_change_id is not None:
        route_prefix = workflow_id_namespace_prefix_for_value(
            resolved_change_id,
            "C",
            include_task_change_origins=True,
        )
        if route_prefix is not None and route_prefix.startswith(LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX):
            try:
                get_local_change(ctx, resolved_change_id)
            except KeyError:
                local_change_route = False
            else:
                local_change_route = True
    try:
        if all_completed_local or graph_run_session:
            if resolved_change_id is not None:
                raise ValueError("Batch workflow land does not accept CHANGE_ID.")
            data = (
                _workflow_land_batch_run(
                    ctx,
                    all_completed_local=all_completed_local,
                    graph_run_session_id=graph_run_session,
                    remote_name=remote,
                    summary=summary,
                    tests=tests,
                    lint=lint,
                    security=security,
                    license=license,
                    author_mode=author_mode,
                    model=model,
                    session=session,
                    checkpoint=checkpoint,
                    reviewer=reviewer,
                    review_message=review_message,
                    target=target,
                    mode=mode,
                )
                if apply
                else _workflow_land_batch_payload(
                    ctx,
                    all_completed_local=all_completed_local,
                    graph_run_session_id=graph_run_session,
                    remote_name=remote,
                    target=target,
                )
            )
        else:
            data = (
                (
                    _workflow_land_completed_local_apply(
                        ctx,
                        change_id=resolved_change_id,
                        remote_name=remote,
                        summary=summary,
                        tests=tests,
                        lint=lint,
                        security=security,
                        license=license,
                        author_mode=author_mode,
                        model=model,
                        session=session,
                        checkpoint=checkpoint,
                        reviewer=reviewer,
                        review_message=review_message,
                        target=target,
                        mode=mode,
                    )
                    if apply
                    else _workflow_land_completed_local_payload(
                        ctx,
                        change_id=resolved_change_id,
                        remote_name=remote,
                    )
                )
                if local_change_route
                else (
                    _workflow_land_apply(
                        ctx,
                        change_id=change_id,
                        patchset_id=None,
                        remote_name=remote,
                        snapshot_message=snapshot_message,
                        patchset_summary=summary,
                        tests=tests,
                        lint=lint,
                        security=security,
                        license=license,
                        author_mode=author_mode,
                        model=model,
                        session=session,
                        checkpoint=checkpoint,
                        reviewer=reviewer,
                        review_message=review_message,
                        target=target,
                        mode=mode,
                    )
                    if apply
                    else _workflow_land_payload(
                        ctx,
                        change_id=change_id,
                        patchset_id=None,
                        remote_name=remote,
                    )
                )
            )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    if all_completed_local or graph_run_session:
        rprint(
            "\n".join(
                [
                    "ait workflow land batch",
                    "",
                    f"- status: {data.get('status') or 'unknown'}",
                    f"- mode: {data.get('mode') or 'unknown'}",
                    f"- remote: {data.get('remote') or remote or 'unknown'}",
                    f"- completed: {data.get('completed_items') or 0} / {data.get('total_items') or 0}",
                    f"- ready: {data.get('ready_items') or 0}",
                    f"- blocked: {data.get('blocked_items') or 0}",
                ]
            )
        )
        return
    rprint(_render_workflow_land_text(data))
