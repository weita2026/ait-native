from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

from ..remote_client import (
    RemoteError,
    advance_task_dag_run as remote_advance_task_dag_run,
    append_session_event as remote_append_session_event,
    create_session as remote_create_session,
    get_session as remote_get_session,
    list_session_events as remote_list_session_events,
)
from ..store import RepoContext, current_line, load_config
from .remote_ci_readiness_helpers import _remote_error_status_code, _remote_read_task_dag_readiness
from .runtime_defaults import _normalize_text_value
from .task_dag_execute_run_state import (
    _task_dag_execute_run_summary,
    _task_dag_execute_state_digest,
    _task_dag_execute_state_from_snapshot,
    _task_dag_graph_run_id,
    _task_dag_state_snapshot_payload,
)
from .task_dag_runtime_helpers import (
    _task_dag_graph_for_remote,
    _task_dag_readiness_payload,
    _task_dag_readiness_from_remote_inventory,
    _task_dag_relative_path,
)
from .task_dag_telegram_watch import (
    _trigger_task_dag_execute_run_telegram_notifications as _fallback_trigger_task_dag_execute_run_telegram_notifications,
)


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)


def _trigger_task_dag_execute_run_telegram_notifications(*args: Any, **kwargs: Any) -> Any:
    return _app_override(
        "_trigger_task_dag_execute_run_telegram_notifications",
        _fallback_trigger_task_dag_execute_run_telegram_notifications,
    )(*args, **kwargs)


