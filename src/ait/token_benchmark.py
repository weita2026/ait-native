from __future__ import annotations

import json
import statistics
from pathlib import Path
from typing import Any

PASSED_QUALITY_STATES = {"passed", "pass", "success", "ok", "complete", "completed"}
EQUIVALENT_COMPLETION_STATES = {"equivalent", "complete", "completed", "accepted", "done"}
STATIC_WEB_HARDENING_BENCHMARK_KIND = "static_web_hardening_task"
STATIC_WEB_HARDENING_DAG_PREP_ROLES = ("sprint_card_setup", "dag_json_authoring")
STATIC_WEB_HARDENING_DAG_EXECUTION_ROLE = "worker_execution"


def run_token_savings_benchmark(manifest_path: Path) -> dict[str, Any]:
    manifest_path = Path(manifest_path)
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise ValueError(f"Benchmark manifest not found: {manifest_path}") from exc
    except json.JSONDecodeError as exc:
        raise ValueError(f"Benchmark manifest is not valid JSON: {manifest_path}: {exc}") from exc
    return evaluate_token_savings_manifest(manifest, manifest_path=manifest_path)


def extract_codex_token_usage(session_jsonl_path: Path) -> dict[str, Any]:
    """Extract the latest Codex token_count usage from a local session JSONL file."""

    session_jsonl_path = Path(session_jsonl_path)
    try:
        lines = session_jsonl_path.read_text(encoding="utf-8").splitlines()
    except FileNotFoundError as exc:
        raise ValueError(f"Codex session JSONL not found: {session_jsonl_path}") from exc

    latest_info: dict[str, Any] | None = None
    latest_turn_completed_usage: dict[str, Any] | None = None
    event_count = 0
    for lineno, line in enumerate(lines, start=1):
        if not line.strip():
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ValueError(f"Codex session JSONL is not valid JSONL: {session_jsonl_path}:{lineno}: {exc}") from exc
        if not isinstance(record, dict):
            continue
        if record.get("type") == "turn.completed" and isinstance(record.get("usage"), dict):
            latest_turn_completed_usage = record["usage"]
            event_count += 1
            continue
        payload = record.get("payload")
        if not isinstance(payload, dict) or payload.get("type") != "token_count":
            continue
        info = payload.get("info")
        if not isinstance(info, dict):
            continue
        if not isinstance(info.get("total_token_usage"), dict):
            continue
        latest_info = info
        event_count += 1

    if latest_info is None and latest_turn_completed_usage is None:
        raise ValueError(f"No Codex token_count usage found in: {session_jsonl_path}")

    if latest_info is not None:
        last_usage = _extract_codex_usage_payload(latest_info.get("last_token_usage") or {})
        total_usage = _extract_codex_usage_payload(latest_info.get("total_token_usage") or {})
        measured_usage = _codex_usage_for_manifest(total_usage)
        usage_source = "codex_jsonl_total_token_usage"
    else:
        last_usage = _extract_codex_usage_payload(latest_turn_completed_usage or {})
        total_usage = _extract_codex_usage_payload(latest_turn_completed_usage or {})
        measured_usage = _codex_usage_for_manifest(total_usage)
        usage_source = "codex_exec_turn_completed_usage"
    return {
        "session_jsonl_path": str(session_jsonl_path),
        "token_event_count": event_count,
        "usage_source": usage_source,
        "last_token_usage": last_usage,
        "total_token_usage": total_usage,
        "manifest_usage": measured_usage,
    }


def _summed_manifest_usage(rows: list[dict[str, Any]]) -> dict[str, int | None]:
    prompt_tokens = 0
    completion_tokens = 0
    total_tokens = 0
    cached_input_tokens = 0
    reasoning_output_tokens = 0
    saw_prompt = False
    saw_completion = False
    saw_total = False
    saw_cached = False
    saw_reasoning = False

    for row in rows:
        usage = row.get("usage") if isinstance(row.get("usage"), dict) else {}
        if usage.get("prompt_tokens") is not None:
            prompt_tokens += int(usage["prompt_tokens"])
            saw_prompt = True
        if usage.get("completion_tokens") is not None:
            completion_tokens += int(usage["completion_tokens"])
            saw_completion = True
        if usage.get("total_tokens") is not None:
            total_tokens += int(usage["total_tokens"])
            saw_total = True
        if usage.get("cached_input_tokens") is not None:
            cached_input_tokens += int(usage["cached_input_tokens"])
            saw_cached = True
        if usage.get("reasoning_output_tokens") is not None:
            reasoning_output_tokens += int(usage["reasoning_output_tokens"])
            saw_reasoning = True

    return {
        "prompt_tokens": prompt_tokens if saw_prompt else None,
        "completion_tokens": completion_tokens if saw_completion else None,
        "total_tokens": total_tokens if saw_total else None,
        "cached_input_tokens": cached_input_tokens if saw_cached else None,
        "reasoning_output_tokens": reasoning_output_tokens if saw_reasoning else None,
    }


def _normalized_text_list(values: Any) -> list[str]:
    if not isinstance(values, list):
        return []
    return [str(value).strip() for value in values if str(value).strip()]


