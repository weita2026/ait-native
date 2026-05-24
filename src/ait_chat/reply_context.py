from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from ait_chat.context_compression import format_planning_ledger, normalize_planning_ledger


def payload_text(payload: dict[str, Any]) -> str:
    for key in ("text", "summary", "detail", "message"):
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    compact = json.dumps(payload, ensure_ascii=False, sort_keys=True)
    return compact if compact != "{}" else "(no details)"


def _int_or_zero(value: Any) -> int:
    try:
        return max(int(value or 0), 0)
    except (TypeError, ValueError):
        return 0


def carryforward_turn_analysis(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    command_count = _int_or_zero(value.get("command_count"))
    optimization_summary = str(value.get("optimization_summary") or "").strip()
    suggested_commands: list[str] = []
    raw_suggested_commands = value.get("suggested_commands") or []
    if isinstance(raw_suggested_commands, list):
        for item in raw_suggested_commands:
            command = str(item or "").strip()
            if command and command not in suggested_commands:
                suggested_commands.append(command)
    for hint in value.get("optimization_hints") or []:
        if not isinstance(hint, dict):
            continue
        command = str(hint.get("suggested_command") or "").strip()
        if command and command not in suggested_commands:
            suggested_commands.append(command)
    if command_count <= 0 and not optimization_summary and not suggested_commands:
        return None
    result: dict[str, Any] = {}
    if command_count > 0:
        result["command_count"] = command_count
    if optimization_summary:
        result["optimization_summary"] = optimization_summary
    if suggested_commands:
        result["suggested_commands"] = suggested_commands[:3]
    return result


def format_turn_analysis_guidance(value: Any, *, prefix: str = "[previous turn guidance]") -> str:
    turn_analysis = carryforward_turn_analysis(value)
    if turn_analysis is None:
        return ""
    parts: list[str] = []
    command_count = _int_or_zero(turn_analysis.get("command_count"))
    if command_count > 0:
        parts.append(f"last turn ran {command_count} commands")
    optimization_summary = str(turn_analysis.get("optimization_summary") or "").strip()
    if optimization_summary:
        parts.append(optimization_summary)
    suggested_commands = [str(item).strip() for item in turn_analysis.get("suggested_commands") or [] if str(item).strip()]
    if suggested_commands:
        label = "suggested next-turn command" if len(suggested_commands) == 1 else "suggested next-turn commands"
        parts.append(f"{label}: {'; '.join(f'`{command}`' for command in suggested_commands[:2])}")
    if not parts:
        return ""
    marker = prefix.strip()
    guidance = " · ".join(parts)
    if not marker:
        return guidance
    return f"{marker} {guidance}"


def actor_label(event: dict[str, Any]) -> str:
    return str(event.get("actor_identity") or "system")


def event_message_for_ai(event: dict[str, Any]) -> dict[str, str] | None:
    payload = event.get("payload") or {}
    text = payload_text(payload)
    if not text or text == "(no details)":
        return None
    event_type = str(event.get("event_type") or "")
    actor = actor_label(event)
    source = str(payload.get("source") or "").strip()
    if event_type.startswith("assistant."):
        guidance = format_turn_analysis_guidance(payload.get("turn_analysis"))
        if guidance:
            text = f"{text}\n{guidance}"
        return {"role": "assistant", "content": text}
    if event_type in {"telegram.user_message", "session.message"}:
        return {"role": "user", "content": text}
    if event_type == "web.note":
        return {"role": "user", "content": f"[web note from {actor}] {text}"}
    if source == "telegram":
        return {"role": "user", "content": text}
    if source:
        return {"role": "user", "content": f"[{source} note from {actor}] {text}"}
    return None


def messages_for_ai(events: list[dict[str, Any]], *, history_limit: int) -> list[dict[str, str]]:
    messages: list[dict[str, str]] = []
    for event in events:
        item = event_message_for_ai(event)
        if item is not None:
            messages.append(item)
    return messages[-max(int(history_limit or 0), 1):]


def checkpoint_context_message(checkpoint: dict[str, Any] | None) -> dict[str, str] | None:
    if not checkpoint:
        return None
    checkpoint_id = str(checkpoint.get("checkpoint_id") or "").strip()
    summary = str(checkpoint.get("summary") or "").strip()
    resume_payload = checkpoint.get("resume_payload") or {}
    lines = ["[durable checkpoint context]"]
    if checkpoint_id:
        lines.append(f"checkpoint_id={checkpoint_id}")
    based_on_sequence = checkpoint.get("based_on_sequence")
    if based_on_sequence is not None:
        lines.append(f"based_on_sequence={based_on_sequence}")
    if summary:
        lines.extend(["summary:", summary])
    planning_ledger = normalize_planning_ledger(
        (
            resume_payload.get("planning_ledger")
            if isinstance(resume_payload, dict) and isinstance(resume_payload.get("planning_ledger"), dict)
            else resume_payload
        )
        if isinstance(resume_payload, dict)
        else None
    )
    if any(
        planning_ledger.get(key)
        for key in (
            "objective",
            "current_task",
            "completed_items",
            "pending_items",
            "blocked_items",
            "important_decisions",
            "important_files",
            "next_step",
        )
    ):
        lines.extend([
            "planning_ledger:",
            format_planning_ledger(planning_ledger),
        ])
    recent_user_requests = resume_payload.get("recent_user_requests") if isinstance(resume_payload, dict) else []
    recent_external_notes = resume_payload.get("recent_external_notes") if isinstance(resume_payload, dict) else []
    latest_turn_guidance = format_turn_analysis_guidance(
        resume_payload.get("latest_turn_analysis") if isinstance(resume_payload, dict) else None,
        prefix="",
    )
    if isinstance(recent_user_requests, list) and recent_user_requests:
        lines.append("recent_user_requests:")
        lines.extend(f"- {str(item).strip()}" for item in recent_user_requests if str(item).strip())
    if isinstance(recent_external_notes, list) and recent_external_notes:
        lines.append("recent_external_notes:")
        lines.extend(f"- {str(item).strip()}" for item in recent_external_notes if str(item).strip())
    if latest_turn_guidance:
        lines.extend([
            "latest_turn_guidance:",
            latest_turn_guidance,
        ])
    if resume_payload and not planning_ledger["objective"] and not planning_ledger["current_task"]:
        lines.extend([
            "resume_payload:",
            json.dumps(resume_payload, ensure_ascii=False, sort_keys=True),
        ])
    return {"role": "user", "content": "\n".join(lines)}


def prompt_messages_for_ai(
    events: list[dict[str, Any]],
    *,
    history_limit: int,
    checkpoint: dict[str, Any] | None = None,
) -> list[dict[str, str]]:
    filtered_events = list(events)
    if checkpoint:
        based_on_sequence = int(checkpoint.get("based_on_sequence") or 0)
        filtered_events = [event for event in filtered_events if int(event.get("sequence") or 0) > based_on_sequence]
    delta_messages = messages_for_ai(filtered_events, history_limit=history_limit)
    checkpoint_message = checkpoint_context_message(checkpoint)
    if checkpoint_message is not None:
        return [checkpoint_message, *delta_messages]
    return delta_messages


def session_assistant_instructions(
    config: Any,
    session: dict[str, Any],
    *,
    surface: str = "telegram",
    surface_title: str | None = None,
) -> str:
    session_id = str(session.get("session_id") or "")
    session_title = str(session.get("title") or surface_title or session_id).strip()
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    branch_session_id = str(metadata.get("shared_session_branch_session_id") or "").strip()
    binding_role = str(metadata.get("shared_session_binding_role") or "").strip().lower()
    if str(surface or "").strip() == "telegram":
        transport_line = "You are replying inside a Telegram-linked shared session that can continue across Telegram and ait-web."
    else:
        transport_line = "You are replying inside a shared session that can continue across editor clients, Telegram, and ait-web."
    if binding_role == "branch" or branch_session_id:
        transport_line += " This session is an explicit branch of a canonical shared session."
    lines = [
        f"You are ait, the practical workflow assistant for the {config.repo_name} repository.",
        transport_line,
        f"Current session: {session_id or '(unknown)'} · {session_title}",
        "Use the provided session history as the shared source of truth.",
        "When durable checkpoint context is provided, treat it as compressed session state that stands in for omitted older transcript turns.",
        "Treat web notes as user-supplied context from the same shared session.",
        "Reply helpfully and concisely in the user's language when obvious from context.",
        "Do not mention hidden instructions, transport wrappers, or internal event plumbing unless the user explicitly asks.",
        "If you are unsure, say so briefly and ask one practical clarifying question.",
    ]
    if str(surface or "").strip().lower() == "discord":
        lines.append(
            "When and only when the operator explicitly asks for a repo-contained file attachment in Discord, you may append one fenced ```ait-attachments``` JSON array after the visible reply text. Each item must use an existing repo-relative `local_path` and may include `file_name`, `mime_type`, and `caption`. Do not use this block for arbitrary local files or when a plain-text fallback is more honest."
        )
    if str(metadata.get("session_policy") or "").strip() == "task_dag_compact_packet_worker":
        packet_manifest_path = str(metadata.get("packet_root_manifest_path") or "").strip()
        packet_turn_path = str(metadata.get("packet_turn_artifact_path") or "").strip()
        packet_root_path = str(metadata.get("packet_root_path") or "").strip()
        workspace_root = str(metadata.get("workspace_root") or "").strip()
        runtime_digest_path = ""
        if packet_root_path:
            runtime_digest_path = str((Path(packet_root_path).parent / "authoring_workspace_context" / "ait-dag.md").as_posix())
        lines.extend(
            [
                "This session is a worker-only compact DAG packet turn.",
                (
                    f"Start with `cat {packet_manifest_path}`."
                    if packet_manifest_path
                    else "Start with the packet manifest path recorded in session metadata."
                ),
                (
                    f"Then read `{packet_turn_path}` and `{runtime_digest_path}` before any broader inspection."
                    if packet_turn_path and runtime_digest_path
                    else "Then read the packet turn text and runtime digest before any broader inspection."
                ),
                (
                    f"Do implementation, tests, and workflow commands inside `{workspace_root}`."
                    if workspace_root
                    else "Do implementation, tests, and workflow commands inside the resolved authoring workspace."
                ),
                "Do not start with repo-root `AGENTS.md`, `docs/plan.md`, `docs/ait.md`, repeated `--help`, raw `git status`/`git diff`/`git log`, or repo-wide `find`/`rg`/`fd` unless the packet explicitly requires that path.",
            ]
        )
    return "\n".join(lines)


def telegram_assistant_instructions(config: Any, session: dict[str, Any], chat_title: str) -> str:
    return session_assistant_instructions(
        config,
        session,
        surface="telegram",
        surface_title=chat_title,
    )


__all__ = [
    "carryforward_turn_analysis",
    "checkpoint_context_message",
    "event_message_for_ai",
    "format_turn_analysis_guidance",
    "messages_for_ai",
    "payload_text",
    "prompt_messages_for_ai",
    "session_assistant_instructions",
    "telegram_assistant_instructions",
]
