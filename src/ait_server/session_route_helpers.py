from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Mapping

from ait.repo_paths import RepoContext
from ait_chat.session_reply import load_reply_generation_config, payload_text

from . import server_store as _server_store
from .local_repo_seams import (
    get_worktree as local_get_worktree,
    resolve_bound_repo_root,
    resolve_workflow_segment_attachment,
    summarize_workflow_segments,
)
from .server_store import (
    ServerContext,
    create_session_checkpoint,
    get_session,
    get_session_checkpoint,
    list_changes,
    list_plans,
    list_session_checkpoints,
    list_session_events,
    list_tasks,
)


def _latest_session_checkpoint(ctx: ServerContext, session: dict[str, Any]) -> dict[str, Any] | None:
    checkpoint_id = str(session.get("head_checkpoint_id") or "").strip()
    if checkpoint_id:
        try:
            return get_session_checkpoint(ctx, checkpoint_id)
        except KeyError:
            pass
    session_id = str(session.get("session_id") or "").strip()
    if not session_id:
        return None
    checkpoints = list_session_checkpoints(ctx, session_id)
    return checkpoints[0] if checkpoints else None


def _trim_checkpoint_text(value: str | None, *, limit: int) -> str:
    text = str(value or "").strip()
    if len(text) <= limit:
        return text
    return f"{text[: max(limit - 1, 0)].rstrip()}…"


def _is_telegram_linked_session(session: dict[str, Any]) -> bool:
    metadata = session.get("metadata") or {}
    if str(session.get("session_kind") or "").strip() == "telegram_chat":
        return True
    return str(metadata.get("source") or "").strip() == "telegram"


def _reply_generation_repo_name(*, session: dict[str, Any] | None = None, repo_name: str | None = None) -> str:
    resolved = str(repo_name or (session or {}).get("repo_name") or "").strip()
    return resolved or "ait"


def _reply_generation_repo_root(
    *,
    session: dict[str, Any] | None = None,
    repo_name: str | None = None,
    requested_repo_root: str | None = None,
) -> Path:
    metadata = session.get("metadata") if isinstance(session, dict) and isinstance(session.get("metadata"), dict) else {}
    fallback_root = Path(os.environ.get("AIT_REPO_ROOT") or Path.cwd()).expanduser()
    resolved_repo_root = resolve_bound_repo_root(
        _reply_generation_repo_name(session=session, repo_name=repo_name),
        preferred_workspace_root=(metadata or {}).get("workspace_root"),
        preferred_repo_root=requested_repo_root or (metadata or {}).get("repo_root"),
        fallback_root=fallback_root,
    )
    worktree_name = str((session or {}).get("worktree_name") or "").strip()
    if not worktree_name:
        return resolved_repo_root
    try:
        repo_ctx = RepoContext.discover(resolved_repo_root)
        worktree = local_get_worktree(repo_ctx, worktree_name)
        worktree_path = str(worktree.get("path") or "").strip()
        if not worktree_path:
            return resolved_repo_root
        return RepoContext.discover(Path(worktree_path).expanduser()).root.resolve()
    except (FileNotFoundError, KeyError, ValueError):
        return resolved_repo_root


def _reply_generation_config(
    *,
    session: dict[str, Any] | None = None,
    repo_name: str | None = None,
    requested_repo_root: str | None = None,
):
    resolved_repo_name = _reply_generation_repo_name(session=session, repo_name=repo_name)
    resolved_repo_root = _reply_generation_repo_root(
        session=session,
        repo_name=resolved_repo_name,
        requested_repo_root=requested_repo_root,
    )
    reply_env = dict(os.environ)
    reply_env.pop("AIT_REPO_NAME", None)
    reply_env.pop("AIT_TELEGRAM_REPO_NAME", None)
    reply_env.pop("AIT_TELEGRAM_ENV_PATH", None)
    reply_env.pop("AIT_CHAT_ENV_PATH", None)
    return load_reply_generation_config(
        repo_name=resolved_repo_name,
        repo_root=resolved_repo_root,
        env=reply_env,
    )


