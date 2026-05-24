from __future__ import annotations

from ..local_promotion_stale_base_guard import _require_fresh_bound_task_worktree
from ..plan_markdown_authoring import _guard_markdown_task_dispatch
from ..workflow_authoring import _create_change_record, _workflow_uses_local_scope
from ..workflow_identity_helpers import (
    _aligned_remote_publish_identity_request,
    _require_remote_workflow_identity_family,
)
from ..shared import export_app_namespace
from ..shared import (
    COMPLETED_LOCAL_BATCH_PROMOTION_GUIDANCE as _COMPLETED_LOCAL_BATCH_PROMOTION_GUIDANCE,
    LOCAL_SCOPE_OVERRIDE_HELP as _LOCAL_SCOPE_OVERRIDE_HELP,
    REMOTE_SCOPE_OVERRIDE_HELP as _REMOTE_SCOPE_OVERRIDE_HELP,
    REMOTE_TARGET_DEFAULT_HELP as _REMOTE_TARGET_DEFAULT_HELP,
)

export_app_namespace(globals())

@change_app.command(
    "create",
    help=(
        "Open a change for an existing task when work reaches a review or shared boundary. "
        "In repositories configured with `workflow_mode=solo_remote`, omitting "
        "`--local` and `--remote` usually creates remote-backed lineage."
    ),
)
def change_create(
    task: str = typer.Option(..., "--task"),
    title: str = typer.Option(..., "--title"),
    base_line: Optional[str] = typer.Option(None, "--base-line"),
    risk: str = typer.Option("medium", "--risk"),
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    resolved_base_line = base_line or _default_line_name(ctx)
    try:
        _guard_active_root_worktree(ctx, "change create")
        _guard_readonly_root_main(ctx, "change create")
        _guard_task_bound_authoring(ctx, "change create", task_id=task)
        _guard_bound_task_match(ctx, task_id=task, command_name="change create")
        _guard_markdown_task_dispatch(
            ctx,
            plan_id=None,
            plan_revision_id=None,
            plan_item_ref=None,
            command_name="change create",
        )
        use_local = _workflow_uses_local_scope(ctx, kind="change", local=local, remote_name=remote)
        data = _create_change_record(
            ctx,
            task_id=task,
            title=title,
            base_line=resolved_base_line,
            risk=risk,
            local=use_local,
            remote_name=remote,
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@change_app.command("list", help="List change records in the effective local or remote workflow scope.", short_help="List change records in effective scope.")
def change_list(
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    json_output: bool = typer.Option(False, "--json"),
):
    """List change records in the effective local or remote workflow scope."""
    ctx = _ctx()
    try:
        use_local = _workflow_uses_local_scope(ctx, kind="change", local=local, remote_name=remote)
        if use_local:
            rows = list_local_changes(ctx)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            rows = remote_list_changes(remote_row["url"], repo_name)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    table = Table(title="ait native changes")
    table.add_column("change_id")
    table.add_column("title")
    table.add_column("lane")
    table.add_column("base_line")
    table.add_column("patchsets")
    table.add_column("status")
    if use_local:
        table.add_column("publication")
    for row in rows:
        cells = [row["change_id"], row["title"], row.get("lane") or "", row["base_line"], str(row["current_patchset_number"]), row["status"]]
        if use_local:
            cells.append(row["publication_state"])
        table.add_row(*cells)
    rprint(table)


@change_app.command("show", help="Inspect one change record in the effective local or remote workflow scope.", short_help="Inspect one change record.")
def change_show(
    change_id: str,
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    repo: Optional[str] = typer.Option(None, "--repo", help="Resolve the change ref within this remote repository when using repo-scoped workflow identity."),
    json_output: bool = typer.Option(False, "--json"),
):
    """Inspect one change record in the effective local or remote workflow scope."""
    ctx = _ctx()
    use_local = False
    selected_remote_name: str | None = None
    try:
        use_local = _workflow_uses_local_scope(ctx, kind="change", local=local, remote_name=remote)
        if use_local:
            data = get_local_change(ctx, change_id)
        else:
            remote_row, _ = _remote_tuple(ctx, remote)
            selected_remote_name = str(remote_row.get("name") or remote or "selected remote")
            data = remote_get_change(remote_row["url"], change_id, repo_name=repo)
    except (KeyError, RemoteError, ValueError) as exc:
        if not use_local:
            raise typer.BadParameter(
                _workflow_show_scope_error_message(
                    ctx,
                    kind="change",
                    workflow_id=change_id,
                    remote_name=selected_remote_name or remote or "selected remote",
                    exc=exc,
                )
            ) from exc
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@change_app.command(
    "revert",
    help=(
        "Restore the current workspace back to the change fork snapshot for paths changed by the current recorded change tip, "
        "without moving line heads or creating a new snapshot."
    ),
    short_help="Undo a change into the workspace.",
)
def change_revert(
    change_id: str,
    force: bool = typer.Option(False, "--force", help="Overwrite selected unsaved workspace changes"),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview revert actions only"),
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    repo: Optional[str] = typer.Option(None, "--repo", help="Resolve the change ref within this remote repository when using repo-scoped workflow identity."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    use_local = False
    selected_remote_name: str | None = None
    try:
        use_local = _workflow_uses_local_scope(ctx, kind="change", local=local, remote_name=remote)
        if use_local:
            change = get_local_change(ctx, change_id)
        else:
            remote_row, _ = _remote_tuple(ctx, remote)
            selected_remote_name = str(remote_row.get("name") or remote or "selected remote")
            change = remote_get_change(remote_row["url"], change_id, repo_name=repo)
    except (KeyError, RemoteError, ValueError) as exc:
        if not use_local:
            raise typer.BadParameter(
                _workflow_show_scope_error_message(
                    ctx,
                    kind="change",
                    workflow_id=change_id,
                    remote_name=selected_remote_name or remote or "selected remote",
                    exc=exc,
                )
            ) from exc
        raise typer.BadParameter(str(exc)) from exc
    try:
        data = _run_locked_workspace_command(
            ctx,
            "change revert",
            lambda: local_revert_change(
                ctx,
                change_id=str(change.get("change_id") or change_id),
                task_id=_normalize_text_value(change.get("task_id")),
                fork_snapshot_id=_normalize_text_value(change.get("fork_snapshot_id")),
                force=force,
                dry_run=dry_run,
            ),
        )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@change_app.command(
    "replay",
    help=(
        "Replay the delta from a change fork snapshot to its latest recorded local tip onto the current workspace for the selected target line, "
        "without moving line heads or creating a new snapshot."
    ),
    short_help="Replay one change delta into the workspace.",
)
def change_replay(
    change_id: str,
    onto: str = typer.Option(..., "--onto", help="Replay onto this line; it must already be the current workspace line"),
    force: bool = typer.Option(False, "--force", help="Overwrite selected unsaved workspace changes"),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview replay actions only"),
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    repo: Optional[str] = typer.Option(None, "--repo", help="Resolve the change ref within this remote repository when using repo-scoped workflow identity."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    use_local = False
    selected_remote_name: str | None = None
    try:
        use_local = _workflow_uses_local_scope(ctx, kind="change", local=local, remote_name=remote)
        if use_local:
            change = get_local_change(ctx, change_id)
        else:
            remote_row, _ = _remote_tuple(ctx, remote)
            selected_remote_name = str(remote_row.get("name") or remote or "selected remote")
            change = remote_get_change(remote_row["url"], change_id, repo_name=repo)
    except (KeyError, RemoteError, ValueError) as exc:
        if not use_local:
            raise typer.BadParameter(
                _workflow_show_scope_error_message(
                    ctx,
                    kind="change",
                    workflow_id=change_id,
                    remote_name=selected_remote_name or remote or "selected remote",
                    exc=exc,
                )
            ) from exc
        raise typer.BadParameter(str(exc)) from exc
    try:
        data = _run_locked_workspace_command(
            ctx,
            "change replay",
            lambda: local_replay_change(
                ctx,
                change_id=str(change.get("change_id") or change_id),
                onto_line=onto,
                task_id=_normalize_text_value(change.get("task_id")),
                fork_snapshot_id=_normalize_text_value(change.get("fork_snapshot_id")),
                force=force,
                dry_run=dry_run,
            ),
        )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@change_app.command("close", help="Archive a change that should stop instead of continuing toward publish or land.", short_help="Archive an abandoned change.")
def change_close(
    change_id: str,
    local: bool = typer.Option(False, "--local", help=_LOCAL_SCOPE_OVERRIDE_HELP),
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_SCOPE_OVERRIDE_HELP),
    json_output: bool = typer.Option(False, "--json"),
):
    """Archive a change that should stop instead of continuing toward publish or land."""
    ctx = _ctx()
    try:
        use_local = _workflow_uses_local_scope(ctx, kind="change", local=local, remote_name=remote)
        if use_local:
            data = close_local_change(ctx, change_id, "archived")
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            data = remote_close_change(remote_row["url"], change_id, "archived", repo_name=repo_name)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@change_app.command(
    "publish",
    help=(
        "Promote a local draft change into shared remote workflow state after its task is published. "
        "This is the explicit local→remote exception path for in-progress local drafts, "
        "not the normal start for new `solo_remote` work or for already-landed `solo_local` slices."
    ),
)
def change_publish(
    change_id: str,
    remote: Optional[str] = typer.Option(None, "--remote", help=_REMOTE_TARGET_DEFAULT_HELP),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        local_change = get_local_change(ctx, change_id)
        if local_change["status"] == "landed":
            raise ValueError(
                f"Local change {change_id} is already landed locally. {_COMPLETED_LOCAL_BATCH_PROMOTION_GUIDANCE}"
            )
        if local_change["status"] != "draft":
            raise ValueError(f"Local change {change_id} is {local_change['status']} and cannot be published")
        local_task = get_local_task(ctx, local_change["task_id"])
        if local_task["publication_state"] != "published":
            raise KeyError(f"Local task {local_task['task_id']} must be published before publishing change {change_id}")
        _require_fresh_bound_task_worktree(
            ctx,
            task_id=str(local_task["task_id"]),
            change_id=change_id,
            operation=f"publishing change {change_id}",
        )
        remote_row, repo_name = _remote_tuple(ctx, remote)
        if local_change["repo_name"] != repo_name:
            raise KeyError(f"Local change {change_id} belongs to repository {local_change['repo_name']}, not {repo_name}")
        remote_task_id = _normalize_text_value(local_task.get("published_task_id")) or local_task["task_id"]
        namespace_prefix = _effective_id_namespace_prefix(ctx)["value"]
        requested_change_id = _aligned_remote_publish_identity_request(
            remote_row["url"],
            repo_name,
            local_change,
            entity_type="change",
            namespace_prefix=namespace_prefix,
        )
        remote_change = remote_create_change(
            remote_row["url"],
            repo_name,
            remote_task_id,
            local_change["title"],
            local_change["base_line"],
            local_change["risk_tier"],
            change_id=requested_change_id,
            fork_snapshot_id=_normalize_text_value(local_change.get("fork_snapshot_id")),
            forked_from_line=_normalize_text_value(local_change.get("forked_from_line")) or local_change["base_line"],
        )
        published_change_id = _require_remote_workflow_identity_family(
            "change",
            remote_change,
            namespace_prefix=namespace_prefix,
            requested_id=requested_change_id,
        )
        published_change_id = (
            _normalize_text_value(remote_change.get("published_change_id"))
            or published_change_id
            or change_id
        )
        data = mark_local_change_published(
            ctx,
            change_id,
            remote_name=_normalize_text_value(remote_row.get("name")) or remote,
            published_change_id=published_change_id,
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)
