from __future__ import annotations

import asyncio
from concurrent.futures import Future, ThreadPoolExecutor, wait
import json
import math
import mimetypes
import os
import re
import sys
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence
from urllib.parse import urlencode
from urllib.request import Request, urlopen

import httpx

from ait_agent.envelope import (
    build_transport_binding_metadata,
    build_transport_session_metadata,
)
from ait_agent.runtime_backend import (
    AgentRuntimeConfigError,
    AgentRuntimeTarget,
    LocalAitRuntime,
    resolve_agent_runtime_target,
)
from ait_agent.transport_retry import (
    exception_chain as _shared_exception_chain,
    is_loopback_url as _shared_is_loopback_url,
    is_retryable_server_read_error as _shared_is_retryable_server_read_error,
    is_retryable_transport_error as _shared_is_retryable_transport_error,
    retry_transport_operation_async as _shared_retry_transport_operation_async,
    retry_delay_seconds as _shared_retry_delay_seconds,
    retry_transport_operation as _shared_retry_transport_operation,
    timeout_value as _shared_timeout_value,
)
from ait_chat.session_reply import AiReplyResult
from .background_sync import TelegramBackgroundSyncManager
from .commands import (
    TELEGRAM_COMMANDS_BY_NAME,
    TELEGRAM_COMMAND_SPECS,
    TelegramCommandRuntime,
    WORKFLOW_QUERY_EXAMPLES,
)
from .event_triggers import (
    EventTriggerRegistry,
    load_event_trigger_registry,
    parse_fresh_topic_trigger,
)
from .formatting import (
    TelegramMessageChunk,
    _markdownish_parse_error,
    _render_markdownish_message_chunks,
    _telegram_message_chunks,
)
from .turn_inputs import _music_attachments_from_message, _normalized_turn_text, _speech_attachments_from_message, normalize_user_text
from .graph_watches import (
    TelegramGraphWatchManager,
    auto_register_graph_watch,
    trigger_graph_watch_notifications,
    upsert_graph_watch_for_chat,
    _runtime_repo_root,
)
from .logical_turns import (
    PendingTelegramTextUpdate,
    TelegramLogicalTurn,
    TelegramLogicalTurnBuffer,
)
from .live_replies import TelegramLiveReplyManager
from .owner_bootstrap import TelegramOwnerBootstrapGate
from .reply_turns import PendingTelegramReplyTurn, TelegramReplyTurnManager, TelegramReplyTurnSpool
from .session_links import TelegramSessionLinkCoordinator
from .session_views import (
    _session_url,
    format_session_events,
    format_session_status,
)
from .speech_to_text import (
    LocalSpeechToTextError,
    LocalSpeechToTextRuntime,
    LocalSpeechToTextTurnInput,
)
from .update_dispatch import TelegramUpdateDispatch
from .workflow_notifications import (
    format_attention_summary,
    format_change_land_summary,
    format_change_summary,
    format_queue_summary,
    format_ready_summary,
    format_task_audit_summary,
    format_task_summary,
    format_workflow_notification,
)
from .workflow_queries import (
    CHANGE_ID_PATTERN,
    PLAN_ID_PATTERN,
    TASK_ID_PATTERN,
    actor_identity as _actor_identity,
    chat_title as _chat_title,
    detect_workflow_query,
    parse_command,
    user_display_name as _user_display_name,
)
from .config import (
    BotConfig,
    BotRuntimeError,
    load_config,
)
from .runtime import (
    TelegramSyncStateStore,
    utc_now_iso,
)
from .worker_config import AIT_TELEGRAM_TERMINATION_CONTEXT_ENV, load_config_for_telegram_worker


TURN_MERGE_POLL_INTERVAL_SECONDS = 0.05
TELEGRAM_DELIVERY_RETRY_ATTEMPTS = 3
TELEGRAM_DELIVERY_RETRY_BASE_DELAY_SECONDS = 1.0
TELEGRAM_POLL_RETRY_ATTEMPTS = 4
TELEGRAM_POLL_RETRY_BASE_DELAY_SECONDS = 1.0
AIT_SERVER_READ_RETRY_ATTEMPTS = 4
AIT_SERVER_READ_RETRY_BASE_DELAY_SECONDS = 0.75
DEFERRED_REPLY_RECOVERY_ATTEMPTS = 4
DEFERRED_REPLY_RECOVERY_BASE_DELAY_SECONDS = 0.75
DEFERRED_REPLY_WATCH_MAX_WAIT_SECONDS = 90.0
DEFERRED_REPLY_WATCH_POLL_INTERVAL_SECONDS = 5.0
TELEGRAM_BACKGROUND_SYNC_FAILURE_BACKOFF_THRESHOLD = 2
TELEGRAM_BACKGROUND_SYNC_FAILURE_BACKOFF_BASE_SECONDS = 15.0
TELEGRAM_BACKGROUND_SYNC_FAILURE_BACKOFF_MAX_SECONDS = 120.0
TELEGRAM_REPLY_SPOOL_LIMIT = 20
TELEGRAM_DELIVERY_RETRYABLE_ERRNOS = frozenset({54, 60, 61, 104, 110, 111})
TELEGRAM_DELIVERY_RETRYABLE_MARKERS = (
    "timed out",
    "connection reset by peer",
    "remote end closed connection without response",
    "temporarily unavailable",
    "connection aborted",
    "broken pipe",
    "network is unreachable",
)
RETRYABLE_SERVER_READ_MARKERS = (
    "500 internal server error",
    "502 bad gateway",
    "503 service unavailable",
    "504 gateway timeout",
)
MISSING_SESSION_SERVER_READ_MARKERS = (
    "unknown session",
    "session not found on current backend",
)
_SKIP_LOGICAL_TURN = object()


def _is_retryable_telegram_delivery_error(exc: BaseException) -> bool:
    return _shared_is_retryable_transport_error(
        exc,
        errnos=TELEGRAM_DELIVERY_RETRYABLE_ERRNOS,
        markers=TELEGRAM_DELIVERY_RETRYABLE_MARKERS,
    )


def _is_retryable_server_read_error(exc: BaseException) -> bool:
    return _shared_is_retryable_server_read_error(
        exc,
        errnos=TELEGRAM_DELIVERY_RETRYABLE_ERRNOS,
        transport_markers=TELEGRAM_DELIVERY_RETRYABLE_MARKERS,
        server_markers=RETRYABLE_SERVER_READ_MARKERS,
    )


def _is_missing_session_server_read_error(exc: BaseException) -> bool:
    text = str(exc).strip().lower()
    return bool(text) and any(marker in text for marker in MISSING_SESSION_SERVER_READ_MARKERS)


def _background_sync_backoff_delay_seconds(failure_streak: int) -> float:
    base_seconds = max(float(TELEGRAM_BACKGROUND_SYNC_FAILURE_BACKOFF_BASE_SECONDS), 0.0)
    max_seconds = max(float(TELEGRAM_BACKGROUND_SYNC_FAILURE_BACKOFF_MAX_SECONDS), base_seconds)
    threshold = max(int(TELEGRAM_BACKGROUND_SYNC_FAILURE_BACKOFF_THRESHOLD), 1)
    retry_index = max(int(failure_streak) - threshold, 0)
    return min(max_seconds, _shared_retry_delay_seconds(base_seconds, retry_index))


def _deferred_reply_recovery_delay_seconds(attempt: int) -> float:
    return _shared_retry_delay_seconds(float(DEFERRED_REPLY_RECOVERY_BASE_DELAY_SECONDS), attempt)


def _is_definite_server_write_no_connection_error(exc: BaseException) -> bool:
    for current in _shared_exception_chain(exc):
        if isinstance(current, ConnectionRefusedError):
            return True
        if isinstance(current, OSError) and getattr(current, "errno", None) in {61, 101, 111, 113}:
            return True
    text = str(exc).strip().lower()
    if not text or "post " not in text:
        return False
    return any(
        marker in text
        for marker in (
            "connection refused",
            "failed to establish a new connection",
            "network is unreachable",
            "no route to host",
        )
    )


@dataclass(frozen=True)
class TelegramRuntimeSnapshot:
    target: AgentRuntimeTarget
    local_runtime: LocalAitRuntime | None = None

    @property
    def mode(self) -> str:
        return self.target.mode

    @property
    def repo_root(self) -> Path:
        return self.target.repo_root

    @property
    def repo_name(self) -> str:
        return self.target.repo_name

    @property
    def remote_name(self) -> str | None:
        return self.target.remote_name

    @property
    def server_url(self) -> str | None:
        return self.target.server_url

    @property
    def signature(self) -> str:
        return _runtime_backend_signature(
            self.target.mode,
            self.target.remote_name,
            self.target.server_url,
        )



from .transport_io import (
    _async_bytes_request,
    _async_json_request,
    _async_multipart_json_request,
    _bytes_request as _transport_bytes_request,
    _clean_optional_str,
    _compact_mapping,
    _consume_pending_termination_context,
    _json_request as _transport_json_request,
    _multipart_json_request as _transport_multipart_json_request,
    _parse_bool,
    _parse_int,
    _positive_int,
    _runtime_backend_signature,
    _runtime_link_fields,
    _runtime_signature_from_link,
    _termination_context_path,
    parse_webhook_payload,
)


def _json_request(
    url: str,
    *,
    method: str = "GET",
    payload: dict[str, Any] | None = None,
    headers: Mapping[str, str] | None = None,
    timeout: float | None = 20.0,
) -> Any:
    return _transport_json_request(
        url,
        method=method,
        payload=payload,
        headers=headers,
        timeout=timeout,
        request_cls=Request,
        urlopen_impl=urlopen,
    )


def _multipart_json_request(
    url: str,
    *,
    fields: Mapping[str, object],
    file_field: str,
    file_name: str,
    file_bytes: bytes,
    mime_type: str,
    headers: Mapping[str, str] | None = None,
    timeout: float | None = 20.0,
) -> Any:
    return _transport_multipart_json_request(
        url,
        fields=fields,
        file_field=file_field,
        file_name=file_name,
        file_bytes=file_bytes,
        mime_type=mime_type,
        headers=headers,
        timeout=timeout,
        request_cls=Request,
        urlopen_impl=urlopen,
    )


