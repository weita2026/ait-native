from __future__ import annotations

import json
from dataclasses import replace
from pathlib import Path
from typing import Any

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

from ait.repo_paths import RepoContext
from ait.store import add_remote, init_repo, load_config as load_repo_config, save_config as save_repo_config
from ait_agent.discord.app import (
    AitApiClient,
    BotConfig,
    DiscordBotService,
    BotRuntimeError,
    DiscordApiClient,
    DEFAULT_DISCORD_HTTP_USER_AGENT,
    DISCORD_DIRECT_MESSAGES_INTENT,
    DISCORD_GUILD_MESSAGES_INTENT,
    DISCORD_MESSAGE_CONTENT_INTENT,
    InvalidInteractionSignatureError,
    _drop_message_content_intent,
    _resolve_gateway_base_url,
    _should_drop_message_content_intent_for_gateway_error,
    load_config,
    parse_interaction_payload,
    run_interaction_payload,
)
from ait_agent.discord.runtime import DiscordSyncStateStore


class FakeDiscordApi:
    def __init__(self):
        self.edited_messages: list[tuple[str, str, str]] = []
        self.followup_messages: list[tuple[str, str, str]] = []
        self.channel_messages: list[tuple[str, str]] = []
        self.followup_attachments: list[tuple[str, str, str, str]] = []
        self.channel_attachments: list[tuple[str, str, str]] = []
        self.channel_history: dict[str, list[dict[str, Any]]] = {}
        self._next_message_id = 1

    def _message_id(self) -> str:
        message_id = f'discord-out-{self._next_message_id:04d}'
        self._next_message_id += 1
        return message_id

    def _record_channel_message(self, channel_id: str, text: str, *, bot: bool = True) -> str:
        message_id = self._message_id()
        history = self.channel_history.setdefault(channel_id, [])
        history.insert(
            0,
            {
                'id': message_id,
                'content': text,
                'author': {
                    'id': 'discord-bot-user',
                    'bot': bot,
                },
                'channel_id': channel_id,
            },
        )
        return message_id

    def _record_channel_attachment(self, channel_id: str, file_name: str, caption: str = '', *, bot: bool = True) -> str:
        message_id = self._message_id()
        history = self.channel_history.setdefault(channel_id, [])
        history.insert(
            0,
            {
                'id': message_id,
                'content': caption,
                'attachments': [{'id': message_id, 'filename': file_name}],
                'author': {
                    'id': 'discord-bot-user',
                    'bot': bot,
                },
                'channel_id': channel_id,
            },
        )
        return message_id

    def edit_original_response(self, application_id: str, interaction_token: str, text: str) -> list[str]:
        self.edited_messages.append((application_id, interaction_token, text))
        return []

    def send_followup(self, application_id: str, interaction_token: str, text: str) -> list[str]:
        self.followup_messages.append((application_id, interaction_token, text))
        return [self._message_id()]

    def send_followup_attachment(self, application_id: str, interaction_token: str, attachment: dict[str, Any]) -> list[str]:
        file_name = str(attachment.get('file_name') or Path(str(attachment.get('local_path') or '')).name or 'attachment.bin')
        caption = str(attachment.get('caption') or '')
        self.followup_attachments.append((application_id, interaction_token, file_name, caption))
        return [self._message_id()]

    def send_channel_message(self, channel_id: str, text: str) -> list[str]:
        self.channel_messages.append((channel_id, text))
        return [self._record_channel_message(channel_id, text)]

    def send_channel_attachment(self, channel_id: str, attachment: dict[str, Any]) -> list[str]:
        file_name = str(attachment.get('file_name') or Path(str(attachment.get('local_path') or '')).name or 'attachment.bin')
        caption = str(attachment.get('caption') or '')
        self.channel_attachments.append((channel_id, file_name, caption))
        return [self._record_channel_attachment(channel_id, file_name, caption)]

    def list_channel_messages(self, channel_id: str, *, limit: int = 25) -> list[dict[str, Any]]:
        return list(self.channel_history.get(channel_id, []))[:limit]


