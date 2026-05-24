from __future__ import annotations

import importlib
import base64
import hashlib
import json
import os
from pathlib import Path

import pytest

from ._shared import runner, app, running_server, server_store_module, ServerContext, fake_postgres_context


cli_module = importlib.import_module("ait.cli.app")
plan_cli_module = importlib.import_module("ait.cli.commands.plan")
attest_cli_module = importlib.import_module("ait.cli.commands.attest")
change_cli_module = importlib.import_module("ait.cli.commands.change")
patchset_cli_module = importlib.import_module("ait.cli.commands.patchset")
policy_cli_module = importlib.import_module("ait.cli.commands.policy")
review_cli_module = importlib.import_module("ait.cli.commands.review")
review_submission_helpers = importlib.import_module("ait.cli.review_submission_helpers")
task_cli_module = importlib.import_module("ait.cli.commands.task")


def _snapshot_bundle(
    repo_name: str,
    snapshot_id: str,
    *,
    parent_snapshot_id: str | None,
    line_name: str,
    message: str,
    files: dict[str, bytes],
) -> dict:
    file_rows = []
    for path, data in files.items():
        blob_id = f"BLB-{snapshot_id}-{path.replace('/', '_')}"
        file_rows.append(
            {
                "path": path,
                "blob_id": blob_id,
                "size_bytes": len(data),
                "mode": "100644",
                "sha256": hashlib.sha256(data).hexdigest(),
                "content_b64": base64.b64encode(data).decode("ascii"),
            }
        )
    return {
        "snapshot_id": snapshot_id,
        "repo_name": repo_name,
        "parent_snapshot_id": parent_snapshot_id,
        "line_name": line_name,
        "message": message,
        "files": file_rows,
    }


def _publish_ready_change(ctx: ServerContext, repo_name: str, task_title: str, change_title: str, suffix: str):
    task = server_store_module.create_task(ctx, repo_name, task_title, "repo scoped cli", "high")
    change = server_store_module.create_change(ctx, repo_name, task["task_id"], change_title, "main", "medium")
    base_snapshot = server_store_module.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            f"SNP-{suffix}-BASE",
            parent_snapshot_id=None,
            line_name="main",
            message="base",
            files={"README.md": b"base\n"},
        ),
    )
    revision_snapshot = server_store_module.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            f"SNP-{suffix}-REV",
            parent_snapshot_id=base_snapshot["snapshot_id"],
            line_name="main",
            message="revision",
            files={"README.md": f"base\n{suffix}\n".encode("utf-8")},
        ),
    )
    server_store_module.update_line(ctx, repo_name, "main", base_snapshot["snapshot_id"])
    patchset = server_store_module.publish_patchset(
        ctx,
        change["change_id"],
        base_snapshot["snapshot_id"],
        revision_snapshot["snapshot_id"],
        f"patchset {suffix}",
        "human_only",
    )
    server_store_module.upsert_attestation(
        ctx,
        patchset["patchset_id"],
        "human_only",
        {"tests": "pass"},
        {"policy_readable": True},
    )
    server_store_module.record_review(ctx, change["change_id"], patchset["patchset_id"], "alice@example.com", "approve", None)
    server_store_module.evaluate_policy(ctx, patchset["patchset_id"])
    return task, change, patchset


def _compatibility_task_graph(plan_id: str, plan_revision_id: str, graph_id: str = "repo-a/task-dag") -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph_id,
        "repo_name": "repo-a",
        "source_plan": {
            "artifact_path": "docs/sprints/repo_a.md",
            "plan_id": plan_id,
            "plan_revision_id": plan_revision_id,
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
                "title": "Compatibility node",
                "depends_on": [],
                "task_template": {"title": "Compatibility task", "risk_tier": "low"},
            },
        ],
        "edges": [],
    }


