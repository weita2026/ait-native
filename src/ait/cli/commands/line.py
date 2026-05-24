from __future__ import annotations

from ..shared import export_app_namespace

export_app_namespace(globals())

@line_app.command(
    "list",
    help="List local or remote lines and their current head snapshots.",
    short_help="List lines.",
)
def line_list(
    all_lines: bool = typer.Option(False, "--all", help="Include archived lines"),
    archived: bool = typer.Option(False, "--archived", help="Show only archived lines"),
    remote: Optional[str] = typer.Option(None, "--remote", help="List lines from the selected remote instead of the local store."),
    json_output: bool = typer.Option(False, "--json"),
):
    if all_lines and archived:
        raise typer.BadParameter("--all and --archived cannot be used together")
    ctx = _ctx()
    try:
        if remote:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            rows = remote_list_lines(remote_row["url"], repo_name)
        else:
            rows = list_lines(ctx)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if archived:
        rows = [row for row in rows if (row.get("status") or "active") == "archived"]
    elif not all_lines:
        rows = [row for row in rows if (row.get("status") or "active") != "archived"]
    if json_output:
        _emit(rows, True)
        return
    table = Table(title="ait lines")
    table.add_column("line")
    table.add_column("status")
    table.add_column("head_snapshot_id")
    table.add_column("archived_at")
    for row in rows:
        table.add_row(row["line_name"], row.get("status") or "active", row["head_snapshot_id"] or "", row.get("archived_at") or "")
    rprint(table)


@line_app.command(
    "create",
    help="Create a new line from the current head or an explicit snapshot.",
    short_help="Create a line.",
)
def line_create(
    name: str,
    from_snapshot: Optional[str] = typer.Option(None, "--from-snapshot"),
    switch: bool = typer.Option(False, "--switch", help="Also make the new line the current local line."),
    restore: bool = typer.Option(False, "--restore", help="With --switch, also restore the new line head into the workspace."),
    force: bool = typer.Option(False, "--force", help="Allow --restore to overwrite unsaved workspace changes."),
    json_output: bool = typer.Option(False, "--json"),
):
    if restore and not switch:
        raise typer.BadParameter("--restore requires --switch on `ait line create`.")
    if force and not restore:
        raise typer.BadParameter("--force only applies together with --restore")
    try:
        ctx = _ctx()
        def _create_line_command() -> dict[str, Any]:
            current_before = current_line(ctx)
            created = create_line(ctx, name, from_snapshot)
            if not switch:
                return created
            if restore:
                restored = local_restore_workspace(ctx, line_name=name, force=force, switch_current_line=True)
                restored["created_line"] = created
                restored["switched"] = True
                restored["restored"] = True
                return restored
            switch_line(ctx, name)
            payload = dict(created)
            payload["current_line_before"] = current_before
            payload["current_line"] = name
            payload["switched"] = True
            payload["restored"] = False
            return payload

        data = _run_locked_workspace_command(ctx, "line create", _create_line_command)
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@line_app.command(
    "switch",
    help="Switch the current local line and optionally restore its head snapshot.",
    short_help="Switch lines.",
)
def line_switch(
    name: str,
    restore: bool = typer.Option(False, "--restore", help="Also restore the selected line head into the workspace"),
    force: bool = typer.Option(False, "--force", help="Allow --restore to overwrite unsaved workspace changes"),
    json_output: bool = typer.Option(False, "--json"),
):
    if force and not restore:
        raise typer.BadParameter("--force only applies together with --restore")
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "line switch",
            lambda: local_restore_workspace(ctx, line_name=name, force=force, switch_current_line=True)
            if restore
            else switch_line(ctx, name),
        )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@line_app.command(
    "show",
    help="Inspect one line and its current head snapshot.",
    short_help="Inspect one line.",
)
def line_show(name: Optional[str] = typer.Argument(None), json_output: bool = typer.Option(False, "--json")):
    try:
        data = get_line(_ctx(), name)
    except KeyError as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@line_app.command(
    "archive",
    help="Archive a local line or close a shared remote line.",
    short_help="Archive a line.",
)
def line_archive(
    name: str,
    remote: Optional[str] = typer.Option(None, "--remote", help="Archive the shared remote line instead of the local line"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        if remote:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            data = close_remote_line(remote_row["url"], repo_name, name)
        else:
            data = _run_locked_workspace_command(ctx, "line archive", lambda: local_archive_line(ctx, name))
    except (KeyError, ValueError, RemoteError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@line_app.command(
    "cleanup-candidates",
    help="Inspect line cleanup candidates for review-base, review, and wip lifecycle lanes.",
    short_help="List line cleanup candidates.",
)
def line_cleanup_candidates(
    older_than: str = typer.Option("7d", "--older-than", help="Idle threshold like 7d, 12h, or 30m."),
    cleanup_kind: Optional[str] = typer.Option(
        None,
        "--kind",
        help="Filter to one lifecycle kind: review_base, review, or wip.",
    ),
    include_protected: bool = typer.Option(False, "--include-protected", help="Also list protected rows and their reasons."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        data = local_list_line_cleanup_candidates(
            _ctx(),
            older_than=older_than,
            include_protected=include_protected,
            cleanup_kind=cleanup_kind,
        )
    except (KeyError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_line_cleanup_candidates(data)


@line_app.command(
    "cleanup",
    help="Archive line cleanup candidates after an explicit confirmation.",
    short_help="Clean up lines.",
)
def line_cleanup(
    older_than: str = typer.Option("7d", "--older-than", help="Idle threshold like 7d, 12h, or 30m."),
    cleanup_kind: Optional[str] = typer.Option(
        None,
        "--kind",
        help="Filter to one lifecycle kind: review_base, review, or wip.",
    ),
    limit: Optional[int] = typer.Option(None, "--limit", min=1, help="Only archive the first N selected candidates."),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview cleanup without archiving anything."),
    yes: bool = typer.Option(False, "--yes", help="Confirm line cleanup."),
    json_output: bool = typer.Option(False, "--json"),
):
    if not dry_run and not yes:
        raise typer.BadParameter("Pass --yes to apply line cleanup, or use --dry-run to preview it.")
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "line cleanup",
            lambda: local_cleanup_lines(
                ctx,
                older_than=older_than,
                cleanup_kind=cleanup_kind,
                limit=limit,
                dry_run=dry_run,
            ),
        )
    except (KeyError, ValueError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_line_cleanup_report(data)
