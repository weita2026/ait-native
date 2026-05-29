from __future__ import annotations

import re

import pytest

from ._shared import *  # noqa: F401,F403


def _help_inventory(*argv: str) -> tuple[list[str], list[str]]:
    help_out = runner.invoke(app, [*argv, "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout

    section: str | None = None
    options: list[str] = []
    commands: list[str] = []
    for raw_line in help_out.stdout.splitlines():
        line = raw_line.rstrip()
        if line.startswith("╭─ Options"):
            section = "options"
            continue
        if line.startswith("╭─ Commands"):
            section = "commands"
            continue
        if line.startswith("╭─ Arguments"):
            section = "arguments"
            continue
        if line.startswith("╰"):
            section = None
            continue
        if section == "options":
            option_match = re.match(r"│\s*(?:\*\s+)?(--[a-z0-9-]+)\b", line)
            if option_match:
                options.append(option_match.group(1))
        elif section == "commands":
            command_match = re.match(r"│\s*([a-z][a-z0-9-]*)\s{2,}", line)
            if command_match:
                commands.append(command_match.group(1))
    return options, commands


@pytest.mark.parametrize(
    ("argv", "expected_commands"),
    [
        pytest.param(("doctor",), ["runtime-root", "postgres"], id="doctor"),
        pytest.param(("stack",), ["create", "list", "show", "add", "remove", "reorder", "update", "graph"], id="stack"),
        pytest.param(("stash",), ["save", "list", "show", "apply", "pop", "drop"], id="stash"),
        pytest.param(("auth",), ["whoami", "grant", "bindings"], id="auth"),
        pytest.param(("ref",), ["list", "show", "history", "move"], id="ref"),
    ],
)
def test_extra_help_group_command_inventory_is_stable(argv: tuple[str, ...], expected_commands: list[str]):
    options, commands = _help_inventory(*argv)
    assert options == ["--help"]
    assert commands == expected_commands


@pytest.mark.parametrize(
    ("argv", "expected_options"),
    [
        pytest.param(("fetch",), ["--remote", "--line", "--snapshot", "--json", "--help"], id="fetch"),
        pytest.param(("pull",), ["--remote", "--line", "--json", "--help"], id="pull"),
        pytest.param(("doctor", "runtime-root"), ["--server-data", "--json", "--help"], id="doctor-runtime-root"),
        pytest.param(
            ("doctor", "postgres"),
            ["--server-data", "--backend", "--dsn", "--content-schema", "--control-schema", "--connect", "--json", "--help"],
            id="doctor-postgres",
        ),
        pytest.param(("stack", "create"), ["--title", "--change", "--landing-policy", "--remote", "--json", "--help"], id="stack-create"),
        pytest.param(("stack", "list"), ["--remote", "--json", "--help"], id="stack-list"),
        pytest.param(("stack", "show"), ["--remote", "--json", "--help"], id="stack-show"),
        pytest.param(("stack", "add"), ["--change", "--position", "--remote", "--json", "--help"], id="stack-add"),
        pytest.param(("stack", "remove"), ["--change", "--remote", "--json", "--help"], id="stack-remove"),
        pytest.param(("stack", "reorder"), ["--change", "--position", "--remote", "--json", "--help"], id="stack-reorder"),
        pytest.param(("stack", "update"), ["--title", "--landing-policy", "--status", "--remote", "--json", "--help"], id="stack-update"),
        pytest.param(("stack", "graph"), ["--remote", "--json", "--help"], id="stack-graph"),
        pytest.param(("stash", "save"), ["--message", "--keep-workspace", "--json", "--help"], id="stash-save"),
        pytest.param(("stash", "list"), ["--json", "--help"], id="stash-list"),
        pytest.param(("stash", "show"), ["--json", "--help"], id="stash-show"),
        pytest.param(("stash", "apply"), ["--force", "--json", "--help"], id="stash-apply"),
        pytest.param(("stash", "pop"), ["--force", "--json", "--help"], id="stash-pop"),
        pytest.param(("stash", "drop"), ["--json", "--help"], id="stash-drop"),
        pytest.param(("auth", "whoami"), ["--remote", "--repo", "--json", "--help"], id="auth-whoami"),
        pytest.param(("auth", "bindings"), ["--repo", "--remote", "--json", "--help"], id="auth-bindings"),
        pytest.param(("ref", "list"), ["--json", "--help"], id="ref-list"),
        pytest.param(("ref", "show"), ["--json", "--help"], id="ref-show"),
        pytest.param(("ref", "history"), ["--limit", "--json", "--help"], id="ref-history"),
        pytest.param(("ref", "move"), ["--json", "--help"], id="ref-move"),
        pytest.param(("line", "show"), ["--json", "--help"], id="line-show"),
        pytest.param(("line", "archive"), ["--remote", "--json", "--help"], id="line-archive"),
        pytest.param(("workspace", "status"), ["--snapshot", "--line", "--json", "--help"], id="workspace-status"),
        pytest.param(("snapshot", "diff"), ["--include-text", "--max-bytes", "--json", "--help"], id="snapshot-diff"),
        pytest.param(("snapshot", "revert"), ["--force", "--dry-run", "--json", "--help"], id="snapshot-revert"),
        pytest.param(("snapshot", "replay"), ["--onto", "--force", "--dry-run", "--json", "--help"], id="snapshot-replay"),
        pytest.param(("plan", "graph"), ["--from-json", "--remote", "--json", "--help"], id="plan-graph"),
        pytest.param(("plan", "ready"), ["--from-json", "--remote", "--json", "--help"], id="plan-ready"),
        pytest.param(("plan", "schedule"), ["--from-json", "--remote", "--json", "--help"], id="plan-schedule"),
        pytest.param(("plan", "progress"), ["--from-json", "--remote", "--json", "--help"], id="plan-progress"),
        pytest.param(
            ("plan", "execute"),
            [
                "--from-json",
                "--auto-compact-worker",
                "--comparison-evidence-report",
                "--comparison-evidence-workload",
                "--pause-run",
                "--resume-run",
                "--abort-run",
                "--run-session",
                "--latest-run",
                "--yes",
                "--allow-stale",
                "--remote",
                "--json",
                "--help",
            ],
            id="plan-execute",
        ),
    ],
)
def test_extra_help_option_inventory_is_stable(argv: tuple[str, ...], expected_options: list[str]):
    options, commands = _help_inventory(*argv)
    assert commands == []
    assert options == expected_options


def test_fetch_help_distinguishes_non_mutating_refresh_from_pull() -> None:
    fetch_out = runner.invoke(app, ["fetch", "--help"], catch_exceptions=False)
    assert fetch_out.exit_code == 0, fetch_out.stdout
    fetch_help = " ".join(fetch_out.stdout.split())
    assert "without moving local line heads or restoring workspace files" in fetch_help

    pull_out = runner.invoke(app, ["pull", "--help"], catch_exceptions=False)
    assert pull_out.exit_code == 0, pull_out.stdout
    pull_help = " ".join(pull_out.stdout.split())
    assert "refresh local line heads and snapshots" in pull_help
