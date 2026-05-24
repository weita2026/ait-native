from __future__ import annotations

import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path

from typer.testing import CliRunner

import ait_agent.cli as agent_cli
import ait_agent.discord.app as discord_app
import ait_agent.line.app as line_app
import ait_agent.slack.app as slack_app
import ait_agent.telegram.app as telegram_app
from ait.repo_paths import RepoContext


runner = CliRunner()
AUTHORED_ROOT = RepoContext.discover(Path(__file__).resolve().parents[1]).repo_root


def test_ait_agent_telegram_delegates_to_existing_worker(monkeypatch):
    calls: list[str] = []

    def fake_telegram_main() -> None:
        calls.append("telegram")

    monkeypatch.setattr(telegram_app, "main", fake_telegram_main)

    result = runner.invoke(agent_cli.app, ["telegram"])

    assert result.exit_code == 0
    assert calls == ["telegram"]


def test_ait_agent_telegram_webhook_mode_delegates_to_webhook_entrypoint(monkeypatch):
    calls: list[str] = []

    def fake_webhook_main() -> None:
        calls.append("webhook")

    monkeypatch.setattr(telegram_app, "webhook_main", fake_webhook_main)

    result = runner.invoke(agent_cli.app, ["telegram", "--mode", "webhook"], input='{"message": {"chat": {"id": 1}, "text": "hello"}}')

    assert result.exit_code == 0
    assert calls == ["webhook"]


def test_ait_agent_line_delegates_to_existing_worker(monkeypatch):
    calls: list[str] = []

    def fake_line_main() -> None:
        calls.append("line")

    monkeypatch.setattr(line_app, "main", fake_line_main)

    result = runner.invoke(agent_cli.app, ["line"])

    assert result.exit_code == 0
    assert calls == ["line"]


def test_ait_agent_line_webhook_mode_delegates_to_webhook_entrypoint(monkeypatch):
    calls: list[str] = []

    def fake_webhook_main() -> None:
        calls.append("webhook")

    monkeypatch.setattr(line_app, "webhook_main", fake_webhook_main)

    result = runner.invoke(agent_cli.app, ["line", "--mode", "webhook"], input='{\"events\": []}')

    assert result.exit_code == 0
    assert calls == ["webhook"]


def test_ait_agent_discord_delegates_to_existing_worker(monkeypatch):
    calls: list[str] = []

    def fake_discord_main() -> None:
        calls.append("discord")

    monkeypatch.setattr(discord_app, "main", fake_discord_main)

    result = runner.invoke(agent_cli.app, ["discord"])

    assert result.exit_code == 0
    assert calls == ["discord"]


def test_ait_agent_discord_interaction_mode_delegates_to_entrypoint(monkeypatch):
    calls: list[str] = []

    def fake_interaction_main() -> None:
        calls.append("interaction")

    monkeypatch.setattr(discord_app, "interaction_main", fake_interaction_main)

    result = runner.invoke(agent_cli.app, ["discord", "--mode", "interaction"], input='{"type": 1}')

    assert result.exit_code == 0
    assert calls == ["interaction"]


def test_ait_agent_cli_module_help_avoids_package_circular_import():
    repo_root = Path(__file__).resolve().parents[1]
    env = dict(os.environ)
    existing = env.get("PYTHONPATH", "")
    env["PYTHONPATH"] = f"{repo_root / 'src'}" + (os.pathsep + existing if existing else "")
    result = subprocess.run(
        [sys.executable, "-c", "from ait_agent.cli import main; main()", "discord", "--help"],
        cwd=repo_root,
        env=env,
        capture_output=True,
        text=True,
        timeout=20,
    )
    assert result.returncode == 0, result.stderr or result.stdout
    assert "ImportError" not in (result.stderr + result.stdout)
    assert "Discord worker mode" in result.stdout


def test_pid_is_alive_treats_zombie_process_as_not_running(monkeypatch):
    def fake_kill(pid: int, sig: int) -> None:
        assert pid == 4242
        assert sig == 0

    class FakeCompleted:
        returncode = 0
        stdout = "Z+\n"

    monkeypatch.setattr(agent_cli.os, "kill", fake_kill)
    monkeypatch.setattr(agent_cli.subprocess, "run", lambda *args, **kwargs: FakeCompleted())

    assert agent_cli._pid_is_alive(4242) is False


def test_stop_pid_waits_briefly_after_sigkill_before_reporting_still_running(monkeypatch):
    signals_sent: list[int] = []
    clock = {"now": 0.0}
    alive_states = iter([True, True, True, False])

    def fake_kill(pid: int, sig: int) -> None:
        assert pid == 4242
        signals_sent.append(sig)

    monkeypatch.setattr(agent_cli.os, "kill", fake_kill)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: next(alive_states))
    monkeypatch.setattr(agent_cli.time, "monotonic", lambda: clock["now"])
    monkeypatch.setattr(agent_cli.time, "sleep", lambda seconds: clock.__setitem__("now", clock["now"] + seconds))

    assert agent_cli._stop_pid(4242, timeout_seconds=0.2, kill_grace_seconds=0.2) == "killed"
    assert signals_sent == [agent_cli.signal.SIGTERM, agent_cli.signal.SIGKILL]


def test_ait_agent_slack_delegates_to_existing_worker(monkeypatch):
    calls: list[str] = []

    def fake_slack_main() -> None:
        calls.append("slack")

    monkeypatch.setattr(slack_app, "main", fake_slack_main)

    result = runner.invoke(agent_cli.app, ["slack"])

    assert result.exit_code == 0
    assert calls == ["slack"]


