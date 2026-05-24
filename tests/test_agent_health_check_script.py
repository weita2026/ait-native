from __future__ import annotations

import importlib.util
import json
import sys
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "check_agent_health.py"
SPEC = importlib.util.spec_from_file_location("check_agent_health", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
check_agent_health = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = check_agent_health
SPEC.loader.exec_module(check_agent_health)


def test_build_report_detects_stuck_unanswered_telegram_turn(tmp_path: Path, monkeypatch) -> None:
    sync_state_path = tmp_path / "telegram-sync.json"
    sync_state_path.write_text(
        json.dumps(
            {
                "chats": {
                    "7661100833": {
                        "binding_role": "primary_shared",
                        "chat_title": "唯達",
                        "canonical_session_id": "S-123",
                        "last_sync_at": "2026-05-07T06:47:02+08:00",
                        "last_synced_sequence": 155,
                        "telegram_live_delivered_sequences": [152, 154],
                    }
                }
            }
        ),
        encoding="utf-8",
    )

    def fake_command_json(command: list[str], *, timeout: float):
        assert command[-3:] == ["telegram", "supervisor", "status"]
        return (
            {
                "running_count": 1,
                "worker_count": 1,
                "workers": [
                    {
                        "name": "main",
                        "kind": "telegram",
                        "running": True,
                        "pid": 44449,
                        "log_file": "/tmp/ait-agent-telegram-main.log",
                        "sync_state_path": str(sync_state_path),
                    }
                ],
            },
            None,
        )

    def fake_fetch_json(url: str, *, timeout: float):
        if url.endswith("/healthz"):
            return {
                "db_backend": "postgres",
                "runtime_root": "/runtime/server-data",
                "queue_mode": "inline",
                "live_turn_pressure": {
                    "pressure_state": "saturated",
                    "in_flight_turns": 1,
                    "queued_turns": 0,
                    "oldest_in_flight_turn_age_seconds": 2610.0,
                },
            }
        if "/v1/native/sessions/" in url:
            return [
                {
                    "sequence": 154,
                    "event_type": "assistant.reply",
                    "actor_type": "ai_assistant",
                    "created_at": "2026-05-07T06:04:56+08:00",
                    "payload": {"text": "已做完 1、2"},
                },
                {
                    "sequence": 155,
                    "event_type": "telegram.user_message",
                    "actor_type": "telegram_user",
                    "created_at": "2026-05-07T06:05:56+08:00",
                    "payload": {"text": "繼續1，2"},
                },
            ]
        raise AssertionError(url)

    monkeypatch.setattr(check_agent_health, "_command_json", fake_command_json)
    monkeypatch.setattr(check_agent_health, "_fetch_json", fake_fetch_json)
    monkeypatch.setattr(
        check_agent_health,
        "_now_utc",
        lambda: datetime(2026, 5, 6, 22, 49, 26, tzinfo=timezone.utc),
    )

    report = check_agent_health.build_report(
        server_url="http://127.0.0.1:8088",
        sync_state_path=None,
        worker_name=None,
        chat_id=None,
        session_id=None,
        timeout=5.0,
        stuck_seconds=300.0,
        event_limit=6,
    )

    assert report["overall_state"] == "fail"
    assert report["server"]["state"] == "warn"
    assert report["telegram"]["state"] == "ok"
    assert report["session"]["state"] == "fail"
    assert report["session"]["events_source"] == "server_http"
    assert report["session"]["pending_reply"] is True
    assert report["session"]["latest_event"]["event_type"] == "telegram.user_message"
    assert "Inspect Telegram worker logs" in "\n".join(report["recommendations"])


def test_build_report_reads_local_session_events_for_local_runtime(tmp_path: Path, monkeypatch) -> None:
    sync_state_path = tmp_path / "telegram-sync.json"
    sync_state_path.write_text(
        json.dumps(
            {
                "chats": {
                    "7661100833": {
                        "binding_role": "primary_shared",
                        "chat_title": "唯達",
                        "canonical_session_id": "S-LOCAL-1",
                        "runtime_backend_mode": "local",
                        "last_sync_at": "2026-05-07T06:47:02+08:00",
                        "last_synced_sequence": 42,
                        "telegram_live_delivered_sequences": [40, 42],
                    }
                }
            }
        ),
        encoding="utf-8",
    )

    commands: list[list[str]] = []

    def fake_command_json(command: list[str], *, timeout: float):
        commands.append(command)
        if command[-3:] == ["telegram", "supervisor", "status"]:
            return (
                {
                    "running_count": 1,
                    "worker_count": 1,
                    "workers": [
                        {
                            "name": "main",
                            "kind": "telegram",
                            "running": True,
                            "pid": 44449,
                            "log_file": "/tmp/ait-agent-telegram-main.log",
                            "sync_state_path": str(sync_state_path),
                        }
                    ],
                },
                None,
            )
        if len(command) >= 4 and command[1:3] == ["session", "events"]:
            assert "--local" in command
            return (
                [
                    {
                        "sequence": 41,
                        "event_type": "telegram.user_message",
                        "actor_type": "telegram_user",
                        "created_at": "2026-05-07T06:05:56+08:00",
                        "payload": {"text": "status?"},
                    },
                    {
                        "sequence": 42,
                        "event_type": "assistant.reply",
                        "actor_type": "ai_assistant",
                        "created_at": "2026-05-07T06:06:10+08:00",
                        "payload": {"text": "all good"},
                    },
                ],
                None,
            )
        raise AssertionError(command)

    def fake_fetch_json(url: str, *, timeout: float):
        if url.endswith("/healthz"):
            return {
                "db_backend": "postgres",
                "runtime_root": "/runtime/server-data",
                "queue_mode": "inline",
                "live_turn_pressure": {
                    "pressure_state": "idle",
                    "in_flight_turns": 0,
                    "queued_turns": 0,
                    "oldest_in_flight_turn_age_seconds": None,
                },
            }
        raise AssertionError(url)

    monkeypatch.setattr(check_agent_health, "_command_json", fake_command_json)
    monkeypatch.setattr(check_agent_health, "_fetch_json", fake_fetch_json)
    monkeypatch.setenv("AIT_BIN", "ait")

    report = check_agent_health.build_report(
        server_url="http://127.0.0.1:8088",
        sync_state_path=None,
        worker_name=None,
        chat_id=None,
        session_id=None,
        timeout=5.0,
        stuck_seconds=300.0,
        event_limit=6,
    )

    assert report["overall_state"] == "ok"
    assert report["session"]["state"] == "ok"
    assert report["session"]["runtime_backend_mode"] == "local"
    assert report["session"]["events_source"] == "local_cli"
    assert report["session"].get("events_error") is None
    assert report["session"]["latest_event"]["event_type"] == "assistant.reply"
    assert any(len(command) >= 4 and command[1:3] == ["session", "events"] for command in commands)


def test_build_report_reads_local_session_events_from_runtime_backend_signature(tmp_path: Path, monkeypatch) -> None:
    sync_state_path = tmp_path / "telegram-sync.json"
    sync_state_path.write_text(
        json.dumps(
            {
                "chats": {
                    "7661100833": {
                        "binding_role": "primary_shared",
                        "chat_title": "唯達",
                        "canonical_session_id": "S-LOCAL-2",
                        "runtime_backend_signature": "local|-|-",
                        "last_sync_at": "2026-05-07T06:47:02+08:00",
                        "last_synced_sequence": 52,
                        "telegram_live_delivered_sequences": [50, 52],
                    }
                }
            }
        ),
        encoding="utf-8",
    )

    commands: list[list[str]] = []

    def fake_command_json(command: list[str], *, timeout: float):
        commands.append(command)
        if command[-3:] == ["telegram", "supervisor", "status"]:
            return (
                {
                    "running_count": 1,
                    "worker_count": 1,
                    "workers": [
                        {
                            "name": "main",
                            "kind": "telegram",
                            "running": True,
                            "pid": 44449,
                            "log_file": "/tmp/ait-agent-telegram-main.log",
                            "sync_state_path": str(sync_state_path),
                        }
                    ],
                },
                None,
            )
        if len(command) >= 4 and command[1:3] == ["session", "events"]:
            assert "--local" in command
            return (
                [
                    {
                        "sequence": 51,
                        "event_type": "telegram.user_message",
                        "actor_type": "telegram_user",
                        "created_at": "2026-05-07T06:05:56+08:00",
                        "payload": {"text": "still local?"},
                    },
                    {
                        "sequence": 52,
                        "event_type": "assistant.reply",
                        "actor_type": "ai_assistant",
                        "created_at": "2026-05-07T06:06:10+08:00",
                        "payload": {"text": "yes"},
                    },
                ],
                None,
            )
        raise AssertionError(command)

    def fake_fetch_json(url: str, *, timeout: float):
        if url.endswith("/healthz"):
            return {
                "db_backend": "postgres",
                "runtime_root": "/runtime/server-data",
                "queue_mode": "inline",
                "live_turn_pressure": {
                    "pressure_state": "idle",
                    "in_flight_turns": 0,
                    "queued_turns": 0,
                    "oldest_in_flight_turn_age_seconds": None,
                },
            }
        raise AssertionError(url)

    monkeypatch.setattr(check_agent_health, "_command_json", fake_command_json)
    monkeypatch.setattr(check_agent_health, "_fetch_json", fake_fetch_json)
    monkeypatch.setenv("AIT_BIN", "ait")

    report = check_agent_health.build_report(
        server_url="http://127.0.0.1:8088",
        sync_state_path=None,
        worker_name=None,
        chat_id=None,
        session_id=None,
        timeout=5.0,
        stuck_seconds=300.0,
        event_limit=6,
    )

    assert report["overall_state"] == "ok"
    assert report["session"]["state"] == "ok"
    assert report["session"]["runtime_backend_mode"] == "local"
    assert report["session"]["events_source"] == "local_cli"
    assert report["session"].get("events_error") is None
    assert report["session"]["latest_event"]["event_type"] == "assistant.reply"
    assert any(len(command) >= 4 and command[1:3] == ["session", "events"] for command in commands)


def test_build_report_handles_missing_server_but_running_worker(tmp_path: Path, monkeypatch) -> None:
    sync_state_path = tmp_path / "telegram-sync.json"
    sync_state_path.write_text(json.dumps({"chats": {}}), encoding="utf-8")

    def fake_command_json(command: list[str], *, timeout: float):
        return (
            {
                "running_count": 1,
                "worker_count": 1,
                "workers": [
                    {
                        "name": "main",
                        "kind": "telegram",
                        "running": True,
                        "pid": 44449,
                        "log_file": "/tmp/ait-agent-telegram-main.log",
                        "sync_state_path": str(sync_state_path),
                    }
                ],
            },
            None,
        )

    def fake_fetch_json(url: str, *, timeout: float):
        raise check_agent_health.urllib.error.URLError("[Errno 61] Connection refused")

    monkeypatch.setattr(check_agent_health, "_command_json", fake_command_json)
    monkeypatch.setattr(check_agent_health, "_fetch_json", fake_fetch_json)

    report = check_agent_health.build_report(
        server_url="http://127.0.0.1:8088",
        sync_state_path=None,
        worker_name=None,
        chat_id=None,
        session_id=None,
        timeout=5.0,
        stuck_seconds=300.0,
        event_limit=4,
    )

    assert report["server"]["state"] == "fail"
    assert report["telegram"]["state"] == "ok"
    assert report["session"]["state"] == "warn"
    assert any("curl http://127.0.0.1:8088/healthz" in row for row in report["recommendations"])
