from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

from .static_web_benchmark import (
    COMPARISON_STYLES,
    DEFAULT_AGGREGATE_CANDIDATE_MODE,
    DEFAULT_BASELINE_MODE,
    GAME_OVER_BEHAVIOR_KEYS,
    VICTORY_BEHAVIOR_KEYS,
    _bounded_int,
    _coerce_optional_bool,
    _completion_is_equivalent,
    _delta_float,
    _delta_int,
    _extract_usage,
    _flag_from_run_or_behavior,
    _format_float,
    _format_int,
    _format_percent,
    _list_text,
    _median_float,
    _median_int,
    _normalized_comparison_style,
    _normalized_mode_list,
    _normalized_text_list,
    _optional_float,
    _optional_nonnegative_int,
    _ratio,
    _review_text,
)

BENCHMARK_KIND = "static_web_hardening_task"
WORKLOAD_KIND = "task_dag_2d_plane_shooter_release_hardening"
PASSING_CHECK_STATES = {"pass", "passed", "complete", "completed", "accepted", "ok", "yes", "true"}
KNOWN_CHECK_STATES = PASSING_CHECK_STATES | {"fail", "failed", "not_run", "unknown", "partial", "warn"}
BOOTSTRAP_SURFACES = frozenset({"reality", "normalized_execution"})
COMPARISON_FAMILIES = frozenset({"core_single_session", "staged_compiled_coordinator"})
AIT_LINEAR_BINDING_PATHS = frozenset({"self_authored_ref", "preseeded_ref", "advisory_override"})
PLAN_TASK_BINDING_MODES = frozenset({"advisory", "strict", "required"})
RUBRIC_WEIGHTS = {
    "regression_free_medium_behavior": 20,
    "hardening_systems_completeness": 20,
    "shared_contracts_and_release_docs": 20,
    "validation_and_startup_quality": 15,
    "code_structure_and_convergence_quality": 15,
    "benchmark_hygiene": 10,
}
REQUIRED_RUBRIC_KEYS = tuple(RUBRIC_WEIGHTS.keys())
REQUIRED_CONTRACT_FAMILY_ALIASES = {
    "replay": ("replay",),
    "settings": ("setting", "settings", "preference", "preferences"),
    "benchmark": ("benchmark", "runbook"),
    "release": ("release", "checklist", "manifest"),
}
DAG_COST_ACCOUNTING_POLICIES = frozenset({"not_recorded", "reported_separately", "included_in_measured_total"})
DAG_PREPARATION_ROLES = ("sprint_card_setup", "dag_json_authoring")
DAG_EXECUTION_ROLE = "worker_execution"


def _mode_display_label(*, mode: str, bootstrap_surface: str | None = None) -> str:
    normalized_mode = str(mode or "").strip()
    normalized_surface = str(bootstrap_surface or "").strip()
    if normalized_mode == "ait_linear":
        if normalized_surface == "reality":
            return "ait_linear_reality"
        if normalized_surface == "normalized_execution":
            return "ait_linear_normalized"
    return normalized_mode


def _normalized_dag_cost_accounting_policy(value: Any) -> str | None:
    text = str(value or "").strip().lower()
    if not text:
        return None
    if text not in DAG_COST_ACCOUNTING_POLICIES:
        allowed = ", ".join(sorted(DAG_COST_ACCOUNTING_POLICIES))
        raise ValueError(f"dag_cost_accounting_policy must be one of: {allowed}")
    return text


def _normalize_usage_breakdown_map(value: Any) -> dict[str, dict[str, int | None]]:
    if not isinstance(value, dict):
        return {}
    normalized: dict[str, dict[str, int | None]] = {}
    for raw_role, raw_details in value.items():
        role = str(raw_role or "").strip()
        if not role:
            continue
        details = raw_details if isinstance(raw_details, dict) else {}
        usage = _extract_usage(details)
        normalized[role] = {
            "prompt_tokens": usage.get("prompt_tokens"),
            "completion_tokens": usage.get("completion_tokens"),
            "total_tokens": usage.get("total_tokens"),
            "cached_input_tokens": usage.get("cached_input_tokens"),
            "reasoning_output_tokens": usage.get("reasoning_output_tokens"),
            "session_count": _optional_nonnegative_int(details.get("session_count")),
            "token_event_count": _optional_nonnegative_int(details.get("token_event_count")),
        }
    return normalized


def _summed_usage_breakdown_roles(
    breakdown: dict[str, dict[str, int | None]],
    roles: tuple[str, ...] | list[str],
) -> dict[str, int | None]:
    prompt_tokens = 0
    completion_tokens = 0
    total_tokens = 0
    session_count = 0
    token_event_count = 0
    saw_prompt = False
    saw_completion = False
    saw_total = False
    saw_session_count = False
    saw_token_event_count = False
    for role in roles:
        details = breakdown.get(role) if isinstance(breakdown.get(role), dict) else {}
        if details.get("prompt_tokens") is not None:
            prompt_tokens += int(details["prompt_tokens"])
            saw_prompt = True
        if details.get("completion_tokens") is not None:
            completion_tokens += int(details["completion_tokens"])
            saw_completion = True
        if details.get("total_tokens") is not None:
            total_tokens += int(details["total_tokens"])
            saw_total = True
        if details.get("session_count") is not None:
            session_count += int(details["session_count"])
            saw_session_count = True
        if details.get("token_event_count") is not None:
            token_event_count += int(details["token_event_count"])
            saw_token_event_count = True
    return {
        "prompt_tokens": prompt_tokens if saw_prompt else None,
        "completion_tokens": completion_tokens if saw_completion else None,
        "total_tokens": total_tokens if saw_total else None,
        "session_count": session_count if saw_session_count else None,
        "token_event_count": token_event_count if saw_token_event_count else None,
    }


def run_static_web_hardening_task_benchmark(manifest_path: Path) -> dict[str, Any]:
    manifest_path = Path(manifest_path)
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise ValueError(f"Benchmark manifest not found: {manifest_path}") from exc
    except json.JSONDecodeError as exc:
        raise ValueError(f"Benchmark manifest is not valid JSON: {manifest_path}: {exc}") from exc
    return evaluate_static_web_hardening_task_manifest(manifest, manifest_path=manifest_path)


