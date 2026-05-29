from __future__ import annotations

import asyncio
import json
import mimetypes
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Mapping
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
    retry_transport_operation as _shared_retry_transport_operation,
    retry_transport_operation_async as _shared_retry_transport_operation_async,
    retry_delay_seconds as _shared_retry_delay_seconds,
    timeout_value as _shared_timeout_value,
)

from .config import BotConfig, BotRuntimeError
from .formatting import _markdownish_parse_error, _telegram_message_chunks
from .graph_watches import _runtime_repo_root


TELEGRAM_DELIVERY_RETRY_ATTEMPTS = 3
TELEGRAM_DELIVERY_RETRY_BASE_DELAY_SECONDS = 1.0
TELEGRAM_POLL_RETRY_ATTEMPTS = 4
TELEGRAM_POLL_RETRY_BASE_DELAY_SECONDS = 1.0
AIT_SERVER_READ_RETRY_ATTEMPTS = 4
AIT_SERVER_READ_RETRY_BASE_DELAY_SECONDS = 0.75
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

    def send_photo(self, chat_id: str | int, attachment: Mapping[str, Any]) -> None:
        self._send_attachment_payload(
            method_name="sendPhoto",
            file_field="photo",
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

    async def send_photo(self, chat_id: str | int, attachment: Mapping[str, Any]) -> None:
        await self._send_attachment_payload(
            method_name="sendPhoto",
            file_field="photo",
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

__all__ = [
    "AIT_SERVER_READ_RETRY_ATTEMPTS",
    "AIT_SERVER_READ_RETRY_BASE_DELAY_SECONDS",
    "AsyncAitApiClient",
    "AsyncTelegramApiClient",
    "AitApiClient",
    "MISSING_SESSION_SERVER_READ_MARKERS",
    "RETRYABLE_SERVER_READ_MARKERS",
    "TELEGRAM_DELIVERY_RETRYABLE_ERRNOS",
    "TELEGRAM_DELIVERY_RETRYABLE_MARKERS",
    "TELEGRAM_DELIVERY_RETRY_ATTEMPTS",
    "TELEGRAM_DELIVERY_RETRY_BASE_DELAY_SECONDS",
    "TELEGRAM_POLL_RETRY_ATTEMPTS",
    "TELEGRAM_POLL_RETRY_BASE_DELAY_SECONDS",
    "TelegramApiClient",
    "TelegramRuntimeSnapshot",
    "_bytes_request",
    "_is_missing_session_server_read_error",
    "_is_retryable_server_read_error",
    "_is_retryable_telegram_delivery_error",
    "_json_request",
    "_multipart_json_request",
]
