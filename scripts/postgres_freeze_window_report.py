#!/usr/bin/env python3
"""Capture Wave 0 freeze-window evidence for the PostgreSQL repo_id cutover."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


DEFAULT_TIMEOUT = 10.0


def _now_utc() -> datetime:
    return datetime.now(timezone.utc)


def _parse_datetime(value: str | None) -> datetime:
    text = str(value or "").strip()
    if not text:
        return _now_utc()
    parsed = datetime.fromisoformat(text.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _run_command(command: list[str], *, timeout: float) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, capture_output=True, text=True, timeout=timeout, check=False)


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


def _command_record(name: str, command: list[str], *, timeout: float) -> dict[str, Any]:
    payload, error = _command_json(command, timeout=timeout)
    return {
        "name": name,
        "command": command,
        "ok": error is None,
        "error": error,
        "payload": payload,
    }


def build_freeze_window_report(
    *,
    owner: str,
    oncall: str | None,
    window_id: str | None,
    notes: str | None,
    freeze_started_at: datetime,
    writes_blocked: bool,
    manual_writes_blocked: bool,
    workers_stopped: bool,
    server_stopped: bool,
    web_stopped: bool,
    command_results: list[dict[str, Any]],
) -> dict[str, Any]:
    gate_checks = {
        "writes_blocked": writes_blocked,
        "manual_writes_blocked": manual_writes_blocked,
        "workers_stopped": workers_stopped,
        "server_stopped": server_stopped,
        "web_stopped": web_stopped,
    }
    command_failures = [row["name"] for row in command_results if not row.get("ok")]
    go_no_go_ready = all(gate_checks.values()) and not command_failures
    recommendations: list[str] = []
    if not writes_blocked:
        recommendations.append("Block or pause all write-producing traffic before moving past Wave 0.")
    if not manual_writes_blocked:
        recommendations.append("Confirm manual operator writes are paused during the maintenance window.")
    if not workers_stopped:
        recommendations.append("Stop background workers or verify no write-capable jobs remain active.")
    if not server_stopped:
        recommendations.append("Stop or otherwise freeze ait-server writes before destructive rebuild steps.")
    if not web_stopped:
        recommendations.append("Stop or gate ait-web so new shared writes cannot enter the cutover window.")
    for row in command_results:
        if not row.get("ok"):
            recommendations.append(f"Capture `{row['name']}` successfully or attach equivalent evidence before Wave 1.")
    if go_no_go_ready:
        recommendations.append("Wave 0 freeze evidence is complete; operator approval can decide whether Wave 1 may begin.")
    return {
        "generated_at": _now_utc().isoformat(),
        "window": {
            "owner": owner,
            "oncall": oncall,
            "window_id": window_id,
            "freeze_started_at": freeze_started_at.isoformat(),
            "notes": notes,
        },
        "gate_checks": gate_checks,
        "commands": command_results,
        "command_failures": command_failures,
        "go_no_go_ready": go_no_go_ready,
        "recommendations": recommendations,
    }


def capture_freeze_window_report(
    *,
    owner: str,
    oncall: str | None,
    window_id: str | None,
    notes: str | None,
    freeze_started_at: datetime,
    writes_blocked: bool,
    manual_writes_blocked: bool,
    workers_stopped: bool,
    server_stopped: bool,
    web_stopped: bool,
    remote: str,
    timeout: float,
    include_queue_summary: bool,
    include_runtime_root: bool,
    include_postgres_doctor: bool,
) -> dict[str, Any]:
    command_results: list[dict[str, Any]] = []
    if include_queue_summary:
        command_results.append(
            _command_record(
                "queue_summary",
                ["ait", "queue", "summary", "--all-changes", "--remote", remote, "--json"],
                timeout=timeout,
            )
        )
    if include_runtime_root:
        command_results.append(
            _command_record(
                "runtime_root_doctor",
                ["ait", "doctor", "runtime-root", "--json"],
                timeout=timeout,
            )
        )
    if include_postgres_doctor:
        command_results.append(
            _command_record(
                "postgres_doctor",
                ["ait", "doctor", "postgres", "--connect", "--json"],
                timeout=timeout,
            )
        )
    return build_freeze_window_report(
        owner=owner,
        oncall=oncall,
        window_id=window_id,
        notes=notes,
        freeze_started_at=freeze_started_at,
        writes_blocked=writes_blocked,
        manual_writes_blocked=manual_writes_blocked,
        workers_stopped=workers_stopped,
        server_stopped=server_stopped,
        web_stopped=web_stopped,
        command_results=command_results,
    )


def format_report(payload: dict[str, Any]) -> str:
    window = dict(payload.get("window") or {})
    lines = [
        f"Owner: {window.get('owner')}",
        f"Freeze started at: {window.get('freeze_started_at')}",
        f"Go/no-go ready: {'yes' if payload.get('go_no_go_ready') else 'no'}",
        "",
        "Gate checks:",
    ]
    for name, ok in dict(payload.get("gate_checks") or {}).items():
        lines.append(f"- {name}: {'ok' if ok else 'pending'}")
    commands = list(payload.get("commands") or [])
    if commands:
        lines.append("")
        lines.append("Captured commands:")
        for row in commands:
            status = "ok" if row.get("ok") else f"fail ({row.get('error')})"
            lines.append(f"- {row.get('name')}: {status}")
    recommendations = list(payload.get("recommendations") or [])
    if recommendations:
        lines.append("")
        lines.append("Recommendations:")
        for row in recommendations:
            lines.append(f"- {row}")
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--owner", required=True, help="Named cutover owner for the maintenance window.")
    parser.add_argument("--oncall", help="Optional on-call or escalation contact.")
    parser.add_argument("--window-id", help="Optional maintenance window identifier or ticket id.")
    parser.add_argument("--notes", help="Optional free-form operator notes.")
    parser.add_argument("--freeze-started-at", help="ISO-8601 freeze timestamp. Defaults to now.")
    parser.add_argument("--writes-blocked", action="store_true", help="Mark application/shared writes as blocked.")
    parser.add_argument("--manual-writes-blocked", action="store_true", help="Mark manual operator writes as blocked.")
    parser.add_argument("--workers-stopped", action="store_true", help="Mark background workers as stopped or paused.")
    parser.add_argument("--server-stopped", action="store_true", help="Mark ait-server as stopped or otherwise frozen.")
    parser.add_argument("--web-stopped", action="store_true", help="Mark ait-web as stopped or otherwise gated.")
    parser.add_argument("--remote", default="origin", help="Remote name for queue summary capture. Default: origin.")
    parser.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT, help=f"Per-command timeout in seconds. Default: {DEFAULT_TIMEOUT}.")
    parser.add_argument("--skip-queue-summary", action="store_true", help="Skip `ait queue summary --all-changes --json` capture.")
    parser.add_argument("--skip-runtime-root", action="store_true", help="Skip `ait doctor runtime-root --json` capture.")
    parser.add_argument("--skip-postgres-doctor", action="store_true", help="Skip `ait doctor postgres --connect --json` capture.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of a text summary.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        payload = capture_freeze_window_report(
            owner=args.owner,
            oncall=args.oncall,
            window_id=args.window_id,
            notes=args.notes,
            freeze_started_at=_parse_datetime(args.freeze_started_at),
            writes_blocked=bool(args.writes_blocked),
            manual_writes_blocked=bool(args.manual_writes_blocked),
            workers_stopped=bool(args.workers_stopped),
            server_stopped=bool(args.server_stopped),
            web_stopped=bool(args.web_stopped),
            remote=str(args.remote or "origin"),
            timeout=float(args.timeout),
            include_queue_summary=not bool(args.skip_queue_summary),
            include_runtime_root=not bool(args.skip_runtime_root),
            include_postgres_doctor=not bool(args.skip_postgres_doctor),
        )
    except Exception as exc:
        print(_json_dump({"ok": False, "error": str(exc)}), file=sys.stderr)
        return 1

    text = _json_dump(payload) if args.json else format_report(payload)
    if args.output:
        output_path = Path(args.output).expanduser()
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(text + ("" if text.endswith("\n") else "\n"), encoding="utf-8")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
