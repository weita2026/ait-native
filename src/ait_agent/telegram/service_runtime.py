from __future__ import annotations

import math
import threading
import time
from concurrent.futures import Future, ThreadPoolExecutor, wait
from typing import Any, Callable


class TelegramServiceRuntime:
    def __init__(
        self,
        *,
        config: Any,
        telegram_api: Any,
        state_load: Callable[[], Any],
        state_update_last_update_id: Callable[[int], None],
        submit_update: Callable[[dict[str, Any]], Future[Any]],
        run_background_sync_once: Callable[[], list[Future[Any]]],
        log_runtime_error: Callable[[str, Exception], None],
        runtime_error_type: type[Exception],
        sleep: Callable[[float], None] = time.sleep,
        monotonic: Callable[[], float] = time.monotonic,
    ) -> None:
        self._config = config
        self._telegram_api = telegram_api
        self._state_load = state_load
        self._state_update_last_update_id = state_update_last_update_id
        self._submit_update = submit_update
        self._run_background_sync_once = run_background_sync_once
        self._log_runtime_error = log_runtime_error
        self._runtime_error_type = runtime_error_type
        self._sleep = sleep
        self._monotonic = monotonic
        self._stop_requested = False
        self._dispatch_lock = threading.Lock()
        self._dispatchers: dict[str, ThreadPoolExecutor] = {}
        self._reply_dispatch_lock = threading.Lock()
        self._reply_dispatchers: dict[str, ThreadPoolExecutor] = {}
        self._dispatch_futures_lock = threading.Lock()
        self._dispatch_futures: set[Future[Any]] = set()

    def stop(self) -> None:
        self._stop_requested = True
        with self._dispatch_lock:
            executors = list(self._dispatchers.values())
            self._dispatchers.clear()
        with self._reply_dispatch_lock:
            reply_executors = list(self._reply_dispatchers.values())
            self._reply_dispatchers.clear()
        for executor in executors:
            executor.shutdown(wait=False, cancel_futures=False)
        for executor in reply_executors:
            executor.shutdown(wait=False, cancel_futures=False)

    def run_forever(self) -> None:
        next_background_sync_at = self.next_background_sync_deadline() if self._config.background_sync_enabled else None
        while not self._stop_requested:
            try:
                next_background_sync_at = self.run_due_background_sync(next_background_sync_at)
                state = self._state_load()
                offset = int(state.last_update_id or 0) + 1
                updates = self._telegram_api.get_updates(
                    offset=offset,
                    timeout_seconds=self.poll_timeout_seconds(next_background_sync_at),
                )
                if not updates:
                    continue
                for update in updates:
                    self._submit_update(update)
                    update_id = int(update.get("update_id") or 0)
                    if update_id:
                        self._state_update_last_update_id(update_id)
            except self._runtime_error_type as exc:
                self._log_runtime_error("Telegram polling failed with bot runtime error.", exc)
                self._sleep(1.0)
            except Exception as exc:  # pragma: no cover - defensive daemon hardening
                self._log_runtime_error("Telegram polling crashed unexpectedly.", exc)
                self._sleep(1.0)

    def submit_serialized(self, queue_key: str, fn, *args: Any) -> Future[Any]:
        with self._dispatch_lock:
            executor = self._dispatchers.get(queue_key)
            if executor is None:
                executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix=f"ait-telegram-{queue_key}")
                self._dispatchers[queue_key] = executor
        future = executor.submit(fn, *args)
        return self._track_future(future)

    def submit_reply_serialized(self, queue_key: str, fn, *args: Any) -> Future[Any]:
        with self._reply_dispatch_lock:
            executor = self._reply_dispatchers.get(queue_key)
            if executor is None:
                executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix=f"ait-telegram-reply-{queue_key}")
                self._reply_dispatchers[queue_key] = executor
        future = executor.submit(fn, *args)
        return self._track_future(future)

    def wait_for_idle(self, timeout: float | None = None) -> bool:
        deadline = None if timeout is None else self._monotonic() + timeout
        while True:
            with self._dispatch_futures_lock:
                futures = [future for future in self._dispatch_futures if not future.done()]
            if not futures:
                return True
            remaining = None if deadline is None else max(deadline - self._monotonic(), 0.0)
            if remaining is not None and remaining <= 0:
                return False
            wait(futures, timeout=remaining)

    def forget_future(self, future: Future[Any]) -> None:
        with self._dispatch_futures_lock:
            self._dispatch_futures.discard(future)

    def next_background_sync_deadline(self) -> float:
        return self._monotonic() + self._config.background_sync_interval_seconds

    def poll_timeout_seconds(self, next_background_sync_at: float | None) -> int:
        timeout_seconds = self._config.poll_timeout_seconds
        if not self._config.background_sync_enabled or next_background_sync_at is None:
            return timeout_seconds
        seconds_until_sync = max(next_background_sync_at - self._monotonic(), 0.0)
        return min(timeout_seconds, max(1, int(math.ceil(seconds_until_sync))))

    def run_due_background_sync(self, next_background_sync_at: float | None) -> float | None:
        if not self._config.background_sync_enabled:
            return None
        if next_background_sync_at is None:
            next_background_sync_at = self.next_background_sync_deadline()
        if self._monotonic() < next_background_sync_at:
            return next_background_sync_at
        self._run_background_sync_once()
        return self.next_background_sync_deadline()

    def _track_future(self, future: Future[Any]) -> Future[Any]:
        with self._dispatch_futures_lock:
            self._dispatch_futures.add(future)
        future.add_done_callback(self.forget_future)
        return future
