from __future__ import annotations

import hashlib
import json
import sys
import time
from pathlib import Path
from typing import Any

from ait_protocol.common import utc_now

from ..remote_client import list_sessions as remote_list_sessions
from ..store import RepoContext
from .task_dag_readiness_views import (
    _task_dag_change_focus_policy,
    _task_dag_node_index,
    _task_dag_node_lineage,
    _task_dag_view_rows,
    _task_dag_workflow_summary,
)
from .task_dag_runtime_helpers import _task_dag_relative_path


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)


def _app_time_ns() -> int:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is not None:
        app_time = getattr(app_module, "time", None)
        time_ns = getattr(app_time, "time_ns", None)
        if callable(time_ns):
            return int(time_ns())
    return time.time_ns()


def _task_dag_graph_run_id(plan_id: str, graph_id: str) -> str:
    utc_now_fn = _app_override("utc_now", utc_now)
    seed = f"{plan_id}:{graph_id}:{utc_now_fn()}:{_app_time_ns()}".encode("utf-8")
    return f"graph-run-{hashlib.sha1(seed).hexdigest()[:12]}"


def _task_dag_state_bucket_ids(readiness: dict[str, Any]) -> dict[str, list[str]]:
    view_rows_fn = _app_override("_task_dag_view_rows", _task_dag_view_rows)
    buckets = {
        "ready_node_ids": [],
        "running_node_ids": [],
        "blocked_node_ids": [],
        "completed_node_ids": [],
        "dispatched_node_ids": [],
    }
    for row in view_rows_fn(readiness):
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
    encoded = json.dumps(canonical, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha1(encoded).hexdigest()


def _task_dag_execute_state_from_snapshot(snapshot: dict[str, Any]) -> str:
    gate_handoff = snapshot.get("gate_handoff") if isinstance(snapshot.get("gate_handoff"), dict) else {}
    handoff_kind = str(gate_handoff.get("kind") or "").strip()
    if handoff_kind == "safety_boundary":
        return "paused_for_safety_boundary"
    if handoff_kind == "converged_gate_bundle":
        return "paused_for_converged_gate_bundle"
    total_nodes = 0
    workflow_summary = snapshot.get("workflow_summary")
    if isinstance(workflow_summary, dict):
        try:
            total_nodes = int(workflow_summary.get("total_nodes") or 0)
        except (TypeError, ValueError):
            total_nodes = 0
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
    view_rows_fn = _app_override("_task_dag_view_rows", _task_dag_view_rows)
    return {
        str(row.get("node_id") or ""): row
        for row in view_rows_fn(readiness)
        if str(row.get("node_id") or "").strip()
    }


def _task_dag_gate_handoff_payload(
    *,
    graph: dict[str, Any],
    readiness: dict[str, Any],
    snapshot: dict[str, Any],
    profile: dict[str, Any],
) -> dict[str, Any] | None:
    node_rows_by_id_fn = _app_override("_task_dag_node_rows_by_id", _task_dag_node_rows_by_id)
    node_index_fn = _app_override("_task_dag_node_index", _task_dag_node_index)
    node_lineage_fn = _app_override("_task_dag_node_lineage", _task_dag_node_lineage)
    if not profile.get("auto_continue_supported"):
        return None
    rows_by_id = node_rows_by_id_fn(readiness)
    ready = {str(node_id) for node_id in snapshot.get("ready_node_ids") or [] if str(node_id)}
    running = {str(node_id) for node_id in snapshot.get("running_node_ids") or [] if str(node_id)}
    completed = {str(node_id) for node_id in snapshot.get("completed_node_ids") or [] if str(node_id)}

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
        graph_node = node_index_fn(graph).get(node_id) or {}
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
        lineage = node_lineage_fn(row)
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
    ctx: RepoContext,
    plan_id: str,
    plan_revision_id: str | None,
    graph: dict[str, Any],
    graph_path: Path,
    readiness: dict[str, Any],
    graph_run_id: str,
) -> dict[str, Any]:
    auto_continue_profile_fn = _app_override("_task_dag_auto_continue_profile", None)
    if auto_continue_profile_fn is None:
        raise RuntimeError("ait.cli.app did not expose _task_dag_auto_continue_profile.")
    workflow_summary_fn = _app_override("_task_dag_workflow_summary", _task_dag_workflow_summary)
    view_rows_fn = _app_override("_task_dag_view_rows", _task_dag_view_rows)
    state_bucket_ids_fn = _app_override("_task_dag_state_bucket_ids", _task_dag_state_bucket_ids)
    change_focus_policy_fn = _app_override("_task_dag_change_focus_policy", _task_dag_change_focus_policy)
    gate_handoff_payload_fn = _app_override("_task_dag_gate_handoff_payload", _task_dag_gate_handoff_payload)
    execute_state_from_snapshot_fn = _app_override("_task_dag_execute_state_from_snapshot", _task_dag_execute_state_from_snapshot)
    execute_state_digest_fn = _app_override("_task_dag_execute_state_digest", _task_dag_execute_state_digest)
    relative_path_fn = _app_override("_task_dag_relative_path", _task_dag_relative_path)

    profile = auto_continue_profile_fn(ctx, graph)
    workflow_summary = workflow_summary_fn(
        view_rows_fn(readiness),
        next_action=(readiness.get("summary") or {}).get("next_action") if isinstance(readiness.get("summary"), dict) else None,
    )
    readiness_summary = readiness.get("summary") if isinstance(readiness.get("summary"), dict) else {}
    snapshot = {
        "graph_run_id": graph_run_id,
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id,
        "graph_id": str(graph.get("graph_id") or ""),
        "graph_artifact_path": relative_path_fn(ctx, graph_path),
        "workflow_summary": workflow_summary,
        "readiness_summary": readiness_summary,
        **state_bucket_ids_fn(readiness),
        "next_action": workflow_summary.get("next_action"),
    }
    snapshot["workflow_mode"] = profile.get("workflow_mode")
    snapshot["change_strategy"] = profile.get("change_strategy")
    snapshot["final_land_disposition"] = profile.get("final_land_disposition")
    snapshot["final_remote_disposition_default"] = bool(profile.get("final_remote_disposition_default"))
    snapshot["auto_continue_supported"] = bool(profile.get("auto_continue_supported"))
    snapshot["auto_node_bootstrap_supported"] = bool(profile.get("auto_node_bootstrap_supported"))
    snapshot["gate_strategy"] = profile.get("gate_strategy")
    snapshot["final_gate_bundle"] = list(profile.get("final_gate_bundle") or [])
    snapshot["execution_only_node_ids"] = list(profile.get("execution_only_node_ids") or [])
    snapshot["converged_output_node_ids"] = list(profile.get("converged_output_node_ids") or [])
    snapshot["safety_boundary_node_ids"] = list(profile.get("safety_boundary_node_ids") or [])
    change_focus_policy = change_focus_policy_fn(
        view_rows_fn(readiness),
        execution_only_node_ids=profile.get("execution_only_node_ids") or [],
    )
    snapshot["change_focus_policy"] = change_focus_policy
    snapshot["next_focus_node_id"] = (
        (change_focus_policy.get("next_focus") or {}).get("node_id")
        if isinstance(change_focus_policy.get("next_focus"), dict)
        else None
    )
    snapshot["next_focus_task_id"] = (
        (change_focus_policy.get("next_focus") or {}).get("task_id")
        if isinstance(change_focus_policy.get("next_focus"), dict)
        else None
    )
    snapshot["next_focus_change_id"] = (
        (change_focus_policy.get("next_focus") or {}).get("change_id")
        if isinstance(change_focus_policy.get("next_focus"), dict)
        else None
    )
    gate_handoff = gate_handoff_payload_fn(
        graph=graph,
        readiness=readiness,
        snapshot=snapshot,
        profile=profile,
    )
    if gate_handoff:
        snapshot["gate_handoff"] = gate_handoff
        snapshot["pause_reason"] = gate_handoff.get("kind")
        snapshot["next_action"] = gate_handoff.get("next_action") or snapshot.get("next_action")
    snapshot["execution_state"] = execute_state_from_snapshot_fn(snapshot)
    snapshot["readiness_digest"] = execute_state_digest_fn(snapshot)
    return snapshot


