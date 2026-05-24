from __future__ import annotations

from typing import Any, Mapping, Sequence


def _clean_optional_str(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _normalize_positive_int(value: object) -> int | None:
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    return parsed if parsed > 0 else None


def _normalize_message_ids(values: Sequence[object] | None) -> list[int]:
    if not values:
        return []
    normalized: list[int] = []
    seen: set[int] = set()
    for value in values:
        parsed = _normalize_positive_int(value)
        if parsed is None or parsed in seen:
            continue
        seen.add(parsed)
        normalized.append(parsed)
    return normalized


def _compact_dict(payload: Mapping[str, Any]) -> dict[str, Any]:
    compact: dict[str, Any] = {}
    for key, value in payload.items():
        if value is None:
            continue
        if isinstance(value, Mapping):
            nested = _compact_dict(value)
            if not nested:
                continue
            compact[key] = nested
            continue
        if isinstance(value, list) and not value:
            continue
        compact[key] = value
    return compact


def _normalize_transport_attachment(value: object) -> dict[str, Any] | None:
    if not isinstance(value, Mapping):
        return None
    payload = {
        "kind": _clean_optional_str(value.get("kind")),
        "media_kind": _clean_optional_str(value.get("media_kind")),
        "telegram_file_id": _clean_optional_str(value.get("telegram_file_id") or value.get("file_id")),
        "telegram_file_unique_id": _clean_optional_str(
            value.get("telegram_file_unique_id") or value.get("file_unique_id")
        ),
        "file_name": _clean_optional_str(value.get("file_name")),
        "mime_type": _clean_optional_str(value.get("mime_type")),
        "caption": _clean_optional_str(value.get("caption")),
        "title": _clean_optional_str(value.get("title")),
        "performer": _clean_optional_str(value.get("performer")),
        "duration_seconds": _normalize_positive_int(value.get("duration_seconds") or value.get("duration")),
        "file_size_bytes": _normalize_positive_int(value.get("file_size_bytes") or value.get("file_size")),
        "telegram_file_path": _clean_optional_str(value.get("telegram_file_path")),
        "local_path": _clean_optional_str(value.get("local_path") or value.get("path")),
        "url": _clean_optional_str(value.get("url")),
    }
    compact = _compact_dict(payload)
    if not compact:
        return None
    kind = str(compact.get("kind") or compact.get("media_kind") or "file").strip().lower()
    compact["kind"] = kind
    return compact


def _normalize_transport_attachments(values: Sequence[object] | None) -> list[dict[str, Any]]:
    if not values:
        return []
    normalized: list[dict[str, Any]] = []
    for value in values:
        attachment = _normalize_transport_attachment(value)
        if attachment is not None:
            normalized.append(attachment)
    return normalized


def _message_label(message_ids: Sequence[int], fallback: str) -> str:
    if message_ids:
        return "-".join(str(value) for value in message_ids)
    return fallback


def build_transport_session_metadata(
    *,
    transport: str,
    channel_id: str | int,
    channel_title: str | None = None,
    channel_kind: str | None = None,
    thread_id: str | int | None = None,
    linked_via: str | None = None,
    metadata_extra: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "source": _clean_optional_str(transport) or "unknown",
        "transport": _clean_optional_str(transport) or "unknown",
        "transport_channel_id": str(channel_id),
        "transport_channel_title": _clean_optional_str(channel_title),
        "transport_channel_kind": _clean_optional_str(channel_kind),
        "transport_thread_id": _clean_optional_str(thread_id),
        "linked_via": _clean_optional_str(linked_via) or f"ait-agent {transport}",
    }
    if metadata_extra:
        payload.update(dict(metadata_extra))
    return _compact_dict(payload)


def build_transport_binding_metadata(
    *,
    transport: str,
    surface_id: str | int,
    surface_title: str | None = None,
    surface_kind: str | None = None,
    thread_id: str | int | None = None,
    binding_role: str | None = None,
    canonical_session_id: str | None = None,
    active_session_id: str | None = None,
    branch_session_id: str | None = None,
    branch_kind: str | None = None,
    relink_reason: str | None = None,
    reply_target: Mapping[str, Any] | None = None,
    metadata_extra: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    resolved_branch_session_id = _clean_optional_str(branch_session_id)
    resolved_canonical_session_id = _clean_optional_str(canonical_session_id) or _clean_optional_str(active_session_id)
    resolved_active_session_id = (
        _clean_optional_str(active_session_id)
        or resolved_branch_session_id
        or resolved_canonical_session_id
    )
    payload: dict[str, Any] = {
        "shared_session_transport": _clean_optional_str(transport) or "unknown",
        "shared_session_surface_id": str(surface_id),
        "shared_session_surface_title": _clean_optional_str(surface_title),
        "shared_session_surface_kind": _clean_optional_str(surface_kind),
        "shared_session_thread_id": _clean_optional_str(thread_id),
        "shared_session_binding_role": _clean_optional_str(binding_role),
        "shared_session_canonical_session_id": resolved_canonical_session_id,
        "shared_session_active_session_id": resolved_active_session_id,
        "shared_session_branch_session_id": resolved_branch_session_id,
        "shared_session_branch_kind": _clean_optional_str(branch_kind),
        "shared_session_relink_reason": _clean_optional_str(relink_reason),
    }
    compact_reply_target = _compact_dict(dict(reply_target or {}))
    if compact_reply_target:
        payload["shared_session_transport_reply_target"] = compact_reply_target
    if metadata_extra:
        payload.update(dict(metadata_extra))
    return _compact_dict(payload)


def build_transport_event_envelope(
    *,
    transport: str,
    actor_identity: str,
    channel_id: str | int,
    text: str,
    actor_transport_id: str | int | None = None,
    actor_username: str | None = None,
    actor_display_name: str | None = None,
    actor_is_bot: bool | None = None,
    channel_title: str | None = None,
    channel_kind: str | None = None,
    thread_id: str | int | None = None,
    message_id: int | None = None,
    message_ids: Sequence[object] | None = None,
    occurred_at: str | None = None,
    event_id: str | None = None,
    dedupe_key: str | None = None,
    attachments: Sequence[object] | None = None,
    metadata: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    normalized_transport = _clean_optional_str(transport) or "unknown"
    normalized_message_id = _normalize_positive_int(message_id)
    normalized_message_ids = _normalize_message_ids(message_ids)
    if normalized_message_id is not None and normalized_message_id not in normalized_message_ids:
        normalized_message_ids.append(normalized_message_id)
    primary_message_id = normalized_message_id if normalized_message_id is not None else (
        normalized_message_ids[-1] if normalized_message_ids else None
    )
    normalized_attachments = _normalize_transport_attachments(attachments)
    label = _message_label(normalized_message_ids, "event")
    channel_key = str(channel_id)
    payload = {
        "schema_version": 1,
        "transport": normalized_transport,
        "event_kind": "message",
        "event_id": _clean_optional_str(event_id) or f"{normalized_transport}:{channel_key}:message:{label}",
        "dedupe_key": _clean_optional_str(dedupe_key) or f"{normalized_transport}:{channel_key}:message:{label}",
        "occurred_at": _clean_optional_str(occurred_at),
        "actor": {
            "actor_identity": _clean_optional_str(actor_identity),
            "transport_user_id": _clean_optional_str(actor_transport_id),
            "username": _clean_optional_str(actor_username),
            "display_name": _clean_optional_str(actor_display_name),
            "is_bot": actor_is_bot,
        },
        "channel": {
            "channel_id": channel_key,
            "channel_title": _clean_optional_str(channel_title),
            "channel_kind": _clean_optional_str(channel_kind),
            "thread_id": _clean_optional_str(thread_id),
        },
        "message": {
            "text": str(text),
            "message_id": primary_message_id,
            "message_ids": normalized_message_ids,
            "logical_turn_message_count": len(normalized_message_ids) or 1,
            "attachments": normalized_attachments,
        },
        "metadata": dict(metadata or {}),
    }
    return _compact_dict(payload)


def build_transport_reply_envelope(
    *,
    transport: str,
    channel_id: str | int,
    text: str,
    channel_title: str | None = None,
    channel_kind: str | None = None,
    thread_id: str | int | None = None,
    delivery_kind: str = "chat_reply",
    reply_to_event_id: str | None = None,
    reply_to_message_id: int | None = None,
    reply_to_message_ids: Sequence[object] | None = None,
    attachments: Sequence[object] | None = None,
    metadata: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    normalized_transport = _clean_optional_str(transport) or "unknown"
    normalized_reply_to_message_id = _normalize_positive_int(reply_to_message_id)
    normalized_reply_to_message_ids = _normalize_message_ids(reply_to_message_ids)
    if (
        normalized_reply_to_message_id is not None
        and normalized_reply_to_message_id not in normalized_reply_to_message_ids
    ):
        normalized_reply_to_message_ids.append(normalized_reply_to_message_id)
    normalized_attachments = _normalize_transport_attachments(attachments)
    payload = {
        "schema_version": 1,
        "transport": normalized_transport,
        "delivery_kind": _clean_optional_str(delivery_kind) or "chat_reply",
        "target": {
            "channel_id": str(channel_id),
            "channel_title": _clean_optional_str(channel_title),
            "channel_kind": _clean_optional_str(channel_kind),
            "thread_id": _clean_optional_str(thread_id),
        },
        "reply_to": {
            "event_id": _clean_optional_str(reply_to_event_id),
            "message_id": normalized_reply_to_message_id,
            "message_ids": normalized_reply_to_message_ids,
            "logical_turn_message_count": len(normalized_reply_to_message_ids) or None,
        },
        "message": {
            "text": str(text),
            "attachments": normalized_attachments,
        },
        "metadata": dict(metadata or {}),
    }
    return _compact_dict(payload)