class FakeAitApi:
    def __init__(
        self,
        reply_text: str = 'AI says hello from Discord.',
        *,
        reply_attachments: list[dict[str, Any]] | None = None,
        turn_exception: BaseException | None = None,
        recovery_events: list[dict[str, Any]] | None = None,
    ):
        self.reply_text = reply_text
        self.reply_attachments = list(reply_attachments or [])
        self.turn_exception = turn_exception
        self.recovery_events = list(recovery_events or [])
        self._next_session = 1
        self.turn_calls: list[dict[str, Any]] = []
        self.session_create_calls: list[dict[str, Any]] = []
        self.sessions: dict[str, dict[str, Any]] = {}

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
        self.session_create_calls.append(
            {
                'channel_id': channel_id,
                'channel_title': channel_title,
                'channel_kind': channel_kind,
                'source_user_id': source_user_id,
                'guild_id': guild_id,
                'application_id': application_id,
                'session_kind': session_kind,
                'title_prefix': title_prefix,
                'binding_role': binding_role,
                'canonical_session_id': canonical_session_id,
                'active_session_id': active_session_id,
                'branch_session_id': branch_session_id,
                'branch_kind': branch_kind,
                'relink_reason': relink_reason,
                'metadata_extra': dict(metadata_extra or {}),
            }
        )
        session_id = f'AITS-DISCORD-{self._next_session:04d}'
        self._next_session += 1
        payload = {
            'session_id': session_id,
            'title': f'{title_prefix} · {channel_title}',
            'metadata': {
                'discord_channel_id': channel_id,
                'discord_channel_kind': channel_kind,
                'discord_source_user_id': source_user_id,
                'discord_guild_id': guild_id,
                'discord_application_id': application_id,
                **dict(metadata_extra or {}),
            },
        }
        self.sessions[session_id] = payload
        return payload

    def get_session(self, session_id: str, *, actor_identity: str = 'ait-agent-discord') -> dict[str, Any]:
        del actor_identity
        if session_id not in self.sessions:
            raise BotRuntimeError(f'Unknown test session: {session_id}')
        return dict(self.sessions[session_id])

    def create_discord_turn(self, session_id: str, **kwargs: Any) -> dict[str, Any]:
        self.turn_calls.append({'session_id': session_id, **kwargs})
        if self.turn_exception is not None:
            raise self.turn_exception
        return {
            'ok': True,
            'session_id': session_id,
            'user_event': {'sequence': 1, 'payload': {'transport_envelope': kwargs['transport_envelope']}},
            'assistant_event': {
                'sequence': 2,
                'payload': {
                    'reply_to_sequence': 1,
                    'text': self.reply_text,
                    'transport_reply_envelope': {
                        'transport': 'discord',
                        'message': {
                            'text': self.reply_text,
                            'attachments': self.reply_attachments,
                        },
                    },
                },
            },
            'reply_text': self.reply_text,
        }

    def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50) -> list[dict[str, Any]]:
        del session_id, limit
        return [event for event in self.recovery_events if int(event.get('sequence') or 0) > after_sequence]


def _keypair() -> tuple[Ed25519PrivateKey, str]:
    private_key = Ed25519PrivateKey.generate()
    public_key = private_key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw).hex()
    return private_key, public_key


def _config(state_path: Path, *, public_key: str, application_id: str = '123456789012345678') -> BotConfig:
    return BotConfig(
        application_id=application_id,
        public_key=public_key,
        bot_token=None,
        ait_server_url='http://127.0.0.1:8088',
        ait_web_url=None,
        repo_name='ait',
        request_timeout_seconds=20.0,
        turn_timeout_seconds=300.0,
        sync_state_path=state_path,
        discord_api_base_url='https://discord.com/api/v10',
        discord_http_user_agent=DEFAULT_DISCORD_HTTP_USER_AGENT,
        bind_host='127.0.0.1',
        bind_port=8092,
        interaction_path='/interactions',
        gateway_intents=0,
    )


def _interaction_payload(*, interaction_id: str = '112233445566778899', interaction_token: str = 'discord-token-1', text: str = 'Hello from Discord') -> dict[str, Any]:
    return {
        'id': interaction_id,
        'type': 2,
        'token': interaction_token,
        'application_id': '123456789012345678',
        'channel_id': '998877665544332211',
        'guild_id': '556677889900112233',
        'data': {
            'id': '887766554433221100',
            'name': 'ask',
            'type': 1,
            'options': [
                {
                    'name': 'text',
                    'type': 3,
                    'value': text,
                }
            ],
        },
        'member': {
            'user': {
                'id': 'U-discord-1',
                'username': 'weita',
                'global_name': 'WeiTa',
            }
        },
    }


def _message_payload(*, message_id: str = '998899887777666655', text: str = 'Hello from Discord chat') -> dict[str, Any]:
    return {
        'id': message_id,
        'type': 0,
        'channel_id': '998877665544332211',
        'guild_id': '556677889900112233',
        'content': text,
        'author': {
            'id': 'U-discord-1',
            'username': 'weita',
            'global_name': 'WeiTa',
            'bot': False,
        },
    }


def _sign_payload(raw_payload: str, private_key: Ed25519PrivateKey, *, timestamp: str = '1714990000') -> tuple[str, str]:
    signature = private_key.sign(f'{timestamp}{raw_payload}'.encode('utf-8')).hex()
    return signature, timestamp


def test_parse_interaction_payload_requires_type():
    with pytest.raises(Exception):
        parse_interaction_payload('{}')


def test_discord_service_handles_ping_interaction(tmp_path: Path):
    private_key, public_key = _keypair()
    config = _config(tmp_path / 'discord-sync.json', public_key=public_key)
    service = DiscordBotService(
        config,
        discord_api=FakeDiscordApi(),
        ait_api=FakeAitApi(),
        state_store=DiscordSyncStateStore(config.sync_state_path),
    )
    raw_payload = json.dumps({'type': 1}, ensure_ascii=False)
    signature, timestamp = _sign_payload(raw_payload, private_key)

    response = service.handle_interaction_payload(
        raw_payload,
        signature=signature,
        signature_timestamp=timestamp,
    )

    assert response == {'type': 1}


