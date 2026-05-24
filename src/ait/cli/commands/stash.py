from __future__ import annotations

from ..shared import export_app_namespace

export_app_namespace(globals())


@stash_app.command(
    "save",
    help="Save temporary local-only WIP into a stash snapshot without advancing the current line head.",
    short_help="Save temporary local WIP.",
)
def stash_save(
    message: Optional[str] = typer.Option(None, "--message", help="Optional stash note for later inspection."),
    keep_workspace: bool = typer.Option(
        False,
        "--keep-workspace",
        help="Keep the current workspace content after saving instead of restoring the current line head.",
    ),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _run_locked_task_bound_authoring_command(
            ctx,
            "stash save",
            lambda: create_stash(ctx, message, keep_workspace=keep_workspace),
        )
    except (ValueError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@stash_app.command(
    "list",
    help="List temporary local-only stashes without touching workspace content.",
    short_help="List temporary stashes.",
)
def stash_list(json_output: bool = typer.Option(False, "--json")):
    rows = list_stashes(_ctx())
    if json_output:
        _emit(rows, True)
        return
    table = Table(title="ait stashes")
    table.add_column("stash_id")
    table.add_column("line")
    table.add_column("snapshot_id")
    table.add_column("files")
    table.add_column("created_at")
    table.add_column("message")
    for row in rows:
        table.add_row(
            row["stash_id"],
            row["source_line_name"],
            row["snapshot_id"],
            str(row["file_count"]),
            row["created_at"],
            row.get("message") or "",
        )
    rprint(table)


@stash_app.command(
    "show",
    help="Inspect one temporary stash without restoring its workspace content.",
    short_help="Inspect one stash.",
)
def stash_show(stash_id: str, json_output: bool = typer.Option(False, "--json")):
    try:
        data = get_stash(_ctx(), stash_id)
    except KeyError as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@stash_app.command(
    "apply",
    help="Restore workspace content from a stash without dropping the stash record.",
    short_help="Restore a stash into the workspace.",
)
def stash_apply(
    stash_id: str,
    force: bool = typer.Option(False, "--force", help="Overwrite unsaved workspace changes while applying the stash."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _run_locked_task_bound_authoring_command(
            ctx,
            "stash apply",
            lambda: apply_stash(ctx, stash_id, force=force, drop=False),
        )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@stash_app.command(
    "pop",
    help="Restore workspace content from a stash and then drop the stash record.",
    short_help="Restore and drop a stash.",
)
def stash_pop(
    stash_id: str,
    force: bool = typer.Option(False, "--force", help="Overwrite unsaved workspace changes while restoring the stash."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _run_locked_task_bound_authoring_command(
            ctx,
            "stash pop",
            lambda: apply_stash(ctx, stash_id, force=force, drop=True),
        )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@stash_app.command(
    "drop",
    help="Delete a stash record without restoring its workspace content.",
    short_help="Delete a stash.",
)
def stash_drop(stash_id: str, json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    try:
        data = _run_locked_task_bound_authoring_command(ctx, "stash drop", lambda: drop_stash(ctx, stash_id))
    except (KeyError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)
