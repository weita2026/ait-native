from __future__ import annotations

import json
import statistics
from pathlib import Path
from typing import Any

BENCHMARK_KIND = "static_web_task"
WORKLOAD_KIND = "task_dag_2d_plane_shooter"
DEFAULT_BASELINE_MODE = "git_linear"
DEFAULT_AGGREGATE_CANDIDATE_MODE = "ait_dag_current"
COMPARISON_STYLES = {"prompt_only", "reviewed_plan"}
RUBRIC_WEIGHTS = {
    "functional_completeness": 45,
    "deterministic_benchmark_harness": 15,
    "style_fidelity": 15,
    "code_structure_and_integration_quality": 15,
    "benchmark_hygiene": 10,
}
REQUIRED_RUBRIC_KEYS = tuple(RUBRIC_WEIGHTS.keys())
EQUIVALENT_COMPLETION_STATES = {"equivalent", "complete", "completed", "accepted", "done", "pass", "passed", "true"}
GAME_OVER_BEHAVIOR_KEYS = {"game_over", "clear_game_over", "has_clear_game_over"}
VICTORY_BEHAVIOR_KEYS = {"victory", "clear_victory", "has_clear_victory"}


def run_static_web_task_benchmark(manifest_path: Path) -> dict[str, Any]:
    manifest_path = Path(manifest_path)
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise ValueError(f"Benchmark manifest not found: {manifest_path}") from exc
    except json.JSONDecodeError as exc:
        raise ValueError(f"Benchmark manifest is not valid JSON: {manifest_path}: {exc}") from exc
    return evaluate_static_web_task_manifest(manifest, manifest_path=manifest_path)


def evaluate_static_web_task_manifest(
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
    comparison_style_default = _normalized_comparison_style(manifest.get("comparison_style"), default="prompt_only")
    minimum_comparable_runs = _nonnegative_int(manifest.get("minimum_comparable_runs"), default=1)
    known_confounders = _normalized_text_list(manifest.get("known_confounders"))
    description = str(manifest.get("description") or "").strip() or None

    normalized_workloads: list[dict[str, Any]] = []
    all_runs: list[dict[str, Any]] = []
    discovered_modes: list[str] = []
    measured_candidates = True

    for workload_index, workload in enumerate(raw_workloads, start=1):
        if not isinstance(workload, dict):
            raise ValueError(f"workload at index {workload_index} must be an object.")
        workload_id = str(workload.get("workload_id") or workload.get("id") or "").strip()
        if not workload_id:
            raise ValueError(f"workload at index {workload_index} is missing workload_id.")
        title = str(workload.get("title") or workload_id).strip() or workload_id
        category = str(workload.get("category") or "unknown").strip() or "unknown"
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
            )
            normalized_runs.append(normalized)
            all_runs.append(normalized)
            mode = str(normalized.get("mode") or "")
            if mode and mode not in discovered_modes:
                discovered_modes.append(mode)
            if normalized.get("comparable") and not normalized.get("measured"):
                measured_candidates = False

        normalized_workloads.append(
            {
                "workload_id": workload_id,
                "title": title,
                "category": category,
                "comparison_style": comparison_style,
                "runs": normalized_runs,
            }
        )

    manifest_candidate_modes = _normalized_mode_list(manifest.get("candidate_modes"))
    candidate_modes = manifest_candidate_modes or [mode for mode in discovered_modes if mode and mode != baseline_mode]
    if not candidate_modes:
        raise ValueError("Benchmark manifest must include at least one candidate mode.")
    missing_candidate_modes = [mode for mode in candidate_modes if mode not in discovered_modes]
    if missing_candidate_modes:
        missing_rendered = ", ".join(missing_candidate_modes)
        raise ValueError(f"candidate_modes not present in workloads: {missing_rendered}")

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
    evidence_type = "measured" if all_runs and measured_candidates else "operational"

    return {
        "benchmark_id": benchmark_id,
        "benchmark_kind": benchmark_kind,
        "workload_kind": workload_kind,
        "manifest_path": str(manifest_path) if manifest_path is not None else None,
        "description": description,
        "comparison_style": comparison_style_default,
        "baseline_mode": baseline_mode,
        "candidate_modes": candidate_modes,
        "aggregate_candidate_mode": aggregate_candidate_mode,
        "minimum_comparable_runs": minimum_comparable_runs,
        "known_confounders": known_confounders,
        "evidence_type": evidence_type,
        "workloads": normalized_workloads,
        "mode_summaries": list(mode_summaries.values()),
        "comparisons": comparisons,
        "aggregate": aggregate,
    }


