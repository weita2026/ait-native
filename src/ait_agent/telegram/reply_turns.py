from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Callable, Mapping

from ait_agent.envelope import build_transport_event_envelope

from .session_views import _session_url
from .turn_inputs import (
    _attachment_should_send_as_audio,
    _transport_reply_attachments,
    _transport_reply_text,
)


RETRYABLE_DEFERRED_REPLY_ERROR_FAMILY_MARKERS = (
    "reconnecting...",
    "unexpected status 503 service unavailable",
    "disconnect/reset before headers",
    "connection timeout",
    "timeout waiting for child process to exit",
    "codex app-server connection closed",
)


def _retryable_deferred_reply_error_text(text: str) -> bool:
    lowered = str(text or "").strip().lower()
    if not lowered:
        return False
    if "codex websocket reply failed:" not in lowered and "codex app-server" not in lowered:
        return False
    return any(marker in lowered for marker in RETRYABLE_DEFERRED_REPLY_ERROR_FAMILY_MARKERS)


@dataclass(frozen=True)
class PendingTelegramReplyTurn:
    session_id: str
    chat_id: str | int
    chat_type: str | None
    chat_title: str
    actor_identity: str
    text: str
    telegram_message_id: int | None
    telegram_message_ids: tuple[int, ...] = ()
    transport_envelope: dict[str, Any] | None = None
    runtime_snapshot: Any | None = None


class TelegramReplyTurnSpool:
    def __init__(
        self,
        *,
        state_get_chat: Callable[[str | int], dict[str, Any] | None],
        state_patch_chat: Callable[..., dict[str, Any] | None],
        now_iso: Callable[[], str],
        spool_limit: int,
    ) -> None:
        self._state_get_chat = state_get_chat
        self._state_patch_chat = state_patch_chat
        self._now_iso = now_iso
        self._spool_limit = spool_limit

    def pending_turn_spool_key(self, pending_turn: PendingTelegramReplyTurn) -> str:
        if isinstance(pending_turn.transport_envelope, Mapping):
            event_id = str(pending_turn.transport_envelope.get("event_id") or "").strip()
            if event_id:
                return event_id
        message_ids: list[str] = []
        for value in [pending_turn.telegram_message_id, *list(pending_turn.telegram_message_ids or ())]:
            try:
                message_id = int(value)
            except (TypeError, ValueError):
                continue
            if message_id > 0:
                message_ids.append(str(message_id))
        if message_ids:
            return f"telegram:{pending_turn.chat_id}:messages:{','.join(message_ids)}"
        return f"telegram:{pending_turn.chat_id}:session:{pending_turn.session_id}:text:{pending_turn.text.strip()}"

    def telegram_reply_spool_entries(self, link: Mapping[str, Any] | None) -> list[dict[str, Any]]:
        values = (link or {}).get("telegram_reply_spool")
        if not isinstance(values, list):
            return []
        return [dict(item) for item in values if isinstance(item, Mapping)]

    def remember_pending_reply_turn(
        self,
        pending_turn: PendingTelegramReplyTurn,
        *,
        status: str,
        attempt_increment: bool = False,
        last_error: str | None = None,
        user_event: Mapping[str, Any] | None = None,
        assistant_event: Mapping[str, Any] | None = None,
    ) -> dict[str, Any] | None:
        current_link = self._state_get_chat(pending_turn.chat_id)
        if not current_link or str(current_link.get("session_id") or "").strip() != pending_turn.session_id:
            return None
        spool_key = self.pending_turn_spool_key(pending_turn)
        existing_entries = self.telegram_reply_spool_entries(current_link)
        existing = next((entry for entry in existing_entries if str(entry.get("spool_key") or "").strip() == spool_key), {})
        message_ids: list[int] = []
        for value in [pending_turn.telegram_message_id, *list(pending_turn.telegram_message_ids or ())]:
            try:
                message_id = int(value)
            except (TypeError, ValueError):
                continue
            if message_id > 0:
                message_ids.append(message_id)
        entry = {
            "spool_key": spool_key,
            "status": status,
            "session_id": pending_turn.session_id,
            "chat_id": str(pending_turn.chat_id),
            "chat_title": pending_turn.chat_title,
            "chat_type": pending_turn.chat_type,
            "text": pending_turn.text,
            "actor_identity": pending_turn.actor_identity,
            "transport_event_id": (
                str(pending_turn.transport_envelope.get("event_id") or "").strip()
                if isinstance(pending_turn.transport_envelope, Mapping)
                else None
            ),
            "telegram_message_id": pending_turn.telegram_message_id,
            "telegram_message_ids": message_ids,
            "queued_at": str(existing.get("queued_at") or self._now_iso()),
            "last_attempt_at": (
                self._now_iso()
                if attempt_increment or status in {"attempting", "failed"}
                else existing.get("last_attempt_at")
            ),
            "attempt_count": max(int(existing.get("attempt_count") or 0) + (1 if attempt_increment else 0), 0),
            "last_error": str(last_error).strip() if last_error else None,
            "last_user_sequence": int(user_event.get("sequence") or 0) if isinstance(user_event, Mapping) else None,
            "last_assistant_sequence": int(assistant_event.get("sequence") or 0)
            if isinstance(assistant_event, Mapping)
            else None,
        }
        next_entries = [item for item in existing_entries if str(item.get("spool_key") or "").strip() != spool_key]
        next_entries.append(entry)
        return self._state_patch_chat(
            pending_turn.chat_id,
            telegram_reply_spool=next_entries[-self._spool_limit :],
        )

    def clear_pending_reply_turn_spool_entry(self, pending_turn: PendingTelegramReplyTurn) -> dict[str, Any] | None:
        current_link = self._state_get_chat(pending_turn.chat_id)
        if not current_link:
            return None
        spool_key = self.pending_turn_spool_key(pending_turn)
        existing_entries = self.telegram_reply_spool_entries(current_link)
        next_entries = [item for item in existing_entries if str(item.get("spool_key") or "").strip() != spool_key]
        return self._state_patch_chat(
            pending_turn.chat_id,
            telegram_reply_spool=next_entries,
        )