def _task_dag_open_execute_run(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
    graph_path: Path,
    readiness: dict[str, Any],
    payload: dict[str, Any],
) -> dict[str, Any]:
    graph_run_id_fn = _app_override("_task_dag_graph_run_id", _task_dag_graph_run_id)
    relative_path_fn = _app_override("_task_dag_relative_path", _task_dag_relative_path)
    state_snapshot_payload_fn = _app_override("_task_dag_state_snapshot_payload", _task_dag_state_snapshot_payload)
    effective_author_mode_fn = _app_override("_effective_author_mode", None)
    effective_model_name_fn = _app_override("_effective_model_name", None)
    current_line_fn = _app_override("current_line", current_line)
    load_config_fn = _app_override("load_config", load_config)
    normalize_text_value_fn = _app_override("_normalize_text_value", _normalize_text_value)
    remote_create_session_fn = _app_override("remote_create_session", remote_create_session)
    remote_append_session_event_fn = _app_override("remote_append_session_event", remote_append_session_event)
    auto_continue_profile_fn = _app_override("_task_dag_auto_continue_profile", None)
    auto_bootstrap_ready_node_ids_fn = _app_override("_task_dag_auto_bootstrap_ready_node_ids", None)
    materialize_node_lineage_fn = _app_override("_task_dag_materialize_node_lineage", None)

    if effective_author_mode_fn is None or effective_model_name_fn is None:
        raise RuntimeError("ait.cli.app did not expose the author/model helpers required for execute-run scaffolding.")

    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    plan_revision_id = str(source_plan.get("plan_revision_id") or "").strip() or None
    graph_id = str(graph.get("graph_id") or "")
    graph_run_id = graph_run_id_fn(plan_id, graph_id)
    graph_artifact_path = relative_path_fn(ctx, graph_path)
    workflow_summary = payload.get("workflow_summary") if isinstance(payload.get("workflow_summary"), dict) else {}
    readiness_summary = payload.get("readiness_summary") if isinstance(payload.get("readiness_summary"), dict) else {}
    execution_strategy = payload.get("execution_strategy") if isinstance(payload.get("execution_strategy"), dict) else {}
    execute_contract = payload.get("execute_run_contract") if isinstance(payload.get("execute_run_contract"), dict) else {}
    compact_packet_surface = payload.get("compact_packet_surface") if isinstance(payload.get("compact_packet_surface"), dict) else {}
    change_focus_policy = (
        execute_contract.get("change_focus_policy")
        if isinstance(execute_contract.get("change_focus_policy"), dict)
        else {}
    )
    next_focus = change_focus_policy.get("next_focus") if isinstance(change_focus_policy.get("next_focus"), dict) else {}
    seeded_worktree_name = None
    if (
        auto_continue_profile_fn is not None
        and auto_bootstrap_ready_node_ids_fn is not None
        and materialize_node_lineage_fn is not None
        and not str(execute_contract.get("next_focus_node_id") or "").strip()
        and not str(execute_contract.get("next_focus_task_id") or "").strip()
    ):
        profile = auto_continue_profile_fn(ctx, graph)
        bootstrap_focus_node_id = ""
        if bool(profile.get("auto_continue_supported")):
            bootstrap_candidates = auto_bootstrap_ready_node_ids_fn(graph, readiness, profile)
            if bootstrap_candidates:
                bootstrap_focus_node_id = str(bootstrap_candidates[0] or "").strip()
        if bootstrap_focus_node_id:
            materialized = materialize_node_lineage_fn(
                ctx=ctx,
                remote_row=remote_row,
                repo_name=repo_name,
                plan_id=plan_id,
                graph=graph,
                readiness=readiness,
                node_id=bootstrap_focus_node_id,
                create_worktree=True,
                allow_execution_only_without_change=True,
            )
            seeded_task_id = str(materialized.get("task_id") or "").strip() or None
            seeded_change_id = str(materialized.get("change_id") or "").strip() or None
            seeded_worktree = materialized.get("worktree") if isinstance(materialized.get("worktree"), dict) else {}
            seeded_worktree_name = normalize_text_value_fn(seeded_worktree.get("name"))
            updated_focus = {
                "focus_unit": "change" if seeded_change_id else "task" if seeded_task_id else "node",
                "node_id": bootstrap_focus_node_id,
                "task_id": seeded_task_id,
                "change_id": seeded_change_id,
            }
            if change_focus_policy:
                focus_queue = change_focus_policy.get("focus_queue") if isinstance(change_focus_policy.get("focus_queue"), list) else []
                updated_focus_queue = []
                replaced = False
                for entry in focus_queue:
                    if not isinstance(entry, dict):
                        continue
                    if str(entry.get("node_id") or "").strip() == bootstrap_focus_node_id:
                        merged = dict(entry)
                        merged.update({k: v for k, v in updated_focus.items() if v is not None})
                        updated_focus_queue.append(merged)
                        replaced = True
                    else:
                        updated_focus_queue.append(dict(entry))
                if not replaced:
                    updated_focus_queue.insert(0, {k: v for k, v in updated_focus.items() if v is not None})
                change_focus_policy = {**change_focus_policy, "focus_queue": updated_focus_queue, "next_focus": updated_focus}
            else:
                change_focus_policy = {"next_focus": updated_focus, "focus_queue": [{k: v for k, v in updated_focus.items() if v is not None}]}
            execute_contract["change_focus_policy"] = change_focus_policy
            execute_contract["next_focus_node_id"] = bootstrap_focus_node_id
            execute_contract["next_focus_task_id"] = seeded_task_id
            execute_contract["next_focus_change_id"] = seeded_change_id
            if compact_packet_surface:
                compact_packet_surface["change_focus_policy"] = change_focus_policy
                compact_packet_surface["next_focus_node_id"] = bootstrap_focus_node_id
                compact_packet_surface["next_focus_task_id"] = seeded_task_id
                compact_packet_surface["next_focus_change_id"] = seeded_change_id
    state_snapshot_payload = state_snapshot_payload_fn(
        ctx=ctx,
        plan_id=plan_id,
        plan_revision_id=plan_revision_id,
        graph=graph,
        graph_path=graph_path,
        readiness=readiness,
        graph_run_id=graph_run_id,
    )
    ready_node_ids = list(state_snapshot_payload.get("ready_node_ids") or [])
    blocked_node_ids = list(state_snapshot_payload.get("blocked_node_ids") or [])
    running_node_ids = list(state_snapshot_payload.get("running_node_ids") or [])
    dispatched_node_ids = list(state_snapshot_payload.get("dispatched_node_ids") or [])
    completed_node_ids = list(state_snapshot_payload.get("completed_node_ids") or [])

    metadata: dict[str, Any] = {
        "author_mode": effective_author_mode_fn(ctx),
        "session_policy": "task_dag_execute_run",
        "graph_run_id": graph_run_id,
        "execution_mode": execute_contract.get("capability_stage") or "scaffold",
        "execution_state": state_snapshot_payload.get("execution_state") or "active",
        "plan_id": plan_id,
        "graph_id": graph_id,
        "task_graph_json": graph_artifact_path,
        "graph_run_contract_version": execute_contract.get("contract_version") or 1,
        "workflow_mode": execute_contract.get("workflow_mode"),
        "change_strategy": execute_contract.get("change_strategy"),
        "final_land_disposition": execute_contract.get("final_land_disposition"),
        "final_remote_disposition_default": bool(execute_contract.get("final_remote_disposition_default")),
        "auto_continue_supported": bool(execute_contract.get("auto_continue_supported")),
        "auto_node_bootstrap_supported": bool(execute_contract.get("auto_node_bootstrap_supported")),
        "worker_execution_mode": execute_contract.get("worker_execution_mode"),
        "worker_execution_label": execute_contract.get("worker_execution_label"),
        "fresh_worker_session": execute_contract.get("fresh_worker_session"),
        "worker_session_count": execute_contract.get("worker_session_count"),
        "worker_session_mode": execute_contract.get("worker_session_mode"),
        "max_total_sessions": execute_contract.get("max_total_sessions"),
        "max_worker_sessions": execute_contract.get("max_worker_sessions"),
        "change_progression_mode": execute_contract.get("change_progression_mode"),
        "change_focus_policy": execute_contract.get("change_focus_policy") or {},
        "next_focus_node_id": execute_contract.get("next_focus_node_id"),
        "next_focus_task_id": execute_contract.get("next_focus_task_id"),
        "next_focus_change_id": execute_contract.get("next_focus_change_id"),
        "gate_strategy": execute_contract.get("gate_strategy"),
        "final_gate_bundle": execute_contract.get("final_gate_bundle") or [],
        "execution_only_node_ids": execute_contract.get("execution_only_node_ids") or [],
        "converged_output_node_ids": execute_contract.get("converged_output_node_ids") or [],
        "safety_boundary_node_ids": execute_contract.get("safety_boundary_node_ids") or [],
        "current_boundary": execute_contract.get("current_boundary"),
        "ready_node_ids": ready_node_ids,
        "blocked_node_ids": blocked_node_ids,
        "running_node_ids": running_node_ids,
        "dispatched_node_ids": dispatched_node_ids,
        "completed_node_ids": completed_node_ids,
        "next_action": workflow_summary.get("next_action"),
        "pause_states": execute_contract.get("pause_states") or [],
        "terminal_states": execute_contract.get("terminal_states") or [],
    }
    if compact_packet_surface:
        metadata["compact_packet_surface"] = compact_packet_surface
    if plan_revision_id:
        metadata["plan_revision_id"] = plan_revision_id

    session = remote_create_session_fn(
        remote_row["url"],
        repo_name,
        "task_graph_run",
        title=f"Task DAG execute: {graph_id or plan_id}",
        line_name=current_line_fn(ctx),
        worktree_name=seeded_worktree_name,
        model_name=effective_model_name_fn(ctx),
        metadata=metadata,
    )
    session_id = str(session.get("session_id") or "")
    event_type = "task_graph.execution_started"
    event_payload = {
        "graph_run_id": graph_run_id,
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id,
        "graph_id": graph_id,
        "graph_artifact_path": graph_artifact_path,
        "workflow_summary": workflow_summary,
        "readiness_summary": readiness_summary,
        "execution_strategy": execution_strategy,
        "execute_run_contract": execute_contract,
        "change_focus_policy": execute_contract.get("change_focus_policy") or {},
        "next_focus_node_id": execute_contract.get("next_focus_node_id"),
        "next_focus_task_id": execute_contract.get("next_focus_task_id"),
        "next_focus_change_id": execute_contract.get("next_focus_change_id"),
    }
    if compact_packet_surface:
        event_payload["compact_packet_surface"] = compact_packet_surface
    event = remote_append_session_event_fn(
        remote_row["url"],
        session_id,
        event_type,
        event_payload,
        repo_name=repo_name,
    )
    state_snapshot_event = remote_append_session_event_fn(
        remote_row["url"],
        session_id,
        "task_graph.state_snapshot",
        state_snapshot_payload,
        repo_name=repo_name,
    )
    return {
        "session_id": session_id,
        "session_kind": session.get("session_kind") or session.get("kind") or "task_graph_run",
        "graph_run_id": graph_run_id,
        "title": session.get("title"),
        "event_type": event_type,
        "event": event,
        "state_snapshot_event": state_snapshot_event,
        "metadata": metadata,
    }


