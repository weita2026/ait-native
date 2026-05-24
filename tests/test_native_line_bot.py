from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from ait_agent.line.app import BotConfig, LineBotService, build_webhook_signature, parse_webhook_payload, run_webhook_events
from ait_agent.line.runtime import LineSyncStateStore


class FakeLineApi:
    def __init__(self):
        self.sent_messages: list[tuple[str, str | None, str]] = []

    def send_text(self, channel_id: str, text: str, *, reply_token: str | None = None) -> None:
        self.sent_messages.append((channel_id, reply_token, text))


class FakeAitApi:
    def __init__(self, reply_text: str = 'AI says hello.'):
        self.reply_text = reply_text
        self._next_session = 1
        self.turn_calls: list[dict[str, Any]] = []
        self.sessions: dict[str, dict[str, Any]] = {}

    def create_session(self, *, channel_id: str, channel_title: str, channel_kind: str | None, source_user_id: str | None, session_kind: str = 'line_chat', title_prefix: str = 'LINE chat') -> dict[str, Any]:
        session_id = f'AITS-LINE-{self._next_session:04d}'
        self._next_session += 1
        payload = {
            'session_id': session_id,
            'title': f'{title_prefix} · {channel_title}',
            'metadata': {
                'line_channel_id': channel_id,
                'line_channel_kind': channel_kind,
                'line_source_user_id': source_user_id,
            },
        }
        self.sessions[session_id] = payload
        return payload

    def create_line_turn(self, session_id: str, **kwargs: Any) -> dict[str, Any]:
        self.turn_calls.append({'session_id': session_id, **kwargs})
        return {
            'ok': True,
            'session_id': session_id,
            'user_event': {'sequence': 1, 'payload': {'transport_envelope': kwargs['transport_envelope']}},
            'assistant_event': {
                'sequence': 2,
                'payload': {
                    'text': self.reply_text,
                    'transport_reply_envelope': {
                        'transport': 'line',
                        'message': {'text': self.reply_text},
                    },
                },
            },
            'reply_text': self.reply_text,
        }


def _config(state_path: Path) -> BotConfig:
    return BotConfig(
        channel_access_token='line-access-token',
        channel_secret='line-channel-secret',
        ait_server_url='http://127.0.0.1:8088',
        ait_web_url=None,
        repo_name='ait',
        request_timeout_seconds=20.0,
        sync_state_path=state_path,
        line_api_base_url='https://api.line.me',
        bind_host='127.0.0.1',
        bind_port=8091,
        webhook_path='/callback',
    )


def test_parse_webhook_payload_requires_event_list():
    with pytest.raises(Exception):
        parse_webhook_payload('{}')


def test_line_service_handles_text_event_end_to_end_and_dedupes(tmp_path: Path):
    state_path = tmp_path / 'line-sync.json'
    config = _config(state_path)
    line_api = FakeLineApi()
    ait_api = FakeAitApi()
    service = LineBotService(
        config,
        line_api=line_api,
        ait_api=ait_api,
        state_store=LineSyncStateStore(state_path),
    )
    payload = {
        'destination': 'U-bot',
        'events': [
            {
                'type': 'message',
                'replyToken': 'reply-token-1',
                'webhookEventId': '01HXLINEEVENT001',
                'timestamp': 1714990000000,
                'source': {'type': 'user', 'userId': 'U-user-1'},
                'message': {'id': '987654321', 'type': 'text', 'text': 'Hello from LINE'},
            }
        ],
    }
    raw_payload = json.dumps(payload, ensure_ascii=False)
    signature = build_webhook_signature(raw_payload, config.channel_secret)

    processed = run_webhook_events(raw_payload, signature=signature, service=service)

    assert processed == 1
    assert len(ait_api.turn_calls) == 1
    assert ait_api.turn_calls[0]['transport_envelope']['transport'] == 'line'
    assert ait_api.turn_calls[0]['transport_envelope']['event_id'] == '01HXLINEEVENT001'
    assert ait_api.turn_calls[0]['transport_envelope']['channel']['channel_id'] == 'U-user-1'
    assert line_api.sent_messages == [('U-user-1', 'reply-token-1', 'AI says hello.')]

    binding = service.state_store.get_channel('U-user-1')
    assert binding is not None
    assert binding['session_id'] == 'AITS-LINE-0001'
    assert '01HXLINEEVENT001' in binding['line_recent_webhook_event_ids']

    processed_again = run_webhook_events(raw_payload, signature=signature, service=service)

    assert processed_again == 0
    assert len(ait_api.turn_calls) == 1
    assert len(line_api.sent_messages) == 1


def test_line_service_rejects_bad_signature(tmp_path: Path):
    state_path = tmp_path / 'line-sync.json'
    config = _config(state_path)
    service = LineBotService(config, line_api=FakeLineApi(), ait_api=FakeAitApi(), state_store=LineSyncStateStore(state_path))
    raw_payload = json.dumps({'destination': 'U-bot', 'events': []}, ensure_ascii=False)

    with pytest.raises(Exception):
        service.handle_webhook_payload(raw_payload, signature='bad-signature')
