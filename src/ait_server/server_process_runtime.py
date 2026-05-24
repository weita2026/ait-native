from __future__ import annotations

import json
import os
import subprocess
import sys
from contextlib import contextmanager
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import Any, Mapping

AIT_SERVER_TERMINATION_CONTEXT_ENV = "AIT_SERVER_TERMINATION_CONTEXT_PATH"


def _server_pid_file_path(env: Mapping[str, str] | None = None) -> Path | None:
    source_env = os.environ if env is None else env
    raw = str(source_env.get("AIT_SERVER_PID_FILE") or "").strip()
    if not raw:
        return None
    return Path(raw).expanduser()


def _server_termination_context_path(env: Mapping[str, str] | None = None) -> Path | None:
    source_env = os.environ if env is None else env
    raw = str(source_env.get(AIT_SERVER_TERMINATION_CONTEXT_ENV) or "").strip()
    if raw:
        return Path(raw).expanduser()
    pid_file_path = _server_pid_file_path(source_env)
    if pid_file_path is not None:
        return pid_file_path.with_suffix(".termination.json")
    log_dir = str(source_env.get("AIT_LOG_DIR") or "").strip()
    if log_dir:
        return Path(log_dir).expanduser() / "ait-server-termination.json"
    return None


def _write_server_pid_file(path: Path, *, pid: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as handle:
        handle.write(f"{pid}\n")
        tmp_path = Path(handle.name)
    tmp_path.replace(path)


def _clear_server_pid_file(path: Path, *, pid: int | None = None) -> None:
    if not path.exists():
        return
    if pid is not None:
        try:
            raw = path.read_text(encoding="utf-8").strip()
        except OSError:
            return
        try:
            recorded_pid = int(raw or "0")
        except ValueError:
            return
        if recorded_pid != pid:
            return
    try:
        path.unlink(missing_ok=True)
    except OSError:
        pass


def _consume_pending_server_termination_context(
    *,
    pid: int | None = None,
    env: Mapping[str, str] | None = None,
) -> dict[str, Any] | None:
    path = _server_termination_context_path(env)
    if path is None or not path.exists():
        return None
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    expected_pid = os.getpid() if pid is None else pid
    try:
        context_pid = int(payload.get("pid") or 0)
    except (TypeError, ValueError):
        return None
    if context_pid != expected_pid:
        return None
    try:
        path.unlink(missing_ok=True)
    except OSError:
        pass
    return payload


def _server_signal_stop_suffix(signum: int) -> str:
    payload = _consume_pending_server_termination_context()
    if not payload:
        return ""
    details: list[str] = [f"signal={signum}"]
    reason = str(payload.get("reason") or "").strip()
    if reason:
        details.append(f"reason={reason}")
    worker_name = str(payload.get("worker_name") or "").strip()
    if worker_name:
        details.append(f"worker={worker_name}")
    issued_at = str(payload.get("issued_at") or "").strip()
    if issued_at:
        details.append(f"issued_at={issued_at}")
    issued_by_pid = payload.get("issued_by_pid")
    if issued_by_pid is not None:
        details.append(f"issued_by_pid={issued_by_pid}")
    return f" ({', '.join(details)})"


def _process_command_for_pid(pid: int) -> str | None:
    try:
        result = subprocess.run(
            ["ps", "-p", str(pid), "-o", "command="],
            check=False,
            capture_output=True,
            text=True,
        )
    except Exception:
        return None
    if result.returncode != 0:
        return None
    command = result.stdout.strip()
    return command or None


def _server_startup_identity_summary(
    *,
    pid: int | None = None,
    env: Mapping[str, str] | None = None,
) -> str:
    current_pid = os.getpid() if pid is None else pid
    parent_pid = os.getppid()
    details = [
        f"pid={current_pid}",
        f"ppid={parent_pid}",
        f"cwd={Path.cwd()}",
        f"argv={' '.join(sys.argv)}",
    ]
    pid_file_path = _server_pid_file_path(env)
    if pid_file_path is not None:
        details.append(f"pid_file={pid_file_path}")
    termination_context_path = _server_termination_context_path(env)
    if termination_context_path is not None:
        details.append(f"termination_context_path={termination_context_path}")
    parent_command = _process_command_for_pid(parent_pid)
    if parent_command:
        details.append(f"parent_command={parent_command}")
    return ", ".join(details)


@contextmanager
def _server_runtime_identity(pid: int | None = None, env: Mapping[str, str] | None = None):
    current_pid = os.getpid() if pid is None else pid
    pid_file_path = _server_pid_file_path(env)
    if pid_file_path is not None:
        _write_server_pid_file(pid_file_path, pid=current_pid)
    try:
        yield
    finally:
        if pid_file_path is not None:
            _clear_server_pid_file(pid_file_path, pid=current_pid)