def test_discord_service_handles_text_interaction_end_to_end_and_dedupes(tmp_path: Path):
    private_key, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    discord_api = FakeDiscordApi()
    ait_api = FakeAitApi()
    service = DiscordBotService(
        config,
        discord_api=discord_api,
        ait_api=ait_api,
        state_store=DiscordSyncStateStore(state_path),
    )
    service._start_background_reply = lambda pending: service._run_pending_followup_safe(pending)  # type: ignore[method-assign]
    payload = _interaction_payload()
    raw_payload = json.dumps(payload, ensure_ascii=False)
    signature, timestamp = _sign_payload(raw_payload, private_key)

    response = run_interaction_payload(
        raw_payload,
        signature=signature,
        signature_timestamp=timestamp,
        service=service,
    )

    assert response == {'type': 5}
    assert len(ait_api.turn_calls) == 1
    assert ait_api.turn_calls[0]['transport_envelope']['transport'] == 'discord'
    assert ait_api.turn_calls[0]['transport_envelope']['event_id'] == '112233445566778899'
    assert ait_api.turn_calls[0]['transport_envelope']['channel']['channel_id'] == '998877665544332211'
    assert discord_api.edited_messages == [('123456789012345678', 'discord-token-1', 'AI says hello from Discord.')]
    assert discord_api.followup_messages == []

    binding = service.state_store.get_channel('998877665544332211')
    assert binding is not None
    assert binding['session_id'] == 'AITS-DISCORD-0001'
    assert '112233445566778899' in binding['discord_recent_interaction_ids']

    duplicate = run_interaction_payload(
        raw_payload,
        signature=signature,
        signature_timestamp=timestamp,
        service=service,
    )

    assert duplicate['type'] == 4


def test_discord_service_fresh_topic_interaction_creates_new_session_without_ai_turn(tmp_path: Path):
    private_key, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    discord_api = FakeDiscordApi()
    ait_api = FakeAitApi()
    service = DiscordBotService(
        config,
        discord_api=discord_api,
        ait_api=ait_api,
        state_store=DiscordSyncStateStore(state_path),
    )
    service._start_background_reply = lambda pending: service._run_pending_followup_safe(pending)  # type: ignore[method-assign]

    first_payload = _interaction_payload(text='Hello from Discord')
    first_raw_payload = json.dumps(first_payload, ensure_ascii=False)
    first_signature, first_timestamp = _sign_payload(first_raw_payload, private_key)
    first_response = run_interaction_payload(
        first_raw_payload,
        signature=first_signature,
        signature_timestamp=first_timestamp,
        service=service,
    )

    trigger_payload = _interaction_payload(
        interaction_id='112233445566778900',
        interaction_token='discord-token-2',
        text='換個話題',
    )
    trigger_raw_payload = json.dumps(trigger_payload, ensure_ascii=False)
    trigger_signature, trigger_timestamp = _sign_payload(trigger_raw_payload, private_key, timestamp='1714990001')
    trigger_response = run_interaction_payload(
        trigger_raw_payload,
        signature=trigger_signature,
        signature_timestamp=trigger_timestamp,
        service=service,
    )

    assert first_response == {'type': 5}
    assert trigger_response['type'] == 4
    assert trigger_response['data']['content'].startswith('Started a fresh Discord-linked session.')
    assert len(ait_api.turn_calls) == 1
    assert len(ait_api.session_create_calls) == 2
    assert ait_api.session_create_calls[-1]['relink_reason'] == 'fresh_topic_event_trigger'
    assert discord_api.edited_messages == [('123456789012345678', 'discord-token-1', 'AI says hello from Discord.')]

    binding = service.state_store.get_channel('998877665544332211')
    assert binding is not None
    assert binding['session_id'] == 'AITS-DISCORD-0002'
    assert binding['canonical_session_id'] == 'AITS-DISCORD-0002'
    assert binding['previous_session_id'] == 'AITS-DISCORD-0001'
    assert binding['relink_reason'] == 'fresh_topic_event_trigger'
    assert '112233445566778900' in binding['discord_recent_interaction_ids']


def test_discord_gateway_info_uses_explicit_user_agent(tmp_path: Path, monkeypatch):
    config = replace(
        _config(tmp_path / 'discord-sync.json', public_key='ab' * 32),
        bot_token='discord-bot-token',
    )
    captured: dict[str, Any] = {}

    class _Response:
        status = 200

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb):
            return False

        def read(self) -> bytes:
            return json.dumps({'url': 'wss://gateway.discord.gg', 'shards': 1}).encode('utf-8')

    def fake_urlopen(request, timeout=0):
        captured['timeout'] = timeout
        captured['headers'] = dict(request.header_items())
        captured['method'] = request.get_method()
        return _Response()

    monkeypatch.setattr('ait_agent.discord.clients.urlopen', fake_urlopen)

    payload = DiscordApiClient(config).gateway_info()

    assert payload['url'] == 'wss://gateway.discord.gg'
    assert captured['method'] == 'GET'
    assert captured['headers']['Authorization'] == 'Bot discord-bot-token'
    assert captured['headers']['User-agent'] == DEFAULT_DISCORD_HTTP_USER_AGENT


