from __future__ import annotations

from typing import Any

HELP_FLAGS = frozenset({"--help", "-h"})
WORKFLOW_GUIDE_TOPICS = {
    "inventory": {
        "summary": "Use one inventory surface first, then drill down only where the workflow actually points.",
        "when_to_use": [
            "You need to answer what remains or what should land next.",
            "You are about to rerun queue, task list, or change list in the same turn.",
        ],
        "commands": [
            {
                "label": "Shared queue",
                "command": "ait queue summary --all-changes",
                "detail": "Use this first for the shared non-landed picture across tasks and changes.",
            },
            {
                "label": "One task readiness",
                "command": "ait task audit <task-id>",
                "detail": "Prefer this over rebuilding one task from `task show` plus task-scoped `change list`.",
            },
            {
                "label": "One change detail",
                "command": "ait change show <change-id>",
                "detail": "Open the focus change only after the queue or task audit points you there.",
            },
        ],
        "avoid": [
            "Do not rerun the same queue or list command in the same turn unless workflow state changed.",
        ],
    },
    "land": {
        "summary": "Use the workflow-land helper as the public remote-land path instead of rediscovering low-level gates by hand.",
        "when_to_use": [
            "You want to see what still blocks one remote change from landing.",
            "You want the helper to advance safe remote-land steps without teaching the low-level gate commands first.",
        ],
        "commands": [
            {
                "label": "Workflow land helper",
                "command": "ait workflow land <change-id>",
                "detail": "Show the current patchset, attestation, review, policy, and land gate for one change in one view.",
            },
            {
                "label": "Advance safe steps",
                "command": "ait workflow land <change-id> --apply",
                "detail": "Create any needed snapshot or patchset updates, continue review and policy gates, and stop only when a real blocker still needs attention.",
            },
            {
                "label": "Complete task",
                "command": "ait task complete <task-id>",
                "detail": "Close the task once the landed change fully satisfied the goal.",
            },
        ],
        "avoid": [
            "Do not rediscover the same land path with many separate low-level gate or help commands in one turn.",
        ],
    },
}
WORKFLOW_GUIDE_LAND_TOP_LEVELS = frozenset(
    {"attest", "land", "patchset", "policy", "review", "snapshot", "task", "workspace", "worktree"}
)
WORKFLOW_GUIDE_INVENTORY_TOP_LEVELS = frozenset({"change", "queue", "task"})


def _workflow_guide_payload(topic: str | None = None) -> dict[str, Any]:
    if topic is None:
        return {
            "topics": [
                {
                    "topic": name,
                    "summary": str(payload["summary"]),
                    "command": f"ait workflow guide {name}",
                }
                for name, payload in WORKFLOW_GUIDE_TOPICS.items()
            ]
        }
    key = str(topic or "").strip().lower()
    payload = WORKFLOW_GUIDE_TOPICS.get(key)
    if payload is None:
        raise KeyError(f"Unknown workflow guide topic: {topic}. Available topics: {', '.join(sorted(WORKFLOW_GUIDE_TOPICS))}")
    return {"topic": key, **payload}


def _render_workflow_guide_text(data: dict[str, Any]) -> str:
    topic = str(data.get("topic") or "").strip()
    if not topic:
        lines = ["ait workflow guides", ""]
        for row in data.get("topics") or []:
            if not isinstance(row, dict):
                continue
            lines.append(f"- {row.get('topic')}: {row.get('summary')} ({row.get('command')})")
        return "\n".join(lines)
    lines = [f"ait workflow guide · {topic}", "", str(data.get("summary") or "").strip()]
    when_to_use = [str(item).strip() for item in data.get("when_to_use") or [] if str(item).strip()]
    if when_to_use:
        lines.extend(["", "When to use", *[f"- {item}" for item in when_to_use]])
    commands = [row for row in data.get("commands") or [] if isinstance(row, dict)]
    if commands:
        lines.extend(["", "Recommended commands"])
        for row in commands:
            label = str(row.get("label") or "").strip()
            command = str(row.get("command") or "").strip()
            detail = str(row.get("detail") or "").strip()
            if label and command:
                lines.append(f"- {label}: {command}")
            elif command:
                lines.append(f"- {command}")
            if detail:
                lines.append(f"  {detail}")
    avoid = [str(item).strip() for item in data.get("avoid") or [] if str(item).strip()]
    if avoid:
        lines.extend(["", "Avoid", *[f"- {item}" for item in avoid]])
    return "\n".join(lines)


def _common_token_prefix(token_groups: list[list[str]]) -> list[str]:
    if not token_groups:
        return []
    prefix = list(token_groups[0])
    for group in token_groups[1:]:
        shared = 0
        for left, right in zip(prefix, group):
            if left != right:
                break
            shared += 1
        prefix = prefix[:shared]
        if not prefix:
            break
    return prefix


def _help_entrypoint(help_targets: list[list[str]]) -> str | None:
    prefix = _common_token_prefix(help_targets)
    if not prefix:
        return None
    return " ".join([*prefix, "--help"])


def _workflow_guide_topic_for_help_rows(help_rows: list[dict[str, Any]]) -> str | None:
    top_levels = {str(row.get("top_level") or "") for row in help_rows if str(row.get("top_level") or "").strip()}
    if len(top_levels & WORKFLOW_GUIDE_LAND_TOP_LEVELS) >= 3:
        return "land"
    if top_levels and top_levels.issubset(WORKFLOW_GUIDE_INVENTORY_TOP_LEVELS) and len(top_levels) >= 2:
        return "inventory"
    return None


def _help_burst_turn(turn_commands: list[dict[str, Any]], *, turn_index: int | None = None) -> dict[str, Any] | None:
    help_rows = [command for command in turn_commands if any(token in HELP_FLAGS for token in command.get("tokens") or [])]
    if len(help_rows) < 2:
        return None
    workflow_topic = _workflow_guide_topic_for_help_rows(help_rows)
    suggested_command = f"ait workflow guide {workflow_topic}" if workflow_topic else None
    detail = "Reuse one broader help entry point before drilling into narrower subcommands."
    if suggested_command:
        detail = f"Start with `{suggested_command}` before walking several narrower help screens for the same flow."
    else:
        help_targets = [
            [token for token in (row.get("tokens") or []) if token not in HELP_FLAGS]
            for row in help_rows
        ]
        suggested_command = _help_entrypoint(help_targets)
        if suggested_command:
            detail = f"Start with `{suggested_command}` once before drilling into narrower help commands."
    cluster = {
        "code": "help_burst",
        "summary": "This turn reopened several help screens for the same workflow.",
        "detail": detail,
        "matched_commands": [row["raw_command"] for row in help_rows[:5]],
        "matched_count": len(help_rows),
        "suggested_command": suggested_command,
    }
    if workflow_topic:
        cluster["workflow_topic"] = workflow_topic
    if turn_index is not None:
        cluster["turn_index"] = turn_index
    return cluster
