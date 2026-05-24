from __future__ import annotations

import re
from typing import Any


MAX_LIST_ITEMS = 12
FILE_PATH_PATTERN = re.compile(
    r"\b(?:docs/)?[A-Za-z0-9_./-]+\.(?:py|mjs|cjs|js|ts|tsx|jsx|json|css|html|md|sh|yml|yaml|toml|ini|sql|txt)\b"
)


def clean_string(value: Any, max_length: int = 800) -> str:
    if not isinstance(value, str):
        return ""
    return re.sub(r"\s+", " ", value).strip()[:max_length]


def clean_multiline_text(value: Any, max_length: int = 4000) -> str:
    if not isinstance(value, str):
        return ""
    text = str(value).replace("\r", "")
    lines = [line.rstrip() for line in text.splitlines()]
    return "\n".join(lines).strip()[:max_length]


def clean_list(values: Any, max_items: int = MAX_LIST_ITEMS, max_length: int = 220) -> list[str]:
    if not isinstance(values, list):
        return []
    unique: list[str] = []
    seen: set[str] = set()
    for value in values:
        cleaned = clean_string(value, max_length=max_length)
        key = cleaned.lower()
        if not cleaned or key in seen:
            continue
        seen.add(key)
        unique.append(cleaned)
        if len(unique) >= max_items:
            break
    return unique


def extract_important_files(*texts: Any, limit: int = 16) -> list[str]:
    matches: list[str] = []
    seen: set[str] = set()
    for text in texts:
        if not isinstance(text, str):
            continue
        for match in FILE_PATH_PATTERN.finditer(text):
            candidate = clean_string(match.group(0).lstrip("./"), max_length=260)
            key = candidate.lower()
            if not candidate or key in seen:
                continue
            seen.add(key)
            matches.append(candidate)
            if len(matches) >= limit:
                return matches
    return matches


def normalize_planning_ledger(ledger: dict[str, Any] | None = None) -> dict[str, Any]:
    payload = ledger or {}
    return {
        "objective": clean_string(payload.get("objective"), 240),
        "current_task": clean_string(
            payload.get("current_task") or payload.get("currentTask"),
            240,
        ),
        "completed_items": clean_list(
            payload.get("completed_items") or payload.get("completedItems"),
        ),
        "pending_items": clean_list(
            payload.get("pending_items") or payload.get("pendingItems"),
        ),
        "blocked_items": clean_list(
            payload.get("blocked_items") or payload.get("blockedItems"),
        ),
        "important_decisions": clean_list(
            payload.get("important_decisions") or payload.get("importantDecisions"),
        ),
        "important_files": clean_list(
            payload.get("important_files") or payload.get("importantFiles"),
            max_items=16,
            max_length=260,
        ),
        "next_step": clean_string(
            payload.get("next_step") or payload.get("nextStep"),
            240,
        ),
    }


def merge_planning_ledger(
    base_ledger: dict[str, Any] | None = None,
    delta_ledger: dict[str, Any] | None = None,
) -> dict[str, Any]:
    base = normalize_planning_ledger(base_ledger)
    delta = normalize_planning_ledger(delta_ledger)
    completed = {item.lower() for item in delta["completed_items"]}
    return normalize_planning_ledger(
        {
            "objective": delta["objective"] or base["objective"],
            "current_task": delta["current_task"] or base["current_task"],
            "completed_items": [*base["completed_items"], *delta["completed_items"]],
            "pending_items": [
                *[
                    item
                    for item in base["pending_items"]
                    if item.lower() not in completed
                ],
                *delta["pending_items"],
            ],
            "blocked_items": [
                *[
                    item
                    for item in base["blocked_items"]
                    if item.lower() not in completed
                ],
                *delta["blocked_items"],
            ],
            "important_decisions": [
                *base["important_decisions"],
                *delta["important_decisions"],
            ],
            "important_files": [*base["important_files"], *delta["important_files"]],
            "next_step": delta["next_step"] or base["next_step"],
        }
    )


def format_planning_ledger(ledger: dict[str, Any] | None = None) -> str:
    normalized = normalize_planning_ledger(ledger)
    return "\n".join(
        [
            f"Objective: {normalized['objective'] or 'Unknown'}",
            f"Current task: {normalized['current_task'] or 'Unknown'}",
            f"Completed: {'; '.join(normalized['completed_items']) or 'None'}",
            f"Pending: {'; '.join(normalized['pending_items']) or 'None'}",
            f"Blocked: {'; '.join(normalized['blocked_items']) or 'None'}",
            f"Decisions: {'; '.join(normalized['important_decisions']) or 'None'}",
            f"Important files: {'; '.join(normalized['important_files']) or 'None'}",
            f"Next step: {normalized['next_step'] or 'Unknown'}",
        ]
    )


def build_checkpoint_planning_ledger(
    *,
    previous_ledger: dict[str, Any] | None,
    latest_user_request: str,
    latest_assistant_reply: str,
    recent_user_requests: list[str],
    recent_external_notes: list[str],
    summary_text: str,
) -> dict[str, Any]:
    return merge_planning_ledger(
        previous_ledger,
        {
            "objective": latest_user_request,
            "current_task": latest_user_request,
            "important_files": extract_important_files(
                latest_user_request,
                latest_assistant_reply,
                summary_text,
                *recent_user_requests,
                *recent_external_notes,
            ),
            "next_step": latest_assistant_reply,
        },
    )
