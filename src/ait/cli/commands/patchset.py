from __future__ import annotations

from ..workflow_land_publish import _publish_patchset_from_current_line
from ..shared import export_app_namespace

export_app_namespace(globals())

@patchset_app.command(
    "publish",
    help="Publish the current base/revision snapshot pair for a change as a formal review patchset.",
    short_help="Publish the current review snapshot pair as a patchset.",
    hidden=True,
)
def patchset_publish(
    change: str = typer.Option(..., "--change"),
    summary: str = typer.Option(..., "--summary"),
    author_mode: AuthorMode | None = typer.Option(None, "--author-mode"),
    allow_empty: bool = typer.Option(False, "--allow-empty", help="Allow publishing when base and revision snapshots are identical."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        def _publish_patchset() -> dict:
            try:
                return _publish_patchset_from_current_line(
                    ctx,
                    change_id=change,
                    summary=summary,
                    remote_name=remote,
                    author_mode=author_mode,
                    allow_empty=allow_empty,
                )
            except RemoteError as exc:
                try:
                    local_change = get_local_change(ctx, change)
                except KeyError:
                    raise exc
                if local_change["publication_state"] != "published":
                    raise KeyError(f"Local change {change} is not published yet. Run `ait change publish {change}` first.") from exc
                raise exc

        data = _run_locked_task_bound_authoring_command(ctx, "patchset publish", _publish_patchset)
    except (KeyError, RemoteError, ValueError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _touch_worktree_usage_safely(ctx)
    _emit(data, json_output)


@patchset_app.command(
    "list",
    help="List published patchsets for one change so operators can inspect review history and choose the active candidate.",
    short_help="List published patchsets for a change.",
)
def patchset_list(
    change: str = typer.Option(..., "--change"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    repo: Optional[str] = typer.Option(None, "--repo", help="Resolve the change ref within this remote repository when using repo-scoped workflow identity."),
    json_output: bool = typer.Option(False, "--json"),
):
    """List published patchsets for one change so operators can inspect review history and choose the active candidate."""
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        rows = remote_list_patchsets(remote_row["url"], change, repo_name=repo)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    table = Table(title=f"patchsets for {change}")
    table.add_column("patchset_id")
    table.add_column("number")
    table.add_column("base")
    table.add_column("revision")
    table.add_column("state")
    table.add_column("evaluation")
    for row in rows:
        table.add_row(row["patchset_id"], str(row["patchset_number"]), row["base_snapshot_id"], row["revision_snapshot_id"], row["publish_state"], row.get("evaluation_state") or "")
    rprint(table)


@patchset_app.command(
    "show",
    help="Inspect one published patchset, including its base/revision snapshots and evaluation state.",
    short_help="Inspect one published patchset.",
)
def patchset_show(
    patchset_id: str,
    remote: Optional[str] = typer.Option(None, "--remote"),
    repo: Optional[str] = typer.Option(None, "--repo", help="Resolve the patchset within this remote repository when using repo-scoped workflow identity."),
    change: Optional[str] = typer.Option(None, "--change", help="Optional repo-scoped change ref when looking up a patchset by local patchset number."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        if repo and patchset_id.isdigit() and not change:
            raise typer.BadParameter("Repo-scoped numeric patchset refs require --change.")
        data = remote_get_patchset(remote_row["url"], patchset_id, repo_name=repo, change_ref=change)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@patchset_app.command(
    "select",
    help="Select which published patchset on a change should be treated as the active review and landing candidate.",
    short_help="Select the active patchset for a change.",
)
def patchset_select(
    patchset_id: str,
    change: str = typer.Option(..., "--change"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        data = remote_select_patchset(remote_row["url"], change, patchset_id, repo_name=repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@patchset_app.command(
    "rerun-ci",
    help="Queue or run the server-backed patchset CI slice for one published patchset.",
    short_help="Rerun patchset CI for one patchset.",
)
def patchset_rerun_ci(
    patchset_id: str,
    remote: Optional[str] = typer.Option(None, "--remote"),
    repo: Optional[str] = typer.Option(None, "--repo", help="Resolve the patchset within this remote repository when using repo-scoped workflow identity."),
    trigger: str = typer.Option("manual_rerun", "--trigger"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    remote_row: dict[str, Any] | None = None
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        data = remote_run_patchset_ci(remote_row["url"], patchset_id, trigger=trigger, repo_name=repo or repo_name)
    except (KeyError, RemoteError) as exc:
        if isinstance(exc, RemoteError) and remote_row is not None:
            raise typer.BadParameter(
                _ci_route_mismatch_guidance(
                    base_url=str(remote_row["url"]),
                    route_label="patchset_run_ci_route",
                    cli_hint="ait repo ci-capabilities --remote origin",
                    exc=exc,
                )
            ) from exc
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@patchset_app.command(
    "ci-status",
    help="Inspect the latest patchset CI status, suite outcomes, and rerun hint for one patchset.",
    short_help="Show patchset CI status.",
)
def patchset_ci_status_cmd(
    patchset_id: str,
    remote: Optional[str] = typer.Option(None, "--remote"),
    recent_limit: int = typer.Option(10, "--recent-limit", min=1, help="Number of recent patchset.ci jobs to include in the status payload."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        patchset = remote_get_patchset(remote_row["url"], patchset_id, repo_name=repo_name)
        data = remote_read_patchset_ci_status(
            remote_row["url"],
            str(patchset.get("patchset_id") or patchset_id),
            recent_limit=recent_limit,
        )
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    summary = Table(title=f"patchset CI {data['patchset_id']}")
    summary.add_column("field")
    summary.add_column("value")
    latest_job = data.get("latest_job") or {}
    summary.add_row("change", str(data.get("change_id") or ""))
    summary.add_row("tests", str(data.get("tests_status") or "pending"))
    summary.add_row("latest job", str(latest_job.get("job_id") or "none"))
    summary.add_row("job state", str(latest_job.get("state") or "none"))
    summary.add_row("suites", ", ".join(data.get("selected_suite_ids") or []) or "none")
    summary.add_row("blocking failures", ", ".join(data.get("blocking_failures") or []) or "none")
    summary.add_row("rerun", str((data.get("rerun") or {}).get("cli") or ""))
    rprint(summary)
    suite_results = list(data.get("suite_results") or [])
    if suite_results:
        suites = Table(title="suite results")
        suites.add_column("suite")
        suites.add_column("status")
        suites.add_column("blocking")
        suites.add_column("runner")
        suites.add_column("artifacts")
        for suite in suite_results:
            artifacts = ", ".join(
                str(payload.get("path") or key)
                for key, payload in dict(suite.get("artifacts") or {}).items()
                if isinstance(payload, dict) and payload.get("path")
            )
            suites.add_row(
                str(suite.get("suite_id") or ""),
                str(suite.get("status") or ""),
                "yes" if suite.get("blocking") else "no",
                str(suite.get("runner_kind") or ""),
                artifacts or "—",
            )
        rprint(suites)
    recent_jobs = list(data.get("recent_jobs") or [])
    if recent_jobs:
        jobs = Table(title="recent patchset ci jobs")
        jobs.add_column("job_id")
        jobs.add_column("state")
        jobs.add_column("tests")
        jobs.add_column("updated")
        for job in recent_jobs:
            jobs.add_row(
                str(job.get("job_id") or ""),
                str(job.get("state") or ""),
                str(job.get("tests_status") or ""),
                str(job.get("updated_at") or ""),
            )
        rprint(jobs)
