from __future__ import annotations

import importlib

from ait.cli import session_views

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_session_view_helpers() -> None:
    helper_names = [
        "_render_sessions",
        "_render_session_events",
        "_render_checkpoints",
        "_render_session_analysis",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(session_views, name)


def test_session_view_cell_helpers_preserve_preview_contract() -> None:
    assert session_views._command_summary_cells([]) == "none"
    assert session_views._top_command_cells([]) == "none"
    assert session_views._command_summary_cells(
        [
            {"command_path": "task show", "count": 2},
            {"command_path": "change list", "count": 1},
        ]
    ) == "task show (2), change list (1)"
    assert session_views._top_command_cells(
        [
            {"command": "pwd", "count": 1},
            {"command": "ls docs", "count": 2},
        ]
    ) == "pwd (1), ls docs (2)"


def test_render_session_listing_and_events_smoke(capsys) -> None:
    session_views._render_sessions(
        [
            {
                "session_id": "S-123",
                "session_kind": "agent_run",
                "title": "CLI extraction",
                "status": "active",
                "line_name": "feature/rt-1149",
                "last_event_sequence": 4,
                "head_checkpoint_id": "K-123",
            }
        ]
    )
    session_views._render_session_events(
        "S-123",
        [
            {
                "sequence": 4,
                "event_type": "assistant.reply",
                "actor_identity": "codex",
                "created_at": "2026-05-17T13:00:00Z",
            }
        ],
    )
    session_views._render_checkpoints(
        "S-123",
        [
            {
                "checkpoint_id": "K-123",
                "based_on_sequence": 4,
                "snapshot_id": "SNP-123",
                "created_at": "2026-05-17T13:01:00Z",
                "summary": "Persist current context",
            }
        ],
    )
    captured = capsys.readouterr().out
    assert "ait sessions" in captured
    assert "session events for S-123" in captured
    assert "checkpoints for S-123" in captured
    assert "active" in captured
    assert "K-123" in captured
    assert "Persist" in captured
    assert "context" in captured


def test_render_session_analysis_smoke(capsys) -> None:
    session_views._render_session_analysis(
        "S-123",
        {
            "event_count": 6,
            "ait_command_count": 4,
            "codex_command_count": 9,
            "distinct_command_paths": 3,
            "conversation_turns": [
                {
                    "turn_index": 1,
                    "start_sequence": 1,
                    "end_sequence": 3,
                    "ait_command_count": 4,
                    "codex_command_count": 5,
                    "command_paths": [
                        {"command_path": "task show", "count": 2},
                        {"command_path": "change list", "count": 1},
                    ],
                    "codex_optimization_summary": "Several read-only shell probes could have been merged into one command.",
                }
            ],
            "codex_turn_count": 2,
            "unscoped_ait_command_count": 0,
            "command_paths": [{"command_path": "task show", "count": 2, "example": "ait task show T-1 --json"}],
            "capture_modes": [{"capture_mode": "manual", "count": 4}],
            "codex_turns": [
                {
                    "turn_index": 1,
                    "assistant_reply_sequence": 3,
                    "command_count": 5,
                    "top_commands": [{"command": "pwd", "count": 1}],
                    "optimization_summary": "Several read-only shell probes could have been merged into one command.",
                }
            ],
            "merge_opportunities": [
                {
                    "code": "queue_summary_inventory_merge",
                    "observed_count": 4,
                    "suggested_command": "ait queue summary --all-changes",
                    "summary": "This workflow inventory turn could have been answered with one queue summary command.",
                }
            ],
            "burst_clusters": [
                {
                    "code": "queue_summary_inventory_merge",
                    "turn_count": 1,
                    "suggested_command": "ait queue summary --all-changes",
                    "summary": "Inventory burst",
                }
            ],
            "optimization_hints": [
                {
                    "code": "avoid_duplicate_commands",
                    "summary": "The same ait command was issued repeatedly.",
                    "detail": "Reuse the previous result or wait for state to change before repeating the same command.",
                }
            ],
            "codex_optimization_hints": [
                {
                    "code": "merge_inspection_commands",
                    "turn_count": 2,
                    "suggested_command": "pwd && ls docs",
                    "summary": "Several read-only shell probes could have been merged into one command.",
                }
            ],
        },
    )
    captured = capsys.readouterr().out
    assert "session analysis for S-123" in captured
    assert "ait command paths for S-123" in captured
    assert "capture modes for S-123" in captured
    assert "conversation turns for S-123" in captured
    assert "codex turn analysis for S-123" in captured
    assert "merge opportunities for S-123" in captured
    assert "burst clusters for S-123" in captured
    assert "optimization hints for S-123" in captured
    assert "codex optimization hints for S-123" in captured
    assert "task show (2)" in captured
    assert "pwd (1)" in captured
    assert "ait queue summary" in captured
    assert "all-changes" in captured
