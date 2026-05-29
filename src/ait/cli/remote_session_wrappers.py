from __future__ import annotations

from typing import Any

from ..remote_client import (
    advance_task_dag_run as _remote_advance_task_dag_run,
    append_session_event as _remote_append_session_event,
    close_session as _remote_close_session,
    create_session_checkpoint as _remote_create_session_checkpoint,
    create_session_turn as _remote_create_session_turn,
    get_session as _remote_get_session,
    list_session_checkpoints as _remote_list_session_checkpoints,
    list_session_events as _remote_list_session_events,
    resume_session as _remote_resume_session,
)
from ..repo_paths import RepoContext
from ..store_remotes import (
    list_remotes,
)
from .workflow_mode_config import _normalize_text_value


def _maybe_ctx() -> RepoContext | None:
    try:
        return RepoContext.discover()
    except FileNotFoundError:
        return None


def _infer_remote_repo_name(base_url: str, repo_name: str | None = None) -> str | None:
    if repo_name is not None:
        return repo_name
    normalized_base_url = _normalize_text_value(base_url)
    if not normalized_base_url:
        return None
    normalized_base_url = normalized_base_url.rstrip("/")
    ctx = _maybe_ctx()
    if ctx is None:
        return None
    candidate_repo_names: list[str] = []
    for row in list_remotes(ctx):
        candidate_url = _normalize_text_value(row.get("url"))
        if not candidate_url:
            continue
        if candidate_url.rstrip("/") != normalized_base_url:
            continue
        candidate_repo_name = _normalize_text_value(row.get("repo_name"))
        if candidate_repo_name is not None:
            candidate_repo_names.append(candidate_repo_name)
    candidate_repo_names = list(dict.fromkeys(candidate_repo_names))
    if len(candidate_repo_names) != 1:
        return None
    return candidate_repo_names[0]


def remote_get_session(
    base_url: str,
    session_id: str,
    repo_name: str | None = None,
) -> dict[str, Any]:
    return _remote_get_session(base_url, session_id, repo_name=_infer_remote_repo_name(base_url, repo_name))


def remote_append_session_event(
    base_url: str,
    session_id: str,
    event_type: str,
    payload: dict[str, Any] | None = None,
    repo_name: str | None = None,
) -> dict[str, Any]:
    return _remote_append_session_event(
        base_url,
        session_id,
        event_type,
        payload,
        repo_name=_infer_remote_repo_name(base_url, repo_name),
    )


def remote_create_session_turn(
    base_url: str,
    session_id: str,
    *,
    text: str,
    surface: str | None = None,
    title: str | None = None,
    repo_name: str | None = None,
) -> dict[str, Any]:
    return _remote_create_session_turn(
        base_url,
        session_id,
        text=text,
        surface=surface,
        title=title,
        repo_name=_infer_remote_repo_name(base_url, repo_name),
    )


def remote_create_session_checkpoint(
    base_url: str,
    session_id: str,
    summary: str,
    *,
    snapshot_id: str | None = None,
    resume_payload: dict[str, Any] | None = None,
    based_on_sequence: int | None = None,
    checkpoint_id: str | None = None,
    repo_name: str | None = None,
) -> dict[str, Any]:
    return _remote_create_session_checkpoint(
        base_url,
        session_id,
        summary,
        snapshot_id=snapshot_id,
        resume_payload=resume_payload,
        based_on_sequence=based_on_sequence,
        checkpoint_id=checkpoint_id,
        repo_name=_infer_remote_repo_name(base_url, repo_name),
    )


def remote_list_session_checkpoints(
    base_url: str,
    session_id: str,
    repo_name: str | None = None,
) -> list[dict[str, Any]]:
    return _remote_list_session_checkpoints(
        base_url,
        session_id,
        repo_name=_infer_remote_repo_name(base_url, repo_name),
    )


def remote_list_session_events(
    base_url: str,
    session_id: str,
    *,
    after_sequence: int = 0,
    limit: int = 200,
    repo_name: str | None = None,
) -> list[dict[str, Any]]:
    return _remote_list_session_events(
        base_url,
        session_id,
        after_sequence=after_sequence,
        limit=limit,
        repo_name=_infer_remote_repo_name(base_url, repo_name),
    )


def remote_resume_session(
    base_url: str,
    session_id: str,
    *,
    after_sequence: int | None = None,
    limit: int = 200,
    repo_name: str | None = None,
) -> dict[str, Any]:
    return _remote_resume_session(
        base_url,
        session_id,
        after_sequence=after_sequence,
        limit=limit,
        repo_name=_infer_remote_repo_name(base_url, repo_name),
    )


def remote_close_session(
    base_url: str,
    session_id: str,
    status: str = "paused",
    repo_name: str | None = None,
) -> dict[str, Any]:
    return _remote_close_session(
        base_url,
        session_id,
        status=status,
        repo_name=_infer_remote_repo_name(base_url, repo_name),
    )


def remote_advance_task_dag_run(
    base_url: str,
    session_id: str,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
    repo_name: str | None = None,
) -> dict[str, Any]:
    return _remote_advance_task_dag_run(
        base_url,
        session_id,
        graph,
        current_plan_revision_id=current_plan_revision_id,
        repo_name=_infer_remote_repo_name(base_url, repo_name),
    )