def _task_dag_record_execute_run(*args: Any, **kwargs: Any) -> Any:
    return _task_dag_open_execute_run(*args, **kwargs)


def _task_dag_load_execute_run(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
    graph_path: Path,
    readiness: dict[str, Any],
    session_id: str,
) -> dict[str, Any]:
    remote_get_session_fn = _app_override("remote_get_session", remote_get_session)
    remote_list_session_events_fn = _app_override("remote_list_session_events", remote_list_session_events)
    execute_run_summary_fn = _app_override("_task_dag_execute_run_summary", _task_dag_execute_run_summary)
    relative_path_fn = _app_override("_task_dag_relative_path", _task_dag_relative_path)
    if not session_id:
        raise ValueError("Recorded graph-run inspection requires a session id.")
    session = remote_get_session_fn(remote_row["url"], session_id, repo_name=repo_name)
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    session_plan_id = str(metadata.get("plan_id") or "").strip()
    session_graph_id = str(metadata.get("graph_id") or "").strip()
    graph_id = str(graph.get("graph_id") or "")
    if session_plan_id and session_plan_id != plan_id:
        raise ValueError(f"Session {session_id} belongs to plan {session_plan_id}, not {plan_id}.")
    if session_graph_id and session_graph_id != graph_id:
        raise ValueError(f"Session {session_id} belongs to graph {session_graph_id}, not {graph_id}.")
    events = remote_list_session_events_fn(remote_row["url"], session_id, repo_name=repo_name)
    return {
        **execute_run_summary_fn(session, events),
        "graph_artifact_path": relative_path_fn(ctx, graph_path),
        "current_readiness_summary": readiness.get("summary") or {},
    }