def test_ait_agent_slack_command_mode_delegates_to_entrypoint(monkeypatch):
    calls: list[str] = []

    def fake_command_main() -> None:
        calls.append("command")

    monkeypatch.setattr(slack_app, "command_main", fake_command_main)

    result = runner.invoke(
        agent_cli.app,
        ["slack", "--mode", "command"],
        input="token=fake&team_id=T1&channel_id=C1&user_id=U1&command=%2Fait&text=hello&response_url=https%3A%2F%2Fhooks.slack.com%2Fcommands",
    )

    assert result.exit_code == 0
    assert calls == ["command"]


def test_pyproject_exposes_ait_agent_console_script():
    pyproject_path = AUTHORED_ROOT / "pyproject.toml"
    pyproject = tomllib.loads(pyproject_path.read_text(encoding="utf-8"))

    assert pyproject["project"]["scripts"]["ait-agent"] == "ait_agent.cli:main"


def test_ait_agent_telegram_worker_config_round_trip(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "telegram",
            "add",
            "main",
            "--token",
            "123456:secret-token",
            "--username",
            "ait_main_bot",
            "--sync-state-path",
            "runtime/telegram-main.json",
            "--pid-file",
            "runtime/telegram-main.pid",
            "--log-file",
            "runtime/telegram-main.log",
            "--env-path",
            "runtime/telegram.env",
            "--json",
        ],
    )

    assert add_result.exit_code == 0
    add_payload = json.loads(add_result.output)
    assert add_payload["kind"] == "telegram"
    assert add_payload["name"] == "main"
    assert add_payload["username"] == "ait_main_bot"
    assert add_payload["token_set"] is True
    assert "secret-token" not in add_result.output

    stored = json.loads(config_path.read_text(encoding="utf-8"))
    assert stored["workers"]["telegram/main"]["token"] == "123456:secret-token"

    list_result = runner.invoke(agent_cli.app, ["telegram", "list", "--json"])

    assert list_result.exit_code == 0
    list_payload = json.loads(list_result.output)
    assert [worker["name"] for worker in list_payload] == ["main"]
    assert list_payload[0]["sync_state_path"] == "runtime/telegram-main.json"
    assert list_payload[0]["env_path"] == "runtime/telegram.env"
    assert "secret-token" not in list_result.output

    remove_result = runner.invoke(agent_cli.app, ["telegram", "remove", "main", "--json"])

    assert remove_result.exit_code == 0
    assert json.loads(remove_result.output) == {"kind": "telegram", "name": "main", "removed": True}
    assert json.loads(config_path.read_text(encoding="utf-8"))["workers"] == {}


def test_ait_agent_line_worker_config_round_trip(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "line",
            "add",
            "main",
            "--token",
            "line-access-token",
            "--secret",
            "line-channel-secret",
            "--sync-state-path",
            "runtime/line-main.json",
            "--pid-file",
            "runtime/line-main.pid",
            "--log-file",
            "runtime/line-main.log",
            "--env-path",
            "runtime/line.env",
            "--json",
        ],
    )

    assert add_result.exit_code == 0
    add_payload = json.loads(add_result.output)
    assert add_payload["kind"] == "line"
    assert add_payload["name"] == "main"
    assert add_payload["token_set"] is True
    assert add_payload["secret_set"] is True
    assert "line-access-token" not in add_result.output
    assert "line-channel-secret" not in add_result.output

    stored = json.loads(config_path.read_text(encoding="utf-8"))
    assert stored["workers"]["line/main"]["token"] == "line-access-token"
    assert stored["workers"]["line/main"]["secret"] == "line-channel-secret"

    list_result = runner.invoke(agent_cli.app, ["line", "list", "--json"])

    assert list_result.exit_code == 0
    list_payload = json.loads(list_result.output)
    assert [worker["name"] for worker in list_payload] == ["main"]
    assert list_payload[0]["sync_state_path"] == "runtime/line-main.json"
    assert list_payload[0]["env_path"] == "runtime/line.env"
    assert "line-access-token" not in list_result.output
    assert "line-channel-secret" not in list_result.output

    remove_result = runner.invoke(agent_cli.app, ["line", "remove", "main", "--json"])

    assert remove_result.exit_code == 0
    assert json.loads(remove_result.output) == {"kind": "line", "name": "main", "removed": True}
    assert json.loads(config_path.read_text(encoding="utf-8"))["workers"] == {}


