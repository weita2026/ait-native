from __future__ import annotations

import json
from pathlib import Path

import ait_server.read_models as read_models
import ait_server.read_models_domains.task_dag as task_dag_domain
import ait_server.task_graph_runs as task_graph_runs
from ait_server import server_store
from ait_server.server_control import connect
from ait.task_dag_readiness import (
    build_task_dag_promotion_policy,
    build_task_graph_execution_strategy,
    compute_task_graph_readiness,
    task_dag_change_strategy,
    task_dag_final_remote_disposition_default,
)
from ait_server.read_models import (
    task_dag_graph_from_facts,
    task_dag_progress,
    task_dag_progress_from_facts,
    task_dag_readiness_from_facts,
    task_dag_schedule_from_facts,
)
from tests.postgres_fake import fake_postgres_context


REPO_ROOT = Path(__file__).resolve().parents[1]
MVP_GRAPH_PATH = REPO_ROOT / "docs/sprints/task_dag_scheduler_parallel_execution.task_graph.json"


def _graph() -> dict:
    return json.loads(MVP_GRAPH_PATH.read_text(encoding="utf-8"))


def _revision(graph: dict) -> str:
    return graph["source_plan"]["plan_revision_id"]


def _nodes_by_id(payload: dict) -> dict[str, dict]:
    return {node["node_id"]: node for node in payload["nodes"]}


def _completed_task(node_id: str, plan_item_ref: str) -> dict:
    return {
        "task_id": f"T-{node_id}",
        "plan_item_ref": plan_item_ref,
        "status": "completed",
        "created_at": f"2026-01-01T00:00:0{node_id}Z",
    }


def test_task_dag_readiness_marks_ready_completed_and_dependency_blocked():
    graph = _graph()
    workflow = {
        "tasks": [
            _completed_task("A", "task-dag-scheduler-parallel/dispatch-artifacts"),
            _completed_task("B", "task-dag-scheduler-parallel/json-loader"),
        ],
    }

    payload = compute_task_graph_readiness(graph, workflow, current_plan_revision_id=_revision(graph))
    nodes = _nodes_by_id(payload)

    assert payload["counts"] == {"ready": 2, "running": 0, "blocked": 3, "completed": 2, "total": 7}
    assert payload["summary"]["next_action"] == "start C"
    assert nodes["A"]["state"] == "completed"
    assert nodes["B"]["state"] == "completed"
    assert nodes["C"]["state"] == "ready"
    assert nodes["D"]["state"] == "ready"
    assert nodes["E"]["state"] == "blocked"
    assert nodes["E"]["blockers"][0]["type"] == "dependency"
    assert nodes["F"]["state"] == "blocked"
    assert nodes["G"]["state"] == "blocked"


def test_task_dag_readiness_prefers_effective_completed_task_over_newer_canceled_duplicate():
    graph = {
        "schema_version": 1,
        "graph_id": "graph-canceled-duplicate",
        "source_plan": {
            "artifact_path": "docs/sprints/graph-canceled-duplicate.md",
            "plan_id": "PL-demo",
            "plan_revision_id": "PR-demo",
        },
        "execution_policy": {
            "default_mode": "local_execution_dag_with_selective_promotion",
            "change_strategy": "local_first_final_local_land",
        },
        "nodes": [
            {
                "node_id": "D",
                "node_kind": "task",
                "title": "D",
                "plan_item_ref": "demo/d",
                "task_template": {"title": "Do D", "risk_tier": "medium"},
            },
            {
                "node_id": "E",
                "node_kind": "task",
                "title": "E",
                "plan_item_ref": "demo/e",
                "depends_on": ["D"],
                "task_template": {"title": "Do E", "risk_tier": "medium"},
            },
        ],
        "edges": [{"from": "D", "to": "E", "edge_kind": "depends_on"}],
    }
    workflow = {
        "tasks": [
            {
                "task_id": "LT-1184",
                "plan_item_ref": "demo/d",
                "status": "completed",
                "created_at": "2026-05-21T11:00:00Z",
                "updated_at": "2026-05-21T11:10:00Z",
            },
            {
                "task_id": "LT-1185",
                "plan_item_ref": "demo/d",
                "status": "canceled",
                "created_at": "2026-05-21T11:11:00Z",
                "updated_at": "2026-05-21T11:12:00Z",
            },
        ],
    }

    payload = compute_task_graph_readiness(graph, workflow, current_plan_revision_id="PR-demo")
    nodes = _nodes_by_id(payload)

    assert nodes["D"]["state"] == "completed"
    assert nodes["D"]["lineage"]["task_id"] == "LT-1184"
    assert nodes["E"]["state"] == "ready"
    assert payload["summary"]["next_action"] == "start E"


def test_task_dag_readiness_honors_explicit_local_progress_and_completion_states():
    graph = {
            "schema_version": 1,
            "graph_id": "graph-local-progress",
            "source_plan": {
                "artifact_path": "docs/sprints/graph-local-progress.md",
                "plan_id": "PL-demo",
                "plan_revision_id": "PR-demo",
            },
        "execution_policy": {
            "default_mode": "local_execution_dag_with_selective_promotion",
            "change_strategy": "local_first_final_remote_land",
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "A",
                "plan_item_ref": "demo/a",
                "workflow_boundary": "execution_only",
                "task_template": {"title": "A", "intent": "Do A", "risk_tier": "medium"},
            },
                {
                    "node_id": "B",
                    "node_kind": "task",
                    "title": "B",
                    "plan_item_ref": "demo/b",
                    "depends_on": ["A"],
                    "converged_output": True,
                    "task_template": {"title": "B", "intent": "Do B", "risk_tier": "medium"},
                },
        ],
        "edges": [{"from": "A", "to": "B", "edge_kind": "depends_on"}],
    }
    running_payload = compute_task_graph_readiness(
        graph,
        {
            "node_states": [
                {
                    "node_state_id": "NS-1",
                    "node_id": "A",
                    "state": "running",
                    "reason": "Local edits started in the bound worktree.",
                    "created_at": "2026-01-01T00:00:01Z",
                }
            ]
        },
        current_plan_revision_id="PR-demo",
    )
    running_nodes = _nodes_by_id(running_payload)
    assert running_nodes["A"]["state"] == "running"
    assert running_nodes["A"]["explicit_state"] == "running"
    assert running_nodes["A"]["reason"] == "Local edits started in the bound worktree."
    assert running_nodes["B"]["state"] == "blocked"

    completed_payload = compute_task_graph_readiness(
        graph,
        {
            "node_states": [
                {
                    "node_state_id": "NS-2",
                    "node_id": "A",
                    "state": "completed",
                    "reason": "Local lane acceptance passed and the task was closed honestly.",
                    "created_at": "2026-01-01T00:00:02Z",
                }
            ]
        },
        current_plan_revision_id="PR-demo",
    )
    completed_nodes = _nodes_by_id(completed_payload)
    assert completed_nodes["A"]["state"] == "completed"
    assert completed_nodes["A"]["reason"] == "Local lane acceptance passed and the task was closed honestly."
    assert completed_nodes["B"]["state"] == "ready"


def test_task_dag_readiness_reconciles_execution_only_ancestors_after_converged_completion():
    graph = {
        "schema_version": 1,
        "graph_id": "graph-completed-converged-reconciliation",
        "source_plan": {
            "artifact_path": "docs/sprints/graph-completed-converged-reconciliation.md",
            "plan_id": "PL-demo",
            "plan_revision_id": "PR-demo",
        },
        "execution_policy": {
            "default_mode": "local_execution_dag_with_selective_promotion",
            "change_strategy": "local_first_final_remote_land",
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "A",
                "plan_item_ref": "demo/a",
                "workflow_boundary": "execution_only",
                "task_template": {"title": "A", "intent": "Do A", "risk_tier": "medium"},
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "B",
                "plan_item_ref": "demo/b",
                "workflow_boundary": "execution_only",
                "depends_on": ["A"],
                "task_template": {"title": "B", "intent": "Do B", "risk_tier": "medium"},
            },
            {
                "node_id": "F",
                "node_kind": "task",
                "title": "F",
                "plan_item_ref": "demo/f",
                "depends_on": ["B"],
                "converged_output": True,
                "task_template": {"title": "F", "intent": "Converge F", "risk_tier": "medium"},
            },
            {"node_id": "L", "node_kind": "land_gate", "title": "Land", "depends_on": ["F"]},
        ],
        "edges": [
            {"from": "A", "to": "B", "edge_kind": "depends_on"},
            {"from": "B", "to": "F", "edge_kind": "depends_on"},
            {"from": "F", "to": "L", "edge_kind": "depends_on"},
        ],
    }
    workflow = {
        "tasks": [
            {
                "task_id": "T-F",
                "plan_item_ref": "demo/f",
                "status": "completed",
                "created_at": "2026-01-01T00:00:10Z",
            }
        ],
        "changes": [
            {
                "change_id": "C-F",
                "task_id": "T-F",
                "status": "landed",
                "current_patchset_id": "P-F",
                "created_at": "2026-01-01T00:01:00Z",
                "updated_at": "2026-01-01T00:05:00Z",
            }
        ],
        "patchsets": [
            {
                "patchset_id": "P-F",
                "change_id": "C-F",
                "base_snapshot_id": "SNP-base-F",
                "revision_snapshot_id": "SNP-rev-F",
                "evaluation_state": "pass",
                "created_at": "2026-01-01T00:02:00Z",
            }
        ],
        "land_requests": [
            {
                "submission_id": "LAND-F",
                "change_id": "C-F",
                "patchset_id": "P-F",
                "status": "succeeded",
                "created_at": "2026-01-01T00:06:00Z",
                "result_json": json.dumps({"landed_snapshot_id": "SNP-rev-F"}),
            }
        ],
    }

    payload = compute_task_graph_readiness(graph, workflow, current_plan_revision_id="PR-demo")
    nodes = _nodes_by_id(payload)

    assert nodes["A"]["state"] == "completed"
    assert nodes["B"]["state"] == "completed"
    assert nodes["F"]["state"] == "completed"
    assert nodes["L"]["state"] == "completed"
    assert payload["counts"] == {"ready": 0, "running": 0, "blocked": 0, "completed": 4, "total": 4}
    assert payload["summary"]["next_action"] == "complete task graph"
    assert "execution-only lineage is reconciled as completed" in nodes["A"]["reason"]


