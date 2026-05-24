from __future__ import annotations

from dataclasses import dataclass
import threading
import time
from typing import Any, Callable


@dataclass(frozen=True)
class PendingTelegramTextUpdate:
    update_key: str
    chat_key: str
    update: dict[str, Any]
    normalized_text: str
    mergeable: bool
    actor_identity: str
    received_at: float
    telegram_message_id: int | None


@dataclass(frozen=True)
class TelegramLogicalTurn:
    update: dict[str, Any]
    text: str
    actor_identity: str
    telegram_message_id: int | None
    telegram_message_ids: tuple[int, ...] = ()


class TelegramLogicalTurnBuffer:
    def __init__(
        self,
        *,
        username: str,
        merge_window_seconds: float,
        max_messages: int,
        poll_interval_seconds: float,
        update_key: Callable[[dict[str, Any]], str],
        dispatch_key_for_chat: Callable[[str | int], str],
        normalize_user_text: Callable[[str, str], str],
        parse_command: Callable[[str, str], tuple[str, str] | None],
        detect_workflow_query: Callable[[str], tuple[str, str | None] | None],
        actor_identity: Callable[[dict[str, Any], str | int], str],
        skip_logical_turn: object,
    ) -> None:
        self._username = username
        self._merge_window_seconds = merge_window_seconds
        self._max_messages = max_messages
        self._poll_interval_seconds = poll_interval_seconds
        self._update_key = update_key
        self._dispatch_key_for_chat = dispatch_key_for_chat
        self._normalize_user_text = normalize_user_text
        self._parse_command = parse_command
        self._detect_workflow_query = detect_workflow_query
        self._actor_identity = actor_identity
        self._skip_logical_turn = skip_logical_turn
        self._lock = threading.Lock()
        self._pending_text_updates: dict[str, list[PendingTelegramTextUpdate]] = {}

    def logical_turn_merge_enabled(self) -> bool:
        return self._merge_window_seconds > 0 and self._max_messages > 1

    def buffer_submitted_text_update(self, update: dict[str, Any]) -> None:
        candidate = self._classify_pending_text_update(update)
        if candidate is None:
            return
        with self._lock:
            queue = self._pending_text_updates.setdefault(candidate.chat_key, [])
            if any(item.update_key == candidate.update_key for item in queue):
                return
            queue.append(candidate)

    def claim_logical_turn(self, update: dict[str, Any]) -> TelegramLogicalTurn | object | None:
        candidate = self._classify_pending_text_update(update)
        if candidate is None:
            return None
        with self._lock:
            queue = self._pending_text_updates.get(candidate.chat_key) or []
            entry = next((item for item in queue if item.update_key == candidate.update_key), None)
            if entry is None:
                return self._skip_logical_turn
            if not entry.mergeable:
                self._remove_pending_text_update_locked(entry.chat_key, entry.update_key)
                return None
        return self._consume_logical_turn(candidate.chat_key, candidate.update_key)

    def _consume_logical_turn(self, chat_key: str, update_key: str) -> TelegramLogicalTurn | object:
        while True:
            with self._lock:
                queue = self._pending_text_updates.get(chat_key) or []
                current_index = next((index for index, item in enumerate(queue) if item.update_key == update_key), None)
                if current_index is None:
                    return self._skip_logical_turn

                first = queue[current_index]
                if not first.mergeable:
                    self._remove_pending_text_update_locked(chat_key, update_key)
                    return self._skip_logical_turn

                selected = [first]
                boundary_seen = False
                for item in queue[current_index + 1 :]:
                    if not item.mergeable or item.actor_identity != first.actor_identity:
                        boundary_seen = True
                        break
                    selected.append(item)
                    if len(selected) >= self._max_messages:
                        break

                latest_received_at = max(item.received_at for item in selected)
                quiet_elapsed = time.monotonic() - latest_received_at
                reached_limit = len(selected) >= self._max_messages
                if boundary_seen or reached_limit or quiet_elapsed >= self._merge_window_seconds:
                    del queue[current_index : current_index + len(selected)]
                    if queue:
                        self._pending_text_updates[chat_key] = queue
                    else:
                        self._pending_text_updates.pop(chat_key, None)
                    return self._build_logical_turn(selected)
                sleep_for = min(
                    max(self._merge_window_seconds - quiet_elapsed, 0.0),
                    self._poll_interval_seconds,
                )
            time.sleep(sleep_for)

    def _build_logical_turn(self, updates: list[PendingTelegramTextUpdate]) -> TelegramLogicalTurn:
        message_ids = tuple(item.telegram_message_id for item in updates if item.telegram_message_id is not None)
        return TelegramLogicalTurn(
            update=updates[0].update,
            text="\n\n".join(item.normalized_text for item in updates if item.normalized_text).strip(),
            actor_identity=updates[0].actor_identity,
            telegram_message_id=message_ids[-1] if message_ids else None,
            telegram_message_ids=message_ids,
        )

    def _classify_pending_text_update(self, update: dict[str, Any]) -> PendingTelegramTextUpdate | None:
        message = update.get("message") or {}
        text = message.get("text")
        if not isinstance(text, str) or not text.strip():
            return None
        chat = message.get("chat") or {}
        from_user = message.get("from") or {}
        chat_id = chat.get("id")
        if chat_id is None:
            return None
        normalized = self._normalize_user_text(text, self._username)
        command = self._parse_command(text, self._username)
        workflow_query = None if command else self._detect_workflow_query(normalized)
        return PendingTelegramTextUpdate(
            update_key=self._update_key(update),
            chat_key=self._dispatch_key_for_chat(chat_id),
            update=update,
            normalized_text=normalized,
            mergeable=bool(normalized) and command is None and workflow_query is None,
            actor_identity=self._actor_identity(from_user, chat_id),
            received_at=time.monotonic(),
            telegram_message_id=message.get("message_id"),
        )

    def _remove_pending_text_update_locked(self, chat_key: str, update_key: str) -> None:
        queue = self._pending_text_updates.get(chat_key) or []
        remaining = [item for item in queue if item.update_key != update_key]
        if remaining:
            self._pending_text_updates[chat_key] = remaining
        else:
            self._pending_text_updates.pop(chat_key, None)