def test_ait_agent_discord_worker_config_round_trip(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "discord",
            "add",
            "sidecar",
            "--application-id",
            "123456789012345678",
            "--bot-token",
            "discord-bot-token",
            "--sync-state-path",
            "runtime/discord-sidecar.json",
            "--pid-file",
            "runtime/discord-sidecar.pid",
            "--log-file",
            "runtime/discord-sidecar.log",
            "--env-path",
            "runtime/discord.env",
            "--json",
        ],
    )

    assert add_result.exit_code == 0
    add_payload = json.loads(add_result.output)
    assert add_payload["kind"] == "discord"
    assert add_payload["name"] == "sidecar"
    assert add_payload["application_id"] == "123456789012345678"
    assert add_payload["bot_token_set"] is True

    stored = json.loads(config_path.read_text(encoding="utf-8"))
    assert stored["workers"]["discord/sidecar"]["application_id"] == "123456789012345678"
    assert stored["workers"]["discord/sidecar"]["bot_token"] == "discord-bot-token"

    list_result = runner.invoke(agent_cli.app, ["discord", "list", "--json"])

    assert list_result.exit_code == 0
    list_payload = json.loads(list_result.output)
    assert [worker["name"] for worker in list_payload] == ["sidecar"]
    assert list_payload[0]["sync_state_path"] == "runtime/discord-sidecar.json"
    assert list_payload[0]["env_path"] == "runtime/discord.env"

    remove_result = runner.invoke(agent_cli.app, ["discord", "remove", "sidecar", "--json"])

    assert remove_result.exit_code == 0
    assert json.loads(remove_result.output) == {"kind": "discord", "name": "sidecar", "removed": True}
    assert json.loads(config_path.read_text(encoding="utf-8"))["workers"] == {}


def test_ait_agent_slack_worker_config_round_trip(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "slack",
            "add",
            "sidecar",
            "--app-token",
            "xapp-slack-token",
            "--sync-state-path",
            "runtime/slack-sidecar.json",
            "--pid-file",
            "runtime/slack-sidecar.pid",
            "--log-file",
            "runtime/slack-sidecar.log",
            "--env-path",
            "runtime/slack.env",
            "--json",
        ],
    )

    assert add_result.exit_code == 0
    add_payload = json.loads(add_result.output)
    assert add_payload["kind"] == "slack"
    assert add_payload["name"] == "sidecar"
    assert add_payload["app_token_set"] is True
    assert "xapp-slack-token" not in add_result.output

    stored = json.loads(config_path.read_text(encoding="utf-8"))
    assert stored["workers"]["slack/sidecar"]["app_token"] == "xapp-slack-token"

    list_result = runner.invoke(agent_cli.app, ["slack", "list", "--json"])

    assert list_result.exit_code == 0
    list_payload = json.loads(list_result.output)
    assert [worker["name"] for worker in list_payload] == ["sidecar"]
    assert list_payload[0]["sync_state_path"] == "runtime/slack-sidecar.json"
    assert list_payload[0]["env_path"] == "runtime/slack.env"
    assert "xapp-slack-token" not in list_result.output

    remove_result = runner.invoke(agent_cli.app, ["slack", "remove", "sidecar", "--json"])

    assert remove_result.exit_code == 0
    assert json.loads(remove_result.output) == {"kind": "slack", "name": "sidecar", "removed": True}
    assert json.loads(config_path.read_text(encoding="utf-8"))["workers"] == {}


def test_ait_agent_telegram_config_invalid_json_is_hardened(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    config_path.write_text("{", encoding="utf-8")
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))

    config, issues = agent_cli._load_config_payload()
    assert config == {"version": 1, "workers": {}}
    assert any("Invalid JSON" in issue for issue in issues)

    result = runner.invoke(agent_cli.app, ["telegram", "supervisor", "status", "--json"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["config_valid"] is False
    assert payload["config_version"] == 1
    assert isinstance(payload["config_issues"], list)


def test_ait_agent_telegram_config_schema_validation_normalizes_bad_workers(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    config_path.write_text(
        json.dumps(
            {
                "version": "bad",
                "workers": {
                    "telegram/main": {
                        "kind": 123,
                        "name": None,
                        "token": 999,
                        "username": ["bad", "name"],
                        "sync_state_path": 77,
                        "pid_file": True,
                        "log_file": 12,
                        "env_path": 12,
                    }
                },
            },
            sort_keys=True,
        ),
        encoding="utf-8",
    )
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))

    config, issues = agent_cli._load_config_payload()

    assert config["version"] == 1
    assert any("version" in issue for issue in issues)
    assert any("non-string" in issue for issue in issues)
    worker = config["workers"]["telegram/main"]
    assert worker["kind"] == "telegram"
    assert worker["name"] == "main"
    assert worker["token"] is None


def test_ait_agent_telegram_status_includes_config_and_pid_health(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(tmp_path / "stale-telegram.env"))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "telegram",
            "add",
            "main",
            "--token",
            "123456:secret-token",
            "--username",
            "ait_main_bot",
            "--sync-state-path",
            "runtime/state.json",
            "--pid-file",
            "runtime/worker.pid",
            "--log-file",
            "runtime/worker.log",
            "--json",
        ],
    )
    assert add_result.exit_code == 0

    pid_file = repo_root / "runtime/worker.pid"
    pid_file.parent.mkdir(parents=True, exist_ok=True)
    pid_file.write_text("not-a-pid", encoding="utf-8")

    status_result = runner.invoke(agent_cli.app, ["telegram", "status", "main", "--json"])

    assert status_result.exit_code == 0
    status_payload = json.loads(status_result.output)
    assert status_payload["config_valid"] is True
    assert status_payload["running"] is False
    assert status_payload["pid"] is None
    assert status_payload["env_path"] == str(repo_root / ".ait/agent-runtime/telegram.env")
    assert status_payload["health"]["pid_file_state"] == "invalid"
    assert status_payload["health"]["pid_file_valid"] is False