def test_server_task_dag_node_state_rows_extract_local_progress_events(monkeypatch):
    sessions = [
        {
            "session_id": "S-GRAPH-1",
            "session_kind": "task_graph_run",
            "metadata": {"graph_run_id": "graph-run-1"},
        }
    ]

    monkeypatch.setattr(
        task_dag_domain,
        "list_session_events",
        lambda ctx, session_id: [
            {
                "sequence": 1,
                "event_type": "task_graph.node_local_progress",
                "payload": {
                    "node_id": "A",
                    "status": "running",
                    "summary": "Bound worktree created and local edits started.",
                },
                "created_at": "2026-01-01T00:00:01Z",
            },
            {
                "sequence": 2,
                "event_type": "task_graph.node_completed",
                "payload": {
                    "node_id": "A",
                    "summary": "Task completed after local verification.",
                },
                "created_at": "2026-01-01T00:00:02Z",
            },
        ],
    )

    rows = task_dag_domain._task_dag_node_state_rows_from_sessions(object(), sessions)

    assert rows[0]["node_id"] == "A"
    assert rows[0]["state"] == "running"
    assert rows[0]["reason"] == "Bound worktree created and local edits started."
    assert rows[1]["node_id"] == "A"
    assert rows[1]["state"] == "completed"
    assert rows[1]["graph_run_id"] == "graph-run-1"


def test_task_dag_readiness_explains_gate_review_policy_and_land_blockers():
    graph = _graph()
    workflow = {
        "tasks": [
            _completed_task("B", "task-dag-scheduler-parallel/json-loader"),
            {
                "task_id": "T-C",
                "plan_item_ref": "task-dag-scheduler-parallel/readiness-read-model",
                "status": "active",
                "created_at": "2026-01-01T00:00:10Z",
            },
        ],
        "changes": [
            {
                "change_id": "C-C",
                "task_id": "T-C",
                "status": "review",
                "current_patchset_id": "P-C",
                "created_at": "2026-01-01T00:01:00Z",
                "updated_at": "2026-01-01T00:03:00Z",
            }
        ],
        "patchsets": [
            {
                "patchset_id": "P-C",
                "change_id": "C-C",
                "base_snapshot_id": "SNP-base-C",
                "revision_snapshot_id": "SNP-rev-C",
                "evaluation_state": "pending",
                "policy_required": True,
                "created_at": "2026-01-01T00:02:00Z",
            }
        ],
        "review_summaries": [{"change_id": "C-C", "patchset_id": "P-C", "blocking": 1, "approvals": 0}],
        "policy_statuses": [{"patchset_id": "P-C", "decision": "pending", "created_at": "2026-01-01T00:02:30Z"}],
        "land_requests": [
            {
                "submission_id": "LAND-C",
                "change_id": "C-C",
                "patchset_id": "P-C",
                "status": "blocked",
                "blocker_class": "POLICY_BLOCKED",
                "created_at": "2026-01-01T00:04:00Z",
            }
        ],
    }

    payload = compute_task_graph_readiness(graph, workflow, current_plan_revision_id=_revision(graph))
    node = _nodes_by_id(payload)["C"]

    assert node["state"] == "blocked"
    assert node["lineage"] == {
        "task_id": "T-C",
        "change_id": "C-C",
        "superseded_change_ids": [],
        "session_id": None,
        "checkpoint_id": None,
        "patchset_id": "P-C",
        "patchset_base_snapshot_id": "SNP-base-C",
        "patchset_revision_snapshot_id": "SNP-rev-C",
        "land_id": "LAND-C",
        "landed_snapshot_id": None,
        "task_run_id": None,
        "node_id": "C",
    }
    assert node["evidence"]["snapshot_ids"] == ["SNP-base-C", "SNP-rev-C"]
    assert {blocker["type"] for blocker in node["blockers"]} == {"gate", "review", "policy", "land"}


def test_task_dag_readiness_explains_stale_plan_revision_and_hotspot_claims():
    graph = _graph()
    workflow = {
        "tasks": [_completed_task("B", "task-dag-scheduler-parallel/json-loader")],
        "hotspot_claims": [
            {
                "hotspot_key": "module:ait_server.read_models",
                "holder_node_id": "D",
                "status": "active",
            }
        ],
    }

    payload = compute_task_graph_readiness(graph, workflow, current_plan_revision_id="PR-newer")
    node = _nodes_by_id(payload)["C"]

    assert payload["stale_source_plan"] is True
    assert node["state"] == "blocked"
    assert [blocker["type"] for blocker in node["blockers"]] == ["stale_plan_revision", "hotspot"]
    assert node["blockers"][0]["source_plan_revision_id"] == _revision(graph)
    assert node["blockers"][1]["holder_node_id"] == "D"


def test_task_dag_readiness_marks_land_gate_completed_after_dependency_land():
    graph = {
        "schema_version": 1,
        "graph_id": "demo/land-gate-complete",
        "repo_name": "demo",
        "source_plan": {
            "artifact_path": "docs/sprints/demo_land_gate.md",
            "plan_id": "PL-demo",
            "plan_ref": "demo/root",
            "plan_revision_id": "PR-demo",
        },
        "execution_policy": {
            "solo_gate_strategy": task_graph_runs.TASK_DAG_SOLO_GATE_STRATEGY,
            "final_gate_bundle": ["review", "attestation", "policy", "land"],
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Converged output",
                "plan_item_ref": "demo/a",
                "depends_on": [],
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
                "task_template": {"title": "Do A", "risk_tier": "medium"},
            },
            {
                "node_id": "L",
                "node_kind": "land_gate",
                "title": "Accept remote land",
                "depends_on": ["A"],
                "completion_rule": "selected patchset is landed on target line",
                "safety_boundary_reason": "final governance boundary",
            },
        ],
        "edges": [{"from": "A", "to": "L", "edge_kind": "depends_on"}],
    }
    workflow = {
        "tasks": [{"task_id": "T-A", "plan_item_ref": "demo/a", "status": "completed"}],
        "changes": [
            {
                "change_id": "C-A",
                "task_id": "T-A",
                "status": "landed",
                "current_patchset_id": "P-A",
            }
        ],
        "patchsets": [
            {
                "patchset_id": "P-A",
                "change_id": "C-A",
                "base_snapshot_id": "SNP-base-A",
                "revision_snapshot_id": "SNP-rev-A",
            }
        ],
        "lands": [
            {
                "submission_id": "LAND-A",
                "change_id": "C-A",
                "patchset_id": "P-A",
                "status": "succeeded",
                "result": {"landed_snapshot_id": "SNP-rev-A", "target_line": "main"},
            }
        ],
    }

    payload = compute_task_graph_readiness(graph, workflow, current_plan_revision_id="PR-demo")
    nodes = _nodes_by_id(payload)

    assert nodes["A"]["state"] == "completed"
    assert nodes["L"]["state"] == "completed"
    assert nodes["L"]["reason"] == "Dependency land evidence satisfies the land gate."
    assert nodes["L"]["lineage"]["change_id"] == "C-A"
    assert nodes["L"]["lineage"]["land_id"] == "LAND-A"
    assert nodes["L"]["lineage"]["landed_snapshot_id"] == "SNP-rev-A"
    assert nodes["L"]["session_recommendation"]["action"] == "none"
    assert payload["summary"]["next_action"] == "complete task graph"


def test_task_dag_server_auto_continue_profile_defaults_to_final_remote_disposition():
    graph = {
        "execution_policy": {
            "default_mode": "local_execution_dag_with_selective_promotion",
            "final_gate_bundle": ["review", "attestation", "policy", "land"],
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
            }
        ],
    }

    profile = task_graph_runs._task_dag_auto_continue_profile(graph)

    assert profile["auto_continue_supported"] is True
    assert profile["auto_node_bootstrap_supported"] is True
    assert profile["change_strategy"] == "local_first_final_remote_land"
    assert profile["final_remote_disposition_default"] is True
    assert profile["converged_output_node_ids"] == ["A"]
    assert profile["final_gate_bundle"] == ["review", "attestation", "policy", "land"]


