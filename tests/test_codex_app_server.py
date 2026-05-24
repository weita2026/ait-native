from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

import ait_chat.codex_app_server as codex_app_server


class ConnectionClosedOK(Exception):
    pass


class ConnectionClosedError(Exception):
    pass


def test_websocket_close_diagnostics_classifies_normal_close_code():
    exc = ConnectionClosedError("closed normally")
    exc.rcvd = SimpleNamespace(code=1000, reason="normal closure")

    fields = codex_app_server._websocket_close_diagnostics(exc)

    assert fields["close_rcvd_code"] == 1000
    assert fields["close_rcvd_reason"] == "normal closure"
    assert fields["close_kind"] == "normal"


def test_websocket_close_diagnostics_classifies_error_type_without_code():
    fields = codex_app_server._websocket_close_diagnostics(ConnectionClosedError("boom"))

    assert fields["close_kind"] == "abnormal"


def test_websocket_close_diagnostics_classifies_ok_type_without_code():
    fields = codex_app_server._websocket_close_diagnostics(ConnectionClosedOK("bye"))

    assert fields["close_kind"] == "normal"


def test_managed_stderr_log_uses_configured_log_dir(tmp_path, monkeypatch):
    log_dir = tmp_path / "codex-logs"
    monkeypatch.setenv("AIT_CODEX_APP_SERVER_LOG_DIR", str(log_dir))

    handle, path, error = codex_app_server._open_managed_stderr_log(tmp_path, 12345)

    assert error is None
    assert path == log_dir / "codex-app-server-12345.stderr.log"
    assert handle is not None
    try:
        handle.write("stderr line\n")
    finally:
        handle.close()
    text = path.read_text(encoding="utf-8")
    assert "ait managed Codex app-server start" in text
    assert "stderr line" in text


def test_managed_registry_registers_and_unregisters_process(tmp_path: Path, monkeypatch):
    monkeypatch.setenv("AIT_CODEX_APP_SERVER_LOG_DIR", str(tmp_path / "logs"))

    codex_app_server._register_managed_app_server(
        tmp_path,
        pid=12345,
        port=45678,
        listen_url="ws://127.0.0.1:45678",
        bin_path="/Applications/Codex.app/Contents/Resources/codex",
        stderr_log_path=tmp_path / "logs" / "codex-app-server-45678.stderr.log",
    )

    registry_path = codex_app_server._managed_registry_path(tmp_path)
    payload = json.loads(registry_path.read_text(encoding="utf-8"))
    assert payload["processes"][0]["pid"] == 12345
    assert payload["processes"][0]["port"] == 45678

    codex_app_server._unregister_managed_app_server(tmp_path, 12345)

    payload = json.loads(registry_path.read_text(encoding="utf-8"))
    assert payload["processes"] == []


def test_prune_dry_run_targets_only_orphaned_ait_managed_log_ports(tmp_path: Path, monkeypatch):
    log_dir = tmp_path / "logs"
    monkeypatch.setenv("AIT_CODEX_APP_SERVER_LOG_DIR", str(log_dir))
    handle, _path, _error = codex_app_server._open_managed_stderr_log(tmp_path, 45678)
    assert handle is not None
    handle.close()

    monkeypatch.setattr(
        codex_app_server,
        "_list_codex_app_server_processes",
        lambda: [
            codex_app_server.ManagedCodexAppServerProcess(
                pid=101,
                ppid=1,
                command="/Applications/Codex.app/Contents/Resources/codex app-server --listen ws://127.0.0.1:45678",
            ),
            codex_app_server.ManagedCodexAppServerProcess(
                pid=102,
                ppid=999,
                command="/Applications/Codex.app/Contents/Resources/codex app-server --listen ws://127.0.0.1:45678",
            ),
            codex_app_server.ManagedCodexAppServerProcess(
                pid=103,
                ppid=1,
                command="/Applications/Codex.app/Contents/Resources/codex app-server --listen ws://127.0.0.1:55555",
            ),
            codex_app_server.ManagedCodexAppServerProcess(
                pid=104,
                ppid=1,
                command="/Users/me/.vscode/extensions/openai/bin/codex app-server --analytics-default-enabled",
            ),
        ],
    )

    actions = codex_app_server.prune_stale_managed_codex_app_servers(tmp_path, dry_run=True)

    assert actions == [
        {
            "pid": 101,
            "ppid": 1,
            "port": 45678,
            "reason": "orphaned_managed_stderr_log",
            "dry_run": True,
            "result": "would_terminate",
        }
    ]


def test_prune_terminates_orphaned_registry_entries(tmp_path: Path, monkeypatch):
    monkeypatch.setenv("AIT_CODEX_APP_SERVER_LOG_DIR", str(tmp_path / "logs"))
    codex_app_server._register_managed_app_server(
        tmp_path,
        pid=201,
        port=45678,
        listen_url="ws://127.0.0.1:45678",
        bin_path="codex",
        stderr_log_path=None,
    )
    monkeypatch.setattr(
        codex_app_server,
        "_list_codex_app_server_processes",
        lambda: [
            codex_app_server.ManagedCodexAppServerProcess(
                pid=201,
                ppid=1,
                command="/Applications/Codex.app/Contents/Resources/codex app-server --listen ws://127.0.0.1:45678",
            )
        ],
    )
    terminated: list[int] = []
    monkeypatch.setattr(
        codex_app_server,
        "_terminate_process",
        lambda pid, *, kill_grace_seconds: terminated.append(pid) or "terminated",
    )

    actions = codex_app_server.prune_stale_managed_codex_app_servers(tmp_path)

    assert terminated == [201]
    assert actions[0]["reason"] == "orphaned_registry_entry"
    assert codex_app_server._read_managed_registry(tmp_path) == {}
