from __future__ import annotations

import json
from collections.abc import Iterable, Mapping
from typing import Any

from .plan_graph import topological_node_order, validate_task_graph


PASS_DECISIONS = {"pass", "not_required"}
PASS_EVALUATIONS = {"pass", "not_required"}
OPEN_CHANGE_STATES = {"draft", "review", "gated", "blocked", "approved", "landable"}
RUNNING_SESSION_STATES = {"active", "paused"}
RUNNING_LAND_STATES = {"queued", "running"}
BLOCKED_LAND_STATES = {"blocked", "failed", "error"}
CHANGE_STATUS_RANK = {
    "landed": 7,
    "landable": 6,
    "approved": 5,
    "gated": 4,
    "review": 3,
    "blocked": 2,
    "draft": 1,
    "archived": 0,
    "superseded": 0,
    "canceled": 0,
    "abandoned": 0,
    "later_promotion_excluded": 0,
}
TASK_STATUS_RANK = {
    "completed": 5,
    "ready_to_complete": 4,
    "active": 3,
    "claimed": 2,
    "draft": 1,
    "planned": 1,
    "archived": 0,
    "superseded": 0,
    "canceled": 0,
}
ACTIVE_HOTSPOT_STATES = {"active", "claimed", "locked", "running"}
ACTIVE_TASK_RUN_STATES = {"claimed", "running", "active"}
PASS_GATE_STATES = {"pass", "passed", "satisfied", "complete", "completed", "not_required"}
EXPLICIT_RUNNING_NODE_STATES = {"running", "in_progress", "local_progress"}
EXPLICIT_COMPLETED_NODE_STATES = {"completed", "landed", "superseded"}
EXPLICIT_BLOCKED_NODE_STATES = {"blocked", "failed"}
DEFAULT_TASK_DAG_BATCH_SIZE = 4
DEFAULT_TASK_DAG_MAX_BATCH_SESSIONS = 1
DEFAULT_TASK_DAG_MAX_WORKER_SESSIONS = 1
DEFAULT_TASK_DAG_WORKER_EXECUTION_MODE = "worker_only_compact_packet"
DEFAULT_TASK_DAG_WORKER_EXECUTION_LABEL = "worker-only compact packet executed in one fresh worker session"
SUPPORTED_TASK_DAG_WORKER_EXECUTION_MODES = frozenset({DEFAULT_TASK_DAG_WORKER_EXECUTION_MODE})
LEGACY_TASK_DAG_TRANSPORT_DEFAULT_MODE = "compact_packet_worker"
DEFAULT_SOLO_LOCAL_DAG_MODE = "local_execution_dag"
DEFAULT_SOLO_REMOTE_DAG_MODE = "local_execution_dag_with_selective_promotion"
DEFAULT_TEAM_REMOTE_DAG_MODE = "shared_workflow_dag_with_guarded_full_auto_continuation"
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
    terminal = {node_id for node_id in task_nodes if not successors.get(node_id)}
    return terminal


def _task_dag_task_ancestor_ids(
    graph: Mapping[str, Any] | dict[str, Any],
    target_node_ids: set[str],
) -> set[str]:
    if not target_node_ids:
        return set()
    task_nodes = {
        _clean_text(node.get("node_id")) or ""
        for node in graph.get("nodes", [])
        if isinstance(node, Mapping) and str(node.get("node_kind") or "").strip().lower() == "task"
    }
    reverse_dependencies: dict[str, set[str]] = {node_id: set() for node_id in task_nodes if node_id}
    for edge in graph.get("edges", []) or []:
        if not isinstance(edge, Mapping):
            continue
        from_node = _clean_text(edge.get("from")) or ""
        to_node = _clean_text(edge.get("to")) or ""
        if from_node in task_nodes and to_node in reverse_dependencies:
            reverse_dependencies[to_node].add(from_node)
    pending = list(target_node_ids)
    ancestors: set[str] = set()
    while pending:
        current = pending.pop()
        for parent in reverse_dependencies.get(current, set()):
            if parent in ancestors or parent in target_node_ids:
                continue
            ancestors.add(parent)
            pending.append(parent)
    return ancestors


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