def _compact_dag_worker_live_turn_guard(session: dict[str, Any] | None) -> str | None:
    if not isinstance(session, dict):
        return None
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    if not isinstance(metadata, dict):
        return None
    session_policy = str(metadata.get("session_policy") or "").strip()
    if session_policy != "task_dag_compact_packet_worker":
        return None
    session_id = str(session.get("session_id") or "").strip() or "<session-id>"
    compact_surface = metadata.get("compact_packet_surface") if isinstance(metadata.get("compact_packet_surface"), dict) else {}
    packet_generation_required = bool(compact_surface.get("packet_generation_required")) if isinstance(compact_surface, dict) else False
    packet_available = bool(metadata.get("packet_available"))
    batch_id = str(metadata.get("batch_id") or "").strip() or None
    if packet_generation_required and not packet_available and batch_id is not None:
        return (
            f"Session {session_id} is a compact DAG batch session scaffold (batch {batch_id}) "
            "with packet generation still pending. Start the worker through `ait plan execute ... "
            "--auto-compact-worker --yes`; server-generated live turns are disabled for this session."
        )
    return (
        f"Session {session_id} is a compact DAG worker session. Compact worker replies are generated "
        "locally and the remote session is reserved for durable lineage/events, so ait-server live turn "
        "routes are disabled for this session."
    )


def _telegram_context_runtime_state(
    ctx: ServerContext,
    session: dict[str, Any],
    *,
    reply_config=None,
) -> dict[str, Any] | None:
    if not _is_telegram_linked_session(session):
        return None
    resolved_reply_config = reply_config or _reply_generation_config(session=session)
    threshold = max(int(resolved_reply_config.telegram_checkpoint_event_threshold or 0), 2)
    summary_event_limit = max(int(resolved_reply_config.telegram_checkpoint_summary_event_limit or 0), 2)
    last_event_sequence = int(session.get("last_event_sequence") or 0)
    checkpoint = _latest_session_checkpoint(ctx, session)
    checkpoint_id = str((checkpoint or {}).get("checkpoint_id") or "").strip() or None
    checkpoint_sequence = int((checkpoint or {}).get("based_on_sequence") or 0)
    delta_event_count = max(last_event_sequence - checkpoint_sequence, 0)
    if checkpoint is None:
        freshness = "missing"
        reply_context_mode = "recent_tail"
        refresh_recommended = False
    else:
        freshness = "fresh" if delta_event_count < threshold else "stale"
        reply_context_mode = "checkpoint_delta"
        refresh_recommended = delta_event_count >= threshold
    return {
        "reply_context_mode": reply_context_mode,
        "has_checkpoint": checkpoint is not None,
        "checkpoint_id": checkpoint_id,
        "checkpoint_created_at": (checkpoint or {}).get("created_at"),
        "checkpoint_based_on_sequence": checkpoint_sequence,
        "last_event_sequence": last_event_sequence,
        "delta_event_count": delta_event_count,
        "checkpoint_event_threshold": threshold,
        "checkpoint_summary_event_limit": summary_event_limit,
        "events_until_refresh": max(threshold - delta_event_count, 0),
        "checkpoint_freshness": freshness,
        "refresh_recommended": refresh_recommended,
    }


def _session_with_telegram_runtime_state(
    ctx: ServerContext,
    session: dict[str, Any],
    *,
    reply_config=None,
) -> dict[str, Any]:
    runtime_state = _telegram_context_runtime_state(ctx, session, reply_config=reply_config)
    if runtime_state is None:
        return session
    enriched = dict(session)
    enriched["telegram_context_runtime"] = runtime_state
    return enriched


def _normalized_telegram_message_ids(
    values: list[int] | None,
    *,
    telegram_message_id: int | None,
) -> list[int]:
    normalized = [int(value) for value in (values or []) if int(value) > 0]
    if telegram_message_id is not None and int(telegram_message_id) > 0 and int(telegram_message_id) not in normalized:
        normalized.append(int(telegram_message_id))
    if not normalized and telegram_message_id is not None and int(telegram_message_id) > 0:
        normalized = [int(telegram_message_id)]
    return normalized


