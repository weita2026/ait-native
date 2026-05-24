from __future__ import annotations

import shlex
from typing import Any

from ..server_paths import ServerContext
from ..server_queue import list_jobs
from ..server_store import get_attestation, get_change, get_patchset


_TG1_REQUIRED_SUITE_ID = "tg1_required"


def _cli_command(parts: list[str]) -> str:
    return " ".join(shlex.quote(str(part)) for part in parts if str(part).strip())


def _unique_strs(values: list[Any] | tuple[Any, ...] | set[Any] | None) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for value in list(values or []):
        text = str(value or "").strip()
        if not text or text in seen:
            continue
        seen.add(text)
        ordered.append(text)
    return ordered


def _repo_job_status(job: dict[str, Any]) -> str:
    result = dict(job.get("result") or {})
    status = str(result.get("status") or "").strip()
    if status:
        return status
    state = str(job.get("state") or "").strip().lower()
    if state == "succeeded":
        return "pass"
    if state in {"failed", "blocked"}:
        return "fail"
    if state in {"queued", "running"}:
        return "pending"
    return state or "unknown"


def _patchset_job_status(job: dict[str, Any]) -> str:
    result = dict(job.get("result") or {})
    status = str(result.get("tests_status") or "").strip()
    if status:
        return status
    state = str(job.get("state") or "").strip().lower()
    if state == "succeeded":
        return "pass"
    if state in {"failed", "blocked"}:
        return "fail"
    if state in {"queued", "running"}:
        return "pending"
    return state or "unknown"


def _suite_ids_for_job(job: dict[str, Any]) -> list[str]:
    result = dict(job.get("result") or {})
    selected = _unique_strs(result.get("selected_suite_ids") or [])
    if selected:
        return selected
    payload = dict(job.get("payload") or {})
    return _unique_strs(payload.get("suite_ids") or [])


def _planes_for_job(job: dict[str, Any]) -> list[str]:
    result = dict(job.get("result") or {})
    selected = _unique_strs(result.get("selected_planes") or [])
    if selected:
        return selected
    payload = dict(job.get("payload") or {})
    plane = str(payload.get("plane") or "").strip()
    return [plane] if plane else []


def _summary_artifacts_for_suite_results(suite_results: list[dict[str, Any]]) -> list[dict[str, Any]]:
    artifacts: list[dict[str, Any]] = []
    for suite in suite_results:
        suite_id = str(suite.get("suite_id") or "").strip()
        for key, payload in dict(suite.get("artifacts") or {}).items():
            if not isinstance(payload, dict):
                continue
            path = str(payload.get("path") or "").strip()
            if not path:
                continue
            if key not in {"summary_json", "summary_markdown"}:
                continue
            artifacts.append(
                {
                    "suite_id": suite_id,
                    "artifact_key": key,
                    "path": path,
                    "exists": bool(payload.get("exists", False)),
                    "size_bytes": payload.get("size_bytes"),
                }
            )
    return artifacts


def _optional_int(value: Any) -> int | None:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _normalize_tg1_required_summary(
    suite_results: list[dict[str, Any]],
    *,
    selected_suite_ids: list[str],
    blocking_failures: list[str],
    tests_status: str,
) -> dict[str, Any] | None:
    suite_result = next(
        (dict(item) for item in suite_results if str(item.get("suite_id") or "").strip() == _TG1_REQUIRED_SUITE_ID),
        None,
    )
    summary = dict((suite_result or {}).get("tg1_required_summary") or {})
    if suite_result is None and _TG1_REQUIRED_SUITE_ID not in set(selected_suite_ids):
        return None
    status = str(summary.get("status") or (suite_result or {}).get("status") or "").strip().lower()
    if not status:
        if _TG1_REQUIRED_SUITE_ID in set(blocking_failures):
            status = "fail"
        elif str(tests_status or "").strip().lower() in {"pending", "queued", "running"}:
            status = "pending"
        elif _TG1_REQUIRED_SUITE_ID in set(selected_suite_ids):
            status = "pass"
    return {
        "status": status or None,
        "validation_status": str(summary.get("validation_status") or "").strip() or None,
        "pytest_status": str(summary.get("pytest_status") or "").strip() or None,
        "live_count": _optional_int(summary.get("live_count")),
        "minimum_count": _optional_int(summary.get("minimum_count")),
    }