def _bytes_request(
    url: str,
    *,
    method: str = "GET",
    headers: Mapping[str, str] | None = None,
    timeout: float | None = 20.0,
) -> bytes:
    return _transport_bytes_request(
        url,
        method=method,
        headers=headers,
        timeout=timeout,
        request_cls=Request,
        urlopen_impl=urlopen,
    )


class TelegramApiClient:
    def __init__(self, config: BotConfig):
        self.config = config
        self.base_url = f"https://api.telegram.org/bot{config.token}"

    def _retry_delivery(self, action: Callable[[], Any]) -> Any:
        return _shared_retry_transport_operation(
            action,
            attempts=TELEGRAM_DELIVERY_RETRY_ATTEMPTS,
            base_delay_seconds=TELEGRAM_DELIVERY_RETRY_BASE_DELAY_SECONDS,
            retry_filter=_is_retryable_telegram_delivery_error,
        )

    def _post_json(self, method_name: str, payload: dict[str, Any]) -> Any:
        return self._retry_delivery(
            lambda: _json_request(
                f"{self.base_url}/{method_name}",
                method="POST",
                payload=payload,
                timeout=self.config.request_timeout_seconds,
            )
        )

    def _post_multipart(
        self,
        method_name: str,
        *,
        fields: Mapping[str, object],
        file_field: str,
        file_name: str,
        file_bytes: bytes,
        mime_type: str,
    ) -> Any:
        return self._retry_delivery(
            lambda: _multipart_json_request(
                f"{self.base_url}/{method_name}",
                fields=fields,
                file_field=file_field,
                file_name=file_name,
                file_bytes=file_bytes,
                mime_type=mime_type,
                timeout=self.config.request_timeout_seconds,
            )
        )

    def get_updates(self, *, offset: int, timeout_seconds: int) -> list[dict[str, Any]]:
        query = urlencode({"offset": offset, "timeout": timeout_seconds, "allowed_updates": json.dumps(["message"])})
        payload = _shared_retry_transport_operation(
            lambda: _json_request(
                f"{self.base_url}/getUpdates?{query}",
                timeout=_shared_timeout_value(self.config.request_timeout_seconds, minimum=timeout_seconds + 10),
            ),
            attempts=TELEGRAM_POLL_RETRY_ATTEMPTS,
            base_delay_seconds=TELEGRAM_POLL_RETRY_BASE_DELAY_SECONDS,
            retry_filter=_shared_is_retryable_transport_error,
        )
        if not payload or not payload.get("ok"):
            raise BotRuntimeError(f"Telegram getUpdates failed: {payload!r}")
        return list(payload.get("result") or [])

    def get_file(self, file_id: str) -> dict[str, Any]:
        payload = self._post_json("getFile", {"file_id": file_id})
        result = payload.get("result") if isinstance(payload, Mapping) else None
        if not payload or not payload.get("ok") or not isinstance(result, Mapping):
            raise BotRuntimeError(f"Telegram getFile failed: {payload!r}")
        return dict(result)

    def download_file_bytes(self, file_path: str) -> bytes:
        normalized_file_path = str(file_path or "").strip().lstrip("/")
        if not normalized_file_path:
            raise BotRuntimeError("Telegram file download requires a file_path.")
        file_url = f"https://api.telegram.org/file/bot{self.config.token}/{normalized_file_path}"
        return self._retry_delivery(
            lambda: _bytes_request(
                file_url,
                timeout=self.config.request_timeout_seconds,
            )
        )

    def send_message(self, chat_id: str | int, text: str) -> None:
        for chunk in _telegram_message_chunks(self.config, text):
            payload = {
                "chat_id": chat_id,
                "text": chunk.text,
                "disable_web_page_preview": True,
            }
            if chunk.parse_mode:
                payload["parse_mode"] = chunk.parse_mode
            try:
                response = self._post_json("sendMessage", payload)
            except BotRuntimeError as exc:
                if not chunk.parse_mode or not _markdownish_parse_error(exc):
                    raise
                response = self._post_json(
                    "sendMessage",
                    {
                        "chat_id": chat_id,
                        "text": chunk.plain_text,
                        "disable_web_page_preview": True,
                    },
                )
            if not response or not response.get("ok"):
                raise BotRuntimeError(f"Telegram sendMessage failed: {response!r}")

    def _send_attachment_payload(
        self,
        *,
        method_name: str,
        file_field: str,
        chat_id: str | int,
        attachment: Mapping[str, Any],
        extra_fields: Mapping[str, object] | None = None,
    ) -> None:
        payload: dict[str, object] = {"chat_id": chat_id}
        if extra_fields:
            payload.update(dict(extra_fields))
        caption = _clean_optional_str(attachment.get("caption"))
        if caption:
            payload["caption"] = caption
        for key in ("title", "performer"):
            value = _clean_optional_str(attachment.get(key))
            if value:
                payload[key] = value
        duration_seconds = _positive_int(attachment.get("duration_seconds") or attachment.get("duration"))
        if duration_seconds is not None:
            payload["duration"] = duration_seconds
        source = (
            _clean_optional_str(attachment.get("telegram_file_id"))
            or _clean_optional_str(attachment.get("url"))
            or None
        )
        if source is not None:
            payload[file_field] = source
            response = self._post_json(method_name, payload)
            if not response or not response.get("ok"):
                raise BotRuntimeError(f"Telegram {method_name} failed: {response!r}")
            return
        local_path_value = _clean_optional_str(attachment.get("local_path"))
        if local_path_value is None:
            raise BotRuntimeError(
                f"Telegram {method_name} attachment requires one of telegram_file_id, url, or local_path."
            )
        local_path = Path(local_path_value).expanduser()
        if not local_path.exists() or not local_path.is_file():
            raise BotRuntimeError(f"Telegram attachment path does not exist: {local_path}")
        file_name = _clean_optional_str(attachment.get("file_name")) or local_path.name
        mime_type = _clean_optional_str(attachment.get("mime_type")) or mimetypes.guess_type(file_name)[0] or (
            "application/octet-stream"
        )
        response = self._post_multipart(
            method_name,
            fields=payload,
            file_field=file_field,
            file_name=file_name,
            file_bytes=local_path.read_bytes(),
            mime_type=mime_type,
        )
        if not response or not response.get("ok"):
            raise BotRuntimeError(f"Telegram {method_name} failed: {response!r}")

    def send_audio(self, chat_id: str | int, attachment: Mapping[str, Any]) -> None:
        self._send_attachment_payload(
            method_name="sendAudio",
            file_field="audio",
            chat_id=chat_id,
            attachment=attachment,
        )

    def send_document(self, chat_id: str | int, attachment: Mapping[str, Any]) -> None:
        self._send_attachment_payload(
            method_name="sendDocument",
            file_field="document",
            chat_id=chat_id,
            attachment=attachment,
        )


