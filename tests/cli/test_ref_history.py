from __future__ import annotations

import json
from pathlib import Path

from ._shared import *  # noqa: F401,F403


def test_ref_history_reports_snapshot_ancestry_and_move_breadcrumbs(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-ref-history"
    repo.mkdir()
    tracked = repo / "app.py"
    monkeypatch.chdir(repo)

    tracked.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-ref-history"], catch_exceptions=False).exit_code == 0

    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout
    seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]

    tracked.write_text("print('feature')\n", encoding="utf-8")
    head_out = runner.invoke(app, ["snapshot", "create", "--message", "feature", "--json"], catch_exceptions=False)
    assert head_out.exit_code == 0, head_out.stdout
    head_snapshot_id = json.loads(head_out.stdout)["snapshot_id"]

    move_back_out = runner.invoke(app, ["ref", "move", "lines/main", seed_snapshot_id, "--json"], catch_exceptions=False)
    assert move_back_out.exit_code == 0, move_back_out.stdout

    move_forward_out = runner.invoke(app, ["ref", "move", "lines/main", head_snapshot_id, "--json"], catch_exceptions=False)
    assert move_forward_out.exit_code == 0, move_forward_out.stdout

    history_out = runner.invoke(app, ["ref", "history", "--json"], catch_exceptions=False)
    assert history_out.exit_code == 0, history_out.stdout
    payload = json.loads(history_out.stdout)

    assert payload["name"] == "lines/main"
    assert payload["line_name"] == "main"
    assert payload["current_target_snapshot_id"] == head_snapshot_id
    assert payload["snapshot_count"] == 2
    assert [row["snapshot_id"] for row in payload["snapshots"]] == [head_snapshot_id, seed_snapshot_id]
    assert payload["snapshots"][0]["position_from_head"] == 0
    assert payload["snapshots"][0]["is_current_target"] is True
    assert payload["snapshots"][0]["parent_snapshot_id"] == seed_snapshot_id

    assert payload["move_event_count"] == 2
    assert payload["move_events"][0]["target_snapshot_id"] == head_snapshot_id
    assert payload["move_events"][0]["previous_target_snapshot_id"] == seed_snapshot_id
    assert payload["move_events"][1]["target_snapshot_id"] == seed_snapshot_id
    assert payload["move_events"][1]["previous_target_snapshot_id"] == head_snapshot_id


def test_ref_history_accepts_explicit_line_ref_and_limit(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-ref-history-limit"
    repo.mkdir()
    tracked = repo / "app.py"
    monkeypatch.chdir(repo)

    tracked.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-ref-history-limit"], catch_exceptions=False).exit_code == 0

    first_out = runner.invoke(app, ["snapshot", "create", "--message", "one", "--json"], catch_exceptions=False)
    assert first_out.exit_code == 0, first_out.stdout

    tracked.write_text("print('two')\n", encoding="utf-8")
    second_out = runner.invoke(app, ["snapshot", "create", "--message", "two", "--json"], catch_exceptions=False)
    assert second_out.exit_code == 0, second_out.stdout

    tracked.write_text("print('three')\n", encoding="utf-8")
    third_out = runner.invoke(app, ["snapshot", "create", "--message", "three", "--json"], catch_exceptions=False)
    assert third_out.exit_code == 0, third_out.stdout
    third_snapshot_id = json.loads(third_out.stdout)["snapshot_id"]

    history_out = runner.invoke(app, ["ref", "history", "lines/main", "--limit", "1", "--json"], catch_exceptions=False)
    assert history_out.exit_code == 0, history_out.stdout
    payload = json.loads(history_out.stdout)

    assert payload["name"] == "lines/main"
    assert payload["limit"] == 1
    assert payload["snapshot_count"] == 1
    assert [row["snapshot_id"] for row in payload["snapshots"]] == [third_snapshot_id]
