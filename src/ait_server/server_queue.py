from __future__ import annotations

import json
from datetime import datetime, timedelta, timezone
from typing import Any

from ait_protocol.common import utc_now
from .server_control import read as read_control, write as write_control
from .server_paths import ServerContext
from .store.repo_ops import _repo_id


class JobPayloadError(ValueError):
    """Raised when a known async job has an invalid payload contract."""


_ASYNC_JOB_SPECS: dict[str, dict[str, Any]] = {
    "repo.ci": {
        "required": {"repo_name": "str"},
        "optional": {
            "repo_id": ("str_or_none", None),
            "suite_ids": ("str_list_or_none", None),
            "plane": ("str_or_none", None),
            "target_line": ("str_or_none", None),
            "trigger": ("str_or_none", None),
            "selector": ("str_or_none", None),
            "task_ids": ("str_list_or_none", None),
            "curated_corpus": ("str_or_none", None),
            "count": ("positive_int_or_none", None),
            "window_days": ("positive_int_or_none", None),
            "dependency_evidence": ("str_list_or_none", None),
            "compliance_evidence": ("str_list_or_none", None),
        },
        "max_attempts": 3,
        "retry_delay_seconds": 3,
    },
    "patchset.ci": {
        "required": {"patchset_id": "str"},
        "optional": {
            "repo_name": ("str_or_none", None),
            "repo_id": ("str_or_none", None),
            "change_id": ("str_or_none", None),
            "change_seq": ("positive_int_or_none", None),
            "patchset_number": ("positive_int_or_none", None),
        },
        "max_attempts": 3,
        "retry_delay_seconds": 3,
    },
    "policy.evaluate": {
        "required": {"patchset_id": "str"},
        "optional": {
            "repo_name": ("str_or_none", None),
            "repo_id": ("str_or_none", None),
            "change_id": ("str_or_none", None),
            "change_seq": ("positive_int_or_none", None),
            "patchset_number": ("positive_int_or_none", None),
        },
        "max_attempts": 5,
        "retry_delay_seconds": 3,
    },
    "land.process": {
        "required": {"submission_id": "str"},
        "optional": {
            "repo_name": ("str_or_none", None),
            "repo_id": ("str_or_none", None),
            "change_id": ("str_or_none", None),
            "change_seq": ("positive_int_or_none", None),
            "patchset_id": ("str_or_none", None),
            "land_seq": ("positive_int_or_none", None),
        },
        "max_attempts": 5,
        "retry_delay_seconds": 3,
    },
    "reconcile.repo": {
        "required": {"repo_name": "str"},
        "optional": {"repair": ("bool", False)},
        "max_attempts": 3,
        "retry_delay_seconds": 3,
    },
    "content.pack": {
        "required": {"repo_name": "str"},
        "optional": {
            "repack": ("bool", False),
            "max_members": ("positive_int_or_none", None),
        },
        "max_attempts": 3,
        "retry_delay_seconds": 3,
    },
    "content.optimize": {
        "required": {"repo_name": "str"},
        "optional": {"repair": ("bool", True)},
        "max_attempts": 3,
        "retry_delay_seconds": 3,
    },
    "content.gc": {
        "required": {"repo_name": "str"},
        "optional": {
            "prune_unreferenced": ("bool", True),
            "prune_orphan_packs": ("bool", True),
        },
        "max_attempts": 3,
        "retry_delay_seconds": 3,
    },
}


def _read(ctx: ServerContext, callback):
    return read_control(ctx, callback)


def _write(ctx: ServerContext, callback):
    return write_control(ctx, callback)


def _repo_scope_predicate() -> str:
    return "(repo_id = ? or (repo_id is null and repo_name = ?))"


def _repo_scope_filter(ctx: ServerContext, repo_name: str | None) -> tuple[list[str], list[Any]]:
    if not repo_name:
        return [], []
    try:
        repo_id = _repo_id(ctx, repo_name)
    except KeyError:
        return ["repo_name = ?"], [repo_name]
    return [_repo_scope_predicate()], [repo_id, repo_name]


def _parse_ts(value: str) -> datetime:
    if value.endswith('Z'):
        value = value[:-1] + '+00:00'
    dt = datetime.fromisoformat(value)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(timezone.utc)


