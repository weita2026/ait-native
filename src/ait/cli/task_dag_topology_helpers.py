from __future__ import annotations

import sys
from typing import Any, Mapping

from ..plan_graph import topological_node_order


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)


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
    workflow_boundary_fn = _app_override("_task_dag_node_workflow_boundary", _task_dag_node_workflow_boundary)
    successor_ids_fn = _app_override("_task_dag_successor_ids", _task_dag_successor_ids)
    explicit = [
        str(node.get("node_id") or "")
        for node in graph.get("nodes", [])
        if isinstance(node, dict) and bool(node.get("converged_output"))
    ]
    if explicit:
        return explicit
    successors = successor_ids_fn(graph)
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
        if workflow_boundary_fn(node) != "execution_only":
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


def _task_dag_execution_only_node_ids(graph: dict[str, Any]) -> list[str]:
    converged_output_node_ids_fn = _app_override("_task_dag_converged_output_node_ids", _task_dag_converged_output_node_ids)
    safety_boundary_node_ids_fn = _app_override("_task_dag_safety_boundary_node_ids", _task_dag_safety_boundary_node_ids)
    converged_output_node_ids = set(converged_output_node_ids_fn(graph))
    safety_boundary_node_ids = set(safety_boundary_node_ids_fn(graph))
    node_ids: list[str] = []
    for node in graph.get("nodes", []):
        if not isinstance(node, dict) or str(node.get("node_kind") or "") != "task":
            continue
        node_id = str(node.get("node_id") or "")
        if not node_id or node_id in converged_output_node_ids or node_id in safety_boundary_node_ids:
            continue
        node_ids.append(node_id)
    return node_ids