def _matching_telegram_user_event(
    event: Mapping[str, Any],
    *,
    text: str,
    chat_id: str,
    telegram_message_id: int | None,
    telegram_message_ids: list[int],
    transport_envelope: Mapping[str, Any] | None,
) -> bool:
    if str(event.get("event_type") or "").strip() != "telegram.user_message":
        return False
    payload = event.get("payload") if isinstance(event.get("payload"), Mapping) else {}
    if str(payload.get("telegram_chat_id") or "") != chat_id:
        return False
    requested_event_id = str((transport_envelope or {}).get("event_id") or "").strip()
    payload_envelope = payload.get("transport_envelope") if isinstance(payload.get("transport_envelope"), Mapping) else {}
    payload_event_id = str(payload.get("event_id") or "").strip()
    payload_envelope_event_id = str(payload_envelope.get("event_id") or "").strip()
    if requested_event_id and requested_event_id in {payload_event_id, payload_envelope_event_id}:
        return True
    if str(payload.get("text") or "").strip() != text:
        return False
    payload_message_ids = _normalized_telegram_message_ids(
        payload.get("telegram_message_ids") if isinstance(payload.get("telegram_message_ids"), list) else None,
        telegram_message_id=payload.get("telegram_message_id"),
    )
    if telegram_message_ids and payload_message_ids == telegram_message_ids:
        return True
    if telegram_message_id is not None and int(payload.get("telegram_message_id") or 0) == int(telegram_message_id):
        return True
    return False


def _assistant_reply_for_telegram_user_event(
    *,
    chat_id: str,
    user_event: Mapping[str, Any],
    events: list[dict[str, Any]],
) -> dict[str, Any] | None:
    user_sequence = int(user_event.get("sequence") or 0)
    if user_sequence <= 0:
        return None
    for event in events:
        if str(event.get("event_type") or "").strip() != "assistant.reply":
            continue
        payload = event.get("payload") if isinstance(event.get("payload"), Mapping) else {}
        if str(payload.get("telegram_chat_id") or "") != chat_id:
            continue
        if int(payload.get("reply_to_sequence") or 0) == user_sequence:
            return dict(event)
    return None


def _render_turn_analysis_footer(turn_analysis: dict[str, Any] | None) -> str:
    if not isinstance(turn_analysis, dict):
        return ""
    command_count = int(turn_analysis.get("command_count") or 0)
    optimization_summary = str(turn_analysis.get("optimization_summary") or "").strip()
    if command_count <= 0 and not optimization_summary:
        return ""
    parts = [f"ran {command_count} commands"]
    if optimization_summary:
        parts.append(optimization_summary)
    return "[turn analysis] " + " · ".join(parts)


def _reply_text_with_turn_analysis(
    text: str,
    *,
    turn_analysis: dict[str, Any] | None,
    append_turn_analysis: bool,
) -> str:
    reply_text = str(text or "").strip()
    if not append_turn_analysis:
        return reply_text
    footer = _render_turn_analysis_footer(turn_analysis)
    if not footer:
        return reply_text
    if not reply_text:
        return footer
    return f"{reply_text}\n\n{footer}"


def _session_workflow_segmentation_summary(
    ctx: ServerContext,
    session: dict[str, Any],
    *,
    limit: int = 500,
) -> dict[str, Any]:
    session_id = str(session.get("session_id") or "")
    events = list_session_events(ctx, session_id, after_sequence=0, limit=max(int(limit), 1))
    repo_name = str(session.get("repo_name") or "").strip()
    plans = list_plans(ctx, repo_name) if repo_name else []
    tasks = list_tasks(ctx, repo_name) if repo_name else []
    changes = list_changes(ctx, repo_name) if repo_name else []
    segments = summarize_workflow_segments(events, session=session)
    for segment in segments:
        segment["attachment_resolution"] = resolve_workflow_segment_attachment(
            segment,
            plans=plans,
            tasks=tasks,
            changes=changes,
        )
    return {
        "session_id": session_id,
        "segment_count": len(segments),
        "segments": segments,
        "latest_segment": segments[-1] if segments else None,
    }


