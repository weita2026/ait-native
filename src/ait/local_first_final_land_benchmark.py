from __future__ import annotations

import json
import statistics
from pathlib import Path
from typing import Any

SUPPORTED_STALE_PREFLIGHT = {"fresh", "stale", "unknown"}


def run_local_first_final_land_benchmark(manifest_path: Path) -> dict[str, Any]:
    manifest_path = Path(manifest_path)
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise ValueError(f"Benchmark manifest not found: {manifest_path}") from exc
    except json.JSONDecodeError as exc:
        raise ValueError(f"Benchmark manifest is not valid JSON: {manifest_path}: {exc}") from exc
    return evaluate_local_first_final_land_manifest(manifest, manifest_path=manifest_path)


def evaluate_local_first_final_land_manifest(
    manifest: dict[str, Any],
    *,
    manifest_path: Path | None = None,
) -> dict[str, Any]:
    if not isinstance(manifest, dict):
        raise ValueError("Benchmark manifest must be a JSON object.")
    raw_workloads = manifest.get("workloads")
    if not isinstance(raw_workloads, list) or not raw_workloads:
        raise ValueError("Benchmark manifest must include a non-empty workloads list.")

    benchmark_id = str(manifest.get("benchmark_id") or (manifest_path.stem if manifest_path else "local-first-final-land-benchmark")).strip()
    if not benchmark_id:
        raise ValueError("benchmark_id must not be empty.")
    candidate_mode = str(manifest.get("candidate_mode") or "ait_dag_local_first_final_land_e2e").strip()
    if not candidate_mode:
        raise ValueError("candidate_mode must not be empty.")
    minimum_comparable_runs = _nonnegative_int(manifest.get("minimum_comparable_runs"), default=3)
    known_confounders = _normalized_text_list(manifest.get("known_confounders"))

    normalized_workloads: list[dict[str, Any]] = []
    all_runs: list[dict[str, Any]] = []
    measured_run_count = 0
    measured_total_tokens: list[int] = []

    for index, workload in enumerate(raw_workloads, start=1):
        if not isinstance(workload, dict):
            raise ValueError(f"workload at index {index} must be an object.")
        workload_id = str(workload.get("workload_id") or workload.get("id") or "").strip()
        if not workload_id:
            raise ValueError(f"workload at index {index} is missing workload_id.")
        category = str(workload.get("category") or "unknown").strip() or "unknown"
        title = str(workload.get("title") or workload_id).strip() or workload_id
        raw_runs = workload.get("runs")
        if not isinstance(raw_runs, list) or not raw_runs:
            raise ValueError(f"workload {workload_id!r} must include a non-empty runs list.")
        normalized_runs: list[dict[str, Any]] = []
        for run_index, run in enumerate(raw_runs, start=1):
            if not isinstance(run, dict):
                raise ValueError(f"workload {workload_id!r} run at index {run_index} must be an object.")
            run_id = str(run.get("run_id") or run.get("id") or f"{workload_id}-run-{run_index}").strip()
            mode = str(run.get("mode") or candidate_mode).strip() or candidate_mode
            landed = bool(run.get("landed"))
            operator_recovery_required = bool(run.get("operator_recovery_required"))
            worker_session_count = _optional_nonnegative_int(run.get("worker_session_count"))
            remote_change_count = _optional_nonnegative_int(run.get("remote_change_count"))
            patchset_count = _optional_nonnegative_int(run.get("patchset_count"))
            elapsed_seconds = _optional_float(run.get("elapsed_seconds"))
            stale_preflight = str(run.get("stale_preflight") or "unknown").strip().lower() or "unknown"
            if stale_preflight not in SUPPORTED_STALE_PREFLIGHT:
                raise ValueError(
                    f"run {run_id!r}: stale_preflight must be one of {sorted(SUPPORTED_STALE_PREFLIGHT)}, got {stale_preflight!r}."
                )
            stale_recovery_attempted = bool(run.get("stale_recovery_attempted"))
            stale_recovery_succeeded = bool(run.get("stale_recovery_succeeded"))
            usage_kind = str(run.get("usage_kind") or "unknown").strip().lower() or "unknown"
            usage = _extract_usage(run.get("usage") if isinstance(run.get("usage"), dict) else {})
            total_tokens = usage.get("total_tokens")
            measured = usage_kind == "measured" and total_tokens is not None
            if measured:
                measured_run_count += 1
                measured_total_tokens.append(int(total_tokens))
            stale_case = stale_preflight == "stale"
            automatic_recovery_success = bool(
                stale_case
                and stale_recovery_attempted
                and stale_recovery_succeeded
                and landed
                and not operator_recovery_required
            )
            worker_only_success = bool(
                landed
                and not operator_recovery_required
                and (worker_session_count is None or worker_session_count <= 1)
            )
            normalized = {
                "run_id": run_id,
                "mode": mode,
                "landed": landed,
                "operator_recovery_required": operator_recovery_required,
                "worker_session_count": worker_session_count,
                "remote_change_count": remote_change_count,
                "patchset_count": patchset_count,
                "elapsed_seconds": elapsed_seconds,
                "stale_preflight": stale_preflight,
                "stale_recovery_attempted": stale_recovery_attempted,
                "stale_recovery_succeeded": stale_recovery_succeeded,
                "automatic_recovery_success": automatic_recovery_success,
                "worker_only_success": worker_only_success,
                "usage_kind": usage_kind,
                "usage": usage,
                "measured": measured,
                "notes": str(run.get("notes") or "").strip() or None,
            }
            normalized_runs.append(normalized)
            all_runs.append({**normalized, "workload_id": workload_id, "category": category})
        normalized_workloads.append(
            {
                "workload_id": workload_id,
                "title": title,
                "category": category,
                "runs": normalized_runs,
            }
        )

    comparable_runs = [run for run in all_runs if run.get("mode") == candidate_mode]
    total_run_count = len(comparable_runs)
    landed_runs = [run for run in comparable_runs if run.get("landed")]
    stale_runs = [run for run in comparable_runs if run.get("stale_preflight") == "stale"]
    stale_attempt_runs = [run for run in stale_runs if run.get("stale_recovery_attempted")]
    stale_success_runs = [run for run in stale_runs if run.get("automatic_recovery_success")]
    operator_recovery_runs = [run for run in comparable_runs if run.get("operator_recovery_required")]
    worker_only_success_runs = [run for run in comparable_runs if run.get("worker_only_success")]
    measured_runs = [run for run in comparable_runs if run.get("measured")]

    aggregate = {
        "candidate_mode": candidate_mode,
        "minimum_comparable_runs": minimum_comparable_runs,
        "total_run_count": total_run_count,
        "landed_count": len(landed_runs),
        "landed_rate": _ratio(len(landed_runs), total_run_count),
        "stale_preflight_count": len(stale_runs),
        "stale_recovery_attempt_count": len(stale_attempt_runs),
        "stale_recovery_success_count": len(stale_success_runs),
        "stale_recovery_success_rate": _ratio(len(stale_success_runs), len(stale_attempt_runs)),
        "operator_recovery_required_count": len(operator_recovery_runs),
        "operator_recovery_free_landed_count": len(
            [run for run in landed_runs if not run.get("operator_recovery_required")]
        ),
        "operator_recovery_free_landed_rate": _ratio(
            len([run for run in landed_runs if not run.get("operator_recovery_required")]),
            total_run_count,
        ),
        "single_worker_success_count": len(worker_only_success_runs),
        "single_worker_success_rate": _ratio(len(worker_only_success_runs), total_run_count),
        "measured_run_count": len(measured_runs),
        "measured_total_tokens_sum": sum(int(run["usage"]["total_tokens"]) for run in measured_runs) if measured_runs else None,
        "measured_median_total_tokens": _median_int([int(run["usage"]["total_tokens"]) for run in measured_runs]),
        "median_remote_change_count": _median_int(
            [int(run["remote_change_count"]) for run in comparable_runs if run.get("remote_change_count") is not None]
        ),
        "median_patchset_count": _median_int(
            [int(run["patchset_count"]) for run in comparable_runs if run.get("patchset_count") is not None]
        ),
        "median_elapsed_seconds": _median_float(
            [float(run["elapsed_seconds"]) for run in comparable_runs if run.get("elapsed_seconds") is not None]
        ),
    }
    aggregate["verdict"] = _verdict(aggregate, comparable_runs)
    aggregate["claim_caveat"] = _caveat(aggregate, comparable_runs)

    evidence_type = "measured" if comparable_runs and len(measured_runs) == len(comparable_runs) else "operational"

    return {
        "benchmark_id": benchmark_id,
        "benchmark_kind": str(manifest.get("benchmark_kind") or "local_first_final_remote_land_reliability"),
        "description": manifest.get("description"),
        "candidate_mode": candidate_mode,
        "minimum_comparable_runs": minimum_comparable_runs,
        "evidence_type": evidence_type,
        "known_confounders": known_confounders,
        "workloads": normalized_workloads,
        "aggregate": aggregate,
    }


