from __future__ import annotations

import json
import math
from pathlib import Path
from typing import Any, Mapping


REQUIRED_TOP_LEVEL_FIELDS = frozenset(
    {
        "schema_version",
        "graph_id",
        "source_plan",
        "nodes",
        "edges",
        "execution_policy",
    }
)
SUPPORTED_NODE_KINDS = frozenset({"task", "gate_node", "land_gate"})
SUPPORTED_EDGE_KINDS = frozenset({"depends_on", "land_after", "gate_after"})
SUPPORTED_WORKFLOW_BOUNDARIES = frozenset({"execution_only", "reviewable_output"})
SUPPORTED_SOLO_GATE_STRATEGIES = frozenset({"end_of_dag_gate_concentration"})
SUPPORTED_FINAL_GATE_ACTIONS = frozenset({"review", "attestation", "policy", "land"})
SUPPORTED_EXECUTION_POLICY_MODES = frozenset({"guarded_full_dag_convergence"})
SUPPORTED_WORKER_EXECUTION_MODES = frozenset({"worker_only_compact_packet"})
HOTSPOT_PREFIXES = ("file:", "dir:", "module:", "lane:", "contract:", "line:")
LOCK_KEY_PREFIXES = HOTSPOT_PREFIXES


class TaskGraphValidationError(ValueError):
    """Raised when a Task DAG graph JSON artifact is structurally invalid."""


TERMINAL_PROGRESS_STATES = frozenset({"completed", "landed", "superseded"})
PARTIAL_PROGRESS_STATES = frozenset({"claimed", "running", "review", "landable"})
COUNTED_PROGRESS_STATES = frozenset({"blocked", "ready", "running", "completed"})
DEFAULT_PROGRESS_FRACTIONS: dict[str, float] = {
    "blocked": 0.0,
    "ready": 0.0,
    "claimed": 0.1,
    "running": 0.35,
    "review": 0.65,
    "landable": 0.9,
    "completed": 1.0,
    "landed": 1.0,
    "superseded": 1.0,
}


