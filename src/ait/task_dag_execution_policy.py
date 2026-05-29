from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any


DEFAULT_TASK_DAG_BATCH_SIZE = 4
DEFAULT_TASK_DAG_WORKER_EXECUTION_MODE = "worker_only_compact_packet"
DEFAULT_TASK_DAG_WORKER_EXECUTION_LABEL = "worker-only compact packet executed in one fresh worker session"
SUPPORTED_TASK_DAG_WORKER_EXECUTION_MODES = frozenset({DEFAULT_TASK_DAG_WORKER_EXECUTION_MODE})
DEFAULT_SOLO_REMOTE_DAG_MODE = "local_execution_dag_with_selective_promotion"
TASK_DAG_CHANGE_STRATEGY_LOCAL_FIRST_FINAL_REMOTE_LAND = "local_first_final_remote_land"
TASK_DAG_CHANGE_STRATEGY_LOCAL_FIRST_FINAL_LOCAL_LAND = "local_first_final_local_land"
TASK_DAG_CHANGE_STRATEGY_REMOTE_BACKED_SELECTIVE_PROMOTION = "remote_backed_selective_promotion"
SUPPORTED_TASK_DAG_CHANGE_STRATEGIES = frozenset(
    {
        TASK_DAG_CHANGE_STRATEGY_LOCAL_FIRST_FINAL_REMOTE_LAND,
        TASK_DAG_CHANGE_STRATEGY_LOCAL_FIRST_FINAL_LOCAL_LAND,
        TASK_DAG_CHANGE_STRATEGY_REMOTE_BACKED_SELECTIVE_PROMOTION,
    }
)


def _task_dag_converged_task_node_ids(graph: Mapping[str, Any] | dict[str, Any]) -> set[str]:
    task_nodes = {
        _clean_text(node.get("node_id")) or ""
        for node in graph.get("nodes", [])
        if isinstance(node, Mapping) and str(node.get("node_kind") or "").strip().lower() == "task"
    }
    explicit = {
        _clean_text(node.get("node_id")) or ""
        for node in graph.get("nodes", [])
        if isinstance(node, Mapping)
        and str(node.get("node_kind") or "").strip().lower() == "task"
        and bool(node.get("converged_output"))
    }
    if explicit:
        return {node_id for node_id in explicit if node_id}
    successors: dict[str, set[str]] = {node_id: set() for node_id in task_nodes if node_id}
    for edge in graph.get("edges", []) or []:
        if not isinstance(edge, Mapping):
            continue
        from_node = _clean_text(edge.get("from")) or ""
        to_node = _clean_text(edge.get("to")) or ""
        if from_node in successors and to_node in task_nodes:
            successors[from_node].add(to_node)
    return {node_id for node_id in task_nodes if not successors.get(node_id)}


def _task_dag_worker_execution_mode(policy: Mapping[str, Any]) -> dict[str, Any]:
    mode_id = _clean_text(policy.get("worker_execution_mode")) or DEFAULT_TASK_DAG_WORKER_EXECUTION_MODE
    if mode_id not in SUPPORTED_TASK_DAG_WORKER_EXECUTION_MODES:
        mode_id = DEFAULT_TASK_DAG_WORKER_EXECUTION_MODE
    return {
        "mode_id": mode_id,
        "label": DEFAULT_TASK_DAG_WORKER_EXECUTION_LABEL,
        "fresh_worker_session": True,
        "worker_session_count": 1,
    }


def _task_dag_canonical_default_mode(policy: Mapping[str, Any]) -> str:
    return DEFAULT_SOLO_REMOTE_DAG_MODE


def task_dag_change_strategy(
    graph: Mapping[str, Any] | dict[str, Any],
    *,
    workflow_mode: str | None = None,
) -> str | None:
    policy = graph.get("execution_policy") if isinstance(graph.get("execution_policy"), Mapping) else {}
    configured = _clean_text(policy.get("change_strategy"))
    if configured in SUPPORTED_TASK_DAG_CHANGE_STRATEGIES:
        return configured
    return TASK_DAG_CHANGE_STRATEGY_LOCAL_FIRST_FINAL_REMOTE_LAND