def _task_dag_advance_execute_run(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
    graph_path: Path,
    readiness: dict[str, Any],
    session_id: str,
) -> dict[str, Any]:
    graph_for_remote_fn = _app_override("_task_dag_graph_for_remote", _task_dag_graph_for_remote)
    remote_advance_task_dag_run_fn = _app_override("remote_advance_task_dag_run", remote_advance_task_dag_run)
    remote_error_status_code_fn = _app_override("_remote_error_status_code", _remote_error_status_code)
    remote_get_session_fn = _app_override("remote_get_session", remote_get_session)
    remote_list_session_events_fn = _app_override("remote_list_session_events", remote_list_session_events)
    execute_run_summary_fn = _app_override("_task_dag_execute_run_summary", _task_dag_execute_run_summary)
    execute_state_digest_fn = _app_override("_task_dag_execute_state_digest", _task_dag_execute_state_digest)
    graph_run_id_fn = _app_override("_task_dag_graph_run_id", _task_dag_graph_run_id)
    auto_continue_profile_fn = _app_override("_task_dag_auto_continue_profile", None)
    auto_bootstrap_node_ids_for_run_fn = _app_override("_task_dag_auto_bootstrap_node_ids_for_run", None)
    bootstrap_node_fn = _app_override("_task_dag_bootstrap_node", None)
    readiness_payload_fn = _app_override("_task_dag_readiness_payload", _task_dag_readiness_payload)
    state_snapshot_payload_fn = _app_override("_task_dag_state_snapshot_payload", _task_dag_state_snapshot_payload)
    relative_path_fn = _app_override("_task_dag_relative_path", _task_dag_relative_path)
    remote_append_session_event_fn = _app_override("remote_append_session_event", remote_append_session_event)

    if auto_continue_profile_fn is None or auto_bootstrap_node_ids_for_run_fn is None or bootstrap_node_fn is None:
        raise RuntimeError("ait.cli.app did not expose the execute-run bootstrap helpers required for advance-run.")
    if not session_id:
        raise ValueError("Guarded execute-run advance requires a graph-run session id.")
    remote_graph = graph_for_remote_fn(ctx, graph, graph_path)
    try:
        return remote_advance_task_dag_run_fn(
            remote_row["url"],
            session_id,
            remote_graph,
            repo_name=repo_name,
        )
    except RemoteError as exc:
        if remote_error_status_code_fn(exc) != 404:
            raise
    session = remote_get_session_fn(remote_row["url"], session_id, repo_name=repo_name)
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    session_plan_id = str(metadata.get("plan_id") or "").strip()
    session_graph_id = str(metadata.get("graph_id") or "").strip()
    graph_id = str(graph.get("graph_id") or "")
    if session_plan_id and session_plan_id != plan_id:
        raise ValueError(f"Session {session_id} belongs to plan {session_plan_id}, not {plan_id}.")
    if session_graph_id and session_graph_id != graph_id:
        raise ValueError(f"Session {session_id} belongs to graph {session_graph_id}, not {graph_id}.")

    events = remote_list_session_events_fn(remote_row["url"], session_id, repo_name=repo_name)
    prior_summary = execute_run_summary_fn(session, events)
    previous_snapshot = (
        prior_summary.get("latest_state_snapshot")
        if isinstance(prior_summary.get("latest_state_snapshot"), dict)
        else {}
    )
    previous_digest = str(previous_snapshot.get("readiness_digest") or "").strip()
    if not previous_digest and previous_snapshot:
        previous_digest = execute_state_digest_fn(previous_snapshot)

    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    plan_revision_id = str(source_plan.get("plan_revision_id") or "").strip() or None
    graph_run_id = str(metadata.get("graph_run_id") or previous_snapshot.get("graph_run_id") or "").strip()
    if not graph_run_id:
        graph_run_id = graph_run_id_fn(plan_id, graph_id)

    profile = auto_continue_profile_fn(ctx, graph)
    auto_bootstrapped_node_ids: list[str] = []
    if profile.get("auto_continue_supported"):
        auto_bootstrap_ready_node_ids = auto_bootstrap_node_ids_for_run_fn(
            graph=graph,
            readiness=readiness,
            profile=profile,
            session_metadata=metadata,
            events=events,
        )
        for ready_node_id in auto_bootstrap_ready_node_ids:
            bootstrap_node_fn(
                ctx=ctx,
                remote_row=remote_row,
                repo_name=repo_name,
                plan_id=plan_id,
                graph=graph,
                graph_path=graph_path,
                readiness=readiness,
                node_id=ready_node_id,
                graph_run_session_id=session_id,
                create_worktree=True,
                allow_execution_only_without_change=True,
            )
            auto_bootstrapped_node_ids.append(ready_node_id)
        if auto_bootstrapped_node_ids:
            readiness = readiness_payload_fn(
                ctx,
                graph,
                _normalize_text_value(remote_row.get("name")) or _normalize_text_value(remote_row.get("remote_name")),
            )

    current_snapshot = state_snapshot_payload_fn(
        ctx=ctx,
        plan_id=plan_id,
        plan_revision_id=plan_revision_id,
        graph=graph,
        graph_path=graph_path,
        readiness=readiness,
        graph_run_id=graph_run_id,
    )
    current_digest = str(current_snapshot.get("readiness_digest") or "").strip()

    previous_completed = {str(node_id) for node_id in previous_snapshot.get("completed_node_ids") or [] if str(node_id)}
    current_completed = {str(node_id) for node_id in current_snapshot.get("completed_node_ids") or [] if str(node_id)}
    completed_regressions = sorted(previous_completed - current_completed)
    if completed_regressions:
        joined = ", ".join(completed_regressions)
        raise ValueError(f"Guarded execute-run advance detected completed-node regression for {joined}.")

    if previous_digest and previous_digest == current_digest:
        if auto_bootstrapped_node_ids:
            advance_event_payload = {
                "graph_run_id": graph_run_id,
                "plan_id": plan_id,
                "plan_revision_id": plan_revision_id,
                "graph_id": graph_id,
                "graph_artifact_path": relative_path_fn(ctx, graph_path),
                "trigger": "auto_bootstrap",
                "previous_readiness_digest": previous_digest or None,
                "readiness_digest": current_digest,
                "auto_bootstrapped_node_ids": auto_bootstrapped_node_ids,
                "newly_unblocked_node_ids": [],
                "newly_completed_node_ids": [],
                "workflow_summary": current_snapshot.get("workflow_summary") or {},
                "readiness_summary": current_snapshot.get("readiness_summary") or {},
                "next_action": current_snapshot.get("next_action"),
                "execution_state": current_snapshot.get("execution_state"),
            }
            advance_event = remote_append_session_event_fn(
                remote_row["url"],
                session_id,
                "task_graph.execution_advanced",
                advance_event_payload,
                repo_name=repo_name,
            )
            state_snapshot_event = remote_append_session_event_fn(
                remote_row["url"],
                session_id,
                "task_graph.state_snapshot",
                current_snapshot,
                repo_name=repo_name,
            )
            telegram_graph_watch_notifications = _trigger_task_dag_execute_run_telegram_notifications(
                ctx,
                remote_row=remote_row,
                repo_name=repo_name,
                plan_id=plan_id,
            )
            return {
                **execute_run_summary_fn(session, [*events, advance_event, state_snapshot_event]),
                "graph_artifact_path": relative_path_fn(ctx, graph_path),
                "current_readiness_summary": readiness.get("summary") or {},
                "advanced": True,
                "execution_state": current_snapshot.get("execution_state"),
                "workflow_summary": current_snapshot.get("workflow_summary") or {},
                "advance_event": advance_event,
                "state_snapshot_event": state_snapshot_event,
                "previous_state_snapshot": previous_snapshot or None,
                "latest_state_snapshot": current_snapshot,
                "auto_bootstrapped_node_ids": auto_bootstrapped_node_ids,
                "newly_unblocked_node_ids": [],
                "newly_completed_node_ids": [],
                "telegram_graph_watch_notifications": telegram_graph_watch_notifications,
            }
        return {
            **prior_summary,
            "graph_artifact_path": relative_path_fn(ctx, graph_path),
            "current_readiness_summary": readiness.get("summary") or {},
            "advanced": False,
            "execution_state": (previous_snapshot or current_snapshot).get("execution_state"),
            "workflow_summary": (previous_snapshot or current_snapshot).get("workflow_summary") or {},
            "noop_reason": "readiness state unchanged",
            "previous_state_snapshot": previous_snapshot or None,
            "latest_state_snapshot": previous_snapshot or current_snapshot,
            "auto_bootstrapped_node_ids": auto_bootstrapped_node_ids,
            "newly_unblocked_node_ids": [],
            "newly_completed_node_ids": [],
        }

    previous_blocked = {str(node_id) for node_id in previous_snapshot.get("blocked_node_ids") or [] if str(node_id)}
    current_executable = {
        *[str(node_id) for node_id in current_snapshot.get("ready_node_ids") or [] if str(node_id)],
        *[str(node_id) for node_id in current_snapshot.get("running_node_ids") or [] if str(node_id)],
        *[str(node_id) for node_id in current_snapshot.get("completed_node_ids") or [] if str(node_id)],
    }
    newly_unblocked = sorted(previous_blocked & current_executable)
    newly_completed = sorted(current_completed - previous_completed)
    trigger = "auto_bootstrap" if auto_bootstrapped_node_ids else ("dependency_completion" if newly_unblocked else "state_change")

    advance_event_payload = {
        "graph_run_id": graph_run_id,
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id,
        "graph_id": graph_id,
        "graph_artifact_path": relative_path_fn(ctx, graph_path),
        "trigger": trigger,
        "previous_readiness_digest": previous_digest or None,
        "readiness_digest": current_digest,
        "auto_bootstrapped_node_ids": auto_bootstrapped_node_ids,
        "newly_unblocked_node_ids": newly_unblocked,
        "newly_completed_node_ids": newly_completed,
        "workflow_summary": current_snapshot.get("workflow_summary") or {},
        "readiness_summary": current_snapshot.get("readiness_summary") or {},
        "next_action": current_snapshot.get("next_action"),
        "execution_state": current_snapshot.get("execution_state"),
    }
    advance_event = remote_append_session_event_fn(
        remote_row["url"],
        session_id,
        "task_graph.execution_advanced",
        advance_event_payload,
        repo_name=repo_name,
    )
    state_snapshot_event = remote_append_session_event_fn(
        remote_row["url"],
        session_id,
        "task_graph.state_snapshot",
        current_snapshot,
        repo_name=repo_name,
    )
    telegram_graph_watch_notifications = _trigger_task_dag_execute_run_telegram_notifications(
        ctx,
        remote_row=remote_row,
        repo_name=repo_name,
        plan_id=plan_id,
    )
    return {
        **execute_run_summary_fn(session, [*events, advance_event, state_snapshot_event]),
        "graph_artifact_path": relative_path_fn(ctx, graph_path),
        "current_readiness_summary": readiness.get("summary") or {},
        "advanced": True,
        "execution_state": current_snapshot.get("execution_state"),
        "workflow_summary": current_snapshot.get("workflow_summary") or {},
        "advance_event": advance_event,
        "state_snapshot_event": state_snapshot_event,
        "previous_state_snapshot": previous_snapshot or None,
        "latest_state_snapshot": current_snapshot,
        "auto_bootstrapped_node_ids": auto_bootstrapped_node_ids,
        "newly_unblocked_node_ids": newly_unblocked,
        "newly_completed_node_ids": newly_completed,
        "telegram_graph_watch_notifications": telegram_graph_watch_notifications,
    }


