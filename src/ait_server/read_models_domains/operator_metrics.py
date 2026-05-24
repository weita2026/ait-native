from __future__ import annotations

import os
from typing import Any, Mapping

from ..server_paths import ServerContext


def _legacy_read_models_module():
    from .. import read_models as legacy_read_models

    return legacy_read_models


_OPERATOR_ACTION_PRIORITY = {
    "migrate_to_postgres": 60,
    "reclaim_stale": 50,
    "inspect_failed": 40,
    "exhausted_failed": 40,
    "wait_for_retry": 30,
    "monitor_workers": 20,
    "inspect": 10,
    "optimize": 10,
    "repack": 10,
    "pack": 10,
    "gc": 10,
    "none": 0,
}


def _int_metric(value: Any) -> int:
    try:
        return int(value or 0)
    except (TypeError, ValueError):
        return 0


def _count_rows(rows: list[Mapping[str, Any]], field: str) -> dict[str, int]:
    summary: dict[str, int] = {}
    for row in rows:
        key = str(row.get(field) or "unknown")
        summary[key] = summary.get(key, 0) + 1
    return dict(sorted(summary.items()))


def _merge_count_summary(target: dict[str, int], source: Mapping[str, Any]) -> None:
    for key, value in source.items():
        target[str(key)] = target.get(str(key), 0) + _int_metric(value)


def _ranked_operator_action(actions: list[str]) -> str:
    if not actions:
        return "none"
    return max(actions, key=lambda item: (_OPERATOR_ACTION_PRIORITY.get(item, 1), item))


def operator_pressure_cache_ttl_seconds() -> float:
    raw = str(os.environ.get("AIT_SERVER_PRESSURE_METRICS_CACHE_TTL_SECONDS", "5")).strip()
    try:
        return max(float(raw), 0.0)
    except ValueError:
        return 5.0


def _first_present(mappings: tuple[Mapping[str, Any], ...], *keys: str) -> Any:
    for mapping in mappings:
        for key in keys:
            if key in mapping and mapping.get(key) is not None:
                return mapping.get(key)
    return None