def test_task_dag_promotion_policy_prefers_canonical_final_remote_disposition():
    graph = {
        "execution_policy": {"default_mode": "local_execution_dag_with_selective_promotion"},
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "workflow_boundary": "execution_only",
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
            },
        ],
    }

    policy = build_task_dag_promotion_policy(graph, workflow_mode="solo_remote")

    assert policy["selective_promotion_default"] is True
    assert policy["change_strategy"] == "local_first_final_remote_land"
    assert policy["final_remote_disposition_default"] is True
    assert policy["local_lineage_allowed_for_execution_only"] is True
    assert policy["shared_promotion_boundary"] == "single_final_reviewable_output_then_remote_land"
    assert "local task/change/snapshot/local-land lineage" in policy["current_boundary"]
    assert policy["promotion_helper_command"] == 'ait workflow publish --task <task-id> --summary "final output" --target-line main'


def test_task_dag_promotion_policy_supports_explicit_final_local_land_strategy():
    graph = {
        "execution_policy": {
            "default_mode": "local_execution_dag_with_selective_promotion",
            "change_strategy": "local_first_final_local_land",
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "workflow_boundary": "execution_only",
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
            },
        ],
    }

    policy = build_task_dag_promotion_policy(graph, workflow_mode="solo_remote")

    assert task_dag_change_strategy(graph, workflow_mode="solo_remote") == "local_first_final_local_land"
    assert task_dag_final_remote_disposition_default(graph, workflow_mode="solo_remote") is False
    assert policy["change_strategy"] == "local_first_final_local_land"
    assert policy["final_land_disposition"] == "local"
    assert policy["final_remote_disposition_default"] is False
    assert policy["later_remote_promotion_allowed_after_local_land"] is True
    assert policy["shared_promotion_boundary"] == "single_final_reviewable_output_then_local_land"
    assert "final local-land boundary" in policy["current_boundary"]
    assert "later remote-promote through the repo-wide completed-local helper" in policy["current_boundary"]
    assert policy["promotion_helper_command"] == "ait workflow land-local <change-id>"
    assert policy["later_remote_promotion_helper_command"] == "ait workflow land --all-completed-local --remote <name>"


def test_server_read_model_fact_shim_uses_shared_readiness_logic():
    graph = _graph()
    payload = task_dag_readiness_from_facts(
        graph,
        {"tasks": [_completed_task("B", "task-dag-scheduler-parallel/json-loader")]},
        current_plan_revision_id=_revision(graph),
    )

    assert payload["graph_id"] == "task-dag-scheduler/parallel-execution-mvp"
    assert _nodes_by_id(payload)["C"]["state"] == "ready"


def test_task_dag_server_workflow_facts_scope_to_graph_links(monkeypatch):
    graph = _graph()
    refs = {node["node_id"]: node["plan_item_ref"] for node in graph["nodes"]}
    calls = {
        "get_change": [],
        "checkpoints": [],
        "patchsets": [],
        "reviews": [],
        "policies": [],
        "lands": [],
    }

    monkeypatch.setattr(
        read_models,
        "list_tasks",
        lambda ctx, repo_name: [
            {"task_id": "T-C", "plan_id": graph["source_plan"]["plan_id"], "plan_item_ref": refs["C"], "status": "active"},
            {"task_id": "T-cross", "plan_id": "PL-other", "plan_item_ref": refs["C"], "status": "active"},
            {"task_id": "T-ignore", "plan_item_ref": "elsewhere/ref", "status": "active"},
        ],
    )
    monkeypatch.setattr(
        read_models,
        "list_changes",
        lambda ctx, repo_name: [
            {"change_id": "C-C", "task_id": "T-C", "status": "review"},
            {"change_id": "C-ignore", "task_id": "T-ignore", "status": "review"},
        ],
    )

    def fake_get_change(ctx, change_id):
        calls["get_change"].append(change_id)
        return {
            "change_id": change_id,
            "task_id": "T-C",
            "status": "review",
            "current_patchset_id": "P-C",
            "selected_patchset_id": "P-C",
        }

    monkeypatch.setattr(read_models, "get_change", fake_get_change)
    monkeypatch.setattr(
        read_models,
        "list_sessions",
        lambda ctx, repo_name: [
            {"session_id": "S-C", "task_id": "T-C", "change_id": "C-C", "status": "active"},
            {"session_id": "S-ignore", "task_id": "T-ignore", "change_id": "C-ignore", "status": "active"},
        ],
    )

    def fake_list_session_checkpoints(ctx, session_id):
        calls["checkpoints"].append(session_id)
        return [{"checkpoint_id": "K-C", "session_id": session_id}]

    monkeypatch.setattr(read_models, "list_session_checkpoints", fake_list_session_checkpoints)

    def fake_list_patchsets(ctx, change_id):
        calls["patchsets"].append(change_id)
        return [
            {
                "patchset_id": "P-C",
                "change_id": change_id,
                "base_snapshot_id": "SNP-base",
                "revision_snapshot_id": "SNP-rev",
                "evaluation_state": "pass",
                "policy_required": True,
            }
        ]

    monkeypatch.setattr(read_models, "list_patchsets", fake_list_patchsets)

    def fake_list_reviews(ctx, change_id):
        calls["reviews"].append(change_id)
        return {"approvals": 1, "blocking": 0, "comments": 0}

    monkeypatch.setattr(read_models, "list_reviews", fake_list_reviews)

    def fake_latest_land_summary(ctx, change_id):
        calls["lands"].append(change_id)
        return {"submission_id": "LAND-C", "change_id": change_id, "patchset_id": "P-C", "status": "queued"}

    monkeypatch.setattr(read_models, "_latest_land_summary", fake_latest_land_summary)

    def fake_get_policy_status(ctx, patchset_id):
        calls["policies"].append(patchset_id)
        return {"patchset_id": patchset_id, "decision": "pass"}

    monkeypatch.setattr(read_models, "get_policy_status", fake_get_policy_status)

    workflow, resolved_revision_id = read_models._task_dag_workflow_facts(
        object(),
        graph,
        current_plan_revision_id=_revision(graph),
    )

    assert resolved_revision_id == _revision(graph)
    assert [task["task_id"] for task in workflow["tasks"]] == ["T-C"]
    assert [change["change_id"] for change in workflow["changes"]] == ["C-C"]
    assert [session["session_id"] for session in workflow["sessions"]] == ["S-C"]
    assert calls == {
        "get_change": ["C-C"],
        "checkpoints": ["S-C"],
        "patchsets": ["C-C"],
        "reviews": ["C-C"],
        "policies": ["P-C"],
        "lands": ["C-C"],
    }


def test_task_dag_readiness_tracks_task_run_owner_surface_and_session_policy():
    graph = _graph()
    refs = {node["node_id"]: node["plan_item_ref"] for node in graph["nodes"]}
    workflow = {
        "tasks": [
            _completed_task("A", refs["A"]),
            _completed_task("B", refs["B"]),
            {
                "task_id": "T-C",
                "plan_item_ref": refs["C"],
                "status": "active",
                "created_at": "2026-01-01T00:00:10Z",
            },
        ],
        "task_runs": [
            {
                "task_run_id": "TR-C",
                "node_id": "C",
                "task_id": "T-C",
                "session_id": "S-C",
                "status": "running",
                "claimed_by": "agent-a",
                "hotspot_keys": ["module:ait_server.read_models"],
                "created_at": "2026-01-01T00:01:00Z",
            }
        ],
        "surface_bindings": [
            {
                "surface": "telegram",
                "surface_id": "chat-1",
                "session_id": "S-C",
                "status": "active",
            }
        ],
    }

    payload = compute_task_graph_readiness(graph, workflow, current_plan_revision_id=_revision(graph))
    node = _nodes_by_id(payload)["C"]

    assert node["state"] == "running"
    assert node["lineage"]["task_run_id"] == "TR-C"
    assert node["owner_session_id"] == "S-C"
    assert node["session_recommendation"]["action"] == "reuse_primary_session"
    assert node["surface_bindings"] == [
        {"surface": "telegram", "surface_id": "chat-1", "session_id": "S-C", "status": "active"}
    ]


def test_task_dag_readiness_supports_explicit_gate_results():
    graph = _graph()
    graph["nodes"][0]["gate_rules"] = ["contract:docs-approved"]
    workflow = {"gate_results": [{"gate": "contract:docs-approved", "status": "pending"}]}

    payload = compute_task_graph_readiness(graph, workflow, current_plan_revision_id=_revision(graph))
    node = _nodes_by_id(payload)["A"]

    assert node["state"] == "blocked"
    assert node["blockers"][0]["code"] == "explicit_gate_not_satisfied"

    workflow["gate_results"][0]["status"] = "passed"
    payload = compute_task_graph_readiness(graph, workflow, current_plan_revision_id=_revision(graph))
    assert _nodes_by_id(payload)["A"]["state"] == "ready"


def test_task_dag_readiness_treats_waived_gate_results_as_blocked():
    graph = _graph()
    graph["nodes"][0]["gate_rules"] = ["contract:docs-approved"]
    workflow = {"gate_results": [{"gate": "contract:docs-approved", "status": "waived"}]}

    payload = compute_task_graph_readiness(graph, workflow, current_plan_revision_id=_revision(graph))
    node = _nodes_by_id(payload)["A"]

    assert node["state"] == "blocked"
    assert node["blockers"][0]["code"] == "explicit_gate_not_satisfied"
    assert node["blockers"][0]["message"] == "Gate contract:docs-approved is waived."