def _safe_parse_ts(value: Any) -> datetime | None:
    if value is None:
        return None
    text = str(value).strip()
    if not text:
        return None
    try:
        return _parse_ts(text)
    except (TypeError, ValueError):
        return None


def _count_by(rows: list[dict[str, Any]], key: str) -> dict[str, int]:
    counts: dict[str, int] = {}
    for row in rows:
        value = str(row.get(key) or "unknown")
        counts[value] = counts.get(value, 0) + 1
    return dict(sorted(counts.items()))


def supported_async_job_types() -> list[str]:
    return sorted(_ASYNC_JOB_SPECS)


def async_job_contract() -> list[dict[str, Any]]:
    contract: list[dict[str, Any]] = []
    for job_type in supported_async_job_types():
        spec = _ASYNC_JOB_SPECS[job_type]
        required = dict(spec["required"])
        optional = {name: {"type": item[0], "default": item[1]} for name, item in spec["optional"].items()}
        contract.append(
            {
                "job_type": job_type,
                "required": required,
                "optional": optional,
                "max_attempts": int(spec["max_attempts"]),
                "retry_delay_seconds": int(spec["retry_delay_seconds"]),
            }
        )
    return contract


def _job_spec(job_type: str) -> dict[str, Any]:
    spec = _ASYNC_JOB_SPECS.get(job_type)
    if spec is None:
        known = ", ".join(supported_async_job_types())
        raise JobPayloadError(f"Unsupported async job type: {job_type}. Expected one of: {known}")
    return spec


def _coerce_payload_value(job_type: str, field: str, kind: str, value: Any) -> Any:
    if kind == "str":
        text = str(value or "").strip()
        if not text:
            raise JobPayloadError(f"{job_type} requires non-empty payload field `{field}`.")
        return text
    if kind == "str_or_none":
        if value is None:
            return None
        text = str(value).strip()
        return text or None
    if kind == "str_list_or_none":
        if value is None:
            return None
        if isinstance(value, str):
            text = value.strip()
            return [text] if text else []
        if isinstance(value, (list, tuple)):
            normalized: list[str] = []
            for item in value:
                text = str(item or "").strip()
                if text:
                    normalized.append(text)
            return normalized
        raise JobPayloadError(f"{job_type} payload field `{field}` must be a list of strings or null.")
    if kind == "bool":
        if isinstance(value, bool):
            return value
        if isinstance(value, int) and value in {0, 1}:
            return bool(value)
        if isinstance(value, str):
            normalized = value.strip().lower()
            if normalized in {"1", "true", "yes", "on"}:
                return True
            if normalized in {"0", "false", "no", "off"}:
                return False
        raise JobPayloadError(f"{job_type} payload field `{field}` must be a boolean.")
    if kind == "positive_int_or_none":
        if value is None:
            return None
        if isinstance(value, bool):
            raise JobPayloadError(f"{job_type} payload field `{field}` must be a positive integer or null.")
        try:
            converted = int(value)
        except (TypeError, ValueError) as exc:
            raise JobPayloadError(f"{job_type} payload field `{field}` must be a positive integer or null.") from exc
        if converted <= 0:
            raise JobPayloadError(f"{job_type} payload field `{field}` must be greater than zero when set.")
        return converted
    if kind == "positive_int":
        if isinstance(value, bool):
            raise JobPayloadError(f"{job_type} payload field `{field}` must be a positive integer.")
        try:
            converted = int(value)
        except (TypeError, ValueError) as exc:
            raise JobPayloadError(f"{job_type} payload field `{field}` must be a positive integer.") from exc
        if converted <= 0:
            raise JobPayloadError(f"{job_type} payload field `{field}` must be greater than zero.")
        return converted
    raise JobPayloadError(f"{job_type} payload field `{field}` uses unsupported contract type `{kind}`.")


