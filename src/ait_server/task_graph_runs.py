from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any, Mapping

from .task_dag_seams import (
    build_task_dag_promotion_policy,
    build_task_dag_token_budget_hint_summary,
    build_task_graph_execution_strategy,
    build_task_graph_progress,
    topological_node_order,
)

from .server_paths import ServerContext
from .server_store import (
    append_session_event,
    create_change,
    create_session,
    create_task,
    get_content_repository,
    get_session,
    list_session_events,
)

TASK_DAG_SOLO_GATE_STRATEGY = "end_of_dag_gate_concentration"
DEFAULT_TASK_DAG_FINAL_GATE_BUNDLE = ("review", "attestation", "policy", "land")


def _task_dag_final_remote_disposition_default(graph: dict[str, Any]) -> bool:
    promotion_policy = build_task_dag_promotion_policy(graph)
    return str(promotion_policy.get("change_strategy") or "").strip().lower() == "local_first_final_remote_land"


def _task_dag_node_lineage(row: dict[str, Any]) -> dict[str, Any]:
    return row.get("lineage") if isinstance(row.get("lineage"), dict) else {}


def _task_dag_view_state(row: dict[str, Any]) -> str:
    state = str(row.get("state") or "blocked")
    explicit_state = str(row.get("explicit_state") or "").strip().lower()
    if explicit_state in {"running", "in_progress", "local_progress"}:
        return "running"
    lineage = _task_dag_node_lineage(row)
    if state in {"ready", "running"}:
        task_id = lineage.get("task_id")
        change_id = lineage.get("change_id")
        task_run_id = lineage.get("task_run_id")
        if task_id and not change_id and not task_run_id:
            return "ready"
    return state


def _task_dag_workflow_state(row: dict[str, Any]) -> str:
    view_state = _task_dag_view_state(row)
    if view_state == "completed":
        return "completed"
    explicit_state = str(row.get("explicit_state") or "").strip().lower()
    if view_state == "running" and explicit_state in {"running", "in_progress", "local_progress"}:
        return "running"
    lineage = _task_dag_node_lineage(row)
    if lineage.get("task_id") and not lineage.get("change_id") and not lineage.get("task_run_id"):
        return "dispatched"
    return view_state


def _task_dag_view_row(row: dict[str, Any]) -> dict[str, Any]:
    lineage = _task_dag_node_lineage(row)
    view_state = _task_dag_view_state(row)
    workflow_state = _task_dag_workflow_state(row)
    view_row = {
        "node_id": row.get("node_id"),
        "node_kind": row.get("node_kind"),
        "title": row.get("title"),
        "plan_item_ref": row.get("plan_item_ref"),
        "state": view_state,
        "workflow_state": workflow_state,
        "reason": row.get("reason"),
        "depends_on": row.get("depends_on") or [],
        "lock_keys": row.get("lock_keys") or [],
        "hotspot_keys": row.get("hotspot_keys") or [],
        "lineage": lineage,
        "task_id": lineage.get("task_id"),
        "change_id": lineage.get("change_id"),
        "session_id": lineage.get("session_id"),
        "task_run_id": lineage.get("task_run_id"),
        "checkpoint_id": lineage.get("checkpoint_id"),
        "patchset_id": lineage.get("patchset_id"),
        "patchset_base_snapshot_id": lineage.get("patchset_base_snapshot_id"),
        "patchset_revision_snapshot_id": lineage.get("patchset_revision_snapshot_id"),
        "land_id": lineage.get("land_id"),
        "landed_snapshot_id": lineage.get("landed_snapshot_id"),
        "owner_session_id": row.get("owner_session_id"),
        "owner_session_kind": row.get("owner_session_kind"),
        "owner_session_policy": row.get("owner_session_policy"),
        "task_run": row.get("task_run"),
        "session_recommendation": row.get("session_recommendation") or {},
        "surface_bindings": row.get("surface_bindings") or [],
        "blockers": row.get("blockers") or [],
    }
    if workflow_state == "dispatched":
        view_row["reason"] = row.get("reason") or "Task exists without linked change evidence yet."
    return view_row


def _task_dag_view_rows(readiness: dict[str, Any]) -> list[dict[str, Any]]:
    return [_task_dag_view_row(row) for row in readiness.get("nodes") or [] if isinstance(row, dict)]


