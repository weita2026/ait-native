from __future__ import annotations

import json
from pathlib import Path

from typer.testing import CliRunner

import ait_agent.cli as agent_cli


runner = CliRunner()


def _add_worker(name: str, repo_root: Path) -> None:
    result = runner.invoke(
        agent_cli.app,
        [
            "telegram",
            "add",
            name,
            "--token",
            f"123456:{name}-secret-token",
            "--username",
            f"ait_{name}_bot",
            "--sync-state-path",
            f"runtime/{name}.json",
            "--pid-file",
            f"runtime/{name}.pid",
            "--log-file",
            f"runtime/{name}.log",
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    (repo_root / "runtime").mkdir(parents=True, exist_ok=True)


def _worker_by_name(payload: dict[str, object], name: str) -> dict[str, object]:
    workers = payload["workers"]
    assert isinstance(workers, list)
    for worker in workers:
        assert isinstance(worker, dict)
        if worker["name"] == name:
            return worker
    raise AssertionError(f"missing worker {name}")


def test_ait_agent_telegram_supervisor_status_empty_config(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))

    result = runner.invoke(agent_cli.app, ["telegram", "supervisor", "status", "--json"])

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload == {
        "action": "status",
        "config_valid": True,
        "config_version": 1,
        "kind": "telegram-supervisor",
        "running_count": 0,
        "worker_count": 0,
        "workers": [],
    }


def test_ait_agent_telegram_supervisor_start_skips_running_workers(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))
    _add_worker("main", repo_root)
    _add_worker("side", repo_root)
    (repo_root / "runtime/main.pid").write_text("1111\n", encoding="utf-8")

    class FakeProcess:
        def __init__(self, pid: int):
            self.pid = pid

    launched: list[list[str]] = []
    alive = {1111: True, 2222: True}

    def fake_popen(command, **kwargs):
        launched.append(command)
        return FakeProcess(2222)

    monkeypatch.setattr(agent_cli.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: alive.get(pid, False))

    result = runner.invoke(agent_cli.app, ["telegram", "supervisor", "start", "--json"])

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["kind"] == "telegram-supervisor"
    assert payload["action"] == "start"
    assert payload["worker_count"] == 2
    assert payload["running_count"] == 2
    assert _worker_by_name(payload, "main")["started"] is False
    side = _worker_by_name(payload, "side")
    assert side["started"] is True
    assert side["pid"] == 2222
    assert launched == [[agent_cli.sys.executable, "-m", "ait_agent.telegram.app"]]
    assert not (repo_root / ".ait/agent-runtime/telegram-side-termination.json").exists()


def test_ait_agent_telegram_supervisor_stop_marks_stopped_and_not_running(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))
    _add_worker("main", repo_root)
    _add_worker("side", repo_root)
    main_pid = repo_root / "runtime/main.pid"
    main_pid.write_text("1111\n", encoding="utf-8")

    alive = {1111: True}
    stopped: list[int] = []

    def fake_stop_pid(pid: int, *, timeout_seconds: float = 10.0) -> str:
        stopped.append(pid)
        alive[pid] = False
        return "stopped"

    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: alive.get(pid, False))
    monkeypatch.setattr(agent_cli, "_stop_pid", fake_stop_pid)

    result = runner.invoke(agent_cli.app, ["telegram", "supervisor", "stop", "--json"])

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["action"] == "stop"
    assert payload["worker_count"] == 2
    assert payload["running_count"] == 0
    main = _worker_by_name(payload, "main")
    assert main["stopped"] is True
    assert main["stop_state"] == "stopped"
    side = _worker_by_name(payload, "side")
    assert side["stopped"] is False
    assert side["stop_state"] == "not_running"
    assert stopped == [1111]
    assert not main_pid.exists()
    termination_context = json.loads(
        (repo_root / ".ait/agent-runtime/telegram-main-termination.json").read_text(encoding="utf-8")
    )
    assert termination_context["pid"] == 1111
    assert termination_context["reason"] == "supervisor_telegram_stop"


