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
        pytest.param(("release",), ["check", "build", "formula", "show", "publish", "candidate"], id="release"),
        pytest.param(("release", "candidate"), ["create"], id="release-candidate"),
    ],
)
def test_release_help_group_command_inventory_is_stable(argv: tuple[str, ...], expected_commands: list[str]):
    options, commands = _help_inventory(*argv)
    assert options == ["--help"]
    assert commands == expected_commands


@pytest.mark.parametrize(
    ("argv", "expected_options"),
    [
        pytest.param(("release", "candidate", "create"), ["--version", "--line", "--profile", "--json", "--help"], id="release-candidate-create"),
        pytest.param(("release", "check"), ["--tests-command", "--skip-tests-reason", "--json", "--help"], id="release-check"),
        pytest.param(("release", "build"), ["--json", "--help"], id="release-build"),
        pytest.param(("release", "formula"), ["--name", "--json", "--help"], id="release-formula"),
        pytest.param(("release", "show"), ["--remote", "--json", "--help"], id="release-show"),
        pytest.param(("release", "publish"), ["--remote", "--json", "--help"], id="release-publish"),
    ],
)
def test_release_help_option_inventory_is_stable(argv: tuple[str, ...], expected_options: list[str]):
    options, commands = _help_inventory(*argv)
    assert commands == []
    assert options == expected_options
