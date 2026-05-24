from __future__ import annotations

from ..shared import export_app_namespace

export_app_namespace(globals())

@snapshot_app.command(
    "create",
    help="Freeze the current workspace line head as an immutable snapshot revision.",
    short_help="Freeze current workspace as immutable revision.",
)
def snapshot_create(message: Optional[str] = typer.Option(None, "--message"), json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    try:
        data = _run_locked_task_bound_authoring_command(ctx, "snapshot create", lambda: create_snapshot(ctx, message))
    except (ValueError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _touch_worktree_usage_safely(ctx)
    _emit(data, json_output)


@snapshot_app.command(
    "list",
    help="List immutable snapshots on the current line so operators can inspect revision history and choose ids for show or diff.",
    short_help="List immutable snapshots on the current line.",
)
def snapshot_list(json_output: bool = typer.Option(False, "--json")):
    rows = list_snapshots(_ctx())
    if json_output:
        _emit(rows, True)
        return
    table = Table(title="ait snapshots")
    table.add_column("snapshot_id")
    table.add_column("line")
    table.add_column("parent")
    table.add_column("files")
    table.add_column("message")
    for row in rows:
        table.add_row(row["snapshot_id"], row["line_name"], row["parent_snapshot_id"] or "", str(row["file_count"]), row.get("message") or "")
    rprint(table)


@snapshot_app.command(
    "show",
    help="Inspect one immutable snapshot, including its file manifest, parent, and message metadata.",
    short_help="Inspect one immutable snapshot.",
)
def snapshot_show(snapshot_id: str, json_output: bool = typer.Option(False, "--json")):
    """Inspect one immutable snapshot, including its file manifest, parent, and message metadata."""
    try:
        data = get_snapshot(_ctx(), snapshot_id)
    except KeyError as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@snapshot_app.command(
    "diff",
    help="Compare two immutable snapshots to inspect which files changed between revisions.",
    short_help="Compare two immutable snapshots.",
)
def snapshot_diff_cmd(
    old_snapshot_id: str,
    new_snapshot_id: str,
    include_text: bool = typer.Option(False, "--include-text", help="Include inline text diffs for changed text files"),
    max_bytes: int = typer.Option(
        DEFAULT_SNAPSHOT_DIFF_MAX_BYTES,
        "--max-bytes",
        min=1,
        help="Maximum blob size to decode for inline text diff",
    ),
    json_output: bool = typer.Option(False, "--json"),
):
    """Compare two immutable snapshots to inspect which files changed between revisions."""
    try:
        data = build_snapshot_diff(
            _ctx(),
            old_snapshot_id,
            new_snapshot_id,
            include_text=include_text,
            max_bytes=max_bytes,
        )
    except KeyError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return

    table = Table(title=f"ait snapshot diff {old_snapshot_id} → {new_snapshot_id}")
    table.add_column("status")
    table.add_column("path")
    table.add_column("+")
    table.add_column("-")
    for row in data.get("files", []):
        diff = row.get("diff") if isinstance(row, dict) else None
        diff = diff if isinstance(diff, dict) else {}
        table.add_row(
            str(row.get("status") or ""),
            str(row.get("path") or ""),
            str(diff.get("insertions") or ""),
            str(diff.get("deletions") or ""),
        )
    rprint(table)
    if include_text:
        for row in data.get("files", []):
            diff = row.get("diff") if isinstance(row, dict) else None
            if not isinstance(diff, dict) or not diff.get("text"):
                continue
            typer.echo(f"\n--- {row.get('path')} ---")
            typer.echo(str(diff["text"]))


@snapshot_app.command(
    "revert",
    help=(
        "Restore the current workspace back to the parent state of the current line head snapshot, "
        "without moving the line head or creating a new snapshot."
    ),
    short_help="Undo the current head snapshot into the workspace.",
)
def snapshot_revert_cmd(
    snapshot_id: str,
    force: bool = typer.Option(False, "--force", help="Overwrite selected unsaved workspace changes"),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview revert actions only"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "snapshot revert",
            lambda: local_revert_snapshot(
                ctx,
                snapshot_id,
                force=force,
                dry_run=dry_run,
            ),
        )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@snapshot_app.command(
    "replay",
    help=(
        "Replay the delta between one snapshot and its parent onto the current workspace for the selected target line, "
        "without moving the line head or creating a new snapshot."
    ),
    short_help="Replay one snapshot delta into the workspace.",
)
def snapshot_replay_cmd(
    snapshot_id: str,
    onto: str = typer.Option(..., "--onto", help="Replay onto this line; it must already be the current workspace line"),
    force: bool = typer.Option(False, "--force", help="Overwrite selected unsaved workspace changes"),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview replay actions only"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "snapshot replay",
            lambda: local_replay_snapshot(
                ctx,
                snapshot_id,
                onto_line=onto,
                force=force,
                dry_run=dry_run,
            ),
        )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)
