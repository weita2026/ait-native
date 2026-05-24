from __future__ import annotations

from ..shared import export_app_namespace

export_app_namespace(globals())

@stack_app.command("create")
def stack_create(
    title: str = typer.Option(..., "--title"),
    change: list[str] = typer.Option([], "--change"),
    landing_policy: str = typer.Option("ordered", "--landing-policy"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, repo_name = _sync_remote_repository_defaults(ctx, remote)
        data = remote_create_stack(remote_row["url"], repo_name, title, change, landing_policy)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@stack_app.command("list")
def stack_list(remote: Optional[str] = typer.Option(None, "--remote"), json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        rows = remote_list_stacks(remote_row["url"], repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    table = Table(title="ait native stacks")
    table.add_column("stack_id")
    table.add_column("title")
    table.add_column("status")
    table.add_column("changes")
    for row in rows:
        table.add_row(row["stack_id"], row["title"], row["status"], ", ".join(row.get("change_ids") or []))
    rprint(table)


@stack_app.command("show")
def stack_show(stack_id: str, remote: Optional[str] = typer.Option(None, "--remote"), json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_get_stack(remote_row["url"], stack_id)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@stack_app.command("add")
def stack_add(
    stack_id: str,
    change: str = typer.Option(..., "--change"),
    position: Optional[int] = typer.Option(None, "--position"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_stack_add_change(remote_row["url"], stack_id, change, position)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@stack_app.command("remove")
def stack_remove(
    stack_id: str,
    change: str = typer.Option(..., "--change"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_stack_remove_change(remote_row["url"], stack_id, change)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@stack_app.command("reorder")
def stack_reorder(
    stack_id: str,
    change: str = typer.Option(..., "--change"),
    position: int = typer.Option(..., "--position"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_stack_reorder_change(remote_row["url"], stack_id, change, position)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@stack_app.command("update")
def stack_update(
    stack_id: str,
    title: Optional[str] = typer.Option(None, "--title"),
    landing_policy: Optional[str] = typer.Option(None, "--landing-policy"),
    status: Optional[str] = typer.Option(None, "--status"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_update_stack(remote_row["url"], stack_id, title, landing_policy, status)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@stack_app.command("graph")
def stack_graph(stack_id: str, remote: Optional[str] = typer.Option(None, "--remote"), json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_get_stack_graph(remote_row["url"], stack_id)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)
