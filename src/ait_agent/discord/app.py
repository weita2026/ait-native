from __future__ import annotations

from http.client import RemoteDisconnected
import json
import mimetypes
import os
import socket
import signal
import sys
import threading
import time
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Mapping, Sequence
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

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
from ait_agent.telegram.event_triggers import (
    EventTriggerRegistry,
    load_event_trigger_registry,
    parse_fresh_topic_trigger,
)
from ait_agent.transport_retry import (
    is_loopback_url,
    is_retryable_server_read_error,
    is_retryable_transport_error,
    retry_transport_operation,
    timeout_phrase,
)

from .runtime import (
    DiscordSyncStateStore,
    load_simple_env_file,
    resolve_discord_env_path,
    resolve_discord_sync_state_path,
    utc_now_iso,
)
from .clients import (
    AitApiClient,
    BotRuntimeError,
    DiscordApiClient,
    _clean_optional_str,
    _discord_attachment_caption,
    _discord_attachment_delivery_placeholder,
    _discord_attachment_failure_text,
    _discord_attachment_file_name,
    _discord_attachment_upload,
    _env_value,
    _normalize_base_url,
    _parse_int,
    _parse_timeout_seconds,
    _require_discord_bot_token,
    _split_message_chunks,
)

DEFAULT_DISCORD_API_BASE_URL = 'https://discord.com/api/v10'
DEFAULT_DISCORD_HTTP_USER_AGENT = 'curl/8.7.1'
DISCORD_GUILD_MESSAGES_INTENT = 1 << 9
DISCORD_DIRECT_MESSAGES_INTENT = 1 << 12
DISCORD_MESSAGE_CONTENT_INTENT = 1 << 15
DEFAULT_DISCORD_GATEWAY_INTENTS = (
    DISCORD_GUILD_MESSAGES_INTENT
    | DISCORD_DIRECT_MESSAGES_INTENT
    | DISCORD_MESSAGE_CONTENT_INTENT
)
DEFERRED_REPLY_RECOVERY_ATTEMPTS = 4
DEFERRED_REPLY_RECOVERY_BASE_DELAY_SECONDS = 0.75
DEFERRED_REPLY_WATCH_MAX_WAIT_SECONDS = 90.0
DEFERRED_REPLY_WATCH_POLL_INTERVAL_SECONDS = 5.0
DISCORD_DELIVERY_SWEEP_INTERVAL_SECONDS = 30.0
DISCORD_DELIVERY_SWEEP_LOOKBACK_SEQUENCES = 10
DISCORD_DELIVERY_SWEEP_EVENT_LIMIT = 200
DISCORD_LIVE_DELIVERED_SEQUENCE_LIMIT = 200
DISCORD_LIVE_OUTBOUND_MESSAGE_LIMIT = 200
AIT_DISCORD_TERMINATION_CONTEXT_ENV = 'AIT_DISCORD_TERMINATION_CONTEXT_PATH'
PING_INTERACTION_TYPE = 1
APPLICATION_COMMAND_INTERACTION_TYPE = 2
PONG_RESPONSE_TYPE = 1
CHANNEL_MESSAGE_WITH_SOURCE_TYPE = 4
DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE_TYPE = 5
DISCORD_REPLY_MODE_INTERACTION = 'interaction'
DISCORD_REPLY_MODE_CHANNEL_MESSAGE = 'channel_message'


class InvalidInteractionSignatureError(BotRuntimeError):
    pass


@dataclass(frozen=True)
class BotConfig:
    application_id: str
    public_key: str | None
    bot_token: str | None
    ait_server_url: str | None
    ait_web_url: str | None
    repo_name: str
    request_timeout_seconds: float | None
    turn_timeout_seconds: float | None
    sync_state_path: Path
    discord_api_base_url: str
    discord_http_user_agent: str
    bind_host: str
    bind_port: int
    interaction_path: str
    gateway_intents: int = DEFAULT_DISCORD_GATEWAY_INTENTS
    runtime_mode: str = 'remote'
    runtime_remote_name: str | None = None


@dataclass(frozen=True)
class PendingDiscordReply:
    session_id: str
    channel_id: str
    channel_title: str
    channel_kind: str | None
    application_id: str
    event_id: str
    event_kind: str
    reply_mode: str
    interaction_token: str | None
    actor_identity: str
    actor_display_name: str | None
    text: str
    transport_envelope: dict[str, Any]
    source_user_id: str | None
    guild_id: str | None
    command_name: str | None


@dataclass(frozen=True)
class DiscordReplyOutcome:
    text: str
    through_sequence: int
    assistant_event: dict[str, Any] | None = None


@dataclass(frozen=True)
class DiscordDeliveryReceipt:
    reply_mode: str
    message_ids: tuple[str, ...] = ()


