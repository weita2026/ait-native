from __future__ import annotations

import time
from typing import Any, Callable, Mapping


class TelegramLiveReplyManager:
    def __init__(
        self,
        *,
        repo_name: str,
        state_get_chat: Callable[[str | int], dict[str, Any] | None],
        state_upsert_chat: Callable[..., dict[str, Any]],
        state_patch_chat: Callable[..., dict[str, Any] | None],
        ait_api_call: Callable[..., Any],
        send_assistant_event_reply: Callable[[str | int, Mapping[str, Any]], None],
        log_runtime_error: Callable[[str, Exception], None],
        now_iso: Callable[[], str],
        retryable_server_read_error: Callable[[BaseException], bool],
        retryable_transport_error: Callable[[BaseException], bool],
        recovery_attempts: int | Callable[[], int],
        recovery_delay_seconds: Callable[[int], float],
        delivered_sequence_limit: int = 200,
        watch_max_wait_seconds: float | Callable[[], float] = 0.0,
        watch_poll_interval_seconds: float | Callable[[], float] = 0.0,
    ) -> None:
        self._repo_name = repo_name
        self._state_get_chat = state_get_chat
        self._state_upsert_chat = state_upsert_chat
        self._state_patch_chat = state_patch_chat
        self._ait_api_call = ait_api_call
        self._send_assistant_event_reply = send_assistant_event_reply
        self._log_runtime_error = log_runtime_error
        self._now_iso = now_iso
        self._retryable_server_read_error = retryable_server_read_error
        self._retryable_transport_error = retryable_transport_error
        self._recovery_attempts = recovery_attempts
        self._recovery_delay_seconds = recovery_delay_seconds
        self._delivered_sequence_limit = delivered_sequence_limit
        self._watch_max_wait_seconds = watch_max_wait_seconds
        self._watch_poll_interval_seconds = watch_poll_interval_seconds

    def _recovery_attempt_count(self) -> int:
        value = self._recovery_attempts() if callable(self._recovery_attempts) else self._recovery_attempts
        return max(int(value), 1)

    def _watch_max_wait(self) -> float:
        value = self._watch_max_wait_seconds() if callable(self._watch_max_wait_seconds) else self._watch_max_wait_seconds
        return float(value)

    def _watch_poll_interval(self) -> float:
        value = (
            self._watch_poll_interval_seconds()
            if callable(self._watch_poll_interval_seconds)
            else self._watch_poll_interval_seconds
        )
        return float(value)

    def advance_sync_cursor(
        self,
        chat_id: str | int,
        link: dict[str, Any],
        *,
        through_sequence: int,
    ) -> dict[str, Any]:
        return self._state_upsert_chat(
            chat_id,
            session_id=str(link["session_id"]),
            repo_name=self._repo_name,
            chat_type=link.get("chat_type"),
            chat_title=link.get("chat_title"),
            last_synced_sequence=max(int(through_sequence), 0),
            last_sync_at=self._now_iso(),
        )

    def telegram_live_delivered_sequences(self, link: Mapping[str, Any] | None) -> set[int]:
        values = (link or {}).get("telegram_live_delivered_sequences")
        if not isinstance(values, list):
            return set()
        delivered: set[int] = set()
        for value in values:
            try:
                sequence = int(value)
            except (TypeError, ValueError):
                continue
            if sequence > 0:
                delivered.add(sequence)
        return delivered

    def mark_telegram_live_reply_delivered(
        self,
        chat_id: str | int,
        link: dict[str, Any],
        *,
        assistant_event: Mapping[str, Any] | None,
        through_sequence: int,
    ) -> dict[str, Any]:
        sequence = int((assistant_event or {}).get("sequence") or 0)
        current_link = self._state_get_chat(chat_id) or link
        delivered = self.telegram_live_delivered_sequences(current_link)
        if sequence > 0:
            delivered.add(sequence)
        bounded_delivered = sorted(delivered)[-self._delivered_sequence_limit :]
        patched = self._state_patch_chat(
            chat_id,
            last_synced_sequence=max(int(through_sequence), 0),
            last_sync_at=self._now_iso(),
            telegram_live_delivered_sequences=bounded_delivered,
            last_relink_skipped_reply_sequence=None,
            last_relink_skipped_reply_at=None,
            last_relink_skipped_from_session_id=None,
        )
        if patched is not None:
            return patched
        return self.advance_sync_cursor(chat_id, link, through_sequence=through_sequence)

    def pending_turn_matches_user_event(
        self,
        pending_turn: Any,
        event: Mapping[str, Any],
    ) -> bool:
        if str(event.get("event_type") or "") != "telegram.user_message":
            return False
        payload = event.get("payload")
        if not isinstance(payload, Mapping):
            return False
        if str(payload.get("telegram_chat_id") or "") != str(pending_turn.chat_id):
            return False
        envelope = payload.get("transport_envelope")
        expected_event_id = ""
        if isinstance(pending_turn.transport_envelope, Mapping):
            expected_event_id = str(pending_turn.transport_envelope.get("event_id") or "").strip()
        if expected_event_id and isinstance(envelope, Mapping):
            if str(envelope.get("event_id") or "").strip() == expected_event_id:
                return True
        pending_message_ids: set[int] = set()
        for value in [pending_turn.telegram_message_id, *list(pending_turn.telegram_message_ids or ())]:
            try:
                message_id = int(value)
            except (TypeError, ValueError):
                continue
            if message_id > 0:
                pending_message_ids.add(message_id)
        event_message_ids: set[int] = set()
        for value in [payload.get("telegram_message_id"), *list(payload.get("telegram_message_ids") or ())]:
            try:
                message_id = int(value)
            except (TypeError, ValueError):
                continue
            if message_id > 0:
                event_message_ids.add(message_id)
        return bool(pending_message_ids and event_message_ids and pending_message_ids.intersection(event_message_ids))

    def assistant_reply_for_user_event(
        self,
        *,
        chat_id: str | int,
        user_event: Mapping[str, Any],
        events: list[dict[str, Any]],
    ) -> dict[str, Any] | None:
        user_sequence = int(user_event.get("sequence") or 0)
        if user_sequence <= 0:
            return None
        for event in events:
            if str(event.get("event_type") or "") != "assistant.reply":
                continue
            payload = event.get("payload")
            if not isinstance(payload, Mapping):
                continue
            if str(payload.get("telegram_chat_id") or "") != str(chat_id):
                continue
            if int(payload.get("reply_to_sequence") or 0) == user_sequence:
                return event
        return None

    def recover_completed_pending_reply_once(self, pending_turn: Any) -> str:
        current_link = self._state_get_chat(pending_turn.chat_id)
        if not current_link or str(current_link.get("session_id") or "").strip() != pending_turn.session_id:
            return "terminal"
        after_sequence = max(int(current_link.get("last_synced_sequence") or 0) - 5, 0)
        try:
            events = self._ait_api_call(
                "list_session_events",
                pending_turn.session_id,
                after_sequence=after_sequence,
                limit=200,
                runtime_snapshot=pending_turn.runtime_snapshot,
            )
        except Exception as exc:  # pragma: no cover - defensive recovery path
            self._log_runtime_error("Deferred Telegram reply recovery could not read session events.", exc)
            return "retryable" if self._retryable_server_read_error(exc) else "terminal"
        user_event = next(
            (event for event in events if self.pending_turn_matches_user_event(pending_turn, event)),
            None,
        )
        if user_event is None:
            return "retryable"
        assistant_event = self.assistant_reply_for_user_event(
            chat_id=pending_turn.chat_id,
            user_event=user_event,
            events=events,
        )
        if assistant_event is None:
            return "retryable"
        live_link = self._state_get_chat(pending_turn.chat_id)
        if not live_link or str(live_link.get("session_id") or "").strip() != pending_turn.session_id:
            return "terminal"
        try:
            self._send_assistant_event_reply(pending_turn.chat_id, assistant_event)
            self.mark_telegram_live_reply_delivered(
                pending_turn.chat_id,
                live_link,
                assistant_event=assistant_event,
                through_sequence=int(assistant_event.get("sequence") or user_event.get("sequence") or 0),
            )
        except Exception as exc:  # pragma: no cover - defensive recovery path
            self._log_runtime_error("Deferred Telegram reply recovery failed to deliver stored reply.", exc)
            return "retryable" if self._retryable_transport_error(exc) else "terminal"
        return "recovered"

    def recover_completed_pending_reply(self, pending_turn: Any) -> bool:
        attempts = self._recovery_attempt_count()
        for attempt in range(attempts):
            outcome = self.recover_completed_pending_reply_once(pending_turn)
            if outcome == "recovered":
                return True
            if outcome == "terminal" or attempt + 1 >= attempts:
                return False
            time.sleep(self._recovery_delay_seconds(attempt))
        return False

    def watch_for_completed_pending_reply(self, pending_turn: Any) -> bool:
        max_wait_seconds = self._watch_max_wait()
        if max_wait_seconds <= 0:
            return False
        deadline = time.monotonic() + max_wait_seconds
        while time.monotonic() < deadline:
            current_link = self._state_get_chat(pending_turn.chat_id)
            if not current_link or str(current_link.get("session_id") or "").strip() != pending_turn.session_id:
                return False
            remaining_seconds = deadline - time.monotonic()
            if remaining_seconds <= 0:
                break
            time.sleep(min(self._watch_poll_interval(), remaining_seconds))
            if self.recover_completed_pending_reply(pending_turn):
                return True
        return False

    def replay_undelivered_telegram_live_replies(
        self,
        chat_id: str | int,
        link: Mapping[str, Any],
        events: list[dict[str, Any]],
    ) -> tuple[list[dict[str, Any]], bool]:
        replayed_sequences: set[int] = set()
        sent_any = False
        for event in events:
            payload = event.get("payload") if isinstance(event.get("payload"), Mapping) else {}
            if str(payload.get("delivered_via") or "") != "telegram_live":
                continue
            if str(payload.get("telegram_chat_id") or "") != str(chat_id):
                continue
            sequence = int(event.get("sequence") or 0)
            if sequence <= 0 or sequence in self.telegram_live_delivered_sequences(link):
                continue
            try:
                self._send_assistant_event_reply(chat_id, event)
                refreshed_link = self._state_get_chat(chat_id) or dict(link)
                self.mark_telegram_live_reply_delivered(
                    chat_id,
                    refreshed_link,
                    assistant_event=event,
                    through_sequence=sequence,
                )
                replayed_sequences.add(sequence)
                sent_any = True
            except Exception as exc:  # pragma: no cover - defensive replay isolation
                self._log_runtime_error(
                    f"Telegram background sync failed to replay stored live reply for chat {chat_id}.",
                    exc,
                )
        remaining_events = [
            event
            for event in events
            if int(event.get("sequence") or 0) not in replayed_sequences
        ]
        return remaining_events, sent_any

    def should_skip_event_for_chat(
        self,
        chat_id: str | int,
        event: dict[str, Any],
        *,
        link: Mapping[str, Any] | None = None,
    ) -> bool:
        payload = event.get("payload") or {}
        if str(payload.get("source") or "") == "telegram" and str(payload.get("telegram_chat_id") or "") == str(chat_id):
            return True
        if str(payload.get("delivered_via") or "") == "telegram_live" and str(payload.get("telegram_chat_id") or "") == str(chat_id):
            sequence = int(event.get("sequence") or 0)
            return sequence in self.telegram_live_delivered_sequences(link)
        return False
