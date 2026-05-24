from __future__ import annotations

import json

from ._shared import *  # noqa: F401,F403


def _clear_install_env(monkeypatch) -> None:
    for name in (
        "AIT_REPO_ROOT",
        "AIT_NATIVE_WORKSPACE_ROOT",
        "AIT_WORKSPACE_ROOT",
        "AIT_AGENT_CONFIG_PATH",
        "AIT_TELEGRAM_BOT_TOKEN",
        "AIT_TELEGRAM_BOT_USERNAME",
        "AIT_TELEGRAM_ENV_PATH",
        "AIT_TELEGRAM_STATE_PATH",
        "AIT_TELEGRAM_TERMINATION_CONTEXT_PATH",
        "AIT_NATIVE_SERVER_DATA",
        "AIT_NATIVE_SERVER_POSTGRES_DSN",
        "AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA",
        "AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA",
    ):
        monkeypatch.delenv(name, raising=False)


def test_install_help_lists_public_surface() -> None:
    help_out = runner.invoke(app, ["install", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    output = help_out.output or help_out.stdout
    assert "Workflow mode choice" in output
    assert "Optional transport attach" in output
    assert "--server-setup" in output
    assert "skip, connect, or" in output


def test_install_initializes_repo_and_keeps_local_mode_by_default(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "install-local-default"
    repo.mkdir()
    _clear_install_env(monkeypatch)
    monkeypatch.chdir(repo)

    result = runner.invoke(
        app,
        ["install", "--mode", "local", "--attach", "none", "--json"],
        catch_exceptions=False,
    )
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)

    assert payload["repository"]["action"] == "created"
    assert payload["repository"]["repo_initialized"] is True
    assert payload["mode"]["requested_mode"] == "solo_local"
    assert payload["mode"]["effective_mode"] == "solo_local"
    assert payload["server"]["choice"] == "skip"
    assert payload["server"]["action"] == "not_applicable"
    assert payload["runtime_root"]["classification"] == "installed_but_not_configured"
    assert payload["transport_actions"] == []
    assert (repo / ".ait" / "config.json").exists()

    show_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    show_payload = json.loads(show_out.stdout)
    assert show_payload["workflow_mode"]["value"] == "solo_local"


def test_install_remote_attach_both_reuses_repo_local_worker_config_and_is_idempotent(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "install-remote-attach"
    repo.mkdir()
    _clear_install_env(monkeypatch)
    monkeypatch.chdir(repo)
    monkeypatch.delenv("AIT_NATIVE_SERVER_POSTGRES_DSN", raising=False)

    argv = [
        "install",
        "--mode",
        "remote",
        "--attach",
        "both",
        "--telegram-token",
        "telegram-token",
        "--telegram-username",
        "aitbot",
        "--discord-application-id",
        "123456789",
        "--discord-bot-token",
        "discord-token",
        "--json",
    ]
    first = runner.invoke(app, argv, catch_exceptions=False)
    assert first.exit_code == 0, first.stdout
    first_payload = json.loads(first.stdout)

    assert first_payload["mode"]["effective_mode"] == "solo_remote"
    assert first_payload["attach_choice"] == "both"
    assert first_payload["server"]["choice"] == "skip"
    assert first_payload["server"]["action"] == "skipped"
    assert first_payload["postgres"]["classification"] in {
        "healthy",
        "installed_but_not_configured",
        "configured_but_unhealthy",
        "missing",
    }
    transport_actions = {(item["kind"], item["name"]): item["action"] for item in first_payload["transport_actions"]}
    assert transport_actions[("telegram", "main")] == "created"
    assert transport_actions[("discord", "main")] == "created"

    worker_payload = json.loads((repo / ".ait" / "agent-workers.json").read_text(encoding="utf-8"))
    assert worker_payload["workers"]["telegram/main"]["token"] == "telegram-token"
    assert worker_payload["workers"]["discord/main"]["application_id"] == "123456789"

    second = runner.invoke(app, argv, catch_exceptions=False)
    assert second.exit_code == 0, second.stdout
    second_payload = json.loads(second.stdout)
    second_actions = {(item["kind"], item["name"]): item["action"] for item in second_payload["transport_actions"]}
    assert second_payload["mode"]["action"] == "unchanged"
    assert second_actions[("telegram", "main")] == "unchanged"
    assert second_actions[("discord", "main")] == "unchanged"


def test_install_remote_server_connect_adds_default_remote(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "install-remote-server-connect"
    repo.mkdir()
    _clear_install_env(monkeypatch)
    monkeypatch.chdir(repo)

    result = runner.invoke(
        app,
        [
            "install",
            "--mode",
            "remote",
            "--attach",
            "none",
            "--server-setup",
            "connect",
            "--server-url",
            "http://127.0.0.1:8088/",
            "--remote-name",
            "origin",
            "--remote-repo-name",
            "demo-repo",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)

    assert payload["mode"]["effective_mode"] == "solo_remote"
    assert payload["server"] == {
        "choice": "connect",
        "action": "created",
        "classification": "healthy",
        "remote_name": "origin",
        "server_url": "http://127.0.0.1:8088",
        "repo_name": "demo-repo",
        "next_steps": [
            "Verify the ait-server with `ait queue summary --remote origin`.",
            "Publish Markdown lineage with `ait plan sync <file-or-dir> --remote origin` when the plan should become shared.",
        ],
    }

    remotes = runner.invoke(app, ["remote", "list", "--json"], catch_exceptions=False)
    assert remotes.exit_code == 0, remotes.stdout
    remote_rows = json.loads(remotes.stdout)
    assert remote_rows[0]["name"] == "origin"
    assert remote_rows[0]["url"] == "http://127.0.0.1:8088"
    assert remote_rows[0]["repo_name"] == "demo-repo"

    show_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    show_payload = json.loads(show_out.stdout)
    assert show_payload["workflow_mode"]["value"] == "solo_remote"
    assert show_payload["agent_runtime"]["remote_name"] == "origin"
    assert show_payload["agent_runtime"]["server_url"] == "http://127.0.0.1:8088"


def test_install_remote_server_deploy_is_guidance_only(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "install-remote-server-deploy"
    repo.mkdir()
    _clear_install_env(monkeypatch)
    monkeypatch.chdir(repo)

    result = runner.invoke(
        app,
        ["install", "--mode", "remote", "--attach", "none", "--server-setup", "deploy", "--json"],
        catch_exceptions=False,
    )
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)

    assert payload["server"]["choice"] == "deploy"
    assert payload["server"]["action"] == "deferred"
    assert payload["server"]["classification"] == "installed_but_not_configured"
    assert "ait-server" in " ".join(payload["server"]["next_steps"])

    remotes = runner.invoke(app, ["remote", "list", "--json"], catch_exceptions=False)
    assert remotes.exit_code == 0, remotes.stdout
    assert json.loads(remotes.stdout) == []
