from __future__ import annotations

import json

from ait.task_dag_conversation import (
    load_conversation_task_dag_graph,
    render_task_dag_conversation_progress,
    should_render_task_dag_progress,
)


def _graph() -> dict:
    return {
        "schema_version": 1,
        "graph_id": "demo/task-dag",
        "repo_name": "demo",
        "source_plan": {
            "artifact_path": "docs/sprints/demo.md",
            "plan_id": "PL-demo",
            "plan_ref": "demo/root",
            "plan_revision_id": "PR-demo",
        },
        "execution_policy": {
            "mode": "guarded_full_dag_convergence",
            "default_mode": "local_execution_dag_with_selective_promotion",
            "dispatch_model": "compact_packet",
            "worker_execution_mode": "worker_only_compact_packet",
            "max_total_sessions": 1,
            "max_worker_sessions": 1,
            "max_batch_sessions": 1,
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Done",
                "depends_on": [],
                "progress_weight": 1,
                "task_template": {"title": "Do A"},
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Running",
                "depends_on": ["A"],
                "progress_weight": 1,
                "task_template": {"title": "Do B"},
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "Blocked",
                "depends_on": ["B"],
                "progress_weight": 1,
                "task_template": {"title": "Do C"},
            },
        ],
        "edges": [
            {"from": "A", "to": "B"},
            {"from": "B", "to": "C"},
        ],
    }


def test_render_task_dag_conversation_progress_line_and_stall_blockers():
    payload = render_task_dag_conversation_progress(
        _graph(),
        {
            "summary": {
                "total_nodes": 3,
                "completed_nodes": 1,
                "running_nodes": 1,
                "ready_nodes": 0,
                "blocked_nodes": 1,
                "next_action": "unblock C",
            },
            "nodes": [
                {"node_id": "A", "state": "completed", "reason": "done"},
                {"node_id": "B", "state": "running", "reason": "active"},
                {"node_id": "C", "state": "blocked", "reason": "Dependency B is running, not completed."},
            ],
        },
    )

    assert payload["text"].splitlines()[0] == (
        "DAG 33% complete (~45% active) · done 1/3 · running 1 · ready 0 · blocked 1 · next: unblock C"
    )
    assert payload["text"].splitlines()[1] == "Blocked: C - Dependency B is running, not completed."
    assert payload["progress"]["estimated_percent"] == 45
    assert payload["blockers"] == [{"node_id": "C", "reason": "Dependency B is running, not completed."}]


def test_should_render_task_dag_progress_requires_relevant_context():
    assert should_render_task_dag_progress(text="show task DAG progress") is True
    assert should_render_task_dag_progress(text="hello there") is False
    assert should_render_task_dag_progress(
        text="hello there",
        session={"metadata": {"task_graph_json": "docs/sprints/m3_one_hour_sprint_compression_demo.task_graph.json"}},
    ) is True


def test_load_conversation_task_dag_graph_prefers_session_subscription(tmp_path):
    execution_plans = tmp_path / "docs" / "sprints"
    execution_plans.mkdir(parents=True)
    default_graph = _graph()
    default_graph["graph_id"] = "default/parallel"
    subscribed_graph = _graph()
    subscribed_graph["graph_id"] = "subscribed/one-hour"
    subscribed_graph["source_plan"]["plan_id"] = "PL-one-hour"

    (execution_plans / "aaa_parallel_execution.task_graph.json").write_text(json.dumps(default_graph), encoding="utf-8")
    (execution_plans / "m3_one_hour.task_graph.json").write_text(json.dumps(subscribed_graph), encoding="utf-8")

    graph, path = load_conversation_task_dag_graph(
        tmp_path,
        repo_name="demo",
        task_graph_json="docs/sprints/m3_one_hour.task_graph.json",
        plan_id="PL-one-hour",
    )

    assert graph["graph_id"] == "subscribed/one-hour"
    assert path == (execution_plans / "m3_one_hour.task_graph.json").resolve()


def test_load_conversation_task_dag_graph_discovers_docs_sprints_fallback(tmp_path):
    sprint_docs = tmp_path / "docs" / "sprints"
    sprint_docs.mkdir(parents=True)
    graph_payload = _graph()
    (sprint_docs / "legacy.task_graph.json").write_text(json.dumps(graph_payload), encoding="utf-8")

    graph, path = load_conversation_task_dag_graph(tmp_path, repo_name="demo", plan_id="PL-demo")

    assert graph["graph_id"] == "demo/task-dag"
    assert path == (sprint_docs / "legacy.task_graph.json").resolve()
