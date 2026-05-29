from __future__ import annotations

import re

import pytest

from ._shared import *  # noqa: F401,F403


def _help_inventory(*argv: str) -> tuple[list[str], list[str]]:
    help_out = runner.invoke(app, [*argv, "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout

    options: list[str] = []
    commands: list[str] = []
    for raw_line in help_out.stdout.splitlines():
        line = raw_line.rstrip()
        option_match = re.match(r"│\s*(?:\*\s+)?(--[a-z0-9-]+)\b", line)
        if option_match:
            options.append(option_match.group(1))
            continue
        command_match = re.match(r"│\s*([a-z][a-z0-9-]*)\s{2,}", line)
        if command_match:
            commands.append(command_match.group(1))
    return options, commands


def _normalized_help_output(*argv: str) -> str:
    help_out = runner.invoke(app, [*argv, "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    sanitized = re.sub(r"[│╭╰─]+", " ", help_out.stdout)
    return " ".join(sanitized.split())


@pytest.mark.parametrize(
    ("argv", "expected_options", "expected_commands"),
    [
        pytest.param(("plan",), ["--help"], [
            "list",
            "show",
            "revisions",
            "items",
            "candidates",
            "inspect",
            "sync",
            "graph",
            "ready",
            "schedule",
            "progress",
            "execute",
            "session",
        ], id="plan"),
        pytest.param(
            ("task",),
            ["--help"],
            ["start", "list", "show", "tokens", "audit", "backfill-sessions", "canceled", "restart", "complete", "publish"],
            id="task",
        ),
        pytest.param(("change",), ["--help"], ["create", "list", "show", "revert", "replay", "close", "publish"], id="change"),
        pytest.param(("workflow",), ["--help"], ["guide", "land-local", "land"], id="workflow"),
        pytest.param(("patchset",), ["--help"], ["list", "show", "select", "rerun-ci", "ci-status"], id="patchset"),
        pytest.param(("attest",), ["--help"], ["show"], id="attest"),
        pytest.param(("review",), ["--help"], ["show", "code", "task", "team"], id="review"),
        pytest.param(("review", "code"), ["--help"], ["submit", "template"], id="review-code"),
        pytest.param(("review", "task"), ["--help"], ["approve", "request-changes", "comment", "defer"], id="review-task"),
        pytest.param(("review", "team"), ["--help"], ["request", "approve", "request-changes", "comment", "defer"], id="review-team"),
        pytest.param(("policy",), ["--help"], ["show", "waive"], id="policy"),
        pytest.param(("land",), ["--help"], ["show", "retry"], id="land"),
    ],
)
def test_help_group_command_inventory_is_stable(
    argv: tuple[str, ...], expected_options: list[str], expected_commands: list[str]
):
    options, commands = _help_inventory(*argv)
    assert options == expected_options
    assert commands == expected_commands


@pytest.mark.parametrize(
    ("argv", "expected_options"),
    [
        pytest.param(("plan", "show"), ["--revision", "--local", "--remote", "--json", "--help"], id="plan-show"),
        pytest.param(("plan", "items"), ["--revision", "--local", "--remote", "--json", "--help"], id="plan-items"),
        pytest.param(("plan", "candidates"), ["--local", "--remote", "--all", "--json", "--help"], id="plan-candidates"),
        pytest.param(("plan", "inspect"), ["--revision", "--local", "--remote", "--json", "--help"], id="plan-inspect"),
        pytest.param(
            ("plan", "sync"),
            [
                "--plan-ref",
                "--prune",
                "--local",
                "--remote",
                "--source-session",
                "--rebase",
                "--reconcile",
                "--json",
                "--help",
            ],
            id="plan-sync",
        ),
        pytest.param(("task", "start"), ["--title", "--intent", "--task-only", "--change-title", "--base-line", "--risk", "--plan", "--revision", "--plan-item-ref", "--local", "--remote", "--json", "--help"], id="task-start"),
        pytest.param(("task", "tokens"), ["--local", "--remote", "--repo", "--by", "--json", "--help"], id="task-tokens"),
        pytest.param(("task", "audit"), ["--target-line", "--remote", "--json", "--help"], id="task-audit"),
        pytest.param(("task", "backfill-sessions"), ["--task", "--remote", "--json", "--help"], id="task-backfill-sessions"),
        pytest.param(("task", "restart"), ["--local", "--remote", "--json", "--help"], id="task-restart"),
        pytest.param(("task", "complete"), ["--local", "--remote", "--json", "--help"], id="task-complete"),
        pytest.param(("task", "publish"), ["--remote", "--json", "--help"], id="task-publish"),
        pytest.param(("change", "create"), ["--task", "--title", "--base-line", "--risk", "--local", "--remote", "--json", "--help"], id="change-create"),
        pytest.param(("change", "revert"), ["--force", "--dry-run", "--local", "--remote", "--repo", "--json", "--help"], id="change-revert"),
        pytest.param(("change", "replay"), ["--onto", "--force", "--dry-run", "--local", "--remote", "--repo", "--json", "--help"], id="change-replay"),
        pytest.param(("change", "publish"), ["--remote", "--json", "--help"], id="change-publish"),
        pytest.param(("queue", "summary"), ["--remote", "--status", "--all-changes", "--json", "--help"], id="queue-summary"),
        pytest.param(("workflow", "land"), ["--all-completed-local", "--graph-run-session", "--apply", "--snapshot-message", "--summary", "--tests", "--lint", "--security", "--license", "--author-mode", "--model", "--session", "--checkpoint", "--reviewer", "--review-message", "--target", "--mode", "--remote", "--json", "--help"], id="workflow-land"),
        pytest.param(("workflow", "land-local"), ["--target", "--snapshot", "--json", "--help"], id="workflow-land-local"),
        pytest.param(("patchset", "publish"), ["--change", "--summary", "--author-mode", "--allow-empty", "--remote", "--json", "--help"], id="patchset-publish"),
        pytest.param(("patchset", "show"), ["--remote", "--repo", "--change", "--json", "--help"], id="patchset-show"),
        pytest.param(("patchset", "list"), ["--change", "--remote", "--repo", "--json", "--help"], id="patchset-list"),
        pytest.param(("patchset", "select"), ["--change", "--remote", "--json", "--help"], id="patchset-select"),
        pytest.param(("attest", "put"), ["--change", "--tests", "--lint", "--security", "--license", "--author-mode", "--model", "--session", "--checkpoint", "--remote", "--json", "--help"], id="attest-put"),
        pytest.param(("attest", "show"), ["--remote", "--json", "--help"], id="attest-show"),
        pytest.param(("review", "show"), ["--remote", "--json", "--help"], id="review-show"),
        pytest.param(("review", "code", "submit"), ["--verdict", "--reviewer", "--patchset", "--message", "--remote", "--json", "--help"], id="review-code-submit"),
        pytest.param(("review", "task", "approve"), ["--reviewer", "--patchset", "--message", "--remote", "--json", "--help"], id="review-task-approve"),
        pytest.param(("review", "task", "request-changes"), ["--reviewer", "--patchset", "--message", "--remote", "--json", "--help"], id="review-task-request-changes"),
        pytest.param(("review", "task", "comment"), ["--reviewer", "--patchset", "--message", "--remote", "--json", "--help"], id="review-task-comment"),
        pytest.param(("review", "task", "defer"), ["--reviewer", "--patchset", "--message", "--remote", "--json", "--help"], id="review-task-defer"),
        pytest.param(("review", "team", "request"), ["--group", "--patchset", "--note", "--remote", "--json", "--help"], id="review-team-request"),
        pytest.param(("review", "team", "approve"), ["--reviewer", "--patchset", "--message", "--remote", "--json", "--help"], id="review-team-approve"),
        pytest.param(("review", "team", "request-changes"), ["--reviewer", "--patchset", "--message", "--remote", "--json", "--help"], id="review-team-request-changes"),
        pytest.param(("review", "team", "comment"), ["--reviewer", "--patchset", "--message", "--remote", "--json", "--help"], id="review-team-comment"),
        pytest.param(("review", "team", "defer"), ["--reviewer", "--patchset", "--message", "--remote", "--json", "--help"], id="review-team-defer"),
        pytest.param(("policy", "eval"), ["--remote", "--json", "--help"], id="policy-eval"),
        pytest.param(("policy", "show"), ["--remote", "--json", "--help"], id="policy-show"),
        pytest.param(("policy", "waive"), ["--rule", "--reason", "--expires-at", "--remote", "--json", "--help"], id="policy-waive"),
        pytest.param(("land", "submit"), ["--patchset", "--target", "--mode", "--session", "--remote", "--repo", "--json", "--help"], id="land-submit"),
        pytest.param(("land", "show"), ["--remote", "--repo", "--json", "--help"], id="land-show"),
        pytest.param(("land", "retry"), ["--reason", "--remote", "--repo", "--json", "--help"], id="land-retry"),
    ],
)
def test_help_option_inventory_is_stable(argv: tuple[str, ...], expected_options: list[str]):
    options, commands = _help_inventory(*argv)
    assert commands == []
    assert options == expected_options


def test_plan_help_omits_legacy_sync_and_default_line_surface():
    plan_help = runner.invoke(app, ["plan", "--help"], catch_exceptions=False)
    assert plan_help.exit_code == 0, plan_help.stdout
    sync_help = runner.invoke(app, ["plan", "sync", "--help"], catch_exceptions=False)
    assert sync_help.exit_code == 0, sync_help.stdout

    for forbidden in ("line_sync", "root_main_sync", "remote_main_sync", "--default-line"):
        assert forbidden not in plan_help.stdout
        assert forbidden not in sync_help.stdout


def test_task_start_help_keeps_scope_override_copy_explicit():
    normalized = _normalized_help_output("task", "start")
    assert "Force local scope; overrides workflow scope config." in normalized
    assert "omitting both `--local` and `--remote` usually follows the remote-backed default." in normalized
    assert "Force the selected remote scope; overrides workflow scope config." in normalized
    assert "omitting this usually already follows the remote-backed default." in normalized


def test_change_create_help_keeps_scope_override_copy_explicit():
    normalized = _normalized_help_output("change", "create")
    assert "Force local scope; overrides workflow scope config." in normalized
    assert "omitting both `--local` and `--remote` usually follows the remote-backed default." in normalized
    assert "Force the selected remote scope; overrides workflow scope config." in normalized
    assert "omitting this usually already follows the remote-backed default." in normalized


def test_plan_sync_help_keeps_remote_option_boundary_copy_explicit():
    normalized = _normalized_help_output("plan", "sync")
    assert "After local sync, publish touched local plan revisions to the selected remote." in normalized
    assert "Use `--remote origin` for the explicit shared-lineage Markdown sync boundary;" in normalized
    assert "omitting `--remote` keeps the sync local-only." in normalized
