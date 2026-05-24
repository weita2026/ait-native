from __future__ import annotations

import json
import os
import socket
import threading
import time
import urllib.request
from contextlib import contextmanager
from pathlib import Path

import uvicorn
from typer.testing import CliRunner

from ait_native.cli import app
from ait_native.server import create_app
from tests.postgres_fake import fake_postgres_dsn, install_fake_psycopg_global, reset_fake_postgres_runtime

runner = CliRunner()
STACK_CODE_REVIEW_SUMMARY = (
    "Reviewed files: stack_notes.txt; Findings: no blocking findings; "
    "Risks: low stack state regression risk; Tests: targeted native auth stack pytest coverage; "
    "Recommendation: safe to land."
)


@contextmanager
def running_server(data_dir: Path, auth_mode: str = "open"):
    old_data = os.environ.get("AIT_NATIVE_SERVER_DATA")
    old_mode = os.environ.get("AIT_NATIVE_AUTH_MODE")
    old_backend = os.environ.get("AIT_NATIVE_SERVER_DB_BACKEND")
    old_dsn = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN")
    old_content_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA")
    old_control_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA")
    install_fake_psycopg_global()
    reset_fake_postgres_runtime()
    os.environ["AIT_NATIVE_SERVER_DATA"] = str(data_dir)
    os.environ["AIT_NATIVE_AUTH_MODE"] = auth_mode
    os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = "postgres"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = fake_postgres_dsn(data_dir)
    os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA"] = "ait_native_content"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA"] = "ait_native_control"
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    host, port = sock.getsockname()
    sock.close()
    app_obj = create_app()
    config = uvicorn.Config(app_obj, host=host, port=port, log_level="error")
    server = uvicorn.Server(config)
    thread = threading.Thread(target=server.run, daemon=True)
    thread.start()
    base_url = f"http://{host}:{port}"
    deadline = time.time() + 10
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"{base_url}/healthz", timeout=0.5) as resp:
                if resp.status == 200:
                    break
        except Exception:
            time.sleep(0.05)
    else:
        server.should_exit = True
        thread.join(timeout=5)
        raise RuntimeError("native test server did not start")
    try:
        yield base_url
    finally:
        server.should_exit = True
        thread.join(timeout=5)
        reset_fake_postgres_runtime()
        if old_data is None:
            os.environ.pop("AIT_NATIVE_SERVER_DATA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DATA"] = old_data
        if old_mode is None:
            os.environ.pop("AIT_NATIVE_AUTH_MODE", None)
        else:
            os.environ["AIT_NATIVE_AUTH_MODE"] = old_mode
        if old_backend is None:
            os.environ.pop("AIT_NATIVE_SERVER_DB_BACKEND", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = old_backend
        if old_dsn is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_DSN", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = old_dsn
        if old_content_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA"] = old_content_schema
        if old_control_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA"] = old_control_schema


def _bootstrap_main(repo: Path, base_url: str, monkeypatch) -> None:
    monkeypatch.chdir(repo)
    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
    main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
    assert main_snap_out.exit_code == 0, main_snap_out.stdout
    assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0


def _create_plan_bound_task(repo: Path, title: str, intent: str) -> dict:
    artifact_path = Path("docs/sprints/stack_demo.md")
    plan_ref = "stack-demo/bootstrap"
    plan_item_ref = f"{plan_ref}/task"
    plan_file = repo / artifact_path
    plan_file.parent.mkdir(parents=True, exist_ok=True)
    plan_file.write_text(
        (
            "# Stack Demo\n\n"
            f"## Stack Demo Bootstrap [plan-ref: {plan_ref}]\n\n"
            f"- [ ] Bootstrap stack lifecycle coverage [ref: {plan_item_ref}]\n"
        ),
        encoding="utf-8",
    )
    sync_out = runner.invoke(
        app,
        ["plan", "sync", str(artifact_path), "--remote", "origin", "--json"],
        catch_exceptions=False,
    )
    assert sync_out.exit_code == 0, sync_out.stdout
    plan = json.loads(sync_out.stdout)["results"][0]
    pull_out = runner.invoke(app, ["pull", "--line", "main", "--json"], catch_exceptions=False)
    assert pull_out.exit_code == 0, pull_out.stdout
    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--title",
            title,
            "--intent",
            intent,
            "--risk",
            "medium",
            "--plan",
            plan["plan_id"],
            "--plan-item-ref",
            plan_item_ref,
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    return json.loads(task_out.stdout)


def _create_feature_change(worktree_path: Path, line_name: str, task_id: str, title: str) -> tuple[dict, dict]:
    previous_cwd = Path.cwd()
    os.chdir(worktree_path)
    try:
        assert runner.invoke(app, ["line", "create", line_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", line_name], catch_exceptions=False).exit_code == 0
        target = worktree_path / "stack_notes.txt"
        existing = target.read_text(encoding="utf-8") if target.exists() else ""
        target.write_text(existing + f"{line_name}\n", encoding="utf-8")
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", line_name, "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task_id, "--title", title, "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)
        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", title, "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        return change, patchset
    finally:
        os.chdir(previous_cwd)


def _make_landable(change_id: str, patchset_id: str) -> None:
    attest_out = runner.invoke(
        app,
        ["attest", "put", patchset_id, "--tests", "pass", "--lint", "pass", "--security", "pass", "--license", "pass", "--json"],
        catch_exceptions=False,
    )
    assert attest_out.exit_code == 0, attest_out.stdout
    approve_out = runner.invoke(
        app,
        ["review", "approve", change_id, "--reviewer", "alice@example.com", "--patchset", patchset_id, "--json"],
        catch_exceptions=False,
    )
    assert approve_out.exit_code == 0, approve_out.stdout
    code_review_out = runner.invoke(
        app,
        [
            "review",
            "code",
            "submit",
            change_id,
            "--reviewer",
            "codex@example.com",
            "--patchset",
            patchset_id,
            "--verdict",
            "pass",
            "--message",
            STACK_CODE_REVIEW_SUMMARY,
            "--json",
        ],
        catch_exceptions=False,
    )
    assert code_review_out.exit_code == 0, code_review_out.stdout
    policy_out = runner.invoke(app, ["policy", "eval", patchset_id, "--json"], catch_exceptions=False)
    assert policy_out.exit_code == 0, policy_out.stdout
    policy = json.loads(policy_out.stdout)
    assert policy["decision"] == "pass"



def test_native_stack_status_moves_from_active_to_ready_to_land_to_blocked(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-stack") as base_url:
        _bootstrap_main(repo, base_url, monkeypatch)
        task = _create_plan_bound_task(repo, "stack demo", "demo")
        worktree_path = Path(task["worktree"]["open_path"])
        monkeypatch.chdir(worktree_path)

        change1, patchset1 = _create_feature_change(worktree_path, "feature/one", task["task_id"], "Feature one")
        change2, patchset2 = _create_feature_change(worktree_path, "feature/two", task["task_id"], "Feature two")

        stack_out = runner.invoke(
            app,
            ["stack", "create", "--title", "housekeeper stack", "--change", change1["change_id"], "--change", change2["change_id"], "--json"],
            catch_exceptions=False,
        )
        assert stack_out.exit_code == 0, stack_out.stdout
        stack = json.loads(stack_out.stdout)
        assert stack["status"] == "active"

        graph_out = runner.invoke(app, ["stack", "graph", stack["stack_id"], "--json"], catch_exceptions=False)
        assert graph_out.exit_code == 0, graph_out.stdout
        graph = json.loads(graph_out.stdout)
        assert [node["change_id"] for node in graph["nodes"]] == [change1["change_id"], change2["change_id"]]

        reorder_out = runner.invoke(
            app,
            ["stack", "reorder", stack["stack_id"], "--change", change2["change_id"], "--position", "1", "--json"],
            catch_exceptions=False,
        )
        assert reorder_out.exit_code == 0, reorder_out.stdout
        reordered = json.loads(reorder_out.stdout)
        assert reordered["change_ids"][0] == change2["change_id"]

        _make_landable(change1["change_id"], patchset1["patchset_id"])
        _make_landable(change2["change_id"], patchset2["patchset_id"])

        ready_out = runner.invoke(app, ["stack", "show", stack["stack_id"], "--json"], catch_exceptions=False)
        assert ready_out.exit_code == 0, ready_out.stdout
        ready = json.loads(ready_out.stdout)
        assert ready["status"] == "ready_to_land"

        land_out = runner.invoke(
            app,
            ["land", "submit", change1["change_id"], "--patchset", patchset1["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        landed = json.loads(land_out.stdout)
        assert landed["status"] == "succeeded"

        blocked_out = runner.invoke(app, ["stack", "show", stack["stack_id"], "--json"], catch_exceptions=False)
        assert blocked_out.exit_code == 0, blocked_out.stdout
        blocked = json.loads(blocked_out.stdout)
        assert blocked["status"] == "blocked"

        change2_out = runner.invoke(app, ["change", "show", change2["change_id"], "--json"], catch_exceptions=False)
        assert change2_out.exit_code == 0, change2_out.stdout
        change2_state = json.loads(change2_out.stdout)
        assert change2_state["status"] == "blocked"



def test_native_strict_auth_uses_persisted_role_bindings(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-auth"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-auth", auth_mode="strict") as base_url:
        monkeypatch.setenv("AIT_ACTOR", "bootstrap@example.com")
        monkeypatch.setenv("AIT_ROLES", "operator")
        monkeypatch.setenv("AIT_REPOS", "*")
        _bootstrap_main(repo, base_url, monkeypatch)
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"], catch_exceptions=False).exit_code == 0

        grant_out = runner.invoke(
            app,
            [
                "auth", "grant",
                "--actor", "dev@example.com",
                "--role", "repo_contributor",
                "--role", "repo_reviewer",
                "--repo", "housekeeper",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert grant_out.exit_code == 0, grant_out.stdout

        monkeypatch.setenv("AIT_ACTOR", "dev@example.com")
        monkeypatch.delenv("AIT_ROLES", raising=False)
        monkeypatch.delenv("AIT_REPOS", raising=False)

        whoami_out = runner.invoke(app, ["auth", "whoami", "--repo", "housekeeper", "--json"], catch_exceptions=False)
        assert whoami_out.exit_code == 0, whoami_out.stdout
        whoami = json.loads(whoami_out.stdout)
        assert "repo_contributor" in whoami["effective_roles"]
        assert "repo_reviewer" in whoami["effective_roles"]

        monkeypatch.setenv("AIT_ACTOR", "outsider@example.com")
        denied_out = runner.invoke(app, ["task", "start", "--task-only", "--title", "denied", "--intent", "deny", "--risk", "medium"], catch_exceptions=False)
        assert denied_out.exit_code != 0
        denied_text = denied_out.stdout + (denied_out.stderr or "")
        assert "lacks permission" in denied_text

        monkeypatch.setenv("AIT_ACTOR", "dev@example.com")
        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "strict task", "--intent", "prove bindings", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout


def test_native_strict_auth_can_fall_back_to_configured_user_email(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-auth-config"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-auth-config", auth_mode="strict") as base_url:
        monkeypatch.setenv("AIT_ACTOR", "bootstrap@example.com")
        monkeypatch.setenv("AIT_ROLES", "operator")
        monkeypatch.setenv("AIT_REPOS", "*")
        _bootstrap_main(repo, base_url, monkeypatch)
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"], catch_exceptions=False).exit_code == 0

        grant_out = runner.invoke(
            app,
            [
                "auth",
                "grant",
                "--actor",
                "dev@example.com",
                "--role",
                "repo_contributor",
                "--role",
                "repo_reviewer",
                "--repo",
                "housekeeper",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert grant_out.exit_code == 0, grant_out.stdout

        set_out = runner.invoke(
            app,
            ["config", "set", "--user-name", "Dev Example", "--user-email", "dev@example.com", "--json"],
            catch_exceptions=False,
        )
        assert set_out.exit_code == 0, set_out.stdout

        monkeypatch.delenv("AIT_ACTOR", raising=False)
        monkeypatch.delenv("AIT_ROLES", raising=False)
        monkeypatch.delenv("AIT_REPOS", raising=False)

        whoami_out = runner.invoke(app, ["auth", "whoami", "--repo", "housekeeper", "--json"], catch_exceptions=False)
        assert whoami_out.exit_code == 0, whoami_out.stdout
        whoami = json.loads(whoami_out.stdout)
        assert whoami["identity"] == "dev@example.com"
        assert "repo_contributor" in whoami["effective_roles"]
        assert "repo_reviewer" in whoami["effective_roles"]

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "strict config task", "--intent", "use config actor", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