def _telegram_turn_retry_response(
    ctx: ServerContext,
    session: dict[str, Any],
    *,
    user_event: dict[str, Any],
    assistant_event: dict[str, Any],
    reply_config: Any,
) -> dict[str, Any]:
    assistant_payload = assistant_event.get("payload") if isinstance(assistant_event.get("payload"), Mapping) else {}
    assistant_text = str(assistant_payload.get("text") or "").strip()
    turn_analysis = assistant_payload.get("turn_analysis") if isinstance(assistant_payload.get("turn_analysis"), dict) else {}
    reply_text = _reply_text_with_turn_analysis(
        assistant_text,
        turn_analysis=turn_analysis,
        append_turn_analysis=getattr(reply_config, "telegram_append_turn_analysis", False),
    )
    refreshed_session = get_session(ctx, str(session.get("session_id") or ""))
    checkpoint = _latest_session_checkpoint(ctx, refreshed_session)
    assistant_sequence = int(assistant_event.get("sequence") or 0)
    if int((checkpoint or {}).get("based_on_sequence") or 0) < assistant_sequence:
        checkpoint = None
    return {
        "ok": True,
        "session_id": str(session.get("session_id") or ""),
        "user_event": user_event,
        "assistant_event": assistant_event,
        "reply_text": reply_text,
        "turn_analysis": turn_analysis,
        "workflow_segmentation": _session_workflow_segmentation_summary(ctx, refreshed_session),
        "checkpoint": checkpoint,
        "telegram_context_runtime": _telegram_context_runtime_state(
            ctx,
            refreshed_session,
            reply_config=reply_config,
        ),
    }


def _build_telegram_checkpoint_materials(
    session: dict[str, Any],
    *,
    user_event: dict[str, Any],
    assistant_event: dict[str, Any],
    previous_checkpoint: dict[str, Any] | None,
    recent_events: list[dict[str, Any]],
    reason: str,
) -> tuple[str, dict[str, Any]]:
    user_payload = user_event.get("payload") or {}
    assistant_payload = assistant_event.get("payload") or {}
    previous_sequence = int((previous_checkpoint or {}).get("based_on_sequence") or 0)
    assistant_sequence = int(assistant_event.get("sequence") or 0)
    user_sequence = int(user_event.get("sequence") or 0)
    delta_event_count = max(assistant_sequence - previous_sequence, 0)
    latest_user_request = payload_text(user_payload)
    latest_assistant_reply = payload_text(assistant_payload)

    recent_user_requests = [
        payload_text((event.get("payload") or {}))
        for event in recent_events
        if str(event.get("event_type") or "") == "telegram.user_message"
    ]
    recent_external_notes = [
        payload_text((event.get("payload") or {}))
        for event in recent_events
        if str(event.get("event_type") or "") == "web.note"
    ]
    recent_user_requests = [text for text in recent_user_requests if text and text != "(no details)"][-3:]
    recent_external_notes = [text for text in recent_external_notes if text and text != "(no details)"][-2:]

    reason_text = (
        "Initial Telegram checkpoint after the first successful shared-session reply."
        if reason == "initial_turn"
        else f"Telegram checkpoint refreshed after {delta_event_count} post-checkpoint events."
    )
    summary_parts = [
        reason_text,
        f"Latest user request: {_trim_checkpoint_text(latest_user_request, limit=160)}.",
        f"Latest assistant guidance: {_trim_checkpoint_text(latest_assistant_reply, limit=200)}.",
    ]
    if recent_external_notes:
        summary_parts.append(f"Recent web note: {_trim_checkpoint_text(recent_external_notes[-1], limit=160)}.")
    summary = " ".join(part for part in summary_parts if part)

    resume_payload = {
        "source": "telegram",
        "objective": latest_user_request,
        "latest_user_request": latest_user_request,
        "latest_assistant_reply": latest_assistant_reply,
        "recent_user_requests": recent_user_requests,
        "recent_external_notes": recent_external_notes,
        "context": {
            "session_kind": str(session.get("session_kind") or ""),
            "chat_id": str(user_payload.get("telegram_chat_id") or ""),
            "chat_title": user_payload.get("telegram_chat_title"),
            "chat_type": user_payload.get("telegram_chat_type"),
            "last_event_sequence": assistant_sequence,
            "last_user_sequence": user_sequence,
            "last_assistant_sequence": assistant_sequence,
            "events_since_previous_checkpoint": delta_event_count,
            "logical_turn_message_count": int(user_payload.get("logical_turn_message_count") or 0),
            "checkpoint_reason": reason,
        },
    }
    return summary, resume_payload


