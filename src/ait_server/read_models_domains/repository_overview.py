from __future__ import annotations

from typing import Any

from ..server_paths import ServerContext


def _legacy_read_models_module():
    from .. import read_models as legacy_read_models

    return legacy_read_models


def repository_index(ctx: ServerContext) -> dict[str, Any]:
    rm = _legacy_read_models_module()
    with rm.connect_content(ctx) as conn:
        rows = [
            dict(r)
            for r in conn.execute(
                "select repo_name, repo_id, default_line, created_at, updated_at from repositories order by repo_name asc"
            ).fetchall()
        ]
    repositories: list[dict[str, Any]] = []
    repositories_by_name: dict[str, dict[str, Any]] = {}
    total_lines = 0
    for row in rows:
        line_count = len(rm.list_lines(ctx, row["repo_name"]))
        total_lines += line_count
        entry = {
            "repo_name": row["repo_name"],
            "repo_id": row["repo_id"],
            "default_line": row["default_line"],
            "created_at": row["created_at"],
            "updated_at": row["updated_at"],
            "line_count": line_count,
        }
        repositories.append(entry)
        repositories_by_name[str(row["repo_name"])] = entry

    groups_payload: list[dict[str, Any]] = []
    for group in rm.list_repository_groups(ctx):
        repo_entries = [
            repositories_by_name[repo_name]
            for repo_name in group.get("repo_names") or []
            if repo_name in repositories_by_name
        ]
        groups_payload.append(
            {
                "group_id": str(group.get("group_id") or ""),
                "title": str(group.get("title") or ""),
                "sort_index": rm._safe_int(group.get("sort_index")),
                "system_slug": str(group.get("system_slug") or ""),
                "is_main": bool(group.get("is_main")),
                "repo_count": len(repo_entries),
                "repositories": repo_entries,
            }
        )
    return {
        "count": len(repositories),
        "total_lines": total_lines,
        "repositories": repositories,
        "groups": groups_payload,
        "group_count": len(groups_payload),
    }


def _line_work_context(ctx: ServerContext, repo_name: str, head_snapshot_id: str | None) -> dict[str, Any] | None:
    rm = _legacy_read_models_module()
    if not head_snapshot_id:
        return None
    with rm.connect(ctx) as conn:
        scope_predicate, scope_params = rm._repo_scope_filter(ctx, repo_name, alias="c")
        row = conn.execute(
            f"""
            select
                c.change_id,
                c.title as change_title,
                c.status as change_status,
                c.landed_at,
                p.patchset_id,
                p.patchset_number,
                t.task_id,
                t.title as task_title,
                t.status as task_status
            from patchsets p
            join changes c on c.change_id = p.change_id
            join tasks t on t.task_id = c.task_id
            where {scope_predicate} and p.revision_snapshot_id = ?
            order by
                case c.status
                    when 'landed' then 0
                    when 'review' then 1
                    when 'gated' then 2
                    when 'approved' then 3
                    when 'landable' then 4
                    when 'blocked' then 5
                when 'draft' then 6
                else 7
            end,
            p.created_at desc
            limit 1
            """,
            (*scope_params, head_snapshot_id),
        ).fetchone()
    if row is None:
        return None
    return {
        "change_id": row["change_id"],
        "change_title": row["change_title"],
        "change_status": row["change_status"],
        "landed_at": row["landed_at"],
        "patchset_id": row["patchset_id"],
        "patchset_number": row["patchset_number"],
        "task_id": row["task_id"],
        "task_title": row["task_title"],
        "task_status": row["task_status"],
    }


