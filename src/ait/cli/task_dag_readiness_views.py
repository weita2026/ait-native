from __future__ import annotations

import sys
from pathlib import Path
from typing import Any, Iterable, Mapping

from ..plan_graph import build_task_graph_progress
from ..store import RepoContext
from ..task_dag_readiness import (
    build_task_dag_promotion_policy,
    build_task_graph_execution_strategy,
)
from .task_dag_runtime_helpers import _task_dag_relative_path
from .workflow_mode_config import _effective_workflow_mode


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)


def _task_dag_node_index(graph: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {str(node.get("node_id")): node for node in graph.get("nodes", []) if isinstance(node, dict)}


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


def _task_dag_view_row_by_node_id(readiness: dict[str, Any], node_id: str) -> dict[str, Any] | None:
    for row in _task_dag_view_rows(readiness):
        if str(row.get("node_id") or "") == node_id:
            return row
    return None


def _task_dag_change_focus_policy(
    rows: Iterable[Mapping[str, Any]] | None,
    *,
    execution_only_node_ids: Iterable[str] | None = None,
) -> dict[str, Any]:
    excluded_node_ids = {
        str(node_id).strip()
        for node_id in (execution_only_node_ids or [])
        if str(node_id).strip()
    }
    state_priority = {
        "running": 0,
        "claimed": 1,
        "ready": 2,
        "review": 3,
        "landable": 4,
        "dispatched": 5,
    }
    seen: set[tuple[str, str]] = set()
    queued: list[tuple[int, int, dict[str, Any]]] = []
    for index, row in enumerate(rows or []):
        if not isinstance(row, Mapping):
            continue
        node_id = str(row.get("node_id") or "").strip()
        if not node_id or node_id in excluded_node_ids:
            continue
        state = str(row.get("state") or "").strip().lower()
        workflow_state = str(row.get("workflow_state") or state).strip().lower()
        priority = state_priority.get(workflow_state, state_priority.get(state))
        if priority is None:
            continue
        change_id = str(row.get("change_id") or "").strip()
        identity = ("change", change_id) if change_id else ("node", node_id)
        if identity in seen:
            continue
        seen.add(identity)
        queued.append(
            (
                priority,
                index,
                {
                    "focus_unit": "change" if change_id else "node",
                    "node_id": node_id,
                    "plan_item_ref": str(row.get("plan_item_ref") or "").strip() or None,
                    "title": str(row.get("title") or "").strip() or None,
                    "task_id": str(row.get("task_id") or "").strip() or None,
                    "change_id": change_id or None,
                    "patchset_id": str(row.get("patchset_id") or "").strip() or None,
                    "state": state or None,
                    "workflow_state": workflow_state or None,
                },
            )
        )
    focus_queue = [item for _, _, item in sorted(queued, key=lambda row: (row[0], row[1]))]
    next_focus = dict(focus_queue[0]) if focus_queue else None
    return {
        "mode": "single_worker_per_change_patchset",
        "single_worker_session": True,
        "single_active_focus": True,
        "focus_unit": "change",
        "focus_unit_fallback": "node",
        "cut_trigger": "reviewable_boundary",
        "patchset_cut_required": True,
        "patchset_publish_before_next_focus": True,
        "shared_reviewable_diff_allowed": False,
        "focus_queue": focus_queue,
        "next_focus": next_focus,
    }


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
        elif state == "ready":
            counts["ready"] += 1
        elif state == "running":
            counts["running"] += 1
        elif state == "blocked":
            counts["blocked"] += 1
        elif state == "completed":
            counts["completed"] += 1
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


def _task_dag_progress_payload(graph: dict[str, Any], readiness: dict[str, Any]) -> dict[str, Any]:
    summary = readiness.get("summary") if isinstance(readiness.get("summary"), dict) else {}
    view_rows = _task_dag_view_rows(readiness)
    build_progress_fn = _app_override("build_task_graph_progress", build_task_graph_progress)
    return build_progress_fn(
        graph,
        {row["node_id"]: {"state": row["state"], "reason": row.get("reason")} for row in view_rows if row.get("node_id")},
        next_action=summary.get("next_action"),
    )


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


def _task_dag_dispatched_nodes(readiness: dict[str, Any]) -> list[dict[str, Any]]:
    return [row for row in _task_dag_view_rows(readiness) if row.get("workflow_state") == "dispatched"]


def _task_dag_running_nodes(readiness: dict[str, Any]) -> list[dict[str, Any]]:
    return [row for row in _task_dag_view_rows(readiness) if row.get("state") == "running" and row.get("workflow_state") == "running"]


def _task_dag_blocked_nodes(readiness: dict[str, Any]) -> list[dict[str, Any]]:
    return [row for row in _task_dag_view_rows(readiness) if row.get("state") == "blocked"]


def _task_dag_completed_nodes(readiness: dict[str, Any]) -> list[dict[str, Any]]:
    return [row for row in _task_dag_view_rows(readiness) if row.get("state") == "completed"]


def _task_dag_schedule_payload(plan_id: str, graph: dict[str, Any], graph_path: Path, readiness: dict[str, Any], ctx: RepoContext) -> dict[str, Any]:
    readiness_relative_path_fn = _app_override("_task_dag_relative_path", _task_dag_relative_path)
    effective_workflow_mode_fn = _app_override("_effective_workflow_mode", _effective_workflow_mode)
    ready_nodes = _task_dag_ready_nodes(readiness)
    dispatched_nodes = _task_dag_dispatched_nodes(readiness)
    running_nodes = _task_dag_running_nodes(readiness)
    blocked_nodes = _task_dag_blocked_nodes(readiness)
    promotion_policy = build_task_dag_promotion_policy(
        graph,
        ready_nodes,
        workflow_mode=str((effective_workflow_mode_fn(ctx) or {}).get("value") or "custom"),
    )
    ready_actions = []
    for row in ready_nodes:
        node_id = str(row.get("node_id") or "")
        ready_actions.append(
            {
                "node_id": node_id,
                "title": row.get("title"),
                "plan_item_ref": row.get("plan_item_ref"),
                "node_kind": row.get("node_kind"),
                "state": row.get("state"),
                "workflow_state": row.get("workflow_state"),
                "reason": row.get("reason"),
                "task_id": row.get("task_id"),
                "change_id": row.get("change_id"),
                "patchset_id": row.get("patchset_id"),
                "lock_keys": row.get("lock_keys") or [],
                "hotspot_keys": row.get("hotspot_keys") or [],
                "session_recommendation": row.get("session_recommendation") or {},
                "command": f"ait plan execute {plan_id} --from-json {readiness_relative_path_fn(ctx, graph_path)} --auto-compact-worker --yes",
            }
        )
    dispatched_actions = []
    for row in dispatched_nodes:
        dispatched_actions.append(
            {
                "node_id": row.get("node_id"),
                "title": row.get("title"),
                "plan_item_ref": row.get("plan_item_ref"),
                "state": row.get("state"),
                "workflow_state": row.get("workflow_state"),
                "reason": row.get("reason"),
                "task_id": row.get("task_id"),
                "change_id": row.get("change_id"),
                "session_recommendation": row.get("session_recommendation") or {},
            }
        )
    running_actions = []
    for row in running_nodes:
        running_actions.append(
            {
                "node_id": row.get("node_id"),
                "title": row.get("title"),
                "plan_item_ref": row.get("plan_item_ref"),
                "node_kind": row.get("node_kind"),
                "state": row.get("state"),
                "workflow_state": row.get("workflow_state"),
                "reason": row.get("reason"),
                "task_id": row.get("task_id"),
                "change_id": row.get("change_id"),
                "patchset_id": row.get("patchset_id"),
                "lock_keys": row.get("lock_keys") or [],
                "hotspot_keys": row.get("hotspot_keys") or [],
                "session_id": row.get("session_id"),
                "task_run_id": row.get("task_run_id"),
                "surface_bindings": row.get("surface_bindings") or [],
                "session_recommendation": row.get("session_recommendation") or {},
                "action": (row.get("session_recommendation") or {}).get("action") or "continue active workflow evidence",
            }
        )
    return {
        "plan_id": plan_id,
        "graph_id": graph.get("graph_id"),
        "graph_artifact_path": readiness_relative_path_fn(ctx, graph_path),
        "execution_strategy": build_task_graph_execution_strategy(graph, ready_nodes),
        "promotion_policy": promotion_policy,
        "summary": readiness.get("summary") or {},
        "readiness_summary": readiness.get("summary") or {},
        "workflow_summary": _task_dag_workflow_summary(
            _task_dag_view_rows(readiness),
            next_action=(readiness.get("summary") or {}).get("next_action") if isinstance(readiness.get("summary"), dict) else None,
        ),
        "ready": ready_actions,
        "dispatched": dispatched_actions,
        "running": running_actions,
        "blocked": [
            {
                "node_id": row.get("node_id"),
                "title": row.get("title"),
                "reason": row.get("reason"),
                "blockers": row.get("blockers") or [],
                "state": row.get("state"),
                "workflow_state": row.get("workflow_state"),
                "session_recommendation": row.get("session_recommendation") or {},
            }
            for row in blocked_nodes
        ],
    }