class TelegramReplyTurnManager:
    def __init__(
        self,
        *,
        config: Any,
        telegram_api: Any,
        runtime_snapshot: Callable[[], Any],
        handle_owner_bootstrap_gate: Callable[..., bool],
        match_fresh_topic_event_trigger: Callable[[str], dict[str, Any] | None],
        create_fresh_session: Callable[..., dict[str, Any]],
        fresh_topic_confirmation_text: Callable[[Mapping[str, Any], Mapping[str, Any]], str],
        state_get_chat: Callable[[str | int], dict[str, Any] | None],
        state_patch_chat: Callable[..., dict[str, Any] | None],
        ensure_session_link: Callable[..., dict[str, Any] | None],
        submit_reply_serialized: Callable[..., Any],
        dispatch_key_for_chat: Callable[[str | int], str],
        ait_api_call: Callable[..., Any],
        advance_sync_cursor: Callable[..., dict[str, Any]],
        mark_telegram_live_reply_delivered: Callable[..., dict[str, Any]],
        remember_pending_reply_turn: Callable[..., dict[str, Any] | None],
        clear_pending_reply_turn_spool_entry: Callable[[PendingTelegramReplyTurn], dict[str, Any] | None],
        recover_or_watch_completed_pending_reply: Callable[[PendingTelegramReplyTurn, BaseException], bool],
        actor_identity: Callable[[dict[str, Any], str | int], str],
        user_display_name: Callable[[dict[str, Any]], str],
        runtime_error_type: type[Exception],
        model_capacity_error_text: Callable[[str], bool],
        log_runtime_error: Callable[[str, Exception], None],
        safe_send_message: Callable[[str | int, str], None],
        now_iso: Callable[[], str],
    ) -> None:
        self._config = config
        self._telegram_api = telegram_api
        self._runtime_snapshot = runtime_snapshot
        self._handle_owner_bootstrap_gate = handle_owner_bootstrap_gate
        self._match_fresh_topic_event_trigger = match_fresh_topic_event_trigger
        self._create_fresh_session = create_fresh_session
        self._fresh_topic_confirmation_text = fresh_topic_confirmation_text
        self._state_get_chat = state_get_chat
        self._state_patch_chat = state_patch_chat
        self._ensure_session_link = ensure_session_link
        self._submit_reply_serialized = submit_reply_serialized
        self._dispatch_key_for_chat = dispatch_key_for_chat
        self._ait_api_call = ait_api_call
        self._advance_sync_cursor = advance_sync_cursor
        self._mark_telegram_live_reply_delivered = mark_telegram_live_reply_delivered
        self._remember_pending_reply_turn = remember_pending_reply_turn
        self._clear_pending_reply_turn_spool_entry = clear_pending_reply_turn_spool_entry
        self._recover_or_watch_completed_pending_reply = recover_or_watch_completed_pending_reply
        self._actor_identity = actor_identity
        self._user_display_name = user_display_name
        self._runtime_error_type = runtime_error_type
        self._model_capacity_error_text = model_capacity_error_text
        self._log_runtime_error = log_runtime_error
        self._safe_send_message = safe_send_message
        self._now_iso = now_iso

    def handle_normal_text_turn(
        self,
        chat_id: str | int,
        chat: dict[str, Any],
        from_user: dict[str, Any],
        chat_title: str,
        normalized_text: str,
        *,
        telegram_message_id: int | None,
        telegram_message_ids: tuple[int, ...] = (),
        message_attachments: tuple[dict[str, Any], ...] = (),
        actor_identity: str | None = None,
        defer_reply: bool = False,
    ) -> None:
        runtime_snapshot = self._runtime_snapshot()
        if self._handle_owner_bootstrap_gate(
            chat_id,
            chat,
            from_user,
            chat_title,
            raw_text=normalized_text,
            command=None,
            attachments_present=bool(message_attachments),
        ):
            return
        fresh_topic_trigger = self._match_fresh_topic_event_trigger(normalized_text)
        if fresh_topic_trigger is not None:
            link = self._create_fresh_session(
                chat_id,
                chat,
                chat_title,
                runtime_snapshot=runtime_snapshot,
                previous_link=self._state_get_chat(chat_id),
                relink_reason="fresh_topic_event_trigger",
            )
            self._telegram_api.send_message(
                chat_id,
                self._fresh_topic_confirmation_text(link, fresh_topic_trigger),
            )
            return
        link = self._ensure_session_link(
            chat_id,
            chat,
            chat_title,
            runtime_snapshot=runtime_snapshot,
        )
        actor = actor_identity or self._actor_identity(from_user, chat_id)
        transport_envelope = build_transport_event_envelope(
            transport="telegram",
            actor_identity=actor,
            actor_transport_id=from_user.get("id"),
            actor_username=str(from_user.get("username") or "").strip() or None,
            actor_display_name=self._user_display_name(from_user),
            actor_is_bot=bool(from_user.get("is_bot")) if "is_bot" in from_user else None,
            channel_id=chat_id,
            channel_title=chat_title,
            channel_kind=chat.get("type"),
            text=normalized_text,
            message_id=telegram_message_id,
            message_ids=telegram_message_ids,
            attachments=message_attachments,
        )
        pending_turn = PendingTelegramReplyTurn(
            session_id=str(link["session_id"]),
            chat_id=chat_id,
            chat_type=chat.get("type"),
            chat_title=chat_title,
            actor_identity=actor,
            text=normalized_text,
            telegram_message_id=telegram_message_id,
            telegram_message_ids=telegram_message_ids,
            transport_envelope=transport_envelope,
            runtime_snapshot=runtime_snapshot,
        )
        self._remember_pending_reply_turn(pending_turn, status="queued")
        if defer_reply:
            self._submit_reply_serialized(
                self._dispatch_key_for_chat(chat_id),
                self.run_pending_reply_turn_safe,
                pending_turn,
            )
            return
        self.run_pending_reply_turn_safe(pending_turn)

    def run_pending_reply_turn_safe(self, pending_turn: PendingTelegramReplyTurn) -> None:
        try:
            self.run_pending_reply_turn(pending_turn)
        except self._runtime_error_type as exc:
            if self._recover_or_watch_completed_pending_reply(pending_turn, exc):
                self._clear_pending_reply_turn_spool_entry(pending_turn)
                return
            self._remember_pending_reply_turn(pending_turn, status="failed", last_error=str(exc))
            self._log_runtime_error("Deferred Telegram reply failed with bot runtime error.", exc)
            current_link = self._state_get_chat(pending_turn.chat_id)
            if current_link and str(current_link.get("session_id") or "").strip() == pending_turn.session_id:
                self._safe_send_message(pending_turn.chat_id, f"ait Telegram bot error: {exc}")
        except Exception as exc:  # pragma: no cover - exercised in tests via queued reply workers
            if self._recover_or_watch_completed_pending_reply(pending_turn, exc):
                self._clear_pending_reply_turn_spool_entry(pending_turn)
                return
            self._remember_pending_reply_turn(pending_turn, status="failed", last_error=str(exc))
            self._log_runtime_error("Deferred Telegram reply crashed unexpectedly.", exc)
            current_link = self._state_get_chat(pending_turn.chat_id)
            if current_link and str(current_link.get("session_id") or "").strip() == pending_turn.session_id:
                self._safe_send_message(
                    pending_turn.chat_id,
                    "ait Telegram bot hit an unexpected error while processing this update. Check the daemon log and retry if needed.",
                )

    def run_pending_reply_turn(self, pending_turn: PendingTelegramReplyTurn) -> None:
        self._remember_pending_reply_turn(pending_turn, status="attempting", attempt_increment=True)
        turn = self._ait_api_call(
            "create_telegram_turn",
            pending_turn.session_id,
            text=pending_turn.text,
            chat_id=pending_turn.chat_id,
            chat_title=pending_turn.chat_title,
            chat_type=pending_turn.chat_type,
            telegram_message_id=pending_turn.telegram_message_id,
            telegram_message_ids=pending_turn.telegram_message_ids,
            transport_envelope=pending_turn.transport_envelope,
            actor_identity=pending_turn.actor_identity,
            runtime_snapshot=pending_turn.runtime_snapshot,
        )
        user_event = turn.get("user_event") if isinstance(turn, dict) else None
        if not isinstance(user_event, dict):
            raise self._runtime_error_type("ait-server returned an invalid Telegram turn payload.")
        current_link = self._state_get_chat(pending_turn.chat_id)
        live_link_matches = bool(
            current_link and str(current_link.get("session_id") or "").strip() == pending_turn.session_id
        )
        if turn.get("ok"):
            assistant_event = turn.get("assistant_event") if isinstance(turn.get("assistant_event"), dict) else {}
            if not live_link_matches:
                self._state_patch_chat(
                    pending_turn.chat_id,
                    last_relink_skipped_reply_sequence=int(
                        assistant_event.get("sequence") or user_event.get("sequence") or 0
                    ),
                    last_relink_skipped_reply_at=self._now_iso(),
                    last_relink_skipped_from_session_id=pending_turn.session_id,
                )
                self._clear_pending_reply_turn_spool_entry(pending_turn)
                return
            reply_text = str(turn.get("reply_text") or (assistant_event.get("payload") or {}).get("text") or "").strip()
            self.send_assistant_event_reply(pending_turn.chat_id, assistant_event, reply_text=reply_text)
            self._mark_telegram_live_reply_delivered(
                pending_turn.chat_id,
                current_link,
                assistant_event=assistant_event,
                through_sequence=int(assistant_event.get("sequence") or user_event.get("sequence") or 0),
            )
            self._clear_pending_reply_turn_spool_entry(pending_turn)
            return
        if not live_link_matches:
            self._clear_pending_reply_turn_spool_entry(pending_turn)
            return
        self._advance_sync_cursor(
            pending_turn.chat_id,
            current_link,
            through_sequence=int(user_event.get("sequence") or 0),
        )
        error_text = str(turn.get("error") or "Unknown backend reply error.")
        if _retryable_deferred_reply_error_text(error_text) and self._recover_or_watch_completed_pending_reply(
            pending_turn,
            self._runtime_error_type(error_text),
        ):
            self._clear_pending_reply_turn_spool_entry(pending_turn)
            return
        self._remember_pending_reply_turn(
            pending_turn,
            status="failed",
            last_error=error_text,
            user_event=user_event,
        )
        if self._model_capacity_error_text(error_text):
            error_lines = [
                "⚠️ Codex selected model is still at capacity.",
                (
                    f"Logged to {pending_turn.session_id} as event #{user_event.get('sequence')}, "
                    "but automatic continuation retry did not complete or is unavailable."
                ),
                "Please try again later, send `請繼續`, or switch to a lower-effort / fallback model.",
                error_text,
            ]
        else:
            error_lines = [
                f"Logged to {pending_turn.session_id} as event #{user_event.get('sequence')}, but the AI reply failed.",
                error_text,
            ]
        url = _session_url(self._config, pending_turn.session_id)
        if url:
            error_lines.append(url)
        self._telegram_api.send_message(pending_turn.chat_id, "\n".join(error_lines))

    def send_assistant_event_reply(
        self,
        chat_id: str | int,
        assistant_event: Mapping[str, Any],
        *,
        reply_text: str | None = None,
    ) -> None:
        text = str(reply_text or "").strip() or _transport_reply_text(assistant_event)
        attachments = _transport_reply_attachments(assistant_event)
        if not text and not attachments:
            raise self._runtime_error_type("ait-server returned an empty Telegram reply.")
        if text:
            self._telegram_api.send_message(chat_id, text)
        if attachments:
            self.deliver_reply_attachments(chat_id, attachments)

    def deliver_reply_attachments(self, chat_id: str | int, attachments: list[dict[str, Any]]) -> None:
        for attachment in attachments:
            if _attachment_should_send_as_audio(attachment):
                self._telegram_api.send_audio(chat_id, attachment)
            else:
                self._telegram_api.send_document(chat_id, attachment)
