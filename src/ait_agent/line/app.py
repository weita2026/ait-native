from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
import signal
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Mapping, Sequence
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
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

from .runtime import LineSyncStateStore, load_simple_env_file, resolve_line_env_path, resolve_line_sync_state_path, utc_now_iso

DEFAULT_LINE_API_BASE_URL = 'https://api.line.me'
DEFAULT_LINE_MESSAGE_LIMIT = 5000
MAX_LINE_MESSAGE_OBJECTS = 5
AIT_LINE_TERMINATION_CONTEXT_ENV = 'AIT_LINE_TERMINATION_CONTEXT_PATH'


class BotRuntimeError(RuntimeError):
    pass


@dataclass(frozen=True)
class BotConfig:
    channel_access_token: str
    channel_secret: str
    ait_server_url: str | None
    ait_web_url: str | None
    repo_name: str
    request_timeout_seconds: float | None
    sync_state_path: Path
    line_api_base_url: str
    bind_host: str
    bind_port: int
    webhook_path: str
    runtime_mode: str = 'remote'
    runtime_remote_name: str | None = None


@dataclass(frozen=True)
class PendingLineReply:
    session_id: str
    channel_id: str
    channel_title: str
    channel_kind: str | None
    reply_token: str | None
    actor_identity: str
    actor_display_name: str | None
    text: str
    transport_envelope: dict[str, Any]
    source_user_id: str | None
    message_id: str | None
    webhook_event_id: str


