from __future__ import annotations

from typing import Any, Mapping

from ait_protocol.common import utc_now

from ..remote_client import (
    RemoteError,
    append_session_event as _remote_append_session_event,
    create_session as remote_create_session,
    ensure_task_tracking_session as remote_ensure_task_tracking_session,
    get_change as remote_get_change,
    get_session as _remote_get_session,
    get_task as remote_get_task,
    list_sessions as remote_list_sessions,
    submit_land as remote_submit_land,
)
from ..store import RepoContext, current_line, get_local_change, get_local_task, get_worktree as local_get_worktree, load_config
from ..workflow_conversation import infer_workflow_context
from .remote_repository_defaults import _remote_tuple
from .runtime_defaults import _effective_author_mode, _effective_model_name
from .task_tracking_bindings import _default_line_name, _task_tracking_enabled, _task_worktree_repo_ctx, _tracked_session_binding
from .task_worktree_resolution import _session_bound_worktree
from .task_worktree_runtime import _active_root_worktree, _bound_task_id_for_worktree
from .workflow_mode_config import _normalize_text_value


WORKFLOW_BOUNDARY_SESSION_KINDS = frozenset({"agent_run", "task_run"})
WORKFLOW_BOUNDARY_REUSABLE_SESSION_STATUSES = frozenset({"active", "paused"})
WORKFLOW_BOUNDARY_ACTIVE_TASK_STATUSES = frozenset({"active", "planned"})
def _workflow_boundary_status_priority(status: str | None) -> tuple[int, str]:
    normalized = _normalize_text_value(status) or ""
    priority = {
        "active": 0,
        "paused": 1,
        "completed": 2,
        "canceled": 3,
    }.get(normalized, 99)
    return priority, normalized


def _latest_remote_session_matching(rows: list[dict[str, Any]], predicate) -> dict[str, Any] | None:
    candidates = [row for row in rows if predicate(row)]
    if not candidates:
        return None
    candidates.sort(
        key=lambda row: (
            _workflow_boundary_status_priority(str(row.get("status") or "")),
            str(row.get("updated_at") or row.get("created_at") or row.get("session_id") or ""),
        )
    )
    return candidates[0]


def _workflow_boundary_current_worktree(ctx: RepoContext) -> dict[str, Any] | None:
    try:
        if ctx.is_worktree:
            return local_get_worktree(ctx)
        return _active_root_worktree(ctx)
    except (KeyError, ValueError):
        return None


def _remote_task_id_for_local_binding(ctx: RepoContext, task_id: str | None) -> str | None:
    resolved_task_id = _normalize_text_value(task_id)
    if resolved_task_id is None:
        return None
    try:
        local_task = get_local_task(_task_worktree_repo_ctx(ctx), resolved_task_id)
    except KeyError:
        return resolved_task_id
    published_task_id = _normalize_text_value(local_task.get("published_task_id"))
    if published_task_id is not None:
        return published_task_id
    if _normalize_text_value(local_task.get("publication_state")) == "published":
        return resolved_task_id
    return None


def _workflow_boundary_resolve_task_id(
    ctx: RepoContext,
    *,
    local: bool,
    remote_name: str | None,
    repo_name_override: str | None = None,
    task_id: str | None = None,
    change_id: str | None = None,
) -> str | None:
    resolved_task_id = _normalize_text_value(task_id)
    if resolved_task_id is not None:
        return resolved_task_id
    current_worktree = _workflow_boundary_current_worktree(ctx)
    resolved_task_id = _bound_task_id_for_worktree(ctx, current_worktree)
    if resolved_task_id is not None:
        if local:
            return resolved_task_id
        remote_task_id = _remote_task_id_for_local_binding(ctx, resolved_task_id)
        if remote_task_id is not None:
            return remote_task_id
    resolved_change_id = _normalize_text_value(change_id)
    if resolved_change_id is None:
        return None
    try:
        if local:
            change = get_local_change(_task_worktree_repo_ctx(ctx), resolved_change_id)
        else:
            remote_row, default_repo_name = _remote_tuple(ctx, remote_name)
            repo_name = _normalize_text_value(repo_name_override) or default_repo_name
            change = remote_get_change(remote_row["url"], resolved_change_id, repo_name=repo_name)
    except (KeyError, RemoteError, ValueError):
        return None
    return _normalize_text_value(change.get("task_id"))


