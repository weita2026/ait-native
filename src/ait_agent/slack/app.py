from __future__ import annotations

import hashlib
import hmac
import json
import os
import signal
import sys
import threading
import time
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Mapping
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs
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

from .runtime import (
    SlackSyncStateStore,
    load_simple_env_file,
    resolve_slack_env_path,
    resolve_slack_sync_state_path,
    utc_now_iso,
)

DEFAULT_SLACK_ACK_TEXT = 'ait is thinking...'
DEFAULT_SLACK_COMMAND_PATH = '/command'
DEFAULT_SLACK_MESSAGE_LIMIT = 3000
DEFAULT_SLACK_RESPONSE_TYPE = 'in_channel'
DEFAULT_SLACK_SOCKET_OPEN_URL = 'https://slack.com/api/apps.connections.open'
SLACK_SIGNATURE_VERSION = 'v0'
SLACK_TIMESTAMP_TOLERANCE_SECONDS = 60 * 5
AIT_SLACK_TERMINATION_CONTEXT_ENV = 'AIT_SLACK_TERMINATION_CONTEXT_PATH'


class BotRuntimeError(RuntimeError):
    pass


class InvalidSlackSignatureError(BotRuntimeError):
    pass


@dataclass(frozen=True)
class BotConfig:
    signing_secret: str | None
    app_token: str | None
    ait_server_url: str | None
    ait_web_url: str | None
    repo_name: str
    request_timeout_seconds: float | None
    sync_state_path: Path
    bind_host: str
    bind_port: int
    command_path: str
    ack_text: str
    response_type: str
    socket_open_url: str = DEFAULT_SLACK_SOCKET_OPEN_URL
    runtime_mode: str = 'remote'
    runtime_remote_name: str | None = None


@dataclass(frozen=True)
class PendingSlackReply:
    session_id: str
    channel_id: str
    channel_title: str
    channel_kind: str | None
    response_url: str
    request_id: str
    actor_identity: str
    actor_display_name: str | None
    text: str
    transport_envelope: dict[str, Any]
    source_user_id: str | None
    team_id: str | None
    command_name: str | None
    thread_id: str | None


