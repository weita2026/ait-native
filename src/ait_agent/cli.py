from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import time
from collections import deque
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import Any, Iterator

import typer
from ait_agent.telegram.runtime import load_simple_env_file, telegram_worker_seed_env_defaults

app = typer.Typer(help="Run optional ait external runtime workers.")
telegram_app = typer.Typer(help="Run and configure Telegram agent workers.")
line_app = typer.Typer(help="Run and configure LINE agent workers.")
discord_app = typer.Typer(help="Run and configure Discord agent workers.")
slack_app = typer.Typer(help="Run and configure Slack agent workers.")
telegram_supervisor_app = typer.Typer(help="Supervise configured Telegram workers.")
_STOP_SUCCESS_STATES = {"already_stopped", "stopped", "killed"}
_TELEGRAM_ENV_SEED_KEYS = (
    "AIT_REPO_ROOT",
    "AIT_TELEGRAM_ENV_PATH",
    "AIT_TELEGRAM_BOT_TOKEN",
    "BOT_TOKEN",
    "AIT_TELEGRAM_STATE_PATH",
    "AIT_TELEGRAM_TERMINATION_CONTEXT_PATH",
    "AIT_TELEGRAM_BOT_USERNAME",
    "BOT_USERNAME",
)


@app.callback()
def root() -> None:
    """Run optional ait external runtime workers."""


def _utc_now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def _repo_root() -> Path:
    return Path(os.environ.get("AIT_REPO_ROOT") or Path.cwd()).resolve()


def _config_path() -> Path:
    override = os.environ.get("AIT_AGENT_CONFIG_PATH")
    if override:
        return Path(override).expanduser().resolve()
    return _repo_root() / ".ait" / "agent-workers.json"


def _default_config() -> dict[str, Any]:
    return {"version": 1, "workers": {}}


def _load_config() -> dict[str, Any]:
    return _load_config_payload()[0]


def _load_config_payload() -> tuple[dict[str, Any], list[str]]:
    path = _config_path()
    if not path.exists():
        return _default_config(), []

    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        return _default_config(), [f"Invalid JSON in worker config at {path}: {exc}"]

    if not isinstance(payload, dict):
        return _default_config(), [f"Worker config root must be a JSON object at {path}"]

    issues: list[str] = []
    config: dict[str, Any] = {"version": _coerce_config_version(payload.get("version"), path, issues), "workers": {}}
    raw_workers = payload.get("workers")
    if not isinstance(raw_workers, dict):
        issues.append(f"Worker config at {path} is missing or has invalid `workers` map.")
        return config, issues

    workers: dict[str, Any] = {}
    for key, worker in raw_workers.items():
        normalized = _normalize_worker_entry(key, worker, path, issues)
        if normalized is None:
            continue
        workers[key] = normalized
    config["workers"] = workers
    return config, issues


def _coerce_config_version(value: object, path: Path, issues: list[str]) -> int:
    if isinstance(value, bool) or value is None:
        return 1
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value)
        except ValueError:
            issues.append(f"Worker config at {path} has invalid version value {value!r}; defaulting to 1.")
            return 1
    issues.append(f"Worker config at {path} has invalid version value {value!r}; defaulting to 1.")
    return 1


def _normalize_worker_entry(
    key: object,
    worker: object,
    path: Path,
    issues: list[str],
) -> dict[str, Any] | None:
    if not isinstance(key, str):
        issues.append(f"Worker config contains non-string key {type(key).__name__}; entry skipped.")
        return None
    if not isinstance(worker, dict):
        issues.append(f"Worker {key!r} must be an object; skipping invalid entry.")
        return None
    if "/" not in key:
        issues.append(f"Worker key {key!r} does not match expected `<kind>/<name>` form; skipping invalid entry.")
        return None
    kind, name = (part.strip() for part in key.split("/", 1))
    if not kind or not name:
        issues.append(f"Worker key {key!r} must include both non-empty kind and name; skipping invalid entry.")
        return None

    normalized: dict[str, Any] = dict(worker)
    normalized["kind"] = _coerce_optional_str(worker.get("kind") or kind, path, issues, f"{key}.kind")
    if normalized["kind"] != kind:
        issues.append(f"Worker {key!r} has kind {normalized['kind']!r}; normalized to {kind!r}.")
        normalized["kind"] = kind
    normalized["name"] = _coerce_optional_str(worker.get("name") or name, path, issues, f"{key}.name")

    normalized["token"] = _coerce_optional_str(worker.get("token"), path, issues, f"{key}.token")
    if normalized["token"] is None and "token" in worker:
        issues.append(f"Worker {key!r} has non-string token {worker['token']!r}; value removed to require reconfiguration.")

    normalized["secret"] = _coerce_optional_str(worker.get("secret"), path, issues, f"{key}.secret")
    if normalized["secret"] is None and "secret" in worker:
        issues.append(f"Worker {key!r} has non-string secret {worker['secret']!r}; value removed to require reconfiguration.")
    normalized["app_token"] = _coerce_optional_str(worker.get("app_token"), path, issues, f"{key}.app_token")
    if normalized["app_token"] is None and "app_token" in worker:
        issues.append(f"Worker {key!r} has non-string app_token {worker['app_token']!r}; value removed to require reconfiguration.")
    normalized["bot_token"] = _coerce_optional_str(worker.get("bot_token"), path, issues, f"{key}.bot_token")
    if normalized["bot_token"] is None and "bot_token" in worker:
        issues.append(f"Worker {key!r} has non-string bot_token {worker['bot_token']!r}; value removed to require reconfiguration.")

    normalized["username"] = _coerce_optional_str(worker.get("username"), path, issues, f"{key}.username")
    normalized["application_id"] = _coerce_optional_str(
        worker.get("application_id"), path, issues, f"{key}.application_id"
    )
    normalized["public_key"] = _coerce_optional_str(worker.get("public_key"), path, issues, f"{key}.public_key")

    for field in (
        "sync_state_path",
        "pid_file",
        "log_file",
        "env_path",
        "termination_context_path",
        "created_at",
        "updated_at",
    ):
        if field in normalized:
            normalized[field] = _coerce_optional_str(
                normalized.get(field),
                path,
                issues,
                f"{key}.{field}",
                allow_null=True,
            )

    return normalized


def _coerce_optional_str(
    value: object,
    path: Path,
    issues: list[str],
    field_name: str,
    *,
    allow_null: bool = True,
) -> str | None:
    if value is None:
        return None
    if isinstance(value, str):
        return value
    if not allow_null:
        issues.append(
            f"Worker config at {path} has invalid non-null value type for {field_name}: {type(value).__name__}. "
            "Expected string."
        )
    else:
        issues.append(
            f"Worker config at {path} has invalid value type for {field_name}: {type(value).__name__}. "
            "Expected string."
        )
    return None


def _config_diagnostics_payload(config: dict[str, Any], issues: list[str]) -> dict[str, Any]:
    payload: dict[str, Any] = {"config_version": config.get("version", 1), "config_valid": not issues}
    if issues:
        payload["config_issues"] = issues
    return payload


def _save_config(payload: dict[str, Any]) -> None:
    path = _config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    with NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as handle:
        json.dump(payload, handle, ensure_ascii=False, indent=2, sort_keys=True)
        handle.write("\n")
        tmp_path = Path(handle.name)
    tmp_path.replace(path)


def _worker_key(kind: str, name: str) -> str:
    normalized_name = name.strip()
    if not normalized_name:
        raise typer.BadParameter("Worker name must not be empty.")
    if "/" in normalized_name:
        raise typer.BadParameter("Worker name must not contain '/'.")
    return f"{kind}/{normalized_name}"


def _safe_file_label(name: str) -> str:
    label = "".join(ch if ch.isalnum() or ch in {"-", "_"} else "-" for ch in name.strip())
    return label.strip("-") or "worker"