def test_task_dag_server_fact_views_expose_graph_schedule_and_progress():
    graph = _graph()
    graph["execution_policy"]["worker_execution_mode"] = "worker_only_compact_packet"
    workflow = {"tasks": [_completed_task("A", "task-dag-scheduler-parallel/dispatch-artifacts")]}

    graph_payload = task_dag_graph_from_facts(graph, workflow, current_plan_revision_id=_revision(graph))
    schedule_payload = task_dag_schedule_from_facts(graph, workflow, current_plan_revision_id=_revision(graph))
    progress_payload = task_dag_progress_from_facts(graph, workflow, current_plan_revision_id=_revision(graph))

    assert graph_payload["nodes"][0]["task_id"] == "T-A"
    assert schedule_payload["ready"][0]["node_id"] == "B"
    assert schedule_payload["ready"][0]["session_recommendation"]["action"] == "open_new_session"
    assert schedule_payload["execution_strategy"]["default_mode"] == "local_execution_dag_with_selective_promotion"
    assert schedule_payload["execution_strategy"]["dispatch_model"] == "compact_packet"
    assert schedule_payload["execution_strategy"]["worker_execution_mode"] == "worker_only_compact_packet"
    assert (
        schedule_payload["execution_strategy"]["worker_execution_label"]
        == "worker-only compact packet executed in one fresh worker session"
    )
    assert schedule_payload["execution_strategy"]["worker_session_mode"] == "single_fresh_worker_session"
    assert schedule_payload["execution_strategy"]["max_total_sessions"] == 1
    assert schedule_payload["execution_strategy"]["max_worker_sessions"] == 1
    assert schedule_payload["execution_strategy"]["physical_fanout_default"] is False
    assert progress_payload["progress"]["completed_percent"] == 14


def test_task_dag_progress_exposes_latest_graph_run_summary(monkeypatch):
    graph = _graph()

    monkeypatch.setattr(read_models, "get_plan", lambda ctx, plan_id: {"head_revision": {"plan_revision_id": _revision(graph)}})
    monkeypatch.setattr(read_models, "list_tasks", lambda ctx, repo_name: [])
    monkeypatch.setattr(read_models, "list_changes", lambda ctx, repo_name: [])
    monkeypatch.setattr(
        read_models,
        "list_sessions",
        lambda ctx, repo_name: [
            {
                "session_id": "S-RUN",
                "session_local_id": "0001",
                "repo_name": "ait",
                "repo_id": "REPO-123",
                "session_kind": "task_graph_run",
                "metadata": {
                    "plan_id": graph["source_plan"]["plan_id"],
                    "graph_id": graph["graph_id"],
                    "graph_run_id": "graph-run-1",
                },
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:02:00Z",
            }
        ],
    )
    monkeypatch.setattr(read_models, "list_session_checkpoints", lambda ctx, session_id: [])
    monkeypatch.setattr(
        read_models,
        "list_session_events",
        lambda ctx, session_id: [
            {
                "sequence": 1,
                "event_type": "task_graph.execution_started",
                "payload": {"workflow_summary": {"next_action": "start A"}},
            },
            {
                "sequence": 2,
                "event_type": "task_graph.state_snapshot",
                "payload": {
                    "execution_state": "waiting_for_review",
                    "pause_reason": "review",
                    "next_action": "inspect review",
                    "gate_handoff": {
                        "kind": "converged_gate_bundle",
                        "candidate_node_ids": ["C"],
                        "candidate_change_ids": ["C-123"],
                        "required_gates": ["review", "attestation", "policy", "land"],
                        "promotion_required": False,
                    },
                    "workflow_summary": {"next_action": "inspect review"},
                },
            },
        ],
    )

    payload = task_dag_progress(object(), graph)

    assert payload["latest_graph_run"] == {
        "session_id": "S-RUN",
        "session_local_id": "0001",
        "repo_name": "ait",
        "repo_id": "REPO-123",
        "graph_run_id": "graph-run-1",
        "execution_state": "waiting_for_review",
        "pause_reason": "review",
        "next_action": "inspect review",
        "gate_handoff": {
            "kind": "converged_gate_bundle",
            "candidate_node_ids": ["C"],
            "candidate_change_ids": ["C-123"],
            "required_gates": ["review", "attestation", "policy", "land"],
            "promotion_required": False,
        },
        "latest_event_type": "task_graph.state_snapshot",
        "latest_event_sequence": 2,
        "event_count": 2,
        "workflow_summary": {"next_action": "inspect review"},
    }


def test_task_dag_progress_ignores_unrelated_latest_graph_run(monkeypatch):
    graph = _graph()

    monkeypatch.setattr(read_models, "get_plan", lambda ctx, plan_id: {"head_revision": {"plan_revision_id": _revision(graph)}})
    monkeypatch.setattr(read_models, "list_tasks", lambda ctx, repo_name: [])
    monkeypatch.setattr(read_models, "list_changes", lambda ctx, repo_name: [])
    monkeypatch.setattr(
        read_models,
        "list_sessions",
        lambda ctx, repo_name: [
            {
                "session_id": "S-MATCH",
                "session_local_id": "0001",
                "repo_name": "ait",
                "repo_id": "REPO-123",
                "session_kind": "task_graph_run",
                "metadata": {
                    "plan_id": graph["source_plan"]["plan_id"],
                    "graph_id": graph["graph_id"],
                    "graph_run_id": "graph-run-match",
                },
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:01:00Z",
            },
            {
                "session_id": "S-OTHER",
                "session_local_id": "0002",
                "repo_name": "ait",
                "repo_id": "REPO-123",
                "session_kind": "task_graph_run",
                "metadata": {
                    "plan_id": "PL-OTHER",
                    "graph_id": "other-graph",
                    "graph_run_id": "graph-run-other",
                },
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:02:00Z",
            },
        ],
    )
    monkeypatch.setattr(read_models, "list_session_checkpoints", lambda ctx, session_id: [])

    def fake_events(ctx, session_id):
        if session_id == "S-MATCH":
            return [
                {
                    "sequence": 1,
                    "event_type": "task_graph.state_snapshot",
                    "payload": {
                        "execution_state": "waiting_for_review",
                        "next_action": "inspect matching review",
                        "workflow_summary": {"next_action": "inspect matching review"},
                    },
                }
            ]
        if session_id == "S-OTHER":
            return [
                {
                    "sequence": 1,
                    "event_type": "task_graph.state_snapshot",
                    "payload": {
                        "execution_state": "active",
                        "next_action": "inspect unrelated graph",
                        "workflow_summary": {"next_action": "inspect unrelated graph"},
                    },
                }
            ]
        return []

    monkeypatch.setattr(read_models, "list_session_events", fake_events)

    payload = task_dag_progress(object(), graph)

    assert payload["latest_graph_run"]["session_id"] == "S-MATCH"
    assert payload["latest_graph_run"]["graph_run_id"] == "graph-run-match"
    assert payload["latest_graph_run"]["next_action"] == "inspect matching review"


def test_advance_task_dag_run_records_server_side_progress(monkeypatch):
    graph = _graph()
    workflow = {"tasks": [_completed_task("A", "task-dag-scheduler-parallel/dispatch-artifacts")]}
    readiness = task_dag_readiness_from_facts(graph, workflow, current_plan_revision_id=_revision(graph))
    previous_snapshot = {
        "graph_run_id": "graph-run-1",
        "execution_state": "waiting_for_dependency_completion",
        "plan_id": graph["source_plan"]["plan_id"],
        "plan_revision_id": _revision(graph),
        "graph_id": graph["graph_id"],
        "graph_artifact_path": "docs/sprints/task_dag_scheduler_parallel_execution.task_graph.json",
        "workflow_summary": {"ready_nodes": 0, "blocked_nodes": 1, "completed_nodes": 1, "total_nodes": 7, "next_action": "wait for B"},
        "readiness_summary": {"ready_nodes": 0, "blocked_nodes": 1, "completed_nodes": 1, "next_action": "wait for B"},
        "ready_node_ids": [],
        "running_node_ids": [],
        "blocked_node_ids": ["B"],
        "completed_node_ids": ["A"],
        "dispatched_node_ids": [],
        "next_action": "wait for B",
        "readiness_digest": "digest-before",
    }
    appended = []

    monkeypatch.setattr(
        read_models,
        "task_dag_readiness",
        lambda ctx, current_graph, current_plan_revision_id=None: readiness,
    )
    monkeypatch.setattr(
        task_graph_runs,
        "get_session",
        lambda ctx, session_id: {
            "session_id": session_id,
            "repo_name": "ait",
            "session_kind": "task_graph_run",
            "status": "active",
            "title": "Task DAG execute",
            "metadata": {
                "plan_id": graph["source_plan"]["plan_id"],
                "plan_revision_id": _revision(graph),
                "graph_id": graph["graph_id"],
                "graph_run_id": "graph-run-1",
            },
        },
    )
    monkeypatch.setattr(
        task_graph_runs,
        "list_session_events",
        lambda ctx, session_id: [
            {"sequence": 1, "event_type": "task_graph.execution_started", "payload": {"workflow_summary": {"next_action": "wait for B"}}},
            {"sequence": 2, "event_type": "task_graph.state_snapshot", "payload": previous_snapshot},
        ],
    )
    monkeypatch.setattr(
        task_graph_runs,
        "append_session_event",
        lambda ctx, session_id, event_type, payload, actor_identity=None, actor_type=None: (
            appended.append({"session_id": session_id, "event_type": event_type, "payload": payload})
            or {"event_id": f"EV-{len(appended)}", "event_type": event_type, "payload": payload}
        ),
    )
    monkeypatch.setattr(
        task_graph_runs,
        "_task_dag_auto_bootstrap_ready_node_ids",
        lambda *args, **kwargs: [],
    )

    payload = task_graph_runs.advance_task_dag_run(object(), "S-RUN", graph, current_plan_revision_id=_revision(graph))

    assert payload["advanced"] is True
    assert payload["newly_unblocked_node_ids"] == ["B"]
    assert appended[0]["event_type"] == "task_graph.execution_advanced"
    assert appended[1]["event_type"] == "task_graph.state_snapshot"
    assert payload["latest_state_snapshot"]["ready_node_ids"] == ["B"]