class SlackApiClient:
    def __init__(self, config: BotConfig):
        self.config = config

    def open_socket_url(self) -> str:
        app_token = _require_slack_app_token(self.config)
        response = _json_request(
            self.config.socket_open_url,
            method='POST',
            headers={'Authorization': f'Bearer {app_token}'},
            timeout=self.config.request_timeout_seconds,
        )
        if not isinstance(response, Mapping):
            raise BotRuntimeError('Slack apps.connections.open returned an invalid response payload.')
        if not response.get('ok'):
            error_text = _clean_optional_str(response.get('error')) or 'Unknown Socket Mode open failure.'
            raise BotRuntimeError(f'Slack apps.connections.open failed: {error_text}')
        socket_url = _clean_optional_str(response.get('url'))
        if socket_url is None:
            raise BotRuntimeError('Slack apps.connections.open response did not include a websocket URL.')
        return socket_url

    def send_response(self, response_url: str, text: str, *, response_type: str | None = None) -> None:
        for chunk in _split_message_chunks(text):
            _json_request(
                response_url,
                method='POST',
                payload={
                    'text': chunk,
                    'response_type': response_type or self.config.response_type,
                    'replace_original': False,
                },
                headers={'Content-Type': 'application/json'},
                timeout=self.config.request_timeout_seconds,
            )


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
            raise BotRuntimeError('Slack worker local runtime is unavailable.')
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
        actor_identity: str = 'ait-agent-slack',
        actor_type: str = 'slack_bot',
    ) -> Any:
        if self.config.ait_server_url is None:
            raise BotRuntimeError('Slack worker is in local runtime mode and cannot issue remote HTTP requests.')
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
        team_id: str | None,
        response_url: str,
        thread_id: str | None = None,
        session_kind: str = 'slack_chat',
        title_prefix: str = 'Slack chat',
    ) -> dict[str, Any]:
        metadata_extra = build_transport_binding_metadata(
            transport='slack',
            surface_id=channel_id,
            surface_title=channel_title,
            surface_kind=channel_kind,
            thread_id=thread_id,
            reply_target={
                'channel_id': channel_id,
                'channel_kind': channel_kind,
                'team_id': team_id,
                'response_url': response_url,
                **({'thread_id': thread_id} if thread_id else {}),
                **({'source_user_id': source_user_id} if source_user_id else {}),
            },
        )
        metadata = build_transport_session_metadata(
            transport='slack',
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            thread_id=thread_id,
            linked_via='ait-agent slack',
            metadata_extra={
                'slack_channel_id': channel_id,
                'slack_channel_title': channel_title,
                'slack_channel_kind': channel_kind,
                'slack_team_id': team_id,
                **({'slack_thread_id': thread_id} if thread_id else {}),
                **({'slack_source_user_id': source_user_id} if source_user_id else {}),
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

    def get_session(self, session_id: str, *, actor_identity: str = 'ait-agent-slack') -> dict[str, Any]:
        if self._local_runtime is not None:
            return self._local_call('get_session', session_id)
        return self._request('GET', f'/v1/native/sessions/{session_id}', actor_identity=actor_identity, actor_type='slack_bot')

    def create_slack_turn(
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
                surface='slack',
                title=channel_title,
                actor_identity=actor_identity,
                actor_type='slack_user',
                actor_display_name=actor_display_name,
                transport_envelope=transport_envelope,
            )
        return self._request(
            'POST',
            f'/v1/native/sessions/{session_id}:turn',
            payload={
                'text': text,
                'surface': 'slack',
                'title': channel_title,
                'actor_display_name': actor_display_name,
                'transport_envelope': dict(transport_envelope),
            },
            actor_identity=actor_identity,
            actor_type='slack_user',
        )


class SlackBotService:
    def __init__(
        self,
        config: BotConfig,
        *,
        slack_api: SlackApiClient | None = None,
        ait_api: AitApiClient | None = None,
        state_store: SlackSyncStateStore | None = None,
        defer_replies: bool = True,
    ):
        self.config = config
        self.slack_api = slack_api or SlackApiClient(config)
        self.ait_api = ait_api or AitApiClient(config)
        self.state_store = state_store or SlackSyncStateStore(config.sync_state_path)
        self.defer_replies = bool(defer_replies)

    def handle_command_payload(
        self,
        raw_payload: str,
        *,
        signature: str | None = None,
        signature_timestamp: str | None = None,
    ) -> dict[str, Any]:
        _require_slack_signing_secret(self.config)
        verify_slack_signature(
            raw_payload,
            signature=signature,
            signature_timestamp=signature_timestamp,
            signing_secret=str(self.config.signing_secret or ''),
        )
        payload = parse_command_payload(raw_payload)
        return self._handle_command(payload)

    def handle_socket_envelope(self, envelope: Mapping[str, Any]) -> dict[str, Any]:
        envelope_id = _clean_optional_str(envelope.get('envelope_id'))
        if envelope_id is None:
            raise BotRuntimeError('Slack Socket Mode envelope is missing an envelope id.')
        envelope_type = _clean_optional_str(envelope.get('type'))
        if envelope_type != 'slash_commands':
            return {'envelope_id': envelope_id}
        payload = envelope.get('payload')
        if not isinstance(payload, Mapping):
            raise BotRuntimeError('Slack Socket Mode envelope is missing a slash-command payload object.')
        ack_payload = self._handle_command(payload)
        response: dict[str, Any] = {'envelope_id': envelope_id}
        if bool(envelope.get('accepts_response_payload')):
            response['payload'] = ack_payload
        return response

    def _handle_command(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        if payload.get('ssl_check'):
            return _command_message_response('ok', response_type='ephemeral')
        text = str(payload.get('text') or '').strip()
        if not text:
            return _command_message_response('Slack command must include text content.', response_type='ephemeral')
        channel_id = _required_payload_field(payload, 'channel_id', 'Slack command payload is missing a channel id.')
        response_url = _required_payload_field(payload, 'response_url', 'Slack command payload is missing a response_url.')
        source_user_id = _required_payload_field(payload, 'user_id', 'Slack command payload is missing a user id.')
        command_name = _clean_optional_str(payload.get('command'))
        team_id = _clean_optional_str(payload.get('team_id'))
        channel_name = _clean_optional_str(payload.get('channel_name'))
        thread_id = _clean_optional_str(payload.get('thread_ts'))
        request_id = _slack_request_id(payload)
        if self.state_store.has_processed_command(channel_id, request_id, thread_id=thread_id):
            return _command_message_response('Duplicate Slack command ignored.', response_type='ephemeral')
        channel_kind = _slack_channel_kind(channel_name)
        channel_title = _slack_channel_title(channel_id, channel_name=channel_name)
        link = self._ensure_session_link(
            channel_id,
            channel_kind=channel_kind,
            channel_title=channel_title,
            source_user_id=source_user_id,
            team_id=team_id,
            response_url=response_url,
            thread_id=thread_id,
        )
        actor_identity = _slack_actor_identity(source_user_id)
        actor_display_name = _clean_optional_str(payload.get('user_name')) or source_user_id
        self.state_store.remember_command(
            channel_id,
            request_id,
            thread_id=thread_id,
            source_user_id=source_user_id,
            team_id=team_id,
            command_name=command_name,
        )
        transport_envelope = build_transport_event_envelope(
            transport='slack',
            actor_identity=actor_identity,
            actor_transport_id=source_user_id,
            actor_username=_clean_optional_str(payload.get('user_name')),
            actor_display_name=actor_display_name,
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            thread_id=thread_id,
            text=text,
            occurred_at=utc_now_iso(),
            event_id=request_id,
            dedupe_key=request_id,
            metadata={
                'command_name': command_name,
                'team_id': team_id,
                'response_url_present': True,
            },
        )
        pending = PendingSlackReply(
            session_id=str(link.get('session_id') or ''),
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            response_url=response_url,
            request_id=request_id,
            actor_identity=actor_identity,
            actor_display_name=actor_display_name,
            text=text,
            transport_envelope=transport_envelope,
            source_user_id=source_user_id,
            team_id=team_id,
            command_name=command_name,
            thread_id=thread_id,
        )
        if self.defer_replies:
            self._start_background_reply(pending)
            return _command_message_response(self.config.ack_text, response_type='ephemeral')
        reply_text = self._execute_pending_turn(pending)
        return _command_message_response(reply_text, response_type=self.config.response_type)

    def _start_background_reply(self, pending: PendingSlackReply) -> None:
        worker = threading.Thread(target=self._run_pending_reply_safe, args=(pending,), daemon=True)
        worker.start()

    def _run_pending_reply_safe(self, pending: PendingSlackReply) -> None:
        try:
            reply_text = self._execute_pending_turn(pending)
        except BotRuntimeError as exc:
            error_text = f'ait Slack bot error: {exc}'
            print(error_text, file=sys.stderr, flush=True)
            try:
                self.slack_api.send_response(pending.response_url, error_text, response_type='ephemeral')
            except Exception as reply_exc:  # pragma: no cover
                print(f'ait Slack response delivery failed: {reply_exc}', file=sys.stderr, flush=True)
            return
        if reply_text:
            self.slack_api.send_response(pending.response_url, reply_text, response_type=self.config.response_type)

    def _ensure_session_link(
        self,
        channel_id: str,
        *,
        channel_kind: str | None,
        channel_title: str,
        source_user_id: str | None,
        team_id: str | None,
        response_url: str,
        thread_id: str | None,
    ) -> dict[str, Any]:
        link = self.state_store.get_channel(channel_id, thread_id=thread_id)
        if link and str(link.get('session_id') or '').strip():
            try:
                self.ait_api.get_session(str(link.get('session_id') or ''))
                return self.state_store.upsert_channel(
                    channel_id,
                    thread_id=thread_id,
                    session_id=str(link.get('session_id') or ''),
                    repo_name=self.config.repo_name,
                    channel_title=channel_title,
                    channel_kind=channel_kind,
                    slack_source_user_id=source_user_id,
                    slack_team_id=team_id,
                    slack_reply_target={
                        'channel_id': channel_id,
                        'channel_kind': channel_kind,
                        'team_id': team_id,
                        'response_url': response_url,
                        **({'thread_id': thread_id} if thread_id else {}),
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
            team_id=team_id,
            response_url=response_url,
            thread_id=thread_id,
        )
        return self.state_store.upsert_channel(
            channel_id,
            thread_id=thread_id,
            session_id=str(session.get('session_id') or ''),
            repo_name=self.config.repo_name,
            channel_title=channel_title,
            channel_kind=channel_kind,
            slack_source_user_id=source_user_id,
            slack_team_id=team_id,
            slack_reply_target={
                'channel_id': channel_id,
                'channel_kind': channel_kind,
                'team_id': team_id,
                'response_url': response_url,
                **({'thread_id': thread_id} if thread_id else {}),
                **({'source_user_id': source_user_id} if source_user_id else {}),
            },
        )

    def _execute_pending_turn(self, pending: PendingSlackReply) -> str:
        turn = self.ait_api.create_slack_turn(
            pending.session_id,
            text=pending.text,
            channel_title=pending.channel_title,
            actor_identity=pending.actor_identity,
            actor_display_name=pending.actor_display_name,
            transport_envelope=pending.transport_envelope,
        )
        user_event = turn.get('user_event') if isinstance(turn, dict) else None
        if not isinstance(user_event, dict):
            raise BotRuntimeError('ait-server returned an invalid Slack turn payload.')
        if turn.get('ok'):
            assistant_event = turn.get('assistant_event') if isinstance(turn.get('assistant_event'), dict) else {}
            reply_text = str(turn.get('reply_text') or '').strip() or _assistant_reply_text(assistant_event)
            self.state_store.remember_command(
                pending.channel_id,
                pending.request_id,
                thread_id=pending.thread_id,
                source_user_id=pending.source_user_id,
                team_id=pending.team_id,
                command_name=pending.command_name,
                last_synced_sequence=int(assistant_event.get('sequence') or user_event.get('sequence') or 0),
            )
            return reply_text
        error_text = str(turn.get('error') or 'Unknown backend reply error.').strip()
        self.state_store.remember_command(
            pending.channel_id,
            pending.request_id,
            thread_id=pending.thread_id,
            source_user_id=pending.source_user_id,
            team_id=pending.team_id,
            command_name=pending.command_name,
            last_synced_sequence=int(user_event.get('sequence') or 0),
        )
        return f'Logged to {pending.session_id} as event #{user_event.get("sequence")}, but the AI reply failed.\n{error_text}'


class _SlackCommandServer(ThreadingHTTPServer):
    daemon_threads = True


class SlackCommandHandler(BaseHTTPRequestHandler):
    service: SlackBotService
    command_path: str

    def do_POST(self) -> None:  # noqa: N802
        if self.path != self.command_path:
            self.send_error(404)
            return
        try:
            content_length = int(self.headers.get('Content-Length') or '0')
        except ValueError:
            content_length = 0
        raw_payload = self.rfile.read(max(content_length, 0)).decode('utf-8')
        signature = self.headers.get('X-Slack-Signature')
        signature_timestamp = self.headers.get('X-Slack-Request-Timestamp')
        try:
            response_payload = self.service.handle_command_payload(
                raw_payload,
                signature=signature,
                signature_timestamp=signature_timestamp,
            )
        except InvalidSlackSignatureError as exc:
            self._write_json(401, {'ok': False, 'error': str(exc)})
            return
        except BotRuntimeError as exc:
            self._write_json(400, {'ok': False, 'error': str(exc)})
            return
        except Exception as exc:  # pragma: no cover
            print(f'ait Slack command server crashed: {exc}', file=sys.stderr, flush=True)
            self._write_json(500, {'ok': False, 'error': 'internal slack command error'})
            return
        self._write_json(200, response_payload)

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A003
        print(f'Slack command {self.address_string()} - {format % args}', flush=True)

    def _write_json(self, status_code: int, payload: dict[str, Any]) -> None:
        encoded = json.dumps(payload, ensure_ascii=False).encode('utf-8')
        self.send_response(status_code)
        self.send_header('Content-Type', 'application/json; charset=utf-8')
        self.send_header('Content-Length', str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def build_slack_signature(raw_payload: str, signing_secret: str, *, timestamp: str = '1714990000') -> str:
    base_string = f'{SLACK_SIGNATURE_VERSION}:{timestamp}:{raw_payload}'
    digest = hmac.new(signing_secret.encode('utf-8'), base_string.encode('utf-8'), hashlib.sha256).hexdigest()
    return f'{SLACK_SIGNATURE_VERSION}={digest}'


def verify_slack_signature(
    raw_payload: str,
    *,
    signature: str | None,
    signature_timestamp: str | None,
    signing_secret: str,
    now: float | None = None,
) -> None:
    normalized_signature = str(signature or '').strip()
    if not normalized_signature:
        raise InvalidSlackSignatureError('Missing Slack signature header.')
    normalized_timestamp = str(signature_timestamp or '').strip()
    if not normalized_timestamp:
        raise InvalidSlackSignatureError('Missing Slack timestamp header.')
    try:
        timestamp_value = int(normalized_timestamp)
    except ValueError as exc:
        raise InvalidSlackSignatureError('Invalid Slack timestamp header.') from exc
    current_time = time.time() if now is None else float(now)
    if abs(current_time - timestamp_value) > SLACK_TIMESTAMP_TOLERANCE_SECONDS:
        raise InvalidSlackSignatureError('Slack request timestamp is outside the allowed tolerance.')
    expected = build_slack_signature(raw_payload, signing_secret, timestamp=normalized_timestamp)
    if not hmac.compare_digest(expected, normalized_signature):
        raise InvalidSlackSignatureError('Invalid Slack request signature.')


def parse_command_payload(raw_payload: str) -> dict[str, str]:
    if not raw_payload.strip():
        raise BotRuntimeError('No Slack command payload provided.')
    parsed = parse_qs(raw_payload, keep_blank_values=True)
    if not parsed:
        raise BotRuntimeError('Slack command payload must be form-encoded.')
    return {key: values[-1] if values else '' for key, values in parsed.items()}


def run_command_payload(
    raw_payload: str,
    *,
    signature: str | None = None,
    signature_timestamp: str | None = None,
    service: SlackBotService | None = None,
    config: BotConfig | None = None,
    repo_root: Path | None = None,
) -> dict[str, Any]:
    bot_service = service or SlackBotService(config or load_config(repo_root or Path(os.environ.get('AIT_REPO_ROOT') or Path.cwd())))
    return bot_service.handle_command_payload(
        raw_payload,
        signature=signature,
        signature_timestamp=signature_timestamp,
    )


def _clean_optional_str(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _required_payload_field(payload: Mapping[str, str], key: str, error_text: str) -> str:
    value = _clean_optional_str(payload.get(key))
    if value is None:
        raise BotRuntimeError(error_text)
    return value


def _require_slack_signing_secret(config: BotConfig) -> str:
    secret = _clean_optional_str(config.signing_secret)
    if secret is None:
        raise BotRuntimeError('Missing Slack signing secret for command payload verification.')
    return secret


def _require_slack_app_token(config: BotConfig) -> str:
    token = _clean_optional_str(config.app_token)
    if token is None:
        raise BotRuntimeError('Missing Slack app token. Set AIT_SLACK_APP_TOKEN or SLACK_APP_TOKEN.')
    return token


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


def _split_message_chunks(text: str, *, limit: int = DEFAULT_SLACK_MESSAGE_LIMIT) -> list[str]:
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
    except HTTPError as exc:
        detail = exc.read().decode('utf-8', errors='replace') if exc.fp is not None else ''
        raise BotRuntimeError(f'Slack API request failed with status {exc.code}: {detail or exc.reason}') from exc
    except URLError as exc:
        raise BotRuntimeError(f'Slack API request failed: {exc.reason}') from exc
    if not raw.strip():
        return {}
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return raw


def _command_message_response(text: str, *, response_type: str = 'ephemeral') -> dict[str, Any]:
    return {
        'response_type': response_type,
        'text': str(text or '').strip() or '(empty)',
    }


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


def _slack_actor_identity(user_id: str) -> str:
    return f'slack:{user_id}'


def _slack_channel_kind(channel_name: str | None) -> str:
    if channel_name and channel_name.lower() == 'directmessage':
        return 'dm'
    return 'channel'


def _slack_channel_title(channel_id: str, *, channel_name: str | None) -> str:
    normalized_name = _clean_optional_str(channel_name)
    if normalized_name and normalized_name.lower() != 'directmessage':
        return f'Slack channel · #{normalized_name}'
    if normalized_name and normalized_name.lower() == 'directmessage':
        return f'Slack DM · {channel_id}'
    return f'Slack channel · {channel_id}'


def _slack_request_id(payload: Mapping[str, str]) -> str:
    trigger_id = _clean_optional_str(payload.get('trigger_id'))
    if trigger_id:
        return trigger_id
    channel_id = _clean_optional_str(payload.get('channel_id')) or 'channel'
    user_id = _clean_optional_str(payload.get('user_id')) or 'user'
    command_name = _clean_optional_str(payload.get('command')) or '/ait'
    command_text = _clean_optional_str(payload.get('text')) or 'command'
    return f'slack:{channel_id}:{user_id}:{command_name}:{command_text}'


def _termination_context_path(env: Mapping[str, str] | None = None) -> Path | None:
    source_env = os.environ if env is None else env
    raw = str(source_env.get(AIT_SLACK_TERMINATION_CONTEXT_ENV) or '').strip()
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
    env_path = resolve_slack_env_path(resolved_root, source_env.get('AIT_SLACK_ENV_PATH'))
    values = load_simple_env_file(env_path)
    try:
        runtime_target = resolve_agent_runtime_target(resolved_root)
    except AgentRuntimeConfigError as exc:
        raise BotRuntimeError(str(exc)) from exc
    env_value = lambda *names, default=None: _env_value(values, *names, default=default, env=source_env)
    signing_secret = env_value('AIT_SLACK_SIGNING_SECRET', 'SLACK_SIGNING_SECRET')
    app_token = env_value('AIT_SLACK_APP_TOKEN', 'SLACK_APP_TOKEN')
    ait_server_url = runtime_target.server_url
    ait_web_url_raw = env_value('AIT_SLACK_WEB_URL', 'AIT_WEB_URL')
    ait_web_url = _normalize_base_url(ait_web_url_raw, ait_server_url) if ait_web_url_raw else None
    request_timeout_seconds = _parse_timeout_seconds(env_value('AIT_SLACK_REQUEST_TIMEOUT_SECONDS', 'AIT_SLACK_TIMEOUT_SECONDS'), 20.0, 5.0)
    sync_state_path = resolve_slack_sync_state_path(env_value('AIT_SLACK_STATE_PATH'))
    bind_host = (env_value('AIT_SLACK_BIND_HOST', default='127.0.0.1') or '127.0.0.1').strip() or '127.0.0.1'
    bind_port = _parse_int(env_value('AIT_SLACK_BIND_PORT', default='8093'), 8093, 1)
    command_path = (env_value('AIT_SLACK_COMMAND_PATH', default=DEFAULT_SLACK_COMMAND_PATH) or DEFAULT_SLACK_COMMAND_PATH).strip() or DEFAULT_SLACK_COMMAND_PATH
    if not command_path.startswith('/'):
        command_path = f'/{command_path}'
    ack_text = (env_value('AIT_SLACK_ACK_TEXT', default=DEFAULT_SLACK_ACK_TEXT) or DEFAULT_SLACK_ACK_TEXT).strip() or DEFAULT_SLACK_ACK_TEXT
    response_type = (env_value('AIT_SLACK_RESPONSE_TYPE', default=DEFAULT_SLACK_RESPONSE_TYPE) or DEFAULT_SLACK_RESPONSE_TYPE).strip() or DEFAULT_SLACK_RESPONSE_TYPE
    socket_open_url = _normalize_base_url(
        env_value('AIT_SLACK_SOCKET_OPEN_URL', default=DEFAULT_SLACK_SOCKET_OPEN_URL),
        DEFAULT_SLACK_SOCKET_OPEN_URL,
    )
    return BotConfig(
        signing_secret=signing_secret,
        app_token=app_token,
        ait_server_url=ait_server_url,
        runtime_mode=runtime_target.mode,
        runtime_remote_name=runtime_target.remote_name,
        ait_web_url=ait_web_url,
        repo_name=runtime_target.repo_name,
        request_timeout_seconds=request_timeout_seconds,
        sync_state_path=sync_state_path,
        bind_host=bind_host,
        bind_port=bind_port,
        command_path=command_path,
        ack_text=ack_text,
        response_type=response_type,
        socket_open_url=socket_open_url,
    )


def _install_signal_handlers(server: _SlackCommandServer) -> None:
    def _handler(signum, _frame):
        print(f'Received signal {signum}; stopping ait Slack bot{_signal_stop_suffix(signum)}.', flush=True)
        server.shutdown()

    signal.signal(signal.SIGTERM, _handler)
    signal.signal(signal.SIGINT, _handler)


def _handler_factory(service: SlackBotService, command_path: str):
    class _Handler(SlackCommandHandler):
        pass

    _Handler.service = service
    _Handler.command_path = command_path
    return _Handler


def command_main() -> None:
    repo_root = Path(os.environ.get('AIT_REPO_ROOT') or Path.cwd())
    raw_payload = sys.stdin.read()
    signature = os.environ.get('AIT_SLACK_SIGNATURE') or os.environ.get('X_SLACK_SIGNATURE')
    signature_timestamp = os.environ.get('AIT_SLACK_SIGNATURE_TIMESTAMP') or os.environ.get('X_SLACK_REQUEST_TIMESTAMP')
    try:
        response_payload = run_command_payload(
            raw_payload,
            signature=signature,
            signature_timestamp=signature_timestamp,
            repo_root=repo_root,
        )
    except BotRuntimeError as exc:
        print(f'ait Slack command failed: {exc}', file=sys.stderr, flush=True)
        raise SystemExit(1) from exc
    except Exception as exc:  # pragma: no cover
        print(f'ait Slack command crashed: {exc}', file=sys.stderr, flush=True)
        raise
    print(json.dumps(response_payload, ensure_ascii=False), flush=True)


def main() -> None:
    try:
        from websockets.sync.client import connect
    except ImportError as exc:  # pragma: no cover - dependency error is environment-specific
        raise SystemExit("Missing Python dependency 'websockets'. Install project dependencies before using Slack Socket Mode.") from exc

    repo_root = Path(os.environ.get('AIT_REPO_ROOT') or Path.cwd())
    config = load_config(repo_root)
    config.sync_state_path.parent.mkdir(parents=True, exist_ok=True)
    service = SlackBotService(config)
    slack_api = service.slack_api
    stop_event = threading.Event()
    _install_stop_signal_handlers(stop_event, transport_label='Slack Socket Mode bot')
    print(
        f'ait Slack bot starting · repo={config.repo_name} · backend={config.runtime_mode}'
        f"{f' · remote={config.runtime_remote_name} · server={config.ait_server_url}' if config.ait_server_url else ''}"
        f' · socket_mode={config.socket_open_url} · state={config.sync_state_path}',
        flush=True,
    )
    reconnect_delay_seconds = 2.0
    while not stop_event.is_set():
        try:
            socket_url = slack_api.open_socket_url()
            print(f'ait Slack bot connected · websocket={socket_url}', flush=True)
            with connect(
                socket_url,
                open_timeout=config.request_timeout_seconds,
                close_timeout=1,
                ping_interval=20,
                ping_timeout=20,
                max_size=None,
            ) as websocket:
                while not stop_event.is_set():
                    try:
                        raw_message = websocket.recv(timeout=1.0)
                    except TimeoutError:
                        continue
                    if raw_message is None:
                        break
                    if not isinstance(raw_message, str):
                        raw_message = str(raw_message)
                    try:
                        message = json.loads(raw_message)
                    except json.JSONDecodeError:
                        print('ait Slack bot ignored non-JSON websocket payload.', file=sys.stderr, flush=True)
                        continue
                    if not isinstance(message, Mapping):
                        print('ait Slack bot ignored non-object websocket payload.', file=sys.stderr, flush=True)
                        continue
                    message_type = _clean_optional_str(message.get('type'))
                    if message_type == 'hello':
                        continue
                    if message_type == 'disconnect':
                        reason = _clean_optional_str(message.get('reason')) or 'socket_disconnect'
                        print(f'ait Slack bot reconnecting after Slack disconnect: {reason}', flush=True)
                        break
                    envelope_id = _clean_optional_str(message.get('envelope_id'))
                    if envelope_id is None:
                        continue
                    try:
                        ack_payload = service.handle_socket_envelope(message)
                    except BotRuntimeError as exc:
                        ack_payload = {'envelope_id': envelope_id}
                        if bool(message.get('accepts_response_payload')):
                            ack_payload['payload'] = _command_message_response(
                                f'ait Slack bot error: {exc}',
                                response_type='ephemeral',
                            )
                        print(f'ait Slack bot envelope failed: {exc}', file=sys.stderr, flush=True)
                    websocket.send(json.dumps(ack_payload, ensure_ascii=False))
        except BotRuntimeError as exc:
            print(f'ait Slack bot failed: {exc}', file=sys.stderr, flush=True)
        except Exception as exc:  # pragma: no cover
            print(f'ait Slack bot crashed: {exc}', file=sys.stderr, flush=True)
        if not stop_event.is_set():
            time.sleep(reconnect_delay_seconds)


def _install_stop_signal_handlers(stop_event: threading.Event, *, transport_label: str) -> None:
    def _handler(signum, _frame):
        print(f'Received signal {signum}; stopping {transport_label}{_signal_stop_suffix(signum)}.', flush=True)
        stop_event.set()

    signal.signal(signal.SIGTERM, _handler)
    signal.signal(signal.SIGINT, _handler)


if __name__ == '__main__':  # pragma: no cover
    main()