def _remote_task_tracking_session_row(
    ctx: RepoContext,
    *,
    remote_row: dict[str, Any],
    repo_name: str,
    task_id: str,
) -> dict[str, Any] | None:
    tracked_binding = _tracked_session_binding(ctx)
    tracked_session_id = _normalize_text_value((tracked_binding or {}).get("session_id"))
    tracked_remote_name = _normalize_text_value((tracked_binding or {}).get("remote_name"))
    tracked_task_id = _normalize_text_value((tracked_binding or {}).get("task_id"))
    remote_name = _normalize_text_value(remote_row.get("name"))
    if tracked_session_id and tracked_task_id == task_id and tracked_remote_name == remote_name:
        try:
            task = remote_get_task(remote_row["url"], task_id, repo_name=repo_name)
            if _normalize_text_value(task.get("status")) in WORKFLOW_BOUNDARY_ACTIVE_TASK_STATUSES:
                session = _remote_get_session(remote_row["url"], tracked_session_id, repo_name=repo_name)
                if (
                    _normalize_text_value(session.get("task_id")) == task_id
                    and _normalize_text_value(session.get("status")) in WORKFLOW_BOUNDARY_REUSABLE_SESSION_STATUSES
                ):
                    return session
        except (KeyError, RemoteError, ValueError):
            pass
    sessions = remote_list_sessions(remote_row["url"], repo_name)
    return _latest_remote_session_matching(
        sessions,
        lambda row: _normalize_text_value(row.get("task_id")) == task_id
        and _normalize_text_value(row.get("status")) in WORKFLOW_BOUNDARY_REUSABLE_SESSION_STATUSES,
    )


def _workflow_boundary_generic_session(
    ctx: RepoContext,
    *,
    remote_row: dict[str, Any],
    repo_name: str,
    source_surface: str,
) -> dict[str, Any]:
    worktree = _workflow_boundary_current_worktree(ctx)
    workspace_root = (
        _normalize_text_value((worktree or {}).get("path"))
        or _normalize_text_value((worktree or {}).get("workspace_root"))
        or str(ctx.root.resolve())
    )
    worktree_name = (
        _normalize_text_value((worktree or {}).get("name"))
        or _normalize_text_value(load_config(ctx).get("worktree_name"))
    )
    line_name = (
        _normalize_text_value((worktree or {}).get("current_line"))
        or _normalize_text_value((worktree or {}).get("registered_line_name"))
        or _normalize_text_value(current_line(ctx))
        or _default_line_name(ctx)
    )
    sessions = remote_list_sessions(remote_row["url"], repo_name)
    candidate = _latest_remote_session_matching(
        sessions,
        lambda row: _normalize_text_value(row.get("status")) in WORKFLOW_BOUNDARY_REUSABLE_SESSION_STATUSES
        and _normalize_text_value(row.get("session_kind")) in WORKFLOW_BOUNDARY_SESSION_KINDS
        and (
            (
                worktree_name is not None
                and _normalize_text_value(row.get("worktree_name")) == worktree_name
            )
            or _normalize_text_value((row.get("metadata") or {}).get("workspace_root")) == workspace_root
        ),
    )
    if candidate is not None:
        return candidate
    title = "Workflow boundary session"
    if line_name:
        title = f"{title} · {line_name}"
    metadata = _apply_session_workspace_metadata(
        ctx,
        {
            "tracking_policy": "workflow_boundary_session",
            "workflow_boundary_session": True,
            "source_surface": source_surface,
        },
        worktree=worktree,
    )
    return remote_create_session(
        remote_row["url"],
        repo_name,
        "agent_run",
        title=title,
        line_name=line_name,
        worktree_name=worktree_name,
        model_name=_effective_model_name(ctx),
        metadata=metadata,
    )


