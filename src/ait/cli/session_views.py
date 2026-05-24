from __future__ import annotations

from typing import Any

import typer
from rich import print as rprint
from rich.table import Table


def _command_summary_cells(command_rows: list[dict[str, Any]], *, limit: int = 3) -> str:
    if not command_rows:
        return "none"
    preview = [f"{row['command_path']} ({row['count']})" for row in command_rows[:limit]]
    if len(command_rows) > limit:
        preview.append("...")
    return ", ".join(preview)


def _top_command_cells(command_rows: list[dict[str, Any]], *, limit: int = 3) -> str:
    if not command_rows:
        return "none"
    preview = [
        f"{str(row.get('command') or '')} ({int(row.get('count') or 0)})"
        for row in command_rows[:limit]
        if str(row.get("command") or "").strip()
    ]
    if len(command_rows) > limit:
        preview.append("...")
    return ", ".join(preview) if preview else "none"


def _render_sessions(rows: list[dict]) -> None:
    table = Table(title="ait sessions")
    table.add_column("session_id")
    table.add_column("kind")
    table.add_column("title")
    table.add_column("status")
    table.add_column("line")
    table.add_column("events")
    table.add_column("checkpoint")
    for row in rows:
        table.add_row(
            str(row.get("session_id") or ""),
            str(row.get("session_kind") or ""),
            str(row.get("title") or ""),
            str(row.get("status") or ""),
            str(row.get("line_name") or ""),
            str(row.get("last_event_sequence") or 0),
            str(row.get("head_checkpoint_id") or ""),
        )
    rprint(table)


def _render_session_events(session_id: str, rows: list[dict]) -> None:
    table = Table(title=f"session events for {session_id}")
    table.add_column("sequence")
    table.add_column("type")
    table.add_column("actor")
    table.add_column("created_at")
    for row in rows:
        table.add_row(
            str(row.get("sequence") or ""),
            str(row.get("event_type") or ""),
            str(row.get("actor_identity") or ""),
            str(row.get("created_at") or ""),
        )
    rprint(table)


def _render_checkpoints(session_id: str, rows: list[dict]) -> None:
    table = Table(title=f"checkpoints for {session_id}")
    table.add_column("checkpoint_id")
    table.add_column("sequence")
    table.add_column("snapshot")
    table.add_column("created_at")
    table.add_column("summary")
    for row in rows:
        table.add_row(
            str(row.get("checkpoint_id") or ""),
            str(row.get("based_on_sequence") or ""),
            str(row.get("snapshot_id") or ""),
            str(row.get("created_at") or ""),
            str(row.get("summary") or ""),
        )
    rprint(table)