def task_dag_final_land_disposition(
    graph: Mapping[str, Any] | dict[str, Any],
    *,
    workflow_mode: str | None = None,
) -> str:
    change_strategy = task_dag_change_strategy(graph, workflow_mode=workflow_mode)
    if change_strategy == TASK_DAG_CHANGE_STRATEGY_LOCAL_FIRST_FINAL_LOCAL_LAND:
        return "local"
    return "remote"


def task_dag_final_remote_disposition_default(
    graph: Mapping[str, Any] | dict[str, Any],
    *,
    workflow_mode: str | None = None,
) -> bool:
    return task_dag_final_land_disposition(graph, workflow_mode=workflow_mode) == "remote"


def task_dag_final_output_later_remote_promotion_allowed(
    graph: Mapping[str, Any] | dict[str, Any],
    *,
    workflow_mode: str | None = None,
) -> bool:
    return (
        task_dag_change_strategy(graph, workflow_mode=workflow_mode)
        == TASK_DAG_CHANGE_STRATEGY_LOCAL_FIRST_FINAL_LOCAL_LAND
    )


def build_task_graph_execution_strategy(
    graph: dict[str, Any],
    ready_rows: Iterable[Mapping[str, Any]],
) -> dict[str, Any]:
    """Return the default execution strategy for ready DAG work."""

    policy = graph.get("execution_policy") if isinstance(graph.get("execution_policy"), dict) else {}
    worker_execution = _task_dag_worker_execution_mode(policy)
    batch_size = _positive_int(policy.get("batch_node_target"), DEFAULT_TASK_DAG_BATCH_SIZE)
    dispatch_model = _clean_text(policy.get("dispatch_model")) or "compact_packet"
    default_mode = _task_dag_canonical_default_mode(policy)
    physical_fanout_default = False
    physical_fanout_requires_explicit_opt_in = True
    min_batch_token_budget = max(_int_value(policy.get("min_batch_token_budget")), 0)
    small_node_token_threshold = max(_int_value(policy.get("small_node_token_threshold")), 0)
    packet_token_budget = max(_int_value(policy.get("packet_token_budget")), 0)
    worker_batch_token_floor = max(_int_value(policy.get("worker_batch_token_floor")), 0)
    worker_batch_token_ceiling = max(_int_value(policy.get("worker_batch_token_ceiling")), 0)
    rows = [row for row in ready_rows if isinstance(row, Mapping)]
    node_index = {
        str(node.get("node_id") or ""): node
        for node in graph.get("nodes", [])
        if isinstance(node, Mapping) and str(node.get("node_id") or "").strip()
    }
    prepared_rows = [
        {
            "row": row,
            "node_id": _clean_text(row.get("node_id")) or "",
            "estimated_tokens": _task_dag_node_dispatch_token_estimate(
                node_index.get(_clean_text(row.get("node_id")) or "", {})
            ),
        }
        for row in rows
    ]
    estimated_ready_tokens = sum(item["estimated_tokens"] for item in prepared_rows)
    has_token_estimates = any(item["estimated_tokens"] > 0 for item in prepared_rows)
    ready_count = len(prepared_rows)
    triggered_rules: list[dict[str, Any]] = []

    batches: list[dict[str, Any]] = []
    if ready_count:
        node_ids = [item["node_id"] for item in prepared_rows if item["node_id"]]
        small_node_ids = [
            item["node_id"]
            for item in prepared_rows
            if item["node_id"]
            and item["estimated_tokens"] > 0
            and small_node_token_threshold > 0
            and item["estimated_tokens"] <= small_node_token_threshold
        ]
        batches.append(
            {
                "batch_id": "batch-1",
                "node_ids": node_ids,
                "size": len(node_ids),
                "session_mode": "single_fresh_worker_session",
                "estimated_tokens": estimated_ready_tokens,
                "small_node_ids": small_node_ids,
                "coalesced_from_batch_count": 1,
                "fragmentation_vetoed": False,
                "reason": "Compress the ready wave into one worker-only compact packet and execute it in one fresh worker session.",
            }
        )

    summary = (
        f"{worker_execution['mode_id']} · {len(batches)} fresh worker session(s) for {ready_count} ready node(s)"
        if ready_count
        else f"{worker_execution['mode_id']} · no ready worker session"
    )
    ready_wave_only_recommended = bool(
        packet_token_budget > 0 and estimated_ready_tokens > 0 and estimated_ready_tokens > packet_token_budget
    )
    if ready_wave_only_recommended:
        summary = f"{summary}; packet budget recommends keeping remote materialization on the current ready wave"
        triggered_rules.append(
            {
                "role": "worker_packet",
                "code": "packet_ready_wave_only",
                "message": "Compact-packet budget recommends deferring blocked-node materialization until the current ready wave settles.",
                "packet_token_budget": packet_token_budget,
            }
        )
    if worker_batch_token_ceiling > 0 and any(batch.get("estimated_tokens", 0) > worker_batch_token_ceiling for batch in batches):
        triggered_rules.append(
            {
                "role": "batch_worker",
                "code": "worker_batch_ceiling_unmet",
                "message": "At least one worker batch still exceeds the configured ceiling after applying the compact batching heuristic.",
                "worker_batch_token_ceiling": worker_batch_token_ceiling,
            }
        )

    return {
        "default_mode": default_mode,
        "dispatch_model": dispatch_model,
        "worker_execution_mode": worker_execution["mode_id"],
        "worker_execution_label": worker_execution["label"],
        "fresh_worker_session": worker_execution["fresh_worker_session"],
        "worker_session_count": worker_execution["worker_session_count"],
        "worker_session_mode": "single_fresh_worker_session",
        "physical_fanout_default": physical_fanout_default,
        "physical_fanout_requires_explicit_opt_in": physical_fanout_requires_explicit_opt_in,
        "batch_node_target": batch_size,
        "max_total_sessions": 1,
        "max_worker_sessions": 1,
        "max_batch_sessions": 1,
        "configured_max_batch_sessions": 1,
        "token_budget_policy": {
            "enabled": bool(min_batch_token_budget > 0 and has_token_estimates),
            "min_batch_token_budget": min_batch_token_budget if min_batch_token_budget > 0 else None,
            "small_node_token_threshold": small_node_token_threshold if small_node_token_threshold > 0 else None,
            "estimated_ready_tokens": estimated_ready_tokens if has_token_estimates else None,
            "coalesced_batches": 0,
        },
        "cost_policy": {
            "enabled": bool(
                packet_token_budget > 0
                or worker_batch_token_floor > 0
                or worker_batch_token_ceiling > 0
            ),
            "packet_token_budget": packet_token_budget if packet_token_budget > 0 else None,
            "worker_batch_token_floor": worker_batch_token_floor if worker_batch_token_floor > 0 else None,
            "worker_batch_token_ceiling": worker_batch_token_ceiling if worker_batch_token_ceiling > 0 else None,
            "ready_wave_only_recommended": ready_wave_only_recommended,
            "triggered_rules": triggered_rules,
        },
        "ready_node_count": ready_count,
        "recommended_worker_sessions": len(batches),
        "recommended_total_sessions": len(batches),
        "batches": batches,
        "summary": summary,
        "caveat": "Use the worker-only compact packet surface by default; intermediate DAG nodes may keep local task/change/snapshot/local-land lineage while only the final converged output may enter shared workflow.",
    }