class LineApiClient:
    def __init__(self, config: BotConfig):
        self.config = config

    def _request(self, path: str, *, payload: dict[str, Any]) -> Any:
        request_url = f"{self.config.line_api_base_url}{path}"
        return _json_request(
            request_url,
            method='POST',
            payload=payload,
            headers={
                'Authorization': f'Bearer {self.config.channel_access_token}',
                'Content-Type': 'application/json',
            },
            timeout=self.config.request_timeout_seconds,
        )

    def reply_messages(self, reply_token: str, messages: list[dict[str, Any]]) -> Any:
        return self._request('/v2/bot/message/reply', payload={'replyToken': reply_token, 'messages': messages})

    def push_messages(self, channel_id: str, messages: list[dict[str, Any]]) -> Any:
        return self._request('/v2/bot/message/push', payload={'to': channel_id, 'messages': messages})

    def send_text(self, channel_id: str, text: str, *, reply_token: str | None = None) -> None:
        messages = [{'type': 'text', 'text': chunk} for chunk in _split_message_chunks(text)]
        if not messages:
            return
        remaining = list(messages)
        if reply_token:
            self.reply_messages(reply_token, remaining[:MAX_LINE_MESSAGE_OBJECTS])
            remaining = remaining[MAX_LINE_MESSAGE_OBJECTS:]
        while remaining:
            self.push_messages(channel_id, remaining[:MAX_LINE_MESSAGE_OBJECTS])
            remaining = remaining[MAX_LINE_MESSAGE_OBJECTS:]


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
            raise BotRuntimeError('LINE worker local runtime is unavailable.')
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
        actor_identity: str = 'ait-agent-line',
        actor_type: str = 'line_bot',
    ) -> Any:
        if self.config.ait_server_url is None:
            raise BotRuntimeError('LINE worker is in local runtime mode and cannot issue remote HTTP requests.')
        request_url = f"{self.config.ait_server_url}{path}"
        return _json_request(
            request_url,
            method=method,
            payload=payload,
            headers=self._headers(actor_identity, actor_type),
            timeout=self.config.request_timeout_seconds,
        )

    def create_session(
        self,
        *,
        channel_id: str,
        channel_title: str,
        channel_kind: str | None,
        source_user_id: str | None,
        session_kind: str = 'line_chat',
        title_prefix: str = 'LINE chat',
    ) -> dict[str, Any]:
        metadata_extra = build_transport_binding_metadata(
            transport='line',
            surface_id=channel_id,
            surface_title=channel_title,
            surface_kind=channel_kind,
            reply_target={
                'channel_id': channel_id,
                'channel_kind': channel_kind,
                **({'source_user_id': source_user_id} if source_user_id else {}),
            },
        )
        metadata = build_transport_session_metadata(
            transport='line',
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            linked_via='ait-agent line',
            metadata_extra={
                'line_channel_id': channel_id,
                'line_channel_title': channel_title,
                'line_channel_kind': channel_kind,
                **({'line_source_user_id': source_user_id} if source_user_id else {}),
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

    def get_session(self, session_id: str, *, actor_identity: str = 'ait-agent-line') -> dict[str, Any]:
        if self._local_runtime is not None:
            return self._local_call('get_session', session_id)
        return self._request('GET', f'/v1/native/sessions/{session_id}', actor_identity=actor_identity, actor_type='line_bot')

    def create_line_turn(
        self,
        session_id: str,
        *,
        text: str,
        channel_id: str,
        channel_title: str,
        channel_kind: str | None,
        actor_identity: str,
        actor_display_name: str | None,
        transport_envelope: dict[str, Any],
    ) -> dict[str, Any]:
        if self._local_runtime is not None:
            return self._local_call(
                'create_surface_turn',
                session_id,
                text=text,
                surface='line',
                title=channel_title,
                actor_identity=actor_identity,
                actor_type='line_user',
                actor_display_name=actor_display_name,
                transport_envelope=transport_envelope,
            )
        return self._request(
            'POST',
            f'/v1/native/sessions/{session_id}:turn',
            payload={
                'text': text,
                'surface': 'line',
                'title': channel_title,
                'actor_display_name': actor_display_name,
                'transport_envelope': dict(transport_envelope),
            },
            actor_identity=actor_identity,
            actor_type='line_user',
        )


class LineBotService:
    def __init__(
        self,
        config: BotConfig,
        *,
        line_api: LineApiClient | None = None,
        ait_api: AitApiClient | None = None,
        state_store: LineSyncStateStore | None = None,
    ):
        self.config = config
        self.line_api = line_api or LineApiClient(config)
        self.ait_api = ait_api or AitApiClient(config)
        self.state_store = state_store or LineSyncStateStore(config.sync_state_path)

    def handle_webhook_payload(self, raw_payload: str, *, signature: str | None = None) -> int:
        verify_webhook_signature(raw_payload, signature, self.config.channel_secret)
        payload = parse_webhook_payload(raw_payload)
        events = payload.get('events') or []
        processed = 0
        for event in events:
            if self.handle_event(event):
                processed += 1
        return processed

    def handle_event(self, event: Mapping[str, Any]) -> bool:
        if str(event.get('type') or '').strip() != 'message':
            return False
        message = event.get('message')
        if not isinstance(message, Mapping) or str(message.get('type') or '').strip() != 'text':
            return False
        text = str(message.get('text') or '').strip()
        if not text:
            return False
        source = event.get('source')
        if not isinstance(source, Mapping):
            raise BotRuntimeError('LINE webhook message event is missing a valid source object.')
        channel_id = _line_channel_id(source)
        if channel_id is None:
            raise BotRuntimeError('LINE webhook message event is missing a usable channel id.')
        channel_kind = _clean_optional_str(source.get('type'))
        channel_title = _line_channel_title(source)
        source_user_id = _clean_optional_str(source.get('userId'))
        webhook_event_id = _clean_optional_str(event.get('webhookEventId')) or _fallback_event_id(channel_id, message, event)
        if self.state_store.has_processed_event(channel_id, webhook_event_id):
            return False
        link = self._ensure_session_link(
            channel_id,
            channel_kind=channel_kind,
            channel_title=channel_title,
            source_user_id=source_user_id,
        )
        actor_identity = _line_actor_identity(source)
        actor_display_name = source_user_id or channel_id
        message_id = _clean_optional_str(message.get('id'))
        reply_token = _clean_optional_str(event.get('replyToken'))
        transport_envelope = build_transport_event_envelope(
            transport='line',
            actor_identity=actor_identity,
            actor_transport_id=source_user_id,
            actor_display_name=actor_display_name,
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            text=text,
            message_id=_normalize_positive_int(message_id),
            message_ids=[message_id] if message_id else None,
            occurred_at=_timestamp_to_iso(event.get('timestamp')),
            event_id=webhook_event_id,
            dedupe_key=webhook_event_id,
            metadata={
                'delivery_mode': _clean_optional_str(event.get('mode')),
                'is_redelivery': bool(((event.get('deliveryContext') or {}) if isinstance(event.get('deliveryContext'), Mapping) else {}).get('isRedelivery')),
                **({'reply_token': reply_token} if reply_token else {}),
            },
        )
        pending = PendingLineReply(
            session_id=str(link.get('session_id') or ''),
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            reply_token=reply_token,
            actor_identity=actor_identity,
            actor_display_name=actor_display_name,
            text=text,
            transport_envelope=transport_envelope,
            source_user_id=source_user_id,
            message_id=message_id,
            webhook_event_id=webhook_event_id,
        )
        self._run_pending_reply(pending)
        return True

    def _ensure_session_link(
        self,
        channel_id: str,
        *,
        channel_kind: str | None,
        channel_title: str,
        source_user_id: str | None,
    ) -> dict[str, Any]:
        link = self.state_store.get_channel(channel_id)
        if link and str(link.get('session_id') or '').strip():
            try:
                self.ait_api.get_session(str(link.get('session_id') or ''))
                return self.state_store.upsert_channel(
                    channel_id,
                    session_id=str(link.get('session_id') or ''),
                    repo_name=self.config.repo_name,
                    channel_title=channel_title,
                    channel_kind=channel_kind,
                    line_source_user_id=source_user_id,
                    line_reply_target={
                        'channel_id': channel_id,
                        'channel_kind': channel_kind,
                        **({'source_user_id': source_user_id} if source_user_id else {}),
                    },
                )
            except BotRuntimeError:
                pass
        session = self.ait_api.create_session(
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            source_user_id=source_user_id,
        )
        return self.state_store.upsert_channel(
            channel_id,
            session_id=str(session.get('session_id') or ''),
            repo_name=self.config.repo_name,
            channel_title=channel_title,
            channel_kind=channel_kind,
            line_source_user_id=source_user_id,
            line_reply_target={
                'channel_id': channel_id,
                'channel_kind': channel_kind,
                **({'source_user_id': source_user_id} if source_user_id else {}),
            },
        )

    def _run_pending_reply(self, pending: PendingLineReply) -> None:
        turn = self.ait_api.create_line_turn(
            pending.session_id,
            text=pending.text,
            channel_id=pending.channel_id,
            channel_title=pending.channel_title,
            channel_kind=pending.channel_kind,
            actor_identity=pending.actor_identity,
            actor_display_name=pending.actor_display_name,
            transport_envelope=pending.transport_envelope,
        )
        user_event = turn.get('user_event') if isinstance(turn, dict) else None
        if not isinstance(user_event, dict):
            raise BotRuntimeError('ait-server returned an invalid LINE turn payload.')
        if turn.get('ok'):
            assistant_event = turn.get('assistant_event') if isinstance(turn.get('assistant_event'), dict) else {}
            reply_text = (
                str(turn.get('reply_text') or '').strip()
                or _assistant_reply_text(assistant_event)
            )
            if reply_text:
                self.line_api.send_text(pending.channel_id, reply_text, reply_token=pending.reply_token)
            self.state_store.remember_processed_event(
                pending.channel_id,
                pending.webhook_event_id,
                source_user_id=pending.source_user_id,
                message_id=pending.message_id,
                reply_token_present=bool(pending.reply_token),
                last_synced_sequence=int(assistant_event.get('sequence') or user_event.get('sequence') or 0),
            )
            return
        error_text = str(turn.get('error') or 'Unknown backend reply error.').strip()
        self.state_store.remember_processed_event(
            pending.channel_id,
            pending.webhook_event_id,
            source_user_id=pending.source_user_id,
            message_id=pending.message_id,
            reply_token_present=bool(pending.reply_token),
            last_synced_sequence=int(user_event.get('sequence') or 0),
        )
        self.line_api.send_text(
            pending.channel_id,
            f'Logged to {pending.session_id} as event #{user_event.get("sequence")}, but the AI reply failed.\n{error_text}',
            reply_token=pending.reply_token,
        )


class _LineWebhookServer(ThreadingHTTPServer):
    daemon_threads = True


class LineWebhookHandler(BaseHTTPRequestHandler):
    service: LineBotService
    webhook_path: str

    def do_POST(self) -> None:  # noqa: N802
        if self.path != self.webhook_path:
            self.send_error(404)
            return
        try:
            content_length = int(self.headers.get('Content-Length') or '0')
        except ValueError:
            content_length = 0
        raw_payload = self.rfile.read(max(content_length, 0)).decode('utf-8')
        signature = self.headers.get('X-Line-Signature')
        try:
            processed = self.service.handle_webhook_payload(raw_payload, signature=signature)
        except BotRuntimeError as exc:
            self._write_json(400, {'ok': False, 'error': str(exc)})
            return
        except Exception as exc:  # pragma: no cover - defensive crash logging for operator runtime
            print(f'ait LINE webhook crashed: {exc}', file=sys.stderr, flush=True)
            self._write_json(500, {'ok': False, 'error': 'internal webhook error'})
            return
        self._write_json(200, {'ok': True, 'processed_events': processed})

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A003
        print(f'LINE webhook {self.address_string()} - {format % args}', flush=True)

    def _write_json(self, status_code: int, payload: dict[str, Any]) -> None:
        encoded = json.dumps(payload, ensure_ascii=False).encode('utf-8')
        self.send_response(status_code)
        self.send_header('Content-Type', 'application/json; charset=utf-8')
        self.send_header('Content-Length', str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def build_webhook_signature(raw_payload: str, channel_secret: str) -> str:
    digest = hmac.new(channel_secret.encode('utf-8'), raw_payload.encode('utf-8'), hashlib.sha256).digest()
    return base64.b64encode(digest).decode('utf-8')


def verify_webhook_signature(raw_payload: str, signature: str | None, channel_secret: str) -> None:
    normalized_signature = str(signature or '').strip()
    if not normalized_signature:
        raise BotRuntimeError('Missing LINE webhook signature header.')
    expected = build_webhook_signature(raw_payload, channel_secret)
    if not hmac.compare_digest(expected, normalized_signature):
        raise BotRuntimeError('Invalid LINE webhook signature.')


def parse_webhook_payload(raw_payload: str) -> dict[str, Any]:
    if not raw_payload.strip():
        raise BotRuntimeError('No LINE webhook payload provided.')
    try:
        payload = json.loads(raw_payload)
    except json.JSONDecodeError as exc:
        raise BotRuntimeError('LINE webhook payload must be valid JSON.') from exc
    if not isinstance(payload, dict):
        raise BotRuntimeError('LINE webhook payload must be a JSON object.')
    events = payload.get('events')
    if not isinstance(events, list):
        raise BotRuntimeError('LINE webhook payload must include an events list.')
    for index, event in enumerate(events):
        if not isinstance(event, dict):
            raise BotRuntimeError(f'LINE webhook event #{index} must be a JSON object.')
    return payload


def run_webhook_events(
    raw_payload: str,
    *,
    signature: str | None = None,
    service: LineBotService | None = None,
    config: BotConfig | None = None,
    repo_root: Path | None = None,
) -> int:
    bot_service = service or LineBotService(config or load_config(repo_root or Path(os.environ.get('AIT_REPO_ROOT') or Path.cwd())))
    return bot_service.handle_webhook_payload(raw_payload, signature=signature)


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
    try:
        parsed = int(str(value or '').strip())
    except ValueError:
        return fallback
    if parsed <= 0:
        return fallback
    return max(parsed, minimum)


def _split_message_chunks(text: str, *, limit: int = DEFAULT_LINE_MESSAGE_LIMIT) -> list[str]:
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
        data = json.dumps(payload, ensure_ascii=False).encode('utf-8')
    else:
        data = None
    if headers:
        request_headers.update(dict(headers))
    request = Request(url, data=data, headers=request_headers, method=method)
    try:
        with urlopen(request, timeout=timeout) as response:
            raw = response.read().decode('utf-8')
    except HTTPError as exc:
        body = exc.read().decode('utf-8', errors='replace')
        raise BotRuntimeError(f'HTTP {exc.code} calling {url}: {body or exc.reason}') from exc
    except URLError as exc:
        raise BotRuntimeError(f'Failed to call {url}: {exc.reason}') from exc
    if not raw.strip():
        return {}
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return raw


def _line_channel_id(source: Mapping[str, Any]) -> str | None:
    return (
        _clean_optional_str(source.get('groupId'))
        or _clean_optional_str(source.get('roomId'))
        or _clean_optional_str(source.get('userId'))
    )


def _line_channel_title(source: Mapping[str, Any]) -> str:
    channel_id = _line_channel_id(source) or 'unknown'
    kind = _clean_optional_str(source.get('type')) or 'chat'
    return f'LINE {kind} · {channel_id}'


def _line_actor_identity(source: Mapping[str, Any]) -> str:
    return f"line:{_clean_optional_str(source.get('userId')) or _line_channel_id(source) or 'unknown'}"


def _fallback_event_id(channel_id: str, message: Mapping[str, Any], event: Mapping[str, Any]) -> str:
    message_id = _clean_optional_str(message.get('id')) or 'message'
    timestamp = _clean_optional_str(event.get('timestamp')) or utc_now_iso()
    return f'line:{channel_id}:{message_id}:{timestamp}'


def _timestamp_to_iso(value: object) -> str | None:
    try:
        timestamp_ms = int(value)
    except (TypeError, ValueError):
        return None
    return datetime.fromtimestamp(timestamp_ms / 1000.0, tz=timezone.utc).isoformat()


def _normalize_positive_int(value: object) -> int | None:
    try:
        parsed = int(str(value or '').strip())
    except (TypeError, ValueError):
        return None
    return parsed if parsed > 0 else None


def _assistant_reply_text(assistant_event: Mapping[str, Any]) -> str:
    payload = assistant_event.get('payload') if isinstance(assistant_event, Mapping) else None
    if not isinstance(payload, Mapping):
        return ''
    direct = str(payload.get('text') or '').strip()
    if direct:
        return direct
    envelope = payload.get('transport_reply_envelope')
    if isinstance(envelope, Mapping):
        message = envelope.get('message')
        if isinstance(message, Mapping):
            return str(message.get('text') or '').strip()
    return ''


def _termination_context_path(env: Mapping[str, str] | None = None) -> Path | None:
    source_env = os.environ if env is None else env
    raw = str(source_env.get(AIT_LINE_TERMINATION_CONTEXT_ENV) or '').strip()
    if not raw:
        return None
    return Path(raw).expanduser()


def _consume_pending_termination_context(*, pid: int | None = None, env: Mapping[str, str] | None = None) -> dict[str, Any] | None:
    path = _termination_context_path(env)
    if path is None or not path.exists():
        return None
    try:
        payload = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    expected_pid = os.getpid() if pid is None else pid
    try:
        context_pid = int(payload.get('pid') or 0)
    except (TypeError, ValueError):
        return None
    if context_pid != expected_pid:
        return None
    try:
        path.unlink(missing_ok=True)
    except OSError:
        pass
    return payload


def _signal_stop_suffix(signum: int) -> str:
    payload = _consume_pending_termination_context()
    if not payload:
        return ''
    details: list[str] = [f'signal={signum}']
    reason = str(payload.get('reason') or '').strip()
    if reason:
        details.append(f'reason={reason}')
    worker_name = str(payload.get('worker_name') or '').strip()
    if worker_name:
        details.append(f'worker={worker_name}')
    return f" ({', '.join(details)})"


def load_config(repo_root: Path | None = None, *, env: Mapping[str, str] | None = None) -> BotConfig:
    resolved_root = repo_root or Path(os.environ.get('AIT_REPO_ROOT') or Path.cwd())
    source_env = os.environ if env is None else env
    env_path = resolve_line_env_path(resolved_root, source_env.get('AIT_LINE_ENV_PATH'))
    values = load_simple_env_file(env_path)
    try:
        runtime_target = resolve_agent_runtime_target(resolved_root)
    except AgentRuntimeConfigError as exc:
        raise BotRuntimeError(str(exc)) from exc
    env_value = lambda *names, default=None: _env_value(values, *names, default=default, env=source_env)
    token = env_value('AIT_LINE_CHANNEL_ACCESS_TOKEN', 'LINE_CHANNEL_ACCESS_TOKEN')
    if not token:
        raise BotRuntimeError(f'Missing LINE channel access token. Set AIT_LINE_CHANNEL_ACCESS_TOKEN or LINE_CHANNEL_ACCESS_TOKEN in {env_path}.')
    secret = env_value('AIT_LINE_CHANNEL_SECRET', 'LINE_CHANNEL_SECRET')
    if not secret:
        raise BotRuntimeError(f'Missing LINE channel secret. Set AIT_LINE_CHANNEL_SECRET or LINE_CHANNEL_SECRET in {env_path}.')
    ait_server_url = runtime_target.server_url
    ait_web_url_raw = env_value('AIT_LINE_WEB_URL', 'AIT_WEB_URL')
    ait_web_url = _normalize_base_url(ait_web_url_raw, ait_server_url) if ait_web_url_raw else None
    request_timeout_seconds = _parse_timeout_seconds(env_value('AIT_LINE_REQUEST_TIMEOUT_SECONDS', 'AIT_LINE_TIMEOUT_SECONDS'), 20.0, 5.0)
    sync_state_path = resolve_line_sync_state_path(env_value('AIT_LINE_STATE_PATH'))
    line_api_base_url = _normalize_base_url(env_value('AIT_LINE_API_BASE_URL', default=DEFAULT_LINE_API_BASE_URL), DEFAULT_LINE_API_BASE_URL)
    bind_host = (env_value('AIT_LINE_BIND_HOST', default='127.0.0.1') or '127.0.0.1').strip() or '127.0.0.1'
    bind_port = _parse_int(env_value('AIT_LINE_BIND_PORT', default='8091'), 8091, 1)
    webhook_path = (env_value('AIT_LINE_WEBHOOK_PATH', default='/callback') or '/callback').strip() or '/callback'
    if not webhook_path.startswith('/'):
        webhook_path = f'/{webhook_path}'
    return BotConfig(
        channel_access_token=token,
        channel_secret=secret,
        ait_server_url=ait_server_url,
        runtime_mode=runtime_target.mode,
        runtime_remote_name=runtime_target.remote_name,
        ait_web_url=ait_web_url,
        repo_name=runtime_target.repo_name,
        request_timeout_seconds=request_timeout_seconds,
        sync_state_path=sync_state_path,
        line_api_base_url=line_api_base_url,
        bind_host=bind_host,
        bind_port=bind_port,
        webhook_path=webhook_path,
    )


def _install_signal_handlers(server: _LineWebhookServer) -> None:
    def _handler(signum, _frame):
        print(f'Received signal {signum}; stopping ait LINE bot{_signal_stop_suffix(signum)}.', flush=True)
        server.shutdown()

    signal.signal(signal.SIGTERM, _handler)
    signal.signal(signal.SIGINT, _handler)


def _handler_factory(service: LineBotService, webhook_path: str):
    class _Handler(LineWebhookHandler):
        pass

    _Handler.service = service
    _Handler.webhook_path = webhook_path
    return _Handler


def webhook_main() -> None:
    repo_root = Path(os.environ.get('AIT_REPO_ROOT') or Path.cwd())
    raw_payload = sys.stdin.read()
    signature = os.environ.get('AIT_LINE_WEBHOOK_SIGNATURE') or os.environ.get('X_LINE_SIGNATURE')
    try:
        run_webhook_events(raw_payload, signature=signature, repo_root=repo_root)
    except BotRuntimeError as exc:
        print(f'ait LINE webhook failed: {exc}', file=sys.stderr, flush=True)
        raise SystemExit(1) from exc
    except Exception as exc:  # pragma: no cover - defensive crash logging for webhook handler
        print(f'ait LINE webhook crashed: {exc}', file=sys.stderr, flush=True)
        raise


def main() -> None:
    repo_root = Path(os.environ.get('AIT_REPO_ROOT') or Path.cwd())
    config = load_config(repo_root)
    config.sync_state_path.parent.mkdir(parents=True, exist_ok=True)
    service = LineBotService(config)
    server = _LineWebhookServer((config.bind_host, config.bind_port), _handler_factory(service, config.webhook_path))
    _install_signal_handlers(server)
    print(
        f'ait LINE bot starting · repo={config.repo_name} · backend={config.runtime_mode}'
        f"{f' · remote={config.runtime_remote_name} · server={config.ait_server_url}' if config.ait_server_url else ''}"
        f' · listen=http://{config.bind_host}:{config.bind_port}{config.webhook_path} · state={config.sync_state_path}',
        flush=True,
    )
    try:
        server.serve_forever(poll_interval=0.5)
    except BotRuntimeError as exc:
        print(f'ait LINE bot failed: {exc}', file=sys.stderr, flush=True)
        raise SystemExit(1) from exc
    except Exception as exc:  # pragma: no cover - defensive crash logging for daemon mode
        print(f'ait LINE bot crashed: {exc}', file=sys.stderr, flush=True)
        raise
    finally:
        server.server_close()


if __name__ == '__main__':  # pragma: no cover
    main()