def _task_batch_summary(suite_results: list[dict[str, Any]]) -> dict[str, Any] | None:
    for suite in suite_results:
        if str(suite.get("runner_kind") or "").strip().lower() != "task_batch":
            continue
        default_reason = str(suite.get("selector") or "").strip() or None
        selected_tasks = []
        for item in list(suite.get("selected_tasks") or []):
            task = dict(item)
            if default_reason and not str(task.get("selection_reason") or "").strip():
                task["selection_reason"] = default_reason
            selected_tasks.append(task)
        lineage = dict(suite.get("lineage_findings") or {})
        behavior = dict(suite.get("behavior_regressions") or {})
        return {
            "suite_id": str(suite.get("suite_id") or "").strip() or None,
            "selector": str(suite.get("selector") or "").strip() or None,
            "selected_task_count": len(selected_tasks),
            "selected_tasks": selected_tasks,
            "lineage_problem_count": int(lineage.get("problem_count") or 0),
            "lineage_findings": lineage,
            "behavior_status": str(behavior.get("status") or "").strip() or "pending",
            "behavior_regressions": behavior,
        }
    return None


def _patchset_rerun_command(patchset_id: str) -> str:
    return _cli_command(["ait", "patchset", "rerun-ci", patchset_id])


def _repo_rerun_command(job: dict[str, Any]) -> str:
    payload = dict(job.get("payload") or {})
    suite_ids = _unique_strs(payload.get("suite_ids") or []) or _suite_ids_for_job(job)
    parts = ["ait", "repo", "run-ci"]
    plane = str(payload.get("plane") or "").strip()
    if plane:
        parts.extend(["--plane", plane])
    else:
        for suite_id in suite_ids:
            parts.extend(["--suite", suite_id])
    target_line = str(payload.get("target_line") or "").strip()
    if target_line and target_line != "main":
        parts.extend(["--target-line", target_line])
    selector = str(payload.get("selector") or "").strip()
    if selector:
        parts.extend(["--selector", selector])
    for task_id in _unique_strs(payload.get("task_ids") or []):
        parts.extend(["--task-id", task_id])
    curated_corpus = str(payload.get("curated_corpus") or "").strip()
    if curated_corpus:
        parts.extend(["--curated-corpus", curated_corpus])
    count = payload.get("count")
    if count is not None:
        parts.extend(["--count", str(count)])
    window_days = payload.get("window_days")
    if window_days is not None:
        parts.extend(["--window-days", str(window_days)])
    return _cli_command(parts)


def _summarize_patchset_job(job: dict[str, Any]) -> dict[str, Any]:
    result = dict(job.get("result") or {})
    suite_results = [dict(item) for item in list(result.get("suite_results") or [])]
    return {
        "job_id": int(job.get("job_id") or 0),
        "job_type": str(job.get("job_type") or ""),
        "state": str(job.get("state") or ""),
        "diagnostic_status": str(job.get("diagnostic_status") or ""),
        "trigger": str(result.get("trigger") or job.get("payload", {}).get("trigger") or "").strip() or None,
        "created_at": job.get("created_at"),
        "updated_at": job.get("updated_at"),
        "tests_status": _patchset_job_status(job),
        "selected_suite_ids": _unique_strs(result.get("selected_suite_ids") or []),
        "blocking_failures": _unique_strs(result.get("blocking_failures") or []),
        "suite_results": suite_results,
        "rerun": {"cli": _patchset_rerun_command(str(job.get("payload", {}).get("patchset_id") or ""))},
        "payload": dict(job.get("payload") or {}),
        "result": result,
    }


def _summarize_repo_job(job: dict[str, Any]) -> dict[str, Any]:
    result = dict(job.get("result") or {})
    suite_results = [dict(item) for item in list(result.get("suite_results") or [])]
    return {
        "job_id": int(job.get("job_id") or 0),
        "job_type": str(job.get("job_type") or ""),
        "state": str(job.get("state") or ""),
        "diagnostic_status": str(job.get("diagnostic_status") or ""),
        "trigger": str(result.get("trigger") or job.get("payload", {}).get("trigger") or "").strip() or None,
        "created_at": job.get("created_at"),
        "updated_at": job.get("updated_at"),
        "target_line": str(result.get("target_line") or job.get("payload", {}).get("target_line") or "main"),
        "status": _repo_job_status(job),
        "plane": str(job.get("payload", {}).get("plane") or "").strip() or None,
        "selected_planes": _planes_for_job(job),
        "requested_suite_ids": _unique_strs(job.get("payload", {}).get("suite_ids") or []),
        "selected_suite_ids": _suite_ids_for_job(job),
        "blocking_failures": _unique_strs(result.get("blocking_failures") or []),
        "task_batch": _task_batch_summary(suite_results),
        "summary_artifacts": _summary_artifacts_for_suite_results(suite_results),
        "suite_results": suite_results,
        "rerun": {"cli": _repo_rerun_command(job)},
        "payload": dict(job.get("payload") or {}),
        "result": result,
    }