def test_resolve_gateway_base_url_reuses_resume_gateway_url_without_refetching(tmp_path: Path):
    config = replace(
        _config(tmp_path / 'discord-sync.json', public_key='ab' * 32),
        bot_token='discord-bot-token',
    )
    discord_api = DiscordApiClient(config)

    def fail_gateway_info() -> dict[str, Any]:
        raise AssertionError('gateway_info should not be called when resume gateway URL is available.')

    discord_api.gateway_info = fail_gateway_info  # type: ignore[method-assign]

    assert _resolve_gateway_base_url(
        discord_api,
        session_id='discord-session-1',
        resume_gateway_url='wss://gateway.discord.gg',
    ) == 'wss://gateway.discord.gg'


def test_resolve_gateway_base_url_fetches_gateway_info_without_resume_url(tmp_path: Path):
    config = replace(
        _config(tmp_path / 'discord-sync.json', public_key='ab' * 32),
        bot_token='discord-bot-token',
    )
    discord_api = DiscordApiClient(config)
    calls = {'count': 0}

    def fake_gateway_info() -> dict[str, Any]:
        calls['count'] += 1
        return {'url': 'wss://gateway-us-east1.discord.gg'}

    discord_api.gateway_info = fake_gateway_info  # type: ignore[method-assign]

    assert _resolve_gateway_base_url(
        discord_api,
        session_id='discord-session-1',
        resume_gateway_url=None,
    ) == 'wss://gateway-us-east1.discord.gg'
    assert calls['count'] == 1


def test_discord_service_rejects_bad_signature(tmp_path: Path):
    _, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    service = DiscordBotService(
        config,
        discord_api=FakeDiscordApi(),
        ait_api=FakeAitApi(),
        state_store=DiscordSyncStateStore(state_path),
    )
    raw_payload = json.dumps(_interaction_payload(), ensure_ascii=False)

    with pytest.raises(InvalidInteractionSignatureError):
        service.handle_interaction_payload(
            raw_payload,
            signature='00' * 64,
            signature_timestamp='1714990000',
        )


def test_discord_service_handles_plain_message_end_to_end_and_dedupes(tmp_path: Path):
    _, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    discord_api = FakeDiscordApi()
    ait_api = FakeAitApi(reply_text='AI says hello from plain Discord chat.')
    service = DiscordBotService(
        config,
        discord_api=discord_api,
        ait_api=ait_api,
        state_store=DiscordSyncStateStore(state_path),
    )
    service._start_background_reply = lambda pending: service._run_pending_followup_safe(pending)  # type: ignore[method-assign]

    accepted = service.handle_message(_message_payload())

    assert accepted is True
    assert len(ait_api.turn_calls) == 1
    assert ait_api.turn_calls[0]['transport_envelope']['transport'] == 'discord'
    assert ait_api.turn_calls[0]['transport_envelope']['event_id'] == '998899887777666655'
    assert ait_api.turn_calls[0]['transport_envelope']['channel']['channel_id'] == '998877665544332211'
    assert discord_api.channel_messages == [('998877665544332211', 'AI says hello from plain Discord chat.')]
    assert discord_api.edited_messages == []
    assert discord_api.followup_messages == []

    binding = service.state_store.get_channel('998877665544332211')
    assert binding is not None
    assert binding['session_id'] == 'AITS-DISCORD-0001'
    assert '998899887777666655' in binding['discord_recent_message_ids']
    assert binding['discord_live_delivered_sequences'] == [2]
    assert binding['discord_live_outbound_message_ids'] == ['discord-out-0001']
    assert binding['discord_last_live_delivery_mode'] == 'channel_message'
    assert binding['discord_last_live_reply_sequence'] == 2
    assert binding['discord_last_live_reply_to_sequence'] == 1

    duplicate = service.handle_message(_message_payload())

    assert duplicate is False
    assert len(ait_api.turn_calls) == 1
    assert len(discord_api.channel_messages) == 1


def test_discord_service_fresh_topic_message_creates_new_session_without_ai_turn(tmp_path: Path):
    _, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    discord_api = FakeDiscordApi()
    ait_api = FakeAitApi(reply_text='AI says hello from plain Discord chat.')
    service = DiscordBotService(
        config,
        discord_api=discord_api,
        ait_api=ait_api,
        state_store=DiscordSyncStateStore(state_path),
    )
    service._start_background_reply = lambda pending: service._run_pending_followup_safe(pending)  # type: ignore[method-assign]

    first_accepted = service.handle_message(_message_payload())
    trigger_accepted = service.handle_message(
        _message_payload(message_id='998899887777666656', text='換個話題')
    )

    assert first_accepted is True
    assert trigger_accepted is True
    assert len(ait_api.turn_calls) == 1
    assert len(ait_api.session_create_calls) == 2
    assert ait_api.session_create_calls[-1]['relink_reason'] == 'fresh_topic_event_trigger'
    assert discord_api.channel_messages == [
        ('998877665544332211', 'AI says hello from plain Discord chat.'),
        (
            '998877665544332211',
            'Started a fresh Discord-linked session.\nTrigger: 換個話題.\nSession: AITS-DISCORD-0002',
        ),
    ]

    binding = service.state_store.get_channel('998877665544332211')
    assert binding is not None
    assert binding['session_id'] == 'AITS-DISCORD-0002'
    assert binding['canonical_session_id'] == 'AITS-DISCORD-0002'
    assert binding['previous_session_id'] == 'AITS-DISCORD-0001'
    assert binding['relink_reason'] == 'fresh_topic_event_trigger'
    assert '998899887777666656' in binding['discord_recent_message_ids']