def extract_codex_token_usage_bundle(
    session_jsonl_paths: list[Path],
    *,
    session_roles: list[str] | None = None,
) -> dict[str, Any]:
    """Aggregate provider-measured usage from one or more Codex session JSONL files."""

    paths = [Path(path) for path in session_jsonl_paths]
    if not paths:
        raise ValueError("At least one Codex session JSONL is required.")
    normalized_roles = [str(role or "unclassified").strip().lower() or "unclassified" for role in (session_roles or [])]
    if normalized_roles and len(normalized_roles) != len(paths):
        raise ValueError("session_roles must match session_jsonl_paths length when provided.")

    total_event_count = 0
    imported_sessions: list[dict[str, Any]] = []

    for index, session_path in enumerate(paths):
        payload = extract_codex_token_usage(session_path)
        usage = payload["manifest_usage"]
        role = normalized_roles[index] if index < len(normalized_roles) else "unclassified"
        imported_sessions.append(
            {
                "session_jsonl_path": payload["session_jsonl_path"],
                "token_event_count": payload["token_event_count"],
                "role": role,
                "usage": usage,
            }
        )
        total_event_count += int(payload.get("token_event_count") or 0)

    role_breakdown: dict[str, dict[str, Any]] = {}
    for role in sorted({str(row.get("role") or "unclassified") for row in imported_sessions}):
        role_rows = [row for row in imported_sessions if str(row.get("role") or "unclassified") == role]
        role_breakdown[role] = {
            "session_count": len(role_rows),
            "token_event_count": sum(int(row.get("token_event_count") or 0) for row in role_rows),
            "session_jsonl_paths": [str(row.get("session_jsonl_path") or "") for row in role_rows],
            "usage": _summed_manifest_usage(role_rows),
        }

    return {
        "session_jsonl_paths": [str(path) for path in paths],
        "session_count": len(paths),
        "token_event_count": total_event_count,
        "usage_source": "codex_jsonl_total_token_usage_sum",
        "sessions": imported_sessions,
        "role_breakdown": role_breakdown,
        "manifest_usage": _summed_manifest_usage(imported_sessions),
    }


def _manifest_role_breakdown(
    role_breakdown: dict[str, Any],
) -> dict[str, dict[str, int | None]]:
    normalized: dict[str, dict[str, int | None]] = {}
    for role, details in (role_breakdown or {}).items():
        detail_map = details if isinstance(details, dict) else {}
        usage = detail_map.get("usage") if isinstance(detail_map.get("usage"), dict) else {}
        normalized[str(role)] = {
            "prompt_tokens": usage.get("prompt_tokens"),
            "completion_tokens": usage.get("completion_tokens"),
            "total_tokens": usage.get("total_tokens"),
            "cached_input_tokens": usage.get("cached_input_tokens"),
            "reasoning_output_tokens": usage.get("reasoning_output_tokens"),
            "session_count": int(detail_map.get("session_count") or 0),
            "token_event_count": int(detail_map.get("token_event_count") or 0),
        }
    return normalized


def _role_usage_provenance(
    *,
    usage_source: str,
    role_breakdown: dict[str, Any],
    roles: list[str],
) -> dict[str, Any]:
    selected_paths: list[str] = []
    selected_session_count = 0
    selected_token_event_count = 0
    selected_role_breakdown: dict[str, Any] = {}
    for role in roles:
        details = role_breakdown.get(role) if isinstance(role_breakdown.get(role), dict) else {}
        if not details:
            continue
        selected_role_breakdown[role] = details
        selected_paths.extend([str(path) for path in (details.get("session_jsonl_paths") or []) if str(path)])
        selected_session_count += int(details.get("session_count") or 0)
        selected_token_event_count += int(details.get("token_event_count") or 0)
    return {
        "usage_source": usage_source,
        "session_jsonl_paths": selected_paths,
        "session_count": selected_session_count,
        "token_event_count": selected_token_event_count,
        "role_breakdown": selected_role_breakdown,
    }


def _apply_static_web_hardening_dag_cost_accounting(
    run: dict[str, Any],
    *,
    usage_payload: dict[str, Any],
    manifest_role_breakdown: dict[str, dict[str, int | None]],
) -> None:
    role_breakdown = usage_payload.get("role_breakdown") if isinstance(usage_payload.get("role_breakdown"), dict) else {}
    if not role_breakdown:
        return
    recognized_roles = [
        role
        for role in [*STATIC_WEB_HARDENING_DAG_PREP_ROLES, STATIC_WEB_HARDENING_DAG_EXECUTION_ROLE]
        if role in manifest_role_breakdown
    ]
    if not recognized_roles:
        return

    policy = str(run.get("dag_cost_accounting_policy") or "reported_separately").strip().lower()
    run["dag_cost_accounting_policy"] = policy
    if policy != "not_recorded":
        run["dag_cost_breakdown"] = {
            role: manifest_role_breakdown[role]
            for role in recognized_roles
            if role in manifest_role_breakdown
        }
        run["dag_cost_provenance"] = _role_usage_provenance(
            usage_source=usage_payload["usage_source"],
            role_breakdown=role_breakdown,
            roles=recognized_roles,
        )

    if policy == "reported_separately" and STATIC_WEB_HARDENING_DAG_EXECUTION_ROLE in manifest_role_breakdown:
        worker_usage = {
            key: value
            for key, value in manifest_role_breakdown[STATIC_WEB_HARDENING_DAG_EXECUTION_ROLE].items()
            if key
            not in {
                "session_count",
                "token_event_count",
            }
        }
        run["usage"] = worker_usage
        run["usage_provenance"] = _role_usage_provenance(
            usage_source=usage_payload["usage_source"],
            role_breakdown=role_breakdown,
            roles=[STATIC_WEB_HARDENING_DAG_EXECUTION_ROLE],
        )
        run["provider_usage_source"] = str(run.get("provider_usage_source") or "codex session jsonl")
    else:
        run["usage"] = usage_payload["manifest_usage"]
        run["usage_provenance"] = {
            "usage_source": usage_payload["usage_source"],
            "session_jsonl_paths": usage_payload["session_jsonl_paths"],
            "session_count": usage_payload["session_count"],
            "token_event_count": usage_payload["token_event_count"],
            "role_breakdown": role_breakdown,
        }


