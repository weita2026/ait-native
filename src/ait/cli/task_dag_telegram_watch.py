from __future__ import annotations

import os
from typing import Any, Callable

from ait_protocol.common import normalize_optional_text

from ..remote_client import read_task_dag_progress as remote_read_task_dag_progress
from ..repo_paths import RepoContext
from ..store import (
    get_local_change,
    get_local_task,
    list_local_changes,
    list_local_sessions,
    list_local_tasks,
    load_config,
)
from ..task_dag_readiness import compute_task_graph_readiness
from .runtime_defaults import _effective_session_id, _normalize_text_value
from .task_dag_readiness_views import _task_dag_blocked_nodes, _task_dag_progress_payload
from .task_tracking_bindings import _tracked_session_binding
from .workflow_mode_config import _effective_workflow_mode


def _task_dag_telegram_auto_watch_enabled() -> bool:
    value = normalize_optional_text(os.environ.get("AIT_TASK_DAG_AUTO_WATCH_TELEGRAM"))
    if value is None:
        return True
    return value.lower() not in {"0", "false", "no", "off"}


def _task_dag_telegram_trigger_enabled() -> bool:
    value = normalize_optional_text(os.environ.get("AIT_TELEGRAM_GRAPH_TRIGGER_ENABLED"))
    if value is None:
        return True
    return value.lower() not in {"0", "false", "no", "off"}


def _task_dag_telegram_watch_session_hint(ctx: RepoContext) -> str | None:
    explicit = _effective_session_id()
    if explicit:
        return explicit
    binding = _tracked_session_binding(ctx)
    if binding is None:
        return None
    return _normalize_text_value(binding.get("session_id"))


def _task_dag_telegram_watch_repo_name(ctx: RepoContext, repo_name: str | None) -> str:
    resolved = str(repo_name or load_config(ctx).get("repo_name") or ctx.repo_root.name).strip()
    return resolved or ctx.repo_root.name


def _task_dag_telegram_watch_uses_local_progress(ctx: RepoContext) -> bool:
    workflow_mode = str((_effective_workflow_mode(ctx) or {}).get("value") or "").strip().lower()
    return workflow_mode == "solo_local"


def _task_dag_telegram_watch_should_fallback_to_local(exc: BaseException) -> bool:
    text = str(exc or "").strip().lower()
    return "unknown plan" in text or "404" in text


def _task_dag_telegram_watch_progress_reader(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any] | None,
) -> tuple[Callable[[dict[str, Any]], dict[str, Any]], str]:
    local_reader = lambda graph_payload: _local_task_dag_progress_payload(ctx, graph_payload)
    remote_url = str((remote_row or {}).get("url") or "").strip()
    if _task_dag_telegram_watch_uses_local_progress(ctx) or not remote_url:
        return local_reader, "local"

    def remote_then_local(graph_payload: dict[str, Any]) -> dict[str, Any]:
        try:
            return remote_read_task_dag_progress(remote_url, graph_payload)
        except Exception as exc:
            if _task_dag_telegram_watch_should_fallback_to_local(exc):
                return local_reader(graph_payload)
            raise

    return remote_then_local, "remote"


def _maybe_auto_register_task_dag_telegram_watch(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any] | None,
    repo_name: str | None,
    plan_id: str,
    graph_artifact_path: str,
) -> dict[str, Any]:
    if not _task_dag_telegram_auto_watch_enabled():
        return {
            "enabled": False,
            "registered": False,
            "reason": "disabled",
        }
    try:
        from ait_agent.telegram.graph_watches import auto_register_graph_watch
        from ait_agent.telegram.worker_config import load_config_for_telegram_worker
    except Exception as exc:
        return {
            "enabled": True,
            "registered": False,
            "reason": "telegram_runtime_unavailable",
            "detail": str(exc),
        }
    try:
        config = load_config_for_telegram_worker(ctx.root)
        session_hint = _task_dag_telegram_watch_session_hint(ctx)
        chat_hint = normalize_optional_text(os.environ.get("AIT_TELEGRAM_CHAT_ID"))
        resolved_repo_name = _task_dag_telegram_watch_repo_name(ctx, repo_name)
        progress_reader, progress_reader_mode = _task_dag_telegram_watch_progress_reader(
            ctx=ctx,
            remote_row=remote_row,
        )
        result = auto_register_graph_watch(
            config,
            plan_id=plan_id,
            graph_path=graph_artifact_path,
            progress_reader=progress_reader,
            repo_name=resolved_repo_name,
            linked_session_id=session_hint,
            chat_id=chat_hint,
        )
        return {
            "enabled": True,
            "graph_artifact_path": graph_artifact_path,
            "progress_reader_mode": progress_reader_mode,
            "repo_name": resolved_repo_name,
            "session_hint": session_hint,
            "chat_hint": chat_hint,
            **result,
        }
    except Exception as exc:
        return {
            "enabled": True,
            "registered": False,
            "reason": "auto_register_failed",
            "detail": str(exc),
        }


