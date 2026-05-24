from __future__ import annotations

import sys
from pathlib import Path
from typing import Any, Mapping

from ..store import RepoContext
from ..task_dag_readiness import (
    build_task_graph_execution_strategy as _fallback_build_task_graph_execution_strategy,
    task_dag_change_strategy as _fallback_task_dag_change_strategy,
    task_dag_final_land_disposition as _fallback_task_dag_final_land_disposition,
    task_dag_final_output_later_remote_promotion_allowed as _fallback_task_dag_final_output_later_remote_promotion_allowed,
    task_dag_final_remote_disposition_default as _fallback_task_dag_final_remote_disposition_default,
)
from .task_dag_execute_run_state import _task_dag_state_bucket_ids as _fallback_task_dag_state_bucket_ids
from .task_dag_readiness_views import (
    _task_dag_change_focus_policy as _fallback_task_dag_change_focus_policy,
    _task_dag_completed_nodes as _fallback_task_dag_completed_nodes,
    _task_dag_node_index as _fallback_task_dag_node_index,
    _task_dag_ready_nodes as _fallback_task_dag_ready_nodes,
    _task_dag_schedule_payload as _fallback_task_dag_schedule_payload,
    _task_dag_takeover_dispatched_nodes as _fallback_task_dag_takeover_dispatched_nodes,
    _task_dag_view_rows as _fallback_task_dag_view_rows,
)
from .task_dag_runtime_helpers import _task_dag_relative_path as _fallback_task_dag_relative_path
from .task_dag_topology_helpers import (
    _task_dag_converged_output_node_ids as _fallback_task_dag_converged_output_node_ids,
    _task_dag_execution_only_node_ids as _fallback_task_dag_execution_only_node_ids,
    _task_dag_safety_boundary_node_ids as _fallback_task_dag_safety_boundary_node_ids,
)
from .workflow_mode_config import _effective_workflow_mode as _fallback_effective_workflow_mode

TASK_DAG_SOLO_GATE_STRATEGY = "end_of_dag_gate_concentration"
DEFAULT_TASK_DAG_FINAL_GATE_BUNDLE = ("review", "attestation", "policy", "land")


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)


def build_task_graph_execution_strategy(*args: Any, **kwargs: Any) -> Any:
    return _app_override("build_task_graph_execution_strategy", _fallback_build_task_graph_execution_strategy)(*args, **kwargs)


def task_dag_change_strategy(*args: Any, **kwargs: Any) -> Any:
    return _app_override("task_dag_change_strategy", _fallback_task_dag_change_strategy)(*args, **kwargs)


def task_dag_final_land_disposition(*args: Any, **kwargs: Any) -> Any:
    return _app_override("task_dag_final_land_disposition", _fallback_task_dag_final_land_disposition)(*args, **kwargs)


def task_dag_final_remote_disposition_default(*args: Any, **kwargs: Any) -> Any:
    return _app_override("task_dag_final_remote_disposition_default", _fallback_task_dag_final_remote_disposition_default)(*args, **kwargs)


def task_dag_final_output_later_remote_promotion_allowed(*args: Any, **kwargs: Any) -> Any:
    return _app_override(
        "task_dag_final_output_later_remote_promotion_allowed",
        _fallback_task_dag_final_output_later_remote_promotion_allowed,
    )(*args, **kwargs)


