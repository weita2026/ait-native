from __future__ import annotations

from ..runtime_inspection_views import _local_auth_snapshot
from ..shared import export_app_namespace

export_app_namespace(globals())

@auth_app.command(
    "whoami",
    help="Inspect the current actor identity and effective roles for one repo.",
    short_help="Inspect current auth identity.",
)
def auth_whoami(
    remote: Optional[str] = typer.Option(None, "--remote"),
    repo: Optional[str] = typer.Option(None, "--repo"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = None
    try:
        ctx = _ctx()
        remote_row, repo_name = _remote_tuple(ctx, remote)
        data = remote_auth_whoami(remote_row["url"], repo or repo_name)
    except Exception:
        data = _local_auth_snapshot(ctx)
        if repo:
            data["repo_name"] = repo
    _emit(data, json_output)


@auth_app.command(
    "grant",
    help="Grant one or more shared roles to an actor on one remote repo.",
    short_help="Grant repo roles.",
)
def auth_grant(
    actor: str = typer.Option(..., "--actor"),
    role: list[str] = typer.Option(..., "--role"),
    repo: Optional[str] = typer.Option(None, "--repo"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        data = remote_grant_roles(remote_row["url"], repo or repo_name, actor, role)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@auth_app.command(
    "bindings",
    help="List the shared role bindings recorded for one remote repo.",
    short_help="List repo role bindings.",
)
def auth_bindings(
    repo: Optional[str] = typer.Option(None, "--repo"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        rows = remote_list_role_bindings(remote_row["url"], repo or repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    table = Table(title=f"role bindings for {repo or repo_name}")
    table.add_column("actor")
    table.add_column("role")
    table.add_column("created_at")
    for row in rows:
        table.add_row(row["actor_identity"], row["role"], row["created_at"])
    rprint(table)

