from __future__ import annotations

from ..shared import export_app_namespace

export_app_namespace(globals())

@ref_app.command(
    "list",
    help="List the starter line refs and the snapshots they currently target.",
    short_help="List refs.",
)
def ref_list(json_output: bool = typer.Option(False, "--json")):
    rows = [{"name": f"lines/{row['line_name']}", "target_snapshot_id": row["head_snapshot_id"]} for row in list_lines(_ctx())]
    _emit(rows, json_output)


@ref_app.command(
    "show",
    help="Inspect one starter line ref and the snapshot it currently targets.",
    short_help="Inspect one ref.",
)
def ref_show(name: Optional[str] = typer.Argument(None), json_output: bool = typer.Option(False, "--json")):
    if name is None:
        name = f"lines/{current_line(_ctx())}"
    if not name.startswith("lines/"):
        raise typer.BadParameter("Only lines/* refs are supported in this starter.")
    line_name = name.split("/", 1)[1]
    row = get_line(_ctx(), line_name)
    _emit({"name": name, "target_snapshot_id": row["head_snapshot_id"]}, json_output)


@ref_app.command(
    "history",
    help="Inspect the current target snapshot ancestry plus explicit ref-move breadcrumbs for one starter line ref.",
    short_help="Inspect ref ancestry and move breadcrumbs.",
)
def ref_history_cmd(
    name: Optional[str] = typer.Argument(None),
    limit: int = typer.Option(20, "--limit", min=1, help="Maximum number of snapshots and move events to include"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = local_ref_history(ctx, name=name, limit=limit)
    except (KeyError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return

    snapshots = Table(title=f"ait ref history {data['name']} snapshots")
    snapshots.add_column("pos")
    snapshots.add_column("snapshot_id")
    snapshots.add_column("parent")
    snapshots.add_column("message")
    for row in data.get("snapshots", []):
        snapshots.add_row(
            str(row.get("position_from_head")),
            str(row.get("snapshot_id") or ""),
            str(row.get("parent_snapshot_id") or ""),
            str(row.get("message") or ""),
        )
    rprint(snapshots)

    moves = Table(title=f"ait ref history {data['name']} moves")
    moves.add_column("event_id")
    moves.add_column("created_at")
    moves.add_column("target")
    moves.add_column("previous")
    for row in data.get("move_events", []):
        moves.add_row(
            str(row.get("event_id") or ""),
            str(row.get("created_at") or ""),
            str(row.get("target_snapshot_id") or ""),
            str(row.get("previous_target_snapshot_id") or ""),
        )
    rprint(moves)


@ref_app.command(
    "move",
    help="Move one starter line ref to a different snapshot.",
    short_help="Move a ref.",
)
def ref_move(name: str, snapshot_id: str, json_output: bool = typer.Option(False, "--json")):
    if not name.startswith("lines/"):
        raise typer.BadParameter("Only lines/* refs are supported in this starter.")
    line_name = name.split("/", 1)[1]
    ctx = _ctx()
    try:
        row = _run_locked_workspace_command(ctx, "ref move", lambda: move_ref(ctx, line_name, snapshot_id))
    except (KeyError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit({"name": name, "target_snapshot_id": row["head_snapshot_id"]}, json_output)

