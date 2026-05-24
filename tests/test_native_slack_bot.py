from __future__ import annotations

import time
from pathlib import Path
from typing import Any
from urllib.parse import urlencode

import pytest

from ait_agent.slack.app import (
    BotConfig,
    InvalidSlackSignatureError,
    SlackBotService,
    build_slack_signature,
    parse_command_payload,
    run_command_payload,
)
from ait_agent.slack.runtime import SlackSyncStateStore


class FakeSlackApi:
    def __init__(self):
        self.responses: list[tuple[str, str, str | None]] = []

    def send_response(self, response_url: str, text: str, *, response_type: str | None = None) -> None:
        self.responses.append((response_url, text, response_type))


class FakeAitApi:
    def __init__(self, reply_text: str = "AI says hello from Slack."):
        self.reply_text = reply_text
        self._next_session = 1
        self.turn_calls: list[dict[str, Any]] = []
        self.sessions: dict[str, dict[str, Any]] = {}

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
        session_kind: str = "slack_chat",
        title_prefix: str = "Slack chat",
    ) -> dict[str, Any]:
        session_id = f"AITS-SLACK-{self._next_session:04d}"
        self._next_session += 1
        payload = {
            "session_id": session_id,
            "title": f"{title_prefix} · {channel_title}",
            "metadata": {
                "slack_channel_id": channel_id,
                "slack_channel_kind": channel_kind,
                "slack_source_user_id": source_user_id,
                "slack_team_id": team_id,
                "slack_response_url": response_url,
                "slack_thread_id": thread_id,
            },
        }
        self.sessions[session_id] = payload
        return payload

    def create_slack_turn(self, session_id: str, **kwargs: Any) -> dict[str, Any]:
        self.turn_calls.append({"session_id": session_id, **kwargs})
        return {
            "ok": True,
            "session_id": session_id,
            "user_event": {"sequence": 1, "payload": {"transport_envelope": kwargs["transport_envelope"]}},
            "assistant_event": {
                "sequence": 2,
                "payload": {
                    "text": self.reply_text,
                    "transport_reply_envelope": {
                        "transport": "slack",
                        "message": {"text": self.reply_text},
                    },
                },
            },
            "reply_text": self.reply_text,
        }


def _config(state_path: Path) -> BotConfig:
    return BotConfig(
        signing_secret="slack-signing-secret",
        app_token=None,
        ait_server_url="http://127.0.0.1:8088",
        ait_web_url=None,
        repo_name="ait",
        request_timeout_seconds=20.0,
        sync_state_path=state_path,
        bind_host="127.0.0.1",
        bind_port=8093,
        command_path="/command",
        ack_text="ait is thinking...",
        response_type="in_channel",
        socket_open_url="https://slack.com/api/apps.connections.open",
    )


def _command_payload(
    *,
    trigger_id: str = "1337.2468",
    channel_id: str = "C-ops-1",
    channel_name: str = "ops",
    user_id: str = "U-slack-1",
    user_name: str = "weita",
    text: str = "Hello from Slack",
    response_url: str = "https://hooks.slack.com/commands/T000/B000/abc123",
) -> str:
    return urlencode(
        {
            "token": "legacy-token",
            "team_id": "T-team-1",
            "team_domain": "ait",
            "channel_id": channel_id,
            "channel_name": channel_name,
            "user_id": user_id,
            "user_name": user_name,
            "command": "/ait",
            "text": text,
            "response_url": response_url,
            "trigger_id": trigger_id,
        }
    )


def test_parse_command_payload_requires_form_encoded_body():
    with pytest.raises(Exception):
        parse_command_payload("")


