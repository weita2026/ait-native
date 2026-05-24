from __future__ import annotations

import os
from pathlib import Path


DEFAULT_REPLY_ENV_PATH = Path(".ait") / "agent-runtime" / "telegram.env"
DEFAULT_REPLY_OPENAI_TIMEOUT_SECONDS: float | None = None
DEFAULT_REPLY_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS = 30.0
DEFAULT_REPLY_CODEX_TURN_TIMEOUT_SECONDS: float | None = None
DEFAULT_REPLY_CODEX_CHILD_REAP_TIMEOUT_SECONDS = 3.5
PLACEHOLDER_OPENAI_API_KEYS = {
    "your-openai-api-key",
    "sk-your-openai-api-key",
    "your_openai_api_key",
    "replace-with-real-openai-api-key",
}


def resolve_reply_runtime_env_path(
    repo_root: Path | None = None,
    value: str | os.PathLike[str] | None = None,
) -> Path:
    root = repo_root or Path.cwd()
    default_path = root / DEFAULT_REPLY_ENV_PATH
    if value:
        candidate = Path(value).expanduser()
        if repo_root is not None and default_path.exists():
            try:
                resolved_root = root.resolve()
            except OSError:
                resolved_root = root
            try:
                resolved_candidate = candidate.resolve()
            except OSError:
                resolved_candidate = candidate
            if resolved_candidate != default_path.resolve() and resolved_root not in resolved_candidate.parents:
                return default_path
        return candidate
    return default_path


def load_runtime_env_file(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}
    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, raw_value = line.split("=", 1)
        key = key.strip()
        if not key:
            continue
        value = raw_value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        values[key] = value
    return values


def normalize_openai_api_key(value: str | None) -> str | None:
    raw = str(value or "").strip()
    if not raw:
        return None
    if raw.lower() in PLACEHOLDER_OPENAI_API_KEYS:
        return None
    return raw


__all__ = [
    "DEFAULT_REPLY_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS",
    "DEFAULT_REPLY_CODEX_CHILD_REAP_TIMEOUT_SECONDS",
    "DEFAULT_REPLY_CODEX_TURN_TIMEOUT_SECONDS",
    "DEFAULT_REPLY_ENV_PATH",
    "DEFAULT_REPLY_OPENAI_TIMEOUT_SECONDS",
    "PLACEHOLDER_OPENAI_API_KEYS",
    "load_runtime_env_file",
    "normalize_openai_api_key",
    "resolve_reply_runtime_env_path",
]