def test_server_task_dag_state_snapshot_marks_local_first_convergence_gate():
    graph = {
        "graph_id": "graph-local-first-final-land",
        "source_plan": {"plan_id": "PL-demo", "plan_revision_id": "PR-demo"},
        "execution_policy": {
            "default_mode": "local_execution_dag_with_selective_promotion",
            "change_strategy": "local_first_final_remote_land",
            "solo_gate_strategy": task_graph_runs.TASK_DAG_SOLO_GATE_STRATEGY,
            "final_gate_bundle": ["review", "attestation", "policy", "land"],
        },
        "nodes": [
            {
                "node_id": "G",
                "node_kind": "task",
                "title": "Converged output",
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
            }
        ],
        "edges": [],
    }
    readiness = {
        "nodes": [
            {
                "node_id": "G",
                "node_kind": "task",
                "title": "Converged output",
                "state": "completed",
                "reason": "Completed locally and ready for the final remote gate bundle.",
                "depends_on": [],
                "lock_keys": [],
                "hotspot_keys": [],
                "lineage": {
                    "task_id": "T-G",
                    "change_id": "C-G",
                    "landed_snapshot_id": None,
                },
                "task_id": "T-G",
                "change_id": "C-G",
                "session_recommendation": {"action": "none"},
                "blockers": [],
            }
        ],
        "summary": {"next_action": "run final gate bundle"},
    }

    snapshot = task_graph_runs._task_dag_state_snapshot_payload(
        plan_id="PL-demo",
        plan_revision_id="PR-demo",
        graph=graph,
        readiness=readiness,
        graph_run_id="graph-run-1",
    )

    assert snapshot["change_strategy"] == "local_first_final_remote_land"
    assert snapshot["final_remote_disposition_default"] is True
    assert snapshot["gate_handoff"]["kind"] == "converged_gate_bundle"
    assert snapshot["gate_handoff"]["promotion_required"] is False
    assert snapshot["gate_handoff"]["local_convergence_state"] == "converged_output_ready_for_remote_gate_bundle"
    assert snapshot["gate_handoff"]["final_remote_land_ready"] is True


def test_server_task_dag_state_snapshot_marks_local_final_land_gate():
    graph = {
        "graph_id": "graph-local-final-land",
        "source_plan": {"plan_id": "PL-demo", "plan_revision_id": "PR-demo"},
        "execution_policy": {
            "default_mode": "local_execution_dag_with_selective_promotion",
            "change_strategy": "local_first_final_local_land",
            "solo_gate_strategy": task_graph_runs.TASK_DAG_SOLO_GATE_STRATEGY,
        },
        "nodes": [
            {
                "node_id": "G",
                "node_kind": "task",
                "title": "Converged output",
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
            }
        ],
        "edges": [],
    }
    readiness = {
        "nodes": [
            {
                "node_id": "G",
                "node_kind": "task",
                "title": "Converged output",
                "state": "completed",
                "reason": "Completed locally and ready for final local land.",
                "depends_on": [],
                "lock_keys": [],
                "hotspot_keys": [],
                "lineage": {
                    "task_id": "T-G",
                    "change_id": "C-G",
                    "landed_snapshot_id": None,
                },
                "task_id": "T-G",
                "change_id": "C-G",
                "session_recommendation": {"action": "none"},
                "blockers": [],
            }
        ],
        "summary": {"next_action": "run final local land"},
    }

    snapshot = task_graph_runs._task_dag_state_snapshot_payload(
        plan_id="PL-demo",
        plan_revision_id="PR-demo",
        graph=graph,
        readiness=readiness,
        graph_run_id="graph-run-1",
    )

    assert snapshot["change_strategy"] == "local_first_final_local_land"
    assert snapshot["final_land_disposition"] == "local"
    assert snapshot["final_remote_disposition_default"] is False
    assert snapshot["gate_handoff"]["kind"] == "converged_gate_bundle"
    assert snapshot["gate_handoff"]["promotion_required"] is False
    assert snapshot["gate_handoff"]["required_gates"] == []
    assert snapshot["gate_handoff"]["local_convergence_state"] == "converged_output_ready_for_local_land"
    assert snapshot["gate_handoff"]["final_remote_land_ready"] is False
    assert snapshot["gate_handoff"]["final_local_land_ready"] is True
    assert snapshot["gate_handoff"]["next_action"] == "run converged local land"


def test_server_task_dag_bootstrap_node_uses_target_line_for_reviewable_output(tmp_path):
    ctx = fake_postgres_context(tmp_path / "server-data-bootstrap-target-line")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "ait", "main")
    server_store.update_line(ctx, "ait", "release", None)
    graph = {
        "graph_id": "graph-bootstrap-target-line",
        "source_plan": {"plan_id": "", "plan_revision_id": ""},
        "execution_policy": {
            "default_mode": "local_execution_dag_with_selective_promotion",
            "change_strategy": "local_first_final_remote_land",
            "target_line": "release",
        },
        "nodes": [
            {
                "node_id": "G",
                "node_kind": "task",
                "title": "Promote final output",
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
                "target_line": "release",
                "task_template": {"change_title": "Promoted final output", "risk_tier": "medium"},
            }
        ],
        "edges": [],
    }
    readiness = {
        "nodes": [
            {
                "node_id": "G",
                "node_kind": "task",
                "title": "Promote final output",
                "state": "ready",
                "reason": "Ready to bootstrap reviewable output.",
                "depends_on": [],
                "lock_keys": [],
                "hotspot_keys": [],
                "lineage": {
                    "task_id": None,
                    "change_id": None,
                    "landed_snapshot_id": None,
                },
                "task_id": None,
                "change_id": None,
                "session_recommendation": {"action": "start"},
                "blockers": [],
            }
        ],
        "summary": {"next_action": "start G"},
    }

    payload = task_graph_runs._task_dag_bootstrap_node(
        ctx,
        "ait",
        plan_id="",
        plan_revision_id=None,
        graph=graph,
        readiness=readiness,
        node_id="G",
        graph_run_session_id=None,
        actor_identity="alice@example.com",
        actor_type="human",
    )

    assert payload["created_change"]["base_line"] == "release"
    assert payload["created_change"]["forked_from_line"] == "release"
    assert payload["session"]["metadata"]["change_strategy"] == "local_first_final_remote_land"
    assert payload["session"]["metadata"]["final_remote_disposition_default"] is True


def test_server_task_dag_bootstrap_execution_only_node_marks_local_lineage_allowed(tmp_path):
    ctx = fake_postgres_context(tmp_path / "server-data-bootstrap-execution-only")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "ait", "main")
    graph = {
        "graph_id": "graph-bootstrap-execution-only",
        "source_plan": {"plan_id": "", "plan_revision_id": ""},
        "execution_policy": {
            "default_mode": "local_execution_dag_with_selective_promotion",
            "change_strategy": "local_first_final_remote_land",
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Execution-only node",
                "workflow_boundary": "execution_only",
                "task_template": {"change_title": "Should stay local", "risk_tier": "medium"},
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Converged output node",
                "converged_output": True,
            },
        ],
        "edges": [{"from": "A", "to": "B", "edge_kind": "depends_on"}],
    }
    readiness = {
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Execution-only node",
                "state": "ready",
                "reason": "Ready to bootstrap execution-only node.",
                "depends_on": [],
                "lock_keys": [],
                "hotspot_keys": [],
                "lineage": {"task_id": None, "change_id": None, "landed_snapshot_id": None},
                "task_id": None,
                "change_id": None,
                "session_recommendation": {"action": "start"},
                "blockers": [],
            }
        ],
        "summary": {"next_action": "start A"},
    }

    payload = task_graph_runs._task_dag_bootstrap_node(
        ctx,
        "ait",
        plan_id="",
        plan_revision_id=None,
        graph=graph,
        readiness=readiness,
        node_id="A",
        graph_run_session_id=None,
        actor_identity="alice@example.com",
        actor_type="human",
    )

    assert payload["created_change"] is None
    assert payload["change_id"] == ""
    assert payload["workflow_boundary"] == "execution_only"
    assert payload["session"]["metadata"]["local_lineage_allowed"] is True
    assert payload["session"]["metadata"]["remote_change_required"] is False
    assert payload["session"]["metadata"]["remote_workflow_allowed"] is False
    assert payload["session"]["metadata"]["dag_shared_boundary_node"] is False