def import_codex_usage_into_manifest(
    manifest_path: Path,
    *,
    run_sessions: dict[str, list[Path]],
    run_role_sessions: dict[str, list[tuple[str, Path]]] | None = None,
    output_manifest_path: Path | None = None,
    quality: str | None = None,
) -> dict[str, Any]:
    """Fill measured benchmark runs with Codex JSONL token usage by run_id."""

    manifest_path = Path(manifest_path)
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise ValueError(f"Benchmark manifest not found: {manifest_path}") from exc
    except json.JSONDecodeError as exc:
        raise ValueError(f"Benchmark manifest is not valid JSON: {manifest_path}: {exc}") from exc
    if not isinstance(manifest, dict):
        raise ValueError("Benchmark manifest must be a JSON object.")
    benchmark_kind = str(manifest.get("benchmark_kind") or "").strip()
    normalized_role_sessions = {run_id: list(entries) for run_id, entries in (run_role_sessions or {}).items()}
    if not run_sessions and not normalized_role_sessions:
        raise ValueError("At least one run_id=session_jsonl mapping is required.")

    pending = {run_id: list(paths) for run_id, paths in run_sessions.items()}
    pending_role_entries = {run_id: list(entries) for run_id, entries in normalized_role_sessions.items()}
    imported: list[dict[str, Any]] = []
    for workload in manifest.get("workloads") or []:
        if not isinstance(workload, dict):
            continue
        workload_id = str(workload.get("workload_id") or workload.get("id") or "")
        for run in workload.get("runs") or []:
            if not isinstance(run, dict):
                continue
            run_id = str(run.get("run_id") or run.get("id") or "")
            if run_id not in pending and run_id not in pending_role_entries:
                continue
            session_entries = [{"role": "unclassified", "path": path} for path in pending.pop(run_id, [])]
            for role, path in pending_role_entries.pop(run_id, []):
                session_entries.append({"role": str(role or "unclassified"), "path": path})
            usage_payload = extract_codex_token_usage_bundle(
                [Path(entry["path"]) for entry in session_entries],
                session_roles=[str(entry["role"] or "unclassified") for entry in session_entries],
            )
            run["usage_kind"] = "measured"
            manifest_role_breakdown = _manifest_role_breakdown(
                usage_payload.get("role_breakdown") if isinstance(usage_payload.get("role_breakdown"), dict) else {}
            )
            run["usage"] = usage_payload["manifest_usage"]
            if manifest_role_breakdown:
                run["usage_breakdown"] = manifest_role_breakdown
            run["usage_provenance"] = {
                "usage_source": usage_payload["usage_source"],
                "session_jsonl_paths": usage_payload["session_jsonl_paths"],
                "session_count": usage_payload["session_count"],
                "token_event_count": usage_payload["token_event_count"],
                "role_breakdown": usage_payload.get("role_breakdown") or {},
            }
            if not run.get("provider_usage_source"):
                run["provider_usage_source"] = "codex session jsonl"
            if benchmark_kind == STATIC_WEB_HARDENING_BENCHMARK_KIND and str(run.get("mode") or "").strip() == "ait_dag_current":
                _apply_static_web_hardening_dag_cost_accounting(
                    run,
                    usage_payload=usage_payload,
                    manifest_role_breakdown=manifest_role_breakdown,
                )
            if quality:
                run["quality"] = quality
            imported.append(
                {
                    "workload_id": workload_id,
                    "run_id": run_id,
                    "mode": run.get("mode"),
                    "session_jsonl_paths": usage_payload["session_jsonl_paths"],
                    "session_count": usage_payload["session_count"],
                    "token_event_count": usage_payload["token_event_count"],
                    "usage": run.get("usage") if isinstance(run.get("usage"), dict) else usage_payload["manifest_usage"],
                    "role_breakdown": run.get("usage_provenance", {}).get("role_breakdown")
                    if isinstance(run.get("usage_provenance"), dict)
                    else (usage_payload.get("role_breakdown") or {}),
                    "quality": run.get("quality"),
                }
            )

    unresolved = sorted({*pending.keys(), *pending_role_entries.keys()})
    if unresolved:
        missing = ", ".join(unresolved)
        raise ValueError(f"Run id(s) not found in manifest: {missing}")

    output_path = Path(output_manifest_path) if output_manifest_path is not None else None
    if output_path is not None:
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(json.dumps(manifest, indent=2, sort_keys=False) + "\n", encoding="utf-8")

    return {
        "manifest_path": str(manifest_path),
        "output_manifest_path": str(output_path) if output_path is not None else None,
        "imported_count": len(imported),
        "imported_runs": imported,
        "manifest": manifest,
    }


def inspect_token_savings_collection(manifest_path: Path) -> dict[str, Any]:
    manifest_path = Path(manifest_path)
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise ValueError(f"Benchmark manifest not found: {manifest_path}") from exc
    except json.JSONDecodeError as exc:
        raise ValueError(f"Benchmark manifest is not valid JSON: {manifest_path}: {exc}") from exc
    if not isinstance(manifest, dict):
        raise ValueError("Benchmark manifest must be a JSON object.")

    require_equivalent_completion = bool(manifest.get("require_equivalent_completion"))
    manifest_dir = manifest_path.parent
    rows: list[dict[str, Any]] = []
    for workload in manifest.get("workloads") or []:
        if not isinstance(workload, dict):
            continue
        workload_id = str(workload.get("workload_id") or workload.get("id") or "")
        category = str(workload.get("category") or "unknown")
        for run in workload.get("runs") or []:
            if not isinstance(run, dict):
                continue
            usage = _usage_status(run, manifest_dir=manifest_dir)
            usage_kind = str(run.get("usage_kind") or "").lower() or "unknown"
            quality = str(run.get("quality") or run.get("quality_gate") or run.get("quality_gate_status") or "pending").lower()
            quality_passed = quality in PASSED_QUALITY_STATES
            completion_status = _normalized_completion_status(run)
            completion_equivalent = _completion_is_equivalent(run, completion_status=completion_status)
            has_usage = usage["total_tokens"] is not None
            measured_ready = (
                usage_kind == "measured"
                and has_usage
                and quality_passed
                and (not require_equivalent_completion or completion_equivalent is True)
            )
            rows.append(
                {
                    "workload_id": workload_id,
                    "category": category,
                    "run_id": str(run.get("run_id") or run.get("id") or ""),
                    "mode": str(run.get("mode") or ""),
                    "usage_kind": usage_kind,
                    "quality": quality,
                    "quality_passed": quality_passed,
                    "has_usage": has_usage,
                    "completion_status": completion_status,
                    "completion_equivalent": completion_equivalent,
                    "prompt_tokens": usage["prompt_tokens"],
                    "completion_tokens": usage["completion_tokens"],
                    "total_tokens": usage["total_tokens"],
                    "measured_ready": measured_ready,
                    "missing_reason": _collection_missing_reason(
                        usage_kind,
                        has_usage,
                        quality,
                        quality_passed,
                        require_equivalent_completion=require_equivalent_completion,
                        completion_equivalent=completion_equivalent,
                    ),
                }
            )

    missing_rows = [row for row in rows if not row["measured_ready"]]
    missing_usage = [row for row in rows if not row["has_usage"]]
    pending_quality = [row for row in rows if row["quality"] == "pending"]
    ready_to_report = bool(rows) and not missing_rows
    return {
        "benchmark_id": manifest.get("benchmark_id") or manifest_path.stem,
        "manifest_path": str(manifest_path),
        "baseline_mode": manifest.get("baseline_mode") or "git_linear",
        "candidate_modes": manifest.get("candidate_modes") or ["ait_linear", "ait_dag"],
        "require_equivalent_completion": require_equivalent_completion,
        "summary": {
            "total_run_count": len(rows),
            "measured_ready_count": len(rows) - len(missing_rows),
            "missing_run_count": len(missing_rows),
            "missing_usage_count": len(missing_usage),
            "pending_quality_count": len(pending_quality),
            "ready_to_report": ready_to_report,
            "next_action": (
                "run token-savings report"
                if ready_to_report
                else "fill provider usage, set equivalent completion status, and mark passing runs"
            ),
        },
        "runs": rows,
    }


