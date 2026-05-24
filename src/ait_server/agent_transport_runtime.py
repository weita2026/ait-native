"""Explicit server-local seam for `ait_agent` transport/runtime helpers."""

from __future__ import annotations

import os
import sys
from typing import Any, Callable, Mapping, TextIO


def build_transport_reply_envelope(*args: Any, **kwargs: Any) -> Any:
    from ait_agent.envelope import build_transport_reply_envelope as _build_transport_reply_envelope

    return _build_transport_reply_envelope(*args, **kwargs)


def load_config_for_telegram_worker(*args: Any, **kwargs: Any) -> Any:
    from ait_agent.telegram.worker_config import load_config_for_telegram_worker as _load_config_for_telegram_worker

    return _load_config_for_telegram_worker(*args, **kwargs)


def trigger_graph_watch_notifications(*args: Any, **kwargs: Any) -> Any:
    from ait_agent.telegram.graph_watches import trigger_graph_watch_notifications as _trigger_graph_watch_notifications

    return _trigger_graph_watch_notifications(*args, **kwargs)


def utc_now_iso(*args: Any, **kwargs: Any) -> Any:
    from ait_agent.runtime_bindings import utc_now_iso as _utc_now_iso

    return _utc_now_iso(*args, **kwargs)


def task_dag_telegram_trigger_enabled(*, env: Mapping[str, str] | None = None) -> bool:
    source_env = os.environ if env is None else env
    return str(source_env.get("AIT_TELEGRAM_GRAPH_TRIGGER_ENABLED", "true")).strip().lower() not in {
        "0",
        "false",
        "no",
        "off",
    }


def trigger_task_dag_telegram_notifications(
    ctx: Any,
    repo_name: str,
    *,
    event_type: str,
    entity_id: str,
    plan_ids: set[str] | None = None,
    resolve_repo_root: Callable[..., Any],
    progress_reader: Callable[[Any, dict[str, Any]], dict[str, Any]],
    env: Mapping[str, str] | None = None,
    stderr: TextIO | None = None,
) -> None:
    if not task_dag_telegram_trigger_enabled(env=env):
        return
    source_env = os.environ if env is None else env
    stream = sys.stderr if stderr is None else stderr
    try:
        repo_root = resolve_repo_root(repo_name=repo_name)
        config = load_config_for_telegram_worker(
            repo_root,
            name=source_env.get("AIT_TELEGRAM_GRAPH_TRIGGER_WORKER") or None,
        )
        summary = trigger_graph_watch_notifications(
            config,
            repo_name=repo_name,
            plan_ids=plan_ids,
            progress_reader=lambda graph: progress_reader(ctx, graph),
        )
        if summary.get("sent") or summary.get("errors"):
            print(
                "Task DAG Telegram trigger "
                f"event={event_type} entity={entity_id} repo={repo_name} "
                f"sent={summary.get('sent')} errors={summary.get('errors')} checked={summary.get('checked')}",
                file=stream,
                flush=True,
            )
    except Exception as exc:  # pragma: no cover - notification delivery must not break workflow writes
        print(
            f"Task DAG Telegram trigger failed for {event_type} {entity_id}: {type(exc).__name__}: {exc}",
            file=stream,
            flush=True,
        )


__all__ = [
    "build_transport_reply_envelope",
    "load_config_for_telegram_worker",
    "task_dag_telegram_trigger_enabled",
    "trigger_graph_watch_notifications",
    "trigger_task_dag_telegram_notifications",
    "utc_now_iso",
]