def task_dag_selective_promotion_default(
    graph: Mapping[str, Any] | dict[str, Any],
    *,
    workflow_mode: str | None = None,
) -> bool:
    return True


def build_task_dag_promotion_policy(
    graph: Mapping[str, Any] | dict[str, Any],
    ready_rows: Iterable[Mapping[str, Any]] | None = None,
    *,
    workflow_mode: str | None = None,
) -> dict[str, Any]:
    policy = graph.get("execution_policy") if isinstance(graph.get("execution_policy"), Mapping) else {}
    selective_promotion_default = task_dag_selective_promotion_default(graph, workflow_mode=workflow_mode)
    change_strategy = task_dag_change_strategy(graph, workflow_mode=workflow_mode)
    final_land_disposition = task_dag_final_land_disposition(graph, workflow_mode=workflow_mode)
    final_remote_disposition_default = final_land_disposition == "remote"
    later_remote_promotion_allowed = task_dag_final_output_later_remote_promotion_allowed(
        graph,
        workflow_mode=workflow_mode,
    )
    ready_node_ids = {
        _clean_text(row.get("node_id")) or ""
        for row in (ready_rows or [])
        if isinstance(row, Mapping) and _clean_text(row.get("node_id"))
    }
    converged_output_node_ids = _task_dag_converged_task_node_ids(graph)
    execution_only_node_ids: list[str] = []
    reviewable_node_ids: list[str] = []
    ready_execution_only_node_ids: list[str] = []
    for node in graph.get("nodes", []):
        if not isinstance(node, Mapping):
            continue
        node_id = _clean_text(node.get("node_id")) or ""
        if not node_id or str(node.get("node_kind") or "").strip().lower() != "task":
            continue
        if node_id in converged_output_node_ids:
            reviewable_node_ids.append(node_id)
            continue
        execution_only_node_ids.append(node_id)
        if node_id in ready_node_ids:
            ready_execution_only_node_ids.append(node_id)
    return {
        "workflow_mode": _clean_text(workflow_mode),
        "selective_promotion_default": selective_promotion_default,
        "local_execution_default": selective_promotion_default,
        "change_strategy": change_strategy,
        "final_land_disposition": final_land_disposition,
        "final_remote_disposition_default": final_remote_disposition_default,
        "later_remote_promotion_allowed_after_local_land": later_remote_promotion_allowed,
        "execution_only_node_ids": execution_only_node_ids if selective_promotion_default else [],
        "ready_execution_only_node_ids": ready_execution_only_node_ids if selective_promotion_default else [],
        "reviewable_node_ids": reviewable_node_ids if selective_promotion_default else [],
        "local_lineage_allowed_for_execution_only": bool(selective_promotion_default and execution_only_node_ids),
        "remote_task_materialization_skipped_for_execution_only": bool(
            selective_promotion_default and execution_only_node_ids
        ),
        "shared_promotion_boundary": (
            "single_final_reviewable_output_then_remote_land"
            if final_remote_disposition_default
            else "single_final_reviewable_output_then_local_land"
            if change_strategy == TASK_DAG_CHANGE_STRATEGY_LOCAL_FIRST_FINAL_LOCAL_LAND
            else "reviewable_output_or_explicit_gate"
            if selective_promotion_default
            else "task_or_change_creation"
        ),
        "current_boundary": (
            "execution-only nodes may keep local task/change/snapshot/local-land lineage until one converged output is promoted onto the final remote land path."
            if final_remote_disposition_default
            else "execution-only nodes may keep local task/change/snapshot/local-land lineage until one converged output reaches the final local-land boundary. After that local land, the final converged output may still later remote-promote through the repo-wide completed-local helper."
            if later_remote_promotion_allowed
            else "execution-only nodes may keep local task/change/snapshot/local-land lineage until a reviewable output or explicit shared gate is reached."
            if selective_promotion_default
            else "Remote task or change materialization may happen as soon as a node is dispatched."
        ),
        "promotion_helper_command": (
            'ait workflow publish --task <task-id> --summary "final output" --target-line main'
            if final_remote_disposition_default
            else "ait workflow land-local <change-id>"
            if change_strategy == TASK_DAG_CHANGE_STRATEGY_LOCAL_FIRST_FINAL_LOCAL_LAND
            else None
        ),
        "later_remote_promotion_helper_command": (
            "ait workflow land --all-completed-local --remote <name>"
            if later_remote_promotion_allowed
            else None
        ),
    }


