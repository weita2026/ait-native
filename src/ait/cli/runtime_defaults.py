from __future__ import annotations

import json
import os

import typer

from ait_protocol.common import AuthorMode, normalize_author_mode, normalize_optional_text

from ..repo_paths import RepoContext
from ..store import load_config


def _normalize_text_value(value: str | None) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _normalize_model_name(value: str | None) -> str | None:
    return _normalize_text_value(value)


def _detect_model_name() -> str | None:
    for env_name in ("AIT_MODEL", "CODEX_MODEL", "OPENAI_MODEL"):
        value = _normalize_model_name(os.environ.get(env_name))
        if value:
            return value
    return None


def _detect_actor_identity() -> str | None:
    return _normalize_text_value(os.environ.get("AIT_NATIVE_ACTOR") or os.environ.get("AIT_ACTOR"))


def _format_identity(name: str | None, email: str | None) -> str | None:
    if name and email:
        return f"{name} <{email}>"
    return email or name


def _effective_author_mode(ctx: RepoContext, requested: AuthorMode | None = None) -> str:
    if requested is not None:
        return requested.value
    configured = load_config(ctx).get("default_author_mode")
    if configured:
        return normalize_author_mode(configured)
    return AuthorMode.AI_WITH_HUMAN_REVIEW.value


def _effective_model_name(ctx: RepoContext, requested: str | None = None) -> str | None:
    explicit = _normalize_model_name(requested)
    if explicit:
        return explicit
    detected = _detect_model_name()
    if detected:
        return detected
    return _normalize_model_name(load_config(ctx).get("default_model"))


def _effective_session_id(requested: str | None = None) -> str | None:
    explicit = normalize_optional_text(requested)
    if explicit:
        return explicit
    env_session = normalize_optional_text(os.environ.get("AIT_SESSION_ID"))
    if env_session:
        return env_session
    return None


def _effective_checkpoint_id(requested: str | None = None) -> str | None:
    explicit = normalize_optional_text(requested)
    if explicit:
        return explicit
    return normalize_optional_text(os.environ.get("AIT_CHECKPOINT_ID"))


def _effective_reviewer_identity(ctx: RepoContext, requested: str | None = None) -> str | None:
    explicit = _normalize_text_value(requested)
    if explicit:
        return explicit
    cfg = load_config(ctx)
    configured = _format_identity(
        _normalize_text_value(cfg.get("user_name")),
        _normalize_text_value(cfg.get("user_email")),
    )
    if configured:
        return configured
    return _detect_actor_identity()


def _effective_actor_identity(ctx: RepoContext) -> str | None:
    detected = _detect_actor_identity()
    if detected:
        return detected
    cfg = load_config(ctx)
    return _normalize_text_value(cfg.get("user_email")) or _normalize_text_value(cfg.get("user_name"))


def _parse_json_object_option(raw: str | None, option_name: str) -> dict:
    if raw is None:
        return {}
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise typer.BadParameter(f"{option_name} must be valid JSON: {exc}") from exc
    if not isinstance(payload, dict):
        raise typer.BadParameter(f"{option_name} must decode to a JSON object.")
    return payload


def _parse_key_value_options(values: list[str] | None, option_name: str) -> dict[str, str]:
    out: dict[str, str] = {}
    for item in values or []:
        if "=" not in item:
            raise typer.BadParameter(f"{option_name} entries must use key=value syntax: {item}")
        key, value = item.split("=", 1)
        key = key.strip()
        if not key:
            raise typer.BadParameter(f"{option_name} entries must include a non-empty key: {item}")
        out[key] = value
    return out
