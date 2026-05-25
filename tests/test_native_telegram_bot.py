from __future__ import annotations

import asyncio
from http.client import RemoteDisconnected
import json
import sys
import threading
import time
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import pytest
from fastapi.testclient import TestClient

import ait_server.app as server_app
import ait_chat.codex_app_server as codex_app_server_module
import ait_chat.codex_reply as codex_reply_module
import ait_chat.session_reply as session_reply_module
from ait_agent.envelope import (
    build_transport_event_envelope,
    build_transport_reply_envelope,
    build_transport_session_metadata,
)
from ait_agent.runtime_bindings import load_runtime_binding_state
import ait_agent.transport_retry as transport_retry_module
import ait_agent.telegram.app as telegram_app
import ait_agent.telegram.graph_watches as telegram_graph_watches
from ait_chat.session_reply import ReplyGenerationConfig, ReplyGenerationError, generate_session_reply
from ait_server.server_paths import ServerContext
from ait_server.server_store import append_session_event, create_session, create_session_checkpoint, ensure_repository, get_session, initialize, list_session_checkpoints, list_session_events
from ait_agent.telegram.app import AiReplyResult, BotConfig, TelegramBotService, detect_workflow_query, load_config, load_config_for_telegram_worker, normalize_user_text, parse_command, trigger_graph_watch_notifications
from ait_agent.telegram.runtime import TelegramSyncStateStore
from tests.postgres_fake import fake_postgres_dsn, install_fake_psycopg_global, reset_fake_postgres_runtime


class FakeTelegramApi:
    def __init__(self):
        self.sent_messages: list[tuple[str | int, str]] = []
        self.sent_documents: list[tuple[str | int, dict[str, Any]]] = []
        self.sent_audios: list[tuple[str | int, dict[str, Any]]] = []

    def send_message(self, chat_id, text):
        self.sent_messages.append((chat_id, text))

    def send_document(self, chat_id, attachment):
        self.sent_documents.append((chat_id, dict(attachment)))

    def send_audio(self, chat_id, attachment):
        self.sent_audios.append((chat_id, dict(attachment)))


class FakeSpeechToTextRuntime:
    def __init__(self, *, text: str = "transcript", fail_message: str | None = None):
        self.text = text
        self.fail_message = fail_message
        self.calls: list[dict[str, Any]] = []

    def transcribe_message(self, message, *, attachments):
        self.calls.append(
            {
                "message": dict(message),
                "attachments": [dict(item) for item in attachments],
            }
        )
        if self.fail_message is not None:
            raise telegram_app.LocalSpeechToTextError(self.fail_message)
        return telegram_app.LocalSpeechToTextTurnInput(
            text=self.text,
            attachments=tuple(dict(item) for item in attachments),
        )

class FakeWebhookService:
    def __init__(self) -> None:
        self.updates: list[dict] = []

    def handle_update(self, update: dict) -> None:
        self.updates.append(update)


def test_format_session_events_uses_plain_text_for_chat_transcript_events():
    formatted = telegram_app.format_session_events(
        [
            {
                "sequence": 152,
                "event_type": "session.message",
                "payload": {"text": "web prompt"},
                "actor_identity": "telegram:123:@weita",
            },
            {
                "sequence": 153,
                "event_type": "assistant.reply",
                "payload": {"text": "web reply"},
                "actor_identity": "ait-server",
            },
            {
                "sequence": 154,
                "event_type": "web.note",
                "payload": {"text": "Background update from web."},
                "actor_identity": "alice@example.com",
            },
        ]
    )

    assert formatted == "web prompt\n\nweb reply\n\n[154] web.note · alice@example.com\nBackground update from web."


def test_format_session_status_reports_runtime_and_link_metadata(tmp_path: Path):
    config = _config(
        tmp_path / "telegram-sync.json",
        background_sync_enabled=True,
        background_sync_interval_seconds=12.5,
        runtime_mode="remote",
        runtime_remote_name="origin",
    )
    link = {
        "repo_name": "ait",
        "session_id": "S-123",
        "canonical_session_id": "S-00123",
        "chat_id": "7661100833",
        "chat_title": "唯達",
        "workflow_notifications_enabled": True,
        "graph_watches": {"PL-123": {"plan_id": "PL-123"}},
        "last_synced_sequence": 154,
        "runtime_backend_mode": "remote",
        "runtime_backend_remote_name": "origin",
        "binding_role": "primary_shared",
    }
    session = {
        "telegram_context_runtime": {
            "reply_context_mode": "checkpoint_plus_delta",
            "checkpoint_freshness": "fresh",
            "delta_event_count": 3,
            "checkpoint_event_threshold": 12,
            "checkpoint_id": "K-123",
            "checkpoint_based_on_sequence": 151,
        }
    }

    formatted = telegram_app.format_session_status(config, link, session=session)

    assert "ait Telegram status" in formatted
    assert "session=S-123" in formatted
    assert "runtime_link=active" in formatted
    assert "runtime_backend=remote" in formatted
    assert "runtime_remote=origin" in formatted
    assert "sync_mode=background+manual (12.5s interval)" in formatted
    assert "workflow_notifications=on" in formatted
    assert "graph_watches=1" in formatted
    assert "checkpoint_delta_events=3/12" in formatted
    assert "checkpoint_id=K-123" in formatted
    assert "http://127.0.0.1:8000/sessions/S-123" in formatted


def test_format_workflow_notification_splits_land_and_complete_sections():
    config = _config(Path("/tmp/telegram-sync.json"))
    payload = {
        "summary": {"active": 3, "attention_required": 1, "ready_to_land": 1, "ready_to_complete": 1},
        "items": [
            {
                "task": {"task_id": "AITT-1000", "title": "Blocked task"},
                "workflow": {"state": "attention_required", "reason": "Policy is still pending."},
                "primary_gate": "policy",
                "ci_summary": {
                    "patchset_id": "AITP-1000",
                    "tg1_required": {"status": "pass", "live_count": 24, "minimum_count": 24},
                    "remote_land_gate": "pending",
                },
                "next_action": {"code": "inspect_change", "label": "Inspect change", "detail": "Open the focus change and fix policy."},
            },
            {
                "task": {"task_id": "AITT-1001", "title": "Landable task"},
                "workflow": {"state": "ready_to_land", "reason": "1 linked change can land now."},
                "next_action": {"code": "land_change", "label": "Land change", "detail": "Submit land for the selected patchset."},
            },
            {
                "task": {"task_id": "AITT-1002", "title": "Completable task"},
                "workflow": {"state": "ready_to_complete", "reason": "All linked changes are landed."},
                "next_action": {"code": "complete_task", "label": "Complete task", "detail": "Close the task after verifying target line state."},
            },
        ],
    }

    formatted = telegram_app.format_workflow_notification(config, payload)

    assert "\n\nPolicy\n" in formatted
    assert "\n\nReady to land\n" in formatted
    assert "\n\nReady to complete\n" in formatted
    assert "Need attention" not in formatted
    assert "TG-1=pass 24/24" in formatted
    assert "land=pending" in formatted
    assert "Ready now" not in formatted


def test_render_markdownish_message_chunks_formats_common_telegram_reply_patterns():
    chunks = telegram_app._render_markdownish_message_chunks(
        "# Summary\n\n- first\n- `second`\n\n‘quoted note’\n\n```python\nprint('hi')\n```"
    )

    assert len(chunks) == 1
    chunk = chunks[0]
    assert chunk.parse_mode == "HTML"
    assert "<b>Summary</b>" in chunk.text
    assert "• first" in chunk.text
    assert "• <code>second</code>" in chunk.text
    assert "❝ quoted note ❞" in chunk.text
    assert "<pre><code>print(&#x27;hi&#x27;)</code></pre>" in chunk.text


def test_telegram_api_client_send_message_falls_back_to_plain_text_on_markdown_parse_error(tmp_path: Path):
    client = telegram_app.TelegramApiClient(_config(tmp_path / "telegram-sync.json", reply_markdown_enabled=True))
    payloads: list[dict[str, Any]] = []

    def fake_post_json(_method_name: str, payload: dict[str, Any]) -> dict[str, Any]:
        payloads.append(dict(payload))
        if len(payloads) == 1:
            raise telegram_app.BotRuntimeError("Bad Request: can't parse entities: unsupported start tag")
        return {"ok": True}

    client._post_json = fake_post_json  # type: ignore[method-assign]
    client.send_message(123, "# Summary\n\n- item")

    assert payloads[0]["parse_mode"] == "HTML"
    assert payloads[1]["text"] == "Summary\n\n• item"
    assert "parse_mode" not in payloads[1]


class FakeAitApi:
    def __init__(
        self,
        *,
        reply_text: str = "AI says hello.",
        reply_transport_attachments: list[dict[str, Any]] | None = None,
        fail_turn: bool = False,
        fail_error: str = "test ai failure",
        event_trigger_intent: dict[str, Any] | None = None,
    ):
        self._next_session = 1
        self.sessions: dict[str, dict] = {}
        self.events: dict[str, list[dict]] = {}
        self.reply_text = reply_text
        self.reply_transport_attachments = list(reply_transport_attachments or [])
        self.fail_turn = fail_turn
        self.fail_error = fail_error
        self.event_trigger_intent = event_trigger_intent
        self.turn_calls: list[dict] = []
        self.task_dag_progress_payload = {
            "progress": {
                "graph_id": "test-graph",
                "completed_percent": 0,
                "completed_nodes": 0,
                "ready_nodes": 1,
                "running_nodes": 0,
                "blocked_nodes": 1,
                "next_action": "start A",
                "node_states": {
                    "A": {"state": "ready"},
                    "B": {"state": "blocked"},
                },
            },
            "blockers": [],
        }
        self.task_dag_progress_calls: list[dict] = []

    def _telegram_runtime_state(self, session_id: str) -> dict:
        session = self.sessions[session_id]
        last_event_sequence = int(session.get("last_event_sequence") or 0)
        checkpoint_id = str(session.get("head_checkpoint_id") or "").strip() or None
        checkpoint_sequence = int(session.get("checkpoint_based_on_sequence") or 0)
        delta_event_count = max(last_event_sequence - checkpoint_sequence, 0)
        threshold = 6
        if checkpoint_id:
            freshness = "fresh" if delta_event_count < threshold else "stale"
            reply_context_mode = "checkpoint_delta"
        else:
            freshness = "missing"
            reply_context_mode = "recent_tail"
        return {
            "reply_context_mode": reply_context_mode,
            "has_checkpoint": bool(checkpoint_id),
            "checkpoint_id": checkpoint_id,
            "checkpoint_created_at": None,
            "checkpoint_based_on_sequence": checkpoint_sequence,
            "last_event_sequence": last_event_sequence,
            "delta_event_count": delta_event_count,
            "checkpoint_event_threshold": threshold,
            "checkpoint_summary_event_limit": 8,
            "events_until_refresh": max(threshold - delta_event_count, 0),
            "checkpoint_freshness": freshness,
            "refresh_recommended": bool(checkpoint_id and delta_event_count >= threshold),
        }

    def _session_payload(self, session_id: str) -> dict:
        payload = dict(self.sessions[session_id])
        payload["telegram_context_runtime"] = self._telegram_runtime_state(session_id)
        return payload

    def create_session(
        self,
        *,
        chat_id: str,
        chat_title: str | None,
        chat_type: str | None,
        session_kind: str = "telegram_chat",
        title_prefix: str = "Telegram chat",
        metadata_extra: dict[str, Any] | None = None,
    ):
        session_id = f"AITS-TEST-{self._next_session:04d}"
        self._next_session += 1
        session = {
            "session_id": session_id,
            "session_kind": session_kind,
            "last_event_sequence": 0,
            "title": f"{title_prefix} · {chat_title or chat_id}",
            "chat_type": chat_type,
            "metadata": build_transport_session_metadata(
                transport="telegram",
                channel_id=chat_id,
                channel_title=chat_title,
                channel_kind=chat_type,
                linked_via="ait-agent telegram",
                metadata_extra={
                    "telegram_chat_id": str(chat_id),
                    "telegram_chat_title": chat_title,
                    "telegram_chat_type": chat_type,
                    **(metadata_extra or {}),
                },
            ),
        }
        self.sessions[session_id] = session
        self.events[session_id] = []
        return self._session_payload(session_id)

    def get_session(self, session_id: str, *, actor_identity: str = "ait-agent-telegram"):
        return self._session_payload(session_id)

    def append_session_event(self, session_id: str, *, event_type: str, payload: dict, actor_identity: str, actor_type: str):
        sequence = len(self.events[session_id]) + 1
        event = {
            "session_id": session_id,
            "sequence": sequence,
            "event_type": event_type,
            "payload": dict(payload),
            "actor_identity": actor_identity,
            "actor_type": actor_type,
        }
        self.events[session_id].append(event)
        self.sessions[session_id]["last_event_sequence"] = sequence
        return event

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
    ) -> dict:
        self.turn_calls.append(
            {
                "session_id": session_id,
                "text": text,
                "chat_id": str(chat_id),
                "chat_title": chat_title,
                "chat_type": chat_type,
                "telegram_message_id": telegram_message_id,
                "telegram_message_ids": list(telegram_message_ids or []),
                "transport_envelope": dict(transport_envelope or {}),
                "actor_identity": actor_identity,
            }
        )
        user_event = self.append_session_event(
            session_id,
            event_type="telegram.user_message",
            payload={
                "source": "telegram",
                "text": text,
                "telegram_chat_id": str(chat_id),
                "telegram_chat_title": chat_title,
                "telegram_chat_type": chat_type,
                "telegram_message_id": telegram_message_id,
                "telegram_message_ids": list(telegram_message_ids or []),
                "logical_turn_message_count": len(list(telegram_message_ids or [])),
                **({"transport_envelope": dict(transport_envelope)} if transport_envelope else {}),
            },
            actor_identity=actor_identity,
            actor_type="telegram_user",
        )
        if self.fail_turn:
            return {
                "ok": False,
                "session_id": session_id,
                "user_event": user_event,
                "assistant_event": None,
                "reply_text": None,
                "error": self.fail_error,
                "telegram_context_runtime": self._telegram_runtime_state(session_id),
            }
        assistant_event = self.append_session_event(
            session_id,
            event_type="assistant.reply",
            payload={
                "source": "openai",
                "text": self.reply_text,
                "model": "gpt-5.4-mini",
                "response_id": "resp_test_123",
                "usage": {"input_tokens": 11, "output_tokens": 7, "total_tokens": 18},
                "telegram_chat_id": str(chat_id),
                "telegram_chat_title": chat_title,
                "reply_to_sequence": int(user_event["sequence"]),
                "delivered_via": "telegram_live",
                "transport_reply_envelope": build_transport_reply_envelope(
                    transport="telegram",
                    channel_id=chat_id,
                    channel_title=chat_title,
                    channel_kind=chat_type,
                    text=self.reply_text,
                    attachments=self.reply_transport_attachments,
                    reply_to_event_id=(
                        str(transport_envelope.get("event_id") or "").strip() if transport_envelope else None
                    ),
                    reply_to_message_id=telegram_message_id,
                    reply_to_message_ids=telegram_message_ids,
                    metadata={"delivered_via": "telegram_live"},
                ),
            },
            actor_identity="ait-server",
            actor_type="ai_assistant",
        )
        if not self.sessions[session_id].get("head_checkpoint_id"):
            self.sessions[session_id]["head_checkpoint_id"] = "AITK-TEST-0001"
            self.sessions[session_id]["checkpoint_based_on_sequence"] = int(assistant_event["sequence"])
        return {
            "ok": True,
            "session_id": session_id,
            "user_event": user_event,
            "assistant_event": assistant_event,
            "reply_text": self.reply_text,
            "telegram_context_runtime": self._telegram_runtime_state(session_id),
        }

    def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50):
        return [event for event in self.events[session_id] if event["sequence"] > after_sequence][:limit]

    def read_task_queue(self):
        return getattr(self, "task_queue_payload", {
            "summary": {"active": 1, "attention_required": 0, "ready_to_land": 0, "ready_to_complete": 0},
            "items": [
                {
                    "task": {"task_id": "AITT-0010", "title": "Telegram bot task"},
                    "workflow": {"state": "planning", "reason": "No linked changes exist yet."},
                    "next_action": {"code": "create_change", "label": "Create change", "detail": "Open a first change for this task."},
                }
            ],
        })

    def read_task(self, task_id: str):
        return {
            "task": {"task_id": task_id, "title": "Telegram bot task", "status": "active", "risk_tier": "high", "intent": "Build bot"},
            "workflow": {"state": "planning"},
            "changes": [],
            "next_action": {"code": "create_change"},
        }

    def read_task_audit(self, task_id: str, *, target_line: str = "main"):
        return getattr(self, "task_audit_payload", {
            "task": {"task_id": task_id, "title": "Telegram bot task"},
            "workflow": {"state": "ready_to_land", "reason": "1 linked change can land now."},
            "recommended_action": {"code": "land_change", "label": "Land the focus change", "detail": "Open the change and submit land."},
            "target": {"line_name": target_line},
            "summary": {
                "verdict": "not_landed_on_target",
                "open_change_count": 1,
                "landed_change_count": 0,
                "effective_on_target_change_count": 0,
            },
            "changes": [
                {
                    "change": {"change_id": "AITC-0011", "status": "gated"},
                    "target_state": "not_on_target",
                }
            ],
        })

    def read_change(self, change_id: str):
        return getattr(self, "change_detail_payload", {
            "change": {"change_id": change_id, "title": "Implement bot", "status": "draft", "lane": "assisted", "risk_tier": "high"},
            "task": {"task_id": "AITT-0010"},
            "current_patchset": {"patchset_id": "AITP-0011-1"},
            "policy_summary": {"decision": "pending"},
            "review_summary": {"approvals": 0, "blocking": 0, "comments": 0},
            "freshness": {"base_is_fresh": True},
        })

    def read_task_dag_progress(self, graph: dict[str, Any]):
        self.task_dag_progress_calls.append(graph)
        return self.task_dag_progress_payload

    def classify_telegram_event_trigger(self, message: str):
        return self.event_trigger_intent


def _config(state_path: Path, **overrides) -> BotConfig:
    payload = {
        "token": "test-token",
        "username": "ait_test_bot",
        "ait_server_url": "http://127.0.0.1:8088",
        "ait_web_url": "http://127.0.0.1:8000",
        "repo_name": "ait",
        "request_timeout_seconds": 5.0,
        "poll_timeout_seconds": 5,
        "background_sync_enabled": False,
        "background_sync_interval_seconds": 30.0,
        "graph_watch_background_sweep_enabled": False,
        "openai_api_key": "test-openai-key",
        "openai_base_url": "https://api.openai.com/v1",
        "openai_model": "gpt-5.4-mini",
        "openai_reasoning_effort": "low",
        "openai_timeout_seconds": 30.0,
        "openai_max_output_tokens": 700,
        "ai_history_limit": 24,
        "turn_merge_window_seconds": 0.35,
        "turn_merge_max_messages": 4,
        "decoupled_reply_enabled": True,
        "reply_markdown_enabled": False,
        "sync_state_path": state_path,
        "env_path": state_path.parent / ".env",
    }
    payload.update(overrides)
    return BotConfig(**payload)


def _claim_telegram_owner(
    service: TelegramBotService,
    *,
    user_id: int = 456,
    username: str = "weita",
    chat_id: int = 123,
    chat_title: str = "Wei",
    password: str = "ait",
) -> None:
    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/start",
                "chat": {"id": chat_id, "type": "private", "first_name": chat_title},
                "from": {"id": user_id, "username": username},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": password,
                "chat": {"id": chat_id, "type": "private", "first_name": chat_title},
                "from": {"id": user_id, "username": username},
            },
        }
    )


def _runtime_snapshot_stub(label: str, *, mode: str, remote_name: str | None = None, server_url: str | None = None):
    return SimpleNamespace(
        label=label,
        mode=mode,
        remote_name=remote_name,
        server_url=server_url,
        signature=telegram_app._runtime_backend_signature(mode, remote_name, server_url),
    )


def test_ait_api_client_re_resolves_runtime_backend_per_call(monkeypatch, tmp_path: Path):
    config = _config(tmp_path / "telegram-sync.json", runtime_mode="remote", ait_server_url="http://old-startup")
    client = telegram_app.AitApiClient(config)
    repo_root = tmp_path / "repo"
    repo_root.mkdir()

    targets = [
        telegram_app.AgentRuntimeTarget(
            mode="remote",
            workflow_mode="solo_remote",
            repo_root=repo_root,
            repo_name="ait",
            remote_name="origin",
            server_url="http://remote.example.test",
        ),
        telegram_app.AgentRuntimeTarget(
            mode="local",
            workflow_mode="solo_local",
            repo_root=repo_root,
            repo_name="ait",
        ),
    ]
    remote_requests: list[dict[str, Any]] = []
    local_turn_calls: list[dict[str, Any]] = []

    class FakeLocalRuntime:
        def __init__(self, target):
            self.target = target

        def create_telegram_turn(self, session_id: str, **kwargs):
            local_turn_calls.append({"session_id": session_id, **kwargs})
            return {
                "ok": True,
                "session_id": session_id,
                "user_event": {"sequence": 1, "event_type": "telegram.user_message", "payload": {"text": kwargs["text"]}},
                "assistant_event": {"sequence": 2, "event_type": "assistant.reply", "payload": {"text": "local reply"}},
                "reply_text": "local reply",
            }

    def fake_resolve_agent_runtime_target(_repo_root: Path):
        assert targets
        return targets.pop(0)

    def fake_json_request(url: str, *, method: str = "GET", payload: dict[str, Any] | None = None, headers=None, timeout=20.0):
        remote_requests.append({"url": url, "method": method, "payload": dict(payload or {}), "headers": dict(headers or {})})
        return {
            "ok": True,
            "session_id": "S-REMOTE",
            "user_event": {"sequence": 1, "event_type": "telegram.user_message", "payload": {"text": payload["text"]}},
            "assistant_event": {"sequence": 2, "event_type": "assistant.reply", "payload": {"text": "remote reply"}},
            "reply_text": "remote reply",
        }

    monkeypatch.setattr(telegram_app, "resolve_agent_runtime_target", fake_resolve_agent_runtime_target)
    monkeypatch.setattr(telegram_app, "LocalAitRuntime", FakeLocalRuntime)
    monkeypatch.setattr(telegram_app, "_json_request", fake_json_request)

    remote_turn = client.create_telegram_turn(
        "S-REMOTE",
        text="first",
        chat_id=123,
        chat_title="Wei",
        chat_type="private",
        telegram_message_id=10,
        actor_identity="telegram:456:@weita",
    )
    local_turn = client.create_telegram_turn(
        "S-LOCAL",
        text="second",
        chat_id=123,
        chat_title="Wei",
        chat_type="private",
        telegram_message_id=11,
        actor_identity="telegram:456:@weita",
    )

    assert remote_turn["reply_text"] == "remote reply"
    assert local_turn["reply_text"] == "local reply"
    assert remote_requests == [
        {
            "url": "http://remote.example.test/v1/native/sessions/S-REMOTE:telegramTurn",
            "method": "POST",
            "payload": {
                "text": "first",
                "chat_id": "123",
                "chat_title": "Wei",
                "chat_type": "private",
                "telegram_message_id": 10,
            },
            "headers": {
                "X-AIT-Actor": "telegram:456:@weita",
                "X-AIT-Actor-Type": "telegram_user",
            },
        }
    ]
    assert local_turn_calls == [
        {
            "session_id": "S-LOCAL",
            "text": "second",
            "chat_id": 123,
            "chat_title": "Wei",
            "chat_type": "private",
            "telegram_message_id": 11,
            "telegram_message_ids": None,
            "transport_envelope": None,
            "actor_identity": "telegram:456:@weita",
        }
    ]


