from __future__ import annotations

import base64
import importlib
import json
import sys
from pathlib import Path
from types import SimpleNamespace
from typing import Any
from uuid import uuid4

from typer.testing import CliRunner

import pytest

from tests._ram_root import detect_host_ram_root

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from ait_chat.reply_config import ReplyGenerationConfig
from ait.repo_paths import RepoContext
from ait_agent.telegram.runtime import TelegramSyncStateStore

cli_module = importlib.import_module("ait.cli.app")
plan_cli_module = importlib.import_module("ait.cli.commands.plan")
plan_task_linkage_module = importlib.import_module("ait.cli.plan_task_linkage")
task_dag_compact_packet_authoring_module = importlib.import_module("ait.cli.task_dag_compact_packet_authoring")
task_dag_runtime_helpers_module = importlib.import_module("ait.cli.task_dag_runtime_helpers")
task_dag_node_bootstrap_module = importlib.import_module("ait.cli.task_dag_node_bootstrap")
task_dag_compact_worker_runtime_module = importlib.import_module("ait.cli.task_dag_compact_worker_runtime")
task_dag_readiness_module = importlib.import_module("ait.task_dag_readiness")
watch_cli_module = importlib.import_module("ait.cli.task_dag_telegram_watch")
app = cli_module.app


runner = CliRunner()
pytestmark = pytest.mark.usefixtures("explicit_host_ram_root_cleanup")


def _allow_planless_tasks(monkeypatch):
    monkeypatch.setattr(plan_task_linkage_module, "_effective_plan_task_binding", lambda ctx: {"mode": "optional"})


def test_demo_suite_explicit_host_ram_cleanup_contract_creates_probe_root():
    root = detect_host_ram_root()
    if root is None:
        pytest.skip("No host memory-backed root is available on this machine.")

    probe_hash_dir = root / ".ait-repos" / f"pytest-demo-suite-probe-{uuid4().hex}"
    probe_repo_dir = probe_hash_dir / "demo-probe"
    probe_repo_dir.mkdir(parents=True)
    (probe_repo_dir / "README.md").write_text("probe\n", encoding="utf-8")

    assert probe_repo_dir.exists()


def _worker_execution_policy() -> dict:
    return {
        "worker_execution_mode": "worker_only_compact_packet",
    }


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
            "physical_fanout_default": False,
            "physical_fanout_requires_explicit_opt_in": True,
            "validate_source_plan_revision": True,
            "min_batch_token_budget": 300,
            "small_node_token_threshold": 250,
            **_worker_execution_policy(),
            "max_total_sessions": 1,
            "max_worker_sessions": 1,
            "max_batch_sessions": 1,
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Finished node",
                "plan_item_ref": "demo/a",
                "depends_on": [],
                "progress_weight": 1,
                "dispatch_token_estimate": 120,
                "task_template": {"title": "Do A", "risk_tier": "low"},
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Ready node",
                "plan_item_ref": "demo/b",
                "depends_on": ["A"],
                "progress_weight": 1,
                "task_template": {"title": "Do B", "risk_tier": "medium", "token_budget_hint": 220},
            },
        ],
        "edges": [{"from": "A", "to": "B", "edge_kind": "depends_on"}],
    }


def _graph_with_dispatch_artifacts() -> dict:
    graph = _graph()
    graph["dispatch_artifacts"] = {
        "source_markdown": "docs/sprints/demo.md",
        "parallel_execution_markdown": "docs/sprints/demo.md",
        "task_graph_json": "docs/sprints/demo.task_graph.json",
        "supporting_markdown": "docs/sprints/demo_supporting.md",
        "decoupling_plan_markdown": "docs/ait_directory_structure_decoupling_plan.md",
        "ownership_map_markdown": "docs/ait_module_ownership_map.md",
    }
    return graph


def _graph_with_blocked() -> dict:
    return {
        "schema_version": 1,
        "graph_id": "demo/task-dag-materialize",
        "repo_name": "demo",
        "source_plan": {
            "artifact_path": "docs/sprints/demo_materialize.md",
            "plan_id": "PL-demo-materialize",
            "plan_ref": "demo-materialize/root",
            "plan_revision_id": "PR-demo-materialize",
        },
        "execution_policy": {
            "mode": "guarded_full_dag_convergence",
            "validate_source_plan_revision": True,
            **_worker_execution_policy(),
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Finished node",
                "plan_item_ref": "demo/a",
                "depends_on": [],
                "progress_weight": 1,
                "task_template": {"title": "Do A", "risk_tier": "low"},
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Ready node",
                "plan_item_ref": "demo/b",
                "depends_on": ["A"],
                "progress_weight": 1,
                "task_template": {"title": "Do B", "risk_tier": "medium"},
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "Blocked node",
                "plan_item_ref": "demo/c",
                "depends_on": ["B"],
                "progress_weight": 1,
                "task_template": {"title": "Do C", "risk_tier": "high"},
            },
        ],
        "edges": [
            {"from": "A", "to": "B", "edge_kind": "depends_on"},
            {"from": "B", "to": "C", "edge_kind": "depends_on"},
        ],
    }


def _selective_promotion_graph() -> dict:
    return {
        "schema_version": 1,
        "graph_id": "demo/selective-promotion",
        "repo_name": "demo",
        "source_plan": {
            "artifact_path": "docs/sprints/demo_selective_promotion.md",
            "plan_id": "PL-demo-selective-promotion",
            "plan_ref": "demo-selective-promotion/root",
            "plan_revision_id": "PR-demo-selective-promotion",
        },
        "execution_policy": {
            "mode": "guarded_full_dag_convergence",
            "validate_source_plan_revision": True,
            **_worker_execution_policy(),
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Completed node",
                "plan_item_ref": "demo/a",
                "depends_on": [],
                "progress_weight": 1,
                "task_template": {"title": "Do A", "risk_tier": "low"},
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Execution-only node",
                "plan_item_ref": "demo/b",
                "depends_on": ["A"],
                "workflow_boundary": "execution_only",
                "progress_weight": 1,
                "task_template": {"title": "Do B", "risk_tier": "medium"},
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "Reviewable node",
                "plan_item_ref": "demo/c",
                "depends_on": ["A", "B"],
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
                "progress_weight": 1,
                "task_template": {"title": "Do C", "risk_tier": "high"},
            },
        ],
        "edges": [
            {"from": "A", "to": "B", "edge_kind": "depends_on"},
            {"from": "A", "to": "C", "edge_kind": "depends_on"},
            {"from": "B", "to": "C", "edge_kind": "depends_on"},
        ],
    }


def _solo_convergence_graph() -> dict:
    return {
        "schema_version": 1,
        "graph_id": "demo/solo-convergence",
        "repo_name": "demo",
        "source_plan": {
            "artifact_path": "docs/sprints/demo_solo_convergence.md",
            "plan_id": "PL-demo",
            "plan_ref": "demo/root",
            "plan_revision_id": "PR-demo",
        },
        "execution_policy": {
            "mode": "guarded_full_dag_convergence",
            "validate_source_plan_revision": True,
            "solo_gate_strategy": "end_of_dag_gate_concentration",
            "final_gate_bundle": ["review", "attestation", "policy", "land"],
            **_worker_execution_policy(),
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Completed execution-only node",
                "plan_item_ref": "demo/a",
                "depends_on": [],
                "workflow_boundary": "execution_only",
                "progress_weight": 1,
                "task_template": {"title": "Do A", "risk_tier": "low"},
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Execution-only node",
                "plan_item_ref": "demo/b",
                "depends_on": ["A"],
                "workflow_boundary": "execution_only",
                "progress_weight": 1,
                "task_template": {"title": "Do B", "risk_tier": "medium"},
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "Converged output node",
                "plan_item_ref": "demo/c",
                "depends_on": ["B"],
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
                "progress_weight": 1,
                "task_template": {"title": "Do C", "risk_tier": "high"},
            },
        ],
        "edges": [
            {"from": "A", "to": "B", "edge_kind": "depends_on"},
            {"from": "B", "to": "C", "edge_kind": "depends_on"},
        ],
    }


def _solo_safety_boundary_graph() -> dict:
    graph = _solo_convergence_graph()
    graph["graph_id"] = "demo/solo-safety-boundary"
    graph["nodes"][2]["node_kind"] = "gate_node"
    graph["nodes"][2]["workflow_boundary"] = "reviewable_output"
    graph["nodes"][2]["converged_output"] = False
    graph["nodes"][2]["safety_boundary"] = True
    graph["nodes"][2]["safety_boundary_reason"] = "checkpoint before irreversible promotion"
    graph["nodes"][2].pop("task_template", None)
    return graph


def _write_graph(graph: dict | None = None, *, filename: str = "demo.task_graph.json") -> Path:
    path = Path("docs/sprints") / filename
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(graph or _graph()), encoding="utf-8")
    return path


def _write_demo_markdown(filename: str = "demo.md") -> Path:
    path = Path("docs/sprints") / filename
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "\n".join(
            [
                "# Demo plan",
                "",
                "Authority: demo authority",
                "Status: current draft",
                "Scope: compact plan execution test",
                "",
                "## Demo Root [plan-ref: demo/root]",
                "",
                "### Dispatch intent",
                "1. Resolve one stable bundle.",
                "2. Compile one compact IR.",
                "",
                "### Guardrails",
                "- Keep refs stable.",
                "",
                "### Parallel work items",
                "- [ ] Finished node [ref: demo/a]",
                "",
                "Acceptance:",
                "- Bundle has stable refs.",
                "",
                "- [ ] Ready node [ref: demo/b]",
                "",
                "Acceptance:",
                "- IR carries dependency edges.",
                "",
                "### Acceptance criteria",
                "- One compact graph seed exists.",
            ]
        ),
        encoding="utf-8",
    )
    return path


def _write_demo_dispatch_supporting_docs() -> None:
    Path("docs/ait_directory_structure_decoupling_plan.md").write_text(
        "# Demo decoupling root plan\n\n- broad root plan\n",
        encoding="utf-8",
    )
    Path("docs/ait_module_ownership_map.md").write_text(
        "# Demo ownership map\n\n- broad ownership map\n",
        encoding="utf-8",
    )


def _write_comparison_report(filename: str = "demo.report.json") -> Path:
    path = Path(".ait/generated/benchmarks") / filename
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(
            {
                "benchmark_id": "demo-benchmark",
                "workloads": [
                    {
                        "workload_id": "demo_workload",
                        "title": "Demo workload",
                        "baseline_mode": "git_linear",
                        "comparisons": [
                            {
                                "baseline_mode": "git_linear",
                                "baseline_median_total_tokens": 111,
                                "mode": "ait_linear",
                                "candidate_median_total_tokens": 99,
                                "saving_percent": 10.81,
                            },
                            {
                                "baseline_mode": "git_linear",
                                "baseline_median_total_tokens": 111,
                                "mode": "ait_dag",
                                "candidate_median_total_tokens": 44,
                                "saving_percent": 60.36,
                            },
                        ],
                        "runs": [
                            {
                                "mode": "git_linear",
                                "run_id": "demo-git",
                                "prompt_tokens": 100,
                                "completion_tokens": 11,
                                "total_tokens": 111,
                                "quality": "passed",
                                "quality_passed": True,
                                "usage_provenance": {"session_jsonl_paths": ["/tmp/demo-git.jsonl"]},
                            },
                            {
                                "mode": "ait_linear",
                                "run_id": "demo-linear",
                                "prompt_tokens": 90,
                                "completion_tokens": 9,
                                "total_tokens": 99,
                                "quality": "passed",
                                "quality_passed": True,
                                "usage_provenance": {"session_jsonl_paths": ["/tmp/demo-linear.jsonl"]},
                            },
                            {
                                "mode": "ait_dag",
                                "run_id": "demo-dag",
                                "prompt_tokens": 40,
                                "completion_tokens": 4,
                                "total_tokens": 44,
                                "quality": "passed",
                                "quality_passed": True,
                                "usage_provenance": {"session_jsonl_paths": ["/tmp/demo-dag.jsonl"]},
                            },
                        ],
                    }
                ],
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    return path


def _readiness(graph: dict, *, stale: bool = False) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": "PR-new" if stale else graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": stale,
        "summary": {
            "total_nodes": 2,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "completed_nodes": 1,
            "next_action": "start B",
        },
        "counts": {"ready": 1, "running": 0, "blocked": 0, "completed": 1, "total": 2},
        "nodes": [
            {
                "node_id": "A",
                "title": "Finished node",
                "plan_item_ref": "demo/a",
                "state": "completed",
                "reason": "done",
                "depends_on": [],
                "lineage": {
                    "task_id": "T-A",
                    "change_id": "C-A",
                    "patchset_base_snapshot_id": "SNP-base-A",
                    "patchset_revision_snapshot_id": "SNP-rev-A",
                    "landed_snapshot_id": "SNP-rev-A",
                },
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Ready node",
                "plan_item_ref": "demo/b",
                "state": "ready",
                "reason": "ready",
                "depends_on": ["A"],
                "hotspot_keys": ["contract:demo"],
                "lineage": {},
                "session_recommendation": {"action": "open_new_session"},
                "blockers": [],
            },
        ],
    }


def _readiness_with_bound_change(graph: dict, *, task_id: str, change_id: str) -> dict:
    payload = _readiness(graph)
    payload["nodes"][1]["lineage"] = {"task_id": task_id, "change_id": change_id}
    payload["nodes"][1]["session_recommendation"] = {"action": "resume_or_claim"}
    return payload


def _readiness_with_running_bound_change(graph: dict, *, task_id: str, change_id: str) -> dict:
    payload = _readiness_with_bound_change(graph, task_id=task_id, change_id=change_id)
    payload["summary"].update(
        {
            "ready_nodes": 0,
            "running_nodes": 1,
            "next_action": "continue B",
        }
    )
    payload["counts"].update({"ready": 0, "running": 1})
    payload["nodes"][1].update(
        {
            "state": "running",
            "reason": "linked workflow evidence is active",
            "lineage": {
                "task_id": task_id,
                "change_id": change_id,
                "patchset_id": "RP-B-1",
            },
            "session_recommendation": {"action": "resume_or_claim"},
        }
    )
    return payload


def _readiness_with_bound_task_only(graph: dict, *, task_id: str) -> dict:
    payload = _dispatched_readiness(graph)
    payload["nodes"][1]["lineage"] = {"task_id": task_id}
    payload["nodes"][1]["session_recommendation"] = {"action": "resume_or_claim"}
    return payload


def _dispatched_readiness(graph: dict) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": False,
        "summary": {
            "total_nodes": 2,
            "ready_nodes": 0,
            "running_nodes": 1,
            "blocked_nodes": 0,
            "completed_nodes": 1,
            "next_action": "start B",
        },
        "counts": {"ready": 0, "running": 1, "blocked": 0, "completed": 1, "total": 2},
        "nodes": [
            {
                "node_id": "A",
                "title": "Finished node",
                "plan_item_ref": "demo/a",
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
                "title": "Dispatched node",
                "plan_item_ref": "demo/b",
                "state": "running",
                "reason": "task exists without linked change evidence yet",
                "depends_on": ["A"],
                "hotspot_keys": ["contract:demo"],
                "lineage": {"task_id": "T-B"},
                "session_recommendation": {"action": "resume_or_claim"},
                "blockers": [],
            },
        ],
    }


def _completed_task_only_readiness(graph: dict) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": False,
        "summary": {
            "total_nodes": 2,
            "ready_nodes": 0,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "completed_nodes": 2,
            "next_action": "complete task graph",
        },
        "counts": {"ready": 0, "running": 0, "blocked": 0, "completed": 2, "total": 2},
        "nodes": [
            {
                "node_id": "A",
                "title": "Finished node",
                "plan_item_ref": "demo/a",
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
                "title": "Completed task-only node",
                "plan_item_ref": "demo/b",
                "state": "completed",
                "reason": "done",
                "depends_on": ["A"],
                "lineage": {"task_id": "T-B"},
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
        ],
    }


def _init_repo(monkeypatch):
    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")
    result = runner.invoke(app, ["init", "--name", "demo", "--json"], catch_exceptions=False)
    assert result.exit_code == 0


def _readiness_with_blocked(graph: dict) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": False,
        "summary": {
            "total_nodes": 3,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 1,
            "completed_nodes": 1,
            "next_action": "start B",
        },
        "counts": {"ready": 1, "running": 0, "blocked": 1, "completed": 1, "total": 3},
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Finished node",
                "plan_item_ref": "demo/a",
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
                "title": "Ready node",
                "plan_item_ref": "demo/b",
                "state": "ready",
                "reason": "ready",
                "depends_on": ["A"],
                "lineage": {},
                "session_recommendation": {"action": "open_new_session"},
                "blockers": [],
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "Blocked node",
                "plan_item_ref": "demo/c",
                "state": "blocked",
                "reason": "Dependency B is ready, not completed.",
                "depends_on": ["B"],
                "lineage": {},
                "session_recommendation": {"action": "unblock_before_session"},
                "blockers": [
                    {
                        "type": "dependency",
                        "code": "dependency_incomplete",
                        "node_id": "B",
                        "state": "ready",
                        "message": "Dependency B is ready, not completed.",
                    }
                ],
            },
        ],
    }


def _readiness_with_selective_promotion(graph: dict) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": False,
        "summary": {
            "total_nodes": 3,
            "ready_nodes": 2,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "completed_nodes": 1,
            "next_action": "start B",
        },
        "counts": {"ready": 2, "running": 0, "blocked": 0, "completed": 1, "total": 3},
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Completed node",
                "plan_item_ref": "demo/a",
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
                "title": "Execution-only node",
                "plan_item_ref": "demo/b",
                "state": "ready",
                "reason": "ready",
                "depends_on": ["A"],
                "lineage": {},
                "session_recommendation": {"action": "open_new_session"},
                "blockers": [],
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "Reviewable node",
                "plan_item_ref": "demo/c",
                "state": "ready",
                "reason": "ready",
                "depends_on": ["A", "B"],
                "lineage": {},
                "session_recommendation": {"action": "open_new_session"},
                "blockers": [],
            },
        ],
    }


def _readiness_after_dependency_completion(graph: dict) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": False,
        "summary": {
            "total_nodes": 3,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "completed_nodes": 2,
            "next_action": "start C",
        },
        "counts": {"ready": 1, "running": 0, "blocked": 0, "completed": 2, "total": 3},
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Finished node",
                "plan_item_ref": "demo/a",
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
                "title": "Ready node",
                "plan_item_ref": "demo/b",
                "state": "completed",
                "reason": "landed",
                "depends_on": ["A"],
                "lineage": {"task_id": "T-B", "change_id": "C-B"},
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "Blocked node",
                "plan_item_ref": "demo/c",
                "state": "ready",
                "reason": "Dependency B completed.",
                "depends_on": ["B"],
                "lineage": {"task_id": "T-C"},
                "session_recommendation": {"action": "resume_or_claim"},
                "blockers": [],
            },
        ],
    }


def _solo_convergence_initial_readiness(graph: dict) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": False,
        "summary": {
            "total_nodes": 3,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 1,
            "completed_nodes": 1,
            "next_action": "start B",
        },
        "counts": {"ready": 1, "running": 0, "blocked": 1, "completed": 1, "total": 3},
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Completed execution-only node",
                "plan_item_ref": "demo/a",
                "state": "completed",
                "reason": "done",
                "depends_on": [],
                "lineage": {
                    "task_id": "T-A",
                    "landed_snapshot_id": "SNP-A",
                    "completion_snapshot_id": "SNP-A",
                    "completion_fork_snapshot_id": "SNP-base-A",
                },
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Execution-only node",
                "plan_item_ref": "demo/b",
                "state": "ready",
                "reason": "ready",
                "depends_on": ["A"],
                "lineage": {},
                "session_recommendation": {"action": "open_new_session"},
                "blockers": [],
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "Converged output node",
                "plan_item_ref": "demo/c",
                "state": "blocked",
                "reason": "Dependency B is ready, not completed.",
                "depends_on": ["B"],
                "lineage": {},
                "session_recommendation": {"action": "unblock_before_session"},
                "blockers": [
                    {
                        "type": "dependency",
                        "code": "dependency_incomplete",
                        "node_id": "B",
                        "state": "ready",
                        "message": "Dependency B is ready, not completed.",
                    }
                ],
            },
        ],
    }


def _solo_convergence_ready_for_gate(graph: dict) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": False,
        "summary": {
            "total_nodes": 3,
            "ready_nodes": 0,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "completed_nodes": 3,
            "next_action": "run converged output gate bundle",
        },
        "counts": {"ready": 0, "running": 0, "blocked": 0, "completed": 3, "total": 3},
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Completed execution-only node",
                "plan_item_ref": "demo/a",
                "state": "completed",
                "reason": "done",
                "depends_on": [],
                "lineage": {
                    "task_id": "T-A",
                    "landed_snapshot_id": "SNP-A",
                    "completion_snapshot_id": "SNP-A",
                    "completion_fork_snapshot_id": "SNP-base-A",
                },
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Execution-only node",
                "plan_item_ref": "demo/b",
                "state": "completed",
                "reason": "done",
                "depends_on": ["A"],
                "lineage": {
                    "task_id": "T-B",
                    "completion_snapshot_id": "SNP-B",
                    "completion_fork_snapshot_id": "SNP-base-B",
                },
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "Converged output node",
                "plan_item_ref": "demo/c",
                "state": "completed",
                "reason": "done",
                "depends_on": ["B"],
                "lineage": {"task_id": "T-C"},
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
        ],
    }


def _solo_convergence_landed(graph: dict) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": False,
        "summary": {
            "total_nodes": 3,
            "ready_nodes": 0,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "completed_nodes": 3,
            "next_action": "complete task graph",
        },
        "counts": {"ready": 0, "running": 0, "blocked": 0, "completed": 3, "total": 3},
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Completed execution-only node",
                "plan_item_ref": "demo/a",
                "state": "completed",
                "reason": "done",
                "depends_on": [],
                "lineage": {
                    "task_id": "T-A",
                    "landed_snapshot_id": "SNP-A",
                    "completion_snapshot_id": "SNP-A",
                    "completion_fork_snapshot_id": "SNP-base-A",
                },
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Execution-only node",
                "plan_item_ref": "demo/b",
                "state": "completed",
                "reason": "done",
                "depends_on": ["A"],
                "lineage": {
                    "task_id": "T-B",
                    "completion_snapshot_id": "SNP-B",
                    "completion_fork_snapshot_id": "SNP-base-B",
                },
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "C",
                "node_kind": "task",
                "title": "Converged output node",
                "plan_item_ref": "demo/c",
                "state": "completed",
                "reason": "landed",
                "depends_on": ["B"],
                "lineage": {"task_id": "T-C", "change_id": "C-C", "landed_snapshot_id": "SNP-C"},
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
        ],
    }


def _solo_safety_boundary_ready(graph: dict) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": False,
        "summary": {
            "total_nodes": 3,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 1,
            "completed_nodes": 1,
            "next_action": "start B",
        },
        "counts": {"ready": 1, "running": 0, "blocked": 1, "completed": 1, "total": 3},
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Completed execution-only node",
                "plan_item_ref": "demo/a",
                "state": "completed",
                "reason": "done",
                "depends_on": [],
                "lineage": {"task_id": "T-A", "landed_snapshot_id": "SNP-A"},
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Execution-only node",
                "plan_item_ref": "demo/b",
                "state": "ready",
                "reason": "ready",
                "depends_on": ["A"],
                "lineage": {},
                "session_recommendation": {"action": "open_new_session"},
                "blockers": [],
            },
            {
                "node_id": "C",
                "node_kind": "gate_node",
                "title": "Converged output node",
                "plan_item_ref": "demo/c",
                "state": "blocked",
                "reason": "Dependency B is ready, not completed.",
                "depends_on": ["B"],
                "lineage": {},
                "session_recommendation": {"action": "unblock_before_session"},
                "blockers": [
                    {
                        "type": "dependency",
                        "code": "dependency_incomplete",
                        "node_id": "B",
                        "state": "ready",
                        "message": "Dependency B is ready, not completed.",
                    }
                ],
            },
        ],
    }


def _solo_safety_boundary_reached(graph: dict) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": False,
        "summary": {
            "total_nodes": 3,
            "ready_nodes": 0,
            "running_nodes": 0,
            "blocked_nodes": 1,
            "completed_nodes": 2,
            "next_action": "inspect safety boundary C",
        },
        "counts": {"ready": 0, "running": 0, "blocked": 1, "completed": 2, "total": 3},
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Completed execution-only node",
                "plan_item_ref": "demo/a",
                "state": "completed",
                "reason": "done",
                "depends_on": [],
                "lineage": {"task_id": "T-A", "landed_snapshot_id": "SNP-A"},
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Execution-only node",
                "plan_item_ref": "demo/b",
                "state": "completed",
                "reason": "done",
                "depends_on": ["A"],
                "lineage": {"task_id": "T-B"},
                "session_recommendation": {"action": "none"},
                "blockers": [],
            },
            {
                "node_id": "C",
                "node_kind": "gate_node",
                "title": "Converged output node",
                "plan_item_ref": "demo/c",
                "state": "blocked",
                "reason": "checkpoint before irreversible promotion",
                "depends_on": ["B"],
                "lineage": {},
                "session_recommendation": {"action": "unblock_before_session"},
                "blockers": [
                    {
                        "type": "gate",
                        "code": "explicit_gate_not_satisfied",
                        "node_id": "C",
                        "message": "checkpoint before irreversible promotion",
                    }
                ],
            },
        ],
    }


def _stub_remote(monkeypatch, *, stale: bool = False, readiness_factory=None):
    remote_tuple = lambda ctx, remote_name: ({"url": "http://example.test"}, "demo")
    monkeypatch.setattr(cli_module, "_remote_tuple", remote_tuple)
    monkeypatch.setattr(plan_cli_module, "_remote_tuple", remote_tuple)
    monkeypatch.setattr(
        cli_module,
        "remote_read_task_dag_readiness",
        lambda base_url, graph: readiness_factory(graph) if readiness_factory is not None else _readiness(graph, stale=stale),
    )
    monkeypatch.setattr(
        cli_module,
        "remote_advance_task_dag_run",
        lambda base_url, session_id, graph, repo_name=None: (_ for _ in ()).throw(
            cli_module.RemoteError(f"POST {base_url}/v1/native/task-dag-runs/{session_id}:advance failed: 404 Not Found")
        ),
    )
    monkeypatch.setattr(
        cli_module,
        "remote_read_task_dag_progress",
        lambda base_url, graph: (_ for _ in ()).throw(
            cli_module.RemoteError(f"POST {base_url}/v1/native/read/task-dag-progress failed: 404 Not Found")
        ),
    )