def evaluate_static_web_hardening_task_manifest(
    manifest: dict[str, Any],
    *,
    manifest_path: Path | None = None,
) -> dict[str, Any]:
    if not isinstance(manifest, dict):
        raise ValueError("Benchmark manifest must be a JSON object.")
    raw_workloads = manifest.get("workloads")
    if not isinstance(raw_workloads, list) or not raw_workloads:
        raise ValueError("Benchmark manifest must include a non-empty workloads list.")

    benchmark_id = str(manifest.get("benchmark_id") or (manifest_path.stem if manifest_path else BENCHMARK_KIND)).strip()
    if not benchmark_id:
        raise ValueError("benchmark_id must not be empty.")

    benchmark_kind = str(manifest.get("benchmark_kind") or BENCHMARK_KIND).strip() or BENCHMARK_KIND
    workload_kind = str(manifest.get("workload_kind") or WORKLOAD_KIND).strip() or WORKLOAD_KIND
    baseline_mode = str(manifest.get("baseline_mode") or DEFAULT_BASELINE_MODE).strip() or DEFAULT_BASELINE_MODE
    comparison_style_default = _normalized_comparison_style(manifest.get("comparison_style"), default="reviewed_plan")
    comparison_family = _normalized_comparison_family(
        manifest.get("comparison_family"),
        default="core_single_session",
    )
    bootstrap_surface = _normalized_bootstrap_surface(
        manifest.get("bootstrap_surface"),
        default="reality",
    )
    minimum_comparable_runs = _optional_nonnegative_int(manifest.get("minimum_comparable_runs"))
    if minimum_comparable_runs is None:
        minimum_comparable_runs = 1
    known_confounders = _normalized_text_list(manifest.get("known_confounders"))
    description = str(manifest.get("description") or "").strip() or None
    baseline_fixture = _normalize_baseline_fixture(manifest.get("baseline_fixture"), manifest_path=manifest_path)

    normalized_workloads: list[dict[str, Any]] = []
    all_runs: list[dict[str, Any]] = []
    discovered_modes: list[str] = []
    for workload_index, workload in enumerate(raw_workloads, start=1):
        if not isinstance(workload, dict):
            raise ValueError(f"workload at index {workload_index} must be an object.")
        workload_id = str(workload.get("workload_id") or workload.get("id") or "").strip()
        if not workload_id:
            raise ValueError(f"workload at index {workload_index} is missing workload_id.")
        title = str(workload.get("title") or workload_id).strip() or workload_id
        category = str(workload.get("category") or "long").strip() or "long"
        comparison_style = _normalized_comparison_style(workload.get("comparison_style"), default=comparison_style_default)
        raw_runs = workload.get("runs")
        if not isinstance(raw_runs, list) or not raw_runs:
            raise ValueError(f"workload {workload_id!r} must include a non-empty runs list.")

        normalized_runs: list[dict[str, Any]] = []
        for run_index, run in enumerate(raw_runs, start=1):
            if not isinstance(run, dict):
                raise ValueError(f"workload {workload_id!r} run at index {run_index} must be an object.")
            normalized = _normalize_run(
                workload_id=workload_id,
                category=category,
                comparison_style=comparison_style,
                run=run,
                run_index=run_index,
                baseline_mode=baseline_mode,
                baseline_fixture_default=baseline_fixture,
                comparison_family_default=comparison_family,
                bootstrap_surface_default=bootstrap_surface,
                manifest_path=manifest_path,
            )
            normalized_runs.append(normalized)
            all_runs.append(normalized)
            mode = str(normalized.get("mode") or "")
            if mode and mode not in discovered_modes:
                discovered_modes.append(mode)

        normalized_workloads.append(
            {
                "workload_id": workload_id,
                "title": title,
                "category": category,
                "comparison_style": comparison_style,
                "runs": normalized_runs,
            }
        )

    discovered_comparison_families = {str(run.get("comparison_family") or "") for run in all_runs if run.get("comparison_family")}
    if len(discovered_comparison_families) > 1:
        rendered = ", ".join(sorted(discovered_comparison_families))
        raise ValueError(
            "Static web hardening benchmark manifests must keep one comparison_family per report. "
            f"Found: {rendered}"
        )
    discovered_bootstrap_surfaces = {str(run.get("bootstrap_surface") or "") for run in all_runs if run.get("bootstrap_surface")}
    if len(discovered_bootstrap_surfaces) > 1:
        rendered = ", ".join(sorted(discovered_bootstrap_surfaces))
        raise ValueError(
            "Static web hardening benchmark manifests must keep one bootstrap_surface per report. "
            f"Found: {rendered}"
        )

    manifest_candidate_modes = _normalized_mode_list(manifest.get("candidate_modes"))
    candidate_modes = manifest_candidate_modes or [mode for mode in discovered_modes if mode and mode != baseline_mode]
    if not candidate_modes:
        raise ValueError("Benchmark manifest must include at least one candidate mode.")
    missing_candidate_modes = [mode for mode in candidate_modes if mode not in discovered_modes]
    if missing_candidate_modes:
        rendered = ", ".join(missing_candidate_modes)
        raise ValueError(f"candidate_modes not present in workloads: {rendered}")

    aggregate_candidate_mode = str(
        manifest.get("aggregate_candidate_mode")
        or (DEFAULT_AGGREGATE_CANDIDATE_MODE if DEFAULT_AGGREGATE_CANDIDATE_MODE in candidate_modes else candidate_modes[0])
    ).strip()
    if aggregate_candidate_mode not in candidate_modes:
        raise ValueError("aggregate_candidate_mode must be one of candidate_modes.")

    mode_summaries = {
        mode: _build_mode_summary(
            mode,
            [run for run in all_runs if run.get("mode") == mode],
            minimum_comparable_runs=minimum_comparable_runs,
        )
        for mode in [baseline_mode, *candidate_modes]
        if any(run.get("mode") == mode for run in all_runs)
    }

    comparisons = []
    baseline_summary = mode_summaries.get(baseline_mode) or _empty_mode_summary(baseline_mode, minimum_comparable_runs)
    for candidate_mode in candidate_modes:
        candidate_summary = mode_summaries.get(candidate_mode) or _empty_mode_summary(candidate_mode, minimum_comparable_runs)
        comparisons.append(
            _build_mode_comparison(
                baseline_mode=baseline_mode,
                baseline_summary=baseline_summary,
                candidate_mode=candidate_mode,
                candidate_summary=candidate_summary,
                minimum_comparable_runs=minimum_comparable_runs,
            )
        )

    aggregate = next(
        comparison for comparison in comparisons if str(comparison.get("candidate_mode") or "") == aggregate_candidate_mode
    )
    evidence_type = "measured" if all_runs and all(bool(run.get("measured")) for run in all_runs) else "operational"
    mode_labels = {
        mode: _mode_display_label(mode=mode, bootstrap_surface=bootstrap_surface)
        for mode in [baseline_mode, *candidate_modes]
    }

    return {
        "benchmark_id": benchmark_id,
        "benchmark_kind": benchmark_kind,
        "workload_kind": workload_kind,
        "manifest_path": str(manifest_path) if manifest_path is not None else None,
        "description": description,
        "comparison_style": comparison_style_default,
        "comparison_family": comparison_family,
        "bootstrap_surface": bootstrap_surface,
        "baseline_mode": baseline_mode,
        "baseline_mode_label": mode_labels.get(baseline_mode) or baseline_mode,
        "baseline_fixture": baseline_fixture,
        "candidate_modes": candidate_modes,
        "candidate_mode_labels": {mode: mode_labels.get(mode) or mode for mode in candidate_modes},
        "aggregate_candidate_mode": aggregate_candidate_mode,
        "aggregate_candidate_mode_label": mode_labels.get(aggregate_candidate_mode) or aggregate_candidate_mode,
        "minimum_comparable_runs": minimum_comparable_runs,
        "known_confounders": known_confounders,
        "evidence_type": evidence_type,
        "workloads": normalized_workloads,
        "mode_summaries": list(mode_summaries.values()),
        "comparisons": comparisons,
        "aggregate": aggregate,
    }