def _runtime_dir() -> Path:
    return _repo_root() / ".ait" / "agent-runtime"


def _lifecycle_lock_path(kind: str) -> Path:
    return _runtime_dir() / f"{kind}-lifecycle.lock"


def _read_lifecycle_lock_pid(path: Path) -> int | None:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    try:
        pid = int(payload.get("pid") or 0)
    except (TypeError, ValueError):
        return None
    return pid if pid > 0 else None


@contextmanager
def _transport_lifecycle_lock(kind: str, action: str) -> Iterator[None]:
    path = _lifecycle_lock_path(kind)
    path.parent.mkdir(parents=True, exist_ok=True)
    while True:
        try:
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o644)
        except FileExistsError:
            pid = _read_lifecycle_lock_pid(path)
            if pid is not None and not _pid_is_alive(pid):
                path.unlink(missing_ok=True)
                continue
            raise typer.BadParameter(f"{kind.capitalize()} lifecycle lock is busy: {path}")
        break
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump({"action": action, "created_at": _utc_now_iso(), "pid": os.getpid()}, handle, sort_keys=True)
            handle.write("\n")
        yield
    finally:
        path.unlink(missing_ok=True)


@contextmanager
def _telegram_lifecycle_lock(action: str) -> Iterator[None]:
    with _transport_lifecycle_lock("telegram", action):
        yield


@contextmanager
def _line_lifecycle_lock(action: str) -> Iterator[None]:
    with _transport_lifecycle_lock("line", action):
        yield


@contextmanager
def _discord_lifecycle_lock(action: str) -> Iterator[None]:
    with _transport_lifecycle_lock("discord", action):
        yield


@contextmanager
def _slack_lifecycle_lock(action: str) -> Iterator[None]:
    with _transport_lifecycle_lock("slack", action):
        yield


def _runtime_path(value: str | None, default_name: str) -> Path:
    raw = Path(value).expanduser() if value else _runtime_dir() / default_name
    if raw.is_absolute():
        return raw
    return _repo_root() / raw


def _worker_paths(worker: dict[str, Any]) -> dict[str, Path]:
    kind = str(worker.get("kind") or "worker").strip() or "worker"
    label = _safe_file_label(str(worker.get("name") or "worker"))
    return {
        "sync_state_path": _runtime_path(worker.get("sync_state_path"), f"{kind}-{label}-sync.json"),
        "pid_file": _runtime_path(worker.get("pid_file"), f"{kind}-{label}.pid"),
        "log_file": _runtime_path(worker.get("log_file"), f"{kind}-{label}.log"),
        "env_path": _runtime_path(worker.get("env_path"), f"{kind}.env"),
        "termination_context_path": _runtime_path(
            worker.get("termination_context_path"),
            f"{kind}-{label}-termination.json",
        ),
    }


def _write_termination_context(path: Path, *, pid: int, reason: str, worker_name: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "pid": pid,
        "reason": reason,
        "worker_name": worker_name,
        "issued_at": _utc_now_iso(),
        "issued_by_pid": os.getpid(),
    }
    with NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as handle:
        json.dump(payload, handle, ensure_ascii=False, indent=2, sort_keys=True)
        handle.write("\n")
        tmp_path = Path(handle.name)
    tmp_path.replace(path)


def _clear_termination_context(path: Path) -> None:
    path.unlink(missing_ok=True)


def _get_worker(kind: str, name: str, config: dict[str, Any] | None = None) -> dict[str, Any]:
    key = _worker_key(kind, name)
    worker = (config or _load_config()).get("workers", {}).get(key)
    if not isinstance(worker, dict):
        raise typer.BadParameter(f"Unknown {kind} worker: {name}")
    return worker


def _inspect_pid_file(path: Path) -> dict[str, Any]:
    state: dict[str, Any] = {
        "pid_file_exists": False,
        "pid_file_readable": False,
        "pid_file_valid": False,
        "running": False,
        "state": "missing",
    }
    if not path.exists():
        return state

    state["pid_file_exists"] = True
    try:
        raw = path.read_text(encoding="utf-8").strip()
    except OSError:
        state["state"] = "unreadable"
        return state
    state["pid_file_readable"] = True

    if not raw:
        state["state"] = "empty"
        return state

    try:
        pid = int(raw)
    except ValueError:
        state["state"] = "invalid"
        return state

    if pid <= 0:
        state["state"] = "invalid"
        return state

    state["pid"] = pid
    state["pid_file_valid"] = True
    state["running"] = _pid_is_alive(pid)
    state["state"] = "running" if state["running"] else "stale"
    return state


def _read_pid(path: Path) -> int | None:
    info = _inspect_pid_file(path)
    pid = info.get("pid")
    if not info.get("pid_file_valid"):
        return None
    return int(pid) if isinstance(pid, int) else None