def server_metrics(
    ctx: ServerContext,
    *,
    recent_jobs_limit: int = 50,
    stale_after_seconds: int = 300,
) -> dict[str, Any]:
    """Return read-only server-level operator metrics across repositories."""
    rm = _legacy_read_models_module()
    if recent_jobs_limit < 0:
        raise ValueError("recent_jobs_limit must be greater than or equal to zero")
    if stale_after_seconds <= 0:
        raise ValueError("stale_after_seconds must be greater than zero")

    index = rm.repository_index(ctx)
    repository_rows: list[dict[str, Any]] = []
    storage_state_summary: dict[str, int] = {}
    job_state_summary: dict[str, int] = {}
    worker_state_summary: dict[str, int] = {}
    active_workers: dict[str, dict[str, Any]] = {}
    storage_recommended_actions: list[str] = []
    job_recommended_actions: list[str] = []

    storage_metrics = {
        "repo_count": _int_metric(index.get("count")),
        "total_lines": _int_metric(index.get("total_lines")),
        "total_snapshots": 0,
        "tracked_blob_count": 0,
        "packed_blob_count": 0,
        "packed_delta_blob_count": 0,
        "pack_count": 0,
        "logical_tracked_blob_bytes": 0,
        "physical_storage_bytes": 0,
        "storage_savings_bytes": 0,
        "drift_count": 0,
        "repairable_drift_count": 0,
        "repos_needing_attention": 0,
    }

    worker_metrics = {
        "repo_count": _int_metric(index.get("count")),
        "active_worker_count": 0,
        "queued_jobs": 0,
        "running_jobs": 0,
        "succeeded_jobs": 0,
        "failed_jobs": 0,
        "stale_running_jobs": 0,
        "delayed_retry_jobs": 0,
        "exhausted_jobs": 0,
        "active_live_turns": 0,
        "active_live_turn_repositories": 0,
        "oldest_live_turn_age_seconds": 0.0,
    }

    live_turn_metrics = rm.normalize_live_turn_metrics(rm.snapshot_live_turn_metrics())
    live_turn_summary = live_turn_metrics.get("summary") if isinstance(live_turn_metrics.get("summary"), dict) else {}
    live_turn_repo_activity = {
        str(row.get("repo_name") or ""): _int_metric(row.get("active_turns"))
        for row in (live_turn_metrics.get("repo_activity") or [])
        if isinstance(row, Mapping) and str(row.get("repo_name") or "")
    }

    for repo in index.get("repositories") or []:
        repo_name = str(repo.get("repo_name") or "")
        if not repo_name:
            continue
        storage = rm.get_repository_storage(ctx, repo_name)
        workers = rm.repository_worker_status(ctx, repo_name, recent_jobs_limit=recent_jobs_limit)
        validation = storage.get("validation_summary") or {}
        signals = storage.get("signals_summary") or {}
        optimization = storage.get("optimization_summary") or {}
        efficiency = storage.get("efficiency_summary") or {}
        diagnostics = workers.get("diagnostics") or {}

        storage_state = str(validation.get("state") or "unknown")
        storage_action = str(validation.get("recommended_action") or "none")
        job_action = str(diagnostics.get("recommended_action") or "none")
        storage_state_summary[storage_state] = storage_state_summary.get(storage_state, 0) + 1
        storage_recommended_actions.append(storage_action)
        job_recommended_actions.append(job_action)
        _merge_count_summary(job_state_summary, workers.get("state_summary") or {})
        _merge_count_summary(worker_state_summary, workers.get("state_summary") or {})

        active_worker_count = _int_metric(workers.get("worker_count"))
        queued_jobs = _int_metric(workers.get("queued_jobs"))
        running_jobs = _int_metric(workers.get("running_jobs"))
        succeeded_jobs = _int_metric(workers.get("succeeded_jobs"))
        failed_jobs = _int_metric(workers.get("failed_jobs"))
        stale_running_jobs = _int_metric(diagnostics.get("stale_running_jobs"))
        delayed_retry_jobs = _int_metric(diagnostics.get("delayed_retry_jobs"))
        exhausted_jobs = _int_metric(diagnostics.get("exhausted_jobs"))
        drift_count = _int_metric(signals.get("drift_count"))
        repairable_drift_count = _int_metric(signals.get("repairable_drift_count"))
        needs_attention = bool(validation.get("needs_attention", False))

        storage_metrics["total_snapshots"] += _int_metric(storage.get("snapshot_count"))
        storage_metrics["tracked_blob_count"] += _int_metric(optimization.get("tracked_blob_count"))
        storage_metrics["packed_blob_count"] += _int_metric(storage.get("packed_blob_count"))
        storage_metrics["packed_delta_blob_count"] += _int_metric(storage.get("packed_delta_blob_count"))
        storage_metrics["pack_count"] += _int_metric(storage.get("pack_count"))
        storage_metrics["logical_tracked_blob_bytes"] += _int_metric(efficiency.get("logical_tracked_blob_bytes"))
        storage_metrics["physical_storage_bytes"] += _int_metric(efficiency.get("physical_storage_bytes"))
        storage_metrics["storage_savings_bytes"] += _int_metric(efficiency.get("storage_savings_bytes"))
        storage_metrics["drift_count"] += drift_count
        storage_metrics["repairable_drift_count"] += repairable_drift_count
        storage_metrics["repos_needing_attention"] += 1 if needs_attention else 0

        worker_metrics["active_worker_count"] += active_worker_count
        worker_metrics["queued_jobs"] += queued_jobs
        worker_metrics["running_jobs"] += running_jobs
        worker_metrics["succeeded_jobs"] += succeeded_jobs
        worker_metrics["failed_jobs"] += failed_jobs
        worker_metrics["stale_running_jobs"] += stale_running_jobs
        worker_metrics["delayed_retry_jobs"] += delayed_retry_jobs
        worker_metrics["exhausted_jobs"] += exhausted_jobs

        for worker in workers.get("workers") or []:
            worker_id = str(worker.get("worker_id") or "")
            if not worker_id:
                continue
            item = active_workers.setdefault(
                worker_id,
                {
                    "worker_id": worker_id,
                    "running_jobs": 0,
                    "repositories": set(),
                    "oldest_locked_job": None,
                    "latest_locked_job": None,
                },
            )
            item["running_jobs"] += _int_metric(worker.get("running_jobs"))
            item["repositories"].add(repo_name)
            oldest = worker.get("oldest_locked_job")
            latest = worker.get("latest_locked_job")
            if oldest is not None and (item["oldest_locked_job"] is None or str(oldest) < str(item["oldest_locked_job"])):
                item["oldest_locked_job"] = oldest
            if latest is not None and (item["latest_locked_job"] is None or str(latest) > str(item["latest_locked_job"])):
                item["latest_locked_job"] = latest

        repository_rows.append(
            {
                "repo_name": repo_name,
                "default_line": repo.get("default_line"),
                "line_count": _int_metric(repo.get("line_count")),
                "snapshot_count": _int_metric(storage.get("snapshot_count")),
                "storage_state": storage_state,
                "storage_recommended_action": storage_action,
                "storage_needs_attention": needs_attention,
                "tracked_blob_count": _int_metric(optimization.get("tracked_blob_count")),
                "packed_blob_count": _int_metric(storage.get("packed_blob_count")),
                "packed_delta_blob_count": _int_metric(storage.get("packed_delta_blob_count")),
                "physical_storage_bytes": _int_metric(efficiency.get("physical_storage_bytes")),
                "storage_savings_bytes": _int_metric(efficiency.get("storage_savings_bytes")),
                "drift_count": drift_count,
                "repairable_drift_count": repairable_drift_count,
                "worker_count": active_worker_count,
                "queued_jobs": queued_jobs,
                "running_jobs": running_jobs,
                "succeeded_jobs": succeeded_jobs,
                "failed_jobs": failed_jobs,
                "stale_running_jobs": stale_running_jobs,
                "delayed_retry_jobs": delayed_retry_jobs,
                "exhausted_jobs": exhausted_jobs,
                "job_recommended_action": job_action,
                "active_live_turns": live_turn_repo_activity.get(repo_name, 0),
            }
        )

    with rm.connect(ctx) as conn:
        job_rows = [
            dict(row)
            for row in conn.execute(
                """
                select job_id, repo_name, job_type, state, attempt_count, max_attempts,
                       locked_by, locked_at, available_at, last_error, created_at, updated_at
                from jobs
                order by job_id desc
                """
            ).fetchall()
        ]
    diagnostics = rm.job_diagnostics(
        ctx,
        repo_name=None,
        stale_after_seconds=stale_after_seconds,
        limit=max(len(job_rows), recent_jobs_limit, 100),
    )
    job_type_summary = _count_rows(job_rows, "job_type")
    all_job_state_summary = _count_rows(job_rows, "state")
    if all_job_state_summary:
        job_state_summary = all_job_state_summary
    job_action = str(diagnostics.get("recommended_action") or _ranked_operator_action(job_recommended_actions))
    storage_action = _ranked_operator_action(storage_recommended_actions)
    operator_action = job_action if job_action != "none" else ("inspect_storage" if storage_metrics["repos_needing_attention"] else "none")

    active_worker_list = []
    for item in active_workers.values():
        row = dict(item)
        row["repositories"] = sorted(row["repositories"])
        active_worker_list.append(row)
    active_worker_list.sort(key=lambda item: (-_int_metric(item.get("running_jobs")), str(item.get("worker_id"))))
    worker_metrics["active_worker_count"] = len(active_worker_list)
    worker_metrics["workers"] = active_worker_list
    worker_metrics["state_summary"] = dict(sorted(worker_state_summary.items()))
    worker_metrics["active_live_turns"] = _int_metric(live_turn_summary.get("active_turns"))
    worker_metrics["active_live_turn_repositories"] = _int_metric(live_turn_summary.get("active_repositories"))
    worker_metrics["oldest_live_turn_age_seconds"] = round(float(live_turn_summary.get("oldest_active_turn_age_seconds") or 0.0), 3)

    storage_metrics["storage_state_summary"] = dict(sorted(storage_state_summary.items()))
    storage_metrics["recommended_action"] = storage_action
    job_outcome_metrics = {
        "total_jobs": len(job_rows),
        "state_summary": dict(sorted(job_state_summary.items())),
        "job_type_summary": job_type_summary,
        "active_jobs": _int_metric(job_state_summary.get("queued")) + _int_metric(job_state_summary.get("running")),
        "succeeded_jobs": _int_metric(job_state_summary.get("succeeded")),
        "failed_jobs": _int_metric(job_state_summary.get("failed")),
        "stale_running_jobs": _int_metric(diagnostics.get("stale_running_jobs")),
        "delayed_retry_jobs": _int_metric(diagnostics.get("delayed_retry_jobs")),
        "retryable_jobs": _int_metric(diagnostics.get("retryable_jobs")),
        "exhausted_jobs": _int_metric(diagnostics.get("exhausted_jobs")),
        "recommended_action": job_action,
        "recommended_action_reason": diagnostics.get("recommended_action_reason"),
        "recent_jobs_limit": recent_jobs_limit,
        "stale_after_seconds": stale_after_seconds,
        "recent_jobs": job_rows[:recent_jobs_limit],
    }

    return rm.annotate_operator_read_payload({
        "repo_name": "ait",
        "snapshot_at": rm.utc_now(),
        "summary": {
            "repo_count": storage_metrics["repo_count"],
            "total_lines": storage_metrics["total_lines"],
            "repos_needing_storage_attention": storage_metrics["repos_needing_attention"],
            "active_workers": worker_metrics["active_worker_count"],
            "active_jobs": job_outcome_metrics["active_jobs"],
            "failed_jobs": job_outcome_metrics["failed_jobs"],
            "active_live_turns": worker_metrics["active_live_turns"],
            "recommended_action": operator_action,
        },
        "storage_metrics": storage_metrics,
        "worker_metrics": worker_metrics,
        "job_outcome_metrics": job_outcome_metrics,
        "live_turn_metrics": live_turn_metrics,
        "live_turn_pressure": rm.live_turn_pressure_summary(live_turn_metrics),
        "repositories": repository_rows,
    })