def _maybe_refresh_telegram_checkpoint(
    ctx: ServerContext,
    *,
    session: dict[str, Any],
    user_event: dict[str, Any],
    assistant_event: dict[str, Any],
    reply_config,
) -> dict[str, Any] | None:
    if not _is_telegram_linked_session(session):
        return None
    session_id = str(session.get("session_id") or "")
    assistant_sequence = int(assistant_event.get("sequence") or 0)
    if not session_id or assistant_sequence <= 0:
        return None

    previous_checkpoint = next(iter(list_session_checkpoints(ctx, session_id)), None)
    previous_sequence = int((previous_checkpoint or {}).get("based_on_sequence") or 0)
    delta_event_count = max(assistant_sequence - previous_sequence, 0)
    threshold = max(int(reply_config.telegram_checkpoint_event_threshold or 0), 2)

    if previous_checkpoint is None:
        reason = "initial_turn"
    elif delta_event_count >= threshold:
        reason = "event_tail_threshold"
    else:
        return None

    recent_limit = max(int(reply_config.telegram_checkpoint_summary_event_limit or 0), threshold)
    recent_events = list_session_events(
        ctx,
        session_id,
        after_sequence=max(assistant_sequence - recent_limit, 0),
        limit=recent_limit,
    )
    summary, resume_payload = _build_telegram_checkpoint_materials(
        session,
        user_event=user_event,
        assistant_event=assistant_event,
        previous_checkpoint=previous_checkpoint,
        recent_events=recent_events,
        reason=reason,
    )
    return create_session_checkpoint(
        ctx,
        session_id,
        summary,
        resume_payload=resume_payload,
        based_on_sequence=assistant_sequence,
        actor_identity="ait-server",
        actor_type="system_worker",
    )


def _reply_generation_events(
    ctx: ServerContext,
    session_id: str,
    *,
    user_event: dict[str, Any],
    checkpoint_before_reply: dict[str, Any] | None,
    reply_config,
) -> list[dict[str, Any]]:
    context_window = max(int(reply_config.history_limit or 0) * 4, 40)
    if checkpoint_before_reply is not None:
        checkpoint_sequence = int(checkpoint_before_reply.get("based_on_sequence") or 0)
        after_sequence = max(checkpoint_sequence, int(user_event.get("sequence") or 0) - context_window)
    else:
        after_sequence = max(int(user_event.get("sequence") or 0) - context_window, 0)
    return list_session_events(ctx, session_id, after_sequence=after_sequence, limit=context_window)


def _resolve_session_for_repo(
    ctx: ServerContext,
    repo_name: str,
    session_id: str,
) -> dict[str, Any]:
    resolver = getattr(_server_store, "get_session_for_repo", None)
    if callable(resolver):
        try:
            return resolver(ctx, repo_name=repo_name, session_ref=session_id)
        except TypeError:
            try:
                return resolver(ctx, session_ref=session_id, repo_name=repo_name)
            except TypeError:
                return resolver(ctx, repo_name, session_id)

    session = get_session(ctx, session_id)
    if str(session.get("repo_name") or "").strip() != str(repo_name).strip():
        raise KeyError(f"Session {session_id} not found for repository {repo_name}")
    return session


__all__ = [
    "_assistant_reply_for_telegram_user_event",
    "_build_telegram_checkpoint_materials",
    "_compact_dag_worker_live_turn_guard",
    "_is_telegram_linked_session",
    "_latest_session_checkpoint",
    "_matching_telegram_user_event",
    "_maybe_refresh_telegram_checkpoint",
    "_normalized_telegram_message_ids",
    "_render_turn_analysis_footer",
    "_reply_generation_config",
    "_reply_generation_events",
    "_reply_generation_repo_name",
    "_reply_generation_repo_root",
    "_reply_text_with_turn_analysis",
    "_resolve_session_for_repo",
    "_session_with_telegram_runtime_state",
    "_session_workflow_segmentation_summary",
    "_telegram_context_runtime_state",
    "_telegram_turn_retry_response",
    "_trim_checkpoint_text",
]
