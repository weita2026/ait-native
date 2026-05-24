#!/usr/bin/env python3
"""Quick operator check for ait-agent / ait-server / Telegram-linked reply health."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


SEVERITY_ORDER = {"ok": 0, "warn": 1, "fail": 2}
USER_EVENT_TYPES = {"telegram.user_message", "user_message"}
ASSISTANT_EVENT_TYPES = {"assistant.reply", "assistant_message"}


def _now_utc() -> datetime:
    return datetime.now(timezone.utc)


def _parse_datetime(value: Any) -> datetime | None:
    text = str(value or "").strip()
    if not text:
        return None
    try:
        return datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError:
        return None


def _format_age(seconds: float | int | None) -> str | None:
    if seconds is None:
        return None
    total = int(max(0, round(float(seconds))))
    hours, rem = divmod(total, 3600)
    minutes, secs = divmod(rem, 60)
    if hours:
        return f"{hours}h{minutes:02d}m{secs:02d}s"
    if minutes:
        return f"{minutes}m{secs:02d}s"
    return f"{secs}s"


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _run_command(command: list[str], *, timeout: float) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )


def _command_json(command: list[str], *, timeout: float) -> tuple[Any | None, str | None]:
    try:
        completed = _run_command(command, timeout=timeout)
    except (FileNotFoundError, subprocess.TimeoutExpired) as exc:
        return None, str(exc)
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "").strip() or f"exit {completed.returncode}"
        return None, detail
    try:
        return json.loads(completed.stdout), None
    except json.JSONDecodeError as exc:
        return None, f"Could not parse JSON from {' '.join(command)}: {exc}"


def _fetch_json(url: str, *, timeout: float) -> Any:
    with urllib.request.urlopen(url, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


def _server_url_from_env() -> str:
    host = str(os.environ.get("AIT_NATIVE_SERVER_HOST") or "127.0.0.1").strip() or "127.0.0.1"
    if host in {"0.0.0.0", "::", "[::]"}:
        host = "127.0.0.1"
    port = str(os.environ.get("AIT_NATIVE_SERVER_PORT") or "8088").strip() or "8088"
    return f"http://{host}:{port}"


def _resolve_agent_bin() -> str:
    return (
        str(os.environ.get("AIT_AGENT_BIN") or "").strip()
        or shutil.which("ait-agent")
        or "ait-agent"
    )


def _resolve_ait_bin() -> str:
    return (
        str(os.environ.get("AIT_BIN") or "").strip()
        or shutil.which("ait")
        or "ait"
    )


def _read_json_file(path: Path) -> tuple[Any | None, str | None]:
    try:
        return json.loads(path.read_text(encoding="utf-8")), None
    except FileNotFoundError:
        return None, f"Missing file: {path}"
    except json.JSONDecodeError as exc:
        return None, f"Invalid JSON in {path}: {exc}"


def _select_worker(supervisor_status: dict[str, Any] | None, worker_name: str | None) -> dict[str, Any] | None:
    workers = list((supervisor_status or {}).get("workers") or [])
    if not workers:
        return None
    if worker_name:
        for worker in workers:
            if str(worker.get("name") or "") == worker_name:
                return dict(worker)
    for worker in workers:
        if str(worker.get("name") or "") == "main":
            return dict(worker)
    return dict(workers[0])


def _resolve_sync_state_path(
    explicit_path: str | None,
    selected_worker: dict[str, Any] | None,
) -> Path | None:
    if explicit_path:
        return Path(explicit_path).expanduser()
    candidate = str((selected_worker or {}).get("sync_state_path") or "").strip()
    if candidate:
        return Path(candidate).expanduser()
    data_root = str(os.environ.get("AIT_NATIVE_SERVER_DATA") or "").strip()
    if data_root:
        return Path(data_root).expanduser() / "telegram-sync.json"
    return None


def _select_chat_binding(sync_state: dict[str, Any] | None, chat_id: str | None) -> tuple[str | None, dict[str, Any] | None]:
    chats = dict((sync_state or {}).get("chats") or {})
    if not chats:
        return None, None
    if chat_id:
        row = chats.get(chat_id)
        return (chat_id, dict(row)) if isinstance(row, dict) else (None, None)
    if len(chats) == 1:
        selected_chat_id, row = next(iter(chats.items()))
        return str(selected_chat_id), dict(row)
    for selected_chat_id, row in chats.items():
        if isinstance(row, dict) and str(row.get("binding_role") or "") == "primary_shared":
            return str(selected_chat_id), dict(row)
    selected_chat_id, row = next(iter(chats.items()))
    return str(selected_chat_id), dict(row) if isinstance(row, dict) else None


def _normalize_events(payload: Any) -> list[dict[str, Any]]:
    if isinstance(payload, list):
        return [dict(row) for row in payload if isinstance(row, dict)]
    if isinstance(payload, dict):
        items = payload.get("items")
        if isinstance(items, list):
            return [dict(row) for row in items if isinstance(row, dict)]
    return []


def _runtime_backend_mode(binding: dict[str, Any] | None) -> str | None:
    mode = str((binding or {}).get("runtime_backend_mode") or "").strip().lower()
    if mode in {"local", "remote"}:
        return mode
    signature = str((binding or {}).get("runtime_backend_signature") or "").strip().lower()
    if signature.startswith("local|"):
        return "local"
    if signature.startswith("remote|"):
        return "remote"
    return None


def _local_session_events(
    session_id: str,
    *,
    after_sequence: int,
    limit: int,
    timeout: float,
) -> tuple[list[dict[str, Any]], str | None]:
    payload, error = _command_json(
        [
            _resolve_ait_bin(),
            "session",
            "events",
            session_id,
            "--after-sequence",
            str(max(after_sequence, 0)),
            "--limit",
            str(max(limit, 1)),
            "--local",
            "--json",
        ],
        timeout=timeout,
    )
    if error is not None:
        return [], error
    return _normalize_events(payload), None


def _event_summary_row(event: dict[str, Any]) -> dict[str, Any]:
    payload = dict(event.get("payload") or {})
    text = str(payload.get("text") or "").strip()
    if len(text) > 120:
        text = text[:117] + "..."
    return {
        "sequence": event.get("sequence"),
        "event_type": event.get("event_type"),
        "actor_type": event.get("actor_type"),
        "created_at": event.get("created_at"),
        "text": text or None,
    }


def _analyze_session_events(
    events: list[dict[str, Any]],
    *,
    now: datetime,
) -> dict[str, Any]:
    latest_event = dict(events[-1]) if events else None
    latest_reply_sequence: int | None = None
    latest_user_event: dict[str, Any] | None = None
    unanswered_user_event: dict[str, Any] | None = None

    for event in events:
        event_type = str(event.get("event_type") or "")
        if event_type in ASSISTANT_EVENT_TYPES:
            try:
                latest_reply_sequence = int(event.get("sequence") or 0)
            except (TypeError, ValueError):
                latest_reply_sequence = latest_reply_sequence
            if unanswered_user_event is not None and latest_reply_sequence is not None:
                unanswered_sequence = int(unanswered_user_event.get("sequence") or 0)
                if unanswered_sequence > 0 and latest_reply_sequence >= unanswered_sequence:
                    unanswered_user_event = None
        if event_type in USER_EVENT_TYPES:
            latest_user_event = dict(event)
            sequence = int(event.get("sequence") or 0)
            if latest_reply_sequence is None or latest_reply_sequence < sequence:
                unanswered_user_event = dict(event)
            else:
                unanswered_user_event = None

    pending_reply = unanswered_user_event is not None
    pending_reply_age_seconds: float | None = None
    if unanswered_user_event is not None:
        created_at = _parse_datetime(unanswered_user_event.get("created_at"))
        if created_at is not None:
            pending_reply_age_seconds = max(0.0, (now - created_at.astimezone(timezone.utc)).total_seconds())

    return {
        "latest_event": _event_summary_row(latest_event) if latest_event else None,
        "latest_user_event": _event_summary_row(latest_user_event) if latest_user_event else None,
        "latest_unanswered_user_event": _event_summary_row(unanswered_user_event) if unanswered_user_event else None,
        "pending_reply": pending_reply,
        "pending_reply_age_seconds": pending_reply_age_seconds,
    }


def build_report(
    *,
    server_url: str,
    sync_state_path: str | None,
    worker_name: str | None,
    chat_id: str | None,
    session_id: str | None,
    timeout: float,
    stuck_seconds: float,
    event_limit: int,
) -> dict[str, Any]:
    now = _now_utc()
    agent_bin = _resolve_agent_bin()
    supervisor_status, supervisor_error = _command_json(
        [agent_bin, "telegram", "supervisor", "status"],
        timeout=timeout,
    )
    selected_worker = _select_worker(
        supervisor_status if isinstance(supervisor_status, dict) else None,
        worker_name,
    )
    resolved_sync_state_path = _resolve_sync_state_path(sync_state_path, selected_worker)

    server: dict[str, Any] = {
        "state": "fail",
        "url": server_url,
        "healthz_ok": False,
    }
    try:
        healthz = _fetch_json(server_url.rstrip("/") + "/healthz", timeout=timeout)
        pressure = dict(healthz.get("live_turn_pressure") or {})
        server.update(
            {
                "state": "ok",
                "healthz_ok": True,
                "db_backend": healthz.get("db_backend"),
                "runtime_root": healthz.get("runtime_root"),
                "queue_mode": healthz.get("queue_mode"),
                "live_turn_pressure": pressure,
            }
        )
        oldest_age = pressure.get("oldest_in_flight_turn_age_seconds")
        if int(pressure.get("in_flight_turns") or 0) > 0 and float(oldest_age or 0.0) >= stuck_seconds:
            server["state"] = "warn"
    except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, json.JSONDecodeError) as exc:
        server["error"] = str(exc)

    telegram: dict[str, Any] = {
        "state": "fail",
        "agent_bin": agent_bin,
        "supervisor_ok": supervisor_error is None,
        "selected_worker": None,
        "sync_state_path": str(resolved_sync_state_path) if resolved_sync_state_path else None,
    }
    if supervisor_error is not None:
        telegram["error"] = supervisor_error
    else:
        running_count = int((supervisor_status or {}).get("running_count") or 0)
        worker_count = int((supervisor_status or {}).get("worker_count") or 0)
        telegram.update(
            {
                "state": "ok" if running_count > 0 else "warn",
                "running_count": running_count,
                "worker_count": worker_count,
            }
        )
        if selected_worker is not None:
            telegram["selected_worker"] = {
                "name": selected_worker.get("name"),
                "kind": selected_worker.get("kind"),
                "running": bool(selected_worker.get("running")),
                "pid": selected_worker.get("pid"),
                "log_file": selected_worker.get("log_file"),
                "sync_state_path": selected_worker.get("sync_state_path"),
            }
            if not bool(selected_worker.get("running")):
                telegram["state"] = "warn"

    sync_state: dict[str, Any] | None = None
    sync_state_error: str | None = None
    if resolved_sync_state_path is not None:
        payload, sync_state_error = _read_json_file(resolved_sync_state_path)
        if isinstance(payload, dict):
            sync_state = payload

    selected_chat_id: str | None = None
    selected_chat: dict[str, Any] | None = None
    if sync_state is not None:
        selected_chat_id, selected_chat = _select_chat_binding(sync_state, chat_id)

    effective_session_id = (
        str(session_id or "").strip()
        or str((selected_chat or {}).get("canonical_session_id") or "").strip()
        or str((selected_chat or {}).get("session_id") or "").strip()
        or None
    )

    session: dict[str, Any] = {
        "state": "warn",
        "chat_id": selected_chat_id,
        "session_id": effective_session_id,
        "sync_state_ok": sync_state_error is None,
    }
    if sync_state_error is not None:
        session["sync_state_error"] = sync_state_error
    if selected_chat is not None:
        session.update(
            {
                "chat_title": selected_chat.get("chat_title"),
                "binding_role": selected_chat.get("binding_role"),
                "runtime_backend_mode": _runtime_backend_mode(selected_chat),
                "last_sync_at": selected_chat.get("last_sync_at"),
                "last_synced_sequence": selected_chat.get("last_synced_sequence"),
                "delivered_sequence_max": max(selected_chat.get("telegram_live_delivered_sequences") or [0]),
            }
        )

    events: list[dict[str, Any]] = []
    if effective_session_id:
        after_sequence = 0
        if selected_chat is not None:
            after_sequence = max(0, int(selected_chat.get("last_synced_sequence") or 0) - max(1, event_limit))
        if session.get("runtime_backend_mode") == "local":
            session["events_source"] = "local_cli"
            events, events_error = _local_session_events(
                effective_session_id,
                after_sequence=after_sequence,
                limit=max(1, event_limit),
                timeout=timeout,
            )
            if events_error is not None:
                session["events_error"] = events_error
        elif server.get("healthz_ok"):
            session["events_source"] = "server_http"
            query = urllib.parse.urlencode({"after_sequence": after_sequence, "limit": max(1, event_limit)})
            events_url = (
                server_url.rstrip("/")
                + f"/v1/native/sessions/{urllib.parse.quote(effective_session_id, safe='')}/events?{query}"
            )
            try:
                events = _normalize_events(_fetch_json(events_url, timeout=timeout))
            except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, json.JSONDecodeError) as exc:
                session["events_error"] = str(exc)

    analysis = _analyze_session_events(events, now=now)
    session.update(analysis)
    session["recent_events"] = [_event_summary_row(event) for event in events[-event_limit:]]

    if effective_session_id is None:
        session["state"] = "warn"
        session["message"] = "No Telegram-linked session could be resolved."
    elif session.get("pending_reply"):
        age_seconds = float(session.get("pending_reply_age_seconds") or 0.0)
        session["state"] = "warn" if age_seconds < stuck_seconds else "fail"
    elif session.get("latest_event") is not None:
        session["state"] = "ok"

    overall_state = "ok"
    for block in (server, telegram, session):
        if SEVERITY_ORDER[str(block.get("state") or "ok")] > SEVERITY_ORDER[overall_state]:
            overall_state = str(block.get("state"))

    recommendations: list[str] = []
    if not server.get("healthz_ok"):
        recommendations.append(f"Probe server health directly: curl {server_url.rstrip('/')}/healthz")
    if telegram.get("supervisor_ok") is False:
        recommendations.append("Check worker registration: ait-agent telegram supervisor status")
    if session.get("pending_reply"):
        worker = (telegram.get("selected_worker") or {}).get("name") or "main"
        recommendations.append(f"Inspect Telegram worker logs: ait-agent telegram logs {worker} --lines 120")
    if server.get("state") == "warn":
        recommendations.append("A live turn has been active for longer than the stuck threshold; consider restarting the blocked runtime component after checking logs.")

    return {
        "overall_state": overall_state,
        "checked_at": now.isoformat(),
        "stuck_seconds": stuck_seconds,
        "server": server,
        "telegram": telegram,
        "session": session,
        "recommendations": recommendations,
    }


def _format_text_report(report: dict[str, Any]) -> str:
    lines: list[str] = []
    lines.append(f"Overall: {str(report.get('overall_state') or 'unknown').upper()}")

    server = dict(report.get("server") or {})
    lines.append("")
    lines.append(f"Server: {str(server.get('state') or 'unknown').upper()}")
    lines.append(f"- url: {server.get('url')}")
    if server.get("healthz_ok"):
        pressure = dict(server.get("live_turn_pressure") or {})
        lines.append(f"- db_backend: {server.get('db_backend')}")
        lines.append(f"- runtime_root: {server.get('runtime_root')}")
        lines.append(
            "- live_turns: "
            f"in_flight={pressure.get('in_flight_turns', 0)} "
            f"queued={pressure.get('queued_turns', 0)} "
            f"oldest={_format_age(pressure.get('oldest_in_flight_turn_age_seconds')) or 'n/a'} "
            f"pressure={pressure.get('pressure_state')}"
        )
    else:
        lines.append(f"- error: {server.get('error')}")

    telegram = dict(report.get("telegram") or {})
    worker = dict(telegram.get("selected_worker") or {})
    lines.append("")
    lines.append(f"Telegram worker: {str(telegram.get('state') or 'unknown').upper()}")
    if telegram.get("supervisor_ok"):
        lines.append(
            f"- workers: running={telegram.get('running_count', 0)} total={telegram.get('worker_count', 0)}"
        )
        if worker:
            lines.append(
                f"- selected: {worker.get('name')} running={worker.get('running')} pid={worker.get('pid')}"
            )
        if telegram.get("sync_state_path"):
            lines.append(f"- sync_state: {telegram.get('sync_state_path')}")
    else:
        lines.append(f"- error: {telegram.get('error')}")

    session = dict(report.get("session") or {})
    lines.append("")
    lines.append(f"Session: {str(session.get('state') or 'unknown').upper()}")
    if session.get("chat_id"):
        lines.append(
            f"- chat: {session.get('chat_id')} ({session.get('chat_title') or 'unknown'}) role={session.get('binding_role') or 'unknown'}"
        )
    if session.get("session_id"):
        lines.append(f"- session_id: {session.get('session_id')}")
    if session.get("last_sync_at"):
        lines.append(
            f"- sync_cursor: sequence={session.get('last_synced_sequence')} last_sync_at={session.get('last_sync_at')}"
        )
    if session.get("events_source"):
        lines.append(f"- events_source: {session.get('events_source')}")
    if session.get("latest_event"):
        latest = dict(session["latest_event"])
        lines.append(
            f"- latest_event: seq={latest.get('sequence')} type={latest.get('event_type')} at={latest.get('created_at')}"
        )
        if latest.get("text"):
            lines.append(f"  text: {latest.get('text')}")
    if session.get("pending_reply"):
        lines.append(
            f"- pending_reply: yes, age={_format_age(session.get('pending_reply_age_seconds')) or 'n/a'}"
        )
    elif session.get("session_id"):
        lines.append("- pending_reply: no")
    if session.get("events_error"):
        lines.append(f"- events_error: {session.get('events_error')}")
    if session.get("sync_state_error"):
        lines.append(f"- sync_state_error: {session.get('sync_state_error')}")

    recent_events = list(session.get("recent_events") or [])
    if recent_events:
        lines.append("- recent_events:")
        for event in recent_events:
            text = f" text={event.get('text')}" if event.get("text") else ""
            lines.append(
                f"  - seq={event.get('sequence')} type={event.get('event_type')} at={event.get('created_at')}{text}"
            )

    recommendations = list(report.get("recommendations") or [])
    if recommendations:
        lines.append("")
        lines.append("Recommendations:")
        for recommendation in recommendations:
            lines.append(f"- {recommendation}")
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-url", default=_server_url_from_env(), help="Base URL for ait-server health and session probes.")
    parser.add_argument("--sync-state", help="Override the Telegram sync-state JSON path.")
    parser.add_argument("--worker", help="Named Telegram worker to inspect. Defaults to main, otherwise the first configured worker.")
    parser.add_argument("--chat-id", help="Specific Telegram chat id to inspect from telegram-sync.json.")
    parser.add_argument("--session-id", help="Inspect this session id directly instead of resolving one from Telegram sync state.")
    parser.add_argument("--timeout", type=float, default=5.0, help="Per-command and per-request timeout in seconds.")
    parser.add_argument("--stuck-seconds", type=float, default=300.0, help="Age threshold that upgrades an active live turn or unanswered user message to stuck.")
    parser.add_argument("--event-limit", type=int, default=6, help="How many recent session events to summarize.")
    parser.add_argument("--json", action="store_true", help="Emit the full structured report as JSON.")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    report = build_report(
        server_url=str(args.server_url),
        sync_state_path=args.sync_state,
        worker_name=args.worker,
        chat_id=args.chat_id,
        session_id=args.session_id,
        timeout=float(args.timeout),
        stuck_seconds=float(args.stuck_seconds),
        event_limit=max(1, int(args.event_limit)),
    )
    if args.json:
        print(_json_dump(report))
    else:
        print(_format_text_report(report))
    return SEVERITY_ORDER.get(str(report.get("overall_state") or "fail"), 2)


if __name__ == "__main__":
    raise SystemExit(main())
