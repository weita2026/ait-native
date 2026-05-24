from __future__ import annotations

import json
import statistics
from pathlib import Path
from typing import Any

BENCHMARK_KIND = "local_snapshot_performance"
DEFAULT_BASELINE_MODE = "git_baseline"
DEFAULT_CANDIDATE_MODE = "ait_local"
DEFAULT_REQUIRED_CASES = (
    ("workspace_status", "first_run"),
    ("workspace_status", "warm_noop"),
    ("snapshot_create", "first_run"),
    ("snapshot_create", "warm_noop"),
    ("push", "first_run"),
    ("push", "warm_noop"),
)
SUPPORTED_OPERATIONS = {operation for operation, _phase in DEFAULT_REQUIRED_CASES}
SUPPORTED_PHASES = {phase for _operation, phase in DEFAULT_REQUIRED_CASES}
SUPPORTED_EVIDENCE_TYPES = {"measured", "operational"}


def run_local_snapshot_performance_benchmark(manifest_path: Path) -> dict[str, Any]:
    manifest_path = Path(manifest_path)
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise ValueError(f"Benchmark manifest not found: {manifest_path}") from exc
    except json.JSONDecodeError as exc:
        raise ValueError(f"Benchmark manifest is not valid JSON: {manifest_path}: {exc}") from exc
    return evaluate_local_snapshot_performance_manifest(manifest, manifest_path=manifest_path)