def test_ait_agent_telegram_worker_lifecycle_commands(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(tmp_path / "stale-telegram.env"))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "telegram",
            "add",
            "main",
            "--token",
            "123456:secret-token",
            "--username",
            "ait_main_bot",
            "--sync-state-path",
            "runtime/state.json",
            "--pid-file",
            "runtime/worker.pid",
            "--log-file",
            "runtime/worker.log",
            "--json",
        ],
    )
    assert add_result.exit_code == 0
    termination_context_path = repo_root / ".ait/agent-runtime/telegram-main-termination.json"
    termination_context_path.parent.mkdir(parents=True, exist_ok=True)
    termination_context_path.write_text('{"pid": 1, "reason": "stale"}\n', encoding="utf-8")

    launched: dict[str, object] = {}

    class FakeProcess:
        pid = 4242

    def fake_popen(command, *, cwd, env, stdin, stdout, stderr, start_new_session):
        launched["command"] = command
        launched["cwd"] = cwd
        launched["env"] = env
        launched["stdin"] = stdin
        launched["stdout"] = stdout
        launched["stderr"] = stderr
        launched["start_new_session"] = start_new_session
        return FakeProcess()

    alive = {4242: True}
    monkeypatch.setattr(agent_cli.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: alive.get(pid, False))

    start_result = runner.invoke(agent_cli.app, ["telegram", "start", "main", "--json"])

    assert start_result.exit_code == 0
    start_payload = json.loads(start_result.output)
    assert start_payload["started"] is True
    assert start_payload["running"] is True
    assert start_payload["pid"] == 4242
    assert launched["command"][1:] == ["-m", "ait_agent.telegram.app"]
    assert launched["cwd"] == str(repo_root)
    assert launched["env"]["AIT_TELEGRAM_BOT_TOKEN"] == "123456:secret-token"
    assert launched["env"]["AIT_TELEGRAM_BOT_USERNAME"] == "ait_main_bot"
    assert launched["env"]["AIT_TELEGRAM_STATE_PATH"] == str(repo_root / "runtime/state.json")
    assert launched["env"]["AIT_TELEGRAM_ENV_PATH"] == str(repo_root / ".ait/agent-runtime/telegram.env")
    assert launched["env"]["AIT_TELEGRAM_TERMINATION_CONTEXT_PATH"] == str(termination_context_path)
    seeded_env = agent_cli.load_simple_env_file(repo_root / ".ait/agent-runtime/telegram.env")
    assert seeded_env["AIT_REPO_ROOT"] == str(repo_root)
    assert seeded_env["AIT_TELEGRAM_ENV_PATH"] == str(repo_root / ".ait/agent-runtime/telegram.env")
    assert seeded_env["AIT_TELEGRAM_BOT_TOKEN"] == "123456:secret-token"
    assert seeded_env["BOT_TOKEN"] == "123456:secret-token"
    assert seeded_env["AIT_TELEGRAM_STATE_PATH"] == str(repo_root / "runtime/state.json")
    assert seeded_env["AIT_TELEGRAM_TERMINATION_CONTEXT_PATH"] == str(termination_context_path)
    assert seeded_env["AIT_TELEGRAM_BOT_USERNAME"] == "ait_main_bot"
    assert seeded_env["BOT_USERNAME"] == "ait_main_bot"
    assert seeded_env["AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS"] == "inf"
    assert seeded_env["AIT_TELEGRAM_POLL_TIMEOUT_SECONDS"] == "45"
    assert seeded_env["AIT_TELEGRAM_OPENAI_TIMEOUT_SECONDS"] == "inf"
    assert seeded_env["AIT_TELEGRAM_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS"] == "30"
    assert seeded_env["AIT_TELEGRAM_CODEX_TURN_TIMEOUT_SECONDS"] == "inf"
    assert seeded_env["AIT_TELEGRAM_CODEX_CHILD_REAP_TIMEOUT_SECONDS"] == "3.5"
    assert seeded_env["AIT_TELEGRAM_STT_MODE"] == "off"
    assert seeded_env["AIT_TELEGRAM_STT_MODEL"] == "mlx-community/whisper-large-v3-mlx"
    assert seeded_env["AIT_TELEGRAM_STT_DEVICE"] == "auto"
    assert seeded_env["AIT_TELEGRAM_STT_INCLUDE_AUDIO_UPLOADS"] == "false"
    assert (repo_root / "runtime/worker.pid").read_text(encoding="utf-8").strip() == "4242"
    assert not termination_context_path.exists()

    status_result = runner.invoke(agent_cli.app, ["telegram", "status", "main", "--json"])

    assert status_result.exit_code == 0
    status_payload = json.loads(status_result.output)
    assert status_payload["running"] is True
    assert status_payload["pid"] == 4242

    def fake_stop_pid(pid):
        alive[pid] = False
        return "stopped"

    monkeypatch.setattr(agent_cli, "_stop_pid", fake_stop_pid)

    stop_result = runner.invoke(agent_cli.app, ["telegram", "stop", "main", "--json"])

    assert stop_result.exit_code == 0
    stop_payload = json.loads(stop_result.output)
    assert stop_payload["stopped"] is True
    assert stop_payload["running"] is False
    assert stop_payload["stop_state"] == "stopped"
    assert not (repo_root / "runtime/worker.pid").exists()
    termination_context = json.loads(termination_context_path.read_text(encoding="utf-8"))
    assert termination_context["pid"] == 4242
    assert termination_context["reason"] == "cli_telegram_stop"


