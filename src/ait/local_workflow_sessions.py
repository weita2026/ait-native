from __future__ import annotations

import json
from typing import Iterable

from ait_protocol.common import connect_sqlite, utc_now

from . import local_content
from .repo_paths import RepoContext


def _connect_control(ctx: RepoContext):
    return connect_sqlite(ctx.control_db_path)


def _resolve_workflow_task_id(conn, task_id: str) -> str:
    resolved = str(task_id or "").strip()
    if not resolved:
        raise KeyError("Task id is required.")
    row = conn.execute("select task_id from workflow_tasks where task_id = ?", (resolved,)).fetchone()
    if row is not None:
        return str(row["task_id"])
    row = conn.execute("select task_id from workflow_task_aliases where alias_task_id = ?", (resolved,)).fetchone()
    if row is not None:
        return str(row["task_id"])
    raise KeyError(f"Unknown local task: {task_id}")


def _resolve_workflow_change_id(conn, change_id: str) -> str:
    resolved = str(change_id or "").strip()
    if not resolved:
        raise KeyError("Change id is required.")
    row = conn.execute("select change_id from workflow_changes where change_id = ?", (resolved,)).fetchone()
    if row is not None:
        return str(row["change_id"])
    row = conn.execute("select change_id from workflow_change_aliases where alias_change_id = ?", (resolved,)).fetchone()
    if row is not None:
        return str(row["change_id"])
    raise KeyError(f"Unknown local change: {change_id}")


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


def create_workflow_session(
    ctx: RepoContext,
    session_id: str,
    repo_name: str,
    session_kind: str,
    *,
    task_id: str | None = None,
    change_id: str | None = None,
    title: str | None = None,
    line_name: str | None = None,
    worktree_name: str | None = None,
    model_name: str | None = None,
    actor_identity: str | None = None,
    actor_type: str | None = None,
    metadata: dict | None = None,
    status: str = "active",
) -> dict:
    conn = _connect_control(ctx)
    existing = conn.execute("select * from workflow_sessions where session_id = ?", (session_id,)).fetchone()
    if existing is not None:
        conn.close()
        raise ValueError(f"Local session {session_id} already exists")
    resolved_task_id = _resolve_workflow_task_id(conn, task_id) if task_id is not None else None
    resolved_change_id = _resolve_workflow_change_id(conn, change_id) if change_id is not None else None
    if change_id is not None:
        change = conn.execute("select task_id, repo_name from workflow_changes where change_id = ?", (resolved_change_id,)).fetchone()
        if change is None:
            conn.close()
            raise KeyError(f"Unknown local change: {change_id}")
        if change["repo_name"] != repo_name:
            conn.close()
            raise KeyError(f"Local change {change_id} belongs to repository {change['repo_name']}, not {repo_name}")
        if resolved_task_id is None:
            resolved_task_id = change["task_id"]
        elif resolved_task_id != change["task_id"]:
            conn.close()
            raise ValueError(f"Local change {change_id} belongs to task {change['task_id']}, not {resolved_task_id}")
    if resolved_task_id is not None:
        task = conn.execute("select repo_name from workflow_tasks where task_id = ?", (resolved_task_id,)).fetchone()
        if task is None:
            conn.close()
            raise KeyError(f"Unknown local task: {resolved_task_id}")
        if task["repo_name"] != repo_name:
            conn.close()
            raise KeyError(f"Local task {resolved_task_id} belongs to repository {task['repo_name']}, not {repo_name}")
    now = utc_now()
    conn.execute(
        """
        insert into workflow_sessions(
            session_id, repo_name, task_id, change_id, title, session_kind, status, line_name, worktree_name,
            model_name, actor_identity, actor_type, metadata_json, last_event_sequence, head_checkpoint_id, created_at, updated_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, null, ?, ?)
        """,
        (
            session_id,
            repo_name,
            resolved_task_id,
            resolved_change_id,
            title,
            session_kind,
            status,
            line_name,
            worktree_name,
            model_name,
            actor_identity,
            actor_type or "human",
            json.dumps(metadata or {}, sort_keys=True),
            now,
            now,
        ),
    )
    conn.commit()
    row = conn.execute("select * from workflow_sessions where session_id = ?", (session_id,)).fetchone()
    conn.close()
    assert row is not None
    return _session_row(row)


