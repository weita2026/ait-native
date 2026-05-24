from __future__ import annotations

from ..shared import export_app_namespace

export_app_namespace(globals())

@policy_app.command(
    "eval",
    help="Evaluate whether a patchset currently satisfies landing policy requirements.",
    short_help="Evaluate whether a patchset is ready to land.",
    hidden=True,
)
def policy_eval(patchset_id: str, remote: Optional[str] = typer.Option(None, "--remote"), json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        data = remote_evaluate_policy(remote_row["url"], patchset_id, repo_name=repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@policy_app.command(
    "show",
    help="Inspect the last evaluated policy decision and per-check readiness breakdown for a patchset.",
    short_help="Inspect the latest policy decision for a patchset.",
)
def policy_show(patchset_id: str, remote: Optional[str] = typer.Option(None, "--remote"), json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        data = remote_get_policy(remote_row["url"], patchset_id, repo_name=repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    table = Table(title=f"policy for {patchset_id}")
    table.add_column("lane")
    table.add_column("decision")
    table.add_column("evaluated_at")
    table.add_row(data["lane"], data["decision"], str(data.get("evaluated_at") or ""))
    rprint(table)
    if data.get("checks"):
        checks = Table(title="checks")
        checks.add_column("name")
        checks.add_column("status")
        checks.add_column("message")
        for row in data["checks"]:
            checks.add_row(row["name"], row["status"], row.get("message") or "")
        rprint(checks)


@policy_app.command(
    "waive",
    help="Create an explicit policy exception for a specific patchset rule when normal readiness requirements need a documented waiver.",
    short_help="Create a documented policy waiver.",
)
def policy_waive(
    patchset_id: str,
    rule: str = typer.Option(..., "--rule"),
    reason: str = typer.Option(..., "--reason"),
    expires_at: Optional[str] = typer.Option(None, "--expires-at"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        data = remote_create_waiver(remote_row["url"], patchset_id, rule, reason, expires_at, repo_name=repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)