def _readiness_check(
    name: str,
    status: str,
    *,
    summary: str,
    detail: str | None = None,
    recommended_action: str = "none",
) -> dict[str, Any]:
    return {
        "name": name,
        "status": status,
        "summary": summary,
        "detail": detail,
        "recommended_action": recommended_action,
    }


def server_readiness(
    ctx: ServerContext,
    *,
    recent_jobs_limit: int = 50,
    stale_after_seconds: int = 300,
) -> dict[str, Any]:
    """Return a read-only server readiness preflight for operators."""

    rm = _legacy_read_models_module()
    metrics = server_metrics(ctx, recent_jobs_limit=recent_jobs_limit, stale_after_seconds=stale_after_seconds)
    summary = metrics.get("summary") if isinstance(metrics.get("summary"), dict) else {}
    storage = metrics.get("storage_metrics") if isinstance(metrics.get("storage_metrics"), dict) else {}
    jobs = metrics.get("job_outcome_metrics") if isinstance(metrics.get("job_outcome_metrics"), dict) else {}
    workers = metrics.get("worker_metrics") if isinstance(metrics.get("worker_metrics"), dict) else {}
    live_turns = metrics.get("live_turn_metrics") if isinstance(metrics.get("live_turn_metrics"), dict) else {}
    live_turn_summary = live_turns.get("summary") if isinstance(live_turns.get("summary"), dict) else {}
    shared_runtime_policy = rm.evaluate_shared_runtime_policy(
        ctx,
        component="ait-server",
        allow_legacy_override=False,
    )

    checks: list[dict[str, Any]] = [
        _readiness_check(
            "server_health",
            "pass",
            summary="Server process answered the readiness request.",
            detail=f"db_backend={ctx.db_backend}",
        )
    ]

    if not shared_runtime_policy.ok:
        checks.append(
            _readiness_check(
                "shared_runtime_policy",
                "fail",
                summary="Unsupported server runtime backend is blocked by policy.",
                detail=shared_runtime_policy.reason,
                recommended_action="configure_postgres",
            )
        )
    else:
        checks.append(
            _readiness_check(
                "shared_runtime_policy",
                "pass",
                summary=shared_runtime_policy.reason,
            )
        )

    storage_attention = _int_metric(summary.get("repos_needing_storage_attention"))
    drift_count = _int_metric(storage.get("drift_count"))
    if storage_attention or drift_count:
        checks.append(
            _readiness_check(
                "storage_integrity",
                "fail",
                summary="One or more repositories need storage attention.",
                detail=f"repos_needing_attention={storage_attention}; drift_count={drift_count}",
                recommended_action="inspect_storage",
            )
        )
    else:
        checks.append(
            _readiness_check(
                "storage_integrity",
                "pass",
                summary="No repository storage drift or attention count is reported.",
            )
        )

    stale_jobs = _int_metric(jobs.get("stale_running_jobs"))
    failed_jobs = _int_metric(jobs.get("failed_jobs"))
    exhausted_jobs = _int_metric(jobs.get("exhausted_jobs"))
    delayed_retry_jobs = _int_metric(jobs.get("delayed_retry_jobs"))
    active_jobs = _int_metric(jobs.get("active_jobs"))
    job_action = str(jobs.get("recommended_action") or "none")
    if stale_jobs or failed_jobs or exhausted_jobs:
        checks.append(
            _readiness_check(
                "job_recovery",
                "fail",
                summary="Job recovery attention is required before treating the server as ready.",
                detail=(
                    f"stale_running_jobs={stale_jobs}; failed_jobs={failed_jobs}; "
                    f"exhausted_jobs={exhausted_jobs}; delayed_retry_jobs={delayed_retry_jobs}"
                ),
                recommended_action=job_action if job_action != "none" else "inspect_failed",
            )
        )
    elif delayed_retry_jobs:
        checks.append(
            _readiness_check(
                "job_recovery",
                "warn",
                summary="Retryable jobs are waiting for their next attempt.",
                detail=f"delayed_retry_jobs={delayed_retry_jobs}; active_jobs={active_jobs}",
                recommended_action=job_action if job_action != "none" else "wait_for_retry",
            )
        )
    else:
        checks.append(
            _readiness_check(
                "job_recovery",
                "pass",
                summary="No stale, failed, exhausted, or delayed retry jobs are reported.",
                detail=f"active_jobs={active_jobs}",
            )
        )

    postgres_checks: dict[str, Any]
    if ctx.db_backend == "postgres":
        try:
            postgres_checks = rm.postgres_schema_upgrade_checks(ctx, apply=False)
        except Exception as exc:  # pragma: no cover - defensive; exercised by operator environments.
            postgres_checks = {
                "backend": ctx.db_backend,
                "applied": False,
                "ok": False,
                "checks": {},
                "error": str(exc),
            }
        if postgres_checks.get("ok"):
            checks.append(
                _readiness_check(
                    "postgres_schema_versions",
                    "pass",
                    summary="PostgreSQL content/control schema versions match this server.",
                )
            )
        else:
            checks.append(
                _readiness_check(
                    "postgres_schema_versions",
                    "fail",
                    summary="PostgreSQL schema version checks did not pass.",
                    detail=str(postgres_checks.get("error") or "content/control schema version mismatch"),
                    recommended_action="inspect_postgres",
                )
            )
    else:
        postgres_checks = {
            "backend": ctx.db_backend,
            "applied": False,
            "ok": False,
            "skipped": False,
            "reason": "Non-PostgreSQL server runtime backends are not supported.",
        }
        checks.append(
            _readiness_check(
                "postgres_schema_versions",
                "fail",
                summary="PostgreSQL is required for server runtime state.",
                recommended_action="configure_postgres",
            )
        )

    failed_checks = [check for check in checks if check.get("status") == "fail"]
    warning_checks = [check for check in checks if check.get("status") == "warn"]
    action_candidates = [
        str(check.get("recommended_action") or "none")
        for check in checks
        if str(check.get("recommended_action") or "none") != "none"
    ]
    metrics_action = str(summary.get("recommended_action") or "none")
    if metrics_action != "none":
        action_candidates.append(metrics_action)
    recommended_action = _ranked_operator_action(action_candidates)

    return rm.annotate_operator_read_payload({
        "repo_name": "ait",
        "snapshot_at": rm.utc_now(),
        "ready": not failed_checks,
        "recommended_action": recommended_action,
        "runtime": {
            "db_backend": ctx.db_backend,
            "using_postgres": ctx.using_postgres,
            "server_data_root": str(ctx.root),
            "sqlite_local_default_preserved": True,
            "shared_runtime_policy": shared_runtime_policy.as_dict(),
        },
        "summary": {
            "repo_count": _int_metric(summary.get("repo_count")),
            "total_lines": _int_metric(summary.get("total_lines")),
            "active_workers": _int_metric(summary.get("active_workers")),
            "active_jobs": active_jobs,
            "failed_jobs": failed_jobs,
            "active_live_turns": _int_metric(summary.get("active_live_turns")),
            "warning_checks": len(warning_checks),
            "failed_checks": len(failed_checks),
        },
        "checks": checks,
        "metrics_summary": summary,
        "repository_names": [str(row.get("repo_name") or "") for row in metrics.get("repositories") or [] if str(row.get("repo_name") or "")],
        "storage_summary": {
            "repos_needing_attention": _int_metric(storage.get("repos_needing_attention")),
            "drift_count": drift_count,
            "repairable_drift_count": _int_metric(storage.get("repairable_drift_count")),
            "recommended_action": storage.get("recommended_action"),
        },
        "worker_summary": {
            "active_worker_count": _int_metric(workers.get("active_worker_count")),
            "queued_jobs": _int_metric(workers.get("queued_jobs")),
            "running_jobs": _int_metric(workers.get("running_jobs")),
            "stale_running_jobs": stale_jobs,
            "delayed_retry_jobs": delayed_retry_jobs,
            "exhausted_jobs": exhausted_jobs,
            "active_live_turns": _int_metric(workers.get("active_live_turns")),
            "oldest_live_turn_age_seconds": round(float(workers.get("oldest_live_turn_age_seconds") or 0.0), 3),
        },
        "job_summary": {
            "total_jobs": _int_metric(jobs.get("total_jobs")),
            "active_jobs": active_jobs,
            "failed_jobs": failed_jobs,
            "stale_running_jobs": stale_jobs,
            "delayed_retry_jobs": delayed_retry_jobs,
            "exhausted_jobs": exhausted_jobs,
            "recommended_action": jobs.get("recommended_action"),
        },
        "live_turn_summary": {
            "active_turns": _int_metric(live_turn_summary.get("active_turns")),
            "active_repositories": _int_metric(live_turn_summary.get("active_repositories")),
            "oldest_active_turn_started_at": live_turn_summary.get("oldest_active_turn_started_at"),
            "oldest_active_turn_age_seconds": round(float(live_turn_summary.get("oldest_active_turn_age_seconds") or 0.0), 3),
            "recent_completed_turns": _int_metric(live_turn_summary.get("recent_completed_turns")),
            "recent_failed_turns": _int_metric(live_turn_summary.get("recent_failed_turns")),
            "recent_completed_p95_seconds": live_turn_summary.get("recent_completed_p95_seconds"),
        },
        "live_turn_pressure": rm.live_turn_pressure_summary(live_turns),
        "postgres_schema": postgres_checks,
    })
