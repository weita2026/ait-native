from __future__ import annotations

import importlib

from typer.testing import CliRunner

from ait.cli import workflow_guides

cli_app_module = importlib.import_module("ait.cli.app")
runner = CliRunner()


def test_cli_app_reexports_extracted_workflow_guide_helpers() -> None:
    helper_names = [
        "HELP_FLAGS",
        "WORKFLOW_GUIDE_TOPICS",
        "_workflow_guide_payload",
        "_render_workflow_guide_text",
        "_workflow_guide_topic_for_help_rows",
        "_help_burst_turn",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(workflow_guides, name)


def test_workflow_guide_payload_and_render_text_preserve_contract() -> None:
    listing = workflow_guides._workflow_guide_payload()
    assert [row["topic"] for row in listing["topics"]] == ["inventory", "land"]
    assert "ait workflow guides" in workflow_guides._render_workflow_guide_text(listing)

    inventory = workflow_guides._workflow_guide_payload("inventory")
    assert inventory["topic"] == "inventory"
    assert inventory["commands"][0]["command"] == "ait queue summary --all-changes"
    rendered = workflow_guides._render_workflow_guide_text(inventory)
    assert "ait workflow guide · inventory" in rendered
    assert "Recommended commands" in rendered
    assert "Avoid" in rendered

    land = workflow_guides._workflow_guide_payload("land")
    assert land["topic"] == "land"
    assert [row["command"] for row in land["commands"]] == [
        "ait workflow land <change-id>",
        "ait workflow land <change-id> --apply",
        "ait task complete <task-id>",
    ]
    land_rendered = workflow_guides._render_workflow_guide_text(land)
    assert "ait patchset publish" not in land_rendered
    assert "ait policy eval" not in land_rendered
    assert "ait land submit" not in land_rendered


def test_workflow_guide_cli_text_path_renders_without_runtime_nameerror() -> None:
    listing = runner.invoke(cli_app_module.app, ["workflow", "guide"], catch_exceptions=False)
    assert listing.exit_code == 0, listing.stdout
    assert "ait workflow guides" in listing.stdout

    inventory = runner.invoke(cli_app_module.app, ["workflow", "guide", "inventory"], catch_exceptions=False)
    assert inventory.exit_code == 0, inventory.stdout
    assert "ait workflow guide · inventory" in inventory.stdout

    land = runner.invoke(cli_app_module.app, ["workflow", "guide", "land"], catch_exceptions=False)
    assert land.exit_code == 0, land.stdout
    assert "ait workflow guide · land" in land.stdout


def test_workflow_guide_topic_detection_and_help_burst_suggestions() -> None:
    land_help_rows = [
        {"top_level": "snapshot", "tokens": ["snapshot", "create", "--help"], "raw_command": "ait snapshot create --help"},
        {"top_level": "patchset", "tokens": ["patchset", "publish", "--help"], "raw_command": "ait patchset publish --help"},
        {"top_level": "land", "tokens": ["land", "submit", "--help"], "raw_command": "ait land submit --help"},
    ]
    assert workflow_guides._workflow_guide_topic_for_help_rows(land_help_rows) == "land"
    cluster = workflow_guides._help_burst_turn(land_help_rows, turn_index=3)
    assert cluster is not None
    assert cluster["suggested_command"] == "ait workflow guide land"
    assert cluster["workflow_topic"] == "land"
    assert cluster["turn_index"] == 3

    inventory_help_rows = [
        {"top_level": "queue", "tokens": ["queue", "summary", "--help"], "raw_command": "ait queue summary --help"},
        {"top_level": "task", "tokens": ["task", "audit", "--help"], "raw_command": "ait task audit --help"},
    ]
    assert workflow_guides._workflow_guide_topic_for_help_rows(inventory_help_rows) == "inventory"

    generic_help_rows = [
        {"top_level": "task", "tokens": ["task", "show", "--help"], "raw_command": "ait task show --help"},
        {"top_level": "task", "tokens": ["task", "start", "--help"], "raw_command": "ait task start --help"},
    ]
    generic_cluster = workflow_guides._help_burst_turn(generic_help_rows)
    assert generic_cluster is not None
    assert generic_cluster["suggested_command"] == "task --help"
    assert "broader help entry point" in generic_cluster["detail"] or "Start with `task --help`" in generic_cluster["detail"]