class DiscordBotService:
    def __init__(
        self,
        config: BotConfig,
        *,
        discord_api: DiscordApiClient | None = None,
        ait_api: AitApiClient | None = None,
        state_store: DiscordSyncStateStore | None = None,
        defer_replies: bool = True,
        event_trigger_registry: EventTriggerRegistry | None = None,
    ):
        self.config = config
        self.discord_api = discord_api or DiscordApiClient(config)
        self.ait_api = ait_api or AitApiClient(config)
        self.state_store = state_store or DiscordSyncStateStore(config.sync_state_path)
        self.defer_replies = bool(defer_replies)
        self.event_trigger_registry = event_trigger_registry or load_event_trigger_registry(_runtime_repo_root())

    def handle_interaction_payload(
        self,
        raw_payload: str,
        *,
        signature: str | None = None,
        signature_timestamp: str | None = None,
    ) -> dict[str, Any]:
        verify_interaction_signature(
            raw_payload,
            signature=signature,
            signature_timestamp=signature_timestamp,
            public_key=_require_discord_public_key(self.config),
        )
        payload = parse_interaction_payload(raw_payload)
        return self.handle_interaction(payload)

    def handle_interaction(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        interaction_type = _normalize_positive_int(payload.get('type')) or 0
        if interaction_type == PING_INTERACTION_TYPE:
            return {'type': PONG_RESPONSE_TYPE}
        if interaction_type != APPLICATION_COMMAND_INTERACTION_TYPE:
            return _interaction_message_response('Unsupported Discord interaction type for the current ait Discord slice.')

        data = payload.get('data')
        if not isinstance(data, Mapping):
            raise BotRuntimeError('Discord interaction payload is missing command data.')
        text = _interaction_text(data)
        if not text:
            return _interaction_message_response('Discord command must include text content.')
        user = _discord_actor_user(payload)
        if not isinstance(user, Mapping):
            raise BotRuntimeError('Discord interaction payload is missing a usable user object.')
        interaction_id = _clean_optional_str(payload.get('id'))
        if not interaction_id:
            raise BotRuntimeError('Discord interaction payload is missing an interaction id.')
        interaction_token = _clean_optional_str(payload.get('token'))
        if not interaction_token:
            raise BotRuntimeError('Discord interaction payload is missing an interaction token.')
        channel_id = _clean_optional_str(payload.get('channel_id'))
        if not channel_id:
            raise BotRuntimeError('Discord interaction payload is missing a channel id.')
        application_id = _clean_optional_str(payload.get('application_id')) or self.config.application_id
        guild_id = _clean_optional_str(payload.get('guild_id'))
        channel_kind = _discord_channel_kind(payload)
        channel_title = _discord_channel_title(channel_id, guild_id=guild_id)
        command_name = _clean_optional_str(data.get('name'))
        if self.state_store.has_processed_interaction(channel_id, interaction_id):
            return _interaction_message_response('Duplicate Discord interaction ignored.')
        source_user_id = _clean_optional_str(user.get('id'))
        fresh_topic_trigger = self._match_fresh_topic_event_trigger(text)
        if fresh_topic_trigger is not None:
            link = self._create_fresh_session(
                channel_id,
                channel_kind=channel_kind,
                channel_title=channel_title,
                source_user_id=source_user_id,
                guild_id=guild_id,
                application_id=application_id,
                relink_reason='fresh_topic_event_trigger',
            )
            self.state_store.remember_interaction(
                channel_id,
                interaction_id,
                source_user_id=source_user_id,
                guild_id=guild_id,
                command_name=command_name,
            )
            return _interaction_message_response(self._fresh_topic_confirmation_text(link, fresh_topic_trigger))
        link = self._ensure_session_link(
            channel_id,
            channel_kind=channel_kind,
            channel_title=channel_title,
            source_user_id=source_user_id,
            guild_id=guild_id,
            application_id=application_id,
        )
        actor_identity = _discord_actor_identity(user)
        actor_display_name = _discord_actor_display_name(user)
        self.state_store.remember_interaction(
            channel_id,
            interaction_id,
            source_user_id=source_user_id,
            guild_id=guild_id,
            command_name=command_name,
        )
        transport_envelope = build_transport_event_envelope(
            transport='discord',
            actor_identity=actor_identity,
            actor_transport_id=source_user_id,
            actor_username=_clean_optional_str(user.get('username')),
            actor_display_name=actor_display_name,
            actor_is_bot=bool(user.get('bot')) if 'bot' in user else None,
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            text=text,
            occurred_at=utc_now_iso(),
            event_id=interaction_id,
            dedupe_key=interaction_id,
            metadata={
                'command_name': command_name,
                'guild_id': guild_id,
                'application_id': application_id,
                'command_type': _normalize_positive_int(data.get('type')),
            },
        )
        pending = PendingDiscordReply(
            session_id=str(link.get('session_id') or ''),
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            application_id=application_id,
            event_id=interaction_id,
            event_kind='interaction',
            reply_mode=DISCORD_REPLY_MODE_INTERACTION,
            interaction_token=interaction_token,
            actor_identity=actor_identity,
            actor_display_name=actor_display_name,
            text=text,
            transport_envelope=transport_envelope,
            source_user_id=source_user_id,
            guild_id=guild_id,
            command_name=command_name,
        )
        if self.defer_replies:
            self._start_background_reply(pending)
            return {'type': DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE_TYPE}
        reply_text = self._execute_pending_turn(pending)
        return _interaction_message_response(reply_text)

    def handle_message(self, payload: Mapping[str, Any]) -> bool:
        author = _discord_message_author(payload)
        if not isinstance(author, Mapping):
            raise BotRuntimeError('Discord message payload is missing a usable author object.')
        if bool(author.get('bot')) or _clean_optional_str(payload.get('webhook_id')) is not None:
            return False
        text = _discord_message_text(payload)
        if not text:
            return False
        message_id = _clean_optional_str(payload.get('id'))
        if not message_id:
            raise BotRuntimeError('Discord message payload is missing a message id.')
        channel_id = _clean_optional_str(payload.get('channel_id'))
        if not channel_id:
            raise BotRuntimeError('Discord message payload is missing a channel id.')
        guild_id = _clean_optional_str(payload.get('guild_id'))
        channel_kind = _discord_channel_kind(payload)
        channel_title = _discord_channel_title(channel_id, guild_id=guild_id)
        if self.state_store.has_processed_message(channel_id, message_id):
            return False
        source_user_id = _clean_optional_str(author.get('id'))
        fresh_topic_trigger = self._match_fresh_topic_event_trigger(text)
        if fresh_topic_trigger is not None:
            link = self._create_fresh_session(
                channel_id,
                channel_kind=channel_kind,
                channel_title=channel_title,
                source_user_id=source_user_id,
                guild_id=guild_id,
                application_id=self.config.application_id,
                relink_reason='fresh_topic_event_trigger',
            )
            self.state_store.remember_message(
                channel_id,
                message_id,
                source_user_id=source_user_id,
                guild_id=guild_id,
            )
            self.discord_api.send_channel_message(
                channel_id,
                self._fresh_topic_confirmation_text(link, fresh_topic_trigger),
            )
            return True
        link = self._ensure_session_link(
            channel_id,
            channel_kind=channel_kind,
            channel_title=channel_title,
            source_user_id=source_user_id,
            guild_id=guild_id,
            application_id=self.config.application_id,
        )
        actor_identity = _discord_actor_identity(author)
        actor_display_name = _discord_actor_display_name(author)
        self.state_store.remember_message(
            channel_id,
            message_id,
            source_user_id=source_user_id,
            guild_id=guild_id,
        )
        transport_envelope = build_transport_event_envelope(
            transport='discord',
            actor_identity=actor_identity,
            actor_transport_id=source_user_id,
            actor_username=_clean_optional_str(author.get('username')),
            actor_display_name=actor_display_name,
            actor_is_bot=False,
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            text=text,
            occurred_at=utc_now_iso(),
            event_id=message_id,
            dedupe_key=message_id,
            metadata={
                'guild_id': guild_id,
                'application_id': self.config.application_id,
                'message_type': _normalize_non_negative_int(payload.get('type')),
            },
        )
        pending = PendingDiscordReply(
            session_id=str(link.get('session_id') or ''),
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            application_id=self.config.application_id,
            event_id=message_id,
            event_kind='message',
            reply_mode=DISCORD_REPLY_MODE_CHANNEL_MESSAGE,
            interaction_token=None,
            actor_identity=actor_identity,
            actor_display_name=actor_display_name,
            text=text,
            transport_envelope=transport_envelope,
            source_user_id=source_user_id,
            guild_id=guild_id,
            command_name=None,
        )
        self._start_background_reply(pending)
        return True

    def _start_background_reply(self, pending: PendingDiscordReply) -> None:
        worker = threading.Thread(target=self._run_pending_followup_safe, args=(pending,), daemon=True)
        worker.start()

    def _run_pending_followup_safe(self, pending: PendingDiscordReply) -> None:
        try:
            outcome = self._execute_pending_turn(pending)
        except BotRuntimeError as exc:
            recovered_reply = self._recover_completed_pending_reply(pending, exc)
            if recovered_reply:
                self._deliver_reply_outcome_safe(pending, recovered_reply)
                return
            deferred_reply = self._watch_for_completed_pending_reply(pending, exc)
            if deferred_reply:
                self._deliver_reply_outcome_safe(pending, deferred_reply)
                return
            error_text = f'ait Discord bot error: {exc}'
            print(error_text, file=sys.stderr, flush=True)
            try:
                self._send_reply(pending, error_text)
            except Exception as followup_exc:  # pragma: no cover - defensive operator logging
                print(f'ait Discord follow-up failed: {followup_exc}', file=sys.stderr, flush=True)
            return
        if outcome.text or _assistant_reply_attachments(outcome.assistant_event or {}):
            self._deliver_reply_outcome_safe(pending, outcome)

    def _send_reply(
        self,
        pending: PendingDiscordReply,
        text: str,
        *,
        attachments: Sequence[Mapping[str, Any]] = (),
    ) -> DiscordDeliveryReceipt:
        normalized_text = str(text or '').strip()
        normalized_attachments = [dict(item) for item in attachments if isinstance(item, Mapping)]
        if not normalized_text and not normalized_attachments:
            raise BotRuntimeError('ait-server returned an empty Discord reply.')
        if pending.reply_mode == DISCORD_REPLY_MODE_CHANNEL_MESSAGE:
            message_ids: list[str] = []
            if normalized_text:
                message_ids.extend(self.discord_api.send_channel_message(pending.channel_id, normalized_text))
            for attachment in normalized_attachments:
                try:
                    message_ids.extend(self.discord_api.send_channel_attachment(pending.channel_id, attachment))
                except BotRuntimeError as exc:
                    message_ids.extend(
                        self.discord_api.send_channel_message(
                            pending.channel_id,
                            _discord_attachment_failure_text(attachment, exc),
                        )
                    )
            return DiscordDeliveryReceipt(
                reply_mode=DISCORD_REPLY_MODE_CHANNEL_MESSAGE,
                message_ids=tuple(message_ids),
            )
        if pending.reply_mode == DISCORD_REPLY_MODE_INTERACTION and pending.interaction_token:
            message_ids: list[str] = []
            attachment_failures: list[str] = []
            uploaded_attachment_count = 0
            if normalized_text:
                message_ids.extend(
                    self.discord_api.edit_original_response(
                        pending.application_id,
                        pending.interaction_token,
                        normalized_text,
                    )
                )
            for attachment in normalized_attachments:
                try:
                    message_ids.extend(
                        self.discord_api.send_followup_attachment(
                            pending.application_id,
                            pending.interaction_token,
                            attachment,
                        )
                    )
                    uploaded_attachment_count += 1
                except BotRuntimeError as exc:
                    attachment_failures.append(_discord_attachment_failure_text(attachment, exc))
            if not normalized_text:
                summary_text = ''
                if uploaded_attachment_count > 0:
                    summary_text = _discord_attachment_delivery_placeholder(normalized_attachments)
                elif attachment_failures:
                    summary_text = attachment_failures.pop(0)
                if summary_text:
                    message_ids.extend(
                        self.discord_api.edit_original_response(
                            pending.application_id,
                            pending.interaction_token,
                            summary_text,
                        )
                    )
            for failure_text in attachment_failures:
                message_ids.extend(
                    self.discord_api.send_followup(
                        pending.application_id,
                        pending.interaction_token,
                        failure_text,
                    )
                )
            return DiscordDeliveryReceipt(
                reply_mode=DISCORD_REPLY_MODE_INTERACTION,
                message_ids=tuple(message_ids),
            )
        raise BotRuntimeError(f'Unsupported Discord reply mode: {pending.reply_mode}')

    def _deliver_reply_outcome(self, pending: PendingDiscordReply, outcome: DiscordReplyOutcome) -> None:
        assistant_event = outcome.assistant_event
        attachments = _assistant_reply_attachments(assistant_event or {}) if isinstance(assistant_event, Mapping) else []
        receipt = self._send_reply(pending, outcome.text, attachments=attachments)
        if not isinstance(assistant_event, Mapping):
            return
        live_link = self.state_store.get_channel(pending.channel_id)
        if not live_link or str(live_link.get('session_id') or '').strip() != pending.session_id:
            return
        self._mark_discord_live_reply_delivered(
            pending.channel_id,
            live_link,
            assistant_event=assistant_event,
            through_sequence=outcome.through_sequence,
            receipt=receipt,
        )

    def _deliver_reply_outcome_safe(self, pending: PendingDiscordReply, outcome: DiscordReplyOutcome) -> bool:
        try:
            self._deliver_reply_outcome(pending, outcome)
            return True
        except BotRuntimeError as exc:
            print(f'ait Discord live reply delivery failed: {exc}', file=sys.stderr, flush=True)
            return False
        except Exception as exc:  # pragma: no cover - defensive daemon logging for delivery crashes
            print(f'ait Discord live reply crashed: {exc}', file=sys.stderr, flush=True)
            return False

    def _recover_completed_pending_reply(self, pending: PendingDiscordReply, exc: BotRuntimeError) -> DiscordReplyOutcome | None:
        if not is_retryable_server_read_error(exc):
            return None
        attempts = max(int(DEFERRED_REPLY_RECOVERY_ATTEMPTS), 1)
        for attempt in range(attempts):
            outcome = self._recover_completed_pending_reply_once(pending)
            if outcome:
                return outcome
            if attempt + 1 >= attempts:
                return None
            time.sleep(DEFERRED_REPLY_RECOVERY_BASE_DELAY_SECONDS * float(2**attempt))
        return None

    def _watch_for_completed_pending_reply(self, pending: PendingDiscordReply, exc: BotRuntimeError) -> DiscordReplyOutcome | None:
        if not is_retryable_server_read_error(exc):
            return None
        if DEFERRED_REPLY_WATCH_MAX_WAIT_SECONDS <= 0:
            return None
        deadline = time.monotonic() + float(DEFERRED_REPLY_WATCH_MAX_WAIT_SECONDS)
        while time.monotonic() < deadline:
            current_link = self.state_store.get_channel(pending.channel_id)
            if not current_link or str(current_link.get('session_id') or '').strip() != pending.session_id:
                return None
            remaining_seconds = deadline - time.monotonic()
            if remaining_seconds <= 0:
                break
            time.sleep(min(float(DEFERRED_REPLY_WATCH_POLL_INTERVAL_SECONDS), remaining_seconds))
            outcome = self._recover_completed_pending_reply_once(pending)
            if outcome:
                return outcome
        return None

    def _recover_completed_pending_reply_once(self, pending: PendingDiscordReply) -> DiscordReplyOutcome | None:
        current_link = self.state_store.get_channel(pending.channel_id)
        if not current_link or str(current_link.get('session_id') or '').strip() != pending.session_id:
            return None
        after_sequence = max(int(current_link.get('last_synced_sequence') or 0) - 5, 0)
        try:
            events = self.ait_api.list_session_events(pending.session_id, after_sequence=after_sequence, limit=200)
        except Exception as exc:  # pragma: no cover - defensive recovery path
            print(f'ait Discord recovery read failed: {exc}', file=sys.stderr, flush=True)
            return None
        user_event = next((event for event in events if self._pending_turn_matches_user_event(pending, event)), None)
        if not isinstance(user_event, dict):
            return None
        assistant_event = self._assistant_reply_for_user_event(user_event=user_event, events=events)
        if not isinstance(assistant_event, dict):
            return None
        reply_text = _assistant_reply_text(assistant_event)
        reply_attachments = _assistant_reply_attachments(assistant_event)
        if not reply_text and not reply_attachments:
            return None
        live_link = self.state_store.get_channel(pending.channel_id)
        if not live_link or str(live_link.get('session_id') or '').strip() != pending.session_id:
            return None
        self._remember_completed_event(
            pending,
            last_synced_sequence=int(assistant_event.get('sequence') or user_event.get('sequence') or 0),
        )
        return DiscordReplyOutcome(
            text=reply_text,
            through_sequence=int(assistant_event.get('sequence') or user_event.get('sequence') or 0),
            assistant_event=dict(assistant_event),
        )

    def _discord_live_delivered_sequences(self, link: Mapping[str, Any] | None) -> set[int]:
        values = (link or {}).get('discord_live_delivered_sequences')
        if not isinstance(values, list):
            return set()
        delivered: set[int] = set()
        for value in values:
            try:
                sequence = int(value)
            except (TypeError, ValueError):
                continue
            if sequence > 0:
                delivered.add(sequence)
        return delivered

    def _discord_live_outbound_message_ids(self, link: Mapping[str, Any] | None) -> list[str]:
        values = (link or {}).get('discord_live_outbound_message_ids')
        if not isinstance(values, list):
            return []
        return [str(value).strip() for value in values if str(value).strip()]

    def _mark_discord_live_reply_delivered(
        self,
        channel_id: str,
        link: Mapping[str, Any],
        *,
        assistant_event: Mapping[str, Any],
        through_sequence: int,
        receipt: DiscordDeliveryReceipt,
    ) -> dict[str, Any] | None:
        sequence = int(assistant_event.get('sequence') or 0)
        payload = assistant_event.get('payload') if isinstance(assistant_event, Mapping) else None
        reply_to_sequence = int(payload.get('reply_to_sequence') or 0) if isinstance(payload, Mapping) else 0
        current_link = self.state_store.get_channel(channel_id) or dict(link)
        delivered = self._discord_live_delivered_sequences(current_link)
        if sequence > 0:
            delivered.add(sequence)
        outbound_message_ids = self._discord_live_outbound_message_ids(current_link)
        for message_id in receipt.message_ids:
            normalized = str(message_id).strip()
            if not normalized:
                continue
            outbound_message_ids = [value for value in outbound_message_ids if value != normalized]
            outbound_message_ids.append(normalized)
        bounded_delivered = sorted(delivered)[-DISCORD_LIVE_DELIVERED_SEQUENCE_LIMIT:]
        bounded_message_ids = outbound_message_ids[-DISCORD_LIVE_OUTBOUND_MESSAGE_LIMIT:]
        return self.state_store.patch_channel(
            channel_id,
            last_synced_sequence=max(int(through_sequence), 0),
            last_sync_at=utc_now_iso(),
            discord_live_delivered_sequences=bounded_delivered,
            discord_live_outbound_message_ids=bounded_message_ids,
            discord_last_outbound_message_id=bounded_message_ids[-1] if bounded_message_ids else None,
            discord_last_live_delivery_at=utc_now_iso(),
            discord_last_live_delivery_mode=receipt.reply_mode,
            discord_last_live_reply_sequence=sequence or None,
            discord_last_live_reply_to_sequence=reply_to_sequence or None,
        )

    def _assistant_reply_targets_channel(self, assistant_event: Mapping[str, Any], *, channel_id: str) -> bool:
        payload = assistant_event.get('payload') if isinstance(assistant_event, Mapping) else None
        if not isinstance(payload, Mapping):
            return False
        envelope = payload.get('transport_reply_envelope')
        if isinstance(envelope, Mapping):
            transport = str(envelope.get('transport') or '').strip().lower()
            if transport and transport != 'discord':
                return False
            target = envelope.get('target')
            if isinstance(target, Mapping):
                target_channel_id = _clean_optional_str(target.get('channel_id'))
                if target_channel_id and target_channel_id != channel_id:
                    return False
        session_surface = str(payload.get('session_surface') or '').strip().lower()
        if session_surface and session_surface != 'discord':
            return False
        return bool(_assistant_reply_text(assistant_event) or _assistant_reply_attachments(assistant_event))

    def _message_user_event_matches_channel(self, user_event: Mapping[str, Any], *, channel_id: str) -> bool:
        payload = user_event.get('payload') if isinstance(user_event, Mapping) else None
        if not isinstance(payload, Mapping):
            return False
        transport_envelope = payload.get('transport_envelope')
        if not isinstance(transport_envelope, Mapping):
            return False
        if str(transport_envelope.get('transport') or '').strip().lower() != 'discord':
            return False
        if str(transport_envelope.get('event_kind') or '').strip().lower() != 'message':
            return False
        channel = transport_envelope.get('channel')
        if not isinstance(channel, Mapping):
            return False
        return str(channel.get('channel_id') or '').strip() == channel_id

    def _reply_text_seen_in_channel_history(self, text: str, history: Sequence[Mapping[str, Any]]) -> bool:
        wanted_chunks = [chunk.strip() for chunk in _split_message_chunks(text) if chunk.strip()]
        if not wanted_chunks:
            return False
        seen_chunks = {
            str(message.get('content') or '').strip()
            for message in history
            if isinstance(message, Mapping)
            and isinstance(message.get('author'), Mapping)
            and bool(message.get('author', {}).get('bot'))
            and str(message.get('content') or '').strip()
        }
        return all(chunk in seen_chunks for chunk in wanted_chunks)

    def _reply_attachments_seen_in_channel_history(
        self,
        attachments: Sequence[Mapping[str, Any]],
        history: Sequence[Mapping[str, Any]],
    ) -> bool:
        wanted_names = {name for name in (_discord_attachment_file_name(item) for item in attachments) if name}
        if not wanted_names:
            return False
        seen_names: set[str] = set()
        for message in history:
            if not isinstance(message, Mapping):
                continue
            if not isinstance(message.get('author'), Mapping) or not bool(message.get('author', {}).get('bot')):
                continue
            raw_attachments = message.get('attachments')
            if not isinstance(raw_attachments, Sequence) or isinstance(raw_attachments, (str, bytes, bytearray)):
                continue
            for attachment in raw_attachments:
                if not isinstance(attachment, Mapping):
                    continue
                name = _clean_optional_str(attachment.get('filename'))
                if name:
                    seen_names.add(name)
        return wanted_names.issubset(seen_names)

    def _reply_seen_in_channel_history(
        self,
        text: str,
        attachments: Sequence[Mapping[str, Any]],
        history: Sequence[Mapping[str, Any]],
    ) -> bool:
        saw_text = True if not str(text or '').strip() else self._reply_text_seen_in_channel_history(text, history)
        saw_attachments = True if not attachments else self._reply_attachments_seen_in_channel_history(attachments, history)
        return saw_text and saw_attachments and (bool(str(text or '').strip()) or bool(attachments))

    def _collect_undelivered_reply_outcomes(
        self,
        link: Mapping[str, Any],
        *,
        channel_history: Sequence[Mapping[str, Any]] | None = None,
    ) -> list[tuple[DiscordReplyOutcome, DiscordDeliveryReceipt]]:
        session_id = str(link.get('session_id') or '').strip()
        channel_id = str(link.get('surface_id') or link.get('discord_reply_target', {}).get('channel_id') or '').strip()
        if not session_id or not channel_id:
            return []
        delivered_sequences = self._discord_live_delivered_sequences(link)
        after_sequence = max(int(link.get('last_synced_sequence') or 0) - DISCORD_DELIVERY_SWEEP_LOOKBACK_SEQUENCES, 0)
        events = self.ait_api.list_session_events(session_id, after_sequence=after_sequence, limit=DISCORD_DELIVERY_SWEEP_EVENT_LIMIT)
        events_by_sequence = {
            int(event.get('sequence') or 0): dict(event)
            for event in events
            if int(event.get('sequence') or 0) > 0
        }
        outcomes: list[tuple[DiscordReplyOutcome, DiscordDeliveryReceipt]] = []
        for event in events:
            if str(event.get('event_type') or '').strip() != 'assistant.reply':
                continue
            sequence = int(event.get('sequence') or 0)
            if sequence <= 0 or sequence in delivered_sequences:
                continue
            assistant_event = dict(event)
            if not self._assistant_reply_targets_channel(assistant_event, channel_id=channel_id):
                continue
            payload = assistant_event.get('payload') if isinstance(assistant_event, Mapping) else None
            reply_to_sequence = int(payload.get('reply_to_sequence') or 0) if isinstance(payload, Mapping) else 0
            user_event = events_by_sequence.get(reply_to_sequence)
            if not isinstance(user_event, Mapping) or not self._message_user_event_matches_channel(user_event, channel_id=channel_id):
                continue
            reply_text = _assistant_reply_text(assistant_event)
            reply_attachments = _assistant_reply_attachments(assistant_event)
            if not reply_text and not reply_attachments:
                continue
            through_sequence = max(sequence, reply_to_sequence)
            if channel_history is not None and self._reply_seen_in_channel_history(reply_text, reply_attachments, channel_history):
                outcomes.append(
                    (
                        DiscordReplyOutcome(
                            text=reply_text,
                            through_sequence=through_sequence,
                            assistant_event=assistant_event,
                        ),
                        DiscordDeliveryReceipt(reply_mode='channel_history_observed'),
                    )
                )
                continue
            outcomes.append(
                (
                    DiscordReplyOutcome(
                        text=reply_text,
                        through_sequence=through_sequence,
                        assistant_event=assistant_event,
                    ),
                    DiscordDeliveryReceipt(reply_mode=DISCORD_REPLY_MODE_CHANNEL_MESSAGE),
                )
            )
        return outcomes

    def run_delivery_sweep(self) -> int:
        delivered_count = 0
        for link in self.state_store.list_channels(repo_name=self.config.repo_name):
            channel_id = str(link.get('surface_id') or link.get('discord_reply_target', {}).get('channel_id') or '').strip()
            session_id = str(link.get('session_id') or '').strip()
            if not channel_id or not session_id:
                continue
            try:
                channel_history = self.discord_api.list_channel_messages(channel_id, limit=25)
                pending_outcomes = self._collect_undelivered_reply_outcomes(link, channel_history=channel_history)
            except Exception as exc:
                print(f'ait Discord delivery sweep failed to inspect channel {channel_id}: {exc}', file=sys.stderr, flush=True)
                continue
            for outcome, receipt in pending_outcomes:
                live_link = self.state_store.get_channel(channel_id)
                if not live_link or str(live_link.get('session_id') or '').strip() != session_id:
                    break
                try:
                    if receipt.reply_mode == DISCORD_REPLY_MODE_CHANNEL_MESSAGE:
                        reply_attachments = _assistant_reply_attachments(outcome.assistant_event or {})
                        receipt = DiscordDeliveryReceipt(
                            reply_mode=receipt.reply_mode,
                            message_ids=tuple(
                                self._send_reply(
                                    PendingDiscordReply(
                                        session_id=session_id,
                                        channel_id=channel_id,
                                        channel_title=str(live_link.get('surface_title') or ''),
                                        channel_kind=str(live_link.get('surface_kind') or ''),
                                        application_id=self.config.application_id,
                                        event_id='delivery-sweep',
                                        event_kind='message',
                                        reply_mode=DISCORD_REPLY_MODE_CHANNEL_MESSAGE,
                                        interaction_token=None,
                                        actor_identity='ait-agent-discord',
                                        actor_display_name='ait Discord bot',
                                        text='',
                                        transport_envelope={},
                                        source_user_id=None,
                                        guild_id=None,
                                        command_name=None,
                                    ),
                                    outcome.text,
                                    attachments=reply_attachments,
                                ).message_ids
                            ),
                        )
                    self._mark_discord_live_reply_delivered(
                        channel_id,
                        live_link,
                        assistant_event=outcome.assistant_event or {},
                        through_sequence=outcome.through_sequence,
                        receipt=receipt,
                    )
                    delivered_count += 1
                except Exception as exc:
                    print(f'ait Discord delivery sweep failed to deliver stored reply for channel {channel_id}: {exc}', file=sys.stderr, flush=True)
                    break
        return delivered_count

    def _pending_turn_matches_user_event(self, pending: PendingDiscordReply, event: Mapping[str, Any]) -> bool:
        payload = event.get('payload') if isinstance(event, Mapping) else None
        if not isinstance(payload, Mapping):
            return False
        transport_envelope = payload.get('transport_envelope')
        if isinstance(transport_envelope, Mapping):
            transport = str(transport_envelope.get('transport') or '').strip().lower()
            event_id = str(transport_envelope.get('event_id') or '').strip()
            if transport == 'discord' and event_id == pending.event_id:
                return True
        payload_event_id = str(payload.get('event_id') or '').strip()
        return payload_event_id == pending.event_id

    def _assistant_reply_for_user_event(
        self,
        *,
        user_event: Mapping[str, Any],
        events: Sequence[Mapping[str, Any]],
    ) -> dict[str, Any] | None:
        user_sequence = int(user_event.get('sequence') or 0)
        for event in events:
            if str(event.get('event_type') or '').strip() != 'assistant.reply':
                continue
            event_sequence = int(event.get('sequence') or 0)
            if event_sequence <= user_sequence:
                continue
            if _assistant_reply_text(event) or _assistant_reply_attachments(event):
                return dict(event)
        return None

    def _match_fresh_topic_event_trigger(self, text: str) -> dict[str, Any] | None:
        return parse_fresh_topic_trigger(str(text or '').strip(), self.event_trigger_registry.fresh_topic)

    def _create_transport_session(self, **kwargs: Any) -> dict[str, Any]:
        try:
            return self.ait_api.create_session(**kwargs)
        except TypeError as exc:
            if 'unexpected keyword argument' not in str(exc):
                raise
        fallback_kwargs = {
            'channel_id': kwargs['channel_id'],
            'channel_title': kwargs['channel_title'],
            'channel_kind': kwargs.get('channel_kind'),
            'source_user_id': kwargs.get('source_user_id'),
            'guild_id': kwargs.get('guild_id'),
            'application_id': kwargs['application_id'],
            'session_kind': kwargs.get('session_kind', 'discord_chat'),
            'title_prefix': kwargs.get('title_prefix', 'Discord chat'),
            'metadata_extra': kwargs.get('metadata_extra'),
        }
        return self.ait_api.create_session(**fallback_kwargs)

    def _create_fresh_session(
        self,
        channel_id: str,
        *,
        channel_kind: str | None,
        channel_title: str,
        source_user_id: str | None,
        guild_id: str | None,
        application_id: str,
        relink_reason: str,
    ) -> dict[str, Any]:
        previous_link = self.state_store.get_channel(channel_id)
        previous_session_id = str(previous_link.get('session_id') or '').strip() if previous_link else ''
        session = self._create_transport_session(
            channel_id=channel_id,
            channel_title=channel_title,
            channel_kind=channel_kind,
            source_user_id=source_user_id,
            guild_id=guild_id,
            application_id=application_id,
            binding_role='primary_shared',
            canonical_session_id=None,
            active_session_id=None,
            branch_session_id=None,
            branch_kind=None,
            relink_reason=relink_reason,
        )
        return self.state_store.upsert_channel(
            channel_id,
            session_id=str(session.get('session_id') or ''),
            repo_name=self.config.repo_name,
            channel_title=channel_title,
            channel_kind=channel_kind,
            canonical_session_id=str(session.get('session_id') or ''),
            branch_session_id=None,
            binding_role='primary_shared',
            last_synced_sequence=int(session.get('last_event_sequence') or 0),
            last_sync_at=utc_now_iso(),
            discord_source_user_id=source_user_id,
            discord_guild_id=guild_id,
            discord_application_id=application_id,
            discord_reply_target={
                'channel_id': channel_id,
                'channel_kind': channel_kind,
                'guild_id': guild_id,
                'application_id': application_id,
                **({'source_user_id': source_user_id} if source_user_id else {}),
            },
            previous_session_id=previous_session_id or None,
            branch_kind=None,
            relink_reason=relink_reason,
            relinked_at=utc_now_iso() if previous_session_id else None,
        )

    def _fresh_topic_confirmation_text(
        self,
        link: Mapping[str, Any],
        fresh_topic: Mapping[str, Any],
    ) -> str:
        mode = str(fresh_topic.get('mode') or 'clear').strip().lower()
        topic = str(fresh_topic.get('topic') or '').strip()
        trigger_label = str(fresh_topic.get('display_trigger') or self.event_trigger_registry.fresh_topic.clear.display_trigger)
        lines = [
            'Started a fresh Discord-linked session.',
            f'Trigger: {trigger_label}.',
        ]
        if mode == 'topic' and topic:
            lines.append(f'Topic hint: {topic}')
        session_id = str(link.get('session_id') or '').strip()
        if session_id:
            lines.append(f'Session: {session_id}')
        return '\n'.join(lines)

    def _ensure_session_link(
        self,
        channel_id: str,
        *,
        channel_kind: str | None,
        channel_title: str,
        source_user_id: str | None,
        guild_id: str | None,
        application_id: str,
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
                    canonical_session_id=str(link.get('canonical_session_id') or '').strip() or None,
                    branch_session_id=str(link.get('branch_session_id') or '').strip() or None,
                    binding_role=str(link.get('binding_role') or '').strip() or None,
                    discord_source_user_id=source_user_id,
                    discord_guild_id=guild_id,
                    discord_application_id=application_id,
                    discord_reply_target={
                        'channel_id': channel_id,
                        'channel_kind': channel_kind,
                        'guild_id': guild_id,
                        'application_id': application_id,
                        **({'source_user_id': source_user_id} if source_user_id else {}),
                    },
                )
            except BotRuntimeError:
                canonical_session_id = str(link.get('canonical_session_id') or '').strip()
                branch_session_id = str(link.get('branch_session_id') or '').strip()
                if canonical_session_id and branch_session_id and canonical_session_id != branch_session_id:
                    try:
                        self.ait_api.get_session(canonical_session_id)
                        return self.state_store.upsert_channel(
                            channel_id,
                            session_id=canonical_session_id,
                            repo_name=self.config.repo_name,
                            channel_title=channel_title,
                            channel_kind=channel_kind,
                            canonical_session_id=canonical_session_id,
                            branch_session_id=None,
                            binding_role='primary_shared',
                            discord_source_user_id=source_user_id,
                            discord_guild_id=guild_id,
                            discord_application_id=application_id,
                            discord_reply_target={
                                'channel_id': channel_id,
                                'channel_kind': channel_kind,
                                'guild_id': guild_id,
                                'application_id': application_id,
                                **({'source_user_id': source_user_id} if source_user_id else {}),
                            },
                            previous_session_id=branch_session_id,
                            relink_reason='branch_session_missing',
                            relinked_at=utc_now_iso(),
                        )
                    except BotRuntimeError:
                        pass
        return self._create_fresh_session(
            channel_id,
            channel_kind=channel_kind,
            channel_title=channel_title,
            source_user_id=source_user_id,
            guild_id=guild_id,
            application_id=application_id,
            relink_reason='initial_link',
        )

    def _execute_pending_turn(self, pending: PendingDiscordReply) -> DiscordReplyOutcome:
        turn = self.ait_api.create_discord_turn(
            pending.session_id,
            text=pending.text,
            channel_title=pending.channel_title,
            actor_identity=pending.actor_identity,
            actor_display_name=pending.actor_display_name,
            transport_envelope=pending.transport_envelope,
        )
        user_event = turn.get('user_event') if isinstance(turn, dict) else None
        if not isinstance(user_event, dict):
            raise BotRuntimeError('ait-server returned an invalid Discord turn payload.')
        if turn.get('ok'):
            assistant_event = turn.get('assistant_event') if isinstance(turn.get('assistant_event'), dict) else {}
            reply_text = str(turn.get('reply_text') or '').strip() or _assistant_reply_text(assistant_event)
            self._remember_completed_event(
                pending,
                last_synced_sequence=int(assistant_event.get('sequence') or user_event.get('sequence') or 0),
            )
            return DiscordReplyOutcome(
                text=reply_text,
                through_sequence=int(assistant_event.get('sequence') or user_event.get('sequence') or 0),
                assistant_event=dict(assistant_event) if isinstance(assistant_event, Mapping) else None,
            )
        error_text = str(turn.get('error') or 'Unknown backend reply error.').strip()
        self._remember_completed_event(
            pending,
            last_synced_sequence=int(user_event.get('sequence') or 0),
        )
        return DiscordReplyOutcome(
            text=f'Logged to {pending.session_id} as event #{user_event.get("sequence")}, but the AI reply failed.\n{error_text}',
            through_sequence=int(user_event.get('sequence') or 0),
            assistant_event=None,
        )

    def _remember_completed_event(self, pending: PendingDiscordReply, *, last_synced_sequence: int) -> None:
        if pending.event_kind == 'interaction':
            self.state_store.remember_interaction(
                pending.channel_id,
                pending.event_id,
                source_user_id=pending.source_user_id,
                guild_id=pending.guild_id,
                command_name=pending.command_name,
                last_synced_sequence=last_synced_sequence,
            )
            return
        if pending.event_kind == 'message':
            self.state_store.remember_message(
                pending.channel_id,
                pending.event_id,
                source_user_id=pending.source_user_id,
                guild_id=pending.guild_id,
                last_synced_sequence=last_synced_sequence,
            )
            return
        raise BotRuntimeError(f'Unsupported Discord event kind: {pending.event_kind}')


class _DiscordInteractionServer(ThreadingHTTPServer):
    daemon_threads = True


class DiscordInteractionHandler(BaseHTTPRequestHandler):
    service: DiscordBotService
    interaction_path: str

    def do_POST(self) -> None:  # noqa: N802
        if self.path != self.interaction_path:
            self.send_error(404)
            return
        try:
            content_length = int(self.headers.get('Content-Length') or '0')
        except ValueError:
            content_length = 0
        raw_payload = self.rfile.read(max(content_length, 0)).decode('utf-8')
        signature = self.headers.get('X-Signature-Ed25519')
        signature_timestamp = self.headers.get('X-Signature-Timestamp')
        try:
            response_payload = self.service.handle_interaction_payload(
                raw_payload,
                signature=signature,
                signature_timestamp=signature_timestamp,
            )
        except InvalidInteractionSignatureError as exc:
            self._write_json(401, {'ok': False, 'error': str(exc)})
            return
        except BotRuntimeError as exc:
            self._write_json(400, {'ok': False, 'error': str(exc)})
            return
        except Exception as exc:  # pragma: no cover - defensive crash logging for operator runtime
            print(f'ait Discord interaction server crashed: {exc}', file=sys.stderr, flush=True)
            self._write_json(500, {'ok': False, 'error': 'internal interaction error'})
            return
        self._write_json(200, response_payload)

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A003
        print(f'Discord interaction {self.address_string()} - {format % args}', flush=True)

    def _write_json(self, status_code: int, payload: dict[str, Any]) -> None:
        encoded = json.dumps(payload, ensure_ascii=False).encode('utf-8')
        self.send_response(status_code)
        self.send_header('Content-Type', 'application/json; charset=utf-8')
        self.send_header('Content-Length', str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def verify_interaction_signature(
    raw_payload: str,
    *,
    signature: str | None,
    signature_timestamp: str | None,
    public_key: str,
) -> None:
    normalized_signature = str(signature or '').strip()
    if not normalized_signature:
        raise InvalidInteractionSignatureError('Missing Discord interaction signature header.')
    normalized_timestamp = str(signature_timestamp or '').strip()
    if not normalized_timestamp:
        raise InvalidInteractionSignatureError('Missing Discord interaction timestamp header.')
    try:
        signature_bytes = bytes.fromhex(normalized_signature)
    except ValueError as exc:
        raise InvalidInteractionSignatureError('Invalid Discord interaction signature encoding.') from exc
    try:
        public_key_bytes = bytes.fromhex(str(public_key or '').strip())
    except ValueError as exc:
        raise BotRuntimeError('Invalid Discord public key encoding.') from exc
    try:
        verifier = Ed25519PublicKey.from_public_bytes(public_key_bytes)
        verifier.verify(signature_bytes, f'{normalized_timestamp}{raw_payload}'.encode('utf-8'))
    except (ValueError, InvalidSignature) as exc:
        raise InvalidInteractionSignatureError('Invalid Discord interaction signature.') from exc


def _require_discord_public_key(config: BotConfig) -> str:
    public_key = _clean_optional_str(config.public_key)
    if public_key is None:
        raise BotRuntimeError('Missing Discord public key for interaction payload verification.')
    return public_key


def _require_discord_bot_token(config: BotConfig) -> str:
    token = _clean_optional_str(config.bot_token)
    if token is None:
        raise BotRuntimeError('Missing Discord bot token. Set AIT_DISCORD_BOT_TOKEN or DISCORD_BOT_TOKEN.')
    return token


def parse_interaction_payload(raw_payload: str) -> dict[str, Any]:
    if not raw_payload.strip():
        raise BotRuntimeError('No Discord interaction payload provided.')
    try:
        payload = json.loads(raw_payload)
    except json.JSONDecodeError as exc:
        raise BotRuntimeError('Discord interaction payload must be valid JSON.') from exc
    if not isinstance(payload, dict):
        raise BotRuntimeError('Discord interaction payload must be a JSON object.')
    if _normalize_positive_int(payload.get('type')) is None:
        raise BotRuntimeError('Discord interaction payload must include a numeric type.')
    return payload


def run_interaction_payload(
    raw_payload: str,
    *,
    signature: str | None = None,
    signature_timestamp: str | None = None,
    service: DiscordBotService | None = None,
    config: BotConfig | None = None,
    repo_root: Path | None = None,
) -> dict[str, Any]:
    bot_service = service or DiscordBotService(config or load_config(repo_root or Path(os.environ.get('AIT_REPO_ROOT') or Path.cwd())))
    return bot_service.handle_interaction_payload(
        raw_payload,
        signature=signature,
        signature_timestamp=signature_timestamp,
    )


def _normalize_positive_int(value: object) -> int | None:
    try:
        parsed = int(str(value or '').strip())
    except (TypeError, ValueError):
        return None
    return parsed if parsed > 0 else None


def _normalize_non_negative_int(value: object) -> int | None:
    try:
        parsed = int(str(value or '').strip())
    except (TypeError, ValueError):
        return None
    return parsed if parsed >= 0 else None


def _discord_actor_user(payload: Mapping[str, Any]) -> Mapping[str, Any] | None:
    member = payload.get('member')
    if isinstance(member, Mapping):
        user = member.get('user')
        if isinstance(user, Mapping):
            return user
    user = payload.get('user')
    if isinstance(user, Mapping):
        return user
    return None


def _discord_message_author(payload: Mapping[str, Any]) -> Mapping[str, Any] | None:
    author = payload.get('author')
    if isinstance(author, Mapping):
        return author
    return None


def _discord_actor_identity(user: Mapping[str, Any]) -> str:
    return f"discord:{_clean_optional_str(user.get('id')) or 'unknown'}"


def _discord_actor_display_name(user: Mapping[str, Any]) -> str | None:
    return (
        _clean_optional_str(user.get('global_name'))
        or _clean_optional_str(user.get('username'))
        or _clean_optional_str(user.get('id'))
    )


def _discord_channel_kind(payload: Mapping[str, Any]) -> str:
    return 'guild_channel' if _clean_optional_str(payload.get('guild_id')) else 'dm'


def _discord_channel_title(channel_id: str, *, guild_id: str | None) -> str:
    if guild_id:
        return f'Discord channel · {channel_id}'
    return f'Discord DM · {channel_id}'


def _discord_message_text(payload: Mapping[str, Any]) -> str:
    return _clean_optional_str(payload.get('content')) or ''


def _flatten_string_options(options: Sequence[object] | None) -> list[tuple[str | None, str]]:
    values: list[tuple[str | None, str]] = []
    for option in options or []:
        if not isinstance(option, Mapping):
            continue
        name = _clean_optional_str(option.get('name'))
        raw_value = option.get('value')
        if isinstance(raw_value, str) and raw_value.strip():
            values.append((name, raw_value.strip()))
        nested = option.get('options')
        if isinstance(nested, Sequence) and not isinstance(nested, (str, bytes, bytearray)):
            values.extend(_flatten_string_options(nested))
    return values


def _interaction_text(data: Mapping[str, Any]) -> str:
    direct = _clean_optional_str(data.get('text'))
    if direct:
        return direct
    options = data.get('options')
    if isinstance(options, Sequence) and not isinstance(options, (str, bytes, bytearray)):
        values = _flatten_string_options(options)
        for name, value in values:
            if name == 'text':
                return value
        if values:
            return ' '.join(value for _, value in values)
    return ''


def _interaction_message_response(text: str) -> dict[str, Any]:
    return {
        'type': CHANNEL_MESSAGE_WITH_SOURCE_TYPE,
        'data': {
            'content': str(text or '').strip() or '(empty)',
            'allowed_mentions': {'parse': []},
        },
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


def _assistant_reply_attachments(assistant_event: Mapping[str, Any]) -> list[dict[str, Any]]:
    payload = assistant_event.get('payload') if isinstance(assistant_event, Mapping) else None
    if not isinstance(payload, Mapping):
        return []
    envelope = payload.get('transport_reply_envelope')
    if not isinstance(envelope, Mapping):
        return []
    message = envelope.get('message')
    if not isinstance(message, Mapping):
        return []
    attachments = message.get('attachments')
    if not isinstance(attachments, Sequence) or isinstance(attachments, (str, bytes, bytearray)):
        return []
    return [dict(item) for item in attachments if isinstance(item, Mapping)]


def _runtime_repo_root() -> Path:
    env_root = str(os.environ.get('AIT_REPO_ROOT') or '').strip()
    if env_root:
        return Path(env_root)
    return Path.cwd()


def _termination_context_path(env: Mapping[str, str] | None = None) -> Path | None:
    source_env = os.environ if env is None else env
    raw = str(source_env.get(AIT_DISCORD_TERMINATION_CONTEXT_ENV) or '').strip()
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
    env_path = resolve_discord_env_path(resolved_root, source_env.get('AIT_DISCORD_ENV_PATH'))
    values = load_simple_env_file(env_path)
    try:
        runtime_target = resolve_agent_runtime_target(resolved_root)
    except AgentRuntimeConfigError as exc:
        raise BotRuntimeError(str(exc)) from exc
    env_value = lambda *names, default=None: _env_value(values, *names, default=default, env=source_env)
    application_id = env_value('AIT_DISCORD_APPLICATION_ID', 'DISCORD_APPLICATION_ID')
    if not application_id:
        raise BotRuntimeError(
            f'Missing Discord application id. Set AIT_DISCORD_APPLICATION_ID or DISCORD_APPLICATION_ID in {env_path}.'
        )
    public_key = env_value('AIT_DISCORD_PUBLIC_KEY', 'DISCORD_PUBLIC_KEY')
    if public_key:
        try:
            public_key_bytes = bytes.fromhex(public_key)
        except ValueError as exc:
            raise BotRuntimeError('Discord public key must be hex encoded.') from exc
        if len(public_key_bytes) != 32:
            raise BotRuntimeError('Discord public key must decode to 32 bytes.')
    bot_token = env_value('AIT_DISCORD_BOT_TOKEN', 'DISCORD_BOT_TOKEN')
    ait_server_url = runtime_target.server_url
    ait_web_url_raw = env_value('AIT_DISCORD_WEB_URL', 'AIT_WEB_URL')
    ait_web_url = _normalize_base_url(ait_web_url_raw, ait_server_url) if ait_web_url_raw else None
    request_timeout_seconds = _parse_timeout_seconds(
        env_value('AIT_DISCORD_REQUEST_TIMEOUT_SECONDS', 'AIT_DISCORD_TIMEOUT_SECONDS'),
        20.0,
        5.0,
    )
    turn_timeout_seconds = _parse_timeout_seconds(
        env_value(
            'AIT_DISCORD_TURN_TIMEOUT_SECONDS',
            'AIT_DISCORD_CODEX_TURN_TIMEOUT_SECONDS',
            'AIT_CHAT_CODEX_TURN_TIMEOUT_SECONDS',
        ),
        None if request_timeout_seconds is None else max(request_timeout_seconds, 300.0),
        5.0,
    )
    discord_http_user_agent = str(
        env_value('AIT_DISCORD_HTTP_USER_AGENT', 'DISCORD_HTTP_USER_AGENT', default=DEFAULT_DISCORD_HTTP_USER_AGENT)
        or DEFAULT_DISCORD_HTTP_USER_AGENT
    ).strip() or DEFAULT_DISCORD_HTTP_USER_AGENT
    sync_state_path = resolve_discord_sync_state_path(env_value('AIT_DISCORD_STATE_PATH'))
    discord_api_base_url = _normalize_base_url(
        env_value('AIT_DISCORD_API_BASE_URL', default=DEFAULT_DISCORD_API_BASE_URL),
        DEFAULT_DISCORD_API_BASE_URL,
    )
    bind_host = (env_value('AIT_DISCORD_BIND_HOST', default='127.0.0.1') or '127.0.0.1').strip() or '127.0.0.1'
    bind_port = _parse_int(env_value('AIT_DISCORD_BIND_PORT', default='8092'), 8092, 1)
    interaction_path = (env_value('AIT_DISCORD_INTERACTION_PATH', default='/interactions') or '/interactions').strip() or '/interactions'
    if not interaction_path.startswith('/'):
        interaction_path = f'/{interaction_path}'
    gateway_intents = _parse_int(
        env_value('AIT_DISCORD_GATEWAY_INTENTS', default=str(DEFAULT_DISCORD_GATEWAY_INTENTS)),
        DEFAULT_DISCORD_GATEWAY_INTENTS,
        0,
    )
    return BotConfig(
        application_id=application_id,
        public_key=public_key,
        bot_token=bot_token,
        ait_server_url=ait_server_url,
        runtime_mode=runtime_target.mode,
        runtime_remote_name=runtime_target.remote_name,
        ait_web_url=ait_web_url,
        repo_name=runtime_target.repo_name,
        request_timeout_seconds=request_timeout_seconds,
        turn_timeout_seconds=turn_timeout_seconds,
        sync_state_path=sync_state_path,
        discord_api_base_url=discord_api_base_url,
        discord_http_user_agent=discord_http_user_agent,
        bind_host=bind_host,
        bind_port=bind_port,
        interaction_path=interaction_path,
        gateway_intents=gateway_intents,
    )


def _install_signal_handlers(server: _DiscordInteractionServer) -> None:
    def _handler(signum, _frame):
        print(f'Received signal {signum}; stopping ait Discord bot{_signal_stop_suffix(signum)}.', flush=True)
        server.shutdown()

    signal.signal(signal.SIGTERM, _handler)
    signal.signal(signal.SIGINT, _handler)


def _handler_factory(service: DiscordBotService, interaction_path: str):
    class _Handler(DiscordInteractionHandler):
        pass

    _Handler.service = service
    _Handler.interaction_path = interaction_path
    return _Handler


def interaction_main() -> None:
    repo_root = Path(os.environ.get('AIT_REPO_ROOT') or Path.cwd())
    raw_payload = sys.stdin.read()
    signature = os.environ.get('AIT_DISCORD_SIGNATURE') or os.environ.get('AIT_DISCORD_INTERACTION_SIGNATURE')
    signature_timestamp = os.environ.get('AIT_DISCORD_SIGNATURE_TIMESTAMP') or os.environ.get('AIT_DISCORD_INTERACTION_TIMESTAMP')
    try:
        response_payload = run_interaction_payload(
            raw_payload,
            signature=signature,
            signature_timestamp=signature_timestamp,
            repo_root=repo_root,
        )
    except BotRuntimeError as exc:
        print(f'ait Discord interaction failed: {exc}', file=sys.stderr, flush=True)
        raise SystemExit(1) from exc
    except Exception as exc:  # pragma: no cover - defensive crash logging for webhook handler
        print(f'ait Discord interaction crashed: {exc}', file=sys.stderr, flush=True)
        raise
    print(json.dumps(response_payload, ensure_ascii=False), flush=True)


def main() -> None:
    try:
        from websockets.sync.client import connect
    except ImportError as exc:  # pragma: no cover - dependency error is environment-specific
        raise SystemExit("Missing Python dependency 'websockets'. Install project dependencies before using the Discord gateway worker.") from exc

    repo_root = Path(os.environ.get('AIT_REPO_ROOT') or Path.cwd())
    config = load_config(repo_root)
    config.sync_state_path.parent.mkdir(parents=True, exist_ok=True)
    service = DiscordBotService(config)
    discord_api = service.discord_api
    stop_event = threading.Event()
    _install_stop_signal_handlers(stop_event, transport_label='Discord gateway bot')
    print(
        f'ait Discord bot starting · repo={config.repo_name} · backend={config.runtime_mode}'
        f"{f' · remote={config.runtime_remote_name} · server={config.ait_server_url}' if config.ait_server_url else ''}"
        f' · api={config.discord_api_base_url} · state={config.sync_state_path}',
        flush=True,
    )
    session_id: str | None = None
    resume_gateway_url: str | None = None
    sequence: int | None = None
    current_gateway_intents = max(config.gateway_intents, 0)
    reconnect_delay_seconds = 2.0
    next_delivery_sweep_at = 0.0
    while not stop_event.is_set():
        if time.monotonic() >= next_delivery_sweep_at:
            try:
                service.run_delivery_sweep()
            except Exception as exc:  # pragma: no cover - defensive daemon logging for sweep failures
                print(f'ait Discord delivery sweep crashed: {exc}', file=sys.stderr, flush=True)
            next_delivery_sweep_at = time.monotonic() + DISCORD_DELIVERY_SWEEP_INTERVAL_SECONDS
        try:
            gateway_base_url = _resolve_gateway_base_url(
                discord_api,
                session_id=session_id,
                resume_gateway_url=resume_gateway_url,
            )
            gateway_url = _discord_gateway_socket_url(gateway_base_url)
            print(f'ait Discord bot connected · gateway={gateway_url}', flush=True)
            with connect(
                gateway_url,
                open_timeout=config.request_timeout_seconds,
                close_timeout=1,
                ping_interval=None,
                ping_timeout=None,
                max_size=None,
            ) as websocket:
                hello_payload = _recv_gateway_payload(websocket, timeout=config.request_timeout_seconds)
                hello_op = _normalize_positive_int(hello_payload.get('op')) or 0
                if hello_op != 10:
                    raise BotRuntimeError(f'Discord gateway did not start with Hello (received op={hello_op}).')
                hello_data = hello_payload.get('d')
                if not isinstance(hello_data, Mapping):
                    raise BotRuntimeError('Discord Hello payload is missing heartbeat data.')
                heartbeat_interval_ms = _normalize_positive_int(hello_data.get('heartbeat_interval'))
                if heartbeat_interval_ms is None:
                    raise BotRuntimeError('Discord Hello payload did not include a heartbeat interval.')
                heartbeat_interval_seconds = heartbeat_interval_ms / 1000.0
                heartbeat_acknowledged = True
                next_heartbeat_at = time.monotonic() + heartbeat_interval_seconds
                if session_id and sequence is not None:
                    websocket.send(
                        json.dumps(
                            {
                                'op': 6,
                                'd': {
                                    'token': _require_discord_bot_token(config),
                                    'session_id': session_id,
                                    'seq': sequence,
                                },
                            },
                            ensure_ascii=False,
                        )
                    )
                else:
                    websocket.send(
                        json.dumps(
                            {
                                'op': 2,
                                'd': {
                                    'token': _require_discord_bot_token(config),
                                    'intents': current_gateway_intents,
                                    'properties': {
                                        'os': sys.platform,
                                        'browser': 'ait-agent',
                                        'device': 'ait-agent',
                                    },
                                },
                            },
                            ensure_ascii=False,
                        )
                    )
                while not stop_event.is_set():
                    timeout = max(min(next_heartbeat_at - time.monotonic(), 1.0), 0.0)
                    try:
                        payload = _recv_gateway_payload(websocket, timeout=timeout)
                    except TimeoutError:
                        payload = None
                    if payload is None:
                        if time.monotonic() >= next_heartbeat_at:
                            if not heartbeat_acknowledged:
                                raise BotRuntimeError('Discord gateway heartbeat ACK timed out.')
                            websocket.send(json.dumps({'op': 1, 'd': sequence}, ensure_ascii=False))
                            heartbeat_acknowledged = False
                            next_heartbeat_at = time.monotonic() + heartbeat_interval_seconds
                        continue
                    if 's' in payload:
                        next_sequence = _normalize_positive_int(payload.get('s'))
                        if next_sequence is not None:
                            sequence = next_sequence
                    op = _normalize_positive_int(payload.get('op')) or 0
                    if op == 0:
                        event_name = _clean_optional_str(payload.get('t'))
                        data = payload.get('d')
                        if event_name == 'READY' and isinstance(data, Mapping):
                            session_id = _clean_optional_str(data.get('session_id'))
                            resume_gateway_url = _clean_optional_str(data.get('resume_gateway_url')) or resume_gateway_url
                        elif event_name == 'INTERACTION_CREATE' and isinstance(data, Mapping):
                            try:
                                callback_payload = service.handle_interaction(data)
                                interaction_id = _clean_optional_str(data.get('id'))
                                interaction_token = _clean_optional_str(data.get('token'))
                                if interaction_id is None or interaction_token is None:
                                    raise BotRuntimeError('Discord gateway interaction is missing id/token.')
                                discord_api.create_initial_response(interaction_id, interaction_token, callback_payload)
                            except BotRuntimeError as exc:
                                print(f'ait Discord interaction failed: {exc}', file=sys.stderr, flush=True)
                        elif event_name == 'MESSAGE_CREATE' and isinstance(data, Mapping):
                            try:
                                service.handle_message(data)
                            except BotRuntimeError as exc:
                                print(f'ait Discord message failed: {exc}', file=sys.stderr, flush=True)
                        continue
                    if op == 1:
                        websocket.send(json.dumps({'op': 1, 'd': sequence}, ensure_ascii=False))
                        heartbeat_acknowledged = False
                        next_heartbeat_at = time.monotonic() + heartbeat_interval_seconds
                        continue
                    if op == 7:
                        break
                    if op == 9:
                        if not bool(payload.get('d')):
                            session_id = None
                            resume_gateway_url = None
                            sequence = None
                        break
                    if op == 11:
                        heartbeat_acknowledged = True
                        continue
        except BotRuntimeError as exc:
            if _should_drop_message_content_intent_for_gateway_error(exc, gateway_intents=current_gateway_intents):
                current_gateway_intents = _drop_message_content_intent(current_gateway_intents)
                session_id = None
                resume_gateway_url = None
                sequence = None
                print(
                    'ait Discord bot fallback: Discord rejected the privileged Message Content intent; '
                    'continuing without that intent so slash commands stay available. '
                    'Enable Message Content Intent in the Discord Developer Portal to restore guild plain-chat ingestion.',
                    file=sys.stderr,
                    flush=True,
                )
                continue
            print(f'ait Discord bot failed: {exc}', file=sys.stderr, flush=True)
        except Exception as exc:  # pragma: no cover - defensive crash logging for daemon mode
            print(f'ait Discord bot crashed: {exc}', file=sys.stderr, flush=True)
        if not stop_event.is_set():
            time.sleep(reconnect_delay_seconds)


def _discord_gateway_socket_url(base_url: str) -> str:
    normalized = base_url.rstrip('/')
    separator = '&' if '?' in normalized else '?'
    return f'{normalized}{separator}v=10&encoding=json'


def _resolve_gateway_base_url(
    discord_api: DiscordApiClient,
    *,
    session_id: str | None,
    resume_gateway_url: str | None,
) -> str:
    resumed_gateway_url = _clean_optional_str(resume_gateway_url)
    if session_id and resumed_gateway_url:
        return resumed_gateway_url
    gateway_info = discord_api.gateway_info()
    gateway_base_url = _clean_optional_str(gateway_info.get('url'))
    if gateway_base_url is None:
        raise BotRuntimeError('Discord gateway info did not include a gateway URL.')
    return gateway_base_url


def _drop_message_content_intent(gateway_intents: int) -> int:
    return max(int(gateway_intents), 0) & ~DISCORD_MESSAGE_CONTENT_INTENT


def _should_drop_message_content_intent_for_gateway_error(exc: BaseException, *, gateway_intents: int) -> bool:
    if (gateway_intents & DISCORD_MESSAGE_CONTENT_INTENT) == 0:
        return False
    text = str(exc).strip().lower()
    return '4014' in text and 'disallowed intent' in text


def _recv_gateway_payload(websocket: Any, *, timeout: float | None) -> dict[str, Any]:
    raw_payload = websocket.recv(timeout=timeout)
    if not isinstance(raw_payload, str):
        raw_payload = str(raw_payload)
    payload = json.loads(raw_payload)
    if not isinstance(payload, dict):
        raise BotRuntimeError('Discord gateway payload must be a JSON object.')
    return payload


def _install_stop_signal_handlers(stop_event: threading.Event, *, transport_label: str) -> None:
    def _handler(signum, _frame):
        print(f'Received signal {signum}; stopping {transport_label}{_signal_stop_suffix(signum)}.', flush=True)
        stop_event.set()

    signal.signal(signal.SIGTERM, _handler)
    signal.signal(signal.SIGINT, _handler)


if __name__ == '__main__':  # pragma: no cover
    main()