def _task_dag_workflow_summary(rows: list[dict[str, Any]], *, next_action: str | None = None) -> dict[str, Any]:
    counts = {"ready": 0, "running": 0, "blocked": 0, "completed": 0, "dispatched": 0, "total": 0}
    for row in rows:
        state = str(row.get("state") or "blocked")
        workflow_state = str(row.get("workflow_state") or state)
        counts["total"] += 1
        if workflow_state == "dispatched":
            counts["dispatched"] += 1
            continue
        if state in counts:
            counts[state] += 1
        if next_action is None and state == "ready":
            recommendation = row.get("session_recommendation") if isinstance(row.get("session_recommendation"), dict) else {}
            node_id = str(row.get("node_id") or "")
            action = str(recommendation.get("action") or "").strip()
            next_action = f"{action} {node_id}".strip() if action and node_id else action or None
        if next_action is None and state == "running":
            node_id = str(row.get("node_id") or "")
            next_action = f"continue {node_id}" if node_id else "continue workflow"
    if next_action is None:
        next_action = "complete task graph" if counts["blocked"] == 0 else None
    return {
        "ready_nodes": counts["ready"],
        "running_nodes": counts["running"],
        "blocked_nodes": counts["blocked"],
        "completed_nodes": counts["completed"],
        "dispatched_nodes": counts["dispatched"],
        "total_nodes": counts["total"],
        "next_action": next_action,
    }


def _task_dag_ready_nodes(readiness: dict[str, Any]) -> list[dict[str, Any]]:
    return [row for row in _task_dag_view_rows(readiness) if row.get("state") == "ready" and row.get("workflow_state") == "ready"]


def _task_dag_takeover_dispatched_nodes(readiness: dict[str, Any]) -> list[dict[str, Any]]:
    nodes: list[dict[str, Any]] = []
    for row in _task_dag_view_rows(readiness):
        workflow_state = str(row.get("workflow_state") or "")
        recommendation = row.get("session_recommendation") if isinstance(row.get("session_recommendation"), dict) else {}
        action = str(recommendation.get("action") or "").strip()
        if workflow_state == "dispatched" and action == "resume_or_claim":
            nodes.append(row)
    return nodes


def _task_dag_execution_only_node_ids(graph: dict[str, Any]) -> list[str]:
    converged_output_node_ids = set(_task_dag_converged_output_node_ids(graph))
    safety_boundary_node_ids = set(_task_dag_safety_boundary_node_ids(graph))
    node_ids: list[str] = []
    for node in graph.get("nodes", []):
        if not isinstance(node, dict) or str(node.get("node_kind") or "") != "task":
            continue
        node_id = str(node.get("node_id") or "")
        if not node_id or node_id in converged_output_node_ids or node_id in safety_boundary_node_ids:
            continue
        node_ids.append(node_id)
    return node_ids


def _task_dag_node_workflow_boundary(node: Mapping[str, Any]) -> str:
    boundary = str(node.get("workflow_boundary") or "reviewable_output").strip().lower()
    if boundary in {"execution_only", "reviewable_output"}:
        return boundary
    return "reviewable_output"


def _task_dag_successor_ids(graph: dict[str, Any]) -> dict[str, set[str]]:
    successors: dict[str, set[str]] = {
        str(node.get("node_id")): set()
        for node in graph.get("nodes", [])
        if isinstance(node, dict) and str(node.get("node_id") or "").strip()
    }
    for edge in graph.get("edges") or []:
        if not isinstance(edge, dict):
            continue
        from_node = str(edge.get("from") or "").strip()
        to_node = str(edge.get("to") or "").strip()
        if from_node and to_node and from_node in successors:
            successors[from_node].add(to_node)
    return successors


def _task_dag_converged_output_node_ids(graph: dict[str, Any]) -> list[str]:
    explicit = [
        str(node.get("node_id") or "")
        for node in graph.get("nodes", [])
        if isinstance(node, dict) and bool(node.get("converged_output"))
    ]
    if explicit:
        return explicit
    successors = _task_dag_successor_ids(graph)
    task_nodes = {
        str(node.get("node_id") or ""): node
        for node in graph.get("nodes", [])
        if isinstance(node, dict) and str(node.get("node_kind") or "") == "task"
    }
    reviewable_terminal = []
    fallback_terminal = []
    for node_id in topological_node_order(graph):
        node = task_nodes.get(node_id)
        if node is None:
            continue
        task_successors = [dep for dep in successors.get(node_id, set()) if dep in task_nodes]
        if task_successors:
            continue
        fallback_terminal.append(node_id)
        if _task_dag_node_workflow_boundary(node) != "execution_only":
            reviewable_terminal.append(node_id)
    return reviewable_terminal or fallback_terminal


def _task_dag_safety_boundary_node_ids(graph: dict[str, Any]) -> list[str]:
    node_ids = []
    for node in graph.get("nodes", []):
        if not isinstance(node, dict):
            continue
        node_id = str(node.get("node_id") or "").strip()
        if not node_id:
            continue
        if str(node.get("node_kind") or "") in {"gate_node", "land_gate"} or bool(node.get("safety_boundary")):
            node_ids.append(node_id)
    return node_ids