def _normalized_comparison_profiles(
    manifest: dict[str, Any],
    *,
    default_baseline_mode: str,
    default_candidate_modes: list[str],
    default_aggregate_candidate_mode: str,
    default_minimum_count: int,
) -> list[dict[str, Any]]:
    raw_profiles = manifest.get("comparison_profiles") or []
    if not isinstance(raw_profiles, list) or not raw_profiles:
        return [
            {
                "profile_id": "default",
                "title": "Default comparison profile",
                "description": None,
                "profile_kind": "standard_claim",
                "experimental": False,
                "baseline_mode": default_baseline_mode,
                "candidate_modes": default_candidate_modes,
                "aggregate_candidate_mode": default_aggregate_candidate_mode,
                "minimum_comparable_long_workloads": default_minimum_count,
                "claim_target": True,
                "workload_ids": [],
                "workload_tags": [],
            }
        ]

    profiles: list[dict[str, Any]] = []
    for index, raw in enumerate(raw_profiles, start=1):
        if not isinstance(raw, dict):
            raise ValueError("comparison_profiles must contain objects.")
        profile_id = str(raw.get("profile_id") or raw.get("id") or f"profile-{index}").strip()
        if not profile_id:
            raise ValueError("comparison profile id must not be empty.")
        baseline_mode = str(raw.get("baseline_mode") or default_baseline_mode).strip()
        candidate_modes = [str(item).strip() for item in (raw.get("candidate_modes") or default_candidate_modes) if str(item).strip()]
        if not candidate_modes:
            raise ValueError(f"comparison profile {profile_id!r} must include candidate_modes.")
        aggregate_candidate_mode = str(raw.get("aggregate_candidate_mode") or candidate_modes[0]).strip()
        if aggregate_candidate_mode not in candidate_modes:
            raise ValueError(
                f"comparison profile {profile_id!r} aggregate_candidate_mode must be one of candidate_modes: {aggregate_candidate_mode!r}"
            )
        profile_kind = str(raw.get("profile_kind") or ("standard_claim" if index == 1 else "comparison")).strip() or "comparison"
        experimental = bool(
            raw.get("experimental", "server_warm" in profile_kind or "resumed" in profile_kind or "repeated" in profile_kind)
        )
        profiles.append(
            {
                "profile_id": profile_id,
                "title": str(raw.get("title") or profile_id),
                "description": raw.get("description"),
                "profile_kind": profile_kind,
                "experimental": experimental,
                "baseline_mode": baseline_mode,
                "candidate_modes": candidate_modes,
                "aggregate_candidate_mode": aggregate_candidate_mode,
                "minimum_comparable_long_workloads": int(raw.get("minimum_comparable_long_workloads") or default_minimum_count),
                "claim_target": bool(raw.get("claim_target", index == 1 and not experimental)),
                "workload_ids": _normalized_text_list(raw.get("workload_ids")),
                "workload_tags": _normalized_text_list(raw.get("workload_tags")),
            }
        )
    return profiles