def render_local_first_final_land_markdown(payload: dict[str, Any]) -> str:
    aggregate = payload.get("aggregate") if isinstance(payload.get("aggregate"), dict) else {}
    lines = [
        f"# {payload.get('benchmark_id') or 'Local-First Final-Land Benchmark'}",
        "",
        "Generated local-first final remote land benchmark report.",
        "",
        f"- Benchmark kind: `{payload.get('benchmark_kind') or 'local_first_final_remote_land_reliability'}`",
        f"- Evidence type: `{payload.get('evidence_type') or 'operational'}`",
        f"- Candidate mode: `{payload.get('candidate_mode') or ''}`",
        f"- Verdict: `{aggregate.get('verdict') or ''}`",
        f"- Total runs: `{aggregate.get('total_run_count')}`",
        f"- Landed rate: `{_format_percent(aggregate.get('landed_rate'))}`",
        f"- Single-worker success rate: `{_format_percent(aggregate.get('single_worker_success_rate'))}`",
        f"- Stale recovery success rate: `{_format_percent(aggregate.get('stale_recovery_success_rate'))}`",
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
            "## Run summary",
            "",
            "| Workload | Category | Run | Mode | Landed | Stale preflight | Auto recovery | Operator recovery | Worker sessions | Remote changes | Patchsets | Elapsed (s) | Total tokens |",
            "| --- | --- | --- | --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for workload in payload.get("workloads") or []:
        for run in workload.get("runs") or []:
            lines.append(
                "| {workload_id} | {category} | {run_id} | {mode} | {landed} | {stale} | {auto} | {operator} | {workers} | {changes} | {patchsets} | {elapsed} | {tokens} |".format(
                    workload_id=workload.get("workload_id") or "",
                    category=workload.get("category") or "",
                    run_id=run.get("run_id") or "",
                    mode=run.get("mode") or "",
                    landed="yes" if run.get("landed") else "no",
                    stale=run.get("stale_preflight") or "unknown",
                    auto="yes" if run.get("automatic_recovery_success") else "no",
                    operator="yes" if run.get("operator_recovery_required") else "no",
                    workers=_format_int(run.get("worker_session_count")),
                    changes=_format_int(run.get("remote_change_count")),
                    patchsets=_format_int(run.get("patchset_count")),
                    elapsed=_format_float(run.get("elapsed_seconds")),
                    tokens=_format_int((run.get("usage") or {}).get("total_tokens")),
                )
            )
    lines.extend(
        [
            "",
            "## Interpretation",
            "",
            _interpretation_text(payload),
            "",
        ]
    )
    return "\n".join(lines)


def _verdict(aggregate: dict[str, Any], comparable_runs: list[dict[str, Any]]) -> str:
    total_run_count = int(aggregate.get("total_run_count") or 0)
    minimum = int(aggregate.get("minimum_comparable_runs") or 0)
    landed_count = int(aggregate.get("landed_count") or 0)
    stale_preflight_count = int(aggregate.get("stale_preflight_count") or 0)
    stale_recovery_success_count = int(aggregate.get("stale_recovery_success_count") or 0)
    single_worker_success_count = int(aggregate.get("single_worker_success_count") or 0)
    operator_recovery_required_count = int(aggregate.get("operator_recovery_required_count") or 0)
    if total_run_count < minimum:
        return "unproven"
    if landed_count < total_run_count:
        return "not_supported"
    if stale_recovery_success_count < stale_preflight_count:
        return "mixed"
    if operator_recovery_required_count > 0:
        return "mixed"
    if single_worker_success_count == total_run_count:
        return "supported"
    return "mixed"


def _caveat(aggregate: dict[str, Any], comparable_runs: list[dict[str, Any]]) -> str:
    verdict = str(aggregate.get("verdict") or "unproven")
    if verdict == "supported":
        return (
            "All comparable runs landed, stale targets recovered without operator repair, and the candidate stayed within a one-worker execution record."
        )
    if verdict == "mixed":
        return (
            "The candidate landed, but at least one run required operator recovery, extra worker sessions, or did not prove automatic stale-base recovery."
        )
    if verdict == "not_supported":
        return "At least one comparable run failed to land, so the current local-first final-land contract is not yet reliable enough for a benchmark claim."
    return "Collect more comparable runs before using this benchmark as evidence."


def _interpretation_text(payload: dict[str, Any]) -> str:
    aggregate = payload.get("aggregate") if isinstance(payload.get("aggregate"), dict) else {}
    if aggregate.get("verdict") == "supported":
        return (
            "This report supports the operational claim that the local-first final remote land path can converge in one worker session, recover stale targets automatically, and land without coordinator repair on the measured workloads."
        )
    if aggregate.get("verdict") == "mixed":
        return (
            "This report shows partial success: the final-land path can land, but some runs still required extra help or did not prove full worker-only stale-base recovery."
        )
    if aggregate.get("verdict") == "not_supported":
        return (
            "This report does not yet support the operational reliability claim because one or more comparable runs failed before remote land."
        )
    return "This report is still gathering comparable runs and should be treated as setup evidence only."


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
