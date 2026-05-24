from __future__ import annotations

import fcntl
import hashlib
import json
import os
from contextlib import contextmanager
from pathlib import Path
from typing import Any

from ait_protocol.common import utc_now

from ..store import RepoContext
from .plan_markdown_authoring import _guard_execution_worktree_snapshot_markdown
from .task_worktree_runtime import (
    _guard_active_root_worktree,
    _guard_internal_worktree_role,
    _guard_readonly_root_main,
    _guard_task_bound_authoring,
)


class WorkspaceCommandBusyError(ValueError):
    def __init__(self, workspace_root: str, attempted_command: str, holder: dict | None = None):
        summary: list[str] = []
        if isinstance(holder, dict):
            if holder.get("command"):
                summary.append(str(holder["command"]))
            if holder.get("pid") is not None:
                summary.append(f"pid {holder['pid']}")
            if holder.get("started_at"):
                summary.append(f"started {holder['started_at']}")
        detail = f" ({', '.join(summary)})" if summary else ""
        super().__init__(
            f"Workspace command lock is busy for {workspace_root}. "
            f"Another stateful ait command is already running{detail}. "
            f"Wait for it to finish before running `{attempted_command}`."
        )
        self.workspace_root = workspace_root
        self.attempted_command = attempted_command
        self.holder = holder or {}


def _workspace_command_lock_path(ctx: RepoContext) -> Path:
    workspace_id = hashlib.sha256(str(ctx.root.resolve()).encode("utf-8")).hexdigest()[:16]
    return ctx.workspace_dir / "locks" / f"{workspace_id}.lock"


def _read_workspace_command_lock(path: Path) -> dict[str, Any]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError, OSError):
        return {}
    return payload if isinstance(payload, dict) else {}


@contextmanager
def _workspace_command_lock(ctx: RepoContext, command_name: str):
    lock_path = _workspace_command_lock_path(ctx)
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    workspace_root = str(ctx.root.resolve())
    metadata = {
        "command": command_name,
        "pid": os.getpid(),
        "repo_root": str(ctx.repo_root),
        "workspace_root": workspace_root,
        "started_at": utc_now(),
    }
    with lock_path.open("a+", encoding="utf-8") as handle:
        try:
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise WorkspaceCommandBusyError(workspace_root, command_name, _read_workspace_command_lock(lock_path)) from exc
        handle.seek(0)
        handle.truncate()
        json.dump(metadata, handle, indent=2, sort_keys=True)
        handle.write("\n")
        handle.flush()
        try:
            yield metadata
        finally:
            handle.seek(0)
            handle.truncate()
            handle.flush()
            fcntl.flock(handle.fileno(), fcntl.LOCK_UN)


def _run_locked_workspace_command(ctx: RepoContext, command_name: str, operation):
    _guard_internal_worktree_role(ctx, command_name)
    _guard_active_root_worktree(ctx, command_name)
    _guard_readonly_root_main(ctx, command_name)
    with _workspace_command_lock(ctx, command_name):
        return operation()


def _run_locked_task_bound_authoring_command(ctx: RepoContext, command_name: str, operation, **guard_kwargs):
    _guard_internal_worktree_role(ctx, command_name)
    _guard_active_root_worktree(ctx, command_name)
    _guard_readonly_root_main(ctx, command_name)
    _guard_task_bound_authoring(ctx, command_name, **guard_kwargs)
    _guard_execution_worktree_snapshot_markdown(ctx, command_name=command_name)
    with _workspace_command_lock(ctx, command_name):
        return operation()
