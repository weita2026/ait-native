from __future__ import annotations

import os
import threading
import time
from pathlib import Path
from typing import Any, Callable, Mapping

from .server_db import postgres_statement_timeouts
from .task_dag_seams import (
    load_conversation_task_dag_graph,
    render_task_dag_conversation_progress,
    should_render_task_dag_progress,
)


def _schedule_task_dag_notification(
    trigger: Callable[..., None],
    *,
    ctx: Any,
    repo_name: str,
    event_type: str,
    entity_id: str,
) -> dict[str, Any]:
    started = time.monotonic()
    thread_name = f"ait-graph-trigger-{event_type}-{entity_id}".replace("/", "-")[:96]
    worker = threading.Thread(
        target=trigger,
        kwargs={
            "ctx": ctx,
            "repo_name": repo_name,
            "event_type": event_type,
            "entity_id": entity_id,
        },
        name=thread_name,
        daemon=True,
    )
    worker.start()
    return {
        "delivery": "background",
        "thread_name": thread_name,
        "scheduled_seconds": round(time.monotonic() - started, 6),
    }


def _attach_task_dag_notification_followup(
    result: dict[str, Any],
    trigger: Callable[..., None],
    *,
    ctx: Any,
    repo_name: str,
    event_type: str,
    entity_id: str,
) -> dict[str, Any]:
    result["notification_followup"] = _schedule_task_dag_notification(
        trigger,
        ctx=ctx,
        repo_name=repo_name,
        event_type=event_type,
        entity_id=entity_id,
    )
    return result


def _reply_text_with_task_dag_progress(text: str, summary: dict[str, Any] | None) -> str:
    reply_text = str(text or "").strip()
    if not isinstance(summary, dict):
        return reply_text
    progress_text = str(summary.get("text") or "").strip()
    if not progress_text:
        return reply_text
    if not reply_text:
        return progress_text
    if progress_text in reply_text:
        return reply_text
    return f"{reply_text}\n\n{progress_text}"


def _optional_timeout_ms(env_name: str, default_ms: int, *, env: Mapping[str, str] | None = None) -> int | None:
    source_env = os.environ if env is None else env
    raw = str(source_env.get(env_name) or "").strip()
    if not raw:
        return default_ms
    try:
        value = int(raw)
    except ValueError:
        return default_ms
    return max(value, 0) or None


def _task_dag_turn_progress_timeouts(
    default_lock_timeout_ms: int,
    default_statement_timeout_ms: int,
    *,
    env: Mapping[str, str] | None = None,
) -> tuple[int | None, int | None]:
    return (
        _optional_timeout_ms(
            "AIT_TASK_DAG_TURN_PROGRESS_LOCK_TIMEOUT_MS",
            default_lock_timeout_ms,
            env=env,
        ),
        _optional_timeout_ms(
            "AIT_TASK_DAG_TURN_PROGRESS_STATEMENT_TIMEOUT_MS",
            default_statement_timeout_ms,
            env=env,
        ),
    )


def _task_dag_progress_summary_for_turn_impl(
    ctx: Any,
    session: dict[str, Any],
    *,
    text: str,
    surface_title: str | None,
    resolve_repo_root: Callable[..., Path],
    task_dag_readiness_reader: Callable[..., dict[str, Any]],
) -> dict[str, Any] | None:
    repo_name = str(session.get("repo_name") or "").strip()
    if not should_render_task_dag_progress(text=text, session=session, surface_title=surface_title):
        return None
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    loaded = load_conversation_task_dag_graph(
        resolve_repo_root(session=session),
        repo_name=repo_name or None,
        task_graph_json=str(metadata.get("task_graph_json") or "").strip() or None,
        plan_id=str(metadata.get("plan_id") or "").strip() or None,
    )
    if loaded is None:
        return None
    graph, graph_path = loaded
    try:
        readiness = task_dag_readiness_reader(ctx, graph)
    except (KeyError, ValueError):
        return None
    summary = render_task_dag_conversation_progress(graph, readiness)
    summary["graph_artifact_path"] = str(graph_path)
    return summary


def _safe_task_dag_progress_summary_for_turn(
    ctx: Any,
    session: dict[str, Any],
    *,
    text: str,
    surface_title: str | None,
    default_lock_timeout_ms: int,
    default_statement_timeout_ms: int,
    progress_reader: Callable[..., dict[str, Any] | None],
) -> dict[str, Any] | None:
    lock_timeout_ms, statement_timeout_ms = _task_dag_turn_progress_timeouts(
        default_lock_timeout_ms,
        default_statement_timeout_ms,
    )
    with postgres_statement_timeouts(
        lock_timeout_ms=lock_timeout_ms,
        statement_timeout_ms=statement_timeout_ms,
    ):
        return progress_reader(
            ctx,
            session,
            text=text,
            surface_title=surface_title,
        )


__all__ = [
    "_attach_task_dag_notification_followup",
    "_reply_text_with_task_dag_progress",
    "_safe_task_dag_progress_summary_for_turn",
    "_schedule_task_dag_notification",
    "_task_dag_progress_summary_for_turn_impl",
    "_task_dag_turn_progress_timeouts",
]
