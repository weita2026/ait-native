from __future__ import annotations

import re
from pathlib import Path

from ait.plan_graph import load_task_graph, topological_node_order
from ait.repo_paths import RepoContext
from ait.task_dag_conversation import render_task_dag_conversation_progress
from ait.task_dag_readiness import compute_task_graph_readiness


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
MVP_PLAN_PATH = AUTHORED_ROOT / "docs/sprints/task_dag_scheduler_parallel_execution.md"
MVP_GRAPH_PATH = WORKSPACE_ROOT / "docs/sprints/task_dag_scheduler_parallel_execution.task_graph.json"


def _completed_task(node_id: str, plan_item_ref: str) -> dict:
    return {
        "task_id": f"T-{node_id}",
        "plan_item_ref": plan_item_ref,
        "status": "completed",
        "created_at": f"2026-01-01T00:00:{node_id}Z",
    }


def _active_task(node_id: str, plan_item_ref: str) -> dict:
    return {
        "task_id": f"T-{node_id}",
        "plan_item_ref": plan_item_ref,
        "status": "active",
        "created_at": f"2026-01-01T00:01:{node_id}Z",
    }


def _nodes_by_id(payload: dict) -> dict[str, dict]:
    return {node["node_id"]: node for node in payload["nodes"]}


def test_task_dag_mvp_fixture_contracts_match_plan_and_rollout_guardrails():
    graph = load_task_graph(MVP_GRAPH_PATH)
    plan_text = MVP_PLAN_PATH.read_text(encoding="utf-8")
    plan_refs = set(re.findall(r"\[ref: ([^\]]+)\]", plan_text))
    graph_refs = {node["plan_item_ref"] for node in graph["nodes"]}

    assert topological_node_order(graph) == ["A", "B", "C", "D", "E", "F", "G"]
    assert graph_refs == plan_refs
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/task_dag_scheduler_parallel_execution.md"
    assert graph["dispatch_artifacts"] == {
        "source_markdown": "docs/sprints/task_dag_scheduler.md",
        "parallel_execution_markdown": "docs/sprints/task_dag_scheduler_parallel_execution.md",
        "task_graph_json": "docs/sprints/task_dag_scheduler_parallel_execution.task_graph.json",
    }

    policy = graph["execution_policy"]
    assert policy["mode"] == "guarded_full_dag_convergence"
    assert policy["auto_create_tasks"] is False
    assert policy["auto_claim_nodes"] is False
    assert policy["requires_review_policy_land_gates"] is True
    assert policy["validate_source_plan_revision"] is True

    assert {tuple(group["nodes"]) for group in graph["parallel_groups"]} == {("C", "D")}
    assert set(graph["lineage_fields"]) >= {
        "plan_id",
        "plan_revision_id",
        "plan_item_ref",
        "node_id",
        "task_id",
        "change_id",
        "session_id",
        "checkpoint_id",
        "patchset_id",
        "land_id",
    }
    assert graph["progress_model"]["chat_summary_template"].startswith("DAG {completed_percent}% complete")


def test_task_dag_mvp_rollout_readiness_progress_and_conversation_are_consistent():
    graph = load_task_graph(MVP_GRAPH_PATH)
    refs = {node["node_id"]: node["plan_item_ref"] for node in graph["nodes"]}
    workflow = {
        "tasks": [
            _completed_task("A", refs["A"]),
            _completed_task("B", refs["B"]),
            _completed_task("C", refs["C"]),
            _completed_task("D", refs["D"]),
            _active_task("E", refs["E"]),
        ],
    }

    readiness = compute_task_graph_readiness(
        graph,
        workflow,
        current_plan_revision_id=graph["source_plan"]["plan_revision_id"],
    )
    nodes = _nodes_by_id(readiness)
    conversation = render_task_dag_conversation_progress(graph, readiness)

    assert readiness["counts"] == {"ready": 0, "running": 1, "blocked": 2, "completed": 4, "total": 7}
    assert readiness["summary"]["next_action"] == "unblock F"
    assert nodes["E"]["state"] == "running"
    assert nodes["F"]["blockers"][0]["type"] == "dependency"
    assert nodes["G"]["blockers"][0]["node_id"] == "E"

    assert conversation["text"].splitlines()[0] == (
        "DAG 57% complete (~62% active) · done 4/7 · running 1 · ready 0 · blocked 2 · next: unblock F"
    )
    assert conversation["blockers"][0] == {
        "node_id": "F",
        "reason": "Dependency E is running, not completed.",
    }
    assert conversation["progress"]["completed_percent"] == 57
    assert conversation["progress"]["estimated_percent"] == 62


def test_task_dag_mvp_stale_revision_blocks_ready_work_but_remains_advisory_readable():
    graph = load_task_graph(MVP_GRAPH_PATH)
    refs = {node["node_id"]: node["plan_item_ref"] for node in graph["nodes"]}
    workflow = {
        "tasks": [
            _completed_task("A", refs["A"]),
            _completed_task("B", refs["B"]),
            _completed_task("C", refs["C"]),
            _completed_task("D", refs["D"]),
        ],
    }

    readiness = compute_task_graph_readiness(graph, workflow, current_plan_revision_id="PR-newer")
    nodes = _nodes_by_id(readiness)
    conversation = render_task_dag_conversation_progress(graph, readiness)

    assert readiness["stale_source_plan"] is True
    assert readiness["counts"] == {"ready": 0, "running": 0, "blocked": 3, "completed": 4, "total": 7}
    assert nodes["E"]["blockers"][0]["type"] == "stale_plan_revision"
    assert conversation["text"].startswith("DAG 57% complete")
    assert conversation["blockers"][0]["node_id"] == "E"