def evaluate_local_snapshot_performance_manifest(
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
    candidate_mode = str(manifest.get("candidate_mode") or DEFAULT_CANDIDATE_MODE).strip() or DEFAULT_CANDIDATE_MODE
    baseline_mode = str(manifest.get("baseline_mode") or DEFAULT_BASELINE_MODE).strip() or DEFAULT_BASELINE_MODE
    evidence_type = str(manifest.get("evidence_type") or "measured").strip().lower() or "measured"
    if evidence_type not in SUPPORTED_EVIDENCE_TYPES:
        raise ValueError(f"evidence_type must be one of {sorted(SUPPORTED_EVIDENCE_TYPES)}.")
    minimum_comparable_runs = _nonnegative_int(manifest.get("minimum_comparable_runs"), default=1)
    known_confounders = _normalized_text_list(manifest.get("known_confounders"))
    description = str(manifest.get("description") or "").strip() or None
    required_cases = _normalized_required_cases(manifest.get("required_cases"))
    budget_targets = _normalized_budget_targets(manifest.get("budget_targets"))
    for target in budget_targets:
        case_key = (target["operation"], target["phase"])
        if case_key not in required_cases:
            raise ValueError(
                "budget_targets must only reference required_cases; "
                f"{target['operation']}/{target['phase']} is not currently required."
            )
    budget_targets_by_case = {(target["operation"], target["phase"]): target for target in budget_targets}

    normalized_workloads: list[dict[str, Any]] = []
    all_runs: list[dict[str, Any]] = []
    workspace_profiles: list[dict[str, Any]] = []

    for workload_index, workload in enumerate(raw_workloads, start=1):
        if not isinstance(workload, dict):
            raise ValueError(f"workload at index {workload_index} must be an object.")
        workload_id = str(workload.get("workload_id") or workload.get("id") or "").strip()
        if not workload_id:
            raise ValueError(f"workload at index {workload_index} is missing workload_id.")
        title = str(workload.get("title") or workload_id).strip() or workload_id
        category = str(workload.get("category") or "large_workspace").strip() or "large_workspace"
        workspace_profile = _normalize_workspace_profile(workload.get("workspace_profile"), workload_id=workload_id)
        if workspace_profile:
            workspace_profiles.append(workspace_profile)
        raw_runs = workload.get("runs")
        if not isinstance(raw_runs, list) or not raw_runs:
            raise ValueError(f"workload {workload_id!r} must include a non-empty runs list.")

        normalized_runs: list[dict[str, Any]] = []
        for run_index, run in enumerate(raw_runs, start=1):
            if not isinstance(run, dict):
                raise ValueError(f"workload {workload_id!r} run at index {run_index} must be an object.")
            run_id = str(run.get("run_id") or run.get("id") or f"{workload_id}-run-{run_index}").strip()
            mode = str(run.get("mode") or candidate_mode).strip() or candidate_mode
            operation = str(run.get("operation") or "").strip()
            phase = str(run.get("phase") or "").strip()
            if operation not in SUPPORTED_OPERATIONS:
                raise ValueError(f"run {run_id!r}: operation must be one of {sorted(SUPPORTED_OPERATIONS)}.")
            if phase not in SUPPORTED_PHASES:
                raise ValueError(f"run {run_id!r}: phase must be one of {sorted(SUPPORTED_PHASES)}.")
            elapsed_seconds = _nonnegative_float(run.get("elapsed_seconds"), field=f"run {run_id!r} elapsed_seconds")
            healthz_ok = _optional_bool(run.get("healthz_ok"))
            if operation == "push" and healthz_ok is None:
                raise ValueError(f"run {run_id!r}: push runs must record healthz_ok.")
            normalized = {
                "run_id": run_id,
                "mode": mode,
                "operation": operation,
                "phase": phase,
                "elapsed_seconds": elapsed_seconds,
                "healthz_ok": healthz_ok,
                "file_count": _optional_nonnegative_int(run.get("file_count")),
                "total_bytes": _optional_nonnegative_int(run.get("total_bytes")),
                "notes": str(run.get("notes") or "").strip() or None,
            }
            normalized_runs.append(normalized)
            all_runs.append({**normalized, "workload_id": workload_id, "title": title, "category": category})

        normalized_workloads.append(
            {
                "workload_id": workload_id,
                "title": title,
                "category": category,
                "workspace_profile": workspace_profile,
                "runs": normalized_runs,
            }
        )

    case_summaries: list[dict[str, Any]] = []
    comparable_case_count = 0
    push_health_green_case_count = 0
    required_push_case_count = 0
    candidate_run_count = 0
    baseline_run_count = 0

    for operation, phase in required_cases:
        candidate_runs = [
            run
            for run in all_runs
            if run.get("mode") == candidate_mode and run.get("operation") == operation and run.get("phase") == phase
        ]
        baseline_runs = [
            run
            for run in all_runs
            if run.get("mode") == baseline_mode and run.get("operation") == operation and run.get("phase") == phase
        ]
        candidate_run_count += len(candidate_runs)
        baseline_run_count += len(baseline_runs)
        candidate_elapsed = [float(run["elapsed_seconds"]) for run in candidate_runs]
        baseline_elapsed = [float(run["elapsed_seconds"]) for run in baseline_runs]
        candidate_median = _median_float(candidate_elapsed)
        baseline_median = _median_float(baseline_elapsed)
        ratio = _ratio(candidate_median, baseline_median)
        comparable = len(candidate_runs) >= minimum_comparable_runs and len(baseline_runs) >= minimum_comparable_runs
        if comparable:
            comparable_case_count += 1
        push_health_green = None
        if operation == "push":
            required_push_case_count += 1
            push_health_green = bool(candidate_runs) and all(bool(run.get("healthz_ok")) for run in candidate_runs)
            if push_health_green:
                push_health_green_case_count += 1
        budget_target = budget_targets_by_case.get((operation, phase))
        budget_result = _evaluate_budget_target(
            budget_target,
            comparable=comparable,
            candidate_median=candidate_median,
            ratio=ratio,
            push_health_green=push_health_green,
        )
        case_summaries.append(
            {
                "operation": operation,
                "phase": phase,
                "candidate_mode": candidate_mode,
                "baseline_mode": baseline_mode,
                "candidate_run_count": len(candidate_runs),
                "baseline_run_count": len(baseline_runs),
                "candidate_median_elapsed_seconds": candidate_median,
                "baseline_median_elapsed_seconds": baseline_median,
                "candidate_vs_baseline_ratio": ratio,
                "comparable": comparable,
                "candidate_push_health_green": push_health_green,
                "budget_target": budget_target,
                "budget_result": budget_result,
            }
        )

    budget_summary = _summarize_budget(case_summaries, budget_targets=budget_targets)
    aggregate = {
        "candidate_mode": candidate_mode,
        "baseline_mode": baseline_mode,
        "minimum_comparable_runs": minimum_comparable_runs,
        "required_case_count": len(required_cases),
        "comparable_case_count": comparable_case_count,
        "candidate_run_count": candidate_run_count,
        "baseline_run_count": baseline_run_count,
        "push_health_green_case_count": push_health_green_case_count,
        "required_push_case_count": required_push_case_count,
        "workspace_profile_count": len(workspace_profiles),
        "verdict": _verdict(
            comparable_case_count=comparable_case_count,
            required_case_count=len(required_cases),
            push_health_green_case_count=push_health_green_case_count,
            required_push_case_count=required_push_case_count,
        ),
    }
    aggregate["claim_caveat"] = _caveat(aggregate)
    if budget_summary is not None:
        aggregate["budget_verdict"] = budget_summary["verdict"]

    return {
        "benchmark_id": benchmark_id,
        "benchmark_kind": benchmark_kind,
        "manifest_path": str(manifest_path) if manifest_path is not None else None,
        "description": description,
        "evidence_type": evidence_type,
        "candidate_mode": candidate_mode,
        "baseline_mode": baseline_mode,
        "minimum_comparable_runs": minimum_comparable_runs,
        "required_cases": [{"operation": operation, "phase": phase} for operation, phase in required_cases],
        "budget_targets": budget_targets,
        "known_confounders": known_confounders,
        "workloads": normalized_workloads,
        "case_summaries": case_summaries,
        "budget_summary": budget_summary,
        "aggregate": aggregate,
    }


def render_local_snapshot_performance_markdown(payload: dict[str, Any]) -> str:
    aggregate = payload.get("aggregate") if isinstance(payload.get("aggregate"), dict) else {}
    lines = [
        f"# {payload.get('benchmark_id') or 'Local Snapshot Performance Benchmark'}",
        "",
        "Generated local snapshot performance benchmark report.",
        "",
        f"- Benchmark kind: `{payload.get('benchmark_kind') or BENCHMARK_KIND}`",
        f"- Evidence type: `{payload.get('evidence_type') or 'measured'}`",
        f"- Candidate mode: `{payload.get('candidate_mode') or DEFAULT_CANDIDATE_MODE}`",
        f"- Baseline mode: `{payload.get('baseline_mode') or DEFAULT_BASELINE_MODE}`",
        f"- Verdict: `{aggregate.get('verdict') or ''}`",
        f"- Comparable cases: `{aggregate.get('comparable_case_count')}` / `{aggregate.get('required_case_count')}`",
        f"- Push health green cases: `{aggregate.get('push_health_green_case_count')}` / `{aggregate.get('required_push_case_count')}`",
        f"- Caveat: {aggregate.get('claim_caveat') or ''}",
    ]
    budget_summary = payload.get("budget_summary") if isinstance(payload.get("budget_summary"), dict) else None
    if budget_summary:
        lines.extend(
            [
                f"- Budget verdict: `{budget_summary.get('verdict') or ''}`",
                (
                    "- Budget tracked cases: "
                    f"`{budget_summary.get('passed_case_count')}` passed / "
                    f"`{budget_summary.get('tracked_case_count')}` tracked"
                ),
            ]
        )
    if payload.get("description"):
        lines.extend(["", str(payload.get("description"))])
    workspace_profiles = []
    for workload in payload.get("workloads") or []:
        profile = workload.get("workspace_profile") if isinstance(workload.get("workspace_profile"), dict) else {}
        workspace_name = str(profile.get("workspace_name") or workload.get("title") or workload.get("workload_id") or "").strip()
        if workspace_name:
            workspace_profiles.append(
                {
                    "workspace_name": workspace_name,
                    "file_count": profile.get("file_count"),
                    "total_bytes": profile.get("total_bytes"),
                }
            )
    if workspace_profiles:
        lines.extend(["", "## Workspace Profiles", "", "| Workspace | Files | Bytes |", "| --- | ---: | ---: |"])
        for profile in workspace_profiles:
            lines.append(
                "| {workspace} | {files} | {bytes_} |".format(
                    workspace=profile.get("workspace_name") or "",
                    files=_format_int(profile.get("file_count")),
                    bytes_=_format_int(profile.get("total_bytes")),
                )
            )
    known_confounders = _normalized_text_list(payload.get("known_confounders"))
    if known_confounders:
        lines.extend(["", "## Known Confounders", ""])
        lines.extend(f"- {item}" for item in known_confounders)
    lines.extend(
        [
            "",
            "## Case Summary",
            "",
            "| Operation | Phase | Candidate runs | Baseline runs | Candidate median (s) | Baseline median (s) | Ratio vs baseline | Push health green | Comparable |",
            "| --- | --- | ---: | ---: | ---: | ---: | ---: | --- | --- |",
        ]
    )
    for case in payload.get("case_summaries") or []:
        push_health = case.get("candidate_push_health_green")
        push_health_text = "n/a" if push_health is None else ("yes" if push_health else "no")
        lines.append(
            "| {operation} | {phase} | {candidate_runs} | {baseline_runs} | {candidate_median} | {baseline_median} | {ratio} | {push_health} | {comparable} |".format(
                operation=case.get("operation") or "",
                phase=case.get("phase") or "",
                candidate_runs=_format_int(case.get("candidate_run_count")),
                baseline_runs=_format_int(case.get("baseline_run_count")),
                candidate_median=_format_float(case.get("candidate_median_elapsed_seconds")),
                baseline_median=_format_float(case.get("baseline_median_elapsed_seconds")),
                ratio=_format_ratio(case.get("candidate_vs_baseline_ratio")),
                push_health=push_health_text,
                comparable="yes" if case.get("comparable") else "no",
            )
        )
    if budget_summary:
        lines.extend(
            [
                "",
                "## Performance Budget",
                "",
                "| Operation | Phase | Max candidate median (s) | Max ratio vs baseline | Require push health green | Verdict | Reason |",
                "| --- | --- | ---: | ---: | --- | --- | --- |",
            ]
        )
        for case in payload.get("case_summaries") or []:
            target = case.get("budget_target") if isinstance(case.get("budget_target"), dict) else None
            result = case.get("budget_result") if isinstance(case.get("budget_result"), dict) else None
            if not target:
                continue
            lines.append(
                "| {operation} | {phase} | {elapsed_budget} | {ratio_budget} | {require_push_health} | {verdict} | {reason} |".format(
                    operation=case.get("operation") or "",
                    phase=case.get("phase") or "",
                    elapsed_budget=_format_float(target.get("max_candidate_elapsed_seconds")),
                    ratio_budget=_format_ratio(target.get("max_candidate_vs_baseline_ratio")),
                    require_push_health="yes" if target.get("require_push_health_green") else "no",
                    verdict=result.get("verdict") if result else "untracked",
                    reason=result.get("reason") if result else "",
                )
            )
    lines.extend(
        [
            "",
            "## Run Inventory",
            "",
            "| Workload | Category | Run | Mode | Operation | Phase | Elapsed (s) | Healthz green | Files | Bytes |",
            "| --- | --- | --- | --- | --- | --- | ---: | --- | ---: | ---: |",
        ]
    )
    for workload in payload.get("workloads") or []:
        for run in workload.get("runs") or []:
            healthz_ok = run.get("healthz_ok")
            health_text = "n/a" if healthz_ok is None else ("yes" if healthz_ok else "no")
            lines.append(
                "| {workload_id} | {category} | {run_id} | {mode} | {operation} | {phase} | {elapsed} | {health} | {files} | {bytes_} |".format(
                    workload_id=workload.get("workload_id") or "",
                    category=workload.get("category") or "",
                    run_id=run.get("run_id") or "",
                    mode=run.get("mode") or "",
                    operation=run.get("operation") or "",
                    phase=run.get("phase") or "",
                    elapsed=_format_float(run.get("elapsed_seconds")),
                    health=health_text,
                    files=_format_int(run.get("file_count")),
                    bytes_=_format_int(run.get("total_bytes")),
                )
            )
    lines.extend(["", "## Interpretation", "", _interpretation_text(payload), ""])
    return "\n".join(lines)


def _verdict(
    *,
    comparable_case_count: int,
    required_case_count: int,
    push_health_green_case_count: int,
    required_push_case_count: int,
) -> str:
    if comparable_case_count < required_case_count:
        return "incomplete"
    if required_push_case_count and push_health_green_case_count < required_push_case_count:
        return "push_unhealthy"
    return "ready_for_budgeting"


def _caveat(aggregate: dict[str, Any]) -> str:
    verdict = str(aggregate.get("verdict") or "incomplete")
    if verdict == "ready_for_budgeting":
        return "All required operation/phase cases are present with candidate and baseline runs, and every tracked push case kept health checks green."
    if verdict == "push_unhealthy":
        return "The benchmark matrix is present, but at least one tracked push case did not keep health checks green."
    return "The benchmark harness exists, but one or more required operation/phase cases are still missing candidate or baseline evidence."


def _interpretation_text(payload: dict[str, Any]) -> str:
    aggregate = payload.get("aggregate") if isinstance(payload.get("aggregate"), dict) else {}
    verdict = str(aggregate.get("verdict") or "incomplete")
    budget_summary = payload.get("budget_summary") if isinstance(payload.get("budget_summary"), dict) else None
    if verdict == "ready_for_budgeting" and budget_summary:
        budget_verdict = str(budget_summary.get("verdict") or "pending")
        if budget_verdict == "pass":
            return (
                "This report has the full benchmark matrix and every tracked budget threshold currently passes, so it can support milestone-closeout budget review."
            )
        if budget_verdict == "fail":
            return (
                "This report has the full benchmark matrix, but at least one tracked performance-budget threshold is still failing and needs follow-up optimization."
            )
        return (
            "This report has the full benchmark matrix and now tracks explicit budget thresholds, but one or more tracked cases still need comparable evidence before a closeout budget claim is honest."
        )
    if verdict == "ready_for_budgeting":
        return (
            "This report is complete enough to support the later M1A timing and performance-budget slices: every required operation/phase case has both candidate and Git-baseline evidence, and push runs stayed operationally healthy."
        )
    if verdict == "push_unhealthy":
        return (
            "This report has the expected benchmark shape, but the push path still needs operational hardening because at least one push case lost a green health check."
        )
    return (
        "This report shows the benchmark harness wiring, but the benchmark matrix is not complete enough yet to support milestone-closeout budget claims."
    )


def _normalized_required_cases(value: Any) -> tuple[tuple[str, str], ...]:
    if value is None:
        return DEFAULT_REQUIRED_CASES
    if not isinstance(value, list) or not value:
        raise ValueError("required_cases must be a non-empty list when provided.")
    cases: list[tuple[str, str]] = []
    for index, item in enumerate(value, start=1):
        if not isinstance(item, dict):
            raise ValueError(f"required_cases[{index}] must be an object.")
        operation = str(item.get("operation") or "").strip()
        phase = str(item.get("phase") or "").strip()
        if operation not in SUPPORTED_OPERATIONS:
            raise ValueError(f"required_cases[{index}].operation must be one of {sorted(SUPPORTED_OPERATIONS)}.")
        if phase not in SUPPORTED_PHASES:
            raise ValueError(f"required_cases[{index}].phase must be one of {sorted(SUPPORTED_PHASES)}.")
        cases.append((operation, phase))
    return tuple(cases)


def _normalized_budget_targets(value: Any) -> list[dict[str, Any]]:
    if value is None:
        return []
    if not isinstance(value, list) or not value:
        raise ValueError("budget_targets must be a non-empty list when provided.")
    targets: list[dict[str, Any]] = []
    seen_cases: set[tuple[str, str]] = set()
    for index, item in enumerate(value, start=1):
        if not isinstance(item, dict):
            raise ValueError(f"budget_targets[{index}] must be an object.")
        operation = str(item.get("operation") or "").strip()
        phase = str(item.get("phase") or "").strip()
        if operation not in SUPPORTED_OPERATIONS:
            raise ValueError(f"budget_targets[{index}].operation must be one of {sorted(SUPPORTED_OPERATIONS)}.")
        if phase not in SUPPORTED_PHASES:
            raise ValueError(f"budget_targets[{index}].phase must be one of {sorted(SUPPORTED_PHASES)}.")
        case_key = (operation, phase)
        if case_key in seen_cases:
            raise ValueError(f"budget_targets[{index}] duplicates {operation}/{phase}.")
        seen_cases.add(case_key)
        max_candidate_elapsed_seconds = _optional_nonnegative_float(item.get("max_candidate_elapsed_seconds"))
        max_candidate_vs_baseline_ratio = _optional_nonnegative_float(item.get("max_candidate_vs_baseline_ratio"))
        require_push_health_green = _optional_bool(item.get("require_push_health_green"))
        if require_push_health_green and operation != "push":
            raise ValueError(
                f"budget_targets[{index}].require_push_health_green is only valid for push cases."
            )
        if (
            max_candidate_elapsed_seconds is None
            and max_candidate_vs_baseline_ratio is None
            and require_push_health_green is not True
        ):
            raise ValueError(
                "budget_targets[{index}] must set at least one threshold.".format(index=index)
            )
        targets.append(
            {
                "operation": operation,
                "phase": phase,
                "max_candidate_elapsed_seconds": max_candidate_elapsed_seconds,
                "max_candidate_vs_baseline_ratio": max_candidate_vs_baseline_ratio,
                "require_push_health_green": require_push_health_green is True,
                "notes": str(item.get("notes") or "").strip() or None,
            }
        )
    return targets


def _evaluate_budget_target(
    budget_target: dict[str, Any] | None,
    *,
    comparable: bool,
    candidate_median: float | None,
    ratio: float | None,
    push_health_green: bool | None,
) -> dict[str, Any] | None:
    if budget_target is None:
        return None
    if not comparable:
        return {
            "tracked": True,
            "verdict": "pending",
            "reason": "Comparable candidate/baseline evidence is still missing for this case.",
        }

    failed_checks: list[str] = []
    checks: dict[str, Any] = {}

    elapsed_budget = budget_target.get("max_candidate_elapsed_seconds")
    if elapsed_budget is not None:
        elapsed_pass = candidate_median is not None and candidate_median <= float(elapsed_budget)
        checks["candidate_elapsed_seconds"] = {
            "actual": candidate_median,
            "threshold": float(elapsed_budget),
            "pass": bool(elapsed_pass),
        }
        if not elapsed_pass:
            failed_checks.append("candidate median exceeded the tracked seconds budget")

    ratio_budget = budget_target.get("max_candidate_vs_baseline_ratio")
    if ratio_budget is not None:
        ratio_pass = ratio is not None and ratio <= float(ratio_budget)
        checks["candidate_vs_baseline_ratio"] = {
            "actual": ratio,
            "threshold": float(ratio_budget),
            "pass": bool(ratio_pass),
        }
        if not ratio_pass:
            failed_checks.append("candidate/baseline ratio exceeded the tracked budget")

    if budget_target.get("require_push_health_green"):
        push_pass = push_health_green is True
        checks["push_health_green"] = {
            "actual": push_health_green,
            "required": True,
            "pass": push_pass,
        }
        if not push_pass:
            failed_checks.append("push health checks did not stay green")

    if failed_checks:
        return {
            "tracked": True,
            "verdict": "fail",
            "reason": "; ".join(failed_checks),
            "checks": checks,
        }
    return {
        "tracked": True,
        "verdict": "pass",
        "reason": "All tracked thresholds passed for this case.",
        "checks": checks,
    }


def _summarize_budget(case_summaries: list[dict[str, Any]], *, budget_targets: list[dict[str, Any]]) -> dict[str, Any] | None:
    if not budget_targets:
        return None
    tracked_case_count = len(budget_targets)
    passed_case_count = 0
    failed_case_count = 0
    pending_case_count = 0
    for case in case_summaries:
        if not isinstance(case.get("budget_target"), dict):
            continue
        result = case.get("budget_result") if isinstance(case.get("budget_result"), dict) else {}
        verdict = str(result.get("verdict") or "pending")
        if verdict == "pass":
            passed_case_count += 1
        elif verdict == "fail":
            failed_case_count += 1
        else:
            pending_case_count += 1
    summary_verdict = "pass"
    if failed_case_count:
        summary_verdict = "fail"
    elif pending_case_count:
        summary_verdict = "pending"
    return {
        "tracked_case_count": tracked_case_count,
        "passed_case_count": passed_case_count,
        "failed_case_count": failed_case_count,
        "pending_case_count": pending_case_count,
        "verdict": summary_verdict,
    }


def _normalize_workspace_profile(value: Any, *, workload_id: str) -> dict[str, Any] | None:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ValueError(f"workload {workload_id!r}: workspace_profile must be an object when provided.")
    profile = {
        "workspace_name": str(value.get("workspace_name") or workload_id).strip() or workload_id,
        "file_count": _optional_nonnegative_int(value.get("file_count")),
        "total_bytes": _optional_nonnegative_int(value.get("total_bytes")),
        "notes": str(value.get("notes") or "").strip() or None,
    }
    return profile


def _normalized_text_list(values: Any) -> list[str]:
    if not isinstance(values, list):
        return []
    return [str(value).strip() for value in values if str(value).strip()]


def _median_float(values: list[float]) -> float | None:
    if not values:
        return None
    return round(float(statistics.median(values)), 6)


def _ratio(candidate: float | None, baseline: float | None) -> float | None:
    if candidate is None or baseline is None or baseline <= 0:
        return None
    return round(candidate / baseline, 6)


def _nonnegative_int(value: Any, *, default: int) -> int:
    normalized = _optional_nonnegative_int(value)
    return default if normalized is None else int(normalized)


def _optional_nonnegative_int(value: Any) -> int | None:
    if value is None or value == "":
        return None
    try:
        normalized = int(value)
    except (TypeError, ValueError) as exc:
        raise ValueError(f"Expected a non-negative integer, got {value!r}.") from exc
    if normalized < 0:
        raise ValueError(f"Expected a non-negative integer, got {value!r}.")
    return normalized


def _nonnegative_float(value: Any, *, field: str) -> float:
    try:
        normalized = float(value)
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{field} must be a non-negative number.") from exc
    if normalized < 0:
        raise ValueError(f"{field} must be a non-negative number.")
    return round(normalized, 6)


def _optional_nonnegative_float(value: Any) -> float | None:
    if value is None or value == "":
        return None
    try:
        normalized = float(value)
    except (TypeError, ValueError) as exc:
        raise ValueError(f"Expected a non-negative number, got {value!r}.") from exc
    if normalized < 0:
        raise ValueError(f"Expected a non-negative number, got {value!r}.")
    return round(normalized, 6)


def _optional_bool(value: Any) -> bool | None:
    if value is None:
        return None
    if isinstance(value, bool):
        return value
    text = str(value).strip().lower()
    if text in {"true", "yes", "1"}:
        return True
    if text in {"false", "no", "0"}:
        return False
    raise ValueError(f"Expected a boolean value, got {value!r}.")


def _format_float(value: Any) -> str:
    if value is None:
        return "n/a"
    rendered = f"{float(value):.3f}"
    return rendered.rstrip("0").rstrip(".") if "." in rendered else rendered


def _format_int(value: Any) -> str:
    if value is None:
        return "n/a"
    return f"{int(value)}"


def _format_ratio(value: Any) -> str:
    if value is None:
        return "n/a"
    rendered = f"{float(value):.2f}x"
    return rendered