def normalize_async_job_payload(job_type: str, payload: dict[str, Any] | None) -> dict[str, Any]:
    if payload is None:
        payload = {}
    if not isinstance(payload, dict):
        raise JobPayloadError(f"{job_type} payload must be a JSON object.")

    spec = _job_spec(job_type)
    required: dict[str, str] = spec["required"]
    optional: dict[str, tuple[str, Any]] = spec["optional"]
    allowed_fields = set(required) | set(optional)
    extra_fields = sorted(set(payload) - allowed_fields)
    if extra_fields:
        raise JobPayloadError(f"{job_type} payload has unsupported field(s): {', '.join(extra_fields)}")

    normalized: dict[str, Any] = {}
    for field, kind in required.items():
        if field not in payload:
            raise JobPayloadError(f"{job_type} requires payload field `{field}`.")
        normalized[field] = _coerce_payload_value(job_type, field, kind, payload[field])
    for field, (kind, default) in optional.items():
        normalized[field] = _coerce_payload_value(job_type, field, kind, payload.get(field, default))
    return normalized


def retry_delay_seconds_for_job(job_type: str) -> int:
    try:
        return int(_job_spec(job_type)["retry_delay_seconds"])
    except JobPayloadError:
        return 3


def enqueue_async_job(
    ctx: ServerContext,
    repo_name: str,
    job_type: str,
    payload: dict[str, Any],
    *,
    available_at: str | None = None,
    max_attempts: int | None = None,
    dedupe_active: bool = True,
) -> dict[str, Any]:
    spec = _job_spec(job_type)
    normalized_payload = normalize_async_job_payload(job_type, payload)
    resolved_max_attempts = int(max_attempts if max_attempts is not None else spec["max_attempts"])
    return enqueue_job(
        ctx,
        repo_name,
        job_type,
        normalized_payload,
        available_at=available_at,
        max_attempts=resolved_max_attempts,
        dedupe_active=dedupe_active,
    )


def enqueue_job(
    ctx: ServerContext,
    repo_name: str,
    job_type: str,
    payload: dict[str, Any],
    *,
    available_at: str | None = None,
    max_attempts: int = 5,
    dedupe_active: bool = False,
) -> dict[str, Any]:
    now = utc_now()
    available = available_at or now
    payload_json = json.dumps(payload, sort_keys=True)
    try:
        repo_id = _repo_id(ctx, repo_name)
    except KeyError:
        repo_id = None

    def _enqueue(conn):
        if dedupe_active:
            if repo_id is None:
                active_filter = "repo_name = ?"
                active_filter_args = (repo_name,)
            else:
                active_filter = _repo_scope_predicate()
                active_filter_args = (repo_id, repo_name)
            active = conn.execute(
                """
                select *
                from jobs
                where """ + active_filter + """
                  and job_type = ?
                  and payload_json = ?
                  and state in ('queued', 'running')
                order by job_id desc
                limit 1
                """,
                (*active_filter_args, job_type, payload_json),
            ).fetchone()
            if active is not None:
                return _row_to_job(active)
        cur = conn.execute(
            """
            insert into jobs(
                repo_name, repo_id, job_type, state, payload_json, result_json,
                attempt_count, max_attempts, available_at,
                locked_at, locked_by, last_error, created_at, updated_at
            ) values (?, ?, ?, 'queued', ?, '{}', 0, ?, ?, null, null, null, ?, ?)
            """,
            (repo_name, repo_id, job_type, payload_json, max_attempts, available, now, now),
        )
        job_id = cur.lastrowid
        row = conn.execute("select * from jobs where job_id = ?", (job_id,)).fetchone()
        return _row_to_job(row)

    return _write(ctx, _enqueue)



def get_job(ctx: ServerContext, job_id: int) -> dict[str, Any]:
    row = _read(ctx, lambda conn: conn.execute("select * from jobs where job_id = ?", (job_id,)).fetchone())
    if row is None:
        raise KeyError(f"Unknown job: {job_id}")
    return _row_to_job(row)



def list_jobs(ctx: ServerContext, repo_name: str | None = None, state: str | None = None, limit: int = 100) -> list[dict[str, Any]]:
    where: list[str] = []
    params: list[Any] = []
    scope_where, scope_params = _repo_scope_filter(ctx, repo_name)
    where.extend(scope_where)
    params.extend(scope_params)
    if state:
        where.append("state = ?")
        params.append(state)
    sql = "select * from jobs"
    if where:
        sql += " where " + " and ".join(where)
    sql += " order by job_id desc limit ?"
    params.append(limit)
    rows = _read(ctx, lambda conn: conn.execute(sql, tuple(params)).fetchall())
    return [_row_to_job(row) for row in rows]


