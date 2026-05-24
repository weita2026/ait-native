import json
import signal
from pathlib import Path

import ait_server.app as server_app
import ait_server.server_process_runtime as server_process_runtime


def test_server_termination_context_path_defaults_to_pid_file_stem(tmp_path: Path):
    env = {"AIT_SERVER_PID_FILE": str(tmp_path / "ait-server.pid")}

    path = server_app._server_termination_context_path(env)

    assert path == tmp_path / "ait-server.termination.json"


def test_server_termination_context_path_falls_back_to_log_dir(tmp_path: Path):
    env = {"AIT_LOG_DIR": str(tmp_path)}

    path = server_app._server_termination_context_path(env)

    assert path == tmp_path / "ait-server-termination.json"


def test_server_consume_pending_termination_context_only_returns_matching_pid(tmp_path: Path):
    context_path = tmp_path / "ait-server.termination.json"
    context_path.write_text(
        json.dumps(
            {
                "pid": 4242,
                "reason": "backend_server_restart",
                "worker_name": "ait-backend",
                "issued_at": "2026-05-09T03:47:18+00:00",
                "issued_by_pid": 999,
            }
        ),
        encoding="utf-8",
    )
    env = {server_app.AIT_SERVER_TERMINATION_CONTEXT_ENV: str(context_path)}

    assert server_app._consume_pending_server_termination_context(pid=1111, env=env) is None
    assert context_path.exists()

    payload = server_app._consume_pending_server_termination_context(pid=4242, env=env)

    assert payload is not None
    assert payload["reason"] == "backend_server_restart"
    assert not context_path.exists()


def test_server_signal_stop_suffix_includes_context_fields(tmp_path: Path, monkeypatch):
    context_path = tmp_path / "ait-server.termination.json"
    context_path.write_text(
        json.dumps(
            {
                "pid": 5150,
                "reason": "backend_server_stop",
                "worker_name": "ait-backend",
                "issued_at": "2026-05-09T03:47:18+00:00",
                "issued_by_pid": 18430,
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setenv(server_app.AIT_SERVER_TERMINATION_CONTEXT_ENV, str(context_path))
    monkeypatch.setattr(server_app.os, "getpid", lambda: 5150)

    suffix = server_app._server_signal_stop_suffix(signal.SIGTERM)

    assert "signal=15" in suffix
    assert "reason=backend_server_stop" in suffix
    assert "worker=ait-backend" in suffix
    assert "issued_at=2026-05-09T03:47:18+00:00" in suffix
    assert "issued_by_pid=18430" in suffix
    assert not context_path.exists()


def test_server_runtime_identity_writes_and_clears_pid_file(tmp_path: Path):
    pid_file = tmp_path / "ait-server.pid"
    env = {"AIT_SERVER_PID_FILE": str(pid_file)}

    with server_app._server_runtime_identity(pid=4242, env=env):
        assert pid_file.read_text(encoding="utf-8") == "4242\n"

    assert not pid_file.exists()


def test_server_runtime_identity_preserves_foreign_pid_file_on_cleanup(tmp_path: Path):
    pid_file = tmp_path / "ait-server.pid"
    env = {"AIT_SERVER_PID_FILE": str(pid_file)}

    with server_app._server_runtime_identity(pid=4242, env=env):
        pid_file.write_text("9999\n", encoding="utf-8")

    assert pid_file.read_text(encoding="utf-8") == "9999\n"


def test_server_startup_identity_summary_includes_runtime_diagnostics(tmp_path: Path, monkeypatch):
    pid_file = tmp_path / "ait-server.pid"
    termination_path = tmp_path / "ait-server.termination.json"
    env = {
        "AIT_SERVER_PID_FILE": str(pid_file),
        server_app.AIT_SERVER_TERMINATION_CONTEXT_ENV: str(termination_path),
    }
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(server_process_runtime.os, "getppid", lambda: 5151)
    monkeypatch.setattr(server_process_runtime.sys, "argv", ["ait-server", "--host", "127.0.0.1"])
    monkeypatch.setattr(server_process_runtime, "_process_command_for_pid", lambda pid: f"parent-cmd {pid}")

    summary = server_app._server_startup_identity_summary(pid=4242, env=env)

    assert "pid=4242" in summary
    assert "ppid=5151" in summary
    assert f"cwd={tmp_path}" in summary
    assert "argv=ait-server --host 127.0.0.1" in summary
    assert f"pid_file={pid_file}" in summary
    assert f"termination_context_path={termination_path}" in summary
    assert "parent_command=parent-cmd 5151" in summary