def _task_dag_graph_run_session_rows(
    *,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
) -> list[dict[str, Any]]:
    remote_list_sessions_fn = _app_override("remote_list_sessions", remote_list_sessions)
    graph_id = str(graph.get("graph_id") or "")
    graph_plan_item_refs = {
        str(node.get("plan_item_ref") or "").strip()
        for node in graph.get("nodes") or []
        if isinstance(node, dict) and str(node.get("plan_item_ref") or "").strip()
    }
    rows = []
    for session in remote_list_sessions_fn(remote_row["url"], repo_name):
        if str(session.get("session_kind") or "").strip() != "task_graph_run":
            continue
        metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
        session_plan_id = str(metadata.get("plan_id") or "").strip()
        session_graph_id = str(metadata.get("graph_id") or "").strip()
        session_plan_item_ref = str(metadata.get("plan_item_ref") or "").strip()
        if session_plan_id != plan_id:
            continue
        if session_graph_id and session_graph_id == graph_id:
            rows.append(session)
            continue
        if session_plan_item_ref and session_plan_item_ref in graph_plan_item_refs:
            rows.append(session)
    return rows


def _task_dag_latest_execute_run_session(
    *,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
) -> dict[str, Any] | None:
    graph_run_session_rows_fn = _app_override("_task_dag_graph_run_session_rows", _task_dag_graph_run_session_rows)
    rows = graph_run_session_rows_fn(
        remote_row=remote_row,
        repo_name=repo_name,
        plan_id=plan_id,
        graph=graph,
    )
    return rows[0] if rows else None


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
    execution_state = str(state_payload.get("execution_state") or metadata.get("execution_state") or "active")
    return {
        "session_id": session.get("session_id"),
        "session_kind": session.get("session_kind") or session.get("kind") or "task_graph_run",
        "graph_run_id": metadata.get("graph_run_id"),
        "execution_state": execution_state,
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