def render_static_web_hardening_task_markdown(payload: dict[str, Any]) -> str:
    aggregate = payload.get("aggregate") if isinstance(payload.get("aggregate"), dict) else {}
    mode_summaries = payload.get("mode_summaries") if isinstance(payload.get("mode_summaries"), list) else []
    baseline_fixture = payload.get("baseline_fixture") if isinstance(payload.get("baseline_fixture"), dict) else {}
    lines = [
        f"# {payload.get('benchmark_id') or 'Static Web Hardening Benchmark'}",
        "",
        "Generated static web hardening benchmark report.",
        "",
        f"- Benchmark kind: `{payload.get('benchmark_kind') or BENCHMARK_KIND}`",
        f"- Workload kind: `{payload.get('workload_kind') or WORKLOAD_KIND}`",
        f"- Comparison family: `{payload.get('comparison_family') or ''}`",
        f"- Bootstrap surface: `{payload.get('bootstrap_surface') or ''}`",
        f"- Evidence type: `{payload.get('evidence_type') or 'operational'}`",
        f"- Baseline mode: `{payload.get('baseline_mode_label') or payload.get('baseline_mode') or DEFAULT_BASELINE_MODE}`",
        f"- Baseline fixture snapshot: `{baseline_fixture.get('snapshot_id') or 'n/a'}`",
        f"- Baseline fixture digest: `{baseline_fixture.get('digest') or 'n/a'}`",
        f"- Aggregate candidate mode: `{aggregate.get('candidate_mode_label') or payload.get('aggregate_candidate_mode_label') or aggregate.get('candidate_mode') or payload.get('aggregate_candidate_mode') or ''}`",
        f"- Verdict: `{aggregate.get('verdict') or ''}`",
        f"- Comparable runs: `{aggregate.get('candidate_comparable_run_count')}` / `{payload.get('minimum_comparable_runs')}` required",
        f"- Candidate median score: `{_format_int(aggregate.get('candidate_median_score'))}`",
        f"- Candidate pass rate: `{_format_percent(aggregate.get('candidate_pass_rate'))}`",
        f"- Caveat: {aggregate.get('claim_caveat') or ''}",
    ]
    if payload.get("description"):
        lines.extend(["", str(payload.get("description"))])

    known_confounders = _normalized_text_list(payload.get("known_confounders"))
    if known_confounders:
        lines.extend(["", "## Known Confounders", ""])
        lines.extend(f"- {item}" for item in known_confounders)

    lines.extend(
        [
            "",
            "## Mode Summary",
            "",
            "| Mode | Total runs | Comparable | Pass | Borderline | Fail | Median score | Median elapsed (s) | Median total tokens | Verdict |",
            "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |",
        ]
    )
    for summary in mode_summaries:
        lines.append(
            "| {mode} | {total} | {comparable} | {passed} | {borderline} | {failed} | {score} | {elapsed} | {tokens} | {verdict} |".format(
                mode=summary.get("mode_label") or summary.get("mode") or "",
                total=_format_int(summary.get("total_run_count")),
                comparable=_format_int(summary.get("comparable_run_count")),
                passed=_format_int(summary.get("pass_count")),
                borderline=_format_int(summary.get("borderline_count")),
                failed=_format_int(summary.get("fail_count")),
                score=_format_int(summary.get("median_score")),
                elapsed=_format_float(summary.get("median_elapsed_seconds")),
                tokens=_format_int(summary.get("median_total_tokens")),
                verdict=summary.get("verdict") or "",
            )
        )

    lines.extend(
        [
            "",
            "## Run Summary",
            "",
            "| Workload | Run | Mode | Verdict | Score | Comparable | Startup | Validation | Replay | Settings | Mobile | Total tokens |",
            "| --- | --- | --- | --- | ---: | --- | --- | --- | --- | --- | --- | ---: |",
        ]
    )
    for workload in payload.get("workloads") or []:
        for run in workload.get("runs") or []:
            usage = run.get("usage") if isinstance(run.get("usage"), dict) else {}
            lines.append(
                "| {workload_id} | {run_id} | {mode} | {verdict} | {score} | {comparable} | {startup} | {validation} | {replay} | {settings} | {mobile} | {tokens} |".format(
                    workload_id=workload.get("workload_id") or "",
                    run_id=run.get("run_id") or "",
                    mode=run.get("mode_label") or run.get("mode") or "",
                    verdict=run.get("pass_or_fail") or "",
                    score=_format_int(run.get("score")),
                    comparable="yes" if run.get("comparable") else "no",
                    startup="yes" if run.get("startup_check_passed") else "no",
                    validation="yes" if run.get("validation_check_passed") else "no",
                    replay=run.get("replay_check_status") or "",
                    settings=run.get("settings_check_status") or "",
                    mobile=run.get("mobile_input_check_status") or "",
                    tokens=_format_int(usage.get("total_tokens")),
                )
            )

    lines.extend(["", "## Cost Accounting", ""])
    emitted_cost_section = False
    for workload in payload.get("workloads") or []:
        for run in workload.get("runs") or []:
            role_breakdown = run.get("dag_cost_breakdown") if isinstance(run.get("dag_cost_breakdown"), dict) else {}
            if not role_breakdown:
                role_breakdown = run.get("usage_breakdown") if isinstance(run.get("usage_breakdown"), dict) else {}
            if not role_breakdown:
                continue
            emitted_cost_section = True
            dag_preparation_usage = run.get("dag_preparation_usage") if isinstance(run.get("dag_preparation_usage"), dict) else {}
            dag_execution_usage = run.get("dag_execution_usage") if isinstance(run.get("dag_execution_usage"), dict) else {}
            lines.extend(
                [
                    f"### {workload.get('workload_id')} · {run.get('run_id')} · {run.get('mode_label') or run.get('mode')}",
                    "",
                ]
            )
            if run.get("dag_cost_accounting_policy"):
                lines.append(f"- DAG cost accounting policy: `{run.get('dag_cost_accounting_policy')}`")
            if dag_preparation_usage:
                lines.append(
                    "- DAG preparation total: sessions=`{sessions}` prompt=`{prompt}` completion=`{completion}` total=`{total}`".format(
                        sessions=_format_int(dag_preparation_usage.get("session_count")),
                        prompt=_format_int(dag_preparation_usage.get("prompt_tokens")),
                        completion=_format_int(dag_preparation_usage.get("completion_tokens")),
                        total=_format_int(dag_preparation_usage.get("total_tokens")),
                    )
                )
            if dag_execution_usage:
                lines.append(
                    "- Worker execution total: sessions=`{sessions}` prompt=`{prompt}` completion=`{completion}` total=`{total}`".format(
                        sessions=_format_int(dag_execution_usage.get("session_count")),
                        prompt=_format_int(dag_execution_usage.get("prompt_tokens")),
                        completion=_format_int(dag_execution_usage.get("completion_tokens")),
                        total=_format_int(dag_execution_usage.get("total_tokens")),
                    )
                )
            lines.extend(
                [
                    "",
                    "| Role | Sessions | Prompt | Completion | Total |",
                    "| --- | ---: | ---: | ---: | ---: |",
                ]
            )
            for role, details in role_breakdown.items():
                lines.append(
                    "| {role} | {sessions} | {prompt} | {completion} | {total} |".format(
                        role=role,
                        sessions=_format_int(details.get("session_count")),
                        prompt=_format_int(details.get("prompt_tokens")),
                        completion=_format_int(details.get("completion_tokens")),
                        total=_format_int(details.get("total_tokens")),
                    )
                )
            lines.append("")
    if not emitted_cost_section:
        lines.append("No role-based cost accounting was recorded.")
        lines.append("")

    lines.extend(["", "## Review Detail", ""])
    for workload in payload.get("workloads") or []:
        for run in workload.get("runs") or []:
            lines.extend(
                [
                    f"### {workload.get('workload_id')} · {run.get('run_id')} · {run.get('mode_label') or run.get('mode')}",
                    "",
                    f"- Internal mode: `{run.get('mode') or ''}`",
                    f"- Comparison style: `{run.get('comparison_style') or workload.get('comparison_style') or payload.get('comparison_style') or ''}`",
                    f"- Comparison family: `{run.get('comparison_family') or payload.get('comparison_family') or ''}`",
                    f"- Bootstrap surface: `{run.get('bootstrap_surface') or payload.get('bootstrap_surface') or ''}`",
                    f"- Baseline fixture snapshot: `{run.get('baseline_fixture_snapshot') or ''}`",
                    f"- Baseline fixture digest: `{run.get('baseline_fixture_digest') or ''}`",
                    f"- Startup script: `{run.get('startup_script') or ''}`",
                    f"- Validation script: `{run.get('validation_script') or ''}`",
                    f"- Startup check passed: `{bool(run.get('startup_check_passed'))}`",
                    f"- Validation check passed: `{bool(run.get('validation_check_passed'))}`",
                    f"- Runtime closeout: `{run.get('evaluator_runtime_closeout_status') or ''}`",
                    f"- Topology: `{run.get('topology_id') or ''}`",
                    f"- Measured sessions: `{run.get('measured_session_count') if run.get('measured_session_count') is not None else 'n/a'}`",
                    f"- Provider usage source: `{run.get('provider_usage_source') or ''}`",
                    f"- Shared task docs: {_list_text(run.get('shared_task_docs'))}",
                    f"- AIT linear binding path: `{run.get('ait_linear_binding_path') or ''}`",
                    f"- Plan task binding mode: `{run.get('plan_task_binding_mode') or ''}`",
                    f"- DAG cost accounting policy: `{run.get('dag_cost_accounting_policy') or ''}`",
                    f"- DAG preparation tokens: `{_format_int((run.get('dag_preparation_usage') or {}).get('total_tokens'))}`",
                    f"- Worker execution tokens: `{_format_int((run.get('dag_execution_usage') or {}).get('total_tokens'))}`",
                    f"- Replay check: `{run.get('replay_check_status') or ''}`",
                    f"- Settings check: `{run.get('settings_check_status') or ''}`",
                    f"- Mobile input check: `{run.get('mobile_input_check_status') or ''}`",
                    f"- Surface reproducible: `{bool(run.get('surface_reproducible'))}`",
                    f"- Contract files: {_list_text(run.get('contract_files'))}",
                    f"- Release readiness: {_review_text(run.get('release_readiness_summary'))}",
                    f"- Task review summary: {_review_text(run.get('task_review_summary'))}",
                    f"- Code review summary: {_review_text(run.get('code_review_summary'))}",
                    f"- Tests/manual checks: {_list_text(run.get('tests_or_manual_checks'))}",
                    f"- Blocking findings: {_list_text(run.get('blocking_findings'))}",
                    f"- Notes: {str(run.get('notes') or '')}",
                    "",
                ]
            )

    lines.extend(["## Interpretation", "", _interpretation_text(payload), ""])
    return "\n".join(lines)


