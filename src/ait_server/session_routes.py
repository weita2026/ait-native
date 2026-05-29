from __future__ import annotations

import logging
import sys
from typing import Any, Callable

from fastapi import FastAPI, Request

from ait.session_list_contract import session_list_summary_rows
from ait_chat.session_reply import ReplyGenerationError, generate_session_reply as _generate_session_reply_impl

from .agent_transport_runtime import build_transport_reply_envelope, utc_now_iso
from .local_repo_seams import infer_workflow_context
from .read_models import task_dag_readiness
from .read_routes import TaskDagReadinessRequest
from .route_request_models import (
    SessionCheckpointCreate,
    SessionCloseRequest,
    SessionCreate,
    SessionEventAppend,
    SessionResumeRequest,
    SessionTurnRequest,
    TelegramTurnRequest,
)
from .server_auth import AuthzError, actor_from_request, ensure_repo_action
from .server_store import (
    ServerContext,
    append_session_event,
    close_session,
    create_session,
    create_session_checkpoint,
    get_session,
    get_session_checkpoint,
    list_session_checkpoints,
    list_session_events,
    list_sessions,
    resume_session,
)
from .session_route_helpers import (
    _assistant_reply_for_telegram_user_event,
    _compact_dag_worker_live_turn_guard,
    _latest_session_checkpoint,
    _matching_telegram_user_event,
    _maybe_refresh_telegram_checkpoint,
    _normalized_telegram_message_ids,
    _reply_generation_config,
    _reply_generation_events,
    _reply_generation_repo_root,
    _reply_text_with_turn_analysis,
    _resolve_session_for_repo,
    _session_with_telegram_runtime_state,
    _session_workflow_segmentation_summary,
    _telegram_context_runtime_state,
    _telegram_turn_retry_response,
)
from .session_transport_payloads import (
    build_session_assistant_reply_payload,
    build_session_user_message_payload,
    build_telegram_assistant_reply_payload,
    build_telegram_user_message_payload,
)
from .task_dag_route_helpers import (
    _reply_text_with_task_dag_progress as _reply_text_with_task_dag_progress_helper,
    _safe_task_dag_progress_summary_for_turn as _safe_task_dag_progress_summary_for_turn_impl,
    _task_dag_progress_summary_for_turn_impl,
)
from .task_graph_runs import advance_task_dag_run


LOGGER = logging.getLogger(__name__)
_DEFAULT_TASK_DAG_TURN_PROGRESS_LOCK_TIMEOUT_MS = 750
_DEFAULT_TASK_DAG_TURN_PROGRESS_STATEMENT_TIMEOUT_MS = 2500

ErrorBuilder = Callable[[Exception], Exception]
CtxGetter = Callable[[], ServerContext]
AdminCacheClear = Callable[[], None]
LiveTurnStart = Callable[..., str]
LiveTurnFinish = Callable[..., Any]


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait_server.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)


def _generate_session_reply(*args: Any, **kwargs: Any) -> Any:
    return _app_override("generate_session_reply", _generate_session_reply_impl)(*args, **kwargs)


def _reply_text_with_task_dag_progress(text: str, summary: dict[str, Any] | None) -> str:
    return _reply_text_with_task_dag_progress_helper(text, summary)


def _default_task_dag_progress_summary_for_turn(
    ctx: ServerContext,
    session: dict[str, Any],
    *,
    text: str,
    surface_title: str | None,
) -> dict[str, Any] | None:
    return _task_dag_progress_summary_for_turn_impl(
        ctx,
        session,
        text=text,
        surface_title=surface_title,
        resolve_repo_root=_reply_generation_repo_root,
        task_dag_readiness_reader=task_dag_readiness,
    )


def _task_dag_progress_summary_for_turn(
    ctx: ServerContext,
    session: dict[str, Any],
    *,
    text: str,
    surface_title: str | None,
) -> dict[str, Any] | None:
    return _app_override(
        "_task_dag_progress_summary_for_turn",
        _default_task_dag_progress_summary_for_turn,
    )(
        ctx,
        session,
        text=text,
        surface_title=surface_title,
    )