def _effective_workflow_mode(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_effective_workflow_mode", _fallback_effective_workflow_mode)(*args, **kwargs)


def _task_dag_relative_path(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_relative_path", _fallback_task_dag_relative_path)(*args, **kwargs)


def _task_dag_schedule_payload(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_schedule_payload", _fallback_task_dag_schedule_payload)(*args, **kwargs)


def _task_dag_change_focus_policy(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_change_focus_policy", _fallback_task_dag_change_focus_policy)(*args, **kwargs)


def _task_dag_completed_nodes(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_completed_nodes", _fallback_task_dag_completed_nodes)(*args, **kwargs)


def _task_dag_node_index(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_node_index", _fallback_task_dag_node_index)(*args, **kwargs)


def _task_dag_ready_nodes(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_ready_nodes", _fallback_task_dag_ready_nodes)(*args, **kwargs)


def _task_dag_takeover_dispatched_nodes(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_takeover_dispatched_nodes", _fallback_task_dag_takeover_dispatched_nodes)(*args, **kwargs)


def _task_dag_view_rows(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_view_rows", _fallback_task_dag_view_rows)(*args, **kwargs)


def _task_dag_state_bucket_ids(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_state_bucket_ids", _fallback_task_dag_state_bucket_ids)(*args, **kwargs)


def _task_dag_execution_only_node_ids(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_execution_only_node_ids", _fallback_task_dag_execution_only_node_ids)(*args, **kwargs)


def _task_dag_converged_output_node_ids(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_converged_output_node_ids", _fallback_task_dag_converged_output_node_ids)(*args, **kwargs)


def _task_dag_safety_boundary_node_ids(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_safety_boundary_node_ids", _fallback_task_dag_safety_boundary_node_ids)(*args, **kwargs)


def _task_dag_final_remote_disposition_default(ctx: RepoContext, graph: dict[str, Any]) -> bool:
    workflow_mode = str((_effective_workflow_mode(ctx) or {}).get("value") or "custom")
    return task_dag_final_remote_disposition_default(graph, workflow_mode=workflow_mode)


def _task_dag_node_states_from_readiness(readiness: dict[str, Any]) -> dict[str, dict[str, Any]]:
    states: dict[str, dict[str, Any]] = {}
    for row in readiness.get("nodes") or []:
        if not isinstance(row, dict) or not row.get("node_id"):
            continue
        states[str(row["node_id"])] = {
            "state": str(row.get("state") or "blocked"),
            "reason": row.get("reason"),
        }
    return states


def _task_dag_ready_payload(plan_id: str, graph: dict[str, Any], graph_path: Path, readiness: dict[str, Any], ctx: RepoContext) -> dict[str, Any]:
    schedule = _task_dag_schedule_payload(plan_id, graph, graph_path, readiness, ctx)
    return {
        "plan_id": schedule["plan_id"],
        "graph_id": schedule["graph_id"],
        "graph_artifact_path": schedule["graph_artifact_path"],
        "summary": schedule["summary"],
        "readiness_summary": schedule["readiness_summary"],
        "workflow_summary": schedule["workflow_summary"],
        "ready": schedule["ready"],
        "dispatched": schedule["dispatched"],
        "blocked": schedule["blocked"],
    }


def _task_dag_auto_bootstrap_ready_node_ids(
    graph: dict[str, Any],
    readiness: dict[str, Any],
    profile: dict[str, Any],
) -> list[str]:
    graph_nodes = _task_dag_node_index(graph)
    safety_boundary_node_ids = set(profile.get("safety_boundary_node_ids") or [])
    ready_node_ids: list[str] = []
    candidate_rows = _task_dag_ready_nodes(readiness) + _task_dag_takeover_dispatched_nodes(readiness)
    for row in candidate_rows:
        node_id = str(row.get("node_id") or "")
        graph_node = graph_nodes.get(node_id)
        if (
            node_id
            and node_id not in safety_boundary_node_ids
            and isinstance(graph_node, dict)
            and str(graph_node.get("node_kind") or "") == "task"
            and node_id not in ready_node_ids
        ):
            ready_node_ids.append(node_id)
    return ready_node_ids


def _task_dag_bootstrapped_node_ids_from_events(events: list[dict[str, Any]]) -> list[str]:
    node_ids: list[str] = []
    for event in events:
        if not isinstance(event, dict) or str(event.get("event_type") or "") != "task_graph.node_bootstrapped":
            continue
        payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
        node_id = str(payload.get("node_id") or "").strip()
        if node_id and node_id not in node_ids:
            node_ids.append(node_id)
    return node_ids


def _task_dag_int(value: Any, default: int = 0) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def _task_dag_active_bootstrapped_node_ids(
    readiness: dict[str, Any],
    events: list[dict[str, Any]],
) -> list[str]:
    completed_node_ids = {
        str(node_id)
        for node_id in (_task_dag_state_bucket_ids(readiness).get("completed_node_ids") or [])
        if str(node_id)
    }
    return [
        node_id
        for node_id in _task_dag_bootstrapped_node_ids_from_events(events)
        if node_id not in completed_node_ids
    ]


def _task_dag_auto_bootstrap_node_ids_for_run(
    *,
    graph: dict[str, Any],
    readiness: dict[str, Any],
    profile: dict[str, Any],
    session_metadata: Mapping[str, Any],
    events: list[dict[str, Any]],
) -> list[str]:
    candidate_node_ids = _task_dag_auto_bootstrap_ready_node_ids(graph, readiness, profile)
    if not candidate_node_ids:
        return []
    max_worker_sessions = _task_dag_int(session_metadata.get("max_worker_sessions"), -1)
    if max_worker_sessions < 0:
        execution_strategy = build_task_graph_execution_strategy(graph, _task_dag_view_rows(readiness))
        max_worker_sessions = _task_dag_int(execution_strategy.get("max_worker_sessions"), 0)
    active_bootstrapped_node_ids = _task_dag_active_bootstrapped_node_ids(readiness, events)
    available_worker_slots = max(max_worker_sessions - len(active_bootstrapped_node_ids), 0)
    if available_worker_slots <= 0:
        return []
    active_bootstrapped = set(active_bootstrapped_node_ids)
    return [node_id for node_id in candidate_node_ids if node_id not in active_bootstrapped][:available_worker_slots]


def _task_dag_auto_continue_profile(ctx: RepoContext, graph: dict[str, Any]) -> dict[str, Any]:
    workflow_mode = str((_effective_workflow_mode(ctx) or {}).get("value") or "custom")
    policy = graph.get("execution_policy") if isinstance(graph.get("execution_policy"), dict) else {}
    configured_strategy = str(policy.get("gate_strategy") or policy.get("solo_gate_strategy") or "").strip().lower()
    enabled = any(
        isinstance(node, dict) and str(node.get("node_kind") or "") == "task"
        for node in graph.get("nodes", [])
    )
    change_strategy = task_dag_change_strategy(graph, workflow_mode=workflow_mode)
    final_land_disposition = task_dag_final_land_disposition(graph, workflow_mode=workflow_mode)
    later_remote_promotion_allowed = task_dag_final_output_later_remote_promotion_allowed(
        graph,
        workflow_mode=workflow_mode,
    )
    final_gate_bundle = (
        [
            str(value).strip().lower()
            for value in (policy.get("final_gate_bundle") or DEFAULT_TASK_DAG_FINAL_GATE_BUNDLE)
            if str(value).strip()
        ]
        if final_land_disposition == "remote"
        else []
    )
    final_remote_disposition_default = final_land_disposition == "remote"
    return {
        "workflow_mode": workflow_mode,
        "change_strategy": change_strategy,
        "final_land_disposition": final_land_disposition,
        "final_remote_disposition_default": final_remote_disposition_default,
        "later_remote_promotion_allowed_after_local_land": later_remote_promotion_allowed,
        "auto_continue_supported": enabled,
        "auto_node_bootstrap_supported": enabled,
        "gate_strategy": (configured_strategy or TASK_DAG_SOLO_GATE_STRATEGY) if enabled else None,
        "final_gate_bundle": final_gate_bundle if enabled else [],
        "execution_only_node_ids": _task_dag_execution_only_node_ids(graph) if enabled else [],
        "converged_output_node_ids": _task_dag_converged_output_node_ids(graph) if enabled else [],
        "safety_boundary_node_ids": _task_dag_safety_boundary_node_ids(graph) if enabled else [],
    }


def _task_dag_execute_contract(
    ctx: RepoContext,
    *,
    plan_id: str,
    graph: dict[str, Any],
    graph_path: Path,
    execution_strategy: dict[str, Any],
    change_focus_policy: dict[str, Any] | None = None,
) -> dict[str, Any]:
    profile = _task_dag_auto_continue_profile(ctx, graph)
    auto_continue_supported = bool(profile.get("auto_continue_supported"))
    focus_policy = change_focus_policy if isinstance(change_focus_policy, dict) else {}
    next_focus = focus_policy.get("next_focus") if isinstance(focus_policy.get("next_focus"), dict) else {}
    final_land_disposition = str(profile.get("final_land_disposition") or "remote").strip().lower()
    final_remote_disposition_default = bool(profile.get("final_remote_disposition_default"))
    later_remote_promotion_allowed = bool(profile.get("later_remote_promotion_allowed_after_local_land"))
    starter_command = f"ait plan execute {plan_id} --from-json {_task_dag_relative_path(ctx, graph_path)}"
    starter_command += " --auto-compact-worker --yes"
    return {
        "contract_version": 1,
        "starter_command": starter_command,
        "capability_stage": "guarded_full_dag_convergence" if auto_continue_supported else "scaffold",
        "workflow_mode": profile.get("workflow_mode"),
        "change_strategy": profile.get("change_strategy"),
        "final_land_disposition": final_land_disposition,
        "final_remote_disposition_default": final_remote_disposition_default,
        "later_remote_promotion_allowed_after_local_land": later_remote_promotion_allowed,
        "auto_continue_supported": auto_continue_supported,
        "auto_node_bootstrap_supported": bool(profile.get("auto_node_bootstrap_supported")),
        "change_progression_mode": "single_worker_per_change_patchset",
        "change_focus_policy": focus_policy,
        "next_focus_node_id": next_focus.get("node_id"),
        "next_focus_task_id": next_focus.get("task_id"),
        "next_focus_change_id": next_focus.get("change_id"),
        "records_graph_run_session": True,
        "record_run_requires_confirmation": True,
        "gate_model": (
            "converged_output_gate_bundle"
            if auto_continue_supported
            else "explicit_review_attestation_policy_land"
        ),
        "gate_strategy": profile.get("gate_strategy"),
        "final_gate_bundle": profile.get("final_gate_bundle") or [],
        "execution_only_node_ids": profile.get("execution_only_node_ids") or [],
        "converged_output_node_ids": profile.get("converged_output_node_ids") or [],
        "safety_boundary_node_ids": profile.get("safety_boundary_node_ids") or [],
        "current_boundary": (
            "Graph runs record one worker-only compact packet executed in one fresh worker session and keep the whole DAG "
            "inside that same worker/session lineage instead of exposing per-node bootstrap or fan-out surfaces. "
            "Execution-only nodes may keep local task/change/snapshot/local-land lineage until the converged gate bundle when the graph uses local-first final remote disposition. "
            "Reviewable work inside that one worker session must keep one focus change active at a time and cut a patchset before moving to the next reviewable change."
            if auto_continue_supported and final_remote_disposition_default
            else "Graph runs record one worker-only compact packet executed in one fresh worker session and keep the whole DAG "
            "inside that same worker/session lineage instead of exposing per-node bootstrap or fan-out surfaces. "
            "Execution-only nodes may keep local task/change/snapshot/local-land lineage until the converged local-land boundary when the graph uses local-first final local disposition. "
            "After that local land, the final converged output may later remote-promote through `ait workflow land --all-completed-local --remote <name>`. "
            "Reviewable work inside that one worker session must keep one focus change active at a time and cut a patchset before moving to the next reviewable change."
            if auto_continue_supported
            else "Initial compact-packet graph-run contract and one fresh worker-session lineage can be recorded now; "
            "start the bound compact worker when you are ready to carry the DAG forward, keeping one reviewable focus change active until its patchset cut is complete."
        ),
        "pause_states": [
            "waiting_for_dependency_completion",
            "waiting_for_review",
            "waiting_for_attestation",
            "waiting_for_policy",
            "waiting_for_land",
            "paused_for_safety_boundary",
            "paused_for_converged_gate_bundle",
            "manual_operator_pause",
        ],
        "terminal_states": ["completed", "paused_for_converged_gate_bundle", "paused_for_safety_boundary", "failed", "aborted"],
        "recommended_worker_sessions": execution_strategy.get("recommended_worker_sessions", 0),
        "worker_execution_mode": execution_strategy.get("worker_execution_mode"),
        "worker_execution_label": execution_strategy.get("worker_execution_label"),
        "fresh_worker_session": execution_strategy.get("fresh_worker_session"),
        "worker_session_count": execution_strategy.get("worker_session_count"),
        "worker_session_mode": execution_strategy.get("worker_session_mode"),
        "max_total_sessions": execution_strategy.get("max_total_sessions"),
        "max_worker_sessions": execution_strategy.get("max_worker_sessions"),
    }


def _task_dag_execute_payload(plan_id: str, graph: dict[str, Any], graph_path: Path, readiness: dict[str, Any], ctx: RepoContext) -> dict[str, Any]:
    schedule = _task_dag_schedule_payload(plan_id, graph, graph_path, readiness, ctx)
    completed_rows = _task_dag_completed_nodes(readiness)
    execution_strategy = schedule.get("execution_strategy") if isinstance(schedule.get("execution_strategy"), dict) else {}
    promotion_policy = schedule.get("promotion_policy") if isinstance(schedule.get("promotion_policy"), dict) else {}
    change_focus_policy = _task_dag_change_focus_policy(
        [
            *(schedule.get("running") if isinstance(schedule.get("running"), list) else []),
            *(schedule.get("ready") if isinstance(schedule.get("ready"), list) else []),
            *(schedule.get("dispatched") if isinstance(schedule.get("dispatched"), list) else []),
        ],
        execution_only_node_ids=promotion_policy.get("execution_only_node_ids") or [],
    )
    return {
        **schedule,
        "mode": "advisory",
        "validated": True,
        "stale_source_plan": bool(readiness.get("stale_source_plan")),
        "change_focus_policy": change_focus_policy,
        "execute_run_contract": _task_dag_execute_contract(
            ctx,
            plan_id=plan_id,
            graph=graph,
            graph_path=graph_path,
            execution_strategy=execution_strategy,
            change_focus_policy=change_focus_policy,
        ),
        "completed": [
            {
                "node_id": row.get("node_id"),
                "title": row.get("title"),
                "state": row.get("state"),
                "workflow_state": row.get("workflow_state"),
                "task_id": row.get("task_id"),
                "change_id": row.get("change_id"),
                "land_id": row.get("land_id"),
                "landed_snapshot_id": row.get("landed_snapshot_id"),
            }
            for row in completed_rows
        ],
    }