def test_ait_agent_telegram_start_preserves_existing_env_file(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "telegram",
            "add",
            "main",
            "--token",
            "123456:secret-token",
            "--username",
            "ait_main_bot",
            "--json",
        ],
    )
    assert add_result.exit_code == 0

    env_path = repo_root / ".ait/agent-runtime/telegram.env"
    env_path.parent.mkdir(parents=True, exist_ok=True)
    original_env = "AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS=120\nCUSTOM_KEEP=1\n"
    env_path.write_text(original_env, encoding="utf-8")

    class FakeProcess:
        pid = 4242

    monkeypatch.setattr(agent_cli.subprocess, "Popen", lambda *args, **kwargs: FakeProcess())
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: pid == 4242)

    start_result = runner.invoke(agent_cli.app, ["telegram", "start", "main", "--json"])

    assert start_result.exit_code == 0
    assert env_path.read_text(encoding="utf-8") == original_env


def test_ait_agent_line_worker_lifecycle_commands(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))
    monkeypatch.setenv("AIT_LINE_ENV_PATH", str(tmp_path / "stale-line.env"))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "line",
            "add",
            "main",
            "--token",
            "line-access-token",
            "--secret",
            "line-channel-secret",
            "--sync-state-path",
            "runtime/state.json",
            "--pid-file",
            "runtime/worker.pid",
            "--log-file",
            "runtime/worker.log",
            "--json",
        ],
    )
    assert add_result.exit_code == 0
    termination_context_path = repo_root / ".ait/agent-runtime/line-main-termination.json"
    termination_context_path.parent.mkdir(parents=True, exist_ok=True)
    termination_context_path.write_text('{"pid": 1, "reason": "stale"}\n', encoding="utf-8")

    launched: dict[str, object] = {}

    class FakeProcess:
        pid = 5151

    def fake_popen(command, *, cwd, env, stdin, stdout, stderr, start_new_session):
        launched["command"] = command
        launched["cwd"] = cwd
        launched["env"] = env
        launched["stdin"] = stdin
        launched["stdout"] = stdout
        launched["stderr"] = stderr
        launched["start_new_session"] = start_new_session
        return FakeProcess()

    alive = {5151: True}
    monkeypatch.setattr(agent_cli.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: alive.get(pid, False))

    start_result = runner.invoke(agent_cli.app, ["line", "start", "main", "--json"])

    assert start_result.exit_code == 0
    start_payload = json.loads(start_result.output)
    assert start_payload["started"] is True
    assert start_payload["running"] is True
    assert start_payload["pid"] == 5151
    assert launched["command"][1:] == ["-m", "ait_agent.line.app"]
    assert launched["env"]["AIT_LINE_ENV_PATH"] == str(repo_root / ".ait/agent-runtime/line.env")
    assert launched["env"]["AIT_LINE_CHANNEL_ACCESS_TOKEN"] == "line-access-token"
    assert launched["env"]["AIT_LINE_CHANNEL_SECRET"] == "line-channel-secret"
    assert not termination_context_path.exists()

    status_result = runner.invoke(agent_cli.app, ["line", "status", "main", "--json"])

    assert status_result.exit_code == 0
    status_payload = json.loads(status_result.output)
    assert status_payload["running"] is True
    assert status_payload["env_path"] == str(repo_root / ".ait/agent-runtime/line.env")

    def fake_stop_pid(pid):
        alive[pid] = False
        return "stopped"

    monkeypatch.setattr(agent_cli, "_stop_pid", fake_stop_pid)

    stop_result = runner.invoke(agent_cli.app, ["line", "stop", "main", "--json"])

    assert stop_result.exit_code == 0
    stop_payload = json.loads(stop_result.output)
    assert stop_payload["stopped"] is True
    assert stop_payload["stop_state"] == "stopped"
    termination_context = json.loads(termination_context_path.read_text(encoding="utf-8"))
    assert termination_context["reason"] == "cli_line_stop"
    assert termination_context["worker_name"] == "main"


def test_ait_agent_discord_worker_lifecycle_commands(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))
    monkeypatch.setenv("AIT_DISCORD_ENV_PATH", str(tmp_path / "stale-discord.env"))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "discord",
            "add",
            "sidecar",
            "--application-id",
            "123456789012345678",
            "--bot-token",
            "discord-bot-token",
            "--sync-state-path",
            "runtime/state.json",
            "--pid-file",
            "runtime/worker.pid",
            "--log-file",
            "runtime/worker.log",
            "--json",
        ],
    )
    assert add_result.exit_code == 0
    termination_context_path = repo_root / ".ait/agent-runtime/discord-sidecar-termination.json"
    termination_context_path.parent.mkdir(parents=True, exist_ok=True)
    termination_context_path.write_text('{"pid": 1, "reason": "stale"}\n', encoding="utf-8")

    launched: dict[str, object] = {}

    class FakeProcess:
        pid = 6161

    def fake_popen(command, *, cwd, env, stdin, stdout, stderr, start_new_session):
        launched["command"] = command
        launched["cwd"] = cwd
        launched["env"] = env
        launched["stdin"] = stdin
        launched["stdout"] = stdout
        launched["stderr"] = stderr
        launched["start_new_session"] = start_new_session
        return FakeProcess()

    alive = {6161: True}
    monkeypatch.setattr(agent_cli.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: alive.get(pid, False))

    start_result = runner.invoke(agent_cli.app, ["discord", "start", "sidecar", "--json"])

    assert start_result.exit_code == 0
    start_payload = json.loads(start_result.output)
    assert start_payload["started"] is True
    assert start_payload["running"] is True
    assert start_payload["pid"] == 6161
    assert launched["command"][1:] == ["-m", "ait_agent.discord.app"]
    assert launched["env"]["AIT_DISCORD_ENV_PATH"] == str(repo_root / ".ait/agent-runtime/discord.env")
    assert launched["env"]["AIT_DISCORD_APPLICATION_ID"] == "123456789012345678"
    assert launched["env"]["AIT_DISCORD_BOT_TOKEN"] == "discord-bot-token"
    assert not termination_context_path.exists()

    status_result = runner.invoke(agent_cli.app, ["discord", "status", "sidecar", "--json"])

    assert status_result.exit_code == 0
    status_payload = json.loads(status_result.output)
    assert status_payload["running"] is True
    assert status_payload["env_path"] == str(repo_root / ".ait/agent-runtime/discord.env")

    def fake_stop_pid(pid):
        alive[pid] = False
        return "stopped"

    monkeypatch.setattr(agent_cli, "_stop_pid", fake_stop_pid)

    stop_result = runner.invoke(agent_cli.app, ["discord", "stop", "sidecar", "--json"])

    assert stop_result.exit_code == 0
    stop_payload = json.loads(stop_result.output)
    assert stop_payload["stopped"] is True
    assert stop_payload["stop_state"] == "stopped"
    termination_context = json.loads(termination_context_path.read_text(encoding="utf-8"))
    assert termination_context["reason"] == "cli_discord_stop"
    assert termination_context["worker_name"] == "sidecar"