def test_advance_task_dag_run_auto_bootstraps_execution_only_node_without_create_task_crash(tmp_path, monkeypatch):
    data_dir = tmp_path / "server-data"
    ctx = fake_postgres_context(data_dir)
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "ait", "main")
    graph = {
        "graph_id": "graph-execution-only",
        "source_plan": {"plan_id": "", "plan_revision_id": ""},
        "execution_policy": {
            "solo_gate_strategy": task_graph_runs.TASK_DAG_SOLO_GATE_STRATEGY,
            "final_gate_bundle": ["review", "attestation", "policy", "land"],
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Bootstrap execution-only node",
                "workflow_boundary": "execution_only",
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Converged output node",
                "converged_output": True,
            },
        ],
        "edges": [{"from": "A", "to": "B", "edge_kind": "depends_on"}],
    }
    run_session = server_store.create_session(
        ctx,
        "ait",
        "task_graph_run",
        title="Task DAG execute",
        metadata={
            "graph_id": "graph-execution-only",
            "graph_run_id": "graph-run-test",
        },
    )
    readiness_calls = {"count": 0}

    def fake_readiness(_ctx, _graph, current_plan_revision_id=None):
        assert current_plan_revision_id is None
        readiness_calls["count"] += 1
        tasks = server_store.list_tasks(ctx, "ait")
        task_id = tasks[0]["task_id"] if tasks else None
        return {
            "nodes": [
                {
                    "node_id": "A",
                    "node_kind": "task",
                    "title": "Bootstrap execution-only node",
                    "state": "ready",
                    "reason": "Execution-only node is ready to bootstrap.",
                    "depends_on": [],
                    "lock_keys": [],
                    "hotspot_keys": [],
                    "lineage": {
                        "node_id": "A",
                        "task_id": task_id,
                        "change_id": None,
                        "session_id": None,
                        "checkpoint_id": None,
                        "patchset_id": None,
                        "patchset_base_snapshot_id": None,
                        "patchset_revision_snapshot_id": None,
                        "land_id": None,
                        "landed_snapshot_id": None,
                        "task_run_id": None,
                    },
                    "task_id": task_id,
                    "change_id": None,
                    "session_id": None,
                    "task_run_id": None,
                    "checkpoint_id": None,
                    "patchset_id": None,
                    "patchset_base_snapshot_id": None,
                    "patchset_revision_snapshot_id": None,
                    "land_id": None,
                    "landed_snapshot_id": None,
                    "surface_bindings": [],
                    "blockers": [],
                }
            ],
            "summary": {"next_action": "start A"},
        }

    monkeypatch.setattr(read_models, "task_dag_readiness", fake_readiness)

    payload = task_graph_runs.advance_task_dag_run(
        ctx,
        run_session["session_id"],
        graph,
        actor_identity="alice@example.com",
        actor_type="human",
    )

    tasks = server_store.list_tasks(ctx, "ait")
    assert [task["plan_item_ref"] for task in tasks] == [None]
    assert server_store.list_changes(ctx, "ait") == []
    assert payload["auto_bootstrapped_node_ids"] == ["A"]
    assert readiness_calls["count"] == 2
    with connect(ctx) as conn:
        event_row = conn.execute(
            """
            select actor_identity, actor_type
            from events
            where entity_type = 'task' and entity_id = ? and event_type = 'task.created'
            order by event_id desc
            limit 1
            """,
            (tasks[0]["task_id"],),
        ).fetchone()
    assert event_row is not None
    assert event_row["actor_identity"] == "alice@example.com"
    assert event_row["actor_type"] == "human"


def test_advance_task_dag_run_auto_bootstraps_reviewable_task_node_with_change(tmp_path, monkeypatch):
    data_dir = tmp_path / "server-data"
    ctx = fake_postgres_context(data_dir)
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "ait", "main")
    graph = {
        "graph_id": "graph-reviewable-bootstrap",
        "source_plan": {"plan_id": "", "plan_revision_id": ""},
        "execution_policy": {
            "solo_gate_strategy": task_graph_runs.TASK_DAG_SOLO_GATE_STRATEGY,
            "final_gate_bundle": ["review", "attestation", "policy", "land"],
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Bootstrap reviewable node",
            }
        ],
        "edges": [],
    }
    run_session = server_store.create_session(
        ctx,
        "ait",
        "task_graph_run",
        title="Task DAG execute",
        metadata={
            "graph_id": "graph-reviewable-bootstrap",
            "graph_run_id": "graph-run-reviewable",
        },
    )
    readiness_calls = {"count": 0}

    def fake_readiness(_ctx, _graph, current_plan_revision_id=None):
        assert current_plan_revision_id is None
        readiness_calls["count"] += 1
        tasks = server_store.list_tasks(ctx, "ait")
        changes = server_store.list_changes(ctx, "ait")
        task_id = tasks[0]["task_id"] if tasks else None
        change_id = changes[0]["change_id"] if changes else None
        state = "ready" if not change_id else "running"
        workflow_state = "ready" if not change_id else "dispatched"
        return {
            "nodes": [
                {
                    "node_id": "A",
                    "node_kind": "task",
                    "title": "Bootstrap reviewable node",
                    "state": state,
                    "workflow_state": workflow_state,
                    "reason": "Reviewable node is ready to bootstrap.",
                    "depends_on": [],
                    "lock_keys": [],
                    "hotspot_keys": [],
                    "lineage": {
                        "node_id": "A",
                        "task_id": task_id,
                        "change_id": change_id,
                        "session_id": None,
                        "checkpoint_id": None,
                        "patchset_id": None,
                        "patchset_base_snapshot_id": None,
                        "patchset_revision_snapshot_id": None,
                        "land_id": None,
                        "landed_snapshot_id": None,
                        "task_run_id": None,
                    },
                    "task_id": task_id,
                    "change_id": change_id,
                    "session_id": None,
                    "task_run_id": None,
                    "checkpoint_id": None,
                    "patchset_id": None,
                    "patchset_base_snapshot_id": None,
                    "patchset_revision_snapshot_id": None,
                    "land_id": None,
                    "landed_snapshot_id": None,
                    "surface_bindings": [],
                    "blockers": [],
                }
            ],
            "summary": {"next_action": "start A" if not change_id else "continue A"},
        }

    monkeypatch.setattr(read_models, "task_dag_readiness", fake_readiness)

    payload = task_graph_runs.advance_task_dag_run(
        ctx,
        run_session["session_id"],
        graph,
        actor_identity="alice@example.com",
        actor_type="human",
    )

    tasks = server_store.list_tasks(ctx, "ait")
    changes = server_store.list_changes(ctx, "ait")
    assert len(tasks) == 1
    assert len(changes) == 1
    assert changes[0]["task_id"] == tasks[0]["task_id"]
    assert payload["auto_bootstrapped_node_ids"] == ["A"]
    assert readiness_calls["count"] == 2


def test_advance_task_dag_run_does_not_server_bootstrap_local_final_reviewable_node(tmp_path, monkeypatch):
    data_dir = tmp_path / "server-data-local-final"
    ctx = fake_postgres_context(data_dir)
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "ait", "main")
    graph = {
        "graph_id": "graph-local-final-reviewable",
        "source_plan": {"plan_id": "", "plan_revision_id": ""},
        "execution_policy": {
            "solo_gate_strategy": task_graph_runs.TASK_DAG_SOLO_GATE_STRATEGY,
            "change_strategy": "local_first_final_local_land",
            "final_gate_bundle": ["review", "attestation", "policy", "land"],
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Bootstrap local-final reviewable node",
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
            }
        ],
        "edges": [],
    }
    run_session = server_store.create_session(
        ctx,
        "ait",
        "task_graph_run",
        title="Task DAG execute",
        metadata={
            "graph_id": "graph-local-final-reviewable",
            "graph_run_id": "graph-run-local-final-reviewable",
        },
    )

    def fake_readiness(_ctx, _graph, current_plan_revision_id=None):
        return {
            "nodes": [
                {
                    "node_id": "A",
                    "node_kind": "task",
                    "title": "Bootstrap local-final reviewable node",
                    "state": "ready",
                    "workflow_state": "ready",
                    "reason": "Local-final converged node is ready.",
                    "depends_on": [],
                    "lock_keys": [],
                    "hotspot_keys": [],
                    "lineage": {
                        "node_id": "A",
                        "task_id": None,
                        "change_id": None,
                        "session_id": None,
                        "checkpoint_id": None,
                        "patchset_id": None,
                        "patchset_base_snapshot_id": None,
                        "patchset_revision_snapshot_id": None,
                        "land_id": None,
                        "landed_snapshot_id": None,
                        "task_run_id": None,
                    },
                    "task_id": None,
                    "change_id": None,
                    "session_id": None,
                    "task_run_id": None,
                    "checkpoint_id": None,
                    "patchset_id": None,
                    "patchset_base_snapshot_id": None,
                    "patchset_revision_snapshot_id": None,
                    "land_id": None,
                    "landed_snapshot_id": None,
                    "surface_bindings": [],
                    "blockers": [],
                }
            ],
            "summary": {"next_action": "start A"},
        }

    monkeypatch.setattr(read_models, "task_dag_readiness", fake_readiness)

    payload = task_graph_runs.advance_task_dag_run(
        ctx,
        run_session["session_id"],
        graph,
        actor_identity="alice@example.com",
        actor_type="human",
    )

    assert server_store.list_tasks(ctx, "ait") == []
    assert server_store.list_changes(ctx, "ait") == []
    assert payload["auto_bootstrapped_node_ids"] == []
    assert payload["workflow_summary"]["next_action"] == "start A"
    assert payload["latest_state_snapshot"]["ready_node_ids"] == ["A"]
    assert payload["latest_state_snapshot"]["completed_node_ids"] == []


