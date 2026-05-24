from __future__ import annotations

import shlex
from collections import Counter
from typing import Any

from ait_protocol.common import DEFAULT_ID_NAMESPACE_PREFIX, workflow_id_matches_any_namespace_prefix

from ..store import RepoContext, effective_id_namespace_prefix as repo_id_namespace_prefix
from .workflow_guides import HELP_FLAGS

AIT_INVENTORY_COMMAND_PATHS = frozenset({"change list", "change show", "task list", "task show"})
AIT_DUPLICATE_INVENTORY_COMMAND_PATHS = frozenset({"change list", "queue summary", "task audit", "task list"})
AIT_LAND_WORKFLOW_TOP_LEVELS = frozenset({"attest", "land", "patchset", "policy", "review", "snapshot"})


def _ctx() -> RepoContext:
    return RepoContext.discover()


def _command_flag_value(tokens: list[str], flag: str) -> str | None:
    for idx, token in enumerate(tokens):
        if token != flag:
            continue
        if idx + 1 >= len(tokens):
            return None
        value = tokens[idx + 1]
        if value.startswith("-"):
            return None
        return value
    return None


def _looks_like_task_id(value: str | None) -> bool:
    return workflow_id_matches_any_namespace_prefix(
        value,
        "T",
        _analysis_id_namespace_prefix(),
        include_task_change_origins=True,
    )


def _looks_like_change_id(value: str | None) -> bool:
    return workflow_id_matches_any_namespace_prefix(
        value,
        "C",
        _analysis_id_namespace_prefix(),
        include_task_change_origins=True,
    )


def _looks_like_patchset_id(value: str | None) -> bool:
    return workflow_id_matches_any_namespace_prefix(
        value,
        "P",
        _analysis_id_namespace_prefix(),
        include_task_change_origins=True,
    )


def _analysis_id_namespace_prefix() -> str:
    try:
        return repo_id_namespace_prefix(_ctx())
    except Exception:
        return DEFAULT_ID_NAMESPACE_PREFIX


def _task_start_suggested_command(task_start_rows: list[dict[str, Any]], change_create_rows: list[dict[str, Any]]) -> str:
    matched_rows = [*task_start_rows, *change_create_rows]
    parts = ["ait", "task", "start"]
    if any("--local" in row["tokens"] for row in matched_rows):
        parts.append("--local")
    base_line = next((_command_flag_value(row["tokens"], "--base-line") for row in change_create_rows), None)
    if base_line and base_line != "main":
        parts.extend(["--base-line", base_line])
    return " ".join(shlex.quote(part) for part in parts)


def _task_start_turn_merge(turn_commands: list[dict[str, Any]], *, turn_index: int | None = None) -> dict[str, Any] | None:
    task_start_rows = [
        row
        for row in turn_commands
        if row["command_path"] == "task start" and "--task-only" in (row.get("tokens") or [])
    ]
    change_create_rows = [row for row in turn_commands if row["command_path"] == "change create"]
    if not task_start_rows or not change_create_rows:
        return None
    observed_examples = [row["raw_command"] for row in [*task_start_rows, *change_create_rows][:5]]
    opportunity = {
        "code": "task_start_bootstrap_merge",
        "summary": "This task bootstrap turn could have been answered with one `ait task start` command.",
        "observed_count": len(task_start_rows) + len(change_create_rows),
        "minimal_count": 1,
        "avoidable_count": max(len(task_start_rows) + len(change_create_rows) - 1, 0),
        "suggested_command": _task_start_suggested_command(task_start_rows, change_create_rows),
        "matched_command_paths": ["task start", "change create"],
        "observed_examples": observed_examples,
    }
    if turn_index is not None:
        opportunity["turn_index"] = turn_index
    return opportunity


def _task_audit_suggested_command(task_id: str) -> str:
    return " ".join(["ait", "task", "audit", task_id])


def _task_audit_turn_merge(turn_commands: list[dict[str, Any]], *, turn_index: int | None = None) -> dict[str, Any] | None:
    if any(row["command_path"] == "task audit" for row in turn_commands):
        return None
    candidate_targets: list[str] = []
    for row in turn_commands:
        task_id = str(row.get("target") or "").upper()
        if row["command_path"] not in {"task show", "change list"} or not _looks_like_task_id(task_id):
            continue
        if task_id not in candidate_targets:
            candidate_targets.append(task_id)
    for task_id in candidate_targets:
        task_show_rows = [row for row in turn_commands if row["command_path"] == "task show" and str(row.get("target") or "").upper() == task_id]
        change_list_rows = [row for row in turn_commands if row["command_path"] == "change list" and str(row.get("target") or "").upper() == task_id]
        matched_rows = [*task_show_rows, *change_list_rows]
        if not task_show_rows or not change_list_rows:
            continue
        if any("--local" in row["tokens"] for row in matched_rows):
            continue
        opportunity = {
            "code": "task_audit_read_merge",
            "summary": "This task-readiness turn could have been answered with one `ait task audit` command.",
            "task_id": task_id,
            "observed_count": len(matched_rows),
            "minimal_count": 1,
            "avoidable_count": max(len(matched_rows) - 1, 0),
            "suggested_command": _task_audit_suggested_command(task_id),
            "matched_command_paths": ["task show", "change list"],
            "observed_examples": [row["raw_command"] for row in matched_rows[:5]],
        }
        if turn_index is not None:
            opportunity["turn_index"] = turn_index
        return opportunity
    return None