def test_ait_agent_discord_worker_uses_env_only_bot_token(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "discord",
            "add",
            "sidecar",
            "--application-id",
            "123456789012345678",
            "--bot-token",
            "discord-bot-token",
            "--sync-state-path",
            "runtime/state.json",
            "--pid-file",
            "runtime/worker.pid",
            "--log-file",
            "runtime/worker.log",
            "--env-path",
            "runtime/discord.env",
            "--json",
        ],
    )
    assert add_result.exit_code == 0

    payload = json.loads(config_path.read_text(encoding="utf-8"))
    payload["workers"]["discord/sidecar"]["bot_token"] = None
    config_path.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")

    env_path = repo_root / "runtime/discord.env"
    env_path.parent.mkdir(parents=True, exist_ok=True)
    env_path.write_text("AIT_DISCORD_BOT_TOKEN=discord-env-token\n", encoding="utf-8")

    launched: dict[str, object] = {}

    class FakeProcess:
        pid = 7171

    def fake_popen(command, *, cwd, env, stdin, stdout, stderr, start_new_session):
        launched["command"] = command
        launched["cwd"] = cwd
        launched["env"] = env
        launched["stdin"] = stdin
        launched["stdout"] = stdout
        launched["stderr"] = stderr
        launched["start_new_session"] = start_new_session
        return FakeProcess()

    monkeypatch.setattr(agent_cli.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: pid == 7171)

    start_result = runner.invoke(agent_cli.app, ["discord", "start", "sidecar", "--json"])

    assert start_result.exit_code == 0
    start_payload = json.loads(start_result.output)
    assert start_payload["started"] is True
    assert launched["env"]["AIT_DISCORD_BOT_TOKEN"] == "discord-env-token"
    assert launched["env"]["DISCORD_BOT_TOKEN"] == "discord-env-token"

    status_result = runner.invoke(agent_cli.app, ["discord", "status", "sidecar", "--json"])

    assert status_result.exit_code == 0
    status_payload = json.loads(status_result.output)
    assert status_payload["bot_token_set"] is True


def test_ait_agent_slack_worker_lifecycle_commands(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))
    monkeypatch.setenv("AIT_SLACK_ENV_PATH", str(tmp_path / "stale-slack.env"))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "slack",
            "add",
            "sidecar",
            "--app-token",
            "xapp-slack-token",
            "--sync-state-path",
            "runtime/state.json",
            "--pid-file",
            "runtime/worker.pid",
            "--log-file",
            "runtime/worker.log",
            "--json",
        ],
    )
    assert add_result.exit_code == 0
    termination_context_path = repo_root / ".ait/agent-runtime/slack-sidecar-termination.json"
    termination_context_path.parent.mkdir(parents=True, exist_ok=True)
    termination_context_path.write_text('{"pid": 1, "reason": "stale"}\n', encoding="utf-8")

    launched: dict[str, object] = {}

    class FakeProcess:
        pid = 7171

    def fake_popen(command, *, cwd, env, stdin, stdout, stderr, start_new_session):
        launched["command"] = command
        launched["cwd"] = cwd
        launched["env"] = env
        launched["stdin"] = stdin
        launched["stdout"] = stdout
        launched["stderr"] = stderr
        launched["start_new_session"] = start_new_session
        return FakeProcess()

    alive = {7171: True}
    monkeypatch.setattr(agent_cli.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: alive.get(pid, False))

    start_result = runner.invoke(agent_cli.app, ["slack", "start", "sidecar", "--json"])

    assert start_result.exit_code == 0
    start_payload = json.loads(start_result.output)
    assert start_payload["started"] is True
    assert start_payload["running"] is True
    assert start_payload["pid"] == 7171
    assert launched["command"][1:] == ["-m", "ait_agent.slack.app"]
    assert launched["env"]["AIT_SLACK_ENV_PATH"] == str(repo_root / ".ait/agent-runtime/slack.env")
    assert launched["env"]["AIT_SLACK_APP_TOKEN"] == "xapp-slack-token"
    assert launched["env"]["AIT_SLACK_STATE_PATH"] == str(repo_root / "runtime/state.json")
    assert not termination_context_path.exists()

    status_result = runner.invoke(agent_cli.app, ["slack", "status", "sidecar", "--json"])

    assert status_result.exit_code == 0
    status_payload = json.loads(status_result.output)
    assert status_payload["running"] is True
    assert status_payload["env_path"] == str(repo_root / ".ait/agent-runtime/slack.env")

    def fake_stop_pid(pid):
        alive[pid] = False
        return "stopped"

    monkeypatch.setattr(agent_cli, "_stop_pid", fake_stop_pid)

    stop_result = runner.invoke(agent_cli.app, ["slack", "stop", "sidecar", "--json"])

    assert stop_result.exit_code == 0
    stop_payload = json.loads(stop_result.output)
    assert stop_payload["stopped"] is True
    assert stop_payload["stop_state"] == "stopped"
    termination_context = json.loads(termination_context_path.read_text(encoding="utf-8"))
    assert termination_context["reason"] == "cli_slack_stop"
    assert termination_context["worker_name"] == "sidecar"


