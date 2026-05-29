from __future__ import annotations

import json
import os
import re
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import time
from collections import deque
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.error import URLError
from urllib.parse import urlsplit, urlunsplit
from urllib.request import Request, urlopen

from .runtime_config import DEFAULT_REPLY_CODEX_CHILD_REAP_TIMEOUT_SECONDS


DEFAULT_CODEX_APP_SERVER_HOST = "127.0.0.1"
DEFAULT_CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES = 64 * 1024 * 1024
NORMAL_WEBSOCKET_CLOSE_CODES = {1000, 1001}
DEFAULT_CODEX_CHILD_REAP_RETRY_ATTEMPTS = 5
DEFAULT_CODEX_CHILD_REAP_MIN_RETRY_INTERVAL_SECONDS = 0.1
MANAGED_APP_SERVER_REGISTRY_FILENAME = "codex-app-server-registry.json"
MANAGED_STDERR_LOG_RE = re.compile(r"^codex-app-server-(?P<port>[0-9]+)\.stderr\.log$")


class CodexAppServerError(RuntimeError):
    pass


@dataclass(frozen=True)
class CodexAppServerConfig:
    repo_root: Path
    bin_path: str
    model: str
    reasoning_effort: str | None
    sandbox: str
    app_server_url: str | None = None
    app_server_host: str = DEFAULT_CODEX_APP_SERVER_HOST
    app_server_port: int = 0
    ready_timeout_seconds: float = 30.0
    turn_timeout_seconds: float | None = None
    child_kill_grace_seconds: float = 2.0
    child_reap_timeout_seconds: float = DEFAULT_REPLY_CODEX_CHILD_REAP_TIMEOUT_SECONDS
    websocket_max_size_bytes: int | None = DEFAULT_CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES


@dataclass(frozen=True)
class CodexTurnResult:
    text: str
    turn_id: str | None
    usage: dict[str, Any] | None = None
    command_executions: tuple[dict[str, Any], ...] = ()


@dataclass(frozen=True)
class ManagedCodexAppServerProcess:
    pid: int
    ppid: int
    command: str


def resolve_codex_bin(configured_bin: str | None) -> str:
    raw = str(configured_bin or "").strip()
    if raw:
        return raw
    stable_candidates = [
        "/Applications/Codex.app/Contents/Resources/codex",
        str(Path.home() / "Applications" / "Codex.app" / "Contents" / "Resources" / "codex"),
    ]
    for candidate in stable_candidates:
        if candidate and os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate
    return shutil.which("codex") or "codex"


def create_ready_url(ws_url: str) -> str:
    parts = urlsplit(ws_url)
    scheme = "https" if parts.scheme == "wss" else "http"
    return urlunsplit((scheme, parts.netloc, "/readyz", "", ""))


def get_available_port(host: str) -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind((host, 0))
        sock.listen(1)
        address = sock.getsockname()
    return int(address[1])


def _log_now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def _compact_log_value(value: Any, *, limit: int = 180) -> str:
    text = str(value).replace("\n", "\\n").strip()
    if len(text) <= limit:
        return text
    return f"{text[: max(limit - 1, 0)]}…"


def _format_log_fields(fields: dict[str, Any]) -> str:
    rendered: list[str] = []
    for key, value in fields.items():
        if value is None:
            continue
        text = _compact_log_value(value)
        if not text:
            continue
        rendered.append(f"{key}={text}")
    return " ".join(rendered)


def _log_codex_ws(event: str, **fields: Any) -> None:
    suffix = _format_log_fields(fields)
    line = f"{_log_now_iso()} ait codex websocket {event}"
    if suffix:
        line = f"{line} {suffix}"
    print(line, file=sys.stderr, flush=True)


def _elapsed_seconds(start: float) -> str:
    return f"{max(time.monotonic() - start, 0.0):.2f}"


def _child_reap_wait_intervals(
    total_timeout_seconds: float,
    *,
    max_attempts: int = DEFAULT_CODEX_CHILD_REAP_RETRY_ATTEMPTS,
    minimum_interval_seconds: float = DEFAULT_CODEX_CHILD_REAP_MIN_RETRY_INTERVAL_SECONDS,
) -> tuple[float, ...]:
    budget = max(float(total_timeout_seconds or 0.0), float(minimum_interval_seconds or 0.1))
    attempts = 1
    min_interval = max(float(minimum_interval_seconds or 0.1), 0.01)
    max_attempts = max(int(max_attempts or 1), 1)
    for candidate in range(max_attempts, 0, -1):
        weight_sum = candidate * (candidate + 1) / 2.0
        if budget / weight_sum >= min_interval:
            attempts = candidate
            break
    weight_sum = attempts * (attempts + 1) / 2.0
    intervals = [budget * (index + 1) / weight_sum for index in range(attempts)]
    if intervals:
        intervals[-1] = max(budget - sum(intervals[:-1]), min_interval)
    return tuple(intervals)