def test_discord_service_ignores_bot_authored_messages(tmp_path: Path):
    _, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    discord_api = FakeDiscordApi()
    ait_api = FakeAitApi()
    service = DiscordBotService(
        config,
        discord_api=discord_api,
        ait_api=ait_api,
        state_store=DiscordSyncStateStore(state_path),
    )

    ignored = service.handle_message(
        {
            **_message_payload(),
            'author': {
                'id': 'U-discord-bot',
                'username': 'other-bot',
                'global_name': 'OtherBot',
                'bot': True,
            },
        }
    )

    assert ignored is False
    assert ait_api.turn_calls == []
    assert discord_api.channel_messages == []


def test_discord_service_sends_channel_reply_attachments(tmp_path: Path):
    _, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    export_path = tmp_path / 'whitepaper_bundle.md'
    export_path.write_text('# Whitepaper bundle\n', encoding='utf-8')
    discord_api = FakeDiscordApi()
    ait_api = FakeAitApi(
        reply_attachments=[
            {
                'kind': 'document',
                'file_name': export_path.name,
                'mime_type': 'text/markdown',
                'caption': 'Whitepaper bundle',
                'local_path': str(export_path),
            }
        ]
    )
    service = DiscordBotService(
        config,
        discord_api=discord_api,
        ait_api=ait_api,
        state_store=DiscordSyncStateStore(state_path),
    )
    service._start_background_reply = lambda pending: service._run_pending_followup_safe(pending)  # type: ignore[method-assign]

    accepted = service.handle_message(_message_payload())

    assert accepted is True
    assert discord_api.channel_messages == [('998877665544332211', 'AI says hello from Discord.')]
    assert discord_api.channel_attachments == [
        ('998877665544332211', 'whitepaper_bundle.md', 'Whitepaper bundle')
    ]
    binding = service.state_store.get_channel('998877665544332211')
    assert binding is not None
    assert binding['last_synced_sequence'] == 2
    assert binding['discord_live_delivered_sequences'] == [2]
    assert binding['discord_live_outbound_message_ids'] == ['discord-out-0001', 'discord-out-0002']


def test_discord_service_sends_interaction_followup_attachments(tmp_path: Path):
    private_key, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    export_path = tmp_path / 'whitepaper_bundle.md'
    export_path.write_text('# Whitepaper bundle\n', encoding='utf-8')
    discord_api = FakeDiscordApi()
    ait_api = FakeAitApi(
        reply_attachments=[
            {
                'kind': 'document',
                'file_name': export_path.name,
                'mime_type': 'text/markdown',
                'caption': 'Whitepaper bundle',
                'local_path': str(export_path),
            }
        ]
    )
    service = DiscordBotService(
        config,
        discord_api=discord_api,
        ait_api=ait_api,
        state_store=DiscordSyncStateStore(state_path),
    )
    service._start_background_reply = lambda pending: service._run_pending_followup_safe(pending)  # type: ignore[method-assign]
    raw_payload = json.dumps(_interaction_payload(), ensure_ascii=False)
    signature, timestamp = _sign_payload(raw_payload, private_key)

    response = service.handle_interaction_payload(
        raw_payload,
        signature=signature,
        signature_timestamp=timestamp,
    )

    assert response == {'type': 5}
    assert discord_api.edited_messages == [
        ('123456789012345678', 'discord-token-1', 'AI says hello from Discord.')
    ]
    assert discord_api.followup_attachments == [
        ('123456789012345678', 'discord-token-1', 'whitepaper_bundle.md', 'Whitepaper bundle')
    ]
    binding = service.state_store.get_channel('998877665544332211')
    assert binding is not None
    assert binding['last_synced_sequence'] == 2
    assert binding['discord_live_delivered_sequences'] == [2]
    assert binding['discord_live_outbound_message_ids'] == ['discord-out-0001']


def test_discord_service_recovers_reply_after_turn_timeout(tmp_path: Path):
    _, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    discord_api = FakeDiscordApi()
    ait_api = FakeAitApi(
        turn_exception=BotRuntimeError('POST http://127.0.0.1:8088/v1/native/sessions/AITS-DISCORD-0001:turn timed out after 20 seconds.'),
        recovery_events=[
            {
                'sequence': 1,
                'event_type': 'discord.user_message',
                'payload': {
                    'transport_envelope': {
                        'transport': 'discord',
                        'event_id': '998899887777666655',
                    }
                },
            },
            {
                'sequence': 2,
                'event_type': 'assistant.reply',
                'payload': {
                    'text': 'Recovered Discord reply.',
                    'transport_reply_envelope': {
                        'transport': 'discord',
                        'message': {'text': 'Recovered Discord reply.'},
                    },
                },
            },
        ],
    )
    service = DiscordBotService(
        config,
        discord_api=discord_api,
        ait_api=ait_api,
        state_store=DiscordSyncStateStore(state_path),
    )
    service._start_background_reply = lambda pending: service._run_pending_followup_safe(pending)  # type: ignore[method-assign]

    accepted = service.handle_message(_message_payload())

    assert accepted is True
    assert len(ait_api.turn_calls) == 1
    assert discord_api.channel_messages == [('998877665544332211', 'Recovered Discord reply.')]

    binding = service.state_store.get_channel('998877665544332211')
    assert binding is not None
    assert binding['last_synced_sequence'] == 2
    assert binding['discord_live_delivered_sequences'] == [2]
    assert binding['discord_live_outbound_message_ids'] == ['discord-out-0001']