def reclaim_stale_jobs(ctx: ServerContext, stale_after_seconds: int, repo_name: str | None = None) -> dict[str, Any]:
    if stale_after_seconds <= 0:
        raise ValueError("stale_after_seconds must be greater than zero")

    now = utc_now()
    cutoff = (_parse_ts(now) - timedelta(seconds=stale_after_seconds)).isoformat()

    def _reclaim(conn):
        where = ["state = 'running'", "locked_at is not null", "locked_at <= ?"]
        params: list[Any] = [cutoff]
        scope_where, scope_params = _repo_scope_filter(ctx, repo_name)
        where.extend(scope_where)
        params.extend(scope_params)

        rows = conn.execute(
            f"select * from jobs where {' and '.join(where)} order by job_id asc",
            tuple(params),
        ).fetchall()

        requeued: list[int] = []
        failed: list[int] = []
        reclaimed_jobs: list[dict[str, Any]] = []
        job_type_summary: dict[str, int] = {}
        for row in rows:
            job_id = int(row["job_id"])
            attempt_count = int(row["attempt_count"])
            max_attempts = int(row["max_attempts"])
            job_type = str(row["job_type"])
            job_type_summary[job_type] = job_type_summary.get(job_type, 0) + 1
            if attempt_count >= max_attempts:
                state = "failed"
                failed.append(job_id)
                action = "failed"
                message = row["last_error"] or "Worker lease expired after max attempts"
            else:
                state = "queued"
                requeued.append(job_id)
                action = "requeued"
                message = row["last_error"] or "Worker lease expired; job returned to queue"
            reclaimed_jobs.append(
                {
                    "job_id": job_id,
                    "repo_name": row["repo_name"],
                    "job_type": job_type,
                    "previous_attempt_count": attempt_count,
                    "max_attempts": max_attempts,
                    "action": action,
                    "locked_by": row["locked_by"],
                    "locked_at": row["locked_at"],
                }
            )
            conn.execute(
                """
                update jobs
                set state = ?, available_at = ?, locked_at = null, locked_by = null, last_error = ?, updated_at = ?
                where job_id = ?
                """,
                (state, now, message, now, job_id),
            )
        return {
            "rows": rows,
            "requeued": requeued,
            "failed": failed,
            "reclaimed_jobs": reclaimed_jobs,
            "job_type_summary": dict(sorted(job_type_summary.items())),
        }

    payload = _write(ctx, _reclaim)
    return {
        "cutoff": cutoff,
        "stale_after_seconds": stale_after_seconds,
        "repo_name": repo_name,
        "stale_count": len(payload["rows"]),
        "requeued_job_ids": payload["requeued"],
        "failed_job_ids": payload["failed"],
        "reclaimed_jobs": payload["reclaimed_jobs"],
        "job_type_summary": payload["job_type_summary"],
        "recommended_action": "monitor_workers" if payload["requeued"] else ("inspect_failed" if payload["failed"] else "none"),
    }


