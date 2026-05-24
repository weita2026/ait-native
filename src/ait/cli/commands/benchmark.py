from __future__ import annotations

import json

from rich import print as rprint
from rich.table import Table

from ...local_first_final_land_benchmark import (
    render_local_first_final_land_markdown,
    run_local_first_final_land_benchmark,
)
from ...local_snapshot_performance_benchmark import (
    render_local_snapshot_performance_markdown,
    run_local_snapshot_performance_benchmark,
)
from ...static_web_benchmark import (
    render_static_web_task_markdown,
    run_static_web_task_benchmark,
)
from ...static_web_hardening_benchmark import (
    render_static_web_hardening_task_markdown,
    run_static_web_hardening_task_benchmark,
)
from ...strict_rerun_builder import (
    DEFAULT_STRICT_RERUN_SEED_MANIFEST,
    build_strict_rerun_fixture_bundle,
    load_strict_rerun_seed_manifest,
    strict_rerun_workloads_from_manifest,
)
from ...token_benchmark import (
    extract_codex_token_usage,
    import_codex_usage_into_manifest,
    inspect_token_savings_collection,
    render_token_savings_markdown,
    run_token_savings_benchmark,
)
from ..shared import export_app_namespace

export_app_namespace(globals())

@benchmark_app.command("static-web-hardening-task")
def benchmark_static_web_hardening_task_cmd(
    manifest: Path = typer.Option(..., "--manifest", help="JSON static web hardening benchmark manifest."),
    output_json: Optional[Path] = typer.Option(None, "--output-json", help="Write the evaluated benchmark payload to JSON."),
    output_markdown: Optional[Path] = typer.Option(None, "--output-markdown", help="Write a Markdown benchmark report."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        payload = run_static_web_hardening_task_benchmark(manifest)
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if output_json is not None:
        output_json.parent.mkdir(parents=True, exist_ok=True)
        output_json.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if output_markdown is not None:
        output_markdown.parent.mkdir(parents=True, exist_ok=True)
        output_markdown.write_text(render_static_web_hardening_task_markdown(payload), encoding="utf-8")
    if json_output:
        _emit(payload, True)
        return
    _render_static_web_hardening_task_benchmark(payload)


@benchmark_app.command("static-web-task")
def benchmark_static_web_task_cmd(
    manifest: Path = typer.Option(..., "--manifest", help="JSON static web task benchmark manifest."),
    output_json: Optional[Path] = typer.Option(None, "--output-json", help="Write the evaluated benchmark payload to JSON."),
    output_markdown: Optional[Path] = typer.Option(None, "--output-markdown", help="Write a Markdown benchmark report."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        payload = run_static_web_task_benchmark(manifest)
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if output_json is not None:
        output_json.parent.mkdir(parents=True, exist_ok=True)
        output_json.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if output_markdown is not None:
        output_markdown.parent.mkdir(parents=True, exist_ok=True)
        output_markdown.write_text(render_static_web_task_markdown(payload), encoding="utf-8")
    if json_output:
        _emit(payload, True)
        return
    _render_static_web_task_benchmark(payload)


@benchmark_app.command("token-savings")
def benchmark_token_savings_cmd(
    manifest: Path = typer.Option(..., "--manifest", help="JSON benchmark manifest."),
    output_json: Optional[Path] = typer.Option(None, "--output-json", help="Write the evaluated benchmark payload to JSON."),
    output_markdown: Optional[Path] = typer.Option(None, "--output-markdown", help="Write a Markdown benchmark report."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        payload = run_token_savings_benchmark(manifest)
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if output_json is not None:
        output_json.parent.mkdir(parents=True, exist_ok=True)
        output_json.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if output_markdown is not None:
        output_markdown.parent.mkdir(parents=True, exist_ok=True)
        output_markdown.write_text(render_token_savings_markdown(payload), encoding="utf-8")
    if json_output:
        _emit(payload, True)
        return
    _render_token_savings_summary(payload)


@benchmark_app.command("token-savings-status")
def benchmark_token_savings_status_cmd(
    manifest: Path = typer.Option(..., "--manifest", help="JSON benchmark manifest to inspect for measured-run readiness."),
    output_json: Optional[Path] = typer.Option(None, "--output-json", help="Write the collection-readiness payload to JSON."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        payload = inspect_token_savings_collection(manifest)
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if output_json is not None:
        output_json.parent.mkdir(parents=True, exist_ok=True)
        output_json.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if json_output:
        _emit(payload, True)
        return
    _render_token_savings_status(payload)


@benchmark_app.command("codex-usage")
def benchmark_codex_usage_cmd(
    session_jsonl: Path = typer.Option(..., "--session-jsonl", help="Codex session JSONL containing token_count events."),
    output_json: Optional[Path] = typer.Option(None, "--output-json", help="Write extracted Codex usage to JSON."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        payload = extract_codex_token_usage(session_jsonl)
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if output_json is not None:
        output_json.parent.mkdir(parents=True, exist_ok=True)
        output_json.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if json_output:
        _emit(payload, True)
        return
    _render_codex_usage(payload)


@benchmark_app.command("codex-fill-usage")
def benchmark_codex_fill_usage_cmd(
    manifest: Path = typer.Option(..., "--manifest", help="Measured benchmark manifest to update."),
    run_session: Optional[list[str]] = typer.Option(
        None,
        "--run-session",
        help="Run/session mapping in RUN_ID=SESSION_JSONL form. Repeat for multiple runs or repeat one run id to aggregate multiple measured sessions into one benchmark run.",
    ),
    run_role_session: Optional[list[str]] = typer.Option(
        None,
        "--run-role-session",
        help="Role-tagged mapping in RUN_ID:ROLE=SESSION_JSONL form. Use this to separate coordinator, batch_worker, or final_gate measured sessions inside one run.",
    ),
    output_manifest: Path = typer.Option(..., "--output-manifest", help="Write the updated measured manifest here."),
    quality: Optional[str] = typer.Option(None, "--quality", help="Set imported runs to this quality value, e.g. passed."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        payload = import_codex_usage_into_manifest(
            manifest,
            run_sessions=_parse_run_session_mappings(run_session),
            run_role_sessions=_parse_run_role_session_mappings(run_role_session),
            output_manifest_path=output_manifest,
            quality=quality,
        )
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(payload, True)
        return
    _render_codex_usage_import(payload)


@benchmark_app.command("strict-rerun")
def benchmark_strict_rerun_cmd(
    benchmark_id: str = typer.Option(..., "--benchmark-id", help="Benchmark id for the generated strict rerun manifest."),
    output_dir: Path = typer.Option(..., "--output-dir", help="Directory to populate with fixtures, evidence, and the pending manifest."),
    source_root: Path = typer.Option(Path("."), "--source-root", help="Source tree to copy into each fresh fixture root."),
    source_snapshot_id: Optional[str] = typer.Option(None, "--source-snapshot-id", help="Source snapshot id or equivalent provenance handle."),
    seed_manifest: Path = typer.Option(
        DEFAULT_STRICT_RERUN_SEED_MANIFEST,
        "--seed-manifest",
        help="Existing measured manifest whose workloads should seed the strict rerun scaffold.",
    ),
    workload: Optional[list[str]] = typer.Option(
        None,
        "--workload",
        help="Override workloads in WORKLOAD_ID|TITLE|CATEGORY|ACCEPTANCE form. Repeat to define multiple workloads.",
    ),
    description: Optional[str] = typer.Option(None, "--description", help="Description text for the generated manifest."),
    candidate_mode: Optional[list[str]] = typer.Option(
        None,
        "--candidate-mode",
        help="Candidate mode to prepare. Repeat for multiple modes. Defaults to ait_linear and ait_dag.",
    ),
    aggregate_candidate_mode: Optional[str] = typer.Option(
        None,
        "--aggregate-candidate-mode",
        help="Aggregate candidate mode for summary calculations. Defaults to ait_dag when present.",
    ),
    minimum_comparable_long_workloads: Optional[int] = typer.Option(
        None,
        "--minimum-comparable-long-workloads",
        help="Override the long-workload threshold stored in the generated manifest.",
    ),
    bootstrap_profile: str = typer.Option(
        "steady_state_task_cost",
        "--bootstrap-profile",
        help="Benchmark bootstrap profile label, e.g. steady_state_task_cost or first_use_bootstrap_cost.",
    ),
    ait_policy_profile: str = typer.Option("prototype", "--ait-policy-profile", help="Policy profile to use when bootstrapping ait fixtures."),
    default_line: str = typer.Option("main", "--default-line", help="Default ait line for bootstrapped ait fixtures."),
    default_author_mode: str = typer.Option(
        "ai_with_human_review",
        "--default-author-mode",
        help="Default ait author mode recorded into bootstrapped ait fixtures.",
    ),
    default_model: Optional[str] = typer.Option(None, "--default-model", help="Optional default model recorded into bootstrapped ait fixtures."),
    git_user_name: str = typer.Option("Benchmark Fixture", "--git-user-name", help="Local git user.name for prepared fixtures."),
    git_user_email: str = typer.Option(
        "benchmark@example.invalid",
        "--git-user-email",
        help="Local git user.email for prepared fixtures.",
    ),
    force: bool = typer.Option(False, "--force", help="Replace an existing output directory if it is not empty."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        workloads = _strict_rerun_workload_rows(seed_manifest, workload)
        payload = build_strict_rerun_fixture_bundle(
            benchmark_id=benchmark_id,
            output_dir=output_dir,
            source_root=source_root,
            workloads=workloads,
            source_snapshot_id=source_snapshot_id,
            seed_manifest_path=seed_manifest,
            description=description,
            candidate_modes=candidate_mode,
            aggregate_candidate_mode=aggregate_candidate_mode,
            minimum_comparable_long_workloads=minimum_comparable_long_workloads,
            bootstrap_profile=bootstrap_profile,
            ait_policy_profile=ait_policy_profile,
            default_line=default_line,
            default_author_mode=default_author_mode,
            default_model=default_model,
            git_user_name=git_user_name,
            git_user_email=git_user_email,
            force=force,
        )
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(payload, True)
        return
    rprint(
        "\n".join(
            [
                f"Prepared strict rerun bundle `{payload.get('benchmark_id')}`.",
                f"manifest: {payload.get('manifest_path')}",
                f"fixtures: {payload.get('fixture_bundle_path')}",
                f"readme: {payload.get('readme_path')}",
                f"fixtures prepared: {payload.get('fixture_count')}",
            ]
        )
    )


@benchmark_app.command("local-first-final-land")
def benchmark_local_first_final_land_cmd(
    manifest: Path = typer.Option(..., "--manifest", help="JSON local-first final-land benchmark manifest."),
    output_json: Optional[Path] = typer.Option(None, "--output-json", help="Write the evaluated benchmark payload to JSON."),
    output_markdown: Optional[Path] = typer.Option(None, "--output-markdown", help="Write a Markdown benchmark report."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        payload = run_local_first_final_land_benchmark(manifest)
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if output_json is not None:
        output_json.parent.mkdir(parents=True, exist_ok=True)
        output_json.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if output_markdown is not None:
        output_markdown.parent.mkdir(parents=True, exist_ok=True)
        output_markdown.write_text(render_local_first_final_land_markdown(payload), encoding="utf-8")
    if json_output:
        _emit(payload, True)
        return
    _render_local_first_final_land_benchmark(payload)


@benchmark_app.command("local-snapshot-performance")
def benchmark_local_snapshot_performance_cmd(
    manifest: Path = typer.Option(..., "--manifest", help="JSON local snapshot performance benchmark manifest."),
    output_json: Optional[Path] = typer.Option(None, "--output-json", help="Write the evaluated benchmark payload to JSON."),
    output_markdown: Optional[Path] = typer.Option(None, "--output-markdown", help="Write a Markdown benchmark report."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        payload = run_local_snapshot_performance_benchmark(manifest)
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if output_json is not None:
        output_json.parent.mkdir(parents=True, exist_ok=True)
        output_json.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if output_markdown is not None:
        output_markdown.parent.mkdir(parents=True, exist_ok=True)
        output_markdown.write_text(render_local_snapshot_performance_markdown(payload), encoding="utf-8")
    if json_output:
        _emit(payload, True)
        return
    _render_local_snapshot_performance_benchmark(payload)

def _render_token_savings_summary(payload: dict[str, Any]) -> None:
    aggregate = payload.get("aggregate") or {}
    candidate_mode = str(aggregate.get("aggregate_candidate_mode") or payload.get("aggregate_candidate_mode") or "ait_dag")
    savings_percent = aggregate.get("long_candidate_median_saving_percent")
    table = Table(title=f"token-savings benchmark · {payload.get('benchmark_id')}")
    table.add_column("field")
    table.add_column("value")
    table.add_row("evidence", str(payload.get("evidence_type") or ""))
    table.add_row("verdict", str(aggregate.get("verdict") or ""))
    table.add_row("aggregate candidate", candidate_mode)
    table.add_row("long candidate median savings", "n/a" if savings_percent is None else f"{savings_percent}%")
    table.add_row(
        "long comparable workloads",
        f"{aggregate.get('long_candidate_comparable_count')} / {payload.get('minimum_comparable_long_workloads')}",
    )
    table.add_row("claim ready", str(bool(aggregate.get("claim_ready"))))
    table.add_row("caveat", str(aggregate.get("claim_caveat") or ""))
    rprint(table)

    profiles = payload.get("profiles") if isinstance(payload.get("profiles"), list) else []
    if len(profiles) > 1:
        profile_table = Table(title="comparison profiles")
        profile_table.add_column("profile")
        profile_table.add_column("baseline")
        profile_table.add_column("aggregate candidate")
        profile_table.add_column("verdict")
        profile_table.add_column("claim target")
        profile_table.add_column("claim ready")
        for profile in profiles:
            profile_aggregate = profile.get("aggregate") if isinstance(profile.get("aggregate"), dict) else {}
            profile_table.add_row(
                str(profile.get("profile_id") or ""),
                str(profile.get("baseline_mode") or ""),
                str(profile_aggregate.get("aggregate_candidate_mode") or profile.get("aggregate_candidate_mode") or ""),
                str(profile_aggregate.get("verdict") or ""),
                str(bool(profile_aggregate.get("claim_target"))),
                str(bool(profile_aggregate.get("claim_ready"))),
            )
        rprint(profile_table)

    detail = Table(title="workload comparisons")
    detail.add_column("workload")
    detail.add_column("category")
    detail.add_column("mode")
    detail.add_column("baseline")
    detail.add_column("candidate")
    detail.add_column("saving")
    for workload in payload.get("workloads") or []:
        for comparison in workload.get("comparisons") or []:
            saving = comparison.get("saving_percent")
            detail.add_row(
                str(workload.get("workload_id") or ""),
                str(workload.get("category") or ""),
                str(comparison.get("mode") or ""),
                str(comparison.get("baseline_median_total_tokens") or ""),
                str(comparison.get("candidate_median_total_tokens") or ""),
                "" if saving is None else f"{saving}%",
            )
    rprint(detail)


def _render_token_savings_status(payload: dict[str, Any]) -> None:
    summary = payload.get("summary") or {}
    table = Table(title=f"token-savings collection · {payload.get('benchmark_id')}")
    table.add_column("field")
    table.add_column("value")
    for key in (
        "total_run_count",
        "measured_ready_count",
        "missing_run_count",
        "missing_usage_count",
        "pending_quality_count",
        "ready_to_report",
        "next_action",
    ):
        table.add_row(key, str(summary.get(key)))
    rprint(table)

    runs = Table(title="run readiness")
    runs.add_column("workload")
    runs.add_column("mode")
    runs.add_column("run_id")
    runs.add_column("usage")
    runs.add_column("quality")
    runs.add_column("ready")
    runs.add_column("missing")
    for row in payload.get("runs") or []:
        runs.add_row(
            str(row.get("workload_id") or ""),
            str(row.get("mode") or ""),
            str(row.get("run_id") or ""),
            "yes" if row.get("has_usage") else "no",
            str(row.get("quality") or ""),
            "yes" if row.get("measured_ready") else "no",
            str(row.get("missing_reason") or ""),
        )
    rprint(runs)


def _render_local_first_final_land_benchmark(payload: dict[str, Any]) -> None:
    aggregate = payload.get("aggregate") or {}
    table = Table(title=f"local-first final-land benchmark · {payload.get('benchmark_id')}")
    table.add_column("field")
    table.add_column("value")
    table.add_row("candidate mode", str(payload.get("candidate_mode") or ""))
    table.add_row("evidence", str(payload.get("evidence_type") or ""))
    table.add_row("verdict", str(aggregate.get("verdict") or ""))
    table.add_row("total runs", str(aggregate.get("total_run_count") or 0))
    table.add_row("landed rate", "n/a" if aggregate.get("landed_rate") is None else f"{aggregate.get('landed_rate')}%")
    table.add_row(
        "single-worker success rate",
        "n/a" if aggregate.get("single_worker_success_rate") is None else f"{aggregate.get('single_worker_success_rate')}%",
    )
    table.add_row(
        "stale recovery success rate",
        "n/a" if aggregate.get("stale_recovery_success_rate") is None else f"{aggregate.get('stale_recovery_success_rate')}%",
    )
    table.add_row("caveat", str(aggregate.get("claim_caveat") or ""))
    rprint(table)

    detail = Table(title="run detail")
    detail.add_column("workload")
    detail.add_column("run")
    detail.add_column("landed")
    detail.add_column("stale")
    detail.add_column("auto recovery")
    detail.add_column("operator recovery")
    detail.add_column("worker sessions")
    detail.add_column("remote changes")
    detail.add_column("tokens")
    for workload in payload.get("workloads") or []:
        for run in workload.get("runs") or []:
            usage = run.get("usage") or {}
            detail.add_row(
                str(workload.get("workload_id") or ""),
                str(run.get("run_id") or ""),
                "yes" if run.get("landed") else "no",
                str(run.get("stale_preflight") or "unknown"),
                "yes" if run.get("automatic_recovery_success") else "no",
                "yes" if run.get("operator_recovery_required") else "no",
                str(run.get("worker_session_count") or ""),
                str(run.get("remote_change_count") or ""),
                str(usage.get("total_tokens") or ""),
            )
    rprint(detail)


def _render_static_web_task_benchmark(payload: dict[str, Any]) -> None:
    aggregate = payload.get("aggregate") or {}
    table = Table(title=f"static web task benchmark · {payload.get('benchmark_id')}")
    table.add_column("field")
    table.add_column("value")
    table.add_row("workload kind", str(payload.get("workload_kind") or ""))
    table.add_row("baseline mode", str(payload.get("baseline_mode") or ""))
    table.add_row("aggregate candidate", str(aggregate.get("candidate_mode") or payload.get("aggregate_candidate_mode") or ""))
    table.add_row("evidence", str(payload.get("evidence_type") or ""))
    table.add_row("verdict", str(aggregate.get("verdict") or ""))
    table.add_row(
        "comparable runs",
        f"{aggregate.get('candidate_comparable_run_count')} / {payload.get('minimum_comparable_runs')}",
    )
    table.add_row("candidate median score", str(aggregate.get("candidate_median_score") or ""))
    table.add_row(
        "candidate pass rate",
        "n/a" if aggregate.get("candidate_pass_rate") is None else f"{aggregate.get('candidate_pass_rate')}%",
    )
    table.add_row("caveat", str(aggregate.get("claim_caveat") or ""))
    rprint(table)

    detail = Table(title="run detail")
    detail.add_column("workload")
    detail.add_column("run")
    detail.add_column("mode")
    detail.add_column("verdict")
    detail.add_column("score")
    detail.add_column("comparable")
    detail.add_column("elapsed")
    detail.add_column("total tokens")
    for workload in payload.get("workloads") or []:
        for run in workload.get("runs") or []:
            usage = run.get("usage") or {}
            detail.add_row(
                str(workload.get("workload_id") or ""),
                str(run.get("run_id") or ""),
                str(run.get("mode") or ""),
                str(run.get("pass_or_fail") or ""),
                str(run.get("score") or ""),
                "yes" if run.get("comparable") else "no",
                str(run.get("elapsed_seconds") or ""),
                str(usage.get("total_tokens") or ""),
            )
    rprint(detail)


def _render_static_web_hardening_task_benchmark(payload: dict[str, Any]) -> None:
    aggregate = payload.get("aggregate") or {}
    baseline_fixture = payload.get("baseline_fixture") if isinstance(payload.get("baseline_fixture"), dict) else {}
    table = Table(title=f"static web hardening benchmark · {payload.get('benchmark_id')}")
    table.add_column("field")
    table.add_column("value")
    table.add_row("workload kind", str(payload.get("workload_kind") or ""))
    table.add_row("comparison family", str(payload.get("comparison_family") or ""))
    table.add_row("bootstrap surface", str(payload.get("bootstrap_surface") or ""))
    table.add_row("baseline mode", str(payload.get("baseline_mode") or ""))
    table.add_row("baseline fixture snapshot", str(baseline_fixture.get("snapshot_id") or ""))
    table.add_row("baseline fixture digest", str(baseline_fixture.get("digest") or ""))
    table.add_row("aggregate candidate", str(aggregate.get("candidate_mode") or payload.get("aggregate_candidate_mode") or ""))
    table.add_row("evidence", str(payload.get("evidence_type") or ""))
    table.add_row("verdict", str(aggregate.get("verdict") or ""))
    table.add_row(
        "comparable runs",
        f"{aggregate.get('candidate_comparable_run_count')} / {payload.get('minimum_comparable_runs')}",
    )
    table.add_row("candidate median score", str(aggregate.get("candidate_median_score") or ""))
    table.add_row(
        "candidate pass rate",
        "n/a" if aggregate.get("candidate_pass_rate") is None else f"{aggregate.get('candidate_pass_rate')}%",
    )
    table.add_row("caveat", str(aggregate.get("claim_caveat") or ""))
    rprint(table)

    detail = Table(title="run detail")
    detail.add_column("workload")
    detail.add_column("run")
    detail.add_column("mode")
    detail.add_column("verdict")
    detail.add_column("score")
    detail.add_column("validation")
    detail.add_column("runtime closeout")
    detail.add_column("replay")
    detail.add_column("settings")
    detail.add_column("mobile")
    detail.add_column("total tokens")
    for workload in payload.get("workloads") or []:
        for run in workload.get("runs") or []:
            usage = run.get("usage") or {}
            detail.add_row(
                str(workload.get("workload_id") or ""),
                str(run.get("run_id") or ""),
                str(run.get("mode") or ""),
                str(run.get("pass_or_fail") or ""),
                str(run.get("score") or ""),
                "yes" if run.get("validation_check_passed") else "no",
                str(run.get("evaluator_runtime_closeout_status") or ""),
                str(run.get("replay_check_status") or ""),
                str(run.get("settings_check_status") or ""),
                str(run.get("mobile_input_check_status") or ""),
                str(usage.get("total_tokens") or ""),
            )
    rprint(detail)


def _render_codex_usage(payload: dict[str, Any]) -> None:
    table = Table(title="codex token usage")
    table.add_column("field")
    table.add_column("value")
    manifest_usage = payload.get("manifest_usage") or {}
    total_usage = payload.get("total_token_usage") or {}
    table.add_row("session", str(payload.get("session_jsonl_path") or ""))
    table.add_row("token events", str(payload.get("token_event_count") or 0))
    table.add_row("prompt/input tokens", str(manifest_usage.get("prompt_tokens") or ""))
    table.add_row("completion/output tokens", str(manifest_usage.get("completion_tokens") or ""))
    table.add_row("total tokens", str(manifest_usage.get("total_tokens") or ""))
    table.add_row("cached input tokens", str(total_usage.get("cached_input_tokens") or ""))
    table.add_row("reasoning output tokens", str(total_usage.get("reasoning_output_tokens") or ""))
    rprint(table)


def _render_codex_usage_import(payload: dict[str, Any]) -> None:
    table = Table(title="codex usage import")
    table.add_column("field")
    table.add_column("value")
    table.add_row("manifest", str(payload.get("manifest_path") or ""))
    table.add_row("output", str(payload.get("output_manifest_path") or ""))
    table.add_row("imported runs", str(payload.get("imported_count") or 0))
    rprint(table)

    runs = Table(title="imported runs")
    runs.add_column("run_id")
    runs.add_column("mode")
    runs.add_column("sessions")
    runs.add_column("prompt")
    runs.add_column("completion")
    runs.add_column("total")
    runs.add_column("quality")
    runs.add_column("roles")
    for row in payload.get("imported_runs") or []:
        usage = row.get("usage") or {}
        role_breakdown = row.get("role_breakdown") if isinstance(row.get("role_breakdown"), dict) else {}
        runs.add_row(
            str(row.get("run_id") or ""),
            str(row.get("mode") or ""),
            str(row.get("session_count") or 0),
            str(usage.get("prompt_tokens") or ""),
            str(usage.get("completion_tokens") or ""),
            str(usage.get("total_tokens") or ""),
            str(row.get("quality") or ""),
            ",".join(sorted(role_breakdown)) or "unclassified",
        )
    rprint(runs)


def _parse_run_session_mappings(items: list[str] | None) -> dict[str, list[Path]]:
    mappings: dict[str, list[Path]] = {}
    for item in items or []:
        if "=" not in item:
            raise typer.BadParameter("--run-session must use RUN_ID=SESSION_JSONL")
        run_id, path = item.split("=", 1)
        run_id = run_id.strip()
        path = path.strip()
        if not run_id or not path:
            raise typer.BadParameter("--run-session must use RUN_ID=SESSION_JSONL")
        mappings.setdefault(run_id, []).append(Path(path))
    return mappings


def _parse_run_role_session_mappings(items: list[str] | None) -> dict[str, list[tuple[str, Path]]]:
    mappings: dict[str, list[tuple[str, Path]]] = {}
    for item in items or []:
        if "=" not in item or ":" not in item:
            raise typer.BadParameter("--run-role-session must use RUN_ID:ROLE=SESSION_JSONL")
        run_and_role, path = item.split("=", 1)
        run_id, role = run_and_role.split(":", 1)
        run_id = run_id.strip()
        role = role.strip().lower()
        path = path.strip()
        if not run_id or not role or not path:
            raise typer.BadParameter("--run-role-session must use RUN_ID:ROLE=SESSION_JSONL")
        mappings.setdefault(run_id, []).append((role, Path(path)))
    return mappings

def _render_local_snapshot_performance_benchmark(payload: dict[str, Any]) -> None:
    aggregate = payload.get("aggregate") or {}
    table = Table(title=f"local snapshot performance benchmark · {payload.get('benchmark_id')}")
    table.add_column("field")
    table.add_column("value")
    table.add_row("candidate mode", str(payload.get("candidate_mode") or ""))
    table.add_row("baseline mode", str(payload.get("baseline_mode") or ""))
    table.add_row("evidence", str(payload.get("evidence_type") or ""))
    table.add_row("verdict", str(aggregate.get("verdict") or ""))
    table.add_row(
        "comparable cases",
        f"{aggregate.get('comparable_case_count')} / {aggregate.get('required_case_count')}",
    )
    table.add_row(
        "push health green",
        f"{aggregate.get('push_health_green_case_count')} / {aggregate.get('required_push_case_count')}",
    )
    table.add_row("caveat", str(aggregate.get("claim_caveat") or ""))
    rprint(table)

    detail = Table(title="case detail")
    detail.add_column("operation")
    detail.add_column("phase")
    detail.add_column("candidate runs")
    detail.add_column("baseline runs")
    detail.add_column("candidate median (s)")
    detail.add_column("baseline median (s)")
    detail.add_column("ratio")
    detail.add_column("push health")
    detail.add_column("comparable")
    for case in payload.get("case_summaries") or []:
        push_health = case.get("candidate_push_health_green")
        push_health_text = "n/a" if push_health is None else ("yes" if push_health else "no")
        detail.add_row(
            str(case.get("operation") or ""),
            str(case.get("phase") or ""),
            str(case.get("candidate_run_count") or 0),
            str(case.get("baseline_run_count") or 0),
            str(case.get("candidate_median_elapsed_seconds") or ""),
            str(case.get("baseline_median_elapsed_seconds") or ""),
            str(case.get("candidate_vs_baseline_ratio") or ""),
            push_health_text,
            "yes" if case.get("comparable") else "no",
        )
    rprint(detail)


def _strict_rerun_workload_rows(seed_manifest_path: Path, workload_specs: Optional[list[str]]) -> list[dict[str, str]]:
    if workload_specs:
        return [_parse_strict_rerun_workload_spec(spec) for spec in workload_specs]
    seed_manifest = load_strict_rerun_seed_manifest(seed_manifest_path)
    return strict_rerun_workloads_from_manifest(seed_manifest)


def _parse_strict_rerun_workload_spec(spec: str) -> dict[str, str]:
    parts = [segment.strip() for segment in str(spec or "").split("|")]
    if len(parts) != 4 or not all(parts):
        raise ValueError("Each --workload must use WORKLOAD_ID|TITLE|CATEGORY|ACCEPTANCE.")
    return {
        "workload_id": parts[0],
        "title": parts[1],
        "category": parts[2],
        "acceptance": parts[3],
    }