def test_discord_service_waits_for_delayed_reply_after_turn_timeout(tmp_path: Path, monkeypatch):
    _, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    discord_api = FakeDiscordApi()
    ait_api = FakeAitApi(
        turn_exception=BotRuntimeError('POST http://127.0.0.1:8088/v1/native/sessions/AITS-DISCORD-0001:turn timed out after 20 seconds.'),
        recovery_events=[
            {
                'sequence': 1,
                'event_type': 'discord.user_message',
                'payload': {
                    'transport_envelope': {
                        'transport': 'discord',
                        'event_id': '998899887777666655',
                    }
                },
            }
        ],
    )
    service = DiscordBotService(
        config,
        discord_api=discord_api,
        ait_api=ait_api,
        state_store=DiscordSyncStateStore(state_path),
    )
    service._start_background_reply = lambda pending: service._run_pending_followup_safe(pending)  # type: ignore[method-assign]
    monkeypatch.setattr('ait_agent.discord.app.DEFERRED_REPLY_WATCH_MAX_WAIT_SECONDS', 10.0)
    sleep_calls: list[float] = []

    def fake_sleep(seconds: float) -> None:
        sleep_calls.append(seconds)
        if len(sleep_calls) == 4:
            ait_api.recovery_events.append(
                {
                    'sequence': 2,
                    'event_type': 'assistant.reply',
                    'payload': {
                        'text': 'Delayed Discord reply.',
                        'transport_reply_envelope': {
                            'transport': 'discord',
                            'message': {'text': 'Delayed Discord reply.'},
                        },
                    },
                }
            )

    monkeypatch.setattr('ait_agent.discord.app.time.sleep', fake_sleep)

    accepted = service.handle_message(_message_payload())

    assert accepted is True
    assert len(ait_api.turn_calls) == 1
    assert discord_api.channel_messages == [('998877665544332211', 'Delayed Discord reply.')]

    binding = service.state_store.get_channel('998877665544332211')
    assert binding is not None
    assert binding['last_synced_sequence'] == 2
    assert binding['discord_live_delivered_sequences'] == [2]
    assert binding['discord_live_outbound_message_ids'] == ['discord-out-0001']
    assert len(sleep_calls) >= 4


def test_discord_service_delivery_sweep_backfills_observed_replies_and_replays_missing_ones(tmp_path: Path):
    _, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    discord_api = FakeDiscordApi()
    ait_api = FakeAitApi(
        recovery_events=[
            {
                'sequence': 11,
                'event_type': 'session.message',
                'payload': {
                    'transport_envelope': {
                        'transport': 'discord',
                        'event_id': 'm-11',
                        'event_kind': 'message',
                        'channel': {'channel_id': '998877665544332211'},
                    }
                },
            },
            {
                'sequence': 12,
                'event_type': 'assistant.reply',
                'payload': {
                    'reply_to_sequence': 11,
                    'text': 'Visible Discord reply.',
                    'session_surface': 'discord',
                    'transport_reply_envelope': {
                        'transport': 'discord',
                        'target': {'channel_id': '998877665544332211'},
                        'message': {'text': 'Visible Discord reply.'},
                    },
                },
            },
            {
                'sequence': 15,
                'event_type': 'session.message',
                'payload': {
                    'transport_envelope': {
                        'transport': 'discord',
                        'event_id': 'm-15',
                        'event_kind': 'message',
                        'channel': {'channel_id': '998877665544332211'},
                    }
                },
            },
            {
                'sequence': 16,
                'event_type': 'assistant.reply',
                'payload': {
                    'reply_to_sequence': 15,
                    'text': 'Missing Discord reply.',
                    'session_surface': 'discord',
                    'transport_reply_envelope': {
                        'transport': 'discord',
                        'target': {'channel_id': '998877665544332211'},
                        'message': {'text': 'Missing Discord reply.'},
                    },
                },
            },
            {
                'sequence': 17,
                'event_type': 'session.message',
                'payload': {
                    'transport_envelope': {
                        'transport': 'discord',
                        'event_id': 'm-17',
                        'event_kind': 'message',
                        'channel': {'channel_id': '998877665544332211'},
                    }
                },
            },
            {
                'sequence': 18,
                'event_type': 'assistant.reply',
                'payload': {
                    'reply_to_sequence': 17,
                    'text': 'Newest missing Discord reply.',
                    'session_surface': 'discord',
                    'transport_reply_envelope': {
                        'transport': 'discord',
                        'target': {'channel_id': '998877665544332211'},
                        'message': {'text': 'Newest missing Discord reply.'},
                    },
                },
            },
        ],
    )
    state_store = DiscordSyncStateStore(state_path)
    state_store.upsert_channel(
        '998877665544332211',
        session_id='AITS-DISCORD-0001',
        repo_name='ait',
        channel_title='Discord channel · 998877665544332211',
        channel_kind='guild_channel',
        last_synced_sequence=14,
        discord_recent_message_ids=['m-11', 'm-15', 'm-17'],
    )
    discord_api._record_channel_message('998877665544332211', 'Visible Discord reply.')
    service = DiscordBotService(
        config,
        discord_api=discord_api,
        ait_api=ait_api,
        state_store=state_store,
    )

    delivered = service.run_delivery_sweep()

    assert delivered == 3
    assert discord_api.channel_messages == [
        ('998877665544332211', 'Missing Discord reply.'),
        ('998877665544332211', 'Newest missing Discord reply.'),
    ]
    binding = state_store.get_channel('998877665544332211')
    assert binding is not None
    assert binding['last_synced_sequence'] == 18
    assert binding['discord_live_delivered_sequences'] == [12, 16, 18]
    assert binding['discord_live_outbound_message_ids'] == ['discord-out-0002', 'discord-out-0003']
    assert binding['discord_last_live_reply_sequence'] == 18


