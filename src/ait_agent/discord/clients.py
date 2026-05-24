from __future__ import annotations

from http.client import RemoteDisconnected
import json
import mimetypes
import os
import socket
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence
from urllib.parse import urlencode
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from ait_agent.envelope import (
    build_transport_binding_metadata,
    build_transport_event_envelope,
    build_transport_session_metadata,
)
from ait_agent.runtime_backend import (
    AgentRuntimeConfigError,
    LocalAitRuntime,
    resolve_agent_runtime_target,
)
from ait_agent.transport_retry import (
    is_loopback_url,
    is_retryable_server_read_error,
    is_retryable_transport_error,
    retry_transport_operation,
    timeout_phrase,
)


class BotRuntimeError(RuntimeError):
    pass


DEFAULT_DISCORD_MESSAGE_LIMIT = 2000
DEFAULT_DISCORD_HTTP_USER_AGENT = 'curl/8.7.1'
DISCORD_GUILD_MESSAGES_INTENT = 1 << 9
DISCORD_DIRECT_MESSAGES_INTENT = 1 << 12
DISCORD_MESSAGE_CONTENT_INTENT = 1 << 15
DEFAULT_DISCORD_GATEWAY_INTENTS = (
    DISCORD_GUILD_MESSAGES_INTENT
    | DISCORD_DIRECT_MESSAGES_INTENT
    | DISCORD_MESSAGE_CONTENT_INTENT
)
DISCORD_GATEWAY_INFO_RETRY_ATTEMPTS = 4
DISCORD_GATEWAY_INFO_RETRY_BASE_DELAY_SECONDS = 0.75
AIT_SERVER_READ_RETRY_ATTEMPTS = 4
AIT_SERVER_READ_RETRY_BASE_DELAY_SECONDS = 0.75

_REQUEST_TIMEOUT_DEFAULT = object()