def _resolve_remote_workflow_boundary_session(
    ctx: RepoContext,
    *,
    remote_name: str | None,
    repo_name_override: str | None = None,
    command_name: str,
    source_surface: str,
    session_id: str | None = None,
    task_id: str | None = None,
    change_id: str | None = None,
) -> dict[str, Any]:
    remote_row, default_repo_name = _remote_tuple(ctx, remote_name)
    repo_name = _normalize_text_value(repo_name_override) or default_repo_name
    explicit_session_id = _normalize_text_value(session_id)
    if explicit_session_id is not None:
        session = _remote_get_session(remote_row["url"], explicit_session_id, repo_name=repo_name)
        return {
            "remote_row": remote_row,
            "remote_name": remote_row.get("name") or remote_name,
            "repo_name": repo_name,
            "session": session,
            "session_id": explicit_session_id,
            "source": "explicit_session",
        }

    resolved_task_id = _workflow_boundary_resolve_task_id(
        ctx,
        local=False,
        remote_name=remote_name,
        repo_name_override=repo_name,
        task_id=task_id,
        change_id=change_id,
    )
    if resolved_task_id is not None:
        task = remote_get_task(remote_row["url"], resolved_task_id, repo_name=repo_name)
        task_status = _normalize_text_value(task.get("status")) or "active"
        session = _remote_task_tracking_session_row(
            ctx,
            remote_row=remote_row,
            repo_name=repo_name,
            task_id=resolved_task_id,
        )
        if session is None and task_status in WORKFLOW_BOUNDARY_ACTIVE_TASK_STATUSES:
            session = remote_ensure_task_tracking_session(
                remote_row["url"],
                repo_name,
                resolved_task_id,
                tracking_session=_remote_task_tracking_session_seed(
                    ctx,
                    title=str(task.get("title") or resolved_task_id),
                    intent=str(task.get("intent") or command_name),
                    remote_name=remote_name,
                    worktree=_workflow_boundary_current_worktree(ctx),
                    source_surface=source_surface,
                ),
            )
        if session is not None and _normalize_text_value(session.get("status")) in WORKFLOW_BOUNDARY_REUSABLE_SESSION_STATUSES:
            return {
                "remote_row": remote_row,
                "remote_name": remote_row.get("name") or remote_name,
                "repo_name": repo_name,
                "session": session,
                "session_id": str(session["session_id"]),
                "source": "task_tracking_session",
                "task_id": resolved_task_id,
            }

    tracked_binding = _tracked_session_binding(ctx)
    tracked_session_id = _normalize_text_value((tracked_binding or {}).get("session_id"))
    tracked_remote_name = _normalize_text_value((tracked_binding or {}).get("remote_name"))
    if tracked_session_id is not None and tracked_remote_name == _normalize_text_value(remote_row.get("name")):
        tracked_task_id = _normalize_text_value((tracked_binding or {}).get("task_id"))
        try:
            if tracked_task_id is None:
                session = _remote_get_session(remote_row["url"], tracked_session_id, repo_name=repo_name)
                return {
                    "remote_row": remote_row,
                    "remote_name": remote_row.get("name") or remote_name,
                    "repo_name": repo_name,
                    "session": session,
                    "session_id": tracked_session_id,
                    "source": "tracked_session",
                }
            task = remote_get_task(remote_row["url"], tracked_task_id, repo_name=repo_name)
            if _normalize_text_value(task.get("status")) in WORKFLOW_BOUNDARY_ACTIVE_TASK_STATUSES:
                session = _remote_get_session(remote_row["url"], tracked_session_id, repo_name=repo_name)
                if _normalize_text_value(session.get("status")) in WORKFLOW_BOUNDARY_REUSABLE_SESSION_STATUSES:
                    return {
                        "remote_row": remote_row,
                        "remote_name": remote_row.get("name") or remote_name,
                        "repo_name": repo_name,
                        "session": session,
                        "session_id": tracked_session_id,
                        "source": "tracked_session",
                        "task_id": tracked_task_id,
                    }
        except (KeyError, RemoteError, ValueError):
            pass

    session = _workflow_boundary_generic_session(
        ctx,
        remote_row=remote_row,
        repo_name=repo_name,
        source_surface=source_surface,
    )
    return {
        "remote_row": remote_row,
        "remote_name": remote_row.get("name") or remote_name,
        "repo_name": repo_name,
        "session": session,
        "session_id": str(session["session_id"]),
        "source": "workspace_boundary_session",
    }