def compute_fixture_digest(fixture_root: str | Path) -> str:
    root = Path(fixture_root).expanduser()
    if not root.exists() or not root.is_dir():
        raise ValueError(f"Fixture root does not exist or is not a directory: {root}")
    digest = hashlib.sha256()
    files = sorted(path for path in root.rglob("*") if path.is_file())
    if not files:
        raise ValueError(f"Fixture root contains no files: {root}")
    for path in files:
        relpath = path.relative_to(root).as_posix()
        digest.update(relpath.encode("utf-8"))
        digest.update(b"\0")
        digest.update(hashlib.sha256(path.read_bytes()).hexdigest().encode("ascii"))
        digest.update(b"\0")
    return f"sha256:{digest.hexdigest()}"


def _normalize_baseline_fixture(value: Any, *, manifest_path: Path | None) -> dict[str, Any]:
    payload = value if isinstance(value, dict) else {}
    fixture_root = _resolve_path(payload.get("fixture_root"), manifest_path=manifest_path)
    digest = str(payload.get("digest") or "").strip() or None
    if digest is None and fixture_root is not None:
        try:
            digest = compute_fixture_digest(fixture_root)
        except ValueError:
            digest = None
    return {
        "fixture_id": str(payload.get("fixture_id") or "").strip() or None,
        "fixture_root": fixture_root,
        "snapshot_id": str(payload.get("snapshot_id") or payload.get("baseline_fixture_snapshot") or "").strip() or None,
        "digest": digest,
        "source_artifact": str(payload.get("source_artifact") or payload.get("source_benchmark_artifact") or "").strip() or None,
        "shared_task_docs": _normalized_text_list(payload.get("shared_task_docs")),
    }


