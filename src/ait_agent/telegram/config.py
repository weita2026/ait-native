from __future__ import annotations

import json
import math
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping

from ait_agent.runtime_backend import AgentRuntimeConfigError, resolve_agent_runtime_target

from .runtime import (
    DEFAULT_TELEGRAM_OPENAI_TIMEOUT_SECONDS,
    DEFAULT_TELEGRAM_POLL_TIMEOUT_SECONDS,
    DEFAULT_TELEGRAM_REQUEST_TIMEOUT_SECONDS,
    load_simple_env_file,
    normalize_openai_api_key,
    recover_repo_local_sync_state_from_shared_runtime_root,
    resolve_telegram_env_path,
    resolve_telegram_sync_state_path,
)

DEFAULT_OPENAI_BASE_URL = "https://api.openai.com/v1"
DEFAULT_OPENAI_MODEL = "gpt-5.4-mini"
DEFAULT_TELEGRAM_STT_MODEL = "mlx-community/whisper-large-v3-mlx"
DEFAULT_TURN_MERGE_WINDOW_SECONDS = 0.35
DEFAULT_TURN_MERGE_MAX_MESSAGES = 4
VALID_TELEGRAM_STT_MODES = frozenset({"off", "local-stt"})


class BotRuntimeError(RuntimeError):
    pass


@dataclass(frozen=True)
class BotConfig:
    token: str
    username: str
    ait_server_url: str | None
    ait_web_url: str | None
    repo_name: str
    request_timeout_seconds: float | None
    poll_timeout_seconds: int
    background_sync_enabled: bool
    background_sync_interval_seconds: float
    graph_watch_background_sweep_enabled: bool
    openai_api_key: str | None
    openai_base_url: str
    openai_model: str
    openai_reasoning_effort: str | None
    openai_timeout_seconds: float | None
    openai_max_output_tokens: int
    ai_history_limit: int
    turn_merge_window_seconds: float
    turn_merge_max_messages: int
    decoupled_reply_enabled: bool
    reply_markdown_enabled: bool
    sync_state_path: Path
    env_path: Path
    runtime_mode: str = "remote"
    runtime_remote_name: str | None = None
    owner_bootstrap_enabled: bool = False
    stt_mode: str = "off"
    stt_model: str = DEFAULT_TELEGRAM_STT_MODEL
    stt_device: str = "auto"
    stt_compute_type: str | None = None
    stt_language: str | None = None
    stt_include_audio_uploads: bool = False