def test_service_keeps_one_runtime_snapshot_per_telegram_turn(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    snapshot_a = _runtime_snapshot_stub("remote-a", mode="remote", remote_name="origin", server_url="http://remote-a")
    snapshot_b = _runtime_snapshot_stub("local-b", mode="local")

    class SnapshotRecordingAitApi(FakeAitApi):
        def __init__(self):
            super().__init__(reply_text="snapshot reply")
            self.snapshot_requests = 0
            self.create_session_snapshot_labels: list[str | None] = []
            self.turn_snapshot_labels: list[str | None] = []

        def capture_runtime_snapshot(self):
            self.snapshot_requests += 1
            return snapshot_a if self.snapshot_requests == 1 else snapshot_b

        def create_session(self, *, runtime_snapshot=None, **kwargs):
            self.create_session_snapshot_labels.append(getattr(runtime_snapshot, "label", None))
            return super().create_session(
                chat_id=kwargs["chat_id"],
                chat_title=kwargs.get("chat_title"),
                chat_type=kwargs.get("chat_type"),
                session_kind=kwargs.get("session_kind", "telegram_chat"),
                title_prefix=kwargs.get("title_prefix", "Telegram chat"),
                metadata_extra=kwargs.get("metadata_extra"),
            )

        def get_session(self, session_id: str, *, actor_identity: str = "ait-agent-telegram", runtime_snapshot=None):
            return super().get_session(session_id, actor_identity=actor_identity)

        def create_telegram_turn(self, session_id: str, *, runtime_snapshot=None, **kwargs):
            self.turn_snapshot_labels.append(getattr(runtime_snapshot, "label", None))
            return super().create_telegram_turn(session_id, **kwargs)

    ait_api = SnapshotRecordingAitApi()
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Hot-switch me",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    assert ait_api.snapshot_requests == 1
    assert ait_api.create_session_snapshot_labels == ["remote-a"]
    assert ait_api.turn_snapshot_labels == ["remote-a"]


def test_service_relinks_chat_session_when_runtime_backend_changes(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    current_snapshot = _runtime_snapshot_stub("local-now", mode="local")

    class RelinkingAitApi(FakeAitApi):
        def __init__(self):
            super().__init__(reply_text="Relinked reply.")
            self.created_session_ids: list[str] = []
            self.turn_session_ids: list[str] = []

        def capture_runtime_snapshot(self):
            return current_snapshot

        def create_session(self, *, runtime_snapshot=None, **kwargs):
            session = super().create_session(
                chat_id=kwargs["chat_id"],
                chat_title=kwargs.get("chat_title"),
                chat_type=kwargs.get("chat_type"),
                session_kind=kwargs.get("session_kind", "telegram_chat"),
                title_prefix=kwargs.get("title_prefix", "Telegram chat"),
                metadata_extra=kwargs.get("metadata_extra"),
            )
            self.created_session_ids.append(session["session_id"])
            return session

        def get_session(self, session_id: str, *, actor_identity: str = "ait-agent-telegram", runtime_snapshot=None):
            if session_id not in self.sessions:
                raise telegram_app.BotRuntimeError("session not found on current backend")
            return super().get_session(session_id, actor_identity=actor_identity)

        def create_telegram_turn(self, session_id: str, *, runtime_snapshot=None, **kwargs):
            self.turn_session_ids.append(session_id)
            return super().create_telegram_turn(session_id, **kwargs)

    ait_api = RelinkingAitApi()
    state_store = TelegramSyncStateStore(state_path)
    state_store.upsert_chat(
        123,
        session_id="AITS-REMOTE-OLD",
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id="AITS-REMOTE-OLD",
        binding_role="primary_shared",
        last_synced_sequence=4,
        runtime_backend_mode="remote",
        runtime_backend_remote_name="origin",
        runtime_backend_server_url="http://old-remote",
        runtime_backend_signature=telegram_app._runtime_backend_signature("remote", "origin", "http://old-remote"),
    )
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=state_store,
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Continue locally",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    chat_state = state_store.get_chat(123)
    assert chat_state["session_id"] == ait_api.created_session_ids[0]
    assert chat_state["previous_session_id"] == "AITS-REMOTE-OLD"
    assert chat_state["relink_reason"] == "runtime_backend_changed"
    assert chat_state["runtime_backend_signature"] == current_snapshot.signature
    assert ait_api.turn_session_ids == [ait_api.created_session_ids[0]]
    assert telegram_api.sent_messages[-1] == (123, "Relinked reply.")


def test_service_fresh_topic_trigger_creates_new_session_without_ai_turn(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    ait_api = FakeAitApi(reply_text="should not be used")
    telegram_api = FakeTelegramApi()
    state_store = TelegramSyncStateStore(state_path)
    state_store.upsert_chat(
        123,
        session_id="AITS-OLD-0001",
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id="AITS-OLD-0001",
        binding_role="primary_shared",
        last_synced_sequence=4,
    )
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=state_store,
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "換個話題",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    chat_state = state_store.get_chat(123)
    assert chat_state["session_id"] == "AITS-TEST-0001"
    assert chat_state["previous_session_id"] == "AITS-OLD-0001"
    assert chat_state["relink_reason"] == "fresh_topic_event_trigger"
    assert ait_api.turn_calls == []
    assert telegram_api.sent_messages[-1] == (
        123,
        "Started a fresh Telegram-linked session.\nTrigger: 換個話題.\nSession: AITS-TEST-0001",
    )


def test_service_fresh_topic_trigger_with_topic_hint_skips_ai_turn(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    ait_api = FakeAitApi(reply_text="should not be used")
    telegram_api = FakeTelegramApi()
    state_store = TelegramSyncStateStore(state_path)
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=state_store,
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "換個話題跟 release plan 有關！",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    chat_state = state_store.get_chat(123)
    assert chat_state["session_id"] == "AITS-TEST-0001"
    assert chat_state.get("previous_session_id") is None
    assert chat_state["relink_reason"] == "fresh_topic_event_trigger"
    assert ait_api.turn_calls == []
    assert telegram_api.sent_messages[-1] == (
        123,
        "Started a fresh Telegram-linked session.\nTrigger: 換個話題跟…有關.\nTopic hint: release plan\nSession: AITS-TEST-0001",
    )


def test_status_command_reports_relink_required_after_runtime_backend_change(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    current_snapshot = _runtime_snapshot_stub("local-now", mode="local")

    class StatusAitApi(FakeAitApi):
        def capture_runtime_snapshot(self):
            return current_snapshot

        def get_session(self, session_id: str, *, actor_identity: str = "ait-agent-telegram", runtime_snapshot=None):
            raise telegram_app.BotRuntimeError("session not found on current backend")

    state_store = TelegramSyncStateStore(state_path)
    state_store.upsert_chat(
        123,
        session_id="AITS-REMOTE-OLD",
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id="AITS-REMOTE-OLD",
        binding_role="primary_shared",
        last_synced_sequence=4,
        runtime_backend_mode="remote",
        runtime_backend_remote_name="origin",
        runtime_backend_server_url="http://old-remote",
        runtime_backend_signature=telegram_app._runtime_backend_signature("remote", "origin", "http://old-remote"),
    )
    service = TelegramBotService(
        _config(state_path),
        ait_api=StatusAitApi(),
        telegram_api=telegram_api,
        state_store=state_store,
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/status",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    status_text = telegram_api.sent_messages[-1][1]
    assert "runtime_link=relink_required" in status_text
    assert "runtime_backend=local" in status_text
    assert "relink_reason=runtime_backend_changed" in status_text


def test_telegram_api_send_message_retries_retryable_transport_errors(monkeypatch, tmp_path: Path):
    client = telegram_app.TelegramApiClient(_config(tmp_path / "telegram-sync.json", request_timeout_seconds=None))
    request_calls: list[dict[str, Any]] = []
    sleep_delays: list[float] = []

    def fake_json_request(url: str, *, method: str = "GET", payload: dict[str, Any] | None = None, headers=None, timeout=20.0):
        request_calls.append({"url": url, "method": method, "payload": dict(payload or {}), "timeout": timeout})
        if len(request_calls) < 3:
            try:
                raise telegram_app.BotRuntimeError("POST https://api.telegram.org/bottest-token/sendMessage failed: [Errno 60] Operation timed out") from TimeoutError("timed out")
            except telegram_app.BotRuntimeError as exc:
                raise exc
        return {"ok": True}

    monkeypatch.setattr(telegram_app, "_json_request", fake_json_request)
    monkeypatch.setattr(telegram_app.time, "sleep", lambda seconds: sleep_delays.append(seconds))

    client.send_message(123, "hello from ait")

    assert len(request_calls) == 3
    assert all(call["method"] == "POST" for call in request_calls)
    assert all(call["timeout"] is None for call in request_calls)
    assert sleep_delays == [1.0, 2.0]


def test_telegram_api_get_updates_retries_retryable_transport_errors(monkeypatch, tmp_path: Path):
    client = telegram_app.TelegramApiClient(_config(tmp_path / "telegram-sync.json", request_timeout_seconds=None))
    request_calls: list[dict[str, Any]] = []
    sleep_delays: list[float] = []

    def fake_json_request(url: str, *, method: str = "GET", payload: dict[str, Any] | None = None, headers=None, timeout=20.0):
        request_calls.append({"url": url, "timeout": timeout})
        if len(request_calls) < 3:
            raise telegram_app.BotRuntimeError(
                "GET https://api.telegram.org/bottest-token/getUpdates failed: [Errno 54] Connection reset by peer"
            ) from ConnectionResetError(54, "Connection reset by peer")
        return {"ok": True, "result": [{"update_id": 7}]}

    monkeypatch.setattr(telegram_app, "_json_request", fake_json_request)
    monkeypatch.setattr(telegram_app.time, "sleep", lambda seconds: sleep_delays.append(seconds))

    updates = client.get_updates(offset=3, timeout_seconds=5)

    assert updates == [{"update_id": 7}]
    assert len(request_calls) == 3
    assert all(call["timeout"] == 15 for call in request_calls)
    assert sleep_delays == [1.0, 2.0]


def test_telegram_api_get_updates_keeps_larger_explicit_request_timeout(monkeypatch, tmp_path: Path):
    client = telegram_app.TelegramApiClient(_config(tmp_path / "telegram-sync.json", request_timeout_seconds=30.0))
    captured: list[float | None] = []

    def fake_json_request(url: str, *, method: str = "GET", payload: dict[str, Any] | None = None, headers=None, timeout=20.0):
        captured.append(timeout)
        return {"ok": True, "result": []}

    monkeypatch.setattr(telegram_app, "_json_request", fake_json_request)

    updates = client.get_updates(offset=3, timeout_seconds=5)

    assert updates == []
    assert captured == [30.0]


def test_async_telegram_api_send_message_retries_retryable_transport_errors(monkeypatch, tmp_path: Path):
    class DummyAsyncClient:
        async def aclose(self) -> None:
            return None

    client = telegram_app.AsyncTelegramApiClient(
        _config(tmp_path / "telegram-sync.json", request_timeout_seconds=None),
        http_client=DummyAsyncClient(),
    )
    request_calls: list[dict[str, Any]] = []
    sleep_delays: list[float] = []

    async def fake_async_json_request(
        url: str,
        *,
        method: str = "GET",
        payload: dict[str, Any] | None = None,
        headers=None,
        timeout: float | None = 20.0,
        client=None,
    ):
        request_calls.append({"url": url, "method": method, "payload": dict(payload or {}), "timeout": timeout})
        if len(request_calls) < 3:
            try:
                raise telegram_app.BotRuntimeError("POST https://api.telegram.org/bottest-token/sendMessage failed: [Errno 60] Operation timed out") from TimeoutError("timed out")
            except telegram_app.BotRuntimeError as exc:
                raise exc
        return {"ok": True}

    async def fake_sleep(seconds: float) -> None:
        sleep_delays.append(seconds)

    monkeypatch.setattr(telegram_app, "_async_json_request", fake_async_json_request)
    monkeypatch.setattr(transport_retry_module.asyncio, "sleep", fake_sleep)

    asyncio.run(client.send_message(123, "hello from ait"))

    assert len(request_calls) == 3
    assert all(call["method"] == "POST" for call in request_calls)
    assert all(call["timeout"] is None for call in request_calls)
    assert sleep_delays == [1.0, 2.0]


def test_async_telegram_api_get_updates_retries_retryable_transport_errors(monkeypatch, tmp_path: Path):
    class DummyAsyncClient:
        async def aclose(self) -> None:
            return None

    client = telegram_app.AsyncTelegramApiClient(
        _config(tmp_path / "telegram-sync.json", request_timeout_seconds=None),
        http_client=DummyAsyncClient(),
    )
    request_calls: list[dict[str, Any]] = []
    sleep_delays: list[float] = []

    async def fake_async_json_request(
        url: str,
        *,
        method: str = "GET",
        payload: dict[str, Any] | None = None,
        headers=None,
        timeout: float | None = 20.0,
        client=None,
    ):
        request_calls.append({"url": url, "timeout": timeout})
        if len(request_calls) < 3:
            raise telegram_app.BotRuntimeError(
                "GET https://api.telegram.org/bottest-token/getUpdates failed: [Errno 54] Connection reset by peer"
            ) from ConnectionResetError(54, "Connection reset by peer")
        return {"ok": True, "result": [{"update_id": 7}]}

    async def fake_sleep(seconds: float) -> None:
        sleep_delays.append(seconds)

    monkeypatch.setattr(telegram_app, "_async_json_request", fake_async_json_request)
    monkeypatch.setattr(transport_retry_module.asyncio, "sleep", fake_sleep)

    updates = asyncio.run(client.get_updates(offset=3, timeout_seconds=5))

    assert updates == [{"update_id": 7}]
    assert len(request_calls) == 3
    assert all(call["timeout"] == 15 for call in request_calls)
    assert sleep_delays == [1.0, 2.0]


def test_async_ait_api_client_read_calls_are_repo_scoped(monkeypatch, tmp_path: Path):
    class DummyAsyncClient:
        async def aclose(self) -> None:
            return None

    calls: list[dict[str, Any]] = []
    repo_root = tmp_path / "repo"
    repo_root.mkdir()

    async def fake_async_json_request(
        url: str,
        *,
        method: str = "GET",
        payload: dict[str, Any] | None = None,
        headers=None,
        timeout: float | None = 20.0,
        client=None,
    ):
        calls.append({"url": url, "method": method, "payload": payload, "timeout": timeout, "headers": headers})
        return {}

    monkeypatch.setattr(telegram_app, "_async_json_request", fake_async_json_request)
    monkeypatch.setattr(
        telegram_app,
        "resolve_agent_runtime_target",
        lambda _repo_root: telegram_app.AgentRuntimeTarget(
            mode="remote",
            workflow_mode="solo_remote",
            repo_root=repo_root,
            repo_name="ait",
            remote_name="origin",
            server_url="http://127.0.0.1:8088",
        ),
    )
    client = telegram_app.AsyncAitApiClient(
        _config(tmp_path / "telegram-read-scoped.json"),
        http_client=DummyAsyncClient(),
    )

    asyncio.run(client.read_task("AITT-0010"))
    asyncio.run(client.read_task_audit("AITT-0010"))
    asyncio.run(client.read_change("AITC-0011"))

    assert len(calls) == 3
    assert calls[0]["method"] == "GET"
    assert calls[0]["url"].endswith("/v1/native/repositories/ait/read/tasks/AITT-0010")
    assert calls[1]["url"].endswith("/v1/native/repositories/ait/read/tasks/AITT-0010/audit?target_line=main")
    assert calls[2]["url"].endswith("/v1/native/repositories/ait/read/changes/AITC-0011")


def test_async_ait_api_client_retries_retryable_loopback_read_requests(monkeypatch, tmp_path: Path):
    class DummyAsyncClient:
        async def aclose(self) -> None:
            return None

    calls: list[str] = []
    sleep_delays: list[float] = []
    repo_root = tmp_path / "repo"
    repo_root.mkdir()

    async def fake_async_json_request(
        url: str,
        *,
        method: str = "GET",
        payload: dict[str, Any] | None = None,
        headers=None,
        timeout: float | None = 20.0,
        client=None,
    ):
        calls.append(url)
        if len(calls) < 3:
            raise telegram_app.BotRuntimeError(
                f"{method} {url} failed: [Errno 61] Connection refused"
            ) from ConnectionRefusedError(61, "Connection refused")
        return {"summary": {"active": 0}}

    async def fake_sleep(seconds: float) -> None:
        sleep_delays.append(seconds)

    monkeypatch.setattr(telegram_app, "_async_json_request", fake_async_json_request)
    monkeypatch.setattr(transport_retry_module.asyncio, "sleep", fake_sleep)
    monkeypatch.setattr(
        telegram_app,
        "resolve_agent_runtime_target",
        lambda _repo_root: telegram_app.AgentRuntimeTarget(
            mode="remote",
            workflow_mode="solo_remote",
            repo_root=repo_root,
            repo_name="ait",
            remote_name="origin",
            server_url="http://127.0.0.1:8088",
        ),
    )
    client = telegram_app.AsyncAitApiClient(
        _config(tmp_path / "telegram-read-retry.json"),
        http_client=DummyAsyncClient(),
    )

    payload = asyncio.run(client.read_task_queue())

    assert payload == {"summary": {"active": 0}}
    assert len(calls) == 3
    assert sleep_delays == [0.75, 1.5]


def test_async_ait_api_client_retries_retryable_loopback_telegram_turn_writes(monkeypatch, tmp_path: Path):
    calls: list[dict[str, Any]] = []
    sleep_delays: list[float] = []
    repo_root = tmp_path / "repo"
    repo_root.mkdir()

    async def fake_async_json_request(
        url: str,
        *,
        method: str = "GET",
        payload: dict[str, Any] | None = None,
        headers=None,
        timeout: float | None = 20.0,
        client=None,
    ):
        calls.append({"url": url, "method": method, "payload": dict(payload or {}), "headers": dict(headers or {})})
        if len(calls) < 3:
            raise telegram_app.BotRuntimeError(
                f"{method} {url} failed: [Errno 61] Connection refused"
            ) from ConnectionRefusedError(61, "Connection refused")
        return {
            "ok": True,
            "session_id": "S-TEST",
            "user_event": {"sequence": 1, "event_type": "telegram.user_message", "payload": {"text": payload["text"]}},
            "assistant_event": {"sequence": 2, "event_type": "assistant.reply", "payload": {"text": "Recovered reply."}},
            "reply_text": "Recovered reply.",
        }

    async def fake_sleep(seconds: float) -> None:
        sleep_delays.append(seconds)

    monkeypatch.setattr(telegram_app, "_async_json_request", fake_async_json_request)
    monkeypatch.setattr(transport_retry_module.asyncio, "sleep", fake_sleep)
    monkeypatch.setattr(
        telegram_app,
        "resolve_agent_runtime_target",
        lambda _repo_root: telegram_app.AgentRuntimeTarget(
            mode="remote",
            workflow_mode="solo_remote",
            repo_root=repo_root,
            repo_name="ait",
            remote_name="origin",
            server_url="http://127.0.0.1:8088",
        ),
    )
    client = telegram_app.AsyncAitApiClient(_config(tmp_path / "telegram-turn-write-retry.json"))

    payload = asyncio.run(
        client.create_telegram_turn(
            "S-TEST",
            text="retry me",
            chat_id=123,
            chat_title="Wei",
            chat_type="private",
            telegram_message_id=11,
            telegram_message_ids=[11],
            actor_identity="telegram:456:@weita",
        )
    )

    assert payload["reply_text"] == "Recovered reply."
    assert len(calls) == 3
    assert sleep_delays == [0.75, 1.5]


def test_ait_api_client_read_calls_are_repo_scoped(monkeypatch, tmp_path: Path):
    calls: list[dict[str, Any]] = []
    repo_root = tmp_path / "repo"
    repo_root.mkdir()

    def fake_json_request(
        url: str,
        *,
        method: str = "GET",
        payload: dict[str, Any] | None = None,
        headers=None,
        timeout: float | None = 20.0,
    ):
        calls.append({"url": url, "method": method, "payload": payload, "timeout": timeout, "headers": headers})
        return {}

    monkeypatch.setattr(telegram_app, "_json_request", fake_json_request)
    monkeypatch.setattr(
        telegram_app,
        "resolve_agent_runtime_target",
        lambda _repo_root: telegram_app.AgentRuntimeTarget(
            mode="remote",
            workflow_mode="solo_remote",
            repo_root=repo_root,
            repo_name="ait",
            remote_name="origin",
            server_url="http://127.0.0.1:8088",
        ),
    )
    client = telegram_app.AitApiClient(_config(tmp_path / "telegram-read-scoped.json"))

    client.read_task("AITT-0010")
    client.read_task_audit("AITT-0010")
    client.read_change("AITC-0011")

    assert len(calls) == 3
    assert calls[0]["method"] == "GET"
    assert calls[0]["url"].endswith("/v1/native/repositories/ait/read/tasks/AITT-0010")
    assert calls[1]["url"].endswith("/v1/native/repositories/ait/read/tasks/AITT-0010/audit?target_line=main")
    assert calls[2]["url"].endswith("/v1/native/repositories/ait/read/changes/AITC-0011")


def test_trigger_graph_watch_notifications_uses_request_helpers_for_remote_runtime(
    monkeypatch,
    tmp_path: Path,
):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_payload = {
        "schema_version": 1,
        "graph_id": "demo-graph",
        "repo_name": "ait",
        "source_plan": {"plan_id": "PL-TEST123"},
        "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
    }
    graph_path.write_text(json.dumps(graph_payload), encoding="utf-8")

    config = _config(state_path)
    store = TelegramSyncStateStore(state_path)
    store.upsert_chat(
        123,
        session_id="S-123",
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
    )
    telegram_graph_watches.upsert_graph_watch_for_chat(
        config,
        chat_id=123,
        plan_id="PL-TEST123",
        graph_path=str(graph_path),
        progress_reader=lambda graph: {
            "progress": {
                "graph_id": "demo-graph",
                "completed_percent": 0,
                "completed_nodes": 0,
                "ready_nodes": 1,
                "running_nodes": 0,
                "blocked_nodes": 0,
                "next_action": "start A",
                "node_states": {"A": {"state": "ready"}},
            },
            "blockers": [],
        },
        state_store=store,
        notify_policy="never",
    )

    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    request_calls: list[dict[str, Any]] = []
    progress_payload = {
        "progress": {
            "graph_id": "demo-graph",
            "completed_percent": 50,
            "completed_nodes": 1,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "next_action": "start B",
            "node_states": {
                "A": {"state": "completed"},
                "B": {"state": "ready"},
            },
        },
        "blockers": [],
    }

    class _Response:
        def __init__(self, payload: dict[str, Any]):
            self._payload = payload

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb):
            return False

        def read(self) -> bytes:
            return json.dumps(self._payload).encode("utf-8")

    def fake_urlopen(request, timeout=None):
        raw_payload = request.data.decode("utf-8") if request.data else ""
        payload = json.loads(raw_payload) if raw_payload else None
        request_calls.append(
            {
                "url": request.full_url,
                "method": request.get_method(),
                "payload": payload,
                "timeout": timeout,
            }
        )
        if request.full_url.endswith("/v1/native/read/task-dag-progress"):
            return _Response(progress_payload)
        if request.full_url.endswith("/sendMessage"):
            return _Response({"ok": True})
        raise AssertionError(f"Unexpected request URL: {request.full_url}")

    monkeypatch.setattr(telegram_app, "urlopen", fake_urlopen)
    monkeypatch.setattr(
        telegram_app,
        "resolve_agent_runtime_target",
        lambda _repo_root: telegram_app.AgentRuntimeTarget(
            mode="remote",
            workflow_mode="solo_remote",
            repo_root=repo_root,
            repo_name="ait",
            remote_name="origin",
            server_url="http://127.0.0.1:8088",
        ),
    )

    summary = trigger_graph_watch_notifications(
        config,
        repo_name="ait",
        state_store=store,
        progress_reader=telegram_app.AitApiClient(config).read_task_dag_progress,
    )

    assert summary["sent"] == 1
    assert len(request_calls) == 2
    assert request_calls[0]["method"] == "POST"
    assert request_calls[0]["url"].endswith("/v1/native/read/task-dag-progress")
    assert request_calls[0]["payload"] == {"graph": graph_payload}
    assert request_calls[1]["method"] == "POST"
    assert request_calls[1]["url"].endswith("/sendMessage")
    assert request_calls[1]["payload"]["text"] == "A —> B (50%)"


def test_trigger_graph_watch_notifications_can_filter_to_target_plan_ids(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_one = tmp_path / "one.task_graph.json"
    graph_two = tmp_path / "two.task_graph.json"
    graph_one.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "graph-one",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-ONE"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    graph_two.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "graph-two",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TWO"},
                "nodes": [{"node_id": "B", "node_kind": "task", "title": "B"}],
            }
        ),
        encoding="utf-8",
    )

    config = _config(state_path)
    store = TelegramSyncStateStore(state_path)
    store.upsert_chat(
        123,
        session_id="S-123",
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
    )
    for plan_id, graph_path, node_id in (
        ("PL-ONE", graph_one, "A"),
        ("PL-TWO", graph_two, "B"),
    ):
        telegram_graph_watches.upsert_graph_watch_for_chat(
            config,
            chat_id=123,
            plan_id=plan_id,
            graph_path=str(graph_path),
            progress_reader=lambda graph, node_id=node_id: {
                "progress": {
                    "graph_id": graph["graph_id"],
                    "completed_percent": 0,
                    "completed_nodes": 0,
                    "ready_nodes": 1,
                    "running_nodes": 0,
                    "blocked_nodes": 0,
                    "next_action": f"start {node_id}",
                    "node_states": {node_id: {"state": "ready"}},
                },
                "blockers": [],
            },
            state_store=store,
            notify_policy="never",
        )

    telegram_api = FakeTelegramApi()
    progress_calls: list[str] = []

    def progress_reader(graph: dict[str, Any]) -> dict[str, Any]:
        progress_calls.append(str(graph.get("graph_id") or ""))
        return {
            "progress": {
                "graph_id": graph["graph_id"],
                "completed_percent": 100 if graph["graph_id"] == "graph-two" else 0,
                "completed_nodes": 1 if graph["graph_id"] == "graph-two" else 0,
                "ready_nodes": 0 if graph["graph_id"] == "graph-two" else 1,
                "running_nodes": 0,
                "blocked_nodes": 0,
                "next_action": "complete task graph" if graph["graph_id"] == "graph-two" else "start A",
                "node_states": {"B" if graph["graph_id"] == "graph-two" else "A": {"state": "completed" if graph["graph_id"] == "graph-two" else "ready"}},
            },
            "blockers": [],
        }

    summary = trigger_graph_watch_notifications(
        config,
        repo_name="ait",
        plan_ids={"PL-TWO"},
        state_store=store,
        telegram_api=telegram_api,
        progress_reader=progress_reader,
    )

    assert summary["checked"] == 1
    assert summary["sent"] == 1
    assert progress_calls == ["graph-two"]
    assert len(telegram_api.sent_messages) == 1
    assert telegram_api.sent_messages[0][1] == "start B —> complete task graph (100%)"


def test_ait_api_client_retries_retryable_loopback_read_requests(monkeypatch, tmp_path: Path):
    calls: list[str] = []
    sleep_delays: list[float] = []
    repo_root = tmp_path / "repo"
    repo_root.mkdir()

    def fake_json_request(
        url: str,
        *,
        method: str = "GET",
        payload: dict[str, Any] | None = None,
        headers=None,
        timeout: float | None = 20.0,
    ):
        calls.append(url)
        if len(calls) < 3:
            raise telegram_app.BotRuntimeError(
                f"{method} {url} failed: [Errno 61] Connection refused"
            ) from ConnectionRefusedError(61, "Connection refused")
        return {"summary": {"active": 0}}

    monkeypatch.setattr(telegram_app, "_json_request", fake_json_request)
    monkeypatch.setattr(telegram_app.time, "sleep", lambda seconds: sleep_delays.append(seconds))
    monkeypatch.setattr(
        telegram_app,
        "resolve_agent_runtime_target",
        lambda _repo_root: telegram_app.AgentRuntimeTarget(
            mode="remote",
            workflow_mode="solo_remote",
            repo_root=repo_root,
            repo_name="ait",
            remote_name="origin",
            server_url="http://127.0.0.1:8088",
        ),
    )
    client = telegram_app.AitApiClient(_config(tmp_path / "telegram-read-retry.json"))

    payload = client.read_task_queue()

    assert payload == {"summary": {"active": 0}}
    assert len(calls) == 3
    assert sleep_delays == [0.75, 1.5]


def test_ait_api_client_retries_retryable_loopback_telegram_turn_writes(monkeypatch, tmp_path: Path):
    calls: list[dict[str, Any]] = []
    sleep_delays: list[float] = []
    repo_root = tmp_path / "repo"
    repo_root.mkdir()

    def fake_json_request(
        url: str,
        *,
        method: str = "GET",
        payload: dict[str, Any] | None = None,
        headers=None,
        timeout: float | None = 20.0,
    ):
        calls.append({"url": url, "method": method, "payload": dict(payload or {}), "headers": dict(headers or {})})
        if len(calls) < 3:
            raise telegram_app.BotRuntimeError(
                f"{method} {url} failed: [Errno 61] Connection refused"
            ) from ConnectionRefusedError(61, "Connection refused")
        return {
            "ok": True,
            "session_id": "S-TEST",
            "user_event": {"sequence": 1, "event_type": "telegram.user_message", "payload": {"text": payload["text"]}},
            "assistant_event": {"sequence": 2, "event_type": "assistant.reply", "payload": {"text": "Recovered reply."}},
            "reply_text": "Recovered reply.",
        }

    monkeypatch.setattr(telegram_app, "_json_request", fake_json_request)
    monkeypatch.setattr(telegram_app.time, "sleep", lambda seconds: sleep_delays.append(seconds))
    monkeypatch.setattr(
        telegram_app,
        "resolve_agent_runtime_target",
        lambda _repo_root: telegram_app.AgentRuntimeTarget(
            mode="remote",
            workflow_mode="solo_remote",
            repo_root=repo_root,
            repo_name="ait",
            remote_name="origin",
            server_url="http://127.0.0.1:8088",
        ),
    )
    client = telegram_app.AitApiClient(_config(tmp_path / "telegram-turn-write-retry.json"))

    payload = client.create_telegram_turn(
        "S-TEST",
        text="retry me",
        chat_id=123,
        chat_title="Wei",
        chat_type="private",
        telegram_message_id=11,
        telegram_message_ids=[11],
        actor_identity="telegram:456:@weita",
    )

    assert payload["reply_text"] == "Recovered reply."
    assert len(calls) == 3
    assert sleep_delays == [0.75, 1.5]


def test_telegram_api_send_message_does_not_retry_non_retryable_transport_errors(monkeypatch, tmp_path: Path):
    client = telegram_app.TelegramApiClient(_config(tmp_path / "telegram-sync.json"))
    request_calls: list[str] = []
    sleep_delays: list[float] = []

    def fake_json_request(url: str, *, method: str = "GET", payload: dict[str, Any] | None = None, headers=None, timeout=20.0):
        request_calls.append(url)
        raise telegram_app.BotRuntimeError("POST https://api.telegram.org/bottest-token/sendMessage failed: 400 bad request")

    monkeypatch.setattr(telegram_app, "_json_request", fake_json_request)
    monkeypatch.setattr(telegram_app.time, "sleep", lambda seconds: sleep_delays.append(seconds))

    with pytest.raises(telegram_app.BotRuntimeError, match="400 bad request"):
        client.send_message(123, "hello from ait")

    assert request_calls == ["https://api.telegram.org/bottest-token/sendMessage"]
    assert sleep_delays == []


def test_consume_pending_termination_context_only_returns_matching_pid(tmp_path: Path):
    context_path = tmp_path / "telegram-termination.json"
    context_path.write_text(
        json.dumps(
            {
                "pid": 4242,
                "reason": "cli_telegram_stop",
                "worker_name": "main",
                "issued_at": "2026-05-05T11:30:00+00:00",
                "issued_by_pid": 999,
            }
        ),
        encoding="utf-8",
    )
    env = {telegram_app.AIT_TELEGRAM_TERMINATION_CONTEXT_ENV: str(context_path)}

    assert telegram_app._consume_pending_termination_context(pid=1111, env=env) is None
    assert context_path.exists()

    payload = telegram_app._consume_pending_termination_context(pid=4242, env=env)

    assert payload is not None
    assert payload["reason"] == "cli_telegram_stop"
    assert not context_path.exists()


def _reply_config(**overrides) -> ReplyGenerationConfig:
    payload = {
        "repo_name": "ait",
        "openai_api_key": "test-openai-key",
        "openai_base_url": "https://api.openai.com/v1",
        "openai_model": "gpt-5.4-mini",
        "openai_reasoning_effort": "low",
        "openai_timeout_seconds": 30.0,
        "openai_max_output_tokens": 700,
        "history_limit": 24,
        "telegram_checkpoint_event_threshold": 6,
        "telegram_checkpoint_summary_event_limit": 8,
        "telegram_append_turn_analysis": False,
    }
    payload.update(overrides)
    return ReplyGenerationConfig(**payload)


def _write_bound_repo_checkout(
    root: Path,
    repo_name: str,
    *,
    env_lines: list[str] | None = None,
    worker_payload: dict[str, Any] | None = None,
) -> Path:
    runtime_dir = root / ".ait" / "agent-runtime"
    runtime_dir.mkdir(parents=True, exist_ok=True)
    (root / ".ait" / "config.json").write_text(json.dumps({"repo_name": repo_name}), encoding="utf-8")
    (runtime_dir / "telegram.env").write_text("\n".join(env_lines or []) + ("\n" if env_lines else ""), encoding="utf-8")
    if worker_payload is not None:
        (root / ".ait" / "agent-workers.json").write_text(json.dumps(worker_payload), encoding="utf-8")
    return root


def test_load_config_for_telegram_worker_ignores_stale_env_path(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    runtime_dir = repo_root / ".ait" / "agent-runtime"
    runtime_dir.mkdir(parents=True)
    env_path = runtime_dir / "telegram.env"
    env_path.write_text(
        "\n".join(
            [
                "AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS=inf",
                "AIT_TELEGRAM_BACKGROUND_SYNC_ENABLED=true",
                "AIT_TELEGRAM_STATE_PATH=/tmp/ignored-by-worker.json",
            ]
        ),
        encoding="utf-8",
    )
    config_path = repo_root / ".ait" / "agent-workers.json"
    config_path.write_text(
        json.dumps(
            {
                "version": 1,
                "workers": {
                    "telegram/main": {
                        "kind": "telegram",
                        "name": "main",
                        "token": "123456:worker-token",
                        "username": "ait_main_bot",
                        "sync_state_path": "runtime/telegram-sync.json",
                    }
                },
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(repo_root / "telegram-bot" / ".env"))

    config = load_config_for_telegram_worker(repo_root)

    assert config.token == "123456:worker-token"
    assert config.username == "ait_main_bot"
    assert config.env_path == env_path
    assert config.sync_state_path == repo_root / "runtime" / "telegram-sync.json"
    assert config.request_timeout_seconds is None
    assert config.background_sync_enabled is True


def test_load_config_for_telegram_worker_seeds_empty_repo_local_sync_state_from_shared_runtime_state(
    tmp_path: Path,
    monkeypatch,
):
    repo_root = _write_bound_repo_checkout(
        tmp_path / "repo",
        "ait",
        env_lines=[
            "AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS=inf",
            "AIT_TELEGRAM_BACKGROUND_SYNC_ENABLED=true",
            "AIT_TELEGRAM_STATE_PATH=/tmp/ignored-by-worker.json",
        ],
        worker_payload={
            "version": 1,
            "workers": {
                "telegram/main": {
                    "kind": "telegram",
                    "name": "main",
                    "token": "123456:worker-token",
                    "username": "ait_main_bot",
                    "sync_state_path": "runtime/telegram-sync.json",
                }
            },
        },
    )
    data_dir = tmp_path / "server-data"
    shared_state_path = data_dir / "telegram-sync.json"
    shared_store = TelegramSyncStateStore(shared_state_path)
    shared_store.upsert_chat(
        123,
        session_id="AITS-TEST-0001",
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id="AITS-TEST-0001",
        binding_role="primary_shared",
        last_synced_sequence=8,
    )
    shared_store.upsert_chat(
        456,
        session_id="AITS-TEST-9999",
        repo_name="ait_test",
        chat_type="private",
        chat_title="Other Repo",
        canonical_session_id="AITS-TEST-9999",
        binding_role="primary_shared",
        last_synced_sequence=4,
    )
    shared_store.save_bootstrap_auth(
        {
            "owner_user_id": "456",
            "owner_chat_id": "123",
            "owner_chat_title": "Wei",
            "owner_chat_type": "private",
        }
    )
    shared_store.update_last_update_id(57)
    config_path = repo_root / ".ait" / "agent-workers.json"
    env_path = repo_root / ".ait" / "agent-runtime" / "telegram.env"
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(repo_root / "telegram-bot" / ".env"))

    config = load_config_for_telegram_worker(repo_root)

    state = load_runtime_binding_state(repo_root / "runtime" / "telegram-sync.json")
    assert config.token == "123456:worker-token"
    assert config.username == "ait_main_bot"
    assert config.env_path == env_path
    assert config.sync_state_path == repo_root / "runtime" / "telegram-sync.json"
    assert config.request_timeout_seconds is None
    assert config.background_sync_enabled is True
    assert state.last_update_id == 57
    assert set(state.chats) == {"123"}
    assert state.chats["123"]["session_id"] == "AITS-TEST-0001"
    assert state.surface_bindings["telegram:123"]["repo_name"] == "ait"
    assert state.telegram_bootstrap_auth["owner_user_id"] == "456"


def _configure_server_env(tmp_path: Path, monkeypatch) -> ServerContext:
    data_dir = tmp_path / "server-data"
    install_fake_psycopg_global()
    reset_fake_postgres_runtime()
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_AUTH_MODE", "open")
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")
    ctx = ServerContext.from_env()
    initialize(ctx)
    ensure_repository(ctx, "ait", "main")
    return ctx


def test_normalize_and_parse_command():
    assert normalize_user_text("@ait_test_bot   hello   world", "ait_test_bot") == "hello world"
    assert parse_command("/status@ait_test_bot", "ait_test_bot") == ("status", "")
    assert parse_command("/status@other_bot", "ait_test_bot") is None
    assert detect_workflow_query("queue") == ("queue", None)
    assert detect_workflow_query("what needs attention") == ("attention", None)
    assert detect_workflow_query("what can land") == ("ready", None)
    assert detect_workflow_query("task aitt-0010") == ("task", "AITT-0010")
    assert detect_workflow_query("audit aitt-0010") == ("audit", "AITT-0010")
    assert detect_workflow_query("change aitc-0011") == ("change", "AITC-0011")
    assert detect_workflow_query("land aitc-0011") == ("land", "AITC-0011")


def test_normalized_turn_text_appends_music_attachment_summary():
    attachments = telegram_app._music_attachments_from_message(
        {
            "caption": "請幫我處理這首歌",
            "audio": {
                "file_id": "tg-audio-001",
                "file_unique_id": "unique-audio-001",
                "file_name": "demo-track.mp3",
                "mime_type": "audio/mpeg",
                "duration": 42,
                "file_size": 1_048_576,
                "title": "Demo Track",
                "performer": "AI Band",
            },
        }
    )

    normalized = telegram_app._normalized_turn_text(
        raw_text="@ait_test_bot   請幫我處理這首歌",
        username="ait_test_bot",
        attachments=attachments,
    )

    assert normalized.startswith("請幫我處理這首歌")
    assert "Telegram music upload:" in normalized
    assert "demo-track.mp3" in normalized
    assert "performer=AI Band" in normalized
    assert "1.0 MB" in normalized


def test_transport_reply_attachment_helpers_preserve_envelope_payload():
    assistant_event = {
        "payload": {
            "text": "fallback text",
            "transport_reply_envelope": {
                "message": {
                    "text": "reply text",
                    "attachments": [
                        {"kind": "audio", "file_name": "demo-track.mp3", "mime_type": "audio/mpeg"},
                        {"kind": "document", "file_name": "notes.txt", "mime_type": "text/plain"},
                    ],
                }
            },
        }
    }

    attachments = telegram_app._transport_reply_attachments(assistant_event)

    assert telegram_app._transport_reply_text(assistant_event) == "reply text"
    assert attachments[0]["file_name"] == "demo-track.mp3"
    assert telegram_app._attachment_should_send_as_audio(attachments[0]) is True
    assert telegram_app._attachment_should_send_as_audio(attachments[1]) is False


def test_parse_webhook_payload_accepts_object_and_array():
    assert telegram_app.parse_webhook_payload(
        '{"update_id": 1, "message": {"chat": {"id": 1}, "text": "hello"}}'
    ) == [{"update_id": 1, "message": {"chat": {"id": 1}, "text": "hello"}}]

    assert telegram_app.parse_webhook_payload('[{"update_id": 1}, {"update_id": 2}]') == [
        {"update_id": 1},
        {"update_id": 2},
    ]


def test_parse_webhook_payload_validates_shape():
    with pytest.raises(telegram_app.BotRuntimeError, match="must be a JSON object or array"):
        telegram_app.parse_webhook_payload("5")

    with pytest.raises(telegram_app.BotRuntimeError, match="item #0"):
        telegram_app.parse_webhook_payload("[1]")


def test_run_webhook_updates_routes_updates_to_service():
    service = FakeWebhookService()
    payload = '[{"update_id": 1, "message": {"chat": {"id": 2}}}, {"update_id": 2, "message": {"chat": {"id": 3}}}]'

    telegram_app.run_webhook_updates(payload, service=service)

    assert service.updates == [
        {"update_id": 1, "message": {"chat": {"id": 2}}},
        {"update_id": 2, "message": {"chat": {"id": 3}}},
    ]


def test_load_config_treats_infinite_timeouts_as_unbounded(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text(
        "\n".join(
            [
                "BOT_TOKEN=test-token",
                "AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS=inf",
                "AIT_TELEGRAM_OPENAI_TIMEOUT_SECONDS=inf",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    for name in [
        "AIT_TELEGRAM_BOT_TOKEN",
        "BOT_TOKEN",
        "AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS",
        "AIT_TELEGRAM_TIMEOUT_SECONDS",
        "AIT_TELEGRAM_OPENAI_TIMEOUT_SECONDS",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = telegram_app.load_config(repo_root)

    assert config.request_timeout_seconds is None
    assert config.openai_timeout_seconds is None

def test_load_config_reads_local_stt_settings(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text(
        "\n".join(
            [
                "BOT_TOKEN=test-token",
                "AIT_TELEGRAM_STT_MODE=local-stt",
                "AIT_TELEGRAM_STT_MODEL=mlx-community/whisper-large-v3-mlx",
                "AIT_TELEGRAM_STT_DEVICE=cpu",
                "AIT_TELEGRAM_STT_COMPUTE_TYPE=int8",
                "AIT_TELEGRAM_STT_LANGUAGE=zh",
                "AIT_TELEGRAM_STT_INCLUDE_AUDIO_UPLOADS=true",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    for name in [
        "AIT_TELEGRAM_BOT_TOKEN",
        "BOT_TOKEN",
        "AIT_TELEGRAM_STT_MODE",
        "AIT_TELEGRAM_STT_MODEL",
        "AIT_TELEGRAM_STT_DEVICE",
        "AIT_TELEGRAM_STT_COMPUTE_TYPE",
        "AIT_TELEGRAM_STT_LANGUAGE",
        "AIT_TELEGRAM_STT_INCLUDE_AUDIO_UPLOADS",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = telegram_app.load_config(repo_root)

    assert config.stt_mode == "local-stt"
    assert config.stt_model == "mlx-community/whisper-large-v3-mlx"
    assert config.stt_device == "cpu"
    assert config.stt_compute_type == "int8"
    assert config.stt_language == "zh"
    assert config.stt_include_audio_uploads is True


def test_local_speech_to_text_runtime_uses_mlx_whisper_backend(tmp_path: Path, monkeypatch):
    calls: list[dict[str, Any]] = []

    class FakeMlxWhisperModule:
        @staticmethod
        def transcribe(audio_path: str, **kwargs):
            calls.append({"audio_path": audio_path, "kwargs": dict(kwargs)})
            return {"text": "  今天先修 Telegram 語音 STT。  "}

    monkeypatch.setitem(sys.modules, "mlx_whisper", FakeMlxWhisperModule)

    runtime = telegram_app.LocalSpeechToTextRuntime(
        _config(
            tmp_path / "telegram-sync.json",
            stt_mode="local-stt",
            stt_model="mlx-community/whisper-large-v3-mlx",
            stt_language="zh",
        ),
        telegram_api=FakeTelegramApi(),
    )
    audio_path = tmp_path / "voice.ogg"
    audio_path.write_bytes(b"voice")

    transcript = runtime._transcribe_local_file(audio_path)

    assert transcript == "今天先修 Telegram 語音 STT。"
    assert calls == [
        {
            "audio_path": str(audio_path),
            "kwargs": {
                "path_or_hf_repo": "mlx-community/whisper-large-v3-mlx",
                "verbose": False,
                "language": "zh",
            },
        }
    ]



def test_load_reply_generation_config_treats_infinite_timeout_as_unbounded(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text("AIT_CHAT_OPENAI_TIMEOUT_SECONDS=inf\n", encoding="utf-8")
    for name in [
        "AIT_CHAT_OPENAI_TIMEOUT_SECONDS",
        "AIT_TELEGRAM_OPENAI_TIMEOUT_SECONDS",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.openai_timeout_seconds is None


def test_load_reply_generation_config_defaults_codex_to_housekeeper_full_access(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text("", encoding="utf-8")
    for name in [
        "AIT_CHAT_CODEX_SANDBOX",
        "AIT_TELEGRAM_CODEX_SANDBOX",
        "CODEX_SANDBOX",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.codex_sandbox == "danger-full-access"
    assert config.codex_turn_timeout_seconds is None
    assert (
        config.codex_websocket_max_size_bytes
        == codex_app_server_module.DEFAULT_CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES
    )
    assert config.codex_persistent_client is True
    assert config.codex_primary_surfaces == ("discord",)


def test_load_reply_generation_config_reads_codex_primary_surface_override(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text(
        "AIT_TELEGRAM_CODEX_PRIMARY_SURFACES=discord, editor\n",
        encoding="utf-8",
    )
    for name in [
        "AIT_CHAT_CODEX_PRIMARY_SURFACES",
        "AIT_TELEGRAM_CODEX_PRIMARY_SURFACES",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.codex_primary_surfaces == ("discord", "editor")


def test_load_reply_generation_config_reads_codex_websocket_max_size(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text(
        "AIT_TELEGRAM_CODEX_WEBSOCKET_MAX_SIZE_BYTES=33554432\n",
        encoding="utf-8",
    )
    for name in [
        "AIT_CHAT_CODEX_WEBSOCKET_MAX_SIZE_BYTES",
        "AIT_TELEGRAM_CODEX_WEBSOCKET_MAX_SIZE_BYTES",
        "AIT_CHAT_CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES",
        "AIT_TELEGRAM_CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES",
        "CODEX_APP_SERVER_WEBSOCKET_MAX_SIZE_BYTES",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.codex_websocket_max_size_bytes == 33_554_432


def test_load_reply_generation_config_reads_shared_child_reap_timeout_override(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text(
        "AIT_CHAT_CODEX_CHILD_REAP_TIMEOUT_SECONDS=30\n",
        encoding="utf-8",
    )
    for name in [
        "AIT_CHAT_CODEX_CHILD_REAP_TIMEOUT_SECONDS",
        "AIT_TELEGRAM_CODEX_CHILD_REAP_TIMEOUT_SECONDS",
        "CODEX_APP_SERVER_CHILD_REAP_TIMEOUT_MS",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.codex_child_reap_timeout_seconds == 30.0


def test_load_reply_generation_config_prefers_shared_child_reap_timeout_override(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text(
        "\n".join(
            [
                "AIT_CHAT_CODEX_CHILD_REAP_TIMEOUT_SECONDS=30",
                "AIT_TELEGRAM_CODEX_CHILD_REAP_TIMEOUT_SECONDS=7",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    for name in [
        "AIT_CHAT_CODEX_CHILD_REAP_TIMEOUT_SECONDS",
        "AIT_TELEGRAM_CODEX_CHILD_REAP_TIMEOUT_SECONDS",
        "CODEX_APP_SERVER_CHILD_REAP_TIMEOUT_MS",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.codex_child_reap_timeout_seconds == 30.0


def test_load_reply_generation_config_reads_codex_persistent_client_override(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text(
        "AIT_TELEGRAM_CODEX_PERSISTENT_CLIENT=false\n",
        encoding="utf-8",
    )
    for name in [
        "AIT_CHAT_CODEX_PERSISTENT_CLIENT",
        "AIT_TELEGRAM_CODEX_PERSISTENT_CLIENT",
        "CODEX_APP_SERVER_PERSISTENT_CLIENT",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.codex_persistent_client is False


def test_load_reply_generation_config_reads_codex_worker_pool_strategy(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text(
        "AIT_TELEGRAM_CODEX_WORKER_POOL_STRATEGY=chat\n",
        encoding="utf-8",
    )
    for name in [
        "AIT_CHAT_CODEX_WORKER_POOL_STRATEGY",
        "AIT_TELEGRAM_CODEX_WORKER_POOL_STRATEGY",
        "CODEX_WORKER_POOL_STRATEGY",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.codex_worker_pool_strategy == "chat"


def test_load_reply_generation_config_falls_back_to_session_on_invalid_worker_pool_strategy(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text(
        "AIT_TELEGRAM_CODEX_WORKER_POOL_STRATEGY=invalid\n",
        encoding="utf-8",
    )
    for name in [
        "AIT_CHAT_CODEX_WORKER_POOL_STRATEGY",
        "AIT_TELEGRAM_CODEX_WORKER_POOL_STRATEGY",
        "CODEX_WORKER_POOL_STRATEGY",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.codex_worker_pool_strategy == "session"


def test_load_reply_generation_config_reads_capacity_retry_options(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text(
        "\n".join(
            [
                "AIT_TELEGRAM_CODEX_CAPACITY_RETRY_LIMIT=2",
                "AIT_TELEGRAM_CODEX_CAPACITY_CONTINUE_TEXT=continue please",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    for name in [
        "AIT_CHAT_CODEX_CAPACITY_RETRY_LIMIT",
        "AIT_TELEGRAM_CODEX_CAPACITY_RETRY_LIMIT",
        "CODEX_CAPACITY_RETRY_LIMIT",
        "AIT_CHAT_CODEX_CAPACITY_CONTINUE_TEXT",
        "AIT_TELEGRAM_CODEX_CAPACITY_CONTINUE_TEXT",
        "CODEX_CAPACITY_CONTINUE_TEXT",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.codex_capacity_retry_limit == 2
    assert config.codex_capacity_continue_text == "continue please"


def test_load_config_ignores_placeholder_openai_api_key(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text("BOT_TOKEN=test-token\n", encoding="utf-8")
    monkeypatch.setenv("OPENAI_API_KEY", "your-openai-api-key")
    for name in [
        "AIT_TELEGRAM_OPENAI_API_KEY",
        "AIT_OPENAI_API_KEY",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = telegram_app.load_config(repo_root)

    assert config.openai_api_key is None


def test_load_reply_generation_config_ignores_placeholder_openai_api_key(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text("", encoding="utf-8")
    monkeypatch.setenv("OPENAI_API_KEY", "your-openai-api-key")
    for name in [
        "AIT_CHAT_OPENAI_API_KEY",
        "AIT_TELEGRAM_OPENAI_API_KEY",
        "AIT_OPENAI_API_KEY",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.openai_api_key is None


def test_load_reply_generation_config_parses_turn_analysis_toggle(tmp_path: Path, monkeypatch):
    repo_root = tmp_path / "repo"
    env_dir = repo_root / ".ait" / "agent-runtime"
    env_dir.mkdir(parents=True)
    (env_dir / "telegram.env").write_text("AIT_TELEGRAM_APPEND_TURN_ANALYSIS=true\n", encoding="utf-8")
    for name in [
        "AIT_CHAT_APPEND_TURN_ANALYSIS",
        "AIT_TELEGRAM_APPEND_TURN_ANALYSIS",
        "AIT_TELEGRAM_ENV_PATH",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = session_reply_module.load_reply_generation_config(repo_root=repo_root)

    assert config.telegram_append_turn_analysis is True


def test_event_message_for_ai_treats_session_message_as_user():
    event = {
        "event_type": "session.message",
        "payload": {"source": "vscode", "text": "Please check the failing tests."},
        "actor_identity": "weita",
    }

    assert session_reply_module.event_message_for_ai(event) == {
        "role": "user",
        "content": "Please check the failing tests.",
    }


def test_codex_turn_analysis_summarizes_command_churn():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc ls", "commandActions": [{"type": "listFiles", "command": "ls"}]},
            {"command": "/bin/zsh -lc pwd", "commandActions": [{"type": "unknown", "command": "pwd"}]},
            {"command": "/bin/zsh -lc ls", "commandActions": [{"type": "listFiles", "command": "ls"}]},
            {"command": "/bin/zsh -lc \"sed -n '1,20p' docs/ait_native_quickstart.md\"", "commandActions": [{"type": "readFiles", "command": "sed -n '1,20p' docs/ait_native_quickstart.md"}]},
        ]
    )

    assert analysis is not None
    assert analysis["command_count"] == 4
    assert analysis["distinct_command_count"] == 3
    assert analysis["top_commands"][0] == {"command": "ls", "count": 2}
    assert any(hint["code"] == "avoid_repeated_commands" for hint in analysis["optimization_hints"])
    assert any(hint["code"] == "batch_shell_inspection" for hint in analysis["optimization_hints"])


def test_codex_turn_analysis_flags_mergeable_help_queries():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc \"ait task --help\"", "commandActions": [{"type": "unknown", "command": "ait task --help"}]},
            {"command": "/bin/zsh -lc \"ait task start --help\"", "commandActions": [{"type": "unknown", "command": "ait task start --help"}]},
            {"command": "/bin/zsh -lc \"ait task show --help\"", "commandActions": [{"type": "unknown", "command": "ait task show --help"}]},
        ]
    )

    assert analysis is not None
    help_hint = next(hint for hint in analysis["optimization_hints"] if hint["code"] == "consolidate_help_queries")
    assert help_hint["suggested_command"] == "ait task --help"
    assert help_hint["matched_commands"] == ["ait task --help", "ait task start --help", "ait task show --help"]
    assert analysis["optimization_summary"] == "Several help commands were used in this turn."


def test_codex_turn_analysis_suggests_workflow_guide_for_land_help_burst():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc \"./.venv/bin/ait snapshot create --help\"", "commandActions": [{"type": "unknown", "command": "./.venv/bin/ait snapshot create --help"}]},
            {"command": "/bin/zsh -lc \"printf ready; .venv/bin/ait patchset publish --help\"", "commandActions": [{"type": "unknown", "command": "printf ready; .venv/bin/ait patchset publish --help"}]},
            {"command": "/bin/zsh -lc \".venv/bin/ait land submit --help\"", "commandActions": [{"type": "unknown", "command": ".venv/bin/ait land submit --help"}]},
        ]
    )

    assert analysis is not None
    help_hint = next(hint for hint in analysis["optimization_hints"] if hint["code"] == "prefer_workflow_guide")
    assert help_hint["suggested_command"] == "ait workflow guide land"
    help_cluster = next(cluster for cluster in analysis["burst_clusters"] if cluster["code"] == "help_burst")
    assert help_cluster["suggested_command"] == "ait workflow guide land"
    assert help_hint["matched_commands"] == [
        "./.venv/bin/ait snapshot create --help",
        "printf ready; .venv/bin/ait patchset publish --help",
        ".venv/bin/ait land submit --help",
    ]
    assert analysis["optimization_summary"] == "This help burst could likely start with one workflow guide."


def test_codex_turn_analysis_suggests_task_start_for_bootstrap_turn():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc \"ait task start --task-only --title 'Bootstrap native workflow' --intent 'Adopt snapshot-based review' --risk medium\"", "commandActions": [{"type": "unknown", "command": "ait task start --task-only --title 'Bootstrap native workflow' --intent 'Adopt snapshot-based review' --risk medium"}]},
            {"command": "/bin/zsh -lc \"ait change create --task AITT-0001 --title 'Bootstrap native workflow' --base-line feature/bootstrap --risk medium\"", "commandActions": [{"type": "unknown", "command": "ait change create --task AITT-0001 --title 'Bootstrap native workflow' --base-line feature/bootstrap --risk medium"}]},
        ]
    )

    assert analysis is not None
    task_start_hint = next(hint for hint in analysis["optimization_hints"] if hint["code"] == "prefer_task_start")
    assert task_start_hint["suggested_command"] == "ait task start --base-line feature/bootstrap"
    assert task_start_hint["matched_commands"] == [
        "ait task start --task-only --title 'Bootstrap native workflow' --intent 'Adopt snapshot-based review' --risk medium",
        "ait change create --task AITT-0001 --title 'Bootstrap native workflow' --base-line feature/bootstrap --risk medium",
    ]
    assert analysis["optimization_summary"] == "This task bootstrap turn could likely use `ait task start`."


def test_codex_turn_analysis_suggests_task_audit_for_task_readiness_reads():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc \"ait task show AITT-0001 --json\"", "commandActions": [{"type": "unknown", "command": "ait task show AITT-0001 --json"}]},
            {"command": "/bin/zsh -lc \"ait change list --task AITT-0001 --json\"", "commandActions": [{"type": "unknown", "command": "ait change list --task AITT-0001 --json"}]},
        ]
    )

    assert analysis is not None
    audit_hint = next(hint for hint in analysis["optimization_hints"] if hint["code"] == "prefer_task_audit")
    assert audit_hint["suggested_command"] == "ait task audit AITT-0001"
    assert audit_hint["matched_commands"] == ["ait task show AITT-0001 --json", "ait change list --task AITT-0001 --json"]
    assert all(hint["code"] != "queue_summary_for_inventory" for hint in analysis["optimization_hints"])
    assert analysis["optimization_summary"] == "This task-readiness turn could likely use `ait task audit`."


def test_codex_turn_analysis_suggests_queue_summary_for_inventory_reads():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc \"ait task list --json\"", "commandActions": [{"type": "unknown", "command": "ait task list --json"}]},
            {"command": "/bin/zsh -lc \"ait change list --json\"", "commandActions": [{"type": "unknown", "command": "ait change list --json"}]},
            {"command": "/bin/zsh -lc \"ait task show AITT-0001 --json\"", "commandActions": [{"type": "unknown", "command": "ait task show AITT-0001 --json"}]},
        ]
    )

    assert analysis is not None
    queue_hint = next(hint for hint in analysis["optimization_hints"] if hint["code"] == "queue_summary_for_inventory")
    assert queue_hint["suggested_command"] == "ait queue summary --all-changes"
    inventory_cluster = next(cluster for cluster in analysis["burst_clusters"] if cluster["code"] == "inventory_burst")
    assert inventory_cluster["suggested_command"] == "ait queue summary --all-changes"
    assert queue_hint["matched_commands"] == ["ait task list --json", "ait change list --json", "ait task show AITT-0001 --json"]
    assert analysis["optimization_summary"] == "This workflow inventory turn could likely start with one queue summary command."


def test_codex_turn_analysis_flags_duplicate_inventory_reads_from_wrapped_commands():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc \"printf ready; .venv/bin/ait queue summary --all-changes --json\"", "commandActions": [{"type": "unknown", "command": "printf ready; .venv/bin/ait queue summary --all-changes --json"}]},
            {"command": "/bin/zsh -lc \"./.venv/bin/ait queue summary --all-changes --json\"", "commandActions": [{"type": "unknown", "command": "./.venv/bin/ait queue summary --all-changes --json"}]},
        ]
    )

    assert analysis is not None
    duplicate_hint = next(hint for hint in analysis["optimization_hints"] if hint["code"] == "duplicate_inventory_reads")
    inventory_cluster = next(cluster for cluster in analysis["burst_clusters"] if cluster["code"] == "inventory_burst")
    assert inventory_cluster["summary"] == "This turn reran the same workflow inventory command."
    assert duplicate_hint["matched_commands"] == [
        "printf ready; .venv/bin/ait queue summary --all-changes --json",
        "./.venv/bin/ait queue summary --all-changes --json",
    ]
    assert analysis["optimization_summary"] == "The same workflow inventory command was rerun in this turn."


def test_codex_turn_analysis_flags_duplicate_inventory_reads_from_env_prefixed_and_shell_control_commands():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc \"PYTHONPATH=src:. .venv/bin/ait queue summary --all-changes --json\"", "commandActions": [{"type": "unknown", "command": "PYTHONPATH=src:. .venv/bin/ait queue summary --all-changes --json"}]},
            {"command": "/bin/zsh -lc \"if [ -x .venv/bin/ait ]; then timeout 15 .venv/bin/ait queue summary --all-changes --json; fi\"", "commandActions": [{"type": "unknown", "command": "if [ -x .venv/bin/ait ]; then timeout 15 .venv/bin/ait queue summary --all-changes --json; fi"}]},
        ]
    )

    assert analysis is not None
    duplicate_hint = next(hint for hint in analysis["optimization_hints"] if hint["code"] == "duplicate_inventory_reads")
    inventory_cluster = next(cluster for cluster in analysis["burst_clusters"] if cluster["code"] == "inventory_burst")
    assert duplicate_hint["matched_commands"] == [
        "PYTHONPATH=src:. .venv/bin/ait queue summary --all-changes --json",
        "if [ -x .venv/bin/ait ]; then timeout 15 .venv/bin/ait queue summary --all-changes --json; fi",
    ]
    assert inventory_cluster["summary"] == "This turn reran the same workflow inventory command."
    assert analysis["optimization_summary"] == "The same workflow inventory command was rerun in this turn."


def test_codex_turn_analysis_flags_duplicate_inventory_reads_from_env_prefixed_python_module_invocations():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc \"PYTHONPATH=src:. .venv/bin/python -m ait.cli queue summary --all-changes --json\"", "commandActions": [{"type": "unknown", "command": "PYTHONPATH=src:. .venv/bin/python -m ait.cli queue summary --all-changes --json"}]},
            {"command": "/bin/zsh -lc \"env PYTHONPATH=src:. timeout 15 .venv/bin/python -m ait.cli queue summary --all-changes --json\"", "commandActions": [{"type": "unknown", "command": "env PYTHONPATH=src:. timeout 15 .venv/bin/python -m ait.cli queue summary --all-changes --json"}]},
        ]
    )

    assert analysis is not None
    duplicate_hint = next(hint for hint in analysis["optimization_hints"] if hint["code"] == "duplicate_inventory_reads")
    inventory_cluster = next(cluster for cluster in analysis["burst_clusters"] if cluster["code"] == "inventory_burst")
    assert duplicate_hint["matched_commands"] == [
        "PYTHONPATH=src:. .venv/bin/python -m ait.cli queue summary --all-changes --json",
        "env PYTHONPATH=src:. timeout 15 .venv/bin/python -m ait.cli queue summary --all-changes --json",
    ]
    assert inventory_cluster["summary"] == "This turn reran the same workflow inventory command."
    assert analysis["optimization_summary"] == "The same workflow inventory command was rerun in this turn."


def test_codex_turn_analysis_suggests_workflow_land_for_land_burst():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc \"ait patchset publish --change AITC-0001 --summary 'review summary'\"", "commandActions": [{"type": "unknown", "command": "ait patchset publish --change AITC-0001 --summary 'review summary'"}]},
            {"command": "/bin/zsh -lc \"ait attest put AITP-0001-1 --tests pass\"", "commandActions": [{"type": "unknown", "command": "ait attest put AITP-0001-1 --tests pass"}]},
            {"command": "/bin/zsh -lc \"ait review task approve AITC-0001 --patchset AITP-0001-1\"", "commandActions": [{"type": "unknown", "command": "ait review task approve AITC-0001 --patchset AITP-0001-1"}]},
            {"command": "/bin/zsh -lc \"ait policy eval AITP-0001-1\"", "commandActions": [{"type": "unknown", "command": "ait policy eval AITP-0001-1"}]},
            {"command": "/bin/zsh -lc \"ait land submit AITC-0001 --patchset AITP-0001-1 --target main --mode direct\"", "commandActions": [{"type": "unknown", "command": "ait land submit AITC-0001 --patchset AITP-0001-1 --target main --mode direct"}]},
        ]
    )

    assert analysis is not None
    land_hint = next(hint for hint in analysis["optimization_hints"] if hint["code"] == "prefer_workflow_land")
    assert land_hint["suggested_command"] == "ait workflow land AITC-0001"
    land_cluster = next(cluster for cluster in analysis["burst_clusters"] if cluster["code"] == "land_workflow_burst")
    assert land_cluster["suggested_command"] == "ait workflow land AITC-0001"
    assert land_cluster["top_levels"] == ["attest", "land", "patchset", "policy", "review"]
    assert analysis["optimization_summary"] == "This land workflow turn could likely start with one workflow land helper."


def test_codex_workflow_land_suggested_command_uses_change_placeholder_when_only_patchset_is_known():
    assert codex_reply_module._workflow_land_suggested_command(None, "AITP-0001-1") == "ait workflow land <change-id>"


def test_codex_turn_analysis_flags_repeated_file_reads():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc \"sed -n '1,20p' docs/ait_native_quickstart.md\"", "commandActions": [{"type": "readFiles", "command": "sed -n '1,20p' docs/ait_native_quickstart.md"}]},
            {"command": "/bin/zsh -lc \"head -n 5 docs/ait_native_quickstart.md\"", "commandActions": [{"type": "readFiles", "command": "head -n 5 docs/ait_native_quickstart.md"}]},
            {"command": "/bin/zsh -lc \"cat src/ait_chat/codex_reply.py\"", "commandActions": [{"type": "readFiles", "command": "cat src/ait_chat/codex_reply.py"}]},
        ]
    )

    assert analysis is not None
    read_hint = next(hint for hint in analysis["optimization_hints"] if hint["code"] == "reuse_file_read")
    assert read_hint["target"] == "docs/ait_native_quickstart.md"
    assert read_hint["matched_commands"] == ["sed -n '1,20p' docs/ait_native_quickstart.md", "head -n 5 docs/ait_native_quickstart.md"]
    assert analysis["optimization_summary"] == "The same file was inspected multiple times."


def test_codex_turn_analysis_flags_mergeable_inspection_chain():
    analysis = codex_reply_module._turn_analysis_for_commands(
        [
            {"command": "/bin/zsh -lc pwd", "commandActions": [{"type": "unknown", "command": "pwd"}]},
            {"command": "/bin/zsh -lc \"ls docs\"", "commandActions": [{"type": "listFiles", "command": "ls docs"}]},
            {"command": "/bin/zsh -lc \"sed -n '1,20p' docs/ait_native_quickstart.md\"", "commandActions": [{"type": "readFiles", "command": "sed -n '1,20p' docs/ait_native_quickstart.md"}]},
        ]
    )

    assert analysis is not None
    merge_hint = next(hint for hint in analysis["optimization_hints"] if hint["code"] == "merge_inspection_commands")
    assert merge_hint["suggested_command"] == "pwd && ls docs && sed -n '1,20p' docs/ait_native_quickstart.md"
    assert merge_hint["matched_commands"] == ["pwd", "ls docs", "sed -n '1,20p' docs/ait_native_quickstart.md"]
    assert analysis["optimization_summary"] == "Several read-only shell probes could have been merged into one command."


def test_codex_base_instructions_prompt_for_fewer_shell_hops():
    instructions = codex_reply_module.codex_base_instructions(Path("/tmp/repo"))

    assert "prefer one combined shell invocation" in instructions
    assert "prefer `ait queue summary`" in instructions
    assert "prefer `ait task audit <task-id>`" in instructions
    assert "prefer `ait workflow land <change-id>`" in instructions
    assert "prefer `ait task start`" in instructions
    assert "prefer `ait workflow guide <topic>`" in instructions


def test_codex_base_instructions_use_packet_bootstrap_for_task_dag_surface():
    instructions = codex_reply_module.codex_base_instructions(
        Path("/tmp/repo"),
        surface="task_dag_compact_packet",
    )

    assert "worker-only compact DAG packet session" in instructions
    assert "Start from the packet manifest path supplied in the session context" in instructions
    assert "Do not begin with repo-root governance discovery" in instructions
    assert "raw git status/diff/log probes" in instructions
    assert "docs/plan.md" not in instructions


def test_telegram_json_request_wraps_timeout_error(monkeypatch):
    def fake_urlopen(request, timeout=None):
        raise TimeoutError("boom")

    monkeypatch.setattr(telegram_app, "urlopen", fake_urlopen)

    with pytest.raises(telegram_app.BotRuntimeError, match=r"timed out after 7 seconds"):
        telegram_app._json_request("http://example.test/telegram", timeout=7.0)


def test_telegram_api_client_send_audio_uses_multipart_for_local_file(tmp_path: Path, monkeypatch):
    captured: dict[str, Any] = {}

    def fake_multipart(url, **kwargs):
        captured["url"] = url
        captured.update(kwargs)
        return {"ok": True}

    monkeypatch.setattr(telegram_app, "_multipart_json_request", fake_multipart)

    track_path = tmp_path / "demo.mp3"
    track_path.write_bytes(b"demo-track")
    client = telegram_app.TelegramApiClient(_config(tmp_path / "telegram-sync.json"))

    client.send_audio(
        123,
        {
            "kind": "audio",
            "file_name": "demo.mp3",
            "mime_type": "audio/mpeg",
            "local_path": str(track_path),
            "caption": "demo caption",
            "title": "Demo",
            "performer": "AI",
            "duration_seconds": 42,
        },
    )

    assert captured["url"].endswith("/sendAudio")
    assert captured["fields"] == {
        "chat_id": 123,
        "caption": "demo caption",
        "title": "Demo",
        "performer": "AI",
        "duration": 42,
    }
    assert captured["file_field"] == "audio"
    assert captured["file_name"] == "demo.mp3"
    assert captured["file_bytes"] == b"demo-track"
    assert captured["mime_type"] == "audio/mpeg"


def test_telegram_api_client_send_document_accepts_telegram_file_id(tmp_path: Path, monkeypatch):
    captured: dict[str, Any] = {}

    def fake_json(url, *, method="GET", payload=None, headers=None, timeout=None):
        captured["url"] = url
        captured["method"] = method
        captured["payload"] = payload
        return {"ok": True}

    monkeypatch.setattr(telegram_app, "_json_request", fake_json)

    client = telegram_app.TelegramApiClient(_config(tmp_path / "telegram-sync.json"))
    client.send_document(
        123,
        {
            "kind": "document",
            "telegram_file_id": "tg-doc-123",
            "file_name": "song.flac",
            "mime_type": "audio/flac",
        },
    )

    assert captured["url"].endswith("/sendDocument")
    assert captured["method"] == "POST"
    assert captured["payload"] == {"chat_id": 123, "document": "tg-doc-123"}


def test_session_reply_json_request_wraps_timeout_error(monkeypatch):
    def fake_urlopen(request, timeout=None):
        raise TimeoutError("boom")

    monkeypatch.setattr(session_reply_module, "urlopen", fake_urlopen)

    with pytest.raises(ReplyGenerationError, match=r"timed out after 11 seconds"):
        session_reply_module._json_request("https://api.openai.com/v1/responses", method="POST", timeout=11.0)


def test_state_store_round_trip(tmp_path: Path):
    store = TelegramSyncStateStore(tmp_path / "telegram-sync.json")
    initial = store.load()
    assert initial.last_update_id == 0
    assert initial.chats == {}
    assert initial.bootstrap_auth == {}

    chat = store.upsert_chat(123, session_id="AITS-0001", repo_name="ait", chat_title="Demo chat", last_synced_sequence=4)
    assert chat["session_id"] == "AITS-0001"
    assert store.get_chat(123)["last_synced_sequence"] == 4

    store.update_last_update_id(99)
    assert store.load().last_update_id == 99
    assert store.linkage_by_session()["AITS-0001"]["chat_title"] == "Demo chat"

    patched = store.patch_chat(123, workflow_notifications_enabled=True, last_queue_summary_digest='{"actionable": true}')
    assert patched["workflow_notifications_enabled"] is True
    assert store.get_chat(123)["last_queue_summary_digest"] == '{"actionable": true}'

    store.save_bootstrap_auth({"owner_user_id": "456", "blacklist": {"789": {"attempt_count": 3}}})
    assert store.get_bootstrap_auth()["owner_user_id"] == "456"

    store.patch_chat(123, workflow_notifications_enabled=False)
    assert store.get_bootstrap_auth()["blacklist"]["789"]["attempt_count"] == 3


def test_load_config_parses_background_sync_and_merge_settings(tmp_path: Path, monkeypatch):
    from ait.store import init_repo

    init_repo(tmp_path, "ait", "main")
    env_path = tmp_path / "telegram.env"
    env_path.write_text(
        "\n".join(
            [
                "BOT_TOKEN=test-token",
                "AIT_TELEGRAM_BACKGROUND_SYNC_ENABLED=true",
                "AIT_TELEGRAM_BACKGROUND_SYNC_INTERVAL_SECONDS=12",
                "AIT_TELEGRAM_GRAPH_WATCH_BACKGROUND_SWEEP_ENABLED=true",
                "AIT_TELEGRAM_TURN_MERGE_WINDOW_SECONDS=0.2",
                "AIT_TELEGRAM_TURN_MERGE_MAX_MESSAGES=5",
                "AIT_TELEGRAM_DECOUPLED_REPLY_ENABLED=false",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(env_path))
    for name in [
        "AIT_TELEGRAM_BACKGROUND_SYNC_ENABLED",
        "AIT_TELEGRAM_BACKGROUND_SYNC_INTERVAL_SECONDS",
        "AIT_TELEGRAM_GRAPH_WATCH_BACKGROUND_SWEEP_ENABLED",
        "AIT_TELEGRAM_TURN_MERGE_WINDOW_SECONDS",
        "AIT_TELEGRAM_TURN_MERGE_MAX_MESSAGES",
        "AIT_TELEGRAM_DECOUPLED_REPLY_ENABLED",
        "AIT_TELEGRAM_BOT_TOKEN",
        "BOT_TOKEN",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = load_config(tmp_path)

    assert config.background_sync_enabled is True
    assert config.background_sync_interval_seconds == 12.0
    assert config.graph_watch_background_sweep_enabled is True
    assert config.turn_merge_window_seconds == 0.2
    assert config.turn_merge_max_messages == 5
    assert config.decoupled_reply_enabled is False


def test_load_config_defaults_reply_markdown_on(tmp_path: Path, monkeypatch):
    from ait.store import init_repo

    init_repo(tmp_path, "ait", "main")
    env_path = tmp_path / "telegram.env"
    env_path.write_text("BOT_TOKEN=test-token\n", encoding="utf-8")
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(env_path))
    for name in [
        "AIT_TELEGRAM_REPLY_MARKDOWN_ENABLED",
        "AIT_TELEGRAM_MARKDOWN_ENABLED",
        "AIT_TELEGRAM_BOT_TOKEN",
        "BOT_TOKEN",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = load_config(tmp_path)

    assert config.reply_markdown_enabled is True


def test_load_config_allows_reply_markdown_opt_out(tmp_path: Path, monkeypatch):
    from ait.store import init_repo

    init_repo(tmp_path, "ait", "main")
    env_path = tmp_path / "telegram.env"
    env_path.write_text(
        "BOT_TOKEN=test-token\nAIT_TELEGRAM_REPLY_MARKDOWN_ENABLED=false\n",
        encoding="utf-8",
    )
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(env_path))
    for name in [
        "AIT_TELEGRAM_REPLY_MARKDOWN_ENABLED",
        "AIT_TELEGRAM_MARKDOWN_ENABLED",
        "AIT_TELEGRAM_BOT_TOKEN",
        "BOT_TOKEN",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = load_config(tmp_path)

    assert config.reply_markdown_enabled is False


def test_load_config_defaults_owner_bootstrap_auth_on(tmp_path: Path, monkeypatch):
    from ait.store import init_repo

    init_repo(tmp_path, "ait", "main")
    env_path = tmp_path / "telegram.env"
    env_path.write_text("BOT_TOKEN=test-token\n", encoding="utf-8")
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(env_path))
    for name in [
        "AIT_TELEGRAM_OWNER_BOOTSTRAP_ENABLED",
        "AIT_TELEGRAM_BOT_TOKEN",
        "BOT_TOKEN",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = load_config(tmp_path)

    assert config.owner_bootstrap_enabled is True


def test_load_config_allows_owner_bootstrap_auth_opt_out(tmp_path: Path, monkeypatch):
    from ait.store import init_repo

    init_repo(tmp_path, "ait", "main")
    env_path = tmp_path / "telegram.env"
    env_path.write_text(
        "BOT_TOKEN=test-token\nAIT_TELEGRAM_OWNER_BOOTSTRAP_ENABLED=false\n",
        encoding="utf-8",
    )
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(env_path))
    for name in [
        "AIT_TELEGRAM_OWNER_BOOTSTRAP_ENABLED",
        "AIT_TELEGRAM_BOT_TOKEN",
        "BOT_TOKEN",
    ]:
        monkeypatch.delenv(name, raising=False)

    config = load_config(tmp_path)

    assert config.owner_bootstrap_enabled is False


def test_generate_session_reply_prefers_checkpoint_plus_delta_messages(monkeypatch):
    captured: dict[str, dict] = {}

    def fake_request(url: str, *, method: str = "GET", payload: dict | None = None, headers=None, timeout: float = 20.0):
        captured["payload"] = payload or {}
        return {
            "id": "resp_test_checkpoint",
            "model": "gpt-5.4-mini",
            "usage": {"input_tokens": 21, "output_tokens": 7, "total_tokens": 28},
            "output": [
                {
                    "type": "message",
                    "content": [{"type": "output_text", "text": "Checkpoint-aware answer."}],
                }
            ],
        }

    monkeypatch.setattr(session_reply_module, "_json_request", fake_request)

    checkpoint = {
        "checkpoint_id": "AITK-TEST-0001",
        "based_on_sequence": 2,
        "summary": "Checkpoint summary for Telegram session.",
        "resume_payload": {
            "objective": "Ship the feature",
            "context": {"phase": "review"},
            "latest_turn_analysis": {
                "command_count": 4,
                "optimization_summary": "This workflow inventory turn could likely start with one queue summary command.",
                "suggested_commands": ["ait queue summary --all-changes"],
            },
        },
    }
    events = [
        {
            "sequence": 1,
            "event_type": "telegram.user_message",
            "payload": {"source": "telegram", "text": "old request"},
            "actor_identity": "telegram:123:@weita",
        },
        {
            "sequence": 2,
            "event_type": "assistant.reply",
            "payload": {"source": "openai", "text": "old answer"},
            "actor_identity": "ait-server",
        },
        {
            "sequence": 3,
            "event_type": "web.note",
            "payload": {"source": "web", "text": "Policy is still pending."},
            "actor_identity": "alice@example.com",
        },
        {
            "sequence": 4,
            "event_type": "telegram.user_message",
            "payload": {"source": "telegram", "text": "new request"},
            "actor_identity": "telegram:123:@weita",
        },
    ]

    result = generate_session_reply(
        _reply_config(history_limit=4),
        session={"session_id": "AITS-TEST-0001", "title": "Telegram chat · Wei"},
        events=events,
        chat_id="123",
        chat_title="Wei",
        checkpoint=checkpoint,
    )

    payload = captured["payload"]
    assert result.text == "Checkpoint-aware answer."
    assert payload["metadata"]["checkpoint_id"] == "AITK-TEST-0001"
    assert payload["metadata"]["context_mode"] == "checkpoint_delta"
    assert payload["input"][0]["content"].startswith("[durable checkpoint context]")
    assert "Checkpoint summary for Telegram session." in payload["input"][0]["content"]
    assert "planning_ledger:" in payload["input"][0]["content"]
    assert "Objective: Ship the feature" in payload["input"][0]["content"]
    assert "latest_turn_guidance:" in payload["input"][0]["content"]
    assert "last turn ran 4 commands" in payload["input"][0]["content"]
    assert "`ait queue summary --all-changes`" in payload["input"][0]["content"]
    assert [message["content"] for message in payload["input"][1:]] == [
        "[web note from alice@example.com] Policy is still pending.",
        "new request",
    ]


def test_generate_session_reply_falls_back_to_codex_when_openai_key_is_missing(monkeypatch):
    captured: dict[str, Any] = {}

    def fake_codex_reply(config, *, session, messages, chat_id, chat_title, assistant_instructions, surface="telegram", actor_identity=None):
        captured["session"] = session
        captured["messages"] = messages
        captured["chat_id"] = chat_id
        captured["chat_title"] = chat_title
        captured["assistant_instructions"] = assistant_instructions
        captured["surface"] = surface
        captured["actor_identity"] = actor_identity
        return AiReplyResult(
            text="Codex-backed answer.",
            model="gpt-5.4",
            response_id="turn_codex_123",
            source="codex",
        )

    monkeypatch.setattr(session_reply_module, "generate_codex_session_reply", fake_codex_reply)

    result = generate_session_reply(
        _reply_config(openai_api_key=None),
        session={"session_id": "AITS-TEST-0002", "title": "Telegram chat · Wei"},
        events=[
            {
                "sequence": 1,
                "event_type": "telegram.user_message",
                "payload": {"source": "telegram", "text": "Please restore the Codex websocket path."},
                "actor_identity": "telegram:123:@weita",
            }
        ],
        chat_id="123",
        chat_title="Wei",
        actor_identity="telegram:123:@weita",
    )

    assert result.text == "Codex-backed answer."
    assert result.source == "codex"
    assert captured["chat_id"] == "123"
    assert captured["chat_title"] == "Wei"
    assert captured["surface"] == "telegram"
    assert captured["actor_identity"] == "telegram:123:@weita"
    assert captured["messages"] == [{"role": "user", "content": "Please restore the Codex websocket path."}]
    assert "Telegram-linked shared session" in captured["assistant_instructions"]
    assert "do the work directly" in captured["assistant_instructions"]


def test_generate_session_reply_prefers_codex_for_discord_when_openai_key_exists(monkeypatch):
    captured: dict[str, Any] = {}

    def fake_codex_reply(config, *, session, messages, chat_id, chat_title, assistant_instructions, surface="telegram", actor_identity=None):
        captured["surface"] = surface
        captured["chat_id"] = chat_id
        captured["messages"] = messages
        return AiReplyResult(
            text="Discord Codex answer.",
            model="gpt-5.4",
            response_id="turn_codex_discord",
            source="codex",
        )

    def fail_openai(*args, **kwargs):
        raise AssertionError("Discord Codex-primary routing should not hit OpenAI when Codex succeeds.")

    monkeypatch.setattr(session_reply_module, "generate_codex_session_reply", fake_codex_reply)
    monkeypatch.setattr(session_reply_module, "_json_request", fail_openai)

    result = generate_session_reply(
        _reply_config(openai_api_key="test-openai-key"),
        session={"session_id": "AITS-TEST-DISCORD-0001", "title": "Discord · Wei"},
        events=[
            {
                "sequence": 1,
                "event_type": "session.message",
                "payload": {"source": "discord", "text": "Use Codex first on Discord."},
                "actor_identity": "discord:123",
            }
        ],
        chat_id="discord-channel-123",
        chat_title="Wei",
        surface="discord",
        actor_identity="discord:123",
    )

    assert result.text == "Discord Codex answer."
    assert result.source == "codex"
    assert captured["surface"] == "discord"
    assert captured["chat_id"] == "discord-channel-123"
    assert captured["messages"] == [{"role": "user", "content": "Use Codex first on Discord."}]


def test_generate_session_reply_falls_back_to_openai_after_discord_codex_failure(monkeypatch):
    captured: dict[str, Any] = {}

    def fail_codex(*args, **kwargs):
        raise RuntimeError("Codex app-server exited")

    def fake_openai_request(url: str, *, method: str = "GET", payload: dict | None = None, headers=None, timeout: float = 20.0):
        captured["payload"] = payload or {}
        return {
            "id": "resp_discord_openai_fallback",
            "model": "gpt-5.4",
            "usage": {"input_tokens": 11, "output_tokens": 5, "total_tokens": 16},
            "output": [
                {
                    "type": "message",
                    "content": [{"type": "output_text", "text": "Discord OpenAI fallback answer."}],
                }
            ],
        }

    monkeypatch.setattr(session_reply_module, "generate_codex_session_reply", fail_codex)
    monkeypatch.setattr(session_reply_module, "_json_request", fake_openai_request)

    result = generate_session_reply(
        _reply_config(openai_api_key="test-openai-key", openai_model="gpt-5.4"),
        session={"session_id": "AITS-TEST-DISCORD-0002", "title": "Discord · Wei"},
        events=[
            {
                "sequence": 1,
                "event_type": "session.message",
                "payload": {"source": "discord", "text": "Still answer if Codex is down."},
                "actor_identity": "discord:123",
            }
        ],
        chat_id="discord-channel-123",
        chat_title="Wei",
        surface="discord",
        actor_identity="discord:123",
    )

    assert result.text == "Discord OpenAI fallback answer."
    assert result.source == "openai"
    assert captured["payload"]["metadata"]["surface"] == "discord"


def test_codex_worker_pool_key_resolution_is_deterministic():
    session = {"session_id": "S-SESSION"}

    assert (
        codex_reply_module._codex_worker_pool_key(
            strategy="session",
            session=session,
            surface="telegram",
            chat_id="123",
            actor_identity="telegram:1",
        )
        == "session:S-SESSION"
    )
    assert (
        codex_reply_module._codex_worker_pool_key(
            strategy="bot",
            session=session,
            surface="telegram",
            chat_id="123",
            actor_identity="telegram:1",
        )
        == "bot:telegram:1"
    )
    assert (
        codex_reply_module._codex_worker_pool_key(
            strategy="chat",
            session=session,
            surface="telegram",
            chat_id="123",
            actor_identity=None,
        )
        == "chat:123"
    )
    assert (
        codex_reply_module._codex_worker_pool_key(
            strategy="chat",
            session={"session_id": ""},
            surface="telegram",
            chat_id="",
            actor_identity="ignored",
        )
        == "surface:telegram:anonymous"
    )
    assert (
        codex_reply_module._codex_worker_pool_key(
            strategy="invalid",
            session=session,
            surface="telegram",
            chat_id="999",
            actor_identity=None,
        )
        == "session:S-SESSION"
    )


def test_generate_codex_session_reply_routes_persistent_clients_by_bot(monkeypatch, tmp_path: Path):
    codex_reply_module._reset_persistent_codex_clients_for_tests()

    class FakeCodexClient:
        instances = 0
        starts = 0
        run_turn_calls: list[tuple[str | int | None, str]] = []

        def __init__(self, config):
            FakeCodexClient.instances += 1
            self.config = config

        def start(self):
            FakeCodexClient.starts += 1

        def close(self):
            return None

        def start_thread(self, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            return {"id": f"thread-{FakeCodexClient.instances + FakeCodexClient.starts}"}

        def resume_thread(self, thread_id, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            return {"id": thread_id}

        def run_turn(self, *, thread_id, input_text, trace_context):
            FakeCodexClient.run_turn_calls.append((thread_id, trace_context.get("retry_attempt")))
            return SimpleNamespace(
                text=f"reply {len(FakeCodexClient.run_turn_calls)}",
                turn_id=f"turn-{len(FakeCodexClient.run_turn_calls)}",
                usage=None,
                command_executions=(),
            )

    monkeypatch.setattr(codex_reply_module, "CodexAppServerClient", FakeCodexClient)

    config = _reply_config(openai_api_key=None, repo_root=tmp_path, codex_worker_pool_strategy="bot")
    first = codex_reply_module.generate_codex_session_reply(
        config,
        session={"session_id": "S-BOT"},
        messages=[{"role": "user", "content": "first"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
        actor_identity="bot-A",
    )
    second = codex_reply_module.generate_codex_session_reply(
        config,
        session={"session_id": "S-BOT"},
        messages=[{"role": "user", "content": "second"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
        actor_identity="bot-B",
    )
    third = codex_reply_module.generate_codex_session_reply(
        config,
        session={"session_id": "S-BOT"},
        messages=[{"role": "user", "content": "third"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
        actor_identity="bot-A",
    )

    assert first.text == "reply 1"
    assert second.text == "reply 2"
    assert third.text == "reply 3"
    assert FakeCodexClient.instances == 2
    assert FakeCodexClient.starts == 2
    assert FakeCodexClient.run_turn_calls[0][0] == FakeCodexClient.run_turn_calls[2][0]
    assert FakeCodexClient.run_turn_calls[0][0] != FakeCodexClient.run_turn_calls[1][0]


def test_generate_codex_session_reply_routes_persistent_clients_by_chat(monkeypatch, tmp_path: Path):
    codex_reply_module._reset_persistent_codex_clients_for_tests()

    class FakeCodexClient:
        instances = 0
        starts = 0
        start_threads = 0
        run_thread_ids: list[str] = []

        def __init__(self, config):
            FakeCodexClient.instances += 1
            self.config = config

        def start(self):
            FakeCodexClient.starts += 1

        def close(self):
            return None

        def start_thread(self, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            FakeCodexClient.start_threads += 1
            return {"id": f"thread-{FakeCodexClient.start_threads}"}

        def resume_thread(self, thread_id, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            return {"id": thread_id}

        def run_turn(self, *, thread_id, input_text, trace_context):
            FakeCodexClient.run_thread_ids.append(thread_id)
            return SimpleNamespace(
                text=f"reply {len(FakeCodexClient.run_thread_ids)}",
                turn_id=f"turn-{len(FakeCodexClient.run_thread_ids)}",
                usage=None,
                command_executions=(),
            )

    monkeypatch.setattr(codex_reply_module, "CodexAppServerClient", FakeCodexClient)

    config = _reply_config(openai_api_key=None, repo_root=tmp_path, codex_worker_pool_strategy="chat")
    first = codex_reply_module.generate_codex_session_reply(
        config,
        session={"session_id": "S-CHAT-A"},
        messages=[{"role": "user", "content": "first"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
    )
    second = codex_reply_module.generate_codex_session_reply(
        config,
        session={"session_id": "S-CHAT-B"},
        messages=[{"role": "user", "content": "second"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
    )
    third = codex_reply_module.generate_codex_session_reply(
        config,
        session={"session_id": "S-CHAT-C"},
        messages=[{"role": "user", "content": "third"}],
        chat_id="456",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
    )

    assert first.text == "reply 1"
    assert second.text == "reply 2"
    assert third.text == "reply 3"
    assert FakeCodexClient.instances == 2
    assert FakeCodexClient.starts == 2
    assert FakeCodexClient.start_threads == 3
    assert FakeCodexClient.run_thread_ids[0] != FakeCodexClient.run_thread_ids[1]
    assert FakeCodexClient.run_thread_ids[0] != FakeCodexClient.run_thread_ids[2]


def test_generate_codex_session_reply_retries_once_when_app_server_connection_closes(monkeypatch, tmp_path: Path):
    codex_reply_module._reset_persistent_codex_clients_for_tests()

    class FakeCodexClient:
        attempts = 0
        developer_instructions: list[str] = []

        def __init__(self, config):
            self.config = config
            self.is_started = False

        def start(self):
            self.is_started = True

        def close(self):
            self.is_started = False

        def start_thread(self, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            self.developer_instructions.append(developer_instructions)
            return {"id": f"thread-{len(self.developer_instructions)}"}

        def resume_thread(self, thread_id, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            self.developer_instructions.append(developer_instructions)
            return {"id": thread_id}

        def run_turn(self, *, thread_id, input_text, trace_context):
            self.__class__.attempts += 1
            if self.__class__.attempts == 1:
                raise codex_reply_module.CodexAppServerError("Codex app-server connection closed.")
            return SimpleNamespace(
                text="Recovered after reconnect.",
                turn_id="turn-retry-2",
                usage=None,
                command_executions=(),
            )

    monkeypatch.setattr(codex_reply_module, "CodexAppServerClient", FakeCodexClient)

    result = codex_reply_module.generate_codex_session_reply(
        _reply_config(openai_api_key=None, repo_root=tmp_path),
        session={"session_id": "S-RETRY", "title": "Telegram chat · Wei"},
        messages=[{"role": "user", "content": "please continue"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
    )

    assert result.text == "Recovered after reconnect."
    assert result.response_id == "turn-retry-2"
    assert FakeCodexClient.attempts == 2
    assert len(FakeCodexClient.developer_instructions) == 2
    assert "Retry safety note" not in FakeCodexClient.developer_instructions[0]
    assert "Retry safety note" in FakeCodexClient.developer_instructions[1]
    assert "Do not blindly repeat non-idempotent actions" in FakeCodexClient.developer_instructions[1]


def test_generate_codex_session_reply_does_not_retry_non_connection_errors(monkeypatch, tmp_path: Path):
    codex_reply_module._reset_persistent_codex_clients_for_tests()

    class FakeCodexClient:
        attempts = 0

        def __init__(self, config):
            self.config = config
            self.is_started = False

        def start(self):
            self.is_started = True

        def close(self):
            self.is_started = False

        def start_thread(self, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            return {"id": "thread-1"}

        def run_turn(self, *, thread_id, input_text, trace_context):
            self.__class__.attempts += 1
            raise codex_reply_module.CodexAppServerError("Codex turn failed.")

    monkeypatch.setattr(codex_reply_module, "CodexAppServerClient", FakeCodexClient)

    with pytest.raises(RuntimeError, match="Codex turn failed"):
        codex_reply_module.generate_codex_session_reply(
            _reply_config(openai_api_key=None, repo_root=tmp_path),
            session={"session_id": "S-FAIL", "title": "Telegram chat · Wei"},
            messages=[{"role": "user", "content": "please continue"}],
            chat_id="123",
            chat_title="Wei",
            assistant_instructions="Use the shared session.",
        )

    assert FakeCodexClient.attempts == 1


def test_generate_codex_session_reply_retries_capacity_with_continue_prompt(monkeypatch, tmp_path: Path):
    codex_reply_module._reset_persistent_codex_clients_for_tests()

    class FakeCodexClient:
        attempts = 0
        input_texts: list[str] = []

        def __init__(self, config):
            self.config = config
            self.is_started = False

        def start(self):
            self.is_started = True

        def close(self):
            self.is_started = False

        def start_thread(self, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            return {"id": "thread-capacity"}

        def run_turn(self, *, thread_id, input_text, trace_context):
            self.__class__.attempts += 1
            self.__class__.input_texts.append(input_text)
            if self.__class__.attempts == 1:
                raise codex_reply_module.CodexAppServerError(
                    "Selected model is at capacity. Please try a different model."
                )
            return SimpleNamespace(
                text="Recovered after capacity retry.",
                turn_id="turn-capacity-2",
                usage=None,
                command_executions=(),
            )

    monkeypatch.setattr(codex_reply_module, "CodexAppServerClient", FakeCodexClient)

    result = codex_reply_module.generate_codex_session_reply(
        _reply_config(
            openai_api_key=None,
            repo_root=tmp_path,
            codex_capacity_retry_limit=1,
            codex_capacity_continue_text="請繼續",
        ),
        session={"session_id": "S-CAPACITY", "title": "Telegram chat · Wei"},
        messages=[{"role": "user", "content": "run the M4A DAG"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
    )

    assert result.text == "Recovered after capacity retry."
    assert FakeCodexClient.attempts == 2
    assert "run the M4A DAG" in FakeCodexClient.input_texts[0]
    assert "請繼續" in FakeCodexClient.input_texts[1]
    assert "automatic capacity retry" in FakeCodexClient.input_texts[1]


def test_generate_codex_session_reply_reports_when_capacity_retries_are_exhausted(monkeypatch, tmp_path: Path):
    codex_reply_module._reset_persistent_codex_clients_for_tests()

    class FakeCodexClient:
        attempts = 0

        def __init__(self, config):
            self.config = config
            self.is_started = False

        def start(self):
            self.is_started = True

        def close(self):
            self.is_started = False

        def start_thread(self, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            return {"id": "thread-capacity-exhausted"}

        def run_turn(self, *, thread_id, input_text, trace_context):
            self.__class__.attempts += 1
            raise codex_reply_module.CodexAppServerError(
                "Selected model is at capacity. Please try a different model."
            )

    monkeypatch.setattr(codex_reply_module, "CodexAppServerClient", FakeCodexClient)

    with pytest.raises(RuntimeError, match="after 1 automatic continuation retry"):
        codex_reply_module.generate_codex_session_reply(
            _reply_config(
                openai_api_key=None,
                repo_root=tmp_path,
                codex_capacity_retry_limit=1,
                codex_capacity_continue_text="請繼續",
            ),
            session={"session_id": "S-CAPACITY-EXHAUSTED", "title": "Telegram chat · Wei"},
            messages=[{"role": "user", "content": "run the M4A DAG"}],
            chat_id="123",
            chat_title="Wei",
            assistant_instructions="Use the shared session.",
        )

    assert FakeCodexClient.attempts == 2


def test_generate_codex_session_reply_retries_connection_close_only_once(monkeypatch, tmp_path: Path):
    codex_reply_module._reset_persistent_codex_clients_for_tests()

    class FakeCodexClient:
        attempts = 0
        developer_instructions: list[str] = []

        def __init__(self, config):
            self.config = config
            self.is_started = False

        def start(self):
            self.is_started = True

        def close(self):
            self.is_started = False

        def start_thread(self, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            self.developer_instructions.append(developer_instructions)
            return {"id": f"thread-{len(self.developer_instructions)}"}

        def resume_thread(self, thread_id, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            self.developer_instructions.append(developer_instructions)
            return {"id": thread_id}

        def run_turn(self, *, thread_id, input_text, trace_context):
            self.__class__.attempts += 1
            raise codex_reply_module.CodexAppServerError("Codex app-server connection closed.")

    monkeypatch.setattr(codex_reply_module, "CodexAppServerClient", FakeCodexClient)

    with pytest.raises(RuntimeError, match="Codex app-server connection closed"):
        codex_reply_module.generate_codex_session_reply(
            _reply_config(openai_api_key=None, repo_root=tmp_path),
            session={"session_id": "S-RETRY-LIMIT", "title": "Telegram chat · Wei"},
            messages=[{"role": "user", "content": "please continue"}],
            chat_id="123",
            chat_title="Wei",
            assistant_instructions="Use the shared session.",
        )

    assert FakeCodexClient.attempts == 2
    assert len(FakeCodexClient.developer_instructions) == 2
    assert "Retry safety note" not in FakeCodexClient.developer_instructions[0]
    assert "Retry safety note" in FakeCodexClient.developer_instructions[1]


def test_generate_codex_session_reply_retries_retryable_teardown_failure_once(monkeypatch, tmp_path: Path):
    codex_reply_module._reset_persistent_codex_clients_for_tests()

    class FakeCodexClient:
        attempts = 0
        instances = 0
        closes = 0
        developer_instructions: list[str] = []

        def __init__(self, config):
            self.config = config
            self.is_started = False
            self.__class__.instances += 1

        def start(self):
            self.is_started = True

        def close(self):
            self.is_started = False
            self.__class__.closes += 1

        def start_thread(self, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            self.developer_instructions.append(developer_instructions)
            return {"id": f"thread-{len(self.developer_instructions)}"}

        def resume_thread(self, thread_id, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            self.developer_instructions.append(developer_instructions)
            return {"id": thread_id}

        def run_turn(self, *, thread_id, input_text, trace_context):
            self.__class__.attempts += 1
            if self.__class__.attempts == 1:
                raise codex_reply_module.CodexAppServerError(
                    "Reconnecting... 2/5\ntimeout waiting for child process to exit"
                )
            return SimpleNamespace(
                text="Recovered after teardown retry.",
                turn_id="turn-retryable-teardown-2",
                usage=None,
                command_executions=(),
            )

    monkeypatch.setattr(codex_reply_module, "CodexAppServerClient", FakeCodexClient)

    result = codex_reply_module.generate_codex_session_reply(
        _reply_config(openai_api_key=None, repo_root=tmp_path),
        session={"session_id": "S-RETRYABLE-TEARDOWN", "title": "Telegram chat · Wei"},
        messages=[{"role": "user", "content": "please continue"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
    )

    assert result.text == "Recovered after teardown retry."
    assert FakeCodexClient.attempts == 2
    assert FakeCodexClient.instances == 2
    assert FakeCodexClient.closes == 1
    assert len(FakeCodexClient.developer_instructions) == 2
    assert "Retry safety note" not in FakeCodexClient.developer_instructions[0]
    assert "Retry safety note" in FakeCodexClient.developer_instructions[1]


def test_generate_codex_session_reply_reuses_persistent_client_thread(monkeypatch, tmp_path: Path):
    codex_reply_module._reset_persistent_codex_clients_for_tests()

    class FakeCodexClient:
        instances = 0
        starts = 0
        start_threads = 0
        run_thread_ids: list[str] = []

        def __init__(self, config):
            self.config = config
            self.is_started = False
            self.__class__.instances += 1

        def start(self):
            self.is_started = True
            self.__class__.starts += 1

        def close(self):
            self.is_started = False

        def start_thread(self, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            self.__class__.start_threads += 1
            assert persist_extended_history is True
            return {"id": "thread-persistent-1"}

        def run_turn(self, *, thread_id, input_text, trace_context):
            self.__class__.run_thread_ids.append(thread_id)
            return SimpleNamespace(
                text=f"reply {len(self.__class__.run_thread_ids)}",
                turn_id=f"turn-{len(self.__class__.run_thread_ids)}",
                usage=None,
                command_executions=(),
            )

    monkeypatch.setattr(codex_reply_module, "CodexAppServerClient", FakeCodexClient)
    config = _reply_config(openai_api_key=None, repo_root=tmp_path)

    first = codex_reply_module.generate_codex_session_reply(
        config,
        session={"session_id": "S-PERSIST", "title": "Telegram chat · Wei"},
        messages=[{"role": "user", "content": "first"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
    )
    second = codex_reply_module.generate_codex_session_reply(
        config,
        session={"session_id": "S-PERSIST", "title": "Telegram chat · Wei"},
        messages=[{"role": "user", "content": "second"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
    )

    assert first.text == "reply 1"
    assert second.text == "reply 2"
    assert FakeCodexClient.instances == 1
    assert FakeCodexClient.starts == 1
    assert FakeCodexClient.start_threads == 1
    assert FakeCodexClient.run_thread_ids == ["thread-persistent-1", "thread-persistent-1"]


def test_generate_codex_session_reply_resumes_thread_after_persistent_reconnect(monkeypatch, tmp_path: Path):
    codex_reply_module._reset_persistent_codex_clients_for_tests()

    class FakeCodexClient:
        run_count = 0
        resume_thread_ids: list[str] = []
        resume_developer_instructions: list[str] = []

        def __init__(self, config):
            self.config = config
            self.is_started = False

        def start(self):
            self.is_started = True

        def close(self):
            self.is_started = False

        def start_thread(self, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            return {"id": "thread-before-reconnect"}

        def resume_thread(self, thread_id, *, base_instructions, developer_instructions, persist_extended_history, trace_context):
            self.__class__.resume_thread_ids.append(thread_id)
            self.__class__.resume_developer_instructions.append(developer_instructions)
            assert persist_extended_history is True
            return {"id": thread_id}

        def run_turn(self, *, thread_id, input_text, trace_context):
            self.__class__.run_count += 1
            if self.__class__.run_count == 2:
                raise codex_reply_module.CodexAppServerError("Codex app-server connection closed.")
            return SimpleNamespace(
                text=f"reply after run {self.__class__.run_count}",
                turn_id=f"turn-{self.__class__.run_count}",
                usage=None,
                command_executions=(),
            )

    monkeypatch.setattr(codex_reply_module, "CodexAppServerClient", FakeCodexClient)
    config = _reply_config(openai_api_key=None, repo_root=tmp_path)

    codex_reply_module.generate_codex_session_reply(
        config,
        session={"session_id": "S-RESUME", "title": "Telegram chat · Wei"},
        messages=[{"role": "user", "content": "first"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
    )
    second = codex_reply_module.generate_codex_session_reply(
        config,
        session={"session_id": "S-RESUME", "title": "Telegram chat · Wei"},
        messages=[{"role": "user", "content": "second"}],
        chat_id="123",
        chat_title="Wei",
        assistant_instructions="Use the shared session.",
    )

    assert second.text == "reply after run 3"
    assert FakeCodexClient.resume_thread_ids == ["thread-before-reconnect"]
    assert "Retry safety note" in FakeCodexClient.resume_developer_instructions[0]


def test_codex_app_server_websocket_close_diagnostics_classifies_abnormal_without_close_frame():
    class ConnectionClosedError(Exception):
        code = None
        reason = ""
        rcvd = None
        sent = None
        rcvd_then_sent = None

    diagnostics = codex_app_server_module._websocket_close_diagnostics(
        ConnectionClosedError("no close frame received or sent")
    )

    assert diagnostics["close_kind"] == "abnormal"
    assert "close_code" not in diagnostics
    assert "close_rcvd_code" not in diagnostics
    assert "close_sent_code" not in diagnostics


def test_codex_app_server_websocket_close_diagnostics_classifies_normal_close_frame():
    class ConnectionClosedOK(Exception):
        code = None
        reason = ""
        rcvd = SimpleNamespace(code=1000, reason="normal shutdown")
        sent = None
        rcvd_then_sent = True

    diagnostics = codex_app_server_module._websocket_close_diagnostics(ConnectionClosedOK())

    assert diagnostics["close_kind"] == "normal"
    assert diagnostics["close_rcvd_code"] == 1000
    assert diagnostics["close_rcvd_reason"] == "normal shutdown"
    assert diagnostics["close_rcvd_then_sent"] is True


def test_codex_app_server_managed_stderr_log_path_prefers_runtime_log_dir(monkeypatch, tmp_path: Path):
    monkeypatch.delenv("AIT_CODEX_APP_SERVER_LOG_DIR", raising=False)
    monkeypatch.setenv("AIT_LOG_DIR", str(tmp_path / "runtime-logs"))

    path = codex_app_server_module._managed_stderr_log_path(tmp_path / "repo", 12345)

    assert path == tmp_path / "runtime-logs" / "codex-app-server-12345.stderr.log"


def test_codex_app_server_connect_uses_configured_websocket_max_size(monkeypatch, tmp_path: Path):
    captured: dict[str, Any] = {}

    class FakeWebsocket:
        def close(self):
            captured["closed"] = True

    def fake_connect(target_url, **kwargs):
        captured["target_url"] = target_url
        captured.update(kwargs)
        return FakeWebsocket()

    monkeypatch.setattr("websockets.sync.client.connect", fake_connect)
    monkeypatch.setattr(codex_app_server_module, "_log_codex_ws", lambda *args, **kwargs: None)

    client = codex_app_server_module.CodexAppServerClient(
        codex_app_server_module.CodexAppServerConfig(
            repo_root=tmp_path,
            bin_path="codex",
            model="gpt-5.4",
            reasoning_effort=None,
            sandbox="workspace-write",
            websocket_max_size_bytes=33_554_432,
        )
    )

    client._connect_websocket("ws://127.0.0.1:12345")
    client.close()

    assert captured["target_url"] == "ws://127.0.0.1:12345"
    assert captured["max_size"] == 33_554_432
    assert captured["closed"] is True


def test_service_session_command_links_chat_and_sync(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    session_update = {
        "update_id": 1,
        "message": {
            "message_id": 10,
            "text": "/session",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }
    service.handle_update(session_update)
    assert "ait Telegram status" in telegram_api.sent_messages[-1][1]
    session_id = service.state_store.get_chat(123)["session_id"]

    message_update = {
        "update_id": 2,
        "message": {
            "message_id": 11,
            "text": "@ait_test_bot   hello   from   telegram",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }
    service.handle_update(message_update)
    assert ait_api.turn_calls[-1]["text"] == "hello from telegram"
    assert ait_api.turn_calls[-1]["actor_identity"] == "telegram:456:@weita"
    assert ait_api.turn_calls[-1]["transport_envelope"]["transport"] == "telegram"
    assert ait_api.turn_calls[-1]["transport_envelope"]["actor"]["transport_user_id"] == "456"
    assert ait_api.turn_calls[-1]["transport_envelope"]["channel"]["channel_id"] == "123"
    assert ait_api.turn_calls[-1]["transport_envelope"]["message"]["message_id"] == 11
    assert ait_api.events[session_id][-2]["payload"]["text"] == "hello from telegram"
    assert ait_api.events[session_id][-2]["payload"]["transport_envelope"]["transport"] == "telegram"
    assert ait_api.events[session_id][-1]["event_type"] == "assistant.reply"
    assert ait_api.events[session_id][-1]["payload"]["text"] == "AI says hello."
    assert ait_api.events[session_id][-1]["payload"]["transport_reply_envelope"]["transport"] == "telegram"
    assert telegram_api.sent_messages[-1][1] == "AI says hello."
    assert service.state_store.get_chat(123)["last_synced_sequence"] == 2
    assert service.state_store.get_chat(123)["telegram_live_delivered_sequences"] == [2]

    ait_api.append_session_event(
        session_id,
        event_type="web.note",
        payload={"source": "web", "text": "Please publish the patchset."},
        actor_identity="alice@example.com",
        actor_type="human",
    )
    sync_update = {
        "update_id": 3,
        "message": {
            "message_id": 12,
            "text": "/sync",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }
    service.handle_update(sync_update)
    assert "web.note" in telegram_api.sent_messages[-1][1]
    assert "Please publish the patchset." in telegram_api.sent_messages[-1][1]
    assert "assistant.reply" not in telegram_api.sent_messages[-1][1]

    queue_update = {
        "update_id": 4,
        "message": {
            "message_id": 13,
            "text": "queue",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }
    service.handle_update(queue_update)
    assert "ait queue" in telegram_api.sent_messages[-1][1]


def test_service_recovers_deferred_reply_when_backend_connection_drops_after_append(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()

    class DisconnectAfterAppendAitApi(FakeAitApi):
        def create_telegram_turn(self, session_id: str, **kwargs) -> dict:
            super().create_telegram_turn(session_id, **kwargs)
            raise ConnectionError("Remote end closed connection without response")

    ait_api = DisconnectAfterAppendAitApi(reply_text="Recovered live reply.")
    service = TelegramBotService(
        _config(state_path, turn_merge_window_seconds=0.0, turn_merge_max_messages=1),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.submit_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Long backend reply",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    ).result(timeout=1.0)
    assert service.wait_for_idle(timeout=1.0)

    session_id = service.state_store.get_chat(123)["session_id"]
    assert [event["event_type"] for event in ait_api.events[session_id]] == [
        "telegram.user_message",
        "assistant.reply",
    ]
    assert telegram_api.sent_messages == [(123, "Recovered live reply.")]
    assert service.state_store.get_chat(123)["last_synced_sequence"] == 2
    assert service.state_store.get_chat(123)["telegram_live_delivered_sequences"] == [2]


def test_service_retries_deferred_reply_recovery_when_backend_reads_temporarily_fail(tmp_path: Path, monkeypatch):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()

    class FlakyDisconnectAfterAppendAitApi(FakeAitApi):
        def __init__(self, **kwargs):
            super().__init__(**kwargs)
            self.recovery_read_attempts = 0

        def create_telegram_turn(self, session_id: str, **kwargs) -> dict:
            super().create_telegram_turn(session_id, **kwargs)
            raise RemoteDisconnected("Remote end closed connection without response")

        def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50):
            self.recovery_read_attempts += 1
            if self.recovery_read_attempts < 3:
                raise telegram_app.BotRuntimeError(
                    "GET http://127.0.0.1:8088/v1/native/sessions/S-TEST/events failed: [Errno 61] Connection refused"
                ) from ConnectionRefusedError(61, "Connection refused")
            return super().list_session_events(session_id, after_sequence=after_sequence, limit=limit)

    ait_api = FlakyDisconnectAfterAppendAitApi(reply_text="Recovered after retry.")
    service = TelegramBotService(
        _config(state_path, turn_merge_window_seconds=0.0, turn_merge_max_messages=1),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    sleep_delays: list[float] = []
    monkeypatch.setattr(telegram_app.time, "sleep", lambda seconds: sleep_delays.append(seconds))

    service.submit_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Long backend reply",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    ).result(timeout=1.0)
    assert service.wait_for_idle(timeout=1.0)

    assert telegram_api.sent_messages == [(123, "Recovered after retry.")]
    assert ait_api.recovery_read_attempts == 3
    assert sleep_delays[:2] == [0.75, 1.5]


def test_service_watches_for_completed_deferred_reply_after_retryable_restart_failure(tmp_path: Path, monkeypatch):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()

    class DelayedAssistantAitApi(FakeAitApi):
        def __init__(self, **kwargs):
            super().__init__(**kwargs)
            self.pending_session_id: str | None = None
            self.pending_user_event: dict[str, Any] | None = None
            self.injected_reply = False

        def create_telegram_turn(self, session_id: str, **kwargs) -> dict:
            self.pending_session_id = session_id
            self.turn_calls.append(
                {
                    "session_id": session_id,
                    "text": kwargs["text"],
                    "chat_id": str(kwargs["chat_id"]),
                    "chat_title": kwargs["chat_title"],
                    "chat_type": kwargs["chat_type"],
                    "telegram_message_id": kwargs["telegram_message_id"],
                    "telegram_message_ids": list(kwargs.get("telegram_message_ids") or []),
                    "transport_envelope": dict(kwargs.get("transport_envelope") or {}),
                    "actor_identity": kwargs["actor_identity"],
                }
            )
            self.pending_user_event = self.append_session_event(
                session_id,
                event_type="telegram.user_message",
                payload={
                    "source": "telegram",
                    "text": kwargs["text"],
                    "telegram_chat_id": str(kwargs["chat_id"]),
                    "telegram_chat_title": kwargs["chat_title"],
                    "telegram_chat_type": kwargs["chat_type"],
                    "telegram_message_id": kwargs["telegram_message_id"],
                    "telegram_message_ids": list(kwargs.get("telegram_message_ids") or []),
                    "logical_turn_message_count": len(list(kwargs.get("telegram_message_ids") or [])),
                    **(
                        {"transport_envelope": dict(kwargs["transport_envelope"])}
                        if kwargs.get("transport_envelope")
                        else {}
                    ),
                },
                actor_identity=kwargs["actor_identity"],
                actor_type="telegram_user",
            )
            raise RemoteDisconnected("Remote end closed connection without response")

        def inject_assistant_reply(self) -> None:
            if self.injected_reply or self.pending_session_id is None or self.pending_user_event is None:
                return
            self.injected_reply = True
            self.append_session_event(
                self.pending_session_id,
                event_type="assistant.reply",
                payload={
                    "source": "openai",
                    "text": self.reply_text,
                    "model": "gpt-5.4-mini",
                    "response_id": "resp_test_watch_123",
                    "usage": {"input_tokens": 11, "output_tokens": 7, "total_tokens": 18},
                    "telegram_chat_id": "123",
                    "telegram_chat_title": "Wei",
                    "reply_to_sequence": int(self.pending_user_event["sequence"]),
                    "delivered_via": "telegram_live",
                    "transport_reply_envelope": build_transport_reply_envelope(
                        transport="telegram",
                        channel_id="123",
                        channel_title="Wei",
                        channel_kind="private",
                        text=self.reply_text,
                        reply_to_message_id=10,
                        reply_to_message_ids=[10],
                        metadata={"delivered_via": "telegram_live"},
                    ),
                },
                actor_identity="ait-server",
                actor_type="ai_assistant",
            )

    ait_api = DelayedAssistantAitApi(reply_text="Recovered by watch.")
    service = TelegramBotService(
        _config(state_path, turn_merge_window_seconds=0.0, turn_merge_max_messages=1),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    sleep_delays: list[float] = []
    monkeypatch.setattr(telegram_app, "DEFERRED_REPLY_RECOVERY_ATTEMPTS", 1)
    monkeypatch.setattr(telegram_app, "DEFERRED_REPLY_WATCH_MAX_WAIT_SECONDS", 15.0)
    monkeypatch.setattr(telegram_app, "DEFERRED_REPLY_WATCH_POLL_INTERVAL_SECONDS", 5.0)

    def fake_sleep(seconds: float) -> None:
        sleep_delays.append(seconds)
        if seconds >= 5.0:
            ait_api.inject_assistant_reply()

    monkeypatch.setattr(telegram_app.time, "sleep", fake_sleep)

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Watch for the stored reply",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    assert telegram_api.sent_messages == [(123, "Recovered by watch.")]
    assert sleep_delays == [5.0]
    assert service.state_store.get_chat(123)["last_synced_sequence"] == 2
    assert service.state_store.get_chat(123)["telegram_live_delivered_sequences"] == [2]


def test_service_skips_deferred_reply_recovery_when_server_write_never_connected(tmp_path: Path, monkeypatch):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()

    class NeverConnectedAitApi(FakeAitApi):
        def __init__(self, **kwargs):
            super().__init__(**kwargs)
            self.recovery_read_attempts = 0

        def create_telegram_turn(self, session_id: str, **kwargs) -> dict:
            raise telegram_app.BotRuntimeError(
                "POST http://127.0.0.1:8088/v1/native/sessions/S-TEST:telegramTurn failed: [Errno 61] Connection refused"
            ) from ConnectionRefusedError(61, "Connection refused")

        def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50):
            self.recovery_read_attempts += 1
            return super().list_session_events(session_id, after_sequence=after_sequence, limit=limit)

    ait_api = NeverConnectedAitApi(reply_text="Should not be recovered.")
    service = TelegramBotService(
        _config(state_path, turn_merge_window_seconds=0.0, turn_merge_max_messages=1),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    sleep_delays: list[float] = []
    monkeypatch.setattr(telegram_app.time, "sleep", lambda seconds: sleep_delays.append(seconds))

    service.submit_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Fail fast when the server is down",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    ).result(timeout=1.0)
    assert service.wait_for_idle(timeout=1.0)

    assert ait_api.recovery_read_attempts == 0
    assert sleep_delays == []
    assert len(telegram_api.sent_messages) == 1
    assert telegram_api.sent_messages[0][0] == 123
    assert "Connection refused" in telegram_api.sent_messages[0][1]
    link = service.state_store.get_chat(123)
    assert link is not None
    assert link["telegram_reply_spool"][0]["status"] == "failed"


def test_service_persists_pending_reply_spool_scaffold_during_blocked_turn(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    started = threading.Event()
    release = threading.Event()

    class BlockingAitApi(FakeAitApi):
        def create_telegram_turn(self, session_id: str, **kwargs) -> dict:
            started.set()
            assert release.wait(timeout=2.0), "test should release blocked Telegram turn"
            return super().create_telegram_turn(session_id, **kwargs)

    state_store = TelegramSyncStateStore(state_path)
    service = TelegramBotService(
        _config(state_path, turn_merge_window_seconds=0.0, turn_merge_max_messages=1),
        ait_api=BlockingAitApi(reply_text="Spool released."),
        telegram_api=telegram_api,
        state_store=state_store,
    )

    future = service.submit_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Block until the spool is visible",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    assert started.wait(timeout=1.0), "pending Telegram turn should reach the blocked backend call"

    link = state_store.get_chat(123)
    assert link is not None
    spool = link["telegram_reply_spool"]
    assert len(spool) == 1
    assert spool[0]["status"] == "attempting"
    assert spool[0]["attempt_count"] == 1
    assert spool[0]["telegram_message_ids"] == [10]
    assert spool[0]["text"] == "Block until the spool is visible"

    release.set()
    future.result(timeout=2.0)
    assert service.wait_for_idle(timeout=2.0)

    assert state_store.get_chat(123).get("telegram_reply_spool", []) == []
    assert telegram_api.sent_messages[-1] == (123, "Spool released.")


def test_sync_surfaces_unacked_telegram_live_reply_after_lost_response(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi(reply_text="Recovered by sync.")
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    session = ait_api.create_session(chat_id="123", chat_title="Wei", chat_type="private")
    service.state_store.upsert_chat(
        123,
        session_id=session["session_id"],
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        last_synced_sequence=0,
    )
    ait_api.create_telegram_turn(
        session["session_id"],
        text="Lost HTTP response",
        chat_id="123",
        chat_title="Wei",
        chat_type="private",
        telegram_message_id=10,
        telegram_message_ids=[10],
        actor_identity="telegram:456:@weita",
    )

    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": "/sync",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    assert telegram_api.sent_messages[-1] == (123, "Recovered by sync.")
    assert service.state_store.get_chat(123)["last_synced_sequence"] == 2


def test_service_music_audio_upload_creates_turn_with_attachment_metadata(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    update = {
        "update_id": 1,
        "message": {
            "message_id": 11,
            "caption": "請幫我處理這首歌",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
            "audio": {
                "file_id": "tg-audio-001",
                "file_unique_id": "unique-audio-001",
                "file_name": "demo-track.mp3",
                "mime_type": "audio/mpeg",
                "duration": 42,
                "file_size": 1_048_576,
                "title": "Demo Track",
                "performer": "AI Band",
            },
        },
    }

    service.handle_update(update)

    turn = ait_api.turn_calls[-1]
    assert "請幫我處理這首歌" in turn["text"]
    assert "Telegram music upload:" in turn["text"]
    attachments = turn["transport_envelope"]["message"]["attachments"]
    assert attachments == [
        {
            "kind": "audio",
            "media_kind": "music",
            "telegram_file_id": "tg-audio-001",
            "telegram_file_unique_id": "unique-audio-001",
            "file_name": "demo-track.mp3",
            "mime_type": "audio/mpeg",
            "caption": "請幫我處理這首歌",
            "title": "Demo Track",
            "performer": "AI Band",
            "duration_seconds": 42,
            "file_size_bytes": 1_048_576,
        }
    ]
    assert telegram_api.sent_messages[-1] == (123, "AI says hello.")


def test_service_voice_upload_uses_local_stt_runtime(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    stt_runtime = FakeSpeechToTextRuntime(text="請整理成三點摘要\n\n[local speech transcript]\n今天先修 Telegram 語音 STT。")
    service = TelegramBotService(
        _config(state_path, stt_mode="local-stt"),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
        speech_to_text_runtime=stt_runtime,
    )

    update = {
        "update_id": 1,
        "message": {
            "message_id": 11,
            "caption": "請整理成三點摘要",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
            "voice": {
                "file_id": "tg-voice-001",
                "file_unique_id": "unique-voice-001",
                "mime_type": "audio/ogg",
                "duration": 9,
                "file_size": 2048,
            },
        },
    }

    service.handle_update(update)

    turn = ait_api.turn_calls[-1]
    assert turn["text"] == "請整理成三點摘要\n\n[local speech transcript]\n今天先修 Telegram 語音 STT。"
    attachments = turn["transport_envelope"]["message"]["attachments"]
    assert attachments == [
        {
            "kind": "voice",
            "media_kind": "speech",
            "telegram_file_id": "tg-voice-001",
            "telegram_file_unique_id": "unique-voice-001",
            "mime_type": "audio/ogg",
            "caption": "請整理成三點摘要",
            "duration_seconds": 9,
            "file_size_bytes": 2048,
        }
    ]
    assert stt_runtime.calls[0]["attachments"] == attachments
    assert telegram_api.sent_messages[-1] == (123, "AI says hello.")

def test_service_voice_upload_reports_when_local_stt_is_disabled(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path, stt_mode="off"),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 11,
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
                "voice": {
                    "file_id": "tg-voice-001",
                    "file_unique_id": "unique-voice-001",
                    "mime_type": "audio/ogg",
                    "duration": 9,
                    "file_size": 2048,
                },
            },
        }
    )

    assert ait_api.turn_calls == []
    assert telegram_api.sent_messages[-1] == (
        123,
        "Local STT is not enabled for this Telegram worker. Set `AIT_TELEGRAM_STT_MODE=local-stt` and retry.",
    )

def test_service_audio_upload_uses_local_stt_when_audio_opt_in_is_enabled(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    stt_runtime = FakeSpeechToTextRuntime(text="這段音訊請轉成代辦事項。")
    service = TelegramBotService(
        _config(
            state_path,
            stt_mode="local-stt",
            stt_include_audio_uploads=True,
        ),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
        speech_to_text_runtime=stt_runtime,
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 11,
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
                "audio": {
                    "file_id": "tg-audio-voice-001",
                    "file_unique_id": "unique-audio-voice-001",
                    "file_name": "voice-note.mp3",
                    "mime_type": "audio/mpeg",
                    "duration": 12,
                    "file_size": 4096,
                },
            },
        }
    )

    turn = ait_api.turn_calls[-1]
    assert turn["text"] == "這段音訊請轉成代辦事項。"
    assert turn["transport_envelope"]["message"]["attachments"] == [
        {
            "kind": "audio",
            "media_kind": "speech",
            "telegram_file_id": "tg-audio-voice-001",
            "telegram_file_unique_id": "unique-audio-voice-001",
            "file_name": "voice-note.mp3",
            "mime_type": "audio/mpeg",
            "duration_seconds": 12,
            "file_size_bytes": 4096,
        }
    ]
    assert telegram_api.sent_messages[-1] == (123, "AI says hello.")

def test_service_voice_upload_reports_local_stt_failures(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    stt_runtime = FakeSpeechToTextRuntime(
        fail_message="Local STT failed while transcribing that audio. Please retry or send text instead."
    )
    service = TelegramBotService(
        _config(state_path, stt_mode="local-stt"),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
        speech_to_text_runtime=stt_runtime,
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 11,
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
                "voice": {
                    "file_id": "tg-voice-001",
                    "file_unique_id": "unique-voice-001",
                    "mime_type": "audio/ogg",
                    "duration": 9,
                    "file_size": 2048,
                },
            },
        }
    )

    assert ait_api.turn_calls == []
    assert telegram_api.sent_messages[-1] == (
        123,
        "Local STT failed while transcribing that audio. Please retry or send text instead.",
    )

def test_service_ignores_non_music_document_without_text(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    update = {
        "update_id": 1,
        "message": {
            "message_id": 11,
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
            "document": {
                "file_id": "tg-doc-001",
                "file_unique_id": "unique-doc-001",
                "file_name": "notes.pdf",
                "mime_type": "application/pdf",
                "file_size": 512,
            },
        },
    }

    service.handle_update(update)

    assert ait_api.turn_calls == []
    assert telegram_api.sent_messages == []


def test_service_sends_music_reply_attachments(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    track_path = tmp_path / "reply-track.mp3"
    track_path.write_bytes(b"demo-track")
    lossless_path = tmp_path / "reply-track.flac"
    lossless_path.write_bytes(b"demo-lossless")
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi(
        reply_text="這是你要的音樂檔。",
        reply_transport_attachments=[
            {
                "kind": "audio",
                "file_name": "reply-track.mp3",
                "mime_type": "audio/mpeg",
                "local_path": str(track_path),
                "title": "Reply Track",
                "performer": "ait",
            },
            {
                "kind": "document",
                "file_name": "reply-track.flac",
                "mime_type": "audio/flac",
                "local_path": str(lossless_path),
            },
        ],
    )
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    update = {
        "update_id": 1,
        "message": {
            "message_id": 11,
            "text": "請把音樂檔回傳給我",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }

    service.handle_update(update)

    assert telegram_api.sent_messages[-1] == (123, "這是你要的音樂檔。")
    assert telegram_api.sent_audios == [
        (
            123,
            {
                "kind": "audio",
                "file_name": "reply-track.mp3",
                "mime_type": "audio/mpeg",
                "local_path": str(track_path),
                "title": "Reply Track",
                "performer": "ait",
            },
        )
    ]
    assert telegram_api.sent_documents == [
        (
            123,
            {
                "kind": "document",
                "file_name": "reply-track.flac",
                "mime_type": "audio/flac",
                "local_path": str(lossless_path),
            },
        )
    ]


def test_service_logs_message_when_backend_reply_fails(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi(fail_turn=True)
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Hello from Telegram",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    session_id = service.state_store.get_chat(123)["session_id"]
    assert ait_api.events[session_id][-1]["event_type"] == "telegram.user_message"
    assert "AI reply failed" in telegram_api.sent_messages[-1][1]
    assert "test ai failure" in telegram_api.sent_messages[-1][1]
    assert service.state_store.get_chat(123)["last_synced_sequence"] == 1


def test_service_recovers_retryable_failed_turn_from_stored_assistant_event(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()

    class RetryableFailedTurnAitApi(FakeAitApi):
        def create_telegram_turn(self, session_id: str, **kwargs) -> dict:
            turn = super().create_telegram_turn(session_id, **kwargs)
            return {
                **turn,
                "ok": False,
                "assistant_event": None,
                "reply_text": None,
                "error": "Codex websocket reply failed: Reconnecting... 2/5\ntimeout waiting for child process to exit",
            }

    ait_api = RetryableFailedTurnAitApi(reply_text="Recovered after retryable failed turn.")
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Hello from Telegram",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    assert telegram_api.sent_messages == [(123, "Recovered after retryable failed turn.")]
    link = service.state_store.get_chat(123)
    assert link["last_synced_sequence"] == 2
    assert link["telegram_live_delivered_sequences"] == [2]
    assert link.get("telegram_reply_spool", []) == []


def test_service_sends_capacity_notice_when_backend_capacity_retries_exhaust(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi(
        fail_turn=True,
        fail_error=(
            "Selected model is at capacity after 1 automatic continuation retry. "
            "Telegram has been notified."
        ),
    )
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Run the DAG",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    sent = telegram_api.sent_messages[-1][1]
    assert "still at capacity" in sent
    assert "automatic continuation retry did not complete or is unavailable" in sent
    assert "send `請繼續`" in sent


def test_help_command_includes_examples_and_group_guidance(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    service = TelegramBotService(
        _config(state_path),
        ait_api=FakeAitApi(),
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/help",
                "chat": {"id": 123, "type": "group", "title": "AIT room"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    help_text = telegram_api.sent_messages[-1][1]
    assert "Thin Telegram transport for a shared ait session." in help_text
    assert "Sync mode: manual_only." in help_text
    assert "Workflow query examples" in help_text
    assert "what should land next" in help_text
    assert "@ait_test_bot summarize what should land next" in help_text
    assert "No linked session yet." in help_text


def test_owner_bootstrap_start_prompts_for_repository_password(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path, owner_bootstrap_enabled=True),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/start",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    assert telegram_api.sent_messages == [
        (123, "Telegram bootstrap is locked. Send the repository-name password as plain text.")
    ]
    assert ait_api.turn_calls == []
    assert service.state_store.get_bootstrap_auth()["pending_user_id"] == "456"
    assert service.state_store.get_chat(123) is None


def test_owner_bootstrap_success_claims_owner_and_then_allows_turns(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path, owner_bootstrap_enabled=True),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    _claim_telegram_owner(service)
    auth_state = service.state_store.get_bootstrap_auth()

    assert auth_state["owner_user_id"] == "456"
    assert telegram_api.sent_messages[-1] == (
        123,
        "Owner verified. Telegram access is now bound to this user id. Send /help or a normal message to continue.",
    )
    assert ait_api.turn_calls == []

    service.handle_update(
        {
            "update_id": 3,
            "message": {
                "message_id": 12,
                "text": "Hello after claim",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    assert ait_api.turn_calls[-1]["text"] == "Hello after claim"
    assert telegram_api.sent_messages[-1] == (123, "AI says hello.")
    assert service.state_store.get_chat(123)["session_id"].startswith("AITS-TEST-")


def test_owner_bootstrap_auto_adopts_existing_private_chat_link(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    state_store = TelegramSyncStateStore(state_path)
    existing_session = ait_api.create_session(
        chat_id="123",
        chat_title="Wei",
        chat_type="private",
    )
    state_store.upsert_chat(
        123,
        session_id=existing_session["session_id"],
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id=existing_session["session_id"],
        binding_role="primary_shared",
        last_synced_sequence=0,
    )
    service = TelegramBotService(
        _config(state_path, owner_bootstrap_enabled=True),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=state_store,
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Hello after runtime resync",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    auth_state = state_store.get_bootstrap_auth()
    assert auth_state["owner_user_id"] == "456"
    assert auth_state["owner_claim_reason"] == "existing_private_chat_link"
    assert ait_api.turn_calls[-1]["text"] == "Hello after runtime resync"
    assert ait_api.turn_calls[-1]["session_id"] == existing_session["session_id"]
    assert telegram_api.sent_messages[-1] == (123, "AI says hello.")


def test_load_config_for_telegram_worker_seeded_repo_local_state_keeps_owner_private_chat_usable(
    tmp_path: Path,
    monkeypatch,
):
    repo_root = _write_bound_repo_checkout(
        tmp_path / "repo",
        "ait",
        env_lines=["AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS=inf"],
        worker_payload={
            "version": 1,
            "workers": {
                "telegram/main": {
                    "kind": "telegram",
                    "name": "main",
                    "token": "123456:worker-token",
                    "sync_state_path": "runtime/telegram-sync.json",
                }
            },
        },
    )
    data_dir = tmp_path / "server-data"
    shared_state_path = data_dir / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    existing_session = ait_api.create_session(
        chat_id="123",
        chat_title="Wei",
        chat_type="private",
    )
    shared_store = TelegramSyncStateStore(shared_state_path)
    shared_store.upsert_chat(
        123,
        session_id=existing_session["session_id"],
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id=existing_session["session_id"],
        binding_role="primary_shared",
        last_synced_sequence=0,
    )
    shared_store.save_bootstrap_auth(
        {
            "owner_user_id": "456",
            "owner_chat_id": "123",
            "owner_chat_title": "Wei",
            "owner_chat_type": "private",
        }
    )
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(repo_root / ".ait" / "agent-workers.json"))
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(repo_root / ".ait" / "agent-runtime" / "telegram.env"))

    config = load_config_for_telegram_worker(repo_root)
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(config.sync_state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Hello after runtime resync",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    assert ait_api.turn_calls[-1]["text"] == "Hello after runtime resync"
    assert ait_api.turn_calls[-1]["session_id"] == existing_session["session_id"]
    assert telegram_api.sent_messages == [(123, "AI says hello.")]
    assert service.state_store.get_bootstrap_auth()["owner_user_id"] == "456"


def test_owner_bootstrap_blacklists_after_three_failed_password_attempts(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path, owner_bootstrap_enabled=True),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/start",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    for update_id, message_id, text in [(2, 11, "wrong-1"), (3, 12, "wrong-2"), (4, 13, "wrong-3")]:
        service.handle_update(
            {
                "update_id": update_id,
                "message": {
                    "message_id": message_id,
                    "text": text,
                    "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                    "from": {"id": 456, "username": "weita"},
                },
            }
        )

    assert telegram_api.sent_messages[1] == (123, "Incorrect password. 2 attempts remaining.")
    assert telegram_api.sent_messages[2] == (123, "Incorrect password. 1 attempt remaining.")
    assert telegram_api.sent_messages[3] == (
        123,
        "Incorrect password. This Telegram user id is now blocked until local reset clears the runtime auth state.",
    )
    auth_state = service.state_store.get_bootstrap_auth()
    assert "456" in auth_state["blacklist"]
    assert "pending_user_id" not in auth_state

    sent_before_retry = list(telegram_api.sent_messages)
    service.handle_update(
        {
            "update_id": 5,
            "message": {
                "message_id": 14,
                "text": "/start",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    assert telegram_api.sent_messages == sent_before_retry
    assert ait_api.turn_calls == []


@pytest.mark.parametrize("text", ["/queue", "/session", "queue", "hello there"])
def test_owner_bootstrap_blocks_unauthorized_entrypoints(tmp_path: Path, text: str):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path, owner_bootstrap_enabled=True),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    _claim_telegram_owner(service)
    message_count = len(telegram_api.sent_messages)

    service.handle_update(
        {
            "update_id": 3,
            "message": {
                "message_id": 12,
                "text": text,
                "chat": {"id": 999, "type": "private", "first_name": "Mallory"},
                "from": {"id": 789, "username": "mallory"},
            },
        }
    )

    assert len(telegram_api.sent_messages) == message_count
    assert ait_api.turn_calls == []
    assert service.state_store.get_chat(999) is None


def test_status_command_reports_runtime_link_and_sync_mode(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    service = TelegramBotService(
        _config(state_path),
        ait_api=FakeAitApi(),
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": "/status",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    status_text = telegram_api.sent_messages[-1][1]
    assert "ait Telegram status" in status_text
    assert "runtime_link=active" in status_text
    assert "sync_mode=manual_only" in status_text
    assert "workflow_notifications=off" in status_text
    assert "reply_context_mode=recent_tail" in status_text
    assert "checkpoint_freshness=missing" in status_text
    assert "checkpoint_delta_events=0/6" in status_text
    assert "last_sync_at=" in status_text


def test_unknown_command_points_to_help_examples(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    service = TelegramBotService(
        _config(state_path),
        ait_api=FakeAitApi(),
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/unknown",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    message = telegram_api.sent_messages[-1][1]
    assert "Unknown command /unknown." in message
    assert "/queue" in message
    assert "/task AITT-..." in message
    assert "/change AITC-..." in message
    assert "/notify on|off|status" in message
    assert "/sync" in message


def test_status_reports_relink_skip_diagnostics_with_source_session(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    service = TelegramBotService(
        _config(state_path),
        ait_api=FakeAitApi(),
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.state_store.patch_chat(
        123,
        last_relink_skipped_reply_sequence=27,
        last_relink_skipped_reply_at="2026-05-06T00:00:00+00:00",
        last_relink_skipped_from_session_id="AITS-OLD",
    )

    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": "/status",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    status_text = telegram_api.sent_messages[-1][1]
    assert "last_relink_skipped_reply_sequence=27" in status_text
    assert "last_relink_skipped_reply_at=2026-05-06T00:00:00+00:00" in status_text
    assert "last_relink_skipped_from_session_id=AITS-OLD" in status_text


def test_attention_ready_audit_and_land_commands(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    ait_api.task_queue_payload = {
        "summary": {"active": 3, "attention_required": 1, "ready_to_land": 1, "ready_to_complete": 1},
        "items": [
            {
                "task": {"task_id": "AITT-1000", "title": "Blocked task"},
                "workflow": {"state": "attention_required", "reason": "Policy is still pending."},
                "primary_gate": "policy",
                "next_action": {"code": "inspect_change", "label": "Inspect change", "detail": "Open the focus change and fix policy."},
            },
            {
                "task": {"task_id": "AITT-1001", "title": "Landable task"},
                "workflow": {"state": "ready_to_land", "reason": "1 linked change can land now."},
                "next_action": {"code": "land_change", "label": "Land change", "detail": "Submit land for the selected patchset."},
            },
            {
                "task": {"task_id": "AITT-1002", "title": "Completable task"},
                "workflow": {"state": "ready_to_complete", "reason": "All linked changes are landed."},
                "next_action": {"code": "complete_task", "label": "Complete task", "detail": "Close the task after verifying target line state."},
            },
        ],
    }
    ait_api.task_audit_payload = {
        "task": {"task_id": "AITT-1001", "title": "Landable task"},
        "workflow": {"state": "ready_to_land", "reason": "1 linked change can land now."},
        "recommended_action": {"code": "land_change", "label": "Land change", "detail": "Submit land for the selected patchset."},
        "target": {"line_name": "main"},
        "summary": {
            "verdict": "not_landed_on_target",
            "open_change_count": 1,
            "landed_change_count": 0,
            "effective_on_target_change_count": 0,
        },
        "changes": [{"change": {"change_id": "AITC-1001", "status": "gated"}, "target_state": "not_on_target"}],
    }
    ait_api.change_detail_payload = {
        "change": {"change_id": "AITC-1001", "title": "Landable change", "status": "gated", "lane": "assisted", "risk_tier": "medium"},
        "task": {"task_id": "AITT-1001"},
        "current_patchset": {"patchset_id": "AITP-1001-1"},
        "policy_summary": {"decision": "pass"},
        "review_summary": {"approvals": 1, "blocking": 0, "comments": 2},
        "freshness": {"base_is_fresh": True},
    }
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    for index, text in enumerate(("/attention", "/ready", "/audit AITT-1001", "/land AITC-1001"), start=1):
        service.handle_update(
            {
                "update_id": index,
                "message": {
                    "message_id": 10 + index,
                    "text": text,
                    "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                    "from": {"id": 456, "username": "weita"},
                },
            }
        )

    assert "AITT-1000" in telegram_api.sent_messages[0][1]
    assert "Ready to land" in telegram_api.sent_messages[1][1]
    assert "verdict=not_landed_on_target" in telegram_api.sent_messages[2][1]
    assert "land_state=ready_to_land" in telegram_api.sent_messages[3][1]


def test_notify_command_toggles_runtime_preferences(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": "/notify on",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    enabled_link = service.state_store.get_chat(123)
    assert enabled_link["workflow_notifications_enabled"] is True
    assert "Workflow notifications enabled" in telegram_api.sent_messages[-1][1]
    assert telegram_api.sent_messages[-1][1].endswith("Complete")

    service.handle_update(
        {
            "update_id": 3,
            "message": {
                "message_id": 12,
                "text": "/notify status",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    assert "workflow_notifications=on" in telegram_api.sent_messages[-1][1]

    service.handle_update(
        {
            "update_id": 4,
            "message": {
                "message_id": 13,
                "text": "/notify off",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    disabled_link = service.state_store.get_chat(123)
    assert disabled_link["workflow_notifications_enabled"] is False
    assert "Workflow notifications disabled" in telegram_api.sent_messages[-1][1]


def test_background_sync_sends_queue_notifications_when_enabled(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    ait_api.task_queue_payload = {
        "summary": {"active": 1, "attention_required": 1, "ready_to_land": 0, "ready_to_complete": 0},
        "items": [
            {
                "task": {"task_id": "AITT-2000", "title": "Attention task"},
                "workflow": {"state": "attention_required", "reason": "Tests are still pending."},
                "primary_gate": "ci",
                "ci_summary": {
                    "patchset_id": "AITP-2000",
                    "tg1_required": {"status": "fail", "live_count": 23, "minimum_count": 24},
                    "remote_land_gate": "blocked",
                },
                "next_action": {"code": "inspect_change", "label": "Inspect change", "detail": "Review the missing test evidence."},
            }
        ],
    }
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": "/notify on",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    enabled_message = telegram_api.sent_messages[-1][1]
    assert "\n\nCI\n" in enabled_message
    assert "TG-1=fail 23/24" in enabled_message
    assert "land=blocked" in enabled_message
    assert "state=" not in enabled_message
    assert "next=" not in enabled_message
    message_count = len(telegram_api.sent_messages)

    ait_api.task_queue_payload = {
        "summary": {"active": 1, "attention_required": 0, "ready_to_land": 1, "ready_to_complete": 0},
        "items": [
            {
                "task": {"task_id": "AITT-2001", "title": "Ready task"},
                "workflow": {"state": "ready_to_land", "reason": "Patchset is landable."},
                "next_action": {"code": "land_change", "label": "Land change", "detail": "Submit land for the selected patchset."},
            }
        ],
    }

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(telegram_api.sent_messages) == message_count + 1
    notification = telegram_api.sent_messages[-1][1]
    assert "workflow (ait)" in notification
    assert "Ready to land" in notification
    assert "Ready now" not in notification
    assert "attention=" not in notification
    assert "AITT-2001" in notification
    assert "state=" not in notification
    assert "next=" not in notification


def test_background_sync_does_not_repeat_same_actionable_notification_when_only_summary_drifts(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    ait_api.task_queue_payload = {
        "summary": {"active": 2, "attention_required": 1, "ready_to_land": 0, "ready_to_complete": 0},
        "items": [
            {
                "task": {"task_id": "AITT-2000", "title": "Attention task"},
                "workflow": {"state": "attention_required", "reason": "Tests are still pending."},
                "primary_gate": "ci",
                "next_action": {"code": "inspect_change", "label": "Inspect change", "detail": "Review the missing test evidence."},
            },
            {
                "task": {"task_id": "AITT-2998", "title": "Planning task"},
                "workflow": {"state": "planning", "reason": "No linked changes exist yet."},
                "next_action": {"code": "create_change", "label": "Create change", "detail": "Open a first change for this task."},
            },
        ],
    }
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": "/notify on",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    message_count = len(telegram_api.sent_messages)

    ait_api.task_queue_payload = {
        "summary": {"active": 3, "attention_required": 1, "ready_to_land": 0, "ready_to_complete": 0},
        "items": [
            {
                "task": {"task_id": "AITT-2000", "title": "Attention task"},
                "workflow": {"state": "attention_required", "reason": "Tests are still pending."},
                "primary_gate": "ci",
                "next_action": {"code": "inspect_change", "label": "Inspect change", "detail": "Review the missing test evidence."},
            },
            {
                "task": {"task_id": "AITT-2998", "title": "Planning task"},
                "workflow": {"state": "planning", "reason": "No linked changes exist yet."},
                "next_action": {"code": "create_change", "label": "Create change", "detail": "Open a first change for this task."},
            },
            {
                "task": {"task_id": "AITT-2999", "title": "Another planning task"},
                "workflow": {"state": "planning", "reason": "Still waiting for a first change."},
                "next_action": {"code": "create_change", "label": "Create change", "detail": "Open a first change for this task."},
            },
        ],
    }

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(telegram_api.sent_messages) == message_count


def test_background_sync_sends_ready_to_complete_section_when_only_completable_items_remain(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    ait_api.task_queue_payload = {
        "summary": {"active": 1, "attention_required": 0, "ready_to_land": 0, "ready_to_complete": 1},
        "items": [
            {
                "task": {"task_id": "AITT-2002", "title": "Completable task"},
                "workflow": {"state": "ready_to_complete", "reason": "All linked changes are landed; the task can complete."},
                "next_action": {"code": "complete_task", "label": "Complete task", "detail": "Close the task after verifying target line state."},
            }
        ],
    }
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": "/notify on",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    notification = telegram_api.sent_messages[-1][1]
    assert "workflow (ait)" in notification
    assert "Ready to complete" in notification
    assert "Ready now" not in notification
    assert "AITT-2002" in notification


def test_background_sync_sends_complete_when_actionable_queue_clears(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    ait_api.task_queue_payload = {
        "summary": {"active": 1, "attention_required": 1, "ready_to_land": 0, "ready_to_complete": 0},
        "items": [
            {
                "task": {"task_id": "AITT-2000", "title": "Attention task"},
                "workflow": {"state": "attention_required", "reason": "Tests are still pending."},
                "primary_gate": "ci",
                "next_action": {"code": "inspect_change", "label": "Inspect change", "detail": "Review the missing test evidence."},
            }
        ],
    }
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": "/notify on",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    message_count = len(telegram_api.sent_messages)

    ait_api.task_queue_payload = {
        "summary": {"active": 1, "attention_required": 0, "ready_to_land": 0, "ready_to_complete": 0},
        "items": [
            {
                "task": {"task_id": "AITT-2002", "title": "Planning task"},
                "workflow": {"state": "planning", "reason": "No linked changes exist yet."},
                "next_action": {"code": "create_change", "label": "Create change", "detail": "Open a first change for this task."},
            }
        ],
    }

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(telegram_api.sent_messages) == message_count + 1
    assert telegram_api.sent_messages[-1][1] == "workflow (ait)\n\nComplete"


def test_watchgraph_command_stores_runtime_graph_watch(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": f"/watchgraph PL-TEST123 {graph_path}",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    link = service.state_store.get_chat(123)
    assert "PL-TEST123" in link["graph_watches"]
    assert link["graph_watches"]["PL-TEST123"]["last_progress_digest"]
    assert link["graph_watches"]["PL-TEST123"]["last_next_action"] == "start A"
    assert ait_api.task_dag_progress_calls[0]["graph_id"] == "demo-graph"
    start_message = telegram_api.sent_messages[-1][1]
    assert start_message.startswith("ait graph start")
    assert "repo=ait" in start_message
    assert "plan=PL-TEST123" in start_message
    assert "graph=demo-graph" in start_message
    assert "completed_percent=0" in start_message


def test_auto_register_graph_watch_uses_session_link_and_skips_duplicate_start_notification(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    store = TelegramSyncStateStore(state_path)
    store.upsert_chat(123, session_id="S-TELEGRAM-1", repo_name="ait", chat_type="private", chat_title="Wei")
    config = _config(state_path)
    progress_calls: list[dict[str, Any]] = []
    progress_payload = {
        "progress": {
            "graph_id": "demo-graph",
            "completed_percent": 0,
            "completed_nodes": 0,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "next_action": "start A",
            "node_states": {"A": {"state": "ready"}},
        },
        "blockers": [],
    }

    first = telegram_app.auto_register_graph_watch(
        config,
        plan_id="PL-TEST123",
        graph_path=str(graph_path),
        progress_reader=lambda graph: progress_calls.append(graph) or progress_payload,
        repo_name="ait",
        linked_session_id="S-TELEGRAM-1",
        state_store=store,
        telegram_api=telegram_api,
    )

    assert first["registered"] is True
    assert first["created"] is True
    assert first["notification_sent"] is True
    assert first["resolution_mode"] == "session_id"
    assert progress_calls[0]["graph_id"] == "demo-graph"
    assert len(telegram_api.sent_messages) == 1
    assert telegram_api.sent_messages[-1][1].startswith("ait graph start")

    second = telegram_app.auto_register_graph_watch(
        config,
        plan_id="PL-TEST123",
        graph_path=str(graph_path),
        progress_reader=lambda graph: (_ for _ in ()).throw(AssertionError("existing watch should not re-read progress")),
        repo_name="ait",
        linked_session_id="S-TELEGRAM-1",
        state_store=store,
        telegram_api=telegram_api,
    )

    assert second["registered"] is True
    assert second["already_registered"] is True
    assert second["notification_sent"] is False
    assert len(telegram_api.sent_messages) == 1


def test_task_dag_trigger_sends_graph_progress_changes(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": f"/watchgraph PL-TEST123 {graph_path}",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    message_count = len(telegram_api.sent_messages)
    initial_progress_calls = len(ait_api.task_dag_progress_calls)

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(ait_api.task_dag_progress_calls) == initial_progress_calls
    assert len(telegram_api.sent_messages) == message_count

    progress_payload = {
        "progress": {
            "graph_id": "demo-graph",
            "completed_percent": 50,
            "estimated_percent": 87,
            "completed_nodes": 1,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "next_action": "start B",
            "node_states": {"A": {"state": "completed"}, "B": {"state": "ready"}},
        },
        "blockers": [],
    }
    progress_calls: list[dict[str, Any]] = []
    summary = trigger_graph_watch_notifications(
        config,
        repo_name="ait",
        state_store=service.state_store,
        telegram_api=telegram_api,
        progress_reader=lambda graph: progress_calls.append(graph) or progress_payload,
    )

    assert summary["sent"] == 1
    assert progress_calls[0]["graph_id"] == "demo-graph"
    assert len(telegram_api.sent_messages) == message_count + 1
    update_message = telegram_api.sent_messages[-1][1]
    assert update_message == "A —> B (50%)"
    assert "87" not in update_message
    assert "repo=" not in update_message
    assert "plan=PL-TEST123" not in update_message
    assert "graph=demo-graph" not in update_message

    message_count = len(telegram_api.sent_messages)
    summary = trigger_graph_watch_notifications(
        config,
        repo_name="ait",
        state_store=service.state_store,
        telegram_api=telegram_api,
        progress_reader=lambda graph: progress_payload,
    )
    assert summary["sent"] == 0
    assert len(telegram_api.sent_messages) == message_count

    completed_payload = {
        "progress": {
            "graph_id": "demo-graph",
            "completed_percent": 100,
            "completed_nodes": 2,
            "ready_nodes": 0,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "next_action": "complete task graph",
            "node_states": {"A": {"state": "completed"}, "B": {"state": "completed"}},
        },
        "blockers": [],
    }
    summary = trigger_graph_watch_notifications(
        config,
        repo_name="ait",
        state_store=service.state_store,
        telegram_api=telegram_api,
        progress_reader=lambda graph: completed_payload,
    )

    assert summary["sent"] == 1
    assert telegram_api.sent_messages[-1][1] == "start B —> complete task graph (100%)"


def test_watchgraph_command_includes_graph_run_state_when_present(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    ait_api.task_dag_progress_payload = {
        "progress": {
            "graph_id": "demo-graph",
            "completed_percent": 0,
            "completed_nodes": 0,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 1,
            "next_action": "start A",
            "node_states": {"A": {"state": "ready"}, "B": {"state": "blocked"}},
        },
        "latest_graph_run": {
            "session_id": "S-RUN",
            "session_local_id": "0001",
            "graph_run_id": "graph-run-1",
            "execution_state": "active",
            "next_action": "start A",
            "gate_handoff": {
                "kind": "converged_gate_bundle",
                "required_gates": ["review", "attestation", "policy", "land"],
                "promotion_required": True,
            },
        },
        "blockers": [],
    }
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": f"/watchgraph PL-TEST123 {graph_path}",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    start_message = telegram_api.sent_messages[-1][1]
    assert "execution_state=active" in start_message
    assert "run_session=S-RUN" in start_message
    assert "run_session_local_id=0001" in start_message
    assert "run_next_action=start A" in start_message
    assert "gate_handoff=converged_gate_bundle" in start_message
    assert "required_gates=review,attestation,policy,land" in start_message
    assert "promotion_required=true" in start_message


def test_task_dag_trigger_sends_graph_run_state_changes(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    ait_api.task_dag_progress_payload = {
        "progress": {
            "graph_id": "demo-graph",
            "completed_percent": 0,
            "completed_nodes": 0,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 1,
            "next_action": "start A",
            "node_states": {"A": {"state": "ready"}, "B": {"state": "blocked"}},
        },
        "latest_graph_run": {
            "session_id": "S-RUN",
            "session_local_id": "0001",
            "graph_run_id": "graph-run-1",
            "execution_state": "active",
            "next_action": "start A",
        },
        "blockers": [],
    }
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": f"/watchgraph PL-TEST123 {graph_path}",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    message_count = len(telegram_api.sent_messages)

    waiting_payload = {
        "progress": {
            "graph_id": "demo-graph",
            "completed_percent": 0,
            "completed_nodes": 0,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 1,
            "next_action": "start A",
            "node_states": {"A": {"state": "ready"}, "B": {"state": "blocked"}},
        },
        "latest_graph_run": {
            "session_id": "S-RUN",
            "session_local_id": "0001",
            "graph_run_id": "graph-run-1",
            "execution_state": "waiting_for_review",
            "pause_reason": "review",
            "next_action": "start A",
            "gate_handoff": {
                "kind": "converged_gate_bundle",
                "required_gates": ["review", "attestation", "policy", "land"],
                "promotion_required": False,
            },
        },
        "blockers": [],
    }
    summary = trigger_graph_watch_notifications(
        config,
        repo_name="ait",
        state_store=service.state_store,
        telegram_api=telegram_api,
        progress_reader=lambda graph: waiting_payload,
    )

    assert summary["sent"] == 1
    assert len(telegram_api.sent_messages) == message_count + 1
    assert telegram_api.sent_messages[-1][1] == "A —> A (0%) · state=waiting_for_review · pause=review · gate=converged_gate_bundle"


def test_task_dag_trigger_falls_back_from_stale_active_graph_run_action(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    ait_api.task_dag_progress_payload = {
        "progress": {
            "graph_id": "demo-graph",
            "completed_percent": 0,
            "completed_nodes": 0,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 1,
            "next_action": "start A",
            "node_states": {"A": {"state": "ready"}, "B": {"state": "blocked"}},
        },
        "latest_graph_run": {
            "session_id": "S-RUN",
            "session_local_id": "0001",
            "graph_run_id": "graph-run-1",
            "execution_state": "active",
            "next_action": "start A",
            "workflow_summary": {
                "completed_nodes": 0,
                "ready_nodes": 1,
                "running_nodes": 0,
                "blocked_nodes": 1,
                "total_nodes": 2,
                "next_action": "start A",
            },
        },
        "blockers": [],
    }
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": f"/watchgraph PL-TEST123 {graph_path}",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    message_count = len(telegram_api.sent_messages)

    stale_active_payload = {
        "progress": {
            "graph_id": "demo-graph",
            "completed_percent": 50,
            "completed_nodes": 1,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "next_action": "start B",
            "node_states": {"A": {"state": "completed"}, "B": {"state": "ready"}},
        },
        "latest_graph_run": {
            "session_id": "S-RUN",
            "session_local_id": "0001",
            "graph_run_id": "graph-run-1",
            "execution_state": "active",
            "next_action": "start A",
            "workflow_summary": {
                "completed_nodes": 0,
                "ready_nodes": 1,
                "running_nodes": 0,
                "blocked_nodes": 1,
                "total_nodes": 2,
                "next_action": "start A",
            },
        },
        "blockers": [],
    }
    summary = trigger_graph_watch_notifications(
        config,
        repo_name="ait",
        state_store=service.state_store,
        telegram_api=telegram_api,
        progress_reader=lambda graph: stale_active_payload,
    )

    assert summary["sent"] == 1
    assert len(telegram_api.sent_messages) == message_count + 1
    assert telegram_api.sent_messages[-1][1] == "A —> B (50%) · state=active"


def test_background_sync_does_not_poll_graph_watches_by_default(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": f"/watchgraph PL-TEST123 {graph_path}",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    message_count = len(telegram_api.sent_messages)
    initial_progress_calls = len(ait_api.task_dag_progress_calls)
    ait_api.task_dag_progress_payload = {
        "progress": {
            "graph_id": "demo-graph",
            "completed_percent": 50,
            "estimated_percent": 87,
            "completed_nodes": 1,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "next_action": "start B",
            "node_states": {"A": {"state": "completed"}, "B": {"state": "ready"}},
        },
        "blockers": [],
    }

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert service.config.graph_watch_background_sweep_enabled is False
    assert len(ait_api.task_dag_progress_calls) == initial_progress_calls
    assert len(telegram_api.sent_messages) == message_count


def test_background_sync_sweeps_graph_watch_progress_changes(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    config = BotConfig(
        **{
            **_config(state_path).__dict__,
            "background_sync_enabled": True,
            "graph_watch_background_sweep_enabled": True,
        }
    )
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": f"/watchgraph PL-TEST123 {graph_path}",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    message_count = len(telegram_api.sent_messages)
    initial_progress_calls = len(ait_api.task_dag_progress_calls)
    ait_api.task_dag_progress_payload = {
        "progress": {
            "graph_id": "demo-graph",
            "completed_percent": 50,
            "completed_nodes": 1,
            "ready_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 0,
            "next_action": "start B",
            "node_states": {"A": {"state": "completed"}, "B": {"state": "ready"}},
        },
        "blockers": [],
    }

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(ait_api.task_dag_progress_calls) == initial_progress_calls + 1
    assert len(telegram_api.sent_messages) == message_count + 1
    update_message = telegram_api.sent_messages[-1][1]
    assert update_message == "A —> B (50%)"
    assert "87" not in update_message
    assert "repo=" not in update_message
    assert "plan=PL-TEST123" not in update_message
    assert "graph=demo-graph" not in update_message

    message_count = len(telegram_api.sent_messages)
    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(telegram_api.sent_messages) == message_count


def test_background_sync_warns_once_per_missing_graph_watch_streak(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    config = BotConfig(
        **{
            **_config(state_path).__dict__,
            "background_sync_enabled": True,
            "graph_watch_background_sweep_enabled": True,
        }
    )
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": f"/watchgraph PL-TEST123 {graph_path}",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    graph_path.unlink()
    message_count = len(telegram_api.sent_messages)

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(telegram_api.sent_messages) == message_count + 1
    assert telegram_api.sent_messages[-1][1] == (
        "ait graph watch missing · repo=ait\n"
        "plan=PL-TEST123\n"
        f"file={graph_path}\n"
        "missing_streak=1\n"
        "The watch stays registered; restore the graph file or disable the watch when it is no longer needed."
    )
    link = service.state_store.get_chat(123)
    assert link["graph_watches"]["PL-TEST123"]["missing_graph_count"] == 1

    message_count = len(telegram_api.sent_messages)
    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(telegram_api.sent_messages) == message_count
    link = service.state_store.get_chat(123)
    assert link["graph_watches"]["PL-TEST123"]["missing_graph_count"] == 2
    assert "PL-TEST123" in link["graph_watches"]


def test_watchgraph_status_reports_missing_graph_watch_streak(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    config = BotConfig(
        **{
            **_config(state_path).__dict__,
            "background_sync_enabled": True,
            "graph_watch_background_sweep_enabled": True,
        }
    )
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": f"/watchgraph PL-TEST123 {graph_path}",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    graph_path.unlink()

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    service.handle_update(
        {
            "update_id": 3,
            "message": {
                "message_id": 12,
                "text": "/watchgraph status",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    status_text = telegram_api.sent_messages[-1][1]
    assert "missing_file_behavior=warn_once_per_streak" in status_text
    assert f"- PL-TEST123 · {graph_path}" in status_text
    assert "missing_streak=1" in status_text


def test_trigger_graph_watch_notifications_warns_once_for_missing_file(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    config = BotConfig(**{**_config(state_path).__dict__, "background_sync_enabled": True})
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": f"/watchgraph PL-TEST123 {graph_path}",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    graph_path.unlink()
    message_count = len(telegram_api.sent_messages)

    summary = trigger_graph_watch_notifications(
        config,
        progress_reader=ait_api.read_task_dag_progress,
        state_store=service.state_store,
        telegram_api=telegram_api,
    )

    assert summary["missing_files"] == 1
    assert len(telegram_api.sent_messages) == message_count + 1
    assert telegram_api.sent_messages[-1][1].startswith("ait graph watch missing · repo=ait\nplan=PL-TEST123\n")
    link = service.state_store.get_chat(123)
    assert link["graph_watches"]["PL-TEST123"]["missing_graph_count"] == 1


def test_background_sync_reports_graph_watch_generic_errors_per_watch(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    config = BotConfig(
        **{
            **_config(state_path).__dict__,
            "background_sync_enabled": True,
            "graph_watch_background_sweep_enabled": True,
        }
    )
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "/session",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    service.handle_update(
        {
            "update_id": 2,
            "message": {
                "message_id": 11,
                "text": f"/watchgraph PL-TEST123 {graph_path}",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    message_count = len(telegram_api.sent_messages)

    def boom(_graph: dict[str, Any]) -> dict[str, Any]:
        raise ValueError("boom generic")

    ait_api.read_task_dag_progress = boom  # type: ignore[assignment]

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(telegram_api.sent_messages) == message_count + 1
    assert telegram_api.sent_messages[-1][1] == (
        "ait graph watch error · repo=ait\n"
        "plan=PL-TEST123\n"
        "boom generic"
    )

    message_count = len(telegram_api.sent_messages)
    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(telegram_api.sent_messages) == message_count


def test_background_sync_sends_new_session_events(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    config = _config(state_path)
    service = TelegramBotService(
        config,
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Hello from Telegram",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    session_id = service.state_store.get_chat(123)["session_id"]
    ait_api.append_session_event(
        session_id,
        event_type="web.note",
        payload={"source": "web", "text": "Background update from web."},
        actor_identity="alice@example.com",
        actor_type="human",
    )

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert "Background update from web." in telegram_api.sent_messages[-1][1]
    assert service.state_store.get_chat(123)["last_synced_sequence"] == 3


def test_background_sync_replays_undelivered_telegram_live_replies(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi(reply_text="Recovered by sweep.")
    service = TelegramBotService(
        _config(state_path, background_sync_enabled=True),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )
    session = ait_api.create_session(chat_id="123", chat_title="Wei", chat_type="private")
    service.state_store.upsert_chat(
        123,
        session_id=session["session_id"],
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        last_synced_sequence=0,
    )
    ait_api.create_telegram_turn(
        session["session_id"],
        text="Lost live delivery",
        chat_id="123",
        chat_title="Wei",
        chat_type="private",
        telegram_message_id=10,
        telegram_message_ids=[10],
        actor_identity="telegram:456:@weita",
    )

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert telegram_api.sent_messages == [("123", "Recovered by sweep.")]
    assert service.state_store.get_chat(123)["last_synced_sequence"] == 2
    assert service.state_store.get_chat(123)["telegram_live_delivered_sequences"] == [2]

    message_count = len(telegram_api.sent_messages)
    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(telegram_api.sent_messages) == message_count


def test_background_sync_is_silent_when_no_new_events(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Hello from Telegram",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )
    message_count = len(telegram_api.sent_messages)

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert len(telegram_api.sent_messages) == message_count


def test_background_sync_applies_retry_backoff_after_consecutive_retryable_failures(tmp_path: Path, monkeypatch):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()

    class FlakyBackgroundSyncAitApi(FakeAitApi):
        def __init__(self):
            super().__init__()
            self.read_attempts = 0

        def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50):
            self.read_attempts += 1
            raise telegram_app.BotRuntimeError(
                f"GET http://127.0.0.1:8088/v1/native/sessions/{session_id}/events failed: [Errno 61] Connection refused"
            ) from ConnectionRefusedError(61, "Connection refused")

    ait_api = FlakyBackgroundSyncAitApi()
    state_store = TelegramSyncStateStore(state_path)
    session = ait_api.create_session(chat_id="123", chat_title="Wei", chat_type="private")
    state_store.upsert_chat(
        123,
        session_id=session["session_id"],
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        last_synced_sequence=0,
    )
    service = TelegramBotService(
        _config(state_path, background_sync_enabled=True),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=state_store,
    )
    current_time = {"now": 1000.0}
    monkeypatch.setattr(telegram_app, "TELEGRAM_BACKGROUND_SYNC_FAILURE_BACKOFF_THRESHOLD", 1)
    monkeypatch.setattr(telegram_app, "TELEGRAM_BACKGROUND_SYNC_FAILURE_BACKOFF_BASE_SECONDS", 15.0)
    monkeypatch.setattr(telegram_app, "TELEGRAM_BACKGROUND_SYNC_FAILURE_BACKOFF_MAX_SECONDS", 120.0)
    monkeypatch.setattr(telegram_app.time, "time", lambda: current_time["now"])

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    link = state_store.get_chat(123)
    assert ait_api.read_attempts == 1
    assert link["background_sync_failure_streak"] == 1
    assert link["background_sync_retry_after_epoch"] == 1015.0

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert ait_api.read_attempts == 1

    current_time["now"] = 1016.0
    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    assert ait_api.read_attempts == 2


def test_background_sync_proactively_relinks_missing_session_to_fresh_session(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    missing_session_id = "AITS-MISSING-0001"

    class MissingSessionAitApi(FakeAitApi):
        def get_session(self, session_id: str, *, actor_identity: str = "ait-agent-telegram", runtime_snapshot=None):
            if session_id not in self.sessions:
                raise telegram_app.BotRuntimeError("session not found on current backend")
            return super().get_session(session_id, actor_identity=actor_identity)

        def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50):
            if session_id == missing_session_id:
                raise telegram_app.BotRuntimeError(
                    f"GET http://127.0.0.1:8088/v1/native/sessions/{session_id}/events?after_sequence={after_sequence}&limit={limit} "
                    f"failed: 404 {{\"detail\":\"'Unknown session: {session_id}'\"}}"
                )
            return super().list_session_events(session_id, after_sequence=after_sequence, limit=limit)

    state_store = TelegramSyncStateStore(state_path)
    state_store.upsert_chat(
        123,
        session_id=missing_session_id,
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id=missing_session_id,
        binding_role="primary_shared",
        last_synced_sequence=4,
    )
    service = TelegramBotService(
        _config(state_path, background_sync_enabled=True),
        ait_api=MissingSessionAitApi(),
        telegram_api=telegram_api,
        state_store=state_store,
    )

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    link = state_store.get_chat(123)
    assert link["session_id"] == "AITS-TEST-0001"
    assert link["canonical_session_id"] == "AITS-TEST-0001"
    assert not str(link.get("branch_session_id") or "").strip()
    assert link["previous_session_id"] == missing_session_id
    assert link["relink_reason"] == "session_missing"
    assert len(telegram_api.sent_messages) == 0

    futures = service.run_background_sync_once()
    assert len(futures) == 1
    for future in futures:
        future.result(timeout=1.0)
    assert len(telegram_api.sent_messages) == 0


def test_background_sync_keeps_workflow_notifications_after_missing_session_relink(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    missing_session_id = "AITS-MISSING-0001"

    class MissingSessionAitApi(FakeAitApi):
        def get_session(self, session_id: str, *, actor_identity: str = "ait-agent-telegram", runtime_snapshot=None):
            if session_id not in self.sessions:
                raise telegram_app.BotRuntimeError("session not found on current backend")
            return super().get_session(session_id, actor_identity=actor_identity)

        def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50):
            if session_id == missing_session_id:
                raise telegram_app.BotRuntimeError(
                    f"GET http://127.0.0.1:8088/v1/native/sessions/{session_id}/events?after_sequence={after_sequence}&limit={limit} "
                    f"failed: 404 {{\"detail\":\"'Unknown session: {session_id}'\"}}"
                )
            return super().list_session_events(session_id, after_sequence=after_sequence, limit=limit)

    ait_api = MissingSessionAitApi()
    ait_api.task_queue_payload = {
        "summary": {"active": 1, "attention_required": 0, "ready_to_land": 1, "ready_to_complete": 0},
        "items": [
            {
                "task": {"task_id": "AITT-2001", "title": "Ready task"},
                "workflow": {"state": "ready_to_land", "reason": "Patchset is landable."},
                "next_action": {"code": "land_change", "label": "Land change", "detail": "Submit land for the selected patchset."},
            }
        ],
    }
    state_store = TelegramSyncStateStore(state_path)
    state_store.upsert_chat(
        123,
        session_id=missing_session_id,
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id=missing_session_id,
        binding_role="primary_shared",
        workflow_notifications_enabled=True,
        last_synced_sequence=4,
    )
    service = TelegramBotService(
        _config(state_path, background_sync_enabled=True),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=state_store,
    )

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    link = state_store.get_chat(123)
    assert link["session_id"] == "AITS-TEST-0001"
    assert link["canonical_session_id"] == "AITS-TEST-0001"
    assert link["previous_session_id"] == missing_session_id
    assert link["relink_reason"] == "session_missing"
    assert link["workflow_notifications_enabled"] is True
    assert link["background_sync_failure_streak"] == 0
    assert link.get("background_sync_last_error") is None
    assert "Ready to land" in telegram_api.sent_messages[-1][1]

    message_count = len(telegram_api.sent_messages)
    futures = service.run_background_sync_once()
    assert len(futures) == 1
    for future in futures:
        future.result(timeout=1.0)
    assert len(telegram_api.sent_messages) == message_count


def test_background_sync_keeps_graph_watch_sweeps_after_missing_session_relink(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "graph_id": "demo-graph",
                "repo_name": "ait",
                "source_plan": {"plan_id": "PL-TEST123"},
                "nodes": [{"node_id": "A", "node_kind": "task", "title": "A"}],
            }
        ),
        encoding="utf-8",
    )
    telegram_api = FakeTelegramApi()
    missing_session_id = "AITS-MISSING-0001"

    class MissingSessionAitApi(FakeAitApi):
        def get_session(self, session_id: str, *, actor_identity: str = "ait-agent-telegram", runtime_snapshot=None):
            if session_id not in self.sessions:
                raise telegram_app.BotRuntimeError("session not found on current backend")
            return super().get_session(session_id, actor_identity=actor_identity)

        def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50):
            if session_id == missing_session_id:
                raise telegram_app.BotRuntimeError(
                    f"GET http://127.0.0.1:8088/v1/native/sessions/{session_id}/events?after_sequence={after_sequence}&limit={limit} "
                    f"failed: 404 {{\"detail\":\"'Unknown session: {session_id}'\"}}"
                )
            return super().list_session_events(session_id, after_sequence=after_sequence, limit=limit)

    state_store = TelegramSyncStateStore(state_path)
    state_store.upsert_chat(
        123,
        session_id=missing_session_id,
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id=missing_session_id,
        binding_role="primary_shared",
        graph_watches={"PL-TEST123": {"plan_id": "PL-TEST123", "graph_path": str(graph_path)}},
        last_synced_sequence=4,
    )
    service = TelegramBotService(
        _config(
            state_path,
            background_sync_enabled=True,
            graph_watch_background_sweep_enabled=True,
        ),
        ait_api=MissingSessionAitApi(),
        telegram_api=telegram_api,
        state_store=state_store,
    )

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    link = state_store.get_chat(123)
    assert link["session_id"] == "AITS-TEST-0001"
    assert link["canonical_session_id"] == "AITS-TEST-0001"
    assert "start A" in telegram_api.sent_messages[-1][1]

    graph_path.unlink()
    futures = service.run_background_sync_once()
    assert len(futures) == 1
    for future in futures:
        future.result(timeout=1.0)

    assert telegram_api.sent_messages[-1][1] == (
        "ait graph watch missing · repo=ait\n"
        "plan=PL-TEST123\n"
        f"file={graph_path}\n"
        "missing_streak=1\n"
        "The watch stays registered; restore the graph file or disable the watch when it is no longer needed."
    )
    link = state_store.get_chat(123)
    assert link["graph_watches"]["PL-TEST123"]["missing_graph_count"] == 1


def test_background_sync_restores_canonical_session_after_missing_branch_session(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    missing_branch_session_id = "AITS-BRANCH-MISSING-0001"

    class MissingBranchSessionAitApi(FakeAitApi):
        def get_session(self, session_id: str, *, actor_identity: str = "ait-agent-telegram", runtime_snapshot=None):
            if session_id == missing_branch_session_id:
                raise telegram_app.BotRuntimeError("session not found on current backend")
            return super().get_session(session_id, actor_identity=actor_identity)

        def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50):
            if session_id == missing_branch_session_id:
                raise telegram_app.BotRuntimeError(
                    f"GET http://127.0.0.1:8088/v1/native/sessions/{session_id}/events?after_sequence={after_sequence}&limit={limit} "
                    f"failed: 404 {{\"detail\":\"'Unknown session: {session_id}'\"}}"
                )
            return super().list_session_events(session_id, after_sequence=after_sequence, limit=limit)

    ait_api = MissingBranchSessionAitApi()
    canonical_session = ait_api.create_session(chat_id="123", chat_title="Wei", chat_type="private")
    state_store = TelegramSyncStateStore(state_path)
    state_store.upsert_chat(
        123,
        session_id=missing_branch_session_id,
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id=canonical_session["session_id"],
        branch_session_id=missing_branch_session_id,
        binding_role="branch",
        last_synced_sequence=4,
    )
    service = TelegramBotService(
        _config(state_path, background_sync_enabled=True),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=state_store,
    )

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    link = state_store.get_chat(123)
    assert link["session_id"] == canonical_session["session_id"]
    assert link["canonical_session_id"] == canonical_session["session_id"]
    assert not str(link.get("branch_session_id") or "").strip()
    assert link["previous_session_id"] == missing_branch_session_id
    assert link["relink_reason"] == "branch_session_missing"


def test_background_sync_creates_fresh_session_when_branch_and_canonical_sessions_are_missing(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    missing_branch_session_id = "AITS-BRANCH-MISSING-0001"
    missing_canonical_session_id = "AITS-CANONICAL-MISSING-0001"

    class MissingBranchAndCanonicalAitApi(FakeAitApi):
        def get_session(self, session_id: str, *, actor_identity: str = "ait-agent-telegram", runtime_snapshot=None):
            if session_id in {missing_branch_session_id, missing_canonical_session_id}:
                raise telegram_app.BotRuntimeError("session not found on current backend")
            return super().get_session(session_id, actor_identity=actor_identity)

        def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50):
            if session_id == missing_branch_session_id:
                raise telegram_app.BotRuntimeError(
                    f"GET http://127.0.0.1:8088/v1/native/sessions/{session_id}/events?after_sequence={after_sequence}&limit={limit} "
                    f"failed: 404 {{\"detail\":\"'Unknown session: {session_id}'\"}}"
                )
            return super().list_session_events(session_id, after_sequence=after_sequence, limit=limit)

    state_store = TelegramSyncStateStore(state_path)
    state_store.upsert_chat(
        123,
        session_id=missing_branch_session_id,
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id=missing_canonical_session_id,
        branch_session_id=missing_branch_session_id,
        binding_role="branch",
        last_synced_sequence=4,
    )
    service = TelegramBotService(
        _config(state_path, background_sync_enabled=True),
        ait_api=MissingBranchAndCanonicalAitApi(),
        telegram_api=telegram_api,
        state_store=state_store,
    )

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    link = state_store.get_chat(123)
    assert link["session_id"] == "AITS-TEST-0001"
    assert link["canonical_session_id"] == "AITS-TEST-0001"
    assert not str(link.get("branch_session_id") or "").strip()
    assert link["previous_session_id"] == missing_branch_session_id
    assert link["relink_reason"] == "session_missing"


def test_missing_session_relink_preserves_previous_session_id_for_next_message(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    missing_session_id = "AITS-MISSING-0001"

    class MissingSessionAitApi(FakeAitApi):
        def get_session(self, session_id: str, *, actor_identity: str = "ait-agent-telegram", runtime_snapshot=None):
            if session_id not in self.sessions:
                raise telegram_app.BotRuntimeError("session not found on current backend")
            return super().get_session(session_id, actor_identity=actor_identity)

        def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50):
            if session_id == missing_session_id:
                raise telegram_app.BotRuntimeError(
                    f"GET http://127.0.0.1:8088/v1/native/sessions/{session_id}/events?after_sequence={after_sequence}&limit={limit} "
                    f"failed: 404 {{\"detail\":\"'Unknown session: {session_id}'\"}}"
                )
            return super().list_session_events(session_id, after_sequence=after_sequence, limit=limit)

    state_store = TelegramSyncStateStore(state_path)
    state_store.upsert_chat(
        123,
        session_id=missing_session_id,
        repo_name="ait",
        chat_type="private",
        chat_title="Wei",
        canonical_session_id=missing_session_id,
        binding_role="primary_shared",
        last_synced_sequence=4,
    )
    ait_api = MissingSessionAitApi(reply_text="Recovered after relink.")
    service = TelegramBotService(
        _config(state_path, background_sync_enabled=True),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=state_store,
    )

    futures = service.run_background_sync_once()
    for future in futures:
        future.result(timeout=1.0)

    link = state_store.get_chat(123)
    assert link["session_id"] == "AITS-TEST-0001"
    assert link["previous_session_id"] == missing_session_id
    assert link["relink_reason"] == "session_missing"

    service.handle_update(
        {
            "update_id": 1,
            "message": {
                "message_id": 10,
                "text": "Continue after relink",
                "chat": {"id": 123, "type": "private", "first_name": "Wei"},
                "from": {"id": 456, "username": "weita"},
            },
        }
    )

    link = state_store.get_chat(123)
    assert link["session_id"] == "AITS-TEST-0001"
    assert link["previous_session_id"] == missing_session_id
    assert link["relink_reason"] == "session_missing"
    assert telegram_api.sent_messages[-1] == (123, "Recovered after relink.")


def test_service_submit_update_merges_bursty_text_into_one_logical_turn(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path, turn_merge_window_seconds=0.05, turn_merge_max_messages=4),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    first_update = {
        "update_id": 1,
        "message": {
            "message_id": 10,
            "text": "First request",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }
    second_update = {
        "update_id": 2,
        "message": {
            "message_id": 11,
            "text": "with one more detail",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }

    future1 = service.submit_update(first_update)
    time.sleep(0.01)
    future2 = service.submit_update(second_update)
    future1.result(timeout=1.0)
    future2.result(timeout=1.0)
    assert service.wait_for_idle(timeout=1.0)

    assert len(ait_api.turn_calls) == 1
    assert ait_api.turn_calls[0]["text"] == "First request\n\nwith one more detail"
    assert ait_api.turn_calls[0]["telegram_message_id"] == 11
    assert ait_api.turn_calls[0]["telegram_message_ids"] == [10, 11]
    assert ait_api.turn_calls[0]["transport_envelope"]["message"]["message_ids"] == [10, 11]
    assert ait_api.turn_calls[0]["transport_envelope"]["message"]["logical_turn_message_count"] == 2
    session_id = service.state_store.get_chat(123)["session_id"]
    assert ait_api.events[session_id][-2]["payload"]["logical_turn_message_count"] == 2
    assert ait_api.events[session_id][-2]["payload"]["telegram_message_ids"] == [10, 11]
    assert telegram_api.sent_messages == [(123, "AI says hello.")]


def test_service_submit_update_command_marks_a_merge_boundary(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path, turn_merge_window_seconds=0.05, turn_merge_max_messages=4),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    text_update = {
        "update_id": 1,
        "message": {
            "message_id": 10,
            "text": "Need a quick summary",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }
    status_update = {
        "update_id": 2,
        "message": {
            "message_id": 11,
            "text": "/status",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }

    future1 = service.submit_update(text_update)
    time.sleep(0.01)
    future2 = service.submit_update(status_update)
    future1.result(timeout=1.0)
    future2.result(timeout=1.0)
    assert service.wait_for_idle(timeout=1.0)

    assert len(ait_api.turn_calls) == 1
    assert ait_api.turn_calls[0]["text"] == "Need a quick summary"
    assert telegram_api.sent_messages[0] == (123, "AI says hello.")
    assert "ait Telegram status" in telegram_api.sent_messages[1][1]


def test_service_submit_update_does_not_merge_different_group_actors(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()
    ait_api = FakeAitApi()
    service = TelegramBotService(
        _config(state_path, turn_merge_window_seconds=0.05, turn_merge_max_messages=4),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    first_update = {
        "update_id": 1,
        "message": {
            "message_id": 10,
            "text": "@ait_test_bot first group question",
            "chat": {"id": 999, "type": "group", "title": "AIT room"},
            "from": {"id": 456, "username": "weita"},
        },
    }
    second_update = {
        "update_id": 2,
        "message": {
            "message_id": 11,
            "text": "@ait_test_bot second group question",
            "chat": {"id": 999, "type": "group", "title": "AIT room"},
            "from": {"id": 789, "username": "other"},
        },
    }

    future1 = service.submit_update(first_update)
    time.sleep(0.01)
    future2 = service.submit_update(second_update)
    future1.result(timeout=1.0)
    future2.result(timeout=1.0)
    assert service.wait_for_idle(timeout=1.0)

    assert [call["text"] for call in ait_api.turn_calls] == ["first group question", "second group question"]
    assert [call["actor_identity"] for call in ait_api.turn_calls] == ["telegram:456:@weita", "telegram:789:@other"]
    assert [message for _, message in telegram_api.sent_messages] == ["AI says hello.", "AI says hello."]


def test_service_submit_update_serializes_same_chat_but_allows_other_chats(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()

    class BlockingAitApi(FakeAitApi):
        def __init__(self):
            super().__init__()
            self.first_chat_started = threading.Event()
            self.release_first_chat = threading.Event()
            self.other_chat_finished = threading.Event()
            self.started_chat_ids: list[str] = []

        def create_telegram_turn(self, session_id: str, **kwargs) -> dict:
            chat_id = str(kwargs["chat_id"])
            self.started_chat_ids.append(chat_id)
            if chat_id == "123" and not self.first_chat_started.is_set():
                self.first_chat_started.set()
                assert self.release_first_chat.wait(timeout=2.0)
            result = super().create_telegram_turn(session_id, **kwargs)
            if chat_id == "456":
                self.other_chat_finished.set()
            return result

    ait_api = BlockingAitApi()
    service = TelegramBotService(
        _config(state_path, turn_merge_window_seconds=0.0, turn_merge_max_messages=1),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    first_update = {
        "update_id": 1,
        "message": {
            "message_id": 10,
            "text": "First blocking message",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }
    second_same_chat = {
        "update_id": 2,
        "message": {
            "message_id": 11,
            "text": "Second queued message",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }
    other_chat_update = {
        "update_id": 3,
        "message": {
            "message_id": 12,
            "text": "Other chat message",
            "chat": {"id": 456, "type": "private", "first_name": "Other"},
            "from": {"id": 789, "username": "other"},
        },
    }

    future1 = service.submit_update(first_update)
    assert ait_api.first_chat_started.wait(timeout=1.0)
    future1.result(timeout=1.0)
    future2 = service.submit_update(second_same_chat)
    future3 = service.submit_update(other_chat_update)
    future2.result(timeout=1.0)
    future3.result(timeout=1.0)
    assert ait_api.other_chat_finished.wait(timeout=1.0)
    assert ait_api.started_chat_ids == ["123", "456"]

    ait_api.release_first_chat.set()
    assert service.wait_for_idle(timeout=1.0)

    assert ait_api.started_chat_ids == ["123", "456", "123"]
    assert sorted(call["chat_id"] for call in ait_api.turn_calls) == ["123", "123", "456"]


def test_service_submit_update_reports_unexpected_errors_and_continues(tmp_path: Path):
    state_path = tmp_path / "telegram-sync.json"
    telegram_api = FakeTelegramApi()

    class FlakyQueueAitApi(FakeAitApi):
        def __init__(self):
            super().__init__()
            self.fail_queue_once = True

        def read_task_queue(self):
            if self.fail_queue_once:
                self.fail_queue_once = False
                raise RuntimeError("queue exploded")
            return super().read_task_queue()

    ait_api = FlakyQueueAitApi()
    service = TelegramBotService(
        _config(state_path),
        ait_api=ait_api,
        telegram_api=telegram_api,
        state_store=TelegramSyncStateStore(state_path),
    )

    queue_update = {
        "update_id": 1,
        "message": {
            "message_id": 10,
            "text": "queue",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }
    normal_update = {
        "update_id": 2,
        "message": {
            "message_id": 11,
            "text": "Hello after failure",
            "chat": {"id": 123, "type": "private", "first_name": "Wei"},
            "from": {"id": 456, "username": "weita"},
        },
    }

    service.submit_update(queue_update).result(timeout=1.0)
    assert "unexpected error" in telegram_api.sent_messages[-1][1].lower()

    service.submit_update(normal_update).result(timeout=1.0)
    assert service.wait_for_idle(timeout=1.0)
    assert telegram_api.sent_messages[-1][1] == "AI says hello."


def test_server_telegram_turn_endpoint_appends_user_and_assistant_events(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · Wei",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        assert config.repo_name == "ait"
        assert events[-1]["event_type"] == "telegram.user_message"
        assert chat_id == "123"
        assert chat_title == "Wei"
        assert checkpoint is None
        return AiReplyResult(
            text="Server-side hello.",
            model="gpt-5.4-mini",
            response_id="resp_server_123",
            usage={"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)
    transport_envelope = build_transport_event_envelope(
        transport="telegram",
        actor_identity="telegram:456:@weita",
        actor_transport_id="456",
        actor_username="weita",
        channel_id="123",
        channel_title="Wei",
        channel_kind="private",
        text="hello from telegram",
        message_id=11,
        message_ids=[10, 11],
    )

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={
                "text": "hello from telegram",
                "chat_id": "123",
                "chat_title": "Wei",
                "chat_type": "private",
                "telegram_message_id": 11,
                "telegram_message_ids": [10, 11],
                "transport_envelope": transport_envelope,
            },
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["ok"] is True
    assert payload["reply_text"] == "Server-side hello."
    assert payload["assistant_event"]["event_type"] == "assistant.reply"
    assert payload["checkpoint"]["based_on_sequence"] == 2
    assert payload["telegram_context_runtime"]["reply_context_mode"] == "checkpoint_delta"
    assert payload["telegram_context_runtime"]["checkpoint_freshness"] == "fresh"
    assert payload["telegram_context_runtime"]["delta_event_count"] == 0
    assert payload["checkpoint"]["resume_payload"]["objective"] == "hello from telegram"
    assert payload["checkpoint"]["resume_payload"]["context"]["checkpoint_reason"] == "initial_turn"

    events = list_session_events(ctx, session["session_id"], after_sequence=0, limit=10)
    assert [event["event_type"] for event in events] == ["telegram.user_message", "assistant.reply"]
    assert events[0]["payload"]["telegram_message_ids"] == [10, 11]
    assert events[0]["payload"]["logical_turn_message_count"] == 2
    assert events[0]["payload"]["transport_envelope"]["event_id"] == transport_envelope["event_id"]
    assert events[-1]["payload"]["text"] == "Server-side hello."
    assert events[-1]["payload"]["generated_via"] == "ait_server"
    assert events[-1]["payload"]["transport_reply_envelope"]["transport"] == "telegram"
    assert events[-1]["payload"]["transport_reply_envelope"]["reply_to"]["event_id"] == transport_envelope["event_id"]
    checkpoints = list_session_checkpoints(ctx, session["session_id"])
    assert len(checkpoints) == 1
    assert get_session(ctx, session["session_id"])["head_checkpoint_id"] == payload["checkpoint"]["checkpoint_id"]


def test_server_telegram_turn_endpoint_reuses_existing_reply_for_duplicate_transport_event(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · Wei",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )
    generation_calls: list[str] = []

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        generation_calls.append(str(events[-1]["event_type"]))
        return AiReplyResult(
            text="Server-side hello.",
            model="gpt-5.4-mini",
            response_id="resp_server_retry_123",
            usage={"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)
    transport_envelope = build_transport_event_envelope(
        transport="telegram",
        actor_identity="telegram:456:@weita",
        actor_transport_id="456",
        actor_username="weita",
        channel_id="123",
        channel_title="Wei",
        channel_kind="private",
        text="hello from telegram",
        message_id=11,
        message_ids=[11],
    )
    request_payload = {
        "text": "hello from telegram",
        "chat_id": "123",
        "chat_title": "Wei",
        "chat_type": "private",
        "telegram_message_id": 11,
        "telegram_message_ids": [11],
        "transport_envelope": transport_envelope,
    }

    with TestClient(server_app.create_app()) as client:
        first = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json=request_payload,
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )
        second = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json=request_payload,
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    assert first.status_code == 200
    assert second.status_code == 200
    assert first.json()["reply_text"] == "Server-side hello."
    assert second.json()["reply_text"] == "Server-side hello."
    assert generation_calls == ["telegram.user_message"]

    events = list_session_events(ctx, session["session_id"], after_sequence=0, limit=10)
    assert [event["event_type"] for event in events] == ["telegram.user_message", "assistant.reply"]


def test_server_telegram_turn_endpoint_reuses_existing_user_event_when_retry_arrives_after_append(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · Wei",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )
    transport_envelope = build_transport_event_envelope(
        transport="telegram",
        actor_identity="telegram:456:@weita",
        actor_transport_id="456",
        actor_username="weita",
        channel_id="123",
        channel_title="Wei",
        channel_kind="private",
        text="hello from telegram",
        message_id=11,
        message_ids=[11],
    )
    existing_user_event = append_session_event(
        ctx,
        session["session_id"],
        "telegram.user_message",
        {
            "source": "telegram",
            "text": "hello from telegram",
            "telegram_chat_id": "123",
            "telegram_chat_title": "Wei",
            "telegram_chat_type": "private",
            "telegram_message_id": 11,
            "telegram_message_ids": [11],
            "logical_turn_message_count": 1,
            "transport_envelope": transport_envelope,
        },
        actor_identity="telegram:456:@weita",
        actor_type="telegram_user",
    )
    seen_user_sequences: list[int] = []

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        seen_user_sequences.append(int(events[-1]["sequence"]))
        return AiReplyResult(
            text="Recovered after append-only retry.",
            model="gpt-5.4-mini",
            response_id="resp_server_retry_after_append",
            usage={"input_tokens": 8, "output_tokens": 4, "total_tokens": 12},
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={
                "text": "hello from telegram",
                "chat_id": "123",
                "chat_title": "Wei",
                "chat_type": "private",
                "telegram_message_id": 11,
                "telegram_message_ids": [11],
                "transport_envelope": transport_envelope,
            },
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["reply_text"] == "Recovered after append-only retry."
    assert seen_user_sequences == [int(existing_user_event["sequence"])]

    events = list_session_events(ctx, session["session_id"], after_sequence=0, limit=10)
    assert [event["event_type"] for event in events] == ["telegram.user_message", "assistant.reply"]
    assert int(events[0]["sequence"]) == int(existing_user_event["sequence"])


def test_server_telegram_turn_rejects_compact_dag_worker_sessions(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "agent_run",
        title="Compact DAG worker · Wei",
        metadata={
            "session_policy": "task_dag_compact_packet_worker",
            "packet_available": True,
            "compact_packet_surface": {
                "surface_id": "worker_only_compact_ait_dag_packet",
                "packet_generation_required": True,
            },
        },
        actor_identity="weita@example.com",
        actor_type="human",
    )

    monkeypatch.setattr(
        server_app,
        "generate_session_reply",
        lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("compact-worker sessions should not use ait-server live turns")),
    )

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={
                "text": "hello from telegram",
                "chat_id": "123",
                "chat_title": "Wei",
                "chat_type": "private",
                "telegram_message_id": 11,
            },
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    assert response.status_code == 400
    detail = str(response.json()["detail"]).lower()
    assert "compact dag worker session" in detail
    assert "generated locally" in detail
    assert "ait-server live turn routes are disabled" in detail
    assert list_session_events(ctx, session["session_id"], after_sequence=0, limit=10) == []


def test_server_health_and_admin_metrics_report_active_live_turns(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    monkeypatch.setenv("AIT_SERVER_PRESSURE_METRICS_CACHE_TTL_SECONDS", "60")
    session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · Wei",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )
    started = threading.Event()
    release = threading.Event()
    response_holder: dict[str, Any] = {}

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        started.set()
        assert release.wait(timeout=5), "test should release the blocked live turn"
        return AiReplyResult(
            text="Server-side long turn done.",
            model="gpt-5.4-mini",
            response_id="resp_server_live_turn",
            usage={"input_tokens": 12, "output_tokens": 6, "total_tokens": 18},
            turn_analysis={"command_count": 2, "top_commands": [{"command": "rg", "count": 2}]},
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:

        def _post_turn() -> None:
            response_holder["response"] = client.post(
                f"/v1/native/sessions/{session['session_id']}:telegramTurn",
                json={
                    "text": "hello from telegram",
                    "chat_id": "123",
                    "chat_title": "Wei",
                    "chat_type": "private",
                    "telegram_message_id": 11,
                    "telegram_message_ids": [11],
                },
                headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
            )

        worker = threading.Thread(target=_post_turn, daemon=True)
        worker.start()
        assert started.wait(timeout=5), "live turn should be active before probing health surfaces"

        health_payload = client.get("/healthz").json()
        metrics_payload = client.get("/v1/native/admin/metrics").json()
        metrics_cached_payload = client.get("/v1/native/admin/metrics").json()
        readiness_payload = client.get("/v1/native/admin/readiness").json()

        release.set()
        worker.join(timeout=5)
        assert not worker.is_alive(), "turn thread should complete after release"

        response = response_holder["response"]
        assert response.status_code == 200
        assert response.json()["ok"] is True

        health_after = client.get("/healthz").json()
        metrics_after = client.get("/v1/native/admin/metrics").json()

    assert health_payload["live_turn_pressure"]["in_flight_turns"] == 1
    assert health_payload["live_turn_pressure"]["queued_turns"] == 0
    assert health_payload["live_turns"]["active_turns"] == 1
    assert health_payload["pressure_metrics_cache_ttl_seconds"] == 60.0
    assert health_payload["cache_state"] == "computed"
    assert health_payload["cache_ttl_seconds"] == 60.0
    assert metrics_payload["summary"]["active_live_turns"] == 1
    assert metrics_payload["worker_metrics"]["active_live_turns"] == 1
    assert metrics_payload["live_turn_metrics"]["summary"]["active_turns"] == 1
    assert metrics_payload["live_turn_pressure"]["in_flight_turns"] == 1
    assert metrics_payload["repositories"][0]["active_live_turns"] == 1
    assert metrics_payload["cache_state"] == "computed"
    assert metrics_cached_payload["cache_state"] == "cached"
    assert metrics_cached_payload["summary"]["active_live_turns"] == 1
    assert readiness_payload["summary"]["active_live_turns"] == 1
    assert readiness_payload["live_turn_summary"]["active_turns"] == 1
    assert readiness_payload["live_turn_pressure"]["in_flight_turns"] == 1
    assert readiness_payload["cache_state"] == "computed"
    assert health_after["live_turn_pressure"]["in_flight_turns"] == 0
    assert metrics_after["summary"]["active_live_turns"] == 0


def test_server_telegram_turn_endpoint_uses_linked_repo_checkout_for_cross_repo_session(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    primary_root = _write_bound_repo_checkout(
        tmp_path / "ait-root",
        "ait",
        env_lines=["AIT_REPO_NAME=ait"],
    )
    repo_root = _write_bound_repo_checkout(
        tmp_path / "AI_Radio_Engine",
        "AI_Radio_Engine",
        env_lines=["AIT_REPO_NAME=AI_Radio_Engine"],
    )
    monkeypatch.setenv("AIT_REPO_ROOT", str(primary_root))
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(primary_root / ".ait" / "agent-runtime" / "telegram.env"))
    ensure_repository(ctx, "AI_Radio_Engine", "main")
    session = create_session(
        ctx,
        "AI_Radio_Engine",
        "telegram_chat",
        title="Telegram chat · Radio",
        metadata={"source": "telegram", "telegram_chat_id": "321", "telegram_chat_title": "Radio"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        assert config.repo_name == "AI_Radio_Engine"
        assert config.repo_root == repo_root.resolve()
        assert events[-1]["event_type"] == "telegram.user_message"
        assert chat_id == "321"
        return AiReplyResult(
            text="Repo-specific hello.",
            model="gpt-5.4-mini",
            response_id="resp_repo_specific_123",
            usage={"input_tokens": 8, "output_tokens": 4, "total_tokens": 12},
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={
                "text": "hello from radio repo",
                "chat_id": "321",
                "chat_title": "Radio",
                "chat_type": "private",
                "telegram_message_id": 21,
            },
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["ok"] is True
    assert payload["reply_text"] == "Repo-specific hello."


def test_server_session_turn_endpoint_appends_user_and_assistant_events(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "agent_run",
        title="Editor chat · Wei",
        metadata={"source": "vscode"},
        actor_identity="weita@example.com",
        actor_type="human",
    )
    captured: dict[str, Any] = {}

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        captured["checkpoint"] = checkpoint
        captured["events"] = events
        captured["chat_id"] = chat_id
        captured["chat_title"] = chat_title
        captured["surface"] = surface
        assert events[-1]["event_type"] == "session.message"
        return AiReplyResult(
            text="Server-side session hello.",
            model="gpt-5.4-mini",
            response_id="resp_session_turn_123",
            usage={"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
            source="codex",
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:turn",
            json={
                "text": "hello from vscode",
                "surface": "vscode",
                "title": "VSCode Codex",
            },
            headers={"X-AIT-Actor": "weita@example.com", "X-AIT-Actor-Type": "human"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["ok"] is True
    assert payload["reply_text"] == "Server-side session hello."
    assert payload["assistant_event"]["event_type"] == "assistant.reply"
    assert payload["assistant_event"]["payload"]["delivered_via"] == "session_live"
    assert payload["assistant_event"]["payload"]["session_surface"] == "vscode"
    assert payload["assistant_event"]["payload"]["surface_title"] == "VSCode Codex"
    assert payload["surface"] == "vscode"
    assert captured["checkpoint"] is None
    assert captured["surface"] == "vscode"
    assert captured["chat_id"] == session["session_id"]
    assert captured["chat_title"] == "VSCode Codex"

    events = list_session_events(ctx, session["session_id"], after_sequence=0, limit=10)
    assert [event["event_type"] for event in events] == ["session.message", "assistant.reply"]
    assert events[0]["payload"]["source"] == "vscode"
    assert events[0]["payload"]["surface_title"] == "VSCode Codex"
    assert events[-1]["payload"]["text"] == "Server-side session hello."
    assert events[-1]["payload"]["generated_via"] == "ait_server"


def test_server_session_turn_rejects_compact_dag_worker_sessions(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "agent_run",
        title="Compact DAG worker · VSCode",
        metadata={
            "session_policy": "task_dag_compact_packet_worker",
            "packet_available": True,
            "compact_packet_surface": {
                "surface_id": "worker_only_compact_ait_dag_packet",
                "packet_generation_required": True,
            },
        },
        actor_identity="weita@example.com",
        actor_type="human",
    )

    monkeypatch.setattr(
        server_app,
        "generate_session_reply",
        lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("compact-worker sessions should not use ait-server live turns")),
    )

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:turn",
            json={
                "text": "hello from vscode",
                "surface": "vscode",
                "title": "VSCode Codex",
            },
            headers={"X-AIT-Actor": "weita@example.com", "X-AIT-Actor-Type": "human"},
        )

    assert response.status_code == 400
    detail = str(response.json()["detail"]).lower()
    assert "compact dag worker session" in detail
    assert "generated locally" in detail
    assert "ait-server live turn routes are disabled" in detail
    assert list_session_events(ctx, session["session_id"], after_sequence=0, limit=10) == []


def test_server_session_turn_endpoint_persists_transport_envelope_for_line_surface(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "agent_run",
        title="LINE chat · Wei",
        metadata={"source": "line"},
        actor_identity="ait-agent-line",
        actor_type="line_bot",
    )
    transport_envelope = build_transport_event_envelope(
        transport="line",
        actor_identity="line:U-user-1",
        actor_transport_id="U-user-1",
        actor_display_name="U-user-1",
        channel_id="U-user-1",
        channel_title="LINE user · U-user-1",
        channel_kind="user",
        text="hello from line",
        message_id=987654321,
        event_id="01HXLINEEVENT001",
        dedupe_key="01HXLINEEVENT001",
    )

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        assert events[-1]["event_type"] == "session.message"
        assert events[-1]["payload"]["transport_envelope"]["transport"] == "line"
        assert surface == "line"
        return AiReplyResult(
            text="Server-side LINE hello.",
            model="gpt-5.4-mini",
            response_id="resp_line_turn_123",
            usage={"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
            source="codex",
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:turn",
            json={
                "text": "hello from line",
                "surface": "line",
                "title": "LINE user · U-user-1",
                "transport_envelope": transport_envelope,
            },
            headers={"X-AIT-Actor": "line:U-user-1", "X-AIT-Actor-Type": "line_user"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["ok"] is True
    assert payload["assistant_event"]["payload"]["delivered_via"] == "line_live"
    assert payload["assistant_event"]["payload"]["transport_reply_envelope"]["transport"] == "line"
    assert payload["assistant_event"]["payload"]["transport_reply_envelope"]["reply_to"]["event_id"] == "01HXLINEEVENT001"

    events = list_session_events(ctx, session["session_id"], after_sequence=0, limit=10)
    assert [event["event_type"] for event in events] == ["session.message", "assistant.reply"]
    assert events[0]["payload"]["transport_envelope"]["event_id"] == "01HXLINEEVENT001"
    assert events[-1]["payload"]["transport_reply_envelope"]["transport"] == "line"


def test_server_session_turn_endpoint_persists_transport_reply_attachments_for_discord_surface(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "agent_run",
        title="Discord channel · 998877665544332211",
        metadata={"source": "discord"},
        actor_identity="ait-agent-discord",
        actor_type="discord_bot",
    )
    export_path = tmp_path / "AIT_WHITEPAPER_DRAFT.md"
    export_path.write_text("# Whitepaper\n", encoding="utf-8")
    transport_envelope = build_transport_event_envelope(
        transport="discord",
        actor_identity="discord:U-user-1",
        actor_transport_id="U-user-1",
        actor_display_name="U-user-1",
        channel_id="998877665544332211",
        channel_title="Discord channel · 998877665544332211",
        channel_kind="guild_channel",
        text="請上傳白皮書",
        message_id=987654321,
        event_id="discord:998877665544332211:message:987654321",
        dedupe_key="discord:998877665544332211:message:987654321",
    )

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        assert events[-1]["event_type"] == "session.message"
        assert events[-1]["payload"]["transport_envelope"]["transport"] == "discord"
        assert surface == "discord"
        return AiReplyResult(
            text="這是白皮書草稿。",
            attachments=(
                {
                    "kind": "document",
                    "file_name": export_path.name,
                    "mime_type": "text/markdown",
                    "caption": "ait whitepaper draft",
                    "local_path": str(export_path),
                },
            ),
            model="gpt-5.4-mini",
            response_id="resp_discord_turn_123",
            usage={"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
            source="codex",
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:turn",
            json={
                "text": "請上傳白皮書",
                "surface": "discord",
                "title": "Discord channel · 998877665544332211",
                "transport_envelope": transport_envelope,
            },
            headers={"X-AIT-Actor": "discord:U-user-1", "X-AIT-Actor-Type": "discord_user"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["ok"] is True
    assert payload["assistant_event"]["payload"]["delivered_via"] == "discord_live"
    assert payload["assistant_event"]["payload"]["transport_reply_envelope"]["transport"] == "discord"
    assert payload["assistant_event"]["payload"]["transport_reply_envelope"]["message"]["attachments"] == [
        {
            "kind": "document",
            "file_name": "AIT_WHITEPAPER_DRAFT.md",
            "mime_type": "text/markdown",
            "caption": "ait whitepaper draft",
            "local_path": str(export_path),
        }
    ]

    events = list_session_events(ctx, session["session_id"], after_sequence=0, limit=10)
    assert [event["event_type"] for event in events] == ["session.message", "assistant.reply"]
    assert events[-1]["payload"]["transport_reply_envelope"]["transport"] == "discord"
    assert events[-1]["payload"]["transport_reply_envelope"]["message"]["attachments"][0]["local_path"] == str(export_path)


@pytest.mark.parametrize(
    (
        "session_kind",
        "session_metadata",
        "endpoint_suffix",
        "request_payload",
        "expected_event_type",
        "expected_segment_class",
        "expected_classification_source",
        "expected_signal_kinds",
        "expected_boundary_behaviors",
        "expected_attachment_hints",
    ),
    [
        (
            "planning_session_relay",
            {
                "source": "vscode",
                "plan_id": "PL-PLAN-001",
                "planning_session_id": "PS-PLAN-001",
                "graph_id": "workflow-conversation-segmentation/parallel-dispatch",
                "node_id": "A",
            },
            "turn",
            {
                "text": "Please remote sync this planning artifact.",
                "surface": "vscode",
                "title": "VSCode Codex",
            },
            "session.message",
            "planning",
            "boundary_signals",
            ["plan_sync"],
            ["close_after"],
            {
                "plan_id": "PL-PLAN-001",
                "planning_session_id": "PS-PLAN-001",
                "graph_id": "workflow-conversation-segmentation/parallel-dispatch",
                "node_id": "A",
            },
        ),
        (
            "agent_run",
            {
                "source": "vscode",
                "plan_id": "PL-EXEC-001",
                "task_id": "T-EXEC-001",
                "task_graph_json": "docs/sprints/workflow_conversation_segmentation.task_graph.json",
                "graph_run_id": "GR-EXEC-001",
                "node_id": "C",
            },
            "turn",
            {
                "text": "Use ait task start and ait change create for the first execution slice.",
                "surface": "vscode",
                "title": "VSCode Codex",
            },
            "session.message",
            "task_execution",
            "attachment_hints",
            [],
            [],
            {
                "plan_id": "PL-EXEC-001",
                "task_id": "T-EXEC-001",
                "task_graph_json": "docs/sprints/workflow_conversation_segmentation.task_graph.json",
                "graph_run_id": "GR-EXEC-001",
                "node_id": "C",
            },
        ),
        (
            "telegram_chat",
            {
                "source": "telegram",
                "telegram_chat_id": "123",
                "telegram_chat_title": "Wei",
                "task_id": "T-LAND-001",
                "change_id": "AITC-0001",
                "graph_run_id": "GR-LAND-001",
                "node_id": "L",
            },
            "telegramTurn",
            {
                "text": "Please remote land AITC-0001 after policy clears.",
                "chat_id": "123",
                "chat_title": "Wei",
                "chat_type": "private",
                "telegram_message_id": 31,
            },
            "telegram.user_message",
            "change_land",
            "boundary_signals",
            ["land"],
            ["close_after"],
            {
                "task_id": "T-LAND-001",
                "change_id": "AITC-0001",
                "graph_run_id": "GR-LAND-001",
                "node_id": "L",
            },
        ),
    ],
)
def test_server_turn_endpoints_persist_inferred_workflow_context_on_user_events(
    tmp_path: Path,
    monkeypatch,
    session_kind: str,
    session_metadata: dict[str, Any],
    endpoint_suffix: str,
    request_payload: dict[str, Any],
    expected_event_type: str,
    expected_segment_class: str,
    expected_classification_source: str,
    expected_signal_kinds: list[str],
    expected_boundary_behaviors: list[str],
    expected_attachment_hints: dict[str, str],
):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        session_kind,
        title="Workflow context test",
        metadata=session_metadata,
        actor_identity="tester@example.com",
        actor_type="human",
    )

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        return AiReplyResult(text="Workflow reply.", model="gpt-5.4-mini", source="codex")

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:{endpoint_suffix}",
            json=request_payload,
            headers={"X-AIT-Actor": "tester@example.com", "X-AIT-Actor-Type": "human"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["ok"] is True
    assert payload["user_event"]["event_type"] == expected_event_type
    workflow_context = payload["user_event"]["payload"]["workflow_context"]
    assert workflow_context["segment_class"] == expected_segment_class
    assert workflow_context["classification_source"] == expected_classification_source
    assert [row["kind"] for row in workflow_context["signals"]] == expected_signal_kinds
    assert [row["boundary_behavior"] for row in workflow_context["signals"]] == expected_boundary_behaviors
    assert workflow_context["attachment_hints"] == expected_attachment_hints
    assert payload["workflow_segmentation"]["latest_segment"]["segment_class"] == expected_segment_class
    assert payload["workflow_segmentation"]["latest_segment"]["signal_kinds"] == expected_signal_kinds
    assert payload["workflow_segmentation"]["latest_segment"]["durable_objects"]
    assert payload["workflow_segmentation"]["latest_segment"]["attachment_resolution"]["status"] == "attached"

    events = list_session_events(ctx, session["session_id"], after_sequence=0, limit=10)
    assert [event["event_type"] for event in events] == [expected_event_type, "assistant.reply"]
    persisted_workflow_context = events[0]["payload"]["workflow_context"]
    assert persisted_workflow_context == workflow_context


def test_workflow_segments_endpoint_keeps_task_execution_in_the_post_sync_window(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "planning_session_relay",
        title="Workflow segment endpoint",
        metadata={"plan_id": "PL-SEG-001", "planning_session_id": "PS-SEG-001"},
        actor_identity="tester@example.com",
        actor_type="human",
    )
    append_session_event(
        ctx,
        session["session_id"],
        "session.message",
        {
            "text": "Please remote sync this planning artifact.",
            "workflow_context": {
                "segment_class": "planning",
                "classification_source": "boundary_signals",
                "attachment_hints": {"plan_id": "PL-SEG-001", "planning_session_id": "PS-SEG-001"},
                "signals": [
                    {
                        "kind": "plan_sync",
                        "segment_class": "planning",
                        "boundary_behavior": "close_after",
                        "source": "text",
                    }
                ],
            },
        },
        actor_identity="tester@example.com",
        actor_type="human",
    )
    append_session_event(
        ctx,
        session["session_id"],
        "session.message",
        {
            "text": "Use ait task start so execution leaves the planning window.",
            "workflow_context": {
                "segment_class": "task_execution",
                "classification_source": "attachment_hints",
                "attachment_hints": {"task_id": "T-SEG-001"},
                "signals": [],
            },
        },
        actor_identity="tester@example.com",
        actor_type="human",
    )

    with TestClient(server_app.create_app()) as client:
        response = client.get(
            f"/v1/native/sessions/{session['session_id']}/workflow-segments",
            headers={"X-AIT-Actor": "tester@example.com", "X-AIT-Actor-Type": "human"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["segment_count"] == 2
    assert [(segment["sequence_start"], segment["sequence_end"]) for segment in payload["segments"]] == [(1, 1), (2, 2)]
    assert payload["latest_segment"]["segment_class"] == "task_execution"
    assert payload["latest_segment"]["signal_kinds"] == []
    assert payload["latest_segment"]["durable_objects"] == [{"object_type": "task", "object_id": "T-SEG-001"}]
    assert payload["latest_segment"]["attachment_resolution"]["primary_target"] == {"object_type": "task", "object_id": "T-SEG-001"}


def test_server_turn_endpoints_append_task_dag_progress_when_requested(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    editor_session = create_session(
        ctx,
        "ait",
        "agent_run",
        title="Editor chat · DAG",
        metadata={"source": "vscode"},
        actor_identity="weita@example.com",
        actor_type="human",
    )
    telegram_session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · DAG",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        return AiReplyResult(text="Base reply.", model="gpt-5.4-mini")

    dag_summary = {
        "text": "DAG 57% complete (~62% active) · done 4/7 · running 1 · ready 0 · blocked 2 · next: unblock F",
        "progress": {"completed_percent": 57, "estimated_percent": 62},
        "blockers": [{"node_id": "F", "reason": "Dependency E is running, not completed."}],
    }

    def fake_task_dag_progress(ctx, session, *, text, surface_title):
        assert "task dag" in text.lower()
        return dag_summary

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)
    monkeypatch.setattr(server_app, "_task_dag_progress_summary_for_turn", fake_task_dag_progress)

    with TestClient(server_app.create_app()) as client:
        editor_response = client.post(
            f"/v1/native/sessions/{editor_session['session_id']}:turn",
            json={"text": "show task DAG progress", "surface": "vscode", "title": "VSCode Codex"},
            headers={"X-AIT-Actor": "weita@example.com", "X-AIT-Actor-Type": "human"},
        )
        telegram_response = client.post(
            f"/v1/native/sessions/{telegram_session['session_id']}:telegramTurn",
            json={
                "text": "show task DAG progress",
                "chat_id": "123",
                "chat_title": "Wei",
                "chat_type": "private",
                "telegram_message_id": 12,
            },
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    assert editor_response.status_code == 200
    editor_payload = editor_response.json()
    assert editor_payload["reply_text"] == f"Base reply.\n\n{dag_summary['text']}"
    assert editor_payload["assistant_event"]["payload"]["text"] == f"Base reply.\n\n{dag_summary['text']}"
    assert editor_payload["assistant_event"]["payload"]["task_dag_progress"] == dag_summary

    assert telegram_response.status_code == 200
    telegram_payload = telegram_response.json()
    assert telegram_payload["reply_text"] == f"Base reply.\n\n{dag_summary['text']}"
    assert telegram_payload["assistant_event"]["payload"]["text"] == f"Base reply.\n\n{dag_summary['text']}"
    assert telegram_payload["assistant_event"]["payload"]["task_dag_progress"] == dag_summary


def test_server_telegram_turn_persists_reply_when_task_dag_progress_fails(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · DAG",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        return AiReplyResult(text="Base reply.", model="gpt-5.4-mini")

    def failing_task_dag_progress(ctx, session, *, text, surface_title):
        raise RuntimeError("plan_revisions lock timeout")

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)
    monkeypatch.setattr(server_app, "_task_dag_progress_summary_for_turn", failing_task_dag_progress)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={
                "text": "show task DAG progress",
                "chat_id": "123",
                "chat_title": "Wei",
                "chat_type": "private",
                "telegram_message_id": 12,
            },
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["ok"] is True
    assert payload["reply_text"] == "Base reply."
    assert payload["assistant_event"]["event_type"] == "assistant.reply"
    assert payload["assistant_event"]["payload"]["text"] == "Base reply."
    assert "task_dag_progress" not in payload["assistant_event"]["payload"]

    events = list_session_events(ctx, session["session_id"], after_sequence=0, limit=10)
    assert [event["event_type"] for event in events] == ["telegram.user_message", "assistant.reply"]
    assert events[-1]["payload"]["text"] == "Base reply."


def test_server_task_creation_triggers_graph_notification_delivery(tmp_path: Path, monkeypatch):
    _configure_server_env(tmp_path, monkeypatch)
    calls: list[dict[str, Any]] = []

    def fake_trigger(ctx, repo_name, *, event_type, entity_id):
        calls.append({"repo_name": repo_name, "event_type": event_type, "entity_id": entity_id})

    monkeypatch.setattr(server_app, "_trigger_task_dag_telegram_notifications", fake_trigger)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            "/v1/native/repositories/ait/tasks",
            json={
                "title": "Graph task",
                "intent": "exercise active graph trigger",
                "risk_tier": "low",
                "unplanned": True,
            },
            headers={"X-AIT-Actor": "ait-agent-telegram", "X-AIT-Actor-Type": "telegram_bot"},
        )

    assert response.status_code == 200
    task_id = response.json()["task_id"]
    assert calls == [{"repo_name": "ait", "event_type": "task.created", "entity_id": task_id}]


def test_server_graph_trigger_uses_named_worker_config_when_shell_env_is_stale(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    repo_root = _write_bound_repo_checkout(
        tmp_path / "repo",
        "ait",
        env_lines=["AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS=inf"],
    )
    env_path = repo_root / ".ait" / "agent-runtime" / "telegram.env"
    sync_path = repo_root / "runtime" / "telegram-sync.json"
    worker_config_path = repo_root / ".ait" / "agent-workers.json"
    worker_config_path.write_text(
        json.dumps(
            {
                "version": 1,
                "workers": {
                    "telegram/main": {
                        "kind": "telegram",
                        "name": "main",
                        "token": "123456:worker-token",
                        "sync_state_path": str(sync_path),
                    }
                },
            }
        ),
        encoding="utf-8",
    )
    captured: dict[str, Any] = {}

    def fake_trigger(config, *, repo_name, progress_reader, plan_ids=None):
        captured["repo_name"] = repo_name
        captured["env_path"] = config.env_path
        captured["sync_state_path"] = config.sync_state_path
        captured["token"] = config.token
        captured["request_timeout_seconds"] = config.request_timeout_seconds
        captured["plan_ids"] = plan_ids
        return {"checked": 0, "sent": 0, "errors": 0}

    monkeypatch.setenv("AIT_REPO_ROOT", str(repo_root))
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(worker_config_path))
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(repo_root / "telegram-bot" / ".env"))
    monkeypatch.setattr(telegram_graph_watches, "trigger_graph_watch_notifications", fake_trigger)

    server_app._trigger_task_dag_telegram_notifications(
        ctx,
        "ait",
        event_type="task.created",
        entity_id="T-1",
    )

    assert captured == {
        "repo_name": "ait",
        "env_path": env_path,
        "sync_state_path": sync_path,
        "token": "123456:worker-token",
        "request_timeout_seconds": None,
        "plan_ids": None,
    }


def test_server_graph_trigger_prefers_repo_specific_worker_checkout_over_stale_global_binding(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    primary_root = _write_bound_repo_checkout(
        tmp_path / "ait-root",
        "ait",
        env_lines=["AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS=5"],
        worker_payload={
            "version": 1,
            "workers": {
                "telegram/main": {
                    "kind": "telegram",
                    "name": "main",
                    "token": "111111:ait-token",
                    "sync_state_path": "runtime/ait-sync.json",
                }
            },
        },
    )
    repo_root = _write_bound_repo_checkout(
        tmp_path / "AI_Radio_Engine",
        "AI_Radio_Engine",
        env_lines=["AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS=inf"],
        worker_payload={
            "version": 1,
            "workers": {
                "telegram/main": {
                    "kind": "telegram",
                    "name": "main",
                    "token": "999999:radio-token",
                    "sync_state_path": "runtime/radio-sync.json",
                }
            },
        },
    )
    captured: dict[str, Any] = {}

    def fake_trigger(config, *, repo_name, progress_reader, plan_ids=None):
        captured["repo_name"] = repo_name
        captured["env_path"] = config.env_path
        captured["sync_state_path"] = config.sync_state_path
        captured["token"] = config.token
        captured["request_timeout_seconds"] = config.request_timeout_seconds
        captured["plan_ids"] = plan_ids
        return {"checked": 0, "sent": 0, "errors": 0}

    monkeypatch.setenv("AIT_REPO_ROOT", str(primary_root))
    monkeypatch.setenv("AIT_AGENT_CONFIG_PATH", str(primary_root / ".ait" / "agent-workers.json"))
    monkeypatch.setenv("AIT_TELEGRAM_ENV_PATH", str(primary_root / ".ait" / "agent-runtime" / "telegram.env"))
    monkeypatch.setattr(telegram_graph_watches, "trigger_graph_watch_notifications", fake_trigger)

    server_app._trigger_task_dag_telegram_notifications(
        ctx,
        "AI_Radio_Engine",
        event_type="task.created",
        entity_id="T-1",
    )

    assert captured == {
        "repo_name": "AI_Radio_Engine",
        "env_path": repo_root / ".ait" / "agent-runtime" / "telegram.env",
        "sync_state_path": repo_root / "runtime" / "radio-sync.json",
        "token": "999999:radio-token",
        "request_timeout_seconds": None,
        "plan_ids": None,
    }


def test_server_telegram_turn_endpoint_prefers_checkpoint_plus_delta_context(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · Wei",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )
    append_session_event(
        ctx,
        session["session_id"],
        "telegram.user_message",
        {"source": "telegram", "text": "old request", "telegram_chat_id": "123"},
        actor_identity="telegram:456:@weita",
        actor_type="telegram_user",
    )
    append_session_event(
        ctx,
        session["session_id"],
        "assistant.reply",
        {"source": "openai", "text": "old answer"},
        actor_identity="ait-server",
        actor_type="ai_assistant",
    )
    checkpoint = create_session_checkpoint(
        ctx,
        session["session_id"],
        "Checkpoint summary for the old exchange.",
        resume_payload={"objective": "Ship feature", "context": {"phase": "review"}},
        based_on_sequence=2,
        actor_identity="ait-server",
        actor_type="system_worker",
    )
    append_session_event(
        ctx,
        session["session_id"],
        "web.note",
        {"source": "web", "text": "Policy is still pending."},
        actor_identity="alice@example.com",
        actor_type="human",
    )

    captured: dict[str, Any] = {}

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        captured["checkpoint"] = checkpoint
        captured["events"] = events
        return AiReplyResult(
            text="Checkpoint-aware server reply.",
            model="gpt-5.4-mini",
            response_id="resp_server_checkpoint",
            usage={"input_tokens": 12, "output_tokens": 6, "total_tokens": 18},
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={"text": "new request", "chat_id": "123", "chat_title": "Wei", "telegram_message_id": 21},
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["ok"] is True
    assert payload["reply_text"] == "Checkpoint-aware server reply."
    assert payload["telegram_context_runtime"]["reply_context_mode"] == "checkpoint_delta"
    assert payload["telegram_context_runtime"]["delta_event_count"] == 3
    assert payload["telegram_context_runtime"]["checkpoint_freshness"] == "fresh"
    assert captured["checkpoint"]["checkpoint_id"] == checkpoint["checkpoint_id"]
    assert [event["sequence"] for event in captured["events"]] == [3, 4]
    assert [event["event_type"] for event in captured["events"]] == ["web.note", "telegram.user_message"]


def test_server_telegram_turn_endpoint_persists_reply_source(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · Wei",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        return AiReplyResult(
            text="Codex reply.",
            model="gpt-5.4",
            response_id="turn_codex_123",
            source="codex",
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={"text": "please fix it", "chat_id": "123", "chat_title": "Wei"},
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["assistant_event"]["payload"]["source"] == "codex"
    events = list_session_events(ctx, session["session_id"], after_sequence=0, limit=10)
    assert events[-1]["payload"]["source"] == "codex"


def test_server_telegram_turn_endpoint_appends_turn_analysis_when_enabled(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    monkeypatch.setenv("AIT_TELEGRAM_APPEND_TURN_ANALYSIS", "true")
    session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · Wei",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        assert config.telegram_append_turn_analysis is True
        return AiReplyResult(
            text="Codex reply.",
            model="gpt-5.4",
            response_id="turn_codex_analysis",
            source="codex",
            turn_analysis={
                "command_count": 3,
                "distinct_command_count": 2,
                "commands": ["ls", "pwd", "ls"],
                "top_commands": [{"command": "ls", "count": 2}, {"command": "pwd", "count": 1}],
                "optimization_hints": [
                    {
                        "code": "avoid_repeated_commands",
                        "summary": "The same command was rerun in this turn.",
                        "detail": "Reuse earlier output instead of repeating `ls` unless state changed.",
                    }
                ],
                "optimization_summary": "The same command was rerun in this turn.",
            },
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={"text": "please check", "chat_id": "123", "chat_title": "Wei"},
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["reply_text"] == "Codex reply.\n\n[turn analysis] ran 3 commands · The same command was rerun in this turn."
    assert payload["turn_analysis"]["command_count"] == 3
    assert payload["assistant_event"]["payload"]["text"] == "Codex reply."
    assert payload["assistant_event"]["payload"]["turn_analysis"]["top_commands"][0] == {"command": "ls", "count": 2}


def test_server_session_turn_endpoint_appends_turn_analysis_when_enabled(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    monkeypatch.setenv("AIT_CHAT_APPEND_TURN_ANALYSIS", "true")
    session = create_session(
        ctx,
        "ait",
        "agent_run",
        title="Editor chat · Wei",
        metadata={"source": "vscode"},
        actor_identity="weita@example.com",
        actor_type="human",
    )

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        assert config.telegram_append_turn_analysis is True
        assert surface == "vscode"
        return AiReplyResult(
            text="Codex reply.",
            model="gpt-5.4",
            response_id="turn_codex_session_analysis",
            source="codex",
            turn_analysis={
                "command_count": 3,
                "distinct_command_count": 2,
                "commands": ["ls", "pwd", "ls"],
                "top_commands": [{"command": "ls", "count": 2}, {"command": "pwd", "count": 1}],
                "optimization_hints": [
                    {
                        "code": "avoid_repeated_commands",
                        "summary": "The same command was rerun in this turn.",
                        "detail": "Reuse earlier output instead of repeating `ls` unless state changed.",
                    }
                ],
                "optimization_summary": "The same command was rerun in this turn.",
            },
        )

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:turn",
            json={"text": "please check", "surface": "vscode", "title": "VSCode Codex"},
            headers={"X-AIT-Actor": "weita@example.com", "X-AIT-Actor-Type": "human"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["reply_text"] == "Codex reply.\n\n[turn analysis] ran 3 commands · The same command was rerun in this turn."
    assert payload["turn_analysis"]["command_count"] == 3
    assert payload["assistant_event"]["payload"]["text"] == "Codex reply."
    assert payload["assistant_event"]["payload"]["turn_analysis"]["top_commands"][0] == {"command": "ls", "count": 2}


def test_server_telegram_turn_endpoint_returns_partial_success_on_reply_failure(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · Wei",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )

    def fake_fail(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        assert checkpoint is None
        raise ReplyGenerationError("backend reply unavailable")

    monkeypatch.setattr(server_app, "generate_session_reply", fake_fail)

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={
                "text": "hello from telegram",
                "chat_id": "123",
                "chat_title": "Wei",
            },
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["ok"] is False
    assert payload["assistant_event"] is None
    assert "backend reply unavailable" in payload["error"]
    assert payload["checkpoint"] is None
    assert payload["telegram_context_runtime"]["reply_context_mode"] == "recent_tail"
    assert payload["telegram_context_runtime"]["checkpoint_freshness"] == "missing"
    assert payload["telegram_context_runtime"]["delta_event_count"] == 1

    events = list_session_events(ctx, session["session_id"], after_sequence=0, limit=10)
    assert [event["event_type"] for event in events] == ["telegram.user_message"]
    assert list_session_checkpoints(ctx, session["session_id"]) == []


def test_server_telegram_turn_endpoint_refreshes_checkpoint_after_event_threshold(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    monkeypatch.setenv("AIT_CHAT_CHECKPOINT_EVENT_THRESHOLD", "4")
    monkeypatch.setenv("AIT_CHAT_CHECKPOINT_SUMMARY_EVENT_LIMIT", "6")
    session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · Wei",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )

    replies = iter(
        [
            AiReplyResult(text="First answer.", model="gpt-5.4-mini", response_id="resp_server_1", usage={"total_tokens": 10}),
            AiReplyResult(text="Second answer.", model="gpt-5.4-mini", response_id="resp_server_2", usage={"total_tokens": 11}),
            AiReplyResult(text="Third answer.", model="gpt-5.4-mini", response_id="resp_server_3", usage={"total_tokens": 12}),
        ]
    )

    def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
        return next(replies)

    monkeypatch.setattr(server_app, "generate_session_reply", fake_generate)

    with TestClient(server_app.create_app()) as client:
        response1 = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={"text": "first request", "chat_id": "123", "chat_title": "Wei", "telegram_message_id": 11},
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )
        response2 = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={"text": "second request", "chat_id": "123", "chat_title": "Wei", "telegram_message_id": 12},
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )
        response3 = client.post(
            f"/v1/native/sessions/{session['session_id']}:telegramTurn",
            json={"text": "third request", "chat_id": "123", "chat_title": "Wei", "telegram_message_id": 13},
            headers={"X-AIT-Actor": "telegram:456:@weita", "X-AIT-Actor-Type": "telegram_user"},
        )

    payload1 = response1.json()
    payload2 = response2.json()
    payload3 = response3.json()
    assert payload1["checkpoint"] is not None
    assert payload2["checkpoint"] is None
    assert payload3["checkpoint"] is not None
    assert payload3["checkpoint"]["checkpoint_id"] != payload1["checkpoint"]["checkpoint_id"]
    assert payload1["telegram_context_runtime"]["delta_event_count"] == 0
    assert payload2["telegram_context_runtime"]["delta_event_count"] == 2
    assert payload2["telegram_context_runtime"]["checkpoint_freshness"] == "fresh"
    assert payload3["telegram_context_runtime"]["delta_event_count"] == 0

    checkpoints = list_session_checkpoints(ctx, session["session_id"])
    assert len(checkpoints) == 2
    assert checkpoints[0]["based_on_sequence"] == 6
    assert checkpoints[0]["resume_payload"]["context"]["checkpoint_reason"] == "event_tail_threshold"
    assert checkpoints[0]["resume_payload"]["recent_user_requests"][-1] == "third request"
    assert get_session(ctx, session["session_id"])["head_checkpoint_id"] == checkpoints[0]["checkpoint_id"]


def test_server_session_endpoint_reports_telegram_checkpoint_runtime_state(tmp_path: Path, monkeypatch):
    ctx = _configure_server_env(tmp_path, monkeypatch)
    session = create_session(
        ctx,
        "ait",
        "telegram_chat",
        title="Telegram chat · Wei",
        metadata={"source": "telegram", "telegram_chat_id": "123", "telegram_chat_title": "Wei"},
        actor_identity="ait-agent-telegram",
        actor_type="telegram_bot",
    )
    append_session_event(
        ctx,
        session["session_id"],
        "telegram.user_message",
        {"source": "telegram", "text": "old request", "telegram_chat_id": "123"},
        actor_identity="telegram:456:@weita",
        actor_type="telegram_user",
    )
    append_session_event(
        ctx,
        session["session_id"],
        "assistant.reply",
        {"source": "openai", "text": "old answer"},
        actor_identity="ait-server",
        actor_type="ai_assistant",
    )
    checkpoint = create_session_checkpoint(
        ctx,
        session["session_id"],
        "Checkpoint summary for the old exchange.",
        resume_payload={"objective": "Ship feature", "context": {"phase": "review"}},
        based_on_sequence=2,
        actor_identity="ait-server",
        actor_type="system_worker",
    )
    append_session_event(
        ctx,
        session["session_id"],
        "web.note",
        {"source": "web", "text": "Policy is still pending."},
        actor_identity="alice@example.com",
        actor_type="human",
    )

    with TestClient(server_app.create_app()) as client:
        response = client.get(f"/v1/native/sessions/{session['session_id']}")

    assert response.status_code == 200
    payload = response.json()
    runtime = payload["telegram_context_runtime"]
    assert runtime["reply_context_mode"] == "checkpoint_delta"
    assert runtime["checkpoint_id"] == checkpoint["checkpoint_id"]
    assert runtime["checkpoint_based_on_sequence"] == 2
    assert runtime["last_event_sequence"] == 3
    assert runtime["delta_event_count"] == 1
    assert runtime["checkpoint_event_threshold"] == 6
    assert runtime["events_until_refresh"] == 5
    assert runtime["checkpoint_freshness"] == "fresh"