class DiscordApiClient:
    def __init__(self, config: BotConfig):
        self.config = config

    def _request(
        self,
        path: str,
        *,
        payload: dict[str, Any] | None = None,
        method: str = 'POST',
        headers: Mapping[str, str] | None = None,
        allow_retry: bool = False,
        timeout: float | None | object = _REQUEST_TIMEOUT_DEFAULT,
    ) -> Any:
        request_url = f"{self.config.discord_api_base_url}{path}"
        request_headers = {'User-Agent': self.config.discord_http_user_agent}
        if headers:
            request_headers.update({str(key): str(value) for key, value in headers.items()})
        request_timeout = self.config.request_timeout_seconds if timeout is _REQUEST_TIMEOUT_DEFAULT else timeout
        action = lambda: _json_request(
            request_url,
            method=method,
            payload=payload,
            headers=request_headers,
            timeout=request_timeout,
        )
        if allow_retry:
            return retry_transport_operation(
                action,
                attempts=DISCORD_GATEWAY_INFO_RETRY_ATTEMPTS,
                base_delay_seconds=DISCORD_GATEWAY_INFO_RETRY_BASE_DELAY_SECONDS,
                retry_filter=is_retryable_transport_error,
            )
        return action()

    def _request_multipart(
        self,
        path: str,
        *,
        fields: Mapping[str, object],
        file_field: str,
        file_name: str,
        file_bytes: bytes,
        mime_type: str,
        headers: Mapping[str, str] | None = None,
        allow_retry: bool = False,
        timeout: float | None | object = _REQUEST_TIMEOUT_DEFAULT,
    ) -> Any:
        request_url = f"{self.config.discord_api_base_url}{path}"
        request_headers = {'User-Agent': self.config.discord_http_user_agent}
        if headers:
            request_headers.update({str(key): str(value) for key, value in headers.items()})
        request_timeout = self.config.request_timeout_seconds if timeout is _REQUEST_TIMEOUT_DEFAULT else timeout
        action = lambda: _multipart_json_request(
            request_url,
            fields=fields,
            file_field=file_field,
            file_name=file_name,
            file_bytes=file_bytes,
            mime_type=mime_type,
            headers=request_headers,
            timeout=request_timeout,
        )
        if allow_retry:
            return retry_transport_operation(
                action,
                attempts=DISCORD_GATEWAY_INFO_RETRY_ATTEMPTS,
                base_delay_seconds=DISCORD_GATEWAY_INFO_RETRY_BASE_DELAY_SECONDS,
                retry_filter=is_retryable_transport_error,
            )
        return action()

    def gateway_info(self) -> dict[str, Any]:
        bot_token = _require_discord_bot_token(self.config)
        payload = self._request(
            '/gateway/bot',
            method='GET',
            headers={'Authorization': f'Bot {bot_token}'},
            allow_retry=True,
        )
        if not isinstance(payload, dict):
            raise BotRuntimeError('Discord Get Gateway Bot returned an invalid response payload.')
        gateway_url = _clean_optional_str(payload.get('url'))
        if gateway_url is None:
            raise BotRuntimeError('Discord Get Gateway Bot did not return a gateway URL.')
        return payload

    def create_initial_response(self, interaction_id: str, interaction_token: str, payload: Mapping[str, Any]) -> None:
        self._request(
            f'/interactions/{interaction_id}/{interaction_token}/callback',
            payload=dict(payload),
        )

    def edit_original_response(self, application_id: str, interaction_token: str, text: str) -> list[str]:
        chunks = _split_message_chunks(text)
        message_ids: list[str] = []
        self._request(
            f'/webhooks/{application_id}/{interaction_token}/messages/@original',
            method='PATCH',
            payload={
                'content': chunks[0],
                'allowed_mentions': {'parse': []},
            },
        )
        # The original interaction response edit does not reliably return a durable
        # message id across Discord surfaces, so outbound receipts come from
        # follow-up messages when they exist.
        for chunk in chunks[1:]:
            message_ids.extend(self.send_followup(application_id, interaction_token, chunk))
        return message_ids

    def send_followup(self, application_id: str, interaction_token: str, text: str) -> list[str]:
        message_ids: list[str] = []
        for chunk in _split_message_chunks(text):
            payload = self._request(
                f'/webhooks/{application_id}/{interaction_token}',
                payload={
                    'content': chunk,
                    'allowed_mentions': {'parse': []},
                },
            )
            message_id = _discord_message_id_from_response(payload)
            if message_id:
                message_ids.append(message_id)
        return message_ids

    def send_followup_attachment(
        self,
        application_id: str,
        interaction_token: str,
        attachment: Mapping[str, Any],
    ) -> list[str]:
        upload = _discord_attachment_upload(attachment)
        payload = self._request_multipart(
            f'/webhooks/{application_id}/{interaction_token}',
            fields={
                'payload_json': json.dumps(
                    _discord_attachment_message_payload(upload.file_name, upload.caption),
                    ensure_ascii=False,
                )
            },
            file_field='files[0]',
            file_name=upload.file_name,
            file_bytes=upload.file_bytes,
            mime_type=upload.mime_type,
        )
        message_id = _discord_message_id_from_response(payload)
        return [message_id] if message_id else []

    def send_channel_message(self, channel_id: str, text: str) -> list[str]:
        bot_token = _require_discord_bot_token(self.config)
        message_ids: list[str] = []
        for chunk in _split_message_chunks(text):
            payload = self._request(
                f'/channels/{channel_id}/messages',
                headers={'Authorization': f'Bot {bot_token}'},
                payload={
                    'content': chunk,
                    'allowed_mentions': {'parse': []},
                },
            )
            message_id = _discord_message_id_from_response(payload)
            if message_id:
                message_ids.append(message_id)
        return message_ids

    def send_channel_attachment(self, channel_id: str, attachment: Mapping[str, Any]) -> list[str]:
        bot_token = _require_discord_bot_token(self.config)
        upload = _discord_attachment_upload(attachment)
        payload = self._request_multipart(
            f'/channels/{channel_id}/messages',
            headers={'Authorization': f'Bot {bot_token}'},
            fields={
                'payload_json': json.dumps(
                    _discord_attachment_message_payload(upload.file_name, upload.caption),
                    ensure_ascii=False,
                )
            },
            file_field='files[0]',
            file_name=upload.file_name,
            file_bytes=upload.file_bytes,
            mime_type=upload.mime_type,
        )
        message_id = _discord_message_id_from_response(payload)
        return [message_id] if message_id else []

    def list_channel_messages(self, channel_id: str, *, limit: int = 25) -> list[dict[str, Any]]:
        bot_token = _require_discord_bot_token(self.config)
        payload = self._request(
            f'/channels/{channel_id}/messages?{urlencode({"limit": max(int(limit), 1)})}',
            method='GET',
            headers={'Authorization': f'Bot {bot_token}'},
        )
        if not isinstance(payload, list):
            raise BotRuntimeError('Discord channel message history returned an invalid response payload.')
        return [dict(item) for item in payload if isinstance(item, Mapping)]


