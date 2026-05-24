from __future__ import annotations

import re
from typing import Any, Mapping

TASK_ID_PATTERN = re.compile(r"\b((?:AIT|L|R)?T-[A-Z0-9]+)\b", re.IGNORECASE)
CHANGE_ID_PATTERN = re.compile(r"\b((?:AIT|L|R)?C-[A-Z0-9]+)\b", re.IGNORECASE)
PLAN_ID_PATTERN = re.compile(r"\b(PL-[A-Z0-9]+)\b", re.IGNORECASE)


def parse_command(text: str, username: str) -> tuple[str, str] | None:
    raw = str(text or "").strip()
    if not raw.startswith("/"):
        return None
    first, _, rest = raw.partition(" ")
    command = first[1:].strip()
    if not command:
        return None
    if "@" in command:
        command_name, _, target = command.partition("@")
        if username and target and target.lower() != username.lower():
            return None
        command = command_name
    return command.lower(), rest.strip()


def detect_workflow_query(text: str) -> tuple[str, str | None] | None:
    normalized = str(text or "").strip()
    lowered = normalized.lower()
    if lowered in {"queue", "task queue", "queue summary", "what remains", "what should land next"}:
        return ("queue", None)
    if lowered in {"attention", "needs attention", "what needs attention", "what is blocked"}:
        return ("attention", None)
    if lowered in {"ready", "ready to land", "ready to complete", "what can land", "what can complete"}:
        return ("ready", None)
    task_match = TASK_ID_PATTERN.search(normalized)
    if task_match and lowered.startswith(("task", "aitt-", "lt-", "rt-", "t-", "任務")):
        return ("task", task_match.group(1).upper())
    if task_match and lowered.startswith(("audit", "task audit")):
        return ("audit", task_match.group(1).upper())
    change_match = CHANGE_ID_PATTERN.search(normalized)
    if change_match and lowered.startswith(("change", "aitc-", "lc-", "rc-", "c-", "變更")):
        return ("change", change_match.group(1).upper())
    if change_match and lowered.startswith(("land", "change land", "land readiness")):
        return ("land", change_match.group(1).upper())
    return None


def chat_title(chat: dict[str, Any]) -> str:
    if chat.get("title"):
        return str(chat["title"])
    first = str(chat.get("first_name") or "").strip()
    last = str(chat.get("last_name") or "").strip()
    username = str(chat.get("username") or "").strip()
    full = " ".join(part for part in [first, last] if part).strip()
    return full or (f"@{username}" if username else str(chat.get("id") or "telegram-chat"))


def actor_identity(from_user: dict[str, Any], chat_id: str | int) -> str:
    user_id = str(from_user.get("id") or chat_id)
    username = str(from_user.get("username") or "").strip()
    if username:
        return f"telegram:{user_id}:@{username}"
    return f"telegram:{user_id}"


def user_display_name(from_user: Mapping[str, Any]) -> str | None:
    first = str(from_user.get("first_name") or "").strip()
    last = str(from_user.get("last_name") or "").strip()
    full = " ".join(part for part in [first, last] if part).strip()
    if full:
        return full
    username = str(from_user.get("username") or "").strip()
    if username:
        return f"@{username}"
    return None