def test_slack_service_handles_text_command_end_to_end_and_dedupes(tmp_path: Path):
    state_path = tmp_path / "slack-sync.json"
    config = _config(state_path)
    slack_api = FakeSlackApi()
    ait_api = FakeAitApi()
    service = SlackBotService(
        config,
        slack_api=slack_api,
        ait_api=ait_api,
        state_store=SlackSyncStateStore(state_path),
    )
    service._start_background_reply = lambda pending: service._run_pending_reply_safe(pending)  # type: ignore[method-assign]
    raw_payload = _command_payload()
    timestamp = str(int(time.time()))
    signature = build_slack_signature(raw_payload, config.signing_secret, timestamp=timestamp)

    response = run_command_payload(
        raw_payload,
        signature=signature,
        signature_timestamp=timestamp,
        service=service,
    )

    assert response == {"response_type": "ephemeral", "text": "ait is thinking..."}
    assert len(ait_api.turn_calls) == 1
    assert ait_api.turn_calls[0]["transport_envelope"]["transport"] == "slack"
    assert ait_api.turn_calls[0]["transport_envelope"]["event_id"] == "1337.2468"
    assert ait_api.turn_calls[0]["transport_envelope"]["channel"]["channel_id"] == "C-ops-1"
    assert slack_api.responses == [
        ("https://hooks.slack.com/commands/T000/B000/abc123", "AI says hello from Slack.", "in_channel")
    ]

    binding = service.state_store.get_channel("C-ops-1")
    assert binding is not None
    assert binding["session_id"] == "AITS-SLACK-0001"
    assert "1337.2468" in binding["slack_recent_request_ids"]

    duplicate = run_command_payload(
        raw_payload,
        signature=signature,
        signature_timestamp=timestamp,
        service=service,
    )

    assert duplicate == {"response_type": "ephemeral", "text": "Duplicate Slack command ignored."}
    assert len(ait_api.turn_calls) == 1
    assert len(slack_api.responses) == 1


def test_slack_service_handles_socket_mode_envelope_end_to_end_and_dedupes(tmp_path: Path):
    state_path = tmp_path / "slack-sync.json"
    config = _config(state_path)
    slack_api = FakeSlackApi()
    ait_api = FakeAitApi()
    service = SlackBotService(
        config,
        slack_api=slack_api,
        ait_api=ait_api,
        state_store=SlackSyncStateStore(state_path),
    )
    service._start_background_reply = lambda pending: service._run_pending_reply_safe(pending)  # type: ignore[method-assign]
    envelope = {
        "envelope_id": "env-1",
        "type": "slash_commands",
        "accepts_response_payload": True,
        "payload": {
            "team_id": "T-team-1",
            "team_domain": "ait",
            "channel_id": "C-ops-1",
            "channel_name": "ops",
            "user_id": "U-slack-1",
            "user_name": "weita",
            "command": "/ait",
            "text": "Hello from Slack",
            "response_url": "https://hooks.slack.com/commands/T000/B000/abc123",
            "trigger_id": "1337.2468",
        },
    }

    response = service.handle_socket_envelope(envelope)

    assert response == {
        "envelope_id": "env-1",
        "payload": {"response_type": "ephemeral", "text": "ait is thinking..."},
    }
    assert len(ait_api.turn_calls) == 1
    assert slack_api.responses == [
        ("https://hooks.slack.com/commands/T000/B000/abc123", "AI says hello from Slack.", "in_channel")
    ]

    duplicate = service.handle_socket_envelope(envelope)

    assert duplicate == {
        "envelope_id": "env-1",
        "payload": {"response_type": "ephemeral", "text": "Duplicate Slack command ignored."},
    }
    assert len(ait_api.turn_calls) == 1


def test_slack_service_handles_ssl_check(tmp_path: Path):
    state_path = tmp_path / "slack-sync.json"
    config = _config(state_path)
    service = SlackBotService(
        config,
        slack_api=FakeSlackApi(),
        ait_api=FakeAitApi(),
        state_store=SlackSyncStateStore(state_path),
    )
    raw_payload = urlencode({"ssl_check": "1"})
    timestamp = str(int(time.time()))
    signature = build_slack_signature(raw_payload, config.signing_secret, timestamp=timestamp)

    response = service.handle_command_payload(
        raw_payload,
        signature=signature,
        signature_timestamp=timestamp,
    )

    assert response == {"response_type": "ephemeral", "text": "ok"}


def test_slack_service_rejects_bad_signature(tmp_path: Path):
    state_path = tmp_path / "slack-sync.json"
    config = _config(state_path)
    service = SlackBotService(
        config,
        slack_api=FakeSlackApi(),
        ait_api=FakeAitApi(),
        state_store=SlackSyncStateStore(state_path),
    )
    raw_payload = _command_payload()
    timestamp = str(int(time.time()))

    with pytest.raises(InvalidSlackSignatureError):
        service.handle_command_payload(
            raw_payload,
            signature="v0=" + ("00" * 32),
            signature_timestamp=timestamp,
        )