def build_task_dag_token_budget_hint_summary(
    graph: Mapping[str, Any] | dict[str, Any],
    ready_rows: Iterable[Mapping[str, Any]] | None = None,
) -> dict[str, Any]:
    policy = graph.get("execution_policy") if isinstance(graph.get("execution_policy"), Mapping) else {}
    min_batch_token_budget = max(_int_value(policy.get("min_batch_token_budget")), 0)
    small_node_token_threshold = max(_int_value(policy.get("small_node_token_threshold")), 0)
    ready_node_ids = {
        _clean_text(row.get("node_id")) or ""
        for row in (ready_rows or [])
        if isinstance(row, Mapping) and _clean_text(row.get("node_id"))
    }
    nodes: list[dict[str, Any]] = []
    ready_unhinted_node_ids: list[str] = []
    all_unhinted_node_ids: list[str] = []
    ready_hinted_node_count = 0

    for node in graph.get("nodes", []):
        if not isinstance(node, Mapping):
            continue
        node_id = _clean_text(node.get("node_id")) or ""
        if not node_id or str(node.get("node_kind") or "").strip().lower() != "task":
            continue
        estimate = _task_dag_node_dispatch_token_estimate(node)
        ready = node_id in ready_node_ids
        if estimate <= 0:
            all_unhinted_node_ids.append(node_id)
            if ready:
                ready_unhinted_node_ids.append(node_id)
            continue
        if ready:
            ready_hinted_node_count += 1
        nodes.append(
            {
                "node_id": node_id,
                "title": node.get("title"),
                "dispatch_token_estimate": estimate,
                "hint_source": _task_dag_node_dispatch_token_hint_source(node),
                "ready": ready,
                "small_node_candidate": bool(small_node_token_threshold > 0 and estimate <= small_node_token_threshold),
            }
        )

    estimated_total = sum(_int_value(row.get("dispatch_token_estimate")) for row in nodes)
    return {
        "hinted_node_count": len(nodes),
        "ready_hinted_node_count": ready_hinted_node_count,
        "unhinted_node_count": len(all_unhinted_node_ids),
        "ready_unhinted_node_ids": ready_unhinted_node_ids,
        "all_unhinted_node_ids": all_unhinted_node_ids,
        "estimated_total_tokens": estimated_total if nodes else None,
        "min_batch_token_budget": min_batch_token_budget if min_batch_token_budget > 0 else None,
        "small_node_token_threshold": small_node_token_threshold if small_node_token_threshold > 0 else None,
        "nodes": nodes,
    }