def _normalize_run(
    *,
    workload_id: str,
    category: str,
    comparison_style: str,
    run: dict[str, Any],
    run_index: int,
    baseline_mode: str,
    baseline_fixture_default: dict[str, Any],
    comparison_family_default: str,
    bootstrap_surface_default: str,
    manifest_path: Path | None,
) -> dict[str, Any]:
    run_id = str(run.get("run_id") or run.get("id") or f"{workload_id}-run-{run_index}").strip()
    mode = str(run.get("mode") or "").strip()
    if not mode:
        raise ValueError(f"workload {workload_id!r} run {run_id!r} is missing mode.")

    run_comparison_style = _normalized_comparison_style(run.get("comparison_style"), default=comparison_style)
    comparison_family = _normalized_comparison_family(
        run.get("comparison_family"),
        default=comparison_family_default,
    )
    bootstrap_surface = _normalized_bootstrap_surface(
        run.get("bootstrap_surface"),
        default=bootstrap_surface_default,
    )
    fixture_root = _resolve_path(run.get("fixture_root"), manifest_path=manifest_path)
    entry_path = str(run.get("entry_path") or "").strip() or None
    entry_url = str(run.get("entry_url") or "").strip() or None
    benchmark_url = str(run.get("benchmark_url") or "").strip() or None
    startup_script = str(run.get("startup_script") or "").strip() or None
    validation_script = str(run.get("validation_script") or "").strip() or None
    startup_check_passed = bool(run.get("startup_check_passed"))
    validation_check_passed = bool(run.get("validation_check_passed"))
    topology_id = str(run.get("topology_id") or mode).strip() or mode
    measured_session_count = _optional_nonnegative_int(run.get("measured_session_count"))
    provider_usage_source = str(run.get("provider_usage_source") or "").strip() or None
    baseline_fixture_root = _resolve_path(run.get("baseline_fixture_root"), manifest_path=manifest_path) or baseline_fixture_default.get("fixture_root")
    baseline_fixture_snapshot = str(
        run.get("baseline_fixture_snapshot")
        or run.get("baseline_snapshot_id")
        or baseline_fixture_default.get("snapshot_id")
        or ""
    ).strip() or None
    baseline_fixture_digest = str(run.get("baseline_fixture_digest") or "").strip() or None
    if baseline_fixture_digest is None:
        baseline_fixture_digest = baseline_fixture_default.get("digest")
    if baseline_fixture_digest is None and baseline_fixture_root is not None:
        try:
            baseline_fixture_digest = compute_fixture_digest(baseline_fixture_root)
        except ValueError:
            baseline_fixture_digest = None
    baseline_source_artifact = str(
        run.get("baseline_source_artifact")
        or run.get("baseline_benchmark_artifact")
        or baseline_fixture_default.get("source_artifact")
        or ""
    ).strip() or None

    equivalent_task_completion = _completion_is_equivalent(run)
    undocumented_manual_edits_required = bool(run.get("undocumented_manual_edits_required"))
    required_behavior, required_behavior_failures = _normalize_required_behavior(run.get("required_behavior"))
    medium_regression_flag = _coerce_optional_bool(
        run.get("medium_regression_passed"),
        run.get("regression_free_medium_behavior"),
    )
    medium_regression_passed = bool(medium_regression_flag) if medium_regression_flag is not None else not required_behavior_failures
    rubric = _normalize_rubric(run.get("rubric"))
    score = sum(int(rubric[key]) for key in REQUIRED_RUBRIC_KEYS)

    replay_check_status = _normalized_check_status(run.get("replay_check_status"))
    settings_check_status = _normalized_check_status(run.get("settings_check_status"))
    mobile_input_check_status = _normalized_check_status(run.get("mobile_input_check_status"))
    verification_status = str(run.get("verification_status") or "").strip() or None
    rework_count = _optional_nonnegative_int(run.get("rework_count"))
    elapsed_seconds = _optional_float(run.get("elapsed_seconds"))
    commands_run = _optional_nonnegative_int(run.get("commands_run"))
    files_read = _optional_nonnegative_int(run.get("files_read"))
    files_edited = _optional_nonnegative_int(run.get("files_edited"))
    tests_or_manual_checks = _normalized_text_list(run.get("tests_or_manual_checks"))
    usage_kind = str(run.get("usage_kind") or "unknown").strip().lower() or "unknown"
    usage = _extract_usage(run.get("usage") if isinstance(run.get("usage"), dict) else {})
    usage_breakdown = _normalize_usage_breakdown_map(run.get("usage_breakdown"))
    usage_provenance = run.get("usage_provenance") if isinstance(run.get("usage_provenance"), dict) else None
    measured = (
        usage_kind == "measured"
        and usage.get("total_tokens") is not None
        and int(usage.get("total_tokens") or 0) > 0
        and measured_session_count is not None
        and measured_session_count > 0
        and provider_usage_source is not None
        and not _is_placeholder_text(provider_usage_source)
    )
    task_review_summary = run.get("task_review_summary")
    code_review_summary = run.get("code_review_summary")
    release_readiness_summary = run.get("release_readiness_summary")
    notes = str(run.get("notes") or "").strip() or None
    shared_task_docs = _normalized_text_list(run.get("shared_task_docs"))
    if not shared_task_docs:
        shared_task_docs = list(baseline_fixture_default.get("shared_task_docs") or [])
    ait_linear_binding_path = _normalized_ait_linear_binding_path(run.get("ait_linear_binding_path"))
    plan_task_binding_mode = _normalized_plan_task_binding_mode(run.get("plan_task_binding_mode"))
    evaluator_runtime_closeout_status = _normalized_check_status(run.get("evaluator_runtime_closeout_status"))
    surface_reproducible = _coerce_optional_bool(run.get("surface_reproducible"))
    dag_cost_accounting_policy = _normalized_dag_cost_accounting_policy(run.get("dag_cost_accounting_policy"))
    dag_cost_breakdown = _normalize_usage_breakdown_map(run.get("dag_cost_breakdown"))
    dag_cost_provenance = run.get("dag_cost_provenance") if isinstance(run.get("dag_cost_provenance"), dict) else None
    if mode == "ait_dag_current" and not dag_cost_breakdown and usage_breakdown:
        dag_cost_breakdown = dict(usage_breakdown)
    contract_files = _normalized_text_list(run.get("contract_files"))
    contract_families_present = _detect_contract_families(contract_files)
    missing_contract_families = [
        family for family in REQUIRED_CONTRACT_FAMILY_ALIASES if family not in contract_families_present
    ]
    dag_preparation_usage = _summed_usage_breakdown_roles(dag_cost_breakdown, list(DAG_PREPARATION_ROLES))
    dag_execution_usage = _summed_usage_breakdown_roles(dag_cost_breakdown, [DAG_EXECUTION_ROLE])

    game_over_present = _flag_from_run_or_behavior(run, required_behavior, aliases=GAME_OVER_BEHAVIOR_KEYS)
    victory_present = _flag_from_run_or_behavior(run, required_behavior, aliases=VICTORY_BEHAVIOR_KEYS)

    startup_script_exists = _path_exists(fixture_root, startup_script)
    validation_script_exists = _path_exists(fixture_root, validation_script)
    missing_shared_task_doc_paths = [path for path in shared_task_docs if not _path_exists(fixture_root, path)]
    missing_contract_paths = [path for path in contract_files if not _path_exists(fixture_root, path)]

    blockers: list[str] = []
    if not fixture_root:
        blockers.append("Missing fixture_root for the candidate run.")
    if not entry_url:
        blockers.append("Missing entry_url for the candidate run.")
    if not benchmark_url:
        blockers.append("Missing benchmark_url for the candidate run.")
    if not baseline_fixture_snapshot:
        blockers.append("Missing baseline fixture snapshot/provenance handle.")
    if not baseline_fixture_digest:
        blockers.append("Missing baseline fixture digest.")
    if not startup_script:
        blockers.append("Missing project-local startup script.")
    elif not startup_script_exists:
        blockers.append("Startup script path is missing from the fixture root.")
    if not startup_check_passed:
        blockers.append("Startup script did not pass the startup check.")
    if not validation_script:
        blockers.append("Missing project-local validation script.")
    elif not validation_script_exists:
        blockers.append("Validation script path is missing from the fixture root.")
    if not validation_check_passed:
        blockers.append("Validation script did not pass the validation check.")
    if not _check_status_passed(evaluator_runtime_closeout_status):
        blockers.append("Evaluator runtime closeout did not pass.")
    if surface_reproducible is not True:
        blockers.append("Exact benchmark surface is not yet marked reproducible.")
    if not medium_regression_passed:
        blockers.append("Medium benchmark behavior regressed.")
    if required_behavior_failures:
        blockers.append(f"Missing required behavior: {', '.join(required_behavior_failures)}.")
    if not _check_status_passed(replay_check_status):
        blockers.append("Replay validation did not pass.")
    if not _check_status_passed(settings_check_status):
        blockers.append("Settings validation did not pass.")
    if not _check_status_passed(mobile_input_check_status):
        blockers.append("Mobile-input validation did not pass.")
    if missing_contract_families:
        blockers.append(f"Missing contract artifact families: {', '.join(missing_contract_families)}.")
    if not shared_task_docs:
        blockers.append("Missing shared benchmark-owned task docs list.")
    if missing_shared_task_doc_paths:
        blockers.append(f"Missing shared task doc paths: {', '.join(missing_shared_task_doc_paths)}.")
    if missing_contract_paths:
        blockers.append(f"Missing contract file paths: {', '.join(missing_contract_paths)}.")
    if not game_over_present:
        blockers.append("Missing clear game over state.")
    if not victory_present:
        blockers.append("Missing clear victory state.")
    if undocumented_manual_edits_required:
        blockers.append("Run required undocumented manual editing after generation.")
    if comparison_family == "core_single_session" and measured_session_count not in {None, 1}:
        blockers.append("Core single-session comparisons must use exactly one measured session.")
    if comparison_family == "staged_compiled_coordinator" and (
        measured_session_count is None or measured_session_count < 2
    ):
        blockers.append("Staged compiled/coordinator comparisons must record at least two measured sessions.")
    if mode == "ait_linear":
        if ait_linear_binding_path is None:
            blockers.append("AIT linear runs must declare ait_linear_binding_path.")
        if plan_task_binding_mode is None:
            blockers.append("AIT linear runs must declare plan_task_binding_mode.")
        if bootstrap_surface == "reality":
            if ait_linear_binding_path != "self_authored_ref":
                blockers.append("Reality bootstrap AIT linear runs must use self_authored_ref.")
            if plan_task_binding_mode == "advisory":
                blockers.append("Reality bootstrap AIT linear runs must not hide binding cost behind advisory mode.")
        elif bootstrap_surface == "normalized_execution":
            if ait_linear_binding_path not in {"preseeded_ref", "advisory_override"}:
                blockers.append(
                    "Normalized execution AIT linear runs must use preseeded_ref or advisory_override."
                )
            if ait_linear_binding_path == "advisory_override" and plan_task_binding_mode != "advisory":
                blockers.append("advisory_override requires plan_task_binding_mode=advisory.")
            if ait_linear_binding_path == "preseeded_ref" and plan_task_binding_mode == "advisory":
                blockers.append("preseeded_ref must not be paired with advisory plan_task_binding_mode.")

    pass_or_fail = _run_verdict(score=score, blockers=blockers)
    comparable = bool(equivalent_task_completion and not blockers)

    return {
        "workload_id": workload_id,
        "category": category,
        "run_id": run_id,
        "mode": mode,
        "mode_label": _mode_display_label(mode=mode, bootstrap_surface=bootstrap_surface),
        "comparison_style": run_comparison_style,
        "comparison_family": comparison_family,
        "bootstrap_surface": bootstrap_surface,
        "baseline_mode": baseline_mode,
        "fixture_root": fixture_root,
        "entry_path": entry_path,
        "entry_url": entry_url,
        "benchmark_url": benchmark_url,
        "startup_script": startup_script,
        "validation_script": validation_script,
        "startup_check_passed": startup_check_passed,
        "validation_check_passed": validation_check_passed,
        "startup_script_exists": startup_script_exists,
        "validation_script_exists": validation_script_exists,
        "topology_id": topology_id,
        "measured_session_count": measured_session_count,
        "provider_usage_source": provider_usage_source,
        "baseline_fixture_root": baseline_fixture_root,
        "baseline_fixture_snapshot": baseline_fixture_snapshot,
        "baseline_fixture_digest": baseline_fixture_digest,
        "baseline_source_artifact": baseline_source_artifact,
        "equivalent_task_completion": equivalent_task_completion,
        "undocumented_manual_edits_required": undocumented_manual_edits_required,
        "required_behavior": required_behavior,
        "required_behavior_failures": required_behavior_failures,
        "medium_regression_passed": medium_regression_passed,
        "replay_check_status": replay_check_status,
        "settings_check_status": settings_check_status,
        "mobile_input_check_status": mobile_input_check_status,
        "verification_status": verification_status,
        "shared_task_docs": shared_task_docs,
        "ait_linear_binding_path": ait_linear_binding_path,
        "plan_task_binding_mode": plan_task_binding_mode,
        "evaluator_runtime_closeout_status": evaluator_runtime_closeout_status,
        "surface_reproducible": bool(surface_reproducible),
        "contract_files": contract_files,
        "contract_families_present": contract_families_present,
        "missing_contract_families": missing_contract_families,
        "missing_shared_task_doc_paths": missing_shared_task_doc_paths,
        "missing_contract_paths": missing_contract_paths,
        "rubric": rubric,
        "score": score,
        "blocking_findings": blockers,
        "pass_or_fail": pass_or_fail,
        "comparable": comparable,
        "rework_count": rework_count,
        "elapsed_seconds": elapsed_seconds,
        "commands_run": commands_run,
        "files_read": files_read,
        "files_edited": files_edited,
        "tests_or_manual_checks": tests_or_manual_checks,
        "usage_kind": usage_kind,
        "usage": usage,
        "usage_breakdown": usage_breakdown or None,
        "usage_provenance": usage_provenance,
        "measured": measured,
        "dag_cost_accounting_policy": dag_cost_accounting_policy,
        "dag_cost_breakdown": dag_cost_breakdown or None,
        "dag_cost_provenance": dag_cost_provenance,
        "dag_preparation_usage": dag_preparation_usage if dag_cost_breakdown else None,
        "dag_execution_usage": dag_execution_usage if dag_cost_breakdown else None,
        "task_review_summary": task_review_summary,
        "code_review_summary": code_review_summary,
        "release_readiness_summary": release_readiness_summary,
        "notes": notes,
    }


