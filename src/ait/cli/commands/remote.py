from __future__ import annotations

from ..shared import export_app_namespace

export_app_namespace(globals())

@remote_app.command(
    "add",
    help="Register one shared ait-native remote and optionally make it the default.",
    short_help="Register a remote.",
)
def remote_add(
    name: str,
    url: str,
    repo_name: Optional[str] = typer.Option(None, "--repo-name"),
    default: bool = typer.Option(False, "--default", help="Mark as default push/pull remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    data = add_remote(_ctx(), name, url, repo_name, make_default=default)
    _emit(data, json_output)


@remote_app.command(
    "list",
    help="List configured ait-native remotes and their default push/pull roles.",
    short_help="List remotes.",
)
def remote_list(json_output: bool = typer.Option(False, "--json")):
    rows = list_remotes(_ctx())
    if json_output:
        _emit(rows, True)
        return
    table = Table(title="ait native remotes")
    table.add_column("name")
    table.add_column("url")
    table.add_column("repo_name")
    table.add_column("default_push")
    table.add_column("default_pull")
    for row in rows:
        table.add_row(row["name"], row["url"], row.get("repo_name") or "", str(row["is_default_push"]), str(row["is_default_pull"]))
    rprint(table)


