from __future__ import annotations

import json
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any
from urllib.request import urlopen

from ait_chat.codex_reply import generate_codex_session_reply
from ait_chat.reply_config import ReplyGenerationConfig, load_reply_generation_config
from ait_chat.reply_attachments import extract_reply_attachments
from ait_chat.reply_context import (
    carryforward_turn_analysis,
    checkpoint_context_message,
    event_message_for_ai,
    format_turn_analysis_guidance,
    messages_for_ai,
    payload_text,
    prompt_messages_for_ai,
    session_assistant_instructions,
    telegram_assistant_instructions,
)
from ait_chat.reply_http import json_request as _shared_json_request
from ait_chat.reply_http import response_output_text as _shared_response_output_text


class ReplyGenerationError(RuntimeError):
    pass


@dataclass(frozen=True)
class AiReplyResult:
    text: str
    model: str
    response_id: str | None = None
    usage: dict[str, Any] | None = None
    source: str = "openai"
    turn_analysis: dict[str, Any] | None = None
    attachments: tuple[dict[str, Any], ...] = ()


def _timeout_phrase(timeout: float | None) -> str:
    if timeout is None:
        return ""
    return f" after {timeout:g} seconds"


def _finalize_ai_reply_result(
    result: AiReplyResult,
    *,
    surface: str,
    repo_root: Path | None,
) -> AiReplyResult:
    normalized_surface = str(surface or "").strip().lower()
    if normalized_surface not in {"discord", "telegram"}:
        return result
    cleaned_text, attachments = extract_reply_attachments(result.text, repo_root=repo_root, surface=normalized_surface)
    if cleaned_text == result.text and attachments == tuple(result.attachments):
        return result
    return replace(result, text=cleaned_text, attachments=attachments or tuple(result.attachments))


def _json_request(
    url: str,
    *,
    method: str = "GET",
    payload: dict[str, Any] | None = None,
    headers: dict[str, str] | None = None,
    timeout: float | None = 20.0,
) -> Any:
    return _shared_json_request(
        url,
        method=method,
        payload=payload,
        headers=headers,
        timeout=timeout,
        urlopen_fn=urlopen,
        error_cls=ReplyGenerationError,
        timeout_phrase=_timeout_phrase,
    )


def response_output_text(payload: dict[str, Any]) -> str:
    return _shared_response_output_text(payload)


def generate_session_reply(
    config: ReplyGenerationConfig,
    *,
    session: dict[str, Any],
    events: list[dict[str, Any]],
    chat_id: str | int | None = None,
    chat_title: str | None = None,
    checkpoint: dict[str, Any] | None = None,
    surface: str = "telegram",
    actor_identity: str | None = None,
) -> AiReplyResult:
    messages = prompt_messages_for_ai(events, history_limit=config.history_limit, checkpoint=checkpoint)
    if not messages or (checkpoint is not None and len(messages) == 1):
        raise ReplyGenerationError("Cannot build AI context from the current session events.")
    normalized_surface = str(surface or "").strip() or "session"
    surface_title = str(chat_title or session.get("title") or session.get("session_id") or "").strip()
    context_id: str | int = chat_id if chat_id is not None else (str(session.get("session_id") or "").strip() or normalized_surface)
    assistant_instructions = "\n".join(
        [
            session_assistant_instructions(
                config,
                session,
                surface=normalized_surface,
                surface_title=surface_title,
            ),
            "Use the supplied shared-session transcript as prior context.",
            "When the user asks for concrete repository work, do the work directly when practical instead of only describing a plan.",
        ]
    )
    normalized_codex_primary_surfaces = {
        str(value).strip().lower() for value in (config.codex_primary_surfaces or ()) if str(value).strip()
    }
    codex_primary_for_surface = (
        "*" in normalized_codex_primary_surfaces or normalized_surface.lower() in normalized_codex_primary_surfaces
    )

    def _finalize(result: AiReplyResult) -> AiReplyResult:
        return _finalize_ai_reply_result(result, surface=normalized_surface, repo_root=config.repo_root)

    def _codex_reply() -> AiReplyResult:
        return _finalize(
            generate_codex_session_reply(
                config,
                session=session,
                messages=messages,
                chat_id=context_id,
                chat_title=surface_title,
                assistant_instructions=assistant_instructions,
                surface=normalized_surface,
                actor_identity=actor_identity,
            )
        )

    api_key = (config.openai_api_key or "").strip()
    if codex_primary_for_surface:
        try:
            return _codex_reply()
        except RuntimeError as exc:
            if not api_key:
                raise ReplyGenerationError(f"Codex websocket reply failed: {exc}") from exc
    if not api_key:
        try:
            return _codex_reply()
        except RuntimeError as exc:
            raise ReplyGenerationError(f"Codex websocket reply failed: {exc}") from exc
    payload: dict[str, Any] = {
        "model": config.openai_model,
        "store": False,
        "instructions": session_assistant_instructions(
            config,
            session,
            surface=normalized_surface,
            surface_title=surface_title,
        ),
        "input": messages,
        "text": {"format": {"type": "text"}},
        "metadata": {
            "source": "ait_server_telegram_chat" if normalized_surface == "telegram" else "ait_server_session_turn",
            "repo_name": config.repo_name,
            "session_id": str(session.get("session_id") or ""),
            "checkpoint_id": str((checkpoint or {}).get("checkpoint_id") or ""),
            "context_mode": "checkpoint_delta" if checkpoint is not None else "recent_tail",
        },
    }
    if normalized_surface == "telegram":
        payload["metadata"]["telegram_chat_id"] = str(context_id)
    else:
        payload["metadata"]["surface"] = normalized_surface
        if surface_title:
            payload["metadata"]["surface_title"] = surface_title
    if config.openai_reasoning_effort:
        payload["reasoning"] = {"effort": config.openai_reasoning_effort}
    if int(config.openai_max_output_tokens or 0) > 0:
        payload["max_output_tokens"] = int(config.openai_max_output_tokens)
    response = _json_request(
        f"{config.openai_base_url}/responses",
        method="POST",
        payload=payload,
        headers={"Authorization": f"Bearer {api_key}"},
        timeout=config.openai_timeout_seconds,
    )
    if not isinstance(response, dict):
        raise ReplyGenerationError("OpenAI returned an invalid response payload.")
    text = response_output_text(response)
    if not text:
        raise ReplyGenerationError("OpenAI returned an empty text response.")
    usage = response.get("usage") if isinstance(response.get("usage"), dict) else None
    response_id = str(response.get("id") or "").strip() or None
    model_name = str(response.get("model") or config.openai_model).strip() or config.openai_model
    return _finalize(AiReplyResult(text=text, model=model_name, response_id=response_id, usage=usage, source="openai"))