class AitApiClient:
    def __init__(self, config: BotConfig):
        self.config = config
        self._local_runtime = (
            LocalAitRuntime(resolve_agent_runtime_target(Path(os.environ.get('AIT_REPO_ROOT') or Path.cwd())))
            if config.runtime_mode == 'local'
            else None
        )

    def _headers(self, actor_identity: str, actor_type: str) -> dict[str, str]:
        return {
            'X-AIT-Actor': actor_identity,
            'X-AIT-Actor-Type': actor_type,
        }

    def _local_call(self, fn_name: str, /, *args: Any, **kwargs: Any) -> Any:
        runtime = self._local_runtime
        if runtime is None:
            raise BotRuntimeError('Discord worker local runtime is unavailable.')
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
        actor_identity: str = 'ait-agent-discord',
        actor_type: str = 'discord_bot',
        allow_retry: bool = False,
        timeout: float | None | object = _REQUEST_TIMEOUT_DEFAULT,
    ) -> Any:
        if self.config.ait_server_url is None:
            raise BotRuntimeError('Discord worker is in local runtime mode and cannot issue remote HTTP requests.')
        request_url = f"{self.config.ait_server_url}{path}"
        request_timeout = self.config.request_timeout_seconds if timeout is _REQUEST_TIMEOUT_DEFAULT else timeout
        action = lambda: _json_request(
            request_url,
            method=method,
            payload=payload,
            headers=self._headers(actor_identity, actor_type),
            timeout=request_timeout,
        )
        if allow_retry and self.config.ait_server_url and is_loopback_url(self.config.ait_server_url):
            return retry_transport_operation(
                action,
                attempts=AIT_SERVER_READ_RETRY_ATTEMPTS,
                base_delay_seconds=AIT_SERVER_READ_RETRY_BASE_DELAY_SECONDS,
                retry_filter=is_retryable_server_read_error,
            )
        return action()

    def create_session(
        self,
        *,
        channel_id: str,
        channel_title: str,
        channel_kind: str | None,
        source_user_id: str | None,
        guild_id: str | None,
        application_id: str,
        session_kind: str = 'discord_chat',
        title_prefix: str = 'Discord chat',
        binding_role: str | None = None,
        canonical_session_id: str | None = None,
        active_session_id: str | None = None,
        branch_session_id: str | None = None,
        branch_kind: str | None = None,
        relink_reason: str | None = None,
        metadata_extra: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        metadata_extra = dict(metadata_extra or {})
        metadata_extra.update(build_transport_binding_metadata(
            transport='discord',
            surface_id=channel_id,
            surface_title=channel_title,
            surface_kind=channel_kind,
            binding_role=binding_role,
            canonical_session_id=canonical_session_id,
            active_session_id=active_session_id,
            branch_session_id=branch_session_id,
            branch_kind=branch_kind,
            relink_reason=relink_reason,
            reply_target={
                'channel_id': channel_id,
                'channel_kind': channel_kind,
                'guild_id': guild_id,
                'application_id': application_id,
                **({'source_user_id': source_user_id} if source_user_id else {}),
            },
        ))
        metadata = build_transport_session_metadata(
            transport='discord',
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            linked_via='ait-agent discord',
            metadata_extra={
                'discord_channel_id': channel_id,
                'discord_channel_title': channel_title,
                'discord_channel_kind': channel_kind,
                'discord_guild_id': guild_id,
                'discord_application_id': application_id,
                **({'discord_source_user_id': source_user_id} if source_user_id else {}),
                **metadata_extra,
            },
        )
        title = f'{title_prefix} · {channel_title}'
        if self._local_runtime is not None:
            return self._local_call(
                'create_session',
                session_kind=session_kind,
                title=title,
                metadata=metadata,
            )
        return self._request('POST', f'/v1/native/repositories/{self.config.repo_name}/sessions', payload={'session_kind': session_kind, 'title': title, 'metadata': metadata})

    def get_session(self, session_id: str, *, actor_identity: str = 'ait-agent-discord') -> dict[str, Any]:
        if self._local_runtime is not None:
            return self._local_call('get_session', session_id)
        return self._request(
            'GET',
            f'/v1/native/sessions/{session_id}',
            actor_identity=actor_identity,
            actor_type='discord_bot',
            allow_retry=True,
        )

    def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50) -> list[dict[str, Any]]:
        if self._local_runtime is not None:
            return list(self._local_call('list_session_events', session_id, after_sequence=after_sequence, limit=limit) or [])
        query = urlencode({'after_sequence': after_sequence, 'limit': limit})
        return list(
            self._request(
                'GET',
                f'/v1/native/sessions/{session_id}/events?{query}',
                allow_retry=True,
            )
            or []
        )

    def create_discord_turn(
        self,
        session_id: str,
        *,
        text: str,
        channel_title: str,
        actor_identity: str,
        actor_display_name: str | None,
        transport_envelope: dict[str, Any],
    ) -> dict[str, Any]:
        if self._local_runtime is not None:
            return self._local_call(
                'create_surface_turn',
                session_id,
                text=text,
                surface='discord',
                title=channel_title,
                actor_identity=actor_identity,
                actor_type='discord_user',
                actor_display_name=actor_display_name,
                transport_envelope=transport_envelope,
            )
        return self._request(
            'POST',
            f'/v1/native/sessions/{session_id}:turn',
            payload={
                'text': text,
                'surface': 'discord',
                'title': channel_title,
                'actor_display_name': actor_display_name,
                'transport_envelope': dict(transport_envelope),
            },
            actor_identity=actor_identity,
            actor_type='discord_user',
            timeout=self.config.turn_timeout_seconds,
        )