def _task_dag_node_dispatch_token_estimate(node: Mapping[str, Any] | dict[str, Any]) -> int:
    template = node.get("task_template") if isinstance(node.get("task_template"), Mapping) else {}
    for value in (
        node.get("dispatch_token_estimate"),
        node.get("token_budget_hint"),
        template.get("dispatch_token_estimate"),
        template.get("token_budget_hint"),
    ):
        estimate = _int_value(value)
        if estimate > 0:
            return estimate
    return 0


def _task_dag_node_dispatch_token_hint_source(node: Mapping[str, Any] | dict[str, Any]) -> str | None:
    template = node.get("task_template") if isinstance(node.get("task_template"), Mapping) else {}
    if _int_value(node.get("dispatch_token_estimate")) > 0:
        return "node.dispatch_token_estimate"
    if _int_value(node.get("token_budget_hint")) > 0:
        return "node.token_budget_hint"
    if _int_value(template.get("dispatch_token_estimate")) > 0:
        return "task_template.dispatch_token_estimate"
    if _int_value(template.get("token_budget_hint")) > 0:
        return "task_template.token_budget_hint"
    return None


def _clean_text(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _positive_int(value: Any, default: int) -> int:
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return default
    return parsed if parsed > 0 else default


def _int_value(value: Any) -> int:
    try:
        return int(value or 0)
    except (TypeError, ValueError):
        return 0
