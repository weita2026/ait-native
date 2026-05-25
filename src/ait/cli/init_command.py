from __future__ import annotations

from pathlib import Path
from typing import Optional

import typer

from ait_protocol.common import AuthorMode

from .bootstrap_views import (
    _INIT_BOOTSTRAP_DIRECTORIES,
    _INIT_BOOTSTRAP_FILES,
    _INIT_FORBIDDEN_BOOTSTRAP_PATHS,
    _emit,
    _init_next_steps,
    _render_init_summary,
)
from .workflow_mode_config import (
    _effective_task_worktree,
    _effective_workflow_mode,
)
from ..store import init_repo, load_config, load_policy


def register_init_command(app: typer.Typer) -> None:
    @app.command("init")
    def init_cmd(
        name: Optional[str] = typer.Option(None, "--name", help="Repository name"),
        default_line: str = typer.Option("main", "--default-line", help="Default line name"),
        policy_profile: str = typer.Option("prototype", "--policy-profile", help="Initial repository policy profile"),
        default_author_mode: AuthorMode = typer.Option(
            AuthorMode.AI_WITH_HUMAN_REVIEW,
            "--default-author-mode",
            help="Default provenance author mode for patchsets and attestations",
        ),
        default_model: Optional[str] = typer.Option(
            None,
            "--default-model",
            help="Default provenance model name when auto-detection is unavailable",
        ),
        json_output: bool = typer.Option(False, "--json", help="JSON output"),
    ):
        cwd = Path.cwd()
        repo_name = name or cwd.name
        try:
            ctx = init_repo(
                cwd,
                name,
                default_line,
                policy_profile_name=policy_profile,
                default_author_mode=default_author_mode.value,
                default_model=default_model,
            )
        except KeyError as exc:
            raise typer.BadParameter(str(exc)) from exc
        policy = load_policy(ctx)
        config = load_config(ctx)
        payload = {
            "repo_root": str(ctx.root),
            "repo_name": repo_name,
            "default_line": default_line,
            "workflow_mode": _effective_workflow_mode(ctx),
            "task_worktree": _effective_task_worktree(ctx),
            "policy_profile": policy["policy_id"],
            "default_author_mode": config.get("default_author_mode"),
            "default_model": config.get("default_model"),
            "bootstrap_files": [{"path": path} for path in _INIT_BOOTSTRAP_FILES],
            "bootstrap_guide": {"path": "ait-native.md"},
            "bootstrap_directories": [{"path": path} for path in _INIT_BOOTSTRAP_DIRECTORIES],
            "forbidden_bootstrap_paths": [{"path": path} for path in _INIT_FORBIDDEN_BOOTSTRAP_PATHS],
            "next_steps": _init_next_steps(repo_name, default_line),
        }
        if json_output:
            _emit(payload, True)
        else:
            _render_init_summary(payload)