def _trigger_task_dag_execute_run_telegram_notifications(
    ctx: RepoContext,
    *,
    remote_row: dict[str, Any] | None,
    repo_name: str | None = None,
    plan_id: str,
) -> dict[str, Any]:
    if not _task_dag_telegram_trigger_enabled():
        return {
            "enabled": False,
            "checked": 0,
            "sent": 0,
            "errors": 0,
            "plan_id": plan_id,
            "reason": "disabled",
        }
    try:
        from ait_agent.telegram.graph_watches import trigger_graph_watch_notifications
        from ait_agent.telegram.worker_config import load_config_for_telegram_worker
    except Exception as exc:
        return {
            "enabled": True,
            "checked": 0,
            "sent": 0,
            "errors": 1,
            "plan_id": plan_id,
            "reason": "telegram_runtime_unavailable",
            "detail": str(exc),
        }
    resolved_repo_name = _task_dag_telegram_watch_repo_name(ctx, repo_name)
    try:
        config = load_config_for_telegram_worker(
            ctx.repo_root,
            name=os.environ.get("AIT_TELEGRAM_GRAPH_TRIGGER_WORKER") or None,
        )
        progress_reader, progress_reader_mode = _task_dag_telegram_watch_progress_reader(
            ctx=ctx,
            remote_row=remote_row,
        )
        summary = trigger_graph_watch_notifications(
            config,
            repo_name=resolved_repo_name,
            plan_ids={plan_id},
            progress_reader=progress_reader,
        )
        return {
            "enabled": True,
            "plan_id": plan_id,
            "progress_reader_mode": progress_reader_mode,
            "repo_name": resolved_repo_name,
            **summary,
        }
    except Exception as exc:
        return {
            "enabled": True,
            "checked": 0,
            "sent": 0,
            "errors": 1,
            "plan_id": plan_id,
            "repo_name": resolved_repo_name,
            "reason": "trigger_failed",
            "detail": f"{type(exc).__name__}: {exc}",
        }


def _local_task_dag_progress_payload(ctx: RepoContext, graph: dict[str, Any]) -> dict[str, Any]:
    sessions = list_local_sessions(ctx)
    workflow = {
        "tasks": list_local_tasks(ctx),
        "changes": list_local_changes(ctx),
        "sessions": sessions,
        "checkpoints": [],
    }
    readiness = compute_task_graph_readiness(graph, workflow)
    return {
        "schema_version": 1,
        "graph_id": graph.get("graph_id"),
        "source_plan": graph.get("source_plan") or {},
        "readiness_summary": readiness.get("summary") or {},
        "progress": _task_dag_progress_payload(graph, readiness),
        "blockers": [
            {
                "node_id": row.get("node_id"),
                "title": row.get("title"),
                "reason": row.get("reason"),
                "blockers": row.get("blockers") or [],
            }
            for row in _task_dag_blocked_nodes(readiness)
        ],
    }


def _local_task_dag_notification_plan_ids(
    ctx: RepoContext,
    *,
    event_type: str,
    entity_id: str,
) -> set[str] | None:
    if str(event_type or "").strip() != "change.local_landed":
        return None
    try:
        change = get_local_change(ctx, entity_id)
    except KeyError:
        return None
    task_id = _normalize_text_value(change.get("task_id"))
    if task_id is None:
        return None
    try:
        task = get_local_task(ctx, task_id)
    except KeyError:
        return None
    plan_id = _normalize_text_value(task.get("plan_id"))
    if plan_id is None:
        return None
    return {plan_id}


def trigger_local_task_dag_telegram_notifications(
    ctx: RepoContext,
    *,
    repo_name: str | None = None,
    event_type: str,
    entity_id: str,
) -> dict[str, Any]:
    if not _task_dag_telegram_trigger_enabled():
        return {
            "enabled": False,
            "checked": 0,
            "sent": 0,
            "errors": 0,
            "event_type": event_type,
            "entity_id": entity_id,
            "reason": "disabled",
        }
    try:
        from ait_agent.telegram.graph_watches import trigger_graph_watch_notifications
        from ait_agent.telegram.worker_config import load_config_for_telegram_worker
    except Exception as exc:
        return {
            "enabled": True,
            "checked": 0,
            "sent": 0,
            "errors": 1,
            "event_type": event_type,
            "entity_id": entity_id,
            "reason": "telegram_runtime_unavailable",
            "detail": str(exc),
        }
    resolved_repo_name = str(repo_name or load_config(ctx).get("repo_name") or ctx.repo_root.name).strip() or ctx.repo_root.name
    try:
        config = load_config_for_telegram_worker(
            ctx.repo_root,
            name=os.environ.get("AIT_TELEGRAM_GRAPH_TRIGGER_WORKER") or None,
        )
        plan_ids = _local_task_dag_notification_plan_ids(ctx, event_type=event_type, entity_id=entity_id)
        summary = trigger_graph_watch_notifications(
            config,
            repo_name=resolved_repo_name,
            plan_ids=plan_ids,
            progress_reader=lambda graph: _local_task_dag_progress_payload(ctx, graph),
        )
        return {
            "enabled": True,
            "event_type": event_type,
            "entity_id": entity_id,
            **summary,
        }
    except Exception as exc:
        return {
            "enabled": True,
            "checked": 0,
            "sent": 0,
            "errors": 1,
            "event_type": event_type,
            "entity_id": entity_id,
            "reason": "trigger_failed",
            "detail": f"{type(exc).__name__}: {exc}",
        }
