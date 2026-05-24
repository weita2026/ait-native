from __future__ import annotations

import importlib
from collections.abc import Sequence

_COMMAND_MODULES = (
    "blame",
    "config",
    "queue",
    "queue_workflow_land",
    "workflow",
    "benchmark",
    "gc",
    "doctor",
    "repo",
    "remote",
    "line",
    "worktree",
    "workspace",
    "stash",
    "snapshot",
    "ref",
    "plan",
    "plan_session",
    "task",
    "session",
    "change",
    "patchset",
    "land",
    "review",
    "attest",
    "policy",
    "auth",
    "stack",
    "release",
)

_COMMAND_MODULE_SET = frozenset(_COMMAND_MODULES)

_PRIMARY_COMMAND_MODULES: dict[str, tuple[str, ...]] = {
    "auth": ("auth",),
    "benchmark": ("benchmark",),
    "blame": ("blame",),
    "change": ("change",),
    "config": ("config",),
    "doctor": ("doctor",),
    "fetch": ("repo",),
    "gc": ("gc",),
    "history": ("config",),
    "land": ("land",),
    "line": ("line",),
    "patchset": ("patchset",),
    "plan": ("plan", "plan_session"),
    "policy": ("policy",),
    "pull": ("repo",),
    "push": ("repo",),
    "queue": ("queue",),
    "ref": ("ref",),
    "release": ("release",),
    "remote": ("remote",),
    "repo": ("repo",),
    "review": ("review",),
    "session": ("session",),
    "snapshot": ("snapshot",),
    "stack": ("stack",),
    "stash": ("stash",),
    "status": ("config",),
    "task": ("task",),
    "workflow": ("queue_workflow_land", "workflow"),
    "workspace": ("workspace",),
    "worktree": ("worktree",),
    "attest": ("attest",),
}

_IMPORTED_MODULES: set[str] = set()


def _bootstrap_targets(command_tokens: Sequence[str] | None = None) -> tuple[str, ...]:
    if not command_tokens:
        return _COMMAND_MODULES
    first = str(command_tokens[0] or "").strip()
    if not first or first.startswith("-"):
        return _COMMAND_MODULES
    modules = _PRIMARY_COMMAND_MODULES.get(first)
    if modules is None:
        return _COMMAND_MODULES
    return modules


def bootstrap_cli_commands(command_tokens: Sequence[str] | None = None) -> None:
    package = __package__ or "ait.cli.commands"
    for module_name in _bootstrap_targets(command_tokens):
        if module_name in _IMPORTED_MODULES:
            continue
        if module_name not in _COMMAND_MODULE_SET:
            continue
        importlib.import_module(f".{module_name}", package)
        _IMPORTED_MODULES.add(module_name)


__all__ = ["bootstrap_cli_commands"]
