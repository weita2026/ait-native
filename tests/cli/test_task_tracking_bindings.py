from __future__ import annotations

import importlib
import json
from pathlib import Path

from ait.cli import task_tracking_bindings
from ait.repo_paths import RepoContext

from ._shared import _bind_task_worktree, app, runner

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_task_tracking_binding_helpers() -> None:
    helper_names = [
        "TRACKED_SESSION_CONFIG_KEYS",
        "TRACKED_SESSION_SCOPES",
        "_clear_tracked_session_binding",
        "_clear_tracked_session_binding_if_matches",
        "_default_line_name",
        "_set_tracked_session_binding",
        "_task_tracking_enabled",
        "_task_tracking_hard_disabled",
        "_task_tracking_mode",
        "_task_worktree_repo_ctx",
        "_touch_worktree_usage_safely",
        "_tracked_session_binding",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(task_tracking_bindings, name)


def test_task_tracking_binding_contract(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-tracking-bindings"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--task-tracking", "on"], catch_exceptions=False).exit_code == 0

    ctx = RepoContext.discover()
    assert task_tracking_bindings._task_tracking_mode(ctx) == "on"
    assert task_tracking_bindings._task_tracking_enabled(ctx) is True
    assert task_tracking_bindings._task_tracking_hard_disabled(ctx) is False
    assert task_tracking_bindings._tracked_session_binding(ctx) is None

    local_binding = task_tracking_bindings._set_tracked_session_binding(
        ctx,
        task_id="T-LOCAL-1",
        session_id="S-LOCAL-1",
        scope="local",
    )
    assert local_binding == {
        "task_id": "T-LOCAL-1",
        "session_id": "S-LOCAL-1",
        "scope": "local",
        "remote_name": None,
    }

    task_tracking_bindings._clear_tracked_session_binding_if_matches(ctx, task_id="T-OTHER")
    assert task_tracking_bindings._tracked_session_binding(ctx) == local_binding

    remote_binding = task_tracking_bindings._set_tracked_session_binding(
        ctx,
        task_id="RT-REMOTE-1",
        session_id="S-REMOTE-1",
        scope="remote",
        remote_name="origin",
    )
    assert remote_binding == {
        "task_id": "RT-REMOTE-1",
        "session_id": "S-REMOTE-1",
        "scope": "remote",
        "remote_name": "origin",
    }

    show_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    assert json.loads(show_out.stdout)["tracked_session"] == remote_binding

    task_tracking_bindings._clear_tracked_session_binding_if_matches(ctx, session_id="S-REMOTE-1")
    assert task_tracking_bindings._tracked_session_binding(ctx) is None

    assert runner.invoke(app, ["config", "set", "--task-tracking", "off"], catch_exceptions=False).exit_code == 0
    assert task_tracking_bindings._task_tracking_mode(ctx) == "off"
    assert task_tracking_bindings._task_tracking_enabled(ctx) is False
    assert task_tracking_bindings._task_tracking_hard_disabled(ctx) is True


def test_task_tracking_worktree_repo_context_and_touch_contract(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-tracking-worktree"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(
        app,
        ["init", "--name", "housekeeper", "--default-line", "release-main", "--json"],
        catch_exceptions=False,
    )
    assert init_out.exit_code == 0, init_out.stdout
    assert runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    ).exit_code == 0

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Track worktree context",
            "--intent",
            "exercise extracted worktree binding helpers",
            "--risk",
            "low",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    worktree_path = _bind_task_worktree(task["task_id"], monkeypatch, chdir=False)

    root_ctx = RepoContext.discover(repo)
    worktree_ctx = RepoContext.discover(worktree_path)

    assert task_tracking_bindings._default_line_name(root_ctx) == "release-main"
    assert task_tracking_bindings._task_worktree_repo_ctx(root_ctx).root == root_ctx.root

    repo_ctx = task_tracking_bindings._task_worktree_repo_ctx(worktree_ctx)
    assert worktree_ctx.is_worktree is True
    assert repo_ctx.is_worktree is False
    assert repo_ctx.root == root_ctx.root
    assert repo_ctx.repo_root == root_ctx.repo_root

    touched_from_root = task_tracking_bindings._touch_worktree_usage_safely(root_ctx)
    assert touched_from_root is not None
    touched = task_tracking_bindings._touch_worktree_usage_safely(worktree_ctx)
    assert touched is not None