def _managed_stderr_log_path(repo_root: Path, port: int) -> Path:
    raw_dir = os.getenv("AIT_CODEX_APP_SERVER_LOG_DIR") or os.getenv("AIT_LOG_DIR")
    log_dir = Path(raw_dir).expanduser() if raw_dir else repo_root / ".ait" / "logs"
    return log_dir / f"codex-app-server-{port}.stderr.log"


def _managed_log_dir(repo_root: Path) -> Path:
    return _managed_stderr_log_path(repo_root, 0).parent


def _managed_registry_path(repo_root: Path) -> Path:
    raw_path = os.getenv("AIT_CODEX_APP_SERVER_REGISTRY_PATH")
    if raw_path:
        return Path(raw_path).expanduser()
    return _managed_log_dir(repo_root) / MANAGED_APP_SERVER_REGISTRY_FILENAME


def _read_managed_registry(repo_root: Path) -> dict[str, dict[str, Any]]:
    path = _managed_registry_path(repo_root)
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    rows = payload.get("processes") if isinstance(payload, dict) else payload
    if not isinstance(rows, list):
        return {}
    registry: dict[str, dict[str, Any]] = {}
    for row in rows:
        if not isinstance(row, dict):
            continue
        pid = str(row.get("pid") or "").strip()
        if pid.isdigit():
            registry[pid] = dict(row)
    return registry


def _write_managed_registry(repo_root: Path, registry: dict[str, dict[str, Any]]) -> None:
    path = _managed_registry_path(repo_root)
    rows = sorted(registry.values(), key=lambda row: int(row.get("pid") or 0))
    payload = {"version": 1, "updated_at": _log_now_iso(), "processes": rows}
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp_path = path.with_suffix(path.suffix + ".tmp")
    tmp_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    tmp_path.replace(path)


def _register_managed_app_server(
    repo_root: Path,
    *,
    pid: int,
    port: int,
    listen_url: str,
    bin_path: str,
    stderr_log_path: Path | None,
) -> None:
    try:
        registry = _read_managed_registry(repo_root)
        registry[str(pid)] = {
            "pid": int(pid),
            "parent_pid": os.getpid(),
            "port": int(port),
            "listen_url": listen_url,
            "repo_root": str(repo_root),
            "bin_path": bin_path,
            "stderr_log": str(stderr_log_path) if stderr_log_path is not None else None,
            "started_at": _log_now_iso(),
        }
        _write_managed_registry(repo_root, registry)
    except OSError as exc:
        _log_codex_ws("app_server.registry_write_failed", pid=pid, port=port, error=str(exc))


def _unregister_managed_app_server(repo_root: Path, pid: int | None) -> None:
    if pid is None:
        return
    try:
        registry = _read_managed_registry(repo_root)
        if registry.pop(str(pid), None) is not None:
            _write_managed_registry(repo_root, registry)
    except OSError as exc:
        _log_codex_ws("app_server.registry_remove_failed", pid=pid, error=str(exc))


def _managed_log_ports(repo_root: Path) -> set[int]:
    log_dir = _managed_log_dir(repo_root)
    ports: set[int] = set()
    try:
        rows = list(log_dir.iterdir())
    except OSError:
        return ports
    for path in rows:
        match = MANAGED_STDERR_LOG_RE.match(path.name)
        if not match:
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        if "ait managed Codex app-server start" not in text:
            continue
        ports.add(int(match.group("port")))
    return ports