def compute_task_graph_readiness(
    graph: dict[str, Any],
    workflow: Mapping[str, Any] | None = None,
    *,
    current_plan_revision_id: str | None = None,
) -> dict[str, Any]:
    """Build a deterministic readiness view for a task DAG graph."""

    graph = validate_task_graph(graph)
    workflow = workflow or {}
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    source_plan_revision_id = _clean_text(source_plan.get("plan_revision_id"))
    validates_source_revision = bool((graph.get("execution_policy") or {}).get("validate_source_plan_revision", False))
    stale_source_plan = bool(
        validates_source_revision
        and source_plan_revision_id
        and current_plan_revision_id
        and source_plan_revision_id != current_plan_revision_id
    )

    nodes_by_id = {str(node["node_id"]): node for node in graph["nodes"]}
    order = topological_node_order(graph)
    facts = _WorkflowFacts.from_mapping(workflow)
    rows_by_id: dict[str, dict[str, Any]] = {}

    for node_id in order:
        node = nodes_by_id[node_id]
        row = _node_readiness_row(
            node,
            facts,
            stale_source_plan=stale_source_plan,
            source_plan_revision_id=source_plan_revision_id,
            current_plan_revision_id=current_plan_revision_id,
        )
        rows_by_id[node_id] = row

    for node_id in order:
        row = rows_by_id[node_id]
        if row["state"] == "completed":
            continue
        dependency_blockers = []
        dependency_rows: list[dict[str, Any]] = []
        for dependency_id in row["depends_on"]:
            dependency = rows_by_id[dependency_id]
            dependency_rows.append(dependency)
            if dependency["state"] != "completed":
                dependency_blockers.append(
                    {
                        "type": "dependency",
                        "code": "dependency_incomplete",
                        "node_id": dependency_id,
                        "state": dependency["state"],
                        "message": f"Dependency {dependency_id} is {dependency['state']}, not completed.",
                    }
                )
        if dependency_blockers:
            row["blockers"] = dependency_blockers + row["blockers"]
            row["state"] = "blocked"
            row["reason"] = row["blockers"][0]["message"]
        elif _complete_land_gate_from_dependencies(row, dependency_rows):
            continue
        elif row["state"] == "unclaimed":
            row["state"] = "ready"
            row["reason"] = "Dependencies are complete and no active workflow claim exists."

    converged_node_ids = _task_dag_converged_task_node_ids(graph)
    completed_converged_node_ids = {
        node_id
        for node_id in converged_node_ids
        if rows_by_id.get(node_id, {}).get("state") == "completed"
    }
    if completed_converged_node_ids:
        reconciled_ancestor_ids = _task_dag_task_ancestor_ids(graph, completed_converged_node_ids)
        for node_id in reconciled_ancestor_ids:
            row = rows_by_id.get(node_id)
            node = nodes_by_id.get(node_id)
            if row is None or node is None:
                continue
            if str(node.get("workflow_boundary") or "").strip().lower() != "execution_only":
                continue
            if row.get("state") == "completed":
                continue
            row["state"] = "completed"
            row["reason"] = (
                "Unique converged output already completed; "
                "upstream execution-only lineage is reconciled as completed."
            )
            row["blockers"] = []
            row["session_recommendation"] = {
                "action": "none",
                "reason": "Node is complete.",
                "session_id": row.get("owner_session_id"),
                "refuse_duplicate_execution": False,
            }

    rows = [rows_by_id[node_id] for node_id in order]
    counts = {state: 0 for state in ("ready", "running", "blocked", "completed")}
    for row in rows:
        state = str(row.get("state") or "blocked")
        if state in counts:
            counts[state] += 1
        elif state in {"claimed", "review", "landable"}:
            counts["running"] += 1
        else:
            counts["blocked"] += 1
    counts["total"] = len(rows)

    next_ready = next((row["node_id"] for row in rows if row["state"] == "ready"), None)
    next_blocked = next((row["node_id"] for row in rows if row["state"] == "blocked"), None)
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": source_plan,
        "source_plan_revision_id": source_plan_revision_id,
        "current_plan_revision_id": current_plan_revision_id,
        "stale_source_plan": stale_source_plan,
        "summary": {
            "total_nodes": counts["total"],
            "ready_nodes": counts["ready"],
            "running_nodes": counts["running"],
            "blocked_nodes": counts["blocked"],
            "completed_nodes": counts["completed"],
            "next_action": f"start {next_ready}" if next_ready else f"unblock {next_blocked}" if next_blocked else "complete task graph",
        },
        "counts": counts,
        "nodes": rows,
    }


