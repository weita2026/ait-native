from __future__ import annotations

import json
from pathlib import Path

from ._shared import *  # noqa: F401,F403


def test_snapshot_replay_replays_snapshot_delta_onto_current_line_workspace(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-snapshot-replay"
    repo.mkdir()
    tracked = repo / "app.py"
    removed = repo / "obsolete.txt"
    tracked.write_text("print('base')\n", encoding="utf-8")
    removed.write_text("old\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-snapshot-replay"], catch_exceptions=False).exit_code == 0

    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout
    seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]

    assert runner.invoke(app, ["line", "create", "feature/source"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/source", "--restore"], catch_exceptions=False).exit_code == 0

    tracked.write_text("print('source replay')\n", encoding="utf-8")
    (repo / "notes.txt").write_text("replayed\n", encoding="utf-8")
    removed.unlink()
    source_out = runner.invoke(app, ["snapshot", "create", "--message", "source replay", "--json"], catch_exceptions=False)
    assert source_out.exit_code == 0, source_out.stdout
    source_snapshot_id = json.loads(source_out.stdout)["snapshot_id"]

    switch_main = runner.invoke(app, ["line", "switch", "main", "--restore", "--json"], catch_exceptions=False)
    assert switch_main.exit_code == 0, switch_main.stdout

    dry_run_out = runner.invoke(
        app,
        ["snapshot", "replay", source_snapshot_id, "--onto", "main", "--dry-run", "--json"],
        catch_exceptions=False,
    )
    assert dry_run_out.exit_code == 0, dry_run_out.stdout
    dry_run_payload = json.loads(dry_run_out.stdout)
    assert dry_run_payload["applied"] is False
    assert dry_run_payload["snapshot_id"] == source_snapshot_id
    assert dry_run_payload["parent_snapshot_id"] == seed_snapshot_id
    assert dry_run_payload["onto_line"] == "main"
    assert dry_run_payload["onto_line_head_snapshot_id"] == seed_snapshot_id
    assert dry_run_payload["mutation_scope"] == "workspace_only"
    assert dry_run_payload["moves_line_head"] is False
    assert dry_run_payload["creates_snapshot"] is False
    assert dry_run_payload["affected_paths"] == ["app.py", "notes.txt", "obsolete.txt"]
    assert dry_run_payload["delta_summary"]["deleted"] == ["obsolete.txt"]
    assert dry_run_payload["plan"]["write_paths"] == ["app.py", "notes.txt"]
    assert dry_run_payload["plan"]["remove_paths"] == ["obsolete.txt"]

    replay_out = runner.invoke(
        app,
        ["snapshot", "replay", source_snapshot_id, "--onto", "main", "--json"],
        catch_exceptions=False,
    )
    assert replay_out.exit_code == 0, replay_out.stdout
    replayed = json.loads(replay_out.stdout)
    assert replayed["applied"] is True
    assert tracked.read_text(encoding="utf-8") == "print('source replay')\n"
    assert (repo / "notes.txt").read_text(encoding="utf-8") == "replayed\n"
    assert not removed.exists()

    line_out = runner.invoke(app, ["line", "show", "main", "--json"], catch_exceptions=False)
    assert line_out.exit_code == 0, line_out.stdout
    assert json.loads(line_out.stdout)["head_snapshot_id"] == seed_snapshot_id


def test_change_replay_replays_recorded_change_delta_onto_current_line_workspace(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-change-replay"
    repo.mkdir()
    tracked = repo / "app.py"
    removed = repo / "obsolete.txt"
    tracked.write_text("print('base')\n", encoding="utf-8")
    removed.write_text("old\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-change-replay"], catch_exceptions=False).exit_code == 0

    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout
    seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]
    assert runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory"],
        catch_exceptions=False,
    ).exit_code == 0

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--title",
            "Add agent-first replay",
            "--intent",
            "exercise change replay from a task-bound worktree",
            "--base-line",
            "main",
            "--risk",
            "medium",
            "--local",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    change_id = payload["change"]["change_id"]
    worktree_path = Path(payload["worktree"]["path"])

    with monkeypatch.context() as worktree_context:
        worktree_context.chdir(worktree_path)
        feature_file = worktree_path / "app.py"
        removed_file = worktree_path / "obsolete.txt"
        feature_file.write_text("print('change replay')\n", encoding="utf-8")
        (worktree_path / "notes.txt").write_text("from change replay\n", encoding="utf-8")
        removed_file.unlink()

        head_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"], catch_exceptions=False)
        assert head_out.exit_code == 0, head_out.stdout
        head_snapshot_id = json.loads(head_out.stdout)["snapshot_id"]

        switch_main = runner.invoke(app, ["line", "switch", "main", "--restore", "--json"], catch_exceptions=False)
        assert switch_main.exit_code == 0, switch_main.stdout

        dry_run_out = runner.invoke(
            app,
            ["change", "replay", change_id, "--onto", "main", "--local", "--dry-run", "--json"],
            catch_exceptions=False,
        )
        assert dry_run_out.exit_code == 0, dry_run_out.stdout
        dry_run_payload = json.loads(dry_run_out.stdout)
        assert dry_run_payload["applied"] is False
        assert dry_run_payload["change_id"] == change_id
        assert dry_run_payload["fork_snapshot_id"] == seed_snapshot_id
        assert dry_run_payload["latest_change_snapshot_id"] == head_snapshot_id
        assert dry_run_payload["onto_line"] == "main"
        assert dry_run_payload["onto_line_head_snapshot_id"] == seed_snapshot_id
        assert dry_run_payload["mutation_scope"] == "workspace_only"
        assert dry_run_payload["moves_line_head"] is False
        assert dry_run_payload["creates_snapshot"] is False
        assert dry_run_payload["affected_paths"] == ["app.py", "notes.txt", "obsolete.txt"]

        replay_out = runner.invoke(
            app,
            ["change", "replay", change_id, "--onto", "main", "--local", "--json"],
            catch_exceptions=False,
        )
        assert replay_out.exit_code == 0, replay_out.stdout
        replayed = json.loads(replay_out.stdout)
        assert replayed["applied"] is True
        assert feature_file.read_text(encoding="utf-8") == "print('change replay')\n"
        assert (worktree_path / "notes.txt").read_text(encoding="utf-8") == "from change replay\n"
        assert not removed_file.exists()

        line_out = runner.invoke(app, ["line", "show", "main", "--json"], catch_exceptions=False)
        assert line_out.exit_code == 0, line_out.stdout
        assert json.loads(line_out.stdout)["head_snapshot_id"] == seed_snapshot_id