def list_workflow_sessions(ctx: RepoContext, *, status: str | None = None) -> list[dict]:
    conn = _connect_control(ctx)
    if status:
        rows = [
            dict(r)
            for r in conn.execute(
                "select * from workflow_sessions where status = ? order by updated_at desc, created_at desc, session_id desc",
                (status,),
            )
        ]
    else:
        rows = [
            dict(r)
            for r in conn.execute(
                "select * from workflow_sessions order by updated_at desc, created_at desc, session_id desc"
            )
        ]
    conn.close()
    return [_session_row(row) for row in rows]


def get_workflow_session(ctx: RepoContext, session_id: str) -> dict:
    conn = _connect_control(ctx)
    row = conn.execute("select * from workflow_sessions where session_id = ?", (session_id,)).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local session: {session_id}")
    return _session_row(row)


def append_workflow_session_event(
    ctx: RepoContext,
    session_id: str,
    event_type: str,
    payload: dict | None,
    *,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict:
    conn = _connect_control(ctx)
    row = conn.execute("select last_event_sequence, status from workflow_sessions where session_id = ?", (session_id,)).fetchone()
    if row is None:
        conn.close()
        raise KeyError(f"Unknown local session: {session_id}")
    if row["status"] != "active":
        conn.close()
        raise ValueError(f"Local session {session_id} is {row['status']} and cannot accept new events")
    next_sequence = int(row["last_event_sequence"] or 0) + 1
    now = utc_now()
    conn.execute(
        """
        insert into workflow_session_events(
            session_id, sequence, event_type, payload_json, actor_identity, actor_type, created_at
        ) values (?, ?, ?, ?, ?, ?, ?)
        """,
        (
            session_id,
            next_sequence,
            event_type,
            json.dumps(payload or {}, sort_keys=True),
            actor_identity or "local",
            actor_type or "human",
            now,
        ),
    )
    conn.execute(
        "update workflow_sessions set last_event_sequence = ?, updated_at = ? where session_id = ?",
        (next_sequence, now, session_id),
    )
    conn.commit()
    event_row = conn.execute(
        "select * from workflow_session_events where session_id = ? and sequence = ?",
        (session_id, next_sequence),
    ).fetchone()
    conn.close()
    assert event_row is not None
    return _session_event_row(event_row)


def list_workflow_session_events(
    ctx: RepoContext,
    session_id: str,
    *,
    after_sequence: int = 0,
    limit: int = 200,
) -> list[dict]:
    conn = _connect_control(ctx)
    if conn.execute("select 1 from workflow_sessions where session_id = ?", (session_id,)).fetchone() is None:
        conn.close()
        raise KeyError(f"Unknown local session: {session_id}")
    rows = [
        dict(r)
        for r in conn.execute(
            """
            select * from workflow_session_events
            where session_id = ? and sequence > ?
            order by sequence asc
            limit ?
            """,
            (session_id, max(int(after_sequence), 0), max(int(limit), 1)),
        )
    ]
    conn.close()
    return [_session_event_row(row) for row in rows]


def create_workflow_checkpoint(
    ctx: RepoContext,
    checkpoint_id: str,
    session_id: str,
    summary: str,
    *,
    snapshot_id: str | None = None,
    resume_payload: dict | None = None,
    based_on_sequence: int | None = None,
) -> dict:
    conn = _connect_control(ctx)
    session = conn.execute("select last_event_sequence, status from workflow_sessions where session_id = ?", (session_id,)).fetchone()
    if session is None:
        conn.close()
        raise KeyError(f"Unknown local session: {session_id}")
    if session["status"] not in {"active", "paused"}:
        conn.close()
        raise ValueError(f"Local session {session_id} is {session['status']} and cannot accept checkpoints")
    if snapshot_id is not None and not local_content.snapshot_exists(ctx, snapshot_id):
        conn.close()
        raise KeyError(f"Unknown snapshot: {snapshot_id}")
    effective_sequence = int(session["last_event_sequence"] or 0) if based_on_sequence is None else int(based_on_sequence)
    if effective_sequence < 0 or effective_sequence > int(session["last_event_sequence"] or 0):
        conn.close()
        raise ValueError(
            f"Checkpoint sequence {effective_sequence} is outside session {session_id} event range 0..{int(session['last_event_sequence'] or 0)}"
        )
    now = utc_now()
    conn.execute(
        """
        insert into workflow_checkpoints(
            checkpoint_id, session_id, based_on_sequence, summary, snapshot_id, resume_payload_json, created_at
        ) values (?, ?, ?, ?, ?, ?, ?)
        """,
        (
            checkpoint_id,
            session_id,
            effective_sequence,
            summary,
            snapshot_id,
            json.dumps(resume_payload or {}, sort_keys=True),
            now,
        ),
    )
    conn.execute(
        "update workflow_sessions set head_checkpoint_id = ?, updated_at = ? where session_id = ?",
        (checkpoint_id, now, session_id),
    )
    conn.commit()
    row = conn.execute("select * from workflow_checkpoints where checkpoint_id = ?", (checkpoint_id,)).fetchone()
    conn.close()
    assert row is not None
    return _checkpoint_row(row)


def list_workflow_checkpoints(ctx: RepoContext, session_id: str) -> list[dict]:
    conn = _connect_control(ctx)
    if conn.execute("select 1 from workflow_sessions where session_id = ?", (session_id,)).fetchone() is None:
        conn.close()
        raise KeyError(f"Unknown local session: {session_id}")
    rows = [
        dict(r)
        for r in conn.execute(
            "select * from workflow_checkpoints where session_id = ? order by created_at desc, checkpoint_id desc",
            (session_id,),
        )
    ]
    conn.close()
    return [_checkpoint_row(row) for row in rows]


def get_workflow_checkpoint(ctx: RepoContext, checkpoint_id: str) -> dict:
    conn = _connect_control(ctx)
    row = conn.execute("select * from workflow_checkpoints where checkpoint_id = ?", (checkpoint_id,)).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local checkpoint: {checkpoint_id}")
    return _checkpoint_row(row)


def record_workflow_snapshot_provenance(
    ctx: RepoContext,
    snapshot_id: str,
    *,
    task_id: str | None = None,
    change_id: str | None = None,
    session_id: str | None = None,
    checkpoint_id: str | None = None,
    worktree_name: str | None = None,
    line_name: str | None = None,
    author_mode: str | None = None,
    model_name: str | None = None,
    created_at: str | None = None,
) -> dict:
    conn = _connect_control(ctx)
    now = created_at or utc_now()
    conn.execute(
        """
        insert into workflow_snapshot_provenance(
            snapshot_id, task_id, change_id, session_id, checkpoint_id,
            worktree_name, line_name, author_mode, model_name, created_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        on conflict(snapshot_id) do update set
            task_id = excluded.task_id,
            change_id = excluded.change_id,
            session_id = excluded.session_id,
            checkpoint_id = excluded.checkpoint_id,
            worktree_name = excluded.worktree_name,
            line_name = excluded.line_name,
            author_mode = excluded.author_mode,
            model_name = excluded.model_name,
            created_at = excluded.created_at
        """,
        (
            snapshot_id,
            task_id,
            change_id,
            session_id,
            checkpoint_id,
            worktree_name,
            line_name,
            author_mode,
            model_name,
            now,
        ),
    )
    conn.commit()
    row = conn.execute(
        "select * from workflow_snapshot_provenance where snapshot_id = ?",
        (snapshot_id,),
    ).fetchone()
    conn.close()
    assert row is not None
    return dict(row)


def get_workflow_snapshot_provenance(ctx: RepoContext, snapshot_id: str) -> dict:
    conn = _connect_control(ctx)
    row = conn.execute(
        "select * from workflow_snapshot_provenance where snapshot_id = ?",
        (snapshot_id,),
    ).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local snapshot provenance: {snapshot_id}")
    return dict(row)


def list_workflow_snapshot_provenance(
    ctx: RepoContext,
    *,
    snapshot_ids: Iterable[str] | None = None,
) -> list[dict]:
    conn = _connect_control(ctx)
    rows: list[dict]
    if snapshot_ids is None:
        rows = [
            dict(row)
            for row in conn.execute(
                "select * from workflow_snapshot_provenance order by created_at desc, snapshot_id desc"
            ).fetchall()
        ]
        conn.close()
        return rows
    requested = [str(snapshot_id).strip() for snapshot_id in snapshot_ids if str(snapshot_id).strip()]
    if not requested:
        conn.close()
        return []
    placeholders = ", ".join("?" for _ in requested)
    rows = [
        dict(row)
        for row in conn.execute(
            f"""
            select * from workflow_snapshot_provenance
            where snapshot_id in ({placeholders})
            order by created_at desc, snapshot_id desc
            """,
            tuple(requested),
        ).fetchall()
    ]
    conn.close()
    return rows


def list_workflow_snapshot_provenance_for_change(
    ctx: RepoContext,
    change_id: str,
) -> list[dict]:
    resolved_change_id = str(change_id or "").strip()
    if not resolved_change_id:
        return []
    conn = _connect_control(ctx)
    rows = [
        dict(row)
        for row in conn.execute(
            """
            select * from workflow_snapshot_provenance
            where change_id = ?
            order by created_at desc, snapshot_id desc
            """,
            (resolved_change_id,),
        ).fetchall()
    ]
    conn.close()
    return rows


def resume_workflow_session(ctx: RepoContext, session_id: str, *, after_sequence: int | None = None, limit: int = 200) -> dict:
    conn = _connect_control(ctx)
    session_row = conn.execute("select * from workflow_sessions where session_id = ?", (session_id,)).fetchone()
    if session_row is None:
        conn.close()
        raise KeyError(f"Unknown local session: {session_id}")
    session = _session_row(session_row)
    if session["status"] in {"completed", "canceled"}:
        conn.close()
        raise ValueError(f"Local session {session_id} is {session['status']} and cannot be resumed")
    checkpoint_row = None
    if session.get("head_checkpoint_id"):
        checkpoint_row = conn.execute("select * from workflow_checkpoints where checkpoint_id = ?", (session["head_checkpoint_id"],)).fetchone()
    if checkpoint_row is None:
        checkpoint_row = conn.execute(
            "select * from workflow_checkpoints where session_id = ? order by created_at desc, checkpoint_id desc limit 1",
            (session_id,),
        ).fetchone()
    checkpoint = _checkpoint_row(checkpoint_row) if checkpoint_row is not None else None
    resume_sequence = max(int(after_sequence), 0) if after_sequence is not None else int(checkpoint["based_on_sequence"]) if checkpoint is not None else 0
    event_rows = [
        dict(r)
        for r in conn.execute(
            """
            select * from workflow_session_events
            where session_id = ? and sequence > ?
            order by sequence asc
            limit ?
            """,
            (session_id, resume_sequence, max(int(limit), 1)),
        )
    ]
    now = utc_now()
    if session["status"] == "paused":
        conn.execute("update workflow_sessions set status = 'active', updated_at = ? where session_id = ?", (now, session_id))
    else:
        conn.execute("update workflow_sessions set updated_at = ? where session_id = ?", (now, session_id))
    conn.commit()
    refreshed_row = conn.execute("select * from workflow_sessions where session_id = ?", (session_id,)).fetchone()
    conn.close()
    assert refreshed_row is not None
    return {
        "session": _session_row(refreshed_row),
        "latest_checkpoint": checkpoint,
        "resume_after_sequence": resume_sequence,
        "pending_events": [_session_event_row(row) for row in event_rows],
    }


def close_workflow_session(ctx: RepoContext, session_id: str, status: str) -> dict:
    if status not in {"paused", "completed", "canceled"}:
        raise ValueError(f"Unsupported local session close status: {status}")
    conn = _connect_control(ctx)
    row = conn.execute("select * from workflow_sessions where session_id = ?", (session_id,)).fetchone()
    if row is None:
        conn.close()
        raise KeyError(f"Unknown local session: {session_id}")
    session = dict(row)
    if session["status"] == status:
        conn.close()
        return _session_row(session)
    if session["status"] in {"completed", "canceled"}:
        conn.close()
        raise ValueError(f"Local session {session_id} is already {session['status']}; reopening is not supported")
    now = utc_now()
    conn.execute("update workflow_sessions set status = ?, updated_at = ? where session_id = ?", (status, now, session_id))
    row = conn.execute("select * from workflow_sessions where session_id = ?", (session_id,)).fetchone()
    conn.commit()
    conn.close()
    assert row is not None
    return _session_row(row)