def _normalize_required_behavior(value: Any) -> tuple[list[dict[str, Any]], list[str]]:
    rows: list[dict[str, Any]] = []
    failures: list[str] = []
    if isinstance(value, dict):
        for key, passed in value.items():
            behavior_id = str(key or "").strip()
            if not behavior_id:
                continue
            passed_bool = bool(passed)
            rows.append({"id": behavior_id, "passed": passed_bool})
            if not passed_bool:
                failures.append(behavior_id)
        return rows, failures
    if isinstance(value, list):
        for index, item in enumerate(value, start=1):
            if isinstance(item, dict):
                behavior_id = str(item.get("id") or item.get("name") or f"behavior-{index}").strip()
                passed_bool = bool(item.get("passed"))
            else:
                behavior_id = str(item or f"behavior-{index}").strip()
                passed_bool = bool(item)
            if not behavior_id:
                continue
            rows.append({"id": behavior_id, "passed": passed_bool})
            if not passed_bool:
                failures.append(behavior_id)
    return rows, failures


def _normalize_rubric(value: Any) -> dict[str, int]:
    payload = value if isinstance(value, dict) else {}
    aliases = {
        "regression_free_medium_behavior": ("regression_free_medium_behavior", "medium_regression", "regression"),
        "hardening_systems_completeness": ("hardening_systems_completeness", "hardening_systems", "hardening"),
        "shared_contracts_and_release_docs": ("shared_contracts_and_release_docs", "shared_contracts", "release_docs"),
        "validation_and_startup_quality": ("validation_and_startup_quality", "validation_quality", "startup_quality"),
        "code_structure_and_convergence_quality": (
            "code_structure_and_convergence_quality",
            "code_structure",
            "convergence_quality",
        ),
        "benchmark_hygiene": ("benchmark_hygiene", "hygiene"),
    }
    normalized: dict[str, int] = {}
    for key, weight in RUBRIC_WEIGHTS.items():
        raw_value = None
        for alias in aliases[key]:
            if alias in payload:
                raw_value = payload.get(alias)
                break
        normalized[key] = _bounded_int(raw_value, minimum=0, maximum=weight, default=0)
    return normalized


