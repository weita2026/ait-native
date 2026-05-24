from __future__ import annotations

import json
from pathlib import Path

from ._shared import *  # noqa: F401,F403


def test_snapshot_revert_restores_current_head_parent_without_moving_line_head(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-snapshot-revert"
    repo.mkdir()
    tracked = repo / "app.py"
    created = repo / "notes.txt"
    monkeypatch.chdir(repo)

    tracked.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-snapshot-revert"], catch_exceptions=False).exit_code == 0

    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout
    seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]

    tracked.write_text("print('feature')\n", encoding="utf-8")
    created.write_text("draft\n", encoding="utf-8")
    head_out = runner.invoke(app, ["snapshot", "create", "--message", "feature", "--json"], catch_exceptions=False)
    assert head_out.exit_code == 0, head_out.stdout
    head_snapshot_id = json.loads(head_out.stdout)["snapshot_id"]

    dry_run_out = runner.invoke(
        app,
        ["snapshot", "revert", head_snapshot_id, "--dry-run", "--json"],
        catch_exceptions=False,
    )
    assert dry_run_out.exit_code == 0, dry_run_out.stdout
    dry_run_payload = json.loads(dry_run_out.stdout)
    assert dry_run_payload["applied"] is False
    assert dry_run_payload["snapshot_id"] == head_snapshot_id
    assert dry_run_payload["parent_snapshot_id"] == seed_snapshot_id
    assert dry_run_payload["mutation_scope"] == "workspace_only"
    assert dry_run_payload["moves_line_head"] is False
    assert dry_run_payload["creates_snapshot"] is False
    assert dry_run_payload["affected_paths"] == ["app.py", "notes.txt"]
    assert dry_run_payload["plan"]["write_paths"] == ["app.py"]
    assert dry_run_payload["plan"]["remove_paths"] == ["notes.txt"]

    revert_out = runner.invoke(app, ["snapshot", "revert", head_snapshot_id, "--json"], catch_exceptions=False)
    assert revert_out.exit_code == 0, revert_out.stdout
    reverted = json.loads(revert_out.stdout)
    assert reverted["applied"] is True
    assert reverted["current_line_head_snapshot_id"] == head_snapshot_id
    assert tracked.read_text(encoding="utf-8") == "print('base')\n"
    assert not created.exists()

    line_out = runner.invoke(app, ["line", "show", "main", "--json"], catch_exceptions=False)
    assert line_out.exit_code == 0, line_out.stdout
    assert json.loads(line_out.stdout)["head_snapshot_id"] == head_snapshot_id


def test_snapshot_revert_refuses_non_head_snapshot(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-snapshot-revert-guard"
    repo.mkdir()
    tracked = repo / "app.py"
    monkeypatch.chdir(repo)

    tracked.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-snapshot-revert-guard"], catch_exceptions=False).exit_code == 0

    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout
    seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]

    tracked.write_text("print('feature')\n", encoding="utf-8")
    head_out = runner.invoke(app, ["snapshot", "create", "--message", "feature", "--json"], catch_exceptions=False)
    assert head_out.exit_code == 0, head_out.stdout
    head_snapshot_id = json.loads(head_out.stdout)["snapshot_id"]

    blocked_out = runner.invoke(app, ["snapshot", "revert", seed_snapshot_id], catch_exceptions=False)
    assert blocked_out.exit_code != 0
    output = blocked_out.output or blocked_out.stdout or blocked_out.stderr or ""
    assert "current line head snapshot" in output
    assert head_snapshot_id in output


def test_change_revert_uses_remote_change_lineage_and_keeps_line_head(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-change-revert"
    repo.mkdir()
    tracked = repo / "app.py"
    tracked.write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-change-revert") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        _set_solo_remote_advisory()

        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert seed_out.exit_code == 0, seed_out.stdout
        seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Add agent-first change revert",
                "--intent",
                "exercise change revert against remote-backed lineage",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        payload = json.loads(start_out.stdout)
        change_id = payload["change"]["change_id"]
        worktree_path = Path(payload["worktree"]["path"])

        monkeypatch.chdir(worktree_path)
        feature_file = worktree_path / "app.py"
        extra_file = worktree_path / "notes.txt"
        feature_file.write_text("print('feature')\n", encoding="utf-8")
        extra_file.write_text("draft\n", encoding="utf-8")

        head_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"], catch_exceptions=False)
        assert head_out.exit_code == 0, head_out.stdout
        head_snapshot_id = json.loads(head_out.stdout)["snapshot_id"]

        dry_run_out = runner.invoke(
            app,
            ["change", "revert", change_id, "--dry-run", "--json"],
            catch_exceptions=False,
        )
        assert dry_run_out.exit_code == 0, dry_run_out.stdout
        dry_run_payload = json.loads(dry_run_out.stdout)
        assert dry_run_payload["applied"] is False
        assert dry_run_payload["change_id"] == change_id
        assert dry_run_payload["fork_snapshot_id"] == seed_snapshot_id
        assert dry_run_payload["latest_change_snapshot_id"] == head_snapshot_id
        assert dry_run_payload["mutation_scope"] == "workspace_only"
        assert dry_run_payload["moves_line_head"] is False
        assert dry_run_payload["creates_snapshot"] is False
        assert dry_run_payload["affected_paths"] == ["app.py", "notes.txt"]

        revert_out = runner.invoke(app, ["change", "revert", change_id, "--json"], catch_exceptions=False)
        assert revert_out.exit_code == 0, revert_out.stdout
        reverted = json.loads(revert_out.stdout)
        assert reverted["applied"] is True
        assert reverted["change_id"] == change_id
        assert reverted["current_line_head_snapshot_id"] == head_snapshot_id
        assert feature_file.read_text(encoding="utf-8") == "print('base')\n"
        assert not extra_file.exists()

        current_line = str(payload["worktree"]["current_line"])
        line_out = runner.invoke(app, ["line", "show", current_line, "--json"], catch_exceptions=False)
        assert line_out.exit_code == 0, line_out.stdout
        assert json.loads(line_out.stdout)["head_snapshot_id"] == head_snapshot_id