def _task_dag_refresh_execute_run(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
    graph_path: Path,
    readiness: dict[str, Any],
    session_id: str,
    trigger: str,
    worker_session_id: str | None = None,
) -> dict[str, Any]:
    remote_get_session_fn = _app_override("remote_get_session", remote_get_session)
    remote_list_session_events_fn = _app_override("remote_list_session_events", remote_list_session_events)
    execute_run_summary_fn = _app_override("_task_dag_execute_run_summary", _task_dag_execute_run_summary)
    execute_state_digest_fn = _app_override("_task_dag_execute_state_digest", _task_dag_execute_state_digest)
    graph_run_id_fn = _app_override("_task_dag_graph_run_id", _task_dag_graph_run_id)
    state_snapshot_payload_fn = _app_override("_task_dag_state_snapshot_payload", _task_dag_state_snapshot_payload)
    relative_path_fn = _app_override("_task_dag_relative_path", _task_dag_relative_path)
    remote_append_session_event_fn = _app_override("remote_append_session_event", remote_append_session_event)
    if not session_id:
        raise ValueError("Graph-run refresh requires a graph-run session id.")
    session = remote_get_session_fn(remote_row["url"], session_id, repo_name=repo_name)
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    session_plan_id = str(metadata.get("plan_id") or "").strip()
    session_graph_id = str(metadata.get("graph_id") or "").strip()
    graph_id = str(graph.get("graph_id") or "")
    if session_plan_id and session_plan_id != plan_id:
        raise ValueError(f"Session {session_id} belongs to plan {session_plan_id}, not {plan_id}.")
    if session_graph_id and session_graph_id != graph_id:
        raise ValueError(f"Session {session_id} belongs to graph {session_graph_id}, not {graph_id}.")

    events = remote_list_session_events_fn(remote_row["url"], session_id, repo_name=repo_name)
    prior_summary = execute_run_summary_fn(session, events)
    previous_snapshot = (
        prior_summary.get("latest_state_snapshot")
        if isinstance(prior_summary.get("latest_state_snapshot"), dict)
        else {}
    )
    previous_digest = str(previous_snapshot.get("readiness_digest") or "").strip()
    if not previous_digest and previous_snapshot:
        previous_digest = execute_state_digest_fn(previous_snapshot)

    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    plan_revision_id = str(source_plan.get("plan_revision_id") or "").strip() or None
    graph_run_id = str(metadata.get("graph_run_id") or previous_snapshot.get("graph_run_id") or "").strip()
    if not graph_run_id:
        graph_run_id = graph_run_id_fn(plan_id, graph_id)

    current_snapshot = state_snapshot_payload_fn(
        ctx=ctx,
        plan_id=plan_id,
        plan_revision_id=plan_revision_id,
        graph=graph,
        graph_path=graph_path,
        readiness=readiness,
        graph_run_id=graph_run_id,
    )
    current_digest = str(current_snapshot.get("readiness_digest") or "").strip()

    previous_completed = {str(node_id) for node_id in previous_snapshot.get("completed_node_ids") or [] if str(node_id)}
    current_completed = {str(node_id) for node_id in current_snapshot.get("completed_node_ids") or [] if str(node_id)}
    completed_regressions = sorted(previous_completed - current_completed)
    if completed_regressions:
        joined = ", ".join(completed_regressions)
        raise ValueError(f"Guarded execute-run refresh detected completed-node regression for {joined}.")

    if previous_digest and previous_digest == current_digest:
        return {
            **prior_summary,
            "graph_artifact_path": relative_path_fn(ctx, graph_path),
            "current_readiness_summary": readiness.get("summary") or {},
            "advanced": False,
            "execution_state": (previous_snapshot or current_snapshot).get("execution_state"),
            "workflow_summary": (previous_snapshot or current_snapshot).get("workflow_summary") or {},
            "noop_reason": "readiness state unchanged",
            "previous_state_snapshot": previous_snapshot or None,
            "latest_state_snapshot": previous_snapshot or current_snapshot,
            "newly_unblocked_node_ids": [],
            "newly_completed_node_ids": [],
        }

    previous_blocked = {str(node_id) for node_id in previous_snapshot.get("blocked_node_ids") or [] if str(node_id)}
    current_executable = {
        *[str(node_id) for node_id in current_snapshot.get("ready_node_ids") or [] if str(node_id)],
        *[str(node_id) for node_id in current_snapshot.get("running_node_ids") or [] if str(node_id)],
        *[str(node_id) for node_id in current_snapshot.get("completed_node_ids") or [] if str(node_id)],
    }
    newly_unblocked = sorted(previous_blocked & current_executable)
    newly_completed = sorted(current_completed - previous_completed)

    advance_event_payload = {
        "graph_run_id": graph_run_id,
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id,
        "graph_id": graph_id,
        "graph_artifact_path": relative_path_fn(ctx, graph_path),
        "trigger": trigger,
        "previous_readiness_digest": previous_digest or None,
        "readiness_digest": current_digest,
        "newly_unblocked_node_ids": newly_unblocked,
        "newly_completed_node_ids": newly_completed,
        "workflow_summary": current_snapshot.get("workflow_summary") or {},
        "readiness_summary": current_snapshot.get("readiness_summary") or {},
        "next_action": current_snapshot.get("next_action"),
        "execution_state": current_snapshot.get("execution_state"),
    }
    if worker_session_id:
        advance_event_payload["worker_session_id"] = worker_session_id
    advance_event = remote_append_session_event_fn(
        remote_row["url"],
        session_id,
        "task_graph.execution_advanced",
        advance_event_payload,
        repo_name=repo_name,
    )
    state_snapshot_event = remote_append_session_event_fn(
        remote_row["url"],
        session_id,
        "task_graph.state_snapshot",
        current_snapshot,
        repo_name=repo_name,
    )
    telegram_graph_watch_notifications = _trigger_task_dag_execute_run_telegram_notifications(
        ctx,
        remote_row=remote_row,
        repo_name=repo_name,
        plan_id=plan_id,
    )
    return {
        **execute_run_summary_fn(session, [*events, advance_event, state_snapshot_event]),
        "graph_artifact_path": relative_path_fn(ctx, graph_path),
        "current_readiness_summary": readiness.get("summary") or {},
        "advanced": True,
        "execution_state": current_snapshot.get("execution_state"),
        "workflow_summary": current_snapshot.get("workflow_summary") or {},
        "advance_event": advance_event,
        "state_snapshot_event": state_snapshot_event,
        "previous_state_snapshot": previous_snapshot or None,
        "latest_state_snapshot": current_snapshot,
        "newly_unblocked_node_ids": newly_unblocked,
        "newly_completed_node_ids": newly_completed,
        "telegram_graph_watch_notifications": telegram_graph_watch_notifications,
    }