def _build_mode_summary(mode: str, runs: list[dict[str, Any]], *, minimum_comparable_runs: int) -> dict[str, Any]:
    comparable_runs = [run for run in runs if run.get("comparable")]
    pass_runs = [run for run in comparable_runs if run.get("pass_or_fail") == "pass"]
    borderline_runs = [run for run in comparable_runs if run.get("pass_or_fail") == "borderline"]
    fail_runs = [run for run in runs if run.get("pass_or_fail") == "fail"]
    mode_label = str(runs[0].get("mode_label") or mode) if runs else mode
    return {
        "mode": mode,
        "mode_label": mode_label,
        "minimum_comparable_runs": minimum_comparable_runs,
        "total_run_count": len(runs),
        "comparable_run_count": len(comparable_runs),
        "pass_count": len(pass_runs),
        "borderline_count": len(borderline_runs),
        "fail_count": len(fail_runs),
        "comparable_rate": _ratio(len(comparable_runs), len(runs)),
        "pass_rate": _ratio(len(pass_runs), len(runs)),
        "median_score": _median_int([int(run["score"]) for run in comparable_runs]),
        "median_elapsed_seconds": _median_float(
            [float(run["elapsed_seconds"]) for run in comparable_runs if run.get("elapsed_seconds") is not None]
        ),
        "median_total_tokens": _median_int(
            [
                int(run["usage"]["total_tokens"])
                for run in comparable_runs
                if (run.get("usage") or {}).get("total_tokens") is not None
            ]
        ),
        "verdict": _mode_summary_verdict(
            total_run_count=len(runs),
            comparable_runs=comparable_runs,
            pass_runs=pass_runs,
            borderline_runs=borderline_runs,
            minimum_comparable_runs=minimum_comparable_runs,
        ),
    }


def _empty_mode_summary(mode: str, minimum_comparable_runs: int) -> dict[str, Any]:
    return {
        "mode": mode,
        "mode_label": mode,
        "minimum_comparable_runs": minimum_comparable_runs,
        "total_run_count": 0,
        "comparable_run_count": 0,
        "pass_count": 0,
        "borderline_count": 0,
        "fail_count": 0,
        "comparable_rate": None,
        "pass_rate": None,
        "median_score": None,
        "median_elapsed_seconds": None,
        "median_total_tokens": None,
        "verdict": "unproven",
    }


def _build_mode_comparison(
    *,
    baseline_mode: str,
    baseline_summary: dict[str, Any],
    candidate_mode: str,
    candidate_summary: dict[str, Any],
    minimum_comparable_runs: int,
) -> dict[str, Any]:
    candidate_comparable_run_count = int(candidate_summary.get("comparable_run_count") or 0)
    baseline_comparable_run_count = int(baseline_summary.get("comparable_run_count") or 0)
    total_candidate_runs = int(candidate_summary.get("total_run_count") or 0)
    pass_count = int(candidate_summary.get("pass_count") or 0)
    borderline_count = int(candidate_summary.get("borderline_count") or 0)

    if total_candidate_runs < minimum_comparable_runs or baseline_comparable_run_count < minimum_comparable_runs:
        verdict = "unproven"
    elif candidate_comparable_run_count == 0:
        verdict = "not_supported"
    elif candidate_comparable_run_count < minimum_comparable_runs:
        verdict = "mixed" if pass_count > 0 or borderline_count > 0 else "not_supported"
    elif candidate_summary.get("fail_count"):
        verdict = "mixed" if pass_count == candidate_comparable_run_count else "not_supported"
    elif pass_count == candidate_comparable_run_count:
        verdict = "supported"
    else:
        verdict = "mixed"

    claim_caveat = _comparison_caveat(
        verdict=verdict,
        baseline_mode=str(baseline_summary.get("mode_label") or baseline_mode),
        candidate_mode=str(candidate_summary.get("mode_label") or candidate_mode),
        baseline_comparable_run_count=baseline_comparable_run_count,
        candidate_comparable_run_count=candidate_comparable_run_count,
        minimum_comparable_runs=minimum_comparable_runs,
    )
    return {
        "baseline_mode": baseline_mode,
        "baseline_mode_label": baseline_summary.get("mode_label") or baseline_mode,
        "candidate_mode": candidate_mode,
        "candidate_mode_label": candidate_summary.get("mode_label") or candidate_mode,
        "minimum_comparable_runs": minimum_comparable_runs,
        "baseline_comparable_run_count": baseline_comparable_run_count,
        "candidate_comparable_run_count": candidate_comparable_run_count,
        "candidate_total_run_count": total_candidate_runs,
        "candidate_pass_count": pass_count,
        "candidate_borderline_count": borderline_count,
        "candidate_fail_count": int(candidate_summary.get("fail_count") or 0),
        "candidate_pass_rate": candidate_summary.get("pass_rate"),
        "candidate_median_score": candidate_summary.get("median_score"),
        "baseline_median_score": baseline_summary.get("median_score"),
        "score_delta": _delta_int(candidate_summary.get("median_score"), baseline_summary.get("median_score")),
        "candidate_median_elapsed_seconds": candidate_summary.get("median_elapsed_seconds"),
        "baseline_median_elapsed_seconds": baseline_summary.get("median_elapsed_seconds"),
        "elapsed_delta_seconds": _delta_float(
            candidate_summary.get("median_elapsed_seconds"),
            baseline_summary.get("median_elapsed_seconds"),
        ),
        "candidate_median_total_tokens": candidate_summary.get("median_total_tokens"),
        "baseline_median_total_tokens": baseline_summary.get("median_total_tokens"),
        "token_delta": _delta_int(candidate_summary.get("median_total_tokens"), baseline_summary.get("median_total_tokens")),
        "verdict": verdict,
        "claim_caveat": claim_caveat,
    }