def test_discord_service_delivery_sweep_replays_attachment_only_reply(tmp_path: Path):
    _, public_key = _keypair()
    state_path = tmp_path / 'discord-sync.json'
    config = _config(state_path, public_key=public_key)
    export_path = tmp_path / 'whitepaper_bundle.md'
    export_path.write_text('# Whitepaper bundle\n', encoding='utf-8')
    discord_api = FakeDiscordApi()
    ait_api = FakeAitApi(
        recovery_events=[
            {
                'sequence': 21,
                'event_type': 'session.message',
                'payload': {
                    'transport_envelope': {
                        'transport': 'discord',
                        'event_id': 'm-21',
                        'event_kind': 'message',
                        'channel': {'channel_id': '998877665544332211'},
                    }
                },
            },
            {
                'sequence': 22,
                'event_type': 'assistant.reply',
                'payload': {
                    'reply_to_sequence': 21,
                    'session_surface': 'discord',
                    'transport_reply_envelope': {
                        'transport': 'discord',
                        'target': {'channel_id': '998877665544332211'},
                        'message': {
                            'attachments': [
                                {
                                    'kind': 'document',
                                    'file_name': export_path.name,
                                    'mime_type': 'text/markdown',
                                    'caption': 'Whitepaper bundle',
                                    'local_path': str(export_path),
                                }
                            ]
                        },
                    },
                },
            },
        ]
    )
    state_store = DiscordSyncStateStore(state_path)
    state_store.upsert_channel(
        '998877665544332211',
        session_id='AITS-DISCORD-0001',
        repo_name='ait',
        channel_title='Discord channel · 998877665544332211',
        channel_kind='guild_channel',
        last_synced_sequence=20,
        discord_recent_message_ids=['m-21'],
    )
    service = DiscordBotService(
        config,
        discord_api=discord_api,
        ait_api=ait_api,
        state_store=state_store,
    )

    delivered = service.run_delivery_sweep()

    assert delivered == 1
    assert discord_api.channel_messages == []
    assert discord_api.channel_attachments == [
        ('998877665544332211', 'whitepaper_bundle.md', 'Whitepaper bundle')
    ]
    binding = state_store.get_channel('998877665544332211')
    assert binding is not None
    assert binding['last_synced_sequence'] == 22
    assert binding['discord_live_delivered_sequences'] == [22]
    assert binding['discord_live_outbound_message_ids'] == ['discord-out-0001']


def test_drop_message_content_intent_keeps_other_gateway_intents():
    intents = DISCORD_GUILD_MESSAGES_INTENT | DISCORD_DIRECT_MESSAGES_INTENT | DISCORD_MESSAGE_CONTENT_INTENT

    assert _drop_message_content_intent(intents) == (
        DISCORD_GUILD_MESSAGES_INTENT | DISCORD_DIRECT_MESSAGES_INTENT
    )


def test_should_drop_message_content_intent_only_for_disallowed_intent_error():
    exc = BotRuntimeError('received 4014 (private use) Disallowed intent(s).')
    intents = DISCORD_GUILD_MESSAGES_INTENT | DISCORD_MESSAGE_CONTENT_INTENT

    assert _should_drop_message_content_intent_for_gateway_error(exc, gateway_intents=intents) is True
    assert _should_drop_message_content_intent_for_gateway_error(
        exc,
        gateway_intents=DISCORD_GUILD_MESSAGES_INTENT,
    ) is False
    assert _should_drop_message_content_intent_for_gateway_error(
        BotRuntimeError('received 4004 authentication failed'),
        gateway_intents=intents,
    ) is False