def patchset_ci_status(ctx: ServerContext, patchset_id: str, *, recent_limit: int = 10) -> dict[str, Any]:
    patchset = get_patchset(ctx, patchset_id)
    change = get_change(ctx, patchset["change_id"])
    try:
        attestation = get_attestation(ctx, patchset_id)
    except KeyError:
        attestation = None
    attestation_ci = dict(((attestation or {}).get("detail") or {}).get("patchset_ci") or {})
    jobs = [
        job
        for job in list_jobs(ctx, repo_name=change["repo_name"], limit=max(recent_limit * 10, 50))
        if str(job.get("job_type") or "") == "patchset.ci"
        and str((job.get("payload") or {}).get("patchset_id") or "") == patchset_id
    ]
    recent_jobs = [_summarize_patchset_job(job) for job in jobs[:recent_limit]]
    latest_job = recent_jobs[0] if recent_jobs else None
    suite_results = [dict(item) for item in list(attestation_ci.get("suite_results") or [])]
    selected_suite_ids = _unique_strs(attestation_ci.get("selected_suite_ids") or [])
    blocking_failures = _unique_strs(attestation_ci.get("blocking_failures") or [])
    tests_status = str(attestation_ci.get("tests_status") or "").strip()
    if not tests_status and latest_job is not None:
        tests_status = str(latest_job.get("tests_status") or "").strip()
    if not tests_status:
        tests_status = "pending"
    final_selected_suite_ids = selected_suite_ids or (list(latest_job.get("selected_suite_ids") or []) if latest_job else [])
    final_blocking_failures = blocking_failures or (list(latest_job.get("blocking_failures") or []) if latest_job else [])
    final_suite_results = suite_results or (list(latest_job.get("suite_results") or []) if latest_job else [])
    tg1_required = _normalize_tg1_required_summary(
        final_suite_results,
        selected_suite_ids=final_selected_suite_ids,
        blocking_failures=final_blocking_failures,
        tests_status=tests_status,
    )
    return {
        "patchset_id": patchset_id,
        "change_id": change["change_id"],
        "repo_name": change["repo_name"],
        "available": bool(attestation_ci or latest_job),
        "tests_status": tests_status,
        "selected_suite_ids": final_selected_suite_ids,
        "blocking_failures": final_blocking_failures,
        "suite_results": final_suite_results,
        "tg1_required": tg1_required,
        "attestation_updated_at": (attestation or {}).get("updated_at"),
        "latest_job": latest_job,
        "recent_jobs": recent_jobs,
        "rerun": {"cli": _patchset_rerun_command(patchset_id)},
    }


def repository_ci_runs(
    ctx: ServerContext,
    repo_name: str,
    *,
    limit: int = 20,
    plane: str | None = None,
    suite_id: str | None = None,
) -> dict[str, Any]:
    resolved_limit = max(int(limit), 1)
    normalized_plane = str(plane or "").strip() or None
    normalized_suite = str(suite_id or "").strip() or None
    jobs = [
        job
        for job in list_jobs(ctx, repo_name=repo_name, limit=max(resolved_limit * 10, 50))
        if str(job.get("job_type") or "") == "repo.ci"
    ]
    items: list[dict[str, Any]] = []
    for job in jobs:
        summary = _summarize_repo_job(job)
        if normalized_plane is not None:
            planes = set(summary.get("selected_planes") or [])
            if summary.get("plane"):
                planes.add(str(summary["plane"]))
            if normalized_plane not in planes:
                continue
        if normalized_suite is not None and normalized_suite not in set(summary.get("selected_suite_ids") or []):
            continue
        items.append(summary)
        if len(items) >= resolved_limit:
            break

    latest_by_suite: dict[str, dict[str, Any]] = {}
    latest_by_plane: dict[str, dict[str, Any]] = {}
    for item in items:
        for candidate_suite in item.get("selected_suite_ids") or []:
            latest_by_suite.setdefault(
                candidate_suite,
                {
                    "job_id": item["job_id"],
                    "status": item["status"],
                    "state": item["state"],
                    "updated_at": item["updated_at"],
                },
            )
        for candidate_plane in item.get("selected_planes") or ([] if item.get("plane") is None else [item["plane"]]):
            latest_by_plane.setdefault(
                candidate_plane,
                {
                    "job_id": item["job_id"],
                    "status": item["status"],
                    "state": item["state"],
                    "updated_at": item["updated_at"],
                },
            )

    return {
        "repo_name": repo_name,
        "filters": {"limit": resolved_limit, "plane": normalized_plane, "suite_id": normalized_suite},
        "count": len(items),
        "summary": {
            "active_runs": sum(1 for item in items if item["state"] in {"queued", "running"}),
            "failed_runs": sum(1 for item in items if item["status"] == "fail"),
            "latest_by_suite": latest_by_suite,
            "latest_by_plane": latest_by_plane,
        },
        "items": items,
    }