def render_static_web_task_markdown(payload: dict[str, Any]) -> str:
    aggregate = payload.get("aggregate") if isinstance(payload.get("aggregate"), dict) else {}
    mode_summaries = payload.get("mode_summaries") if isinstance(payload.get("mode_summaries"), list) else []
    lines = [
        f"# {payload.get('benchmark_id') or 'Static Web Task Benchmark'}",
        "",
        "Generated static web task benchmark report.",
        "",
        f"- Benchmark kind: `{payload.get('benchmark_kind') or BENCHMARK_KIND}`",
        f"- Workload kind: `{payload.get('workload_kind') or WORKLOAD_KIND}`",
        f"- Evidence type: `{payload.get('evidence_type') or 'operational'}`",
        f"- Baseline mode: `{payload.get('baseline_mode') or DEFAULT_BASELINE_MODE}`",
        f"- Aggregate candidate mode: `{aggregate.get('candidate_mode') or payload.get('aggregate_candidate_mode') or ''}`",
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
                mode=summary.get("mode") or "",
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
            "| Workload | Run | Mode | Verdict | Score | Comparable | Startup | Seed | Fast boss | Elapsed (s) | Total tokens |",
            "| --- | --- | --- | --- | ---: | --- | --- | --- | --- | ---: | ---: |",
        ]
    )
    for workload in payload.get("workloads") or []:
        for run in workload.get("runs") or []:
            harness = run.get("harness") if isinstance(run.get("harness"), dict) else {}
            usage = run.get("usage") if isinstance(run.get("usage"), dict) else {}
            lines.append(
                "| {workload_id} | {run_id} | {mode} | {verdict} | {score} | {comparable} | {startup} | {seed} | {boss} | {elapsed} | {tokens} |".format(
                    workload_id=workload.get("workload_id") or "",
                    run_id=run.get("run_id") or "",
                    mode=run.get("mode") or "",
                    verdict=run.get("pass_or_fail") or "",
                    score=_format_int(run.get("score")),
                    comparable="yes" if run.get("comparable") else "no",
                    startup="yes" if run.get("startup_check_passed") else "no",
                    seed="yes" if harness.get("seed_control_present") else "no",
                    boss="yes" if harness.get("fast_boss_trigger_present") else "no",
                    elapsed=_format_float(run.get("elapsed_seconds")),
                    tokens=_format_int(usage.get("total_tokens")),
                )
            )

    lines.extend(["", "## Review Detail", ""])
    for workload in payload.get("workloads") or []:
        for run in workload.get("runs") or []:
            lines.extend(
                [
                    f"### {workload.get('workload_id')} · {run.get('run_id')} · {run.get('mode')}",
                    "",
                    f"- Comparison style: `{run.get('comparison_style') or workload.get('comparison_style') or payload.get('comparison_style') or ''}`",
                    f"- Startup script: `{run.get('startup_script') or ''}`",
                    f"- Startup check passed: `{bool(run.get('startup_check_passed'))}`",
                    f"- Entry path: `{run.get('entry_path') or ''}`",
                    f"- Entry URL: `{run.get('entry_url') or ''}`",
                    f"- Benchmark URL: `{run.get('benchmark_url') or ''}`",
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


def _normalize_run(
    *,
    workload_id: str,
    category: str,
    comparison_style: str,
    run: dict[str, Any],
    run_index: int,
    baseline_mode: str,
) -> dict[str, Any]:
    run_id = str(run.get("run_id") or run.get("id") or f"{workload_id}-run-{run_index}").strip()
    mode = str(run.get("mode") or "").strip()
    if not mode:
        raise ValueError(f"workload {workload_id!r} run {run_id!r} is missing mode.")
    run_comparison_style = _normalized_comparison_style(run.get("comparison_style"), default=comparison_style)
    startup_script = str(run.get("startup_script") or "").strip() or None
    startup_check_passed = bool(run.get("startup_check_passed"))
    fixture_root = str(run.get("fixture_root") or "").strip() or None
    entry_path = str(run.get("entry_path") or "").strip() or None
    entry_url = str(run.get("entry_url") or "").strip() or None
    benchmark_url = str(run.get("benchmark_url") or "").strip() or None
    equivalent_task_completion = _completion_is_equivalent(run)
    undocumented_manual_edits_required = bool(run.get("undocumented_manual_edits_required"))
    required_behavior, required_behavior_failures = _normalize_required_behavior(run.get("required_behavior"))
    harness = _normalize_harness(run.get("harness"))
    rubric = _normalize_rubric(run.get("rubric"))
    score = sum(int(rubric[key]) for key in REQUIRED_RUBRIC_KEYS)
    elapsed_seconds = _optional_float(run.get("elapsed_seconds"))
    commands_run = _optional_nonnegative_int(run.get("commands_run"))
    files_read = _optional_nonnegative_int(run.get("files_read"))
    files_edited = _optional_nonnegative_int(run.get("files_edited"))
    tests_or_manual_checks = _normalized_text_list(run.get("tests_or_manual_checks"))
    usage_kind = str(run.get("usage_kind") or "unknown").strip().lower() or "unknown"
    usage = _extract_usage(run.get("usage") if isinstance(run.get("usage"), dict) else {})
    measured = usage_kind == "measured" and usage.get("total_tokens") is not None
    task_review_summary = run.get("task_review_summary")
    code_review_summary = run.get("code_review_summary")
    notes = str(run.get("notes") or "").strip() or None

    game_over_present = _flag_from_run_or_behavior(run, required_behavior, aliases=GAME_OVER_BEHAVIOR_KEYS)
    victory_present = _flag_from_run_or_behavior(run, required_behavior, aliases=VICTORY_BEHAVIOR_KEYS)

    blockers: list[str] = []
    if not startup_script:
        blockers.append("Missing project-local startup script.")
    if not startup_check_passed:
        blockers.append("Startup script did not pass the benchmark startup check.")
    if required_behavior_failures:
        blockers.append(f"Missing required behavior: {', '.join(required_behavior_failures)}.")
    if not harness.get("seed_control_present"):
        blockers.append("Missing deterministic seed control.")
    if not harness.get("fast_boss_trigger_present"):
        blockers.append("Missing fast boss trigger.")
    if not game_over_present:
        blockers.append("Missing clear game over state.")
    if not victory_present:
        blockers.append("Missing clear victory state.")
    if undocumented_manual_edits_required:
        blockers.append("Run required undocumented manual editing after generation.")

    pass_or_fail = _run_verdict(score=score, blockers=blockers)
    comparable = bool(equivalent_task_completion and not blockers)

    return {
        "workload_id": workload_id,
        "category": category,
        "run_id": run_id,
        "mode": mode,
        "comparison_style": run_comparison_style,
        "baseline_mode": baseline_mode,
        "startup_script": startup_script,
        "startup_check_passed": startup_check_passed,
        "fixture_root": fixture_root,
        "entry_path": entry_path,
        "entry_url": entry_url,
        "benchmark_url": benchmark_url,
        "equivalent_task_completion": equivalent_task_completion,
        "undocumented_manual_edits_required": undocumented_manual_edits_required,
        "required_behavior": required_behavior,
        "required_behavior_failures": required_behavior_failures,
        "harness": harness,
        "rubric": rubric,
        "score": score,
        "blocking_findings": blockers,
        "pass_or_fail": pass_or_fail,
        "comparable": comparable,
        "elapsed_seconds": elapsed_seconds,
        "commands_run": commands_run,
        "files_read": files_read,
        "files_edited": files_edited,
        "tests_or_manual_checks": tests_or_manual_checks,
        "usage_kind": usage_kind,
        "usage": usage,
        "measured": measured,
        "task_review_summary": task_review_summary,
        "code_review_summary": code_review_summary,
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


def _normalize_harness(value: Any) -> dict[str, bool]:
    payload = value if isinstance(value, dict) else {}
    seed_control_present = _coerce_optional_bool(
        payload.get("seed_control_present"),
        payload.get("fixed_seed"),
        payload.get("deterministic_seed"),
    )
    fast_boss_trigger_present = _coerce_optional_bool(
        payload.get("fast_boss_trigger_present"),
        payload.get("fast_boss_trigger"),
        payload.get("boss_after_override"),
    )
    helper_controls_documented = _coerce_optional_bool(
        payload.get("helper_controls_documented"),
        payload.get("documented"),
        payload.get("ui_or_note_documented"),
    )
    gameplay_rules_identical = _coerce_optional_bool(
        payload.get("gameplay_rules_identical"),
        payload.get("rules_preserved"),
        payload.get("benchmark_mode_rules_match_normal"),
    )
    return {
        "seed_control_present": bool(seed_control_present),
        "fast_boss_trigger_present": bool(fast_boss_trigger_present),
        "helper_controls_documented": bool(helper_controls_documented),
        "gameplay_rules_identical": bool(gameplay_rules_identical),
    }


def _normalize_rubric(value: Any) -> dict[str, int]:
    payload = value if isinstance(value, dict) else {}
    aliases = {
        "functional_completeness": ("functional_completeness", "functional"),
        "deterministic_benchmark_harness": ("deterministic_benchmark_harness", "deterministic_harness", "harness"),
        "style_fidelity": ("style_fidelity", "style"),
        "code_structure_and_integration_quality": (
            "code_structure_and_integration_quality",
            "code_structure",
            "integration_quality",
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
    return {
        "mode": mode,
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
            [int(run["usage"]["total_tokens"]) for run in comparable_runs if (run.get("usage") or {}).get("total_tokens") is not None]
        ),
        "verdict": _mode_summary_verdict(total_run_count=len(runs), comparable_runs=comparable_runs, pass_runs=pass_runs, borderline_runs=borderline_runs, minimum_comparable_runs=minimum_comparable_runs),
    }


def _empty_mode_summary(mode: str, minimum_comparable_runs: int) -> dict[str, Any]:
    return {
        "mode": mode,
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
        baseline_mode=baseline_mode,
        candidate_mode=candidate_mode,
        baseline_comparable_run_count=baseline_comparable_run_count,
        candidate_comparable_run_count=candidate_comparable_run_count,
        minimum_comparable_runs=minimum_comparable_runs,
    )
    return {
        "baseline_mode": baseline_mode,
        "candidate_mode": candidate_mode,
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
            f"{candidate_mode} cleared the rubric on enough comparable runs against {baseline_mode}; keep conclusions scoped to this workload and comparison style."
        )
    if verdict == "mixed":
        return (
            f"{candidate_mode} shows some comparable success, but at least one run stayed non-comparable or only borderline. Review blockers before making product claims."
        )
    if verdict == "not_supported":
        return (
            f"{candidate_mode} does not yet sustain comparable completion on this benchmark surface, so do not treat the current runs as proof of execution quality."
        )
    return (
        f"Collect at least {minimum_comparable_runs} comparable runs for both {baseline_mode} and {candidate_mode} before using this benchmark as shared evidence."
    )


def _interpretation_text(payload: dict[str, Any]) -> str:
    aggregate = payload.get("aggregate") if isinstance(payload.get("aggregate"), dict) else {}
    verdict = str(aggregate.get("verdict") or "unproven")
    candidate_mode = str(aggregate.get("candidate_mode") or payload.get("aggregate_candidate_mode") or "")
    baseline_mode = str(aggregate.get("baseline_mode") or payload.get("baseline_mode") or "")
    if verdict == "supported":
        return (
            f"This report supports using {candidate_mode} as a credible compared mode against {baseline_mode} on the static web shooter workload, with startup-script validation and deterministic harness checks recorded."
        )
    if verdict == "mixed":
        return (
            f"This report shows partial support for {candidate_mode}: some runs reached comparable completion, but the record still includes borderline or non-comparable outcomes that limit stronger conclusions."
        )
    if verdict == "not_supported":
        return (
            f"This report does not support {candidate_mode} on this benchmark surface yet because comparable completion and blocker-free execution were not sustained."
        )
    return (
        f"This report is still setup evidence only. Gather more comparable {baseline_mode} and {candidate_mode} runs before treating it as benchmark proof."
    )


def _completion_is_equivalent(run: dict[str, Any]) -> bool:
    raw = run.get("equivalent_task_completion")
    if isinstance(raw, bool):
        return raw
    if raw is not None:
        return str(raw).strip().lower() in EQUIVALENT_COMPLETION_STATES
    for key in ("completion_status", "quality", "quality_gate", "quality_gate_status", "acceptance_status"):
        value = run.get(key)
        if value is None:
            continue
        return str(value).strip().lower() in EQUIVALENT_COMPLETION_STATES
    return False


def _flag_from_run_or_behavior(run: dict[str, Any], required_behavior: list[dict[str, Any]], *, aliases: set[str]) -> bool:
    direct_keys = aliases | {f"clear_{next(iter(aliases))}"}
    for key in aliases:
        if key in run:
            return bool(run.get(key))
    for row in required_behavior:
        behavior_id = str(row.get("id") or "").strip().lower()
        if behavior_id in aliases:
            return bool(row.get("passed"))
    return False


def _normalized_comparison_style(value: Any, *, default: str) -> str:
    style = str(value or default).strip().lower() or default
    if style not in COMPARISON_STYLES:
        allowed = ", ".join(sorted(COMPARISON_STYLES))
        raise ValueError(f"comparison_style must be one of: {allowed}.")
    return style


def _normalized_mode_list(values: Any) -> list[str]:
    if not isinstance(values, list):
        return []
    result: list[str] = []
    for value in values:
        mode = str(value or "").strip()
        if mode and mode not in result:
            result.append(mode)
    return result


def _normalized_text_list(values: Any) -> list[str]:
    if not isinstance(values, list):
        return []
    return [str(value).strip() for value in values if str(value).strip()]


def _extract_usage(payload: dict[str, Any]) -> dict[str, int | None]:
    prompt_tokens = _optional_nonnegative_int(payload.get("prompt_tokens") or payload.get("input_tokens"))
    completion_tokens = _optional_nonnegative_int(payload.get("completion_tokens") or payload.get("output_tokens"))
    total_tokens = _optional_nonnegative_int(payload.get("total_tokens"))
    if total_tokens is None and (prompt_tokens is not None or completion_tokens is not None):
        total_tokens = int(prompt_tokens or 0) + int(completion_tokens or 0)
    return {
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": total_tokens,
        "cached_input_tokens": _optional_nonnegative_int(payload.get("cached_input_tokens")),
        "reasoning_output_tokens": _optional_nonnegative_int(payload.get("reasoning_output_tokens")),
    }


def _coerce_optional_bool(*values: Any) -> bool | None:
    for value in values:
        if isinstance(value, bool):
            return value
        if value is None or value == "":
            continue
        text = str(value).strip().lower()
        if text in {"true", "1", "yes", "y", "passed"}:
            return True
        if text in {"false", "0", "no", "n", "failed"}:
            return False
    return None


def _bounded_int(value: Any, *, minimum: int, maximum: int, default: int) -> int:
    if value is None or value == "":
        return default
    parsed = _optional_nonnegative_int(value)
    if parsed is None:
        return default
    if parsed < minimum or parsed > maximum:
        raise ValueError(f"Expected an integer between {minimum} and {maximum}, got {value!r}.")
    return parsed


def _nonnegative_int(value: Any, *, default: int) -> int:
    if value is None or value == "":
        return default
    parsed = _optional_nonnegative_int(value)
    if parsed is None:
        raise ValueError(f"Expected a non-negative integer, got {value!r}.")
    return parsed


def _optional_nonnegative_int(value: Any) -> int | None:
    if value is None or value == "":
        return None
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        raise ValueError(f"Expected a non-negative integer, got {value!r}.") from None
    if parsed < 0:
        raise ValueError(f"Expected a non-negative integer, got {value!r}.")
    return parsed


def _optional_float(value: Any) -> float | None:
    if value is None or value == "":
        return None
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        raise ValueError(f"Expected a number, got {value!r}.") from None
    if parsed < 0:
        raise ValueError(f"Expected a non-negative number, got {value!r}.")
    return round(parsed, 2)


def _ratio(numerator: int, denominator: int) -> float | None:
    if denominator <= 0:
        return None
    return round((numerator / denominator) * 100.0, 2)


def _median_int(values: list[int]) -> int | None:
    if not values:
        return None
    return int(statistics.median(values))


def _median_float(values: list[float]) -> float | None:
    if not values:
        return None
    return round(float(statistics.median(values)), 2)


def _delta_int(candidate: Any, baseline: Any) -> int | None:
    if candidate is None or baseline is None:
        return None
    return int(candidate) - int(baseline)


def _delta_float(candidate: Any, baseline: Any) -> float | None:
    if candidate is None or baseline is None:
        return None
    return round(float(candidate) - float(baseline), 2)


def _format_percent(value: Any) -> str:
    if value is None:
        return "n/a"
    return f"{float(value):.2f}%"


def _format_int(value: Any) -> str:
    if value is None:
        return "n/a"
    return str(int(value))


def _format_float(value: Any) -> str:
    if value is None:
        return "n/a"
    return f"{float(value):.2f}"


def _list_text(value: Any) -> str:
    items = _normalized_text_list(value)
    return ", ".join(items) if items else "none"


def _review_text(value: Any) -> str:
    if isinstance(value, str):
        text = value.strip()
        return text or "none"
    if isinstance(value, dict):
        parts = [f"{key}={str(item).strip()}" for key, item in value.items() if str(item).strip()]
        return "; ".join(parts) if parts else "none"
    return "none"
