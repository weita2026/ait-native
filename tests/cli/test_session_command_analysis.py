from __future__ import annotations

import importlib

from ait.cli import session_command_analysis

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_session_command_analysis_helpers() -> None:
    helper_names = [
        "AIT_TASK_CLOSE_COMMAND_PATHS",
        "AIT_TOP_LEVEL_COMMANDS",
        "_analyze_session_ait_usage",
        "_extract_ait_command",
        "_normalize_turn_analysis_payload",
        "_session_command_phase",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(session_command_analysis, name)


def test_session_command_analysis_parser_and_payload_contract() -> None:
    parsed = session_command_analysis._extract_ait_command(
        "PYTHONPATH=src:. .venv/bin/python -m ait.cli queue summary --all-changes --json"
    )
    assert parsed is not None
    assert parsed["top_level"] == "queue"
    assert parsed["command_path"] == "queue summary"
    assert parsed["target"] is None
    assert parsed["signature"] == "ait queue summary"

    wrapped = session_command_analysis._extract_ait_command(
        "printf ready; .venv/bin/ait patchset publish --change AITC-LOCAL-1 --summary 'review summary'"
    )
    assert wrapped is not None
    assert wrapped["command_path"] == "patchset publish"
    assert wrapped["target"] == "AITC-LOCAL-1"
    assert wrapped["signature"] == "ait patchset publish AITC-LOCAL-1"

    implicit = session_command_analysis._extract_ait_command("policy eval")
    assert implicit is not None
    assert implicit["command_path"] == "policy eval"
    assert implicit["signature"] == "ait policy eval"

    normalized = session_command_analysis._normalize_turn_analysis_payload(
        {
            "command_count": "3",
            "distinct_command_count": "2",
            "commands": ["pwd", "ls docs"],
            "top_commands": [{"command": "pwd", "count": 1}],
            "optimization_hints": [{"code": "merge_inspection_commands", "summary": "merge", "detail": "batch"}],
            "optimization_summary": "Several read-only shell probes could have been merged into one command.",
        }
    )
    assert normalized is not None
    assert normalized["command_count"] == 3
    assert normalized["distinct_command_count"] == 2
    assert normalized["commands"] == ["pwd", "ls docs"]
    assert session_command_analysis._session_command_phase({"command_phase": "FINISHED"}) == "finished"



def test_session_command_analysis_turn_aggregation_and_analysis_contract() -> None:
    events = [
        {"sequence": 1, "event_type": "session.message", "payload": {"text": "Did this task already land?"}},
        {"sequence": 2, "event_type": "tool.result", "payload": {"command": "ait task show AITT-LOCAL-1 --json"}},
        {"sequence": 3, "event_type": "tool.result", "payload": {"command": "ait change list --task AITT-LOCAL-1 --json"}},
        {
            "sequence": 4,
            "event_type": "assistant.reply",
            "payload": {
                "turn_analysis": {
                    "command_count": 4,
                    "distinct_command_count": 3,
                    "commands": ["pwd", "ls docs", "sed -n '1,20p' docs/ait.md"],
                    "top_commands": [{"command": "pwd", "count": 1}, {"command": "ls docs", "count": 1}],
                    "optimization_hints": [
                        {
                            "code": "merge_inspection_commands",
                            "summary": "Several read-only shell probes could have been merged into one command.",
                            "detail": "These adjacent read-only checks could likely be batched into one shell call.",
                            "matched_commands": ["pwd", "ls docs"],
                            "suggested_command": "pwd && ls docs",
                        }
                    ],
                    "optimization_summary": "Several read-only shell probes could have been merged into one command.",
                }
            },
        },
    ]

    commands = session_command_analysis._extract_session_ait_commands(events)
    assert [row["command_path"] for row in commands] == ["task show", "change list"]

    turns, unscoped = session_command_analysis._build_session_turns(events, commands)
    assert unscoped == []
    assert len(turns) == 1
    assert turns[0]["ait_command_count"] == 2
    assert turns[0]["codex_command_count"] == 4
    assert turns[0]["codex_hint_codes"] == ["merge_inspection_commands"]

    analysis = session_command_analysis._analyze_session_ait_usage("S-LOCAL-1", events, after_sequence=0, limit=10)
    assert analysis["ait_command_count"] == 2
    assert analysis["codex_turn_count"] == 1
    assert analysis["codex_command_count"] == 4
    merge = next(row for row in analysis["merge_opportunities"] if row["code"] == "task_audit_read_merge")
    assert merge["suggested_command"] == "ait task audit AITT-LOCAL-1"
    hint = next(row for row in analysis["optimization_hints"] if row["code"] == "prefer_task_audit")
    assert hint["turns"][0]["task_id"] == "AITT-LOCAL-1"
    codex_hint = next(row for row in analysis["codex_optimization_hints"] if row["code"] == "merge_inspection_commands")
    assert codex_hint["suggested_command"] == "pwd && ls docs"