def test_discord_load_config_uses_repo_workflow_mode_and_default_remote(tmp_path: Path):
    repo_root = tmp_path / 'repo'
    repo_root.mkdir()
    init_repo(repo_root, 'ait', 'main')
    repo_ctx = RepoContext.discover(repo_root)
    add_remote(repo_ctx, 'origin', 'http://127.0.0.1:8899', 'ait', make_default=True)
    repo_cfg = load_repo_config(repo_ctx)
    repo_cfg['workflow_mode'] = 'solo_remote'
    save_repo_config(repo_ctx, repo_cfg)

    env_path = repo_root / '.ait' / 'agent-runtime' / 'discord.env'
    env_path.parent.mkdir(parents=True, exist_ok=True)
    env_path.write_text(
        '\n'.join(
            [
                'AIT_DISCORD_APPLICATION_ID=123456789012345678',
                'AIT_DISCORD_BOT_TOKEN=test-bot-token',
            ]
        )
        + '\n',
        encoding='utf-8',
    )

    config = load_config(repo_root, env={'AIT_DISCORD_ENV_PATH': str(env_path)})
    assert config.runtime_mode == 'remote'
    assert config.runtime_remote_name == 'origin'
    assert config.ait_server_url == 'http://127.0.0.1:8899'
    assert config.repo_name == 'ait'

    repo_cfg = load_repo_config(repo_ctx)
    repo_cfg['workflow_mode'] = 'solo_local'
    save_repo_config(repo_ctx, repo_cfg)
    local_config = load_config(repo_root, env={'AIT_DISCORD_ENV_PATH': str(env_path)})
    assert local_config.runtime_mode == 'local'
    assert local_config.runtime_remote_name is None
    assert local_config.ait_server_url is None


def test_discord_load_config_sets_longer_default_turn_timeout(tmp_path: Path):
    repo_root = tmp_path / 'repo'
    repo_root.mkdir()
    init_repo(repo_root, 'ait', 'main')
    repo_ctx = RepoContext.discover(repo_root)
    add_remote(repo_ctx, 'origin', 'http://127.0.0.1:8899', 'ait', make_default=True)
    repo_cfg = load_repo_config(repo_ctx)
    repo_cfg['workflow_mode'] = 'solo_remote'
    save_repo_config(repo_ctx, repo_cfg)

    env_path = repo_root / '.ait' / 'agent-runtime' / 'discord.env'
    env_path.parent.mkdir(parents=True, exist_ok=True)
    env_path.write_text(
        '\n'.join(
            [
                'AIT_DISCORD_APPLICATION_ID=123456789012345678',
                'AIT_DISCORD_BOT_TOKEN=test-bot-token',
            ]
        )
        + '\n',
        encoding='utf-8',
    )

    config = load_config(repo_root, env={'AIT_DISCORD_ENV_PATH': str(env_path)})

    assert config.request_timeout_seconds == 20.0
    assert config.turn_timeout_seconds == 300.0


def test_discord_load_config_turn_timeout_can_follow_shared_codex_turn_timeout(tmp_path: Path):
    repo_root = tmp_path / 'repo'
    repo_root.mkdir()
    init_repo(repo_root, 'ait', 'main')
    repo_ctx = RepoContext.discover(repo_root)
    add_remote(repo_ctx, 'origin', 'http://127.0.0.1:8899', 'ait', make_default=True)
    repo_cfg = load_repo_config(repo_ctx)
    repo_cfg['workflow_mode'] = 'solo_remote'
    save_repo_config(repo_ctx, repo_cfg)

    env_path = repo_root / '.ait' / 'agent-runtime' / 'discord.env'
    env_path.parent.mkdir(parents=True, exist_ok=True)
    env_path.write_text(
        '\n'.join(
            [
                'AIT_DISCORD_APPLICATION_ID=123456789012345678',
                'AIT_DISCORD_BOT_TOKEN=test-bot-token',
                'AIT_CHAT_CODEX_TURN_TIMEOUT_SECONDS=inf',
            ]
        )
        + '\n',
        encoding='utf-8',
    )

    config = load_config(repo_root, env={'AIT_DISCORD_ENV_PATH': str(env_path)})

    assert config.request_timeout_seconds == 20.0
    assert config.turn_timeout_seconds is None


def test_discord_ait_api_client_uses_turn_timeout_for_turn_posts(tmp_path: Path, monkeypatch):
    _, public_key = _keypair()
    config = _config(tmp_path / 'discord-sync.json', public_key=public_key)
    client = AitApiClient(replace(config, turn_timeout_seconds=180.0))
    captured: dict[str, Any] = {}

    def fake_json_request(url: str, *, method: str = 'GET', payload: dict[str, Any] | None = None, headers: dict[str, str] | None = None, timeout: float | None = None) -> dict[str, Any]:
        captured['url'] = url
        captured['method'] = method
        captured['payload'] = payload
        captured['headers'] = headers
        captured['timeout'] = timeout
        return {'ok': True}

    monkeypatch.setattr('ait_agent.discord.clients._json_request', fake_json_request)

    client.create_discord_turn(
        'AITS-DISCORD-0001',
        text='Hello from Discord chat',
        channel_title='Discord channel · 998877665544332211',
        actor_identity='discord:U-discord-1',
        actor_display_name='WeiTa',
        transport_envelope={'transport': 'discord', 'event_id': '998899887777666655'},
    )

    assert captured['url'].endswith('/v1/native/sessions/AITS-DISCORD-0001:turn')
    assert captured['method'] == 'POST'
    assert captured['timeout'] == 180.0