def _compatibility_readiness(graph: dict) -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph["graph_id"],
        "source_plan": graph["source_plan"],
        "source_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "current_plan_revision_id": graph["source_plan"]["plan_revision_id"],
        "stale_source_plan": False,
        "summary": {
            "total_nodes": 1,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "completed_nodes": 0,
            "dispatched_nodes": 0,
            "next_action": "start A",
        },
        "counts": {"ready": 1, "running": 0, "blocked": 0, "completed": 0, "total": 1, "dispatched": 0},
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Compatibility node",
                "plan_item_ref": "repo-a/a",
                "state": "ready",
                "workflow_state": "ready",
                "reason": "ready",
                "depends_on": [],
                "lock_keys": [],
                "hotspot_keys": [],
                "task_id": "",
                "change_id": "",
                "session_id": "",
                "session_recommendation": {"action": "open_new_session"},
                "lineage": {},
            },
        ],
    }


def _create_repo_scoped_plan_graph(ctx: ServerContext, repo_name: str, *, graph_id: str = "repo-a/task-dag") -> tuple[dict, dict]:
    plan = server_store_module.create_plan(
        ctx,
        repo_name,
        "Repo scoped execute run plan",
        "docs/sprints/repo_a.md",
        None,
        "Repo scoped execute run plan",
        [],
        summary="seed",
        artifact_body="# Repo scoped execute run plan\n",
    )
    graph = _compatibility_task_graph(
        plan["plan_id"],
        plan["head_revision"]["plan_revision_id"],
        graph_id=graph_id,
    )
    return plan, graph


def _seed_task_graph_run_session(
    ctx: ServerContext,
    repo_name: str,
    *,
    plan_id: str,
    plan_revision_id: str,
    graph_id: str,
    session_id: str,
    execution_state: str,
    pause_reason: str | None = None,
) -> dict:
    graph_run_id = f"graph-run-{session_id.lower()}"
    session = server_store_module.create_session(
        ctx,
        repo_name,
        "task_graph_run",
        title=f"Task DAG execute: {graph_id}",
        metadata={
            "session_policy": "task_dag_execute_run",
            "graph_run_id": graph_run_id,
            "execution_state": execution_state,
            "plan_id": plan_id,
            "plan_revision_id": plan_revision_id,
            "graph_id": graph_id,
        },
        session_id=session_id,
    )
    workflow_summary = {
        "total_nodes": 1,
        "ready_nodes": 1,
        "running_nodes": 0,
        "blocked_nodes": 0,
        "completed_nodes": 0,
        "dispatched_nodes": 0,
        "next_action": "start A",
    }
    server_store_module.append_session_event(
        ctx,
        session_id,
        "task_graph.execution_started",
        {
            "graph_run_id": graph_run_id,
            "plan_id": plan_id,
            "plan_revision_id": plan_revision_id,
            "graph_id": graph_id,
            "graph_artifact_path": "docs/sprints/repo_a.task_graph.json",
            "workflow_summary": workflow_summary,
            "readiness_summary": workflow_summary,
        },
    )
    state_snapshot = {
        "graph_run_id": graph_run_id,
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id,
        "graph_id": graph_id,
            "graph_artifact_path": "docs/sprints/repo_a.task_graph.json",
        "workflow_summary": workflow_summary,
        "readiness_summary": workflow_summary,
        "ready_node_ids": ["A"],
        "running_node_ids": [],
        "blocked_node_ids": [],
        "completed_node_ids": [],
        "dispatched_node_ids": [],
        "next_action": "start A",
        "execution_state": execution_state,
    }
    if pause_reason is not None:
        state_snapshot["pause_reason"] = pause_reason
    state_snapshot["readiness_digest"] = cli_module._task_dag_execute_state_digest(state_snapshot)
    server_store_module.append_session_event(
        ctx,
        session_id,
        "task_graph.state_snapshot",
        state_snapshot,
    )
    return session


def _drift_session_repo_name(ctx: ServerContext, session_id: str, *, repo_name: str = "repo-a-legacy") -> None:
    with server_store_module.connect(ctx) as conn:
        conn.execute("update sessions set repo_name = ? where session_id = ?", (repo_name, session_id))
        conn.commit()


