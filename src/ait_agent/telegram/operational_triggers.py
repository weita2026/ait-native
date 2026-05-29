from __future__ import annotations

import json
import os
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Mapping

from ait_agent.envelope import build_transport_reply_envelope

from .event_triggers import EventTriggerRegistry, parse_telegram_operational_trigger


def _clean_optional_str(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _positive_int(value: object) -> int | None:
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    return parsed if parsed > 0 else None


def _attachment_list(values: tuple[dict[str, Any], ...] | list[dict[str, Any]] | None) -> list[dict[str, Any]]:
    if not values:
        return []
    return [dict(item) for item in values if isinstance(item, Mapping)]


def _handler_pythonpath(repo_root: Path) -> str:
    src_candidates = [str(Path(__file__).resolve().parents[2])]
    repo_src_path = repo_root / "src"
    if repo_src_path.exists():
        repo_src = str(repo_src_path)
        if repo_src not in src_candidates:
            src_candidates.append(repo_src)
    existing = str(os.environ.get("PYTHONPATH") or "").strip()
    combined = ":".join(src_candidates)
    return combined if not existing else f"{combined}:{existing}"


def _normalized_reply_parts(payload: Mapping[str, Any]) -> tuple[bool, str, list[dict[str, Any]]]:
    reply_payload = payload.get("reply") if isinstance(payload.get("reply"), Mapping) else {}
    top_level_text = _clean_optional_str(payload.get("reply_text") or payload.get("text")) or ""
    nested_text = _clean_optional_str(reply_payload.get("text")) or ""
    top_level_attachments = payload.get("attachments")
    nested_attachments = reply_payload.get("attachments")
    attachments = _attachment_list(nested_attachments if isinstance(nested_attachments, (list, tuple)) else top_level_attachments)
    explicit_handled = payload.get("handled")
    if isinstance(explicit_handled, bool):
        handled = explicit_handled
    else:
        handled = bool(top_level_text or nested_text or attachments)
    return handled, nested_text or top_level_text, attachments


@dataclass(frozen=True)
class TelegramOperationalTriggerMessageContext:
    raw_text: str
    normalized_text: str
    command: tuple[str, str] | None
    telegram_message_id: int | None
    telegram_message_ids: tuple[int, ...] = ()
    reply_to_message: Mapping[str, Any] | None = None
    attachments: tuple[dict[str, Any], ...] = ()
    actor_identity: str | None = None
    message: Mapping[str, Any] | None = None


class TelegramOperationalTriggerDispatcher:
    def __init__(
        self,
        *,
        config: Any,
        repo_root: Path,
        event_trigger_registry: EventTriggerRegistry,
        state_get_chat: Callable[[str | int], dict[str, Any] | None],
        send_assistant_event_reply: Callable[[str | int, Mapping[str, Any]], None],
        safe_send_message: Callable[[str | int, str], None],
        log_runtime_error: Callable[[str, Exception], None],
        runtime_error_type: type[Exception],
    ) -> None:
        self._config = config
        self._repo_root = repo_root
        self._event_trigger_registry = event_trigger_registry
        self._state_get_chat = state_get_chat
        self._send_assistant_event_reply = send_assistant_event_reply
        self._safe_send_message = safe_send_message
        self._log_runtime_error = log_runtime_error
        self._runtime_error_type = runtime_error_type

    def maybe_handle(
        self,
        *,
        chat_id: str | int,
        chat: Mapping[str, Any],
        from_user: Mapping[str, Any],
        chat_title: str,
        context: TelegramOperationalTriggerMessageContext,
    ) -> bool:
        reply_to_message_id = _positive_int((context.reply_to_message or {}).get("message_id"))
        for trigger in self._event_trigger_registry.telegram_operational:
            match_payload = parse_telegram_operational_trigger(
                raw_text=context.raw_text,
                normalized_text=context.normalized_text,
                command=context.command,
                reply_to_message_id=reply_to_message_id,
                config=trigger,
            )
            if match_payload is None:
                continue
            try:
                payload = self._invoke_handler(
                    trigger=trigger,
                    match_payload=match_payload,
                    chat_id=chat_id,
                    chat=chat,
                    from_user=from_user,
                    chat_title=chat_title,
                    context=context,
                    reply_to_message_id=reply_to_message_id,
                )
            except Exception as exc:
                self._log_runtime_error("Telegram operational trigger dispatch failed.", exc)
                self._safe_send_message(chat_id, f"ait Telegram operational trigger failed: {exc}")
                return True
            handled, reply_text, attachments = _normalized_reply_parts(payload)
            if not handled:
                continue
            if reply_text or attachments:
                envelope = build_transport_reply_envelope(
                    transport="telegram",
                    channel_id=chat_id,
                    channel_title=chat_title,
                    channel_kind=str(chat.get("type") or "").strip() or None,
                    text=reply_text,
                    reply_to_message_id=context.telegram_message_id,
                    reply_to_message_ids=context.telegram_message_ids,
                    attachments=attachments,
                    metadata={
                        "delivered_via": "telegram_operational_trigger",
                        "trigger_id": trigger.trigger_id,
                        "trigger_source_path": trigger.source_path,
                    },
                )
                self._send_assistant_event_reply(
                    chat_id,
                    {
                        "event_type": "assistant.reply",
                        "payload": {
                            "text": reply_text,
                            "transport_reply_envelope": envelope,
                        },
                    },
                )
            return True
        return False

    def _invoke_handler(
        self,
        *,
        trigger: Any,
        match_payload: Mapping[str, Any],
        chat_id: str | int,
        chat: Mapping[str, Any],
        from_user: Mapping[str, Any],
        chat_title: str,
        context: TelegramOperationalTriggerMessageContext,
        reply_to_message_id: int | None,
    ) -> dict[str, Any]:
        payload = {
            "schema_version": 1,
            "transport": "telegram",
            "repo_name": str(self._config.repo_name or "").strip() or None,
            "repo_root": str(self._repo_root),
            "trigger": {
                "id": trigger.trigger_id,
                "display_trigger": trigger.display_trigger,
                "source_path": trigger.source_path,
                "match": dict(match_payload),
            },
            "chat": {
                "chat_id": str(chat_id),
                "chat_title": chat_title,
                "chat_type": str(chat.get("type") or "").strip() or None,
                "payload": dict(chat),
            },
            "actor": {
                "actor_identity": _clean_optional_str(context.actor_identity),
                "from_user": dict(from_user),
            },
            "message": {
                "raw_text": context.raw_text,
                "normalized_text": context.normalized_text,
                "command_name": context.command[0] if context.command else None,
                "command_args": context.command[1] if context.command else None,
                "telegram_message_id": context.telegram_message_id,
                "telegram_message_ids": list(context.telegram_message_ids),
                "reply_to_message_id": reply_to_message_id,
                "reply_to_message": dict(context.reply_to_message or {}),
                "attachments": _attachment_list(context.attachments),
                "payload": dict(context.message or {}),
            },
            "session_link": self._state_get_chat(chat_id),
        }
        env = dict(os.environ)
        env["AIT_REPO_ROOT"] = str(self._repo_root)
        env["PYTHONPATH"] = _handler_pythonpath(self._repo_root)
        completed = subprocess.run(
            list(trigger.handler_command),
            cwd=self._repo_root,
            env=env,
            input=json.dumps(payload, ensure_ascii=True),
            capture_output=True,
            text=True,
            check=False,
        )
        if completed.returncode != 0:
            detail = completed.stderr.strip() or completed.stdout.strip() or f"exit code {completed.returncode}"
            raise self._runtime_error_type(detail)
        try:
            response = json.loads(completed.stdout or "{}")
        except json.JSONDecodeError as exc:
            raise self._runtime_error_type("Operational trigger handler returned invalid JSON.") from exc
        if not isinstance(response, dict):
            raise self._runtime_error_type("Operational trigger handler must return a JSON object.")
        return response


__all__ = [
    "TelegramOperationalTriggerDispatcher",
    "TelegramOperationalTriggerMessageContext",
]