def repository_detail(ctx: ServerContext, repo_name: str, *, job_limit: int = 20) -> dict[str, Any]:
    rm = _legacy_read_models_module()
    repo = rm.get_repository(ctx, repo_name)
    lines = rm.list_lines(ctx, repo_name)
    annotated_lines: list[dict[str, Any]] = []
    active_lines: list[dict[str, Any]] = []
    archived_lines: list[dict[str, Any]] = []
    for line in lines:
        item = dict(line)
        item["status"] = item.get("status") or "active"
        item["work_context"] = None if item["line_name"] == repo["default_line"] else _line_work_context(ctx, repo_name, item.get("head_snapshot_id"))
        annotated_lines.append(item)
        if item["status"] == "archived":
            archived_lines.append(item)
        else:
            active_lines.append(item)
    jobs = rm.list_jobs(ctx, repo_name=repo_name, limit=job_limit)
    ci_runs = rm.repository_ci_runs(ctx, repo_name, limit=job_limit)
    diagnostics = rm.job_diagnostics(ctx, repo_name=repo_name, limit=max(job_limit, 100))
    storage = rm.get_repository_storage(ctx, repo_name)
    validation = storage.get("validation_summary") or {}
    signals = storage.get("signals_summary") or {}
    active_jobs = sum(1 for job in jobs if job["state"] in {"queued", "running", "active"})
    failed_jobs = sum(1 for job in jobs if job["state"] in {"failed", "blocked"})
    return {
        "repository": repo,
        "lines": annotated_lines,
        "active_lines": active_lines,
        "archived_lines": archived_lines,
        "line_summary": {
            "total_lines": len(annotated_lines),
            "active_lines": len(active_lines),
            "archived_lines": len(archived_lines),
        },
        "jobs": jobs,
        "ci_runs": ci_runs["items"],
        "ci_summary": ci_runs["summary"],
        "job_diagnostics": diagnostics,
        "storage": storage,
        "storage_summary": {
            "state": validation.get("state", "unknown"),
            "recommended_action": validation.get("recommended_action", "none"),
            "next_actions": list(validation.get("next_actions") or []),
            "reasons": list(validation.get("reasons") or []),
            "needs_attention": bool(validation.get("needs_attention", False)),
            "drift_count": int(signals.get("drift_count", 0)),
            "repairable_drift_count": int(signals.get("repairable_drift_count", 0)),
        },
        "job_summary": {
            "job_limit": job_limit,
            "recent_jobs": len(jobs),
            "active_jobs": active_jobs,
            "failed_jobs": failed_jobs,
            "stale_running_jobs": int(diagnostics.get("stale_running_jobs") or 0),
            "delayed_retry_jobs": int(diagnostics.get("delayed_retry_jobs") or 0),
            "exhausted_jobs": int(diagnostics.get("exhausted_jobs") or 0),
            "recommended_action": diagnostics.get("recommended_action", "none"),
        },
    }


def repository_worker_status(ctx: ServerContext, repo_name: str, *, recent_jobs_limit: int = 20) -> dict[str, Any]:
    rm = _legacy_read_models_module()
    repo = rm.get_repository(ctx, repo_name)
    if recent_jobs_limit < 0:
        raise ValueError("recent_jobs_limit must be greater than or equal to zero")

    with rm.connect(ctx) as conn:
        scope_predicate, scope_params = rm._repo_scope_filter(ctx, repo_name)
        where_clause = f" where {scope_predicate}" if scope_predicate else ""
        rows = conn.execute(
            "select * from jobs" + where_clause + " order by job_id desc",
            scope_params,
        ).fetchall()

    state_summary: dict[str, int] = {}
    workers: dict[str, dict[str, Any]] = {}
    for row in rows:
        state = str(row["state"])
        state_summary[state] = state_summary.get(state, 0) + 1
        if state == "running" and row["locked_by"]:
            locked_by = str(row["locked_by"])
            worker = workers.setdefault(
                locked_by,
                {
                    "worker_id": locked_by,
                    "running_jobs": 0,
                    "oldest_locked_job": None,
                    "latest_locked_job": None,
                },
            )
            worker["running_jobs"] += 1
            if row["locked_at"] is not None:
                locked_at = str(row["locked_at"])
                if worker["oldest_locked_job"] is None or locked_at < worker["oldest_locked_job"]:
                    worker["oldest_locked_job"] = locked_at
                if worker["latest_locked_job"] is None or locked_at > worker["latest_locked_job"]:
                    worker["latest_locked_job"] = locked_at

    active_workers = list(workers.values())
    active_workers.sort(
        key=lambda item: (-int(item.get("running_jobs") or 0), str(item.get("worker_id", ""))),
    )

    recent_jobs = rm.list_jobs(ctx, repo_name=repo_name, limit=recent_jobs_limit)
    diagnostics = rm.job_diagnostics(ctx, repo_name=repo_name, limit=max(recent_jobs_limit, 100))
    return {
        "repo_name": repo["repo_name"],
        "snapshot_at": rows[0]["updated_at"] if rows else None,
        "state_summary": state_summary,
        "workers": active_workers,
        "worker_count": len(active_workers),
        "queued_jobs": state_summary.get("queued", 0),
        "running_jobs": state_summary.get("running", 0),
        "succeeded_jobs": state_summary.get("succeeded", 0),
        "failed_jobs": state_summary.get("failed", 0),
        "diagnostics": diagnostics,
        "recent_jobs": recent_jobs,
    }