def job_diagnostics(
    ctx: ServerContext,
    repo_name: str | None = None,
    *,
    stale_after_seconds: int = 300,
    limit: int = 100,
) -> dict[str, Any]:
    """Return operator-facing recovery diagnostics for async jobs."""
    if stale_after_seconds <= 0:
        raise ValueError("stale_after_seconds must be greater than zero")
    if limit < 0:
        raise ValueError("limit must be greater than or equal to zero")

    now = utc_now()
    now_dt = _parse_ts(now)
    cutoff_dt = now_dt - timedelta(seconds=stale_after_seconds)
    jobs = list_jobs(ctx, repo_name=repo_name, limit=limit)

    stale_jobs: list[dict[str, Any]] = []
    delayed_retry_jobs: list[dict[str, Any]] = []
    retryable_jobs: list[dict[str, Any]] = []
    exhausted_jobs: list[dict[str, Any]] = []
    failed_jobs: list[dict[str, Any]] = []

    for job in jobs:
        state = str(job.get("state") or "")
        locked_at = _safe_parse_ts(job.get("locked_at"))
        available_at = _safe_parse_ts(job.get("available_at"))
        attempts_remaining = int(job.get("attempts_remaining") or 0)
        last_error = job.get("last_error")
        if state == "running" and locked_at is not None and locked_at <= cutoff_dt:
            stale_jobs.append(job)
        if state in {"queued", "running"} and attempts_remaining > 0:
            retryable_jobs.append(job)
        if state == "queued" and last_error and attempts_remaining > 0 and available_at is not None and available_at > now_dt:
            delayed_retry_jobs.append(job)
        if state == "failed" and bool(job.get("attempts_exhausted")):
            exhausted_jobs.append(job)
        if state == "failed":
            failed_jobs.append(job)

    if stale_jobs:
        recommended_action = "reclaim_stale"
        reason = f"{len(stale_jobs)} running job(s) have stale worker locks."
    elif failed_jobs or exhausted_jobs:
        recommended_action = "inspect_failed"
        reason = f"{len(failed_jobs) or len(exhausted_jobs)} job(s) need failure inspection."
    elif delayed_retry_jobs:
        recommended_action = "wait_for_retry"
        reason = f"{len(delayed_retry_jobs)} job(s) are waiting for their retry window."
    elif any(str(job.get("state") or "") in {"queued", "running"} for job in jobs):
        recommended_action = "monitor_workers"
        reason = "Queue has active jobs but no recovery action is required yet."
    else:
        recommended_action = "none"
        reason = "No active or failed async jobs require operator action."

    stale_job_ids = [int(job["job_id"]) for job in stale_jobs]
    delayed_retry_job_ids = [int(job["job_id"]) for job in delayed_retry_jobs]
    retryable_job_ids = [int(job["job_id"]) for job in retryable_jobs]
    exhausted_job_ids = [int(job["job_id"]) for job in exhausted_jobs]
    failed_job_ids = [int(job["job_id"]) for job in failed_jobs]
    state_summary = _count_by(jobs, "state")
    job_type_summary = _count_by(jobs, "job_type")
    recovery_summary = {
        "stale_running_jobs": len(stale_jobs),
        "stale_job_ids": stale_job_ids,
        "retryable_jobs": len(retryable_jobs),
        "retryable_job_ids": retryable_job_ids,
        "delayed_retry_jobs": len(delayed_retry_jobs),
        "delayed_retry_job_ids": delayed_retry_job_ids,
        "exhausted_jobs": len(exhausted_jobs),
        "exhausted_job_ids": exhausted_job_ids,
        "failed_jobs": len(failed_jobs),
        "failed_job_ids": failed_job_ids,
    }
    return {
        "repo_name": repo_name,
        "snapshot_at": now,
        "limit": limit,
        "job_count": len(jobs),
        "stale_after_seconds": stale_after_seconds,
        "stale_cutoff": cutoff_dt.isoformat(),
        "state_summary": state_summary,
        "job_type_summary": job_type_summary,
        "recommended_action": recommended_action,
        "recommended_action_reason": reason,
        "recovery_summary": recovery_summary,
        **recovery_summary,
        "recent_jobs": jobs,
    }


def claim_next_job(ctx: ServerContext, worker_id: str, repo_name: str | None = None) -> dict[str, Any] | None:
    now = utc_now()
    if ctx.db_backend == "postgres":
        def _claim_postgres(conn):
            where = ["state = 'queued'", "available_at <= ?"]
            params: list[Any] = [now]
            scope_where, scope_params = _repo_scope_filter(ctx, repo_name)
            where.extend(scope_where)
            params.extend(scope_params)
            row = conn.execute(
                f"""
                with next_job as (
                    select job_id
                    from jobs
                    where {' and '.join(where)}
                    order by job_id asc
                    limit 1
                    for update skip locked
                )
                update jobs
                set state = 'running',
                    attempt_count = attempt_count + 1,
                    locked_at = ?,
                    locked_by = ?,
                    updated_at = ?
                where job_id in (select job_id from next_job)
                returning *
                """,
                tuple(params + [now, worker_id, now]),
            ).fetchone()
            if row is None:
                return None
            return _row_to_job(row)
        return _write(ctx, _claim_postgres)

    def _claim(conn):
        where = ["state = 'queued'", "available_at <= ?"]
        params = [now]
        scope_where, scope_params = _repo_scope_filter(ctx, repo_name)
        where.extend(scope_where)
        params.extend(scope_params)
        row = conn.execute(
            f"select * from jobs where {' and '.join(where)} order by job_id asc limit 1",
            tuple(params),
        ).fetchone()
        if row is None:
            return None
        job_id = row["job_id"]
        updated = conn.execute(
            """
            update jobs
            set state = 'running',
                attempt_count = attempt_count + 1,
                locked_at = ?,
                locked_by = ?,
                updated_at = ?
            where job_id = ? and state = 'queued'
            """,
            (now, worker_id, now, job_id),
        )
        if updated.rowcount != 1:
            conn.rollback()
            return None
        row = conn.execute("select * from jobs where job_id = ?", (job_id,)).fetchone()
        return _row_to_job(row)
    return _write(ctx, _claim)

