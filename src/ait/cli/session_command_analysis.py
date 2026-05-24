from __future__ import annotations

import shlex
from collections import Counter
from typing import Any

from .session_analysis_merge_helpers import (
    AIT_DUPLICATE_INVENTORY_COMMAND_PATHS,
    AIT_INVENTORY_COMMAND_PATHS,
    _inventory_burst_turn,
    _land_workflow_turn,
    _task_audit_turn_merge,
    _task_start_turn_merge,
)
from .workflow_guides import HELP_FLAGS, _help_burst_turn, _workflow_guide_topic_for_help_rows


def _normalize_text_value(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


AIT_TOP_LEVEL_COMMANDS = frozenset(
    {
        "auth",
        "attest",
        "change",
        "config",
        "doctor",
        "gc",
        "history",
        "init",
        "land",
        "line",
        "patchset",
        "policy",
        "pull",
        "push",
        "queue",
        "ref",
        "remote",
        "repo",
        "review",
        "session",
        "stash",
        "snapshot",
        "stack",
        "status",
        "task",
        "workflow",
        "workspace",
        "worktree",
    }
)

AIT_COMMAND_GROUPS_WITH_SUBCOMMAND = frozenset(
    {
        "auth",
        "attest",
        "change",
        "config",
        "doctor",
        "gc",
        "land",
        "line",
        "patchset",
        "policy",
        "queue",
        "ref",
        "remote",
        "repo",
        "review",
        "session",
        "stash",
        "snapshot",
        "stack",
        "task",
        "workflow",
        "workspace",
        "worktree",
    }
)

AIT_COMMAND_WRAPPER_TOKENS = frozenset(
    {
        "bash",
        "builtin",
        "command",
        "env",
        "exec",
        "gtimeout",
        "nice",
        "noglob",
        "nohup",
        "pipenv",
        "poetry",
        "python",
        "python3",
        "run",
        "sh",
        "stdbuf",
        "time",
        "timeout",
        "uv",
        "uvx",
        "zsh",
    }
)

AIT_MODULE_NAMES = frozenset({"ait", "ait.cli", "ait_native.cli"})
AIT_PREFIX_CONTROL_TOKENS = frozenset({"!", "do", "elif", "else", "then"})
AIT_TURN_USER_EVENT_TYPES = frozenset({"session.message", "telegram.user_message", "web.note"})
AIT_SHOW_COMMAND_PATHS = frozenset(
    {
        "change show",
        "land show",
        "line show",
        "patchset show",
        "policy show",
        "ref show",
        "repo show",
        "review show",
        "session checkpoint-show",
        "session show",
        "stash show",
        "snapshot show",
        "stack show",
        "task show",
        "worktree show",
    }
)
AIT_TASK_CLOSE_COMMAND_PATHS = frozenset({"task canceled", "task complete"})
SHELL_SEGMENT_SEPARATOR_TOKENS = frozenset({"&&", "||", "|"})
def _is_ait_wrapper_token(token: str) -> bool:
    normalized = str(token or "").rsplit("/", 1)[-1]
    return normalized in AIT_COMMAND_WRAPPER_TOKENS or normalized.startswith("python")


def _is_shell_env_assignment(token: str) -> bool:
    text = str(token or "").strip()
    if "=" not in text or text.startswith("="):
        return False
    name, _, _value = text.partition("=")
    if not name:
        return False
    if not (name[0].isalpha() or name[0] == "_"):
        return False
    return all(char.isalnum() or char == "_" for char in name[1:])


def _is_shell_duration_or_number_token(token: str) -> bool:
    text = str(token or "").strip()
    if not text:
        return False
    if text[0] in {"+", "-"}:
        text = text[1:]
    if not text:
        return False
    if text.isdigit():
        return True
    if len(text) >= 2 and text[:-1].isdigit() and text[-1] in {"s", "m", "h", "d"}:
        return True
    return False


def _is_ignorable_ait_prefix_token(token: str) -> bool:
    text = str(token or "").strip()
    if not text:
        return False
    return (
        _is_ait_wrapper_token(text)
        or _is_shell_env_assignment(text)
        or _is_shell_duration_or_number_token(text)
        or text in AIT_PREFIX_CONTROL_TOKENS
    )


def _shell_split_command(raw: str) -> list[str]:
    text = str(raw or "").strip()
    if not text:
        return []
    try:
        return shlex.split(text)
    except ValueError:
        return text.split()


def _is_ait_command_token(token: str) -> bool:
    return str(token or "").rsplit("/", 1)[-1] == "ait"


def _split_shell_command_segments(tokens: list[str]) -> list[list[str]]:
    segments: list[list[str]] = []
    current: list[str] = []
    for token in tokens:
        if token in SHELL_SEGMENT_SEPARATOR_TOKENS:
            if current:
                segments.append(current)
                current = []
            continue
        stripped = token.rstrip(";")
        if stripped:
            current.append(stripped)
        if stripped != token:
            if current:
                segments.append(current)
                current = []
    if current:
        segments.append(current)
    return segments or [tokens]


def _find_wrapped_ait_token(tokens: list[str]) -> int | None:
    for idx, token in enumerate(tokens):
        if not _is_ait_command_token(token):
            continue
        prefix = [item for item in tokens[:idx] if item and not item.startswith("-")]
        if all(_is_ignorable_ait_prefix_token(item) for item in prefix):
            return idx
    return None


def _find_wrapped_ait_module(tokens: list[str]) -> int | None:
    for idx in range(len(tokens) - 1):
        if tokens[idx] != "-m":
            continue
        module_name = tokens[idx + 1]
        prefix = [item for item in tokens[:idx] if item and not item.startswith("-")]
        if module_name in AIT_MODULE_NAMES and all(_is_ignorable_ait_prefix_token(item) for item in prefix):
            return idx
    return None


def _extract_ait_command(raw_command: Any) -> dict[str, Any] | None:
    raw_text = str(raw_command or "").strip()
    if not raw_text:
        return None
    tokens = _shell_split_command(raw_text)
    if not tokens:
        return None
    for segment in _split_shell_command_segments(tokens):
        if not segment:
            continue
        remainder: list[str]
        if segment[0] in AIT_TOP_LEVEL_COMMANDS:
            remainder = segment
        else:
            module_idx = _find_wrapped_ait_module(segment)
            if module_idx is not None:
                remainder = segment[module_idx + 2 :]
            else:
                ait_idx = _find_wrapped_ait_token(segment)
                if ait_idx is None:
                    continue
                remainder = segment[ait_idx + 1 :]
        if not remainder:
            continue
        top_level = remainder[0]
        if top_level not in AIT_TOP_LEVEL_COMMANDS:
            continue
        command_path_tokens = [top_level]
        if top_level in AIT_COMMAND_GROUPS_WITH_SUBCOMMAND and len(remainder) > 1 and not remainder[1].startswith("-"):
            command_path_tokens.append(remainder[1])
        command_path = " ".join(command_path_tokens)
        trailing_tokens = remainder[len(command_path_tokens) :]
        target = next((token for token in trailing_tokens if token != "--" and not token.startswith("-")), None)
        signature = f"ait {command_path}" + (f" {target}" if target else "")
        return {
            "raw_command": raw_text,
            "tokens": list(segment),
            "top_level": top_level,
            "command_path": command_path,
            "target": target,
            "signature": signature,
        }
    return None


def _session_event_command_values(payload: dict[str, Any]) -> list[str]:
    raw_values: list[str] = []
    raw_command = payload.get("command")
    if isinstance(raw_command, str) and raw_command.strip():
        raw_values.append(raw_command.strip())
    elif isinstance(raw_command, list):
        raw_values.extend(str(item).strip() for item in raw_command if str(item).strip())
    raw_commands = payload.get("commands")
    if isinstance(raw_commands, list):
        raw_values.extend(str(item).strip() for item in raw_commands if str(item).strip())
    return raw_values


def _session_command_phase(payload: dict[str, Any]) -> str | None:
    value = _normalize_text_value(payload.get("command_phase"))
    if value is None:
        return None
    lowered = value.lower()
    if lowered in {"started", "finished"}:
        return lowered
    return None


def _extract_session_ait_commands(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    commands: list[dict[str, Any]] = []
    ordinal = 0
    started_command_ids: set[str] = set()
    for event in events:
        payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
        raw_values = _session_event_command_values(payload)
        command_id = _normalize_text_value(payload.get("command_id"))
        phase = _session_command_phase(payload)
        for raw_value in raw_values:
            parsed = _extract_ait_command(raw_value)
            if parsed is None:
                continue
            if phase == "finished" and command_id is not None and command_id in started_command_ids:
                continue
            ordinal += 1
            commands.append(
                {
                    **parsed,
                    "sequence": int(event.get("sequence") or 0),
                    "event_type": str(event.get("event_type") or ""),
                    "actor_identity": str(event.get("actor_identity") or ""),
                    "command_id": command_id,
                    "command_phase": phase,
                    "capture_mode": _normalize_text_value(payload.get("capture_mode")) or "manual",
                    "order": ordinal,
                }
            )
            if phase == "started" and command_id is not None:
                started_command_ids.add(command_id)
    return commands


def _session_turn_role(event: dict[str, Any]) -> str | None:
    event_type = str(event.get("event_type") or "")
    if event_type.startswith("assistant."):
        return "assistant"
    if event_type in AIT_TURN_USER_EVENT_TYPES:
        return "user"
    return None


def _int_or_zero(value: Any) -> int:
    try:
        return int(value or 0)
    except (TypeError, ValueError):
        return 0


def _normalize_turn_analysis_payload(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    command_count = _int_or_zero(value.get("command_count"))
    distinct_command_count = _int_or_zero(value.get("distinct_command_count"))
    optimization_summary = str(value.get("optimization_summary") or "").strip()
    commands = [str(row).strip() for row in value.get("commands") or [] if str(row).strip()]
    top_commands = [dict(row) for row in value.get("top_commands") or [] if isinstance(row, dict)]
    optimization_hints = [dict(row) for row in value.get("optimization_hints") or [] if isinstance(row, dict)]
    if command_count <= 0 and not optimization_summary and not optimization_hints:
        return None
    return {
        "command_count": command_count,
        "distinct_command_count": distinct_command_count,
        "commands": commands,
        "top_commands": top_commands,
        "optimization_hints": optimization_hints,
        "optimization_summary": optimization_summary,
    }


def _turn_analysis_hint_codes(turn_analysis: dict[str, Any] | None) -> list[str]:
    if not isinstance(turn_analysis, dict):
        return []
    seen: set[str] = set()
    codes: list[str] = []
    for row in turn_analysis.get("optimization_hints") or []:
        if not isinstance(row, dict):
            continue
        code = str(row.get("code") or "").strip()
        if not code or code in seen:
            continue
        seen.add(code)
        codes.append(code)
    return codes


def _signature_command_path(signature: str) -> str | None:
    parts = str(signature or "").split()
    if len(parts) >= 3:
        return " ".join(parts[1:3])
    if len(parts) >= 2:
        return parts[1]
    return None


def _finalize_session_turn(turn: dict[str, Any]) -> dict[str, Any]:
    command_rows = [
        {
            "command_path": command_path,
            "count": count,
            "example": turn["command_examples"].get(command_path),
        }
        for command_path, count in sorted(turn["command_counts"].items(), key=lambda item: (-item[1], item[0]))
    ]
    return {
        "turn_index": turn["turn_index"],
        "user_event_sequence": turn["user_event_sequence"],
        "user_event_type": turn["user_event_type"],
        "assistant_reply_sequence": turn["assistant_reply_sequence"],
        "assistant_event_type": turn["assistant_event_type"],
        "start_sequence": turn["start_sequence"],
        "end_sequence": turn["end_sequence"],
        "ait_command_count": turn["ait_command_count"],
        "distinct_command_paths": len(turn["command_counts"]),
        "command_paths": command_rows,
        "codex_command_count": _int_or_zero(turn.get("codex_command_count")),
        "codex_distinct_command_count": _int_or_zero(turn.get("codex_distinct_command_count")),
        "codex_optimization_summary": str(turn.get("codex_optimization_summary") or ""),
        "codex_hint_codes": list(turn.get("codex_hint_codes") or []),
        "codex_top_commands": list(turn.get("codex_top_commands") or []),
    }


def _build_session_turns(events: list[dict[str, Any]], commands: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    commands_by_sequence: dict[int, list[dict[str, Any]]] = {}
    for command in commands:
        commands_by_sequence.setdefault(int(command["sequence"]), []).append(command)
    turns: list[dict[str, Any]] = []
    unscoped_commands: list[dict[str, Any]] = []
    current_turn: dict[str, Any] | None = None
    next_turn_index = 1
    for event in events:
        sequence = int(event.get("sequence") or 0)
        role = _session_turn_role(event)
        if role == "user":
            if current_turn is not None:
                turns.append(_finalize_session_turn(current_turn))
            current_turn = {
                "turn_index": next_turn_index,
                "user_event_sequence": sequence,
                "user_event_type": str(event.get("event_type") or ""),
                "assistant_reply_sequence": None,
                "assistant_event_type": None,
                "start_sequence": sequence,
                "end_sequence": sequence,
                "ait_command_count": 0,
                "command_counts": Counter(),
                "command_examples": {},
                "codex_command_count": 0,
                "codex_distinct_command_count": 0,
                "codex_optimization_summary": "",
                "codex_hint_codes": [],
                "codex_top_commands": [],
            }
            next_turn_index += 1
        for command in commands_by_sequence.get(sequence, []):
            if current_turn is None:
                unscoped_commands.append(command)
                continue
            current_turn["ait_command_count"] += 1
            current_turn["end_sequence"] = max(int(current_turn["end_sequence"]), sequence)
            current_turn["command_counts"][command["command_path"]] += 1
            current_turn["command_examples"].setdefault(command["command_path"], command["raw_command"])
        if role == "assistant" and current_turn is not None:
            payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
            turn_analysis = _normalize_turn_analysis_payload(payload.get("turn_analysis"))
            if turn_analysis is not None:
                current_turn["codex_command_count"] = int(turn_analysis["command_count"])
                current_turn["codex_distinct_command_count"] = int(turn_analysis["distinct_command_count"])
                current_turn["codex_optimization_summary"] = str(turn_analysis["optimization_summary"])
                current_turn["codex_hint_codes"] = _turn_analysis_hint_codes(turn_analysis)
                current_turn["codex_top_commands"] = list(turn_analysis.get("top_commands") or [])[:3]
            current_turn["assistant_reply_sequence"] = sequence
            current_turn["assistant_event_type"] = str(event.get("event_type") or "")
            current_turn["end_sequence"] = max(int(current_turn["end_sequence"]), sequence)
            turns.append(_finalize_session_turn(current_turn))
            current_turn = None
    if current_turn is not None:
        turns.append(_finalize_session_turn(current_turn))
    return turns, unscoped_commands


def _detect_repeated_command_runs(commands: list[dict[str, Any]]) -> list[dict[str, Any]]:
    runs: list[dict[str, Any]] = []
    current: dict[str, Any] | None = None
    for command in commands:
        signature = str(command["signature"])
        if current is not None and current["signature"] == signature:
            current["count"] += 1
            current["end_sequence"] = int(command["sequence"])
            current["sequences"].append(int(command["sequence"]))
            continue
        if current is not None and current["count"] > 1:
            runs.append(current)
        current = {
            "signature": signature,
            "command_path": command["command_path"],
            "count": 1,
            "start_sequence": int(command["sequence"]),
            "end_sequence": int(command["sequence"]),
            "sequences": [int(command["sequence"])],
            "example": command["raw_command"],
        }
    if current is not None and current["count"] > 1:
        runs.append(current)
    return runs


def _commands_for_turn(turn: dict[str, Any], commands: list[dict[str, Any]]) -> list[dict[str, Any]]:
    start_sequence = int(turn.get("start_sequence") or 0)
    end_sequence = int(turn.get("end_sequence") or 0)
    return [
        command
        for command in commands
        if start_sequence <= int(command.get("sequence") or 0) <= end_sequence
    ]


def _build_codex_turns(events: list[dict[str, Any]], turns: list[dict[str, Any]]) -> list[dict[str, Any]]:
    turn_index_by_reply_sequence = {
        _int_or_zero(turn.get("assistant_reply_sequence")): _int_or_zero(turn.get("turn_index"))
        for turn in turns
        if _int_or_zero(turn.get("assistant_reply_sequence")) > 0
    }
    codex_turns: list[dict[str, Any]] = []
    for event in events:
        if str(event.get("event_type") or "") != "assistant.reply":
            continue
        payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
        turn_analysis = _normalize_turn_analysis_payload(payload.get("turn_analysis"))
        if turn_analysis is None:
            continue
        assistant_sequence = _int_or_zero(event.get("sequence"))
        codex_turns.append(
            {
                "turn_index": turn_index_by_reply_sequence.get(assistant_sequence),
                "assistant_reply_sequence": assistant_sequence,
                "command_count": int(turn_analysis["command_count"]),
                "distinct_command_count": int(turn_analysis["distinct_command_count"]),
                "commands": list(turn_analysis.get("commands") or []),
                "top_commands": list(turn_analysis.get("top_commands") or []),
                "optimization_hints": list(turn_analysis.get("optimization_hints") or []),
                "optimization_summary": str(turn_analysis.get("optimization_summary") or ""),
                "hint_codes": _turn_analysis_hint_codes(turn_analysis),
            }
        )
    return codex_turns


def _session_codex_optimization_hints(codex_turns: list[dict[str, Any]]) -> list[dict[str, Any]]:
    aggregated: dict[str, dict[str, Any]] = {}
    for turn in codex_turns:
        for hint in turn.get("optimization_hints") or []:
            if not isinstance(hint, dict):
                continue
            code = str(hint.get("code") or "").strip()
            if not code:
                continue
            row = aggregated.setdefault(
                code,
                {
                    "code": code,
                    "summary": str(hint.get("summary") or "").strip(),
                    "detail": str(hint.get("detail") or "").strip(),
                    "turn_count": 0,
                    "command_count": 0,
                    "matched_count": 0,
                    "suggested_commands": [],
                    "turns": [],
                },
            )
            row["turn_count"] += 1
            row["command_count"] += _int_or_zero(turn.get("command_count"))
            matched_commands = [str(item).strip() for item in hint.get("matched_commands") or [] if str(item).strip()]
            row["matched_count"] += len(matched_commands)
            suggested_command = str(hint.get("suggested_command") or "").strip()
            if suggested_command and suggested_command not in row["suggested_commands"]:
                row["suggested_commands"].append(suggested_command)
            turn_row = {
                "turn_index": turn.get("turn_index"),
                "assistant_reply_sequence": turn.get("assistant_reply_sequence"),
                "command_count": turn.get("command_count"),
            }
            if suggested_command:
                turn_row["suggested_command"] = suggested_command
            if matched_commands:
                turn_row["matched_count"] = len(matched_commands)
            if len(row["turns"]) < 5:
                row["turns"].append(turn_row)
    results: list[dict[str, Any]] = []
    for row in sorted(aggregated.values(), key=lambda item: (-_int_or_zero(item.get("turn_count")), str(item.get("code") or ""))):
        suggested_commands = list(row.get("suggested_commands") or [])
        results.append(
            {
                "code": str(row.get("code") or ""),
                "summary": str(row.get("summary") or ""),
                "detail": str(row.get("detail") or ""),
                "turn_count": _int_or_zero(row.get("turn_count")),
                "command_count": _int_or_zero(row.get("command_count")),
                "matched_count": _int_or_zero(row.get("matched_count")),
                "suggested_command": suggested_commands[0] if suggested_commands else None,
                "suggested_commands": suggested_commands[:3],
                "turns": list(row.get("turns") or []),
            }
        )
    return results


def _session_merge_opportunities(
    commands: list[dict[str, Any]],
    path_counts: Counter[str],
    signature_counts: Counter[str],
    path_examples: dict[str, str],
    signature_examples: dict[str, str],
    repeated_runs: list[dict[str, Any]],
    turns: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    opportunities: list[dict[str, Any]] = []
    for turn in turns:
        turn_commands = _commands_for_turn(turn, commands)
        task_start_merge = _task_start_turn_merge(turn_commands, turn_index=int(turn.get("turn_index") or 0))
        if task_start_merge is not None:
            opportunities.append(task_start_merge)
        task_audit_merge = _task_audit_turn_merge(turn_commands, turn_index=int(turn.get("turn_index") or 0))
        if task_audit_merge is not None:
            opportunities.append(task_audit_merge)
        command_rows = turn.get("command_paths") or []
        turn_counts = {str(row.get("command_path") or ""): int(row.get("count") or 0) for row in command_rows}
        inventory_paths = [path for path in sorted(AIT_INVENTORY_COMMAND_PATHS) if turn_counts.get(path, 0)]
        if (
            len(inventory_paths) < 2
            or turn_counts.get("queue summary", 0)
            or (task_audit_merge is not None and not any(command["command_path"] == "task list" for command in turn_commands))
        ):
            continue
        observed_count = sum(turn_counts[path] for path in inventory_paths)
        suggested_command = "ait queue summary --all-changes" if turn_counts.get("change list", 0) else "ait queue summary"
        observed_examples = [
            str(row.get("example") or path_examples.get(str(row.get("command_path") or "")) or "")
            for row in command_rows
            if str(row.get("command_path") or "") in inventory_paths
        ]
        opportunities.append(
            {
                "code": "queue_summary_inventory_merge",
                "summary": "This workflow inventory turn could have been answered with one queue summary command.",
                "turn_index": int(turn.get("turn_index") or 0),
                "observed_count": observed_count,
                "minimal_count": 1,
                "avoidable_count": max(observed_count - 1, 0),
                "suggested_command": suggested_command,
                "matched_command_paths": inventory_paths,
                "observed_examples": observed_examples[:5],
            }
        )
    for run in repeated_runs:
        opportunities.append(
            {
                "code": "dedupe_repeated_command",
                "summary": "A repeated command run can be reduced to one execution.",
                "observed_count": int(run["count"]),
                "minimal_count": 1,
                "avoidable_count": max(int(run["count"]) - 1, 0),
                "suggested_command": str(run["example"]),
                "signature": str(run["signature"]),
                "sequence_range": [int(run["start_sequence"]), int(run["end_sequence"])],
            }
        )
    for signature, count in sorted(signature_counts.items(), key=lambda item: (-item[1], item[0])):
        command_path = _signature_command_path(signature)
        if count < 2 or command_path not in AIT_SHOW_COMMAND_PATHS:
            continue
        opportunities.append(
            {
                "code": "reuse_show_result",
                "summary": "Repeated reads of the same workflow object can usually be collapsed to one show command.",
                "observed_count": int(count),
                "minimal_count": 1,
                "avoidable_count": max(int(count) - 1, 0),
                "suggested_command": str(signature_examples.get(signature) or signature),
                "signature": str(signature),
            }
        )
    return opportunities


def _session_optimization_hints(
    commands: list[dict[str, Any]],
    path_counts: Counter[str],
    signature_counts: Counter[str],
    signature_examples: dict[str, str],
    repeated_runs: list[dict[str, Any]],
    turns: list[dict[str, Any]],
    codex_turns: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    hints: list[dict[str, Any]] = []
    task_start_turns: list[dict[str, Any]] = []
    task_audit_turns: list[dict[str, Any]] = []
    queue_summary_turns: list[dict[str, Any]] = []
    workflow_guide_turns: list[dict[str, Any]] = []
    workflow_land_turns: list[dict[str, Any]] = []
    for turn in turns:
        turn_commands = _commands_for_turn(turn, commands)
        task_start_merge = _task_start_turn_merge(turn_commands, turn_index=int(turn.get("turn_index") or 0))
        if task_start_merge is not None:
            task_start_turns.append(task_start_merge)
        task_audit_merge = _task_audit_turn_merge(turn_commands, turn_index=int(turn.get("turn_index") or 0))
        if task_audit_merge is not None:
            task_audit_turns.append(task_audit_merge)
        turn_counts = Counter(command["command_path"] for command in turn_commands)
        inventory_paths = [path for path in sorted(AIT_INVENTORY_COMMAND_PATHS) if turn_counts.get(path, 0)]
        if (
            len(inventory_paths) >= 2
            and turn_counts.get("queue summary", 0) == 0
            and not (task_audit_merge is not None and not turn_counts.get("task list", 0))
        ):
            queue_summary_turns.append(
                {
                    "turn_index": int(turn.get("turn_index") or 0),
                    "suggested_command": "ait queue summary --all-changes" if turn_counts.get("change list", 0) else "ait queue summary",
                    "matched_commands": [command["raw_command"] for command in turn_commands if command["command_path"] in inventory_paths][:5],
                    "matched_count": sum(int(turn_counts[path]) for path in inventory_paths),
                }
            )
        help_rows = [command for command in turn_commands if any(token in HELP_FLAGS for token in command.get("tokens") or [])]
        workflow_topic = _workflow_guide_topic_for_help_rows(help_rows)
        if workflow_topic and len(help_rows) >= 2:
            workflow_guide_turns.append(
                {
                    "turn_index": int(turn.get("turn_index") or 0),
                    "suggested_command": f"ait workflow guide {workflow_topic}",
                    "matched_commands": [command["raw_command"] for command in help_rows[:5]],
                }
            )
        land_cluster = _land_workflow_turn(turn_commands, turn_index=int(turn.get("turn_index") or 0))
        if land_cluster is not None:
            workflow_land_turns.append(land_cluster)
    if task_start_turns:
        hints.append(
            {
                "code": "prefer_task_start",
                "summary": "Some task bootstrap turns could likely use `ait task start`.",
                "detail": "When the goal is to open a task plus its first change, `ait task start` without `--task-only` can replace `ait task start --task-only` plus `ait change create`.",
                "turns": [
                    {
                        "turn_index": int(row["turn_index"]),
                        "suggested_command": str(row["suggested_command"]),
                        "matched_count": int(row["observed_count"]),
                    }
                    for row in task_start_turns[:5]
                ],
                "matched_count": sum(int(row["observed_count"]) for row in task_start_turns),
            }
        )
    if task_audit_turns:
        hints.append(
            {
                "code": "prefer_task_audit",
                "summary": "Some task-readiness turns could likely use `ait task audit`.",
                "detail": "When checking one task's readiness or target-line state, `ait task audit <task-id>` can replace separate `ait task show` and task-scoped `ait change list` reads.",
                "turns": [
                    {
                        "turn_index": int(row["turn_index"]),
                        "task_id": str(row["task_id"]),
                        "suggested_command": str(row["suggested_command"]),
                        "matched_count": int(row["observed_count"]),
                    }
                    for row in task_audit_turns[:5]
                ],
                "matched_count": sum(int(row["observed_count"]) for row in task_audit_turns),
            }
        )
    if workflow_guide_turns:
        primary = workflow_guide_turns[0]
        hints.append(
            {
                "code": "prefer_workflow_guide",
                "summary": "Some help-heavy turns could likely start with one workflow guide.",
                "detail": f"Prefer `{primary['suggested_command']}` before walking several separate `--help` screens for the same flow.",
                "matched_commands": primary["matched_commands"],
                "turns": [
                    {
                        "turn_index": int(row["turn_index"]),
                        "suggested_command": str(row["suggested_command"]),
                    }
                    for row in workflow_guide_turns[:5]
                ],
            }
        )
    if workflow_land_turns:
        primary = workflow_land_turns[0]
        hints.append(
            {
                "code": "prefer_workflow_land",
                "summary": "Some land-heavy turns could likely start with one workflow land helper.",
                "detail": f"Prefer `{primary['suggested_command']}` when one turn keeps hopping across patchset, attestation, review, policy, and land status for the same change.",
                "matched_commands": primary["matched_commands"],
                "turns": [
                    {
                        "turn_index": int(row["turn_index"]),
                        "suggested_command": str(row["suggested_command"]),
                        "matched_count": int(row["matched_count"]),
                    }
                    for row in workflow_land_turns[:5]
                ],
                "matched_count": sum(int(row["matched_count"]) for row in workflow_land_turns),
            }
        )
    if queue_summary_turns:
        primary = queue_summary_turns[0]
        hints.append(
            {
                "code": "queue_summary_for_inventory",
                "summary": "Workflow inventory can usually come from one queue view.",
                "detail": f"Prefer `{primary['suggested_command']}` when the goal is to understand what remains or what should land next.",
                "matched_commands": primary["matched_commands"],
                "matched_count": sum(int(row["matched_count"]) for row in queue_summary_turns),
                "turns": [
                    {
                        "turn_index": int(row["turn_index"]),
                        "suggested_command": str(row["suggested_command"]),
                    }
                    for row in queue_summary_turns[:5]
                ],
            }
        )
    repeated_inventory_runs = [run for run in repeated_runs if str(run.get("command_path") or "") in AIT_DUPLICATE_INVENTORY_COMMAND_PATHS]
    if repeated_inventory_runs:
        hints.append(
            {
                "code": "duplicate_inventory_reads",
                "summary": "The same workflow inventory command was rerun.",
                "detail": "Reuse the earlier queue or list output unless workflow state changed in between.",
                "runs": repeated_inventory_runs[:5],
            }
        )
    if repeated_runs:
        hints.append(
            {
                "code": "avoid_duplicate_commands",
                "summary": "The same ait command was issued repeatedly.",
                "detail": "Reuse the previous result or wait for state to change before repeating the same command.",
                "redundant_command_count": sum(max(int(run["count"]) - 1, 0) for run in repeated_runs),
                "runs": repeated_runs[:5],
            }
        )
    repeated_show_targets: list[dict[str, Any]] = []
    for signature, count in sorted(signature_counts.items(), key=lambda item: (-item[1], item[0])):
        command_path = _signature_command_path(signature)
        if count < 2 or command_path not in AIT_SHOW_COMMAND_PATHS:
            continue
        repeated_show_targets.append(
            {
                "signature": signature,
                "count": count,
                "example": signature_examples.get(signature, signature),
            }
        )
    if repeated_show_targets:
        hints.append(
            {
                "code": "reuse_loaded_object_context",
                "summary": "The same workflow object was opened multiple times.",
                "detail": "Keep the fetched task, change, or session detail in working context instead of calling `show` again.",
                "targets": repeated_show_targets[:5],
            }
        )
    heavy_turns = [
        {
            "turn_index": turn["turn_index"],
            "ait_command_count": turn["ait_command_count"],
            "command_paths": [row["command_path"] for row in turn["command_paths"][:4]],
        }
        for turn in turns
        if int(turn["ait_command_count"]) >= 4
    ]
    if heavy_turns:
        hints.append(
            {
                "code": "reduce_commands_per_turn",
                "summary": "Some conversation turns relied on many ait commands.",
                "detail": "Those turns are good candidates for a tighter plan-first prompt, batching, or one-command summary views before drilling down.",
                "turns": heavy_turns[:5],
            }
        )
    shell_helper_turns = [
        {
            "turn_index": turn.get("turn_index"),
            "assistant_reply_sequence": turn.get("assistant_reply_sequence"),
            "command_count": turn.get("command_count"),
            "hint_codes": [
                code
                for code in turn.get("hint_codes") or []
                if code in {"merge_inspection_commands", "consolidate_file_discovery", "reuse_file_read"}
            ],
            "summary": str(turn.get("optimization_summary") or ""),
        }
        for turn in codex_turns
        if any(code in {"merge_inspection_commands", "consolidate_file_discovery", "reuse_file_read"} for code in turn.get("hint_codes") or [])
    ]
    if len(shell_helper_turns) >= 2:
        hints.append(
            {
                "code": "promote_repeated_shell_workflow",
                "summary": "Repeated Codex shell-heavy turns may deserve an `ait` helper.",
                "detail": "When the same raw shell inspection pattern keeps recurring across Codex turns, promote it into a small Python helper or a dedicated `ait` command instead of replaying the probes manually.",
                "turns": shell_helper_turns[:5],
            }
        )
    return hints


def _aggregate_turn_clusters(clusters: list[dict[str, Any]]) -> list[dict[str, Any]]:
    aggregated: dict[str, dict[str, Any]] = {}
    for cluster in clusters:
        code = str(cluster.get("code") or "").strip()
        if not code:
            continue
        row = aggregated.setdefault(
            code,
            {
                "code": code,
                "summary": str(cluster.get("summary") or "").strip(),
                "detail": str(cluster.get("detail") or "").strip(),
                "turn_count": 0,
                "matched_count": 0,
                "suggested_commands": [],
                "turns": [],
            },
        )
        row["turn_count"] += 1
        row["matched_count"] += _int_or_zero(cluster.get("matched_count"))
        suggested_command = str(cluster.get("suggested_command") or "").strip()
        if suggested_command and suggested_command not in row["suggested_commands"]:
            row["suggested_commands"].append(suggested_command)
        turn_row = {
            "turn_index": _int_or_zero(cluster.get("turn_index")),
            "matched_count": _int_or_zero(cluster.get("matched_count")),
        }
        if suggested_command:
            turn_row["suggested_command"] = suggested_command
        if len(row["turns"]) < 5:
            row["turns"].append(turn_row)
    return [
        {
            "code": str(row.get("code") or ""),
            "summary": str(row.get("summary") or ""),
            "detail": str(row.get("detail") or ""),
            "turn_count": _int_or_zero(row.get("turn_count")),
            "matched_count": _int_or_zero(row.get("matched_count")),
            "suggested_command": (row.get("suggested_commands") or [None])[0],
            "suggested_commands": list(row.get("suggested_commands") or [])[:3],
            "turns": list(row.get("turns") or []),
        }
        for row in sorted(aggregated.values(), key=lambda item: (-_int_or_zero(item.get("turn_count")), str(item.get("code") or "")))
    ]


def _session_burst_clusters(turns: list[dict[str, Any]], commands: list[dict[str, Any]]) -> list[dict[str, Any]]:
    clusters: list[dict[str, Any]] = []
    for turn in turns:
        turn_index = int(turn.get("turn_index") or 0)
        turn_commands = _commands_for_turn(turn, commands)
        task_audit_merge = _task_audit_turn_merge(turn_commands, turn_index=turn_index)
        inventory_cluster = _inventory_burst_turn(turn_commands, turn_index=turn_index, task_audit_merge=task_audit_merge)
        if inventory_cluster is not None:
            clusters.append(inventory_cluster)
        help_cluster = _help_burst_turn(turn_commands, turn_index=turn_index)
        if help_cluster is not None:
            clusters.append(help_cluster)
        land_cluster = _land_workflow_turn(turn_commands, turn_index=turn_index)
        if land_cluster is not None:
            clusters.append(land_cluster)
    return _aggregate_turn_clusters(clusters)


def _analyze_session_ait_usage(session_id: str, events: list[dict[str, Any]], *, after_sequence: int, limit: int) -> dict[str, Any]:
    commands = _extract_session_ait_commands(events)
    path_counts: Counter[str] = Counter(command["command_path"] for command in commands)
    top_level_counts: Counter[str] = Counter(command["top_level"] for command in commands)
    signature_counts: Counter[str] = Counter(command["signature"] for command in commands)
    path_examples: dict[str, str] = {}
    signature_examples: dict[str, str] = {}
    for command in commands:
        path_examples.setdefault(str(command["command_path"]), str(command["raw_command"]))
        signature_examples.setdefault(str(command["signature"]), str(command["raw_command"]))
    repeated_runs = _detect_repeated_command_runs(commands)
    turns, unscoped_commands = _build_session_turns(events, commands)
    codex_turns = _build_codex_turns(events, turns)
    codex_optimization_hints = _session_codex_optimization_hints(codex_turns)
    optimization_hints = _session_optimization_hints(commands, path_counts, signature_counts, signature_examples, repeated_runs, turns, codex_turns)
    burst_clusters = _session_burst_clusters(turns, commands)
    return {
        "session_id": session_id,
        "analysis_window": {
            "after_sequence": max(int(after_sequence), 0),
            "limit": max(int(limit), 1),
            "event_count": len(events),
        },
        "event_count": len(events),
        "ait_command_count": len(commands),
        "distinct_command_paths": len(path_counts),
        "distinct_command_signatures": len(signature_counts),
        "unscoped_ait_command_count": len(unscoped_commands),
        "top_level_commands": [
            {"top_level": top_level, "count": count}
            for top_level, count in sorted(top_level_counts.items(), key=lambda item: (-item[1], item[0]))
        ],
        "capture_modes": [
            {"capture_mode": capture_mode, "count": count}
            for capture_mode, count in sorted(Counter(str(command.get("capture_mode") or "manual") for command in commands).items(), key=lambda item: (-item[1], item[0]))
        ],
        "command_paths": [
            {
                "command_path": command_path,
                "count": count,
                "example": path_examples.get(command_path),
            }
            for command_path, count in sorted(path_counts.items(), key=lambda item: (-item[1], item[0]))
        ],
        "repeated_command_runs": repeated_runs,
        "conversation_turns": turns,
        "codex_turn_count": len(codex_turns),
        "codex_command_count": sum(_int_or_zero(turn.get("command_count")) for turn in codex_turns),
        "codex_turns": codex_turns,
        "burst_clusters": burst_clusters,
        "merge_opportunities": _session_merge_opportunities(
            commands,
            path_counts,
            signature_counts,
            path_examples,
            signature_examples,
            repeated_runs,
            turns,
        ),
        "codex_optimization_hints": codex_optimization_hints,
        "optimization_hints": optimization_hints,
    }
