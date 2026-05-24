from __future__ import annotations

import json
import math
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping

from ait_chat.codex_app_server import (
    DEFAULT_CODEX_APP_SERVER_HOST,
    DEFAULT_CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES,
    resolve_codex_bin,
)
from ait_chat.runtime_config import (
    DEFAULT_REPLY_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS,
    DEFAULT_REPLY_CODEX_CHILD_REAP_TIMEOUT_SECONDS,
    DEFAULT_REPLY_CODEX_TURN_TIMEOUT_SECONDS,
    DEFAULT_REPLY_OPENAI_TIMEOUT_SECONDS,
    load_runtime_env_file,
    normalize_openai_api_key,
    resolve_reply_runtime_env_path,
)


DEFAULT_OPENAI_BASE_URL = "https://api.openai.com/v1"
DEFAULT_OPENAI_MODEL = "gpt-5.4-mini"
DEFAULT_CODEX_MODEL = "gpt-5.4"
DEFAULT_CODEX_REASONING_EFFORT = "xhigh"
DEFAULT_CODEX_PRIMARY_SURFACES = ("discord",)
# Match the trusted Telegram-linked execution mode, which resolves to Codex full
# access unless operators override it in the ait-agent Telegram env file.
DEFAULT_CODEX_SANDBOX = "danger-full-access"
DEFAULT_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS = DEFAULT_REPLY_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS
DEFAULT_CODEX_TURN_TIMEOUT_SECONDS = DEFAULT_REPLY_CODEX_TURN_TIMEOUT_SECONDS
DEFAULT_CODEX_CHILD_KILL_GRACE_SECONDS = 2.0
DEFAULT_CODEX_CHILD_REAP_TIMEOUT_SECONDS = DEFAULT_REPLY_CODEX_CHILD_REAP_TIMEOUT_SECONDS
DEFAULT_CODEX_PERSISTENT_CLIENT = True
DEFAULT_CODEX_WORKER_POOL_STRATEGY = "session"
DEFAULT_CODEX_CAPACITY_RETRY_LIMIT = 1
DEFAULT_CODEX_CAPACITY_CONTINUE_TEXT = "請繼續"
VALID_CODEX_WORKER_POOL_STRATEGIES = frozenset({"session", "chat", "bot"})
DEFAULT_TELEGRAM_CHECKPOINT_EVENT_THRESHOLD = 6
DEFAULT_TELEGRAM_CHECKPOINT_SUMMARY_EVENT_LIMIT = 8


@dataclass(frozen=True)
class ReplyGenerationConfig:
    repo_name: str
    openai_api_key: str | None
    openai_base_url: str
    openai_model: str
    openai_reasoning_effort: str | None
    openai_timeout_seconds: float | None
    openai_max_output_tokens: int
    history_limit: int
    telegram_checkpoint_event_threshold: int
    telegram_checkpoint_summary_event_limit: int
    telegram_append_turn_analysis: bool = False
    repo_root: Path | None = None
    codex_bin: str = "codex"
    codex_model: str = DEFAULT_CODEX_MODEL
    codex_reasoning_effort: str | None = DEFAULT_CODEX_REASONING_EFFORT
    codex_sandbox: str = DEFAULT_CODEX_SANDBOX
    codex_app_server_url: str | None = None
    codex_app_server_host: str = DEFAULT_CODEX_APP_SERVER_HOST
    codex_app_server_port: int = 0
    codex_app_server_ready_timeout_seconds: float = DEFAULT_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS
    codex_turn_timeout_seconds: float | None = DEFAULT_CODEX_TURN_TIMEOUT_SECONDS
    codex_child_kill_grace_seconds: float = DEFAULT_CODEX_CHILD_KILL_GRACE_SECONDS
    codex_child_reap_timeout_seconds: float = DEFAULT_CODEX_CHILD_REAP_TIMEOUT_SECONDS
    codex_websocket_max_size_bytes: int | None = DEFAULT_CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES
    codex_persistent_client: bool = DEFAULT_CODEX_PERSISTENT_CLIENT
    codex_worker_pool_strategy: str = DEFAULT_CODEX_WORKER_POOL_STRATEGY
    codex_capacity_retry_limit: int = DEFAULT_CODEX_CAPACITY_RETRY_LIMIT
    codex_capacity_continue_text: str = DEFAULT_CODEX_CAPACITY_CONTINUE_TEXT
    codex_primary_surfaces: tuple[str, ...] = DEFAULT_CODEX_PRIMARY_SURFACES