def _workflow_land_target_ids(turn_commands: list[dict[str, Any]]) -> tuple[list[str], list[str]]:
    change_ids: list[str] = []
    patchset_ids: list[str] = []
    for row in turn_commands:
        tokens = list(row.get("tokens") or [])
        candidates = [
            str(row.get("target") or "").strip().upper(),
            str(_command_flag_value(tokens, "--change") or "").strip().upper(),
            str(_command_flag_value(tokens, "--patchset") or "").strip().upper(),
        ]
        for candidate in candidates:
            if _looks_like_change_id(candidate) and candidate not in change_ids:
                change_ids.append(candidate)
            if _looks_like_patchset_id(candidate) and candidate not in patchset_ids:
                patchset_ids.append(candidate)
    return change_ids, patchset_ids


def _workflow_land_suggested_command(change_id: str | None = None, patchset_id: str | None = None) -> str:
    if _looks_like_change_id(change_id):
        return " ".join(["ait", "workflow", "land", str(change_id).upper()])
    if _looks_like_patchset_id(patchset_id):
        return "ait workflow land <change-id>"
    token = "C" if _analysis_id_namespace_prefix() == "" else f"{_analysis_id_namespace_prefix()}C"
    return f"ait workflow land {token}-..."


def _land_workflow_turn(turn_commands: list[dict[str, Any]], *, turn_index: int | None = None) -> dict[str, Any] | None:
    if any(row["command_path"] == "workflow land" for row in turn_commands):
        return None
    matched_rows = [
        row
        for row in turn_commands
        if row.get("top_level") in AIT_LAND_WORKFLOW_TOP_LEVELS and not any(token in HELP_FLAGS for token in row.get("tokens") or [])
    ]
    distinct_top_levels = sorted({str(row.get("top_level") or "").strip() for row in matched_rows if str(row.get("top_level") or "").strip()})
    if len(distinct_top_levels) < 3:
        return None
    change_ids, patchset_ids = _workflow_land_target_ids(matched_rows)
    cluster = {
        "code": "land_workflow_burst",
        "summary": "This turn crossed several land-workflow steps.",
        "detail": "A single `ait workflow land` view can summarize the next gate instead of rediscovering patchset, attestation, review, policy, and land status one command at a time.",
        "matched_commands": [row["raw_command"] for row in matched_rows[:5]],
        "matched_count": len(matched_rows),
        "top_levels": distinct_top_levels,
        "suggested_command": _workflow_land_suggested_command(change_id=change_ids[0] if len(change_ids) == 1 else None, patchset_id=patchset_ids[0] if len(patchset_ids) == 1 else None),
    }
    if len(change_ids) == 1:
        cluster["change_id"] = change_ids[0]
    if len(patchset_ids) == 1:
        cluster["patchset_id"] = patchset_ids[0]
    if turn_index is not None:
        cluster["turn_index"] = turn_index
    return cluster


def _inventory_burst_turn(
    turn_commands: list[dict[str, Any]],
    *,
    turn_index: int | None = None,
    task_audit_merge: dict[str, Any] | None = None,
) -> dict[str, Any] | None:
    turn_counts = Counter(command["command_path"] for command in turn_commands)
    inventory_paths = [path for path in sorted(AIT_INVENTORY_COMMAND_PATHS) if turn_counts.get(path, 0)]
    repeated_signatures = Counter(
        row["signature"]
        for row in turn_commands
        if row["command_path"] in AIT_DUPLICATE_INVENTORY_COMMAND_PATHS
    )
    repeated_signature = next((signature for signature, count in repeated_signatures.items() if count > 1), None)

    summary = "This turn revisited workflow inventory several times."
    detail = "Prefer one inventory summary first, then drill down only where it points."
    suggested_command: str | None = None
    matched_rows: list[dict[str, Any]] = []

    if task_audit_merge is not None:
        suggested_command = str(task_audit_merge["suggested_command"])
        matched_rows = [
            row
            for row in turn_commands
            if row["command_path"] in {"task show", "change list"}
            and str(row.get("target") or "").upper() == str(task_audit_merge.get("task_id") or "").upper()
        ]
        summary = "This turn rebuilt one task's readiness from several inventory reads."
        detail = "Use one `ait task audit` view instead of separate task and change reads when the goal is one task's readiness."
    elif (
        len(inventory_paths) >= 2
        and turn_counts.get("queue summary", 0) == 0
        and not (task_audit_merge is not None and not turn_counts.get("task list", 0))
    ):
        suggested_command = "ait queue summary --all-changes" if turn_counts.get("change list", 0) else "ait queue summary"
        matched_rows = [row for row in turn_commands if row["command_path"] in inventory_paths]
        summary = "This turn stitched workflow inventory together from several commands."
        detail = "Start with one queue summary view, then open a task or change only if the queue shows a real gap."
    elif repeated_signature:
        suggested_command = next((row["raw_command"] for row in turn_commands if row["signature"] == repeated_signature), None)
        matched_rows = [row for row in turn_commands if row["signature"] == repeated_signature]
        summary = "This turn reran the same workflow inventory command."
        detail = "Reuse the earlier queue or list output unless workflow state changed in between."
    else:
        return None

    cluster = {
        "code": "inventory_burst",
        "summary": summary,
        "detail": detail,
        "matched_commands": [row["raw_command"] for row in matched_rows[:5]],
        "matched_count": len(matched_rows),
        "suggested_command": suggested_command,
    }
    if turn_index is not None:
        cluster["turn_index"] = turn_index
    return cluster