def test_ait_agent_telegram_restart_calls_stop_and_restarts_running_worker(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "telegram",
            "add",
            "main",
            "--token",
            "123456:secret-token",
            "--username",
            "ait_main_bot",
            "--sync-state-path",
            "runtime/state.json",
            "--pid-file",
            "runtime/worker.pid",
            "--log-file",
            "runtime/worker.log",
            "--json",
        ],
    )
    assert add_result.exit_code == 0

    pid_file = repo_root / "runtime/worker.pid"
    log_file = repo_root / "runtime/worker.log"
    pid_file.parent.mkdir(parents=True, exist_ok=True)
    pid_file.write_text("1111\n", encoding="utf-8")
    log_file.write_text("booting\n", encoding="utf-8")

    class FakeProcess:
        def __init__(self, pid: int):
            self.pid = pid

    launched: list[dict[str, object]] = []
    alive: dict[int, bool] = {1111: True, 2222: True}
    stopped: list[int] = []

    def fake_popen(
        command: list[str],
        *,
        cwd: str,
        env: dict[str, str],
        stdin: object,
        stdout: object,
        stderr: object,
        start_new_session: bool,
    ) -> FakeProcess:
        launched.append(
            {
                "command": command,
                "cwd": cwd,
                "env": env,
                "stdin": stdin,
                "stdout": stdout,
                "stderr": stderr,
                "start_new_session": start_new_session,
            }
        )
        return FakeProcess(2222)

    def fake_pid_is_alive(pid: int) -> bool:
        return alive.get(pid, False)

    def fake_stop_pid(pid: int, *, timeout_seconds: float = 10.0) -> str:
        stopped.append(pid)
        alive[pid] = False
        return "stopped"

    monkeypatch.setattr(agent_cli.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", fake_pid_is_alive)
    monkeypatch.setattr(agent_cli, "_stop_pid", fake_stop_pid)

    restart_result = runner.invoke(agent_cli.app, ["telegram", "restart", "main", "--json"])

    assert restart_result.exit_code == 0
    restart_payload = json.loads(restart_result.output)
    assert restart_payload["kind"] == "telegram"
    assert restart_payload["name"] == "main"
    assert restart_payload["running"] is True
    assert restart_payload["pid"] == 2222
    assert restart_payload["started"] is True
    assert restart_payload["stopped"] is True
    assert restart_payload["stop_state"] == "stopped"
    assert restart_payload["restarted"] is True
    assert restart_payload["restart_blocked"] is False
    assert stopped == [1111]
    assert len(launched) == 1
    assert launched[0]["command"][1:] == ["-m", "ait_agent.telegram.app"]
    assert launched[0]["cwd"] == str(repo_root)
    assert launched[0]["env"]["AIT_TELEGRAM_BOT_TOKEN"] == "123456:secret-token"
    assert launched[0]["env"]["AIT_TELEGRAM_BOT_USERNAME"] == "ait_main_bot"
    assert launched[0]["env"]["AIT_TELEGRAM_STATE_PATH"] == str(repo_root / "runtime/state.json")
    assert launched[0]["env"]["AIT_TELEGRAM_ENV_PATH"] == str(repo_root / ".ait/agent-runtime/telegram.env")
    assert launched[0]["env"]["AIT_TELEGRAM_TERMINATION_CONTEXT_PATH"] == str(
        repo_root / ".ait/agent-runtime/telegram-main-termination.json"
    )
    assert pid_file.read_text(encoding="utf-8").strip() == "2222"
    assert not (repo_root / ".ait/agent-runtime/telegram-main-termination.json").exists()


def test_ait_agent_telegram_restart_does_not_duplicate_still_running_worker(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "telegram",
            "add",
            "main",
            "--token",
            "123456:secret-token",
            "--username",
            "ait_main_bot",
            "--sync-state-path",
            "runtime/state.json",
            "--pid-file",
            "runtime/worker.pid",
            "--log-file",
            "runtime/worker.log",
            "--json",
        ],
    )
    assert add_result.exit_code == 0

    pid_file = repo_root / "runtime/worker.pid"
    pid_file.parent.mkdir(parents=True, exist_ok=True)
    pid_file.write_text("1111\n", encoding="utf-8")

    def fail_popen(*args, **kwargs):
        raise AssertionError("restart must not launch a duplicate worker")

    monkeypatch.setattr(agent_cli.subprocess, "Popen", fail_popen)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: pid == 1111)
    monkeypatch.setattr(agent_cli, "_stop_pid", lambda pid, timeout_seconds=10.0: "still_running")

    restart_result = runner.invoke(agent_cli.app, ["telegram", "restart", "main", "--json"])

    assert restart_result.exit_code == 0
    restart_payload = json.loads(restart_result.output)
    assert restart_payload["running"] is True
    assert restart_payload["pid"] == 1111
    assert restart_payload["started"] is False
    assert restart_payload["stopped"] is False
    assert restart_payload["stop_state"] == "still_running"
    assert restart_payload["restarted"] is False
    assert restart_payload["restart_blocked"] is True
    assert pid_file.read_text(encoding="utf-8").strip() == "1111"
    termination_context = json.loads(
        (repo_root / ".ait/agent-runtime/telegram-main-termination.json").read_text(encoding="utf-8")
    )
    assert termination_context["pid"] == 1111
    assert termination_context["reason"] == "cli_telegram_restart"