def _render_session_analysis(session_id: str, analysis: dict[str, Any]) -> None:
    summary = Table(title=f"session analysis for {session_id}")
    summary.add_column("metric")
    summary.add_column("value")
    summary.add_row("events", str(analysis.get("event_count") or 0))
    summary.add_row("ait commands", str(analysis.get("ait_command_count") or 0))
    summary.add_row("codex commands", str(analysis.get("codex_command_count") or 0))
    summary.add_row("distinct command paths", str(analysis.get("distinct_command_paths") or 0))
    summary.add_row("conversation turns", str(len(analysis.get("conversation_turns") or [])))
    summary.add_row("codex turns", str(analysis.get("codex_turn_count") or 0))
    summary.add_row("unscoped ait commands", str(analysis.get("unscoped_ait_command_count") or 0))
    summary.add_row("merge opportunities", str(len(analysis.get("merge_opportunities") or [])))
    rprint(summary)

    command_paths = analysis.get("command_paths") or []
    if command_paths:
        command_table = Table(title=f"ait command paths for {session_id}")
        command_table.add_column("command_path")
        command_table.add_column("count")
        command_table.add_column("example")
        for row in command_paths:
            command_table.add_row(
                str(row.get("command_path") or ""),
                str(row.get("count") or 0),
                str(row.get("example") or ""),
            )
        rprint(command_table)
    else:
        typer.echo("No structured ait commands were found in the selected session event window.")

    capture_modes = analysis.get("capture_modes") or []
    if capture_modes:
        capture_table = Table(title=f"capture modes for {session_id}")
        capture_table.add_column("capture_mode")
        capture_table.add_column("count")
        for row in capture_modes:
            capture_table.add_row(str(row.get("capture_mode") or ""), str(row.get("count") or 0))
        rprint(capture_table)

    turns = analysis.get("conversation_turns") or []
    if turns:
        turns_table = Table(title=f"conversation turns for {session_id}")
        turns_table.add_column("turn")
        turns_table.add_column("seq")
        turns_table.add_column("ait commands")
        turns_table.add_column("codex commands")
        turns_table.add_column("top paths")
        turns_table.add_column("codex summary")
        for turn in turns:
            seq_range = f"{turn.get('start_sequence')}..{turn.get('end_sequence')}"
            turns_table.add_row(
                str(turn.get("turn_index") or ""),
                seq_range,
                str(turn.get("ait_command_count") or 0),
                str(turn.get("codex_command_count") or 0),
                _command_summary_cells(turn.get("command_paths") or []),
                str(turn.get("codex_optimization_summary") or ""),
            )
        rprint(turns_table)

    codex_turns = analysis.get("codex_turns") or []
    if codex_turns:
        codex_turns_table = Table(title=f"codex turn analysis for {session_id}")
        codex_turns_table.add_column("turn")
        codex_turns_table.add_column("assistant seq")
        codex_turns_table.add_column("commands")
        codex_turns_table.add_column("top commands")
        codex_turns_table.add_column("summary")
        for turn in codex_turns:
            codex_turns_table.add_row(
                str(turn.get("turn_index") or ""),
                str(turn.get("assistant_reply_sequence") or ""),
                str(turn.get("command_count") or 0),
                _top_command_cells(turn.get("top_commands") or []),
                str(turn.get("optimization_summary") or ""),
            )
        rprint(codex_turns_table)

    merge_opportunities = analysis.get("merge_opportunities") or []
    if merge_opportunities:
        merge_table = Table(title=f"merge opportunities for {session_id}")
        merge_table.add_column("code")
        merge_table.add_column("observed")
        merge_table.add_column("suggested")
        merge_table.add_column("summary")
        for row in merge_opportunities:
            merge_table.add_row(
                str(row.get("code") or ""),
                str(row.get("observed_count") or 0),
                str(row.get("suggested_command") or ""),
                str(row.get("summary") or ""),
            )
        rprint(merge_table)

    burst_clusters = analysis.get("burst_clusters") or []
    if burst_clusters:
        cluster_table = Table(title=f"burst clusters for {session_id}")
        cluster_table.add_column("code")
        cluster_table.add_column("turns")
        cluster_table.add_column("suggested")
        cluster_table.add_column("summary")
        for row in burst_clusters:
            cluster_table.add_row(
                str(row.get("code") or ""),
                str(row.get("turn_count") or 0),
                str(row.get("suggested_command") or ""),
                str(row.get("summary") or ""),
            )
        rprint(cluster_table)

    hints = analysis.get("optimization_hints") or []
    if hints:
        hints_table = Table(title=f"optimization hints for {session_id}")
        hints_table.add_column("code")
        hints_table.add_column("summary")
        hints_table.add_column("detail")
        for hint in hints:
            hints_table.add_row(
                str(hint.get("code") or ""),
                str(hint.get("summary") or ""),
                str(hint.get("detail") or ""),
            )
        rprint(hints_table)

    codex_hints = analysis.get("codex_optimization_hints") or []
    if codex_hints:
        codex_hints_table = Table(title=f"codex optimization hints for {session_id}")
        codex_hints_table.add_column("code")
        codex_hints_table.add_column("turns")
        codex_hints_table.add_column("suggested")
        codex_hints_table.add_column("summary")
        for hint in codex_hints:
            codex_hints_table.add_row(
                str(hint.get("code") or ""),
                str(hint.get("turn_count") or 0),
                str(hint.get("suggested_command") or ""),
                str(hint.get("summary") or ""),
            )
        rprint(codex_hints_table)
