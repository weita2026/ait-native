from __future__ import annotations

import json
from pathlib import Path

from ._shared import *  # noqa: F401,F403


def test_stash_save_apply_drop_and_snapshot_history_stay_local_first(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-stash"
    repo.mkdir()
    tracked = repo / "app.py"
    created = repo / "notes.txt"
    monkeypatch.chdir(repo)

    tracked.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-stash"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout
    seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]

    tracked.write_text("print('wip')\n", encoding="utf-8")
    created.write_text("draft\n", encoding="utf-8")

    save_out = runner.invoke(app, ["stash", "save", "--message", "wip stash", "--json"], catch_exceptions=False)
    assert save_out.exit_code == 0, save_out.stdout
    saved = json.loads(save_out.stdout)
    assert saved["stash_id"].startswith("STH-")
    assert saved["snapshot_kind"] == "stash"
    assert saved["current_line"] == "main"
    assert saved["line_head_snapshot_id_before"] == seed_snapshot_id
    assert saved["line_head_snapshot_id_after"] == seed_snapshot_id
    assert saved["workspace_cleared"] is True
    assert tracked.read_text(encoding="utf-8") == "print('base')\n"
    assert not created.exists()

    line_out = runner.invoke(app, ["line", "show", "main", "--json"], catch_exceptions=False)
    assert line_out.exit_code == 0, line_out.stdout
    assert json.loads(line_out.stdout)["head_snapshot_id"] == seed_snapshot_id

    snapshot_list_out = runner.invoke(app, ["snapshot", "list", "--json"], catch_exceptions=False)
    assert snapshot_list_out.exit_code == 0, snapshot_list_out.stdout
    listed_snapshots = json.loads(snapshot_list_out.stdout)
    assert [row["snapshot_id"] for row in listed_snapshots] == [seed_snapshot_id]

    list_out = runner.invoke(app, ["stash", "list", "--json"], catch_exceptions=False)
    assert list_out.exit_code == 0, list_out.stdout
    listed = json.loads(list_out.stdout)
    assert [row["stash_id"] for row in listed] == [saved["stash_id"]]
    assert listed[0]["snapshot_id"] == saved["snapshot_id"]
    assert listed[0]["message"] == "wip stash"

    show_out = runner.invoke(app, ["stash", "show", saved["stash_id"], "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["stash_id"] == saved["stash_id"]
    assert shown["snapshot_id"] == saved["snapshot_id"]

    apply_out = runner.invoke(app, ["stash", "apply", saved["stash_id"], "--json"], catch_exceptions=False)
    assert apply_out.exit_code == 0, apply_out.stdout
    applied = json.loads(apply_out.stdout)
    assert applied["applied"] is True
    assert applied["dropped"] is False
    assert applied["line_head_snapshot_id_before"] == seed_snapshot_id
    assert applied["line_head_snapshot_id_after"] == seed_snapshot_id
    assert tracked.read_text(encoding="utf-8") == "print('wip')\n"
    assert created.read_text(encoding="utf-8") == "draft\n"

    drop_out = runner.invoke(app, ["stash", "drop", saved["stash_id"], "--json"], catch_exceptions=False)
    assert drop_out.exit_code == 0, drop_out.stdout
    dropped = json.loads(drop_out.stdout)
    assert dropped["dropped"] is True

    final_list_out = runner.invoke(app, ["stash", "list", "--json"], catch_exceptions=False)
    assert final_list_out.exit_code == 0, final_list_out.stdout
    assert json.loads(final_list_out.stdout) == []


def test_stash_pop_restores_workspace_and_removes_stash(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-stash-pop"
    repo.mkdir()
    tracked = repo / "app.py"
    monkeypatch.chdir(repo)

    tracked.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-stash-pop"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout
    seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]

    tracked.write_text("print('saved but kept')\n", encoding="utf-8")
    save_out = runner.invoke(
        app,
        ["stash", "save", "--message", "keep", "--keep-workspace", "--json"],
        catch_exceptions=False,
    )
    assert save_out.exit_code == 0, save_out.stdout
    saved = json.loads(save_out.stdout)
    assert saved["workspace_cleared"] is False
    assert tracked.read_text(encoding="utf-8") == "print('saved but kept')\n"

    tracked.write_text("print('different local change')\n", encoding="utf-8")
    blocked_pop_out = runner.invoke(app, ["stash", "pop", saved["stash_id"], "--json"], catch_exceptions=False)
    assert blocked_pop_out.exit_code != 0
    assert "Workspace has unsaved changes" in (blocked_pop_out.output or blocked_pop_out.stdout or blocked_pop_out.stderr or "")

    pop_out = runner.invoke(app, ["stash", "pop", saved["stash_id"], "--force", "--json"], catch_exceptions=False)
    assert pop_out.exit_code == 0, pop_out.stdout
    popped = json.loads(pop_out.stdout)
    assert popped["applied"] is True
    assert popped["dropped"] is True
    assert popped["line_head_snapshot_id_before"] == seed_snapshot_id
    assert popped["line_head_snapshot_id_after"] == seed_snapshot_id
    assert tracked.read_text(encoding="utf-8") == "print('saved but kept')\n"

    final_list_out = runner.invoke(app, ["stash", "list", "--json"], catch_exceptions=False)
    assert final_list_out.exit_code == 0, final_list_out.stdout
    assert json.loads(final_list_out.stdout) == []


def test_stash_save_rejects_clean_workspace(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-stash-clean"
    repo.mkdir()
    tracked = repo / "app.py"
    monkeypatch.chdir(repo)

    tracked.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-stash-clean"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0

    clean_out = runner.invoke(app, ["stash", "save"], catch_exceptions=False)
    assert clean_out.exit_code != 0
    assert "Workspace is already clean" in (clean_out.output or clean_out.stdout or clean_out.stderr or "")