def test_task_dag_execution_strategy_batches_ready_nodes_by_default():
    graph = _graph()
    graph["execution_policy"].pop("worker_execution_mode", None)
    ready_rows = [{"node_id": chr(ord("A") + index), "title": f"Node {index}"} for index in range(20)]

    strategy = build_task_graph_execution_strategy(graph, ready_rows)

    assert strategy["default_mode"] == "local_execution_dag_with_selective_promotion"
    assert strategy["dispatch_model"] == "compact_packet"
    assert strategy["worker_execution_mode"] == "worker_only_compact_packet"
    assert strategy["worker_session_mode"] == "single_fresh_worker_session"
    assert strategy["max_total_sessions"] == 1
    assert strategy["max_worker_sessions"] == 1
    assert strategy["physical_fanout_requires_explicit_opt_in"] is True
    assert strategy["recommended_worker_sessions"] == 1
    assert strategy["recommended_total_sessions"] == 1
    assert [batch["size"] for batch in strategy["batches"]] == [20]
    assert strategy["cost_policy"]["triggered_rules"] == []


def test_task_dag_execution_strategy_rewrites_legacy_transport_default_mode():
    graph = _graph()
    graph["execution_policy"]["default_mode"] = "compact_packet_worker"
    graph["execution_policy"].pop("dag_default", None)
    graph["execution_policy"].pop("change_strategy", None)

    strategy = build_task_graph_execution_strategy(graph, [{"node_id": "A", "title": "Node A"}])

    assert strategy["default_mode"] == "local_execution_dag_with_selective_promotion"
    assert strategy["worker_execution_mode"] == "worker_only_compact_packet"


def test_task_dag_execution_strategy_caps_ready_nodes_under_single_worker_topology():
    graph = _graph()
    graph["execution_policy"].update(
        {
            "batch_node_target": 2,
            "max_batch_sessions": 3,
            "min_batch_token_budget": 500,
            "small_node_token_threshold": 250,
        }
    )
    graph["nodes"] = [
        {"node_id": "A", "node_kind": "task", "dispatch_token_estimate": 100, "depends_on": []},
        {"node_id": "B", "node_kind": "task", "dispatch_token_estimate": 120, "depends_on": []},
        {"node_id": "C", "node_kind": "task", "dispatch_token_estimate": 700, "depends_on": []},
        {"node_id": "D", "node_kind": "task", "dispatch_token_estimate": 750, "depends_on": []},
    ]
    ready_rows = [{"node_id": node["node_id"], "title": node["node_id"]} for node in graph["nodes"]]

    strategy = build_task_graph_execution_strategy(graph, ready_rows)

    assert strategy["token_budget_policy"]["enabled"] is True
    assert strategy["token_budget_policy"]["estimated_ready_tokens"] == 1670
    assert strategy["token_budget_policy"]["coalesced_batches"] == 0
    assert strategy["recommended_worker_sessions"] == 1
    assert strategy["batches"][0]["fragmentation_vetoed"] is False
    assert strategy["batches"][0]["small_node_ids"] == ["A", "B"]
    assert strategy["batches"][0]["estimated_tokens"] == 1670
    assert strategy["batches"][0]["size"] == 4
    assert strategy["cost_policy"]["triggered_rules"] == []


def test_task_dag_execution_strategy_exposes_cost_policy_rules():
    graph = _graph()
    graph["execution_policy"].update(
        {
            "batch_node_target": 2,
            "max_batch_sessions": 4,
            "packet_token_budget": 600,
            "worker_batch_token_floor": 300,
            "worker_batch_token_ceiling": 500,
        }
    )
    graph["nodes"] = [
        {"node_id": "B", "node_kind": "task", "dispatch_token_estimate": 200, "depends_on": []},
        {"node_id": "C", "node_kind": "task", "dispatch_token_estimate": 200, "depends_on": []},
        {"node_id": "A", "node_kind": "task", "dispatch_token_estimate": 900, "depends_on": []},
    ]
    ready_rows = [{"node_id": node["node_id"], "title": node["node_id"]} for node in graph["nodes"]]

    strategy = build_task_graph_execution_strategy(graph, ready_rows)

    assert strategy["cost_policy"]["enabled"] is True
    assert strategy["cost_policy"]["ready_wave_only_recommended"] is True
    triggered_codes = {row["code"] for row in strategy["cost_policy"]["triggered_rules"]}
    assert "packet_ready_wave_only" in triggered_codes
    assert "worker_batch_ceiling_unmet" in triggered_codes
    assert strategy["recommended_worker_sessions"] == 1
    assert strategy["recommended_total_sessions"] == 1
    assert strategy["batches"][0]["estimated_tokens"] == 1300


def test_task_dag_execution_strategy_supports_explicit_worker_execution_mode_override():
    graph = _graph()
    graph["execution_policy"]["worker_execution_mode"] = "worker_only_compact_packet"
    ready_rows = [{"node_id": "A", "title": "Node A"}, {"node_id": "B", "title": "Node B"}]

    strategy = build_task_graph_execution_strategy(graph, ready_rows)

    assert strategy["worker_execution_mode"] == "worker_only_compact_packet"
    assert strategy["worker_session_mode"] == "single_fresh_worker_session"
    assert strategy["max_total_sessions"] == 1
    assert strategy["max_worker_sessions"] == 1
    assert strategy["recommended_worker_sessions"] == 1
    assert strategy["recommended_total_sessions"] == 1


def test_advance_task_dag_run_limits_takeover_to_available_worker_slots(monkeypatch):
    graph = {
        "graph_id": "graph-takeover-cap",
        "source_plan": {"plan_id": "PL-demo", "plan_revision_id": "PR-demo"},
        "execution_policy": {
            "solo_gate_strategy": task_graph_runs.TASK_DAG_SOLO_GATE_STRATEGY,
            "final_gate_bundle": ["review", "attestation", "policy", "land"],
            "max_worker_sessions": 1,
        },
        "nodes": [
            {"node_id": "A", "node_kind": "task", "title": "A", "workflow_boundary": "execution_only"},
            {"node_id": "B", "node_kind": "task", "title": "B", "workflow_boundary": "execution_only"},
            {"node_id": "C", "node_kind": "task", "title": "C", "workflow_boundary": "execution_only"},
        ],
        "edges": [],
    }
    readiness = {
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "A",
                "state": "completed",
                "reason": "done",
                "depends_on": [],
                "lineage": {"task_id": "T-A", "change_id": "C-A"},
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "B",
                "state": "ready",
                "reason": "task exists without linked change evidence yet",
                "depends_on": [],
                "lineage": {"task_id": "T-B"},
                "session_recommendation": {"action": "resume_or_claim"},
                "blockers": [],
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "C",
                "state": "ready",
                "reason": "task exists without linked change evidence yet",
                "depends_on": [],
                "lineage": {"task_id": "T-C"},
                "session_recommendation": {"action": "resume_or_claim"},
                "blockers": [],
            },
        ],
        "summary": {"next_action": "continue B"},
    }
    previous_snapshot = {
        "graph_run_id": "graph-run-1",
        "execution_state": "active",
        "plan_id": "PL-demo",
        "plan_revision_id": "PR-demo",
        "graph_id": "graph-takeover-cap",
        "graph_artifact_path": "docs/sprints/demo.task_graph.json",
        "workflow_summary": {
            "ready_nodes": 2,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "completed_nodes": 1,
            "dispatched_nodes": 2,
            "total_nodes": 3,
            "next_action": "continue B",
        },
        "readiness_summary": {"next_action": "continue B"},
        "ready_node_ids": ["B", "C"],
        "running_node_ids": [],
        "blocked_node_ids": [],
        "completed_node_ids": ["A"],
        "dispatched_node_ids": ["B", "C"],
        "next_action": "continue B",
        "readiness_digest": "digest-before",
    }
    bootstrapped: list[str] = []
    appended = []

    monkeypatch.setattr(read_models, "task_dag_readiness", lambda ctx, current_graph, current_plan_revision_id=None: readiness)
    monkeypatch.setattr(
        task_graph_runs,
        "get_session",
        lambda ctx, session_id: {
            "session_id": session_id,
            "repo_name": "ait",
            "session_kind": "task_graph_run",
            "status": "active",
            "title": "Task DAG execute",
            "metadata": {
                "plan_id": "PL-demo",
                "plan_revision_id": "PR-demo",
                "graph_id": "graph-takeover-cap",
                "graph_run_id": "graph-run-1",
                "max_worker_sessions": 1,
            },
        },
    )
    monkeypatch.setattr(
        task_graph_runs,
        "list_session_events",
        lambda ctx, session_id: [
            {"sequence": 1, "event_type": "task_graph.execution_started", "payload": {"workflow_summary": {"next_action": "continue B"}}},
            {"sequence": 2, "event_type": "task_graph.state_snapshot", "payload": previous_snapshot},
        ],
    )
    monkeypatch.setattr(
        task_graph_runs,
        "_task_dag_bootstrap_node",
        lambda *args, **kwargs: bootstrapped.append(str(kwargs.get("node_id") or ""))
        or {"node_id": kwargs.get("node_id"), "task_id": f"T-{kwargs.get('node_id')}"},
    )
    monkeypatch.setattr(
        task_graph_runs,
        "append_session_event",
        lambda ctx, session_id, event_type, payload, actor_identity=None, actor_type=None: (
            appended.append({"session_id": session_id, "event_type": event_type, "payload": payload})
            or {"event_id": f"EV-{len(appended)}", "event_type": event_type, "payload": payload}
        ),
    )

    payload = task_graph_runs.advance_task_dag_run(object(), "S-RUN", graph, current_plan_revision_id="PR-demo")

    assert bootstrapped == ["B"]
    assert payload["auto_bootstrapped_node_ids"] == ["B"]
    assert appended[0]["event_type"] == "task_graph.execution_advanced"
    assert appended[1]["event_type"] == "task_graph.state_snapshot"



