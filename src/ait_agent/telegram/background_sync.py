from __future__ import annotations

from typing import Any, Callable, Mapping

from .session_views import format_session_events
from .workflow_notifications import (
    _graph_watches,
    _queue_digest,
    _queue_digest_actionable,
    format_workflow_notification,
)


class TelegramBackgroundSyncManager:
    def __init__(
        self,
        *,
        config: Any,
        ait_api: Any,
        telegram_api: Any,
        state_get_chat: Callable[[str | int], dict[str, Any] | None],
        state_patch_chat: Callable[..., dict[str, Any] | None],
        sync_session: Callable[[str | int, dict[str, Any]], list[dict[str, Any]]],
        mark_missing_session_relink_required: Callable[[str | int, Mapping[str, Any], Any | None], dict[str, Any] | None],
        should_skip_event_for_chat: Callable[[str | int, dict[str, Any], Mapping[str, Any] | None], bool],
        runtime_snapshot: Callable[[], Any],
        session_missing_relink_reason: Callable[[Mapping[str, Any] | None, Any | None], str],
        run_graph_notifications_for_chat: Callable[[str, dict[str, Any]], bool],
        log_runtime_error: Callable[[str, Exception], None],
        missing_session_server_read_error: Callable[[BaseException], bool],
        retryable_server_read_error: Callable[[BaseException], bool],
        retryable_transport_error: Callable[[BaseException], bool],
        runtime_error_type: type[Exception],
        now_iso: Callable[[], str],
        backoff_threshold: int | Callable[[], int],
        backoff_delay_seconds: Callable[[int], float],
        clock_time: Callable[[], float],
        replay_undelivered_telegram_live_replies: Callable[[str | int, Mapping[str, Any], list[dict[str, Any]]], tuple[list[dict[str, Any]], bool]],
    ) -> None:
        self._config = config
        self._ait_api = ait_api
        self._telegram_api = telegram_api
        self._state_get_chat = state_get_chat
        self._state_patch_chat = state_patch_chat
        self._sync_session = sync_session
        self._mark_missing_session_relink_required = mark_missing_session_relink_required
        self._should_skip_event_for_chat = should_skip_event_for_chat
        self._runtime_snapshot = runtime_snapshot
        self._session_missing_relink_reason = session_missing_relink_reason
        self._run_graph_notifications_for_chat = run_graph_notifications_for_chat
        self._log_runtime_error = log_runtime_error
        self._missing_session_server_read_error = missing_session_server_read_error
        self._retryable_server_read_error = retryable_server_read_error
        self._retryable_transport_error = retryable_transport_error
        self._runtime_error_type = runtime_error_type
        self._now_iso = now_iso
        self._backoff_threshold = backoff_threshold
        self._backoff_delay_seconds = backoff_delay_seconds
        self._clock_time = clock_time
        self._replay_undelivered_telegram_live_replies = replay_undelivered_telegram_live_replies

    def _background_sync_backoff_threshold_value(self) -> int:
        value = self._backoff_threshold() if callable(self._backoff_threshold) else self._backoff_threshold
        return max(int(value), 1)

    def sync_session(self, chat_id: str | int, link: dict[str, Any]) -> list[dict[str, Any]]:
        return self._sync_session(chat_id, link)

    def mark_missing_session_relink_required(
        self,
        chat_id: str | int,
        link: Mapping[str, Any],
        *,
        runtime_snapshot: Any | None = None,
    ) -> dict[str, Any] | None:
        snapshot = runtime_snapshot or self._runtime_snapshot()
        return self._mark_missing_session_relink_required(
            chat_id,
            link,
            runtime_snapshot=snapshot,
        )

    def should_skip_event_for_chat(
        self,
        chat_id: str | int,
        event: dict[str, Any],
        *,
        link: Mapping[str, Any] | None = None,
    ) -> bool:
        return self._should_skip_event_for_chat(chat_id, event, link)

    def background_sync_error_is_retryable(self, exc: BaseException) -> bool:
        return self._retryable_server_read_error(exc) or self._retryable_transport_error(exc)

    def background_sync_backoff_active(self, link: Mapping[str, Any] | None) -> bool:
        retry_after_epoch = float((link or {}).get("background_sync_retry_after_epoch") or 0.0)
        return retry_after_epoch > self._clock_time()

    def link_has_background_sync_work(self, link: Mapping[str, Any] | None) -> bool:
        if not isinstance(link, Mapping):
            return False
        if str(link.get("session_id") or "").strip():
            return True
        if bool(link.get("workflow_notifications_enabled")):
            return True
        return bool(self._config.graph_watch_background_sweep_enabled and _graph_watches(link))

    def record_background_sync_success(self, chat_id: str | int) -> dict[str, Any] | None:
        return self._state_patch_chat(
            chat_id,
            background_sync_failure_streak=0,
            background_sync_retry_after_epoch=None,
            background_sync_last_failure_at=None,
            background_sync_last_error=None,
        )

    def record_background_sync_failure(
        self,
        chat_id: str | int,
        *,
        link: Mapping[str, Any] | None,
        exc: BaseException,
    ) -> dict[str, Any] | None:
        current_link = self._state_get_chat(chat_id) or dict(link or {})
        failure_streak = max(int(current_link.get("background_sync_failure_streak") or 0) + 1, 1)
        retry_after_epoch = None
        if self.background_sync_error_is_retryable(exc):
            threshold = self._background_sync_backoff_threshold_value()
            if failure_streak >= threshold:
                retry_after_epoch = self._clock_time() + self._backoff_delay_seconds(failure_streak)
        return self._state_patch_chat(
            chat_id,
            background_sync_failure_streak=failure_streak,
            background_sync_retry_after_epoch=retry_after_epoch,
            background_sync_last_failure_at=self._now_iso(),
            background_sync_last_error=str(exc).strip() or type(exc).__name__,
        )

    def _run_background_sync_pass(
        self,
        chat_id: str,
        link: Mapping[str, Any],
    ) -> bool:
        sent_any = False
        if str(link.get("session_id") or "").strip():
            events = self.sync_session(chat_id, dict(link))
            events, replayed_any = self._replay_undelivered_telegram_live_replies(chat_id, link, events)
            sent_any = replayed_any or sent_any
            if events:
                self._telegram_api.send_message(chat_id, format_session_events(events))
                sent_any = True
        latest_link = self._state_get_chat(chat_id) or dict(link)
        if bool(latest_link.get("workflow_notifications_enabled")):
            sent_any = self.run_workflow_notifications_for_chat(chat_id, latest_link) or sent_any
            latest_link = self._state_get_chat(chat_id) or dict(link)
        if self._config.graph_watch_background_sweep_enabled and _graph_watches(latest_link):
            sent_any = self.run_graph_notifications_for_chat(chat_id, latest_link) or sent_any
        return sent_any

    def run_background_sync_for_chat(self, chat_id: str) -> bool:
        link: dict[str, Any] | None = None
        try:
            link = self._state_get_chat(chat_id)
            if not self.link_has_background_sync_work(link):
                return False
            if self.background_sync_backoff_active(link):
                return False
            sent_any = self._run_background_sync_pass(chat_id, link)
            self.record_background_sync_success(chat_id)
            return sent_any
        except self._runtime_error_type as exc:
            if link is not None and self._missing_session_server_read_error(exc):
                try:
                    relinked_link = self.mark_missing_session_relink_required(chat_id, link)
                    latest_link = self._state_get_chat(chat_id) or dict(relinked_link or {})
                    if self.link_has_background_sync_work(latest_link):
                        sent_any = self._run_background_sync_pass(chat_id, latest_link)
                        self.record_background_sync_success(chat_id)
                        return sent_any
                    self.record_background_sync_success(chat_id)
                    return False
                except Exception as recovery_exc:  # pragma: no cover - recovery fallback path
                    exc = recovery_exc
            self.record_background_sync_failure(chat_id, link=link, exc=exc)
            self._log_runtime_error(f"Telegram background sync failed for chat {chat_id}.", exc)
            return False
        except Exception as exc:  # pragma: no cover - defensive background sync isolation
            self.record_background_sync_failure(chat_id, link=link, exc=exc)
            self._log_runtime_error(f"Telegram background sync crashed for chat {chat_id}.", exc)
            return False

    def run_workflow_notifications_for_chat(self, chat_id: str, link: dict[str, Any]) -> bool:
        payload = self._ait_api.read_task_queue()
        current_digest = _queue_digest(payload)
        previous_digest = str(link.get("last_queue_summary_digest") or "")
        previous_actionable = _queue_digest_actionable(previous_digest)
        current_actionable = _queue_digest_actionable(current_digest)
        if current_digest == previous_digest:
            return False
        self._state_patch_chat(
            chat_id,
            last_queue_summary_digest=current_digest,
            last_queue_notification_at=self._now_iso() if (current_actionable or previous_actionable) else None,
        )
        if current_actionable:
            self._telegram_api.send_message(chat_id, format_workflow_notification(self._config, payload))
            return True
        if previous_actionable:
            self._telegram_api.send_message(
                chat_id,
                f"workflow ({self._config.repo_name})\n\nComplete",
            )
            return True
        return False

    def run_graph_notifications_for_chat(self, chat_id: str, link: dict[str, Any]) -> bool:
        return self._run_graph_notifications_for_chat(chat_id, link)
