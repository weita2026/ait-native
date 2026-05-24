from __future__ import annotations

import importlib
import json
from pathlib import Path

import pytest

from ait.cli import plan_task_linkage
from ait.repo_paths import RepoContext
from ait_native.store import get_local_task, mark_local_plan_published

from ._shared import _write_plan_artifact, app, runner

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_plan_task_linkage_helpers() -> None:
    helper_names = [
        "_normalize_plan_task_linkage",
        "_published_local_task_plan_linkage",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(plan_task_linkage, name)


def test_normalize_plan_task_linkage_contract(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-plan-task-linkage"
    repo.mkdir()
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    ctx = RepoContext.discover()

    with pytest.raises(ValueError, match=r"`--plan-item-ref` requires `--plan` or `--revision`"):
        plan_task_linkage._normalize_plan_task_linkage(ctx, plan_item_ref="demo/item")

    with pytest.raises(ValueError, match=r"Required plan/task binding requires `--plan` \(or `--revision`\) and `--plan-item-ref`"):
        plan_task_linkage._normalize_plan_task_linkage(ctx, require_execution_binding=True)

    assert runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    ).exit_code == 0
    assert plan_task_linkage._normalize_plan_task_linkage(
        ctx,
        plan_id=" PL-LOCAL-1 ",
        local=True,
    ) == ("PL-LOCAL-1", None, None)

    assert runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "strict", "--json"],
        catch_exceptions=False,
    ).exit_code == 0
    with pytest.raises(ValueError, match=r"Strict plan/task binding requires `--plan-item-ref`"):
        plan_task_linkage._normalize_plan_task_linkage(ctx, plan_id="PL-LOCAL-1")

    assert plan_task_linkage._normalize_plan_task_linkage(
        ctx,
        plan_id=" PL-LOCAL-1 ",
        plan_revision_id=" PR-LOCAL-1 ",
        plan_item_ref=" feature/slice ",
    ) == ("PL-LOCAL-1", "PR-LOCAL-1", "feature/slice")


def test_published_local_task_plan_linkage_contract(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-published-plan-task-linkage"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "required", "--json"],
        catch_exceptions=False,
    ).exit_code == 0

    plan_file = _write_plan_artifact(
        repo,
        "docs/sprints/plan_task_linkage_contract.md",
        "# Plan Task Linkage Contract\n\n## Slice [plan-ref: plan-task-linkage/root]\n\n- [ ] keep linkage stable [ref: plan-task-linkage/stable]\n",
    )
    sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"], catch_exceptions=False)
    assert sync_out.exit_code == 0, sync_out.stdout
    sync_payload = json.loads(sync_out.stdout)
    plan_id = str(sync_payload["results"][0]["plan_id"])
    plan_revision_id = str(sync_payload["results"][0]["plan_revision_id"])

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Link local task to published plan metadata",
            "--intent",
            "exercise extracted plan-task linkage helpers",
            "--risk",
            "low",
            "--plan",
            plan_id,
            "--plan-item-ref",
            "plan-task-linkage/stable",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task_payload = json.loads(task_out.stdout)

    ctx = RepoContext.discover()
    local_task = get_local_task(ctx, str(task_payload["task_id"]))

    with pytest.raises(ValueError, match=r"linked to unpublished local plan"):
        plan_task_linkage._published_local_task_plan_linkage(ctx, local_task)

    mark_local_plan_published(
        ctx,
        plan_id,
        remote_name="origin",
        published_plan_id="PL-REMOTE-PLAN-1",
        published_head_revision_id="PR-REMOTE-PLAN-1",
        revision_mappings=[(plan_revision_id, "PR-REMOTE-PLAN-1")],
    )

    local_task = get_local_task(ctx, str(task_payload["task_id"]))
    assert plan_task_linkage._published_local_task_plan_linkage(ctx, local_task) == (
        "PL-REMOTE-PLAN-1",
        "PR-REMOTE-PLAN-1",
        "plan-task-linkage/stable",
    )

    revision_only_task = dict(local_task)
    revision_only_task["plan_id"] = None
    assert plan_task_linkage._published_local_task_plan_linkage(ctx, revision_only_task) == (
        "PL-REMOTE-PLAN-1",
        "PR-REMOTE-PLAN-1",
        "plan-task-linkage/stable",
    )


def test_task_start_rejects_dag_owned_plan_item_ref_until_task_dag_multi_worker_is_enabled(
    tmp_path: Path,
    monkeypatch,
) -> None:
    repo = tmp_path / "housekeeper-dag-task-bootstrap-guard"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    plan_file = _write_plan_artifact(
        repo,
        "docs/sprints/dag_task_bootstrap_guard.md",
        "# DAG Task Bootstrap Guard\n\n"
        "## Guard fan-out [plan-ref: dag-task-bootstrap-guard/root]\n\n"
        "- [ ] reject DAG-owned task bootstrap [ref: dag-task-bootstrap-guard/guard]\n",
    )
    sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"], catch_exceptions=False)
    assert sync_out.exit_code == 0, sync_out.stdout
    sync_payload = json.loads(sync_out.stdout)
    plan_id = str(sync_payload["results"][0]["plan_id"])
    plan_revision_id = str(sync_payload["results"][0]["plan_revision_id"])

    graph_path = repo / "docs/sprints/dag_task_bootstrap_guard.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "dag-task-bootstrap-guard/task-graph",
                "repo_name": "housekeeper",
                "source_plan": {
                    "artifact_path": "docs/sprints/dag_task_bootstrap_guard.md",
                    "plan_id": plan_id,
                    "plan_ref": "dag-task-bootstrap-guard/root",
                    "plan_revision_id": plan_revision_id,
                },
                "dispatch_artifacts": {
                    "source_markdown": "docs/sprints/dag_task_bootstrap_guard.md",
                    "parallel_execution_markdown": "docs/sprints/dag_task_bootstrap_guard.md",
                    "task_graph_json": "docs/sprints/dag_task_bootstrap_guard.task_graph.json",
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
                        "title": "Guard bootstrap",
                        "plan_item_ref": "dag-task-bootstrap-guard/guard",
                        "depends_on": [],
                        "task_template": {"title": "Guard bootstrap", "risk_tier": "low"},
                    }
                ],
                "edges": [],
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    blocked_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Blocked DAG task bootstrap",
            "--intent",
            "prove plan_item_ref guard",
            "--risk",
            "low",
            "--plan",
            plan_id,
            "--plan-item-ref",
            "dag-task-bootstrap-guard/guard",
        ],
        catch_exceptions=False,
    )
    assert blocked_out.exit_code != 0
    assert "manual task bootstrap is blocked" in (blocked_out.output or blocked_out.stdout)
    assert "--auto-compact-worker" in (blocked_out.output or blocked_out.stdout)
    assert "docs/sprints/dag_task_bootstrap_guard.task_graph.json" in (blocked_out.output or blocked_out.stdout)

    enabled_out = runner.invoke(
        app,
        ["config", "set", "--task-dag-allow-multi-worker", "on", "--json"],
        catch_exceptions=False,
    )
    assert enabled_out.exit_code == 0, enabled_out.stdout

    allowed_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Allowed DAG task bootstrap",
            "--intent",
            "prove config opt-in bypass",
            "--risk",
            "low",
            "--plan",
            plan_id,
            "--plan-item-ref",
            "dag-task-bootstrap-guard/guard",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert allowed_out.exit_code == 0, allowed_out.stdout
