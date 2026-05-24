from __future__ import annotations

import json

from ._shared import *  # noqa: F401,F403


def test_repo_migrate_command_triggers_admin_migrations_route(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-migrations"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False).exit_code == 0
    base_url = "http://example.invalid"
    assert runner.invoke(
        app,
        ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

    calls: list[tuple[str, str, str]] = []

    monkeypatch.setattr(
        cli_module,
        "remote_run_repo_migrations",
        lambda cfg_url, repo_name: calls.append(("migrate", cfg_url, repo_name))
        or {
            "repo_name": repo_name,
            "migrations": [
                "control_plane_repo_id_backfill",
                "control_plane_local_key_backfill",
            ],
        },
    )

    migrate_out = runner.invoke(app, ["repo", "migrate", "--remote", "origin", "--json"], catch_exceptions=False)
    assert migrate_out.exit_code == 0, migrate_out.stdout
    payload = json.loads(migrate_out.stdout)
    assert payload["repo_name"] == "housekeeper"
    assert payload["migrations"] == [
        "control_plane_repo_id_backfill",
        "control_plane_local_key_backfill",
    ]
    assert calls == [
        ("migrate", base_url, "housekeeper"),
    ]
