from __future__ import annotations

import importlib
from pathlib import Path

from ait.cli import task_worktree_guidance
from ait.repo_paths import RepoContext

from ._shared import app, runner

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_task_worktree_guidance_helpers() -> None:
    helper_names = [
        "_attach_task_worktree_guidance",
        "_render_task_worktree_guidance",
        "_task_auto_worktree_source_status",
        "_task_worktree_guidance",
        "_task_worktree_output",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(task_worktree_guidance, name)


def test_task_worktree_output_and_guidance_contract(tmp_path: Path, monkeypatch, capsys) -> None:
    repo = tmp_path / "housekeeper-task-worktree-guidance"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    ctx = RepoContext.discover()

    alias_root = repo / ".ait" / "worktree-links" / "rt-demo"
    alias_root.mkdir(parents=True)
    worktree = task_worktree_guidance._task_worktree_output(
        {
            "name": "rt-demo",
            "alias_path": str(alias_root),
            "path": str(tmp_path / "actual-worktree"),
            "current_line": "feature/rt-demo",
        }
    )
    assert worktree["open_path"] == str(alias_root)
    assert worktree["cd_command"] == f"cd {alias_root}"
    assert "AIT_WORKTREE_NAME=rt-demo" in worktree["shell_command"]

    source_status = {
        "clean": False,
        "changed_count": 6,
        "changed_paths": [
            "app.py",
            "pkg/a.py",
            "pkg/b.py",
            "pkg/c.py",
            "pkg/d.py",
            "pkg/e.py",
        ],
    }
    guidance = task_worktree_guidance._task_worktree_guidance(
        ctx,
        worktree,
        source_workspace_status=source_status,
    )
    assert guidance is not None
    assert guidance["switch_required"] is True
    assert guidance["target_workspace_root"] == str(alias_root)
    assert guidance["source_workspace_changed_count"] == 6
    assert guidance["source_workspace_changed_paths"] == source_status["changed_paths"]
    assert "Current workspace had 6 changed path(s):" in guidance["source_workspace_summary"]
    assert "+1 more" in guidance["source_workspace_summary"]
    assert "Existing workspace changes were not copied into the new task worktree." in guidance["dirty_source_warning"]

    payload = task_worktree_guidance._attach_task_worktree_guidance(
        ctx,
        {"task_id": "T-DEMO-1"},
        worktree=worktree,
        source_workspace_status=source_status,
    )
    assert payload["worktree_guidance"] == guidance

    task_worktree_guidance._render_task_worktree_guidance(guidance)
    rendered = capsys.readouterr().out
    assert "task worktree:" in rendered
    assert "Continue in the task worktree with:" in rendered
    assert "Existing workspace changes were not copied into the new task worktree." in rendered


def test_task_auto_worktree_source_status_tracks_workspace_drift(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-auto-worktree-source-status"
    repo.mkdir()
    tracked = repo / "app.py"
    tracked.write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout

    ctx = RepoContext.discover()
    clean_status = task_worktree_guidance._task_auto_worktree_source_status(ctx)
    assert clean_status is not None
    assert clean_status["clean"] is True

    tracked.write_text("print('changed')\n", encoding="utf-8")
    dirty_status = task_worktree_guidance._task_auto_worktree_source_status(ctx)
    assert dirty_status is not None
    assert dirty_status["clean"] is False
    assert "app.py" in dirty_status["changed_paths"]