def _mode_summary_verdict(
    *,
    total_run_count: int,
    comparable_runs: list[dict[str, Any]],
    pass_runs: list[dict[str, Any]],
    borderline_runs: list[dict[str, Any]],
    minimum_comparable_runs: int,
) -> str:
    comparable_count = len(comparable_runs)
    if total_run_count < minimum_comparable_runs:
        return "unproven"
    if comparable_count == 0:
        return "not_supported"
    if comparable_count < minimum_comparable_runs:
        return "mixed"
    if len(pass_runs) == comparable_count:
        return "supported"
    if len(pass_runs) + len(borderline_runs) == comparable_count:
        return "mixed"
    return "not_supported"


def _run_verdict(*, score: int, blockers: list[str]) -> str:
    if blockers:
        return "fail"
    if score >= 85:
        return "pass"
    if score >= 75:
        return "borderline"
    return "fail"


def _comparison_caveat(
    *,
    verdict: str,
    baseline_mode: str,
    candidate_mode: str,
    baseline_comparable_run_count: int,
    candidate_comparable_run_count: int,
    minimum_comparable_runs: int,
) -> str:
    if verdict == "supported":
        return (
            f"{candidate_mode} cleared the v2 hardening rubric on enough comparable runs against {baseline_mode}; keep conclusions scoped to this workload family and comparison style."
        )
    if verdict == "mixed":
        return (
            f"{candidate_mode} shows partial comparable success, but at least one hardening run stayed non-comparable or borderline. Review blockers before using this as product evidence."
        )
    if verdict == "not_supported":
        return (
            f"{candidate_mode} does not yet sustain comparable completion on this hardening benchmark surface, so do not treat the current runs as proof of long-task frontend execution quality."
        )
    return (
        f"Collect at least {minimum_comparable_runs} comparable runs for both {baseline_mode} and {candidate_mode} before using this hardening workload as shared evidence."
    )


def _interpretation_text(payload: dict[str, Any]) -> str:
    aggregate = payload.get("aggregate") if isinstance(payload.get("aggregate"), dict) else {}
    verdict = str(aggregate.get("verdict") or "unproven")
    candidate_mode = str(
        aggregate.get("candidate_mode_label")
        or payload.get("aggregate_candidate_mode_label")
        or aggregate.get("candidate_mode")
        or payload.get("aggregate_candidate_mode")
        or ""
    )
    baseline_mode = str(
        aggregate.get("baseline_mode_label")
        or payload.get("baseline_mode_label")
        or aggregate.get("baseline_mode")
        or payload.get("baseline_mode")
        or ""
    )
    if verdict == "supported":
        return (
            f"This report supports using {candidate_mode} as a credible compared mode against {baseline_mode} on the release-hardening shooter workload, with startup, validation, replay, settings, mobile-input, and contract checks recorded."
        )
    if verdict == "mixed":
        return (
            f"This report shows partial support for {candidate_mode}: some runs reached comparable completion, but the record still includes borderline or non-comparable hardening outcomes that limit stronger conclusions."
        )
    if verdict == "not_supported":
        return (
            f"This report does not support {candidate_mode} on this hardening benchmark surface yet because comparable completion and blocker-free validation were not sustained."
        )
    return (
        f"This report is still setup evidence only. Gather more comparable {baseline_mode} and {candidate_mode} hardening runs before treating it as benchmark proof."
    )


def _normalized_comparison_family(value: Any, *, default: str) -> str:
    text = str(value or default).strip().lower().replace("-", "_")
    aliases = {
        "core_single_session": "core_single_session",
        "single_session": "core_single_session",
        "core": "core_single_session",
        "staged_compiled_coordinator": "staged_compiled_coordinator",
        "staged_coordinator": "staged_compiled_coordinator",
        "compiled_coordinator": "staged_compiled_coordinator",
        "staged": "staged_compiled_coordinator",
    }
    normalized = aliases.get(text)
    if normalized is None or normalized not in COMPARISON_FAMILIES:
        allowed = ", ".join(sorted(COMPARISON_FAMILIES))
        raise ValueError(f"comparison_family must be one of: {allowed}")
    return normalized


def _normalized_bootstrap_surface(value: Any, *, default: str) -> str:
    text = str(value or default).strip().lower().replace("-", "_")
    aliases = {
        "reality": "reality",
        "reality_bootstrap": "reality",
        "normalized_execution": "normalized_execution",
        "normalized": "normalized_execution",
    }
    normalized = aliases.get(text)
    if normalized is None or normalized not in BOOTSTRAP_SURFACES:
        allowed = ", ".join(sorted(BOOTSTRAP_SURFACES))
        raise ValueError(f"bootstrap_surface must be one of: {allowed}")
    return normalized


def _normalized_ait_linear_binding_path(value: Any) -> str | None:
    text = str(value or "").strip().lower()
    if not text:
        return None
    if text not in AIT_LINEAR_BINDING_PATHS:
        allowed = ", ".join(sorted(AIT_LINEAR_BINDING_PATHS))
        raise ValueError(f"ait_linear_binding_path must be one of: {allowed}")
    return text


def _normalized_plan_task_binding_mode(value: Any) -> str | None:
    text = str(value or "").strip().lower()
    if not text:
        return None
    if text not in PLAN_TASK_BINDING_MODES:
        allowed = ", ".join(sorted(PLAN_TASK_BINDING_MODES))
        raise ValueError(f"plan_task_binding_mode must be one of: {allowed}")
    return text


def _normalized_check_status(value: Any) -> str:
    if isinstance(value, bool):
        return "passed" if value else "failed"
    text = str(value or "").strip().lower()
    if not text:
        return "unknown"
    if text not in KNOWN_CHECK_STATES:
        return text
    return text


def _check_status_passed(status: str) -> bool:
    return str(status or "").strip().lower() in PASSING_CHECK_STATES


def _detect_contract_families(contract_files: list[str]) -> list[str]:
    detected: list[str] = []
    for family, aliases in REQUIRED_CONTRACT_FAMILY_ALIASES.items():
        lowered = [item.lower() for item in contract_files]
        if any(any(alias in item for alias in aliases) for item in lowered):
            detected.append(family)
    return detected


def _path_exists(fixture_root: str | None, path_value: str | None) -> bool:
    if not fixture_root or not path_value:
        return False
    candidate = Path(path_value)
    if not candidate.is_absolute():
        candidate = Path(fixture_root) / candidate
    return candidate.exists()


def _is_placeholder_text(value: str | None) -> bool:
    text = str(value or "").strip()
    return bool(text) and text.startswith("<") and text.endswith(">")


def _resolve_path(value: Any, *, manifest_path: Path | None) -> str | None:
    text = str(value or "").strip()
    if not text:
        return None
    path = Path(text).expanduser()
    if path.is_absolute():
        return str(path.resolve(strict=False))
    if manifest_path is None:
        return str((Path.cwd() / path).resolve(strict=False))
    manifest_relative = (manifest_path.parent / path).resolve(strict=False)
    if manifest_relative.exists():
        return str(manifest_relative)
    cwd_relative = (Path.cwd() / path).resolve(strict=False)
    if cwd_relative.exists():
        return str(cwd_relative)
    return str(manifest_relative)