def test_task_dag_readiness_carries_completion_snapshot_evidence_from_node_states():
    graph = _solo_convergence_graph()
    workflow = {
        "tasks": [
            {
                "task_id": "T-B",
                "plan_item_ref": "demo/b",
                "status": "completed",
            }
        ],
        "node_states": [
            {
                "node_state_id": "NS-1",
                "node_id": "B",
                "state": "completed",
                "task_id": "T-B",
                "completion_snapshot_id": "SNP-B",
                "completion_fork_snapshot_id": "SNP-base-B",
                "completion_line_name": "feature/t-b",
                "completion_worktree_name": "lt-b",
            }
        ],
    }

    readiness = task_dag_readiness_module.compute_task_graph_readiness(graph, workflow)
    row_by_id = {row["node_id"]: row for row in readiness["nodes"]}

    assert row_by_id["B"]["state"] == "completed"
    assert row_by_id["B"]["lineage"]["task_id"] == "T-B"
    assert row_by_id["B"]["lineage"]["completion_snapshot_id"] == "SNP-B"
    assert row_by_id["B"]["lineage"]["completion_fork_snapshot_id"] == "SNP-base-B"
    assert row_by_id["B"]["lineage"]["completion_line_name"] == "feature/t-b"
    assert row_by_id["B"]["lineage"]["completion_worktree_name"] == "lt-b"


def test_task_dag_local_readiness_payload_reads_graph_run_node_events(monkeypatch):
    graph = _solo_convergence_graph()
    sessions = [
        {
            "session_id": "S-GRAPH",
            "session_kind": "task_graph_run",
            "metadata": {
                "plan_id": graph["source_plan"]["plan_id"],
                "graph_id": graph["graph_id"],
                "graph_run_id": "RUN-1",
            },
        }
    ]
    events = {
        "S-GRAPH": [
            {
                "sequence": 1,
                "event_type": "task_graph.node_completed",
                "payload": {
                    "node_id": "A",
                    "task_id": "LT-A",
                    "summary": "A completed locally.",
                    "completion_snapshot_id": "SNP-A",
                    "completion_fork_snapshot_id": "SNP-base-A",
                    "completion_line_name": "feature/lt-a",
                    "completion_worktree_name": "lt-a",
                },
                "created_at": "2026-01-01T00:00:01Z",
            }
        ]
    }
    monkeypatch.setattr(
        cli_module,
        "get_local_plan",
        lambda ctx, plan_id: {"head_revision": {"plan_revision_id": graph["source_plan"]["plan_revision_id"]}},
    )
    monkeypatch.setattr(cli_module, "list_local_tasks", lambda ctx: [])
    monkeypatch.setattr(cli_module, "list_local_changes", lambda ctx: [])
    monkeypatch.setattr(cli_module, "list_local_sessions", lambda ctx: sessions)
    monkeypatch.setattr(cli_module, "list_local_session_events", lambda ctx, session_id: events[session_id])

    payload = task_dag_runtime_helpers_module._task_dag_local_readiness_payload(SimpleNamespace(), graph)
    rows = {row["node_id"]: row for row in payload["nodes"]}

    assert rows["A"]["state"] == "completed"
    assert rows["A"]["lineage"]["task_id"] == "LT-A"
    assert rows["A"]["lineage"]["completion_snapshot_id"] == "SNP-A"
    assert rows["B"]["state"] == "ready"


def test_task_dag_remote_inventory_readiness_reads_graph_run_node_events(monkeypatch):
    graph = _solo_convergence_graph()
    sessions = [
        {
            "session_id": "S-REMOTE-GRAPH",
            "session_kind": "task_graph_run",
            "metadata": {
                "plan_id": graph["source_plan"]["plan_id"],
                "graph_id": graph["graph_id"],
                "graph_run_id": "REMOTE-RUN-1",
            },
        }
    ]
    monkeypatch.setattr(
        cli_module,
        "remote_get_plan",
        lambda base_url, plan_id: {"head_revision": {"plan_revision_id": graph["source_plan"]["plan_revision_id"]}},
    )
    monkeypatch.setattr(cli_module, "remote_list_tasks", lambda base_url, repo_name: [])
    monkeypatch.setattr(cli_module, "remote_list_changes", lambda base_url, repo_name: [])
    monkeypatch.setattr(cli_module, "remote_list_sessions", lambda base_url, repo_name: sessions)
    monkeypatch.setattr(cli_module, "remote_list_session_checkpoints", lambda base_url, session_id, repo_name=None: [])
    monkeypatch.setattr(cli_module, "remote_list_patchsets", lambda base_url, change_id, repo_name=None: [])
    monkeypatch.setattr(cli_module, "remote_list_reviews", lambda base_url, change_id, repo_name=None: {})
    monkeypatch.setattr(cli_module, "remote_get_policy", lambda base_url, patchset_id, repo_name=None: {})
    monkeypatch.setattr(
        cli_module,
        "remote_list_session_events",
        lambda base_url, session_id, repo_name=None: [
            {
                "sequence": 1,
                "event_type": "task_graph.node_completed",
                "payload": {
                    "node_id": "A",
                    "task_id": "RT-A",
                    "summary": "A completed through remote graph-run evidence.",
                    "completion_snapshot_id": "SNP-A",
                    "completion_fork_snapshot_id": "SNP-base-A",
                },
                "created_at": "2026-01-01T00:00:01Z",
            }
        ],
    )

    payload = task_dag_runtime_helpers_module._task_dag_readiness_from_remote_inventory(
        {"url": "http://example.test"},
        "demo",
        graph,
    )
    rows = {row["node_id"]: row for row in payload["nodes"]}

    assert rows["A"]["state"] == "completed"
    assert rows["A"]["lineage"]["task_id"] == "RT-A"
    assert rows["B"]["state"] == "ready"