def test_ait_agent_telegram_supervisor_restart_handles_mixed_worker_states(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))
    _add_worker("main", repo_root)
    _add_worker("side", repo_root)
    _add_worker("stuck", repo_root)
    (repo_root / "runtime/main.pid").write_text("1111\n", encoding="utf-8")
    (repo_root / "runtime/stuck.pid").write_text("3333\n", encoding="utf-8")

    class FakeProcess:
        def __init__(self, pid: int):
            self.pid = pid

    alive = {1111: True, 3333: True}
    launched: list[list[str]] = []
    new_pids = iter([2222, 4444])

    def fake_popen(command, **kwargs):
        pid = next(new_pids)
        alive[pid] = True
        launched.append(command)
        return FakeProcess(pid)

    def fake_stop_pid(pid: int, *, timeout_seconds: float = 10.0) -> str:
        if pid == 3333:
            return "still_running"
        alive[pid] = False
        return "stopped"

    monkeypatch.setattr(agent_cli.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: alive.get(pid, False))
    monkeypatch.setattr(agent_cli, "_stop_pid", fake_stop_pid)

    result = runner.invoke(agent_cli.app, ["telegram", "supervisor", "restart", "--json"])

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["action"] == "restart"
    assert payload["worker_count"] == 3
    assert payload["running_count"] == 3
    main = _worker_by_name(payload, "main")
    assert main["pid"] == 2222
    assert main["stopped"] is True
    assert main["restarted"] is True
    assert main["restart_blocked"] is False
    side = _worker_by_name(payload, "side")
    assert side["pid"] == 4444
    assert side["stopped"] is False
    assert side["stop_state"] == "not_running"
    assert side["restarted"] is False
    assert side["restart_blocked"] is False
    stuck = _worker_by_name(payload, "stuck")
    assert stuck["pid"] == 3333
    assert stuck["started"] is False
    assert stuck["stopped"] is False
    assert stuck["stop_state"] == "still_running"
    assert stuck["restarted"] is False
    assert stuck["restart_blocked"] is True
    assert len(launched) == 2
    assert not (repo_root / ".ait/agent-runtime/telegram-main-termination.json").exists()
    stuck_context = json.loads(
        (repo_root / ".ait/agent-runtime/telegram-stuck-termination.json").read_text(encoding="utf-8")
    )
    assert stuck_context["pid"] == 3333
    assert stuck_context["reason"] == "supervisor_telegram_restart"


def test_ait_agent_telegram_supervisor_run_starts_stopped_workers(tmp_path, monkeypatch):
    config_path = tmp_path / "agent-workers.json"
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))
    _add_worker("main", repo_root)
    _add_worker("side", repo_root)
    (repo_root / "runtime/main.pid").write_text("1111\n", encoding="utf-8")

    class FakeProcess:
        def __init__(self, pid: int):
            self.pid = pid

    launched: list[list[str]] = []
    alive = {1111: True, 2222: True}

    def fake_popen(command, **kwargs):
        launched.append(command)
        return FakeProcess(2222)

    monkeypatch.setattr(agent_cli.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(agent_cli, "_pid_is_alive", lambda pid: alive.get(pid, False))

    result = runner.invoke(
        agent_cli.app,
        [
            "telegram",
            "supervisor",
            "run",
            "--once",
            "--json",
        ],
    )

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["kind"] == "telegram-supervisor"
    assert payload["action"] == "run"
    assert payload["cycle"] == 1
    assert payload["worker_count"] == 2
    assert payload["started_count"] == 1
    assert payload["running_count"] == 2
    assert _worker_by_name(payload, "main")["start_state"] == "already_running"
    assert _worker_by_name(payload, "side")["start_state"] == "started"
    assert _worker_by_name(payload, "side")["started"] is True
    assert _worker_by_name(payload, "side")["pid"] == 2222
    assert launched == [[agent_cli.sys.executable, "-m", "ait_agent.telegram.app"]]


def test_ait_agent_telegram_supervisor_run_once_cycles_and_interval(monkeypatch):
    calls: dict[str, list[float]] = {"cycles": []}
    sleep_calls: list[float] = []

    monkeypatch.setattr(agent_cli, "_start_stopped_telegram_workers", lambda **_: [])
    monkeypatch.setattr(agent_cli.time, "sleep", lambda seconds: sleep_calls.append(seconds))
    monkeypatch.setattr(
        agent_cli,
        "_emit",
        lambda payload, *, json_output: calls["cycles"].append(payload["cycle"]),
    )

    agent_cli._telegram_supervisor_run(interval_seconds=7.5, once=False, json_output=False, max_cycles=2)

    assert calls["cycles"] == [1, 2]
    assert sleep_calls == [7.5]
