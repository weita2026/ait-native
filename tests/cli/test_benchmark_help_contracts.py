from __future__ import annotations

import re

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


def test_benchmark_help_lists_static_web_task_command() -> None:
    options, commands = _help_inventory("benchmark")
    assert options == ["--help"]
    assert commands == [
        "static-web-hardening-task",
        "static-web-task",
        "token-savings",
        "token-savings-status",
        "codex-usage",
        "codex-fill-usage",
        "strict-rerun",
        "local-first-final-land",
        "local-snapshot-performance",
    ]


def test_benchmark_static_web_task_help_options_are_stable() -> None:
    options, commands = _help_inventory("benchmark", "static-web-task")
    assert commands == []
    assert options == ["--manifest", "--output-json", "--output-markdown", "--json", "--help"]


def test_benchmark_static_web_hardening_task_help_options_are_stable() -> None:
    options, commands = _help_inventory("benchmark", "static-web-hardening-task")
    assert commands == []
    assert options == ["--manifest", "--output-json", "--output-markdown", "--json", "--help"]


def test_benchmark_local_snapshot_performance_help_options_are_stable() -> None:
    options, commands = _help_inventory("benchmark", "local-snapshot-performance")
    assert commands == []
    assert options == ["--manifest", "--output-json", "--output-markdown", "--json", "--help"]