def _require_discord_bot_token(config: BotConfig) -> str:
    token = _clean_optional_str(config.bot_token)
    if token is None:
        raise BotRuntimeError('Missing Discord bot token. Set AIT_DISCORD_BOT_TOKEN or DISCORD_BOT_TOKEN.')
    return token

def _clean_optional_str(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _env_value(values: Mapping[str, str], *names: str, default: str | None = None, env: Mapping[str, str] | None = None) -> str | None:
    source_env = os.environ if env is None else env
    for name in names:
        raw = str(source_env.get(name) or values.get(name) or '').strip()
        if raw:
            return raw
    return default


def _normalize_base_url(value: str | None, fallback: str) -> str:
    raw = str(value or '').strip() or fallback
    return raw.rstrip('/')


def _parse_timeout_seconds(value: str | None, fallback: float, minimum: float) -> float | None:
    raw = str(value or '').strip().lower()
    if not raw:
        return fallback
    if raw in {'inf', 'infinite', 'none'}:
        return None
    try:
        parsed = float(raw)
    except ValueError:
        return fallback
    if parsed <= 0:
        return fallback
    return max(parsed, minimum)


def _parse_int(value: str | None, fallback: int, minimum: int) -> int:
    raw = str(value or '').strip()
    if not raw:
        return fallback
    try:
        parsed = int(raw)
    except ValueError:
        return fallback
    if parsed < minimum:
        return fallback
    return parsed


def _split_message_chunks(text: str, *, limit: int = DEFAULT_DISCORD_MESSAGE_LIMIT) -> list[str]:
    content = str(text or '').strip()
    if not content:
        return ['(empty)']
    chunks: list[str] = []
    remaining = content
    while len(remaining) > limit:
        split_at = remaining.rfind('\n', 0, limit)
        if split_at < int(limit * 0.5):
            split_at = remaining.rfind(' ', 0, limit)
        if split_at < int(limit * 0.5):
            split_at = limit
        chunks.append(remaining[:split_at].rstrip())
        remaining = remaining[split_at:].lstrip()
    if remaining:
        chunks.append(remaining)
    return chunks


def _json_request(
    url: str,
    *,
    method: str = 'GET',
    payload: dict[str, Any] | None = None,
    headers: Mapping[str, str] | None = None,
    timeout: float | None = 20.0,
) -> Any:
    request_headers = {'Accept': 'application/json'}
    if payload is not None:
        request_headers['Content-Type'] = 'application/json'
    if headers:
        request_headers.update({str(key): str(value) for key, value in headers.items()})
    body = None if payload is None else json.dumps(payload, ensure_ascii=False).encode('utf-8')
    request = Request(url, data=body, headers=request_headers, method=method.upper())
    try:
        with urlopen(request, timeout=timeout) as response:
            raw = response.read().decode('utf-8')
    except (TimeoutError, socket.timeout) as exc:
        raise BotRuntimeError(f'{method.upper()} {url} timed out{timeout_phrase(timeout)}.') from exc
    except OverflowError as exc:
        raise BotRuntimeError(f'{method.upper()} {url} failed: invalid timeout value {timeout!r}.') from exc
    except HTTPError as exc:
        detail = exc.read().decode('utf-8', errors='replace') if exc.fp is not None else ''
        raise BotRuntimeError(f'{method.upper()} {url} failed: {exc.code} {detail or exc.reason}') from exc
    except URLError as exc:
        raise BotRuntimeError(f'{method.upper()} {url} failed: {exc.reason}') from exc
    except (RemoteDisconnected, ConnectionResetError, BrokenPipeError, ConnectionAbortedError, OSError) as exc:
        raise BotRuntimeError(f'{method.upper()} {url} failed: {exc}') from exc
    if not raw.strip():
        return {}
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return raw


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
    boundary = f'aitdiscord-{uuid.uuid4().hex}'
    chunks: list[bytes] = []
    for key, value in fields.items():
        if value is None:
            continue
        chunks.extend(
            [
                f'--{boundary}\r\n'.encode('utf-8'),
                f'Content-Disposition: form-data; name="{key}"\r\n\r\n'.encode('utf-8'),
                str(value).encode('utf-8'),
                b'\r\n',
            ]
        )
    chunks.extend(
        [
            f'--{boundary}\r\n'.encode('utf-8'),
            (
                f'Content-Disposition: form-data; name="{file_field}"; filename="{file_name}"\r\n'
                f'Content-Type: {mime_type}\r\n\r\n'
            ).encode('utf-8'),
            file_bytes,
            b'\r\n',
            f'--{boundary}--\r\n'.encode('utf-8'),
        ]
    )
    request_headers = {
        'Accept': 'application/json',
        'Content-Type': f'multipart/form-data; boundary={boundary}',
    }
    if headers:
        request_headers.update({str(key): str(value) for key, value in headers.items()})
    request = Request(url, data=b''.join(chunks), headers=request_headers, method='POST')
    try:
        with urlopen(request, timeout=timeout) as response:
            raw = response.read().decode('utf-8')
    except (TimeoutError, socket.timeout) as exc:
        raise BotRuntimeError(f'POST {url} timed out{timeout_phrase(timeout)}.') from exc
    except OverflowError as exc:
        raise BotRuntimeError(f'POST {url} failed: invalid timeout value {timeout!r}.') from exc
    except HTTPError as exc:
        detail = exc.read().decode('utf-8', errors='replace') if exc.fp is not None else ''
        raise BotRuntimeError(f'POST {url} failed: {exc.code} {detail or exc.reason}') from exc
    except URLError as exc:
        raise BotRuntimeError(f'POST {url} failed: {exc.reason}') from exc
    except (RemoteDisconnected, ConnectionResetError, BrokenPipeError, ConnectionAbortedError, OSError) as exc:
        raise BotRuntimeError(f'POST {url} failed: {exc}') from exc
    if not raw.strip():
        return {}
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return raw


def _discord_message_id_from_response(payload: Any) -> str | None:
    if not isinstance(payload, Mapping):
        return None
    return _clean_optional_str(payload.get('id'))


@dataclass(frozen=True)
class _DiscordAttachmentUpload:
    file_name: str
    file_bytes: bytes
    mime_type: str
    caption: str


def _discord_attachment_file_name(attachment: Mapping[str, Any]) -> str | None:
    explicit = _clean_optional_str(attachment.get('file_name'))
    if explicit:
        return explicit
    local_path = _clean_optional_str(attachment.get('local_path'))
    if local_path:
        return Path(local_path).name or None
    url = _clean_optional_str(attachment.get('url'))
    if url:
        return Path(url.rstrip('/')).name or None
    return None


def _discord_attachment_caption(attachment: Mapping[str, Any]) -> str:
    return str(attachment.get('caption') or attachment.get('title') or attachment.get('file_name') or '').strip()


def _discord_attachment_message_payload(file_name: str, caption: str) -> dict[str, Any]:
    content = str(caption or '').strip()
    attachment_entry: dict[str, Any] = {'id': 0, 'filename': file_name}
    if content:
        attachment_entry['description'] = content
    payload: dict[str, Any] = {
        'allowed_mentions': {'parse': []},
        'attachments': [attachment_entry],
    }
    if content:
        payload['content'] = content
    return payload


def _discord_attachment_upload(attachment: Mapping[str, Any]) -> _DiscordAttachmentUpload:
    local_path = _clean_optional_str(attachment.get('local_path'))
    if local_path is None:
        raise BotRuntimeError('Discord outbound attachment delivery currently requires attachment.local_path.')
    path = Path(local_path).expanduser()
    if not path.is_file():
        raise BotRuntimeError(f'Discord outbound attachment file is missing: {path}')
    file_name = _discord_attachment_file_name(attachment) or path.name or 'attachment.bin'
    mime_type = (
        _clean_optional_str(attachment.get('mime_type'))
        or mimetypes.guess_type(file_name)[0]
        or 'application/octet-stream'
    )
    try:
        file_bytes = path.read_bytes()
    except OSError as exc:
        raise BotRuntimeError(f'Failed to read Discord outbound attachment file {path}: {exc}') from exc
    return _DiscordAttachmentUpload(
        file_name=file_name,
        file_bytes=file_bytes,
        mime_type=mime_type,
        caption=_discord_attachment_caption(attachment),
    )


def _discord_attachment_delivery_placeholder(attachments: Sequence[Mapping[str, Any]]) -> str:
    count = len([item for item in attachments if isinstance(item, Mapping)])
    if count <= 0:
        return 'Sent a Discord attachment.'
    if count == 1:
        attachment = next((item for item in attachments if isinstance(item, Mapping)), {})
        label = _discord_attachment_file_name(attachment) or _discord_attachment_caption(attachment)
        return f'Sent attachment: {label}' if label else 'Sent one Discord attachment.'
    return f'Sent {count} Discord attachments.'


def _discord_attachment_failure_text(attachment: Mapping[str, Any], exc: BotRuntimeError) -> str:
    label = _discord_attachment_file_name(attachment) or _discord_attachment_caption(attachment) or 'attachment'
    return f'Could not upload Discord attachment `{label}`. Fallback to text/path only.\n{exc}'

__all__ = [
    "AIT_SERVER_READ_RETRY_ATTEMPTS",
    "AIT_SERVER_READ_RETRY_BASE_DELAY_SECONDS",
    "BotRuntimeError",
    "DEFAULT_DISCORD_MESSAGE_LIMIT",
    "DISCORD_GATEWAY_INFO_RETRY_ATTEMPTS",
    "DISCORD_GATEWAY_INFO_RETRY_BASE_DELAY_SECONDS",
    "DiscordApiClient",
    "AitApiClient",
    "_clean_optional_str",
    "_discord_attachment_caption",
    "_discord_attachment_delivery_placeholder",
    "_discord_attachment_failure_text",
    "_discord_attachment_file_name",
    "_discord_attachment_message_payload",
    "_discord_attachment_upload",
    "_discord_message_id_from_response",
    "_env_value",
    "_json_request",
    "_multipart_json_request",
    "_normalize_base_url",
    "_parse_int",
    "_parse_timeout_seconds",
    "_require_discord_bot_token",
    "_split_message_chunks",
]

