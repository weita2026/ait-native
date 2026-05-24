from __future__ import annotations

import importlib
import json
from pathlib import Path

from ait.cli import runtime_inspection_views
from ait.repo_paths import RepoContext

from ._shared import app, runner

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_runtime_inspection_helpers() -> None:
    helper_names = [
        "_local_auth_snapshot",
        "_storage_validation_view",
        "_history_rows",
        "_config_summary",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(runtime_inspection_views, name)


def test_runtime_inspection_history_rows_line_contract(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-runtime-inspection-history"
    repo.mkdir()
    source = repo / "app.py"
    source.write_text("print('seed')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper-runtime-inspection-history"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout

    first_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert first_out.exit_code == 0, first_out.stdout
    first_snapshot = json.loads(first_out.stdout)

    source.write_text("print('updated')\n", encoding="utf-8")
    second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
    assert second_out.exit_code == 0, second_out.stdout
    second_snapshot = json.loads(second_out.stdout)

    ctx = RepoContext.discover(repo)
    rows = runtime_inspection_views._history_rows(ctx, line_name="main")

    assert [row["snapshot_id"] for row in rows[:2]] == [
        second_snapshot["snapshot_id"],
        first_snapshot["snapshot_id"],
    ]
    assert rows[0]["graph"] == "@"
    assert rows[0]["is_current_head"] is True
    assert rows[0]["is_selected_line_head"] is True
    assert rows[1]["graph"] == "|"
    assert rows[1]["is_current_head"] is False
    assert rows[1]["is_selected_line_head"] is False
