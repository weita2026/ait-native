from __future__ import annotations

import json
from typing import Any, Mapping

from .workflow_notifications import _graph_watches


def _event_actor_display_name(payload: Mapping[str, Any]) -> str | None:
    direct = str(payload.get("actor_display_name") or "").strip()
    if direct:
        return direct
    envelope = payload.get("transport_envelope")
    if isinstance(envelope, Mapping):
        display_name = str(envelope.get("actor_display_name") or "").strip()
        if display_name:
            return display_name
        username = str(envelope.get("actor_username") or "").strip().lstrip("@")
        if username:
            return f"@{username}"
    return None


def _telegram_actor_username(actor_identity: str) -> str | None:
    text = str(actor_identity or "").strip()
    if not text.startswith("telegram:"):
        return None
    username = text.split(":@", 1)[1].strip().lstrip("@") if ":@" in text else ""
    if not username:
        return None
    return f"@{username}"


def _actor_label(event: dict[str, Any]) -> str:
    payload = event.get("payload") or {}
    label = _event_actor_display_name(payload)
    if label:
        return label
    actor = str(event.get("actor_identity") or "system").strip() or "system"
    return _telegram_actor_username(actor) or actor


def _payload_text(payload: dict[str, Any]) -> str:
    for key in ("text", "summary", "detail", "message"):
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    compact = json.dumps(payload, ensure_ascii=False, sort_keys=True)
    return compact if compact != "{}" else "(no details)"


def _payload_primary_text(payload: dict[str, Any]) -> str | None:
    for key in ("text", "summary", "detail", "message"):
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return None


def _format_session_event_block(event: dict[str, Any]) -> str:
    payload = event.get("payload") or {}
    event_type = str(event.get("event_type") or "").strip()
    primary_text = _payload_primary_text(payload)
    if primary_text and event_type in {"telegram.user_message", "session.message", "assistant.reply"}:
        return primary_text
    return f"[{event.get('sequence')}] {event_type} · {_actor_label(event)}\n{_payload_text(payload)}"


def _session_url(config: Any, session_id: str) -> str | None:
    if not getattr(config, "ait_web_url", None):
        return None
    return f"{config.ait_web_url}/sessions/{session_id}"


def _format_seconds(value: float) -> str:
    if float(value).is_integer():
        return str(int(value))
    return f"{value:g}"


def _sync_mode_label(config: Any) -> str:
    if not getattr(config, "background_sync_enabled", False):
        return "manual_only"
    return f"background+manual ({_format_seconds(config.background_sync_interval_seconds)}s interval)"


def _notification_mode_label(config: Any, link: dict[str, Any]) -> str:
    if not bool(link.get("workflow_notifications_enabled")):
        return "off"
    if getattr(config, "background_sync_enabled", False):
        return "on"
    return "armed_waiting_for_background_sync"


def _session_runtime_lines(session: dict[str, Any] | None) -> list[str]:
    runtime = (session or {}).get("telegram_context_runtime") or {}
    if not isinstance(runtime, dict) or not runtime:
        return []
    threshold = max(int(runtime.get("checkpoint_event_threshold") or 0), 0)
    delta_event_count = max(int(runtime.get("delta_event_count") or 0), 0)
    threshold_text = str(threshold) if threshold > 0 else "?"
    lines = [
        f"reply_context_mode={runtime.get('reply_context_mode') or 'recent_tail'}",
        f"checkpoint_freshness={runtime.get('checkpoint_freshness') or 'unknown'}",
        f"checkpoint_delta_events={delta_event_count}/{threshold_text}",
    ]
    checkpoint_id = str(runtime.get("checkpoint_id") or "").strip()
    if checkpoint_id:
        lines.append(f"checkpoint_id={checkpoint_id}")
    checkpoint_sequence = int(runtime.get("checkpoint_based_on_sequence") or 0)
    if checkpoint_id or checkpoint_sequence > 0:
        lines.append(f"checkpoint_sequence={checkpoint_sequence}")
    return lines


def format_session_status(config: Any, link: dict[str, Any], session: dict[str, Any] | None = None) -> str:
    session_id = str(link.get("session_id") or "not linked")
    canonical_session_id = str(link.get("canonical_session_id") or session_id).strip() or session_id
    branch_session_id = str(link.get("branch_session_id") or "").strip()
    relink_reason = str(link.get("relink_reason") or "").strip()
    runtime_link_status = "active" if session is not None else ("relink_required" if relink_reason else "unavailable")
    runtime_backend_mode = str(link.get("runtime_backend_mode") or getattr(config, "runtime_mode", "") or "").strip() or "unknown"
    runtime_backend_remote_name = str(
        link.get("runtime_backend_remote_name") or getattr(config, "runtime_remote_name", "") or ""
    ).strip()
    lines = [
        "ait Telegram status",
        f"repo={link.get('repo_name') or getattr(config, 'repo_name', '')}",
        f"session={session_id}",
        f"canonical_session={canonical_session_id}",
        f"chat={link.get('chat_title') or link.get('chat_id')}",
        f"runtime_link={runtime_link_status}",
        f"runtime_backend={runtime_backend_mode}",
        f"binding_role={link.get('binding_role') or ('branch' if branch_session_id else 'primary_shared')}",
        f"sync_mode={_sync_mode_label(config)}",
        f"workflow_notifications={_notification_mode_label(config, link)}",
        f"graph_watches={len(_graph_watches(link))}",
        f"last_synced_sequence={int(link.get('last_synced_sequence') or 0)}",
    ]
    if runtime_backend_remote_name:
        lines.append(f"runtime_remote={runtime_backend_remote_name}")
    if branch_session_id:
        lines.append(f"branch_session={branch_session_id}")
    if relink_reason:
        lines.append(f"relink_reason={relink_reason}")
    last_sync_at = str(link.get("last_sync_at") or "").strip()
    if last_sync_at:
        lines.append(f"last_sync_at={last_sync_at}")
    last_queue_notification_at = str(link.get("last_queue_notification_at") or "").strip()
    if last_queue_notification_at:
        lines.append(f"last_queue_notification_at={last_queue_notification_at}")
    skipped_sequence = int(link.get("last_relink_skipped_reply_sequence") or 0)
    if skipped_sequence > 0:
        lines.append(f"last_relink_skipped_reply_sequence={skipped_sequence}")
    skipped_at = str(link.get("last_relink_skipped_reply_at") or "").strip()
    if skipped_at:
        lines.append(f"last_relink_skipped_reply_at={skipped_at}")
    skipped_from_session_id = str(link.get("last_relink_skipped_from_session_id") or "").strip()
    if skipped_from_session_id:
        lines.append(f"last_relink_skipped_from_session_id={skipped_from_session_id}")
    lines.extend(_session_runtime_lines(session))
    url = _session_url(config, session_id)
    if url:
        lines.append(url)
    return "\n".join(lines)


def format_session_events(events: list[dict[str, Any]]) -> str:
    if not events:
        return "No new session events."
    blocks = [_format_session_event_block(event) for event in events]
    return "\n\n".join(blocks)