def test_cli_repo_scoped_show_and_land_routes(tmp_path: Path):
    data_dir = tmp_path / "server-data"
    ctx = fake_postgres_context(data_dir)
    server_store_module.initialize(ctx)
    server_store_module.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    task_a1, change_a1, patchset_a1 = _publish_ready_change(ctx, "repo-a", "Task A1", "Change A1", "A1")
    server_store_module.create_land_request(ctx, change_a1["change_id"], patchset_a1["patchset_id"], "main", "direct")
    _, _, _ = _publish_ready_change(ctx, "repo-a", "Task A2", "Change A2", "A2")

    with running_server(data_dir) as base_url:
        with runner.isolated_filesystem():
            os.chdir(os.getcwd())
            assert runner.invoke(app, ["init", "--name", "repo-a"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(
                app,
                ["remote", "add", "origin", base_url, "--repo-name", "repo-a", "--default"],
                catch_exceptions=False,
            ).exit_code == 0
            assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

            task_out = runner.invoke(
                app,
                ["task", "show", "1", "--remote", "origin", "--repo", "repo-a", "--json"],
                catch_exceptions=False,
            )
            assert task_out.exit_code == 0
            assert json.loads(task_out.stdout)["task_id"] == task_a1["task_id"]

            change_out = runner.invoke(
                app,
                ["change", "show", "1", "--remote", "origin", "--repo", "repo-a", "--json"],
                catch_exceptions=False,
            )
            assert change_out.exit_code == 0
            assert json.loads(change_out.stdout)["change_id"] == change_a1["change_id"]

            patchset_out = runner.invoke(
                app,
                ["patchset", "show", "1", "--change", "1", "--remote", "origin", "--repo", "repo-a", "--json"],
                catch_exceptions=False,
            )
            assert patchset_out.exit_code == 0
            assert json.loads(patchset_out.stdout)["patchset_id"] == patchset_a1["patchset_id"]

            land_out = runner.invoke(
                app,
                ["land", "show", "1", "--remote", "origin", "--repo", "repo-a", "--json"],
                catch_exceptions=False,
            )
            assert land_out.exit_code == 0
            assert json.loads(land_out.stdout)["land_seq"] == 1

            submit_out = runner.invoke(
                app,
                ["land", "submit", "2", "--patchset", "1", "--target", "main", "--mode", "direct", "--remote", "origin", "--repo", "repo-a", "--json"],
                catch_exceptions=False,
            )
            assert submit_out.exit_code == 2
            submit_output = submit_out.output or submit_out.stdout
            assert "requires a task-bound worktree" in submit_output
            assert "ait task start" in submit_output


def test_cli_repo_scoped_show_and_land_routes_with_drifted_repo_name(tmp_path: Path):
    data_dir = tmp_path / "server-data-drift"
    ctx = fake_postgres_context(data_dir)
    server_store_module.initialize(ctx)
    server_store_module.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    task, change, patchset = _publish_ready_change(ctx, "repo-a", "Task Drift", "Change Drift", "DRIFT")
    server_store_module.create_land_request(ctx, change["change_id"], patchset["patchset_id"], "main", "direct")

    with server_store_module.connect(ctx) as conn:
        drifted_repo_name = "repo-a-legacy"
        conn.execute("update tasks set repo_name = ? where task_id = ?", (drifted_repo_name, task["task_id"]))
        conn.execute("update changes set repo_name = ? where change_id = ?", (drifted_repo_name, change["change_id"]))
        conn.commit()

    with running_server(data_dir) as base_url:
        with runner.isolated_filesystem():
            os.chdir(os.getcwd())
            assert runner.invoke(app, ["init", "--name", "repo-a"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(
                app,
                ["remote", "add", "origin", base_url, "--repo-name", "repo-a", "--default"],
                catch_exceptions=False,
            ).exit_code == 0
            assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

            task_out = runner.invoke(
                app,
                ["task", "show", "1", "--remote", "origin", "--repo", "repo-a", "--json"],
                catch_exceptions=False,
            )
            assert task_out.exit_code == 0
            assert json.loads(task_out.stdout)["task_id"] == task["task_id"]

            change_out = runner.invoke(
                app,
                ["change", "show", "1", "--remote", "origin", "--repo", "repo-a", "--json"],
                catch_exceptions=False,
            )
            assert change_out.exit_code == 0
            assert json.loads(change_out.stdout)["change_id"] == change["change_id"]

            patchset_out = runner.invoke(
                app,
                ["patchset", "show", "1", "--change", "1", "--remote", "origin", "--repo", "repo-a", "--json"],
                catch_exceptions=False,
            )
            assert patchset_out.exit_code == 0
            assert json.loads(patchset_out.stdout)["patchset_id"] == patchset["patchset_id"]

            land_out = runner.invoke(
                app,
                ["land", "show", "1", "--remote", "origin", "--repo", "repo-a", "--json"],
                catch_exceptions=False,
            )
            assert land_out.exit_code == 0
            assert json.loads(land_out.stdout)["land_seq"] == 1

            submit_out = runner.invoke(
                app,
                ["land", "submit", "1", "--patchset", "1", "--target", "main", "--mode", "direct", "--remote", "origin", "--repo", "repo-a", "--json"],
                catch_exceptions=False,
            )
            assert submit_out.exit_code == 2
            submit_output = submit_out.output or submit_out.stdout
            assert "requires a task-bound worktree" in submit_output
            assert "ait task start" in submit_output


def test_cli_repo_scoped_session_routes_with_local_refs(tmp_path: Path):
    data_dir = tmp_path / "server-data-sessions"
    ctx = fake_postgres_context(data_dir)
    server_store_module.initialize(ctx)
    server_store_module.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    task, change, _ = _publish_ready_change(ctx, "repo-a", "Task Session", "Change Session", "SES")
    session = server_store_module.create_session(
        ctx,
        "repo-a",
        "task_graph_run",
        task_id=task["task_id"],
        change_id=change["change_id"],
        title="Repo session",
    )

    with running_server(data_dir) as base_url:
        with runner.isolated_filesystem():
            os.chdir(os.getcwd())
            assert runner.invoke(app, ["init", "--name", "repo-a"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(
                app,
                ["remote", "add", "origin", base_url, "--repo-name", "repo-a", "--default"],
                catch_exceptions=False,
            ).exit_code == 0
            assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

            session_out = runner.invoke(
                app,
                ["session", "show", session["session_local_id"], "--remote", "origin", "--json"],
                catch_exceptions=False,
            )
            assert session_out.exit_code == 0
            assert json.loads(session_out.stdout)["session_id"] == session["session_id"]

            append_out = runner.invoke(
                app,
                ["session", "append", session["session_local_id"], "--type", "session.note", "--text", "hello", "--remote", "origin", "--json"],
                catch_exceptions=False,
            )
            assert append_out.exit_code == 0
            assert json.loads(append_out.stdout)["event_type"] == "session.note"

            events_out = runner.invoke(
                app,
                ["session", "events", session["session_local_id"], "--remote", "origin", "--json"],
                catch_exceptions=False,
            )
            assert events_out.exit_code == 0
            events_payload = json.loads(events_out.stdout)
            assert any(row["event_type"] == "session.note" for row in events_payload)

            checkpoint_out = runner.invoke(
                app,
                ["session", "checkpoint", session["session_local_id"], "--summary", "repo checkpoint", "--remote", "origin", "--json"],
                catch_exceptions=False,
            )
            assert checkpoint_out.exit_code == 0
            checkpoint_payload = json.loads(checkpoint_out.stdout)
            assert checkpoint_payload["session_id"] == session["session_id"]

            checkpoints_out = runner.invoke(
                app,
                ["session", "checkpoints", session["session_local_id"], "--remote", "origin", "--json"],
                catch_exceptions=False,
            )
            assert checkpoints_out.exit_code == 0
            checkpoints_payload = json.loads(checkpoints_out.stdout)
            assert checkpoints_payload[0]["checkpoint_id"] == checkpoint_payload["checkpoint_id"]

            close_out = runner.invoke(
                app,
                ["session", "close", session["session_local_id"], "--status", "paused", "--remote", "origin", "--json"],
                catch_exceptions=False,
            )
            assert close_out.exit_code == 0
            assert json.loads(close_out.stdout)["status"] == "paused"

            resume_out = runner.invoke(
                app,
                ["session", "resume", session["session_local_id"], "--remote", "origin", "--json"],
                catch_exceptions=False,
            )
            assert resume_out.exit_code == 0
            assert json.loads(resume_out.stdout)["session"]["status"] == "active"


def test_cli_plan_execute_latest_run_uses_repo_scoped_session_routes_with_drifted_repo_name(tmp_path: Path):
    data_dir = tmp_path / "server-data-execute-latest"
    ctx = fake_postgres_context(data_dir)
    server_store_module.initialize(ctx)
    server_store_module.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    plan, graph = _create_repo_scoped_plan_graph(ctx, "repo-a")
    older = _seed_task_graph_run_session(
        ctx,
        "repo-a",
        plan_id=plan["plan_id"],
        plan_revision_id=plan["head_revision"]["plan_revision_id"],
        graph_id=graph["graph_id"],
        session_id="S-RUN-OLD",
        execution_state="waiting_for_review",
        pause_reason="review",
    )
    latest = _seed_task_graph_run_session(
        ctx,
        "repo-a",
        plan_id=plan["plan_id"],
        plan_revision_id=plan["head_revision"]["plan_revision_id"],
        graph_id=graph["graph_id"],
        session_id="S-RUN-LATEST",
        execution_state="manual_operator_pause",
        pause_reason="manual",
    )
    _drift_session_repo_name(ctx, latest["session_id"])

    with running_server(data_dir) as base_url:
        with runner.isolated_filesystem():
            os.chdir(os.getcwd())
            graph_path = Path("docs/sprints/repo_a.task_graph.json")
            graph_path.parent.mkdir(parents=True, exist_ok=True)
            graph_path.write_text(json.dumps(graph), encoding="utf-8")
            assert runner.invoke(app, ["init", "--name", "repo-a"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(
                app,
                ["remote", "add", "origin", base_url, "--repo-name", "repo-a", "--default"],
                catch_exceptions=False,
            ).exit_code == 0
            assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

            result = runner.invoke(
                app,
                [
                    "plan",
                    "execute",
                    plan["plan_id"],
                    "--from-json",
                    str(graph_path),
                    "--latest-run",
                    "--remote",
                    "origin",
                    "--json",
                ],
                catch_exceptions=False,
            )

    assert result.exit_code == 0
    payload = json.loads(result.stdout)
    assert payload["mode"] == "inspect_run"
    assert payload["recorded_run"]["session_id"] == latest["session_id"]
    assert payload["recorded_run"]["execution_state"] == "manual_operator_pause"
    assert payload["recorded_run"]["event_count"] == 2
    assert payload["recorded_run"]["latest_state_snapshot"]["pause_reason"] == "manual"
    assert payload["recorded_run"]["session_id"] != older["session_id"]


@pytest.mark.parametrize(
    ("flag", "event_type"),
    [
        ("--resume-run", "task_graph.execution_resumed"),
        ("--retry-run", "task_graph.execution_retried"),
    ],
)
def test_cli_plan_execute_repo_scoped_latest_run_controls_work_with_drifted_session_repo_name(
    tmp_path: Path,
    flag: str,
    event_type: str,
):
    data_dir = tmp_path / f"server-data-execute-control-{flag.removeprefix('--').removesuffix('-run')}"
    ctx = fake_postgres_context(data_dir)
    server_store_module.initialize(ctx)
    server_store_module.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    plan, graph = _create_repo_scoped_plan_graph(ctx, "repo-a")
    session = _seed_task_graph_run_session(
        ctx,
        "repo-a",
        plan_id=plan["plan_id"],
        plan_revision_id=plan["head_revision"]["plan_revision_id"],
        graph_id=graph["graph_id"],
        session_id="S-RUN-CONTROL",
        execution_state="waiting_for_review",
        pause_reason="review",
    )
    _drift_session_repo_name(ctx, session["session_id"])

    with running_server(data_dir) as base_url:
        with runner.isolated_filesystem():
            os.chdir(os.getcwd())
            graph_path = Path("docs/sprints/repo_a.task_graph.json")
            graph_path.parent.mkdir(parents=True, exist_ok=True)
            graph_path.write_text(json.dumps(graph), encoding="utf-8")
            assert runner.invoke(app, ["init", "--name", "repo-a"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(
                app,
                ["remote", "add", "origin", base_url, "--repo-name", "repo-a", "--default"],
                catch_exceptions=False,
            ).exit_code == 0
            assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

            result = runner.invoke(
                app,
                [
                    "plan",
                    "execute",
                    plan["plan_id"],
                    "--from-json",
                    str(graph_path),
                    flag,
                    "--latest-run",
                    "--remote",
                    "origin",
                    "--yes",
                    "--json",
                ],
                catch_exceptions=False,
            )

    assert result.exit_code == 0
    payload = json.loads(result.stdout)
    assert payload["mode"] == f"{flag.removeprefix('--').replace('-', '_')}"
    controlled = payload["controlled_run"]
    assert controlled["session_id"] == session["session_id"]
    assert controlled["controlled"] is True
    assert controlled["execution_state"] == "active"
    assert controlled["control_event"]["event_type"] == event_type
    assert controlled["state_snapshot_event"]["payload"]["execution_state"] == "active"

    event_types = [row["event_type"] for row in server_store_module.list_session_events(ctx, session["session_id"])]
    assert event_types[-2:] == [event_type, "task_graph.state_snapshot"]


def test_cli_repo_scoped_first_party_commands_pass_repo_name(tmp_path: Path, monkeypatch):
    remote_url = "http://example.test"
    call_log: dict[str, dict[str, str]] = {}

    def remote_tuple(_ctx, remote_name):
        return {"url": remote_url, "name": remote_name}, "repo-a"

    for module in (
        task_cli_module,
        change_cli_module,
        patchset_cli_module,
        review_cli_module,
        policy_cli_module,
        attest_cli_module,
    ):
        monkeypatch.setattr(module, "_remote_tuple", remote_tuple)

    def fake_remote_read_task_audit(base_url, task_id, target_line="main", repo_name=None):
        call_log["audit"] = {"base_url": base_url, "task_id": task_id, "repo_name": repo_name}
        assert target_line == "main"
        return {
            "task": {"title": "Task", "status": "in-progress"},
            "workflow": {"state": "active", "reason": "running"},
            "summary": {
                "verdict": "pass",
                "open_change_count": 0,
                "landed_change_count": 0,
                "effective_on_target_change_count": 0,
                "stale_workflow_records": False,
            },
            "target": {"line_name": "main", "head_snapshot_id": "S1"},
            "recommended_action": {"label": "continue", "detail": "ok"},
            "changes": [],
        }

    def fake_remote_close_task(base_url, task_id, status="completed", repo_name=None):
        call_log["task_canceled"] = {"base_url": base_url, "task_id": task_id, "repo_name": repo_name}
        return {"task_id": task_id, "status": status}

    def fake_remote_close_change(base_url, change_id, status="archived", repo_name=None):
        call_log["change_close"] = {"base_url": base_url, "change_id": change_id, "repo_name": repo_name}
        return {"change_id": change_id, "status": status}

    def fake_remote_select_patchset(base_url, change_id, patchset_id, repo_name=None):
        call_log["patchset_select"] = {"base_url": base_url, "change_id": change_id, "patchset_id": patchset_id, "repo_name": repo_name}
        return {"change_id": change_id, "patchset_id": patchset_id}

    def fake_remote_list_patchsets(base_url, change_id, repo_name=None):
        call_log.setdefault("review_patchsets", {"base_url": base_url, "change_id": change_id, "repo_name": repo_name})
        call_log["review_patchsets"] = {"base_url": base_url, "change_id": change_id, "repo_name": repo_name}
        return [{"patchset_id": "PS-1"}]

    def fake_remote_request_review(base_url, change_id, patchset_id, reviewer_groups, note, repo_name=None):
        call_log["review_request"] = {
            "base_url": base_url,
            "change_id": change_id,
            "patchset_id": patchset_id,
            "repo_name": repo_name,
        }
        return {"change_id": change_id, "action": "requested"}

    def fake_remote_record_review(base_url, change_id, patchset_id, reviewer, action, comment=None, blocking=False, repo_name=None):
        call_log["review_record"] = {
            "base_url": base_url,
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer": reviewer,
            "repo_name": repo_name,
            "action": action,
            "blocking": str(blocking),
        }
        return {"change_id": change_id, "action": action}

    def fake_remote_list_reviews(base_url, change_id, repo_name=None):
        call_log["review_show"] = {"base_url": base_url, "change_id": change_id, "repo_name": repo_name}
        return {"current_patchset_id": "PS-1", "approvals": 1, "blocking": 0, "comments": 0, "review_requests": [], "reviews": []}

    def fake_remote_evaluate_policy(base_url, patchset_id, repo_name=None):
        call_log["policy_eval"] = {"base_url": base_url, "patchset_id": patchset_id, "repo_name": repo_name}
        return {"patchset_id": patchset_id}

    def fake_remote_get_policy(base_url, patchset_id, repo_name=None):
        call_log["policy_show"] = {"base_url": base_url, "patchset_id": patchset_id, "repo_name": repo_name}
        return {"lane": "main", "decision": "pass", "evaluated_at": "0", "checks": []}

    def fake_remote_create_waiver(base_url, patchset_id, rule_name, reason, expires_at=None, repo_name=None):
        call_log["policy_waive"] = {
            "base_url": base_url,
            "patchset_id": patchset_id,
            "rule_name": rule_name,
            "repo_name": repo_name,
        }
        return {"patchset_id": patchset_id, "rule_name": rule_name}

    def fake_remote_put_attestation(base_url, patchset_id, author_mode, evaluation_summary, provenance_summary=None, detail=None, repo_name=None):
        call_log["attest_put"] = {"base_url": base_url, "patchset_id": patchset_id, "repo_name": repo_name}
        return {"patchset_id": patchset_id, "author_mode": author_mode}

    def fake_remote_get_attestation(base_url, patchset_id, repo_name=None):
        call_log["attest_show"] = {"base_url": base_url, "patchset_id": patchset_id, "repo_name": repo_name}
        return {"patchset_id": patchset_id, "checks": {}}

    monkeypatch.setattr(task_cli_module, "remote_read_task_audit", fake_remote_read_task_audit)
    monkeypatch.setattr(task_cli_module, "remote_close_task", fake_remote_close_task)
    monkeypatch.setattr(change_cli_module, "remote_close_change", fake_remote_close_change)
    monkeypatch.setattr(patchset_cli_module, "remote_select_patchset", fake_remote_select_patchset)
    monkeypatch.setattr(review_cli_module, "remote_list_reviews", fake_remote_list_reviews)
    monkeypatch.setattr(review_submission_helpers, "_remote_tuple", remote_tuple)
    monkeypatch.setattr(review_submission_helpers, "remote_list_patchsets", fake_remote_list_patchsets)
    monkeypatch.setattr(review_submission_helpers, "remote_request_review", fake_remote_request_review)
    monkeypatch.setattr(review_submission_helpers, "remote_record_review", fake_remote_record_review)
    monkeypatch.setattr(policy_cli_module, "remote_evaluate_policy", fake_remote_evaluate_policy)
    monkeypatch.setattr(policy_cli_module, "remote_get_policy", fake_remote_get_policy)
    monkeypatch.setattr(policy_cli_module, "remote_create_waiver", fake_remote_create_waiver)
    monkeypatch.setattr(attest_cli_module, "remote_list_patchsets", fake_remote_list_patchsets)
    monkeypatch.setattr(attest_cli_module, "remote_put_attestation", fake_remote_put_attestation)
    monkeypatch.setattr(attest_cli_module, "remote_get_attestation", fake_remote_get_attestation)

    with runner.isolated_filesystem():
        assert runner.invoke(app, ["init", "--name", "repo-a"], catch_exceptions=False).exit_code == 0

        assert (
            runner.invoke(app, ["task", "audit", "1", "--remote", "origin", "--json"], catch_exceptions=False).exit_code
            == 0
        )
        assert (
            runner.invoke(app, ["task", "canceled", "1", "--remote", "origin", "--json"], catch_exceptions=False).exit_code
            == 0
        )
        assert (
            runner.invoke(app, ["change", "close", "2", "--remote", "origin", "--json"], catch_exceptions=False).exit_code
            == 0
        )
        assert (
            runner.invoke(
                app,
                ["patchset", "select", "2", "--change", "3", "--remote", "origin", "--json"],
                catch_exceptions=False,
            ).exit_code
            == 0
        )
        assert (
            runner.invoke(
                app,
                ["review", "team", "request", "4", "--group", "core", "--remote", "origin", "--json"],
                catch_exceptions=False,
            ).exit_code
            == 0
        )
        assert (
            runner.invoke(app, ["review", "show", "4", "--remote", "origin", "--json"], catch_exceptions=False).exit_code == 0
        )
        assert (
            runner.invoke(
                app,
                ["policy", "eval", "PS-1", "--remote", "origin", "--json"],
                catch_exceptions=False,
            ).exit_code
            == 0
        )
        assert (
            runner.invoke(app, ["policy", "show", "PS-1", "--remote", "origin", "--json"], catch_exceptions=False).exit_code
            == 0
        )
        assert (
            runner.invoke(
                app,
                [
                    "policy",
                    "waive",
                    "PS-1",
                    "--rule",
                    "require_tests",
                    "--reason",
                    "compatibility",
                    "--remote",
                    "origin",
                    "--json",
                ],
                catch_exceptions=False,
            ).exit_code
            == 0
        )
        assert (
            runner.invoke(
                app,
                ["attest", "put", "PS-1", "--tests", "pass", "--remote", "origin", "--json"],
                catch_exceptions=False,
            ).exit_code
            == 0
        )
        assert (
            runner.invoke(app, ["attest", "show", "PS-1", "--remote", "origin", "--json"], catch_exceptions=False).exit_code
            == 0
        )

    for command in (
        "audit",
        "task_canceled",
        "change_close",
        "patchset_select",
        "review_patchsets",
        "review_request",
        "review_show",
        "policy_eval",
        "policy_show",
        "policy_waive",
        "attest_put",
        "attest_show",
    ):
        assert call_log[command]["repo_name"] == "repo-a", f"{command} did not receive repo name"


def test_task_audit_local_draft_skips_remote_read(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-audit-local-draft"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert (
        runner.invoke(
            app,
            ["remote", "add", "origin", "http://example.test", "--repo-name", "housekeeper", "--default"],
            catch_exceptions=False,
        ).exit_code
        == 0
    )
    assert (
        runner.invoke(
            app,
            ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
            catch_exceptions=False,
        ).exit_code
        == 0
    )
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False).exit_code == 0

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Audit local draft",
            "--intent",
            "skip remote task audit for unpublished local draft tasks",
            "--risk",
            "low",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task_id = json.loads(task_out.stdout)["task_id"]

    def fail_remote_read_task_audit(*_args, **_kwargs):
        raise AssertionError("local draft task audit should not call the remote audit endpoint")

    def fail_remote_repository_lookup(*_args, **_kwargs):
        raise AssertionError("local draft task audit should not query remote repository metadata")

    def fail_remote_target_lookup(*_args, **_kwargs):
        raise AssertionError("local draft task audit should not query remote target-line ancestry")

    monkeypatch.setattr(task_cli_module, "remote_read_task_audit", fail_remote_read_task_audit)
    monkeypatch.setattr(task_cli_module, "remote_get_repository", fail_remote_repository_lookup)
    monkeypatch.setattr(task_cli_module, "get_remote_line", fail_remote_target_lookup)

    audit_out = runner.invoke(app, ["task", "audit", task_id, "--json"], catch_exceptions=False)
    assert audit_out.exit_code == 0, audit_out.stdout
    payload = json.loads(audit_out.stdout)
    assert payload["audit_source"]["mode"] == "local_draft"
    assert payload["audit_source"]["remote_task_missing"] is True
    assert payload["summary"]["verdict"] == "no_changes"


def test_task_audit_human_readable_local_draft_renders_without_name_error(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-audit-human-readable"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert (
        runner.invoke(
            app,
            ["remote", "add", "origin", "http://example.test", "--repo-name", "housekeeper", "--default"],
            catch_exceptions=False,
        ).exit_code
        == 0
    )
    assert (
        runner.invoke(
            app,
            ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
            catch_exceptions=False,
        ).exit_code
        == 0
    )
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False).exit_code == 0

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Audit local draft",
            "--intent",
            "render human-readable task audit output without rich import errors",
            "--risk",
            "low",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task_id = json.loads(task_out.stdout)["task_id"]

    audit_out = runner.invoke(app, ["task", "audit", task_id], catch_exceptions=False)
    assert audit_out.exit_code == 0, audit_out.stdout
    assert f"task audit {task_id}" in audit_out.stdout
    assert "Audit local draft" in audit_out.stdout
    assert "linked changes" in audit_out.stdout