def build_task_graph_execution_strategy(
    graph: dict[str, Any],
    ready_rows: Iterable[Mapping[str, Any]],
) -> dict[str, Any]:
    """Return the default execution strategy for ready DAG work.

    The default is intentionally packet-first: compress the current ready wave
    into one worker-only compact packet and execute it in one fresh worker
    session.
    """

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
            else
            "execution-only nodes may keep local task/change/snapshot/local-land lineage until a reviewable output or explicit shared gate is reached."
            if selective_promotion_default
            else "Remote task or change materialization may happen as soon as a node is dispatched."
        ),
        "promotion_helper_command": (
            'ait workflow publish --task <task-id> --summary "final output" --target-line main'
            if final_remote_disposition_default
            else 'ait workflow land-local <change-id>'
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


def _coalesce_task_dag_batches_under_token_budget(
    batches: list[dict[str, Any]],
    min_batch_token_budget: int,
) -> list[dict[str, Any]]:
    coalesced = [dict(batch) for batch in batches]
    while len(coalesced) > 1:
        target_index = next(
            (
                index
                for index, batch in enumerate(coalesced)
                if batch.get("estimated_tokens", 0) > 0 and batch.get("estimated_tokens", 0) < min_batch_token_budget
            ),
            None,
        )
        if target_index is None:
            break
        merge_index = target_index - 1 if target_index > 0 else target_index + 1
        if merge_index < 0 or merge_index >= len(coalesced):
            break
        primary = dict(coalesced[merge_index])
        secondary = dict(coalesced[target_index])
        if merge_index > target_index:
            merged_node_ids = [*secondary.get("node_ids", []), *primary.get("node_ids", [])]
            merged_small_node_ids = [*secondary.get("small_node_ids", []), *primary.get("small_node_ids", [])]
        else:
            merged_node_ids = [*primary.get("node_ids", []), *secondary.get("node_ids", [])]
            merged_small_node_ids = [*primary.get("small_node_ids", []), *secondary.get("small_node_ids", [])]
        merged = {
            "node_ids": merged_node_ids,
            "estimated_tokens": _int_value(primary.get("estimated_tokens")) + _int_value(secondary.get("estimated_tokens")),
            "small_node_ids": list(dict.fromkeys(node_id for node_id in merged_small_node_ids if node_id)),
            "fragmentation_vetoed": True,
            "coalesced_from_batch_count": _int_value(primary.get("coalesced_from_batch_count"))
            + _int_value(secondary.get("coalesced_from_batch_count")),
        }
        first = min(target_index, merge_index)
        second = max(target_index, merge_index)
        coalesced[first] = merged
        del coalesced[second]
    return coalesced


def _node_readiness_row(
    node: dict[str, Any],
    facts: "_WorkflowFacts",
    *,
    stale_source_plan: bool,
    source_plan_revision_id: str | None,
    current_plan_revision_id: str | None,
) -> dict[str, Any]:
    node_id = str(node["node_id"])
    tasks = facts.tasks_for_node(node)
    task_ids = [_clean_text(task.get("task_id")) for task in tasks]
    changes = facts.changes_for_tasks(task_ids)
    sessions = facts.sessions_for_tasks_and_changes(task_ids, [_clean_text(change.get("change_id")) for change in changes])
    checkpoints = facts.checkpoints_for_sessions([_clean_text(session.get("session_id")) for session in sessions])
    patchsets = facts.patchsets_for_changes([_clean_text(change.get("change_id")) for change in changes])
    lands = facts.lands_for_changes_and_patchsets(
        [_clean_text(change.get("change_id")) for change in changes],
        [_clean_text(patchset.get("patchset_id")) for patchset in patchsets],
    )
    primary_task = _effective_task(tasks, changes, lands)
    primary_change = _effective_change(changes, patchsets, lands)
    primary_change_id = _clean_text(primary_change.get("change_id")) if primary_change else None
    primary_task_id = _clean_text(primary_task.get("task_id")) if primary_task else None
    current_patchset = _current_patchset(
        primary_change,
        [patchset for patchset in patchsets if _clean_text(patchset.get("change_id")) == primary_change_id] or patchsets,
    )
    effective_sessions = _effective_sessions(
        sessions,
        task_id=primary_task_id,
        change_id=primary_change_id,
    )
    latest_session = _latest_row(effective_sessions, "session_id")
    latest_checkpoint = _latest_row(checkpoints, "checkpoint_id")
    latest_land = _latest_row(
        _effective_lands(
            lands,
            change_id=primary_change_id,
            patchset_id=_clean_text(current_patchset.get("patchset_id")) if current_patchset else None,
        ),
        "submission_id",
        "land_id",
    )
    task_runs = facts.task_runs_for_node(
        node,
        task_ids=[primary_task_id] if primary_task_id else task_ids,
        change_ids=[primary_change_id] if primary_change_id else [_clean_text(change.get("change_id")) for change in changes],
    )
    active_task_run = facts.active_task_run(task_runs)
    surface_bindings = facts.surface_bindings_for_sessions(
        [_clean_text(active_task_run.get("session_id")) if active_task_run else None]
        + [_clean_text(session.get("session_id")) for session in effective_sessions]
    )
    latest_session_id = _clean_text(latest_session.get("session_id")) if latest_session else None
    latest_session_kind = _clean_text(latest_session.get("session_kind")) if latest_session else None
    latest_session_metadata = latest_session.get("metadata") if latest_session and isinstance(latest_session.get("metadata"), dict) else {}
    owner_session_id = (
        _clean_text(active_task_run.get("session_id")) if active_task_run else None
    ) or (_clean_text(latest_session.get("session_id")) if latest_session else None)
    owner_session_kind = latest_session_kind if owner_session_id and owner_session_id == latest_session_id else None
    owner_session_policy = (
        _clean_text(latest_session_metadata.get("session_policy") or latest_session_metadata.get("tracking_policy"))
        if owner_session_kind is not None
        else None
    )
    lineage = {
        "task_id": _clean_text(primary_task.get("task_id")) if primary_task else None,
        "change_id": _clean_text(primary_change.get("change_id")) if primary_change else None,
        "superseded_change_ids": [
            _clean_text(change.get("change_id"))
            for change in changes
            if _clean_text(change.get("change_id"))
            and _clean_text(change.get("change_id")) != _clean_text(primary_change.get("change_id"))
        ],
        "session_id": owner_session_id,
        "checkpoint_id": _clean_text(latest_checkpoint.get("checkpoint_id")) if latest_checkpoint else None,
        "patchset_id": _clean_text(current_patchset.get("patchset_id")) if current_patchset else None,
        "patchset_base_snapshot_id": _clean_text(current_patchset.get("base_snapshot_id")) if current_patchset else None,
        "patchset_revision_snapshot_id": _clean_text(current_patchset.get("revision_snapshot_id")) if current_patchset else None,
        "land_id": _land_id(latest_land) if latest_land else None,
        "landed_snapshot_id": _land_result_value(latest_land, "landed_snapshot_id") if latest_land else None,
        "task_run_id": _clean_text(active_task_run.get("task_run_id")) if active_task_run else None,
        "node_id": node_id,
    }

    blockers: list[dict[str, Any]] = []
    if stale_source_plan:
        blockers.append(
            {
                "type": "stale_plan_revision",
                "code": "source_plan_stale",
                "source_plan_revision_id": source_plan_revision_id,
                "current_plan_revision_id": current_plan_revision_id,
                "message": "Task graph source plan revision is not the current plan revision.",
            }
        )
    blockers.extend(_explicit_node_blockers(node, facts))
    blockers.extend(facts.hotspot_blockers(node))
    blockers.extend(_gate_blockers(primary_change, current_patchset, facts))
    blockers.extend(_review_blockers(primary_change, current_patchset, facts))
    blockers.extend(_policy_blockers(current_patchset, facts))
    blockers.extend(_land_blockers(latest_land))

    explicit_state = facts.explicit_node_state(node_id)
    explicit_state_row = facts.explicit_node_state_row(node_id)
    if explicit_state_row:
        for field in (
            "task_id",
            "change_id",
            "completion_snapshot_id",
            "completion_fork_snapshot_id",
            "completion_line_name",
            "completion_worktree_name",
        ):
            value = _clean_text(explicit_state_row.get(field))
            if value and not _clean_text(lineage.get(field)):
                lineage[field] = value
    explicit_reason = (
        _clean_text((explicit_state_row or {}).get("reason"))
        or _clean_text((explicit_state_row or {}).get("message"))
    )
    completed = _node_is_completed(primary_task, changes, lands) or explicit_state in EXPLICIT_COMPLETED_NODE_STATES
    running = _node_is_running(primary_task, changes, sessions, patchsets, lands, active_task_run)
    if completed:
        state = "completed"
        reason = explicit_reason or "Linked task/change/land evidence is completed."
        blockers = []
    elif explicit_state in EXPLICIT_BLOCKED_NODE_STATES and not blockers:
        blockers.append(
            {
                "type": "node_state",
                "code": "explicit_node_blocked",
                "node_id": node_id,
                "state": explicit_state,
                "message": explicit_reason or f"Node {node_id} is explicitly {explicit_state}.",
            }
        )
        state = "blocked"
        reason = blockers[0]["message"]
    elif explicit_state in EXPLICIT_RUNNING_NODE_STATES and not blockers:
        state = "running"
        reason = explicit_reason or "Local-first execution reported progress."
    elif blockers:
        state = "blocked"
        reason = blockers[0]["message"]
    elif running:
        state = "running"
        reason = "Linked workflow evidence is active."
    else:
        state = "unclaimed"
        reason = "Waiting for dependency evaluation."

    return {
        "node_id": node_id,
        "node_kind": node.get("node_kind"),
        "title": node.get("title"),
        "plan_item_ref": node.get("plan_item_ref"),
        "state": state,
        "reason": reason,
        "depends_on": sorted(str(value) for value in node.get("depends_on", []) if isinstance(value, str)),
        "lock_keys": sorted(str(value) for value in node.get("lock_keys", []) if isinstance(value, str)),
        "hotspot_keys": sorted(str(value) for value in node.get("hotspot_keys", []) if isinstance(value, str)),
        "lineage": lineage,
        "owner_session_id": owner_session_id,
        "owner_session_kind": owner_session_kind,
        "owner_session_policy": owner_session_policy,
        "explicit_state": explicit_state,
        "task_run": _task_run_view(active_task_run),
        "session_recommendation": _session_recommendation(
            state=state,
            owner_session_id=owner_session_id,
            owner_session_kind=owner_session_kind,
            task=primary_task,
            active_task_run=active_task_run,
            surface_bindings=surface_bindings,
        ),
        "surface_bindings": surface_bindings,
        "evidence": {
            "task_ids": sorted(value for value in task_ids if value),
            "task_run_ids": sorted(
                _clean_text(task_run.get("task_run_id")) for task_run in task_runs if _clean_text(task_run.get("task_run_id"))
            ),
            "change_ids": sorted(_clean_text(change.get("change_id")) for change in changes if _clean_text(change.get("change_id"))),
            "change_statuses": sorted(_clean_text(change.get("status")) for change in changes if _clean_text(change.get("status"))),
            "session_ids": sorted(_clean_text(session.get("session_id")) for session in sessions if _clean_text(session.get("session_id"))),
            "checkpoint_ids": sorted(_clean_text(checkpoint.get("checkpoint_id")) for checkpoint in checkpoints if _clean_text(checkpoint.get("checkpoint_id"))),
            "patchset_ids": sorted(_clean_text(patchset.get("patchset_id")) for patchset in patchsets if _clean_text(patchset.get("patchset_id"))),
            "land_ids": sorted(_land_id(land) for land in lands if _land_id(land)),
            "land_statuses": sorted(_clean_text(land.get("status")) for land in lands if _clean_text(land.get("status"))),
            "snapshot_ids": sorted(
                value
                for value in {
                    lineage.get("patchset_base_snapshot_id"),
                    lineage.get("patchset_revision_snapshot_id"),
                    lineage.get("landed_snapshot_id"),
                }
                if value
            ),
        },
        "blockers": blockers,
    }


def _gate_blockers(
    change: dict[str, Any] | None,
    patchset: dict[str, Any] | None,
    facts: "_WorkflowFacts",
) -> list[dict[str, Any]]:
    blockers: list[dict[str, Any]] = []
    if not change or not patchset:
        return blockers
    evaluation_state = _clean_text(patchset.get("evaluation_state"))
    if evaluation_state and evaluation_state not in PASS_EVALUATIONS:
        blockers.append(
            {
                "type": "gate",
                "code": "patchset_evaluation_not_passed",
                "change_id": _clean_text(change.get("change_id")),
                "patchset_id": _clean_text(patchset.get("patchset_id")),
                "evaluation_state": evaluation_state,
                "message": f"Patchset evaluation is {evaluation_state}.",
            }
        )
    if _truthy(patchset.get("policy_required")) and facts.policy_for_patchset(_clean_text(patchset.get("patchset_id"))) is None:
        blockers.append(
            {
                "type": "gate",
                "code": "policy_missing",
                "change_id": _clean_text(change.get("change_id")),
                "patchset_id": _clean_text(patchset.get("patchset_id")),
                "message": "Patchset requires a policy decision before landing.",
            }
        )
    return blockers


def _review_blockers(
    change: dict[str, Any] | None,
    patchset: dict[str, Any] | None,
    facts: "_WorkflowFacts",
) -> list[dict[str, Any]]:
    if not change:
        return []
    summary = facts.review_summary(_clean_text(change.get("change_id")), _clean_text(patchset.get("patchset_id")) if patchset else None)
    if int(summary["blocking"]) <= 0 and _clean_text(change.get("status")) != "blocked":
        return []
    return [
        {
            "type": "review",
            "code": "blocking_review",
            "change_id": _clean_text(change.get("change_id")),
            "patchset_id": _clean_text(patchset.get("patchset_id")) if patchset else None,
            "blocking": int(summary["blocking"]),
            "message": "Blocking review feedback is recorded on this change.",
        }
    ]


def _policy_blockers(patchset: dict[str, Any] | None, facts: "_WorkflowFacts") -> list[dict[str, Any]]:
    if not patchset:
        return []
    policy = facts.policy_for_patchset(_clean_text(patchset.get("patchset_id")))
    if policy is None:
        return []
    decision = _clean_text(policy.get("decision")) or "pending"
    if decision in PASS_DECISIONS:
        return []
    return [
        {
            "type": "policy",
            "code": "policy_not_passed",
            "patchset_id": _clean_text(patchset.get("patchset_id")),
            "decision": decision,
            "message": f"Policy decision is {decision}.",
        }
    ]


def _land_blockers(land: dict[str, Any] | None) -> list[dict[str, Any]]:
    if not land:
        return []
    status = _clean_text(land.get("status"))
    if status not in BLOCKED_LAND_STATES:
        return []
    return [
        {
            "type": "land",
            "code": "land_not_succeeded",
            "land_id": _land_id(land),
            "patchset_id": _clean_text(land.get("patchset_id")),
            "status": status,
            "blocker_class": _clean_text(land.get("blocker_class")),
            "message": f"Latest land request is {status}.",
        }
    ]


def _explicit_node_blockers(node: dict[str, Any], facts: "_WorkflowFacts") -> list[dict[str, Any]]:
    blockers: list[dict[str, Any]] = []
    node_id = _clean_text(node.get("node_id"))
    gate_rules = [str(value).strip() for value in node.get("gate_rules", []) if _clean_text(value)]
    land_gate_rules = [str(value).strip() for value in node.get("land_gate_rules", []) if _clean_text(value)]
    for gate_rule in gate_rules + land_gate_rules:
        result = facts.gate_result(gate_rule, node_id)
        status = _clean_text(result.get("status")) if result else None
        decision = _clean_text(result.get("decision")) if result else None
        normalized = (decision or status or "missing").lower()
        if normalized in PASS_GATE_STATES:
            continue
        blockers.append(
            {
                "type": "gate",
                "code": "explicit_gate_not_satisfied",
                "node_id": node_id,
                "gate": gate_rule,
                "status": status,
                "decision": decision,
                "message": f"Gate {gate_rule} is {normalized}.",
            }
        )
    return blockers


def _node_is_completed(task: dict[str, Any] | None, changes: list[dict[str, Any]], lands: list[dict[str, Any]]) -> bool:
    if task and _clean_text(task.get("status")) == "completed":
        return True
    if any(_clean_text(change.get("status")) == "landed" for change in changes):
        return True
    return any(_clean_text(land.get("status")) == "succeeded" for land in lands)


def _node_is_running(
    task: dict[str, Any] | None,
    changes: list[dict[str, Any]],
    sessions: list[dict[str, Any]],
    patchsets: list[dict[str, Any]],
    lands: list[dict[str, Any]],
    active_task_run: dict[str, Any] | None,
) -> bool:
    if active_task_run is not None:
        return True
    if task and _clean_text(task.get("status")) == "active":
        return True
    if any(_clean_text(change.get("status")) in OPEN_CHANGE_STATES for change in changes):
        return True
    if any(_clean_text(session.get("status")) in RUNNING_SESSION_STATES for session in sessions):
        return True
    if patchsets:
        return True
    return any(_clean_text(land.get("status")) in RUNNING_LAND_STATES for land in lands)


def _task_run_view(task_run: dict[str, Any] | None) -> dict[str, Any] | None:
    if not task_run:
        return None
    return {
        "task_run_id": _clean_text(task_run.get("task_run_id")),
        "status": _clean_text(task_run.get("status")),
        "node_id": _clean_text(task_run.get("node_id")),
        "task_id": _clean_text(task_run.get("task_id")),
        "change_id": _clean_text(task_run.get("change_id")),
        "session_id": _clean_text(task_run.get("session_id")),
        "claimed_by": _clean_text(task_run.get("claimed_by")),
        "claim_expires_at": _clean_text(task_run.get("claim_expires_at")),
    }


def _session_recommendation(
    *,
    state: str,
    owner_session_id: str | None,
    owner_session_kind: str | None,
    task: dict[str, Any] | None,
    active_task_run: dict[str, Any] | None,
    surface_bindings: list[dict[str, Any]],
) -> dict[str, Any]:
    if state == "completed":
        action = "none"
        reason = "Node is complete."
    elif state == "blocked":
        action = "unblock_before_session"
        reason = "Resolve blockers before opening or reusing a work session."
    elif active_task_run is not None and owner_session_id:
        action = "reuse_primary_session"
        reason = "Node already has an active primary task_run session."
    elif active_task_run is not None:
        action = "continue_claim"
        reason = "Node is claimed but has no primary session yet."
    elif owner_session_kind == "task_run":
        action = "resume_or_claim"
        reason = "Only the task tracking session exists; a fresh execution session can still take over this node."
    elif owner_session_id or surface_bindings:
        action = "attach_surface"
        reason = "Attach to the existing session/surface binding instead of opening duplicate work."
    elif task is not None:
        action = "resume_or_claim"
        reason = "A task exists; claim or resume it before starting another execution."
    else:
        action = "open_new_session"
        reason = "No active owner or linked task exists."
    return {
        "action": action,
        "reason": reason,
        "session_id": owner_session_id,
        "refuse_duplicate_execution": action in {"reuse_primary_session", "continue_claim", "attach_surface"},
    }


class _WorkflowFacts:
    def __init__(
        self,
        *,
        tasks: list[dict[str, Any]],
        changes: list[dict[str, Any]],
        sessions: list[dict[str, Any]],
        checkpoints: list[dict[str, Any]],
        patchsets: list[dict[str, Any]],
        reviews: list[dict[str, Any]],
        policies: list[dict[str, Any]],
        lands: list[dict[str, Any]],
        hotspots: list[dict[str, Any]],
        task_runs: list[dict[str, Any]],
        surface_bindings: list[dict[str, Any]],
        gate_results: list[dict[str, Any]],
        node_states: list[dict[str, Any]],
    ) -> None:
        self.tasks = tasks
        self.changes = changes
        self.sessions = sessions
        self.checkpoints = checkpoints
        self.patchsets = patchsets
        self.reviews = reviews
        self.policies = policies
        self.lands = lands
        self.hotspots = hotspots
        self.task_runs = task_runs
        self.surface_bindings = surface_bindings
        self.gate_results = gate_results
        self.node_states = node_states

    @classmethod
    def from_mapping(cls, workflow: Mapping[str, Any]) -> "_WorkflowFacts":
        return cls(
            tasks=_rows(workflow.get("tasks")),
            changes=_rows(workflow.get("changes")),
            sessions=_rows(workflow.get("sessions")),
            checkpoints=_rows(workflow.get("checkpoints")),
            patchsets=_rows(workflow.get("patchsets")),
            reviews=_rows(workflow.get("reviews")) + _rows(workflow.get("review_summaries")),
            policies=_rows(workflow.get("policies")) + _rows(workflow.get("policy_statuses")),
            lands=_rows(workflow.get("lands")) + _rows(workflow.get("land_requests")),
            hotspots=_hotspot_rows(workflow),
            task_runs=_rows(workflow.get("task_runs")) + _rows(workflow.get("runs")),
            surface_bindings=_rows(workflow.get("surface_bindings")) + _rows(workflow.get("session_links")),
            gate_results=_rows(workflow.get("gate_results")) + _rows(workflow.get("gate_evaluations")),
            node_states=_rows(workflow.get("node_states")),
        )

    def tasks_for_node(self, node: dict[str, Any]) -> list[dict[str, Any]]:
        node_id = _clean_text(node.get("node_id"))
        plan_item_ref = _clean_text(node.get("plan_item_ref"))
        matches = []
        for task in self.tasks:
            metadata = task.get("metadata") if isinstance(task.get("metadata"), dict) else {}
            task_node_id = _clean_text(task.get("node_id")) or _clean_text(metadata.get("node_id"))
            if task_node_id and task_node_id == node_id:
                matches.append(task)
                continue
            if plan_item_ref and _clean_text(task.get("plan_item_ref")) == plan_item_ref:
                matches.append(task)
        return _latest_first(matches, "task_id")

    def changes_for_tasks(self, task_ids: Iterable[str | None]) -> list[dict[str, Any]]:
        wanted = {value for value in task_ids if value}
        return _latest_first([change for change in self.changes if _clean_text(change.get("task_id")) in wanted], "change_id")

    def sessions_for_tasks_and_changes(
        self,
        task_ids: Iterable[str | None],
        change_ids: Iterable[str | None],
    ) -> list[dict[str, Any]]:
        wanted_tasks = {value for value in task_ids if value}
        wanted_changes = {value for value in change_ids if value}
        return _latest_first(
            [
                session
                for session in self.sessions
                if _clean_text(session.get("task_id")) in wanted_tasks or _clean_text(session.get("change_id")) in wanted_changes
            ],
            "session_id",
        )

    def checkpoints_for_sessions(self, session_ids: Iterable[str | None]) -> list[dict[str, Any]]:
        wanted = {value for value in session_ids if value}
        return _latest_first(
            [checkpoint for checkpoint in self.checkpoints if _clean_text(checkpoint.get("session_id")) in wanted],
            "checkpoint_id",
        )

    def patchsets_for_changes(self, change_ids: Iterable[str | None]) -> list[dict[str, Any]]:
        wanted = {value for value in change_ids if value}
        return _latest_first([patchset for patchset in self.patchsets if _clean_text(patchset.get("change_id")) in wanted], "patchset_id")

    def lands_for_changes_and_patchsets(
        self,
        change_ids: Iterable[str | None],
        patchset_ids: Iterable[str | None],
    ) -> list[dict[str, Any]]:
        wanted_changes = {value for value in change_ids if value}
        wanted_patchsets = {value for value in patchset_ids if value}
        return _latest_first(
            [
                land
                for land in self.lands
                if _clean_text(land.get("change_id")) in wanted_changes or _clean_text(land.get("patchset_id")) in wanted_patchsets
            ],
            "submission_id",
            "land_id",
        )

    def task_runs_for_node(
        self,
        node: dict[str, Any],
        *,
        task_ids: Iterable[str | None],
        change_ids: Iterable[str | None],
    ) -> list[dict[str, Any]]:
        node_id = _clean_text(node.get("node_id"))
        plan_item_ref = _clean_text(node.get("plan_item_ref"))
        wanted_tasks = {value for value in task_ids if value}
        wanted_changes = {value for value in change_ids if value}
        matches = []
        for task_run in self.task_runs:
            metadata = task_run.get("metadata") if isinstance(task_run.get("metadata"), dict) else {}
            run_node_id = _clean_text(task_run.get("node_id")) or _clean_text(metadata.get("node_id"))
            run_plan_item_ref = _clean_text(task_run.get("plan_item_ref")) or _clean_text(metadata.get("plan_item_ref"))
            run_task_id = _clean_text(task_run.get("task_id"))
            run_change_id = _clean_text(task_run.get("change_id"))
            if run_node_id and run_node_id == node_id:
                matches.append(task_run)
            elif plan_item_ref and run_plan_item_ref == plan_item_ref:
                matches.append(task_run)
            elif run_task_id in wanted_tasks or run_change_id in wanted_changes:
                matches.append(task_run)
        return _latest_first(matches, "task_run_id", "run_id")

    def active_task_run(self, task_runs: list[dict[str, Any]]) -> dict[str, Any] | None:
        active = [task_run for task_run in task_runs if (_clean_text(task_run.get("status")) or "active") in ACTIVE_TASK_RUN_STATES]
        return _latest_row(active, "task_run_id", "run_id")

    def surface_bindings_for_sessions(self, session_ids: Iterable[str | None]) -> list[dict[str, Any]]:
        wanted = {value for value in session_ids if value}
        if not wanted:
            return []
        rows = []
        for binding in self.surface_bindings:
            session_id = _clean_text(binding.get("session_id"))
            if session_id not in wanted:
                continue
            rows.append(
                {
                    "surface": _clean_text(binding.get("surface")) or _clean_text(binding.get("surface_kind")),
                    "surface_id": _clean_text(binding.get("surface_id")) or _clean_text(binding.get("telegram_chat_id")),
                    "session_id": session_id,
                    "status": _clean_text(binding.get("status")) or "active",
                }
            )
        return _latest_first(rows, "surface_id", "session_id")

    def gate_result(self, gate: str, node_id: str | None) -> dict[str, Any] | None:
        matches = []
        for result in self.gate_results:
            gate_name = _clean_text(result.get("gate")) or _clean_text(result.get("gate_id")) or _clean_text(result.get("name"))
            result_node_id = _clean_text(result.get("node_id"))
            if gate_name != gate:
                continue
            if result_node_id not in {None, node_id}:
                continue
            matches.append(result)
        return _latest_row(matches, "gate_result_id", "gate_id", "node_id")

    def explicit_node_state(self, node_id: str | None) -> str | None:
        row = self.explicit_node_state_row(node_id)
        if row is None:
            return None
        return _clean_text(row.get("state")) or _clean_text(row.get("status"))

    def explicit_node_state_row(self, node_id: str | None) -> dict[str, Any] | None:
        matches = [row for row in self.node_states if _clean_text(row.get("node_id")) == node_id]
        return _latest_row(matches, "node_state_id", "node_id")

    def review_summary(self, change_id: str | None, patchset_id: str | None) -> dict[str, int]:
        approvals = 0
        blocking = 0
        comments = 0
        for review in self.reviews:
            if change_id and _clean_text(review.get("change_id")) not in {None, change_id}:
                continue
            if patchset_id and _clean_text(review.get("patchset_id")) not in {None, patchset_id}:
                continue
            approvals += _int_value(review.get("approvals") if "approvals" in review else review.get("approval_count"))
            blocking += _int_value(review.get("blocking") if "blocking" in review else review.get("blocking_count"))
            comments += _int_value(review.get("comments") if "comments" in review else review.get("comment_count"))
            action = _clean_text(review.get("action")) or _clean_text(review.get("status")) or _clean_text(review.get("decision"))
            if action in {"approve", "approved"}:
                approvals += 1
            if action in {"block", "blocked", "reject", "rejected", "changes_requested", "request_changes"}:
                blocking += 1
        return {"approvals": approvals, "blocking": blocking, "comments": comments}

    def policy_for_patchset(self, patchset_id: str | None) -> dict[str, Any] | None:
        if not patchset_id:
            return None
        policies = [policy for policy in self.policies if _clean_text(policy.get("patchset_id")) == patchset_id]
        return _latest_row(policies, "policy_id", "patchset_id")

    def hotspot_blockers(self, node: dict[str, Any]) -> list[dict[str, Any]]:
        node_id = _clean_text(node.get("node_id"))
        node_hotspots = {_clean_text(value) for value in node.get("hotspot_keys", []) if _clean_text(value)}
        blockers = []
        for hotspot in self.hotspots + self._task_run_hotspots():
            key = _clean_text(hotspot.get("hotspot_key")) or _clean_text(hotspot.get("key"))
            holder = _clean_text(hotspot.get("holder_node_id")) or _clean_text(hotspot.get("node_id"))
            status = _clean_text(hotspot.get("status")) or "active"
            if key not in node_hotspots or holder in {None, node_id} or status not in ACTIVE_HOTSPOT_STATES:
                continue
            blockers.append(
                {
                    "type": "hotspot",
                    "code": "hotspot_claimed",
                    "hotspot_key": key,
                    "holder_node_id": holder,
                    "message": f"Hotspot {key} is claimed by node {holder}.",
                }
            )
        return sorted(blockers, key=lambda item: (item["hotspot_key"], item["holder_node_id"]))

    def _task_run_hotspots(self) -> list[dict[str, Any]]:
        rows: list[dict[str, Any]] = []
        for task_run in self.task_runs:
            status = _clean_text(task_run.get("status")) or "active"
            if status not in ACTIVE_TASK_RUN_STATES:
                continue
            holder = _clean_text(task_run.get("node_id"))
            for key in task_run.get("hotspot_keys") or []:
                if not _clean_text(key):
                    continue
                rows.append({"hotspot_key": _clean_text(key), "holder_node_id": holder, "status": status})
        return rows


def _current_patchset(change: dict[str, Any] | None, patchsets: list[dict[str, Any]]) -> dict[str, Any] | None:
    if not change:
        return None
    current_patchset_id = _clean_text(change.get("current_patchset_id")) or _clean_text(change.get("selected_patchset_id"))
    if current_patchset_id:
        for patchset in patchsets:
            if _clean_text(patchset.get("patchset_id")) == current_patchset_id:
                return patchset
    return _latest_row(patchsets, "patchset_id")


def _rows(value: Any) -> list[dict[str, Any]]:
    if value is None:
        return []
    if isinstance(value, Mapping):
        return [dict(value)]
    if not isinstance(value, Iterable) or isinstance(value, (str, bytes)):
        return []
    return [dict(item) for item in value if isinstance(item, Mapping)]


def _hotspot_rows(workflow: Mapping[str, Any]) -> list[dict[str, Any]]:
    rows = _rows(workflow.get("hotspot_claims")) + _rows(workflow.get("hotspot_locks"))
    locks = workflow.get("hotspot_locks")
    if isinstance(locks, Mapping):
        for key, holder in locks.items():
            if isinstance(holder, Mapping):
                rows.append({"hotspot_key": key, **dict(holder)})
            else:
                rows.append({"hotspot_key": key, "holder_node_id": holder, "status": "active"})
    return rows


def _latest_row(rows: Iterable[dict[str, Any]], *id_fields: str) -> dict[str, Any] | None:
    ordered = _latest_first(rows, *id_fields)
    return ordered[0] if ordered else None


def _change_status_rank(change: Mapping[str, Any] | None) -> int:
    status = (_clean_text((change or {}).get("status")) or "").lower()
    return CHANGE_STATUS_RANK.get(status, 1)


def _change_has_effective_land(change: Mapping[str, Any] | None, lands: Iterable[dict[str, Any]]) -> bool:
    change_id = _clean_text((change or {}).get("change_id"))
    if not change_id:
        return False
    if (_clean_text((change or {}).get("status")) or "").lower() == "landed":
        return True
    return any(
        _clean_text(land.get("change_id")) == change_id and (_clean_text(land.get("status")) or "").lower() == "succeeded"
        for land in lands
    )


def _change_has_selected_patchset(change: Mapping[str, Any] | None, patchsets: Iterable[dict[str, Any]]) -> bool:
    change_id = _clean_text((change or {}).get("change_id"))
    if not change_id:
        return False
    if _clean_text((change or {}).get("current_patchset_id")) or _clean_text((change or {}).get("selected_patchset_id")):
        return True
    return any(_clean_text(patchset.get("change_id")) == change_id for patchset in patchsets)


def _task_status_rank(task: Mapping[str, Any] | None) -> int:
    status = (_clean_text((task or {}).get("status")) or "").lower()
    return TASK_STATUS_RANK.get(status, 1)


def _task_has_effective_completion(
    task: Mapping[str, Any] | None,
    changes: Iterable[dict[str, Any]],
    lands: Iterable[dict[str, Any]],
) -> bool:
    task_id = _clean_text((task or {}).get("task_id"))
    if not task_id:
        return False
    task_changes = [dict(change) for change in changes if _clean_text(change.get("task_id")) == task_id]
    task_change_ids = {_clean_text(change.get("change_id")) for change in task_changes}
    task_change_ids.discard(None)
    task_lands = [dict(land) for land in lands if _clean_text(land.get("change_id")) in task_change_ids]
    return _node_is_completed(dict(task), task_changes, task_lands)


def _effective_task(
    tasks: Iterable[dict[str, Any]],
    changes: Iterable[dict[str, Any]],
    lands: Iterable[dict[str, Any]],
) -> dict[str, Any] | None:
    candidates = [dict(task) for task in tasks]
    if not candidates:
        return None
    ordered = sorted(
        candidates,
        key=lambda task: (
            1 if _task_has_effective_completion(task, changes, lands) else 0,
            _task_status_rank(task),
            _clean_text(task.get("updated_at")) or _clean_text(task.get("created_at")) or "",
            _clean_text(task.get("task_id")) or "",
        ),
        reverse=True,
    )
    return ordered[0] if ordered else None


def _effective_change(
    changes: Iterable[dict[str, Any]],
    patchsets: Iterable[dict[str, Any]],
    lands: Iterable[dict[str, Any]],
) -> dict[str, Any] | None:
    candidates = [dict(change) for change in changes]
    if not candidates:
        return None
    ordered = sorted(
        candidates,
        key=lambda change: (
            1 if _change_has_effective_land(change, lands) else 0,
            _change_status_rank(change),
            1 if _change_has_selected_patchset(change, patchsets) else 0,
            _clean_text(change.get("updated_at")) or _clean_text(change.get("created_at")) or "",
            _clean_text(change.get("change_id")) or "",
        ),
        reverse=True,
    )
    return ordered[0] if ordered else None


def _effective_sessions(
    sessions: Iterable[dict[str, Any]],
    *,
    task_id: str | None,
    change_id: str | None,
) -> list[dict[str, Any]]:
    rows = [dict(session) for session in sessions]
    if change_id:
        change_sessions = [row for row in rows if _clean_text(row.get("change_id")) == change_id]
        if change_sessions:
            return _latest_first(change_sessions, "session_id")
    if task_id:
        task_sessions = [row for row in rows if _clean_text(row.get("task_id")) == task_id]
        if task_sessions:
            return _latest_first(task_sessions, "session_id")
    return _latest_first(rows, "session_id")


def _effective_lands(
    lands: Iterable[dict[str, Any]],
    *,
    change_id: str | None,
    patchset_id: str | None,
) -> list[dict[str, Any]]:
    rows = [dict(land) for land in lands]
    if patchset_id:
        patchset_lands = [row for row in rows if _clean_text(row.get("patchset_id")) == patchset_id]
        if patchset_lands:
            succeeded = [row for row in patchset_lands if _clean_text(row.get("status")) == "succeeded"]
            return _latest_first(succeeded or patchset_lands, "submission_id", "land_id")
    if change_id:
        change_lands = [row for row in rows if _clean_text(row.get("change_id")) == change_id]
        if change_lands:
            succeeded = [row for row in change_lands if _clean_text(row.get("status")) == "succeeded"]
            return _latest_first(succeeded or change_lands, "submission_id", "land_id")
    return _latest_first(rows, "submission_id", "land_id")


def _latest_first(rows: Iterable[dict[str, Any]], *id_fields: str) -> list[dict[str, Any]]:
    return sorted((dict(row) for row in rows), key=lambda row: _row_sort_key(row, id_fields), reverse=True)


def _row_sort_key(row: dict[str, Any], id_fields: tuple[str, ...]) -> tuple[str, str, str]:
    updated_at = _clean_text(row.get("updated_at")) or _clean_text(row.get("created_at")) or ""
    created_at = _clean_text(row.get("created_at")) or ""
    identifier = next((_clean_text(row.get(field)) for field in id_fields if _clean_text(row.get(field))), "")
    return (updated_at, created_at, identifier or "")


def _land_id(land: dict[str, Any] | None) -> str | None:
    if not land:
        return None
    return _clean_text(land.get("submission_id")) or _clean_text(land.get("land_id"))


def _land_result_value(land: dict[str, Any] | None, key: str) -> str | None:
    if not land:
        return None
    direct = _clean_text(land.get(key))
    if direct:
        return direct
    result = land.get("result")
    if not isinstance(result, Mapping):
        result_json = land.get("result_json")
        if isinstance(result_json, str) and result_json.strip():
            try:
                parsed = json.loads(result_json)
            except json.JSONDecodeError:
                parsed = None
            result = parsed if isinstance(parsed, Mapping) else None
    if isinstance(result, Mapping):
        return _clean_text(result.get(key))
    return None


def _lineage_with_landed_snapshot(row: Mapping[str, Any] | None) -> dict[str, Any] | None:
    if not isinstance(row, Mapping):
        return None
    lineage = row.get("lineage")
    if not isinstance(lineage, Mapping):
        return None
    if not _clean_text(lineage.get("landed_snapshot_id")):
        return None
    return dict(lineage)


def _complete_land_gate_from_dependencies(
    row: dict[str, Any],
    dependency_rows: list[dict[str, Any]],
) -> bool:
    if str(row.get("node_kind") or "") != "land_gate":
        return False
    if row.get("blockers"):
        return False
    dependency_lineage = None
    for dependency in dependency_rows:
        dependency_lineage = _lineage_with_landed_snapshot(dependency)
        if dependency_lineage is not None:
            break
        evidence = dependency.get("evidence") if isinstance(dependency.get("evidence"), Mapping) else {}
        change_statuses = {str(value).strip().lower() for value in evidence.get("change_statuses", []) if _clean_text(value)}
        land_statuses = {str(value).strip().lower() for value in evidence.get("land_statuses", []) if _clean_text(value)}
        if "landed" in change_statuses or "succeeded" in land_statuses:
            dependency_lineage = dict(dependency.get("lineage") or {})
            break
    if dependency_lineage is None:
        return False
    lineage = row.get("lineage") if isinstance(row.get("lineage"), dict) else {}
    for field in ("task_id", "change_id", "patchset_id", "land_id", "landed_snapshot_id"):
        value = _clean_text(dependency_lineage.get(field))
        if value and not _clean_text(lineage.get(field)):
            lineage[field] = value
    row["lineage"] = lineage
    row["state"] = "completed"
    row["reason"] = "Dependency land evidence satisfies the land gate."
    row["blockers"] = []
    row["session_recommendation"] = {
        "action": "none",
        "reason": "Node is complete.",
        "session_id": row.get("owner_session_id"),
        "refuse_duplicate_execution": False,
    }
    return True


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


def _truthy(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    if value is None:
        return False
    return str(value).strip().lower() in {"1", "true", "yes", "on"}
