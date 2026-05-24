from __future__ import annotations

from typing import Optional

import click
import typer

from .bootstrap_views import _emit
from .review_submission_helpers import _review_action_result
from .session_runtime_helpers import (
    _finish_session_command_tracking,
    _start_session_command_tracking,
)
from ..command_profiling import _finish_command_profiling, _start_command_profiling
from ..store import RepoContext


class AITTyperGroup(typer.core.TyperGroup):
    def invoke(self, ctx: click.Context):
        protected_args = getattr(ctx, "_protected_args", None)
        if protected_args is None:
            protected_args = getattr(ctx, "protected_args", []) or []
        command_tokens = [str(item) for item in protected_args]
        command_tokens.extend(str(item) for item in ctx.args or [])
        tracker = _start_session_command_tracking(command_tokens)
        profiler = _start_command_profiling(command_tokens)
        try:
            result = super().invoke(ctx)
        except click.ClickException as exc:
            _finish_session_command_tracking(tracker, exc.exit_code)
            _finish_command_profiling(profiler, exc.exit_code)
            raise
        except click.exceptions.Exit as exc:
            _finish_session_command_tracking(tracker, int(exc.exit_code or 0))
            _finish_command_profiling(profiler, int(exc.exit_code or 0))
            raise
        except click.Abort:
            _finish_session_command_tracking(tracker, 1)
            _finish_command_profiling(profiler, 1)
            raise
        except Exception:
            _finish_session_command_tracking(tracker, 1)
            _finish_command_profiling(profiler, 1)
            raise
        _finish_session_command_tracking(tracker, 0)
        _finish_command_profiling(profiler, 0)
        return result


app = typer.Typer(help="ait native local repository and workflow tool", cls=AITTyperGroup)
auth_app = typer.Typer(help="Inspect and manage native auth and role bindings")
repo_app = typer.Typer(help="Inspect advanced operator state for the bound remote repository")
gc_app = typer.Typer(help="Run advanced pack, compaction, and garbage-collection utilities for local native storage")
doctor_app = typer.Typer(help="Run local workflow and deployment checks")
queue_app = typer.Typer(help="Use helper inventory reads for the shared queue and change status")
workflow_app = typer.Typer(help="Use helper/orchestrator entrypoints for common workflow bursts")
benchmark_app = typer.Typer(help="Run local benchmark utilities")
remote_app = typer.Typer(help="Manage ait-native remotes")
config_app = typer.Typer(help="Inspect effective workflow modes and update local repository defaults")
line_app = typer.Typer(help="Manage native lines")
worktree_app = typer.Typer(help="Manage isolated native worktrees that share the same .ait repository state")
workspace_app = typer.Typer(help="Restore local workspace content from snapshots or line heads")
stash_app = typer.Typer(help="Park, inspect, restore, and drop temporary local-only stash snapshots")
snapshot_app = typer.Typer(help="Freeze and inspect immutable workspace snapshots for review and land gates")
ref_app = typer.Typer(help="Inspect and move advanced native refs")
plan_app = typer.Typer(help="Manage durable native plans")
plan_session_app = typer.Typer(help="Manage remote plan-rooted planning sessions")
task_app = typer.Typer(help="Manage native tasks")
change_app = typer.Typer(help="Manage native changes")
session_app = typer.Typer(help="Manage native sessions, events, and checkpoints")
stack_app = typer.Typer(help="Manage native stacks")
patchset_app = typer.Typer(help="Run core patchset publication and inspection commands")
review_app = typer.Typer(help="Run core AI-code, human-task, and optional team review-lane commands")
attest_app = typer.Typer(help="Run core patchset evidence and provenance commands")
policy_app = typer.Typer(help="Run core readiness-gate evaluation and waiver commands")
land_app = typer.Typer(help="Run core guarded remote landing commands")
release_app = typer.Typer(help="Create, check, build, publish, and inspect native release candidates")

_SUBAPPS_REGISTERED = False


def register_cli_subapps() -> None:
    global _SUBAPPS_REGISTERED
    if _SUBAPPS_REGISTERED:
        return
    app.add_typer(auth_app, name="auth")
    app.add_typer(repo_app, name="repo")
    app.add_typer(gc_app, name="gc")
    app.add_typer(doctor_app, name="doctor")
    app.add_typer(queue_app, name="queue")
    app.add_typer(workflow_app, name="workflow")
    app.add_typer(benchmark_app, name="benchmark")
    app.add_typer(remote_app, name="remote")
    app.add_typer(config_app, name="config")
    app.add_typer(line_app, name="line")
    app.add_typer(worktree_app, name="worktree")
    app.add_typer(workspace_app, name="workspace")
    app.add_typer(stash_app, name="stash")
    app.add_typer(snapshot_app, name="snapshot")
    app.add_typer(ref_app, name="ref")
    plan_app.add_typer(plan_session_app, name="session")
    app.add_typer(plan_app, name="plan")
    app.add_typer(task_app, name="task")
    app.add_typer(change_app, name="change")
    app.add_typer(session_app, name="session")
    app.add_typer(stack_app, name="stack")
    app.add_typer(patchset_app, name="patchset")
    app.add_typer(review_app, name="review")
    app.add_typer(attest_app, name="attest")
    app.add_typer(policy_app, name="policy")
    app.add_typer(land_app, name="land")
    app.add_typer(release_app, name="release")
    _SUBAPPS_REGISTERED = True


def _ctx() -> RepoContext:
    try:
        return RepoContext.discover()
    except FileNotFoundError as exc:
        raise typer.BadParameter(str(exc)) from exc


TASK_TRACKING_MODES = frozenset({"on", "off"})


def _review_action(
    change_id: str,
    reviewer: Optional[str],
    action: str,
    blocking: bool,
    patchset: Optional[str],
    message: Optional[str],
    remote: Optional[str],
    json_output: bool,
) -> None:
    ctx = _ctx()
    data = _review_action_result(
        ctx,
        change_id=change_id,
        reviewer=reviewer,
        action=action,
        blocking=blocking,
        patchset=patchset,
        message=message,
        remote=remote,
    )
    _emit(data, json_output)


__all__ = [
    "AITTyperGroup",
    "TASK_TRACKING_MODES",
    "_ctx",
    "_review_action",
    "app",
    "attest_app",
    "auth_app",
    "benchmark_app",
    "change_app",
    "config_app",
    "doctor_app",
    "gc_app",
    "land_app",
    "line_app",
    "patchset_app",
    "plan_app",
    "plan_session_app",
    "policy_app",
    "queue_app",
    "ref_app",
    "register_cli_subapps",
    "release_app",
    "remote_app",
    "repo_app",
    "review_app",
    "session_app",
    "stash_app",
    "snapshot_app",
    "stack_app",
    "task_app",
    "workflow_app",
    "workspace_app",
    "worktree_app",
]