def _env_value(
    values: dict[str, str],
    *names: str,
    default: str | None = None,
    env: Mapping[str, str] | None = None,
) -> str | None:
    source_env = os.environ if env is None else env
    for name in names:
        raw = source_env.get(name)
        if raw is not None and raw.strip() != "":
            return raw.strip()
        fallback = values.get(name)
        if fallback is not None and str(fallback).strip() != "":
            return str(fallback).strip()
    return default


def _normalize_base_url(value: str | None, fallback: str) -> str:
    raw = (value or fallback).strip() or fallback
    return raw.rstrip("/")


def _parse_float(value: str | None, fallback: float, minimum: float) -> float:
    try:
        parsed = float(value or "")
    except ValueError:
        return fallback
    if parsed <= 0:
        return fallback
    return max(parsed, minimum)


def _parse_timeout_seconds(value: str | None, fallback: float | None, minimum: float) -> float | None:
    raw = str(value or "").strip()
    if not raw:
        return fallback
    if raw.lower() in {"inf", "infinite", "none", "null", "unlimited"}:
        return None
    try:
        parsed = float(raw)
    except ValueError:
        return fallback
    if not math.isfinite(parsed):
        return None
    if parsed <= 0:
        return fallback
    return max(parsed, minimum)


def _parse_int(value: str | None, fallback: int, minimum: int) -> int:
    try:
        parsed = int(value or "")
    except ValueError:
        return fallback
    if parsed <= 0:
        return fallback
    return max(parsed, minimum)


def _parse_int_optional(value: str | None, fallback: int, minimum: int) -> int:
    raw = str(value or "").strip()
    if not raw:
        return fallback
    try:
        parsed = int(raw)
    except ValueError:
        return fallback
    if parsed <= 0:
        return fallback
    return max(parsed, minimum)


def _parse_non_negative_int(value: str | None, fallback: int, maximum: int | None = None) -> int:
    raw = str(value or "").strip()
    if not raw:
        parsed = fallback
    else:
        try:
            parsed = int(raw)
        except ValueError:
            parsed = fallback
    parsed = max(parsed, 0)
    if maximum is not None:
        parsed = min(parsed, maximum)
    return parsed


def _parse_byte_limit(value: str | None, fallback: int | None, minimum: int) -> int | None:
    raw = str(value or "").strip()
    if not raw:
        return fallback
    if raw.lower() in {"inf", "infinite", "none", "null", "unlimited"}:
        return None
    try:
        parsed = int(raw)
    except ValueError:
        return fallback
    if parsed <= 0:
        return fallback
    return max(parsed, minimum)


def _parse_bool(value: str | None, fallback: bool) -> bool:
    raw = str(value or "").strip().lower()
    if not raw:
        return fallback
    if raw in {"1", "true", "yes", "on"}:
        return True
    if raw in {"0", "false", "no", "off"}:
        return False
    return fallback


def _parse_surface_list(value: str | None, fallback: tuple[str, ...]) -> tuple[str, ...]:
    raw = str(value or "").strip()
    if not raw:
        return fallback
    if raw.lower() in {"none", "null", "off", "false", "no"}:
        return ()
    parsed: list[str] = []
    seen: set[str] = set()
    for token in raw.replace(";", ",").split(","):
        surface = str(token).strip().lower()
        if not surface or surface in seen:
            continue
        seen.add(surface)
        parsed.append(surface)
    return tuple(parsed) if parsed else fallback


def _parse_timeout_milliseconds(value: str | None, fallback_seconds: float, minimum_seconds: float) -> float:
    raw = str(value or "").strip()
    if not raw:
        return fallback_seconds
    try:
        parsed = float(raw)
    except ValueError:
        return fallback_seconds
    if parsed <= 0:
        return fallback_seconds
    return max(parsed / 1000.0, minimum_seconds)