def _pid_status_text(pid: int) -> str | None:
    try:
        completed = subprocess.run(
            ["ps", "-o", "stat=", "-p", str(pid)],
            capture_output=True,
            text=True,
            timeout=1,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if completed.returncode != 0:
        return ""
    return str(completed.stdout or "").strip()


def _pid_is_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    status = _pid_status_text(pid)
    if status is None:
        return True
    if not status:
        return False
    return "Z" not in status.upper()


def _stop_pid(pid: int, *, timeout_seconds: float = 10.0, kill_grace_seconds: float = 2.0) -> str:
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        return "already_stopped"
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if not _pid_is_alive(pid):
            return "stopped"
        time.sleep(0.1)
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        return "stopped"
    kill_deadline = time.monotonic() + max(kill_grace_seconds, 0.0)
    while time.monotonic() < kill_deadline:
        if not _pid_is_alive(pid):
            return "killed"
        time.sleep(0.1)
    return "still_running"


def _redact_token(token: str | None) -> dict[str, Any]:
    raw = str(token or "")
    if not raw:
        return {"token_set": False, "token_preview": None}
    if len(raw) <= 4:
        preview = "*" * len(raw)
    else:
        preview = f"{'*' * max(len(raw) - 4, 4)}{raw[-4:]}"
    return {"token_set": True, "token_preview": preview}


def _redact_secret(secret: str | None) -> dict[str, Any]:
    raw = str(secret or "")
    if not raw:
        return {"secret_set": False, "secret_preview": None}
    if len(raw) <= 4:
        preview = "*" * len(raw)
    else:
        preview = f"{'*' * max(len(raw) - 4, 4)}{raw[-4:]}"
    return {"secret_set": True, "secret_preview": preview}


def _redact_named_token(name: str, token: str | None) -> dict[str, Any]:
    raw = str(token or "")
    if not raw:
        return {f"{name}_set": False, f"{name}_preview": None}
    if len(raw) <= 4:
        preview = "*" * len(raw)
    else:
        preview = f"{'*' * max(len(raw) - 4, 4)}{raw[-4:]}"
    return {f"{name}_set": True, f"{name}_preview": preview}


def _public_worker(worker: dict[str, Any]) -> dict[str, Any]:
    public = {key: value for key, value in worker.items() if key not in {"token", "secret", "app_token", "bot_token"}}
    public.update(_redact_token(worker.get("token")))
    public.update(_redact_secret(worker.get("secret")))
    public.update(_redact_named_token("app_token", worker.get("app_token")))
    public.update(_redact_named_token("bot_token", worker.get("bot_token")))
    return public


def _worker_status(
    worker: dict[str, Any],
    *,
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> dict[str, Any]:
    if config is None:
        config = _load_config()
    paths = _worker_paths(worker)
    pid_info = _inspect_pid_file(paths["pid_file"])
    pid = pid_info.get("pid")
    if not isinstance(pid, int):
        pid = None
    running = bool(pid and pid_info.get("running"))
    payload = _public_worker(worker)
    if str(worker.get("kind") or "").strip() == "discord" and not payload.get("bot_token_set"):
        env_bot_token = _runtime_env_value(paths["env_path"], "AIT_DISCORD_BOT_TOKEN", "DISCORD_BOT_TOKEN")
        if env_bot_token:
            payload.update(_redact_named_token("bot_token", env_bot_token))
    payload.update(_config_diagnostics_payload(config, config_issues or []))
    payload.update(
        {
            "running": running,
            "pid": pid,
            "sync_state_path": str(paths["sync_state_path"]),
            "pid_file": str(paths["pid_file"]),
            "log_file": str(paths["log_file"]),
            "env_path": str(paths["env_path"]),
            "termination_context_path": str(paths["termination_context_path"]),
            "health": {
                "pid_file_exists": pid_info["pid_file_exists"],
                "pid_file_readable": pid_info["pid_file_readable"],
                "pid_file_valid": pid_info["pid_file_valid"],
                "pid_file_state": pid_info["state"],
                "log_exists": paths["log_file"].exists(),
                "log_size_bytes": paths["log_file"].stat().st_size if paths["log_file"].exists() else 0,
                "sync_state_exists": paths["sync_state_path"].exists(),
                "env_exists": paths["env_path"].exists(),
                "termination_context_exists": paths["termination_context_path"].exists(),
            },
        }
    )
    return payload


def _runtime_env_value(path: Path, *names: str) -> str | None:
    values = load_simple_env_file(path)
    for name in names:
        raw = str(values.get(name) or "").strip()
        if raw:
            return raw
    return None


def _telegram_worker_env(worker: dict[str, Any], paths: dict[str, Path]) -> dict[str, str]:
    token = str(worker.get("token") or "").strip()
    if not token:
        raise typer.BadParameter(f"Telegram worker {worker.get('name') or ''} does not have a token.")
    env = dict(os.environ)
    env["AIT_REPO_ROOT"] = str(_repo_root())
    env["AIT_TELEGRAM_ENV_PATH"] = str(paths["env_path"])
    env["AIT_TELEGRAM_BOT_TOKEN"] = token
    env["BOT_TOKEN"] = token
    username = str(worker.get("username") or "").strip()
    if username:
        env["AIT_TELEGRAM_BOT_USERNAME"] = username
        env["BOT_USERNAME"] = username
    env["AIT_TELEGRAM_STATE_PATH"] = str(paths["sync_state_path"])
    env["AIT_TELEGRAM_TERMINATION_CONTEXT_PATH"] = str(paths["termination_context_path"])
    return env


def _seed_telegram_worker_env_file(path: Path, env: dict[str, str]) -> bool:
    if path.exists():
        return False
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "# Seeded by `ait-agent telegram start` on first worker launch.\n",
        "# Later edits are preserved; re-running start does not overwrite this file.\n",
    ]
    for key in _TELEGRAM_ENV_SEED_KEYS:
        value = env.get(key)
        if not value:
            continue
        if "\n" in value or "\r" in value:
            raise typer.BadParameter(f"Telegram worker env value for {key} contains a newline and cannot be seeded.")
        lines.append(f"{key}={value}\n")
    for key, value in telegram_worker_seed_env_defaults().items():
        if "\n" in value or "\r" in value:
            raise typer.BadParameter(f"Telegram worker seeded default for {key} contains a newline and cannot be seeded.")
        lines.append(f"{key}={value}\n")
    path.write_text("".join(lines), encoding="utf-8")
    return True


def _line_worker_env(worker: dict[str, Any], paths: dict[str, Path]) -> dict[str, str]:
    token = str(worker.get("token") or "").strip()
    if not token:
        raise typer.BadParameter(f"LINE worker {worker.get('name') or ''} does not have a token.")
    secret = str(worker.get("secret") or "").strip()
    if not secret:
        raise typer.BadParameter(f"LINE worker {worker.get('name') or ''} does not have a secret.")
    env = dict(os.environ)
    env["AIT_REPO_ROOT"] = str(_repo_root())
    env["AIT_LINE_ENV_PATH"] = str(paths["env_path"])
    env["AIT_LINE_CHANNEL_ACCESS_TOKEN"] = token
    env["LINE_CHANNEL_ACCESS_TOKEN"] = token
    env["AIT_LINE_CHANNEL_SECRET"] = secret
    env["LINE_CHANNEL_SECRET"] = secret
    env["AIT_LINE_STATE_PATH"] = str(paths["sync_state_path"])
    env["AIT_LINE_TERMINATION_CONTEXT_PATH"] = str(paths["termination_context_path"])
    return env


def _discord_worker_env(worker: dict[str, Any], paths: dict[str, Path]) -> dict[str, str]:
    application_id = str(worker.get("application_id") or "").strip()
    if not application_id:
        raise typer.BadParameter(f"Discord worker {worker.get('name') or ''} does not have an application id.")
    bot_token = str(worker.get("bot_token") or "").strip()
    if not bot_token:
        bot_token = _runtime_env_value(paths["env_path"], "AIT_DISCORD_BOT_TOKEN", "DISCORD_BOT_TOKEN") or ""
    if not bot_token:
        raise typer.BadParameter(
            f"Discord worker {worker.get('name') or ''} does not have a bot token in config or {paths['env_path']}."
        )
    env = dict(os.environ)
    env["AIT_REPO_ROOT"] = str(_repo_root())
    env["AIT_DISCORD_ENV_PATH"] = str(paths["env_path"])
    env["AIT_DISCORD_APPLICATION_ID"] = application_id
    env["DISCORD_APPLICATION_ID"] = application_id
    env["AIT_DISCORD_BOT_TOKEN"] = bot_token
    env["DISCORD_BOT_TOKEN"] = bot_token
    env["AIT_DISCORD_STATE_PATH"] = str(paths["sync_state_path"])
    env["AIT_DISCORD_TERMINATION_CONTEXT_PATH"] = str(paths["termination_context_path"])
    return env


def _slack_worker_env(worker: dict[str, Any], paths: dict[str, Path]) -> dict[str, str]:
    app_token = str(worker.get("app_token") or "").strip()
    if not app_token:
        raise typer.BadParameter(f"Slack worker {worker.get('name') or ''} does not have an app token.")
    env = dict(os.environ)
    env["AIT_REPO_ROOT"] = str(_repo_root())
    env["AIT_SLACK_ENV_PATH"] = str(paths["env_path"])
    env["AIT_SLACK_APP_TOKEN"] = app_token
    env["SLACK_APP_TOKEN"] = app_token
    env["AIT_SLACK_STATE_PATH"] = str(paths["sync_state_path"])
    env["AIT_SLACK_TERMINATION_CONTEXT_PATH"] = str(paths["termination_context_path"])
    return env


def _start_telegram_worker(
    worker: dict[str, Any],
    *,
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> dict[str, Any]:
    paths = _worker_paths(worker)
    paths["pid_file"].parent.mkdir(parents=True, exist_ok=True)
    paths["log_file"].parent.mkdir(parents=True, exist_ok=True)
    paths["sync_state_path"].parent.mkdir(parents=True, exist_ok=True)
    _clear_termination_context(paths["termination_context_path"])
    env = _telegram_worker_env(worker, paths)
    _seed_telegram_worker_env_file(paths["env_path"], env)
    command = [sys.executable, "-m", "ait_agent.telegram.app"]
    with paths["log_file"].open("ab") as log_handle:
        process = subprocess.Popen(
            command,
            cwd=str(_repo_root()),
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    paths["pid_file"].write_text(f"{process.pid}\n", encoding="utf-8")
    payload = _worker_status(worker, config=config, config_issues=config_issues)
    payload.update({"started": True, "command": command})
    return payload


def _stop_telegram_worker(
    worker: dict[str, Any],
    *,
    reason: str = "telegram_stop",
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> tuple[dict[str, Any], str]:
    paths = _worker_paths(worker)
    pid = _read_pid(paths["pid_file"])
    if not pid or not _pid_is_alive(pid):
        paths["pid_file"].unlink(missing_ok=True)
        _clear_termination_context(paths["termination_context_path"])
        payload = _worker_status(worker, config=config, config_issues=config_issues)
        payload.update({"stopped": False, "stop_state": "not_running"})
        return payload, "not_running"
    _write_termination_context(
        paths["termination_context_path"],
        pid=pid,
        reason=reason,
        worker_name=str(worker.get("name") or "worker"),
    )
    stop_state = _stop_pid(pid)
    if stop_state in _STOP_SUCCESS_STATES:
        paths["pid_file"].unlink(missing_ok=True)
    payload = _worker_status(worker, config=config, config_issues=config_issues)
    payload.update({"stopped": stop_state in _STOP_SUCCESS_STATES, "stop_state": stop_state})
    return payload, stop_state


def _start_line_worker(
    worker: dict[str, Any],
    *,
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> dict[str, Any]:
    paths = _worker_paths(worker)
    paths["pid_file"].parent.mkdir(parents=True, exist_ok=True)
    paths["log_file"].parent.mkdir(parents=True, exist_ok=True)
    paths["sync_state_path"].parent.mkdir(parents=True, exist_ok=True)
    _clear_termination_context(paths["termination_context_path"])
    env = _line_worker_env(worker, paths)
    command = [sys.executable, "-m", "ait_agent.line.app"]
    with paths["log_file"].open("ab") as log_handle:
        process = subprocess.Popen(
            command,
            cwd=str(_repo_root()),
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    paths["pid_file"].write_text(f"{process.pid}\n", encoding="utf-8")
    payload = _worker_status(worker, config=config, config_issues=config_issues)
    payload.update({"started": True, "command": command})
    return payload


def _stop_line_worker(
    worker: dict[str, Any],
    *,
    reason: str = "line_stop",
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> tuple[dict[str, Any], str]:
    paths = _worker_paths(worker)
    pid = _read_pid(paths["pid_file"])
    if not pid or not _pid_is_alive(pid):
        paths["pid_file"].unlink(missing_ok=True)
        _clear_termination_context(paths["termination_context_path"])
        payload = _worker_status(worker, config=config, config_issues=config_issues)
        payload.update({"stopped": False, "stop_state": "not_running"})
        return payload, "not_running"
    _write_termination_context(
        paths["termination_context_path"],
        pid=pid,
        reason=reason,
        worker_name=str(worker.get("name") or "worker"),
    )
    stop_state = _stop_pid(pid)
    if stop_state in _STOP_SUCCESS_STATES:
        paths["pid_file"].unlink(missing_ok=True)
    payload = _worker_status(worker, config=config, config_issues=config_issues)
    payload.update({"stopped": stop_state in _STOP_SUCCESS_STATES, "stop_state": stop_state})
    return payload, stop_state


def _start_discord_worker(
    worker: dict[str, Any],
    *,
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> dict[str, Any]:
    paths = _worker_paths(worker)
    paths["pid_file"].parent.mkdir(parents=True, exist_ok=True)
    paths["log_file"].parent.mkdir(parents=True, exist_ok=True)
    paths["sync_state_path"].parent.mkdir(parents=True, exist_ok=True)
    _clear_termination_context(paths["termination_context_path"])
    env = _discord_worker_env(worker, paths)
    command = [sys.executable, "-m", "ait_agent.discord.app"]
    with paths["log_file"].open("ab") as log_handle:
        process = subprocess.Popen(
            command,
            cwd=str(_repo_root()),
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    paths["pid_file"].write_text(f"{process.pid}\n", encoding="utf-8")
    payload = _worker_status(worker, config=config, config_issues=config_issues)
    payload.update({"started": True, "command": command})
    return payload


def _stop_discord_worker(
    worker: dict[str, Any],
    *,
    reason: str = "discord_stop",
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> tuple[dict[str, Any], str]:
    paths = _worker_paths(worker)
    pid = _read_pid(paths["pid_file"])
    if not pid or not _pid_is_alive(pid):
        paths["pid_file"].unlink(missing_ok=True)
        _clear_termination_context(paths["termination_context_path"])
        payload = _worker_status(worker, config=config, config_issues=config_issues)
        payload.update({"stopped": False, "stop_state": "not_running"})
        return payload, "not_running"
    _write_termination_context(
        paths["termination_context_path"],
        pid=pid,
        reason=reason,
        worker_name=str(worker.get("name") or "worker"),
    )
    stop_state = _stop_pid(pid)
    if stop_state in _STOP_SUCCESS_STATES:
        paths["pid_file"].unlink(missing_ok=True)
    payload = _worker_status(worker, config=config, config_issues=config_issues)
    payload.update({"stopped": stop_state in _STOP_SUCCESS_STATES, "stop_state": stop_state})
    return payload, stop_state


def _start_slack_worker(
    worker: dict[str, Any],
    *,
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> dict[str, Any]:
    paths = _worker_paths(worker)
    paths["pid_file"].parent.mkdir(parents=True, exist_ok=True)
    paths["log_file"].parent.mkdir(parents=True, exist_ok=True)
    paths["sync_state_path"].parent.mkdir(parents=True, exist_ok=True)
    _clear_termination_context(paths["termination_context_path"])
    env = _slack_worker_env(worker, paths)
    command = [sys.executable, "-m", "ait_agent.slack.app"]
    with paths["log_file"].open("ab") as log_handle:
        process = subprocess.Popen(
            command,
            cwd=str(_repo_root()),
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    paths["pid_file"].write_text(f"{process.pid}\n", encoding="utf-8")
    payload = _worker_status(worker, config=config, config_issues=config_issues)
    payload.update({"started": True, "command": command})
    return payload


def _stop_slack_worker(
    worker: dict[str, Any],
    *,
    reason: str = "slack_stop",
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> tuple[dict[str, Any], str]:
    paths = _worker_paths(worker)
    pid = _read_pid(paths["pid_file"])
    if not pid or not _pid_is_alive(pid):
        paths["pid_file"].unlink(missing_ok=True)
        _clear_termination_context(paths["termination_context_path"])
        payload = _worker_status(worker, config=config, config_issues=config_issues)
        payload.update({"stopped": False, "stop_state": "not_running"})
        return payload, "not_running"
    _write_termination_context(
        paths["termination_context_path"],
        pid=pid,
        reason=reason,
        worker_name=str(worker.get("name") or "worker"),
    )
    stop_state = _stop_pid(pid)
    if stop_state in _STOP_SUCCESS_STATES:
        paths["pid_file"].unlink(missing_ok=True)
    payload = _worker_status(worker, config=config, config_issues=config_issues)
    payload.update({"stopped": stop_state in _STOP_SUCCESS_STATES, "stop_state": stop_state})
    return payload, stop_state


def _iter_telegram_workers(config: dict[str, Any] | None = None) -> list[dict[str, Any]]:
    return [
        worker
        for key, worker in sorted((config or _load_config()).get("workers", {}).items())
        if key.startswith("telegram/") and isinstance(worker, dict)
    ]


def _iter_line_workers(config: dict[str, Any] | None = None) -> list[dict[str, Any]]:
    return [
        worker
        for key, worker in sorted((config or _load_config()).get("workers", {}).items())
        if key.startswith("line/") and isinstance(worker, dict)
    ]


def _iter_discord_workers(config: dict[str, Any] | None = None) -> list[dict[str, Any]]:
    return [
        worker
        for key, worker in sorted((config or _load_config()).get("workers", {}).items())
        if key.startswith("discord/") and isinstance(worker, dict)
    ]


def _iter_slack_workers(config: dict[str, Any] | None = None) -> list[dict[str, Any]]:
    return [
        worker
        for key, worker in sorted((config or _load_config()).get("workers", {}).items())
        if key.startswith("slack/") and isinstance(worker, dict)
    ]


def _start_stopped_telegram_workers(
    *, config: dict[str, Any] | None = None, config_issues: list[str] | None = None
) -> list[dict[str, Any]]:
    if config is None:
        config = _load_config()
    workers: list[dict[str, Any]] = []
    for worker in _iter_telegram_workers(config=config):
        status = _worker_status(worker, config=config, config_issues=config_issues)
        if status["running"]:
            status["started"] = False
            status["start_state"] = "already_running"
            workers.append(status)
            continue
        payload = _start_telegram_worker(worker, config=config, config_issues=config_issues)
        payload["start_state"] = "started"
        workers.append(payload)
    return workers


def _telegram_supervisor_run_payload(
    action: str,
    *,
    interval_seconds: float | None = None,
    cycle: int | None = None,
) -> dict[str, Any]:
    config, config_issues = _load_config_payload()
    with _telegram_lifecycle_lock(f"telegram/supervisor/{action}"):
        workers = _start_stopped_telegram_workers(config=config, config_issues=config_issues)
    running_count = sum(1 for worker in workers if worker.get("running"))
    started_count = sum(1 for worker in workers if worker.get("start_state") == "started")
    payload: dict[str, Any] = {
        "kind": "telegram-supervisor",
        "action": action,
        **_config_diagnostics_payload(config, config_issues),
        "worker_count": len(workers),
        "running_count": running_count,
        "started_count": started_count,
        "workers": workers,
    }
    if cycle is not None:
        payload["cycle"] = cycle
    if interval_seconds is not None:
        payload["interval_seconds"] = interval_seconds
    return payload


def _telegram_supervisor_run(
    *, interval_seconds: float, once: bool, json_output: bool, max_cycles: int | None = None
) -> None:
    cycle = 0
    while True:
        cycle += 1
        payload = _telegram_supervisor_run_payload(
            "run",
            interval_seconds=interval_seconds,
            cycle=cycle,
        )
        _emit(payload, json_output=json_output)
        if once or (max_cycles is not None and cycle >= max_cycles):
            return
        time.sleep(interval_seconds)


def _restart_telegram_worker(
    worker: dict[str, Any],
    *,
    reason: str = "telegram_restart",
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> dict[str, Any]:
    status = _worker_status(worker, config=config, config_issues=config_issues)
    if status["running"]:
        status, stop_state = _stop_telegram_worker(
            worker,
            reason=reason,
            config=config,
            config_issues=config_issues,
        )
        if stop_state not in _STOP_SUCCESS_STATES:
            status.update({"started": False, "restarted": False, "restart_blocked": True})
            return status
    else:
        status.update({"stopped": False, "stop_state": "not_running"})
    payload = _start_telegram_worker(worker, config=config, config_issues=config_issues)
    payload.update(
        {
            "stopped": status["stopped"],
            "stop_state": status["stop_state"],
            "restarted": status["stop_state"] in _STOP_SUCCESS_STATES,
            "restart_blocked": False,
        }
    )
    return payload


def _restart_line_worker(
    worker: dict[str, Any],
    *,
    reason: str = "line_restart",
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> dict[str, Any]:
    status = _worker_status(worker, config=config, config_issues=config_issues)
    if status["running"]:
        status, stop_state = _stop_line_worker(
            worker,
            reason=reason,
            config=config,
            config_issues=config_issues,
        )
        if stop_state not in _STOP_SUCCESS_STATES:
            status.update({"started": False, "restarted": False, "restart_blocked": True})
            return status
    else:
        status.update({"stopped": False, "stop_state": "not_running"})
    payload = _start_line_worker(worker, config=config, config_issues=config_issues)
    payload.update(
        {
            "stopped": status["stopped"],
            "stop_state": status["stop_state"],
            "restarted": status["stop_state"] in _STOP_SUCCESS_STATES,
            "restart_blocked": False,
        }
    )
    return payload


def _restart_discord_worker(
    worker: dict[str, Any],
    *,
    reason: str = "discord_restart",
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> dict[str, Any]:
    status = _worker_status(worker, config=config, config_issues=config_issues)
    if status["running"]:
        status, stop_state = _stop_discord_worker(
            worker,
            reason=reason,
            config=config,
            config_issues=config_issues,
        )
        if stop_state not in _STOP_SUCCESS_STATES:
            status.update({"started": False, "restarted": False, "restart_blocked": True})
            return status
    else:
        status.update({"stopped": False, "stop_state": "not_running"})
    payload = _start_discord_worker(worker, config=config, config_issues=config_issues)
    payload.update(
        {
            "stopped": status["stopped"],
            "stop_state": status["stop_state"],
            "restarted": status["stop_state"] in _STOP_SUCCESS_STATES,
            "restart_blocked": False,
        }
    )
    return payload


def _restart_slack_worker(
    worker: dict[str, Any],
    *,
    reason: str = "slack_restart",
    config: dict[str, Any] | None = None,
    config_issues: list[str] | None = None,
) -> dict[str, Any]:
    status = _worker_status(worker, config=config, config_issues=config_issues)
    if status["running"]:
        status, stop_state = _stop_slack_worker(
            worker,
            reason=reason,
            config=config,
            config_issues=config_issues,
        )
        if stop_state not in _STOP_SUCCESS_STATES:
            status.update({"started": False, "restarted": False, "restart_blocked": True})
            return status
    else:
        status.update({"stopped": False, "stop_state": "not_running"})
    payload = _start_slack_worker(worker, config=config, config_issues=config_issues)
    payload.update(
        {
            "stopped": status["stopped"],
            "stop_state": status["stop_state"],
            "restarted": status["stop_state"] in _STOP_SUCCESS_STATES,
            "restart_blocked": False,
        }
    )
    return payload


def _read_log_tail(log_path: Path, line_count: int) -> list[str]:
    if not log_path.exists():
        return []
    tail: deque[str] = deque(maxlen=line_count)
    with log_path.open("r", encoding="utf-8", errors="replace") as handle:
        for line in handle:
            tail.append(line.rstrip("\n"))
    return list(tail)


def _emit(payload: Any, *, json_output: bool) -> None:
    if json_output:
        typer.echo(json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True))
        return
    if isinstance(payload, list):
        for item in payload:
            typer.echo(f"{item['kind']}/{item['name']}\t{item.get('username') or '-'}\t{item.get('token_preview') or '-'}")
        return
    if isinstance(payload, dict):
        typer.echo(json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True))
        return
    typer.echo(str(payload))


@telegram_app.callback(invoke_without_command=True)
def telegram(
    ctx: typer.Context,
    mode: str = typer.Option("poll", "--mode", "-m", help="Telegram worker mode: poll (default) or webhook."),
) -> None:
    """Run the Telegram runtime worker."""
    if ctx.invoked_subcommand is not None:
        return
    normalized_mode = mode.strip().lower()
    if normalized_mode == "poll":
        from ait_agent.telegram.app import main as telegram_main

        telegram_main()
        return
    if normalized_mode == "webhook":
        from ait_agent.telegram.app import webhook_main

        webhook_main()
        return
    raise typer.BadParameter("mode must be either 'poll' or 'webhook'")


@telegram_app.command("add")
def telegram_add(
    name: str,
    token: str = typer.Option(..., "--token", help="Telegram bot token for this named worker."),
    username: str | None = typer.Option(None, "--username", help="Telegram bot username for operator visibility."),
    sync_state_path: str | None = typer.Option(None, "--sync-state-path", help="Per-worker Telegram sync-state path."),
    pid_file: str | None = typer.Option(None, "--pid-file", help="Per-worker PID file path."),
    log_file: str | None = typer.Option(None, "--log-file", help="Per-worker log file path."),
    env_path: str | None = typer.Option(None, "--env-path", help="Per-worker Telegram env file path."),
    json_output: bool = typer.Option(False, "--json"),
) -> None:
    """Add or update a named Telegram worker config."""
    key = _worker_key("telegram", name)
    config = _load_config()
    workers = dict(config.get("workers") or {})
    now = _utc_now_iso()
    current = dict(workers.get(key) or {})
    worker = {
        "kind": "telegram",
        "name": name.strip(),
        "token": token,
        "username": username,
        "sync_state_path": sync_state_path,
        "pid_file": pid_file,
        "log_file": log_file,
        "env_path": env_path,
        "created_at": current.get("created_at") or now,
        "updated_at": now,
    }
    workers[key] = worker
    config["workers"] = workers
    _save_config(config)
    _emit(_public_worker(worker), json_output=json_output)


@telegram_app.command("list")
def telegram_list(json_output: bool = typer.Option(False, "--json")) -> None:
    """List named Telegram worker configs."""
    config = _load_config()
    workers = [
        _public_worker(worker)
        for key, worker in sorted((config.get("workers") or {}).items())
        if key.startswith("telegram/") and isinstance(worker, dict)
    ]
    _emit(workers, json_output=json_output)


@telegram_app.command("status")
def telegram_status(name: str | None = typer.Argument(None), json_output: bool = typer.Option(False, "--json")) -> None:
    """Show named Telegram worker runtime status."""
    config, config_issues = _load_config_payload()
    if name is not None:
        worker = _get_worker("telegram", name, config=config)
        _emit(_worker_status(worker, config=config, config_issues=config_issues), json_output=json_output)
        return
    workers = [
        _worker_status(worker, config=config, config_issues=config_issues)
        for key, worker in sorted((config.get("workers") or {}).items())
        if key.startswith("telegram/") and isinstance(worker, dict)
    ]
    _emit(workers, json_output=json_output)


@telegram_app.command("start")
def telegram_start(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Start a named Telegram worker in daemon mode."""
    with _telegram_lifecycle_lock(f"telegram/start/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("telegram", name, config=config)
        status = _worker_status(worker, config=config, config_issues=config_issues)
        if status["running"]:
            status["started"] = False
            _emit(status, json_output=json_output)
            return
        payload = _start_telegram_worker(worker, config=config, config_issues=config_issues)
    _emit(payload, json_output=json_output)


@telegram_app.command("stop")
def telegram_stop(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Stop a named Telegram worker."""
    with _telegram_lifecycle_lock(f"telegram/stop/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("telegram", name, config=config)
        payload, _ = _stop_telegram_worker(
            worker,
            reason="cli_telegram_stop",
            config=config,
            config_issues=config_issues,
        )
    _emit(payload, json_output=json_output)


@telegram_app.command("restart")
def telegram_restart(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Stop and restart a named Telegram worker in daemon mode."""
    with _telegram_lifecycle_lock(f"telegram/restart/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("telegram", name, config=config)
        payload = _restart_telegram_worker(
            worker,
            reason="cli_telegram_restart",
            config=config,
            config_issues=config_issues,
        )
    _emit(payload, json_output=json_output)


@telegram_supervisor_app.command("status")
def telegram_supervisor_status(json_output: bool = typer.Option(False, "--json")) -> None:
    """Show runtime status for all Telegram workers."""
    config, config_issues = _load_config_payload()
    workers: list[dict[str, Any]] = []
    for worker in _iter_telegram_workers(config=config):
        status = _worker_status(worker, config=config, config_issues=config_issues)
        workers.append(status)
    running_count = sum(1 for worker in workers if worker.get("running"))
    _emit(
        {
            "kind": "telegram-supervisor",
            "action": "status",
            **_config_diagnostics_payload(config, config_issues),
            "worker_count": len(workers),
            "running_count": running_count,
            "workers": workers,
        },
        json_output=json_output,
    )


@telegram_supervisor_app.command("start")
def telegram_supervisor_start(json_output: bool = typer.Option(False, "--json")) -> None:
    """Start all configured Telegram workers."""
    _emit(_telegram_supervisor_run_payload("start"), json_output=json_output)


@telegram_supervisor_app.command("run")
def telegram_supervisor_run(
    interval_seconds: float = typer.Option(30.0, "--interval-seconds", min=1.0),
    once: bool = typer.Option(False, "--once", help="Run once and exit."),
    json_output: bool = typer.Option(False, "--json"),
) -> None:
    """Continuously start stopped Telegram workers at an interval."""
    _telegram_supervisor_run(
        interval_seconds=interval_seconds,
        once=once,
        json_output=json_output,
    )


@telegram_supervisor_app.command("stop")
def telegram_supervisor_stop(json_output: bool = typer.Option(False, "--json")) -> None:
    """Stop all running Telegram workers."""
    workers: list[dict[str, Any]] = []
    with _telegram_lifecycle_lock("telegram/supervisor/stop"):
        config, config_issues = _load_config_payload()
        for worker in _iter_telegram_workers(config=config):
            payload, _ = _stop_telegram_worker(
                worker,
                reason="supervisor_telegram_stop",
                config=config,
                config_issues=config_issues,
            )
            workers.append(payload)
    running_count = sum(1 for worker in workers if worker.get("running"))
    _emit(
        {
            "kind": "telegram-supervisor",
            "action": "stop",
            **_config_diagnostics_payload(config, config_issues),
            "worker_count": len(workers),
            "running_count": running_count,
            "workers": workers,
        },
        json_output=json_output,
    )


@telegram_supervisor_app.command("restart")
def telegram_supervisor_restart(json_output: bool = typer.Option(False, "--json")) -> None:
    """Restart all Telegram workers."""
    workers: list[dict[str, Any]] = []
    with _telegram_lifecycle_lock("telegram/supervisor/restart"):
        config, config_issues = _load_config_payload()
        for worker in _iter_telegram_workers(config=config):
            workers.append(
                _restart_telegram_worker(
                    worker,
                    reason="supervisor_telegram_restart",
                    config=config,
                    config_issues=config_issues,
                )
            )
    running_count = sum(1 for worker in workers if worker.get("running"))
    _emit(
        {
            "kind": "telegram-supervisor",
            "action": "restart",
            **_config_diagnostics_payload(config, config_issues),
            "worker_count": len(workers),
            "running_count": running_count,
            "workers": workers,
        },
        json_output=json_output,
    )


@telegram_app.command("logs")
def telegram_logs(
    name: str,
    lines: int = typer.Option(100, "--lines", min=1, max=10000),
    json_output: bool = typer.Option(False, "--json"),
) -> None:
    """Read the last N lines from a named Telegram worker's log."""
    config, config_issues = _load_config_payload()
    worker = _get_worker("telegram", name, config=config)
    paths = _worker_paths(worker)
    log_lines = _read_log_tail(paths["log_file"], lines)
    status = _worker_status(worker, config=config, config_issues=config_issues)
    status.update({"lines": log_lines, "log_exists": paths["log_file"].exists(), "lines_requested": lines})
    if json_output:
        _emit(status, json_output=True)
        return
    if not log_lines:
        typer.echo(f"No log lines available for {name}.")
        return
    for line in log_lines:
        typer.echo(line)


@telegram_app.command("remove")
def telegram_remove(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Remove a named Telegram worker config."""
    key = _worker_key("telegram", name)
    config = _load_config()
    workers = dict(config.get("workers") or {})
    removed = workers.pop(key, None)
    config["workers"] = workers
    _save_config(config)
    payload = {"removed": removed is not None, "kind": "telegram", "name": name.strip()}
    _emit(payload, json_output=json_output)


@line_app.callback(invoke_without_command=True)
def line(
    ctx: typer.Context,
    mode: str = typer.Option("serve", "--mode", "-m", help="LINE worker mode: serve (default) or webhook."),
) -> None:
    """Run the LINE runtime worker."""
    if ctx.invoked_subcommand is not None:
        return
    normalized_mode = mode.strip().lower()
    if normalized_mode == "serve":
        from ait_agent.line.app import main as line_main

        line_main()
        return
    if normalized_mode == "webhook":
        from ait_agent.line.app import webhook_main

        webhook_main()
        return
    raise typer.BadParameter("mode must be either 'serve' or 'webhook'")


@line_app.command("add")
def line_add(
    name: str,
    token: str = typer.Option(..., "--token", help="LINE channel access token for this named worker."),
    secret: str = typer.Option(..., "--secret", help="LINE channel secret for this named worker."),
    sync_state_path: str | None = typer.Option(None, "--sync-state-path", help="Per-worker LINE sync-state path."),
    pid_file: str | None = typer.Option(None, "--pid-file", help="Per-worker PID file path."),
    log_file: str | None = typer.Option(None, "--log-file", help="Per-worker log file path."),
    env_path: str | None = typer.Option(None, "--env-path", help="Per-worker LINE env file path."),
    json_output: bool = typer.Option(False, "--json"),
) -> None:
    """Add or update a named LINE worker config."""
    key = _worker_key("line", name)
    config = _load_config()
    workers = dict(config.get("workers") or {})
    now = _utc_now_iso()
    current = dict(workers.get(key) or {})
    worker = {
        "kind": "line",
        "name": name.strip(),
        "token": token,
        "secret": secret,
        "sync_state_path": sync_state_path,
        "pid_file": pid_file,
        "log_file": log_file,
        "env_path": env_path,
        "created_at": current.get("created_at") or now,
        "updated_at": now,
    }
    workers[key] = worker
    config["workers"] = workers
    _save_config(config)
    _emit(_public_worker(worker), json_output=json_output)


@line_app.command("list")
def line_list(json_output: bool = typer.Option(False, "--json")) -> None:
    """List named LINE worker configs."""
    config = _load_config()
    workers = [
        _public_worker(worker)
        for key, worker in sorted((config.get("workers") or {}).items())
        if key.startswith("line/") and isinstance(worker, dict)
    ]
    _emit(workers, json_output=json_output)


@line_app.command("status")
def line_status(name: str | None = typer.Argument(None), json_output: bool = typer.Option(False, "--json")) -> None:
    """Show named LINE worker runtime status."""
    config, config_issues = _load_config_payload()
    if name is not None:
        worker = _get_worker("line", name, config=config)
        _emit(_worker_status(worker, config=config, config_issues=config_issues), json_output=json_output)
        return
    workers = [
        _worker_status(worker, config=config, config_issues=config_issues)
        for worker in _iter_line_workers(config=config)
    ]
    _emit(workers, json_output=json_output)


@line_app.command("start")
def line_start(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Start a named LINE worker in daemon mode."""
    with _line_lifecycle_lock(f"line/start/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("line", name, config=config)
        status = _worker_status(worker, config=config, config_issues=config_issues)
        if status["running"]:
            status["started"] = False
            _emit(status, json_output=json_output)
            return
        payload = _start_line_worker(worker, config=config, config_issues=config_issues)
    _emit(payload, json_output=json_output)


@line_app.command("stop")
def line_stop(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Stop a named LINE worker."""
    with _line_lifecycle_lock(f"line/stop/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("line", name, config=config)
        payload, _ = _stop_line_worker(
            worker,
            reason="cli_line_stop",
            config=config,
            config_issues=config_issues,
        )
    _emit(payload, json_output=json_output)


@line_app.command("restart")
def line_restart(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Stop and restart a named LINE worker in daemon mode."""
    with _line_lifecycle_lock(f"line/restart/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("line", name, config=config)
        payload = _restart_line_worker(
            worker,
            reason="cli_line_restart",
            config=config,
            config_issues=config_issues,
        )
    _emit(payload, json_output=json_output)


@line_app.command("logs")
def line_logs(
    name: str,
    lines: int = typer.Option(100, "--lines", min=1, max=10000),
    json_output: bool = typer.Option(False, "--json"),
) -> None:
    """Read the last N lines from a named LINE worker's log."""
    config, config_issues = _load_config_payload()
    worker = _get_worker("line", name, config=config)
    paths = _worker_paths(worker)
    log_lines = _read_log_tail(paths["log_file"], lines)
    status = _worker_status(worker, config=config, config_issues=config_issues)
    status.update({"lines": log_lines, "log_exists": paths["log_file"].exists(), "lines_requested": lines})
    if json_output:
        _emit(status, json_output=True)
        return
    if not log_lines:
        typer.echo(f"No log lines available for {name}.")
        return
    for line in log_lines:
        typer.echo(line)


@line_app.command("remove")
def line_remove(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Remove a named LINE worker config."""
    key = _worker_key("line", name)
    config = _load_config()
    workers = dict(config.get("workers") or {})
    removed = workers.pop(key, None)
    config["workers"] = workers
    _save_config(config)
    payload = {"removed": removed is not None, "kind": "line", "name": name.strip()}
    _emit(payload, json_output=json_output)


@discord_app.callback(invoke_without_command=True)
def discord(
    ctx: typer.Context,
    mode: str = typer.Option("serve", "--mode", "-m", help="Discord worker mode: serve (default) or interaction."),
) -> None:
    """Run the Discord runtime worker."""
    if ctx.invoked_subcommand is not None:
        return
    normalized_mode = mode.strip().lower()
    if normalized_mode == "serve":
        from ait_agent.discord.app import main as discord_main

        discord_main()
        return
    if normalized_mode == "interaction":
        from ait_agent.discord.app import interaction_main

        interaction_main()
        return
    raise typer.BadParameter("mode must be either 'serve' or 'interaction'")


@discord_app.command("add")
def discord_add(
    name: str,
    application_id: str = typer.Option(..., "--application-id", help="Discord application id for this named worker."),
    bot_token: str = typer.Option(..., "--bot-token", help="Discord bot token for this named worker."),
    sync_state_path: str | None = typer.Option(None, "--sync-state-path", help="Per-worker Discord sync-state path."),
    pid_file: str | None = typer.Option(None, "--pid-file", help="Per-worker PID file path."),
    log_file: str | None = typer.Option(None, "--log-file", help="Per-worker log file path."),
    env_path: str | None = typer.Option(None, "--env-path", help="Per-worker Discord env file path."),
    json_output: bool = typer.Option(False, "--json"),
) -> None:
    """Add or update a named Discord worker config."""
    key = _worker_key("discord", name)
    config = _load_config()
    workers = dict(config.get("workers") or {})
    now = _utc_now_iso()
    current = dict(workers.get(key) or {})
    worker = {
        "kind": "discord",
        "name": name.strip(),
        "application_id": application_id,
        "bot_token": bot_token,
        "sync_state_path": sync_state_path,
        "pid_file": pid_file,
        "log_file": log_file,
        "env_path": env_path,
        "created_at": current.get("created_at") or now,
        "updated_at": now,
    }
    workers[key] = worker
    config["workers"] = workers
    _save_config(config)
    _emit(_public_worker(worker), json_output=json_output)


@discord_app.command("list")
def discord_list(json_output: bool = typer.Option(False, "--json")) -> None:
    """List named Discord worker configs."""
    config = _load_config()
    workers = [
        _public_worker(worker)
        for worker in _iter_discord_workers(config=config)
    ]
    _emit(workers, json_output=json_output)


@discord_app.command("status")
def discord_status(name: str | None = typer.Argument(None), json_output: bool = typer.Option(False, "--json")) -> None:
    """Show named Discord worker runtime status."""
    config, config_issues = _load_config_payload()
    if name is not None:
        worker = _get_worker("discord", name, config=config)
        _emit(_worker_status(worker, config=config, config_issues=config_issues), json_output=json_output)
        return
    workers = [
        _worker_status(worker, config=config, config_issues=config_issues)
        for worker in _iter_discord_workers(config=config)
    ]
    _emit(workers, json_output=json_output)


@discord_app.command("start")
def discord_start(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Start a named Discord worker in daemon mode."""
    with _discord_lifecycle_lock(f"discord/start/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("discord", name, config=config)
        status = _worker_status(worker, config=config, config_issues=config_issues)
        if status["running"]:
            status["started"] = False
            _emit(status, json_output=json_output)
            return
        payload = _start_discord_worker(worker, config=config, config_issues=config_issues)
    _emit(payload, json_output=json_output)


@discord_app.command("stop")
def discord_stop(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Stop a named Discord worker."""
    with _discord_lifecycle_lock(f"discord/stop/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("discord", name, config=config)
        payload, _ = _stop_discord_worker(
            worker,
            reason="cli_discord_stop",
            config=config,
            config_issues=config_issues,
        )
    _emit(payload, json_output=json_output)


@discord_app.command("restart")
def discord_restart(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Stop and restart a named Discord worker in daemon mode."""
    with _discord_lifecycle_lock(f"discord/restart/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("discord", name, config=config)
        payload = _restart_discord_worker(
            worker,
            reason="cli_discord_restart",
            config=config,
            config_issues=config_issues,
        )
    _emit(payload, json_output=json_output)


@discord_app.command("logs")
def discord_logs(
    name: str,
    lines: int = typer.Option(100, "--lines", min=1, max=10000),
    json_output: bool = typer.Option(False, "--json"),
) -> None:
    """Read the last N lines from a named Discord worker's log."""
    config, config_issues = _load_config_payload()
    worker = _get_worker("discord", name, config=config)
    paths = _worker_paths(worker)
    log_lines = _read_log_tail(paths["log_file"], lines)
    status = _worker_status(worker, config=config, config_issues=config_issues)
    status.update({"lines": log_lines, "log_exists": paths["log_file"].exists(), "lines_requested": lines})
    if json_output:
        _emit(status, json_output=True)
        return
    if not log_lines:
        typer.echo(f"No log lines available for {name}.")
        return
    for line in log_lines:
        typer.echo(line)


@discord_app.command("remove")
def discord_remove(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Remove a named Discord worker config."""
    key = _worker_key("discord", name)
    config = _load_config()
    workers = dict(config.get("workers") or {})
    removed = workers.pop(key, None)
    config["workers"] = workers
    _save_config(config)
    payload = {"removed": removed is not None, "kind": "discord", "name": name.strip()}
    _emit(payload, json_output=json_output)


@slack_app.callback(invoke_without_command=True)
def slack(
    ctx: typer.Context,
    mode: str = typer.Option("serve", "--mode", "-m", help="Slack worker mode: serve (default) or command."),
) -> None:
    """Run the Slack runtime worker."""
    if ctx.invoked_subcommand is not None:
        return
    normalized_mode = mode.strip().lower()
    if normalized_mode == "serve":
        from ait_agent.slack.app import main as slack_main

        slack_main()
        return
    if normalized_mode == "command":
        from ait_agent.slack.app import command_main

        command_main()
        return
    raise typer.BadParameter("mode must be either 'serve' or 'command'")


@slack_app.command("add")
def slack_add(
    name: str,
    app_token: str = typer.Option(..., "--app-token", help="Slack app token for Socket Mode on this named worker."),
    sync_state_path: str | None = typer.Option(None, "--sync-state-path", help="Per-worker Slack sync-state path."),
    pid_file: str | None = typer.Option(None, "--pid-file", help="Per-worker PID file path."),
    log_file: str | None = typer.Option(None, "--log-file", help="Per-worker log file path."),
    env_path: str | None = typer.Option(None, "--env-path", help="Per-worker Slack env file path."),
    json_output: bool = typer.Option(False, "--json"),
) -> None:
    """Add or update a named Slack worker config."""
    key = _worker_key("slack", name)
    config = _load_config()
    workers = dict(config.get("workers") or {})
    now = _utc_now_iso()
    current = dict(workers.get(key) or {})
    worker = {
        "kind": "slack",
        "name": name.strip(),
        "app_token": app_token,
        "sync_state_path": sync_state_path,
        "pid_file": pid_file,
        "log_file": log_file,
        "env_path": env_path,
        "created_at": current.get("created_at") or now,
        "updated_at": now,
    }
    workers[key] = worker
    config["workers"] = workers
    _save_config(config)
    _emit(_public_worker(worker), json_output=json_output)


@slack_app.command("list")
def slack_list(json_output: bool = typer.Option(False, "--json")) -> None:
    """List named Slack worker configs."""
    config = _load_config()
    workers = [
        _public_worker(worker)
        for worker in _iter_slack_workers(config=config)
    ]
    _emit(workers, json_output=json_output)


@slack_app.command("status")
def slack_status(name: str | None = typer.Argument(None), json_output: bool = typer.Option(False, "--json")) -> None:
    """Show named Slack worker runtime status."""
    config, config_issues = _load_config_payload()
    if name is not None:
        worker = _get_worker("slack", name, config=config)
        _emit(_worker_status(worker, config=config, config_issues=config_issues), json_output=json_output)
        return
    workers = [
        _worker_status(worker, config=config, config_issues=config_issues)
        for worker in _iter_slack_workers(config=config)
    ]
    _emit(workers, json_output=json_output)


@slack_app.command("start")
def slack_start(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Start a named Slack worker in daemon mode."""
    with _slack_lifecycle_lock(f"slack/start/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("slack", name, config=config)
        status = _worker_status(worker, config=config, config_issues=config_issues)
        if status["running"]:
            status["started"] = False
            _emit(status, json_output=json_output)
            return
        payload = _start_slack_worker(worker, config=config, config_issues=config_issues)
    _emit(payload, json_output=json_output)


@slack_app.command("stop")
def slack_stop(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Stop a named Slack worker."""
    with _slack_lifecycle_lock(f"slack/stop/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("slack", name, config=config)
        payload, _ = _stop_slack_worker(
            worker,
            reason="cli_slack_stop",
            config=config,
            config_issues=config_issues,
        )
    _emit(payload, json_output=json_output)


@slack_app.command("restart")
def slack_restart(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Stop and restart a named Slack worker in daemon mode."""
    with _slack_lifecycle_lock(f"slack/restart/{name}"):
        config, config_issues = _load_config_payload()
        worker = _get_worker("slack", name, config=config)
        payload = _restart_slack_worker(
            worker,
            reason="cli_slack_restart",
            config=config,
            config_issues=config_issues,
        )
    _emit(payload, json_output=json_output)


@slack_app.command("logs")
def slack_logs(
    name: str,
    lines: int = typer.Option(100, "--lines", min=1, max=10000),
    json_output: bool = typer.Option(False, "--json"),
) -> None:
    """Read the last N lines from a named Slack worker's log."""
    config, config_issues = _load_config_payload()
    worker = _get_worker("slack", name, config=config)
    paths = _worker_paths(worker)
    log_lines = _read_log_tail(paths["log_file"], lines)
    status = _worker_status(worker, config=config, config_issues=config_issues)
    status.update({"lines": log_lines, "log_exists": paths["log_file"].exists(), "lines_requested": lines})
    if json_output:
        _emit(status, json_output=True)
        return
    if not log_lines:
        typer.echo(f"No log lines available for {name}.")
        return
    for line in log_lines:
        typer.echo(line)


@slack_app.command("remove")
def slack_remove(name: str, json_output: bool = typer.Option(False, "--json")) -> None:
    """Remove a named Slack worker config."""
    key = _worker_key("slack", name)
    config = _load_config()
    workers = dict(config.get("workers") or {})
    removed = workers.pop(key, None)
    config["workers"] = workers
    _save_config(config)
    payload = {"removed": removed is not None, "kind": "slack", "name": name.strip()}
    _emit(payload, json_output=json_output)


telegram_app.add_typer(telegram_supervisor_app, name="supervisor")
app.add_typer(telegram_app, name="telegram")
app.add_typer(line_app, name="line")
app.add_typer(discord_app, name="discord")
app.add_typer(slack_app, name="slack")


def main() -> None:
    app()