def test_ait_agent_telegram_restart_starts_stopped_worker_if_not_running(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "telegram",
            "add",
            "sidecar",
            "--token",
            "123456:secret-token",
            "--username",
            "ait_sidecar_bot",
            "--sync-state-path",
            "runtime/sidecar-state.json",
            "--pid-file",
            "runtime/sidecar.pid",
            "--log-file",
            "runtime/sidecar.log",
            "--json",
        ],
    )
    assert add_result.exit_code == 0

    pid_file = repo_root / "runtime/sidecar.pid"
    pid_file.parent.mkdir(parents=True, exist_ok=True)
    pid_file.write_text("1111\n", encoding="utf-8")

    class FakeProcess:
        def __init__(self, pid: int):
            self.pid = pid

    launched: list[dict[str, object]] = []
    stopped: list[int] = []
    alive: dict[int, bool] = {1111: False, 3333: True}

    def fake_popen(
        command: list[str],
        *,
        cwd: str,
        env: dict[str, str],
        stdin: object,
        stdout: object,
        stderr: object,
        start_new_session: bool,
    ) -> FakeProcess:
        launched.append({"command": command})
        return FakeProcess(3333)

    monkeypatch.setattr(agent_cli.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: alive.get(pid, False))
    monkeypatch.setattr(agent_cli, "_stop_pid", lambda pid, timeout_seconds=10.0: stopped.append(pid) or "not_started")

    restart_result = runner.invoke(agent_cli.app, ["telegram", "restart", "sidecar", "--json"])

    assert restart_result.exit_code == 0
    restart_payload = json.loads(restart_result.output)
    assert restart_payload["kind"] == "telegram"
    assert restart_payload["name"] == "sidecar"
    assert restart_payload["pid"] == 3333
    assert restart_payload["started"] is True
    assert restart_payload["stopped"] is False
    assert restart_payload["stop_state"] == "not_running"
    assert restart_payload["restarted"] is False
    assert restart_payload["restart_blocked"] is False
    assert stopped == []
    assert len(launched) == 1
    assert launched[0]["command"][1:] == ["-m", "ait_agent.telegram.app"]
    assert pid_file.read_text(encoding="utf-8").strip() == "3333"
    assert not (repo_root / ".ait/agent-runtime/telegram-sidecar-termination.json").exists()


def test_ait_agent_telegram_logs_returns_tail_lines_and_handles_missing_log(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))

    add_result = runner.invoke(
        agent_cli.app,
        [
            "telegram",
            "add",
            "main",
            "--token",
            "123456:secret-token",
            "--username",
            "ait_main_bot",
            "--sync-state-path",
            "runtime/state.json",
            "--pid-file",
            "runtime/worker.pid",
            "--log-file",
            "runtime/worker.log",
            "--json",
        ],
    )
    assert add_result.exit_code == 0

    log_path = repo_root / "runtime/worker.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log_path.write_text("line 1\nline 2\nline 3\nline 4\n", encoding="utf-8")

    logs_result = runner.invoke(agent_cli.app, ["telegram", "logs", "main", "--lines", "2", "--json"])

    assert logs_result.exit_code == 0
    logs_payload = json.loads(logs_result.output)
    assert logs_payload["lines"] == ["line 3", "line 4"]
    assert logs_payload["log_exists"] is True
    assert logs_payload["lines_requested"] == 2
    assert logs_payload["log_file"] == str(log_path)

    missing_config_add = runner.invoke(
        agent_cli.app,
        [
            "telegram",
            "add",
            "missing",
            "--token",
            "123456:secret-token",
            "--username",
            "ait_missing_bot",
            "--sync-state-path",
            "runtime/missing-state.json",
            "--pid-file",
            "runtime/missing.pid",
            "--log-file",
            "runtime/missing.log",
            "--json",
        ],
    )
    assert missing_config_add.exit_code == 0

    missing_logs = runner.invoke(agent_cli.app, ["telegram", "logs", "missing", "--lines", "2", "--json"])
    assert missing_logs.exit_code == 0
    missing_payload = json.loads(missing_logs.output)
    assert missing_payload["lines"] == []
    assert missing_payload["log_exists"] is False
    assert missing_payload["lines_requested"] == 2