class AsyncTelegramApiClient:
    def __init__(self, config: BotConfig, *, http_client: httpx.AsyncClient | None = None):
        self.config = config
        self.base_url = f"https://api.telegram.org/bot{config.token}"
        self._http_client = http_client
        self._owns_http_client = http_client is None

    def _client(self) -> httpx.AsyncClient:
        if self._http_client is None:
            self._http_client = httpx.AsyncClient()
        return self._http_client

    async def aclose(self) -> None:
        if self._owns_http_client and self._http_client is not None:
            await self._http_client.aclose()
        self._http_client = None

    async def _retry_delivery(self, action: Callable[[], Any]) -> Any:
        return await _shared_retry_transport_operation_async(
            action,
            attempts=TELEGRAM_DELIVERY_RETRY_ATTEMPTS,
            base_delay_seconds=TELEGRAM_DELIVERY_RETRY_BASE_DELAY_SECONDS,
            retry_filter=_is_retryable_telegram_delivery_error,
        )

    async def _post_json(self, method_name: str, payload: dict[str, Any]) -> Any:
        async def _action() -> Any:
            return await _async_json_request(
                f"{self.base_url}/{method_name}",
                method="POST",
                payload=payload,
                timeout=self.config.request_timeout_seconds,
                client=self._client(),
            )

        return await self._retry_delivery(_action)

    async def _post_multipart(
        self,
        method_name: str,
        *,
        fields: Mapping[str, object],
        file_field: str,
        file_name: str,
        file_bytes: bytes,
        mime_type: str,
    ) -> Any:
        async def _action() -> Any:
            return await _async_multipart_json_request(
                f"{self.base_url}/{method_name}",
                fields=fields,
                file_field=file_field,
                file_name=file_name,
                file_bytes=file_bytes,
                mime_type=mime_type,
                timeout=self.config.request_timeout_seconds,
                client=self._client(),
            )

        return await self._retry_delivery(_action)

    async def get_updates(self, *, offset: int, timeout_seconds: int) -> list[dict[str, Any]]:
        query = urlencode({"offset": offset, "timeout": timeout_seconds, "allowed_updates": json.dumps(["message"])})

        async def _action() -> Any:
            return await _async_json_request(
                f"{self.base_url}/getUpdates?{query}",
                timeout=_shared_timeout_value(self.config.request_timeout_seconds, minimum=timeout_seconds + 10),
                client=self._client(),
            )

        payload = await _shared_retry_transport_operation_async(
            _action,
            attempts=TELEGRAM_POLL_RETRY_ATTEMPTS,
            base_delay_seconds=TELEGRAM_POLL_RETRY_BASE_DELAY_SECONDS,
            retry_filter=_shared_is_retryable_transport_error,
        )
        if not payload or not payload.get("ok"):
            raise BotRuntimeError(f"Telegram getUpdates failed: {payload!r}")
        return list(payload.get("result") or [])

    async def get_file(self, file_id: str) -> dict[str, Any]:
        payload = await self._post_json("getFile", {"file_id": file_id})
        result = payload.get("result") if isinstance(payload, Mapping) else None
        if not payload or not payload.get("ok") or not isinstance(result, Mapping):
            raise BotRuntimeError(f"Telegram getFile failed: {payload!r}")
        return dict(result)

    async def download_file_bytes(self, file_path: str) -> bytes:
        normalized_file_path = str(file_path or "").strip().lstrip("/")
        if not normalized_file_path:
            raise BotRuntimeError("Telegram file download requires a file_path.")
        file_url = f"https://api.telegram.org/file/bot{self.config.token}/{normalized_file_path}"

        async def _action() -> bytes:
            return await _async_bytes_request(
                file_url,
                timeout=self.config.request_timeout_seconds,
                client=self._client(),
            )

        return await self._retry_delivery(_action)

    async def send_message(self, chat_id: str | int, text: str) -> None:
        for chunk in _telegram_message_chunks(self.config, text):
            payload = {
                "chat_id": chat_id,
                "text": chunk.text,
                "disable_web_page_preview": True,
            }
            if chunk.parse_mode:
                payload["parse_mode"] = chunk.parse_mode
            try:
                response = await self._post_json("sendMessage", payload)
            except BotRuntimeError as exc:
                if not chunk.parse_mode or not _markdownish_parse_error(exc):
                    raise
                response = await self._post_json(
                    "sendMessage",
                    {
                        "chat_id": chat_id,
                        "text": chunk.plain_text,
                        "disable_web_page_preview": True,
                    },
                )
            if not response or not response.get("ok"):
                raise BotRuntimeError(f"Telegram sendMessage failed: {response!r}")

    async def _send_attachment_payload(
        self,
        *,
        method_name: str,
        file_field: str,
        chat_id: str | int,
        attachment: Mapping[str, Any],
        extra_fields: Mapping[str, object] | None = None,
    ) -> None:
        payload: dict[str, object] = {"chat_id": chat_id}
        if extra_fields:
            payload.update(dict(extra_fields))
        caption = _clean_optional_str(attachment.get("caption"))
        if caption:
            payload["caption"] = caption
        for key in ("title", "performer"):
            value = _clean_optional_str(attachment.get(key))
            if value:
                payload[key] = value
        duration_seconds = _positive_int(attachment.get("duration_seconds") or attachment.get("duration"))
        if duration_seconds is not None:
            payload["duration"] = duration_seconds
        source = (
            _clean_optional_str(attachment.get("telegram_file_id"))
            or _clean_optional_str(attachment.get("url"))
            or None
        )
        if source is not None:
            payload[file_field] = source
            response = await self._post_json(method_name, payload)
            if not response or not response.get("ok"):
                raise BotRuntimeError(f"Telegram {method_name} failed: {response!r}")
            return
        local_path_value = _clean_optional_str(attachment.get("local_path"))
        if local_path_value is None:
            raise BotRuntimeError(
                f"Telegram {method_name} attachment requires one of telegram_file_id, url, or local_path."
            )
        local_path = Path(local_path_value).expanduser()
        if not local_path.exists() or not local_path.is_file():
            raise BotRuntimeError(f"Telegram attachment path does not exist: {local_path}")
        file_name = _clean_optional_str(attachment.get("file_name")) or local_path.name
        mime_type = _clean_optional_str(attachment.get("mime_type")) or mimetypes.guess_type(file_name)[0] or (
            "application/octet-stream"
        )
        file_bytes = await asyncio.to_thread(local_path.read_bytes)
        response = await self._post_multipart(
            method_name,
            fields=payload,
            file_field=file_field,
            file_name=file_name,
            file_bytes=file_bytes,
            mime_type=mime_type,
        )
        if not response or not response.get("ok"):
            raise BotRuntimeError(f"Telegram {method_name} failed: {response!r}")

    async def send_audio(self, chat_id: str | int, attachment: Mapping[str, Any]) -> None:
        await self._send_attachment_payload(
            method_name="sendAudio",
            file_field="audio",
            chat_id=chat_id,
            attachment=attachment,
        )

    async def send_document(self, chat_id: str | int, attachment: Mapping[str, Any]) -> None:
        await self._send_attachment_payload(
            method_name="sendDocument",
            file_field="document",
            chat_id=chat_id,
            attachment=attachment,
        )