def _clean_optional_str(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


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


def _parse_non_negative_float(value: str | None, fallback: float) -> float:
    try:
        parsed = float(value or "")
    except ValueError:
        return fallback
    if parsed < 0:
        return fallback
    return parsed


def _parse_int(value: str | None, fallback: int, minimum: int) -> int:
    try:
        parsed = int(value or "")
    except ValueError:
        return fallback
    if parsed <= 0:
        return fallback
    return max(parsed, minimum)


def _parse_bool(value: str | None, default: bool) -> bool:
    raw = str(value or "").strip().lower()
    if not raw:
        return default
    if raw in {"1", "true", "yes", "on"}:
        return True
    if raw in {"0", "false", "no", "off"}:
        return False
    return default


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


def load_config(repo_root: Path | None = None, *, env: Mapping[str, str] | None = None) -> BotConfig:
    source_env = os.environ if env is None else env
    env_path = resolve_telegram_env_path(repo_root, source_env.get("AIT_TELEGRAM_ENV_PATH"))
    values = load_simple_env_file(env_path)
    resolved_root = repo_root or Path(source_env.get("AIT_REPO_ROOT") or Path.cwd())
    try:
        runtime_target = resolve_agent_runtime_target(resolved_root)
    except AgentRuntimeConfigError as exc:
        raise BotRuntimeError(str(exc)) from exc
    repo_default_model = _load_repo_default_model(repo_root)
    env_value = lambda *names, default=None: _env_value(values, *names, default=default, env=source_env)
    token = env_value("AIT_TELEGRAM_BOT_TOKEN", "BOT_TOKEN")
    if not token:
        raise BotRuntimeError(
            f"Missing Telegram bot token. Set AIT_TELEGRAM_BOT_TOKEN or BOT_TOKEN in {env_path}."
        )
    username = (env_value("AIT_TELEGRAM_BOT_USERNAME", "BOT_USERNAME", default="") or "").strip().lstrip("@")
    ait_server_url = runtime_target.server_url
    ait_web_url_raw = env_value("AIT_TELEGRAM_WEB_URL", "AIT_WEB_URL")
    ait_web_url = _normalize_base_url(ait_web_url_raw, ait_server_url) if ait_web_url_raw else None
    request_timeout_seconds = _parse_timeout_seconds(
        env_value("AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS", "AIT_TELEGRAM_TIMEOUT_SECONDS"),
        DEFAULT_TELEGRAM_REQUEST_TIMEOUT_SECONDS,
        5.0,
    )
    poll_timeout_seconds = _parse_int(
        env_value("AIT_TELEGRAM_POLL_TIMEOUT_SECONDS"),
        DEFAULT_TELEGRAM_POLL_TIMEOUT_SECONDS,
        5,
    )
    background_sync_enabled = _parse_bool(
        env_value("AIT_TELEGRAM_BACKGROUND_SYNC_ENABLED", "AIT_TELEGRAM_SYNC_ENABLED"),
        False,
    )
    background_sync_interval_seconds = _parse_float(
        env_value("AIT_TELEGRAM_BACKGROUND_SYNC_INTERVAL_SECONDS", "AIT_TELEGRAM_SYNC_INTERVAL_SECONDS"),
        30.0,
        5.0,
    )
    graph_watch_background_sweep_enabled = _parse_bool(
        env_value(
            "AIT_TELEGRAM_GRAPH_WATCH_BACKGROUND_SWEEP_ENABLED",
            "AIT_TELEGRAM_GRAPH_WATCH_SWEEP_ENABLED",
        ),
        False,
    )
    openai_api_key = normalize_openai_api_key(
        env_value("AIT_TELEGRAM_OPENAI_API_KEY", "AIT_OPENAI_API_KEY", "OPENAI_API_KEY")
    )
    openai_base_url = _normalize_base_url(
        env_value("AIT_TELEGRAM_OPENAI_BASE_URL", "AIT_OPENAI_BASE_URL", "OPENAI_BASE_URL"),
        DEFAULT_OPENAI_BASE_URL,
    )
    openai_model = (
        env_value(
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
    openai_reasoning_effort = (env_value("AIT_TELEGRAM_REASONING_EFFORT", default="low") or "").strip() or None
    default_openai_timeout_seconds = request_timeout_seconds
    if default_openai_timeout_seconds is None:
        default_openai_timeout_seconds = DEFAULT_TELEGRAM_OPENAI_TIMEOUT_SECONDS
    elif DEFAULT_TELEGRAM_OPENAI_TIMEOUT_SECONDS is not None:
        default_openai_timeout_seconds = max(
            default_openai_timeout_seconds,
            DEFAULT_TELEGRAM_OPENAI_TIMEOUT_SECONDS,
        )
    openai_timeout_seconds = _parse_timeout_seconds(
        env_value("AIT_TELEGRAM_OPENAI_TIMEOUT_SECONDS"),
        default_openai_timeout_seconds,
        10.0,
    )
    openai_max_output_tokens = _parse_int(
        env_value("AIT_TELEGRAM_MAX_OUTPUT_TOKENS"),
        700,
        64,
    )
    ai_history_limit = _parse_int(
        env_value("AIT_TELEGRAM_HISTORY_LIMIT"),
        24,
        4,
    )
    turn_merge_window_seconds = _parse_non_negative_float(
        env_value("AIT_TELEGRAM_TURN_MERGE_WINDOW_SECONDS"),
        DEFAULT_TURN_MERGE_WINDOW_SECONDS,
    )
    turn_merge_max_messages = _parse_int(
        env_value("AIT_TELEGRAM_TURN_MERGE_MAX_MESSAGES"),
        DEFAULT_TURN_MERGE_MAX_MESSAGES,
        1,
    )
    decoupled_reply_enabled = _parse_bool(
        env_value("AIT_TELEGRAM_DECOUPLED_REPLY_ENABLED"),
        True,
    )
    reply_markdown_enabled = _parse_bool(
        env_value("AIT_TELEGRAM_REPLY_MARKDOWN_ENABLED", "AIT_TELEGRAM_MARKDOWN_ENABLED"),
        True,
    )
    owner_bootstrap_enabled = _parse_bool(
        env_value("AIT_TELEGRAM_OWNER_BOOTSTRAP_ENABLED"),
        True,
    )
    stt_mode = (env_value("AIT_TELEGRAM_STT_MODE", default="off") or "off").strip().lower() or "off"
    if stt_mode not in VALID_TELEGRAM_STT_MODES:
        raise BotRuntimeError(
            f"Unsupported Telegram STT mode {stt_mode!r}. Use one of: {', '.join(sorted(VALID_TELEGRAM_STT_MODES))}."
        )
    stt_model = (
        env_value("AIT_TELEGRAM_STT_MODEL", default=DEFAULT_TELEGRAM_STT_MODEL) or DEFAULT_TELEGRAM_STT_MODEL
    ).strip() or DEFAULT_TELEGRAM_STT_MODEL
    stt_device = (env_value("AIT_TELEGRAM_STT_DEVICE", default="auto") or "auto").strip().lower() or "auto"
    stt_compute_type = _clean_optional_str(env_value("AIT_TELEGRAM_STT_COMPUTE_TYPE"))
    stt_language = _clean_optional_str(env_value("AIT_TELEGRAM_STT_LANGUAGE"))
    stt_include_audio_uploads = _parse_bool(
        env_value("AIT_TELEGRAM_STT_INCLUDE_AUDIO_UPLOADS"),
        False,
    )
    state_path = resolve_telegram_sync_state_path(env_value("AIT_TELEGRAM_STATE_PATH"))
    recover_repo_local_sync_state_from_shared_runtime_root(
        state_path,
        repo_name=runtime_target.repo_name,
    )
    return BotConfig(
        token=token,
        username=username,
        ait_server_url=ait_server_url,
        ait_web_url=ait_web_url,
        repo_name=runtime_target.repo_name,
        request_timeout_seconds=request_timeout_seconds,
        poll_timeout_seconds=poll_timeout_seconds,
        background_sync_enabled=background_sync_enabled,
        background_sync_interval_seconds=background_sync_interval_seconds,
        graph_watch_background_sweep_enabled=graph_watch_background_sweep_enabled,
        openai_api_key=openai_api_key,
        openai_base_url=openai_base_url,
        openai_model=openai_model,
        openai_reasoning_effort=openai_reasoning_effort,
        openai_timeout_seconds=openai_timeout_seconds,
        openai_max_output_tokens=openai_max_output_tokens,
        ai_history_limit=ai_history_limit,
        turn_merge_window_seconds=turn_merge_window_seconds,
        turn_merge_max_messages=turn_merge_max_messages,
        decoupled_reply_enabled=decoupled_reply_enabled,
        reply_markdown_enabled=reply_markdown_enabled,
        sync_state_path=state_path,
        env_path=env_path,
        runtime_mode=runtime_target.mode,
        runtime_remote_name=runtime_target.remote_name,
        owner_bootstrap_enabled=owner_bootstrap_enabled,
        stt_mode=stt_mode,
        stt_model=stt_model,
        stt_device=stt_device,
        stt_compute_type=stt_compute_type,
        stt_language=stt_language,
        stt_include_audio_uploads=stt_include_audio_uploads,
    )


__all__ = [
    "BotConfig",
    "BotRuntimeError",
    "DEFAULT_OPENAI_BASE_URL",
    "DEFAULT_OPENAI_MODEL",
    "DEFAULT_TELEGRAM_STT_MODEL",
    "DEFAULT_TURN_MERGE_MAX_MESSAGES",
    "DEFAULT_TURN_MERGE_WINDOW_SECONDS",
    "VALID_TELEGRAM_STT_MODES",
    "load_config",
]