def _evaluate_comparison_profile(
    profile: dict[str, Any],
    *,
    normalized_workloads: list[dict[str, Any]],
    evidence_type: str,
) -> dict[str, Any]:
    baseline_mode = str(profile.get("baseline_mode") or "")
    candidate_modes = [str(item) for item in profile.get("candidate_modes") or []]
    aggregate_candidate_mode = str(profile.get("aggregate_candidate_mode") or "")
    minimum_count = int(profile.get("minimum_comparable_long_workloads") or 3)
    workload_ids = {str(item) for item in profile.get("workload_ids") or [] if str(item)}
    workload_tags = {str(item) for item in profile.get("workload_tags") or [] if str(item)}
    long_savings_by_mode: dict[str, list[float]] = {mode: [] for mode in candidate_modes}
    workloads_payload: list[dict[str, Any]] = []

    for workload in normalized_workloads:
        workload_id = str(workload.get("workload_id") or "")
        workload_tag_set = {str(item) for item in workload.get("tags") or [] if str(item)}
        if workload_ids and workload_id not in workload_ids:
            continue
        if workload_tags and not workload_tags.issubset(workload_tag_set):
            continue
        by_mode = workload["passed_runs_by_mode"]
        baseline_total = _median_total_tokens(by_mode.get(baseline_mode, []))
        comparisons: list[dict[str, Any]] = []
        for mode in candidate_modes:
            candidate_total = _median_total_tokens(by_mode.get(mode, []))
            saving_ratio = _saving_ratio(baseline_total, candidate_total)
            comparison = {
                "mode": mode,
                "baseline_mode": baseline_mode,
                "baseline_median_total_tokens": baseline_total,
                "candidate_median_total_tokens": candidate_total,
                "saving_ratio": saving_ratio,
                "saving_percent": None if saving_ratio is None else round(saving_ratio * 100, 2),
                "comparable": saving_ratio is not None,
                "candidate_run_count": len(by_mode.get(mode, [])),
            }
            comparisons.append(comparison)
            if workload["category"] == "long" and saving_ratio is not None:
                long_savings_by_mode.setdefault(mode, []).append(saving_ratio)
        workloads_payload.append(
            {
                "workload_id": workload_id,
                "title": workload["title"],
                "category": workload["category"],
                "tags": workload.get("tags") or [],
                "run_count": workload["run_count"],
                "passed_run_count": workload["passed_run_count"],
                "baseline_mode": baseline_mode,
                "baseline_passed_run_count": len(by_mode.get(baseline_mode, [])),
                "runs": workload["runs"],
                "comparisons": comparisons,
            }
        )

    primary_savings = long_savings_by_mode.get(aggregate_candidate_mode, [])
    long_median = _median_float(primary_savings)
    verdict = _verdict(long_median, comparable_count=len(primary_savings), minimum_count=minimum_count)
    claim_target = bool(profile.get("claim_target"))
    claim_ready = claim_target and verdict == "supported" and evidence_type == "measured"
    per_mode: dict[str, Any] = {}
    for mode in candidate_modes:
        savings = long_savings_by_mode.get(mode, [])
        median = _median_float(savings)
        per_mode[mode] = {
            "comparable_count": len(savings),
            "median_saving_ratio": median,
            "median_saving_percent": None if median is None else round(median * 100, 2),
        }
    aggregate = {
        "aggregate_candidate_mode": aggregate_candidate_mode,
        "long_candidate_comparable_count": len(primary_savings),
        "long_candidate_median_saving_ratio": long_median,
        "long_candidate_median_saving_percent": None if long_median is None else round(long_median * 100, 2),
        "per_mode": per_mode,
        "verdict": verdict,
        "claim_target": claim_target,
        "claim_ready": claim_ready,
        "claim_caveat": _claim_caveat(
            verdict,
            evidence_type,
            len(primary_savings),
            minimum_count,
            claim_target=claim_target,
            profile_kind=str(profile.get("profile_kind") or ""),
            aggregate_candidate_mode=aggregate_candidate_mode,
        ),
    }
    return {
        "profile_id": profile["profile_id"],
        "title": profile.get("title"),
        "description": profile.get("description"),
        "profile_kind": profile.get("profile_kind") or "comparison",
        "experimental": bool(profile.get("experimental")),
        "baseline_mode": baseline_mode,
        "candidate_modes": candidate_modes,
        "aggregate_candidate_mode": aggregate_candidate_mode,
        "minimum_comparable_long_workloads": minimum_count,
        "claim_target": claim_target,
        "workload_ids": sorted(workload_ids),
        "workload_tags": sorted(workload_tags),
        "workloads": workloads_payload,
        "aggregate": aggregate,
    }


def evaluate_token_savings_manifest(manifest: dict[str, Any], *, manifest_path: Path | None = None) -> dict[str, Any]:
    if not isinstance(manifest, dict):
        raise ValueError("Benchmark manifest must be a JSON object.")
    manifest_dir = manifest_path.parent if manifest_path else Path.cwd()
    baseline_mode = str(manifest.get("baseline_mode") or "git_linear")
    candidate_modes = [str(item) for item in manifest.get("candidate_modes") or ["ait_linear", "ait_dag"]]
    aggregate_candidate_mode = str(manifest.get("aggregate_candidate_mode") or "ait_dag")
    if aggregate_candidate_mode not in candidate_modes:
        raise ValueError(
            f"aggregate_candidate_mode must be one of candidate_modes: {aggregate_candidate_mode!r}"
        )
    min_long = int(manifest.get("minimum_comparable_long_workloads") or 3)
    require_equivalent_completion = bool(manifest.get("require_equivalent_completion"))
    all_usage_kinds: list[str] = []
    normalized_workloads: list[dict[str, Any]] = []

    workloads = manifest.get("workloads") or []
    if not isinstance(workloads, list):
        raise ValueError("manifest.workloads must be a list.")

    for workload in workloads:
        if not isinstance(workload, dict):
            raise ValueError("Each workload must be an object.")
        workload_id = str(workload.get("workload_id") or workload.get("id") or "")
        if not workload_id:
            raise ValueError("Each workload needs workload_id.")
        category = str(workload.get("category") or "unknown")
        runs = workload.get("runs") or []
        if not isinstance(runs, list):
            raise ValueError(f"workload {workload_id}: runs must be a list.")

        normalized_runs = [
            _normalize_run(
                run,
                manifest_dir=manifest_dir,
                workload_id=workload_id,
                require_equivalent_completion=require_equivalent_completion,
            )
            for run in runs
        ]
        all_usage_kinds.extend(run["usage_kind"] for run in normalized_runs)
        passed_runs = [run for run in normalized_runs if run["quality_passed"]]
        by_mode: dict[str, list[dict[str, Any]]] = {}
        for run in passed_runs:
            by_mode.setdefault(run["mode"], []).append(run)

        baseline_total = _median_total_tokens(by_mode.get(baseline_mode, []))
        normalized_workloads.append(
            {
                "workload_id": workload_id,
                "title": workload.get("title") or workload_id,
                "category": category,
                "tags": _normalized_text_list(workload.get("tags")),
                "run_count": len(normalized_runs),
                "passed_run_count": len(passed_runs),
                "runs": normalized_runs,
                "passed_runs_by_mode": by_mode,
            }
        )

    evidence_type = _combine_usage_kinds(all_usage_kinds)
    profiles = [
        _evaluate_comparison_profile(profile, normalized_workloads=normalized_workloads, evidence_type=evidence_type)
        for profile in _normalized_comparison_profiles(
            manifest,
            default_baseline_mode=baseline_mode,
            default_candidate_modes=candidate_modes,
            default_aggregate_candidate_mode=aggregate_candidate_mode,
            default_minimum_count=min_long,
        )
    ]
    primary_profile = profiles[0]

    return {
        "benchmark_id": manifest.get("benchmark_id") or (manifest_path.stem if manifest_path else "token-savings-benchmark"),
        "description": manifest.get("description"),
        "known_confounders": _normalized_text_list(manifest.get("known_confounders")),
        "manifest_path": str(manifest_path) if manifest_path else None,
        "baseline_mode": primary_profile["baseline_mode"],
        "candidate_modes": primary_profile["candidate_modes"],
        "aggregate_candidate_mode": primary_profile["aggregate_candidate_mode"],
        "minimum_comparable_long_workloads": primary_profile["minimum_comparable_long_workloads"],
        "require_equivalent_completion": require_equivalent_completion,
        "evidence_type": evidence_type,
        "workloads": primary_profile["workloads"],
        "aggregate": primary_profile["aggregate"],
        "profiles": profiles,
    }