def load_task_graph(path: str | Path) -> dict[str, Any]:
    graph_path = Path(path)
    try:
        data = json.loads(graph_path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise TaskGraphValidationError(f"Unable to read task graph JSON: {graph_path}") from exc
    except json.JSONDecodeError as exc:
        raise TaskGraphValidationError(f"Invalid task graph JSON in {graph_path}: {exc.msg}") from exc
    return validate_task_graph(data)


def validate_task_graph(data: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(data, dict):
        raise TaskGraphValidationError("Task graph must be a JSON object.")

    missing_fields = sorted(REQUIRED_TOP_LEVEL_FIELDS - set(data))
    if missing_fields:
        raise TaskGraphValidationError(f"Task graph is missing required field(s): {', '.join(missing_fields)}")

    nodes = data.get("nodes")
    edges = data.get("edges")
    source_plan = data.get("source_plan")
    execution_policy = data.get("execution_policy")
    dispatch_artifacts = data.get("dispatch_artifacts")
    if not isinstance(nodes, list):
        raise TaskGraphValidationError("Task graph field `nodes` must be a list.")
    if not isinstance(edges, list):
        raise TaskGraphValidationError("Task graph field `edges` must be a list.")
    if not isinstance(source_plan, dict):
        raise TaskGraphValidationError("Task graph field `source_plan` must be an object.")
    if not isinstance(execution_policy, dict):
        raise TaskGraphValidationError("Task graph field `execution_policy` must be an object.")
    if dispatch_artifacts is not None:
        _validate_dispatch_artifacts(dispatch_artifacts)

    node_ids: set[str] = set()
    for index, node in enumerate(nodes):
        if not isinstance(node, dict):
            raise TaskGraphValidationError(f"Task graph node at index {index} must be an object.")
        node_id = _required_text(node, "node_id", f"Task graph node at index {index}")
        if node_id in node_ids:
            raise TaskGraphValidationError(f"Duplicate task graph node_id: {node_id}")
        node_ids.add(node_id)
        node_kind = _required_text(node, "node_kind", f"Task graph node {node_id}")
        if node_kind not in SUPPORTED_NODE_KINDS:
            raise TaskGraphValidationError(
                f"Task graph node {node_id} has unsupported node_kind: {node_kind}. "
                f"Expected one of: {', '.join(sorted(SUPPORTED_NODE_KINDS))}."
            )
        workflow_boundary = node.get("workflow_boundary")
        if workflow_boundary is not None:
            if not isinstance(workflow_boundary, str) or workflow_boundary.strip() not in SUPPORTED_WORKFLOW_BOUNDARIES:
                raise TaskGraphValidationError(
                    f"Task graph node {node_id} workflow_boundary must be one of: "
                    f"{', '.join(sorted(SUPPORTED_WORKFLOW_BOUNDARIES))}."
                )
        for field in ("converged_output", "safety_boundary"):
            value = node.get(field)
            if value is not None and not isinstance(value, bool):
                raise TaskGraphValidationError(f"Task graph node {node_id} {field} must be a boolean when present.")
        safety_boundary_reason = node.get("safety_boundary_reason")
        if safety_boundary_reason is not None and (
            not isinstance(safety_boundary_reason, str) or not safety_boundary_reason.strip()
        ):
            raise TaskGraphValidationError(
                f"Task graph node {node_id} safety_boundary_reason must be a non-empty string when present."
            )
        _validate_task_node_template(node_id, node_kind, node.get("task_template"))
        depends_on = node.get("depends_on", [])
        if depends_on is None:
            depends_on = []
        if not isinstance(depends_on, list):
            raise TaskGraphValidationError(f"Task graph node {node_id} depends_on must be a list.")
        for dependency in depends_on:
            if not isinstance(dependency, str) or not dependency.strip():
                raise TaskGraphValidationError(f"Task graph node {node_id} has an invalid dependency.")
        _validate_key_list(node, "hotspot_keys", "hotspot key", HOTSPOT_PREFIXES)
        _validate_key_list(node, "lock_keys", "lock key", LOCK_KEY_PREFIXES)
        _node_progress_weight(node, data.get("progress_model") if isinstance(data.get("progress_model"), dict) else {})

    for index, edge in enumerate(edges):
        if not isinstance(edge, dict):
            raise TaskGraphValidationError(f"Task graph edge at index {index} must be an object.")
        from_node = _required_text(edge, "from", f"Task graph edge at index {index}")
        to_node = _required_text(edge, "to", f"Task graph edge at index {index}")
        edge_kind = str(edge.get("edge_kind") or "depends_on").strip()
        if edge_kind not in SUPPORTED_EDGE_KINDS:
            raise TaskGraphValidationError(f"Task graph edge {from_node}->{to_node} has unsupported edge_kind: {edge_kind}")
        if from_node not in node_ids:
            raise TaskGraphValidationError(f"Task graph edge references unknown source node: {from_node}")
        if to_node not in node_ids:
            raise TaskGraphValidationError(f"Task graph edge references unknown target node: {to_node}")

    for node in nodes:
        node_id = _required_text(node, "node_id", "Task graph node")
        for dependency in node.get("depends_on") or []:
            if dependency not in node_ids:
                raise TaskGraphValidationError(f"Task graph node {node_id} depends on unknown node: {dependency}")

    _validate_source_plan(source_plan)
    _validate_execution_policy(execution_policy)
    topological_node_order(data)
    _validate_single_path_dag_contract(data)
    _validate_lock_conflicts(data)
    return data


def _validate_source_plan(source_plan: Mapping[str, Any]) -> None:
    for field in ("artifact_path", "plan_id", "plan_revision_id"):
        value = source_plan.get(field)
        if not isinstance(value, str) or not value.strip():
            raise TaskGraphValidationError(f"Task graph source_plan must include non-empty `{field}`.")


def _validate_execution_policy(execution_policy: Mapping[str, Any]) -> None:
    mode = execution_policy.get("mode")
    normalized_mode = None
    if mode is not None:
        if not isinstance(mode, str) or not mode.strip():
            raise TaskGraphValidationError("Task graph execution_policy.mode must be a non-empty string when present.")
        normalized_mode = mode.strip()
        if normalized_mode not in SUPPORTED_EXECUTION_POLICY_MODES:
            raise TaskGraphValidationError(
                "Task graph execution_policy.mode must be one of: "
                f"{', '.join(sorted(SUPPORTED_EXECUTION_POLICY_MODES))}."
            )
    for field in (
        "default_mode",
        "dispatch_model",
        "worker_execution_mode",
    ):
        value = execution_policy.get(field)
        if value is not None and (not isinstance(value, str) or not value.strip()):
            raise TaskGraphValidationError(f"Task graph execution_policy.{field} must be a non-empty string when present.")
    for retired_field in ("session_topology_default", "session_topology_modes", "coordinator_token_budget"):
        if retired_field in execution_policy:
            raise TaskGraphValidationError(
                f"Task graph execution_policy.{retired_field} is retired. "
                "Use worker-only compact packet execution fields instead."
            )
    for field in ("auto_create_tasks", "auto_claim_nodes", "auto_land", "requires_review_policy_land_gates", "validate_source_plan_revision"):
        value = execution_policy.get(field)
        if value is not None and not isinstance(value, bool):
            raise TaskGraphValidationError(f"Task graph execution_policy.{field} must be a boolean when present.")
    for field in ("local_execution_default", "local_first_final_land"):
        value = execution_policy.get(field)
        if value is not None and not isinstance(value, bool):
            raise TaskGraphValidationError(f"Task graph execution_policy.{field} must be a boolean when present.")
    for field in ("batch_node_target", "max_batch_sessions", "max_total_sessions", "max_worker_sessions"):
        value = execution_policy.get(field)
        if value is not None and (not isinstance(value, int) or value < 0):
            raise TaskGraphValidationError(f"Task graph execution_policy.{field} must be a non-negative integer when present.")
    default_mode = execution_policy.get("default_mode")
    if default_mode is not None and str(default_mode).strip() not in {
        "local_execution_dag_with_selective_promotion",
        "compact_packet_worker",
    }:
        raise TaskGraphValidationError(
            "Task graph execution_policy.default_mode must stay on the single-path DAG default "
            "`local_execution_dag_with_selective_promotion`."
        )
    dispatch_model = execution_policy.get("dispatch_model")
    if dispatch_model is not None and str(dispatch_model).strip() != "compact_packet":
        raise TaskGraphValidationError(
            "Task graph execution_policy.dispatch_model must stay on the single-path `compact_packet` route."
        )
    dag_default = execution_policy.get("dag_default")
    if dag_default is not None and str(dag_default).strip() != "local_execution_dag_with_selective_promotion":
        raise TaskGraphValidationError(
            "Task graph execution_policy.dag_default must stay on `local_execution_dag_with_selective_promotion`."
        )
    change_strategy = execution_policy.get("change_strategy")
    if change_strategy is not None and str(change_strategy).strip() not in {
        "local_first_final_remote_land",
        "local_first_final_local_land",
        "remote_backed_selective_promotion",
    }:
        raise TaskGraphValidationError(
            "Task graph execution_policy.change_strategy must stay on the single-path final-land route."
        )
    if execution_policy.get("local_execution_default") not in {None, True}:
        raise TaskGraphValidationError(
            "Task graph execution_policy.local_execution_default must stay true when present."
        )
    if execution_policy.get("local_first_final_land") not in {None, True}:
        raise TaskGraphValidationError(
            "Task graph execution_policy.local_first_final_land must stay true when present."
        )
    solo_gate_strategy = execution_policy.get("solo_gate_strategy")
    if solo_gate_strategy is not None:
        if not isinstance(solo_gate_strategy, str) or solo_gate_strategy.strip() not in SUPPORTED_SOLO_GATE_STRATEGIES:
            raise TaskGraphValidationError(
                "Task graph execution_policy.solo_gate_strategy must be one of: "
                f"{', '.join(sorted(SUPPORTED_SOLO_GATE_STRATEGIES))}."
            )
    gate_strategy = execution_policy.get("gate_strategy")
    if gate_strategy is not None and str(gate_strategy).strip() != "end_of_dag_gate_concentration":
        raise TaskGraphValidationError(
            "Task graph execution_policy.gate_strategy must stay on `end_of_dag_gate_concentration`."
        )
    physical_fanout_default = execution_policy.get("physical_fanout_default")
    if physical_fanout_default is not None:
        if not isinstance(physical_fanout_default, bool):
            raise TaskGraphValidationError(
                "Task graph execution_policy.physical_fanout_default must be a boolean when present."
            )
        if physical_fanout_default:
            raise TaskGraphValidationError(
                "Task graph execution_policy.physical_fanout_default=true is no longer supported; "
                "single-path DAGs always keep intermediate nodes local-first until the final converged output."
            )
    physical_fanout_requires_explicit_opt_in = execution_policy.get("physical_fanout_requires_explicit_opt_in")
    if physical_fanout_requires_explicit_opt_in is not None:
        if not isinstance(physical_fanout_requires_explicit_opt_in, bool):
            raise TaskGraphValidationError(
                "Task graph execution_policy.physical_fanout_requires_explicit_opt_in must be a boolean when present."
            )
        if not physical_fanout_requires_explicit_opt_in:
            raise TaskGraphValidationError(
                "Task graph execution_policy.physical_fanout_requires_explicit_opt_in must stay true when present."
            )
    if normalized_mode == "guarded_full_dag_convergence":
        if execution_policy.get("worker_execution_mode") is None:
            raise TaskGraphValidationError(
                "Task graph execution_policy.worker_execution_mode is required when "
                "execution_policy.mode is guarded_full_dag_convergence."
            )
    worker_execution_mode = execution_policy.get("worker_execution_mode")
    if (
        worker_execution_mode is not None
        and str(worker_execution_mode).strip() not in SUPPORTED_WORKER_EXECUTION_MODES
    ):
        raise TaskGraphValidationError(
            "Task graph execution_policy.worker_execution_mode must be one of: "
            f"{', '.join(sorted(SUPPORTED_WORKER_EXECUTION_MODES))}."
        )
    max_total_sessions = execution_policy.get("max_total_sessions")
    max_worker_sessions = execution_policy.get("max_worker_sessions")
    if isinstance(max_total_sessions, int) and isinstance(max_worker_sessions, int):
        if max_total_sessions and max_worker_sessions > max_total_sessions:
            raise TaskGraphValidationError(
                "Task graph execution_policy.max_worker_sessions cannot exceed max_total_sessions."
            )
    final_gate_bundle = execution_policy.get("final_gate_bundle")
    if final_gate_bundle is not None:
        if not isinstance(final_gate_bundle, list):
            raise TaskGraphValidationError("Task graph execution_policy.final_gate_bundle must be a list when present.")
        for gate in final_gate_bundle:
            if not isinstance(gate, str) or gate.strip() not in SUPPORTED_FINAL_GATE_ACTIONS:
                raise TaskGraphValidationError(
                    "Task graph execution_policy.final_gate_bundle entries must be one of: "
                    f"{', '.join(sorted(SUPPORTED_FINAL_GATE_ACTIONS))}."
                )


def _task_successor_ids(data: Mapping[str, Any]) -> dict[str, set[str]]:
    successors: dict[str, set[str]] = {
        str(node.get("node_id")): set()
        for node in data.get("nodes", [])
        if isinstance(node, Mapping) and str(node.get("node_id") or "").strip()
    }
    for edge in data.get("edges") or []:
        if not isinstance(edge, Mapping):
            continue
        from_node = str(edge.get("from") or "").strip()
        to_node = str(edge.get("to") or "").strip()
        if from_node and to_node and from_node in successors:
            successors[from_node].add(to_node)
    return successors


def _validate_single_path_dag_contract(data: Mapping[str, Any]) -> None:
    task_nodes = {
        str(node.get("node_id") or ""): node
        for node in data.get("nodes", [])
        if isinstance(node, Mapping) and str(node.get("node_kind") or "") == "task"
    }
    if not task_nodes:
        return
    successors = _task_successor_ids(data)
    converged_output_node_ids = [node_id for node_id, node in task_nodes.items() if bool(node.get("converged_output"))]
    if len(converged_output_node_ids) > 1:
        raise TaskGraphValidationError(
            "Task graph may declare at most one converged_output task node for the single shared workflow boundary."
        )
    if converged_output_node_ids:
        converged_output_node_id = converged_output_node_ids[0]
    else:
        terminal_task_node_ids = [
            node_id
            for node_id in task_nodes
            if not [candidate for candidate in successors.get(node_id, set()) if candidate in task_nodes]
        ]
        if len(terminal_task_node_ids) != 1:
            raise TaskGraphValidationError(
                "Task graph must expose one terminal task node (or one explicit converged_output node) for the single shared workflow boundary."
            )
        converged_output_node_id = terminal_task_node_ids[0]
    for node_id, node in task_nodes.items():
        workflow_boundary = str(node.get("workflow_boundary") or "").strip().lower()
        task_successors = [candidate for candidate in successors.get(node_id, set()) if candidate in task_nodes]
        if node_id == converged_output_node_id:
            if workflow_boundary == "execution_only":
                raise TaskGraphValidationError(
                    f"Task graph converged_output node {node_id} cannot stay execution_only."
                )
            if task_successors:
                raise TaskGraphValidationError(
                    f"Task graph converged_output node {node_id} must be the final task boundary, not feed later task nodes."
                )
            continue
        if workflow_boundary == "reviewable_output":
            raise TaskGraphValidationError(
                f"Task graph node {node_id} is not the converged output and cannot declare workflow_boundary=reviewable_output."
            )
        if not task_successors:
            raise TaskGraphValidationError(
                f"Task graph node {node_id} is not the converged output and must feed another task node instead of terminating the DAG."
            )


def _validate_dispatch_artifacts(dispatch_artifacts: Any) -> None:
    if not isinstance(dispatch_artifacts, Mapping):
        raise TaskGraphValidationError("Task graph field `dispatch_artifacts` must be an object when present.")
    for field in ("source_markdown", "parallel_execution_markdown", "task_graph_json"):
        value = dispatch_artifacts.get(field)
        if not isinstance(value, str) or not value.strip():
            raise TaskGraphValidationError(f"Task graph dispatch_artifacts must include non-empty `{field}`.")


def _validate_task_node_template(node_id: str, node_kind: str, template: Any) -> None:
    if node_kind != "task":
        if template is not None and not isinstance(template, Mapping):
            raise TaskGraphValidationError(f"Task graph node {node_id} task_template must be an object when present.")
        return
    if not isinstance(template, Mapping):
        raise TaskGraphValidationError(f"Task graph task node {node_id} must include task_template.")
    title = template.get("title")
    if not isinstance(title, str) or not title.strip():
        raise TaskGraphValidationError(f"Task graph task node {node_id} task_template must include non-empty title.")
    risk_tier = template.get("risk_tier")
    if risk_tier is not None and (not isinstance(risk_tier, str) or not risk_tier.strip()):
        raise TaskGraphValidationError(f"Task graph task node {node_id} task_template.risk_tier must be a non-empty string when present.")


def _validate_key_list(
    node: Mapping[str, Any],
    field: str,
    label: str,
    supported_prefixes: tuple[str, ...],
) -> list[str]:
    node_id = str(node.get("node_id") or "")
    raw_values = node.get(field, [])
    if raw_values is None:
        raw_values = []
    if not isinstance(raw_values, list):
        raise TaskGraphValidationError(f"Task graph node {node_id} {field} must be a list.")
    values: list[str] = []
    for raw_value in raw_values:
        if not isinstance(raw_value, str) or not raw_value.strip():
            raise TaskGraphValidationError(f"Task graph node {node_id} has an invalid {label}.")
        value = raw_value.strip()
        if ":" not in value:
            raise TaskGraphValidationError(f"Task graph node {node_id} {label} must include a prefix: {value}")
        if not value.startswith(supported_prefixes):
            raise TaskGraphValidationError(f"Task graph node {node_id} {label} has unsupported prefix: {value}")
        values.append(value)
    return values


def _validate_lock_conflicts(data: dict[str, Any]) -> None:
    nodes = [node for node in data.get("nodes", []) if isinstance(node, Mapping)]
    task_nodes = [node for node in nodes if node.get("node_kind") == "task"]
    if len(task_nodes) < 2:
        return

    node_ids = {_required_text(dict(node), "node_id", "Task graph node") for node in nodes}
    dependency_pairs = _dependency_edge_pairs(data, node_ids)
    reachable = _reachable_nodes(node_ids, dependency_pairs)

    for left_index, left_node in enumerate(task_nodes):
        left_id = _required_text(dict(left_node), "node_id", "Task graph node")
        left_locks = set(_validate_key_list(left_node, "lock_keys", "lock key", LOCK_KEY_PREFIXES))
        if not left_locks:
            continue
        for right_node in task_nodes[left_index + 1:]:
            right_id = _required_text(dict(right_node), "node_id", "Task graph node")
            if right_id in reachable[left_id] or left_id in reachable[right_id]:
                continue
            shared_locks = sorted(left_locks & set(_validate_key_list(right_node, "lock_keys", "lock key", LOCK_KEY_PREFIXES)))
            if shared_locks:
                raise TaskGraphValidationError(
                    f"Task graph lock conflict on {shared_locks[0]} between independent nodes {left_id} and {right_id}. "
                    "Add a dependency edge or split the lock."
                )


def build_task_graph_progress(
    data: dict[str, Any],
    node_states: Mapping[str, str | Mapping[str, Any]] | None = None,
    *,
    next_action: str | None = None,
) -> dict[str, Any]:
    graph = validate_task_graph(data)
    raw_states = node_states or {}
    nodes = [node for node in graph.get("nodes", []) if isinstance(node, dict)]
    node_order = topological_node_order(graph)
    nodes_by_id = {_required_text(node, "node_id", "Task graph node"): node for node in nodes}
    progress_model = graph.get("progress_model") if isinstance(graph.get("progress_model"), dict) else {}
    fractions = _progress_state_fractions(progress_model)

    node_rows: dict[str, dict[str, Any]] = {}
    total_weight = 0.0
    completed_weight = 0.0
    estimated_weight = 0.0
    partial_estimate = False
    counts = {"completed": 0, "running": 0, "ready": 0, "blocked": 0}

    for node_id in node_order:
        node = nodes_by_id[node_id]
        weight = _node_progress_weight(node, progress_model)
        state_input = raw_states.get(node_id, "blocked")
        state_row = _normalize_progress_state(node_id, state_input, fractions)
        state = state_row["state"]
        count_state = _counted_progress_state(state)
        counts[count_state] += 1

        total_weight += weight
        state_fraction = fractions[state]
        estimated_weight += weight * state_fraction
        if state in TERMINAL_PROGRESS_STATES:
            completed_weight += weight
        elif state_fraction > 0:
            partial_estimate = True

        node_rows[node_id] = {
            "node_id": node_id,
            "state": state,
            "count_state": count_state,
            "progress_weight": _compact_number(weight),
            "estimated_fraction": state_fraction,
        }
        if state_row.get("reason"):
            node_rows[node_id]["reason"] = state_row["reason"]

    completed_percent = _floor_percent(completed_weight, total_weight)
    estimated_percent = _floor_percent(estimated_weight, total_weight) if partial_estimate else None

    resolved_next_action = next_action
    if resolved_next_action is None:
        resolved_next_action = _default_progress_next_action(node_order, node_rows)

    return {
        "graph_id": graph.get("graph_id"),
        "completed_percent": completed_percent,
        "estimated_percent": estimated_percent,
        "completed_nodes": counts["completed"],
        "running_nodes": counts["running"],
        "ready_nodes": counts["ready"],
        "blocked_nodes": counts["blocked"],
        "total_nodes": len(nodes),
        "completed_weight": _compact_number(completed_weight),
        "estimated_weight": _compact_number(estimated_weight),
        "total_weight": _compact_number(total_weight),
        "has_estimate": estimated_percent is not None,
        "next_action": resolved_next_action,
        "node_states": node_rows,
    }


def topological_node_order(data: dict[str, Any]) -> list[str]:
    nodes = data.get("nodes")
    edges = data.get("edges")
    if not isinstance(nodes, list) or not isinstance(edges, list):
        raise TaskGraphValidationError("Task graph must include list fields `nodes` and `edges`.")

    node_ids = [_required_text(node, "node_id", "Task graph node") for node in nodes if isinstance(node, dict)]
    node_set = set(node_ids)
    if len(node_ids) != len(node_set):
        raise TaskGraphValidationError("Task graph has duplicate node_id values.")

    adjacency: dict[str, list[str]] = {node_id: [] for node_id in node_ids}
    indegree: dict[str, int] = {node_id: 0 for node_id in node_ids}

    for from_node, to_node in sorted(_dependency_edge_pairs(data, node_set)):
        adjacency[from_node].append(to_node)
        indegree[to_node] += 1

    ready = sorted(node_id for node_id, count in indegree.items() if count == 0)
    ordered: list[str] = []
    while ready:
        node_id = ready.pop(0)
        ordered.append(node_id)
        for next_node in sorted(adjacency[node_id]):
            indegree[next_node] -= 1
            if indegree[next_node] == 0:
                ready.append(next_node)
                ready.sort()

    if len(ordered) != len(node_ids):
        cyclic_nodes = sorted(node_id for node_id, count in indegree.items() if count > 0)
        raise TaskGraphValidationError(f"Task graph contains a cycle involving: {', '.join(cyclic_nodes)}")
    return ordered


def _dependency_edge_pairs(data: Mapping[str, Any], node_set: set[str]) -> set[tuple[str, str]]:
    nodes = data.get("nodes")
    edges = data.get("edges")
    if not isinstance(nodes, list) or not isinstance(edges, list):
        raise TaskGraphValidationError("Task graph must include list fields `nodes` and `edges`.")

    pairs: set[tuple[str, str]] = set()
    for edge in edges:
        if not isinstance(edge, dict):
            raise TaskGraphValidationError("Task graph edge must be an object.")
        from_node = _required_text(edge, "from", "Task graph edge")
        to_node = _required_text(edge, "to", "Task graph edge")
        if from_node not in node_set:
            raise TaskGraphValidationError(f"Task graph edge references unknown source node: {from_node}")
        if to_node not in node_set:
            raise TaskGraphValidationError(f"Task graph edge references unknown target node: {to_node}")
        pairs.add((from_node, to_node))

    for node in nodes:
        if not isinstance(node, dict):
            continue
        node_id = _required_text(node, "node_id", "Task graph node")
        dependencies = node.get("depends_on") or []
        if not isinstance(dependencies, list):
            raise TaskGraphValidationError(f"Task graph node {node_id} depends_on must be a list.")
        for dependency in dependencies:
            if not isinstance(dependency, str) or not dependency.strip():
                raise TaskGraphValidationError(f"Task graph node {node_id} has an invalid dependency.")
            dependency_id = dependency.strip()
            if dependency_id not in node_set:
                raise TaskGraphValidationError(f"Task graph node {node_id} depends on unknown node: {dependency_id}")
            pairs.add((dependency_id, node_id))
    return pairs


def _reachable_nodes(node_ids: set[str], edge_pairs: set[tuple[str, str]]) -> dict[str, set[str]]:
    adjacency: dict[str, set[str]] = {node_id: set() for node_id in node_ids}
    for from_node, to_node in edge_pairs:
        adjacency.setdefault(from_node, set()).add(to_node)

    reachable: dict[str, set[str]] = {}
    for node_id in node_ids:
        seen: set[str] = set()
        pending = list(adjacency.get(node_id, set()))
        while pending:
            next_node = pending.pop()
            if next_node in seen:
                continue
            seen.add(next_node)
            pending.extend(adjacency.get(next_node, set()) - seen)
        reachable[node_id] = seen
    return reachable


def _required_text(row: dict[str, Any], field: str, label: str) -> str:
    value = row.get(field)
    if not isinstance(value, str) or not value.strip():
        raise TaskGraphValidationError(f"{label} must include non-empty `{field}`.")
    return value.strip()


def _progress_state_fractions(progress_model: Mapping[str, Any]) -> dict[str, float]:
    raw_fractions = progress_model.get("estimated_state_fractions")
    fractions = dict(DEFAULT_PROGRESS_FRACTIONS)
    if isinstance(raw_fractions, Mapping):
        for state, value in raw_fractions.items():
            if not isinstance(state, str) or not state.strip():
                raise TaskGraphValidationError("Task graph progress state fraction keys must be non-empty strings.")
            fraction = _number_value(value, f"Task graph progress state {state.strip()} fraction")
            if fraction < 0 or fraction > 1:
                raise TaskGraphValidationError(f"Task graph progress state {state.strip()} fraction must be between 0 and 1.")
            fractions[state.strip()] = fraction
    return fractions


def _node_progress_weight(node: Mapping[str, Any], progress_model: Mapping[str, Any]) -> float:
    node_id = str(node.get("node_id") or "")
    weight = node.get("progress_weight")
    if weight is None:
        node_kind = str(node.get("node_kind") or "")
        if node_kind == "gate_node":
            weight = progress_model.get("gate_node_default_weight", 0)
        elif node_kind == "land_gate":
            weight = progress_model.get("land_gate_default_weight", 0)
        else:
            weight = progress_model.get("default_node_weight", 1)
    numeric_weight = _number_value(weight, f"Task graph node {node_id} progress_weight")
    if numeric_weight < 0:
        raise TaskGraphValidationError(f"Task graph node {node_id} progress_weight must be non-negative.")
    return numeric_weight


def _normalize_progress_state(
    node_id: str,
    state_input: str | Mapping[str, Any],
    fractions: Mapping[str, float],
) -> dict[str, Any]:
    if isinstance(state_input, str):
        state = state_input.strip().lower()
        row: dict[str, Any] = {"state": state}
    elif isinstance(state_input, Mapping):
        row = dict(state_input)
        raw_state = row.get("state") or row.get("status")
        if raw_state is None and any(row.get(key) for key in ("session_id", "open_session", "session_state")):
            raw_state = "running"
        state = str(raw_state or "blocked").strip().lower()
    else:
        raise TaskGraphValidationError(f"Task graph node {node_id} progress state must be a string or object.")

    if state == "canceled":
        superseded = bool(row.get("superseded") or row.get("superseded_by") or row.get("superseded_by_node_id"))
        if superseded:
            state = "superseded"
        else:
            state = "blocked"
            row.setdefault("reason", "canceled_without_supersession")

    if state not in fractions:
        raise TaskGraphValidationError(f"Task graph node {node_id} has unsupported progress state: {state}")
    return {"state": state, "reason": row.get("reason")}


def _counted_progress_state(state: str) -> str:
    if state in TERMINAL_PROGRESS_STATES:
        return "completed"
    if state in PARTIAL_PROGRESS_STATES:
        return "running"
    if state in COUNTED_PROGRESS_STATES:
        return state
    return "blocked"


def _default_progress_next_action(node_order: list[str], node_rows: Mapping[str, Mapping[str, Any]]) -> str | None:
    for state, verb in (("ready", "start"), ("landable", "land"), ("review", "review"), ("running", "continue"), ("claimed", "continue"), ("blocked", "unblock")):
        for node_id in node_order:
            row = node_rows.get(node_id, {})
            if row.get("state") != state:
                continue
            if state == "blocked" and row.get("reason"):
                return f"{verb} {node_id}: {row['reason']}"
            return f"{verb} {node_id}"
    return None


def _floor_percent(value: float, total: float) -> int:
    if total <= 0:
        return 0
    return int(math.floor(100 * value / total))


def _number_value(value: Any, label: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TaskGraphValidationError(f"{label} must be a number.")
    return float(value)


def _compact_number(value: float) -> int | float:
    if value.is_integer():
        return int(value)
    return value
