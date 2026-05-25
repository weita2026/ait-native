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


def test_repo_retire_command_uses_remote_repo_id_guard(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-retire"
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

    calls: list[tuple[str, str, str, str, bool]] = []

    monkeypatch.setattr(
        cli_module,
        "remote_get_repository",
        lambda cfg_url, repo_name: {
            "repo_name": repo_name,
            "repo_id": "REPO-HOUSEKEEPER",
            "default_line": "main",
            "lifecycle_state": "active",
        },
    )
    monkeypatch.setattr(
        cli_module,
        "remote_retire_repo",
        lambda cfg_url, repo_name, *, expected_repo_id, require_verified_export=True: calls.append(
            ("retire", cfg_url, repo_name, expected_repo_id, require_verified_export)
        )
        or {
            "repo_name": repo_name,
            "queued": False,
            "result": {"repo_id": expected_repo_id, "verification": {"verified": True}},
        },
    )

    retire_out = runner.invoke(app, ["repo", "retire", "--remote", "origin", "--json"], catch_exceptions=False)
    assert retire_out.exit_code == 0, retire_out.stdout
    payload = json.loads(retire_out.stdout)
    assert payload["repo_name"] == "housekeeper"
    assert payload["result"]["repo_id"] == "REPO-HOUSEKEEPER"
    assert payload["result"]["verification"]["verified"] is True
    assert calls == [
        ("retire", base_url, "housekeeper", "REPO-HOUSEKEEPER", True),
    ]