def render_token_savings_markdown(payload: dict[str, Any]) -> str:
    aggregate = payload.get("aggregate") or {}
    profiles = payload.get("profiles") if isinstance(payload.get("profiles"), list) and payload.get("profiles") else [payload]
    normalized_profile_kinds = {
        str(profile.get("profile_kind") or "").strip().lower()
        for profile in profiles
        if isinstance(profile, dict)
    }
    all_profiles_non_claim_target = all(
        not bool((profile.get("aggregate") or {}).get("claim_target"))
        for profile in profiles
        if isinstance(profile, dict)
    )
    lines = [
        f"# {payload.get('benchmark_id') or 'Token-Savings Benchmark'}",
        "",
        "Generated token-savings benchmark report.",
        "",
        f"- Evidence type: `{payload.get('evidence_type')}`",
    ]
    if payload.get("require_equivalent_completion"):
        lines.append("- Equivalent completion required: `True`")
    if payload.get("description"):
        lines.extend(["", str(payload.get("description"))])
    known_confounders = _normalized_text_list(payload.get("known_confounders"))
    if known_confounders:
        lines.extend(["", "## Known Confounders", ""])
        lines.extend(f"- {item}" for item in known_confounders)
    for index, profile in enumerate(profiles, start=1):
        profile_aggregate = profile.get("aggregate") if isinstance(profile.get("aggregate"), dict) else aggregate
        heading = profile.get("title") or profile.get("profile_id") or f"profile-{index}"
        lines.extend(
            [
                "",
                f"## {heading}",
                "",
                f"- Baseline mode: `{profile.get('baseline_mode')}`",
                f"- Aggregate candidate mode: `{profile_aggregate.get('aggregate_candidate_mode') or profile.get('aggregate_candidate_mode') or 'ait_dag'}`",
                f"- Profile kind: `{profile.get('profile_kind') or 'comparison'}`",
                f"- Experimental: `{bool(profile.get('experimental'))}`",
                f"- Verdict: `{profile_aggregate.get('verdict')}`",
                f"- Long candidate median savings: `{_format_percent(profile_aggregate.get('long_candidate_median_saving_ratio'))}`",
                f"- Comparable long workloads: `{profile_aggregate.get('long_candidate_comparable_count')}` / `{profile.get('minimum_comparable_long_workloads') or payload.get('minimum_comparable_long_workloads')}` required",
                f"- Claim target: `{bool(profile_aggregate.get('claim_target'))}`",
                f"- Claim ready: `{bool(profile_aggregate.get('claim_ready'))}`",
                f"- Caveat: {profile_aggregate.get('claim_caveat')}",
            ]
        )
        if profile.get("workload_ids") or profile.get("workload_tags"):
            lines.extend(
                [
                    f"- Workload ids: `{', '.join(profile.get('workload_ids') or []) or 'all'}`",
                    f"- Workload tags: `{', '.join(profile.get('workload_tags') or []) or 'all'}`",
                    "",
                ]
            )
        lines.extend(
            [
                "",
                "| Workload | Category | Candidate | Baseline tokens | Candidate tokens | Savings | Comparable |",
                "| --- | --- | --- | ---: | ---: | ---: | --- |",
            ]
        )
        for workload in profile.get("workloads") or []:
            for comparison in workload.get("comparisons") or []:
                lines.append(
                    "| {workload} | {category} | {mode} | {baseline} | {candidate} | {saving} | {comparable} |".format(
                        workload=workload.get("workload_id"),
                        category=workload.get("category"),
                        mode=comparison.get("mode"),
                        baseline=_format_int(comparison.get("baseline_median_total_tokens")),
                        candidate=_format_int(comparison.get("candidate_median_total_tokens")),
                        saving=_format_percent(comparison.get("saving_ratio")),
                        comparable="yes" if comparison.get("comparable") else "no",
                    )
                )

    lines.extend(["", "## Cost accounting", ""])
    for workload in payload.get("workloads") or []:
        for run in workload.get("runs") or []:
            usage_breakdown = run.get("usage_breakdown") if isinstance(run.get("usage_breakdown"), dict) else {}
            if not usage_breakdown:
                continue
            lines.extend(
                [
                    f"### {workload.get('workload_id')} · {run.get('run_id')}",
                    "",
                    "| Role | Sessions | Prompt | Completion | Total |",
                    "| --- | ---: | ---: | ---: | ---: |",
                ]
            )
            for role, details in usage_breakdown.items():
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

    if payload.get("evidence_type") == "measured" and aggregate.get("claim_ready"):
        interpretation = (
            "This provider-measured report is claim-ready only for the packet-only claim profile shown above. "
            "Keep product wording scoped to the measured prompt packets, comparable quality gates, and repository revision."
        )
    elif "same_surface_review_overhead" in normalized_profile_kinds and all_profiles_non_claim_target:
        interpretation = (
            "This report measures same-surface packet overhead only. Use it to quantify "
            "the token cost of the richer review packet versus the compact ait_dag baseline, "
            "and do not restate it as the repository's long-DAG product claim."
        )
    else:
        interpretation = (
            "This report separates packet-only claim evidence from real remote orchestration cost. "
            "Only provider-measured token usage should be used for token-saving benchmark conclusions."
        )
    lines.extend(["## Interpretation", "", interpretation, ""])
    return "\n".join(lines)


