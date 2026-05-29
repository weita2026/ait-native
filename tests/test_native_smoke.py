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


@contextmanager
def running_server(data_dir: Path):
    old = os.environ.get("AIT_NATIVE_SERVER_DATA")
    old_backend = os.environ.get("AIT_NATIVE_SERVER_DB_BACKEND")
    old_dsn = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN")
    old_content_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA")
    old_control_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA")
    install_fake_psycopg_global()
    reset_fake_postgres_runtime()
    os.environ["AIT_NATIVE_SERVER_DATA"] = str(data_dir)
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
        if old is None:
            os.environ.pop("AIT_NATIVE_SERVER_DATA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DATA"] = old
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


def test_native_local_draft_workflow_smoke(tmp_path: Path, monkeypatch):
    repo = tmp_path / "local-draft-smoke"
    repo.mkdir()
    readme = repo / "README.md"
    readme.write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-local-draft-smoke") as base_url:
        monkeypatch.chdir(repo)

        init_out = runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False)
        assert init_out.exit_code == 0, init_out.stdout
        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        main_snapshot = json.loads(main_snap_out.stdout)
        assert (
            runner.invoke(
                app,
                ["config", "set", "--plan-task-binding-mode", "advisory"],
                catch_exceptions=False,
            ).exit_code
            == 0
        )

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Smoke local draft",
                "--intent",
                "exercise local draft publication flow",
                "--change-title",
                "Publish local draft smoke flow",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        started = json.loads(start_out.stdout)
        task_id = started["task_id"]
        change = started["change"]
        worktree_path = Path(started["worktree"]["path"])

        with monkeypatch.context() as worktree_context:
            worktree_context.chdir(worktree_path)
            (worktree_path / "app.py").write_text("print('local draft smoke')\n", encoding="utf-8")
            (worktree_path / "notes.txt").write_text("local draft smoke\n", encoding="utf-8")
            feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"], catch_exceptions=False)
            assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
            feature_snapshot = json.loads(feature_snap_out.stdout)
            assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

            push_out = runner.invoke(app, ["push", "--line", "main", "--json"], catch_exceptions=False)
            assert push_out.exit_code == 0, push_out.stdout
            assert json.loads(push_out.stdout)["head_snapshot_id"] == main_snapshot["snapshot_id"]

            task_publish_out = runner.invoke(app, ["task", "publish", task_id, "--json"], catch_exceptions=False)
            assert task_publish_out.exit_code == 0, task_publish_out.stdout
            published_task = json.loads(task_publish_out.stdout)
            assert published_task["publication_state"] == "published"
            assert published_task["published_task_id"] != task_id
            assert published_task["published_task_id"].startswith("R")
            assert published_task["published_remote_name"] == "origin"
            assert isinstance(published_task["published_at"], str) and published_task["published_at"]

            change_publish_out = runner.invoke(app, ["change", "publish", change["change_id"], "--json"], catch_exceptions=False)
            assert change_publish_out.exit_code == 0, change_publish_out.stdout
            published_change = json.loads(change_publish_out.stdout)
            assert published_change["publication_state"] == "published"
            assert published_change["identity_source"] == change["identity_source"]
            assert published_change["published_change_id"] != change["change_id"]
            assert published_change["published_change_id"].startswith("R")
            assert published_change["published_remote_name"] == "origin"
            assert isinstance(published_change["published_at"], str) and published_change["published_at"]

            patchset_out = runner.invoke(
                app,
                ["patchset", "publish", "--change", change["change_id"], "--summary", "local draft smoke patchset", "--json"],
                catch_exceptions=False,
            )
            assert patchset_out.exit_code == 0, patchset_out.stdout
            patchset = json.loads(patchset_out.stdout)
            assert patchset["base_snapshot_id"] == main_snapshot["snapshot_id"]
            assert patchset["revision_snapshot_id"] == feature_snapshot["snapshot_id"]
            assert patchset["patchset_number"] == 1

            patchset_show_out = runner.invoke(app, ["patchset", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
            assert patchset_show_out.exit_code == 0, patchset_show_out.stdout
            shown = json.loads(patchset_show_out.stdout)
        assert shown["diff_stats"]["files_changed"] == 2


def test_native_remote_review_and_pull_workflow_smoke(tmp_path: Path, monkeypatch):
    repo1 = tmp_path / "remote-smoke-source"
    repo1.mkdir()
    readme1 = repo1 / "README.md"
    readme1.write_text("base\n", encoding="utf-8")

    repo2 = tmp_path / "remote-smoke-target"
    repo2.mkdir()

    with running_server(tmp_path / "server-data-remote-smoke") as base_url:
        monkeypatch.chdir(repo1)

        assert runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        main_snapshot = json.loads(main_snap_out.stdout)

        main_push_out = runner.invoke(app, ["push", "--line", "main", "--json"], catch_exceptions=False)
        assert main_push_out.exit_code == 0, main_push_out.stdout
        assert json.loads(main_push_out.stdout)["head_snapshot_id"] == main_snapshot["snapshot_id"]
        assert (
            runner.invoke(
                app,
                ["config", "set", "--plan-task-binding-mode", "advisory"],
                catch_exceptions=False,
            ).exit_code
            == 0
        )

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Smoke remote workflow",
                "--intent",
                "exercise end-to-end remote review and pull flow",
                "--change-title",
                "Review remote smoke flow",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        started = json.loads(start_out.stdout)
        change = started["change"]
        worktree = started["worktree"]
        worktree_path = Path(worktree["path"])
        feature_line_name = str(worktree["registered_line_name"])

        with monkeypatch.context() as worktree_context:
            worktree_context.chdir(worktree_path)
            (worktree_path / "app.py").write_text("print('remote smoke')\n", encoding="utf-8")
            (worktree_path / "notes.txt").write_text("remote smoke\n", encoding="utf-8")
            feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"], catch_exceptions=False)
            assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
            feature_snapshot = json.loads(feature_snap_out.stdout)

            feature_push_out = runner.invoke(app, ["push", "--line", feature_line_name, "--json"], catch_exceptions=False)
            assert feature_push_out.exit_code == 0, feature_push_out.stdout
            feature_push = json.loads(feature_push_out.stdout)
            assert feature_push["head_snapshot_id"] == feature_snapshot["snapshot_id"]
            assert feature_push["remote_line"]["line_name"] == feature_line_name

            patchset_out = runner.invoke(
                app,
                ["patchset", "publish", "--change", change["change_id"], "--summary", "remote smoke patchset", "--json"],
                catch_exceptions=False,
            )
            assert patchset_out.exit_code == 0, patchset_out.stdout
            patchset = json.loads(patchset_out.stdout)
            patchset_show_out = runner.invoke(app, ["patchset", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
            assert patchset_show_out.exit_code == 0, patchset_show_out.stdout
            shown = json.loads(patchset_show_out.stdout)
        assert patchset["base_snapshot_id"] == main_snapshot["snapshot_id"]
        assert patchset["revision_snapshot_id"] == feature_snapshot["snapshot_id"]
        assert shown["diff_stats"]["files_changed"] == 2

        monkeypatch.chdir(repo2)
        assert runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "target seed", "--json"], catch_exceptions=False).exit_code == 0

        pull_out = runner.invoke(app, ["pull", "--line", feature_line_name, "--json"], catch_exceptions=False)
        assert pull_out.exit_code == 0, pull_out.stdout
        pulled = json.loads(pull_out.stdout)
        assert pulled["line"] == feature_line_name
        assert pulled["head_snapshot_id"] == feature_snapshot["snapshot_id"]

        pulled_line_out = runner.invoke(app, ["line", "show", feature_line_name, "--json"], catch_exceptions=False)
        assert pulled_line_out.exit_code == 0, pulled_line_out.stdout
        pulled_line = json.loads(pulled_line_out.stdout)
        assert pulled_line["head_snapshot_id"] == feature_snapshot["snapshot_id"]

        switch_out = runner.invoke(app, ["line", "switch", feature_line_name, "--restore", "--json"], catch_exceptions=False)
        assert switch_out.exit_code == 0, switch_out.stdout
        switched = json.loads(switch_out.stdout)
        assert switched["current_line"] == feature_line_name
        assert switched["line_head_snapshot_id"] == feature_snapshot["snapshot_id"]

        status_out = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
        assert status_out.exit_code == 0, status_out.stdout
        status = json.loads(status_out.stdout)
        assert status["current_line"] == feature_line_name
        assert status["head_snapshot_id"] == feature_snapshot["snapshot_id"]

        assert (repo2 / "app.py").read_text(encoding="utf-8") == "print('remote smoke')\n"
        assert (repo2 / "notes.txt").read_text(encoding="utf-8") == "remote smoke\n"