def test_plan_graph_schedule_and_progress_read_task_dag(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        graph_out = runner.invoke(
            app,
            ["plan", "graph", "PL-demo", "--from-json", str(graph_path), "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert graph_out.exit_code == 0
        graph_payload = json.loads(graph_out.output)
        assert graph_payload["nodes"][0]["task_id"] == "T-A"
        assert graph_payload["nodes"][0]["patchset_base_snapshot_id"] == "SNP-base-A"
        assert graph_payload["nodes"][0]["patchset_revision_snapshot_id"] == "SNP-rev-A"
        assert graph_payload["nodes"][0]["landed_snapshot_id"] == "SNP-rev-A"
        assert graph_payload["nodes"][1]["state"] == "ready"

        schedule_out = runner.invoke(
            app,
            ["plan", "schedule", "PL-demo", "--from-json", str(graph_path), "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        schedule_payload = json.loads(schedule_out.output)
        assert schedule_payload["ready"][0]["node_id"] == "B"
        assert "plan execute PL-demo" in schedule_payload["ready"][0]["command"]
        assert "--auto-compact-worker --yes" in schedule_payload["ready"][0]["command"]
        assert schedule_payload["ready"][0]["session_recommendation"]["action"] == "open_new_session"
        assert schedule_payload["execution_strategy"]["default_mode"] == "local_execution_dag_with_selective_promotion"
        assert schedule_payload["execution_strategy"]["dispatch_model"] == "compact_packet"
        assert schedule_payload["execution_strategy"]["worker_execution_mode"] == "worker_only_compact_packet"
        assert schedule_payload["execution_strategy"]["worker_session_mode"] == "single_fresh_worker_session"
        assert schedule_payload["execution_strategy"]["max_total_sessions"] == 1
        assert schedule_payload["execution_strategy"]["max_worker_sessions"] == 1
        assert schedule_payload["execution_strategy"]["physical_fanout_default"] is False
        assert schedule_payload["execution_strategy"]["recommended_worker_sessions"] == 1

        ready_out = runner.invoke(
            app,
            ["plan", "ready", "PL-demo", "--from-json", str(graph_path), "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        ready_payload = json.loads(ready_out.output)
        assert ready_payload["ready"][0]["node_id"] == "B"
        assert "running" not in ready_payload

        progress_out = runner.invoke(
            app,
            ["plan", "progress", "PL-demo", "--from-json", str(graph_path), "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        progress_payload = json.loads(progress_out.output)
        assert progress_payload["progress"]["completed_percent"] == 50
        assert progress_payload["progress"]["completed_nodes"] == 1
        assert progress_payload["progress"]["ready_nodes"] == 1


def test_plan_graph_auto_registers_telegram_graph_watch(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        import ait_agent.telegram.app as telegram_app_module

        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        env_path = Path(".ait/agent-runtime/telegram.env")
        env_path.parent.mkdir(parents=True, exist_ok=True)
        state_path = Path("telegram-sync.json")
        env_path.write_text(
            "\n".join(
                [
                    "BOT_TOKEN=test-token",
                    "AIT_REPO_NAME=demo",
                    f"AIT_TELEGRAM_STATE_PATH={state_path}",
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        state_store = TelegramSyncStateStore(state_path)
        state_store.upsert_chat(123, session_id="S-EDITOR-1", repo_name="demo", chat_type="private", chat_title="Wei")
        monkeypatch.setenv("AIT_SESSION_ID", "S-EDITOR-1")
        sent_messages = []
        monkeypatch.setattr(
            telegram_app_module.TelegramApiClient,
            "send_message",
            lambda self, chat_id, text: sent_messages.append((chat_id, text)),
        )
        import ait_agent.telegram.worker_config as telegram_worker_config_module

        monkeypatch.setattr(
            telegram_worker_config_module,
            "load_config_for_telegram_worker",
            lambda repo_root, name=None: telegram_app_module.load_config(
                repo_root,
                env={
                    "AIT_TELEGRAM_ENV_PATH": str(env_path),
                    "BOT_TOKEN": "test-token",
                    "AIT_REPO_NAME": "demo",
                    "AIT_TELEGRAM_STATE_PATH": str(state_path),
                    "AIT_SERVER_URL": "http://example.test",
                },
            ),
        )
        monkeypatch.setattr(
            watch_cli_module,
            "remote_read_task_dag_progress",
            lambda base_url, graph: {
                "progress": {
                    "graph_id": graph["graph_id"],
                    "completed_percent": 50,
                    "completed_nodes": 1,
                    "ready_nodes": 1,
                    "running_nodes": 0,
                    "blocked_nodes": 0,
                    "next_action": "start B",
                    "node_states": {"A": {"state": "completed"}, "B": {"state": "ready"}},
                },
                "blockers": [],
            },
        )

        result = runner.invoke(
            app,
            ["plan", "graph", "PL-demo", "--from-json", str(graph_path), "--remote", "origin", "--json"],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        watch = payload["telegram_graph_watch"]
        assert watch["registered"] is True
        assert watch["created"] is True
        assert watch["notification_sent"] is True
        assert watch["chat_id"] == "123"
        assert watch["resolution_mode"] == "session_id"
        assert sent_messages and sent_messages[0][0] == "123"
        link = state_store.get_chat(123)
        assert "PL-DEMO" in link["graph_watches"]
        assert link["graph_watches"]["PL-DEMO"]["graph_path"] == "docs/sprints/demo.task_graph.json"


def test_plan_graph_auto_registers_telegram_graph_watch_for_solo_local_without_remote_plan_read(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        import ait_agent.telegram.app as telegram_app_module

        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        env_path = Path(".ait/agent-runtime/telegram.env")
        env_path.parent.mkdir(parents=True, exist_ok=True)
        state_path = Path("telegram-sync.json")
        env_path.write_text(
            "\n".join(
                [
                    "BOT_TOKEN=test-token",
                    "AIT_REPO_NAME=demo",
                    f"AIT_TELEGRAM_STATE_PATH={state_path}",
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        state_store = TelegramSyncStateStore(state_path)
        state_store.upsert_chat(123, session_id="S-EDITOR-1", repo_name="demo", chat_type="private", chat_title="Wei")
        monkeypatch.setenv("AIT_SESSION_ID", "S-EDITOR-1")
        sent_messages = []
        monkeypatch.setattr(
            telegram_app_module.TelegramApiClient,
            "send_message",
            lambda self, chat_id, text: sent_messages.append((chat_id, text)),
        )
        import ait_agent.telegram.worker_config as telegram_worker_config_module

        monkeypatch.setattr(
            telegram_worker_config_module,
            "load_config_for_telegram_worker",
            lambda repo_root, name=None: telegram_app_module.load_config(
                repo_root,
                env={
                    "AIT_TELEGRAM_ENV_PATH": str(env_path),
                    "BOT_TOKEN": "test-token",
                    "AIT_REPO_NAME": "demo",
                    "AIT_TELEGRAM_STATE_PATH": str(state_path),
                    "AIT_SERVER_URL": "http://example.test",
                },
            ),
        )
        monkeypatch.setattr(watch_cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_local"})
        monkeypatch.setattr(
            watch_cli_module,
            "remote_read_task_dag_progress",
            lambda *args, **kwargs: (_ for _ in ()).throw(
                AssertionError("solo_local auto-watch should not require remote DAG progress reads")
            ),
        )

        result = runner.invoke(
            app,
            ["plan", "graph", "PL-demo", "--from-json", str(graph_path), "--remote", "origin", "--json"],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        watch = payload["telegram_graph_watch"]
        assert watch["registered"] is True
        assert watch["created"] is True
        assert watch["notification_sent"] is True
        assert watch["progress_reader_mode"] == "local"
        link = state_store.get_chat(123)
        assert "PL-DEMO" in link["graph_watches"]
        assert sent_messages and sent_messages[0][0] == "123"


def test_trigger_local_task_dag_telegram_notifications_uses_local_landed_progress(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        import ait_agent.telegram.graph_watches as telegram_graph_watches
        import ait_agent.telegram.worker_config as telegram_worker_config_module

        Path("app.py").write_text("print('base')\n", encoding="utf-8")
        _init_repo(monkeypatch)
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False).exit_code == 0
        markdown_path = _write_demo_markdown()
        sync_out = runner.invoke(app, ["plan", "sync", str(markdown_path), "--json"], catch_exceptions=False)
        assert sync_out.exit_code == 0, sync_out.stdout

        plan_rows = json.loads(runner.invoke(app, ["plan", "list", "--json"], catch_exceptions=False).stdout)
        plan_row = next(row for row in plan_rows if row["head_artifact_path"] == str(markdown_path))
        plan_id = plan_row["plan_id"]
        plan_revision_id = plan_row["head_revision_id"]

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Finish demo node",
                "--intent",
                "complete a local DAG node and then trigger telegram graph notifications",
                "--change-title",
                "Finish demo node locally",
                "--base-line",
                "main",
                "--plan",
                plan_id,
                "--plan-item-ref",
                "demo/a",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        started = json.loads(start_out.stdout)
        change_id = started["change"]["change_id"]
        bound_worktree_path = Path(started["worktree"]["path"])

        with monkeypatch.context() as worktree_context:
            worktree_context.chdir(bound_worktree_path)
            (bound_worktree_path / "app.py").write_text("print('landed')\n", encoding="utf-8")
            assert runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"], catch_exceptions=False).exit_code == 0
            land_out = runner.invoke(app, ["workflow", "land-local", change_id, "--json"], catch_exceptions=False)
        assert land_out.exit_code == 0, land_out.stdout

        monkeypatch.setattr(
            telegram_worker_config_module,
            "load_config_for_telegram_worker",
            lambda repo_root, name=None: SimpleNamespace(
                repo_root=repo_root,
                env_path=repo_root / ".ait" / "agent-runtime" / "telegram.env",
                sync_state_path=repo_root / "telegram-sync.json",
                token="123456:test-token",
            ),
        )

        captured: dict[str, Any] = {}
        graph = {
            "schema_version": 1,
            "graph_id": "demo/local-land-completion",
            "repo_name": "demo",
            "source_plan": {
                "artifact_path": str(markdown_path),
                "plan_id": plan_id,
                "plan_ref": "demo/root",
                "plan_revision_id": plan_revision_id,
            },
            "execution_policy": {
                "mode": "guarded_full_dag_convergence",
                "validate_source_plan_revision": False,
                **_worker_execution_policy(),
            },
            "nodes": [
                {
                    "node_id": "A",
                    "node_kind": "task",
                    "title": "Finished node",
                    "plan_item_ref": "demo/a",
                    "depends_on": [],
                    "progress_weight": 1,
                    "task_template": {"title": "Do A", "risk_tier": "low"},
                }
            ],
            "edges": [],
        }

        def fake_trigger(config, *, repo_name, progress_reader, **kwargs):
            captured["repo_name"] = repo_name
            captured["config_repo_root"] = str(config.repo_root)
            captured["payload"] = progress_reader(graph)
            return {"checked": 1, "sent": 1, "errors": 0}

        monkeypatch.setattr(telegram_graph_watches, "trigger_graph_watch_notifications", fake_trigger)

        ctx = cli_module.RepoContext.discover(Path.cwd())
        summary = watch_cli_module.trigger_local_task_dag_telegram_notifications(
            ctx,
            repo_name="demo",
            event_type="change.local_landed",
            entity_id=change_id,
        )

        assert summary["enabled"] is True
        assert summary["checked"] == 1
        assert summary["sent"] == 1
        assert summary["errors"] == 0
        assert summary["event_type"] == "change.local_landed"
        assert summary["entity_id"] == change_id
        assert captured["repo_name"] == "demo"
        assert captured["config_repo_root"] == str(Path.cwd())
        assert captured["payload"]["progress"]["completed_percent"] == 100
        assert captured["payload"]["progress"]["completed_nodes"] == 1
        assert captured["payload"]["progress"]["next_action"] == "complete task graph"
        assert captured["payload"]["blockers"] == []


def test_plan_dispatch_command_reports_removed_migration(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()

        result = runner.invoke(
            app,
            ["plan", "dispatch", "PL-demo", "--from-json", str(graph_path), "--create-tasks", "--yes"],
        )

        assert result.exit_code == 2
        assert "`ait plan dispatch` has been removed." in result.output
        assert "ait plan execute <plan-id> --from-json <task-graph-json> --auto-compact-worker --yes" in result.output


def test_plan_execute_is_advisory_by_default(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--json"],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        assert payload["mode"] == "advisory"
        assert payload["validated"] is True
        assert payload["ready"][0]["node_id"] == "B"
        assert payload["completed"][0]["node_id"] == "A"
        contract = payload["execute_run_contract"]
        assert contract["capability_stage"] == "guarded_full_dag_convergence"
        assert contract["auto_continue_supported"] is True
        assert contract["auto_node_bootstrap_supported"] is True
        assert contract["gate_strategy"] == "end_of_dag_gate_concentration"
        assert contract["final_gate_bundle"] == ["review", "attestation", "policy", "land"]
        assert contract["execution_only_node_ids"] == ["A"]
        assert contract["converged_output_node_ids"] == ["B"]
        assert "local task/change/snapshot/local-land lineage" in contract["current_boundary"]
        assert contract["worker_execution_mode"] == "worker_only_compact_packet"
        assert contract["worker_session_mode"] == "single_fresh_worker_session"
        assert contract["max_total_sessions"] == 1
        assert contract["max_worker_sessions"] == 1
        assert contract["change_progression_mode"] == "single_worker_per_change_patchset"
        assert contract["change_focus_policy"]["single_active_focus"] is True
        assert contract["next_focus_node_id"] == "B"
        assert contract["next_focus_change_id"] is None
        assert "ait plan execute PL-demo" in contract["starter_command"]
        assert "--auto-compact-worker --yes" in contract["starter_command"]
        packet_surface = payload["compact_packet_surface"]
        assert packet_surface["per_change_focus_required"] is True
        assert packet_surface["per_change_patchset_required"] is True
        assert packet_surface["change_focus_policy"]["next_focus"]["node_id"] == "B"


def test_plan_execute_preserves_running_change_focus(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(
            monkeypatch,
            readiness_factory=lambda graph: _readiness_with_running_bound_change(
                graph,
                task_id="LT-B",
                change_id="LC-B",
            ),
        )

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--json"],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        assert payload["ready"] == []
        assert payload["running"][0]["node_id"] == "B"
        assert payload["running"][0]["state"] == "running"
        assert payload["running"][0]["workflow_state"] == "running"
        assert payload["running"][0]["task_id"] == "LT-B"
        assert payload["running"][0]["change_id"] == "LC-B"
        assert payload["running"][0]["patchset_id"] == "RP-B-1"

        next_focus = payload["change_focus_policy"]["next_focus"]
        assert next_focus["node_id"] == "B"
        assert next_focus["task_id"] == "LT-B"
        assert next_focus["change_id"] == "LC-B"
        assert next_focus["workflow_state"] == "running"
        assert payload["execute_run_contract"]["next_focus_node_id"] == "B"
        assert payload["execute_run_contract"]["next_focus_change_id"] == "LC-B"
        packet_surface = payload["compact_packet_surface"]
        assert packet_surface["change_focus_policy"]["next_focus"]["node_id"] == "B"
        assert packet_surface["change_focus_policy"]["next_focus"]["change_id"] == "LC-B"


def test_plan_execute_enables_guarded_full_dag_contract_for_solo_graph(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        monkeypatch.setattr(cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"})
        graph_path = _write_graph(_solo_convergence_graph(), filename="demo_solo_convergence.task_graph.json")
        _stub_remote(monkeypatch, readiness_factory=_solo_convergence_initial_readiness)

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--json"],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        contract = payload["execute_run_contract"]
        assert contract["capability_stage"] == "guarded_full_dag_convergence"
        assert contract["workflow_mode"] == "solo_remote"
        assert contract["change_strategy"] == "local_first_final_remote_land"
        assert contract["final_remote_disposition_default"] is True
        assert contract["auto_continue_supported"] is True
        assert contract["auto_node_bootstrap_supported"] is True
        assert contract["gate_strategy"] == "end_of_dag_gate_concentration"
        assert contract["final_gate_bundle"] == ["review", "attestation", "policy", "land"]
        assert contract["execution_only_node_ids"] == ["A", "B"]
        assert contract["converged_output_node_ids"] == ["C"]
        assert "local task/change/snapshot/local-land lineage" in contract["current_boundary"]


def test_plan_execute_rejects_removed_auto_land_alias(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        monkeypatch.setattr(cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"})
        graph_path = _write_graph(_solo_convergence_graph(), filename="demo_solo_convergence.task_graph.json")
        _stub_remote(monkeypatch, readiness_factory=_solo_convergence_initial_readiness)

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--auto-land", "--json"],
            catch_exceptions=False,
        )

        assert result.exit_code != 0
        assert "No such option: --auto-land" in result.output


def test_plan_execute_rejects_removed_local_first_final_land_alias(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        monkeypatch.setattr(cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"})
        graph_path = _write_graph(_solo_convergence_graph(), filename="demo_solo_convergence.task_graph.json")
        _stub_remote(monkeypatch, readiness_factory=_solo_convergence_initial_readiness)

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--local-first-final-land", "--json"],
            catch_exceptions=False,
        )

        assert result.exit_code != 0
        assert "No such option: --local-first-final-land" in result.output


def test_plan_execute_selective_promotion_graph_defaults_to_final_remote_disposition_in_solo_remote(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        monkeypatch.setattr(cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"})
        graph_path = _write_graph(_selective_promotion_graph(), filename="demo_selective_promotion.task_graph.json")
        _stub_remote(monkeypatch, readiness_factory=_readiness_with_selective_promotion)

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo-selective-promotion", "--from-json", str(graph_path), "--json"],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        contract = payload["execute_run_contract"]
        assert contract["change_strategy"] == "local_first_final_remote_land"
        assert contract["final_remote_disposition_default"] is True
        assert "local task/change/snapshot/local-land lineage" in contract["current_boundary"]
        packet_surface = payload["compact_packet_surface"]
        assert packet_surface["final_remote_disposition_default"] is True
        promotion_policy = payload["promotion_policy"]
        assert promotion_policy["final_remote_disposition_default"] is True
        assert promotion_policy["local_lineage_allowed_for_execution_only"] is True
        assert "local task/change/snapshot/local-land lineage" in promotion_policy["current_boundary"]
        assert promotion_policy["change_strategy"] == "local_first_final_remote_land"


def test_plan_execute_supports_explicit_final_local_land_strategy(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        monkeypatch.setattr(cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"})
        graph = _solo_convergence_graph()
        graph["execution_policy"]["change_strategy"] = "local_first_final_local_land"
        graph_path = _write_graph(graph, filename="demo_solo_local_final.task_graph.json")
        _stub_remote(monkeypatch, readiness_factory=_solo_convergence_initial_readiness)

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--json"],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        contract = payload["execute_run_contract"]
        assert contract["change_strategy"] == "local_first_final_local_land"
        assert contract["final_land_disposition"] == "local"
        assert contract["final_remote_disposition_default"] is False
        assert contract["later_remote_promotion_allowed_after_local_land"] is True
        assert "later remote-promote through `ait workflow land --all-completed-local --remote <name>`" in contract["current_boundary"]
        assert payload["promotion_policy"]["final_land_disposition"] == "local"
        assert payload["promotion_policy"]["final_remote_disposition_default"] is False
        assert payload["promotion_policy"]["later_remote_promotion_allowed_after_local_land"] is True
        assert payload["compact_packet_surface"]["final_land_disposition"] == "local"
        assert payload["compact_packet_surface"]["final_remote_disposition_default"] is False


def test_local_final_dag_readiness_payload_prefers_local_inventory_and_completed_lineage(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph = {
            "schema_version": 1,
            "graph_id": "demo/local-final-rerun",
            "repo_name": "demo",
            "source_plan": {
                "artifact_path": "docs/sprints/demo_local_final_rerun.md",
                "plan_id": "PL-demo-local-final",
                "plan_ref": "demo-local-final/root",
                "plan_revision_id": "PR-demo-local-final",
            },
            "execution_policy": {
                "mode": "guarded_full_dag_convergence",
                "change_strategy": "local_first_final_local_land",
                "validate_source_plan_revision": True,
                **_worker_execution_policy(),
            },
            "nodes": [
                {
                    "node_id": "D",
                    "node_kind": "task",
                    "title": "Completed local node",
                    "plan_item_ref": "demo/d",
                    "depends_on": [],
                    "task_template": {"title": "Do D", "risk_tier": "medium"},
                },
                {
                    "node_id": "E",
                    "node_kind": "task",
                    "title": "Next ready node",
                    "plan_item_ref": "demo/e",
                    "depends_on": ["D"],
                    "task_template": {"title": "Do E", "risk_tier": "medium"},
                },
            ],
            "edges": [{"from": "D", "to": "E", "edge_kind": "depends_on"}],
        }
        ctx = RepoContext.discover(Path(".").resolve())

        def _unexpected_remote_tuple(*args, **kwargs):
            raise AssertionError("local-final readiness should not resolve remote inventory")

        monkeypatch.setattr(cli_module, "_remote_tuple", _unexpected_remote_tuple)
        monkeypatch.setattr(
            cli_module,
            "get_local_plan",
            lambda *_args, **_kwargs: {"head_revision": {"plan_revision_id": "PR-demo-local-final"}},
        )
        monkeypatch.setattr(
            cli_module,
            "list_local_tasks",
            lambda *_args, **_kwargs: [
                {
                    "task_id": "LT-1184",
                    "plan_id": "PL-demo-local-final",
                    "plan_item_ref": "demo/d",
                    "status": "completed",
                    "created_at": "2026-05-21T11:00:00Z",
                    "updated_at": "2026-05-21T11:10:00Z",
                },
                {
                    "task_id": "LT-1185",
                    "plan_id": "PL-demo-local-final",
                    "plan_item_ref": "demo/d",
                    "status": "canceled",
                    "created_at": "2026-05-21T11:11:00Z",
                    "updated_at": "2026-05-21T11:12:00Z",
                },
            ],
        )
        monkeypatch.setattr(cli_module, "list_local_changes", lambda *_args, **_kwargs: [])
        monkeypatch.setattr(cli_module, "list_local_sessions", lambda *_args, **_kwargs: [])

        readiness = cli_module._task_dag_readiness_payload(ctx, graph, "origin")
        nodes = {row["node_id"]: row for row in readiness["nodes"]}

        assert nodes["D"]["state"] == "completed"
        assert nodes["D"]["lineage"]["task_id"] == "LT-1184"
        assert nodes["E"]["state"] == "ready"
        assert readiness["summary"]["next_action"] == "start E"


def test_plan_execute_auto_compact_worker_creates_graph_run_session_and_event_before_worker_start(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)
        created_sessions = []
        appended_events = []

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            created_sessions.append(
                {
                    "base_url": base_url,
                    "repo_name": repo_name,
                    "session_kind": session_kind,
                    **kwargs,
                }
            )
            return {
                "session_id": "S-EXEC-1",
                "session_kind": session_kind,
                "title": kwargs.get("title"),
            }

        def fake_append_session_event(base_url, session_id, event_type, payload=None, repo_name=None):
            appended_events.append(
                {
                    "base_url": base_url,
                    "session_id": session_id,
                    "repo_name": repo_name,
                    "event_type": event_type,
                    "payload": payload or {},
                }
            )
            return {"event_id": "EV-1", "event_type": event_type}

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_session_event)
        monkeypatch.setattr(
            plan_cli_module,
            "_guard_task_dag_implementation_authoring_workspace",
            lambda *args, **kwargs: {"workspace_root": str(Path.cwd()), "worktree": {"name": "stub"}},
        )
        monkeypatch.setattr(plan_cli_module, "_task_dag_start_auto_compact_worker", lambda **_kwargs: {"ok": True})

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--auto-compact-worker", "--yes", "--json"],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        assert payload["mode"] == "record_run"
        assert payload["recorded_run"]["session_id"] == "S-EXEC-1"
        assert payload["recorded_run"]["session_kind"] == "task_graph_run"
        assert payload["recorded_run"]["graph_run_id"].startswith("graph-run-")
        assert created_sessions[0]["session_kind"] == "task_graph_run"
        assert created_sessions[0]["repo_name"] == "demo"
        metadata = created_sessions[0]["metadata"]
        assert metadata["session_policy"] == "task_dag_execute_run"
        assert metadata["graph_run_id"].startswith("graph-run-")
        assert metadata["execution_state"] == "active"
        assert metadata["plan_id"] == "PL-demo"
        assert metadata["plan_revision_id"] == "PR-demo"
        assert metadata["graph_id"] == "demo/task-dag"
        assert metadata["task_graph_json"] == "docs/sprints/demo.task_graph.json"
        assert metadata["ready_node_ids"] == ["B"]
        assert metadata["completed_node_ids"] == ["A"]
        assert metadata["final_land_disposition"] == "remote"
        assert metadata["worker_execution_mode"] == "worker_only_compact_packet"
        assert metadata["worker_session_mode"] == "single_fresh_worker_session"
        assert metadata["max_total_sessions"] == 1
        assert metadata["max_worker_sessions"] == 1
        assert metadata["change_progression_mode"] == "single_worker_per_change_patchset"
        assert metadata["change_focus_policy"]["single_active_focus"] is True
        assert metadata["next_focus_node_id"] == "B"
        assert metadata["compact_packet_surface"]["surface_id"] == "worker_only_compact_ait_dag_packet"
        assert appended_events[0]["event_type"] == "task_graph.execution_started"
        assert appended_events[0]["payload"]["graph_artifact_path"] == "docs/sprints/demo.task_graph.json"
        assert appended_events[0]["payload"]["execute_run_contract"]["auto_continue_supported"] is True
        assert appended_events[0]["payload"]["change_focus_policy"]["next_focus"]["node_id"] == "B"
        assert appended_events[0]["payload"]["next_focus_node_id"] == "B"
        assert appended_events[0]["repo_name"] == "demo"
        assert appended_events[1]["event_type"] == "task_graph.state_snapshot"
        assert appended_events[1]["payload"]["execution_state"] == "active"
        assert appended_events[1]["payload"]["change_focus_policy"]["next_focus"]["node_id"] == "B"


def test_plan_execute_auto_compact_worker_auto_registers_telegram_graph_watch_before_worker_start(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        import ait_agent.telegram.app as telegram_app_module

        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        env_path = Path(".ait/agent-runtime/telegram.env")
        env_path.parent.mkdir(parents=True, exist_ok=True)
        state_path = Path("telegram-sync.json")
        env_path.write_text(
            "\n".join(
                [
                    "BOT_TOKEN=test-token",
                    "AIT_REPO_NAME=demo",
                    f"AIT_TELEGRAM_STATE_PATH={state_path}",
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        state_store = TelegramSyncStateStore(state_path)
        state_store.upsert_chat(123, session_id="S-EDITOR-1", repo_name="demo", chat_type="private", chat_title="Wei")
        monkeypatch.setenv("AIT_SESSION_ID", "S-EDITOR-1")
        sent_messages = []
        monkeypatch.setattr(
            telegram_app_module.TelegramApiClient,
            "send_message",
            lambda self, chat_id, text: sent_messages.append((chat_id, text)),
        )
        import ait_agent.telegram.worker_config as telegram_worker_config_module

        monkeypatch.setattr(
            telegram_worker_config_module,
            "load_config_for_telegram_worker",
            lambda repo_root, name=None: telegram_app_module.load_config(
                repo_root,
                env={
                    "AIT_TELEGRAM_ENV_PATH": str(env_path),
                    "BOT_TOKEN": "test-token",
                    "AIT_REPO_NAME": "demo",
                    "AIT_TELEGRAM_STATE_PATH": str(state_path),
                    "AIT_SERVER_URL": "http://example.test",
                },
            ),
        )

        created_sessions = []
        appended_events = []

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            created_sessions.append(
                {
                    "base_url": base_url,
                    "repo_name": repo_name,
                    "session_kind": session_kind,
                    **kwargs,
                }
            )
            return {
                "session_id": "S-EXEC-1",
                "session_kind": session_kind,
                "title": kwargs.get("title"),
            }

        def fake_append_session_event(base_url, session_id, event_type, payload=None, repo_name=None):
            appended_events.append(
                {
                    "base_url": base_url,
                    "session_id": session_id,
                    "repo_name": repo_name,
                    "event_type": event_type,
                    "payload": payload or {},
                }
            )
            return {"event_id": "EV-1", "event_type": event_type}

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_session_event)
        monkeypatch.setattr(
            plan_cli_module,
            "_guard_task_dag_implementation_authoring_workspace",
            lambda *args, **kwargs: {"workspace_root": str(Path.cwd()), "worktree": {"name": "stub"}},
        )
        monkeypatch.setattr(plan_cli_module, "_task_dag_start_auto_compact_worker", lambda **_kwargs: {"ok": True})
        monkeypatch.setattr(
            watch_cli_module,
            "remote_read_task_dag_progress",
            lambda base_url, graph: {
                "progress": {
                    "graph_id": graph["graph_id"],
                    "completed_percent": 50,
                    "completed_nodes": 1,
                    "ready_nodes": 1,
                    "running_nodes": 0,
                    "blocked_nodes": 0,
                    "next_action": "start B",
                    "node_states": {"A": {"state": "completed"}, "B": {"state": "ready"}},
                },
                "blockers": [],
            },
        )

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--auto-compact-worker", "--yes", "--json"],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        watch = payload["telegram_graph_watch"]
        assert watch["registered"] is True
        assert watch["created"] is True
        assert watch["notification_sent"] is True
        assert watch["chat_id"] == "123"
        assert watch["resolution_mode"] == "session_id"
        assert sent_messages and sent_messages[0][0] == "123"
        link = state_store.get_chat(123)
        assert "PL-DEMO" in link["graph_watches"]
        assert link["graph_watches"]["PL-DEMO"]["graph_path"] == "docs/sprints/demo.task_graph.json"
        assert created_sessions[0]["session_kind"] == "task_graph_run"
        assert created_sessions[0]["repo_name"] == "demo"
        assert appended_events[0]["event_type"] == "task_graph.execution_started"
        assert appended_events[0]["repo_name"] == "demo"


def test_plan_execute_auto_compact_worker_records_surface_guidance_before_worker_start(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        created_sessions = []
        appended_events = []

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            created_sessions.append(
                {
                    "base_url": base_url,
                    "repo_name": repo_name,
                    "session_kind": session_kind,
                    **kwargs,
                }
            )
            return {
                "session_id": "S-COMPACT-1",
                "session_kind": session_kind,
                "title": kwargs.get("title"),
            }

        def fake_append_session_event(base_url, session_id, event_type, payload=None, repo_name=None):
            appended_events.append(
                {
                    "base_url": base_url,
                    "session_id": session_id,
                    "repo_name": repo_name,
                    "event_type": event_type,
                    "payload": payload or {},
                }
            )
            return {"event_id": "EV-COMPACT-1", "event_type": event_type}

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_session_event)
        monkeypatch.setattr(
            plan_cli_module,
            "_guard_task_dag_implementation_authoring_workspace",
            lambda *args, **kwargs: {"workspace_root": str(Path.cwd()), "worktree": {"name": "stub"}},
        )
        monkeypatch.setattr(plan_cli_module, "_task_dag_start_auto_compact_worker", lambda **_kwargs: {"ok": True})

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--auto-compact-worker",
                "--yes",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        assert payload["mode"] == "record_run"
        surface = payload["compact_packet_surface"]
        assert surface["surface_id"] == "worker_only_compact_ait_dag_packet"
        assert surface["fresh_worker_session"] is True
        assert surface["physical_fanout"] is False
        assert created_sessions[0]["metadata"]["compact_packet_surface"]["surface_id"] == surface["surface_id"]
        assert appended_events[0]["event_type"] == "task_graph.execution_started"
        assert appended_events[0]["payload"]["compact_packet_surface"]["surface_id"] == surface["surface_id"]


def test_plan_execute_auto_compact_worker_generates_packet_and_posts_turn(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _write_demo_markdown()
        _stub_remote(monkeypatch)

        created_sessions = []
        appended_events = []
        generated_turns = []
        loaded_reply_configs = []

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            session_id = f"S-AUTO-{len(created_sessions) + 1}"
            session = {
                "base_url": base_url,
                "repo_name": repo_name,
                "session_kind": session_kind,
                "session_id": session_id,
                **kwargs,
            }
            created_sessions.append(session)
            return {
                "session_id": session_id,
                "session_kind": session_kind,
                "title": kwargs.get("title"),
                "status": "active",
                "metadata": kwargs.get("metadata") or {},
            }

        session_sequences = {}

        def fake_append_session_event(base_url, session_id, event_type, payload=None, repo_name=None):
            sequence = session_sequences.get(session_id, 0) + 1
            session_sequences[session_id] = sequence
            appended_events.append(
                {
                    "base_url": base_url,
                    "session_id": session_id,
                    "repo_name": repo_name,
                    "event_type": event_type,
                    "sequence": sequence,
                    "payload": payload or {},
                }
            )
            return {"event_id": f"EV-{len(appended_events)}", "event_type": event_type, "sequence": sequence, "payload": payload or {}}

        def fake_list_session_events(base_url, session_id, repo_name=None):
            return [row for row in appended_events if row["session_id"] == session_id]

        def fake_get_session(base_url, session_id, repo_name=None):
            for row in created_sessions:
                if row["session_id"] == session_id:
                    return {
                        "session_id": session_id,
                        "session_kind": row["session_kind"],
                        "status": "active",
                        "title": row.get("title"),
                        "metadata": row.get("metadata") or {},
                    }
            raise KeyError(session_id)

        def fake_load_reply_generation_config(*, repo_name, repo_root, env):
            loaded_reply_configs.append({"repo_name": repo_name, "repo_root": str(repo_root)})
            return object()

        def fake_generate_session_reply(_config, *, session, events, chat_id, chat_title, checkpoint, surface, actor_identity=None):
            generated_turns.append(
                {
                    "session": session,
                    "events": events,
                    "chat_id": chat_id,
                    "chat_title": chat_title,
                    "checkpoint": checkpoint,
                    "surface": surface,
                    "actor_identity": actor_identity,
                }
            )
            return SimpleNamespace(
                text="compact worker completed",
                model="gpt-5.4",
                response_id="resp-demo",
                usage={"inputTokens": 120, "outputTokens": 24, "totalTokens": 144},
                source="codex",
                turn_analysis={"command_count": 0},
            )

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_session_event)
        monkeypatch.setattr(cli_module, "remote_list_session_events", fake_list_session_events)
        monkeypatch.setattr(cli_module, "remote_get_session", fake_get_session)
        monkeypatch.setattr(cli_module, "load_reply_generation_config", fake_load_reply_generation_config)
        monkeypatch.setattr(cli_module, "generate_session_reply", fake_generate_session_reply)
        monkeypatch.setattr(
            cli_module,
            "remote_create_session_turn",
            lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("auto-compact-worker should not use remote session turn")),
        )
        local_task_counter = {"value": 0}

        def fake_create_local_task(ctx, title, intent, risk_tier, *, plan_id=None, origin_plan_revision_id=None, plan_item_ref=None):
            local_task_counter["value"] += 1
            return {
                "task_id": f"LT-AUTO-{local_task_counter['value']}",
                "title": title,
                "intent": intent,
                "risk_tier": risk_tier,
                "plan_id": plan_id,
                "origin_plan_revision_id": origin_plan_revision_id,
                "plan_item_ref": plan_item_ref,
                "status": "active",
            }

        monkeypatch.setattr(task_dag_node_bootstrap_module, "create_local_task", fake_create_local_task)
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_task",
            lambda *args, **kwargs: {
                "task_id": "RT-AUTO-1",
                "title": kwargs.get("title") if kwargs else None,
                "status": "active",
            },
        )
        monkeypatch.setattr(
            cli_module,
            "_task_dag_materialize_node_lineage",
            lambda **kwargs: {
                "task_id": "RT-AUTO-1",
                "change_id": "RC-AUTO-1",
                "worktree": {
                    "name": "lt-auto",
                    "path": str(Path.cwd() / ".ait" / "workspace" / "lt-auto"),
                    "workspace_root": str(Path.cwd() / ".ait" / "workspace" / "lt-auto"),
                    "open_path": str(Path.cwd() / ".ait" / "workspace" / "lt-auto"),
                    "alias_path": str(Path.cwd() / ".ait" / "workspace" / "lt-auto"),
                },
            },
        )

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--auto-compact-worker",
                "--yes",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        assert payload["mode"] == "record_run"
        assert len(created_sessions) == 2
        assert created_sessions[0]["session_kind"] == "task_graph_run"
        assert created_sessions[1]["session_kind"] == "agent_run"
        assert created_sessions[1]["metadata"]["session_policy"] == "task_dag_compact_packet_worker"
        assert created_sessions[1]["metadata"]["physical_fanout"] is False
        auto_worker = payload["auto_compact_worker"]
        assert auto_worker["worker_session_id"] == created_sessions[1]["session_id"]
        assert auto_worker["packet_available"] is True
        assert Path(auto_worker["packet_artifact_path"]).exists()
        assert Path(auto_worker["surface_artifact_path"]).exists()
        assert Path(auto_worker["turn_artifact_path"]).exists()
        assert Path(auto_worker["packet_root_path"]).exists()
        assert Path(auto_worker["packet_root_manifest_path"]).exists()
        assert not Path(auto_worker["packet_root_path"], "planning_compiler_surface.json").exists()
        assert created_sessions[1]["metadata"]["execution_mode"] == "implementation"
        assert auto_worker["execution_mode"] == "implementation"
        assert created_sessions[1]["metadata"]["workspace_root"] == auto_worker["worker_workspace_root"]
        assert created_sessions[1]["metadata"]["repo_root"] == auto_worker["worker_repo_root"]
        assert not created_sessions[1]["metadata"]["workspace_root"].endswith("packet_root")
        assert (
            created_sessions[1]["metadata"]["packet_root_policy"]["max_command_count"]
            == cli_module.DEFAULT_TASK_DAG_IMPLEMENTATION_AUTO_LAND_PACKET_MAX_COMMAND_COUNT
        )
        assert created_sessions[1]["metadata"]["change_focus_policy"]["single_active_focus"] is True
        assert created_sessions[1]["metadata"]["next_focus_node_id"] == "B"
        assert len(generated_turns) == 1
        assert generated_turns[0]["session"]["session_id"] == created_sessions[1]["session_id"]
        assert generated_turns[0]["surface"] == "task_dag_compact_packet"
        packet_text = generated_turns[0]["events"][0]["payload"]["text"]
        assert "Start here:" in packet_text
        assert "Read `cat .ait/generated/task_dag_compact_packets/" in packet_text
        assert "Execution-only focus with file edits:" in packet_text
        assert "`ait workspace status --json`" in packet_text
        assert '`ait snapshot create --message "<focused slice>"`' in packet_text
        assert "Reviewable-output focus:" not in packet_text
        assert '`ait workflow land <change-id> --apply`' not in packet_text
        assert "Keep one reviewable focus change active at a time" not in packet_text
        assert "Do not use physical fan-out" not in packet_text
        assert "Execute this DAG as a worker-only compact `ait_dag` packet." not in packet_text
        assert "Reviewable focus queue:" not in packet_text
        assert "Suggested packet / graph entry files:" in packet_text
        assert auto_worker["packet_root_manifest_path"] in packet_text
        assert "compact_packet.md" not in packet_text
        assert "resolved authoring workspace" in packet_text
        assert "bound task worktree" in packet_text
        assert "Packet prompt:" not in packet_text
        assert "Use only this compact DAG execution packet." not in packet_text
        assert "80%+ packet savings" not in packet_text
        assert "Manifest fallback reply:" not in packet_text
        assert "Current focus context:" in packet_text
        assert "- node B · Ready node" in packet_text
        assert "- plan ref demo/b · boundary task" in packet_text
        assert "IR carries dependency edges." in packet_text
        assert "Packet artifact:" not in packet_text
        assert auto_worker["turn_delivery_status"] == "local_cli_reply_generation"
        assert auto_worker["assistant_reply_sequence"] == 2
        assert (
            auto_worker["reply_poll_timeout_seconds"]
            == cli_module.DEFAULT_TASK_DAG_IMPLEMENTATION_AUTO_LAND_REPLY_POLL_TIMEOUT_SECONDS
        )
        assert auto_worker["usage"]["totalTokens"] == 144
        assert auto_worker["boundary_report"]["status"] == "ok"
        assert auto_worker["change_focus_policy"]["next_focus"]["node_id"] == "B"
        assert auto_worker["next_focus_node_id"] == "B"
        manifest = json.loads(Path(auto_worker["packet_root_manifest_path"]).read_text(encoding="utf-8"))
        assert manifest["current_focus"]["node_id"] == "B"
        assert "compact_packet.md" not in manifest["secondary_context_files"]
        assert not Path(auto_worker["packet_root_path"], "compact_packet.md").exists()
        assert "IR carries dependency edges." in manifest["current_focus"]["acceptance"]
        assert "ignore_non_active_nodes" not in manifest["current_focus"]
        assert "change_focus_policy" not in manifest
        assert "worker_context_scope" not in manifest
        assert "turn_contains_packet_context" not in manifest
        assert "final_remote_disposition_default" not in manifest
        assert "packet_available" not in manifest
        assert "source_surface_artifact_path" not in manifest
        assert "source_packet_artifact_path" not in manifest
        assert "source_turn_artifact_path" not in manifest
        graph_run_rows = [row for row in appended_events if row["session_id"] == created_sessions[0]["session_id"]]
        worker_rows = [row for row in appended_events if row["session_id"] == created_sessions[1]["session_id"]]
        graph_run_events = [row["event_type"] for row in graph_run_rows]
        worker_events = [row["event_type"] for row in worker_rows]
        assert graph_run_events == [
            "task_graph.execution_started",
            "task_graph.state_snapshot",
            "task_graph.compact_packet_generated",
            "task_graph.node_local_progress",
            "task_graph.compact_worker_started",
            "task_graph.compact_worker_boundary_report",
        ]
        assert worker_events == ["session.message", "assistant.reply"]
        assert graph_run_rows[2]["payload"]["packet_artifact_path"] == auto_worker["packet_artifact_path"]
        assert graph_run_rows[2]["payload"]["packet_root_path"] == auto_worker["packet_root_path"]
        assert (
            graph_run_rows[2]["payload"]["reply_poll_timeout_seconds"]
            == cli_module.DEFAULT_TASK_DAG_IMPLEMENTATION_AUTO_LAND_REPLY_POLL_TIMEOUT_SECONDS
        )
        assert graph_run_rows[2]["payload"]["next_focus_node_id"] == "B"
        assert graph_run_rows[3]["payload"]["node_id"] == "B"
        assert graph_run_rows[3]["payload"]["status"] == "running"
        assert graph_run_rows[3]["payload"]["worker_session_id"] == created_sessions[1]["session_id"]
        assert graph_run_rows[4]["payload"]["worker_session_id"] == created_sessions[1]["session_id"]
        assert graph_run_rows[4]["payload"]["turn_delivery_status"] == "local_cli_reply_generation"
        assert graph_run_rows[4]["payload"]["next_focus_node_id"] == "B"
        assert (
            graph_run_rows[4]["payload"]["reply_poll_timeout_seconds"]
            == cli_module.DEFAULT_TASK_DAG_IMPLEMENTATION_AUTO_LAND_REPLY_POLL_TIMEOUT_SECONDS
        )
        assert graph_run_rows[5]["payload"]["status"] == "ok"
        assert graph_run_rows[5]["payload"]["worker_session_id"] == created_sessions[1]["session_id"]
        assert loaded_reply_configs[0]["repo_name"] == "demo"
        assert loaded_reply_configs[0]["repo_root"] == auto_worker["worker_workspace_root"]


def test_plan_execute_auto_compact_worker_records_local_progress_footer(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _write_demo_markdown()
        _stub_remote(monkeypatch)

        created_sessions = []
        appended_events = []

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            session_id = f"S-FOOTER-{len(created_sessions) + 1}"
            created_sessions.append(
                {
                    "base_url": base_url,
                    "repo_name": repo_name,
                    "session_kind": session_kind,
                    "session_id": session_id,
                    **kwargs,
                }
            )
            return {
                "session_id": session_id,
                "session_kind": session_kind,
                "title": kwargs.get("title"),
                "status": "active",
                "metadata": kwargs.get("metadata") or {},
            }

        session_sequences = {}

        def fake_append_session_event(base_url, session_id, event_type, payload=None, repo_name=None):
            sequence = session_sequences.get(session_id, 0) + 1
            session_sequences[session_id] = sequence
            event = {
                "event_id": f"EV-{session_id}-{sequence}",
                "event_type": event_type,
                "sequence": sequence,
                "payload": payload or {},
                "session_id": session_id,
                "repo_name": repo_name,
            }
            appended_events.append(event)
            return event

        def fake_get_session(base_url, session_id, repo_name=None):
            for row in created_sessions:
                if row["session_id"] == session_id:
                    return {
                        "session_id": session_id,
                        "session_kind": row["session_kind"],
                        "status": "active",
                        "title": row.get("title"),
                        "metadata": row.get("metadata") or {},
                    }
            raise KeyError(session_id)

        def fake_list_session_events(base_url, session_id, repo_name=None):
            return [row for row in appended_events if row["session_id"] == session_id]

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_session_event)
        monkeypatch.setattr(cli_module, "remote_get_session", fake_get_session)
        monkeypatch.setattr(cli_module, "remote_list_session_events", fake_list_session_events)
        monkeypatch.setattr(cli_module, "load_reply_generation_config", lambda **kwargs: object())
        monkeypatch.setattr(
            cli_module,
            "generate_session_reply",
            lambda *_args, **_kwargs: SimpleNamespace(
                text=(
                    "compact worker completed\n"
                    'task_dag_local_progress={"node_id":"B","status":"completed","summary":"lane done","tests":["pytest tests/test_task_dag_cli.py"]}'
                ),
                model="gpt-5.4",
                response_id="resp-footer",
                usage={"inputTokens": 80, "outputTokens": 30, "totalTokens": 110},
                source="codex",
                turn_analysis={"command_count": 1},
            ),
        )
        local_task_counter = {"value": 0}

        def fake_create_local_task(ctx, title, intent, risk_tier, *, plan_id=None, origin_plan_revision_id=None, plan_item_ref=None):
            local_task_counter["value"] += 1
            return {
                "task_id": f"LT-FOOTER-{local_task_counter['value']}",
                "title": title,
                "intent": intent,
                "risk_tier": risk_tier,
                "plan_id": plan_id,
                "origin_plan_revision_id": origin_plan_revision_id,
                "plan_item_ref": plan_item_ref,
                "status": "active",
            }

        monkeypatch.setattr(task_dag_node_bootstrap_module, "create_local_task", fake_create_local_task)
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_task",
            lambda *args, **kwargs: {
                "task_id": "RT-FOOTER-1",
                "title": kwargs.get("title") if kwargs else None,
                "status": "active",
            },
        )
        monkeypatch.setattr(
            cli_module,
            "_task_dag_materialize_node_lineage",
            lambda **kwargs: {
                "task_id": "RT-FOOTER-1",
                "change_id": "RC-FOOTER-1",
                "worktree": {
                    "name": "lt-footer",
                    "path": str(Path.cwd() / ".ait" / "workspace" / "lt-footer"),
                    "workspace_root": str(Path.cwd() / ".ait" / "workspace" / "lt-footer"),
                    "open_path": str(Path.cwd() / ".ait" / "workspace" / "lt-footer"),
                    "alias_path": str(Path.cwd() / ".ait" / "workspace" / "lt-footer"),
                },
            },
        )

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--auto-compact-worker",
                "--yes",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        auto_worker = payload["auto_compact_worker"]
        assert auto_worker["local_progress_payload"]["node_id"] == "B"
        assert auto_worker["local_progress_payload"]["status"] == "completed"
        graph_run_events = [
            row["event_type"]
            for row in appended_events
            if row["session_id"] == auto_worker["graph_run_session_id"]
        ]
        assert graph_run_events == [
            "task_graph.execution_started",
            "task_graph.state_snapshot",
            "task_graph.compact_packet_generated",
            "task_graph.node_local_progress",
            "task_graph.node_completed",
            "task_graph.compact_worker_started",
            "task_graph.compact_worker_boundary_report",
        ]


def test_plan_execute_auto_compact_worker_rejects_repo_root_when_matching_bound_worktree_exists(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        _allow_planless_tasks(monkeypatch)
        started = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Do B",
                "--intent",
                "Bootstrap a bound worktree for B",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        start_payload = json.loads(started.output)
        task_id = start_payload["task_id"]
        change_id = start_payload["change"]["change_id"]
        worktree_cd_command = start_payload["worktree_guidance"]["cd_command"]

        try:
            cli_module._guard_task_dag_implementation_authoring_workspace(
                cli_module._ctx(),
                remote_row={"url": "http://example.test"},
                compact_packet_surface={
                    "change_focus_policy": {
                        "next_focus": {
                            "node_id": "B",
                            "task_id": task_id,
                            "change_id": change_id,
                        }
                    }
                },
                command_name="plan execute --auto-compact-worker",
            )
            assert False, "expected bound-worktree guard to reject repo-root authoring"
        except ValueError as exc:
            text = str(exc)
            assert "requires the bound task worktree for compact DAG implementation authoring" in text
            assert worktree_cd_command in text


def test_guard_task_dag_implementation_authoring_workspace_accepts_matching_bound_worktree(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        _allow_planless_tasks(monkeypatch)
        started = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Do B",
                "--intent",
                "Bootstrap a bound worktree for B",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        start_payload = json.loads(started.output)
        task_id = start_payload["task_id"]
        change_id = start_payload["change"]["change_id"]
        worktree_root = Path(start_payload["worktree_guidance"]["target_workspace_root"])
        monkeypatch.chdir(worktree_root)

        workspace = cli_module._guard_task_dag_implementation_authoring_workspace(
            cli_module._ctx(),
            remote_row={"url": "http://example.test"},
            compact_packet_surface={
                "change_focus_policy": {
                    "next_focus": {
                        "node_id": "B",
                        "task_id": task_id,
                        "change_id": change_id,
                    }
                }
            },
            command_name="plan execute --auto-compact-worker",
        )

        assert workspace["worktree"]["bound_task_id"] == task_id
        assert workspace["workspace_root"] == str(worktree_root.resolve())


def test_plan_execute_auto_compact_worker_rejects_repo_root_for_task_only_bound_focus(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        _allow_planless_tasks(monkeypatch)
        started = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Do B",
                "--intent",
                "Bootstrap a bound worktree for B",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        start_payload = json.loads(started.output)
        task_id = start_payload["task_id"]
        worktree_cd_command = start_payload["worktree_guidance"]["cd_command"]
        graph_path = _write_graph()
        _stub_remote(monkeypatch, readiness_factory=lambda graph: _readiness_with_bound_task_only(graph, task_id=task_id))

        created_sessions = []
        monkeypatch.setattr(cli_module, "remote_create_session", lambda *args, **kwargs: created_sessions.append(kwargs))

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--auto-compact-worker", "--yes", "--json"],
        )

        assert result.exit_code != 0
        assert "Invalid value:" in result.output
        assert task_id in result.output
        assert Path(worktree_cd_command.split()[-1]).name in result.output
        assert created_sessions == []


def test_plan_execute_auto_compact_worker_uses_bound_worktree_as_authoring_workspace(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        _allow_planless_tasks(monkeypatch)
        started = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Do B",
                "--intent",
                "Bootstrap a bound worktree for B",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        start_payload = json.loads(started.output)
        task_id = start_payload["task_id"]
        change_id = start_payload["change"]["change_id"]
        worktree_root = Path(start_payload["worktree_guidance"]["target_workspace_root"]).resolve()
        monkeypatch.chdir(worktree_root)
        graph_path = _write_graph()
        _write_demo_markdown()
        _stub_remote(
            monkeypatch,
            readiness_factory=lambda graph: _readiness_with_bound_change(graph, task_id=task_id, change_id=change_id),
        )

        created_sessions = []
        appended_events = []
        generated_turns = []
        loaded_reply_configs = []

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            session_id = f"S-AUTO-WT-{len(created_sessions) + 1}"
            session = {
                "base_url": base_url,
                "repo_name": repo_name,
                "session_kind": session_kind,
                "session_id": session_id,
                **kwargs,
            }
            created_sessions.append(session)
            return {
                "session_id": session_id,
                "session_kind": session_kind,
                "title": kwargs.get("title"),
                "status": "active",
                "metadata": kwargs.get("metadata") or {},
            }

        session_sequences = {}

        def fake_append_session_event(base_url, session_id, event_type, payload=None, repo_name=None):
            sequence = session_sequences.get(session_id, 0) + 1
            session_sequences[session_id] = sequence
            appended_events.append(
                {
                    "base_url": base_url,
                    "session_id": session_id,
                    "repo_name": repo_name,
                    "event_type": event_type,
                    "sequence": sequence,
                    "payload": payload or {},
                }
            )
            return {"event_id": f"EV-WT-{len(appended_events)}", "event_type": event_type, "sequence": sequence, "payload": payload or {}}

        def fake_list_session_events(base_url, session_id, repo_name=None):
            return [row for row in appended_events if row["session_id"] == session_id]

        def fake_get_session(base_url, session_id, repo_name=None):
            for row in created_sessions:
                if row["session_id"] == session_id:
                    return {
                        "session_id": session_id,
                        "session_kind": row["session_kind"],
                        "status": "active",
                        "title": row.get("title"),
                        "metadata": row.get("metadata") or {},
                    }
            raise KeyError(session_id)

        def fake_load_reply_generation_config(*, repo_name, repo_root, env):
            loaded_reply_configs.append({"repo_name": repo_name, "repo_root": str(repo_root)})
            return object()

        def fake_generate_session_reply(_config, *, session, events, chat_id, chat_title, checkpoint, surface, actor_identity=None):
            generated_turns.append(
                {
                    "session": session,
                    "events": events,
                    "chat_id": chat_id,
                    "chat_title": chat_title,
                    "checkpoint": checkpoint,
                    "surface": surface,
                    "actor_identity": actor_identity,
                }
            )
            return SimpleNamespace(
                text="bound worktree worker completed",
                model="gpt-5.4",
                response_id="resp-bound-worktree",
                usage={"inputTokens": 130, "outputTokens": 26, "totalTokens": 156},
                source="codex",
                turn_analysis={"command_count": 0},
            )

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_session_event)
        monkeypatch.setattr(cli_module, "remote_list_session_events", fake_list_session_events)
        monkeypatch.setattr(cli_module, "remote_get_session", fake_get_session)
        monkeypatch.setattr(cli_module, "load_reply_generation_config", fake_load_reply_generation_config)
        monkeypatch.setattr(cli_module, "generate_session_reply", fake_generate_session_reply)
        monkeypatch.setattr(
            cli_module,
            "remote_create_session_turn",
            lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("auto-compact-worker should not use remote session turn")),
        )

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--auto-compact-worker",
                "--yes",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        auto_worker = payload["auto_compact_worker"]
        assert created_sessions[0]["metadata"]["next_focus_task_id"] == task_id
        assert created_sessions[1]["metadata"]["next_focus_task_id"] == task_id
        assert created_sessions[1]["metadata"]["workspace_root"] == str(worktree_root)
        assert created_sessions[1]["metadata"]["repo_root"] == str(worktree_root)
        assert auto_worker["worker_workspace_root"] == str(worktree_root)
        assert auto_worker["worker_repo_root"] == str(worktree_root)
        assert auto_worker["next_focus_task_id"] == task_id
        manifest = json.loads(Path(auto_worker["packet_root_manifest_path"]).read_text(encoding="utf-8"))
        assert manifest["authoring_workspace_root"] == str(worktree_root)
        assert loaded_reply_configs[0]["repo_root"] == str(worktree_root)
        assert len(generated_turns) == 1
        packet_text = generated_turns[0]["events"][0]["payload"]["text"]
        assert str(worktree_root) in packet_text
        graph_run_rows = [row for row in appended_events if row["session_id"] == created_sessions[0]["session_id"]]
        assert graph_run_rows[0]["payload"]["next_focus_task_id"] == task_id
        assert graph_run_rows[2]["payload"]["next_focus_task_id"] == task_id
        assert graph_run_rows[4]["payload"]["next_focus_task_id"] == task_id


def test_task_dag_generate_compact_packet_artifacts_bridges_authoring_workspace_inputs(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph(_graph_with_dispatch_artifacts())
        _write_demo_markdown()
        _write_demo_dispatch_supporting_docs()
        Path("docs/sprints/demo_supporting.md").write_text("supporting\n", encoding="utf-8")
        ctx = RepoContext.discover(Path.cwd())
        worktree_root = (Path.cwd() / ".ait" / "workspace" / "lt-bridge").resolve()
        worktree_root.mkdir(parents=True, exist_ok=True)

        for final_remote_disposition_default in (False, True):
            bundle = task_dag_compact_packet_authoring_module._task_dag_generate_compact_packet_artifacts(
                ctx,
                plan_id="PL-demo",
                graph=_graph_with_dispatch_artifacts(),
                graph_path=graph_path,
                compact_packet_surface=task_dag_compact_packet_authoring_module._task_dag_compact_packet_surface_payload(
                    ctx,
                    graph_path=graph_path,
                    final_remote_disposition_default=final_remote_disposition_default,
                ),
                final_remote_disposition_default=final_remote_disposition_default,
                authoring_workspace_root=str(worktree_root),
            )

            manifest = json.loads(Path(bundle["packet_root_manifest_path"]).read_text(encoding="utf-8"))
            allowed_hints = manifest["allowed_file_hints"]
            assert any("authoring_workspace_context/docs/sprints/demo.task_graph.json" in row for row in allowed_hints)
            assert any("authoring_workspace_context/docs/sprints/demo.md" in row for row in allowed_hints)
            assert any("authoring_workspace_context/docs/sprints/demo_supporting.md" in row for row in allowed_hints)
            assert not any("authoring_workspace_context/AGENTS.md" in row for row in allowed_hints)
            assert not any("authoring_workspace_context/ait-dag.md" in row for row in allowed_hints)
            assert not any("authoring_workspace_context/docs/ait_directory_structure_decoupling_plan.md" in row for row in allowed_hints)
            assert not any("authoring_workspace_context/docs/ait_module_ownership_map.md" in row for row in allowed_hints)
            assert (worktree_root / bundle["packet_root_manifest_path"]).exists()
            assert not (worktree_root / bundle["packet_artifact_path"]).exists()
            assert (worktree_root / bundle["turn_artifact_path"]).exists()
            assert Path(bundle["packet_artifact_path"]).exists()
            assert (
                worktree_root
                / next(row for row in allowed_hints if "authoring_workspace_context/docs/sprints/demo.task_graph.json" in row)
            ).exists()
            assert (
                worktree_root
                / next(row for row in allowed_hints if "authoring_workspace_context/docs/sprints/demo_supporting.md" in row)
            ).exists()
            assert (
                worktree_root
                / next(row for row in allowed_hints if "authoring_workspace_context/docs/sprints/demo.md" in row)
            ).exists()
            packet_markdown = Path(bundle["packet_artifact_path"]).read_text(encoding="utf-8")
            turn_text = (worktree_root / bundle["turn_artifact_path"]).read_text(encoding="utf-8")
            assert not (worktree_root / "authoring_workspace_context" / "ait-dag.md").exists()
            assert "Start here:" in turn_text
            assert "`ait workspace status --json`" in turn_text
            assert "`ait snapshot create --message \"<focused slice>\"`" in turn_text
            assert "`ait task complete --local <task-id>`" in turn_text
            assert "create or reuse the local change" in turn_text
            assert "Reviewable-output focus:" not in turn_text
            assert "`ait workflow land <change-id>`" not in turn_text
            assert "Source: docs/sprints/demo.md#demo/root" not in packet_markdown
            assert "authoring_workspace_context/docs/sprints/demo.md#demo/root" in packet_markdown
            assert bundle["worker_context_scope"] == "packet"
            assert "worker_context_scope" not in manifest
            assert "current_focus" not in manifest
            assert "compact_packet.md" not in manifest["secondary_context_files"]
            assert "current_focus_plan_excerpt.md" not in manifest["secondary_context_files"]
            assert not any("compact_packet.md" in row for row in allowed_hints)
            assert not (worktree_root / bundle["packet_root_path"] / "compact_packet.md").exists()
            assert "Reviewable-output focus:" not in turn_text
            assert "This run ends at remote land" not in turn_text
            assert "Required remote gate bundle for this run" not in turn_text


def test_task_dag_generate_compact_packet_artifacts_keeps_gate_prompts_on_reviewable_focus(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        _write_demo_markdown("demo_selective_promotion.md")
        _write_demo_dispatch_supporting_docs()
        graph = _selective_promotion_graph()
        graph_path = _write_graph(graph, filename="demo_selective_promotion.task_graph.json")
        ctx = RepoContext.discover(Path.cwd())
        worktree_root = (Path.cwd() / ".ait" / "workspace" / "lt-reviewable").resolve()
        worktree_root.mkdir(parents=True, exist_ok=True)

        compact_packet_surface = task_dag_compact_packet_authoring_module._task_dag_compact_packet_surface_payload(
            ctx,
            graph_path=graph_path,
            final_remote_disposition_default=True,
        )
        compact_packet_surface["change_focus_policy"]["next_focus"] = {
            "node_id": "C",
            "task_id": "RT-C",
            "change_id": "RC-C",
        }
        compact_packet_surface["next_focus_node_id"] = "C"
        compact_packet_surface["next_focus_task_id"] = "RT-C"
        compact_packet_surface["next_focus_change_id"] = "RC-C"

        bundle = task_dag_compact_packet_authoring_module._task_dag_generate_compact_packet_artifacts(
            ctx,
            plan_id="PL-demo-selective-promotion",
            graph=graph,
            graph_path=graph_path,
            compact_packet_surface=compact_packet_surface,
            final_remote_disposition_default=True,
            authoring_workspace_root=str(worktree_root),
        )

        manifest = json.loads(Path(bundle["packet_root_manifest_path"]).read_text(encoding="utf-8"))
        allowed_hints = manifest["allowed_file_hints"]
        turn_text = (worktree_root / bundle["turn_artifact_path"]).read_text(encoding="utf-8")

        assert "Reviewable-output focus:" in turn_text
        assert '`ait workflow land <change-id> --apply`' in turn_text
        assert "This run ends at remote land" in turn_text
        assert "Required remote gate bundle for this run" in turn_text
        assert any("/packet_root/current_focus_plan_excerpt.md" in row for row in allowed_hints)
        assert not any("authoring_workspace_context/docs/sprints/demo_selective_promotion.md" in row for row in allowed_hints)
        assert not any("authoring_workspace_context/ait-dag.md" in row for row in allowed_hints)
        assert not any("authoring_workspace_context/docs/ait_directory_structure_decoupling_plan.md" in row for row in allowed_hints)
        assert not any("authoring_workspace_context/docs/ait_module_ownership_map.md" in row for row in allowed_hints)
        assert not (worktree_root / "authoring_workspace_context" / "ait-dag.md").exists()
        assert "current_focus_plan_excerpt.md" in manifest["secondary_context_files"]


def test_task_dag_generate_compact_packet_artifacts_bridges_repo_root_plan_inputs_for_worktree_ctx(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph(_graph_with_dispatch_artifacts())
        _write_demo_markdown()
        _write_demo_dispatch_supporting_docs()
        ctx = RepoContext.discover(Path.cwd())
        runner_root = (Path.cwd() / ".ait" / "workspace" / "lt-runner").resolve()
        runner_root.mkdir(parents=True, exist_ok=True)
        worktree_config_path = runner_root / ".ait-worktree.json"
        worktree_config_path.write_text("{}", encoding="utf-8")
        authoring_root = (Path.cwd() / ".ait" / "workspace" / "lt-bridge").resolve()
        authoring_root.mkdir(parents=True, exist_ok=True)
        worktree_ctx = RepoContext(
            root=runner_root,
            ait_dir=ctx.ait_dir,
            content_db_path=ctx.content_db_path,
            control_db_path=ctx.control_db_path,
            config_path=ctx.config_path,
            worktree_config_path=worktree_config_path,
        )

        bundle = task_dag_compact_packet_authoring_module._task_dag_generate_compact_packet_artifacts(
            worktree_ctx,
            plan_id="PL-demo",
            graph=_graph_with_dispatch_artifacts(),
            graph_path=graph_path.resolve(),
            compact_packet_surface=task_dag_compact_packet_authoring_module._task_dag_compact_packet_surface_payload(
                worktree_ctx,
                graph_path=graph_path.resolve(),
                final_remote_disposition_default=False,
            ),
            final_remote_disposition_default=False,
            authoring_workspace_root=str(authoring_root),
        )

        manifest = json.loads((authoring_root / bundle["packet_root_manifest_path"]).read_text(encoding="utf-8"))
        allowed_hints = manifest["allowed_file_hints"]
        assert not any("/Users/" in row for row in allowed_hints)
        assert any("authoring_workspace_context/docs/sprints/demo.task_graph.json" in row for row in allowed_hints)
        assert any("authoring_workspace_context/docs/sprints/demo.md" in row for row in allowed_hints)
        assert not any("authoring_workspace_context/ait-dag.md" in row for row in allowed_hints)
        assert not any("authoring_workspace_context/docs/ait_directory_structure_decoupling_plan.md" in row for row in allowed_hints)
        assert not any("authoring_workspace_context/docs/ait_module_ownership_map.md" in row for row in allowed_hints)
        assert not any("compact_packet.md" in row for row in allowed_hints)
        assert "compact_packet.md" not in manifest["secondary_context_files"]
        assert "current_focus_plan_excerpt.md" not in manifest["secondary_context_files"]
        assert not (authoring_root / bundle["packet_artifact_path"]).exists()
        assert not (authoring_root / bundle["packet_root_path"] / "compact_packet.md").exists()
        assert not (authoring_root / "authoring_workspace_context" / "ait-dag.md").exists()

        packet_markdown = (runner_root / bundle["packet_artifact_path"]).read_text(encoding="utf-8")
        turn_text = (authoring_root / bundle["turn_artifact_path"]).read_text(encoding="utf-8")
        assert "Packet context:\nCompact DAG execution packet unavailable." not in packet_markdown
        assert "graph-derived only" not in packet_markdown
        assert "/Users/weita/AI_Play_Ground/ait/docs/sprints" not in packet_markdown
        assert "/Users/weita/AI_Play_Ground/ait/docs/sprints" not in turn_text
        assert "authoring_workspace_context/docs/sprints/demo.task_graph.json" in turn_text
        assert "authoring_workspace_context/docs/sprints/demo.md" in packet_markdown


def test_plan_execute_auto_compact_worker_seeds_execution_only_focus_and_ignores_unrelated_root_binding(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        _write_demo_markdown("demo_solo_convergence.md")
        graph_path = _write_graph(_solo_convergence_graph(), filename="demo_solo_convergence.task_graph.json")
        _stub_remote(monkeypatch, readiness_factory=_solo_convergence_initial_readiness)

        created_sessions = []
        appended_events = []
        generated_turns = []

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            session_id = f"S-SEED-{len(created_sessions) + 1}"
            session = {
                "base_url": base_url,
                "repo_name": repo_name,
                "session_kind": session_kind,
                "session_id": session_id,
                **kwargs,
            }
            created_sessions.append(session)
            return {
                "session_id": session_id,
                "session_kind": session_kind,
                "title": kwargs.get("title"),
                "status": "active",
                "metadata": kwargs.get("metadata") or {},
            }

        session_sequences = {}

        def fake_append_session_event(base_url, session_id, event_type, payload=None, repo_name=None):
            sequence = session_sequences.get(session_id, 0) + 1
            session_sequences[session_id] = sequence
            appended_events.append(
                {
                    "base_url": base_url,
                    "session_id": session_id,
                    "repo_name": repo_name,
                    "event_type": event_type,
                    "sequence": sequence,
                    "payload": payload or {},
                }
            )
            return {"event_id": f"EV-SEED-{len(appended_events)}", "event_type": event_type, "sequence": sequence, "payload": payload or {}}

        def fake_list_session_events(base_url, session_id, repo_name=None):
            return [row for row in appended_events if row["session_id"] == session_id]

        def fake_get_session(base_url, session_id, repo_name=None):
            for row in created_sessions:
                if row["session_id"] == session_id:
                    return {
                        "session_id": session_id,
                        "session_kind": row["session_kind"],
                        "status": "active",
                        "title": row.get("title"),
                        "metadata": row.get("metadata") or {},
                    }
            raise KeyError(session_id)

        def fake_load_reply_generation_config(*, repo_name, repo_root, env):
            return object()

        def fake_generate_session_reply(_config, *, session, events, chat_id, chat_title, checkpoint, surface, actor_identity=None):
            generated_turns.append(
                {
                    "session": session,
                    "events": events,
                    "chat_id": chat_id,
                    "chat_title": chat_title,
                    "checkpoint": checkpoint,
                    "surface": surface,
                    "actor_identity": actor_identity,
                }
            )
            return SimpleNamespace(
                text="seeded execution-only focus completed",
                model="gpt-5.4",
                response_id="resp-seeded-focus",
                usage={"inputTokens": 118, "outputTokens": 22, "totalTokens": 140},
                source="codex",
                turn_analysis={"command_count": 0},
            )

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_session_event)
        monkeypatch.setattr(cli_module, "remote_list_session_events", fake_list_session_events)
        monkeypatch.setattr(cli_module, "remote_get_session", fake_get_session)
        monkeypatch.setattr(cli_module, "load_reply_generation_config", fake_load_reply_generation_config)
        monkeypatch.setattr(cli_module, "generate_session_reply", fake_generate_session_reply)
        local_task_counter = {"value": 0}

        def fake_create_local_task(ctx, title, intent, risk_tier, *, plan_id=None, origin_plan_revision_id=None, plan_item_ref=None):
            local_task_counter["value"] += 1
            return {
                "task_id": f"LT-SEED-{local_task_counter['value']}",
                "title": title,
                "intent": intent,
                "risk_tier": risk_tier,
                "plan_id": plan_id,
                "origin_plan_revision_id": origin_plan_revision_id,
                "plan_item_ref": plan_item_ref,
                "status": "active",
            }

        monkeypatch.setattr(task_dag_node_bootstrap_module, "create_local_task", fake_create_local_task)
        foreign_worktree_root = Path.cwd() / ".ait" / "workspace" / "lt-foreign"
        foreign_worktree_root.mkdir(parents=True, exist_ok=True)
        foreign_worktree = {
            "name": "lt-foreign",
            "bound_task_id": "LT-FOREIGN",
            "bound_change_id": "LC-FOREIGN",
            "path": str(foreign_worktree_root),
            "workspace_root": str(foreign_worktree_root),
            "open_path": str(foreign_worktree_root),
            "alias_path": str(foreign_worktree_root),
        }
        monkeypatch.setattr(
            cli_module,
            "_session_bound_worktree",
            lambda *args, **kwargs: dict(foreign_worktree),
        )

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--auto-compact-worker",
                "--yes",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        auto_worker = payload["auto_compact_worker"]
        assert len(created_sessions) == 2
        assert created_sessions[0]["session_kind"] == "task_graph_run"
        assert created_sessions[1]["session_kind"] == "agent_run"
        assert created_sessions[0]["worktree_name"] != "lt-foreign"
        assert created_sessions[1]["worktree_name"] != "lt-foreign"
        assert created_sessions[0]["metadata"]["next_focus_node_id"] == "B"
        assert created_sessions[0]["metadata"]["next_focus_task_id"]
        assert created_sessions[1]["metadata"]["next_focus_task_id"] == created_sessions[0]["metadata"]["next_focus_task_id"]
        assert created_sessions[1]["worktree_name"]
        assert created_sessions[1]["metadata"]["workspace_root"] != str(Path.cwd().resolve())
        assert auto_worker["next_focus_node_id"] == "B"
        assert auto_worker["next_focus_task_id"] == created_sessions[0]["metadata"]["next_focus_task_id"]
        manifest = json.loads(Path(auto_worker["packet_root_manifest_path"]).read_text(encoding="utf-8"))
        assert manifest["authoring_workspace_root"] == created_sessions[1]["metadata"]["workspace_root"]
        graph_run_rows = [row for row in appended_events if row["session_id"] == created_sessions[0]["session_id"]]
        assert graph_run_rows[0]["payload"]["next_focus_task_id"] == created_sessions[0]["metadata"]["next_focus_task_id"]
        assert len(generated_turns) == 1


def test_task_dag_compact_worker_reply_poll_timeout_seconds_uses_execution_mode():
    assert (
        cli_module._task_dag_compact_worker_reply_poll_timeout_seconds({"execution_mode": "implementation"})
        == cli_module.DEFAULT_TASK_DAG_IMPLEMENTATION_PACKET_REPLY_POLL_TIMEOUT_SECONDS
    )
    assert (
        cli_module._task_dag_compact_worker_reply_poll_timeout_seconds(
            {"execution_mode": "implementation", "final_remote_disposition_default": True}
        )
        == cli_module.DEFAULT_TASK_DAG_IMPLEMENTATION_AUTO_LAND_REPLY_POLL_TIMEOUT_SECONDS
    )
    assert (
        cli_module._task_dag_compact_worker_reply_poll_timeout_seconds({"execution_mode": "benchmark"})
        == cli_module.DEFAULT_TASK_DAG_COMPACT_PACKET_REPLY_POLL_TIMEOUT_SECONDS
    )
    assert (
        cli_module._task_dag_compact_worker_reply_poll_timeout_seconds(
            {"execution_mode": "benchmark", "final_remote_disposition_default": True}
        )
        == cli_module.DEFAULT_TASK_DAG_COMPACT_PACKET_REPLY_POLL_TIMEOUT_SECONDS
    )
    assert (
        cli_module._task_dag_compact_worker_reply_poll_timeout_seconds({})
        == cli_module.DEFAULT_TASK_DAG_COMPACT_PACKET_REPLY_POLL_TIMEOUT_SECONDS
    )


def test_task_dag_seed_compact_worker_focus_lineage_bootstraps_node_only_converged_focus(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        from ait.cli.app_surfaces import _ctx

        _init_repo(monkeypatch)
        graph = _solo_convergence_graph()
        graph["execution_policy"]["change_strategy"] = "local_first_final_local_land"
        graph_path = _write_graph(graph, filename="demo_node_only_focus.task_graph.json")
        ctx = _ctx()
        remote_row = {"url": "http://example.test", "name": "origin"}
        recorded_run = {"metadata": {"next_focus_node_id": "C", "graph_id": graph["graph_id"]}}
        compact_packet_surface = {
            "change_focus_policy": {
                "next_focus": {
                    "focus_unit": "node",
                    "node_id": "C",
                }
            },
            "execution_only_node_ids": ["A", "B"],
            "converged_output_node_ids": ["C"],
            "safety_boundary_node_ids": [],
            "final_remote_disposition_default": False,
        }
        readiness = {
            "schema_version": 1,
            "graph_id": graph["graph_id"],
            "source_plan": graph["source_plan"],
            "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
            "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
            "stale_source_plan": False,
            "summary": {
                "total_nodes": 3,
                "ready_nodes": 1,
                "running_nodes": 0,
                "blocked_nodes": 0,
                "completed_nodes": 2,
                "next_action": "start C",
            },
            "nodes": [
                {
                    "node_id": "A",
                    "node_kind": "task",
                    "title": "Completed execution-only node",
                    "plan_item_ref": "demo/a",
                    "state": "completed",
                    "reason": "done",
                    "depends_on": [],
                    "lineage": {"task_id": "T-A"},
                    "session_recommendation": {"action": "none"},
                    "blockers": [],
                },
                {
                    "node_id": "B",
                    "node_kind": "task",
                    "title": "Completed execution-only node",
                    "plan_item_ref": "demo/b",
                    "state": "completed",
                    "reason": "done",
                    "depends_on": ["A"],
                    "lineage": {"task_id": "T-B"},
                    "session_recommendation": {"action": "none"},
                    "blockers": [],
                },
                {
                    "node_id": "C",
                    "node_kind": "task",
                    "title": "Converged output node",
                    "plan_item_ref": "demo/c",
                    "state": "ready",
                    "reason": "ready",
                    "depends_on": ["B"],
                    "lineage": {},
                    "session_recommendation": {"action": "open_new_session"},
                    "blockers": [],
                },
            ],
        }

        monkeypatch.setattr(
            task_dag_compact_worker_runtime_module,
            "_task_dag_readiness_payload",
            lambda current_ctx, current_graph, remote_name: readiness,
        )
        materialize_calls = []

        def fake_materialize_node_lineage(**kwargs):
            materialize_calls.append(kwargs)
            return {
                "task_id": "LT-F",
                "change_id": "LC-F",
                "worktree": {
                    "name": "lt-f",
                    "path": str(Path.cwd() / ".ait" / "workspace" / "lt-f"),
                    "workspace_root": str(Path.cwd() / ".ait" / "workspace" / "lt-f"),
                    "open_path": str(Path.cwd() / ".ait" / "workspace" / "lt-f"),
                    "alias_path": str(Path.cwd() / ".ait" / "workspace" / "lt-f"),
                },
            }

        monkeypatch.setattr(
            task_dag_compact_worker_runtime_module,
            "_task_dag_materialize_node_lineage",
            fake_materialize_node_lineage,
        )

        updated_surface, worktree = task_dag_compact_worker_runtime_module._task_dag_seed_compact_worker_focus_lineage(
            ctx=ctx,
            remote_row=remote_row,
            repo_name="demo",
            plan_id="PL-demo",
            graph=graph,
            graph_path=graph_path,
            recorded_run=recorded_run,
            compact_packet_surface=compact_packet_surface,
        )

        assert len(materialize_calls) == 1
        assert materialize_calls[0]["node_id"] == "C"
        assert updated_surface["next_focus_node_id"] == "C"
        assert updated_surface["next_focus_task_id"] == "LT-F"
        assert updated_surface["next_focus_change_id"] == "LC-F"
        next_focus = updated_surface["change_focus_policy"]["next_focus"]
        assert next_focus["node_id"] == "C"
        assert next_focus["task_id"] == "LT-F"
        assert next_focus["change_id"] == "LC-F"
        assert worktree["name"] == "lt-f"



def test_task_dag_seed_compact_worker_focus_lineage_rematerializes_lineaged_converged_focus_without_worktree(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        from ait.cli.app_surfaces import _ctx

        _init_repo(monkeypatch)
        graph = _solo_convergence_graph()
        graph["execution_policy"]["change_strategy"] = "local_first_final_local_land"
        graph_path = _write_graph(graph, filename="demo_lineaged_focus.task_graph.json")
        ctx = _ctx()
        remote_row = {"url": "http://example.test", "name": "origin"}
        recorded_run = {
            "metadata": {
                "next_focus_node_id": "C",
                "next_focus_task_id": "LT-F",
                "next_focus_change_id": "LC-F",
                "graph_id": graph["graph_id"],
            }
        }
        compact_packet_surface = {
            "change_focus_policy": {
                "next_focus": {
                    "focus_unit": "change",
                    "node_id": "C",
                    "task_id": "LT-F",
                    "change_id": "LC-F",
                }
            },
            "execution_only_node_ids": ["A", "B"],
            "converged_output_node_ids": ["C"],
            "safety_boundary_node_ids": [],
            "final_remote_disposition_default": False,
        }
        readiness = {
            "schema_version": 1,
            "graph_id": graph["graph_id"],
            "source_plan": graph["source_plan"],
            "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
            "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
            "stale_source_plan": False,
            "summary": {
                "total_nodes": 3,
                "ready_nodes": 0,
                "running_nodes": 1,
                "blocked_nodes": 0,
                "completed_nodes": 2,
                "next_action": "resume C",
            },
            "nodes": [
                {
                    "node_id": "A",
                    "node_kind": "task",
                    "title": "Completed execution-only node",
                    "plan_item_ref": "demo/a",
                    "state": "completed",
                    "reason": "done",
                    "depends_on": [],
                    "lineage": {"task_id": "T-A"},
                    "session_recommendation": {"action": "none"},
                    "blockers": [],
                },
                {
                    "node_id": "B",
                    "node_kind": "task",
                    "title": "Completed execution-only node",
                    "plan_item_ref": "demo/b",
                    "state": "completed",
                    "reason": "done",
                    "depends_on": ["A"],
                    "lineage": {"task_id": "T-B"},
                    "session_recommendation": {"action": "none"},
                    "blockers": [],
                },
                {
                    "node_id": "C",
                    "node_kind": "task",
                    "title": "Converged output node",
                    "plan_item_ref": "demo/c",
                    "state": "running",
                    "reason": "lineage exists but bound worktree still needs materialization",
                    "depends_on": ["B"],
                    "lineage": {"task_id": "LT-F", "change_id": "LC-F"},
                    "session_recommendation": {"action": "resume_or_claim"},
                    "blockers": [],
                },
            ],
        }

        monkeypatch.setattr(
            task_dag_compact_worker_runtime_module,
            "_task_dag_readiness_payload",
            lambda current_ctx, current_graph, remote_name: readiness,
        )
        monkeypatch.setattr(
            task_dag_compact_worker_runtime_module,
            "_task_dag_compact_worker_bound_worktree",
            lambda *args, **kwargs: None,
        )
        materialize_calls = []

        def fake_materialize_node_lineage(**kwargs):
            materialize_calls.append(kwargs)
            return {
                "task_id": "LT-F",
                "change_id": "LC-F",
                "worktree": {
                    "name": "lt-f",
                    "path": str(Path.cwd() / ".ait" / "workspace" / "lt-f"),
                    "workspace_root": str(Path.cwd() / ".ait" / "workspace" / "lt-f"),
                    "open_path": str(Path.cwd() / ".ait" / "workspace" / "lt-f"),
                    "alias_path": str(Path.cwd() / ".ait" / "workspace" / "lt-f"),
                },
            }

        monkeypatch.setattr(
            task_dag_compact_worker_runtime_module,
            "_task_dag_materialize_node_lineage",
            fake_materialize_node_lineage,
        )

        updated_surface, worktree = task_dag_compact_worker_runtime_module._task_dag_seed_compact_worker_focus_lineage(
            ctx=ctx,
            remote_row=remote_row,
            repo_name="demo",
            plan_id="PL-demo",
            graph=graph,
            graph_path=graph_path,
            recorded_run=recorded_run,
            compact_packet_surface=compact_packet_surface,
        )

        assert len(materialize_calls) == 1
        assert materialize_calls[0]["node_id"] == "C"
        next_focus = updated_surface["change_focus_policy"]["next_focus"]
        assert next_focus["task_id"] == "LT-F"
        assert next_focus["change_id"] == "LC-F"
        assert worktree["name"] == "lt-f"


def test_task_dag_compact_worker_bound_worktree_root_rejects_missing_workspace_root():
    with runner.isolated_filesystem():
        with pytest.raises(ValueError, match="requires the bound task worktree"):
            task_dag_compact_worker_runtime_module._task_dag_compact_worker_bound_worktree_root(
                {"name": "lt-missing", "bound_task_id": "LT-F"},
                compact_packet_surface={
                    "change_focus_policy": {
                        "next_focus": {
                            "node_id": "C",
                            "task_id": "LT-F",
                            "change_id": "LC-F",
                        }
                    }
                },
                command_name="plan execute --auto-compact-worker",
            )

def test_task_dag_materialize_reviewable_output_uses_local_lineage_for_solo_local(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        from ait.cli.app_surfaces import _ctx

        _init_repo(monkeypatch)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_local"})
        graph = _solo_convergence_graph()
        graph["execution_policy"]["change_strategy"] = "local_first_final_local_land"
        ctx = _ctx()
        readiness = _solo_convergence_ready_for_gate(graph)
        readiness["summary"].update({"ready_nodes": 1, "completed_nodes": 2, "next_action": "start C"})
        readiness["counts"].update({"ready": 1, "completed": 2})
        readiness["nodes"][2].update({"state": "ready", "reason": "ready", "lineage": {}})
        local_task_calls = []
        local_change_calls = []
        repo_ait = Path.cwd() / ".ait"
        dep_a_root = Path.cwd() / "dep-a"
        dep_b_root = Path.cwd() / "dep-b"
        target_root = Path.cwd() / ".ait" / "workspace" / "lt-local-f"
        for root, worktree_name in ((dep_a_root, "lt-a"), (dep_b_root, "lt-b"), (target_root, "lt-local-f")):
            root.mkdir(parents=True, exist_ok=True)
            (root / ".ait-worktree.json").write_text(json.dumps({"worktree_name": worktree_name}), encoding="utf-8")
        (dep_a_root / "a.txt").write_text("LIVE residue A\n", encoding="utf-8")
        (dep_b_root / "nested").mkdir(parents=True, exist_ok=True)
        (dep_b_root / "nested" / "b.txt").write_text("LIVE residue B\n", encoding="utf-8")
        (dep_b_root / "live-only.txt").write_text("should not leak\n", encoding="utf-8")
        (target_root / "obsolete.txt").write_text("remove me\n", encoding="utf-8")
        dep_worktrees = {
            "T-A": {
                "name": "lt-a",
                "path": str(dep_a_root),
                "fork_snapshot_id": "SNP-A",
                "bound_task_id": "T-A",
            },
            "T-B": {
                "name": "lt-b",
                "path": str(dep_b_root),
                "fork_snapshot_id": "SNP-B",
                "bound_task_id": "T-B",
            },
        }
        snapshot_bundles = {
            "SNP-base-A": {"snapshot_id": "SNP-base-A", "files": []},
            "SNP-A": {
                "snapshot_id": "SNP-A",
                "files": [
                    {
                        "path": "a.txt",
                        "sha256": "1ca70c6e7446967c8182584ee1e08ed2f8fce20a0ab14da419b0f54f09996977",
                        "mode": "0o644",
                        "content_b64": base64.b64encode(b"A output\n").decode("ascii"),
                    }
                ],
            },
            "SNP-base-B": {
                "snapshot_id": "SNP-base-B",
                "files": [
                    {
                        "path": "obsolete.txt",
                        "sha256": "58cc47bb940d03f6a97df3eb233384576da4e69d4e3524b99b4c7f4f6a7b5f7a",
                        "mode": "0o644",
                        "content_b64": base64.b64encode(b"remove me\n").decode("ascii"),
                    }
                ],
            },
            "SNP-B": {
                "snapshot_id": "SNP-B",
                "files": [
                    {
                        "path": "nested/b.txt",
                        "sha256": "66f1f0f55f42b4d29b34d2ef38fa3b7f96c6f2f34093dc26406454f2fce5122a",
                        "mode": "0o644",
                        "content_b64": base64.b64encode(b"B output\n").decode("ascii"),
                    }
                ],
            },
        }

        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "create_local_task",
            lambda current_ctx, title, intent, risk_tier, *, plan_id=None, origin_plan_revision_id=None, plan_item_ref=None: (
                local_task_calls.append(
                    {
                        "title": title,
                        "intent": intent,
                        "risk_tier": risk_tier,
                        "plan_id": plan_id,
                        "origin_plan_revision_id": origin_plan_revision_id,
                        "plan_item_ref": plan_item_ref,
                    }
                )
                or {
                    "task_id": "LT-LOCAL-F",
                    "title": title,
                    "status": "active",
                }
            ),
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "create_local_change",
            lambda current_ctx, task_id, title, base_line, risk_tier: (
                local_change_calls.append(
                    {
                        "task_id": task_id,
                        "title": title,
                        "base_line": base_line,
                        "risk_tier": risk_tier,
                    }
                )
                or {
                    "change_id": "LC-LOCAL-F",
                    "task_id": task_id,
                    "title": title,
                    "status": "draft",
                }
            ),
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_task",
            lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("solo_local reviewable node should not open remote task")),
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_change",
            lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("solo_local reviewable node should not open remote change")),
        )
        monkeypatch.setattr(
            plan_cli_module,
            "plan_sync",
            lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("solo_local reviewable node should not auto-publish plan lineage")),
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_repo_ctx", lambda current_ctx: current_ctx)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "current_line", lambda current_ctx: "main")
        monkeypatch.setattr(task_dag_node_bootstrap_module, "get_line", lambda current_ctx, name: {"head_snapshot_id": "SNP-MAIN"})
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "export_snapshot_bundle",
            lambda current_ctx, snapshot_id, repo_name: dict(snapshot_bundles[snapshot_id]),
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "_remote_change_lineage_payload",
            lambda base_url, repo_name, base_line: {"forked_from_line": base_line, "fork_snapshot_id": "SNP-MAIN"},
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_ensure_task_feature_line", lambda *args, **kwargs: {"line_name": "feature/lt-local-f"})
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "_find_bound_task_worktree",
            lambda current_ctx, task_id: dep_worktrees.get(task_id),
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_resolve_task_bound_worktree_name", lambda *args, **kwargs: "lt-local-f")
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "local_add_worktree",
            lambda *args, **kwargs: {"name": "lt-local-f"},
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "local_bind_worktree",
            lambda *args, **kwargs: {
                "name": "lt-local-f",
                "bound_task_id": "LT-LOCAL-F",
                "bound_change_id": "LC-LOCAL-F",
                "current_line": "feature/lt-local-f",
                "path": str(Path.cwd() / ".ait" / "workspace" / "lt-local-f"),
                "workspace_root": str(Path.cwd() / ".ait" / "workspace" / "lt-local-f"),
                "open_path": str(Path.cwd() / ".ait" / "workspace" / "lt-local-f"),
                "alias_path": str(Path.cwd() / ".ait" / "workspace" / "lt-local-f"),
                "auto_created_for_task": True,
                "target_base_line": "main",
            },
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_output", lambda worktree: dict(worktree))

        payload = task_dag_node_bootstrap_module._task_dag_materialize_node_lineage(
            ctx=ctx,
            remote_row={"url": "http://example.test"},
            repo_name="demo",
            plan_id="PL-demo",
            graph=graph,
            readiness=readiness,
            node_id="C",
            create_worktree=True,
            allow_execution_only_without_change=True,
        )

        assert payload["task_id"] == "LT-LOCAL-F"
        assert payload["change_id"] == "LC-LOCAL-F"
        assert payload["workflow_boundary"] == "reviewable_output"
        assert len(local_task_calls) == 1
        assert len(local_change_calls) == 1
        assert local_task_calls[0]["origin_plan_revision_id"] == graph["source_plan"]["plan_revision_id"]
        assert local_change_calls[0]["task_id"] == "LT-LOCAL-F"
        assert payload["worktree"]["name"] == "lt-local-f"
        assert payload["dependency_replay"]["replayed_node_ids"] == ["A", "B"]
        assert (target_root / "a.txt").read_text(encoding="utf-8") == "A output\n"
        assert (target_root / "nested" / "b.txt").read_text(encoding="utf-8") == "B output\n"
        assert not (target_root / "live-only.txt").exists()
        assert not (target_root / "obsolete.txt").exists()


def test_task_dag_materialize_reviewable_output_uses_remote_lineage_for_solo_remote(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        from ait.cli.app_surfaces import _ctx

        _init_repo(monkeypatch)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"})
        monkeypatch.setattr(cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"}, raising=False)
        graph = _solo_convergence_graph()
        graph["source_plan"]["plan_revision_id"] = "PR-local-demo"
        ctx = _ctx()
        readiness = _solo_convergence_ready_for_gate(graph)
        readiness["summary"].update({"ready_nodes": 1, "completed_nodes": 2, "next_action": "start C"})
        readiness["counts"].update({"ready": 1, "completed": 2})
        readiness["nodes"][2].update({"state": "ready", "reason": "ready", "lineage": {}})
        remote_task_calls = []
        remote_change_calls = []
        repo_ait = Path.cwd() / ".ait"
        dep_a_root = Path.cwd() / "remote-dep-a"
        dep_b_root = Path.cwd() / "remote-dep-b"
        target_root = Path.cwd() / ".ait" / "workspace" / "rt-remote-f"
        for root, worktree_name in ((dep_a_root, "lt-a"), (dep_b_root, "lt-b"), (target_root, "rt-remote-f")):
            root.mkdir(parents=True, exist_ok=True)
            (root / ".ait-worktree.json").write_text(json.dumps({"worktree_name": worktree_name}), encoding="utf-8")
        (dep_a_root / "alpha.txt").write_text("LIVE remote residue A\n", encoding="utf-8")
        (dep_b_root / "beta.txt").write_text("LIVE remote residue B\n", encoding="utf-8")
        (dep_b_root / "live-only.txt").write_text("should not leak\n", encoding="utf-8")
        dep_worktrees = {
            "T-A": {
                "name": "lt-a",
                "path": str(dep_a_root),
                "fork_snapshot_id": "SNP-A",
                "bound_task_id": "T-A",
            },
            "T-B": {
                "name": "lt-b",
                "path": str(dep_b_root),
                "fork_snapshot_id": "SNP-B",
                "bound_task_id": "T-B",
            },
        }
        snapshot_bundles = {
            "SNP-base-A": {"snapshot_id": "SNP-base-A", "files": []},
            "SNP-A": {
                "snapshot_id": "SNP-A",
                "files": [
                    {
                        "path": "alpha.txt",
                        "sha256": "202c4153dca0f57790796e1d187e6fe819f2f30644f63741d7f6f6db9e8f4c92",
                        "mode": "0o644",
                        "content_b64": base64.b64encode(b"remote A\n").decode("ascii"),
                    }
                ],
            },
            "SNP-base-B": {"snapshot_id": "SNP-base-B", "files": []},
            "SNP-B": {
                "snapshot_id": "SNP-B",
                "files": [
                    {
                        "path": "beta.txt",
                        "sha256": "39dc8f95c5b99f6c34f90be2797da9d8ac1ea49aa6cc59e5f7b7f1065e8ea6ee",
                        "mode": "0o644",
                        "content_b64": base64.b64encode(b"remote B\n").decode("ascii"),
                    }
                ],
            },
        }

        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "create_local_task",
            lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("solo_remote final node should not open local task")),
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "create_local_change",
            lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("solo_remote final node should not open local change")),
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_task",
            lambda base_url, repo_name, title, intent, risk_tier, **kwargs: (
                remote_task_calls.append(
                    {
                        "base_url": base_url,
                        "repo_name": repo_name,
                        "title": title,
                        "intent": intent,
                        "risk_tier": risk_tier,
                        **kwargs,
                    }
                )
                or {
                    "task_id": "RT-REMOTE-F",
                    "title": title,
                    "status": "active",
                }
            ),
        )
        local_plan_row = {
            "plan_id": "PL-demo",
            "head_revision_id": "PR-local-demo",
            "published_head_revision_id": None,
        }
        local_revision_row = {
            "plan_id": "PL-demo",
            "plan_revision_id": "PR-local-demo",
            "published_plan_revision_id": None,
        }
        plan_sync_calls = []
        monkeypatch.setattr(task_dag_runtime_helpers_module, "get_local_plan", lambda current_ctx, plan_id: dict(local_plan_row))
        monkeypatch.setattr(
            task_dag_runtime_helpers_module,
            "get_local_plan_revision",
            lambda current_ctx, plan_id, plan_revision_id: dict(local_revision_row),
        )
        monkeypatch.setattr(cli_module, "get_local_plan", lambda current_ctx, plan_id: dict(local_plan_row), raising=False)
        monkeypatch.setattr(
            cli_module,
            "get_local_plan_revision",
            lambda current_ctx, plan_id, plan_revision_id: dict(local_revision_row),
            raising=False,
        )
        monkeypatch.setattr(
            plan_cli_module,
            "plan_sync",
            lambda target, **kwargs: (
                plan_sync_calls.append(
                    {
                        "target": str(target),
                        "remote": kwargs.get("remote"),
                        "json_output": kwargs.get("json_output"),
                    }
                )
                or local_plan_row.__setitem__("published_head_revision_id", "PR-remote-demo")
                or local_revision_row.__setitem__("published_plan_revision_id", "PR-remote-demo")
                or print(json.dumps({"status": "ok", "publish_results": [{"published_head_revision_id": "PR-remote-demo"}]}))
            ),
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_change",
            lambda base_url, repo_name, task_id, title, **kwargs: (
                remote_change_calls.append(
                    {
                        "base_url": base_url,
                        "repo_name": repo_name,
                        "task_id": task_id,
                        "title": title,
                        **kwargs,
                    }
                )
                or {
                    "change_id": "RC-REMOTE-F",
                    "task_id": task_id,
                    "title": title,
                    "status": "draft",
                }
            ),
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_repo_ctx", lambda current_ctx: current_ctx)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "current_line", lambda current_ctx: "main")
        monkeypatch.setattr(task_dag_node_bootstrap_module, "get_line", lambda current_ctx, name: {"head_snapshot_id": "SNP-MAIN"})
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "export_snapshot_bundle",
            lambda current_ctx, snapshot_id, repo_name: dict(snapshot_bundles[snapshot_id]),
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "_remote_change_lineage_payload",
            lambda base_url, repo_name, base_line: {"forked_from_line": base_line, "fork_snapshot_id": "SNP-MAIN"},
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_ensure_task_feature_line", lambda *args, **kwargs: {"line_name": "feature/rt-remote-f"})
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "_find_bound_task_worktree",
            lambda current_ctx, task_id: dep_worktrees.get(task_id),
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_resolve_task_bound_worktree_name", lambda *args, **kwargs: "rt-remote-f")
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "local_add_worktree",
            lambda *args, **kwargs: {"name": "rt-remote-f"},
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "local_bind_worktree",
            lambda *args, **kwargs: {
                "name": "rt-remote-f",
                "bound_task_id": "RT-REMOTE-F",
                "bound_change_id": "RC-REMOTE-F",
                "current_line": "feature/rt-remote-f",
                "path": str(Path.cwd() / ".ait" / "workspace" / "rt-remote-f"),
                "workspace_root": str(Path.cwd() / ".ait" / "workspace" / "rt-remote-f"),
                "open_path": str(Path.cwd() / ".ait" / "workspace" / "rt-remote-f"),
                "alias_path": str(Path.cwd() / ".ait" / "workspace" / "rt-remote-f"),
                "auto_created_for_task": True,
                "target_base_line": "main",
            },
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_output", lambda worktree: dict(worktree))

        payload = task_dag_node_bootstrap_module._task_dag_materialize_node_lineage(
            ctx=ctx,
            remote_row={"url": "http://example.test", "name": "origin"},
            repo_name="demo",
            plan_id="PL-demo",
            graph=graph,
            readiness=readiness,
            node_id="C",
            create_worktree=True,
            allow_execution_only_without_change=True,
        )

        assert payload["task_id"] == "RT-REMOTE-F"
        assert payload["change_id"] == "RC-REMOTE-F"
        assert len(remote_task_calls) == 1
        assert len(remote_change_calls) == 1
        assert len(plan_sync_calls) == 1
        assert plan_sync_calls[0]["remote"] == "origin"
        assert plan_sync_calls[0]["json_output"] is True
        assert remote_task_calls[0]["origin_plan_revision_id"] == "PR-remote-demo"
        assert remote_change_calls[0]["task_id"] == "RT-REMOTE-F"
        assert payload["worktree"]["name"] == "rt-remote-f"
        assert payload["dependency_replay"]["replayed_node_ids"] == ["A", "B"]
        assert (target_root / "alpha.txt").read_text(encoding="utf-8") == "remote A\n"
        assert (target_root / "beta.txt").read_text(encoding="utf-8") == "remote B\n"
        assert not (target_root / "live-only.txt").exists()


def test_task_dag_export_snapshot_bundle_accepts_two_arg_app_override(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        from ait.cli.app_surfaces import _ctx

        _init_repo(monkeypatch)
        ctx = _ctx()
        export_calls = []
        monkeypatch.setattr(
            cli_module,
            "export_snapshot_bundle",
            lambda current_ctx, snapshot_id: (
                export_calls.append(
                    {
                        "ctx_root": str(current_ctx.root),
                        "snapshot_id": snapshot_id,
                    }
                )
                or {
                    "snapshot_id": snapshot_id,
                    "files": [],
                }
            ),
            raising=False,
        )

        payload = task_dag_node_bootstrap_module.export_snapshot_bundle(ctx, "SNP-demo", "demo")

        assert payload["snapshot_id"] == "SNP-demo"
        assert export_calls == [
            {
                "ctx_root": str(ctx.root),
                "snapshot_id": "SNP-demo",
            }
        ]


def test_task_dag_graph_for_remote_prefers_published_plan_revision_mapping(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        from ait.cli.app_surfaces import _ctx

        _init_repo(monkeypatch)
        ctx = _ctx()
        graph = _graph_with_dispatch_artifacts()
        graph["source_plan"]["plan_revision_id"] = "PR-local-demo"
        graph_path = Path("docs/sprints/demo.task_graph.json")

        local_plan_row = {
            "plan_id": "PL-demo",
            "head_revision_id": "PR-local-demo",
            "published_head_revision_id": "PR-remote-demo",
        }
        local_revision_row = {
            "plan_id": "PL-demo",
            "plan_revision_id": "PR-local-demo",
            "published_plan_revision_id": "PR-remote-demo",
        }
        monkeypatch.setattr(task_dag_runtime_helpers_module, "get_local_plan", lambda current_ctx, plan_id: dict(local_plan_row))
        monkeypatch.setattr(
            task_dag_runtime_helpers_module,
            "get_local_plan_revision",
            lambda current_ctx, plan_id, plan_revision_id: dict(local_revision_row),
        )
        monkeypatch.setattr(cli_module, "get_local_plan", lambda current_ctx, plan_id: dict(local_plan_row), raising=False)
        monkeypatch.setattr(
            cli_module,
            "get_local_plan_revision",
            lambda current_ctx, plan_id, plan_revision_id: dict(local_revision_row),
            raising=False,
        )

        payload = task_dag_runtime_helpers_module._task_dag_graph_for_remote(ctx, graph, graph_path)

        assert payload["source_plan"]["plan_revision_id"] == "PR-remote-demo"
        assert payload["dispatch_artifacts"]["task_graph_json"] == "docs/sprints/demo.task_graph.json"


def test_task_dag_remote_plan_revision_id_auto_publishes_source_plan_only_in_solo_remote(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        from ait.cli.app_surfaces import _ctx

        _init_repo(monkeypatch)
        ctx = _ctx()
        graph = _graph()
        graph["source_plan"]["plan_revision_id"] = "PR-local-demo"

        local_plan_row = {
            "plan_id": "PL-demo",
            "head_revision_id": "PR-local-demo",
            "published_head_revision_id": None,
        }
        local_revision_row = {
            "plan_id": "PL-demo",
            "plan_revision_id": "PR-local-demo",
            "published_plan_revision_id": None,
        }
        plan_sync_calls = []
        monkeypatch.setattr(task_dag_runtime_helpers_module, "get_local_plan", lambda current_ctx, plan_id: dict(local_plan_row))
        monkeypatch.setattr(
            task_dag_runtime_helpers_module,
            "get_local_plan_revision",
            lambda current_ctx, plan_id, plan_revision_id: dict(local_revision_row),
        )
        monkeypatch.setattr(cli_module, "get_local_plan", lambda current_ctx, plan_id: dict(local_plan_row), raising=False)
        monkeypatch.setattr(
            cli_module,
            "get_local_plan_revision",
            lambda current_ctx, plan_id, plan_revision_id: dict(local_revision_row),
            raising=False,
        )
        monkeypatch.setattr(
            plan_cli_module,
            "plan_sync",
            lambda target, **kwargs: (
                plan_sync_calls.append({"target": str(target), "remote": kwargs.get("remote")})
                or local_plan_row.__setitem__("published_head_revision_id", "PR-remote-demo")
                or local_revision_row.__setitem__("published_plan_revision_id", "PR-remote-demo")
                or print(json.dumps({"status": "ok", "publish_results": [{"published_head_revision_id": "PR-remote-demo"}]}))
            ),
        )

        monkeypatch.setattr(cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"}, raising=False)
        remote_revision_id = task_dag_runtime_helpers_module._task_dag_remote_plan_revision_id(
            ctx,
            graph,
            remote_name="origin",
            auto_publish_if_needed=True,
        )

        assert len(plan_sync_calls) == 1
        assert plan_sync_calls[0]["remote"] == "origin"
        assert remote_revision_id == "PR-remote-demo"

        local_plan_row["published_head_revision_id"] = None
        local_revision_row["published_plan_revision_id"] = None
        monkeypatch.setattr(cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_local"}, raising=False)
        remote_revision_id = task_dag_runtime_helpers_module._task_dag_remote_plan_revision_id(
            ctx,
            graph,
            remote_name="origin",
            auto_publish_if_needed=True,
        )

        assert len(plan_sync_calls) == 1
        assert remote_revision_id == "PR-local-demo"


def test_task_dag_materialize_reviewable_output_blocks_stale_dependency_worktree(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        from ait.cli.app_surfaces import _ctx

        _init_repo(monkeypatch)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"})
        graph = _solo_convergence_graph()
        ctx = _ctx()
        readiness = _solo_convergence_ready_for_gate(graph)
        readiness["summary"].update({"ready_nodes": 1, "completed_nodes": 2, "next_action": "start C"})
        readiness["counts"].update({"ready": 1, "completed": 2})
        readiness["nodes"][2].update({"state": "ready", "reason": "ready", "lineage": {}})
        target_root = Path.cwd() / ".ait" / "workspace" / "rt-remote-f"
        target_root.mkdir(parents=True, exist_ok=True)
        (target_root / ".ait-worktree.json").write_text(json.dumps({"worktree_name": "rt-remote-f"}), encoding="utf-8")
        dep_worktrees = {
            "T-A": {
                "name": "lt-a",
                "bound_task_id": "T-A",
                "fork_snapshot_id": "SNP-base-A",
            },
            "T-B": {
                "name": "lt-b",
                "bound_task_id": "T-B",
                "fork_snapshot_id": "SNP-base-B",
                "retarget": {
                    "needs_retarget": True,
                    "fork_snapshot_id": "SNP-base-B",
                    "target_base_line": "main",
                    "target_base_snapshot_id": "SNP-MAIN",
                    "rebase_state": "idle",
                },
            },
        }

        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_task",
            lambda *args, **kwargs: {"task_id": "RT-REMOTE-F", "status": "active"},
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_change",
            lambda *args, **kwargs: {
                "change_id": "RC-REMOTE-F",
                "status": "draft",
                "base_line": kwargs.get("base_line"),
                "fork_snapshot_id": kwargs.get("fork_snapshot_id"),
                "forked_from_line": kwargs.get("forked_from_line"),
            },
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_repo_ctx", lambda current_ctx: current_ctx)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "current_line", lambda current_ctx: "main")
        monkeypatch.setattr(task_dag_node_bootstrap_module, "get_line", lambda current_ctx, name: {"head_snapshot_id": "SNP-MAIN"})
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_remote_change_lineage_payload", lambda *args, **kwargs: {})
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_ensure_task_feature_line", lambda *args, **kwargs: {"line_name": "feature/rt-remote-f"})
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "_find_bound_task_worktree",
            lambda current_ctx, task_id: dep_worktrees.get(task_id) if task_id in dep_worktrees else None,
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_resolve_task_bound_worktree_name", lambda *args, **kwargs: "rt-remote-f")
        monkeypatch.setattr(task_dag_node_bootstrap_module, "local_add_worktree", lambda *args, **kwargs: {"name": "rt-remote-f"})
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "local_bind_worktree",
            lambda *args, **kwargs: {
                "name": "rt-remote-f",
                "bound_task_id": "RT-REMOTE-F",
                "bound_change_id": "RC-REMOTE-F",
                "current_line": "feature/rt-remote-f",
                "path": str(target_root),
                "workspace_root": str(target_root),
                "open_path": str(target_root),
                "alias_path": str(target_root),
                "auto_created_for_task": True,
                "target_base_line": "main",
            },
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_output", lambda worktree: dict(worktree))
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "export_snapshot_bundle",
            lambda *args, **kwargs: {"snapshot_id": "unused", "files": []},
        )

        try:
            task_dag_node_bootstrap_module._task_dag_materialize_node_lineage(
                ctx=ctx,
                remote_row={"url": "http://example.test"},
                repo_name="demo",
                plan_id="PL-demo",
                graph=graph,
                readiness=readiness,
                node_id="C",
                create_worktree=True,
                allow_execution_only_without_change=True,
            )
        except ValueError as exc:
            message = str(exc)
        else:
            raise AssertionError("expected stale dependency worktree guard")

        assert "dependency node `B`" in message
        assert "lt-b" in message
        assert "ait worktree rebase --onto main" in message


def test_task_dag_dependency_replay_guard_allows_snapshot_already_on_target_line(monkeypatch):
    dependency_row = {
        "node_id": "B",
        "state": "completed",
        "lineage": {
            "task_id": "T-B",
            "completion_snapshot_id": "SNP-B",
            "completion_fork_snapshot_id": "SNP-base-B",
        },
    }
    monkeypatch.setattr(
        task_dag_node_bootstrap_module,
        "_find_bound_task_worktree",
        lambda ctx, task_id: {
            "name": "lt-b",
            "bound_task_id": task_id,
            "retarget": {
                "needs_retarget": True,
                "fork_snapshot_id": "SNP-base-B",
                "target_base_line": "main",
                "target_base_snapshot_id": "SNP-main",
                "rebase_state": "idle",
            },
        },
    )
    monkeypatch.setattr(
        task_dag_node_bootstrap_module,
        "collect_snapshot_chain",
        lambda ctx, snapshot_id: ["SNP-main", "SNP-B", "SNP-base-B"],
    )

    message = task_dag_node_bootstrap_module._task_dag_dependency_replay_guard_message(
        dependency_row=dependency_row,
        dependency_node_id="B",
        node_id="F",
        repo_ctx=SimpleNamespace(),
    )

    assert message is None


def test_task_dag_materialize_reviewable_output_blocks_conflicted_dependency_worktree(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        from ait.cli.app_surfaces import _ctx

        _init_repo(monkeypatch)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"})
        graph = _solo_convergence_graph()
        ctx = _ctx()
        readiness = _solo_convergence_ready_for_gate(graph)
        readiness["summary"].update({"ready_nodes": 1, "completed_nodes": 2, "next_action": "start C"})
        readiness["counts"].update({"ready": 1, "completed": 2})
        readiness["nodes"][2].update({"state": "ready", "reason": "ready", "lineage": {}})
        target_root = Path.cwd() / ".ait" / "workspace" / "rt-remote-f"
        target_root.mkdir(parents=True, exist_ok=True)
        (target_root / ".ait-worktree.json").write_text(json.dumps({"worktree_name": "rt-remote-f"}), encoding="utf-8")
        dep_worktrees = {
            "T-A": {
                "name": "lt-a",
                "bound_task_id": "T-A",
            },
            "T-B": {
                "name": "lt-b",
                "bound_task_id": "T-B",
                "retarget": {
                    "needs_retarget": False,
                    "rebase_state": "conflicted",
                    "rebase_conflict_paths": ["alpha.txt"],
                },
            },
        }

        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_task",
            lambda *args, **kwargs: {"task_id": "RT-REMOTE-F", "status": "active"},
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_change",
            lambda *args, **kwargs: {
                "change_id": "RC-REMOTE-F",
                "status": "draft",
                "base_line": kwargs.get("base_line"),
                "fork_snapshot_id": kwargs.get("fork_snapshot_id"),
                "forked_from_line": kwargs.get("forked_from_line"),
            },
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_repo_ctx", lambda current_ctx: current_ctx)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "current_line", lambda current_ctx: "main")
        monkeypatch.setattr(task_dag_node_bootstrap_module, "get_line", lambda current_ctx, name: {"head_snapshot_id": "SNP-MAIN"})
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_remote_change_lineage_payload", lambda *args, **kwargs: {})
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_ensure_task_feature_line", lambda *args, **kwargs: {"line_name": "feature/rt-remote-f"})
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "_find_bound_task_worktree",
            lambda current_ctx, task_id: dep_worktrees.get(task_id) if task_id in dep_worktrees else None,
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_resolve_task_bound_worktree_name", lambda *args, **kwargs: "rt-remote-f")
        monkeypatch.setattr(task_dag_node_bootstrap_module, "local_add_worktree", lambda *args, **kwargs: {"name": "rt-remote-f"})
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "local_bind_worktree",
            lambda *args, **kwargs: {
                "name": "rt-remote-f",
                "bound_task_id": "RT-REMOTE-F",
                "bound_change_id": "RC-REMOTE-F",
                "current_line": "feature/rt-remote-f",
                "path": str(target_root),
                "workspace_root": str(target_root),
                "open_path": str(target_root),
                "alias_path": str(target_root),
                "auto_created_for_task": True,
                "target_base_line": "main",
            },
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_output", lambda worktree: dict(worktree))
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "export_snapshot_bundle",
            lambda *args, **kwargs: {"snapshot_id": "unused", "files": []},
        )

        try:
            task_dag_node_bootstrap_module._task_dag_materialize_node_lineage(
                ctx=ctx,
                remote_row={"url": "http://example.test"},
                repo_name="demo",
                plan_id="PL-demo",
                graph=graph,
                readiness=readiness,
                node_id="C",
                create_worktree=True,
                allow_execution_only_without_change=True,
            )
        except ValueError as exc:
            message = str(exc)
        else:
            raise AssertionError("expected conflicted dependency worktree guard")

        assert "dependency node `B`" in message
        assert "alpha.txt" in message
        assert "ait worktree rebase --continue" in message


def test_task_dag_materialize_reviewable_output_detects_dependency_hotspots_before_writing(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        from ait.cli.app_surfaces import _ctx

        _init_repo(monkeypatch)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"})
        graph = _solo_convergence_graph()
        ctx = _ctx()
        readiness = _solo_convergence_ready_for_gate(graph)
        readiness["summary"].update({"ready_nodes": 1, "completed_nodes": 2, "next_action": "start C"})
        readiness["counts"].update({"ready": 1, "completed": 2})
        readiness["nodes"][2].update({"state": "ready", "reason": "ready", "lineage": {}})
        target_root = Path.cwd() / ".ait" / "workspace" / "rt-remote-f"
        target_root.mkdir(parents=True, exist_ok=True)
        (target_root / ".ait-worktree.json").write_text(json.dumps({"worktree_name": "rt-remote-f"}), encoding="utf-8")
        snapshot_bundles = {
            "SNP-base-A": {"snapshot_id": "SNP-base-A", "files": []},
            "SNP-base-B": {"snapshot_id": "SNP-base-B", "files": []},
            "SNP-A": {
                "snapshot_id": "SNP-A",
                "files": [
                    {
                        "path": "shared.txt",
                        "sha256": "sha-a",
                        "mode": "0o644",
                        "content_b64": base64.b64encode(b"A output\n").decode("ascii"),
                    }
                ],
            },
            "SNP-B": {
                "snapshot_id": "SNP-B",
                "files": [
                    {
                        "path": "shared.txt",
                        "sha256": "sha-b",
                        "mode": "0o644",
                        "content_b64": base64.b64encode(b"B output\n").decode("ascii"),
                    }
                ],
            },
        }
        dep_worktrees = {
            "T-A": {"name": "lt-a", "bound_task_id": "T-A"},
            "T-B": {"name": "lt-b", "bound_task_id": "T-B"},
        }

        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_task",
            lambda *args, **kwargs: {"task_id": "RT-REMOTE-F", "status": "active"},
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_change",
            lambda *args, **kwargs: {
                "change_id": "RC-REMOTE-F",
                "status": "draft",
                "base_line": kwargs.get("base_line"),
                "fork_snapshot_id": kwargs.get("fork_snapshot_id"),
                "forked_from_line": kwargs.get("forked_from_line"),
            },
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_repo_ctx", lambda current_ctx: current_ctx)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "current_line", lambda current_ctx: "main")
        monkeypatch.setattr(task_dag_node_bootstrap_module, "get_line", lambda current_ctx, name: {"head_snapshot_id": "SNP-MAIN"})
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_remote_change_lineage_payload", lambda *args, **kwargs: {})
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_ensure_task_feature_line", lambda *args, **kwargs: {"line_name": "feature/rt-remote-f"})
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "_find_bound_task_worktree",
            lambda current_ctx, task_id: dep_worktrees.get(task_id) if task_id in dep_worktrees else None,
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_resolve_task_bound_worktree_name", lambda *args, **kwargs: "rt-remote-f")
        monkeypatch.setattr(task_dag_node_bootstrap_module, "local_add_worktree", lambda *args, **kwargs: {"name": "rt-remote-f"})
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "local_bind_worktree",
            lambda *args, **kwargs: {
                "name": "rt-remote-f",
                "bound_task_id": "RT-REMOTE-F",
                "bound_change_id": "RC-REMOTE-F",
                "current_line": "feature/rt-remote-f",
                "path": str(target_root),
                "workspace_root": str(target_root),
                "open_path": str(target_root),
                "alias_path": str(target_root),
                "auto_created_for_task": True,
                "target_base_line": "main",
            },
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_output", lambda worktree: dict(worktree))
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "export_snapshot_bundle",
            lambda current_ctx, snapshot_id, repo_name: dict(snapshot_bundles[snapshot_id]),
        )

        try:
            task_dag_node_bootstrap_module._task_dag_materialize_node_lineage(
                ctx=ctx,
                remote_row={"url": "http://example.test"},
                repo_name="demo",
                plan_id="PL-demo",
                graph=graph,
                readiness=readiness,
                node_id="C",
                create_worktree=True,
                allow_execution_only_without_change=True,
            )
        except ValueError as exc:
            message = str(exc)
        else:
            raise AssertionError("expected dependency hotspot guard")

        assert "shared.txt" in message
        assert "dependency nodes `A` and `B`" in message
        assert not (target_root / "shared.txt").exists()


def test_task_dag_materialize_reviewable_output_bootstraps_feature_line_from_target_line(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        from ait.cli.app_surfaces import _ctx

        _init_repo(monkeypatch)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"})
        graph = _solo_convergence_graph()
        graph["nodes"][2]["target_line"] = "release-main"
        ctx = _ctx()
        readiness = _solo_convergence_ready_for_gate(graph)
        readiness["summary"].update({"ready_nodes": 1, "completed_nodes": 2, "next_action": "start C"})
        readiness["counts"].update({"ready": 1, "completed": 2})
        readiness["nodes"][2].update({"state": "ready", "reason": "ready", "lineage": {}})
        target_root = Path.cwd() / ".ait" / "workspace" / "rt-remote-f"
        target_root.mkdir(parents=True, exist_ok=True)
        (target_root / ".ait-worktree.json").write_text(json.dumps({"worktree_name": "rt-remote-f"}), encoding="utf-8")
        ensure_calls = []
        bind_calls = []

        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_task",
            lambda *args, **kwargs: {"task_id": "RT-REMOTE-F", "status": "active"},
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_change",
            lambda *args, **kwargs: {
                "change_id": "RC-REMOTE-F",
                "status": "draft",
                "base_line": kwargs.get("base_line"),
                "fork_snapshot_id": kwargs.get("fork_snapshot_id"),
                "forked_from_line": kwargs.get("forked_from_line"),
            },
        )
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_repo_ctx", lambda current_ctx: current_ctx)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "current_line", lambda current_ctx: "main")
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "get_line",
            lambda current_ctx, name: {"head_snapshot_id": f"SNP-{name}"},
        )
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "_remote_change_lineage_payload",
            lambda *args, **kwargs: {"fork_snapshot_id": "SNP-release-main", "forked_from_line": "release-main"},
        )

        def fake_ensure_task_feature_line(current_ctx, *, task_id, base_line_name, base_snapshot_id=None):
            ensure_calls.append(
                {"task_id": task_id, "base_line_name": base_line_name, "base_snapshot_id": base_snapshot_id}
            )
            return {"line_name": "feature/rt-remote-f"}

        monkeypatch.setattr(task_dag_node_bootstrap_module, "_ensure_task_feature_line", fake_ensure_task_feature_line)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_find_bound_task_worktree", lambda *args, **kwargs: None)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_resolve_task_bound_worktree_name", lambda *args, **kwargs: "rt-remote-f")
        monkeypatch.setattr(task_dag_node_bootstrap_module, "local_add_worktree", lambda *args, **kwargs: {"name": "rt-remote-f"})

        def fake_local_bind_worktree(*args, **kwargs):
            bind_calls.append(dict(kwargs))
            return {
                "name": "rt-remote-f",
                "bound_task_id": "RT-REMOTE-F",
                "bound_change_id": "RC-REMOTE-F",
                "current_line": "feature/rt-remote-f",
                "path": str(target_root),
                "workspace_root": str(target_root),
                "open_path": str(target_root),
                "alias_path": str(target_root),
                "auto_created_for_task": True,
                "target_base_line": kwargs.get("target_base_line"),
            }

        monkeypatch.setattr(task_dag_node_bootstrap_module, "local_bind_worktree", fake_local_bind_worktree)
        monkeypatch.setattr(task_dag_node_bootstrap_module, "_task_worktree_output", lambda worktree: dict(worktree))
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "export_snapshot_bundle",
            lambda current_ctx, snapshot_id, repo_name: {"snapshot_id": snapshot_id, "files": []},
        )

        payload = task_dag_node_bootstrap_module._task_dag_materialize_node_lineage(
            ctx=ctx,
            remote_row={"url": "http://example.test"},
            repo_name="demo",
            plan_id="PL-demo",
            graph=graph,
            readiness=readiness,
            node_id="C",
            create_worktree=True,
            allow_execution_only_without_change=True,
        )

        assert payload["change_id"] == "RC-REMOTE-F"
        assert ensure_calls == [
            {
                "task_id": "RT-REMOTE-F",
                "base_line_name": "release-main",
                "base_snapshot_id": "SNP-release-main",
            }
        ]
        assert bind_calls[0]["fork_snapshot_id"] == "SNP-release-main"
        assert bind_calls[0]["target_base_line"] == "release-main"


def test_task_dag_validate_compact_worker_contract_rejects_focus_drift():
    graph = _solo_convergence_graph()
    surface = {
        "final_remote_disposition_default": True,
        "change_focus_policy": {
            "next_focus": {
                "node_id": "C",
                "task_id": "RT-C",
                "change_id": "RC-C",
            }
        },
        "next_focus_node_id": "B",
        "next_focus_task_id": "RT-C",
        "next_focus_change_id": "RC-C",
    }
    try:
        task_dag_compact_worker_runtime_module._task_dag_validate_compact_worker_contract(
            graph,
            compact_packet_surface=surface,
        )
    except ValueError as exc:
        message = str(exc)
    else:
        raise AssertionError("expected focus drift guard")

    assert "compact_packet_surface" in message
    assert "node_id" in message


def test_task_dag_validate_compact_worker_contract_rejects_mode_drift():
    graph = _solo_convergence_graph()
    graph["execution_policy"]["change_strategy"] = "local_first_final_local_land"
    surface = {
        "final_remote_disposition_default": True,
        "change_focus_policy": {
            "next_focus": {
                "node_id": "C",
            }
        },
        "next_focus_node_id": "C",
    }
    try:
        task_dag_compact_worker_runtime_module._task_dag_validate_compact_worker_contract(
            graph,
            compact_packet_surface=surface,
        )
    except ValueError as exc:
        message = str(exc)
    else:
        raise AssertionError("expected mode drift guard")

    assert "mode drift" in message
    assert "final_remote_disposition_default=True" in message


def test_task_dag_compact_worker_completion_snapshot_evidence_accepts_string_worktree_root(
    monkeypatch, tmp_path
):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        ctx = RepoContext.discover(Path.cwd())
        monkeypatch.setattr(
            task_dag_compact_worker_runtime_module,
            "_task_dag_compact_worker_bound_worktree",
            lambda *_args, **_kwargs: {
                "workspace_root": str(Path.cwd()),
                "current_line": "main",
                "fork_snapshot_id": "SNP-fork",
                "name": "lt-demo",
            },
        )
        monkeypatch.setattr(
            task_dag_compact_worker_runtime_module,
            "get_line",
            lambda *_args, **_kwargs: {"head_snapshot_id": "SNP-head"},
        )
        monkeypatch.setattr(
            task_dag_compact_worker_runtime_module,
            "workspace_status",
            lambda *_args, **_kwargs: {"clean": True},
        )

        payload = task_dag_compact_worker_runtime_module._task_dag_compact_worker_completion_snapshot_evidence(
            ctx,
            remote_row={},
            compact_packet_surface={},
            node_id="B",
        )

        assert payload["completion_snapshot_id"] == "SNP-head"
        assert payload["completion_fork_snapshot_id"] == "SNP-fork"
        assert payload["completion_line_name"] == "main"
        assert payload["completion_worktree_name"] == "lt-demo"


def test_plan_execute_auto_compact_worker_refreshes_graph_run_through_final_remote_disposition(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        monkeypatch.setattr(cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_remote"})
        graph = _solo_convergence_graph()
        graph_path = _write_graph(graph, filename="demo_solo_convergence.task_graph.json")

        readiness_calls = {"count": 0}

        def fake_readiness(base_url, current_graph, *, repo_name=None, current_plan_revision_id=None):
            readiness_calls["count"] += 1
            if readiness_calls["count"] == 1:
                return _solo_convergence_initial_readiness(current_graph)
            return _solo_convergence_landed(current_graph)

        remote_tuple = lambda ctx, remote_name: ({"url": "http://example.test"}, "demo")
        monkeypatch.setattr(cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(plan_cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(
            cli_module,
            "_task_dag_readiness_payload",
            lambda ctx, current_graph, remote_name: fake_readiness("http://example.test", current_graph, repo_name="demo"),
        )
        monkeypatch.setattr(
            plan_cli_module,
            "_task_dag_readiness_payload",
            lambda ctx, current_graph, remote_name: fake_readiness("http://example.test", current_graph, repo_name="demo"),
        )

        created_sessions = []
        generated_turns = []
        sessions_by_id = {}
        event_store = {}

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            session_id = f"S-AUTO-{len(created_sessions) + 1}"
            session = {
                "session_id": session_id,
                "session_kind": session_kind,
                "status": "active",
                "title": kwargs.get("title"),
                "metadata": kwargs.get("metadata") or {},
                "repo_name": repo_name,
            }
            sessions_by_id[session_id] = session
            event_store.setdefault(session_id, [])
            created_sessions.append({"base_url": base_url, "repo_name": repo_name, **session, **kwargs})
            return session

        def fake_append_session_event(base_url, session_id, event_type, payload=None, repo_name=None):
            events = event_store.setdefault(session_id, [])
            event = {
                "sequence": len(events) + 1,
                "event_type": event_type,
                "payload": payload or {},
                "repo_name": repo_name,
            }
            events.append(event)
            return event

        def fake_list_session_events(base_url, session_id, repo_name=None):
            return list(event_store.get(session_id, []))

        def fake_get_session(base_url, session_id, repo_name=None):
            return sessions_by_id[session_id]

        def fake_close_session(base_url, session_id, status="paused", repo_name=None):
            session = dict(sessions_by_id[session_id])
            session["status"] = status
            sessions_by_id[session_id] = session
            return session

        def fake_load_reply_generation_config(*, repo_name, repo_root, env):
            return object()

        def fake_generate_session_reply(_config, *, session, events, chat_id, chat_title, checkpoint, surface, actor_identity=None):
            generated_turns.append(
                {
                    "session": session,
                    "events": events,
                    "chat_id": chat_id,
                    "chat_title": chat_title,
                    "checkpoint": checkpoint,
                    "surface": surface,
                    "actor_identity": actor_identity,
                }
            )
            return SimpleNamespace(
                text="worker completed and landed",
                model="gpt-5.4",
                response_id="resp-auto-land",
                usage={"inputTokens": 500, "outputTokens": 100, "totalTokens": 600},
                source="codex",
                turn_analysis={"command_count": 7},
            )

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_session_event)
        monkeypatch.setattr(cli_module, "remote_list_session_events", fake_list_session_events)
        monkeypatch.setattr(cli_module, "remote_get_session", fake_get_session)
        monkeypatch.setattr(cli_module, "remote_close_session", fake_close_session)
        monkeypatch.setattr(cli_module, "load_reply_generation_config", fake_load_reply_generation_config)
        monkeypatch.setattr(cli_module, "generate_session_reply", fake_generate_session_reply)
        local_task_counter = {"value": 0}

        def fake_create_local_task(ctx, title, intent, risk_tier, *, plan_id=None, origin_plan_revision_id=None, plan_item_ref=None):
            local_task_counter["value"] += 1
            return {
                "task_id": f"LT-AUTO-{local_task_counter['value']}",
                "title": title,
                "intent": intent,
                "risk_tier": risk_tier,
                "plan_id": plan_id,
                "origin_plan_revision_id": origin_plan_revision_id,
                "plan_item_ref": plan_item_ref,
                "status": "active",
            }

        monkeypatch.setattr(task_dag_node_bootstrap_module, "create_local_task", fake_create_local_task)
        monkeypatch.setattr(
            cli_module,
            "remote_create_session_turn",
            lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("auto-compact-worker should not use remote session turn")),
        )

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--auto-compact-worker",
                "--yes",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        contract = payload["execute_run_contract"]
        assert contract["final_remote_disposition_default"] is True
        auto_worker = payload["auto_compact_worker"]
        assert auto_worker["final_remote_disposition_default"] is True
        assert (
            auto_worker["reply_poll_timeout_seconds"]
            == cli_module.DEFAULT_TASK_DAG_IMPLEMENTATION_AUTO_LAND_REPLY_POLL_TIMEOUT_SECONDS
        )
        assert created_sessions[1]["metadata"]["final_remote_disposition_default"] is True
        assert created_sessions[1]["metadata"]["packet_root_policy"]["policy_strength"] == "authoring_workspace_with_gate_autonomy"
        packet_text = generated_turns[0]["events"][0]["payload"]["text"]
        assert "This run ends at remote land" not in packet_text
        assert "carry it through `ait workflow land`" not in packet_text
        assert "Required remote gate bundle for this run" not in packet_text
        post_worker_run = auto_worker["post_worker_run"]
        assert post_worker_run["advanced"] is True
        assert post_worker_run["execution_state"] == "completed"
        assert post_worker_run["newly_unblocked_node_ids"] == ["C"]
        assert post_worker_run["newly_completed_node_ids"] == ["B", "C"]
        graph_run_session_id = auto_worker["graph_run_session_id"]
        worker_session_id = auto_worker["worker_session_id"]
        assert auto_worker["graph_run_close_out"]["status"] == "completed"
        assert auto_worker["worker_session_close_out"]["status"] == "completed"
        assert sessions_by_id[graph_run_session_id]["status"] == "completed"
        assert sessions_by_id[worker_session_id]["status"] == "completed"
        event_types = [row["event_type"] for row in event_store[graph_run_session_id]]
        assert event_types == [
            "task_graph.execution_started",
            "task_graph.state_snapshot",
            "task_graph.compact_packet_generated",
            "task_graph.node_local_progress",
            "task_graph.compact_worker_started",
            "task_graph.compact_worker_boundary_report",
            "task_graph.execution_advanced",
            "task_graph.state_snapshot",
        ]
        assert event_store[graph_run_session_id][-2]["payload"]["trigger"] == "worker_session_completion"
        assert event_store[graph_run_session_id][-2]["payload"]["worker_session_id"] == auto_worker["worker_session_id"]
        assert event_store[graph_run_session_id][-1]["payload"]["execution_state"] == "completed"


def test_plan_execute_auto_compact_worker_refreshes_graph_run_for_local_final_disposition(monkeypatch, tmp_path):
        with runner.isolated_filesystem(temp_dir=tmp_path):
            _init_repo(monkeypatch)
            monkeypatch.setattr(cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_local"})
            _write_demo_markdown("demo_solo_convergence.md")
            graph = _solo_convergence_graph()
        graph["execution_policy"]["change_strategy"] = "local_first_final_local_land"
        graph_path = _write_graph(graph, filename="demo_solo_local_final_refresh.task_graph.json")

        readiness_calls = {"count": 0}

        def fake_readiness(base_url, current_graph, *, repo_name=None, current_plan_revision_id=None):
            readiness_calls["count"] += 1
            if readiness_calls["count"] == 1:
                return _solo_convergence_initial_readiness(current_graph)
            return _solo_convergence_landed(current_graph)

        remote_tuple = lambda ctx, remote_name: ({"url": "http://example.test"}, "demo")
        monkeypatch.setattr(cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(plan_cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(
            cli_module,
            "_task_dag_readiness_payload",
            lambda ctx, current_graph, remote_name: fake_readiness("http://example.test", current_graph, repo_name="demo"),
        )
        monkeypatch.setattr(
            plan_cli_module,
            "_task_dag_readiness_payload",
            lambda ctx, current_graph, remote_name: fake_readiness("http://example.test", current_graph, repo_name="demo"),
        )

        created_sessions = []
        generated_turns = []
        sessions_by_id = {}
        event_store = {}

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            session_id = f"S-LOCAL-{len(created_sessions) + 1}"
            session = {
                "session_id": session_id,
                "session_kind": session_kind,
                "status": "active",
                "title": kwargs.get("title"),
                "metadata": kwargs.get("metadata") or {},
                "repo_name": repo_name,
            }
            sessions_by_id[session_id] = session
            event_store.setdefault(session_id, [])
            created_sessions.append({"base_url": base_url, "repo_name": repo_name, **session, **kwargs})
            return session

        def fake_append_session_event(base_url, session_id, event_type, payload=None, repo_name=None):
            events = event_store.setdefault(session_id, [])
            event = {
                "sequence": len(events) + 1,
                "event_type": event_type,
                "payload": payload or {},
                "repo_name": repo_name,
            }
            events.append(event)
            return event

        def fake_list_session_events(base_url, session_id, repo_name=None):
            return list(event_store.get(session_id, []))

        def fake_get_session(base_url, session_id, repo_name=None):
            return sessions_by_id[session_id]

        def fake_close_session(base_url, session_id, status="paused", repo_name=None):
            session = dict(sessions_by_id[session_id])
            session["status"] = status
            sessions_by_id[session_id] = session
            return session

        def fake_load_reply_generation_config(*, repo_name, repo_root, env):
            return object()

        def fake_generate_session_reply(_config, *, session, events, chat_id, chat_title, checkpoint, surface, actor_identity=None):
            generated_turns.append(
                {
                    "session": session,
                    "events": events,
                    "chat_id": chat_id,
                    "chat_title": chat_title,
                    "checkpoint": checkpoint,
                    "surface": surface,
                    "actor_identity": actor_identity,
                }
            )
            return SimpleNamespace(
                text="worker completed and landed",
                model="gpt-5.4",
                response_id="resp-local-final",
                usage={"inputTokens": 320, "outputTokens": 80, "totalTokens": 400},
                source="codex",
                turn_analysis={"command_count": 5},
            )

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_session_event)
        monkeypatch.setattr(cli_module, "remote_list_session_events", fake_list_session_events)
        monkeypatch.setattr(cli_module, "remote_get_session", fake_get_session)
        monkeypatch.setattr(cli_module, "remote_close_session", fake_close_session)
        monkeypatch.setattr(cli_module, "load_reply_generation_config", fake_load_reply_generation_config)
        monkeypatch.setattr(cli_module, "generate_session_reply", fake_generate_session_reply)
        telegram_trigger_calls: list[dict[str, Any]] = []

        def fake_trigger_task_dag_execute_run_telegram_notifications(ctx, *, remote_row, repo_name=None, plan_id):
            telegram_trigger_calls.append(
                {
                    "remote_row": remote_row,
                    "repo_name": repo_name,
                    "plan_id": plan_id,
                }
            )
            return {
                "enabled": True,
                "checked": 1,
                "sent": 1,
                "errors": 0,
                "plan_id": plan_id,
                "progress_reader_mode": "local",
                "repo_name": repo_name,
            }

        monkeypatch.setattr(
            cli_module,
            "_trigger_task_dag_execute_run_telegram_notifications",
            fake_trigger_task_dag_execute_run_telegram_notifications,
        )

        local_task_counter = {"value": 0}

        def recording_create_local_task(ctx, title, intent, risk_tier, *, plan_id=None, origin_plan_revision_id=None, plan_item_ref=None):
            local_task_counter["value"] += 1
            return {
                "task_id": f"LT-LOCAL-{local_task_counter['value']}",
                "title": title,
                "intent": intent,
                "risk_tier": risk_tier,
                "plan_id": plan_id,
                "origin_plan_revision_id": origin_plan_revision_id,
                "plan_item_ref": plan_item_ref,
                "status": "active",
            }

        monkeypatch.setattr(task_dag_node_bootstrap_module, "create_local_task", recording_create_local_task)
        monkeypatch.setattr(
            cli_module,
            "remote_create_session_turn",
            lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("auto-compact-worker should not use remote session turn")),
        )

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--auto-compact-worker",
                "--yes",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        contract = payload["execute_run_contract"]
        assert contract["final_remote_disposition_default"] is False
        auto_worker = payload["auto_compact_worker"]
        assert auto_worker["final_remote_disposition_default"] is False
        post_worker_run = auto_worker["post_worker_run"]
        assert post_worker_run["advanced"] is True
        assert post_worker_run["execution_state"] == "completed"
        assert post_worker_run["newly_unblocked_node_ids"] == ["C"]
        assert post_worker_run["newly_completed_node_ids"] == ["B", "C"]
        assert post_worker_run["telegram_graph_watch_notifications"]["sent"] == 1
        assert post_worker_run["telegram_graph_watch_notifications"]["progress_reader_mode"] == "local"
        assert telegram_trigger_calls == [
            {
                "remote_row": {"url": "http://example.test"},
                "repo_name": "demo",
                "plan_id": "PL-demo",
            }
        ]
        graph_run_session_id = auto_worker["graph_run_session_id"]
        worker_session_id = auto_worker["worker_session_id"]
        assert auto_worker["graph_run_close_out"]["status"] == "completed"
        assert auto_worker["worker_session_close_out"]["status"] == "completed"
        assert sessions_by_id[graph_run_session_id]["status"] == "completed"
        assert sessions_by_id[worker_session_id]["status"] == "completed"
        event_types = [row["event_type"] for row in event_store[graph_run_session_id]]
        assert event_types == [
            "task_graph.execution_started",
            "task_graph.state_snapshot",
            "task_graph.compact_packet_generated",
            "task_graph.node_local_progress",
            "task_graph.compact_worker_started",
            "task_graph.compact_worker_boundary_report",
            "task_graph.execution_advanced",
            "task_graph.state_snapshot",
        ]
        assert event_store[graph_run_session_id][-2]["payload"]["trigger"] == "worker_session_completion"
        assert event_store[graph_run_session_id][-2]["payload"]["worker_session_id"] == auto_worker["worker_session_id"]
        assert event_store[graph_run_session_id][-1]["payload"]["execution_state"] == "completed"


def test_plan_execute_auto_compact_worker_continues_next_focus_inside_same_worker_session(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        monkeypatch.setattr(cli_module, "_effective_workflow_mode", lambda ctx: {"value": "solo_local"})
        _write_demo_markdown("demo_solo_convergence.md")
        graph = _solo_convergence_graph()
        graph["execution_policy"]["change_strategy"] = "local_first_final_local_land"
        graph["nodes"][2]["title"] = "Execution-only follow-up node"
        graph["nodes"][2]["workflow_boundary"] = "execution_only"
        graph["nodes"][2]["converged_output"] = False
        graph["nodes"][2]["task_template"] = {"title": "Do C", "risk_tier": "medium"}
        graph["nodes"].append(
            {
                "node_id": "D",
                "node_kind": "task",
                "title": "Converged output node",
                "plan_item_ref": "demo/d",
                "depends_on": ["C"],
                "workflow_boundary": "reviewable_output",
                "converged_output": True,
                "progress_weight": 1,
                "task_template": {"title": "Do D", "risk_tier": "high"},
            }
        )
        graph["edges"].append({"from": "C", "to": "D", "edge_kind": "depends_on"})
        graph_path = _write_graph(graph, filename="demo_solo_local_same_worker.task_graph.json")

        def initial_readiness(current_graph):
            return {
                "schema_version": 1,
                "graph_id": current_graph["graph_id"],
                "source_plan": current_graph["source_plan"],
                "source_plan_revision_id": current_graph["source_plan"]["plan_revision_id"],
                "current_plan_revision_id": current_graph["source_plan"]["plan_revision_id"],
                "stale_source_plan": False,
                "summary": {
                    "total_nodes": 4,
                    "ready_nodes": 1,
                    "running_nodes": 0,
                    "blocked_nodes": 2,
                    "completed_nodes": 1,
                    "next_action": "start B",
                },
                "counts": {"ready": 1, "running": 0, "blocked": 2, "completed": 1, "total": 4},
                "nodes": [
                    {
                        "node_id": "A",
                        "node_kind": "task",
                        "title": "Completed execution-only node",
                        "plan_item_ref": "demo/a",
                        "state": "completed",
                        "reason": "done",
                        "depends_on": [],
                        "lineage": {"task_id": "T-A", "landed_snapshot_id": "SNP-A"},
                        "session_recommendation": {"action": "none"},
                        "blockers": [],
                    },
                    {
                        "node_id": "B",
                        "node_kind": "task",
                        "title": "Execution-only node",
                        "plan_item_ref": "demo/b",
                        "state": "ready",
                        "reason": "ready",
                        "depends_on": ["A"],
                        "lineage": {},
                        "session_recommendation": {"action": "open_new_session"},
                        "blockers": [],
                    },
                    {
                        "node_id": "C",
                        "node_kind": "task",
                        "title": "Execution-only follow-up node",
                        "plan_item_ref": "demo/c",
                        "state": "blocked",
                        "reason": "Dependency B is ready, not completed.",
                        "depends_on": ["B"],
                        "lineage": {},
                        "session_recommendation": {"action": "unblock_before_session"},
                        "blockers": [
                            {
                                "type": "dependency",
                                "code": "dependency_incomplete",
                                "node_id": "B",
                                "state": "ready",
                                "message": "Dependency B is ready, not completed.",
                            }
                        ],
                    },
                    {
                        "node_id": "D",
                        "node_kind": "task",
                        "title": "Converged output node",
                        "plan_item_ref": "demo/d",
                        "state": "blocked",
                        "reason": "Dependency C is blocked, not completed.",
                        "depends_on": ["C"],
                        "lineage": {},
                        "session_recommendation": {"action": "unblock_before_session"},
                        "blockers": [
                            {
                                "type": "dependency",
                                "code": "dependency_incomplete",
                                "node_id": "C",
                                "state": "blocked",
                                "message": "Dependency C is blocked, not completed.",
                            }
                        ],
                    },
                ],
            }

        def mid_readiness(current_graph):
            return {
                **_solo_convergence_initial_readiness(current_graph),
                "summary": {
                    "total_nodes": 4,
                    "ready_nodes": 1,
                    "running_nodes": 0,
                    "blocked_nodes": 1,
                    "completed_nodes": 2,
                    "next_action": "start C",
                },
                "counts": {"ready": 1, "running": 0, "blocked": 1, "completed": 2, "total": 4},
                "nodes": [
                    {
                        "node_id": "A",
                        "node_kind": "task",
                        "title": "Completed execution-only node",
                        "plan_item_ref": "demo/a",
                        "state": "completed",
                        "reason": "done",
                        "depends_on": [],
                        "lineage": {"task_id": "T-A", "landed_snapshot_id": "SNP-A"},
                        "session_recommendation": {"action": "none"},
                        "blockers": [],
                    },
                    {
                        "node_id": "B",
                        "node_kind": "task",
                        "title": "Execution-only node",
                        "plan_item_ref": "demo/b",
                        "state": "completed",
                        "reason": "done",
                        "depends_on": ["A"],
                        "lineage": {"task_id": "T-B"},
                        "session_recommendation": {"action": "none"},
                        "blockers": [],
                    },
                    {
                        "node_id": "C",
                        "node_kind": "task",
                        "title": "Converged output node",
                        "plan_item_ref": "demo/c",
                        "state": "ready",
                        "reason": "ready",
                        "depends_on": ["B"],
                        "lineage": {},
                        "session_recommendation": {"action": "open_new_session"},
                        "blockers": [],
                    },
                    {
                        "node_id": "D",
                        "node_kind": "task",
                        "title": "Converged output node",
                        "plan_item_ref": "demo/d",
                        "state": "blocked",
                        "reason": "Dependency C is ready, not completed.",
                        "depends_on": ["C"],
                        "lineage": {},
                        "session_recommendation": {"action": "unblock_before_session"},
                        "blockers": [
                            {
                                "type": "dependency",
                                "code": "dependency_incomplete",
                                "node_id": "C",
                                "state": "ready",
                                "message": "Dependency C is ready, not completed.",
                            }
                        ],
                    },
                ],
            }

        def final_readiness(current_graph):
            return {
                "schema_version": 1,
                "graph_id": current_graph["graph_id"],
                "source_plan": current_graph["source_plan"],
                "source_plan_revision_id": current_graph["source_plan"]["plan_revision_id"],
                "current_plan_revision_id": current_graph["source_plan"]["plan_revision_id"],
                "stale_source_plan": False,
                "summary": {
                    "total_nodes": 4,
                    "ready_nodes": 0,
                    "running_nodes": 0,
                    "blocked_nodes": 0,
                    "completed_nodes": 4,
                    "next_action": "completed",
                },
                "nodes": [
                    {
                        "node_id": "A",
                        "node_kind": "task",
                        "title": "Completed execution-only node",
                        "plan_item_ref": "demo/a",
                        "state": "completed",
                        "reason": "done",
                        "depends_on": [],
                        "lineage": {"task_id": "T-A", "landed_snapshot_id": "SNP-A"},
                        "session_recommendation": {"action": "none"},
                        "blockers": [],
                    },
                    {
                        "node_id": "B",
                        "node_kind": "task",
                        "title": "Execution-only node",
                        "plan_item_ref": "demo/b",
                        "state": "completed",
                        "reason": "done",
                        "depends_on": ["A"],
                        "lineage": {"task_id": "T-B"},
                        "session_recommendation": {"action": "none"},
                        "blockers": [],
                    },
                    {
                        "node_id": "C",
                        "node_kind": "task",
                        "title": "Execution-only follow-up node",
                        "plan_item_ref": "demo/c",
                        "state": "completed",
                        "reason": "done",
                        "depends_on": ["B"],
                        "lineage": {"task_id": "T-C"},
                        "session_recommendation": {"action": "none"},
                        "blockers": [],
                    },
                    {
                        "node_id": "D",
                        "node_kind": "task",
                        "title": "Converged output node",
                        "plan_item_ref": "demo/d",
                        "state": "completed",
                        "reason": "done",
                        "depends_on": ["C"],
                        "lineage": {"task_id": "T-D"},
                        "session_recommendation": {"action": "none"},
                        "blockers": [],
                    },
                ],
            }

        reply_counter = {"count": 0}

        def fake_readiness(base_url, current_graph, *, repo_name=None, current_plan_revision_id=None):
            if reply_counter["count"] <= 0:
                return initial_readiness(current_graph)
            if reply_counter["count"] == 1:
                return mid_readiness(current_graph)
            return final_readiness(current_graph)

        remote_tuple = lambda ctx, remote_name: ({"url": "http://example.test"}, "demo")
        monkeypatch.setattr(cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(plan_cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(
            cli_module,
            "_task_dag_readiness_payload",
            lambda ctx, current_graph, remote_name: fake_readiness("http://example.test", current_graph, repo_name="demo"),
        )
        monkeypatch.setattr(
            plan_cli_module,
            "_task_dag_readiness_payload",
            lambda ctx, current_graph, remote_name: fake_readiness("http://example.test", current_graph, repo_name="demo"),
        )

        created_sessions = []
        generated_turns = []
        sessions_by_id = {}
        event_store = {}

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            session_id = f"S-SAME-{len(created_sessions) + 1}"
            session = {
                "session_id": session_id,
                "session_kind": session_kind,
                "status": "active",
                "title": kwargs.get("title"),
                "metadata": kwargs.get("metadata") or {},
                "repo_name": repo_name,
            }
            sessions_by_id[session_id] = session
            event_store.setdefault(session_id, [])
            created_sessions.append({"base_url": base_url, "repo_name": repo_name, **session, **kwargs})
            return session

        def fake_append_session_event(base_url, session_id, event_type, payload=None, repo_name=None):
            events = event_store.setdefault(session_id, [])
            event = {
                "sequence": len(events) + 1,
                "event_type": event_type,
                "payload": payload or {},
                "repo_name": repo_name,
            }
            events.append(event)
            return event

        def fake_list_session_events(base_url, session_id, repo_name=None):
            return list(event_store.get(session_id, []))

        def fake_get_session(base_url, session_id, repo_name=None):
            return sessions_by_id[session_id]

        def fake_close_session(base_url, session_id, status="paused", repo_name=None):
            session = dict(sessions_by_id[session_id])
            session["status"] = status
            sessions_by_id[session_id] = session
            return session

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_session_event)
        monkeypatch.setattr(cli_module, "remote_list_session_events", fake_list_session_events)
        monkeypatch.setattr(cli_module, "remote_get_session", fake_get_session)
        monkeypatch.setattr(cli_module, "remote_close_session", fake_close_session)
        def fake_load_reply_generation_config(*, repo_name, repo_root, env):
            return ReplyGenerationConfig(
                repo_name=repo_name,
                openai_api_key=None,
                openai_base_url="https://example.test/v1",
                openai_model="gpt-5.4-mini",
                openai_reasoning_effort=None,
                openai_timeout_seconds=20.0,
                openai_max_output_tokens=1000,
                history_limit=8,
                telegram_checkpoint_event_threshold=6,
                telegram_checkpoint_summary_event_limit=8,
                repo_root=Path(repo_root),
                codex_persistent_client=True,
            )

        monkeypatch.setattr(cli_module, "load_reply_generation_config", fake_load_reply_generation_config)

        def fake_generate_session_reply(config, *, session, events, chat_id, chat_title, checkpoint, surface, actor_identity=None):
            reply_counter["count"] += 1
            generated_turns.append(
                {
                    "repo_root": str(config.repo_root),
                    "codex_persistent_client": config.codex_persistent_client,
                    "session": session,
                    "events": events,
                    "chat_id": chat_id,
                    "chat_title": chat_title,
                    "checkpoint": checkpoint,
                    "surface": surface,
                    "actor_identity": actor_identity,
                }
            )
            node_id = "B" if reply_counter["count"] == 1 else "C"
            return SimpleNamespace(
                text=(
                    f"compact worker completed {node_id}\n"
                    f'task_dag_local_progress={{"node_id":"{node_id}","status":"completed","summary":"{node_id} done"}}'
                ),
                model="gpt-5.4",
                response_id=f"resp-{node_id.lower()}",
                usage={"inputTokens": 90, "outputTokens": 25, "totalTokens": 115},
                source="codex",
                turn_analysis={"command_count": 1},
            )

        monkeypatch.setattr(cli_module, "generate_session_reply", fake_generate_session_reply)

        local_task_counter = {"value": 0}

        def fake_create_local_task(ctx, title, intent, risk_tier, *, plan_id=None, origin_plan_revision_id=None, plan_item_ref=None):
            local_task_counter["value"] += 1
            return {
                "task_id": f"LT-LOCAL-{local_task_counter['value']}",
                "title": title,
                "intent": intent,
                "risk_tier": risk_tier,
                "plan_id": plan_id,
                "origin_plan_revision_id": origin_plan_revision_id,
                "plan_item_ref": plan_item_ref,
                "status": "active",
            }

        monkeypatch.setattr(task_dag_node_bootstrap_module, "create_local_task", fake_create_local_task)
        monkeypatch.setattr(
            task_dag_compact_worker_runtime_module,
            "_task_dag_compact_worker_completion_snapshot_evidence",
            lambda *args, **kwargs: {
                "completion_snapshot_id": f"SNP-{kwargs.get('node_id')}",
                "completion_fork_snapshot_id": f"SNP-base-{kwargs.get('node_id')}",
            },
        )

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--auto-compact-worker",
                "--yes",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        auto_worker = payload["auto_compact_worker"]
        assert len(created_sessions) == 2
        assert created_sessions[0]["session_kind"] == "task_graph_run"
        assert created_sessions[1]["session_kind"] == "agent_run"
        assert len(generated_turns) == 2
        assert generated_turns[0]["repo_root"] != generated_turns[1]["repo_root"]
        assert generated_turns[0]["codex_persistent_client"] is False
        assert generated_turns[1]["codex_persistent_client"] is False
        assert generated_turns[0]["session"]["session_id"] == created_sessions[1]["session_id"]
        assert generated_turns[1]["session"]["session_id"] == created_sessions[1]["session_id"]
        first_turn_text = generated_turns[0]["events"][0]["payload"]["text"]
        second_turn_text = generated_turns[1]["events"][0]["payload"]["text"]
        assert "Current focus context:" in first_turn_text
        assert "Current focus context:" in second_turn_text
        assert "- node B · Execution-only node" in first_turn_text
        assert "- node C · Execution-only follow-up node" in second_turn_text
        assert "- node C · Execution-only follow-up node" not in first_turn_text
        assert "- node B · Execution-only node" not in second_turn_text
        assert auto_worker["continuation_count"] == 1
        assert auto_worker["continued_focus_node_ids"] == ["C"]
        graph_run_session_id = auto_worker["graph_run_session_id"]
        event_types = [row["event_type"] for row in event_store[graph_run_session_id]]
        assert event_types == [
            "task_graph.execution_started",
            "task_graph.state_snapshot",
            "task_graph.compact_packet_generated",
            "task_graph.node_local_progress",
            "task_graph.node_completed",
            "task_graph.compact_worker_started",
            "task_graph.compact_worker_boundary_report",
            "task_graph.execution_advanced",
            "task_graph.state_snapshot",
            "task_graph.compact_packet_generated",
            "task_graph.node_local_progress",
            "task_graph.node_completed",
            "task_graph.compact_worker_started",
            "task_graph.compact_worker_boundary_report",
            "task_graph.execution_advanced",
            "task_graph.state_snapshot",
        ]
        assert event_store[graph_run_session_id][9]["payload"]["next_focus_node_id"] == "C"
        assert event_store[graph_run_session_id][4]["payload"]["completion_snapshot_id"] == "SNP-B"
        assert event_store[graph_run_session_id][4]["payload"]["completion_fork_snapshot_id"] == "SNP-base-B"
        assert event_store[graph_run_session_id][11]["payload"]["completion_snapshot_id"] == "SNP-C"
        assert event_store[graph_run_session_id][11]["payload"]["completion_fork_snapshot_id"] == "SNP-base-C"
        assert event_store[graph_run_session_id][-1]["payload"]["completed_node_ids"] == ["A", "B", "C", "D"]


def test_plan_execute_auto_compact_worker_records_graph_run_failure_when_continuation_bootstrap_raises(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        _write_demo_markdown("demo_solo_convergence.md")
        graph = _solo_convergence_graph()
        graph_path = _write_graph(graph, filename="demo_solo_remote_failure.task_graph.json")

        def readiness_after_b(current_graph):
            return {
                "schema_version": 1,
                "graph_id": current_graph["graph_id"],
                "source_plan": current_graph["source_plan"],
                "source_plan_revision_id": current_graph["source_plan"]["plan_revision_id"],
                "current_plan_revision_id": current_graph["source_plan"]["plan_revision_id"],
                "stale_source_plan": False,
                "summary": {
                    "total_nodes": 3,
                    "ready_nodes": 1,
                    "running_nodes": 0,
                    "blocked_nodes": 0,
                    "completed_nodes": 2,
                    "next_action": "start C",
                },
                "counts": {"ready": 1, "running": 0, "blocked": 0, "completed": 2, "total": 3},
                "nodes": [
                    {
                        "node_id": "A",
                        "node_kind": "task",
                        "title": "Completed execution-only node",
                        "plan_item_ref": "demo/a",
                        "state": "completed",
                        "reason": "done",
                        "depends_on": [],
                        "lineage": {
                            "task_id": "T-A",
                            "landed_snapshot_id": "SNP-A",
                            "completion_snapshot_id": "SNP-A",
                            "completion_fork_snapshot_id": "SNP-base-A",
                        },
                        "session_recommendation": {"action": "none"},
                        "blockers": [],
                    },
                    {
                        "node_id": "B",
                        "node_kind": "task",
                        "title": "Execution-only node",
                        "plan_item_ref": "demo/b",
                        "state": "completed",
                        "reason": "done",
                        "depends_on": ["A"],
                        "lineage": {"task_id": "T-B"},
                        "session_recommendation": {"action": "none"},
                        "blockers": [],
                    },
                    {
                        "node_id": "C",
                        "node_kind": "task",
                        "title": "Converged output node",
                        "plan_item_ref": "demo/c",
                        "state": "ready",
                        "reason": "ready",
                        "depends_on": ["B"],
                        "lineage": {},
                        "session_recommendation": {"action": "open_new_session"},
                        "blockers": [],
                    },
                ],
            }

        reply_counter = {"count": 0}

        def fake_readiness(base_url, current_graph, *, repo_name=None, current_plan_revision_id=None):
            if reply_counter["count"] <= 0:
                return _solo_convergence_initial_readiness(current_graph)
            return readiness_after_b(current_graph)

        remote_tuple = lambda ctx, remote_name: ({"url": "http://example.test"}, "demo")
        monkeypatch.setattr(cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(plan_cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(
            cli_module,
            "_task_dag_readiness_payload",
            lambda ctx, current_graph, remote_name: fake_readiness("http://example.test", current_graph, repo_name="demo"),
        )
        monkeypatch.setattr(
            plan_cli_module,
            "_task_dag_readiness_payload",
            lambda ctx, current_graph, remote_name: fake_readiness("http://example.test", current_graph, repo_name="demo"),
        )

        created_sessions = []
        sessions_by_id = {}
        event_store = {}

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            session_id = f"S-FAIL-{len(created_sessions) + 1}"
            session = {
                "session_id": session_id,
                "session_kind": session_kind,
                "status": "active",
                "title": kwargs.get("title"),
                "metadata": kwargs.get("metadata") or {},
                "repo_name": repo_name,
            }
            sessions_by_id[session_id] = session
            event_store.setdefault(session_id, [])
            created_sessions.append({"base_url": base_url, "repo_name": repo_name, **session, **kwargs})
            return session

        def fake_append_session_event(base_url, session_id, event_type, payload=None, repo_name=None):
            events = event_store.setdefault(session_id, [])
            event = {
                "sequence": len(events) + 1,
                "event_type": event_type,
                "payload": payload or {},
                "repo_name": repo_name,
            }
            events.append(event)
            return event

        def fake_list_session_events(base_url, session_id, repo_name=None):
            return list(event_store.get(session_id, []))

        def fake_get_session(base_url, session_id, repo_name=None):
            return sessions_by_id[session_id]

        def fake_close_session(base_url, session_id, status="paused", repo_name=None):
            session = dict(sessions_by_id[session_id])
            session["status"] = status
            sessions_by_id[session_id] = session
            return session

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_session_event)
        monkeypatch.setattr(cli_module, "remote_list_session_events", fake_list_session_events)
        monkeypatch.setattr(cli_module, "remote_get_session", fake_get_session)
        monkeypatch.setattr(cli_module, "remote_close_session", fake_close_session)

        def fake_load_reply_generation_config(*, repo_name, repo_root, env):
            return ReplyGenerationConfig(
                repo_name=repo_name,
                openai_api_key=None,
                openai_base_url="https://example.test/v1",
                openai_model="gpt-5.4-mini",
                openai_reasoning_effort=None,
                openai_timeout_seconds=20.0,
                openai_max_output_tokens=1000,
                history_limit=8,
                telegram_checkpoint_event_threshold=6,
                telegram_checkpoint_summary_event_limit=8,
                repo_root=Path(repo_root),
                codex_persistent_client=True,
            )

        monkeypatch.setattr(cli_module, "load_reply_generation_config", fake_load_reply_generation_config)

        def fake_generate_session_reply(config, *, session, events, chat_id, chat_title, checkpoint, surface, actor_identity=None):
            reply_counter["count"] += 1
            assert reply_counter["count"] == 1
            return SimpleNamespace(
                text='compact worker completed B\ntask_dag_local_progress={"node_id":"B","status":"completed","summary":"B done"}',
                model="gpt-5.4",
                response_id="resp-b",
                usage={"inputTokens": 90, "outputTokens": 25, "totalTokens": 115},
                source="codex",
                turn_analysis={"command_count": 1},
            )

        monkeypatch.setattr(cli_module, "generate_session_reply", fake_generate_session_reply)

        local_task_counter = {"value": 0}

        def fake_create_local_task(ctx, title, intent, risk_tier, *, plan_id=None, origin_plan_revision_id=None, plan_item_ref=None):
            local_task_counter["value"] += 1
            return {
                "task_id": f"LT-LOCAL-{local_task_counter['value']}",
                "title": title,
                "intent": intent,
                "risk_tier": risk_tier,
                "plan_id": plan_id,
                "origin_plan_revision_id": origin_plan_revision_id,
                "plan_item_ref": plan_item_ref,
                "status": "active",
            }

        monkeypatch.setattr(task_dag_node_bootstrap_module, "create_local_task", fake_create_local_task)
        monkeypatch.setattr(
            task_dag_node_bootstrap_module,
            "remote_create_task",
            lambda *args, **kwargs: (_ for _ in ()).throw(ValueError("demo reviewable bootstrap failed")),
        )
        monkeypatch.setattr(
            task_dag_compact_worker_runtime_module,
            "_task_dag_compact_worker_completion_snapshot_evidence",
            lambda *args, **kwargs: {
                "completion_snapshot_id": "SNP-B",
                "completion_fork_snapshot_id": "SNP-base-B",
            },
        )

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--auto-compact-worker",
                "--yes",
                "--json",
            ],
        )

        assert result.exit_code != 0
        assert "demo reviewable bootstrap failed" in result.output
        graph_run_session_id = created_sessions[0]["session_id"]
        worker_session_id = created_sessions[1]["session_id"]
        event_types = [row["event_type"] for row in event_store[graph_run_session_id]]
        assert event_types == [
            "task_graph.execution_started",
            "task_graph.state_snapshot",
            "task_graph.compact_packet_generated",
            "task_graph.node_local_progress",
            "task_graph.node_completed",
            "task_graph.compact_worker_started",
            "task_graph.compact_worker_boundary_report",
            "task_graph.execution_advanced",
            "task_graph.state_snapshot",
            "task_graph.execution_failed",
            "task_graph.state_snapshot",
        ]
        failure_event = event_store[graph_run_session_id][-2]["payload"]
        assert failure_event["failure_reason"] == "auto_compact_worker_exception"
        assert failure_event["worker_session_id"] == worker_session_id
        assert "demo reviewable bootstrap failed" in failure_event["failure_detail"]
        failed_snapshot = event_store[graph_run_session_id][-1]["payload"]
        assert failed_snapshot["execution_state"] == "failed"
        assert failed_snapshot["next_action"] == "operator retry after compact worker failure"
        assert failed_snapshot["failure_reason"] == "auto_compact_worker_exception"
        assert failed_snapshot["failed_worker_session_id"] == worker_session_id
        assert failed_snapshot["ready_node_ids"] == ["C"]
        assert sessions_by_id[worker_session_id]["status"] == "failed"
        assert sessions_by_id[graph_run_session_id]["status"] == "failed"


def test_plan_execute_auto_compact_worker_packages_comparison_evidence(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph = _graph()
        graph["comparison_evidence_workload_id"] = "demo_workload"
        graph_path = _write_graph(graph, filename="demo_comparison.task_graph.json")
        _write_demo_markdown()
        report_path = _write_comparison_report()
        _stub_remote(monkeypatch)

        created_sessions = []
        generated_turns = []

        def fake_create_session(base_url, repo_name, session_kind, **kwargs):
            session_id = f"S-CMP-{len(created_sessions) + 1}"
            created_sessions.append(
                {
                    "base_url": base_url,
                    "repo_name": repo_name,
                    "session_kind": session_kind,
                    "session_id": session_id,
                    **kwargs,
                }
            )
            return {
                "session_id": session_id,
                "session_kind": session_kind,
                "title": kwargs.get("title"),
            }

        def fake_load_reply_generation_config(*, repo_name, repo_root, env):
            return object()

        def fake_generate_session_reply(_config, *, session, events, chat_id, chat_title, checkpoint, surface, actor_identity=None):
            generated_turns.append(
                {
                    "session": session,
                    "events": events,
                    "chat_id": chat_id,
                    "chat_title": chat_title,
                    "checkpoint": checkpoint,
                    "surface": surface,
                    "actor_identity": actor_identity,
                }
            )
            return SimpleNamespace(
                text="comparison completed",
                model="gpt-5.4",
                response_id="resp-compare",
                usage={"inputTokens": 222, "outputTokens": 33, "totalTokens": 255},
                source="codex",
                turn_analysis={"command_count": 0},
            )

        monkeypatch.setattr(cli_module, "remote_create_session", fake_create_session)
        monkeypatch.setattr(
            cli_module,
            "remote_append_session_event",
            lambda *_args, **_kwargs: {
                "event_id": "EV-CMP",
                "event_type": _args[2] if len(_args) > 2 else "",
                "sequence": 1 if (_args[2] if len(_args) > 2 else "") == "session.message" else 2,
            },
        )
        monkeypatch.setattr(cli_module, "load_reply_generation_config", fake_load_reply_generation_config)
        monkeypatch.setattr(cli_module, "generate_session_reply", fake_generate_session_reply)
        monkeypatch.setattr(
            cli_module,
            "remote_create_session_turn",
            lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("auto-compact-worker should not use remote session turn")),
        )

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--auto-compact-worker",
                "--comparison-evidence-report",
                str(report_path),
                "--yes",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        auto_worker = payload["auto_compact_worker"]
        assert auto_worker["execution_mode"] == "benchmark"
        assert (
            auto_worker["reply_poll_timeout_seconds"]
            == cli_module.DEFAULT_TASK_DAG_COMPACT_PACKET_REPLY_POLL_TIMEOUT_SECONDS
        )
        assert auto_worker["comparison_inputs_packaged"] is True
        assert Path(auto_worker["comparison_evidence_artifact_path"]).exists()
        assert created_sessions[1]["metadata"]["workspace_root"].endswith("packet_root")
        assert created_sessions[1]["metadata"]["repo_root"].endswith("packet_root")
        packet_root = Path(auto_worker["packet_root_path"])
        manifest = json.loads(Path(auto_worker["packet_root_manifest_path"]).read_text(encoding="utf-8"))
        assert manifest["execution_mode"] == "benchmark"
        assert manifest["comparison_evidence_file"] == "comparison_evidence.json"
        assert "comparison_evidence.json" in manifest["secondary_context_files"]
        assert "compact_packet.md" not in manifest["secondary_context_files"]
        assert "comparison_inputs_packaged" not in manifest
        assert "current_focus" not in manifest
        assert not (packet_root / "compact_packet.md").exists()
        assert (packet_root / "comparison_evidence.json").exists()
        assert len(generated_turns) == 1
        packet_text = generated_turns[0]["events"][0]["payload"]["text"]
        assert "Packaged comparison evidence:" in packet_text
        assert "80%+ packet savings" in packet_text
        assert "- git_linear: total_tokens=111" in packet_text
        assert "- ait_linear: total_tokens=99" in packet_text
        assert "- ait_dag: total_tokens=44" in packet_text
        assert "reply exactly with" not in packet_text


def test_plan_execute_can_inspect_recorded_run_session(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        remote_tuple = lambda ctx, remote_name: ({"url": "http://example.test"}, "demo")
        monkeypatch.setattr(cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(plan_cli_module, "_remote_tuple", remote_tuple)

        remote_get_session_calls = []
        monkeypatch.setattr(
            cli_module,
            "remote_get_session",
            lambda base_url, session_id, repo_name=None: remote_get_session_calls.append(
                {"base_url": base_url, "session_id": session_id, "repo_name": repo_name}
            )
            or {
                "session_id": session_id,
                "session_kind": "task_graph_run",
                "status": "active",
                "title": "Task DAG execute: demo/task-dag",
                "created_at": "2026-04-26T00:00:00+00:00",
                "updated_at": "2026-04-26T00:00:01+00:00",
                "metadata": {
                    "session_policy": "task_dag_execute_run",
                    "graph_run_id": "graph-run-abc123",
                    "execution_state": "active",
                    "plan_id": "PL-demo",
                    "graph_id": "demo/task-dag",
                },
            },
        )
        monkeypatch.setattr(
            cli_module,
            "remote_list_session_events",
            lambda base_url, session_id, repo_name=None: [
                {
                    "sequence": 1,
                    "event_type": "task_graph.execution_started",
                    "payload": {"workflow_summary": {"next_action": "start B"}},
                },
                {
                    "sequence": 2,
                    "event_type": "task_graph.state_snapshot",
                    "payload": {
                        "execution_state": "active",
                        "workflow_summary": {"ready_nodes": 1, "blocked_nodes": 0, "next_action": "start B"},
                    },
                },
            ],
        )

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--run-session",
                "S-EXEC-1",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        assert payload["mode"] == "inspect_run"
        recorded = payload["recorded_run"]
        assert recorded["session_id"] == "S-EXEC-1"
        assert recorded["graph_run_id"] == "graph-run-abc123"
        assert recorded["execution_state"] == "active"
        assert recorded["event_count"] == 2
        assert recorded["latest_event_type"] == "task_graph.state_snapshot"
        assert recorded["latest_state_snapshot"]["workflow_summary"]["next_action"] == "start B"
        assert {"base_url": "http://example.test", "session_id": "S-EXEC-1", "repo_name": "demo"} in remote_get_session_calls


def test_plan_execute_can_pause_recorded_run_for_gate(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        readiness = _readiness_after_dependency_completion(_graph())
        _stub_remote(monkeypatch, readiness_factory=lambda graph: readiness)
        remote_get_session_calls = []

        monkeypatch.setattr(
            cli_module,
            "remote_get_session",
            lambda base_url, session_id, repo_name=None: remote_get_session_calls.append(
                {"base_url": base_url, "session_id": session_id, "repo_name": repo_name}
            )
            or {
                "session_id": session_id,
                "session_kind": "task_graph_run",
                "status": "active",
                "title": "Task DAG execute: demo/task-dag",
                "metadata": {
                    "session_policy": "task_dag_execute_run",
                    "graph_run_id": "graph-run-abc123",
                    "plan_id": "PL-demo",
                    "graph_id": "demo/task-dag",
                    "execution_state": "active",
                },
            },
        )

        active_snapshot = {
            "graph_run_id": "graph-run-abc123",
            "execution_state": "active",
            "plan_id": "PL-demo",
            "plan_revision_id": "PR-demo",
            "graph_id": "demo/task-dag",
            "graph_artifact_path": "docs/sprints/demo.task_graph.json",
            "workflow_summary": {
                "ready_nodes": 0,
                "running_nodes": 0,
                "blocked_nodes": 0,
                "completed_nodes": 2,
                "dispatched_nodes": 1,
                "total_nodes": 3,
                "next_action": "start C",
            },
            "readiness_summary": readiness["summary"],
            "ready_node_ids": ["C"],
            "running_node_ids": [],
            "blocked_node_ids": [],
            "completed_node_ids": ["A", "B"],
            "dispatched_node_ids": ["C"],
            "next_action": "start C",
        }
        active_snapshot["readiness_digest"] = cli_module._task_dag_execute_state_digest(active_snapshot)
        monkeypatch.setattr(
            cli_module,
            "remote_list_session_events",
            lambda base_url, session_id, repo_name=None: [
                {"sequence": 1, "event_type": "task_graph.execution_started", "payload": {"workflow_summary": {"next_action": "start C"}}},
                {"sequence": 2, "event_type": "task_graph.state_snapshot", "payload": active_snapshot},
            ],
        )

        appended = []

        def fake_append_event(base_url, session_id, event_type, payload=None, repo_name=None):
            appended.append({"event_type": event_type, "repo_name": repo_name, "payload": payload or {}})
            return {"event_id": f"EV-{len(appended)}", "event_type": event_type, "payload": payload or {}}

        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_event)

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--pause-run",
                "review",
                "--run-session",
                "S-EXEC-1",
                "--yes",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        controlled = payload["controlled_run"]
        assert payload["mode"] == "pause_run"
        assert controlled["controlled"] is True
        assert controlled["operator_action"] == "pause"
        assert controlled["pause_reason"] == "review"
        assert controlled["execution_state"] == "waiting_for_review"
        assert appended[0]["event_type"] == "task_graph.execution_paused"
        assert appended[0]["payload"]["pause_reason"] == "review"
        assert appended[1]["event_type"] == "task_graph.state_snapshot"
        assert appended[1]["payload"]["execution_state"] == "waiting_for_review"
        assert appended[1]["payload"]["next_action"] == "record review approval"
        assert {"base_url": "http://example.test", "session_id": "S-EXEC-1", "repo_name": "demo"} in remote_get_session_calls
        assert appended[0]["repo_name"] == "demo"


def test_plan_execute_can_abort_recorded_run(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        readiness = _readiness_after_dependency_completion(_graph())
        _stub_remote(monkeypatch, readiness_factory=lambda graph: readiness)
        remote_get_session_calls = []

        monkeypatch.setattr(
            cli_module,
            "remote_get_session",
            lambda base_url, session_id, repo_name=None: remote_get_session_calls.append(
                {"base_url": base_url, "session_id": session_id, "repo_name": repo_name}
            )
            or {
                "session_id": session_id,
                "session_kind": "task_graph_run",
                "status": "active",
                "title": "Task DAG execute: demo/task-dag",
                "metadata": {
                    "session_policy": "task_dag_execute_run",
                    "graph_run_id": "graph-run-abc123",
                    "plan_id": "PL-demo",
                    "graph_id": "demo/task-dag",
                    "execution_state": "active",
                },
            },
        )
        monkeypatch.setattr(
            cli_module,
            "remote_list_session_events",
            lambda base_url, session_id, repo_name=None: [
                {"sequence": 1, "event_type": "task_graph.execution_started", "payload": {"workflow_summary": {"next_action": "start C"}}},
            ],
        )

        appended = []

        def fake_append_event(base_url, session_id, event_type, payload=None, repo_name=None):
            appended.append({"event_type": event_type, "repo_name": repo_name, "payload": payload or {}})
            return {"event_id": f"EV-{len(appended)}", "event_type": event_type, "payload": payload or {}}

        monkeypatch.setattr(cli_module, "remote_append_session_event", fake_append_event)

        result = runner.invoke(
            app,
            [
                "plan",
                "execute",
                "PL-demo",
                "--from-json",
                str(graph_path),
                "--abort-run",
                "--run-session",
                "S-EXEC-1",
                "--yes",
                "--json",
            ],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        controlled = payload["controlled_run"]
        assert payload["mode"] == "abort_run"
        assert controlled["controlled"] is True
        assert controlled["operator_action"] == "abort"
        assert controlled["execution_state"] == "aborted"
        assert appended[0]["event_type"] == "task_graph.execution_aborted"
        assert appended[1]["payload"]["execution_state"] == "aborted"
        assert appended[1]["payload"]["next_action"] == "aborted by operator"
        assert {"base_url": "http://example.test", "session_id": "S-EXEC-1", "repo_name": "demo"} in remote_get_session_calls
        assert appended[0]["repo_name"] == "demo"


def test_plan_execute_rejects_removed_record_run_alias(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--record-run", "--json"],
            catch_exceptions=False,
        )

        assert result.exit_code != 0
        assert "No such option: --record-run" in result.output


def test_plan_execute_rejects_removed_retry_run_operator_alias(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--retry-run", "--json"],
            catch_exceptions=False,
        )

        assert result.exit_code != 0
        assert "No such option: --retry-run" in result.output


def test_plan_execute_rejects_removed_compact_packet_run_alias(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--compact-packet-run", "--json"],
            catch_exceptions=False,
        )

        assert result.exit_code != 0
        assert "No such option: --compact-packet-run" in result.output


def test_plan_execute_rejects_removed_compact_packet_surface_alias(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--compact-packet-surface", "--json"],
            catch_exceptions=False,
        )

        assert result.exit_code != 0
        assert "No such option: --compact-packet-surface" in result.output


def test_plan_execute_auto_compact_worker_requires_explicit_confirmation(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch)

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--auto-compact-worker", "--json"],
        )

        assert result.exit_code != 0
        assert "requires --yes" in result.output


def test_plan_execute_auto_compact_worker_blocks_stale_graph_without_override(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch, stale=True)
        created_sessions = []
        monkeypatch.setattr(cli_module, "remote_create_session", lambda *args, **kwargs: created_sessions.append(kwargs))

        result = runner.invoke(
            app,
            ["plan", "execute", "PL-demo", "--from-json", str(graph_path), "--auto-compact-worker", "--yes", "--json"],
        )

        assert result.exit_code != 0
        assert "source plan revision is stale" in result.output
        assert created_sessions == []


def test_plan_task_only_dispatch_is_rendered_as_dispatched_planning(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        remote_tuple = lambda ctx, remote_name: ({"url": "http://example.test"}, "demo")
        monkeypatch.setattr(cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(plan_cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(cli_module, "remote_read_task_dag_readiness", lambda base_url, graph: _dispatched_readiness(graph))
        created = []
        monkeypatch.setattr(cli_module, "remote_create_task", lambda *args, **kwargs: created.append(kwargs))

        graph_out = runner.invoke(
            app,
            ["plan", "graph", "PL-demo", "--from-json", str(graph_path), "--json"],
            catch_exceptions=False,
        )
        graph_payload = json.loads(graph_out.output)
        assert graph_payload["nodes"][1]["state"] == "ready"
        assert graph_payload["nodes"][1]["workflow_state"] == "dispatched"
        assert graph_payload["workflow_summary"]["dispatched_nodes"] == 1
        assert graph_payload["workflow_summary"]["ready_nodes"] == 0

        schedule_out = runner.invoke(
            app,
            ["plan", "schedule", "PL-demo", "--from-json", str(graph_path), "--json"],
            catch_exceptions=False,
        )
        schedule_payload = json.loads(schedule_out.output)
        assert schedule_payload["ready"] == []
        assert schedule_payload["dispatched"][0]["node_id"] == "B"
        assert schedule_payload["dispatched"][0]["workflow_state"] == "dispatched"

        progress_out = runner.invoke(
            app,
            ["plan", "progress", "PL-demo", "--from-json", str(graph_path), "--json"],
            catch_exceptions=False,
        )
        progress_payload = json.loads(progress_out.output)
        assert progress_payload["progress"]["completed_percent"] == 50
        assert progress_payload["progress"]["ready_nodes"] == 1
        assert progress_payload["progress"]["running_nodes"] == 0

        assert created == []


def test_plan_completed_task_only_node_counts_as_completed_not_dispatched(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        _stub_remote(monkeypatch, readiness_factory=_completed_task_only_readiness)

        graph_out = runner.invoke(
            app,
            ["plan", "graph", "PL-demo", "--from-json", str(graph_path), "--json"],
            catch_exceptions=False,
        )

        graph_payload = json.loads(graph_out.output)
        assert graph_payload["nodes"][1]["state"] == "completed"
        assert graph_payload["nodes"][1]["workflow_state"] == "completed"
        assert graph_payload["workflow_summary"]["completed_nodes"] == 2
        assert graph_payload["workflow_summary"]["dispatched_nodes"] == 0
        assert graph_payload["workflow_summary"]["next_action"] == "complete task graph"


def test_plan_progress_falls_back_to_remote_inventory_when_read_endpoint_is_missing(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()
        remote_tuple = lambda ctx, remote_name: ({"url": "http://example.test"}, "demo")
        monkeypatch.setattr(cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(plan_cli_module, "_remote_tuple", remote_tuple)
        monkeypatch.setattr(
            cli_module,
            "remote_read_task_dag_readiness",
            lambda base_url, graph: (_ for _ in ()).throw(cli_module.RemoteError("POST http://example.test failed: 404 Not Found")),
        )
        monkeypatch.setattr(
            cli_module,
            "remote_get_plan",
            lambda base_url, plan_id: {"head_revision": {"plan_revision_id": "PR-demo"}},
        )
        monkeypatch.setattr(
            cli_module,
            "remote_list_tasks",
            lambda base_url, repo_name: [{"task_id": "T-A", "plan_item_ref": "demo/a", "status": "completed"}],
        )
        monkeypatch.setattr(cli_module, "remote_list_changes", lambda base_url, repo_name: [])
        monkeypatch.setattr(cli_module, "remote_list_sessions", lambda base_url, repo_name: [])

        result = runner.invoke(
            app,
            ["plan", "progress", "PL-demo", "--from-json", str(graph_path), "--json"],
            catch_exceptions=False,
        )

        payload = json.loads(result.output)
        assert payload["progress"]["completed_percent"] == 50
        assert payload["progress"]["ready_nodes"] == 1
