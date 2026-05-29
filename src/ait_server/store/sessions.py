from __future__ import annotations

import json
from typing import Any

from ait_protocol.common import generate_namespaced_workflow_id, utc_now

from ..server_content import get_snapshot_repo
from ..server_content_repo_lines import repository_exists
from ..server_control import connect, record_event
from ..server_paths import ServerContext
from .repo_scoped_keys import _local_id_after_first_dash, _repo_scope_predicate
from .repo_ops import _repo_id, _repo_id_namespace_prefix


def _session_local_reference(value: str | None) -> str | None:
    normalized = str(value or "").strip()
    if not normalized:
        return None
    candidate = _local_id_after_first_dash(normalized)
    return candidate or None


def _session_row(row) -> dict:
    out = dict(row)
    out["metadata"] = json.loads(out.pop("metadata_json") or "{}")
    return out


def _session_event_row(row) -> dict:
    out = dict(row)
    out["payload"] = json.loads(out.pop("payload_json") or "{}")
    return out


def _checkpoint_row(row) -> dict:
    out = dict(row)
    out["resume_payload"] = json.loads(out.pop("resume_payload_json") or "{}")
    return out


def create_session(
    ctx: ServerContext,
    repo_name: str,
    session_kind: str,
    *,
    task_id: str | None = None,
    change_id: str | None = None,
    title: str | None = None,
    line_name: str | None = None,
    worktree_name: str | None = None,
    model_name: str | None = None,
    metadata: dict | None = None,
    session_id: str | None = None,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
) -> dict:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        resolved_task_id = task_id
        if change_id is not None:
            change = conn.execute("select task_id, repo_name from changes where change_id = ?", (change_id,)).fetchone()
            if change is None:
                raise KeyError(f"Unknown change: {change_id}")
            if change["repo_name"] != repo_name:
                raise KeyError(f"Change {change_id} belongs to repository {change['repo_name']}, not {repo_name}")
            if resolved_task_id is None:
                resolved_task_id = change["task_id"]
            elif resolved_task_id != change["task_id"]:
                raise ValueError(f"Change {change_id} belongs to task {change['task_id']}, not {resolved_task_id}")
        if resolved_task_id is not None:
            task = conn.execute("select repo_name from tasks where task_id = ?", (resolved_task_id,)).fetchone()
            if task is None:
                raise KeyError(f"Unknown task: {resolved_task_id}")
            if task["repo_name"] != repo_name:
                raise KeyError(f"Task {resolved_task_id} belongs to repository {task['repo_name']}, not {repo_name}")
        if session_id is None:
            session_id = generate_namespaced_workflow_id("S", _repo_id_namespace_prefix(ctx, repo_name))
        session_local_id = _local_id_after_first_dash(session_id)
        existing = conn.execute("select * from sessions where session_id = ?", (session_id,)).fetchone()
        if existing is not None:
            row = dict(existing)
            if (
                row["repo_name"] == repo_name
                and row["task_id"] == resolved_task_id
                and row["change_id"] == change_id
                and row["title"] == title
                and row["session_kind"] == session_kind
                and row["line_name"] == line_name
                and row["worktree_name"] == worktree_name
                and row["model_name"] == model_name
                and json.loads(row["metadata_json"] or "{}") == (metadata or {})
            ):
                return _session_row(row)
            raise ValueError(f"Session {session_id} already exists with different fields")
        now = utc_now()
        conn.execute(
            """
            insert into sessions(
                session_id, repo_name, repo_id, session_local_id, task_id, change_id, title, session_kind, status, line_name, worktree_name,
                model_name, actor_identity, actor_type, metadata_json, last_event_sequence, head_checkpoint_id, created_at, updated_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?, ?, ?, 0, null, ?, ?)
            """,
            (
                session_id,
                repo_name,
                repo_id,
                session_local_id,
                resolved_task_id,
                change_id,
                title,
                session_kind,
                line_name,
                worktree_name,
                model_name,
                actor_identity,
                actor_type,
                json.dumps(metadata or {}, sort_keys=True),
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
                "session_kind": session_kind,
                "task_id": resolved_task_id,
                "change_id": change_id,
                "line_name": line_name,
            },
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        conn.commit()
        row = conn.execute("select * from sessions where session_id = ?", (session_id,)).fetchone()
    assert row is not None
    return _session_row(row)


def list_sessions(ctx: ServerContext, repo_name: str, *, status: str | None = None) -> list[dict]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        if status:
            rows = [
                dict(r)
                for r in conn.execute(
                    "select * from sessions where " + _repo_scope_predicate() + " and status = ? order by updated_at desc, created_at desc",
                    (repo_id, repo_name, status),
                )
            ]
        else:
            rows = [
                dict(r)
                for r in conn.execute(
                    "select * from sessions where " + _repo_scope_predicate() + " order by updated_at desc, created_at desc",
                    (repo_id, repo_name),
                )
            ]
    return [_session_row(row) for row in rows]


def get_session(ctx: ServerContext, session_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select * from sessions where session_id = ?", (session_id,)).fetchone()
    if row is None:
        raise KeyError(f"Unknown session: {session_id}")
    return _session_row(row)


def get_session_for_repo(ctx: ServerContext, repo_name: str, session_ref: str) -> dict:
    repo_id = _repo_id(ctx, repo_name)
    normalized_ref = str(session_ref or "").strip()
    with connect(ctx) as conn:
        row = conn.execute(
            "select * from sessions where " + _repo_scope_predicate() + " and session_id = ?",
            (repo_id, repo_name, normalized_ref),
        ).fetchone()
        if row is None:
            local_ref = _session_local_reference(normalized_ref)
            if local_ref is not None:
                row = conn.execute(
                    "select * from sessions where " + _repo_scope_predicate() + " and session_local_id = ?",
                    (repo_id, repo_name, local_ref),
                ).fetchone()
    if row is None:
        raise KeyError(f"Unknown session {session_ref} for repository {repo_name}")
    return _session_row(row)


def append_session_event(
    ctx: ServerContext,
    session_id: str,
    event_type: str,
    payload: dict | None,
    *,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select repo_name, repo_id, last_event_sequence, status from sessions where session_id = ?", (session_id,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown session: {session_id}")
        if row["status"] != "active":
            raise ValueError(f"Session {session_id} is {row['status']} and cannot accept new events")
        next_sequence = int(row["last_event_sequence"] or 0) + 1
        now = utc_now()
        conn.execute(
            """
            insert into session_events(
                repo_id, session_id, sequence, event_type, payload_json, actor_identity, actor_type, created_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                row["repo_id"] or _repo_id(ctx, row["repo_name"]),
                session_id,
                next_sequence,
                event_type,
                json.dumps(payload or {}, sort_keys=True),
                actor_identity,
                actor_type,
                now,
            ),
        )
        conn.execute(
            "update sessions set last_event_sequence = ?, updated_at = ? where session_id = ?",
            (next_sequence, now, session_id),
        )
        record_event(
            conn,
            "session.event_appended",
            "session",
            session_id,
            {"sequence": next_sequence, "event_type": event_type},
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        conn.commit()
        event_row = conn.execute(
            "select * from session_events where session_id = ? and sequence = ?",
            (session_id, next_sequence),
        ).fetchone()
    assert event_row is not None
    return _session_event_row(event_row)


def list_session_events(
    ctx: ServerContext,
    session_id: str,
    *,
    after_sequence: int = 0,
    limit: int = 200,
) -> list[dict]:
    with connect(ctx) as conn:
        if conn.execute("select 1 from sessions where session_id = ?", (session_id,)).fetchone() is None:
            raise KeyError(f"Unknown session: {session_id}")
        rows = [
            dict(r)
            for r in conn.execute(
                """
                select * from session_events
                where session_id = ? and sequence > ?
                order by sequence asc
                limit ?
                """,
                (session_id, max(int(after_sequence), 0), max(int(limit), 1)),
            )
        ]
    return [_session_event_row(row) for row in rows]


def create_session_checkpoint(
    ctx: ServerContext,
    session_id: str,
    summary: str,
    *,
    snapshot_id: str | None = None,
    resume_payload: dict | None = None,
    based_on_sequence: int | None = None,
    checkpoint_id: str | None = None,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
) -> dict:
    with connect(ctx) as conn:
        session = conn.execute("select repo_name, repo_id, last_event_sequence, status from sessions where session_id = ?", (session_id,)).fetchone()
        if session is None:
            raise KeyError(f"Unknown session: {session_id}")
        if session["status"] not in {"active", "paused"}:
            raise ValueError(f"Session {session_id} is {session['status']} and cannot accept checkpoints")
        if snapshot_id is not None:
            snapshot_repo = get_snapshot_repo(ctx, snapshot_id)
            if snapshot_repo is None:
                raise KeyError(f"Unknown snapshot: {snapshot_id}")
            if snapshot_repo != session["repo_name"]:
                raise KeyError(f"Snapshot {snapshot_id} belongs to repository {snapshot_repo}, not {session['repo_name']}")
        effective_sequence = int(session["last_event_sequence"] or 0) if based_on_sequence is None else int(based_on_sequence)
        if effective_sequence < 0 or effective_sequence > int(session["last_event_sequence"] or 0):
            raise ValueError(
                f"Checkpoint sequence {effective_sequence} is outside session {session_id} event range 0..{int(session['last_event_sequence'] or 0)}"
            )
        if checkpoint_id is None:
            checkpoint_id = generate_namespaced_workflow_id("K", _repo_id_namespace_prefix(ctx, session["repo_name"]))
        checkpoint_local_id = _local_id_after_first_dash(checkpoint_id)
        now = utc_now()
        conn.execute(
            """
            insert into session_checkpoints(
                checkpoint_id, repo_id, checkpoint_local_id, session_id, based_on_sequence, summary, snapshot_id, resume_payload_json, created_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                checkpoint_id,
                session["repo_id"] or _repo_id(ctx, session["repo_name"]),
                checkpoint_local_id,
                session_id,
                effective_sequence,
                summary,
                snapshot_id,
                json.dumps(resume_payload or {}, sort_keys=True),
                now,
            ),
        )
        conn.execute(
            "update sessions set head_checkpoint_id = ?, updated_at = ? where session_id = ?",
            (checkpoint_id, now, session_id),
        )
        record_event(
            conn,
            "session.checkpoint_created",
            "checkpoint",
            checkpoint_id,
            {
                "session_id": session_id,
                "based_on_sequence": effective_sequence,
                "snapshot_id": snapshot_id,
            },
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        conn.commit()
        row = conn.execute("select * from session_checkpoints where checkpoint_id = ?", (checkpoint_id,)).fetchone()
    assert row is not None
    return _checkpoint_row(row)


def list_session_checkpoints(ctx: ServerContext, session_id: str) -> list[dict]:
    with connect(ctx) as conn:
        if conn.execute("select 1 from sessions where session_id = ?", (session_id,)).fetchone() is None:
            raise KeyError(f"Unknown session: {session_id}")
        rows = [
            dict(r)
            for r in conn.execute(
                "select * from session_checkpoints where session_id = ? order by created_at desc, checkpoint_id desc",
                (session_id,),
            )
        ]
    return [_checkpoint_row(row) for row in rows]


def get_session_checkpoint(ctx: ServerContext, checkpoint_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select * from session_checkpoints where checkpoint_id = ?", (checkpoint_id,)).fetchone()
    if row is None:
        raise KeyError(f"Unknown checkpoint: {checkpoint_id}")
    return _checkpoint_row(row)


def get_session_checkpoint_for_repo(ctx: ServerContext, repo_name: str, checkpoint_ref: str) -> dict:
    repo_id = _repo_id(ctx, repo_name)
    normalized_ref = str(checkpoint_ref or "").strip()
    with connect(ctx) as conn:
        row = conn.execute(
            """
            select c.*
            from session_checkpoints c
            join sessions s on s.session_id = c.session_id
            where """
            + _repo_scope_predicate(alias="s")
            + " and c.checkpoint_id = ?",
            (repo_id, repo_name, normalized_ref),
        ).fetchone()
        if row is None:
            local_ref = _session_local_reference(normalized_ref)
            if local_ref is not None:
                row = conn.execute(
                    """
                    select c.*
                    from session_checkpoints c
                    join sessions s on s.session_id = c.session_id
                    where """
                    + _repo_scope_predicate(alias="s")
                    + " and c.checkpoint_local_id = ?",
                    (repo_id, repo_name, local_ref),
                ).fetchone()
    if row is None:
        raise KeyError(f"Unknown checkpoint {checkpoint_ref} for repository {repo_name}")
    return _checkpoint_row(row)


def resume_session(
    ctx: ServerContext,
    session_id: str,
    *,
    after_sequence: int | None = None,
    limit: int = 200,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
) -> dict:
    with connect(ctx) as conn:
        session_row = conn.execute("select * from sessions where session_id = ?", (session_id,)).fetchone()
        if session_row is None:
            raise KeyError(f"Unknown session: {session_id}")
        session = _session_row(session_row)
        if session["status"] in {"completed", "canceled"}:
            raise ValueError(f"Session {session_id} is {session['status']} and cannot be resumed")
        checkpoint_row = None
        if session.get("head_checkpoint_id"):
            checkpoint_row = conn.execute("select * from session_checkpoints where checkpoint_id = ?", (session["head_checkpoint_id"],)).fetchone()
        if checkpoint_row is None:
            checkpoint_row = conn.execute(
                "select * from session_checkpoints where session_id = ? order by created_at desc, checkpoint_id desc limit 1",
                (session_id,),
            ).fetchone()
        checkpoint = _checkpoint_row(checkpoint_row) if checkpoint_row is not None else None
        resume_sequence = max(int(after_sequence), 0) if after_sequence is not None else int(checkpoint["based_on_sequence"]) if checkpoint is not None else 0
        event_rows = [
            dict(r)
            for r in conn.execute(
                """
                select * from session_events
                where session_id = ? and sequence > ?
                order by sequence asc
                limit ?
                """,
                (session_id, resume_sequence, max(int(limit), 1)),
            )
        ]
        now = utc_now()
        if session["status"] == "paused":
            conn.execute("update sessions set status = 'active', updated_at = ? where session_id = ?", (now, session_id))
        else:
            conn.execute("update sessions set updated_at = ? where session_id = ?", (now, session_id))
        record_event(
            conn,
            "session.resumed",
            "session",
            session_id,
            {
                "resume_after_sequence": resume_sequence,
                "latest_checkpoint_id": checkpoint["checkpoint_id"] if checkpoint is not None else None,
                "pending_event_count": len(event_rows),
            },
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        conn.commit()
        refreshed_row = conn.execute("select * from sessions where session_id = ?", (session_id,)).fetchone()
    assert refreshed_row is not None
    return {
        "session": _session_row(refreshed_row),
        "latest_checkpoint": checkpoint,
        "resume_after_sequence": resume_sequence,
        "pending_events": [_session_event_row(row) for row in event_rows],
    }


def close_session(
    ctx: ServerContext,
    session_id: str,
    status: str,
    *,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
) -> dict:
    if status not in {"paused", "completed", "canceled"}:
        raise ValueError(f"Unsupported session close status: {status}")
    with connect(ctx) as conn:
        row = conn.execute("select * from sessions where session_id = ?", (session_id,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown session: {session_id}")
        session = dict(row)
        if session["status"] == status:
            return _session_row(session)
        if session["status"] in {"completed", "canceled"}:
            raise ValueError(f"Session {session_id} is already {session['status']}; reopening is not supported")
        now = utc_now()
        conn.execute("update sessions set status = ?, updated_at = ? where session_id = ?", (status, now, session_id))
        record_event(
            conn,
            "session.closed",
            "session",
            session_id,
            {"status": status, "previous_status": session["status"]},
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        conn.commit()
        row = conn.execute("select * from sessions where session_id = ?", (session_id,)).fetchone()
    assert row is not None
    return _session_row(row)
