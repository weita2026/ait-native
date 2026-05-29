from __future__ import annotations

import hashlib
import os
import shlex
import time
from typing import Any

import click

from ait_protocol.common import normalize_optional_text, utc_now

from ..remote_client import RemoteError
from ..store import (
    RepoContext,
    current_line,
    load_config,
)
from ..store_local_sessions import append_local_session_event, get_local_session
from ..store_remotes import (
    list_remotes,
)
from .remote_repository_defaults import _remote_tuple
from .remote_session_wrappers import remote_append_session_event, remote_get_session
from .runtime_defaults import _effective_actor_identity, _effective_session_id, _normalize_text_value
from .session_command_analysis import _extract_ait_command
from .task_tracking_bindings import _task_tracking_enabled, _task_tracking_hard_disabled, _tracked_session_binding


def _maybe_ctx() -> RepoContext | None:
    try:
        return RepoContext.discover()
    except FileNotFoundError:
        return None


def _env_truthy(name: str) -> bool:
    value = normalize_optional_text(os.environ.get(name))
    if value is None:
        return False
    return value.lower() in {"1", "true", "yes", "on"}


def _session_autolog_enabled() -> bool:
    ctx = _maybe_ctx()
    if _task_tracking_hard_disabled(ctx):
        return False
    value = normalize_optional_text(os.environ.get("AIT_SESSION_AUTOLOG"))
    if value is not None:
        lowered = value.lower()
        if lowered in {"0", "false", "no", "off"}:
            return False
        return True
    if _env_truthy("AIT_SESSION_LOCAL") or _normalize_text_value(os.environ.get("AIT_SESSION_REMOTE")) is not None:
        return True
    return False


def _session_autolog_raw_command(command_tokens: list[str]) -> str:
    if not command_tokens:
        return "ait"
    return "ait " + " ".join(shlex.quote(str(token)) for token in command_tokens)


def _session_autolog_remote_names(ctx: RepoContext) -> list[str]:
    cfg = load_config(ctx)
    names: list[str] = []
    default_remote = _normalize_text_value(cfg.get("default_remote"))
    if default_remote:
        names.append(default_remote)
    for row in list_remotes(ctx):
        name = _normalize_text_value(row.get("name"))
        if name and name not in names:
            names.append(name)
    return names


def _resolve_session_autolog_target(ctx: RepoContext, session_id: str) -> dict[str, Any] | None:
    explicit_remote = _normalize_text_value(os.environ.get("AIT_SESSION_REMOTE"))
    explicit_local = _env_truthy("AIT_SESSION_LOCAL")
    if explicit_local:
        get_local_session(ctx, session_id)
        return {"scope": "local"}
    if explicit_remote:
        remote_row, repo_name = _remote_tuple(ctx, explicit_remote)
        remote_get_session(remote_row["url"], session_id, repo_name=repo_name)
        return {
            "scope": "remote",
            "remote": remote_row,
            "remote_name": remote_row.get("name") or explicit_remote,
            "repo_name": repo_name,
        }
    tracked_binding = _tracked_session_binding(ctx) if _task_tracking_enabled(ctx) else None
    if tracked_binding is not None and tracked_binding["session_id"] == session_id:
        if tracked_binding["scope"] == "local":
            get_local_session(ctx, session_id)
            return {"scope": "local"}
        remote_name = tracked_binding.get("remote_name")
        if remote_name:
            remote_row, repo_name = _remote_tuple(ctx, remote_name)
            remote_get_session(remote_row["url"], session_id, repo_name=repo_name)
            return {
                "scope": "remote",
                "remote": remote_row,
                "remote_name": remote_row.get("name") or remote_name,
                "repo_name": repo_name,
            }
    try:
        get_local_session(ctx, session_id)
        return {"scope": "local"}
    except KeyError:
        pass
    for remote_name in _session_autolog_remote_names(ctx):
        try:
            remote_row, repo_name = _remote_tuple(ctx, remote_name)
            remote_get_session(remote_row["url"], session_id, repo_name=repo_name)
            return {
                "scope": "remote",
                "remote": remote_row,
                "remote_name": remote_row.get("name") or remote_name,
                "repo_name": repo_name,
            }
        except (KeyError, RemoteError, ValueError):
            continue
    return None