def _usage_status(run: dict[str, Any], *, manifest_dir: Path) -> dict[str, int | None]:
    usage_payload: dict[str, Any] = {}
    if isinstance(run.get("usage"), dict):
        usage_payload.update(run["usage"])
    if run.get("usage_file"):
        usage_path = _resolve_path(manifest_dir, str(run["usage_file"]))
        try:
            file_payload = json.loads(usage_path.read_text(encoding="utf-8"))
        except FileNotFoundError as exc:
            raise ValueError(f"usage_file not found: {usage_path}") from exc
        except json.JSONDecodeError as exc:
            raise ValueError(f"usage_file is not valid JSON: {usage_path}: {exc}") from exc
        if isinstance(file_payload, dict):
            usage_payload.update(_usage_object(file_payload))
    return _extract_usage_tokens(usage_payload)


def _extract_codex_usage_payload(payload: dict[str, Any]) -> dict[str, int | None]:
    if not isinstance(payload, dict):
        payload = {}
    input_tokens = _first_int(payload, "input_tokens")
    output_tokens = _first_int(payload, "output_tokens")
    total_tokens = _first_int(payload, "total_tokens")
    if total_tokens is None and (input_tokens is not None or output_tokens is not None):
        total_tokens = (input_tokens or 0) + (output_tokens or 0)
    return {
        "input_tokens": input_tokens,
        "cached_input_tokens": _first_int(payload, "cached_input_tokens"),
        "output_tokens": output_tokens,
        "reasoning_output_tokens": _first_int(payload, "reasoning_output_tokens"),
        "total_tokens": total_tokens,
    }


def _codex_usage_for_manifest(total_usage: dict[str, int | None]) -> dict[str, int | None]:
    prompt_tokens = total_usage.get("input_tokens")
    completion_tokens = total_usage.get("output_tokens")
    if prompt_tokens is None and completion_tokens is None:
        raise ValueError("Codex total_token_usage is missing input_tokens/output_tokens.")
    return {
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": total_usage.get("total_tokens"),
        "cached_input_tokens": total_usage.get("cached_input_tokens"),
        "reasoning_output_tokens": total_usage.get("reasoning_output_tokens"),
    }


def _collection_missing_reason(
    usage_kind: str,
    has_usage: bool,
    quality: str,
    quality_passed: bool,
    *,
    require_equivalent_completion: bool = False,
    completion_equivalent: bool | None = None,
) -> str | None:
    reasons: list[str] = []
    if usage_kind != "measured":
        reasons.append("usage_kind_not_measured")
    if not has_usage:
        reasons.append("missing_usage")
    if quality == "pending":
        reasons.append("quality_pending")
    elif not quality_passed:
        reasons.append("quality_not_passed")
    if require_equivalent_completion:
        if completion_equivalent is None:
            reasons.append("completion_status_missing")
        elif not completion_equivalent:
            reasons.append("completion_not_equivalent")
    return ", ".join(reasons) if reasons else None


def _normalize_run(
    run: dict[str, Any],
    *,
    manifest_dir: Path,
    workload_id: str,
    require_equivalent_completion: bool = False,
) -> dict[str, Any]:
    if not isinstance(run, dict):
        raise ValueError(f"workload {workload_id}: each run must be an object.")
    run_id = str(run.get("run_id") or run.get("id") or "")
    if not run_id:
        raise ValueError(f"workload {workload_id}: run needs run_id.")
    mode = str(run.get("mode") or "")
    if not mode:
        raise ValueError(f"workload {workload_id} run {run_id}: mode is required.")

    usage, usage_kind = _resolve_usage(run, manifest_dir=manifest_dir)
    quality = str(run.get("quality") or run.get("quality_gate") or run.get("quality_gate_status") or "passed").lower()
    completion_status = _normalized_completion_status(run)
    completion_equivalent = _completion_is_equivalent(run, completion_status=completion_status)
    prompt_tokens = usage.get("prompt_tokens")
    completion_tokens = usage.get("completion_tokens")
    total_tokens = usage.get("total_tokens")
    quality_passed = quality in PASSED_QUALITY_STATES and (
        not require_equivalent_completion or completion_equivalent is True
    )
    return {
        "run_id": run_id,
        "mode": mode,
        "quality": quality,
        "quality_passed": quality_passed,
        "completion_status": completion_status,
        "completion_equivalent": completion_equivalent,
        "usage_kind": usage_kind,
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": total_tokens,
        "usage_breakdown": run.get("usage_breakdown") if isinstance(run.get("usage_breakdown"), dict) else None,
        "usage_provenance": run.get("usage_provenance") if isinstance(run.get("usage_provenance"), dict) else None,
        "notes": run.get("notes"),
    }


