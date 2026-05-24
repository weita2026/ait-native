from __future__ import annotations

from ..line_transport_helpers import _fetch_line, _fetch_snapshot, _pull_line, _push_line
from ..runtime_inspection_views import _storage_validation_view
from ..shared import export_app_namespace

export_app_namespace(globals())

@repo_app.command("show")
def repo_show_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        data = remote_get_repository(remote_cfg["url"], repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@repo_app.command("storage")
def repo_storage_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        data = remote_get_repository_storage(remote_cfg["url"], repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@repo_app.command("validate")
def repo_validate_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        data = remote_get_repository_storage(remote_cfg["url"], repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(_storage_validation_view(data), json_output)


@repo_app.command("pack")
def repo_pack_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    repack: bool = typer.Option(False, "--repack"),
    max_members: int | None = typer.Option(None, "--max-members"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        data = remote_pack_repo(remote_cfg["url"], repo_name, repack=repack, max_members=max_members)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@repo_app.command("optimize")
def repo_optimize_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    repair: bool = typer.Option(True, "--repair/--no-repair"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        data = remote_optimize_repo(remote_cfg["url"], repo_name, repair=repair)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@repo_app.command("gc")
def repo_gc_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    prune_unreferenced: bool = typer.Option(True, "--prune-unreferenced/--no-prune-unreferenced"),
    prune_orphan_packs: bool = typer.Option(True, "--prune-orphan-packs/--no-prune-orphan-packs"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        data = remote_gc_repo(
            remote_cfg["url"],
            repo_name,
            prune_unreferenced=prune_unreferenced,
            prune_orphan_packs=prune_orphan_packs,
        )
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@repo_app.command("jobs")
def repo_jobs_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    state: Optional[str] = typer.Option(None, "--state"),
    limit: int = typer.Option(50, "--limit"),
    job_id: Optional[int] = typer.Option(None, "--job-id"),
    diagnostics: bool = typer.Option(False, "--diagnostics", help="Return operator recovery diagnostics instead of a raw job list."),
    stale_after_seconds: int = typer.Option(300, "--stale-after-seconds", help="Stale running-job threshold used by --diagnostics."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        if job_id is not None:
            if diagnostics:
                raise typer.BadParameter("--diagnostics cannot be combined with --job-id")
            data = remote_get_job(remote_cfg["url"], job_id)
        else:
            data = remote_list_jobs(
                remote_cfg["url"],
                repo_name,
                state=state,
                limit=limit,
                diagnostics=diagnostics,
                stale_after_seconds=stale_after_seconds,
            )
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@repo_app.command("metrics")
def repo_metrics_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    recent_jobs_limit: int = typer.Option(50, "--recent-jobs-limit", help="Number of recent jobs to include in the metrics payload."),
    stale_after_seconds: int = typer.Option(300, "--stale-after-seconds", help="Stale running-job threshold used by job outcome metrics."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, _repo_name = _remote_tuple(ctx, remote)
        data = remote_get_server_metrics(
            remote_cfg["url"],
            recent_jobs_limit=recent_jobs_limit,
            stale_after_seconds=stale_after_seconds,
        )
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@repo_app.command("readiness")
def repo_readiness_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    recent_jobs_limit: int = typer.Option(50, "--recent-jobs-limit", help="Number of recent jobs to include in the readiness payload."),
    stale_after_seconds: int = typer.Option(300, "--stale-after-seconds", help="Stale running-job threshold used by readiness job checks."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, _repo_name = _remote_tuple(ctx, remote)
        data = remote_get_server_readiness(
            remote_cfg["url"],
            recent_jobs_limit=recent_jobs_limit,
            stale_after_seconds=stale_after_seconds,
        )
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@repo_app.command("reconcile")
def repo_reconcile_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    repair: bool = typer.Option(False, "--repair"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        data = remote_reconcile_repo(remote_cfg["url"], repo_name, repair=repair)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@repo_app.command("run-ci")
def repo_run_ci_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    suite: list[str] = typer.Option(None, "--suite", help="Run one or more named repo CI suites."),
    plane: Optional[str] = typer.Option(None, "--plane", help="Run all repo CI suites on one plane (nightly, release, or post_land_regression)."),
    target_line: str = typer.Option("main", "--target-line"),
    trigger: str = typer.Option("manual_rerun", "--trigger"),
    selector: Optional[str] = typer.Option(None, "--selector", help="Override the task-batch selector when running the task_batch suite."),
    task_id: list[str] = typer.Option(None, "--task-id", help="Explicit task id(s) for task-batch selector overrides."),
    curated_corpus: Optional[str] = typer.Option(None, "--curated-corpus", help="Checked-in corpus name to use with the curated_corpus task-batch selector."),
    count: Optional[int] = typer.Option(None, "--count", min=1, help="Override the configured recent-task count for task-batch CI."),
    window_days: Optional[int] = typer.Option(None, "--window-days", min=1, help="Override the configured recent-task selection window for task-batch CI."),
    dependency_evidence: list[str] = typer.Option(None, "--dependency-evidence", help="Attach dependency evidence tokens for release-facing CI runs."),
    compliance_evidence: list[str] = typer.Option(None, "--compliance-evidence", help="Attach compliance evidence tokens for release-facing CI runs."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    remote_cfg: dict[str, Any] | None = None
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        data = remote_run_repo_ci(
            remote_cfg["url"],
            repo_name,
            suite_ids=list(suite or []),
            plane=plane,
            target_line=target_line,
            trigger=trigger,
            selector=selector,
            task_ids=list(task_id or []),
            curated_corpus=curated_corpus,
            count=count,
            window_days=window_days,
            dependency_evidence=list(dependency_evidence or []),
            compliance_evidence=list(compliance_evidence or []),
        )
    except (KeyError, RemoteError) as exc:
        if isinstance(exc, RemoteError) and remote_cfg is not None:
            raise typer.BadParameter(
                _ci_route_mismatch_guidance(
                    base_url=str(remote_cfg["url"]),
                    route_label="repo_run_ci_route",
                    cli_hint="ait repo ci-capabilities --remote origin",
                    exc=exc,
                )
            ) from exc
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@repo_app.command("ci-capabilities")
def repo_ci_capabilities_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        healthz = remote_get_server_health(remote_cfg["url"])
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    payload = {
        "repo_name": repo_name,
        "remote": remote_cfg.get("name") or remote,
        "healthz": healthz,
        "ci_capabilities": healthz.get("ci_capabilities") if isinstance(healthz, dict) else None,
        "ci_readiness": healthz.get("ci_readiness") if isinstance(healthz, dict) else None,
    }
    if json_output:
        _emit(payload, True)
        return
    capabilities = dict(payload.get("ci_capabilities") or {})
    readiness = dict(payload.get("ci_readiness") or {})
    summary = Table(title=f"ci capabilities · {repo_name}")
    summary.add_column("field")
    summary.add_column("value")
    summary.add_row("runtime generation", str(readiness.get("runtime_generation") or "unknown"))
    summary.add_row("patchset runCi", str(capabilities.get("patchset_run_ci_route")))
    summary.add_row("repo runCi", str(capabilities.get("repo_run_ci_route")))
    summary.add_row("patchset ci status", str(capabilities.get("patchset_ci_status_route")))
    summary.add_row("repo ci runs", str(capabilities.get("repo_ci_runs_route")))
    summary.add_row(
        "repo planes",
        ", ".join(str(item) for item in list(capabilities.get("supported_repo_planes") or [])) or "none",
    )
    summary.add_row(
        "task selectors",
        ", ".join(str(item) for item in list(capabilities.get("supported_task_batch_selectors") or [])) or "none",
    )
    summary.add_row("stale runtime hint", str(readiness.get("stale_runtime_hint") or ""))
    rprint(summary)


@repo_app.command("ci-runs")
def repo_ci_runs_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    limit: int = typer.Option(20, "--limit", min=1),
    plane: Optional[str] = typer.Option(None, "--plane", help="Filter runs by one plane, such as nightly, release, or post_land_regression."),
    suite: Optional[str] = typer.Option(None, "--suite", help="Filter runs to those that selected one suite id."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        data = remote_read_repository_ci_runs(
            remote_cfg["url"],
            repo_name,
            limit=limit,
            plane=plane,
            suite_id=suite,
        )
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    summary = Table(title=f"repo ci runs · {data['repo_name']}")
    summary.add_column("field")
    summary.add_column("value")
    run_summary = dict(data.get("summary") or {})
    summary.add_row("runs", str(data.get("count") or 0))
    summary.add_row("active", str(run_summary.get("active_runs") or 0))
    summary.add_row("failed", str(run_summary.get("failed_runs") or 0))
    summary.add_row("filter plane", str((data.get("filters") or {}).get("plane") or "any"))
    summary.add_row("filter suite", str((data.get("filters") or {}).get("suite_id") or "any"))
    rprint(summary)
    table = Table(title="recent ci runs")
    table.add_column("job_id")
    table.add_column("state")
    table.add_column("status")
    table.add_column("plane")
    table.add_column("suites")
    table.add_column("selected")
    table.add_column("rerun")
    table.add_column("updated")
    for item in list(data.get("items") or []):
        task_batch = dict(item.get("task_batch") or {})
        selected_detail = (
            f"tasks={task_batch.get('selected_task_count', 0)} lineage={task_batch.get('lineage_problem_count', 0)} behavior={task_batch.get('behavior_status', 'pending')}"
            if task_batch
            else "—"
        )
        table.add_row(
            str(item.get("job_id") or ""),
            str(item.get("state") or ""),
            str(item.get("status") or ""),
            ", ".join(item.get("selected_planes") or ([item["plane"]] if item.get("plane") else [])) or "—",
            ", ".join(item.get("selected_suite_ids") or []) or "—",
            selected_detail,
            str((item.get("rerun") or {}).get("cli") or ""),
            str(item.get("updated_at") or ""),
        )
    rprint(table)


@repo_app.command("migrate")
def repo_migrate_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_cfg, repo_name = _remote_tuple(ctx, remote)
        data = remote_run_repo_migrations(remote_cfg["url"], repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@app.command(
    "fetch",
    help="Advanced operator sync: refresh remote line or snapshot knowledge without moving local line heads or restoring workspace files.",
    short_help="Fetch remote state without moving local line heads.",
)
def fetch_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    line: Optional[str] = typer.Option(None, "--line", help="Refresh one remote line head without updating the local line head."),
    snapshot: Optional[str] = typer.Option(None, "--snapshot", help="Refresh one remote snapshot chain without updating any local line head."),
    json_output: bool = typer.Option(False, "--json"),
):
    if line is not None and snapshot is not None:
        raise typer.BadParameter("--line and --snapshot cannot be combined")
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "fetch",
            lambda: _fetch_snapshot(ctx, remote, snapshot)
            if snapshot is not None
            else _fetch_line(ctx, remote, line or current_line(ctx)),
        )
    except (KeyError, RemoteError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@app.command(
    "push",
    help="Advanced operator sync: upload local line snapshots to the selected remote.",
    short_help="Upload local line snapshots.",
)
def push_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    line: Optional[str] = typer.Option(None, "--line"),
    server_storage_mode: StorageIngestMode = typer.Option(
        StorageIngestMode.DEFAULT,
        "--server-storage-mode",
        help="Control how the remote server ingests new snapshot payloads: default, pack_full, or pack_delta.",
        case_sensitive=False,
    ),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "push",
            lambda: _push_line(ctx, remote, line or current_line(ctx), server_storage_mode=server_storage_mode),
        )
    except (KeyError, RemoteError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@app.command(
    "pull",
    help="Advanced operator sync: refresh local line heads and snapshots from the selected remote.",
    short_help="Refresh local line heads from a remote.",
)
def pull_cmd(
    remote: Optional[str] = typer.Option(None, "--remote"),
    line: Optional[str] = typer.Option(None, "--line"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(ctx, "pull", lambda: _pull_line(ctx, remote, line or current_line(ctx)))
    except (KeyError, RemoteError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)
