from __future__ import annotations

import importlib
import json
import sys
from pathlib import Path

import pytest
from typer.testing import CliRunner

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from ait.cli import app


runner = CliRunner()
cli_app_module = importlib.import_module("ait.cli.app")
pytestmark = pytest.mark.usefixtures("explicit_host_ram_root_cleanup")


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
            "validate_source_plan_revision": True,
            "worker_execution_mode": "worker_only_compact_packet",
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Ready root node",
                "plan_item_ref": "demo/a",
                "depends_on": [],
                "progress_weight": 1,
                "task_template": {"title": "Do A", "risk_tier": "low"},
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Blocked dependent node",
                "plan_item_ref": "demo/b",
                "depends_on": ["A"],
                "progress_weight": 1,
                "task_template": {"title": "Do B", "risk_tier": "medium"},
            },
        ],
        "edges": [{"from": "A", "to": "B", "edge_kind": "depends_on"}],
    }


def _write_graph() -> Path:
    path = Path("docs/sprints/demo.task_graph.json")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(_graph()), encoding="utf-8")
    return path


def _init_repo(monkeypatch):
    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")
    result = runner.invoke(app, ["init", "--name", "demo", "--json"], catch_exceptions=False)
    assert result.exit_code == 0


def test_plan_schedule_falls_back_to_remote_inventory_after_readiness_404(monkeypatch, tmp_path):
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _init_repo(monkeypatch)
        graph_path = _write_graph()

        monkeypatch.setattr(cli_app_module, "_remote_tuple", lambda ctx, remote_name: ({"url": "http://example.test"}, "demo"))

        def _raise_readiness_404(*args, **kwargs):
            raise cli_app_module.RemoteError(
                "POST http://example.test/v1/native/read/task-dag-readiness failed: 404 'Unknown plan: PL-demo'"
            )

        def _raise_plan_404(*args, **kwargs):
            raise cli_app_module.RemoteError(
                "GET http://example.test/v1/native/sprints/PL-demo failed: 404 'Unknown plan: PL-demo'"
            )

        monkeypatch.setattr(cli_app_module, "remote_read_task_dag_readiness", _raise_readiness_404)
        monkeypatch.setattr(cli_app_module, "remote_get_plan", _raise_plan_404)
        monkeypatch.setattr(cli_app_module, "remote_list_tasks", lambda *args, **kwargs: [])
        monkeypatch.setattr(cli_app_module, "remote_list_changes", lambda *args, **kwargs: [])
        monkeypatch.setattr(cli_app_module, "remote_list_sessions", lambda *args, **kwargs: [])

        result = runner.invoke(
            app,
            ["plan", "schedule", "PL-demo", "--from-json", str(graph_path), "--remote", "origin", "--json"],
            catch_exceptions=False,
        )

        assert result.exit_code == 0
        payload = json.loads(result.output)
        assert payload["summary"]["ready_nodes"] == 1
        assert payload["summary"]["blocked_nodes"] == 1
        assert payload["ready"][0]["node_id"] == "A"
        assert payload["blocked"][0]["node_id"] == "B"