def _workflow_boundary_session_payload(
    session: Mapping[str, Any],
    *,
    attachment_hints: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    payload = dict(session)
    metadata = dict(session.get("metadata") or {}) if isinstance(session.get("metadata"), Mapping) else {}
    for key, value in (attachment_hints or {}).items():
        normalized = _normalize_text_value(value)
        if normalized is None:
            continue
        if key in {"plan_id", "planning_session_id", "task_id", "change_id"}:
            payload[key] = normalized
        else:
            metadata[key] = normalized
    if metadata:
        payload["metadata"] = metadata
    return payload


def _append_remote_workflow_boundary_event(
    ctx: RepoContext,
    *,
    session_target: dict[str, Any],
    command_text: str,
    boundary_kind: str,
    attachment_hints: Mapping[str, Any] | None = None,
    extra_payload: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    remote_row = session_target["remote_row"]
    repo_name = session_target["repo_name"]
    session_id = str(session_target["session_id"])
    synthetic_session = _workflow_boundary_session_payload(
        session_target["session"],
        attachment_hints=attachment_hints,
    )
    workflow_context = infer_workflow_context(text=command_text, session=synthetic_session)
    payload = {
        "source": "cli",
        "surface_title": f"ait {boundary_kind}",
        "text": command_text,
        "command": command_text,
        "boundary_kind": boundary_kind,
        "captured_at": str(utc_now()),
        "workflow_context": workflow_context,
        "session_resolution": session_target.get("source"),
    }
    if extra_payload:
        payload.update(dict(extra_payload))
    return _remote_append_session_event(
        remote_row["url"],
        session_id,
        "workflow.boundary",
        payload,
        repo_name=repo_name,
    )


def _select_source_session_id(value: str | None, fallback: str | None) -> str | None:
    return _normalize_text_value(value) or _normalize_text_value(fallback)


def _submit_remote_land_with_boundary_event(
    ctx: RepoContext,
    *,
    remote_name: str | None,
    repo_name_override: str | None = None,
    change_id: str,
    patchset_id: str | None,
    target_line: str,
    mode: str,
    session_id: str | None = None,
) -> dict[str, Any]:
    session_target = _resolve_remote_workflow_boundary_session(
        ctx,
        remote_name=remote_name,
        repo_name_override=repo_name_override,
        command_name="land submit",
        source_surface="cli.land.submit",
        session_id=session_id,
        change_id=change_id,
    )
    command_text = f"ait land submit {change_id}"
    if _normalize_text_value(patchset_id) is not None:
        command_text += f" --patchset {patchset_id}"
    command_text += f" --target {target_line} --mode {mode}"
    _append_remote_workflow_boundary_event(
        ctx,
        session_target=session_target,
        command_text=command_text,
        boundary_kind="land_submit",
        attachment_hints={
            "task_id": session_target.get("task_id"),
            "change_id": change_id,
            "patchset_id": patchset_id,
        },
        extra_payload={
            "target_line": target_line,
            "mode": mode,
        },
    )
    return remote_submit_land(
        session_target["remote_row"]["url"],
        change_id,
        patchset_id,
        target_line,
        mode,
        repo_name=session_target["repo_name"],
    )



def _apply_session_workspace_metadata(
    ctx: RepoContext,
    metadata: dict[str, Any],
    *,
    worktree: dict[str, Any] | None,
) -> dict[str, Any]:
    enriched = dict(metadata)
    if worktree is not None:
        repo_root = _normalize_text_value(worktree.get("repo_root")) or str(ctx.repo_root)
        workspace_root = _normalize_text_value(worktree.get("path")) or _normalize_text_value(worktree.get("workspace_root"))
        enriched.setdefault("repo_root", repo_root)
        if workspace_root:
            enriched.setdefault("workspace_root", workspace_root)
        return enriched
    if ctx.is_worktree:
        enriched.setdefault("repo_root", str(ctx.repo_root))
        enriched.setdefault("workspace_root", str(ctx.root))
    return enriched


def _remote_task_tracking_session_seed(
    ctx: RepoContext,
    *,
    title: str,
    intent: str,
    remote_name: str | None,
    worktree: dict[str, Any] | None = None,
    source_surface: str,
) -> dict[str, Any]:
    bound_worktree = worktree or _session_bound_worktree(ctx, local=False, remote_name=remote_name)
    resolved_line = (
        _normalize_text_value((bound_worktree or {}).get("current_line"))
        or _normalize_text_value((bound_worktree or {}).get("registered_line_name"))
        or current_line(ctx)
    )
    resolved_model = _effective_model_name(ctx)
    worktree_name = _normalize_text_value((bound_worktree or {}).get("name")) or _normalize_text_value(load_config(ctx).get("worktree_name"))
    tracking_policy = "task_tracking" if _task_tracking_enabled(ctx) else "server_task_session"
    metadata = _apply_session_workspace_metadata(
        ctx,
        {
            "author_mode": _effective_author_mode(ctx),
            "objective": intent,
            "tracking_policy": tracking_policy,
            "source_surface": source_surface,
            "task_title": title,
        },
        worktree=bound_worktree,
    )
    payload: dict[str, Any] = {"metadata": metadata}
    if resolved_line is not None:
        payload["line_name"] = resolved_line
    if worktree_name is not None:
        payload["worktree_name"] = worktree_name
    if resolved_model is not None:
        payload["model_name"] = resolved_model
    return payload
