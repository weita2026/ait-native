from __future__ import annotations

import importlib
import json
from pathlib import Path
from types import SimpleNamespace

import pytest

from ait.cli import task_dag_graph_artifacts

cli_app_module = importlib.import_module("ait.cli.app")


def _graph(plan_id: str) -> dict:
    return {
        "schema_version": 1,
        "graph_id": f"demo/{plan_id}",
        "repo_name": "demo",
        "source_plan": {
            "artifact_path": "docs/sprints/demo.md",
            "plan_id": plan_id,
            "plan_ref": "demo/root",
            "plan_revision_id": "PR-demo",
        },
        "execution_policy": {
            "mode": "guarded_full_dag_convergence",
            "validate_source_plan_revision": True,
            "worker_execution_mode": "worker_only_compact_packet",
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Do A",
                "plan_item_ref": "demo/a",
                "depends_on": [],
                "progress_weight": 1,
                "task_template": {"title": "Do A", "risk_tier": "low"},
            }
        ],
        "edges": [],
    }


def _write_graph(path: Path, plan_id: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(_graph(plan_id), indent=2), encoding="utf-8")


def test_cli_app_reexports_graph_loading_helper() -> None:
    assert cli_app_module._load_task_dag_graph_for_plan is task_dag_graph_artifacts._load_task_dag_graph_for_plan


def test_load_task_dag_graph_for_plan_prefers_docs_sprints_family(tmp_path: Path) -> None:
    ctx = SimpleNamespace(root=tmp_path)
    plan_id = "PL-demo"
    docs_plan = tmp_path / "docs" / "plans" / "demo.task_graph.json"
    docs_sprint = tmp_path / "docs" / "sprints" / "demo.task_graph.json"
    _write_graph(docs_plan, plan_id)
    _write_graph(docs_sprint, plan_id)

    graph, path = task_dag_graph_artifacts._load_task_dag_graph_for_plan(ctx, plan_id, None)

    assert graph["source_plan"]["plan_id"] == plan_id
    assert path == docs_sprint


def test_load_task_dag_graph_for_plan_rejects_same_family_duplicates(tmp_path: Path) -> None:
    ctx = SimpleNamespace(root=tmp_path)
    plan_id = "PL-demo"
    path_a = tmp_path / "docs" / "sprints" / "a.task_graph.json"
    path_b = tmp_path / "docs" / "sprints" / "b.task_graph.json"
    _write_graph(path_a, plan_id)
    _write_graph(path_b, plan_id)

    with pytest.raises(ValueError, match="Multiple task graph JSON artifacts match plan 'PL-demo' under docs/sprints"):
        task_dag_graph_artifacts._load_task_dag_graph_for_plan(ctx, plan_id, None)


def test_load_task_dag_graph_for_plan_validates_explicit_plan_id(tmp_path: Path) -> None:
    ctx = SimpleNamespace(root=tmp_path)
    graph_path = tmp_path / "docs" / "sprints" / "demo.task_graph.json"
    _write_graph(graph_path, "PL-other")

    with pytest.raises(ValueError, match="belongs to plan 'PL-other', not 'PL-demo'"):
        task_dag_graph_artifacts._load_task_dag_graph_for_plan(ctx, "PL-demo", graph_path)