def _task_dag_pause_execution_state(reason: str) -> str:
    normalized = str(reason or "manual").strip().lower().replace("-", "_")
    mapping = {
        "manual": "manual_operator_pause",
        "manual_operator_pause": "manual_operator_pause",
        "review": "waiting_for_review",
        "attestation": "waiting_for_attestation",
        "policy": "waiting_for_policy",
        "land": "waiting_for_land",
    }
    if normalized not in mapping:
        allowed = ", ".join(sorted(mapping))
        raise ValueError(f"Unknown pause reason {reason!r}; choose one of: {allowed}.")
    return mapping[normalized]


def _task_dag_control_execute_run(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
    graph_path: Path,
    readiness: dict[str, Any],
    session_id: str,
    action: str,
    pause_reason: str | None = None,
) -> dict[str, Any]:
    remote_get_session_fn = _app_override("remote_get_session", remote_get_session)
    remote_list_session_events_fn = _app_override("remote_list_session_events", remote_list_session_events)
    execute_run_summary_fn = _app_override("_task_dag_execute_run_summary", _task_dag_execute_run_summary)
    graph_run_id_fn = _app_override("_task_dag_graph_run_id", _task_dag_graph_run_id)
    state_snapshot_payload_fn = _app_override("_task_dag_state_snapshot_payload", _task_dag_state_snapshot_payload)
    pause_execution_state_fn = _app_override("_task_dag_pause_execution_state", _task_dag_pause_execution_state)
    execute_state_from_snapshot_fn = _app_override("_task_dag_execute_state_from_snapshot", _task_dag_execute_state_from_snapshot)
    execute_state_digest_fn = _app_override("_task_dag_execute_state_digest", _task_dag_execute_state_digest)
    relative_path_fn = _app_override("_task_dag_relative_path", _task_dag_relative_path)
    remote_append_session_event_fn = _app_override("remote_append_session_event", remote_append_session_event)
    if not session_id:
        raise ValueError("Graph-run operator controls require a graph-run session id.")
    session = remote_get_session_fn(remote_row["url"], session_id, repo_name=repo_name)
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    session_plan_id = str(metadata.get("plan_id") or "").strip()
    session_graph_id = str(metadata.get("graph_id") or "").strip()
    graph_id = str(graph.get("graph_id") or "")
    if session_plan_id and session_plan_id != plan_id:
        raise ValueError(f"Session {session_id} belongs to plan {session_plan_id}, not {plan_id}.")
    if session_graph_id and session_graph_id != graph_id:
        raise ValueError(f"Session {session_id} belongs to graph {session_graph_id}, not {graph_id}.")

    events = remote_list_session_events_fn(remote_row["url"], session_id, repo_name=repo_name)
    prior_summary = execute_run_summary_fn(session, events)
    previous_snapshot = (
        prior_summary.get("latest_state_snapshot")
        if isinstance(prior_summary.get("latest_state_snapshot"), dict)
        else {}
    )
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    plan_revision_id = str(source_plan.get("plan_revision_id") or "").strip() or None
    graph_run_id = str(metadata.get("graph_run_id") or previous_snapshot.get("graph_run_id") or "").strip()
    if not graph_run_id:
        graph_run_id = graph_run_id_fn(plan_id, graph_id)

    state_snapshot = state_snapshot_payload_fn(
        ctx=ctx,
        plan_id=plan_id,
        plan_revision_id=plan_revision_id,
        graph=graph,
        graph_path=graph_path,
        readiness=readiness,
        graph_run_id=graph_run_id,
    )
    state_snapshot["operator_action"] = action
    event_type = "task_graph.execution_resumed"
    action_payload: dict[str, Any] = {}
    if action == "pause":
        execution_state = pause_execution_state_fn(pause_reason or "manual")
        state_snapshot["execution_state"] = execution_state
        state_snapshot["pause_reason"] = str(pause_reason or "manual").strip().lower().replace("-", "_")
        if execution_state == "waiting_for_review":
            state_snapshot["next_action"] = "record review approval"
        elif execution_state == "waiting_for_attestation":
            state_snapshot["next_action"] = "record attestation"
        elif execution_state == "waiting_for_policy":
            state_snapshot["next_action"] = "evaluate policy"
        elif execution_state == "waiting_for_land":
            state_snapshot["next_action"] = "submit land"
        elif not state_snapshot.get("next_action"):
            state_snapshot["next_action"] = "operator resume"
        event_type = "task_graph.execution_paused"
        action_payload["pause_reason"] = state_snapshot["pause_reason"]
    elif action == "resume":
        state_snapshot["execution_state"] = execute_state_from_snapshot_fn(state_snapshot)
        event_type = "task_graph.execution_resumed"
    elif action == "retry":
        state_snapshot["execution_state"] = execute_state_from_snapshot_fn(state_snapshot)
        event_type = "task_graph.execution_retried"
    elif action == "abort":
        state_snapshot["execution_state"] = "aborted"
        state_snapshot["next_action"] = "aborted by operator"
        event_type = "task_graph.execution_aborted"
    else:
        raise ValueError(f"Unknown graph-run operator action: {action}")
    state_snapshot["readiness_digest"] = execute_state_digest_fn(state_snapshot)

    previous_digest = str(previous_snapshot.get("readiness_digest") or "").strip()
    if not previous_digest and previous_snapshot:
        previous_digest = execute_state_digest_fn(previous_snapshot)
    if previous_digest and previous_digest == str(state_snapshot.get("readiness_digest") or ""):
        return {
            **prior_summary,
            "graph_artifact_path": relative_path_fn(ctx, graph_path),
            "current_readiness_summary": readiness.get("summary") or {},
            "operator_action": action,
            "controlled": False,
            "noop_reason": "graph-run state unchanged",
            "previous_state_snapshot": previous_snapshot or None,
            "latest_state_snapshot": previous_snapshot or state_snapshot,
        }

    control_event_payload = {
        "graph_run_id": graph_run_id,
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id,
        "graph_id": graph_id,
        "graph_artifact_path": relative_path_fn(ctx, graph_path),
        "operator_action": action,
        "previous_execution_state": previous_snapshot.get("execution_state") or metadata.get("execution_state"),
        "execution_state": state_snapshot.get("execution_state"),
        "next_action": state_snapshot.get("next_action"),
        "workflow_summary": state_snapshot.get("workflow_summary") or {},
        "readiness_summary": state_snapshot.get("readiness_summary") or {},
        **action_payload,
    }
    control_event = remote_append_session_event_fn(
        remote_row["url"],
        session_id,
        event_type,
        control_event_payload,
        repo_name=repo_name,
    )
    state_snapshot_event = remote_append_session_event_fn(
        remote_row["url"],
        session_id,
        "task_graph.state_snapshot",
        state_snapshot,
        repo_name=repo_name,
    )
    return {
        **execute_run_summary_fn(session, [*events, control_event, state_snapshot_event]),
        "graph_artifact_path": relative_path_fn(ctx, graph_path),
        "current_readiness_summary": readiness.get("summary") or {},
        "operator_action": action,
        "controlled": True,
        "execution_state": state_snapshot.get("execution_state"),
        "workflow_summary": state_snapshot.get("workflow_summary") or {},
        "control_event": control_event,
        "state_snapshot_event": state_snapshot_event,
        "previous_state_snapshot": previous_snapshot or None,
        "latest_state_snapshot": state_snapshot,
        "pause_reason": state_snapshot.get("pause_reason"),
    }