def complete_job(ctx: ServerContext, job_id: int, result: dict[str, Any] | None = None) -> dict[str, Any]:
    now = utc_now()

    def _complete(conn):
        conn.execute(
            "update jobs set state = 'succeeded', result_json = ?, locked_at = null, locked_by = null, last_error = null, updated_at = ? where job_id = ?",
            (json.dumps(result or {}, sort_keys=True), now, job_id),
        )
        row = conn.execute("select * from jobs where job_id = ?", (job_id,)).fetchone()
        return row

    row = _write(ctx, _complete)
    if row is None:
        raise KeyError(f"Unknown job: {job_id}")
    return _row_to_job(row)



def fail_job(
    ctx: ServerContext,
    job_id: int,
    error: str,
    *,
    retryable: bool = True,
    retry_delay_seconds: int = 3,
) -> dict[str, Any]:
    now = utc_now()

    def _fail(conn):
        row = conn.execute("select attempt_count, max_attempts from jobs where job_id = ?", (job_id,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown job: {job_id}")
        attempt_count = int(row["attempt_count"])
        max_attempts = int(row["max_attempts"])
        if retryable and attempt_count < max_attempts:
            available_at = (_parse_ts(now) + timedelta(seconds=retry_delay_seconds)).isoformat()
            state = 'queued'
            locked_at = None
            locked_by = None
        else:
            available_at = now
            state = 'failed'
            locked_at = None
            locked_by = None
        conn.execute(
            """
            update jobs
            set state = ?, available_at = ?, locked_at = ?, locked_by = ?, last_error = ?, updated_at = ?
            where job_id = ?
            """,
            (state, available_at, locked_at, locked_by, error, now, job_id),
        )
        row = conn.execute("select * from jobs where job_id = ?", (job_id,)).fetchone()
        return _row_to_job(row)

    return _write(ctx, _fail)



def _row_to_job(row) -> dict[str, Any]:
    if row is None:
        raise KeyError("Unknown job")
    out = dict(row)
    out["payload"] = json.loads(out.get("payload_json") or "{}")
    out["result"] = json.loads(out.get("result_json") or "{}")

    attempt_count = int(out.get("attempt_count") or 0)
    max_attempts = int(out.get("max_attempts") or 0)
    attempts_remaining = max(max_attempts - attempt_count, 0)
    state = str(out.get("state") or "")
    last_error = out.get("last_error")
    retry_pending = state == "queued" and bool(last_error) and attempts_remaining > 0
    attempts_exhausted = max_attempts > 0 and attempt_count >= max_attempts

    out["attempt_count"] = attempt_count
    out["max_attempts"] = max_attempts
    out["attempts_remaining"] = attempts_remaining
    out["attempts_exhausted"] = attempts_exhausted
    out["retry_pending"] = retry_pending
    out["next_retry_at"] = out.get("available_at") if retry_pending else None
    if state == "running":
        out["diagnostic_status"] = "running"
    elif state == "queued" and retry_pending:
        out["diagnostic_status"] = "retry_pending"
    elif state == "failed" and attempts_exhausted:
        out["diagnostic_status"] = "exhausted_failed"
    elif state == "failed":
        out["diagnostic_status"] = "failed"
    else:
        out["diagnostic_status"] = state or "unknown"
    return out