def _task_dag_auto_bootstrap_ready_node_ids(
    graph: dict[str, Any],
    readiness: dict[str, Any],
    profile: dict[str, Any],
) -> list[str]:
    graph_nodes = {
        str(node.get("node_id") or ""): node
        for node in graph.get("nodes", [])
        if isinstance(node, dict)
    }
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
    if not bool(profile.get("final_remote_disposition_default")):
        converged_output_node_ids = {
            str(node_id)
            for node_id in (profile.get("converged_output_node_ids") or [])
            if str(node_id)
        }
        candidate_node_ids = [
            node_id for node_id in candidate_node_ids if node_id not in converged_output_node_ids
        ]
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
    return [
        node_id
        for node_id in candidate_node_ids
        if node_id not in active_bootstrapped
    ][:available_worker_slots]


def _task_dag_auto_continue_profile(graph: dict[str, Any]) -> dict[str, Any]:
    policy = graph.get("execution_policy") if isinstance(graph.get("execution_policy"), dict) else {}
    configured_strategy = str(policy.get("gate_strategy") or policy.get("solo_gate_strategy") or "").strip().lower()
    enabled = any(
        isinstance(node, dict) and str(node.get("node_kind") or "") == "task"
        for node in graph.get("nodes", [])
    )
    promotion_policy = build_task_dag_promotion_policy(graph)
    final_land_disposition = str(promotion_policy.get("final_land_disposition") or "remote").strip().lower()
    final_gate_bundle = (
        [
            str(value).strip().lower()
            for value in (policy.get("final_gate_bundle") or DEFAULT_TASK_DAG_FINAL_GATE_BUNDLE)
            if str(value).strip()
        ]
        if final_land_disposition == "remote"
        else []
    )
    return {
        "change_strategy": promotion_policy.get("change_strategy"),
        "final_land_disposition": final_land_disposition,
        "final_remote_disposition_default": _task_dag_final_remote_disposition_default(graph),
        "auto_continue_supported": enabled,
        "auto_node_bootstrap_supported": enabled,
        "gate_strategy": (configured_strategy or TASK_DAG_SOLO_GATE_STRATEGY) if enabled else None,
        "final_gate_bundle": final_gate_bundle if enabled else [],
        "execution_only_node_ids": _task_dag_execution_only_node_ids(graph) if enabled else [],
        "converged_output_node_ids": _task_dag_converged_output_node_ids(graph) if enabled else [],
        "safety_boundary_node_ids": _task_dag_safety_boundary_node_ids(graph) if enabled else [],
    }


def _task_dag_graph_artifact_path(graph: dict[str, Any]) -> str | None:
    dispatch = graph.get("dispatch_artifacts") if isinstance(graph.get("dispatch_artifacts"), dict) else {}
    explicit = str(dispatch.get("task_graph_json") or "").strip()
    if explicit:
        return explicit
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    artifact_path = str(source_plan.get("artifact_path") or "").strip()
    if artifact_path.lower().endswith(".md"):
        return str(Path(artifact_path).with_suffix(".task_graph.json"))
    return artifact_path or None


def _task_dag_state_bucket_ids(readiness: dict[str, Any]) -> dict[str, list[str]]:
    buckets = {
        "ready_node_ids": [],
        "running_node_ids": [],
        "blocked_node_ids": [],
        "completed_node_ids": [],
        "dispatched_node_ids": [],
    }
    for row in _task_dag_view_rows(readiness):
        node_id = str(row.get("node_id") or "").strip()
        if not node_id:
            continue
        state = str(row.get("state") or "blocked")
        workflow_state = str(row.get("workflow_state") or state)
        if state == "ready":
            buckets["ready_node_ids"].append(node_id)
        elif state == "running":
            buckets["running_node_ids"].append(node_id)
        elif state == "blocked":
            buckets["blocked_node_ids"].append(node_id)
        elif state == "completed":
            buckets["completed_node_ids"].append(node_id)
        if workflow_state == "dispatched":
            buckets["dispatched_node_ids"].append(node_id)
    return buckets