def _list_codex_app_server_processes() -> list[ManagedCodexAppServerProcess]:
    try:
        result = subprocess.run(
            ["ps", "-axo", "pid=,ppid=,command="],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError:
        return []
    rows: list[ManagedCodexAppServerProcess] = []
    for line in result.stdout.splitlines():
        parts = line.strip().split(None, 2)
        if len(parts) != 3:
            continue
        pid_text, ppid_text, command = parts
        if not (pid_text.isdigit() and ppid_text.isdigit()):
            continue
        if "app-server" not in command or "codex" not in command:
            continue
        rows.append(ManagedCodexAppServerProcess(pid=int(pid_text), ppid=int(ppid_text), command=command))
    return rows


def _managed_process_listen_port(process: ManagedCodexAppServerProcess) -> int | None:
    if "app-server" not in process.command or "--listen" not in process.command:
        return None
    match = re.search(r"--listen(?:=|\s+)(?:ws://|wss://)?[^:\s]+:(?P<port>[0-9]+)", process.command)
    if not match:
        return None
    return int(match.group("port"))


def _is_codex_app_server_command(command: str) -> bool:
    return "codex" in command and "app-server" in command and "--listen" in command


def _pid_is_alive(pid: int) -> bool:
    if pid <= 0:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _terminate_process(pid: int, *, kill_grace_seconds: float) -> str:
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        return "already_exited"
    except OSError as exc:
        return f"term_failed:{exc}"
    deadline = time.monotonic() + max(float(kill_grace_seconds), 0.0)
    while time.monotonic() < deadline:
        if not _pid_is_alive(pid):
            return "terminated"
        time.sleep(0.05)
    if not _pid_is_alive(pid):
        return "terminated"
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        return "terminated"
    except OSError as exc:
        return f"kill_failed:{exc}"
    return "killed"


def prune_stale_managed_codex_app_servers(
    repo_root: Path,
    *,
    dry_run: bool = False,
    kill_grace_seconds: float = 2.0,
    include_log_orphans: bool = True,
    exclude_pids: set[int] | None = None,
) -> list[dict[str, Any]]:
    """Terminate orphaned AIT-managed Codex app-server processes.

    Safety rules are intentionally conservative:
    - registry-backed rows are eligible only after they become orphaned
      (`ppid == 1`);
    - legacy log-backed rows are eligible only when their listening port has an
      AIT-managed stderr log and they are also orphaned;
    - current or explicitly excluded PIDs are never touched.
    """

    repo_root = Path(repo_root)
    excluded = set(exclude_pids or set())
    excluded.add(os.getpid())
    registry = _read_managed_registry(repo_root)
    managed_ports = _managed_log_ports(repo_root) if include_log_orphans else set()
    actions: list[dict[str, Any]] = []
    for process in _list_codex_app_server_processes():
        if process.pid in excluded:
            continue
        if process.ppid != 1:
            continue
        if not _is_codex_app_server_command(process.command):
            continue
        registry_row = registry.get(str(process.pid))
        port = _managed_process_listen_port(process)
        reason: str | None = None
        if registry_row is not None:
            reason = "orphaned_registry_entry"
        elif include_log_orphans and port is not None and port in managed_ports:
            reason = "orphaned_managed_stderr_log"
        if reason is None:
            continue
        action = {
            "pid": process.pid,
            "ppid": process.ppid,
            "port": port,
            "reason": reason,
            "dry_run": bool(dry_run),
        }
        if dry_run:
            action["result"] = "would_terminate"
        else:
            action["result"] = _terminate_process(process.pid, kill_grace_seconds=kill_grace_seconds)
            registry.pop(str(process.pid), None)
        actions.append(action)
    if actions and not dry_run:
        try:
            _write_managed_registry(repo_root, registry)
        except OSError as exc:
            _log_codex_ws("app_server.registry_prune_write_failed", error=str(exc))
    return actions


def _open_managed_stderr_log(repo_root: Path, port: int) -> tuple[Any | None, Path | None, str | None]:
    path = _managed_stderr_log_path(repo_root, port)
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        handle = path.open("a", encoding="utf-8", buffering=1)
    except OSError as exc:
        return None, path, str(exc)
    handle.write(f"\n--- ait managed Codex app-server start {datetime.now(timezone.utc).isoformat()} port={port} ---\n")
    return handle, path, None


def _close_frame_fields(prefix: str, frame: Any) -> dict[str, Any]:
    if frame is None:
        return {}
    fields: dict[str, Any] = {}
    code = getattr(frame, "code", None)
    reason = getattr(frame, "reason", None)
    if code is not None:
        fields[f"{prefix}_code"] = code
    if reason:
        fields[f"{prefix}_reason"] = reason
    return fields


def _websocket_close_diagnostics(exc: BaseException) -> dict[str, Any]:
    fields: dict[str, Any] = {}
    close_code = getattr(exc, "code", None)
    close_reason = getattr(exc, "reason", None)
    if close_code is not None:
        fields["close_code"] = close_code
    if close_reason:
        fields["close_reason"] = close_reason
    fields.update(_close_frame_fields("close_rcvd", getattr(exc, "rcvd", None)))
    fields.update(_close_frame_fields("close_sent", getattr(exc, "sent", None)))
    rcvd_then_sent = getattr(exc, "rcvd_then_sent", None)
    if rcvd_then_sent is not None:
        fields["close_rcvd_then_sent"] = rcvd_then_sent

    error_type = type(exc).__name__
    observed_codes = [
        int(code)
        for code in (
            fields.get("close_code"),
            fields.get("close_rcvd_code"),
            fields.get("close_sent_code"),
        )
        if isinstance(code, int)
    ]
    if error_type == "ConnectionClosedOK" or any(code in NORMAL_WEBSOCKET_CLOSE_CODES for code in observed_codes):
        fields["close_kind"] = "normal"
    elif error_type == "ConnectionClosedError":
        fields["close_kind"] = "abnormal"
    else:
        fields["close_kind"] = "unknown"
    return fields


def _command_preview(command_execution: dict[str, Any]) -> str:
    actions = command_execution.get("commandActions")
    if isinstance(actions, list):
        for row in actions:
            if not isinstance(row, dict):
                continue
            command = str(row.get("command") or "").strip()
            if command:
                return _compact_log_value(command)
    raw = str(command_execution.get("command") or "").strip()
    if not raw:
        return "(unknown command)"
    try:
        tokens = shlex.split(raw)
    except ValueError:
        return _compact_log_value(raw)
    if len(tokens) >= 3 and tokens[1] in {"-lc", "-c"}:
        return _compact_log_value(tokens[2])
    return _compact_log_value(raw)


class CodexAppServerClient:
    def __init__(self, config: CodexAppServerConfig):
        self._config = config
        self._child: subprocess.Popen[bytes] | None = None
        self._child_started_at: float | None = None
        self._managed_pid: int | None = None
        self._stderr_handle: Any | None = None
        self._stderr_log_path: Path | None = None
        self._ws: Any = None
        self._url: str | None = None
        self._next_id = 1
        self._notification_queue: deque[dict[str, Any]] = deque()

    def __enter__(self) -> CodexAppServerClient:
        self.start()
        return self

    def _trace(self, event: str, trace_context: dict[str, Any] | None = None, **fields: Any) -> None:
        payload: dict[str, Any] = {}
        if trace_context:
            payload.update(trace_context)
        payload.update(fields)
        _log_codex_ws(event, **payload)

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

    @property
    def is_started(self) -> bool:
        return self._ws is not None

    def start(self) -> None:
        if self._ws is not None:
            return
        target_url = self._config.app_server_url or self._start_managed_server()
        self._connect_websocket(target_url)
        self.send_request(
            "initialize",
            {
                "clientInfo": {
                    "name": "ait",
                    "title": "ait Telegram reply path",
                    "version": "0.10.6",
                },
                "capabilities": {"experimentalApi": True},
            },
            timeout=self._config.ready_timeout_seconds,
        )
        self.send_notification("initialized")

    def resume_thread(
        self,
        thread_id: str,
        *,
        base_instructions: str,
        developer_instructions: str,
        persist_extended_history: bool = False,
        trace_context: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        response = self.send_request(
            "thread/resume",
            {
                "threadId": thread_id,
                "cwd": str(self._config.repo_root),
                "model": self._config.model,
                "approvalPolicy": "never",
                "sandbox": self._config.sandbox,
                "baseInstructions": base_instructions,
                "developerInstructions": developer_instructions,
                "persistExtendedHistory": persist_extended_history,
            },
            timeout=self._config.ready_timeout_seconds,
        )
        thread = response.get("thread") if isinstance(response, dict) else None
        if not isinstance(thread, dict):
            thread = {"id": thread_id}
        if not str(thread.get("id") or "").strip():
            raise CodexAppServerError("Codex app-server did not return a resumed thread id.")
        self._trace(
            "thread.resumed",
            trace_context,
            thread_id=str(thread.get("id") or "").strip(),
            model=self._config.model,
            sandbox=self._config.sandbox,
            persist_extended_history=persist_extended_history,
        )
        return thread

    def start_thread(
        self,
        *,
        base_instructions: str,
        developer_instructions: str,
        persist_extended_history: bool = False,
        trace_context: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        response = self.send_request(
            "thread/start",
            {
                "cwd": str(self._config.repo_root),
                "model": self._config.model,
                "approvalPolicy": "never",
                "sandbox": self._config.sandbox,
                "baseInstructions": base_instructions,
                "developerInstructions": developer_instructions,
                "experimentalRawEvents": False,
                "persistExtendedHistory": persist_extended_history,
            },
            timeout=self._config.ready_timeout_seconds,
        )
        thread = response.get("thread") if isinstance(response, dict) else None
        if not isinstance(thread, dict) or not str(thread.get("id") or "").strip():
            raise CodexAppServerError("Codex app-server did not return a thread id.")
        self._trace(
            "thread.started",
            trace_context,
            thread_id=str(thread.get("id") or "").strip(),
            model=self._config.model,
            sandbox=self._config.sandbox,
            persist_extended_history=persist_extended_history,
        )
        return thread

    def run_turn(
        self,
        *,
        thread_id: str,
        input_text: str,
        trace_context: dict[str, Any] | None = None,
    ) -> CodexTurnResult:
        started_at = time.monotonic()
        turn_id: str | None = None
        text_chunks: list[str] = []
        final_text = ""
        token_usage: dict[str, Any] | None = None
        command_items: dict[str, dict[str, Any]] = {}
        command_order: list[str] = []
        self._trace(
            "turn.requested",
            trace_context,
            thread_id=thread_id,
            model=self._config.model,
            effort=self._config.reasoning_effort,
            input_chars=len(input_text),
        )
        try:
            response = self.send_request(
                "turn/start",
                {
                    "threadId": thread_id,
                    "cwd": str(self._config.repo_root),
                    "model": self._config.model,
                    "sandbox": self._config.sandbox,
                    "approvalPolicy": "never",
                    "effort": self._config.reasoning_effort,
                    "input": [{"type": "text", "text": input_text, "text_elements": []}],
                },
                timeout=self._config.ready_timeout_seconds,
            )
            result_turn = response.get("turn") if isinstance(response, dict) else None
            if not isinstance(result_turn, dict):
                raise CodexAppServerError("Codex app-server did not return a turn payload.")
            turn_id = str(result_turn.get("id") or "").strip() or None
            status = str(result_turn.get("status") or "").strip().lower()
            self._trace(
                "turn.accepted",
                trace_context,
                thread_id=thread_id,
                turn_id=turn_id,
                initial_status=status or "(missing)",
            )
            if status == "failed":
                raise CodexAppServerError(format_turn_error(result_turn.get("error")))
            if status == "completed":
                reply = final_text.strip() or "".join(text_chunks).strip()
                self._trace(
                    "turn.completed",
                    trace_context,
                    thread_id=thread_id,
                    turn_id=turn_id,
                    elapsed_sec=_elapsed_seconds(started_at),
                    command_count=0,
                    output_chars=len(reply or ""),
                )
                return CodexTurnResult(text=reply or "No response text returned.", turn_id=turn_id)
            deadline = None
            if self._config.turn_timeout_seconds is not None:
                deadline = time.monotonic() + float(self._config.turn_timeout_seconds)
            while True:
                if self._notification_queue:
                    message = self._notification_queue.popleft()
                else:
                    message = self._recv_json(deadline=deadline)
                method = str(message.get("method") or "")
                params = message.get("params") if isinstance(message.get("params"), dict) else {}
                if params.get("threadId") != thread_id:
                    continue
                if method == "turn/started":
                    next_turn_id = str(((params.get("turn") or {}).get("id") or "")).strip()
                    if next_turn_id:
                        turn_id = next_turn_id
                    self._trace(
                        "turn.started",
                        trace_context,
                        thread_id=thread_id,
                        turn_id=turn_id,
                        elapsed_sec=_elapsed_seconds(started_at),
                    )
                    continue
                if method == "item/agentMessage/delta":
                    if turn_id is not None and str(params.get("turnId") or "").strip() not in {"", turn_id}:
                        continue
                    delta = str(params.get("delta") or "")
                    if delta:
                        text_chunks.append(delta)
                    continue
                if method == "thread/tokenUsage/updated":
                    if turn_id is not None and str(params.get("turnId") or "").strip() not in {"", turn_id}:
                        continue
                    usage = params.get("tokenUsage")
                    if isinstance(usage, dict):
                        token_usage = usage
                    continue
                if method == "item/started":
                    if turn_id is not None and str(params.get("turnId") or "").strip() not in {"", turn_id}:
                        continue
                    item = params.get("item") if isinstance(params.get("item"), dict) else {}
                    if item.get("type") == "commandExecution":
                        item_id = str(item.get("id") or "").strip()
                        if item_id:
                            command_items[item_id] = dict(item)
                            command_order.append(item_id)
                            self._trace(
                                "turn.command_started",
                                trace_context,
                                thread_id=thread_id,
                                turn_id=turn_id,
                                command_id=item_id,
                                command=_command_preview(item),
                                elapsed_sec=_elapsed_seconds(started_at),
                            )
                    continue
                if method == "item/completed":
                    if turn_id is not None and str(params.get("turnId") or "").strip() not in {"", turn_id}:
                        continue
                    item = params.get("item") if isinstance(params.get("item"), dict) else {}
                    if item.get("type") == "agentMessage":
                        text = str(item.get("text") or "").strip()
                        if text:
                            final_text = text
                    elif item.get("type") == "commandExecution":
                        item_id = str(item.get("id") or "").strip()
                        if item_id:
                            if item_id not in command_items:
                                command_order.append(item_id)
                            command_items[item_id] = dict(item)
                            self._trace(
                                "turn.command_completed",
                                trace_context,
                                thread_id=thread_id,
                                turn_id=turn_id,
                                command_id=item_id,
                                command=_command_preview(item),
                                exit_code=item.get("exitCode"),
                                elapsed_sec=_elapsed_seconds(started_at),
                            )
                    continue
                if method == "error":
                    if turn_id is not None and str(params.get("turnId") or "").strip() not in {"", turn_id}:
                        continue
                    raise CodexAppServerError(format_turn_error(params.get("error")))
                if method != "turn/completed":
                    continue
                turn = params.get("turn") if isinstance(params.get("turn"), dict) else {}
                completed_turn_id = str(turn.get("id") or "").strip()
                if turn_id is not None and completed_turn_id not in {"", turn_id}:
                    continue
                if completed_turn_id:
                    turn_id = completed_turn_id
                completed_status = str(turn.get("status") or "").strip().lower()
                if completed_status == "failed":
                    raise CodexAppServerError(format_turn_error(turn.get("error")))
                reply = final_text.strip() or "".join(text_chunks).strip()
                ordered_commands = tuple(command_items[item_id] for item_id in command_order if item_id in command_items)
                self._trace(
                    "turn.completed",
                    trace_context,
                    thread_id=thread_id,
                    turn_id=turn_id,
                    elapsed_sec=_elapsed_seconds(started_at),
                    command_count=len(ordered_commands),
                    output_chars=len(reply or ""),
                )
                return CodexTurnResult(
                    text=reply or "No response text returned.",
                    turn_id=turn_id,
                    usage=token_usage,
                    command_executions=ordered_commands,
                )
        except CodexAppServerError as exc:
            self._trace(
                "turn.failed",
                trace_context,
                thread_id=thread_id,
                turn_id=turn_id,
                elapsed_sec=_elapsed_seconds(started_at),
                command_count=len(command_order),
                error=str(exc),
            )
            raise

    def send_notification(self, method: str, params: dict[str, Any] | None = None) -> None:
        payload: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            payload["params"] = params
        self._send_json(payload)

    def send_request(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        timeout: float | None,
    ) -> dict[str, Any]:
        request_id = self._next_id
        self._next_id += 1
        payload: dict[str, Any] = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            payload["params"] = params
        self._send_json(payload)
        deadline = None if timeout is None else time.monotonic() + timeout
        while True:
            message = self._recv_json(deadline=deadline)
            message_id = message.get("id")
            if message_id != request_id:
                self._notification_queue.append(message)
                continue
            if message.get("error") is not None:
                raise CodexAppServerError(format_rpc_error(message.get("error")))
            result = message.get("result")
            if isinstance(result, dict):
                return result
            return {}

    def close(self) -> None:
        ws = self._ws
        self._ws = None
        if ws is not None:
            try:
                ws.close()
            except Exception:
                pass
        child = self._child
        self._child = None
        managed_pid = self._managed_pid
        self._managed_pid = None
        started_at = self._child_started_at
        self._child_started_at = None
        stderr_log_path = self._stderr_log_path
        self._stderr_log_path = None
        stderr_handle = self._stderr_handle
        self._stderr_handle = None
        self._url = None
        if child is None:
            if stderr_handle is not None:
                stderr_handle.close()
            _unregister_managed_app_server(self._config.repo_root, managed_pid)
            return
        if child.poll() is not None:
            _log_codex_ws(
                "app_server.child_already_exited",
                pid=child.pid,
                return_code=child.returncode,
                child_elapsed_sec=_elapsed_seconds(started_at) if started_at is not None else None,
                stderr_log=stderr_log_path,
            )
            if stderr_handle is not None:
                stderr_handle.close()
            _unregister_managed_app_server(self._config.repo_root, managed_pid or child.pid)
            return
        try:
            child.send_signal(signal.SIGTERM)
        except OSError:
            if stderr_handle is not None:
                stderr_handle.close()
            _unregister_managed_app_server(self._config.repo_root, managed_pid or child.pid)
            return
        try:
            child.wait(timeout=self._config.child_kill_grace_seconds)
            _log_codex_ws(
                "app_server.child_stopped",
                pid=child.pid,
                return_code=child.returncode,
                child_elapsed_sec=_elapsed_seconds(started_at) if started_at is not None else None,
                stderr_log=stderr_log_path,
            )
            if stderr_handle is not None:
                stderr_handle.close()
            _unregister_managed_app_server(self._config.repo_root, managed_pid or child.pid)
            return
        except subprocess.TimeoutExpired:
            pass
        try:
            child.kill()
        except OSError:
            if stderr_handle is not None:
                stderr_handle.close()
            _unregister_managed_app_server(self._config.repo_root, managed_pid or child.pid)
            return
        reap_intervals = _child_reap_wait_intervals(self._config.child_reap_timeout_seconds)
        try:
            for attempt, wait_timeout in enumerate(reap_intervals, start=1):
                try:
                    child.wait(timeout=wait_timeout)
                    _log_codex_ws(
                        "app_server.child_killed",
                        pid=child.pid,
                        return_code=child.returncode,
                        child_elapsed_sec=_elapsed_seconds(started_at) if started_at is not None else None,
                        stderr_log=stderr_log_path,
                        reap_attempt=attempt,
                        reap_attempts_total=len(reap_intervals),
                        reap_wait_budget_seconds=self._config.child_reap_timeout_seconds,
                    )
                    break
                except subprocess.TimeoutExpired:
                    if attempt >= len(reap_intervals):
                        _log_codex_ws(
                            "app_server.child_kill_timeout",
                            pid=child.pid,
                            child_elapsed_sec=_elapsed_seconds(started_at) if started_at is not None else None,
                            stderr_log=stderr_log_path,
                            reap_attempt=attempt,
                            reap_attempts_total=len(reap_intervals),
                            reap_wait_budget_seconds=self._config.child_reap_timeout_seconds,
                        )
                        break
                    _log_codex_ws(
                        "app_server.child_kill_retrying",
                        pid=child.pid,
                        child_elapsed_sec=_elapsed_seconds(started_at) if started_at is not None else None,
                        stderr_log=stderr_log_path,
                        reap_attempt=attempt,
                        reap_attempts_total=len(reap_intervals),
                        reap_wait_timeout_seconds=f"{wait_timeout:.2f}",
                    )
        finally:
            if stderr_handle is not None:
                stderr_handle.close()
            _unregister_managed_app_server(self._config.repo_root, managed_pid or child.pid)

    def _child_diagnostics(self) -> dict[str, Any]:
        child = self._child
        if child is None:
            return {}
        return {
            "pid": child.pid,
            "return_code": child.poll(),
            "child_elapsed_sec": _elapsed_seconds(self._child_started_at) if self._child_started_at is not None else None,
            "stderr_log": self._stderr_log_path,
        }

    def _start_managed_server(self) -> str:
        host = str(self._config.app_server_host or DEFAULT_CODEX_APP_SERVER_HOST).strip() or DEFAULT_CODEX_APP_SERVER_HOST
        port = int(self._config.app_server_port or 0)
        if port <= 0:
            port = get_available_port(host)
        listen_url = f"ws://{host}:{port}"
        env = os.environ.copy()
        started_at = time.monotonic()
        _log_codex_ws(
            "app_server.launching",
            listen_url=listen_url,
            repo_root=self._config.repo_root,
            bin_path=self._config.bin_path,
        )
        stderr_handle, stderr_log_path, stderr_log_error = _open_managed_stderr_log(self._config.repo_root, port)
        stderr_target = stderr_handle if stderr_handle is not None else subprocess.DEVNULL
        if stderr_log_error:
            _log_codex_ws(
                "app_server.stderr_log_unavailable",
                listen_url=listen_url,
                stderr_log=stderr_log_path,
                error=stderr_log_error,
            )
        try:
            child = subprocess.Popen(
                [self._config.bin_path, "app-server", "--listen", listen_url],
                cwd=str(self._config.repo_root),
                env=env,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=stderr_target,
                start_new_session=True,
            )
        except FileNotFoundError as exc:
            if stderr_handle is not None:
                stderr_handle.close()
            raise CodexAppServerError(f"Codex binary not found: {self._config.bin_path}") from exc
        self._child = child
        self._child_started_at = started_at
        self._managed_pid = child.pid
        self._stderr_handle = stderr_handle
        self._stderr_log_path = stderr_log_path
        self._url = listen_url
        _register_managed_app_server(
            self._config.repo_root,
            pid=child.pid,
            port=port,
            listen_url=listen_url,
            bin_path=self._config.bin_path,
            stderr_log_path=stderr_log_path,
        )
        ready_url = create_ready_url(listen_url)
        deadline = time.monotonic() + float(self._config.ready_timeout_seconds)
        while time.monotonic() < deadline:
            return_code = child.poll()
            if return_code is not None:
                _log_codex_ws(
                    "app_server.exited",
                    listen_url=listen_url,
                    pid=child.pid,
                    return_code=return_code,
                    elapsed_sec=_elapsed_seconds(started_at),
                    stderr_log=stderr_log_path,
                )
                self.close()
                raise CodexAppServerError(f"Codex app-server exited with code {return_code}.")
            if self._is_ready(ready_url):
                _log_codex_ws(
                    "app_server.ready",
                    listen_url=listen_url,
                    pid=child.pid,
                    elapsed_sec=_elapsed_seconds(started_at),
                    stderr_log=stderr_log_path,
                )
                return listen_url
            time.sleep(0.25)
        self.close()
        _log_codex_ws(
            "app_server.ready_timeout",
            listen_url=listen_url,
            pid=child.pid,
            elapsed_sec=_elapsed_seconds(started_at),
            ready_timeout_seconds=self._config.ready_timeout_seconds,
            stderr_log=stderr_log_path,
        )
        raise CodexAppServerError(f"Timed out waiting for Codex app-server at {listen_url}.")

    def _connect_websocket(self, target_url: str) -> None:
        try:
            from websockets.sync.client import connect
        except ImportError as exc:  # pragma: no cover - dependency error is environment-specific
            raise CodexAppServerError(
                "Missing Python dependency 'websockets'. Install project dependencies before using Codex websocket replies."
            ) from exc
        try:
            self._ws = connect(
                target_url,
                open_timeout=self._config.ready_timeout_seconds,
                close_timeout=1,
                ping_interval=20,
                ping_timeout=20,
                max_size=self._config.websocket_max_size_bytes,
            )
        except Exception as exc:
            self.close()
            _log_codex_ws(
                "app_server.websocket_connect_failed",
                target_url=target_url,
                error_type=type(exc).__name__,
                error=str(exc),
            )
            raise CodexAppServerError(f"Failed to connect to Codex app-server at {target_url}.") from exc
        self._url = target_url
        _log_codex_ws(
            "app_server.websocket_connected",
            target_url=target_url,
            max_size_bytes=self._config.websocket_max_size_bytes,
        )

    def _send_json(self, payload: dict[str, Any]) -> None:
        if self._ws is None:
            raise CodexAppServerError("Codex app-server connection is not open.")
        try:
            self._ws.send(json.dumps(payload))
        except Exception as exc:
            raise CodexAppServerError("Failed to send payload to Codex app-server.") from exc

    def _recv_json(self, *, deadline: float | None) -> dict[str, Any]:
        if self._ws is None:
            raise CodexAppServerError("Codex app-server connection is not open.")
        timeout = None
        if deadline is not None:
            timeout = max(deadline - time.monotonic(), 0.0)
        try:
            raw = self._ws.recv(timeout=timeout)
        except TimeoutError as exc:
            _log_codex_ws("app_server.recv_timeout", target_url=self._url, timeout_seconds=timeout)
            raise CodexAppServerError("Timed out waiting for Codex app-server response.") from exc
        except Exception as exc:
            _log_codex_ws(
                "app_server.connection_closed",
                target_url=self._url,
                error_type=type(exc).__name__,
                error=str(exc),
                **_websocket_close_diagnostics(exc),
                **self._child_diagnostics(),
            )
            raise CodexAppServerError("Codex app-server connection closed.") from exc
        if not isinstance(raw, str):
            raw = str(raw)
        try:
            message = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise CodexAppServerError("Codex app-server returned invalid JSON.") from exc
        if not isinstance(message, dict):
            raise CodexAppServerError("Codex app-server returned an unexpected payload.")
        return message

    def _is_ready(self, ready_url: str) -> bool:
        request = Request(ready_url, headers={"Accept": "application/json"}, method="GET")
        try:
            with urlopen(request, timeout=min(float(self._config.ready_timeout_seconds), 2.0)) as response:
                return 200 <= int(getattr(response, "status", 0) or 0) < 300
        except (OSError, URLError):
            return False


def format_rpc_error(error: Any) -> str:
    if not isinstance(error, dict):
        return "Codex app-server returned an unknown error."
    parts: list[str] = []
    message = str(error.get("message") or "").strip()
    if message:
        parts.append(message)
    data = error.get("data")
    if data is not None:
        try:
            parts.append(json.dumps(data, ensure_ascii=False, sort_keys=True))
        except TypeError:
            pass
    return "\n".join(parts) or "Codex app-server returned an error."


def format_turn_error(error: Any) -> str:
    if not isinstance(error, dict):
        return "Codex turn failed."
    parts = [str(error.get("message") or "").strip(), str(error.get("additionalDetails") or "").strip()]
    message = "\n".join(part for part in parts if part).strip()
    return message or "Codex turn failed."
