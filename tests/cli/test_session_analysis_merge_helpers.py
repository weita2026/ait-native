from __future__ import annotations

import importlib
from pathlib import Path

from ait.cli import session_analysis_merge_helpers

from ._shared import *  # noqa: F401,F403

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_session_analysis_merge_helpers() -> None:
    helper_names = [
        "AIT_DUPLICATE_INVENTORY_COMMAND_PATHS",
        "AIT_INVENTORY_COMMAND_PATHS",
        "AIT_LAND_WORKFLOW_TOP_LEVELS",
        "_analysis_id_namespace_prefix",
        "_command_flag_value",
        "_inventory_burst_turn",
        "_land_workflow_turn",
        "_looks_like_change_id",
        "_looks_like_patchset_id",
        "_looks_like_task_id",
        "_task_audit_suggested_command",
        "_task_audit_turn_merge",
        "_task_start_suggested_command",
        "_task_start_turn_merge",
        "_workflow_land_suggested_command",
        "_workflow_land_target_ids",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(session_analysis_merge_helpers, name)


def test_session_analysis_merge_helper_contract_preserved(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-session-analysis-merge-helpers"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    task_start_rows = [
        {
            "command_path": "task start",
            "tokens": ["task", "start", "--task-only", "--title", "Bootstrap", "--intent", "Preserve review flow"],
            "raw_command": "ait task start --task-only --title Bootstrap --intent 'Preserve review flow'",
        },
        {
            "command_path": "change create",
            "tokens": ["change", "create", "--task", "AITT-LOCAL-1", "--title", "Bootstrap", "--base-line", "feature/bootstrap"],
            "raw_command": "ait change create --task AITT-LOCAL-1 --title Bootstrap --base-line feature/bootstrap",
        },
    ]
    task_start_merge = session_analysis_merge_helpers._task_start_turn_merge(task_start_rows, turn_index=2)
    assert task_start_merge is not None
    assert task_start_merge["suggested_command"] == "ait task start --base-line feature/bootstrap"
    assert task_start_merge["turn_index"] == 2

    task_audit_rows = [
        {
            "command_path": "task show",
            "target": "AITT-LOCAL-1",
            "tokens": ["task", "show", "AITT-LOCAL-1", "--json"],
            "raw_command": "ait task show AITT-LOCAL-1 --json",
        },
        {
            "command_path": "change list",
            "target": "AITT-LOCAL-1",
            "tokens": ["change", "list", "--task", "AITT-LOCAL-1", "--json"],
            "raw_command": "ait change list --task AITT-LOCAL-1 --json",
        },
    ]
    task_audit_merge = session_analysis_merge_helpers._task_audit_turn_merge(task_audit_rows, turn_index=3)
    assert task_audit_merge is not None
    assert task_audit_merge["task_id"] == "AITT-LOCAL-1"
    assert task_audit_merge["suggested_command"] == "ait task audit AITT-LOCAL-1"

    repeated_queue_rows = [
        {
            "command_path": "queue summary",
            "signature": "ait queue summary",
            "raw_command": "PYTHONPATH=src:. .venv/bin/ait queue summary --all-changes --json",
            "tokens": ["queue", "summary", "--all-changes", "--json"],
        },
        {
            "command_path": "queue summary",
            "signature": "ait queue summary",
            "raw_command": "env PYTHONPATH=src:. timeout 15 .venv/bin/ait queue summary --all-changes --json",
            "tokens": ["queue", "summary", "--all-changes", "--json"],
        },
    ]
    repeated_inventory_cluster = session_analysis_merge_helpers._inventory_burst_turn(repeated_queue_rows, turn_index=4)
    assert repeated_inventory_cluster is not None
    assert repeated_inventory_cluster["summary"] == "This turn reran the same workflow inventory command."
    assert repeated_inventory_cluster["suggested_command"] == "PYTHONPATH=src:. .venv/bin/ait queue summary --all-changes --json"

    land_rows = [
        {
            "command_path": "patchset publish",
            "top_level": "patchset",
            "target": "AITC-LOCAL-1",
            "tokens": ["patchset", "publish", "--change", "AITC-LOCAL-1", "--summary", "review summary"],
            "raw_command": "ait patchset publish --change AITC-LOCAL-1 --summary 'review summary'",
        },
        {
            "command_path": "attest put",
            "top_level": "attest",
            "target": "AITP-LOCAL-1",
            "tokens": ["attest", "put", "AITP-LOCAL-1", "--tests", "pass"],
            "raw_command": "ait attest put AITP-LOCAL-1 --tests pass",
        },
        {
            "command_path": "review task approve",
            "top_level": "review",
            "target": "AITC-LOCAL-1",
            "tokens": ["review", "task", "approve", "AITC-LOCAL-1", "--patchset", "AITP-LOCAL-1"],
            "raw_command": "ait review task approve AITC-LOCAL-1 --patchset AITP-LOCAL-1",
        },
        {
            "command_path": "policy eval",
            "top_level": "policy",
            "target": "AITP-LOCAL-1",
            "tokens": ["policy", "eval", "AITP-LOCAL-1"],
            "raw_command": "ait policy eval AITP-LOCAL-1",
        },
        {
            "command_path": "land submit",
            "top_level": "land",
            "target": "AITC-LOCAL-1",
            "tokens": ["land", "submit", "AITC-LOCAL-1", "--patchset", "AITP-LOCAL-1", "--target", "main", "--mode", "direct"],
            "raw_command": "ait land submit AITC-LOCAL-1 --patchset AITP-LOCAL-1 --target main --mode direct",
        },
    ]
    assert session_analysis_merge_helpers._workflow_land_target_ids(land_rows) == (["AITC-LOCAL-1"], ["AITP-LOCAL-1"])
    land_cluster = session_analysis_merge_helpers._land_workflow_turn(land_rows, turn_index=5)
    assert land_cluster is not None
    assert land_cluster["suggested_command"] == "ait workflow land AITC-LOCAL-1"
    assert land_cluster["turn_index"] == 5
