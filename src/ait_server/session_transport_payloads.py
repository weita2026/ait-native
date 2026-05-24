from __future__ import annotations

from collections.abc import Callable, Mapping
from typing import Any

BuildTransportReplyEnvelope = Callable[..., dict[str, Any]]
UtcNowIso = Callable[[], str]


def build_session_user_message_payload(
    *,
    surface: str,
    title: str,
    text: str,
    actor_display_name: str | None,
    transport_envelope: Mapping[str, Any] | None,
    workflow_context: Mapping[str, Any] | None,
    utc_now_iso: UtcNowIso,
) -> dict[str, Any]:
    return {
        "source": surface,
        "surface_title": title,
        "text": text,
        "ingested_at": utc_now_iso(),
        **({"actor_display_name": actor_display_name} if actor_display_name else {}),
        **({"transport_envelope": dict(transport_envelope)} if transport_envelope else {}),
        **({"workflow_context": dict(workflow_context)} if workflow_context else {}),
    }


def build_session_assistant_reply_payload(
    *,
    reply: Any,
    assistant_text: str,
    surface: str,
    title: str,
    session_id: str,
    user_sequence: int,
    transport_envelope: Mapping[str, Any] | None,
    task_dag_progress: Mapping[str, Any] | None,
    build_transport_reply_envelope: BuildTransportReplyEnvelope,
    utc_now_iso: UtcNowIso,
) -> dict[str, Any]:
    delivered_via = "session_live"
    transport_reply_envelope = None
    if transport_envelope:
        transport_name = str(transport_envelope.get("transport") or surface or "transport").strip() or "transport"
        channel = transport_envelope.get("channel") if isinstance(transport_envelope.get("channel"), dict) else {}
        message = transport_envelope.get("message") if isinstance(transport_envelope.get("message"), dict) else {}
        delivered_via = f"{transport_name}_live"
        transport_reply_envelope = build_transport_reply_envelope(
            transport=transport_name,
            channel_id=channel.get("channel_id") or session_id,
            channel_title=channel.get("channel_title"),
            channel_kind=channel.get("channel_kind"),
            thread_id=channel.get("thread_id"),
            text=assistant_text,
            attachments=getattr(reply, "attachments", ()),
            reply_to_event_id=str(transport_envelope.get("event_id") or "").strip() or None,
            reply_to_message_id=message.get("message_id"),
            reply_to_message_ids=message.get("message_ids"),
            metadata={"delivered_via": delivered_via},
        )
    payload = {
        "source": reply.source,
        "generated_via": "ait_server",
        "text": assistant_text,
        "turn_analysis": reply.turn_analysis or {},
        "model": reply.model,
        "response_id": reply.response_id,
        "usage": reply.usage or {},
        "reply_to_sequence": user_sequence,
        "delivered_via": delivered_via,
        "session_surface": surface,
        "surface_title": title,
        "generated_at": utc_now_iso(),
    }
    if transport_reply_envelope is not None:
        payload["transport_reply_envelope"] = transport_reply_envelope
    if task_dag_progress is not None:
        payload["task_dag_progress"] = dict(task_dag_progress)
    return payload


def build_telegram_user_message_payload(
    *,
    text: str,
    chat_id: str | int,
    chat_title: str | None,
    chat_type: str | None,
    telegram_message_id: int | None,
    telegram_message_ids: list[int],
    transport_envelope: Mapping[str, Any] | None,
    workflow_context: Mapping[str, Any] | None,
    utc_now_iso: UtcNowIso,
) -> dict[str, Any]:
    return {
        "source": "telegram",
        "text": text,
        "telegram_chat_id": str(chat_id),
        "telegram_chat_title": chat_title,
        "telegram_chat_type": chat_type,
        "telegram_message_id": telegram_message_id,
        "telegram_message_ids": telegram_message_ids,
        "logical_turn_message_count": len(telegram_message_ids),
        "ingested_at": utc_now_iso(),
        **({"transport_envelope": dict(transport_envelope)} if transport_envelope else {}),
        **({"workflow_context": dict(workflow_context)} if workflow_context else {}),
    }


def build_telegram_assistant_reply_payload(
    *,
    reply: Any,
    assistant_text: str,
    chat_id: str | int,
    chat_title: str | None,
    chat_type: str | None,
    telegram_message_id: int | None,
    telegram_message_ids: list[int],
    transport_envelope: Mapping[str, Any] | None,
    user_sequence: int,
    task_dag_progress: Mapping[str, Any] | None,
    build_transport_reply_envelope: BuildTransportReplyEnvelope,
    utc_now_iso: UtcNowIso,
) -> dict[str, Any]:
    payload = {
        "source": reply.source,
        "generated_via": "ait_server",
        "text": assistant_text,
        "turn_analysis": reply.turn_analysis or {},
        "model": reply.model,
        "response_id": reply.response_id,
        "usage": reply.usage or {},
        "telegram_chat_id": str(chat_id),
        "telegram_chat_title": chat_title,
        "reply_to_sequence": user_sequence,
        "delivered_via": "telegram_live",
        "generated_at": utc_now_iso(),
        "transport_reply_envelope": build_transport_reply_envelope(
            transport="telegram",
            channel_id=chat_id,
            channel_title=chat_title,
            channel_kind=chat_type,
            text=assistant_text,
            reply_to_event_id=(str(transport_envelope.get("event_id") or "").strip() if transport_envelope else None),
            reply_to_message_id=telegram_message_id,
            reply_to_message_ids=telegram_message_ids,
            metadata={"delivered_via": "telegram_live"},
        ),
    }
    if task_dag_progress is not None:
        payload["task_dag_progress"] = dict(task_dag_progress)
    return payload


__all__ = [
    "build_session_assistant_reply_payload",
    "build_session_user_message_payload",
    "build_telegram_assistant_reply_payload",
    "build_telegram_user_message_payload",
]