class AitApiClient:
    def __init__(self, config: BotConfig):
        self.config = config
        self._local_runtime_cache: dict[str, LocalAitRuntime] = {}

    def capture_runtime_snapshot(self) -> TelegramRuntimeSnapshot:
        repo_root = _runtime_repo_root(self.config)
        try:
            target = resolve_agent_runtime_target(repo_root)
        except AgentRuntimeConfigError as exc:
            raise BotRuntimeError(str(exc)) from exc
        local_runtime: LocalAitRuntime | None = None
        if target.mode == "local":
            cache_key = str(target.repo_root)
            local_runtime = self._local_runtime_cache.get(cache_key)
            if local_runtime is None:
                local_runtime = LocalAitRuntime(target)
                self._local_runtime_cache[cache_key] = local_runtime
        return TelegramRuntimeSnapshot(target=target, local_runtime=local_runtime)

    def _headers(self, actor_identity: str, actor_type: str = "telegram_bot") -> dict[str, str]:
        return {
            "X-AIT-Actor": actor_identity,
            "X-AIT-Actor-Type": actor_type,
        }

    def _local_call(self, runtime_snapshot: TelegramRuntimeSnapshot, fn_name: str, /, *args: Any, **kwargs: Any) -> Any:
        runtime = runtime_snapshot.local_runtime
        if runtime is None:
            raise BotRuntimeError("Telegram worker local runtime is unavailable.")
        try:
            return getattr(runtime, fn_name)(*args, **kwargs)
        except (AgentRuntimeConfigError, KeyError, ValueError) as exc:
            raise BotRuntimeError(str(exc)) from exc

    def _request(
        self,
        method: str,
        path: str,
        *,
        payload: dict[str, Any] | None = None,
        actor_identity: str = "ait-agent-telegram",
        actor_type: str = "telegram_bot",
        allow_retry: bool = False,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> Any:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.server_url is None:
            raise BotRuntimeError("Telegram worker is in local runtime mode and cannot issue remote HTTP requests.")
        request_url = f"{snapshot.server_url}{path}"
        action = lambda: _json_request(
            request_url,
            method=method,
            payload=payload,
            headers=self._headers(actor_identity, actor_type),
            timeout=self.config.request_timeout_seconds,
        )
        if allow_retry and snapshot.server_url and _shared_is_loopback_url(snapshot.server_url):
            return _shared_retry_transport_operation(
                action,
                attempts=AIT_SERVER_READ_RETRY_ATTEMPTS,
                base_delay_seconds=AIT_SERVER_READ_RETRY_BASE_DELAY_SECONDS,
                retry_filter=_is_retryable_server_read_error,
            )
        return action()

    def create_session(
        self,
        *,
        chat_id: str,
        chat_title: str | None,
        chat_type: str | None,
        session_kind: str = "telegram_chat",
        title_prefix: str = "Telegram chat",
        binding_role: str | None = None,
        canonical_session_id: str | None = None,
        active_session_id: str | None = None,
        branch_session_id: str | None = None,
        branch_kind: str | None = None,
        relink_reason: str | None = None,
        metadata_extra: dict[str, Any] | None = None,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        repo_root = snapshot.repo_root
        metadata_extra = dict(metadata_extra or {})
        metadata_extra.update(
            build_transport_binding_metadata(
                transport="telegram",
                surface_id=chat_id,
                surface_title=chat_title,
                surface_kind=chat_type,
                binding_role=binding_role,
                canonical_session_id=canonical_session_id,
                active_session_id=active_session_id,
                branch_session_id=branch_session_id,
                branch_kind=branch_kind,
                relink_reason=relink_reason,
                reply_target={"channel_id": str(chat_id)},
            )
        )
        metadata = build_transport_session_metadata(
            transport="telegram",
            channel_id=chat_id,
            channel_title=chat_title,
            channel_kind=chat_type,
            linked_via="ait-agent telegram",
            metadata_extra={
                "telegram_chat_id": str(chat_id),
                "telegram_chat_title": chat_title,
                "telegram_chat_type": chat_type,
                "repo_root": str(repo_root),
                "workspace_root": str(repo_root),
                **metadata_extra,
            },
        )
        payload = {
            "session_kind": session_kind,
            "title": f"{title_prefix} · {chat_title or chat_id}",
            "metadata": metadata,
        }
        if snapshot.local_runtime is not None:
            return self._local_call(
                snapshot,
                "create_session",
                session_kind=session_kind,
                title=payload["title"],
                metadata=metadata,
            )
        return self._request(
            "POST",
            f"/v1/native/repositories/{snapshot.repo_name}/sessions",
            payload=payload,
            runtime_snapshot=snapshot,
        )

    def get_session(
        self,
        session_id: str,
        *,
        actor_identity: str = "ait-agent-telegram",
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return self._local_call(snapshot, "get_session", session_id)
        return self._request(
            "GET",
            f"/v1/native/sessions/{session_id}",
            actor_identity=actor_identity,
            allow_retry=True,
            runtime_snapshot=snapshot,
        )

    def append_session_event(
        self,
        session_id: str,
        *,
        event_type: str,
        payload: dict[str, Any],
        actor_identity: str,
        actor_type: str,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return self._local_call(
                snapshot,
                "append_session_event",
                session_id,
                event_type=event_type,
                payload=payload,
                actor_identity=actor_identity,
                actor_type=actor_type,
            )
        return self._request(
            "POST",
            f"/v1/native/sessions/{session_id}/events",
            payload={"event_type": event_type, "payload": payload},
            actor_identity=actor_identity,
            actor_type=actor_type,
            runtime_snapshot=snapshot,
        )

    def list_session_events(
        self,
        session_id: str,
        *,
        after_sequence: int,
        limit: int = 50,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> list[dict[str, Any]]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return list(
                self._local_call(
                    snapshot,
                    "list_session_events",
                    session_id,
                    after_sequence=after_sequence,
                    limit=limit,
                )
                or []
            )
        query = urlencode({"after_sequence": after_sequence, "limit": limit})
        return list(
            self._request(
                "GET",
                f"/v1/native/sessions/{session_id}/events?{query}",
                allow_retry=True,
                runtime_snapshot=snapshot,
            )
            or []
        )

    def read_task_queue(self, *, runtime_snapshot: TelegramRuntimeSnapshot | None = None) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return self._local_call(snapshot, "read_task_queue")
        query = urlencode({"repo_name": snapshot.repo_name, "status": "active"})
        return self._request("GET", f"/v1/native/read/task-queue?{query}", allow_retry=True, runtime_snapshot=snapshot)

    def read_task(self, task_id: str, *, runtime_snapshot: TelegramRuntimeSnapshot | None = None) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return self._local_call(snapshot, "read_task", task_id)
        return self._request(
            "GET",
            f"/v1/native/repositories/{snapshot.repo_name}/read/tasks/{task_id}",
            allow_retry=True,
            runtime_snapshot=snapshot,
        )

    def read_task_audit(
        self,
        task_id: str,
        *,
        target_line: str = "main",
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return self._local_call(snapshot, "read_task_audit", task_id, target_line=target_line)
        query = urlencode({"target_line": target_line})
        return self._request(
            "GET",
            f"/v1/native/repositories/{snapshot.repo_name}/read/tasks/{task_id}/audit?{query}",
            allow_retry=True,
            runtime_snapshot=snapshot,
        )

    def read_change(self, change_id: str, *, runtime_snapshot: TelegramRuntimeSnapshot | None = None) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return self._local_call(snapshot, "read_change", change_id)
        return self._request(
            "GET",
            f"/v1/native/repositories/{snapshot.repo_name}/read/changes/{change_id}",
            allow_retry=True,
            runtime_snapshot=snapshot,
        )

    def read_task_dag_progress(
        self,
        graph: dict[str, Any],
        *,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return self._local_call(snapshot, "read_task_dag_progress", graph)
        return self._request(
            "POST",
            "/v1/native/read/task-dag-progress",
            payload={"graph": graph},
            allow_retry=True,
            runtime_snapshot=snapshot,
        )

    def create_telegram_turn(
        self,
        session_id: str,
        *,
        text: str,
        chat_id: str | int,
        chat_title: str,
        chat_type: str | None,
        telegram_message_id: int | None,
        telegram_message_ids: list[int] | tuple[int, ...] | None = None,
        transport_envelope: dict[str, Any] | None = None,
        actor_identity: str,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        payload = {
            "text": text,
            "chat_id": str(chat_id),
            "chat_title": chat_title,
            "chat_type": chat_type,
            "telegram_message_id": telegram_message_id,
        }
        if telegram_message_ids:
            payload["telegram_message_ids"] = list(telegram_message_ids)
        if transport_envelope:
            payload["transport_envelope"] = dict(transport_envelope)
        if snapshot.local_runtime is not None:
            return self._local_call(
                snapshot,
                "create_telegram_turn",
                session_id,
                text=text,
                chat_id=chat_id,
                chat_title=chat_title,
                chat_type=chat_type,
                telegram_message_id=telegram_message_id,
                telegram_message_ids=telegram_message_ids,
                transport_envelope=transport_envelope,
                actor_identity=actor_identity,
            )
        return self._request(
            "POST",
            f"/v1/native/sessions/{session_id}:telegramTurn",
            payload=payload,
            actor_identity=actor_identity,
            actor_type="telegram_user",
            allow_retry=True,
            runtime_snapshot=snapshot,
        )


class AsyncAitApiClient(AitApiClient):
    def __init__(self, config: BotConfig, *, http_client: httpx.AsyncClient | None = None):
        super().__init__(config)
        self._http_client = http_client
        self._owns_http_client = http_client is None

    def _client(self) -> httpx.AsyncClient:
        if self._http_client is None:
            self._http_client = httpx.AsyncClient()
        return self._http_client

    async def aclose(self) -> None:
        if self._owns_http_client and self._http_client is not None:
            await self._http_client.aclose()
        self._http_client = None

    async def _local_call_async(
        self,
        runtime_snapshot: TelegramRuntimeSnapshot,
        fn_name: str,
        /,
        *args: Any,
        **kwargs: Any,
    ) -> Any:
        return await asyncio.to_thread(self._local_call, runtime_snapshot, fn_name, *args, **kwargs)

    async def _request_async(
        self,
        method: str,
        path: str,
        *,
        payload: dict[str, Any] | None = None,
        actor_identity: str = "ait-agent-telegram",
        actor_type: str = "telegram_bot",
        allow_retry: bool = False,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> Any:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.server_url is None:
            raise BotRuntimeError("Telegram worker is in local runtime mode and cannot issue remote HTTP requests.")
        request_url = f"{snapshot.server_url}{path}"

        async def _action() -> Any:
            return await _async_json_request(
                request_url,
                method=method,
                payload=payload,
                headers=self._headers(actor_identity, actor_type),
                timeout=self.config.request_timeout_seconds,
                client=self._client(),
            )

        if allow_retry and snapshot.server_url and _shared_is_loopback_url(snapshot.server_url):
            return await _shared_retry_transport_operation_async(
                _action,
                attempts=AIT_SERVER_READ_RETRY_ATTEMPTS,
                base_delay_seconds=AIT_SERVER_READ_RETRY_BASE_DELAY_SECONDS,
                retry_filter=_is_retryable_server_read_error,
            )
        return await _action()

    async def create_session(
        self,
        *,
        chat_id: str,
        chat_title: str | None,
        chat_type: str | None,
        session_kind: str = "telegram_chat",
        title_prefix: str = "Telegram chat",
        binding_role: str | None = None,
        canonical_session_id: str | None = None,
        active_session_id: str | None = None,
        branch_session_id: str | None = None,
        branch_kind: str | None = None,
        relink_reason: str | None = None,
        metadata_extra: dict[str, Any] | None = None,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        repo_root = snapshot.repo_root
        metadata_extra = dict(metadata_extra or {})
        metadata_extra.update(
            build_transport_binding_metadata(
                transport="telegram",
                surface_id=chat_id,
                surface_title=chat_title,
                surface_kind=chat_type,
                binding_role=binding_role,
                canonical_session_id=canonical_session_id,
                active_session_id=active_session_id,
                branch_session_id=branch_session_id,
                branch_kind=branch_kind,
                relink_reason=relink_reason,
                reply_target={"channel_id": str(chat_id)},
            )
        )
        metadata = build_transport_session_metadata(
            transport="telegram",
            channel_id=chat_id,
            channel_title=chat_title,
            channel_kind=chat_type,
            linked_via="ait-agent telegram",
            metadata_extra={
                "telegram_chat_id": str(chat_id),
                "telegram_chat_title": chat_title,
                "telegram_chat_type": chat_type,
                "repo_root": str(repo_root),
                "workspace_root": str(repo_root),
                **metadata_extra,
            },
        )
        payload = {
            "session_kind": session_kind,
            "title": f"{title_prefix} · {chat_title or chat_id}",
            "metadata": metadata,
        }
        if snapshot.local_runtime is not None:
            return await self._local_call_async(
                snapshot,
                "create_session",
                session_kind=session_kind,
                title=payload["title"],
                metadata=metadata,
            )
        return await self._request_async(
            "POST",
            f"/v1/native/repositories/{snapshot.repo_name}/sessions",
            payload=payload,
            runtime_snapshot=snapshot,
        )

    async def get_session(
        self,
        session_id: str,
        *,
        actor_identity: str = "ait-agent-telegram",
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return await self._local_call_async(snapshot, "get_session", session_id)
        return await self._request_async(
            "GET",
            f"/v1/native/sessions/{session_id}",
            actor_identity=actor_identity,
            allow_retry=True,
            runtime_snapshot=snapshot,
        )

    async def append_session_event(
        self,
        session_id: str,
        *,
        event_type: str,
        payload: dict[str, Any],
        actor_identity: str,
        actor_type: str,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return await self._local_call_async(
                snapshot,
                "append_session_event",
                session_id,
                event_type=event_type,
                payload=payload,
                actor_identity=actor_identity,
                actor_type=actor_type,
            )
        return await self._request_async(
            "POST",
            f"/v1/native/sessions/{session_id}/events",
            payload={"event_type": event_type, "payload": payload},
            actor_identity=actor_identity,
            actor_type=actor_type,
            runtime_snapshot=snapshot,
        )

    async def list_session_events(
        self,
        session_id: str,
        *,
        after_sequence: int,
        limit: int = 50,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> list[dict[str, Any]]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return list(
                await self._local_call_async(
                    snapshot,
                    "list_session_events",
                    session_id,
                    after_sequence=after_sequence,
                    limit=limit,
                )
                or []
            )
        query = urlencode({"after_sequence": after_sequence, "limit": limit})
        return list(
            await self._request_async(
                "GET",
                f"/v1/native/sessions/{session_id}/events?{query}",
                allow_retry=True,
                runtime_snapshot=snapshot,
            )
            or []
        )

    async def read_task_queue(self, *, runtime_snapshot: TelegramRuntimeSnapshot | None = None) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return await self._local_call_async(snapshot, "read_task_queue")
        query = urlencode({"repo_name": snapshot.repo_name, "status": "active"})
        return await self._request_async(
            "GET",
            f"/v1/native/read/task-queue?{query}",
            allow_retry=True,
            runtime_snapshot=snapshot,
        )

    async def read_task(self, task_id: str, *, runtime_snapshot: TelegramRuntimeSnapshot | None = None) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return await self._local_call_async(snapshot, "read_task", task_id)
        return await self._request_async(
            "GET",
            f"/v1/native/repositories/{snapshot.repo_name}/read/tasks/{task_id}",
            allow_retry=True,
            runtime_snapshot=snapshot,
        )

    async def read_task_audit(
        self,
        task_id: str,
        *,
        target_line: str = "main",
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return await self._local_call_async(snapshot, "read_task_audit", task_id, target_line=target_line)
        query = urlencode({"target_line": target_line})
        return await self._request_async(
            "GET",
            f"/v1/native/repositories/{snapshot.repo_name}/read/tasks/{task_id}/audit?{query}",
            allow_retry=True,
            runtime_snapshot=snapshot,
        )

    async def read_change(
        self,
        change_id: str,
        *,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return await self._local_call_async(snapshot, "read_change", change_id)
        return await self._request_async(
            "GET",
            f"/v1/native/repositories/{snapshot.repo_name}/read/changes/{change_id}",
            allow_retry=True,
            runtime_snapshot=snapshot,
        )

    async def read_task_dag_progress(
        self,
        graph: dict[str, Any],
        *,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        if snapshot.local_runtime is not None:
            return await self._local_call_async(snapshot, "read_task_dag_progress", graph)
        return await self._request_async(
            "POST",
            "/v1/native/read/task-dag-progress",
            payload={"graph": graph},
            allow_retry=True,
            runtime_snapshot=snapshot,
        )

    async def create_telegram_turn(
        self,
        session_id: str,
        *,
        text: str,
        chat_id: str | int,
        chat_title: str,
        chat_type: str | None,
        telegram_message_id: int | None,
        telegram_message_ids: list[int] | tuple[int, ...] | None = None,
        transport_envelope: dict[str, Any] | None = None,
        actor_identity: str,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any]:
        snapshot = runtime_snapshot or self.capture_runtime_snapshot()
        payload = {
            "text": text,
            "chat_id": str(chat_id),
            "chat_title": chat_title,
            "chat_type": chat_type,
            "telegram_message_id": telegram_message_id,
        }
        if telegram_message_ids:
            payload["telegram_message_ids"] = list(telegram_message_ids)
        if transport_envelope:
            payload["transport_envelope"] = dict(transport_envelope)
        if snapshot.local_runtime is not None:
            return await self._local_call_async(
                snapshot,
                "create_telegram_turn",
                session_id,
                text=text,
                chat_id=chat_id,
                chat_title=chat_title,
                chat_type=chat_type,
                telegram_message_id=telegram_message_id,
                telegram_message_ids=telegram_message_ids,
                transport_envelope=transport_envelope,
                actor_identity=actor_identity,
            )
        return await self._request_async(
            "POST",
            f"/v1/native/sessions/{session_id}:telegramTurn",
            payload=payload,
            actor_identity=actor_identity,
            actor_type="telegram_user",
            allow_retry=True,
            runtime_snapshot=snapshot,
        )




def _is_model_capacity_error_text(text: str) -> bool:
    lowered = str(text or "").lower()
    return "model is at capacity" in lowered or "selected model is at capacity" in lowered

def _telegram_bootstrap_owner_user_id(state: Mapping[str, Any] | None) -> str | None:
    owner_user_id = str((state or {}).get("owner_user_id") or "").strip()
    return owner_user_id or None


def _telegram_bootstrap_pending_user_id(state: Mapping[str, Any] | None) -> str | None:
    pending_user_id = str((state or {}).get("pending_user_id") or "").strip()
    return pending_user_id or None


def _telegram_bootstrap_failed_attempts(state: Mapping[str, Any] | None) -> dict[str, int]:
    raw = (state or {}).get("failed_attempts")
    if not isinstance(raw, Mapping):
        return {}
    attempts: dict[str, int] = {}
    for user_id, value in raw.items():
        try:
            count = int(value)
        except (TypeError, ValueError):
            continue
        if count > 0:
            attempts[str(user_id)] = count
    return attempts


def _telegram_bootstrap_blacklist(state: Mapping[str, Any] | None) -> dict[str, dict[str, Any]]:
    raw = (state or {}).get("blacklist")
    if not isinstance(raw, Mapping):
        return {}
    blacklist: dict[str, dict[str, Any]] = {}
    for user_id, payload in raw.items():
        blacklist[str(user_id)] = dict(payload) if isinstance(payload, Mapping) else {}
    return blacklist


class TelegramBotService:
    def __init__(
        self,
        config: BotConfig,
        ait_api: AitApiClient | None = None,
        telegram_api: TelegramApiClient | None = None,
        state_store: TelegramSyncStateStore | None = None,
        event_trigger_registry: EventTriggerRegistry | None = None,
        speech_to_text_runtime: LocalSpeechToTextRuntime | None = None,
    ):
        self.config = config
        self.ait_api = ait_api or AitApiClient(config)
        self.telegram_api = telegram_api or TelegramApiClient(config)
        self.state_store = state_store or TelegramSyncStateStore(config.sync_state_path)
        self.event_trigger_registry = event_trigger_registry or load_event_trigger_registry(_runtime_repo_root(config))
        self.speech_to_text_runtime = speech_to_text_runtime
        if self.speech_to_text_runtime is None and config.stt_mode == "local-stt":
            self.speech_to_text_runtime = LocalSpeechToTextRuntime(config, telegram_api=self.telegram_api)
        self._stop_requested = False
        self._dispatch_lock = threading.Lock()
        self._dispatchers: dict[str, ThreadPoolExecutor] = {}
        self._reply_dispatch_lock = threading.Lock()
        self._reply_dispatchers: dict[str, ThreadPoolExecutor] = {}
        self._dispatch_futures_lock = threading.Lock()
        self._dispatch_futures: set[Future[Any]] = set()
        self._state_lock = threading.Lock()
        self._update_dispatch = TelegramUpdateDispatch()
        self._logical_turn_buffer = TelegramLogicalTurnBuffer(
            username=self.config.username,
            merge_window_seconds=self.config.turn_merge_window_seconds,
            max_messages=self.config.turn_merge_max_messages,
            poll_interval_seconds=TURN_MERGE_POLL_INTERVAL_SECONDS,
            update_key=self._update_key,
            dispatch_key_for_chat=self._dispatch_key_for_chat,
            normalize_user_text=normalize_user_text,
            parse_command=parse_command,
            detect_workflow_query=detect_workflow_query,
            actor_identity=_actor_identity,
            skip_logical_turn=_SKIP_LOGICAL_TURN,
        )
        self._owner_bootstrap_gate = TelegramOwnerBootstrapGate(
            config=self.config,
            telegram_api=self.telegram_api,
            state_get_chat=self._state_get_chat,
            state_get_bootstrap_auth=self._state_get_bootstrap_auth,
            state_save_bootstrap_auth=self._state_save_bootstrap_auth,
            now_iso=utc_now_iso,
            user_display_name=_user_display_name,
        )
        self._session_link_coordinator = TelegramSessionLinkCoordinator(
            config=self.config,
            ait_api_call=self._ait_api_call,
            state_get_chat=self._state_get_chat,
            state_upsert_chat=self._state_upsert_chat,
            state_patch_chat=self._state_patch_chat,
            runtime_snapshot=self._runtime_snapshot,
            now_iso=utc_now_iso,
            runtime_error_type=BotRuntimeError,
        )
        self._command_runtime = TelegramCommandRuntime(
            config=self.config,
            ait_api=self.ait_api,
            telegram_api=self.telegram_api,
            ensure_session_link=self._ensure_session_link,
            sync_session=self._sync_session,
            state_patch_chat=self._state_patch_chat,
            ait_api_call=self._ait_api_call,
            runtime_snapshot=self._runtime_snapshot,
            command_specs=TELEGRAM_COMMAND_SPECS,
            commands_by_name=TELEGRAM_COMMANDS_BY_NAME,
            workflow_query_examples=WORKFLOW_QUERY_EXAMPLES,
            task_id_pattern=TASK_ID_PATTERN,
            change_id_pattern=CHANGE_ID_PATTERN,
            now_iso=utc_now_iso,
            runtime_error_type=BotRuntimeError,
            custom_handlers={"watchgraph": self._handle_watchgraph_command},
        )
        self._graph_watch_manager = TelegramGraphWatchManager(
            config=self.config,
            ait_api=self.ait_api,
            telegram_api=self.telegram_api,
            state_store=self.state_store,
            ensure_session_link=self._ensure_session_link,
            missing_link_text=self._missing_link_text,
            state_patch_chat=self._state_patch_chat,
            watchgraph_usage=TELEGRAM_COMMANDS_BY_NAME["watchgraph"].usage,
            plan_id_pattern=PLAN_ID_PATTERN,
            runtime_repo_root=_runtime_repo_root,
        )
        self._live_reply_manager = TelegramLiveReplyManager(
            repo_name=self.config.repo_name,
            state_get_chat=self._state_get_chat,
            state_upsert_chat=self._state_upsert_chat,
            state_patch_chat=self._state_patch_chat,
            ait_api_call=self._ait_api_call,
            send_assistant_event_reply=self._send_assistant_event_reply,
            log_runtime_error=self._log_runtime_error,
            now_iso=utc_now_iso,
            retryable_server_read_error=_is_retryable_server_read_error,
            retryable_transport_error=_shared_is_retryable_transport_error,
            recovery_attempts=lambda: DEFERRED_REPLY_RECOVERY_ATTEMPTS,
            recovery_delay_seconds=_deferred_reply_recovery_delay_seconds,
            delivered_sequence_limit=200,
            watch_max_wait_seconds=lambda: DEFERRED_REPLY_WATCH_MAX_WAIT_SECONDS,
            watch_poll_interval_seconds=lambda: DEFERRED_REPLY_WATCH_POLL_INTERVAL_SECONDS,
        )
        self._reply_turn_spool = TelegramReplyTurnSpool(
            state_get_chat=self._state_get_chat,
            state_patch_chat=self._state_patch_chat,
            now_iso=utc_now_iso,
            spool_limit=TELEGRAM_REPLY_SPOOL_LIMIT,
        )
        self._reply_turn_manager = TelegramReplyTurnManager(
            config=self.config,
            telegram_api=self.telegram_api,
            runtime_snapshot=self._runtime_snapshot,
            handle_owner_bootstrap_gate=self._handle_owner_bootstrap_gate,
            match_fresh_topic_event_trigger=self._match_fresh_topic_event_trigger,
            create_fresh_session=self._create_fresh_session,
            fresh_topic_confirmation_text=self._fresh_topic_confirmation_text,
            state_get_chat=self._state_get_chat,
            state_patch_chat=self._state_patch_chat,
            ensure_session_link=self._ensure_session_link,
            submit_reply_serialized=self._submit_reply_serialized,
            dispatch_key_for_chat=self._dispatch_key_for_chat,
            ait_api_call=self._ait_api_call,
            advance_sync_cursor=self._advance_sync_cursor,
            mark_telegram_live_reply_delivered=self._mark_telegram_live_reply_delivered,
            remember_pending_reply_turn=self._remember_pending_reply_turn,
            clear_pending_reply_turn_spool_entry=self._clear_pending_reply_turn_spool_entry,
            recover_or_watch_completed_pending_reply=self._recover_or_watch_completed_pending_reply,
            actor_identity=_actor_identity,
            user_display_name=_user_display_name,
            runtime_error_type=BotRuntimeError,
            model_capacity_error_text=_is_model_capacity_error_text,
            log_runtime_error=self._log_runtime_error,
            safe_send_message=self._safe_send_message,
            now_iso=utc_now_iso,
        )

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
        next_background_sync_at = self._next_background_sync_deadline() if self.config.background_sync_enabled else None
        while not self._stop_requested:
            try:
                next_background_sync_at = self._run_due_background_sync(next_background_sync_at)
                state = self._state_load()
                offset = int(state.last_update_id or 0) + 1
                updates = self.telegram_api.get_updates(
                    offset=offset,
                    timeout_seconds=self._poll_timeout_seconds(next_background_sync_at),
                )
                if not updates:
                    continue
                for update in updates:
                    self.submit_update(update)
                    update_id = int(update.get("update_id") or 0)
                    if update_id:
                        self._state_update_last_update_id(update_id)
            except BotRuntimeError as exc:
                self._log_runtime_error("Telegram polling failed with bot runtime error.", exc)
                time.sleep(1.0)
            except Exception as exc:  # pragma: no cover - defensive daemon hardening
                self._log_runtime_error("Telegram polling crashed unexpectedly.", exc)
                time.sleep(1.0)

    def submit_update(self, update: dict[str, Any]) -> Future[Any]:
        if self._logical_turn_merge_enabled():
            self._buffer_submitted_text_update(update)
        queue_key = self._dispatch_key(update)
        return self._submit_serialized(queue_key, self._handle_submitted_update, update)

    def run_background_sync_once(self) -> list[Future[Any]]:
        futures: list[Future[Any]] = []
        state = self._state_load()
        manager = self._background_sync_manager()
        for chat_id, link in state.chats.items():
            if not manager.link_has_background_sync_work(link):
                continue
            futures.append(
                self._submit_serialized(
                    self._dispatch_key_for_chat(chat_id),
                    self._run_background_sync_for_chat,
                    str(chat_id),
                )
            )
        return futures

    def _background_sync_manager(self) -> TelegramBackgroundSyncManager:
        return TelegramBackgroundSyncManager(
            config=self.config,
            ait_api=self.ait_api,
            telegram_api=self.telegram_api,
            state_get_chat=self._state_get_chat,
            state_patch_chat=self._state_patch_chat,
            sync_session=self._sync_session,
            mark_missing_session_relink_required=self._mark_missing_session_relink_required,
            should_skip_event_for_chat=self._should_skip_event_for_chat,
            runtime_snapshot=self._runtime_snapshot,
            session_missing_relink_reason=self._session_missing_relink_reason,
            run_graph_notifications_for_chat=self._run_graph_notifications_for_chat,
            log_runtime_error=self._log_runtime_error,
            missing_session_server_read_error=_is_missing_session_server_read_error,
            retryable_server_read_error=_is_retryable_server_read_error,
            retryable_transport_error=_shared_is_retryable_transport_error,
            runtime_error_type=BotRuntimeError,
            now_iso=utc_now_iso,
            backoff_threshold=lambda: TELEGRAM_BACKGROUND_SYNC_FAILURE_BACKOFF_THRESHOLD,
            backoff_delay_seconds=_background_sync_backoff_delay_seconds,
            clock_time=lambda: time.time(),
            replay_undelivered_telegram_live_replies=self._replay_undelivered_telegram_live_replies,
        )

    def _submit_serialized(self, queue_key: str, fn, *args: Any) -> Future[Any]:
        with self._dispatch_lock:
            executor = self._dispatchers.get(queue_key)
            if executor is None:
                executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix=f"ait-telegram-{queue_key}")
                self._dispatchers[queue_key] = executor
        future = executor.submit(fn, *args)
        return self._track_future(future)

    def _submit_reply_serialized(self, queue_key: str, fn, *args: Any) -> Future[Any]:
        with self._reply_dispatch_lock:
            executor = self._reply_dispatchers.get(queue_key)
            if executor is None:
                executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix=f"ait-telegram-reply-{queue_key}")
                self._reply_dispatchers[queue_key] = executor
        future = executor.submit(fn, *args)
        return self._track_future(future)

    def _track_future(self, future: Future[Any]) -> Future[Any]:
        with self._dispatch_futures_lock:
            self._dispatch_futures.add(future)
        future.add_done_callback(self._forget_dispatch_future)
        return future

    def wait_for_idle(self, timeout: float | None = None) -> bool:
        deadline = None if timeout is None else time.monotonic() + timeout
        while True:
            with self._dispatch_futures_lock:
                futures = [future for future in self._dispatch_futures if not future.done()]
            if not futures:
                return True
            remaining = None if deadline is None else max(deadline - time.monotonic(), 0.0)
            if remaining is not None and remaining <= 0:
                return False
            wait(futures, timeout=remaining)

    def handle_update(self, update: dict[str, Any]) -> None:
        self._process_update(update, allow_logical_turn_merge=False, defer_normal_text_turn=False)

    def _handle_submitted_update(self, update: dict[str, Any]) -> None:
        self._process_update(
            update,
            allow_logical_turn_merge=self._logical_turn_merge_enabled(),
            defer_normal_text_turn=self.config.decoupled_reply_enabled,
        )

    def _process_update(
        self,
        update: dict[str, Any],
        *,
        allow_logical_turn_merge: bool,
        defer_normal_text_turn: bool,
    ) -> None:
        chat_id = self._chat_id_from_update(update)
        try:
            if allow_logical_turn_merge:
                logical_turn = self._claim_logical_turn(update)
                if logical_turn is _SKIP_LOGICAL_TURN:
                    return
                if isinstance(logical_turn, TelegramLogicalTurn):
                    self._handle_logical_turn(logical_turn, defer_reply=defer_normal_text_turn)
                    return
            self._handle_update_impl(update, defer_normal_text_turn=defer_normal_text_turn)
        except BotRuntimeError as exc:
            self._log_runtime_error("Telegram update failed with bot runtime error.", exc)
            self._safe_send_message(chat_id, f"ait Telegram bot error: {exc}")
        except Exception as exc:  # pragma: no cover - exercised in tests through submit_update workers
            self._log_runtime_error("Telegram update crashed unexpectedly.", exc)
            self._safe_send_message(
                chat_id,
                "ait Telegram bot hit an unexpected error while processing this update. Check the daemon log and retry if needed.",
            )

    def _handle_update_impl(self, update: dict[str, Any], *, defer_normal_text_turn: bool) -> None:
        message = update.get("message") or {}
        chat = message.get("chat") or {}
        from_user = message.get("from") or {}
        chat_id = chat.get("id")
        if chat_id is None:
            return
        chat_title = _chat_title(chat)
        candidate_raw_text = message.get("text")
        if not isinstance(candidate_raw_text, str):
            candidate_raw_text = message.get("caption")
        speech_attachments = _speech_attachments_from_message(
            message,
            include_audio_uploads=self.config.stt_include_audio_uploads,
        )
        if speech_attachments and self._handle_owner_bootstrap_gate(
            chat_id,
            chat,
            from_user,
            chat_title,
            raw_text=candidate_raw_text if isinstance(candidate_raw_text, str) else None,
            command=None,
            attachments_present=True,
        ):
            return
        try:
            if speech_attachments:
                local_stt_turn = self._resolve_local_stt_turn_input(message, speech_attachments)
                raw_text = local_stt_turn.text
                attachments = list(local_stt_turn.attachments)
            else:
                raw_text = candidate_raw_text
                attachments = _music_attachments_from_message(message)
        except LocalSpeechToTextError as exc:
            self.telegram_api.send_message(chat_id, exc.user_message)
            return
        if (not isinstance(raw_text, str) or not raw_text.strip()) and not attachments:
            return
        normalized = _normalized_turn_text(
            raw_text=raw_text if isinstance(raw_text, str) else None,
            username=self.config.username,
            attachments=attachments,
        )
        command = None if attachments else parse_command(str(raw_text or ""), self.config.username)
        if self._handle_owner_bootstrap_gate(
            chat_id,
            chat,
            from_user,
            chat_title,
            raw_text=raw_text if isinstance(raw_text, str) else None,
            command=command,
            attachments_present=bool(attachments),
        ):
            return

        if command:
            name, args = command
            self._handle_command(chat_id, chat, from_user, chat_title, name, args)
            return

        workflow_query = None if attachments else detect_workflow_query(normalized)
        if workflow_query:
            kind, target = workflow_query
            if kind == "queue":
                self.telegram_api.send_message(chat_id, format_queue_summary(self.config, self.ait_api.read_task_queue()))
                return
            if kind == "attention":
                self.telegram_api.send_message(chat_id, format_attention_summary(self.config, self.ait_api.read_task_queue()))
                return
            if kind == "ready":
                self.telegram_api.send_message(chat_id, format_ready_summary(self.config, self.ait_api.read_task_queue()))
                return
            if kind == "task" and target:
                self.telegram_api.send_message(chat_id, format_task_summary(self.config, self.ait_api.read_task(target)))
                return
            if kind == "audit" and target:
                self.telegram_api.send_message(chat_id, format_task_audit_summary(self.config, self.ait_api.read_task_audit(target)))
                return
            if kind == "change" and target:
                self.telegram_api.send_message(chat_id, format_change_summary(self.config, self.ait_api.read_change(target)))
                return
            if kind == "land" and target:
                self.telegram_api.send_message(chat_id, format_change_land_summary(self.config, self.ait_api.read_change(target)))
                return

        if not normalized:
            self.telegram_api.send_message(chat_id, "Send a message after the bot mention, or use /help.")
            return

        self._handle_normal_text_turn(
            chat_id,
            chat,
            from_user,
            chat_title,
            normalized,
            telegram_message_id=message.get("message_id"),
            message_attachments=tuple(attachments),
            defer_reply=defer_normal_text_turn,
        )

    def _handle_logical_turn(self, turn: TelegramLogicalTurn, *, defer_reply: bool) -> None:
        message = turn.update.get("message") or {}
        chat = message.get("chat") or {}
        from_user = message.get("from") or {}
        chat_id = chat.get("id")
        if chat_id is None:
            return
        self._handle_normal_text_turn(
            chat_id,
            chat,
            from_user,
            _chat_title(chat),
            turn.text,
            telegram_message_id=turn.telegram_message_id,
            telegram_message_ids=turn.telegram_message_ids,
            actor_identity=turn.actor_identity,
            defer_reply=defer_reply,
        )

    def _handle_normal_text_turn(
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
        self._reply_turn_manager.handle_normal_text_turn(
            chat_id,
            chat,
            from_user,
            chat_title,
            normalized_text,
            telegram_message_id=telegram_message_id,
            telegram_message_ids=telegram_message_ids,
            message_attachments=message_attachments,
            actor_identity=actor_identity,
            defer_reply=defer_reply,
        )

    def _resolve_local_stt_turn_input(
        self,
        message: Mapping[str, Any],
        attachments: Sequence[Mapping[str, Any]],
    ) -> LocalSpeechToTextTurnInput:
        runtime = self.speech_to_text_runtime
        if runtime is None:
            raise LocalSpeechToTextError(
                "Local STT is not enabled for this Telegram worker. Set `AIT_TELEGRAM_STT_MODE=local-stt` and retry."
            )
        return runtime.transcribe_message(message, attachments=attachments)

    def _run_pending_reply_turn_safe(self, pending_turn: PendingTelegramReplyTurn) -> None:
        self._reply_turn_manager.run_pending_reply_turn_safe(pending_turn)

    def _recover_or_watch_completed_pending_reply(
        self,
        pending_turn: PendingTelegramReplyTurn,
        exc: BaseException,
    ) -> bool:
        # Skip the long watch path when the worker definitively never reached ait-server.
        if _is_definite_server_write_no_connection_error(exc):
            return False
        if self._recover_completed_pending_reply(pending_turn):
            return True
        return _is_retryable_server_read_error(exc) and self._watch_for_completed_pending_reply(pending_turn)

    def _run_pending_reply_turn(self, pending_turn: PendingTelegramReplyTurn) -> None:
        self._reply_turn_manager.run_pending_reply_turn(pending_turn)

    def _send_assistant_event_reply(
        self,
        chat_id: str | int,
        assistant_event: Mapping[str, Any],
        *,
        reply_text: str | None = None,
    ) -> None:
        self._reply_turn_manager.send_assistant_event_reply(
            chat_id,
            assistant_event,
            reply_text=reply_text,
        )

    def _deliver_reply_attachments(self, chat_id: str | int, attachments: list[dict[str, Any]]) -> None:
        self._reply_turn_manager.deliver_reply_attachments(chat_id, attachments)

    def _handle_command(self, chat_id: str | int, chat: dict[str, Any], from_user: dict[str, Any], chat_title: str, name: str, args: str) -> None:
        self._command_runtime.dispatch(chat_id, chat, from_user, chat_title, name, args)

    def _help_text(self, chat_id: str | int, chat: dict[str, Any], chat_title: str) -> str:
        return self._command_runtime.help_text(chat_id, chat, chat_title)

    def _unknown_command_text(self, name: str) -> str:
        return self._command_runtime.unknown_command_text(name)

    def _handle_owner_bootstrap_gate(
        self,
        chat_id: str | int,
        chat: Mapping[str, Any],
        from_user: Mapping[str, Any],
        chat_title: str,
        *,
        raw_text: str | None,
        command: tuple[str, str] | None,
        attachments_present: bool,
    ) -> bool:
        return self._owner_bootstrap_gate.handle(
            chat_id,
            chat,
            from_user,
            chat_title,
            raw_text=raw_text,
            command=command,
            attachments_present=attachments_present,
        )

    def _missing_link_text(self) -> str:
        return self._command_runtime.missing_link_text()

    def _linked_session_detail(self, link: dict[str, Any] | None) -> dict[str, Any] | None:
        return self._command_runtime.linked_session_detail(link)

    def _linked_session_status_text(self, link: dict[str, Any]) -> str:
        return self._command_runtime.linked_session_status_text(link)

    def _handle_help_command(self, chat_id: str | int, chat: dict[str, Any], _from_user: dict[str, Any], chat_title: str, _args: str) -> None:
        self._command_runtime.handle_help_command(chat_id, chat, _from_user, chat_title, _args)

    def _handle_status_command(self, chat_id: str | int, chat: dict[str, Any], _from_user: dict[str, Any], chat_title: str, _args: str) -> None:
        self._command_runtime.handle_status_command(chat_id, chat, _from_user, chat_title, _args)

    def _handle_sync_command(self, chat_id: str | int, chat: dict[str, Any], _from_user: dict[str, Any], chat_title: str, _args: str) -> None:
        self._command_runtime.handle_sync_command(chat_id, chat, _from_user, chat_title, _args)

    def _handle_session_command(self, chat_id: str | int, chat: dict[str, Any], _from_user: dict[str, Any], chat_title: str, _args: str) -> None:
        self._command_runtime.handle_session_command(chat_id, chat, _from_user, chat_title, _args)

    def _handle_queue_command(self, chat_id: str | int, _chat: dict[str, Any], _from_user: dict[str, Any], _chat_title: str, _args: str) -> None:
        self._command_runtime.handle_queue_command(chat_id, _chat, _from_user, _chat_title, _args)

    def _handle_attention_command(self, chat_id: str | int, _chat: dict[str, Any], _from_user: dict[str, Any], _chat_title: str, _args: str) -> None:
        self._command_runtime.handle_attention_command(chat_id, _chat, _from_user, _chat_title, _args)

    def _handle_ready_command(self, chat_id: str | int, _chat: dict[str, Any], _from_user: dict[str, Any], _chat_title: str, _args: str) -> None:
        self._command_runtime.handle_ready_command(chat_id, _chat, _from_user, _chat_title, _args)

    def _handle_task_command(self, chat_id: str | int, _chat: dict[str, Any], _from_user: dict[str, Any], _chat_title: str, args: str) -> None:
        self._command_runtime.handle_task_command(chat_id, _chat, _from_user, _chat_title, args)

    def _handle_audit_command(self, chat_id: str | int, _chat: dict[str, Any], _from_user: dict[str, Any], _chat_title: str, args: str) -> None:
        self._command_runtime.handle_audit_command(chat_id, _chat, _from_user, _chat_title, args)

    def _handle_change_command(self, chat_id: str | int, _chat: dict[str, Any], _from_user: dict[str, Any], _chat_title: str, args: str) -> None:
        self._command_runtime.handle_change_command(chat_id, _chat, _from_user, _chat_title, args)

    def _handle_land_command(self, chat_id: str | int, _chat: dict[str, Any], _from_user: dict[str, Any], _chat_title: str, args: str) -> None:
        self._command_runtime.handle_land_command(chat_id, _chat, _from_user, _chat_title, args)

    def _handle_notify_command(self, chat_id: str | int, chat: dict[str, Any], _from_user: dict[str, Any], chat_title: str, args: str) -> None:
        self._command_runtime.handle_notify_command(chat_id, chat, _from_user, chat_title, args)

    def _handle_watchgraph_command(self, chat_id: str | int, chat: dict[str, Any], _from_user: dict[str, Any], chat_title: str, args: str) -> None:
        self._graph_watch_manager.handle_watchgraph_command(chat_id, chat, chat_title, args)

    def _handle_ping_command(self, chat_id: str | int, _chat: dict[str, Any], _from_user: dict[str, Any], _chat_title: str, _args: str) -> None:
        self._command_runtime.handle_ping_command(chat_id, _chat, _from_user, _chat_title, _args)

    def _notification_status_text(self, link: dict[str, Any]) -> str:
        return self._command_runtime.notification_status_text(link)

    def _notification_enabled_text(self, link: dict[str, Any], payload: dict[str, Any]) -> str:
        return self._command_runtime.notification_enabled_text(link, payload)

    def _session_missing_relink_reason(
        self,
        link: Mapping[str, Any] | None,
        runtime_snapshot: TelegramRuntimeSnapshot | None,
    ) -> str:
        startup_signature = _runtime_backend_signature(
            self.config.runtime_mode,
            self.config.runtime_remote_name,
            self.config.ait_server_url,
        )
        return self._session_link_coordinator.session_missing_relink_reason(
            link,
            runtime_snapshot,
            startup_signature=startup_signature,
        )

    def _match_fresh_topic_event_trigger(self, text: str) -> dict[str, Any] | None:
        return parse_fresh_topic_trigger(str(text or "").strip(), self.event_trigger_registry.fresh_topic)

    def _fresh_topic_confirmation_text(
        self,
        link: Mapping[str, Any],
        fresh_topic: Mapping[str, Any],
    ) -> str:
        mode = str(fresh_topic.get("mode") or "clear").strip().lower()
        topic = str(fresh_topic.get("topic") or "").strip()
        trigger_label = str(fresh_topic.get("display_trigger") or self.event_trigger_registry.fresh_topic.clear.display_trigger)
        lines = [
            "Started a fresh Telegram-linked session.",
            f"Trigger: {trigger_label}.",
        ]
        if mode == "topic" and topic:
            lines.append(f"Topic hint: {topic}")
        session_id = str(link.get("session_id") or "").strip()
        if session_id:
            lines.append(f"Session: {session_id}")
        return "\n".join(lines)

    def _create_transport_session(
        self,
        *,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
        **kwargs: Any,
    ) -> dict[str, Any]:
        return self._session_link_coordinator.create_transport_session(
            runtime_snapshot=runtime_snapshot,
            **kwargs,
        )

    def _create_fresh_session(
        self,
        chat_id: str | int,
        chat: dict[str, Any],
        chat_title: str,
        *,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
        previous_link: Mapping[str, Any] | None = None,
        relink_reason: str = "initial_link",
    ) -> dict[str, Any]:
        return self._session_link_coordinator.create_fresh_session(
            chat_id,
            chat,
            chat_title,
            runtime_snapshot=runtime_snapshot,
            previous_link=previous_link,
            relink_reason=relink_reason,
        )

    def _ensure_session_link(
        self,
        chat_id: str | int,
        chat: dict[str, Any],
        chat_title: str,
        *,
        create_if_missing: bool = True,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
    ) -> dict[str, Any] | None:
        startup_signature = _runtime_backend_signature(
            self.config.runtime_mode,
            self.config.runtime_remote_name,
            self.config.ait_server_url,
        )
        return self._session_link_coordinator.ensure_session_link(
            chat_id,
            chat,
            chat_title,
            create_if_missing=create_if_missing,
            runtime_snapshot=runtime_snapshot,
            startup_signature=startup_signature,
        )

    def _advance_sync_cursor(self, chat_id: str | int, link: dict[str, Any], *, through_sequence: int) -> dict[str, Any]:
        return self._live_reply_manager.advance_sync_cursor(
            chat_id,
            link,
            through_sequence=through_sequence,
        )

    def _telegram_live_delivered_sequences(self, link: Mapping[str, Any] | None) -> set[int]:
        return self._live_reply_manager.telegram_live_delivered_sequences(link)

    def _mark_telegram_live_reply_delivered(
        self,
        chat_id: str | int,
        link: dict[str, Any],
        *,
        assistant_event: Mapping[str, Any] | None,
        through_sequence: int,
    ) -> dict[str, Any]:
        return self._live_reply_manager.mark_telegram_live_reply_delivered(
            chat_id,
            link,
            assistant_event=assistant_event,
            through_sequence=through_sequence,
        )

    def _should_skip_event_for_chat(
        self,
        chat_id: str | int,
        event: dict[str, Any],
        link: Mapping[str, Any] | None = None,
    ) -> bool:
        return self._live_reply_manager.should_skip_event_for_chat(chat_id, event, link=link)

    def _pending_turn_spool_key(self, pending_turn: PendingTelegramReplyTurn) -> str:
        return self._reply_turn_spool.pending_turn_spool_key(pending_turn)

    def _telegram_reply_spool_entries(self, link: Mapping[str, Any] | None) -> list[dict[str, Any]]:
        return self._reply_turn_spool.telegram_reply_spool_entries(link)

    def _remember_pending_reply_turn(
        self,
        pending_turn: PendingTelegramReplyTurn,
        *,
        status: str,
        attempt_increment: bool = False,
        last_error: str | None = None,
        user_event: Mapping[str, Any] | None = None,
        assistant_event: Mapping[str, Any] | None = None,
    ) -> dict[str, Any] | None:
        return self._reply_turn_spool.remember_pending_reply_turn(
            pending_turn,
            status=status,
            attempt_increment=attempt_increment,
            last_error=last_error,
            user_event=user_event,
            assistant_event=assistant_event,
        )

    def _clear_pending_reply_turn_spool_entry(self, pending_turn: PendingTelegramReplyTurn) -> dict[str, Any] | None:
        return self._reply_turn_spool.clear_pending_reply_turn_spool_entry(pending_turn)

    def _watch_for_completed_pending_reply(self, pending_turn: PendingTelegramReplyTurn) -> bool:
        return self._live_reply_manager.watch_for_completed_pending_reply(pending_turn)

    def _pending_turn_matches_user_event(
        self,
        pending_turn: PendingTelegramReplyTurn,
        event: Mapping[str, Any],
    ) -> bool:
        return self._live_reply_manager.pending_turn_matches_user_event(pending_turn, event)

    def _assistant_reply_for_user_event(
        self,
        *,
        chat_id: str | int,
        user_event: Mapping[str, Any],
        events: list[dict[str, Any]],
    ) -> dict[str, Any] | None:
        return self._live_reply_manager.assistant_reply_for_user_event(
            chat_id=chat_id,
            user_event=user_event,
            events=events,
        )

    def _recover_completed_pending_reply_once(self, pending_turn: PendingTelegramReplyTurn) -> str:
        return self._live_reply_manager.recover_completed_pending_reply_once(pending_turn)

    def _recover_completed_pending_reply(self, pending_turn: PendingTelegramReplyTurn) -> bool:
        return self._live_reply_manager.recover_completed_pending_reply(pending_turn)

    def _sync_session(self, chat_id: str | int, link: dict[str, Any]) -> list[dict[str, Any]]:
        return self._session_link_coordinator.sync_session(
            chat_id,
            link,
            should_skip_event_for_chat=self._should_skip_event_for_chat,
        )

    def _mark_missing_session_relink_required(
        self,
        chat_id: str | int,
        link: Mapping[str, Any],
        *,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
        relink_reason: str | None = None,
    ) -> dict[str, Any] | None:
        snapshot = runtime_snapshot or self._runtime_snapshot()
        return self._session_link_coordinator.mark_missing_session_relink_required(
            chat_id,
            link,
            runtime_snapshot=snapshot,
            relink_reason=relink_reason or self._session_missing_relink_reason(link, snapshot),
        )

    def _replay_undelivered_telegram_live_replies(
        self,
        chat_id: str | int,
        link: Mapping[str, Any],
        events: list[dict[str, Any]],
    ) -> tuple[list[dict[str, Any]], bool]:
        return self._live_reply_manager.replay_undelivered_telegram_live_replies(chat_id, link, events)

    def _run_background_sync_for_chat(self, chat_id: str) -> bool:
        return self._background_sync_manager().run_background_sync_for_chat(chat_id)

    def _run_workflow_notifications_for_chat(self, chat_id: str, link: dict[str, Any]) -> bool:
        return self._background_sync_manager().run_workflow_notifications_for_chat(chat_id, link)

    def _run_graph_notifications_for_chat(self, chat_id: str, link: dict[str, Any]) -> bool:
        return self._graph_watch_manager.run_graph_notifications_for_chat(chat_id, link)

    def _logical_turn_merge_enabled(self) -> bool:
        return self._logical_turn_buffer.logical_turn_merge_enabled()

    def _buffer_submitted_text_update(self, update: dict[str, Any]) -> None:
        self._logical_turn_buffer.buffer_submitted_text_update(update)

    def _claim_logical_turn(self, update: dict[str, Any]) -> TelegramLogicalTurn | object | None:
        return self._logical_turn_buffer.claim_logical_turn(update)

    def _state_load(self):
        with self._state_lock:
            return self.state_store.load()

    def _state_get_chat(self, chat_id: str | int) -> dict[str, Any] | None:
        with self._state_lock:
            return self.state_store.get_chat(chat_id)

    def _state_upsert_chat(self, chat_id: str | int, **kwargs: Any) -> dict[str, Any]:
        with self._state_lock:
            return self.state_store.upsert_chat(chat_id, **kwargs)

    def _state_patch_chat(self, chat_id: str | int, **kwargs: Any) -> dict[str, Any] | None:
        with self._state_lock:
            return self.state_store.patch_chat(chat_id, **kwargs)

    def _state_update_last_update_id(self, update_id: int):
        with self._state_lock:
            return self.state_store.update_last_update_id(update_id)

    def _state_get_bootstrap_auth(self) -> dict[str, Any]:
        with self._state_lock:
            return self.state_store.get_bootstrap_auth()

    def _state_save_bootstrap_auth(self, payload: dict[str, Any]) -> dict[str, Any]:
        with self._state_lock:
            return self.state_store.save_bootstrap_auth(payload)

    def _runtime_snapshot(self) -> TelegramRuntimeSnapshot | None:
        capture = getattr(self.ait_api, "capture_runtime_snapshot", None)
        if not callable(capture):
            return None
        return capture()

    def _ait_api_call(
        self,
        method_name: str,
        /,
        *args: Any,
        runtime_snapshot: TelegramRuntimeSnapshot | None = None,
        **kwargs: Any,
    ) -> Any:
        method = getattr(self.ait_api, method_name)
        if runtime_snapshot is None:
            return method(*args, **kwargs)
        try:
            return method(*args, runtime_snapshot=runtime_snapshot, **kwargs)
        except TypeError as exc:
            if "unexpected keyword argument" not in str(exc):
                raise
            return method(*args, **kwargs)

    def _dispatch_key(self, update: dict[str, Any]) -> str:
        return self._update_dispatch.dispatch_key(update)

    def _dispatch_key_for_chat(self, chat_id: str | int) -> str:
        return self._update_dispatch.dispatch_key_for_chat(chat_id)

    def _chat_id_from_update(self, update: dict[str, Any]) -> str | int | None:
        return self._update_dispatch.chat_id_from_update(update)

    def _update_key(self, update: dict[str, Any]) -> str:
        return self._update_dispatch.update_key(update)

    def _forget_dispatch_future(self, future: Future[Any]) -> None:
        with self._dispatch_futures_lock:
            self._dispatch_futures.discard(future)

    def _safe_send_message(self, chat_id: str | int | None, text: str) -> None:
        if chat_id is None:
            return
        try:
            self.telegram_api.send_message(chat_id, text)
        except Exception as exc:  # pragma: no cover - defensive logging around transport failures
            self._log_runtime_error(f"Failed to send Telegram bot error message to chat {chat_id}.", exc)

    def _log_runtime_error(self, message: str, exc: Exception) -> None:
        print(f"{message} {type(exc).__name__}: {exc}", file=sys.stderr, flush=True)

    def _next_background_sync_deadline(self) -> float:
        return time.monotonic() + self.config.background_sync_interval_seconds

    def _poll_timeout_seconds(self, next_background_sync_at: float | None) -> int:
        timeout_seconds = self.config.poll_timeout_seconds
        if not self.config.background_sync_enabled or next_background_sync_at is None:
            return timeout_seconds
        seconds_until_sync = max(next_background_sync_at - time.monotonic(), 0.0)
        return min(timeout_seconds, max(1, int(math.ceil(seconds_until_sync))))

    def _run_due_background_sync(self, next_background_sync_at: float | None) -> float | None:
        if not self.config.background_sync_enabled:
            return None
        if next_background_sync_at is None:
            next_background_sync_at = self._next_background_sync_deadline()
        if time.monotonic() < next_background_sync_at:
            return next_background_sync_at
        self.run_background_sync_once()
        return self._next_background_sync_deadline()
from .service_entry import (
    _install_signal_handlers,
    main,
    run_webhook_updates,
    webhook_main,
)


if __name__ == "__main__":  # pragma: no cover
    main()
