from __future__ import annotations

import json
import os
from pathlib import Path
from typing import TYPE_CHECKING, Any, Mapping

if TYPE_CHECKING:
    from .config import BotConfig


AIT_TELEGRAM_TERMINATION_CONTEXT_ENV = "AIT_TELEGRAM_TERMINATION_CONTEXT_PATH"


def _agent_worker_config_path(repo_root: Path) -> Path:
    default_path = repo_root / ".ait" / "agent-workers.json"
    override = os.environ.get("AIT_AGENT_CONFIG_PATH")
    if override:
        candidate = Path(override).expanduser()
        if default_path.exists():
            try:
                resolved_root = repo_root.resolve()
            except OSError:
                resolved_root = repo_root
            try:
                resolved_candidate = candidate.resolve()
            except OSError:
                resolved_candidate = candidate
            if resolved_candidate != default_path.resolve() and resolved_root not in resolved_candidate.parents:
                return default_path
        return candidate
    return default_path


def _safe_worker_label(name: str) -> str:
    label = "".join(ch if ch.isalnum() or ch in {"-", "_"} else "-" for ch in name.strip())
    return label.strip("-") or "worker"


def _worker_runtime_dir(repo_root: Path) -> Path:
    return repo_root / ".ait" / "agent-runtime"


def _worker_runtime_path(repo_root: Path, value: object, default_name: str) -> Path:
    raw_value = str(value or "").strip()
    raw = Path(raw_value).expanduser() if raw_value else _worker_runtime_dir(repo_root) / default_name
    return raw if raw.is_absolute() else repo_root / raw


def _select_telegram_worker(
    workers: Mapping[str, Any],
    *,
    name: str | None = None,
) -> dict[str, Any] | None:
    requested = str(name or os.environ.get("AIT_TELEGRAM_GRAPH_TRIGGER_WORKER") or "main").strip()
    if requested:
        worker = workers.get(f"telegram/{requested}")
        if isinstance(worker, dict):
            return dict(worker)
    telegram_workers = [
        dict(worker)
        for key, worker in sorted(workers.items())
        if isinstance(key, str) and key.startswith("telegram/") and isinstance(worker, dict)
    ]
    if len(telegram_workers) == 1:
        return telegram_workers[0]
    return None


def _telegram_worker_env_from_agent_config(
    repo_root: Path,
    *,
    name: str | None = None,
) -> dict[str, str] | None:
    path = _agent_worker_config_path(repo_root)
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    workers = payload.get("workers")
    if not isinstance(workers, dict):
        return None
    worker = _select_telegram_worker(workers, name=name)
    if not worker:
        return None
    token = str(worker.get("token") or "").strip()
    if not token:
        return None
    label = _safe_worker_label(str(worker.get("name") or name or "worker"))
    env_path = _worker_runtime_path(repo_root, worker.get("env_path"), "telegram.env")
    sync_state_path = _worker_runtime_path(
        repo_root,
        worker.get("sync_state_path"),
        f"telegram-{label}-sync.json",
    )
    termination_context_path = _worker_runtime_path(
        repo_root,
        worker.get("termination_context_path"),
        f"telegram-{label}-termination.json",
    )
    env = dict(os.environ)
    env["AIT_REPO_ROOT"] = str(repo_root)
    env["AIT_TELEGRAM_ENV_PATH"] = str(env_path)
    env["AIT_TELEGRAM_BOT_TOKEN"] = token
    env["BOT_TOKEN"] = token
    env["AIT_TELEGRAM_STATE_PATH"] = str(sync_state_path)
    env[AIT_TELEGRAM_TERMINATION_CONTEXT_ENV] = str(termination_context_path)
    username = str(worker.get("username") or "").strip()
    if username:
        env["AIT_TELEGRAM_BOT_USERNAME"] = username
        env["BOT_USERNAME"] = username
    return env


def load_config_for_telegram_worker(repo_root: Path | None = None, *, name: str | None = None) -> BotConfig:
    from .config import load_config

    resolved_root = repo_root or Path(os.environ.get("AIT_REPO_ROOT") or Path.cwd())
    worker_env = _telegram_worker_env_from_agent_config(resolved_root, name=name)
    if worker_env is not None:
        return load_config(resolved_root, env=worker_env)
    return load_config(resolved_root)


__all__ = [
    "AIT_TELEGRAM_TERMINATION_CONTEXT_ENV",
    "load_config_for_telegram_worker",
]