def _resolve_session_turn_remote_target(
    ctx: RepoContext,
    session_id: str,
    *,
    remote_name: str | None = None,
) -> dict[str, Any]:
    explicit_remote = _normalize_text_value(remote_name) or _normalize_text_value(os.environ.get("AIT_SESSION_REMOTE"))
    if explicit_remote:
        remote_row, repo_name = _remote_tuple(ctx, explicit_remote)
        session = remote_get_session(remote_row["url"], session_id, repo_name=repo_name)
        return {
            "remote": remote_row,
            "remote_name": remote_row.get("name") or explicit_remote,
            "repo_name": repo_name,
            "session": session,
        }
    tracked_binding = _tracked_session_binding(ctx) if _task_tracking_enabled(ctx) else None
    if tracked_binding is not None and tracked_binding["session_id"] == session_id:
        if tracked_binding["scope"] == "local":
            get_local_session(ctx, session_id)
            raise ValueError(
                f"Session {session_id} is bound as a local tracked session. "
                "`ait session turn` requires a remote session so ait-server can generate the live reply."
            )
        tracked_remote_name = _normalize_text_value(tracked_binding.get("remote_name"))
        if tracked_remote_name:
            remote_row, repo_name = _remote_tuple(ctx, tracked_remote_name)
            session = remote_get_session(remote_row["url"], session_id, repo_name=repo_name)
            return {
                "remote": remote_row,
                "remote_name": remote_row.get("name") or tracked_remote_name,
                "repo_name": repo_name,
                "session": session,
            }
    for candidate_name in _session_autolog_remote_names(ctx):
        try:
            remote_row, repo_name = _remote_tuple(ctx, candidate_name)
            session = remote_get_session(remote_row["url"], session_id, repo_name=repo_name)
            return {
                "remote": remote_row,
                "remote_name": remote_row.get("name") or candidate_name,
                "repo_name": repo_name,
                "session": session,
            }
        except (KeyError, RemoteError, ValueError):
            continue
    try:
        get_local_session(ctx, session_id)
    except KeyError:
        pass
    else:
        raise ValueError(
            f"Session {session_id} exists only locally. "
            "`ait session turn` requires a remote session because the reply is generated by ait-server."
        )
    raise ValueError(
        f"Could not resolve a remote session target for {session_id}. "
        "Use a tracked remote task/session or the repository default remote when available; "
        "pass --remote <name> only for a non-default lookup."
    )


def _compact_dag_worker_session_start_command(
    metadata: dict[str, Any],
    *,
    remote_name: str | None = None,
) -> str | None:
    plan_id = _normalize_text_value(metadata.get("plan_id"))
    task_graph_json = _normalize_text_value(metadata.get("task_graph_json"))
    remote_flag = f" --remote {remote_name}" if _normalize_text_value(remote_name) is not None else ""
    if plan_id and task_graph_json:
        return f"ait plan execute {plan_id} --from-json {task_graph_json} --auto-compact-worker{remote_flag} --yes"
    return None


def _compact_dag_worker_session_turn_guard(
    session: dict[str, Any] | None,
    *,
    remote_name: str | None = None,
) -> str | None:
    if not isinstance(session, dict):
        return None
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    if not isinstance(metadata, dict):
        return None
    if _normalize_text_value(metadata.get("session_policy")) != "task_dag_compact_packet_worker":
        return None
    session_id = _normalize_text_value(session.get("session_id")) or "<session-id>"
    start_command = _compact_dag_worker_session_start_command(metadata, remote_name=remote_name)
    compact_surface = metadata.get("compact_packet_surface")
    packet_generation_required = bool(compact_surface.get("packet_generation_required")) if isinstance(compact_surface, dict) else False
    if packet_generation_required and not bool(metadata.get("packet_available")):
        batch_id = _normalize_text_value(metadata.get("batch_id"))
        if batch_id is not None:
            command = start_command or "ait plan execute <plan-id> --from-json <task-graph-json> --auto-compact-worker --yes"
            return (
                f"Session {session_id} is a compact DAG batch session scaffold (batch {batch_id}) "
                f"with packet generation still pending. Start the worker with `{command}` instead of "
                "`ait session turn`."
            )
    fresh_run_hint = (
        f" If you need a fresh compact-worker pass, start it with `{start_command}`."
        if start_command is not None
        else ""
    )
    return (
        f"Session {session_id} is a compact DAG worker session. Compact worker replies are generated "
        "locally and the remote session is reserved for durable lineage/events, so `ait session turn` "
        f"is disabled for this session.{fresh_run_hint}"
    )


def _detected_editor_surface(requested: str | None = None) -> str:
    explicit = _normalize_text_value(requested)
    if explicit is not None:
        return explicit.lower()
    term_program = str(os.environ.get("TERM_PROGRAM") or "").strip().lower()
    if term_program == "vscode":
        return "vscode"
    for name in ("VSCODE_GIT_IPC_HANDLE", "VSCODE_CWD", "VSCODE_IPC_HOOK_CLI"):
        if _normalize_text_value(os.environ.get(name)) is not None:
            return "vscode"
    return "editor"


