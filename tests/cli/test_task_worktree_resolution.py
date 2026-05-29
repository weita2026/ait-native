from __future__ import annotations

import importlib
import json
from pathlib import Path

import pytest

from ait.cli import task_worktree_runtime
from ait.cli import task_worktree_resolution
from ait.repo_paths import RepoContext
from ait.store import add_worktree, bind_worktree, create_line, create_snapshot
from ait.store_local_changes import create_local_change
from ait.store_local_tasks import create_local_task

from ._shared import _bind_task_worktree, app, runner

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
    assert task_worktree_resolution._legacy_task_bound_worktree_name("RT-Feature_1") == "task-rt-feature-1"
    assert task_worktree_resolution._legacy_task_feature_line_name("RT-Feature_1") == "feature/task-rt-feature-1"

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


def test_shared_bind_task_worktree_defaults_to_normalized_task_id(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-shared-bind-task-worktree-default"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout

    ctx = RepoContext.discover()
    task = create_local_task(ctx, "Shared helper naming", "bind a task worktree without legacy task prefix", "medium")
    task_id = str(task["task_id"])

    worktree_path = _bind_task_worktree(task_id, monkeypatch, chdir=False)
    assert worktree_path.name == task_id.lower()

    bound_worktree = task_worktree_resolution._find_bound_task_worktree(ctx, task_id)
    assert bound_worktree is not None
    assert bound_worktree["name"] == task_id.lower()
    assert bound_worktree["registered_line_name"] == "main"


def test_task_worktree_resolution_reuses_legacy_feature_line_name(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-worktree-resolution-legacy-line"
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
    legacy_line = task_worktree_resolution._legacy_task_feature_line_name("LT-DEMO")
    assert runner.invoke(
        app,
        ["line", "create", legacy_line, "--from-snapshot", json.loads(snap_out.stdout)["snapshot_id"]],
    ).exit_code == 0

    feature_line = task_worktree_resolution._ensure_task_feature_line(
        ctx,
        task_id="LT-DEMO",
        base_line_name="main",
    )
    assert feature_line["line_name"] == legacy_line


def test_task_worktree_resolution_reuses_legacy_bound_worktree_name(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-worktree-resolution-legacy-worktree"
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
    task = create_local_task(ctx, "Legacy worktree reuse", "keep task-* worktrees discoverable", "medium")
    task_id = str(task["task_id"])
    snapshot_id = str(json.loads(snap_out.stdout)["snapshot_id"])
    legacy_line = task_worktree_resolution._legacy_task_feature_line_name(task_id)
    create_line(ctx, legacy_line, snapshot_id)

    legacy_worktree_name = task_worktree_resolution._legacy_task_bound_worktree_name(task_id)
    add_worktree(
        ctx,
        legacy_worktree_name,
        line_name=legacy_line,
        creation_kind="task_auto_created",
        cleanup_policy="after_remote_land",
    )
    bind_worktree(
        ctx,
        legacy_worktree_name,
        task_id=task_id,
        auto_created_for_task=True,
        fork_snapshot_id=snapshot_id,
        forked_from_line="main",
        target_base_line="main",
    )

    bound_worktree = task_worktree_resolution._find_bound_task_worktree(ctx, task_id)
    assert bound_worktree is not None
    assert bound_worktree["name"] == legacy_worktree_name

    resolved_by_task = task_worktree_resolution._session_bound_worktree(
        ctx,
        local=True,
        remote_name=None,
        task_id=task_id,
    )
    assert resolved_by_task is not None
    assert resolved_by_task["name"] == legacy_worktree_name


def test_delayed_task_worktree_materialization_reuses_change_fork_lineage(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-worktree-delayed-materialization"
    repo.mkdir()
    readme = repo / "README.md"
    readme.write_text("base\n", encoding="utf-8")
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
    task = create_local_task(ctx, "Delayed materialization", "reuse change fork lineage", "medium")
    change = create_local_change(ctx, str(task["task_id"]), "Draft delayed materialization", "main", "medium")

    readme.write_text("base\nadvanced main\n", encoding="utf-8")
    advanced_main_snapshot_id = str(create_snapshot(ctx, "advance main")["snapshot_id"])

    worktree = task_worktree_runtime._maybe_auto_create_task_worktree(
        ctx,
        task_id=str(task["task_id"]),
        title=str(task["title"]),
        base_line_name="main",
        change=change,
        change_id=str(change["change_id"]),
    )
    assert worktree is not None
    assert worktree["fork_snapshot_id"] == change["fork_snapshot_id"]
    assert worktree["fork_snapshot_id"] != advanced_main_snapshot_id
    assert worktree["target_base_snapshot_id"] == advanced_main_snapshot_id
    assert worktree["registered_line_name"] == f"feature/{str(task['task_id']).lower()}"


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