def test_advance_task_dag_run_frees_takeover_slot_after_bootstrapped_node_completes(monkeypatch):
    graph = {
        "graph_id": "graph-takeover-resume",
        "source_plan": {"plan_id": "PL-demo", "plan_revision_id": "PR-demo"},
        "execution_policy": {
            "solo_gate_strategy": task_graph_runs.TASK_DAG_SOLO_GATE_STRATEGY,
            "final_gate_bundle": ["review", "attestation", "policy", "land"],
            "max_worker_sessions": 1,
        },
        "nodes": [
            {"node_id": "A", "node_kind": "task", "title": "A", "workflow_boundary": "execution_only"},
            {"node_id": "B", "node_kind": "task", "title": "B", "workflow_boundary": "execution_only"},
            {"node_id": "C", "node_kind": "task", "title": "C", "workflow_boundary": "execution_only"},
        ],
        "edges": [],
    }
    readiness = {
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "A",
                "state": "completed",
                "reason": "done",
                "depends_on": [],
                "lineage": {"task_id": "T-A", "change_id": "C-A"},
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "B",
                "state": "completed",
                "reason": "done",
                "depends_on": [],
                "lineage": {"task_id": "T-B"},
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "C",
                "state": "ready",
                "reason": "task exists without linked change evidence yet",
                "depends_on": [],
                "lineage": {"task_id": "T-C"},
                "session_recommendation": {"action": "resume_or_claim"},
                "blockers": [],
            },
        ],
        "summary": {"next_action": "continue C"},
    }
    previous_snapshot = {
        "graph_run_id": "graph-run-2",
        "execution_state": "active",
        "plan_id": "PL-demo",
        "plan_revision_id": "PR-demo",
        "graph_id": "graph-takeover-resume",
        "graph_artifact_path": "docs/sprints/demo.task_graph.json",
        "workflow_summary": {
            "ready_nodes": 2,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "completed_nodes": 1,
            "dispatched_nodes": 2,
            "total_nodes": 3,
            "next_action": "continue B",
        },
        "readiness_summary": {"next_action": "continue B"},
        "ready_node_ids": ["B", "C"],
        "running_node_ids": [],
        "blocked_node_ids": [],
        "completed_node_ids": ["A"],
        "dispatched_node_ids": ["B", "C"],
        "next_action": "continue B",
        "readiness_digest": "digest-before",
    }
    bootstrapped: list[str] = []

    monkeypatch.setattr(read_models, "task_dag_readiness", lambda ctx, current_graph, current_plan_revision_id=None: readiness)
    monkeypatch.setattr(
        task_graph_runs,
        "get_session",
        lambda ctx, session_id: {
            "session_id": session_id,
            "repo_name": "ait",
            "session_kind": "task_graph_run",
            "status": "active",
            "title": "Task DAG execute",
            "metadata": {
                "plan_id": "PL-demo",
                "plan_revision_id": "PR-demo",
                "graph_id": "graph-takeover-resume",
                "graph_run_id": "graph-run-2",
                "max_worker_sessions": 1,
            },
        },
    )
    monkeypatch.setattr(
        task_graph_runs,
        "list_session_events",
        lambda ctx, session_id: [
            {"sequence": 1, "event_type": "task_graph.execution_started", "payload": {"workflow_summary": {"next_action": "continue B"}}},
            {"sequence": 2, "event_type": "task_graph.node_bootstrapped", "payload": {"node_id": "B", "task_id": "T-B", "session_id": "S-B"}},
            {"sequence": 3, "event_type": "task_graph.state_snapshot", "payload": previous_snapshot},
        ],
    )
    monkeypatch.setattr(
        task_graph_runs,
        "_task_dag_bootstrap_node",
        lambda *args, **kwargs: bootstrapped.append(str(kwargs.get("node_id") or ""))
        or {"node_id": kwargs.get("node_id"), "task_id": f"T-{kwargs.get('node_id')}"},
    )
    monkeypatch.setattr(
        task_graph_runs,
        "append_session_event",
        lambda ctx, session_id, event_type, payload, actor_identity=None, actor_type=None: {
            "event_id": f"EV-{event_type}",
            "event_type": event_type,
            "payload": payload,
        },
    )

    payload = task_graph_runs.advance_task_dag_run(object(), "S-RUN", graph, current_plan_revision_id="PR-demo")

    assert bootstrapped == ["C"]
    assert payload["auto_bootstrapped_node_ids"] == ["C"]


def test_advance_task_dag_run_completes_satisfied_land_gate(monkeypatch):
    graph = {
        "graph_id": "graph-land-gate-complete",
        "source_plan": {"plan_id": "PL-demo", "plan_revision_id": "PR-demo"},
        "execution_policy": {
            "solo_gate_strategy": task_graph_runs.TASK_DAG_SOLO_GATE_STRATEGY,
            "final_gate_bundle": ["review", "attestation", "policy", "land"],
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "A",
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
            },
            {
                "node_id": "L",
                "node_kind": "land_gate",
                "title": "L",
                "depends_on": ["A"],
                "safety_boundary_reason": "final governance boundary",
                "completion_rule": "selected patchset is landed on target line",
            },
        ],
        "edges": [{"from": "A", "to": "L", "edge_kind": "depends_on"}],
    }
    readiness = {
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "A",
                "state": "completed",
                "reason": "done",
                "depends_on": [],
                "lineage": {
                    "task_id": "T-A",
                    "change_id": "C-A",
                    "land_id": "LAND-A",
                    "landed_snapshot_id": "SNP-A",
                },
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "L",
                "node_kind": "land_gate",
                "title": "L",
                "state": "completed",
                "reason": "Dependency land evidence satisfies the land gate.",
                "depends_on": ["A"],
                "lineage": {
                    "change_id": "C-A",
                    "land_id": "LAND-A",
                    "landed_snapshot_id": "SNP-A",
                },
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
        ],
        "summary": {"next_action": "complete task graph"},
    }
    previous_snapshot = {
        "graph_run_id": "graph-run-land-gate",
        "execution_state": "paused_for_safety_boundary",
        "plan_id": "PL-demo",
        "plan_revision_id": "PR-demo",
        "graph_id": "graph-land-gate-complete",
        "graph_artifact_path": "docs/sprints/demo_land_gate.task_graph.json",
        "workflow_summary": {
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "completed_nodes": 1,
            "dispatched_nodes": 0,
            "total_nodes": 2,
            "next_action": "start L",
        },
        "readiness_summary": {"next_action": "start L"},
        "ready_node_ids": ["L"],
        "running_node_ids": [],
        "blocked_node_ids": [],
        "completed_node_ids": ["A"],
        "dispatched_node_ids": [],
        "next_action": "inspect safety boundary L",
        "pause_reason": "safety_boundary",
        "readiness_digest": "digest-before",
    }
    appended: list[dict[str, object]] = []

    monkeypatch.setattr(read_models, "task_dag_readiness", lambda ctx, current_graph, current_plan_revision_id=None: readiness)
    monkeypatch.setattr(
        task_graph_runs,
        "get_session",
        lambda ctx, session_id: {
            "session_id": session_id,
            "repo_name": "ait",
            "session_kind": "task_graph_run",
            "status": "active",
            "title": "Task DAG execute",
            "metadata": {
                "plan_id": "PL-demo",
                "plan_revision_id": "PR-demo",
                "graph_id": "graph-land-gate-complete",
                "graph_run_id": "graph-run-land-gate",
            },
        },
    )
    monkeypatch.setattr(
        task_graph_runs,
        "list_session_events",
        lambda ctx, session_id: [
            {"sequence": 1, "event_type": "task_graph.execution_started", "payload": {"workflow_summary": {"next_action": "start L"}}},
            {"sequence": 2, "event_type": "task_graph.state_snapshot", "payload": previous_snapshot},
        ],
    )
    monkeypatch.setattr(
        task_graph_runs,
        "append_session_event",
        lambda ctx, session_id, event_type, payload, actor_identity=None, actor_type=None: (
            appended.append({"session_id": session_id, "event_type": event_type, "payload": payload})
            or {"event_id": f"EV-{len(appended)}", "event_type": event_type, "payload": payload}
        ),
    )

    payload = task_graph_runs.advance_task_dag_run(object(), "S-RUN", graph, current_plan_revision_id="PR-demo")

    assert payload["advanced"] is True
    assert payload["newly_completed_node_ids"] == ["L"]
    assert payload["execution_state"] == "completed"
    assert payload["latest_state_snapshot"]["completed_node_ids"] == ["A", "L"]
    assert payload["latest_state_snapshot"].get("gate_handoff") is None
    assert appended[0]["event_type"] == "task_graph.execution_advanced"
    assert appended[1]["event_type"] == "task_graph.state_snapshot"
