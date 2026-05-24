from __future__ import annotations

import json
from typing import Any

import typer
from rich import print as rprint

from .task_worktree_guidance import _render_task_worktree_guidance

_INIT_BOOTSTRAP_FILES = ("AGENTS.md", "ait-native.md", "docs/plan.md", "docs/milestone.md")
_INIT_BOOTSTRAP_DIRECTORIES = ("docs/sprints",)
_INIT_FORBIDDEN_BOOTSTRAP_PATHS = ("docs/sprints/README.md",)


def _emit_task_creation_payload(payload: dict[str, Any], *, json_output: bool) -> None:
    _emit(payload, json_output)
    if not json_output:
        _render_task_worktree_guidance(payload.get("worktree_guidance"))


def _emit(data, json_output: bool = False) -> None:
    if json_output:
        typer.echo(json.dumps(data, indent=2, sort_keys=True))
    else:
        rprint(data)


def _init_next_steps(repo_name: str, default_line: str) -> dict[str, list[str]]:
    return {
        "workflow_guides": [
            "ait workflow guide inventory",
            "ait workflow guide land",
        ],
        "solo_local": [
            'ait task start --local --title "Describe the work" --intent "Explain the outcome" --base-line '
            f"{default_line}",
            'ait snapshot create --message "bootstrap"',
        ],
        "optional_solo_remote": [
            f"ait remote add origin <url> --repo-name {repo_name} --default",
            "ait config set --workflow-mode solo_remote",
            "ait plan sync docs/plan.md --remote origin",
            'ait task start --title "Describe the work" --intent "Explain the outcome" --base-line '
            f"{default_line}",
        ],
    }


def _render_init_summary(data: dict[str, Any]) -> None:
    lines = ["ait init complete", ""]
    lines.append(f"- repo: {data.get('repo_name')}")
    lines.append(f"- root: {data.get('repo_root')}")
    lines.append(f"- default line: {data.get('default_line')}")
    workflow_mode = data.get("workflow_mode") if isinstance(data.get("workflow_mode"), dict) else {}
    lines.append(f"- default workflow mode: {workflow_mode.get('value') or 'unknown'}")
    bootstrap_guide = data.get("bootstrap_guide") if isinstance(data.get("bootstrap_guide"), dict) else {}
    if bootstrap_guide.get("path"):
        lines.append(f"- bootstrap guide: {bootstrap_guide['path']}")
    lines.append(f"- policy profile: {data.get('policy_profile')}")
    task_worktree = data.get("task_worktree") if isinstance(data.get("task_worktree"), dict) else {}
    root_mode = task_worktree.get("root_mode") if isinstance(task_worktree.get("root_mode"), dict) else {}
    lines.append(
        "- task worktree root mode: "
        f"{root_mode.get('value') or 'unknown'} ({root_mode.get('source') or 'unknown'})"
    )
    lines.extend(["", "Bootstrap files"])
    for row in data.get("bootstrap_files") or []:
        if isinstance(row, dict):
            lines.append(f"- {row.get('path')}")
    bootstrap_directories = (
        data.get("bootstrap_directories") if isinstance(data.get("bootstrap_directories"), list) else []
    )
    if bootstrap_directories:
        lines.extend(["", "Bootstrap directories"])
        for row in bootstrap_directories:
            if isinstance(row, dict):
                lines.append(f"- {row.get('path')}")
    forbidden_bootstrap_paths = (
        data.get("forbidden_bootstrap_paths") if isinstance(data.get("forbidden_bootstrap_paths"), list) else []
    )
    if forbidden_bootstrap_paths:
        lines.extend(["", "Explicitly not bootstrapped"])
        for row in forbidden_bootstrap_paths:
            if isinstance(row, dict):
                lines.append(f"- {row.get('path')}")
    next_steps = data.get("next_steps") if isinstance(data.get("next_steps"), dict) else {}
    workflow_guides = next_steps.get("workflow_guides") if isinstance(next_steps.get("workflow_guides"), list) else []
    if workflow_guides:
        lines.extend(["", "Recommended guides"])
        for command in workflow_guides:
            lines.append(f"- {command}")
    solo_local = next_steps.get("solo_local") if isinstance(next_steps.get("solo_local"), list) else []
    if solo_local:
        lines.extend(["", "Default solo-local next steps"])
        for command in solo_local:
            lines.append(f"- {command}")
    solo_remote = (
        next_steps.get("optional_solo_remote") if isinstance(next_steps.get("optional_solo_remote"), list) else []
    )
    if solo_remote:
        lines.extend(["", "Optional solo-remote setup"])
        for command in solo_remote:
            lines.append(f"- {command}")
    typer.echo("\n".join(lines))


__all__ = [
    "_INIT_BOOTSTRAP_DIRECTORIES",
    "_INIT_BOOTSTRAP_FILES",
    "_INIT_FORBIDDEN_BOOTSTRAP_PATHS",
    "_emit",
    "_emit_task_creation_payload",
    "_init_next_steps",
    "_render_init_summary",
]