def _safe_task_dag_progress_summary_for_turn(
    ctx: ServerContext,
    session: dict[str, Any],
    *,
    text: str,
    surface_title: str | None,
) -> dict[str, Any] | None:
    try:
        return _safe_task_dag_progress_summary_for_turn_impl(
            ctx,
            session,
            text=text,
            surface_title=surface_title,
            default_lock_timeout_ms=_DEFAULT_TASK_DAG_TURN_PROGRESS_LOCK_TIMEOUT_MS,
            default_statement_timeout_ms=_DEFAULT_TASK_DAG_TURN_PROGRESS_STATEMENT_TIMEOUT_MS,
            progress_reader=_task_dag_progress_summary_for_turn,
        )
    except Exception as exc:
        LOGGER.warning(
            "task DAG progress enrichment skipped for session %s: %s: %s",
            session.get("session_id"),
            type(exc).__name__,
            exc,
        )
        return None


def register_session_routes(
    app: FastAPI,
    *,
    ctx_getter: CtxGetter,
    not_found: ErrorBuilder,
    bad_request: ErrorBuilder,
    conflict: ErrorBuilder,
    clear_admin_response_cache: AdminCacheClear,
    start_live_turn: LiveTurnStart,
    finish_live_turn: LiveTurnFinish,
) -> None:
    @app.post("/v1/native/repositories/{repo_name}/sessions")
    def api_create_session(repo_name: str, req: SessionCreate, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            session = create_session(
                ctx,
                repo_name,
                req.session_kind,
                task_id=req.task_id,
                change_id=req.change_id,
                title=req.title,
                line_name=req.line_name,
                worktree_name=req.worktree_name,
                model_name=req.model_name,
                metadata=req.metadata,
                session_id=req.session_id,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
            return _session_with_telegram_runtime_state(ctx, session)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise conflict(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/sessions")
    def api_list_sessions(
        repo_name: str,
        request: Request,
        status: str | None = None,
        full: bool = False,
    ) -> list[dict]:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            sessions = list_sessions(ctx, repo_name, status=status)
            if not full:
                return session_list_summary_rows(sessions)
            reply_config = _reply_generation_config(repo_name=repo_name)
            return [_session_with_telegram_runtime_state(ctx, session, reply_config=reply_config) for session in sessions]
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/sessions/{session_id}")
    def api_get_session(session_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "read")
            reply_config = _reply_generation_config(session=session)
            return _session_with_telegram_runtime_state(ctx, session, reply_config=reply_config)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/sessions/{session_id}")
    def api_get_repo_session(repo_name: str, session_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            reply_config = _reply_generation_config(session=session)
            return _session_with_telegram_runtime_state(ctx, session, reply_config=reply_config)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.post("/v1/native/sessions/{session_id}/events")
    def api_append_session_event(session_id: str, req: SessionEventAppend, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "contribute")
            return append_session_event(
                ctx,
                session_id,
                req.event_type,
                req.payload,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/sessions/{session_id}/events")
    def api_append_repo_session_event(repo_name: str, session_id: str, req: SessionEventAppend, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return append_session_event(
                ctx,
                str(session.get("session_id") or session_id),
                req.event_type,
                req.payload,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/task-dag-runs/{session_id}:advance")
    def api_advance_task_dag_run(session_id: str, req: TaskDagReadinessRequest, request: Request) -> dict:
        ctx = ctx_getter()
        repo_name = req.graph.get("repo_name")
        if not isinstance(repo_name, str) or not repo_name.strip():
            raise bad_request(ValueError("Task DAG graph must include repo_name for execute-run advancement."))
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name.strip(), "contribute")
        try:
            return advance_task_dag_run(
                ctx,
                session_id,
                req.graph,
                current_plan_revision_id=req.current_plan_revision_id,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/task-dag-runs/{session_id}:advance")
    def api_advance_repo_task_dag_run(repo_name: str, session_id: str, req: TaskDagReadinessRequest, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        normalized_repo_name = repo_name.strip()
        try:
            session = _resolve_session_for_repo(ctx, normalized_repo_name, session_id)
            graph_repo_name = str(req.graph.get("repo_name") or "").strip()
            if graph_repo_name and graph_repo_name != normalized_repo_name:
                raise ValueError("Task DAG graph repo_name does not match the repository path.")
            graph = dict(req.graph)
            graph["repo_name"] = normalized_repo_name
            return api_advance_task_dag_run(
                session_id=str(session.get("session_id") or session_id),
                req=TaskDagReadinessRequest(graph=graph, current_plan_revision_id=req.current_plan_revision_id),
                request=request,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/sessions/{session_id}:turn")
    def api_session_turn(session_id: str, req: SessionTurnRequest, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "contribute")
            if (guard_message := _compact_dag_worker_live_turn_guard(session)) is not None:
                raise ValueError(guard_message)
            text = str(req.text or "").strip()
            if not text:
                raise ValueError("text is required")
            surface = str(req.surface or "").strip() or "editor"
            title = str(req.title or session.get("title") or session_id).strip() or session_id
            actor_display_name = str(req.actor_display_name or "").strip() or None
            transport_envelope = (
                dict(req.transport_envelope)
                if isinstance(req.transport_envelope, dict) and req.transport_envelope
                else None
            )
            workflow_context = infer_workflow_context(text=text, session=session)
            user_event = append_session_event(
                ctx,
                session_id,
                "session.message",
                build_session_user_message_payload(
                    surface=surface,
                    title=title,
                    text=text,
                    actor_display_name=actor_display_name,
                    transport_envelope=transport_envelope,
                    workflow_context=workflow_context,
                    utc_now_iso=utc_now_iso,
                ),
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
            live_turn_token = start_live_turn(
                repo_name=str(session.get("repo_name") or ""),
                session_id=session_id,
                surface=surface,
                title=title,
                actor_identity=actor.identity,
            )
            clear_admin_response_cache()
            try:
                reply_config = _reply_generation_config(session=session)
                checkpoint_before_reply = _latest_session_checkpoint(ctx, session)
                events = _reply_generation_events(
                    ctx,
                    session_id,
                    user_event=user_event,
                    checkpoint_before_reply=checkpoint_before_reply,
                    reply_config=reply_config,
                )
                reply = _generate_session_reply(
                    reply_config,
                    session=session,
                    events=events,
                    chat_id=session_id,
                    chat_title=title,
                    checkpoint=checkpoint_before_reply,
                    surface=surface,
                    actor_identity=actor.identity,
                )
                task_dag_progress = _safe_task_dag_progress_summary_for_turn(
                    ctx,
                    session,
                    text=text,
                    surface_title=title,
                )
                assistant_text = _reply_text_with_task_dag_progress(reply.text, task_dag_progress)
                reply_text = _reply_text_with_turn_analysis(
                    assistant_text,
                    turn_analysis=reply.turn_analysis,
                    append_turn_analysis=getattr(reply_config, "telegram_append_turn_analysis", False),
                )
                assistant_payload = build_session_assistant_reply_payload(
                    reply=reply,
                    assistant_text=assistant_text,
                    surface=surface,
                    title=title,
                    session_id=session_id,
                    user_sequence=int(user_event.get("sequence") or 0),
                    transport_envelope=transport_envelope,
                    task_dag_progress=task_dag_progress,
                    build_transport_reply_envelope=build_transport_reply_envelope,
                    utc_now_iso=utc_now_iso,
                )
                assistant_event = append_session_event(
                    ctx,
                    session_id,
                    "assistant.reply",
                    assistant_payload,
                    actor_identity="ait-server",
                    actor_type="ai_assistant",
                )
                workflow_segmentation = _session_workflow_segmentation_summary(ctx, session)
                finish_live_turn(
                    live_turn_token,
                    status="succeeded",
                    repo_name=str(session.get("repo_name") or ""),
                    session_id=session_id,
                    surface=surface,
                    model=reply.model,
                    response_id=reply.response_id,
                    command_count=int((reply.turn_analysis or {}).get("command_count") or 0),
                    output_chars=len(reply_text),
                )
                clear_admin_response_cache()
                return {
                    "ok": True,
                    "session_id": session_id,
                    "user_event": user_event,
                    "assistant_event": assistant_event,
                    "reply_text": reply_text,
                    "turn_analysis": reply.turn_analysis or {},
                    "workflow_segmentation": workflow_segmentation,
                    "surface": surface,
                }
            except ReplyGenerationError as exc:
                finish_live_turn(
                    live_turn_token,
                    status="failed",
                    repo_name=str(session.get("repo_name") or ""),
                    session_id=session_id,
                    surface=surface,
                    error=str(exc),
                )
                clear_admin_response_cache()
                return {
                    "ok": False,
                    "session_id": session_id,
                    "user_event": user_event,
                    "assistant_event": None,
                    "reply_text": None,
                    "error": str(exc),
                    "surface": surface,
                }
            except Exception as exc:
                finish_live_turn(
                    live_turn_token,
                    status="failed",
                    repo_name=str(session.get("repo_name") or ""),
                    session_id=session_id,
                    surface=surface,
                    error=str(exc),
                )
                clear_admin_response_cache()
                raise
        except KeyError as exc:
            raise not_found(exc) from exc
        except AuthzError:
            raise
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/sessions/{session_id}:turn")
    def api_repo_session_turn(repo_name: str, session_id: str, req: SessionTurnRequest, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return api_session_turn(session_id=str(session.get("session_id") or session_id), req=req, request=request)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/sessions/{session_id}/events")
    def api_list_session_events(
        session_id: str,
        request: Request,
        after_sequence: int = 0,
        limit: int = 200,
    ) -> list[dict]:
        ctx = ctx_getter()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "read")
            return list_session_events(ctx, session_id, after_sequence=after_sequence, limit=limit)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/sessions/{session_id}/events")
    def api_list_repo_session_events(
        repo_name: str,
        session_id: str,
        request: Request,
        after_sequence: int = 0,
        limit: int = 200,
    ) -> list[dict]:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return api_list_session_events(
                session_id=str(session.get("session_id") or session_id),
                request=request,
                after_sequence=after_sequence,
                limit=limit,
            )
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/sessions/{session_id}/workflow-segments")
    def api_list_session_workflow_segments(
        session_id: str,
        request: Request,
        limit: int = 500,
    ) -> dict:
        ctx = ctx_getter()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "read")
            return _session_workflow_segmentation_summary(ctx, session, limit=limit)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/sessions/{session_id}/workflow-segments")
    def api_list_repo_session_workflow_segments(
        repo_name: str,
        session_id: str,
        request: Request,
        limit: int = 500,
    ) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return _session_workflow_segmentation_summary(ctx, session, limit=limit)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/sessions/{session_id}:telegramTurn")
    def api_telegram_turn(session_id: str, req: TelegramTurnRequest, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "contribute")
            if (guard_message := _compact_dag_worker_live_turn_guard(session)) is not None:
                raise ValueError(guard_message)
            text = str(req.text or "").strip()
            if not text:
                raise ValueError("text is required")
            telegram_message_ids = _normalized_telegram_message_ids(
                req.telegram_message_ids,
                telegram_message_id=req.telegram_message_id,
            )
            transport_envelope = (
                dict(req.transport_envelope)
                if isinstance(req.transport_envelope, dict) and req.transport_envelope
                else None
            )
            workflow_context = infer_workflow_context(text=text, session=session)
            reply_config = _reply_generation_config(session=session)
            tail_after_sequence = max(int(session.get("last_event_sequence") or 0) - 250, 0)
            recent_events = list_session_events(ctx, session_id, after_sequence=tail_after_sequence, limit=300)
            user_event = next(
                (
                    dict(event)
                    for event in reversed(recent_events)
                    if _matching_telegram_user_event(
                        event,
                        text=text,
                        chat_id=str(req.chat_id),
                        telegram_message_id=req.telegram_message_id,
                        telegram_message_ids=telegram_message_ids,
                        transport_envelope=transport_envelope,
                    )
                ),
                None,
            )
            if user_event is None:
                user_event = append_session_event(
                    ctx,
                    session_id,
                    "telegram.user_message",
                    build_telegram_user_message_payload(
                        text=text,
                        chat_id=req.chat_id,
                        chat_title=req.chat_title,
                        chat_type=req.chat_type,
                        telegram_message_id=req.telegram_message_id,
                        telegram_message_ids=telegram_message_ids,
                        transport_envelope=transport_envelope,
                        workflow_context=workflow_context,
                        utc_now_iso=utc_now_iso,
                    ),
                    actor_identity=actor.identity,
                    actor_type=actor.actor_type,
                )
                recent_events.append(user_event)
            else:
                assistant_event = _assistant_reply_for_telegram_user_event(
                    chat_id=str(req.chat_id),
                    user_event=user_event,
                    events=recent_events,
                )
                if assistant_event is not None:
                    return _telegram_turn_retry_response(
                        ctx,
                        session,
                        user_event=user_event,
                        assistant_event=assistant_event,
                        reply_config=reply_config,
                    )
            live_turn_token = start_live_turn(
                repo_name=str(session.get("repo_name") or ""),
                session_id=session_id,
                surface="telegram",
                title=req.chat_title or str(session.get("title") or req.chat_id),
                actor_identity=actor.identity,
            )
            clear_admin_response_cache()
            try:
                checkpoint_before_reply = _latest_session_checkpoint(ctx, session)
                events = _reply_generation_events(
                    ctx,
                    session_id,
                    user_event=user_event,
                    checkpoint_before_reply=checkpoint_before_reply,
                    reply_config=reply_config,
                )
                reply = _generate_session_reply(
                    reply_config,
                    session=session,
                    events=events,
                    chat_id=req.chat_id,
                    chat_title=req.chat_title or str(session.get("title") or req.chat_id),
                    checkpoint=checkpoint_before_reply,
                    actor_identity=actor.identity,
                )
                task_dag_progress = _safe_task_dag_progress_summary_for_turn(
                    ctx,
                    session,
                    text=text,
                    surface_title=req.chat_title or str(session.get("title") or req.chat_id),
                )
                assistant_text = _reply_text_with_task_dag_progress(reply.text, task_dag_progress)
                reply_text = _reply_text_with_turn_analysis(
                    assistant_text,
                    turn_analysis=reply.turn_analysis,
                    append_turn_analysis=getattr(reply_config, "telegram_append_turn_analysis", False),
                )
                assistant_payload = build_telegram_assistant_reply_payload(
                    reply=reply,
                    assistant_text=assistant_text,
                    chat_id=req.chat_id,
                    chat_title=req.chat_title,
                    chat_type=req.chat_type,
                    telegram_message_id=req.telegram_message_id,
                    telegram_message_ids=telegram_message_ids,
                    transport_envelope=transport_envelope,
                    user_sequence=int(user_event.get("sequence") or 0),
                    task_dag_progress=task_dag_progress,
                    build_transport_reply_envelope=build_transport_reply_envelope,
                    utc_now_iso=utc_now_iso,
                )
                assistant_event = append_session_event(
                    ctx,
                    session_id,
                    "assistant.reply",
                    assistant_payload,
                    actor_identity="ait-server",
                    actor_type="ai_assistant",
                )
                workflow_segmentation = _session_workflow_segmentation_summary(ctx, session)
                checkpoint = None
                try:
                    checkpoint = _maybe_refresh_telegram_checkpoint(
                        ctx,
                        session=session,
                        user_event=user_event,
                        assistant_event=assistant_event,
                        reply_config=reply_config,
                    )
                except Exception as exc:
                    print(
                        f"Telegram checkpoint refresh failed for {session_id}: {type(exc).__name__}: {exc}",
                        file=sys.stderr,
                        flush=True,
                    )
                refreshed_session = get_session(ctx, session_id)
                finish_live_turn(
                    live_turn_token,
                    status="succeeded",
                    repo_name=str(session.get("repo_name") or ""),
                    session_id=session_id,
                    surface="telegram",
                    model=reply.model,
                    response_id=reply.response_id,
                    command_count=int((reply.turn_analysis or {}).get("command_count") or 0),
                    output_chars=len(reply_text),
                )
                clear_admin_response_cache()
                return {
                    "ok": True,
                    "session_id": session_id,
                    "user_event": user_event,
                    "assistant_event": assistant_event,
                    "reply_text": reply_text,
                    "turn_analysis": reply.turn_analysis or {},
                    "workflow_segmentation": workflow_segmentation,
                    "checkpoint": checkpoint,
                    "telegram_context_runtime": _telegram_context_runtime_state(
                        ctx,
                        refreshed_session,
                        reply_config=reply_config,
                    ),
                }
            except ReplyGenerationError as exc:
                refreshed_session = get_session(ctx, session_id)
                finish_live_turn(
                    live_turn_token,
                    status="failed",
                    repo_name=str(session.get("repo_name") or ""),
                    session_id=session_id,
                    surface="telegram",
                    error=str(exc),
                )
                clear_admin_response_cache()
                return {
                    "ok": False,
                    "session_id": session_id,
                    "user_event": user_event,
                    "assistant_event": None,
                    "reply_text": None,
                    "error": str(exc),
                    "checkpoint": None,
                    "telegram_context_runtime": _telegram_context_runtime_state(
                        ctx,
                        refreshed_session,
                        reply_config=reply_config,
                    ),
                }
            except Exception as exc:
                finish_live_turn(
                    live_turn_token,
                    status="failed",
                    repo_name=str(session.get("repo_name") or ""),
                    session_id=session_id,
                    surface="telegram",
                    error=str(exc),
                )
                clear_admin_response_cache()
                raise
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/sessions/{session_id}/checkpoints")
    def api_create_session_checkpoint(session_id: str, req: SessionCheckpointCreate, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "contribute")
            return create_session_checkpoint(
                ctx,
                session_id,
                req.summary,
                snapshot_id=req.snapshot_id,
                resume_payload=req.resume_payload,
                based_on_sequence=req.based_on_sequence,
                checkpoint_id=req.checkpoint_id,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/sessions/{session_id}/checkpoints")
    def api_create_repo_session_checkpoint(repo_name: str, session_id: str, req: SessionCheckpointCreate, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return api_create_session_checkpoint(session_id=str(session.get("session_id") or session_id), req=req, request=request)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/sessions/{session_id}/checkpoints")
    def api_list_session_checkpoints(session_id: str, request: Request) -> list[dict]:
        ctx = ctx_getter()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "read")
            return list_session_checkpoints(ctx, session_id)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/sessions/{session_id}/checkpoints")
    def api_list_repo_session_checkpoints(repo_name: str, session_id: str, request: Request) -> list[dict]:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return api_list_session_checkpoints(session_id=str(session.get("session_id") or session_id), request=request)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/checkpoints/{checkpoint_id}")
    def api_get_session_checkpoint(checkpoint_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            checkpoint = get_session_checkpoint(ctx, checkpoint_id)
            session = get_session(ctx, checkpoint["session_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "read")
            return checkpoint
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.post("/v1/native/sessions/{session_id}:resume")
    def api_resume_session(session_id: str, req: SessionResumeRequest, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "contribute")
            return resume_session(
                ctx,
                session_id,
                after_sequence=req.after_sequence,
                limit=req.limit,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/sessions/{session_id}:resume")
    def api_repo_session_resume(repo_name: str, session_id: str, req: SessionResumeRequest, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return api_resume_session(session_id=str(session.get("session_id") or session_id), req=req, request=request)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/sessions/{session_id}:close")
    def api_close_session(session_id: str, req: SessionCloseRequest, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "contribute")
            return close_session(
                ctx,
                session_id,
                req.status,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/sessions/{session_id}:close")
    def api_repo_session_close(repo_name: str, session_id: str, req: SessionCloseRequest, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return api_close_session(session_id=str(session.get("session_id") or session_id), req=req, request=request)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc
