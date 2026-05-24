from __future__ import annotations

import json
from typing import Any

from ait_protocol.common import generate_namespaced_workflow_id, utc_now
from ait_protocol.task_statuses import TASK_CLOSED_STATUSES, task_close_session_status

from ..server_content import repository_exists
from ..server_control import connect, record_event
from ..server_paths import ServerContext
from .repo_ops import _repo_id, _repo_id_namespace_prefix


def _legacy_server_store_module():
    from .. import server_store as legacy_server_store

    return legacy_server_store


def _local_id_after_first_dash(*args, **kwargs):
    return _legacy_server_store_module()._local_id_after_first_dash(*args, **kwargs)


def _normalize_optional_text(*args, **kwargs):
    return _legacy_server_store_module()._normalize_optional_text(*args, **kwargs)


def _repo_scope_predicate(*args, **kwargs):
    return _legacy_server_store_module()._repo_scope_predicate(*args, **kwargs)


def _session_row(*args, **kwargs):
    return _legacy_server_store_module()._session_row(*args, **kwargs)


def get_task_for_repo(*args, **kwargs):
    return _legacy_server_store_module().get_task_for_repo(*args, **kwargs)


def _task_tracking_session_title(task: dict[str, Any]) -> str:
    task_id = _normalize_optional_text(task.get("task_id"))
    title = _normalize_optional_text(task.get("title"))
    if task_id and title:
        return f"{task_id}: {title}"
    return title or task_id or "Tracked task session"


def _task_tracking_session_metadata(
    task: dict[str, Any],
    *,
    metadata: dict[str, Any] | None = None,
    provisioned_by: str,
) -> dict[str, Any]:
    merged: dict[str, Any] = {}
    if isinstance(metadata, dict):
        merged.update(metadata)
    task_id = _normalize_optional_text(task.get("task_id"))
    task_intent = _normalize_optional_text(task.get("intent"))
    if task_id:
        merged.setdefault("task_id", task_id)
    if task_intent:
        merged.setdefault("objective", task_intent)
    merged.setdefault("tracking_policy", "server_task_session")
    merged.setdefault("provisioned_by", provisioned_by)
    return merged


def _task_tracking_session_payload(
    task: dict[str, Any],
    session: dict[str, Any],
    *,
    provisioned_by: str,
) -> dict[str, Any]:
    return {
        "mode": "server_guaranteed",
        "task_id": task.get("task_id"),
        "session_id": session.get("session_id"),
        "session_kind": session.get("session_kind") or "task_run",
        "session_status": session.get("status") or "active",
        "session_scope": "remote",
        "provisioned_by": provisioned_by,
    }


def _attach_task_tracking_payload(
    task: dict[str, Any],
    session: dict[str, Any],
    *,
    provisioned_by: str,
) -> dict[str, Any]:
    payload = dict(task)
    payload["tracking"] = _task_tracking_session_payload(task, session, provisioned_by=provisioned_by)
    return payload


def _list_task_tracking_sessions(conn, repo_id: str, repo_name: str, task_id: str) -> list[dict[str, Any]]:
    rows = [
        dict(r)
        for r in conn.execute(
            """
            select *
            from sessions
            where
                task_id = ?
                and session_kind = 'task_run'
                and (repo_id = ? or (repo_id is null and repo_name = ?))
            order by updated_at desc, created_at desc, session_id desc
            """,
            (task_id, repo_id, repo_name),
        )
    ]
    return rows


def _update_task_tracking_session_details(
    conn,
    session_row: dict[str, Any],
    *,
    title: str | None,
    line_name: str | None,
    worktree_name: str | None,
    model_name: str | None,
    metadata: dict[str, Any],
) -> dict[str, Any]:
    updates: dict[str, Any] = {}
    if title is not None and _normalize_optional_text(session_row.get("title")) != title:
        updates["title"] = title
    if line_name is not None and _normalize_optional_text(session_row.get("line_name")) != line_name:
        updates["line_name"] = line_name
    if worktree_name is not None and _normalize_optional_text(session_row.get("worktree_name")) != worktree_name:
        updates["worktree_name"] = worktree_name
    if model_name is not None and _normalize_optional_text(session_row.get("model_name")) != model_name:
        updates["model_name"] = model_name

    current_metadata = json.loads(session_row.get("metadata_json") or "{}")
    merged_metadata = dict(current_metadata)
    merged_metadata.update(metadata)
    if merged_metadata != current_metadata:
        updates["metadata_json"] = json.dumps(merged_metadata, sort_keys=True)

    if not updates:
        return _session_row(session_row)

    now = utc_now()
    updates["updated_at"] = now
    assignments = ", ".join(f"{column} = ?" for column in updates)
    conn.execute(
        f"update sessions set {assignments} where session_id = ?",
        (*updates.values(), session_row["session_id"]),
    )
    refreshed = conn.execute("select * from sessions where session_id = ?", (session_row["session_id"],)).fetchone()
    assert refreshed is not None
    return _session_row(refreshed)


def _close_task_tracking_session_if_needed(
    conn,
    session_row: dict[str, Any],
    *,
    task_status: str,
    actor_identity: str,
    actor_type: str,
) -> dict[str, Any]:
    if "metadata_json" not in session_row:
        session_payload = dict(session_row)
    else:
        session_payload = _session_row(session_row)
    session_close_status = task_close_session_status(task_status)
    if session_close_status is None:
        return session_payload
    current_status = _normalize_optional_text(session_row.get("status")) or "active"
    if current_status == session_close_status:
        return session_payload
    if current_status in {"completed", "canceled"}:
        return session_payload
    now = utc_now()
    conn.execute(
        "update sessions set status = ?, updated_at = ? where session_id = ?",
        (session_close_status, now, session_row["session_id"]),
    )
    record_event(
        conn,
        "session.closed",
        "session",
        session_row["session_id"],
        {"status": session_close_status, "previous_status": current_status, "task_status": task_status},
        actor_identity=actor_identity,
        actor_type=actor_type,
    )
    refreshed = conn.execute("select * from sessions where session_id = ?", (session_row["session_id"],)).fetchone()
    assert refreshed is not None
    return _session_row(refreshed)


def _ensure_task_tracking_session_conn(
    ctx: ServerContext,
    conn,
    task: dict[str, Any],
    *,
    tracking_session: dict[str, Any] | None = None,
    actor_identity: str,
    actor_type: str,
    provisioned_by: str,
) -> dict[str, Any]:
    task_id = _normalize_optional_text(task.get("task_id"))
    repo_name = _normalize_optional_text(task.get("repo_name"))
    repo_id = _normalize_optional_text(task.get("repo_id"))
    if task_id is None or repo_name is None:
        raise ValueError("Task tracking sessions require repo_name and task_id.")
    if repo_id is None:
        repo_id = _repo_id(ctx, repo_name)
    tracking_session = tracking_session if isinstance(tracking_session, dict) else {}
    title = _task_tracking_session_title(task)
    line_name = _normalize_optional_text(tracking_session.get("line_name"))
    worktree_name = _normalize_optional_text(tracking_session.get("worktree_name"))
    model_name = _normalize_optional_text(tracking_session.get("model_name"))
    metadata = _task_tracking_session_metadata(
        task,
        metadata=tracking_session.get("metadata") if isinstance(tracking_session.get("metadata"), dict) else None,
        provisioned_by=provisioned_by,
    )
    task_status = _normalize_optional_text(task.get("status")) or "active"

    existing_rows = _list_task_tracking_sessions(conn, repo_id, repo_name, task_id)
    reusable_existing = next(
        (
            row
            for row in existing_rows
            if (_normalize_optional_text(row.get("status")) or "active") in {"active", "paused"}
        ),
        None,
    )
    should_reuse_existing = reusable_existing is not None or (task_status in TASK_CLOSED_STATUSES and bool(existing_rows))
    if should_reuse_existing:
        session = _update_task_tracking_session_details(
            conn,
            reusable_existing or existing_rows[0],
            title=title,
            line_name=line_name,
            worktree_name=worktree_name,
            model_name=model_name,
            metadata=metadata,
        )
    else:
        session_id = generate_namespaced_workflow_id("S", _repo_id_namespace_prefix(ctx, repo_name))
        session_local_id = _local_id_after_first_dash(session_id)
        now = utc_now()
        conn.execute(
            """
            insert into sessions(
                session_id, repo_name, repo_id, session_local_id, task_id, change_id, title, session_kind, status,
                line_name, worktree_name, model_name, actor_identity, actor_type, metadata_json,
                last_event_sequence, head_checkpoint_id, created_at, updated_at
            ) values (?, ?, ?, ?, ?, null, ?, 'task_run', 'active', ?, ?, ?, ?, ?, ?, 0, null, ?, ?)
            """,
            (
                session_id,
                repo_name,
                repo_id,
                session_local_id,
                task_id,
                title,
                line_name,
                worktree_name,
                model_name,
                actor_identity,
                actor_type,
                json.dumps(metadata, sort_keys=True),
                now,
                now,
            ),
        )
        record_event(
            conn,
            "session.created",
            "session",
            session_id,
            {
                "repo_name": repo_name,
                "session_kind": "task_run",
                "task_id": task_id,
                "change_id": None,
                "line_name": line_name,
            },
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        row = conn.execute("select * from sessions where session_id = ?", (session_id,)).fetchone()
        assert row is not None
        session = _session_row(row)

    return _close_task_tracking_session_if_needed(
        conn,
        session,
        task_status=task_status,
        actor_identity=actor_identity,
        actor_type=actor_type,
    )


def ensure_task_tracking_session(
    ctx: ServerContext,
    repo_name: str,
    task_ref: str,
    *,
    tracking_session: dict[str, Any] | None = None,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
    provisioned_by: str = "task.ensure_tracking_session",
) -> dict[str, Any]:
    task = get_task_for_repo(ctx, repo_name, task_ref)
    with connect(ctx) as conn:
        session = _ensure_task_tracking_session_conn(
            ctx,
            conn,
            task,
            tracking_session=tracking_session,
            actor_identity=actor_identity,
            actor_type=actor_type,
            provisioned_by=provisioned_by,
        )
        conn.commit()
    return session


def backfill_task_tracking_sessions(
    ctx: ServerContext,
    repo_name: str,
    *,
    task_ref: str | None = None,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
) -> dict[str, Any]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    created: list[dict[str, Any]] = []
    scanned_count = 0
    missing_before_count = 0
    with connect(ctx) as conn:
        if task_ref is not None:
            task_rows = [get_task_for_repo(ctx, repo_name, task_ref)]
        else:
            task_rows = [
                dict(r)
                for r in conn.execute(
                    "select * from tasks where " + _repo_scope_predicate() + " order by created_at asc, task_id asc",
                    (_repo_id(ctx, repo_name), repo_name),
                )
            ]
        for task in task_rows:
            scanned_count += 1
            existing_rows = _list_task_tracking_sessions(
                conn,
                str(task.get("repo_id") or _repo_id(ctx, repo_name)),
                repo_name,
                str(task["task_id"]),
            )
            if not existing_rows:
                missing_before_count += 1
            session = _ensure_task_tracking_session_conn(
                ctx,
                conn,
                task,
                tracking_session=None,
                actor_identity=actor_identity,
                actor_type=actor_type,
                provisioned_by="task.backfill_tracking_sessions",
            )
            if not existing_rows:
                created.append(
                    {
                        "task_id": task["task_id"],
                        "task_status": task["status"],
                        "session_id": session["session_id"],
                        "session_status": session["status"],
                    }
                )
        conn.commit()
    return {
        "repo_name": repo_name,
        "task_ref": task_ref,
        "scanned_task_count": scanned_count,
        "missing_task_count": missing_before_count,
        "created_session_count": len(created),
        "created_sessions": created,
    }