def _load_repo_default_model(repo_root: Path | None) -> str | None:
    if repo_root is None:
        return None
    config_path = repo_root / ".ait" / "config.json"
    if not config_path.exists():
        return None
    try:
        payload = json.loads(config_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    value = payload.get("default_model")
    if isinstance(value, str) and value.strip():
        return value.strip()
    return None


def load_reply_generation_config(
    repo_name: str = "ait",
    repo_root: Path | None = None,
    env_path: Path | None = None,
    env: Mapping[str, str] | None = None,
) -> ReplyGenerationConfig:
    source_env = os.environ if env is None else env
    configured_env_path = source_env.get("AIT_CHAT_ENV_PATH") or source_env.get("AIT_TELEGRAM_ENV_PATH")
    env_path = env_path or resolve_reply_runtime_env_path(repo_root, configured_env_path)
    values = load_runtime_env_file(env_path)
    env_value = lambda *names, default=None: _env_value(values, *names, default=default, env=source_env)
    repo_default_model = _load_repo_default_model(repo_root)
    resolved_repo_name = (env_value("AIT_TELEGRAM_REPO_NAME", "AIT_REPO_NAME", default=repo_name) or repo_name).strip()
    openai_api_key = normalize_openai_api_key(
        env_value(
            "AIT_CHAT_OPENAI_API_KEY",
            "AIT_TELEGRAM_OPENAI_API_KEY",
            "AIT_OPENAI_API_KEY",
            "OPENAI_API_KEY",
        )
    )
    openai_base_url = _normalize_base_url(
        env_value(
            "AIT_CHAT_OPENAI_BASE_URL",
            "AIT_TELEGRAM_OPENAI_BASE_URL",
            "AIT_OPENAI_BASE_URL",
            "OPENAI_BASE_URL",
        ),
        DEFAULT_OPENAI_BASE_URL,
    )
    openai_model = (
        env_value(
            "AIT_CHAT_MODEL",
            "AIT_TELEGRAM_MODEL",
            "AIT_TELEGRAM_OPENAI_MODEL",
            "AIT_MODEL",
            "CODEX_MODEL",
            "OPENAI_MODEL",
            default=repo_default_model or DEFAULT_OPENAI_MODEL,
        )
        or repo_default_model
        or DEFAULT_OPENAI_MODEL
    ).strip()
    openai_reasoning_effort = (
        env_value("AIT_CHAT_REASONING_EFFORT", "AIT_TELEGRAM_REASONING_EFFORT", default="low") or ""
    ).strip() or None
    openai_timeout_seconds = _parse_timeout_seconds(
        env_value("AIT_CHAT_OPENAI_TIMEOUT_SECONDS", "AIT_TELEGRAM_OPENAI_TIMEOUT_SECONDS"),
        DEFAULT_REPLY_OPENAI_TIMEOUT_SECONDS,
        10.0,
    )
    codex_bin = resolve_codex_bin(
        env_value("AIT_CHAT_CODEX_BIN", "AIT_TELEGRAM_CODEX_BIN", "CODEX_BIN")
    )
    codex_model = (
        env_value(
            "AIT_CHAT_CODEX_MODEL",
            "AIT_TELEGRAM_CODEX_MODEL",
            "CODEX_MODEL",
            default=DEFAULT_CODEX_MODEL,
        )
        or DEFAULT_CODEX_MODEL
    ).strip()
    codex_reasoning_effort = (
        env_value(
            "AIT_CHAT_CODEX_REASONING_EFFORT",
            "AIT_TELEGRAM_CODEX_REASONING_EFFORT",
            "CODEX_REASONING_EFFORT",
            default=DEFAULT_CODEX_REASONING_EFFORT,
        )
        or ""
    ).strip() or None
    codex_sandbox = (
        env_value(
            "AIT_CHAT_CODEX_SANDBOX",
            "AIT_TELEGRAM_CODEX_SANDBOX",
            "CODEX_SANDBOX",
            default=DEFAULT_CODEX_SANDBOX,
        )
        or DEFAULT_CODEX_SANDBOX
    ).strip()
    codex_app_server_url = env_value(
        "AIT_CHAT_CODEX_APP_SERVER_URL",
        "AIT_TELEGRAM_CODEX_APP_SERVER_URL",
        "CODEX_APP_SERVER_URL",
    )
    codex_app_server_host = (
        env_value(
            "AIT_CHAT_CODEX_APP_SERVER_HOST",
            "AIT_TELEGRAM_CODEX_APP_SERVER_HOST",
            "CODEX_APP_SERVER_HOST",
            default=DEFAULT_CODEX_APP_SERVER_HOST,
        )
        or DEFAULT_CODEX_APP_SERVER_HOST
    ).strip()
    codex_app_server_port = _parse_int_optional(
        env_value(
            "AIT_CHAT_CODEX_APP_SERVER_PORT",
            "AIT_TELEGRAM_CODEX_APP_SERVER_PORT",
            "CODEX_APP_SERVER_PORT",
        ),
        0,
        1,
    )
    codex_app_server_ready_timeout_seconds = _parse_timeout_seconds(
        env_value(
            "AIT_CHAT_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS",
            "AIT_TELEGRAM_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS",
        ),
        None,
        1.0,
    )
    if codex_app_server_ready_timeout_seconds is None:
        codex_app_server_ready_timeout_seconds = _parse_timeout_milliseconds(
            env_value("CODEX_APP_SERVER_READY_TIMEOUT_MS"),
            DEFAULT_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS,
            1.0,
        )
    codex_turn_timeout_seconds = _parse_timeout_seconds(
        env_value(
            "AIT_CHAT_CODEX_TURN_TIMEOUT_SECONDS",
            "AIT_TELEGRAM_CODEX_TURN_TIMEOUT_SECONDS",
        ),
        DEFAULT_CODEX_TURN_TIMEOUT_SECONDS,
        10.0,
    )
    codex_child_kill_grace_seconds = _parse_timeout_seconds(
        env_value(
            "AIT_CHAT_CODEX_CHILD_KILL_GRACE_SECONDS",
            "AIT_TELEGRAM_CODEX_CHILD_KILL_GRACE_SECONDS",
        ),
        None,
        0.1,
    )
    if codex_child_kill_grace_seconds is None:
        codex_child_kill_grace_seconds = _parse_timeout_milliseconds(
            env_value("CODEX_APP_SERVER_CHILD_KILL_GRACE_MS"),
            DEFAULT_CODEX_CHILD_KILL_GRACE_SECONDS,
            0.1,
        )
    codex_child_reap_timeout_seconds = _parse_timeout_seconds(
        env_value(
            "AIT_CHAT_CODEX_CHILD_REAP_TIMEOUT_SECONDS",
            "AIT_TELEGRAM_CODEX_CHILD_REAP_TIMEOUT_SECONDS",
        ),
        None,
        0.1,
    )
    if codex_child_reap_timeout_seconds is None:
        codex_child_reap_timeout_seconds = _parse_timeout_milliseconds(
            env_value("CODEX_APP_SERVER_CHILD_REAP_TIMEOUT_MS"),
            DEFAULT_CODEX_CHILD_REAP_TIMEOUT_SECONDS,
            0.1,
        )
    codex_websocket_max_size_bytes = _parse_byte_limit(
        env_value(
            "AIT_CHAT_CODEX_WEBSOCKET_MAX_SIZE_BYTES",
            "AIT_TELEGRAM_CODEX_WEBSOCKET_MAX_SIZE_BYTES",
            "AIT_CHAT_CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES",
            "AIT_TELEGRAM_CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES",
            "CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES",
        ),
        DEFAULT_CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES,
        1024 * 1024,
    )
    codex_persistent_client = _parse_bool(
        env_value(
            "AIT_CHAT_CODEX_PERSISTENT_CLIENT",
            "AIT_TELEGRAM_CODEX_PERSISTENT_CLIENT",
            "CODEX_APP_SERVER_PERSISTENT_CLIENT",
        ),
        DEFAULT_CODEX_PERSISTENT_CLIENT,
    )
    codex_worker_pool_strategy = env_value(
        "AIT_CHAT_CODEX_WORKER_POOL_STRATEGY",
        "AIT_TELEGRAM_CODEX_WORKER_POOL_STRATEGY",
        "CODEX_WORKER_POOL_STRATEGY",
        default=DEFAULT_CODEX_WORKER_POOL_STRATEGY,
    )
    if not isinstance(codex_worker_pool_strategy, str) or str(codex_worker_pool_strategy).strip() not in VALID_CODEX_WORKER_POOL_STRATEGIES:
        codex_worker_pool_strategy = DEFAULT_CODEX_WORKER_POOL_STRATEGY
    else:
        codex_worker_pool_strategy = str(codex_worker_pool_strategy).strip().lower()
    codex_capacity_retry_limit = _parse_non_negative_int(
        env_value(
            "AIT_CHAT_CODEX_CAPACITY_RETRY_LIMIT",
            "AIT_TELEGRAM_CODEX_CAPACITY_RETRY_LIMIT",
            "CODEX_CAPACITY_RETRY_LIMIT",
        ),
        DEFAULT_CODEX_CAPACITY_RETRY_LIMIT,
        maximum=5,
    )
    codex_capacity_continue_text = (
        env_value(
            "AIT_CHAT_CODEX_CAPACITY_CONTINUE_TEXT",
            "AIT_TELEGRAM_CODEX_CAPACITY_CONTINUE_TEXT",
            "CODEX_CAPACITY_CONTINUE_TEXT",
            default=DEFAULT_CODEX_CAPACITY_CONTINUE_TEXT,
        )
        or DEFAULT_CODEX_CAPACITY_CONTINUE_TEXT
    ).strip() or DEFAULT_CODEX_CAPACITY_CONTINUE_TEXT
    codex_primary_surfaces = _parse_surface_list(
        env_value(
            "AIT_CHAT_CODEX_PRIMARY_SURFACES",
            "AIT_TELEGRAM_CODEX_PRIMARY_SURFACES",
        ),
        DEFAULT_CODEX_PRIMARY_SURFACES,
    )
    openai_max_output_tokens = _parse_int(
        env_value("AIT_CHAT_MAX_OUTPUT_TOKENS", "AIT_TELEGRAM_MAX_OUTPUT_TOKENS"),
        700,
        64,
    )
    history_limit = _parse_int(
        env_value("AIT_CHAT_HISTORY_LIMIT", "AIT_TELEGRAM_HISTORY_LIMIT"),
        24,
        4,
    )
    telegram_checkpoint_event_threshold = _parse_int(
        env_value(
            "AIT_CHAT_CHECKPOINT_EVENT_THRESHOLD",
            "AIT_TELEGRAM_CHECKPOINT_EVENT_THRESHOLD",
        ),
        DEFAULT_TELEGRAM_CHECKPOINT_EVENT_THRESHOLD,
        2,
    )
    telegram_checkpoint_summary_event_limit = _parse_int(
        env_value(
            "AIT_CHAT_CHECKPOINT_SUMMARY_EVENT_LIMIT",
            "AIT_TELEGRAM_CHECKPOINT_SUMMARY_EVENT_LIMIT",
        ),
        DEFAULT_TELEGRAM_CHECKPOINT_SUMMARY_EVENT_LIMIT,
        2,
    )
    telegram_append_turn_analysis = _parse_bool(
        env_value(
            "AIT_CHAT_APPEND_TURN_ANALYSIS",
            "AIT_TELEGRAM_APPEND_TURN_ANALYSIS",
        ),
        False,
    )
    return ReplyGenerationConfig(
        repo_name=resolved_repo_name,
        openai_api_key=openai_api_key,
        openai_base_url=openai_base_url,
        openai_model=openai_model,
        openai_reasoning_effort=openai_reasoning_effort,
        openai_timeout_seconds=openai_timeout_seconds,
        openai_max_output_tokens=openai_max_output_tokens,
        history_limit=history_limit,
        telegram_checkpoint_event_threshold=telegram_checkpoint_event_threshold,
        telegram_checkpoint_summary_event_limit=telegram_checkpoint_summary_event_limit,
        telegram_append_turn_analysis=telegram_append_turn_analysis,
        repo_root=repo_root,
        codex_bin=codex_bin,
        codex_model=codex_model,
        codex_reasoning_effort=codex_reasoning_effort,
        codex_sandbox=codex_sandbox,
        codex_app_server_url=codex_app_server_url,
        codex_app_server_host=codex_app_server_host,
        codex_app_server_port=codex_app_server_port,
        codex_app_server_ready_timeout_seconds=codex_app_server_ready_timeout_seconds,
        codex_turn_timeout_seconds=codex_turn_timeout_seconds,
        codex_child_kill_grace_seconds=codex_child_kill_grace_seconds,
        codex_child_reap_timeout_seconds=codex_child_reap_timeout_seconds,
        codex_websocket_max_size_bytes=codex_websocket_max_size_bytes,
        codex_persistent_client=codex_persistent_client,
        codex_worker_pool_strategy=codex_worker_pool_strategy,
        codex_capacity_retry_limit=codex_capacity_retry_limit,
        codex_capacity_continue_text=codex_capacity_continue_text,
        codex_primary_surfaces=codex_primary_surfaces,
    )


__all__ = [
    "ReplyGenerationConfig",
    "load_reply_generation_config",
]
