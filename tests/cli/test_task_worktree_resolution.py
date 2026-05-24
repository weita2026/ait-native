from __future__ import annotations

import importlib
import json
from pathlib import Path

import pytest

from ait.cli import task_worktree_resolution
from ait.repo_paths import RepoContext

from ._shared import app, runner

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_task_worktree_resolution_helpers() -> None:
    helper_names = [
        "_ensure_task_feature_line",
        "_find_auto_created_task_worktree",
        "_find_bound_task_worktree",
        "_resolve_task_bound_worktree_name",
        "_session_bound_worktree",
        "_task_bound_worktree_name",
        "_task_feature_line_name",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(task_worktree_resolution, name)


def test_task_worktree_resolution_naming_and_feature_line_contract(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-worktree-resolution"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    assert runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    ).exit_code == 0

    ctx = RepoContext.discover()

    assert task_worktree_resolution._task_bound_worktree_name("RT-Feature_1") == "rt-feature-1"
    assert task_worktree_resolution._task_feature_line_name("RT-Feature_1") == "feature/rt-feature-1"

    with pytest.raises(ValueError, match="Task id is required"):
        task_worktree_resolution._task_bound_worktree_name("!!!")

    with pytest.raises(ValueError, match="Task id is required"):
        task_worktree_resolution._task_feature_line_name("!!!")

    feature_line = task_worktree_resolution._ensure_task_feature_line(
        ctx,
        task_id="RT-DEMO",
        base_line_name="main",
    )
    assert feature_line["line_name"] == "feature/rt-demo"
    reused_line = task_worktree_resolution._ensure_task_feature_line(
        ctx,
        task_id="RT-DEMO",
        base_line_name="main",
    )
    assert reused_line["line_name"] == feature_line["line_name"]


def test_task_worktree_resolution_lookup_and_session_contract(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-worktree-resolution-lookup"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    ).exit_code == 0

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Lookup bound worktree",
            "--intent",
            "exercise extracted task worktree resolution helpers",
            "--base-line",
            "main",
            "--risk",
            "low",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    task_id = str(payload["task_id"])
    change_id = str(payload["change"]["change_id"])
    worktree_name = str(payload["worktree"]["name"])

    ctx = RepoContext.discover()
    bound_worktree = task_worktree_resolution._find_bound_task_worktree(ctx, task_id)
    assert bound_worktree is not None
    assert bound_worktree["name"] == worktree_name

    auto_worktree = task_worktree_resolution._find_auto_created_task_worktree(ctx, task_id)
    assert auto_worktree is not None
    assert auto_worktree["name"] == worktree_name

    resolved_by_task = task_worktree_resolution._session_bound_worktree(
        ctx,
        local=True,
        remote_name=None,
        task_id=task_id,
    )
    assert resolved_by_task is not None
    assert resolved_by_task["name"] == worktree_name

    resolved_by_change = task_worktree_resolution._session_bound_worktree(
        ctx,
        local=True,
        remote_name=None,
        change_id=change_id,
    )
    assert resolved_by_change is not None
    assert resolved_by_change["name"] == worktree_name

    resolved_by_name = task_worktree_resolution._session_bound_worktree(
        ctx,
        local=True,
        remote_name=None,
        worktree_name=worktree_name,
    )
    assert resolved_by_name is not None
    assert resolved_by_name["name"] == worktree_name

    assert task_worktree_resolution._resolve_task_bound_worktree_name(ctx, task_id) == f"{worktree_name}-2"

    with pytest.raises(ValueError, match="Unknown worktree"):
        task_worktree_resolution._session_bound_worktree(
            ctx,
            local=True,
            remote_name=None,
            worktree_name="missing-worktree",
        )


def test_find_bound_task_worktree_uses_registry_metadata_fast_path(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-worktree-fast-path"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    ).exit_code == 0

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Fast metadata lookup",
            "--intent",
            "avoid full worktree refresh scans for bound task lookup",
            "--base-line",
            "main",
            "--risk",
            "low",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    task_id = str(payload["task_id"])
    worktree_name = str(payload["worktree"]["name"])

    ctx = RepoContext.discover()
    refresh_flags: list[bool] = []
    original_get_worktree = task_worktree_resolution.local_get_worktree

    def wrapped_get_worktree(repo_ctx, name, *, refresh_status=True):
        refresh_flags.append(bool(refresh_status))
        return original_get_worktree(repo_ctx, name, refresh_status=refresh_status)

    def fail_list_worktrees(*_args, **_kwargs):
        raise AssertionError("metadata-first bound task lookup should not enumerate all worktrees")

    monkeypatch.setattr(task_worktree_resolution, "local_get_worktree", wrapped_get_worktree)
    monkeypatch.setattr(task_worktree_resolution, "local_list_worktrees", fail_list_worktrees)

    bound_worktree = task_worktree_resolution._find_bound_task_worktree(ctx, task_id)

    assert bound_worktree is not None
    assert bound_worktree["name"] == worktree_name
    assert refresh_flags == [False]
