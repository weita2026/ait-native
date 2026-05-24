from __future__ import annotations

from ..shared import export_app_namespace

export_app_namespace(globals())

@workspace_app.command(
    "restore",
    help="Restore workspace files, or selected paths, from a line head or snapshot when operators need to materialize a known revision back into the working tree.",
    short_help="Restore workspace files or selected paths.",
)
def workspace_restore_cmd(
    snapshot_id: Optional[str] = typer.Option(None, "--snapshot", help="Restore a specific snapshot into the workspace"),
    line_name: Optional[str] = typer.Option(None, "--line", help="Restore the head snapshot of a line and switch the current line"),
    path: list[str] = typer.Option(
        None,
        "--path",
        help="Restore only selected workspace paths. Repeat --path to restore more than one path. Requires --snapshot or --line.",
    ),
    force: bool = typer.Option(False, "--force", help="Overwrite unsaved workspace changes"),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview restore actions without writing workspace files"),
    json_output: bool = typer.Option(False, "--json"),
):
    if snapshot_id is not None and line_name is not None:
        raise typer.BadParameter("Choose either --snapshot or --line")
    selected_paths = [item for item in path or [] if str(item).strip()]
    if selected_paths and snapshot_id is None and line_name is None:
        raise typer.BadParameter("Selected-path restore requires --snapshot or --line")
    ctx = _ctx()
    try:
        if selected_paths:
            data = _run_locked_workspace_command(
                ctx,
                "workspace restore",
                lambda: local_restore_workspace_paths(
                    ctx,
                    selected_paths,
                    snapshot_id=snapshot_id,
                    line_name=line_name,
                    force=force,
                    dry_run=dry_run,
                ),
            )
        else:
            data = _run_locked_workspace_command(
                ctx,
                "workspace restore",
                lambda: local_restore_workspace(
                    ctx,
                    snapshot_id=snapshot_id,
                    line_name=line_name,
                    force=force,
                    dry_run=dry_run,
                    switch_current_line=True,
                ),
            )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@workspace_app.command(
    "status",
    help="Inspect workspace drift against the current line head or an explicit snapshot before snapshotting, publishing, or restoring.",
    short_help="Inspect workspace drift against line or snapshot.",
)
def workspace_status_cmd(
    snapshot_id: Optional[str] = typer.Option(None, "--snapshot", help="Compare the workspace against a specific snapshot"),
    line_name: Optional[str] = typer.Option(None, "--line", help="Compare the workspace against the head snapshot of a line"),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        data = local_workspace_status(_ctx(), snapshot_id=snapshot_id, line_name=line_name)
    except (KeyError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_workspace_status(data)