def _resolve_usage(run: dict[str, Any], *, manifest_dir: Path) -> tuple[dict[str, int | None], str]:
    usage_kind = str(run.get("usage_kind") or "").lower() or None
    if usage_kind and usage_kind != "measured":
        raise ValueError(
            f"run {run.get('run_id') or run.get('id')}: token-savings benchmarks require provider-measured usage; got {usage_kind!r}."
        )
    usage_payload: dict[str, Any] = {}

    if isinstance(run.get("usage"), dict):
        usage_payload.update(run["usage"])
    if run.get("usage_file"):
        usage_path = _resolve_path(manifest_dir, str(run["usage_file"]))
        try:
            file_payload = json.loads(usage_path.read_text(encoding="utf-8"))
        except FileNotFoundError as exc:
            raise ValueError(f"usage_file not found: {usage_path}") from exc
        except json.JSONDecodeError as exc:
            raise ValueError(f"usage_file is not valid JSON: {usage_path}: {exc}") from exc
        if isinstance(file_payload, dict):
            usage_payload.update(_usage_object(file_payload))
        usage_kind = usage_kind or "measured"

    usage = _extract_usage_tokens(usage_payload)
    if usage["total_tokens"] is None:
        raise ValueError(f"run {run.get('run_id') or run.get('id')}: provider-measured token usage is required.")
    return usage, "measured"


def _normalized_completion_status(run: dict[str, Any]) -> str | None:
    raw = run.get("completion_status")
    if raw is None:
        raw = run.get("acceptance_status")
    if raw is None:
        return None
    value = str(raw).strip().lower()
    if not value:
        return None
    return value.replace("-", "_").replace(" ", "_")


def _completion_is_equivalent(run: dict[str, Any], *, completion_status: str | None) -> bool | None:
    explicit = run.get("equivalent_completion")
    if explicit is not None:
        if isinstance(explicit, bool):
            return explicit
        value = str(explicit).strip().lower()
        if value in {"true", "1", "yes", "y", "equivalent", "complete", "completed", "accepted", "done"}:
            return True
        if value in {"false", "0", "no", "n", "partial", "blocked", "unverified"}:
            return False
        return bool(explicit)
    if completion_status is None:
        return None
    return completion_status in EQUIVALENT_COMPLETION_STATES


def _usage_object(payload: dict[str, Any]) -> dict[str, Any]:
    nested = payload.get("usage")
    if isinstance(nested, dict):
        return nested
    return payload


def _extract_usage_tokens(payload: dict[str, Any]) -> dict[str, int | None]:
    data = _usage_object(payload)
    prompt = _first_int(data, "prompt_tokens", "input_tokens", "prompt", "input")
    completion = _first_int(data, "completion_tokens", "output_tokens", "completion", "output")
    total = _first_int(data, "total_tokens", "total")
    if total is None and (prompt is not None or completion is not None):
        total = (prompt or 0) + (completion or 0)
    return {"prompt_tokens": prompt, "completion_tokens": completion, "total_tokens": total}


def _resolve_path(base: Path, value: str) -> Path:
    path = Path(value)
    if path.is_absolute():
        return path
    candidate = base / path
    if candidate.exists():
        return candidate
    return Path.cwd() / path


def _first_int(data: dict[str, Any], *keys: str) -> int | None:
    for key in keys:
        value = data.get(key)
        if value is None:
            continue
        try:
            return int(value)
        except (TypeError, ValueError):
            continue
    return None


def _median_total_tokens(runs: list[dict[str, Any]]) -> int | None:
    totals = [int(run["total_tokens"]) for run in runs if run.get("total_tokens") is not None]
    if not totals:
        return None
    return int(statistics.median(totals))


def _median_float(values: list[float]) -> float | None:
    if not values:
        return None
    return float(statistics.median(values))


def _saving_ratio(baseline_total: int | None, candidate_total: int | None) -> float | None:
    if baseline_total is None or candidate_total is None or baseline_total <= 0:
        return None
    return (baseline_total - candidate_total) / baseline_total


def _combine_usage_kinds(kinds: list[str]) -> str:
    normalized = {kind if kind == "measured" else "unknown" for kind in kinds if kind}
    if not normalized:
        return "unknown"
    if len(normalized) == 1:
        return next(iter(normalized))
    return "unknown"


def _verdict(long_median: float | None, *, comparable_count: int, minimum_count: int) -> str:
    if long_median is None or comparable_count < minimum_count:
        return "unproven"
    if long_median < 0:
        return "overstated_or_regression"
    if long_median < 0.20:
        return "not_supported"
    if long_median < 0.40:
        return "directionally_positive"
    return "supported"


def _claim_caveat(
    verdict: str,
    evidence_type: str,
    comparable_count: int,
    minimum_count: int,
    *,
    claim_target: bool = True,
    profile_kind: str | None = None,
    aggregate_candidate_mode: str | None = None,
) -> str:
    if not claim_target:
        normalized_kind = str(profile_kind or "").strip().lower()
        normalized_mode = str(aggregate_candidate_mode or "").strip().lower()
        if normalized_kind == "packet_recovery" or normalized_mode == "packet_candidate":
            return (
                "Directional packet-only evidence only; rerun fresh "
                "measured sessions before treating 80%+ as workload-class support."
            )
        if normalized_kind == "same_surface_review_overhead":
            return (
                "Same-surface packet-overhead evidence only; use it to measure the token cost "
                "of the richer review packet, not to restate the long-DAG product claim."
            )
        if normalized_kind == "reference":
            return "Reference-only packet comparison; do not treat this profile as standalone claim evidence."
        return "This profile measures real orchestration cost separately from the packet-only product-claim baseline."
    if comparable_count < minimum_count:
        return "Not enough comparable long-task runs for a product claim."
    if evidence_type != "measured":
        return "Provider-measured token usage is required before treating this benchmark as evidence."
    if verdict == "supported":
        return "Comparable measured long-task runs support the 40-60% target under this benchmark protocol."
    return "Current evidence does not support a verified 40-60% token-saving claim."


def _format_percent(value: Any) -> str:
    if value is None:
        return "n/a"
    try:
        return f"{float(value) * 100:.2f}%"
    except (TypeError, ValueError):
        return "n/a"


def _format_int(value: Any) -> str:
    if value is None:
        return "n/a"
    try:
        return str(int(value))
    except (TypeError, ValueError):
        return "n/a"
