from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

from ..plan_graph import topological_node_order
from ..store import RepoContext
from ..task_dag_readiness import (
    build_task_dag_promotion_policy,
    build_task_graph_execution_strategy,
)
from .task_dag_readiness_views import (
    _task_dag_blocked_nodes,
    _task_dag_node_index,
    _task_dag_node_lineage,
    _task_dag_progress_payload,
    _task_dag_ready_nodes,
    _task_dag_view_rows,
    _task_dag_workflow_summary,
)
from .task_dag_runtime_helpers import _task_dag_relative_path
from .task_dag_topology_helpers import _task_dag_node_workflow_boundary


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)

def _task_dag_lineage_row(row: dict[str, Any]) -> dict[str, Any]:
    lineage = _task_dag_node_lineage(row)
    return {
        "node_id": row.get("node_id"),
        "node_kind": row.get("node_kind"),
        "title": row.get("title"),
        "plan_item_ref": row.get("plan_item_ref"),
        "state": row.get("state"),
        "workflow_state": row.get("workflow_state"),
        "depends_on": row.get("depends_on") or [],
        "lock_keys": row.get("lock_keys") or [],
        "hotspot_keys": row.get("hotspot_keys") or [],
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
        "session_recommendation": row.get("session_recommendation") or {},
        "surface_bindings": row.get("surface_bindings") or [],
        "reason": row.get("reason"),
    }


def _task_dag_graph_payload(plan_id: str, graph: dict[str, Any], graph_path: Path, readiness: dict[str, Any], ctx: RepoContext) -> dict[str, Any]:
    relative_path_fn = _app_override("_task_dag_relative_path", _task_dag_relative_path)
    view_rows = _task_dag_view_rows(readiness)
    node_rows = [_task_dag_lineage_row(row) for row in view_rows]
    if not node_rows:
        graph_nodes = _task_dag_node_index(graph)
        order = topological_node_order(graph)
        node_rows = [
            {
                "node_id": node_id,
                "title": graph_nodes[node_id].get("title"),
                "plan_item_ref": graph_nodes[node_id].get("plan_item_ref"),
                "state": None,
                "depends_on": graph_nodes[node_id].get("depends_on") or [],
                "lock_keys": graph_nodes[node_id].get("lock_keys") or [],
                "hotspot_keys": graph_nodes[node_id].get("hotspot_keys") or [],
            }
            for node_id in order
        ]
    return {
        "plan_id": plan_id,
        "graph_id": graph.get("graph_id"),
        "graph_artifact_path": relative_path_fn(ctx, graph_path),
        "source_plan": graph.get("source_plan") or {},
        "summary": readiness.get("summary") or {},
        "readiness_summary": readiness.get("summary") or {},
        "workflow_summary": _task_dag_workflow_summary(
            view_rows,
            next_action=(readiness.get("summary") or {}).get("next_action") if isinstance(readiness.get("summary"), dict) else None,
        ),
        "nodes": node_rows,
        "edges": graph.get("edges") or [],
    }


def _task_dag_progress_summary_payload(plan_id: str, graph: dict[str, Any], graph_path: Path, readiness: dict[str, Any], ctx: RepoContext) -> dict[str, Any]:
    relative_path_fn = _app_override("_task_dag_relative_path", _task_dag_relative_path)
    progress = _task_dag_progress_payload(graph, readiness)
    view_rows = _task_dag_view_rows(readiness)
    blockers = []
    for row in _task_dag_blocked_nodes(readiness):
        blockers.append(
            {
                "node_id": row.get("node_id"),
                "title": row.get("title"),
                "reason": row.get("reason"),
                "blockers": row.get("blockers") or [],
            }
        )
    return {
        "plan_id": plan_id,
        "graph_id": graph.get("graph_id"),
        "graph_artifact_path": relative_path_fn(ctx, graph_path),
        "readiness_summary": readiness.get("summary") or {},
        "workflow_summary": _task_dag_workflow_summary(
            view_rows,
            next_action=(readiness.get("summary") or {}).get("next_action") if isinstance(readiness.get("summary"), dict) else None,
        ),
        "progress": progress,
        "blockers": blockers,
    }


def _task_dag_dispatchable_rows(
    readiness: dict[str, Any],
    graph: dict[str, Any],
    limit: int,
    *,
    materialize_all_nodes: bool = False,
    workflow_mode: str | None = None,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    build_promotion_policy_fn = _app_override("build_task_dag_promotion_policy", build_task_dag_promotion_policy)
    build_execution_strategy_fn = _app_override("build_task_graph_execution_strategy", build_task_graph_execution_strategy)
    workflow_boundary_fn = _app_override("_task_dag_node_workflow_boundary", _task_dag_node_workflow_boundary)
    graph_nodes = _task_dag_node_index(graph)
    dispatchable: list[dict[str, Any]] = []
    skipped: list[dict[str, Any]] = []
    ready_nodes = _task_dag_ready_nodes(readiness)
    rows = _task_dag_view_rows(readiness) if materialize_all_nodes else ready_nodes
    promotion_policy = build_promotion_policy_fn(
        graph,
        ready_nodes,
        workflow_mode=workflow_mode,
    )
    selective_promotion_default = bool(promotion_policy.get("selective_promotion_default"))
    execution_strategy = build_execution_strategy_fn(graph, ready_nodes)
    cost_policy = execution_strategy.get("cost_policy") if isinstance(execution_strategy.get("cost_policy"), dict) else {}
    for row in rows:
        node_id = str(row.get("node_id") or "")
        graph_node = graph_nodes.get(node_id)
        lineage = row.get("lineage") if isinstance(row.get("lineage"), dict) else {}
        if lineage.get("task_id"):
            skipped.append({"node_id": node_id, "reason": "node already has a linked task", "task_id": lineage.get("task_id")})
            continue
        if not isinstance(graph_node, dict) or graph_node.get("node_kind") != "task":
            skipped.append({"node_id": node_id, "reason": "node is not a task node"})
            continue
        if not isinstance(graph_node.get("task_template"), dict):
            skipped.append({"node_id": node_id, "reason": "task node has no task_template"})
            continue
        if selective_promotion_default and workflow_boundary_fn(graph_node) == "execution_only":
            skipped.append(
                {
                    "node_id": node_id,
                    "reason": "execution-only node stays local-first, may keep local lineage, and waits for a reviewable output or explicit shared gate",
                    "workflow_boundary": "execution_only",
                    "selective_promotion": "local_first",
                }
            )
            continue
        state = str(row.get("state") or "blocked")
        if materialize_all_nodes and state not in {"ready", "blocked"}:
            skipped.append({"node_id": node_id, "reason": f"node state {state!r} does not need task materialization"})
            continue
        if materialize_all_nodes and state == "blocked" and cost_policy.get("ready_wave_only_recommended"):
            skipped.append(
                {
                    "node_id": node_id,
                    "reason": "cost policy kept remote materialization on the current ready wave",
                    "cost_policy_rule": "packet_ready_wave_only",
                }
            )
            continue
        dispatchable.append({"readiness": row, "graph_node": graph_node})
        if limit and len(dispatchable) >= limit:
            break
    return dispatchable, skipped