def _task_dag_execute_state_digest(snapshot: dict[str, Any]) -> str:
    gate_handoff = snapshot.get("gate_handoff") if isinstance(snapshot.get("gate_handoff"), dict) else {}
    canonical = {
        "plan_revision_id": snapshot.get("plan_revision_id"),
        "graph_id": snapshot.get("graph_id"),
        "execution_state": snapshot.get("execution_state"),
        "change_strategy": snapshot.get("change_strategy"),
        "final_land_disposition": snapshot.get("final_land_disposition"),
        "final_remote_disposition_default": bool(snapshot.get("final_remote_disposition_default")),
        "pause_reason": snapshot.get("pause_reason"),
        "next_action": snapshot.get("next_action"),
        "ready_node_ids": snapshot.get("ready_node_ids") or [],
        "running_node_ids": snapshot.get("running_node_ids") or [],
        "blocked_node_ids": snapshot.get("blocked_node_ids") or [],
        "completed_node_ids": snapshot.get("completed_node_ids") or [],
        "dispatched_node_ids": snapshot.get("dispatched_node_ids") or [],
        "gate_handoff": {
            "kind": gate_handoff.get("kind"),
            "candidate_node_ids": gate_handoff.get("candidate_node_ids") or [],
            "candidate_change_ids": gate_handoff.get("candidate_change_ids") or [],
            "required_gates": gate_handoff.get("required_gates") or [],
            "promotion_required": gate_handoff.get("promotion_required"),
            "local_convergence_state": gate_handoff.get("local_convergence_state"),
            "final_remote_land_ready": gate_handoff.get("final_remote_land_ready"),
            "final_local_land_ready": gate_handoff.get("final_local_land_ready"),
        }
        if gate_handoff
        else None,
    }
    return hashlib.sha1(json.dumps(canonical, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()


def _task_dag_execute_state_from_snapshot(snapshot: dict[str, Any]) -> str:
    gate_handoff = snapshot.get("gate_handoff") if isinstance(snapshot.get("gate_handoff"), dict) else {}
    handoff_kind = str(gate_handoff.get("kind") or "").strip()
    if handoff_kind == "safety_boundary":
        return "paused_for_safety_boundary"
    if handoff_kind == "converged_gate_bundle":
        return "paused_for_converged_gate_bundle"
    workflow_summary = snapshot.get("workflow_summary") if isinstance(snapshot.get("workflow_summary"), dict) else {}
    total_nodes = int(workflow_summary.get("total_nodes") or 0)
    ready_count = len(snapshot.get("ready_node_ids") or [])
    running_count = len(snapshot.get("running_node_ids") or [])
    blocked_count = len(snapshot.get("blocked_node_ids") or [])
    completed_count = len(snapshot.get("completed_node_ids") or [])
    if total_nodes and completed_count >= total_nodes and ready_count == 0 and running_count == 0 and blocked_count == 0:
        return "completed"
    if ready_count > 0 or running_count > 0:
        return "active"
    if blocked_count > 0:
        return "waiting_for_dependency_completion"
    return "active"


def _task_dag_node_rows_by_id(readiness: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        str(row.get("node_id") or ""): row
        for row in _task_dag_view_rows(readiness)
        if str(row.get("node_id") or "").strip()
    }


def _task_dag_gate_handoff_payload(
    *,
    graph: dict[str, Any],
    readiness: dict[str, Any],
    snapshot: dict[str, Any],
    profile: dict[str, Any],
) -> dict[str, Any] | None:
    if not profile.get("auto_continue_supported"):
        return None
    rows_by_id = _task_dag_node_rows_by_id(readiness)
    ready = {str(node_id) for node_id in snapshot.get("ready_node_ids") or [] if str(node_id)}
    running = {str(node_id) for node_id in snapshot.get("running_node_ids") or [] if str(node_id)}
    completed = {str(node_id) for node_id in snapshot.get("completed_node_ids") or [] if str(node_id)}
    node_index = {str(node.get("node_id") or ""): node for node in graph.get("nodes", []) if isinstance(node, dict)}

    for node_id in profile.get("safety_boundary_node_ids") or []:
        row = rows_by_id.get(node_id) or {}
        if node_id in completed:
            continue
        blockers = row.get("blockers") if isinstance(row.get("blockers"), list) else []
        dependency_only_blocked = bool(blockers) and all(
            isinstance(blocker, dict) and str(blocker.get("type") or "").strip() == "dependency"
            for blocker in blockers
        )
        if node_id not in ready and node_id not in running and not (
            str(row.get("state") or "") == "blocked" and not dependency_only_blocked
        ):
            continue
        graph_node = node_index.get(node_id) or {}
        reason = str(
            graph_node.get("safety_boundary_reason")
            or row.get("reason")
            or f"Safety boundary {node_id} requires operator review."
        ).strip()
        return {
            "kind": "safety_boundary",
            "candidate_node_ids": [node_id],
            "candidate_task_ids": [str(row.get("task_id") or "")] if row.get("task_id") else [],
            "candidate_change_ids": [str(row.get("change_id") or "")] if row.get("change_id") else [],
            "required_gates": [],
            "promotion_required": False,
            "reason": reason,
            "next_action": f"inspect safety boundary {node_id}",
        }

    converged_node_ids = [node_id for node_id in profile.get("converged_output_node_ids") or [] if node_id in completed]
    if not converged_node_ids:
        return None

    candidate_task_ids: list[str] = []
    candidate_change_ids: list[str] = []
    promotion_required = False
    unsatisfied_nodes = []
    for node_id in converged_node_ids:
        row = rows_by_id.get(node_id) or {}
        lineage = _task_dag_node_lineage(row)
        task_id = str(lineage.get("task_id") or "").strip()
        change_id = str(lineage.get("change_id") or "").strip()
        landed_snapshot_id = str(lineage.get("landed_snapshot_id") or "").strip()
        if task_id:
            candidate_task_ids.append(task_id)
        if change_id:
            candidate_change_ids.append(change_id)
        if not landed_snapshot_id:
            unsatisfied_nodes.append(node_id)
        if not change_id:
            promotion_required = True

    if not unsatisfied_nodes:
        return None

    final_land_disposition = str(profile.get("final_land_disposition") or "remote").strip().lower()
    next_action = (
        "create converged reviewable output"
        if promotion_required
        else "run converged output gate bundle"
        if final_land_disposition == "remote"
        else "run converged local land"
    )
    return {
        "kind": "converged_gate_bundle",
        "candidate_node_ids": converged_node_ids,
        "candidate_task_ids": sorted(dict.fromkeys(candidate_task_ids)),
        "candidate_change_ids": sorted(dict.fromkeys(candidate_change_ids)),
        "required_gates": list(profile.get("final_gate_bundle") or []),
        "promotion_required": promotion_required,
        "final_land_disposition": final_land_disposition,
        "local_convergence_state": (
            "converged_output_ready_for_promotion"
            if promotion_required
            else "converged_output_ready_for_local_land"
            if final_land_disposition == "local"
            else "converged_output_ready_for_remote_gate_bundle"
        ),
        "final_remote_land_ready": bool(
            final_land_disposition == "remote" and not promotion_required and candidate_change_ids
        ),
        "final_local_land_ready": bool(final_land_disposition == "local" and not promotion_required and candidate_change_ids),
        "next_action": next_action,
    }


def _task_dag_state_snapshot_payload(
    *,
    plan_id: str,
    plan_revision_id: str | None,
    graph: dict[str, Any],
    readiness: dict[str, Any],
    graph_run_id: str,
) -> dict[str, Any]:
    profile = _task_dag_auto_continue_profile(graph)
    workflow_summary = _task_dag_workflow_summary(
        _task_dag_view_rows(readiness),
        next_action=(readiness.get("summary") or {}).get("next_action") if isinstance(readiness.get("summary"), dict) else None,
    )
    readiness_summary = readiness.get("summary") if isinstance(readiness.get("summary"), dict) else {}
    snapshot = {
        "graph_run_id": graph_run_id,
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id,
        "graph_id": str(graph.get("graph_id") or ""),
        "graph_artifact_path": _task_dag_graph_artifact_path(graph),
        "workflow_summary": workflow_summary,
        "readiness_summary": readiness_summary,
        **_task_dag_state_bucket_ids(readiness),
        "next_action": workflow_summary.get("next_action"),
        "change_strategy": profile.get("change_strategy"),
        "final_land_disposition": profile.get("final_land_disposition"),
        "final_remote_disposition_default": bool(profile.get("final_remote_disposition_default")),
        "auto_continue_supported": bool(profile.get("auto_continue_supported")),
        "auto_node_bootstrap_supported": bool(profile.get("auto_node_bootstrap_supported")),
        "gate_strategy": profile.get("gate_strategy"),
        "final_gate_bundle": list(profile.get("final_gate_bundle") or []),
        "execution_only_node_ids": list(profile.get("execution_only_node_ids") or []),
        "converged_output_node_ids": list(profile.get("converged_output_node_ids") or []),
        "safety_boundary_node_ids": list(profile.get("safety_boundary_node_ids") or []),
    }
    gate_handoff = _task_dag_gate_handoff_payload(graph=graph, readiness=readiness, snapshot=snapshot, profile=profile)
    if gate_handoff:
        snapshot["gate_handoff"] = gate_handoff
        snapshot["pause_reason"] = gate_handoff.get("kind")
        snapshot["next_action"] = gate_handoff.get("next_action") or snapshot.get("next_action")
    snapshot["execution_state"] = _task_dag_execute_state_from_snapshot(snapshot)
    snapshot["readiness_digest"] = _task_dag_execute_state_digest(snapshot)
    return snapshot


def _task_dag_execute_run_summary(session: dict[str, Any], events: list[dict[str, Any]]) -> dict[str, Any]:
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    latest_event = events[-1] if events else None
    state_snapshot = None
    for event in reversed(events):
        if str(event.get("event_type") or "").strip() == "task_graph.state_snapshot":
            state_snapshot = event
            break
    state_payload = state_snapshot.get("payload") if isinstance(state_snapshot, dict) and isinstance(state_snapshot.get("payload"), dict) else {}
    latest_payload = latest_event.get("payload") if isinstance(latest_event, dict) and isinstance(latest_event.get("payload"), dict) else {}
    summary = latest_payload.get("workflow_summary") if isinstance(latest_payload.get("workflow_summary"), dict) else {}
    if not summary and isinstance(state_payload.get("workflow_summary"), dict):
        summary = state_payload.get("workflow_summary")
    return {
        "session_id": session.get("session_id"),
        "session_kind": session.get("session_kind") or "task_graph_run",
        "graph_run_id": metadata.get("graph_run_id"),
        "execution_state": str(state_payload.get("execution_state") or metadata.get("execution_state") or "active"),
        "status": session.get("status"),
        "title": session.get("title"),
        "created_at": session.get("created_at"),
        "updated_at": session.get("updated_at"),
        "metadata": metadata,
        "event_count": len(events),
        "latest_event_type": latest_event.get("event_type") if isinstance(latest_event, dict) else None,
        "latest_event_sequence": latest_event.get("sequence") if isinstance(latest_event, dict) else None,
        "latest_state_snapshot": state_payload if state_payload else None,
        "workflow_summary": summary or None,
    }


def _task_dag_graph_run_id(plan_id: str, graph_id: str) -> str:
    return f"graph-run-{hashlib.sha1(f'{plan_id}:{graph_id}'.encode('utf-8')).hexdigest()[:12]}"


def _task_dag_target_line_name(
    ctx: ServerContext,
    repo_name: str,
    graph: dict[str, Any],
    graph_node: Mapping[str, Any],
) -> str:
    policy = graph.get("execution_policy") if isinstance(graph.get("execution_policy"), Mapping) else {}
    template = graph_node.get("task_template") if isinstance(graph_node.get("task_template"), Mapping) else {}
    repository = get_content_repository(ctx, repo_name)
    candidates = [
        str(template.get("target_line") or "").strip() or None,
        str(graph_node.get("target_line") or "").strip() or None,
        str(policy.get("target_line") or "").strip() or None,
        str(repository.get("default_line") or "").strip() or None,
        "main",
    ]
    return next((candidate for candidate in candidates if candidate), "main")


def _task_dag_create_task_for_node(
    ctx: ServerContext,
    repo_name: str,
    *,
    plan_id: str,
    plan_revision_id: str | None,
    graph_node: dict[str, Any],
    actor_identity: str,
    actor_type: str,
) -> dict[str, Any]:
    template = graph_node.get("task_template") if isinstance(graph_node.get("task_template"), dict) else {}
    node_id = str(graph_node.get("node_id") or "")
    title = str(template.get("title") or graph_node.get("title") or f"Task DAG node {node_id}")
    risk_tier = str(template.get("risk_tier") or "medium")
    plan_item_ref = str(graph_node.get("plan_item_ref") or "")
    intent = str(template.get("intent") or f"Execute Task DAG node {node_id} for plan item {plan_item_ref or 'unbound'}.")
    return create_task(
        ctx,
        repo_name,
        title,
        intent,
        risk_tier,
        plan_id=plan_id,
        origin_plan_revision_id=plan_revision_id,
        plan_item_ref=plan_item_ref or None,
        actor_identity=actor_identity,
        actor_type=actor_type,
    )


def _task_dag_view_row_by_node_id(readiness: dict[str, Any], node_id: str) -> dict[str, Any] | None:
    for row in _task_dag_view_rows(readiness):
        if str(row.get("node_id") or "") == node_id:
            return row
    return None


def _task_dag_bootstrap_node(
    ctx: ServerContext,
    repo_name: str,
    *,
    plan_id: str,
    plan_revision_id: str | None,
    graph: dict[str, Any],
    readiness: dict[str, Any],
    node_id: str,
    graph_run_session_id: str | None,
    actor_identity: str,
    actor_type: str,
    allow_execution_only_without_change: bool = False,
) -> dict[str, Any]:
    graph_nodes = {str(node.get("node_id") or ""): node for node in graph.get("nodes", []) if isinstance(node, dict)}
    graph_node = graph_nodes.get(str(node_id))
    if not isinstance(graph_node, dict):
        raise ValueError(f"Unknown node id: {node_id}")
    if str(graph_node.get("node_kind") or "") != "task":
        raise ValueError(f"Node {node_id} is not a task node.")
    row = _task_dag_view_row_by_node_id(readiness, str(node_id))
    if row is None:
        raise ValueError(f"No readiness evidence found for node {node_id}.")
    template = graph_node.get("task_template") if isinstance(graph_node.get("task_template"), dict) else {}

    task_id = str(row.get("task_id") or "").strip()
    if not task_id:
        task = _task_dag_create_task_for_node(
            ctx,
            repo_name,
            plan_id=plan_id,
            plan_revision_id=plan_revision_id,
            graph_node=graph_node,
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        task_id = str(task.get("task_id") or "")

    change_id = str(row.get("change_id") or "").strip()
    created_change = None
    converged_output_node_ids = {
        str(candidate)
        for candidate in (_task_dag_converged_output_node_ids(graph) or [])
        if str(candidate)
    }
    shared_boundary_node = str(node_id) in converged_output_node_ids
    workflow_boundary = "reviewable_output" if shared_boundary_node else "execution_only"
    skip_change = not shared_boundary_node
    promotion_policy = build_task_dag_promotion_policy(graph, [row])
    final_remote_disposition_default = bool(promotion_policy.get("final_remote_disposition_default"))
    local_first_execution_only = not shared_boundary_node or not final_remote_disposition_default
    if change_id and not shared_boundary_node:
        raise ValueError(
            f"Node {node_id} is not the final converged DAG node and cannot carry a shared / remote change under the single-path DAG contract."
        )
    if shared_boundary_node and not final_remote_disposition_default:
        raise ValueError(
            f"Node {node_id} is a local-final converged DAG output and must be materialized by the local compact-worker runtime, not server-side bootstrap."
        )
    if not change_id and not skip_change:
        change_title = str(template.get("change_title") or graph_node.get("title") or f"Task DAG node {node_id}")
        risk_tier = str(template.get("risk_tier") or "medium")
        target_line = _task_dag_target_line_name(ctx, repo_name, graph, graph_node)
        created_change = create_change(
            ctx,
            repo_name,
            task_id,
            change_title,
            base_line=target_line,
            risk_tier=risk_tier,
            forked_from_line=target_line,
        )
        change_id = str(created_change.get("change_id") or "")

    session_metadata: dict[str, Any] = {
        "session_policy": "task_dag_node_bootstrap",
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id,
        "graph_id": str(graph.get("graph_id") or ""),
        "task_graph_json": _task_dag_graph_artifact_path(graph),
        "node_id": str(node_id),
        "plan_item_ref": str(graph_node.get("plan_item_ref") or ""),
        "task_id": task_id,
        "change_id": change_id,
        "bootstrap_state": "bootstrapped",
        "workflow_boundary": workflow_boundary,
        "promotion_mode": "execution_only_local_first" if local_first_execution_only else "reviewable_remote_change",
        "local_lineage_allowed": local_first_execution_only,
        "remote_change_required": not local_first_execution_only,
        "remote_workflow_allowed": shared_boundary_node,
        "single_path_dag": True,
        "dag_shared_boundary_node": shared_boundary_node,
        "change_strategy": promotion_policy.get("change_strategy"),
        "final_remote_disposition_default": bool(promotion_policy.get("final_remote_disposition_default")),
        "graph_run_session_id": graph_run_session_id or None,
    }
    if graph_run_session_id:
        run_session = get_session(ctx, graph_run_session_id)
        run_metadata = run_session.get("metadata") if isinstance(run_session.get("metadata"), dict) else {}
        graph_run_id = str(run_metadata.get("graph_run_id") or "").strip()
        if graph_run_id:
            session_metadata["graph_run_id"] = graph_run_id

    session = create_session(
        ctx,
        repo_name,
        "agent_run",
        task_id=task_id,
        change_id=change_id or None,
        title=f"Task DAG node {node_id}: {graph_node.get('title') or template.get('title') or task_id}",
        metadata=session_metadata,
        actor_identity=actor_identity,
        actor_type=actor_type,
    )
    if graph_run_session_id:
        append_session_event(
            ctx,
            graph_run_session_id,
            "task_graph.node_bootstrapped",
            {
                "node_id": str(node_id),
                "task_id": task_id,
                "change_id": change_id,
                "session_id": session.get("session_id"),
            },
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
    return {
        "node_id": str(node_id),
        "plan_item_ref": graph_node.get("plan_item_ref"),
        "task_id": task_id,
        "change_id": change_id,
        "workflow_boundary": workflow_boundary,
        "created_change": created_change,
        "session": {
            "session_id": session.get("session_id"),
            "session_kind": session.get("session_kind") or "agent_run",
            "title": session.get("title"),
            "metadata": session_metadata,
        },
    }


def advance_task_dag_run(
    ctx: ServerContext,
    session_id: str,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
) -> dict[str, Any]:
    from .read_models import task_dag_readiness

    if not session_id:
        raise ValueError("Guarded execute-run advance requires a graph-run session id.")
    session = get_session(ctx, session_id)
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    plan_id = str(source_plan.get("plan_id") or metadata.get("plan_id") or "").strip()
    graph_id = str(graph.get("graph_id") or "")
    session_plan_id = str(metadata.get("plan_id") or "").strip()
    session_graph_id = str(metadata.get("graph_id") or "").strip()
    if session_plan_id and plan_id and session_plan_id != plan_id:
        raise ValueError(f"Session {session_id} belongs to plan {session_plan_id}, not {plan_id}.")
    if session_graph_id and session_graph_id != graph_id:
        raise ValueError(f"Session {session_id} belongs to graph {session_graph_id}, not {graph_id}.")

    events = list_session_events(ctx, session_id)
    prior_summary = _task_dag_execute_run_summary(session, events)
    previous_snapshot = prior_summary.get("latest_state_snapshot") if isinstance(prior_summary.get("latest_state_snapshot"), dict) else {}
    previous_digest = str(previous_snapshot.get("readiness_digest") or "").strip()
    if not previous_digest and previous_snapshot:
        previous_digest = _task_dag_execute_state_digest(previous_snapshot)

    plan_revision_id = current_plan_revision_id or str(source_plan.get("plan_revision_id") or metadata.get("plan_revision_id") or "").strip() or None
    graph_run_id = str(metadata.get("graph_run_id") or previous_snapshot.get("graph_run_id") or "").strip() or _task_dag_graph_run_id(plan_id, graph_id)

    readiness = task_dag_readiness(ctx, graph, current_plan_revision_id=plan_revision_id)
    profile = _task_dag_auto_continue_profile(graph)
    auto_bootstrapped_node_ids: list[str] = []
    if profile.get("auto_continue_supported"):
        auto_bootstrap_ready_node_ids = _task_dag_auto_bootstrap_node_ids_for_run(
            graph=graph,
            readiness=readiness,
            profile=profile,
            session_metadata=metadata,
            events=events,
        )
        for ready_node_id in auto_bootstrap_ready_node_ids:
            _task_dag_bootstrap_node(
                ctx,
                str(session.get("repo_name") or ""),
                plan_id=plan_id,
                plan_revision_id=plan_revision_id,
                graph=graph,
                readiness=readiness,
                node_id=ready_node_id,
                graph_run_session_id=session_id,
                actor_identity=actor_identity,
                actor_type=actor_type,
                allow_execution_only_without_change=True,
            )
            auto_bootstrapped_node_ids.append(ready_node_id)
        if auto_bootstrapped_node_ids:
            readiness = task_dag_readiness(ctx, graph, current_plan_revision_id=plan_revision_id)

    current_snapshot = _task_dag_state_snapshot_payload(
        plan_id=plan_id,
        plan_revision_id=plan_revision_id,
        graph=graph,
        readiness=readiness,
        graph_run_id=graph_run_id,
    )
    current_digest = str(current_snapshot.get("readiness_digest") or "").strip()

    previous_completed = {str(node_id) for node_id in previous_snapshot.get("completed_node_ids") or [] if str(node_id)}
    current_completed = {str(node_id) for node_id in current_snapshot.get("completed_node_ids") or [] if str(node_id)}
    completed_regressions = sorted(previous_completed - current_completed)
    if completed_regressions:
        raise ValueError(f"Guarded execute-run advance detected completed-node regression for {', '.join(completed_regressions)}.")

    if previous_digest and previous_digest == current_digest and not auto_bootstrapped_node_ids:
        return {
            **prior_summary,
            "graph_artifact_path": _task_dag_graph_artifact_path(graph),
            "current_readiness_summary": readiness.get("summary") or {},
            "advanced": False,
            "execution_state": (previous_snapshot or current_snapshot).get("execution_state"),
            "workflow_summary": (previous_snapshot or current_snapshot).get("workflow_summary") or {},
            "noop_reason": "readiness state unchanged",
            "previous_state_snapshot": previous_snapshot or None,
            "latest_state_snapshot": previous_snapshot or current_snapshot,
            "auto_bootstrapped_node_ids": [],
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

    advance_event = append_session_event(
        ctx,
        session_id,
        "task_graph.execution_advanced",
        {
            "graph_run_id": graph_run_id,
            "plan_id": plan_id,
            "plan_revision_id": plan_revision_id,
            "graph_id": graph_id,
            "graph_artifact_path": _task_dag_graph_artifact_path(graph),
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
        },
        actor_identity=actor_identity,
        actor_type=actor_type,
    )
    state_snapshot_event = append_session_event(
        ctx,
        session_id,
        "task_graph.state_snapshot",
        current_snapshot,
        actor_identity=actor_identity,
        actor_type=actor_type,
    )
    refreshed_session = get_session(ctx, session_id)
    return {
        **_task_dag_execute_run_summary(refreshed_session, [*events, advance_event, state_snapshot_event]),
        "graph_artifact_path": _task_dag_graph_artifact_path(graph),
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
    }