def _default_session_turn_title(surface: str, requested: str | None = None) -> str | None:
    explicit = _normalize_text_value(requested)
    if explicit is not None:
        return explicit
    if surface == "vscode":
        return "VSCode Codex"
    if surface == "editor":
        return "Editor Codex"
    return None


def _build_session_command_log_payload(
    ctx: RepoContext,
    raw_command: str,
    command_tokens: list[str],
    *,
    command_id: str,
    phase: str,
    started_at: str,
    finished_at: str | None = None,
    returncode: int | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "command": raw_command,
        "argv": [str(token) for token in command_tokens],
        "command_id": command_id,
        "command_phase": phase,
        "capture_mode": "auto",
        "cwd": str(ctx.root),
        "workspace_root": str(ctx.root),
        "line_name": current_line(ctx),
        "started_at": started_at,
    }
    cfg = load_config(ctx)
    worktree_name = _normalize_text_value(cfg.get("worktree_name"))
    if worktree_name:
        payload["worktree_name"] = worktree_name
    parsed = _extract_ait_command(raw_command)
    if parsed is not None:
        payload["top_level"] = parsed["top_level"]
        payload["command_path"] = parsed["command_path"]
        payload["target"] = parsed.get("target")
    if finished_at is not None:
        payload["finished_at"] = finished_at
    if returncode is not None:
        payload["returncode"] = int(returncode)
        payload["success"] = int(returncode) == 0
    return payload


def _append_session_autolog_event(
    ctx: RepoContext,
    target: dict[str, Any],
    session_id: str,
    event_type: str,
    payload: dict[str, Any],
) -> dict[str, Any]:
    if target.get("scope") == "local":
        return append_local_session_event(
            ctx,
            session_id,
            event_type,
            payload,
            actor_identity=_effective_actor_identity(ctx),
            actor_type="tool",
        )
    remote_row = target["remote"]
    return remote_append_session_event(
        remote_row["url"],
        session_id,
        event_type,
        payload,
        repo_name=_normalize_text_value(target.get("repo_name")),
    )


def _start_session_command_tracking(command_tokens: list[str]) -> dict[str, Any] | None:
    if not command_tokens or not _session_autolog_enabled():
        return None
    session_id = _effective_session_id()
    if session_id is None:
        return None
    ctx = _maybe_ctx()
    if ctx is None:
        return None
    try:
        target = _resolve_session_autolog_target(ctx, session_id)
    except (KeyError, RemoteError, ValueError) as exc:
        raise click.ClickException(
            f"Active session tracking for {session_id} could not resolve a writable session target: {exc}"
        ) from exc
    if target is None:
        raise click.ClickException(
            f"Active session tracking for {session_id} could not find a matching local or remote session. "
            "Set AIT_SESSION_LOCAL=1 or AIT_SESSION_REMOTE=<name> when needed."
        )
    raw_command = _session_autolog_raw_command(command_tokens)
    parsed = _extract_ait_command(raw_command)
    if parsed is not None and parsed.get("command_path") == "session turn":
        return None
    command_id = hashlib.sha1(f"{session_id}:{os.getpid()}:{time.time_ns()}:{raw_command}".encode("utf-8")).hexdigest()[:16]
    started_at = utc_now()
    started_ns = time.time_ns()
    payload = _build_session_command_log_payload(
        ctx,
        raw_command,
        command_tokens,
        command_id=command_id,
        phase="started",
        started_at=started_at,
    )
    try:
        _append_session_autolog_event(ctx, target, session_id, "tool.command", payload)
    except (KeyError, RemoteError, ValueError) as exc:
        raise click.ClickException(
            f"Active session tracking must record `{raw_command}` before it runs, but the session event append failed: {exc}"
        ) from exc
    return {
        "ctx": ctx,
        "target": target,
        "session_id": session_id,
        "command_id": command_id,
        "command_tokens": [str(token) for token in command_tokens],
        "raw_command": raw_command,
        "started_at": started_at,
        "started_ns": started_ns,
    }


def _finish_session_command_tracking(tracker: dict[str, Any] | None, returncode: int) -> None:
    if not tracker:
        return
    ctx = tracker["ctx"]
    payload = _build_session_command_log_payload(
        ctx,
        str(tracker["raw_command"]),
        list(tracker["command_tokens"]),
        command_id=str(tracker["command_id"]),
        phase="finished",
        started_at=str(tracker["started_at"]),
        finished_at=utc_now(),
        returncode=int(returncode),
    )
    payload["duration_ms"] = max(int((time.time_ns() - int(tracker["started_ns"])) / 1_000_000), 0)
    try:
        _append_session_autolog_event(ctx, tracker["target"], str(tracker["session_id"]), "tool.result", payload)
    except (KeyError, RemoteError, ValueError):
        return
