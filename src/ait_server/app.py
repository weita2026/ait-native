from __future__ import annotations

import logging
import os
import sys
import threading
import time
from pathlib import Path
from typing import Any, Mapping, Optional

import uvicorn
from fastapi import FastAPI, HTTPException, Request, Response

from ait_chat.codex_app_server import prune_stale_managed_codex_app_servers
from ait_chat.session_reply import ReplyGenerationError, generate_session_reply
from .admin_cache import (
    _admin_metrics_cache_ttl_seconds,
    _annotated_admin_payload,
    _cached_admin_payload,
    _clear_admin_response_cache,
)
from .agent_transport_runtime import (
    build_transport_reply_envelope,
    trigger_task_dag_telegram_notifications as _agent_transport_trigger_task_dag_telegram_notifications,
    utc_now_iso,
)
from .local_repo_seams import infer_workflow_context
from .server_process_runtime import (
    AIT_SERVER_TERMINATION_CONTEXT_ENV,
    _consume_pending_server_termination_context,
    _process_command_for_pid,
    _server_pid_file_path,
    _server_runtime_identity,
    _server_signal_stop_suffix,
    _server_startup_identity_summary,
    _server_termination_context_path,
)
from .release_route_helpers import (
    _release_artifact_response,
    _release_response_payload,
)
from .planning_routes import register_planning_routes
from .read_routes import TaskDagReadinessRequest, register_read_routes
from .repository_routes import register_repository_routes
from .stack_routes import register_stack_routes
from .task_routes import register_task_routes
from .route_request_models import (
    ChangeCloseRequest,
    ChangeCreate,
    CreateWaiverRequest,
    GcRequest,
    OptimizeRequest,
    PackRequest,
    PatchsetPublish,
    RecordReviewRequest,
    ReconcileRequest,
    ReleasePublishRequest,
    RepositoryCreate,
    RequestReviewRequest,
    RetryLandRequest,
    RoleBindingGrant,
    RunPatchsetCiRequest,
    RunRepoCiRequest,
    SelectPatchsetRequest,
    SessionCheckpointCreate,
    SessionCloseRequest,
    SessionCreate,
    SessionEventAppend,
    SessionResumeRequest,
    SessionTurnRequest,
    SubmitLandRequest,
    TelegramTurnRequest,
    UpsertAttestationRequest,
)
try:
    from .live_turns import finish_live_turn, snapshot_live_turn_metrics, start_live_turn
except ImportError:  # pragma: no cover - exercised when the runtime helper is absent during partial bootstraps.
    def start_live_turn(**_: Any) -> str:
        return ""

    def finish_live_turn(*_: Any, **__: Any) -> dict[str, Any]:
        return {}

    def snapshot_live_turn_metrics() -> dict[str, Any]:
        return {
            "active_turns": 0,
            "active_repositories": {},
            "oldest_active_turn_started_at": None,
            "oldest_active_turn_age_seconds": None,
            "recent_completed_turns": [],
            "recent_failed_turns": [],
            "recent_completed_p95_seconds": None,
        }

from .server_auth import (
    AuthzError,
    actor_from_request,
    ensure_admin_action,
    ensure_repo_action,
    ensure_review_action,
    whoami_payload,
)
from .server_control import grant_role_bindings, list_role_bindings
from .server_queue import enqueue_async_job, get_job, job_diagnostics, list_jobs
from .read_models_domains.ci_status import patchset_ci_status, repository_ci_runs
from .read_models import (
    live_turn_pressure_summary,
    normalize_live_turn_metrics,
    change_detail,
    patchset_delta,
    repository_detail,
    repository_index,
    repository_worker_status,
    reviewer_inbox,
    server_metrics,
    server_readiness,
    stack_detail,
    task_dag_graph,
    task_dag_progress,
    task_audit,
    task_dag_readiness,
    task_dag_schedule,
    task_detail,
    task_queue,
)
from .patchset_ci import mark_patchset_ci_pending, run_patchset_ci
from .repo_ci import run_repo_ci
from .shared_runtime_policy import enforce_shared_runtime_policy, evaluate_shared_runtime_policy
from .task_graph_runs import advance_task_dag_run
from .workflow_async_jobs import (
    _land_job_payload,
    _maybe_enqueue_land,
    _maybe_follow_patchset_publish_with_ci,
    _maybe_start_patchset_ci,
    _maybe_enqueue_policy,
    _patchset_publish_policy_followup,
    _patchset_ci_job_payload,
    _policy_job_payload,
    _queue_mode,
)
from .task_dag_route_helpers import (
    _attach_task_dag_notification_followup,
    _reply_text_with_task_dag_progress as _reply_text_with_task_dag_progress_helper,
    _safe_task_dag_progress_summary_for_turn as _safe_task_dag_progress_summary_for_turn_impl,
    _schedule_task_dag_notification,
    _task_dag_progress_summary_for_turn_impl,
)
from . import server_store as _server_store
from .server_store import (
    ServerContext,
    append_session_event,
    close_change,
    close_session,
    create_change,
    create_session,
    create_session_checkpoint,
    create_waiver,
    create_land_request,
    evaluate_policy,
    get_attestation,
    get_change,
    get_change_for_repo,
    get_land_request,
    get_land_request_for_repo,
    get_patchset,
    get_patchset_for_repo,
    get_release,
    get_release_for_repo,
    get_policy_status,
    get_repository,
    get_task,
    initialize,
    join_planning_session,
    list_changes,
    list_patchsets,
    list_patchsets_for_repo,
    list_reviews,
    publish_release,
    publish_patchset,
    read_release_artifact,
    record_review,
    request_review,
    retry_land,
    select_patchset,
    submit_land,
    submit_land_for_repo,
    reconcile_repository,
    optimize_repository_storage,
    get_repository_storage,
    pack_repository_storage,
    gc_repository_storage,
    upsert_attestation,
    get_session,
    get_session_checkpoint,
    list_session_checkpoints,
    list_session_events,
    list_sessions,
    resume_session,
)


LOGGER = logging.getLogger(__name__)
_DEFAULT_TASK_DAG_TURN_PROGRESS_LOCK_TIMEOUT_MS = 750
_DEFAULT_TASK_DAG_TURN_PROGRESS_STATEMENT_TIMEOUT_MS = 2500


class _AitUvicornServer(uvicorn.Server):
    def handle_exit(self, sig: int, frame) -> None:  # pragma: no cover - exercised through signal delivery in runtime.
        print(
            f"Received signal {sig}; stopping ait-server{_server_signal_stop_suffix(sig)}.",
            file=sys.stderr,
            flush=True,
        )
        super().handle_exit(sig, frame)



_RUNTIME_CTX: ServerContext | None = None
_RUNTIME_CTX_LOCK = threading.Lock()


def _runtime_ctx(*, force_refresh: bool = False) -> ServerContext:
    global _RUNTIME_CTX
    with _RUNTIME_CTX_LOCK:
        if force_refresh or _RUNTIME_CTX is None:
            ctx = ServerContext.from_env()
            initialize(ctx)
            _RUNTIME_CTX = ctx
        return _RUNTIME_CTX


def _ctx() -> ServerContext:
    return _runtime_ctx()


def _not_found(exc: Exception) -> HTTPException:
    return HTTPException(status_code=404, detail=str(exc))



def _bad_request(exc: Exception) -> HTTPException:
    return HTTPException(status_code=400, detail=str(exc))


def _conflict(exc: Exception) -> HTTPException:
    return HTTPException(status_code=409, detail=str(exc))


def _task_dag_notification_plan_ids_from_task(ctx: ServerContext, task: Mapping[str, Any] | None) -> set[str] | None:
    if not isinstance(task, Mapping):
        return None
    plan_id = str(task.get("plan_id") or "").strip()
    if not plan_id:
        return None
    return {plan_id}


def _task_dag_notification_plan_ids_from_task_id(ctx: ServerContext, task_id: str) -> set[str] | None:
    try:
        return _task_dag_notification_plan_ids_from_task(ctx, get_task(ctx, task_id))
    except KeyError:
        return None


def _task_dag_notification_plan_ids_from_change(ctx: ServerContext, change: Mapping[str, Any] | None) -> set[str] | None:
    if not isinstance(change, Mapping):
        return None
    task_id = str(change.get("task_id") or "").strip()
    if not task_id:
        return None
    return _task_dag_notification_plan_ids_from_task_id(ctx, task_id)


def _task_dag_notification_plan_ids(
    ctx: ServerContext,
    *,
    event_type: str,
    entity_id: str,
) -> set[str] | None:
    event_name = str(event_type or "").strip().lower()
    ref = str(entity_id or "").strip()
    if not ref:
        return None
    try:
        if event_name.startswith("task."):
            return _task_dag_notification_plan_ids_from_task_id(ctx, ref)
        if event_name.startswith("change."):
            return _task_dag_notification_plan_ids_from_change(ctx, get_change(ctx, ref))
        if event_name.startswith("patchset.") or event_name.startswith("attestation.") or event_name.startswith("policy."):
            patchset = get_patchset(ctx, ref)
            return _task_dag_notification_plan_ids_from_change(ctx, get_change(ctx, str(patchset.get("change_id") or "")))
        if event_name.startswith("review."):
            return _task_dag_notification_plan_ids_from_change(ctx, get_change(ctx, ref))
        if event_name.startswith("land."):
            land = get_land_request(ctx, ref)
            return _task_dag_notification_plan_ids_from_change(ctx, get_change(ctx, str(land.get("change_id") or "")))
    except KeyError:
        return None
    return None


def _trigger_task_dag_telegram_notifications(
    ctx: ServerContext,
    repo_name: str,
    *,
    event_type: str,
    entity_id: str,
) -> None:
    _agent_transport_trigger_task_dag_telegram_notifications(
        ctx,
        repo_name,
        event_type=event_type,
        entity_id=entity_id,
        plan_ids=_task_dag_notification_plan_ids(ctx, event_type=event_type, entity_id=entity_id),
        resolve_repo_root=_reply_generation_repo_root,
        progress_reader=task_dag_progress,
    )


def _schedule_task_dag_telegram_notifications(
    ctx: ServerContext,
    repo_name: str,
    *,
    event_type: str,
    entity_id: str,
) -> dict[str, Any]:
    return _schedule_task_dag_notification(
        _trigger_task_dag_telegram_notifications,
        ctx=ctx,
        repo_name=repo_name,
        event_type=event_type,
        entity_id=entity_id,
    )


def _attach_notification_followup(
    result: dict[str, Any],
    ctx: ServerContext,
    repo_name: str,
    *,
    event_type: str,
    entity_id: str,
) -> dict[str, Any]:
    return _attach_task_dag_notification_followup(
        result,
        _trigger_task_dag_telegram_notifications,
        ctx=ctx,
        repo_name=repo_name,
        event_type=event_type,
        entity_id=entity_id,
    )


def _publish_followup_phase_state(
    result: dict[str, Any],
    *,
    deferred_key: str,
    queued_key: str,
    completed_key: str,
) -> str:
    deferred_payload = result.get(deferred_key)
    if isinstance(deferred_payload, dict):
        state = str(deferred_payload.get("state") or "").strip()
        if state:
            return state
    if isinstance(result.get(queued_key), dict):
        return "queued"
    if isinstance(result.get(completed_key), dict):
        return "completed"
    return "not_applicable"


def _patchset_publish_request_path_audit(result: dict[str, Any], timings: dict[str, float]) -> list[dict[str, Any]]:
    ci_state = _publish_followup_phase_state(
        result,
        deferred_key="ci_followup",
        queued_key="ci_job",
        completed_key="ci_result",
    )
    policy_state = _publish_followup_phase_state(
        result,
        deferred_key="policy_followup",
        queued_key="policy_job",
        completed_key="policy",
    )
    notification_state = str((result.get("notification_followup") or {}).get("delivery") or "not_applicable")
    return [
        {
            "phase": "publish_patchset",
            "state": "completed",
            "seconds": timings.get("publish_patchset_seconds", 0.0),
            "required_for_immediate_correctness": True,
            "deferred_safe": False,
            "reason": "Patchset identity, revision selection, and workflow-land patchset readiness depend on this persistence step.",
        },
        {
            "phase": "ci_followup",
            "state": ci_state,
            "seconds": timings.get("ci_followup_seconds", 0.0),
            "required_for_immediate_correctness": False,
            "deferred_safe": True,
            "reason": "Patchset CI evidence is required later for land gates, but patchset publication stays correct when CI is queued or deferred off the request path.",
        },
        {
            "phase": "policy_followup",
            "state": policy_state,
            "seconds": timings.get("policy_followup_seconds", 0.0),
            "required_for_immediate_correctness": False,
            "deferred_safe": True,
            "reason": "Policy may wait for attestation, review, selection, or waiver evidence without changing patchset identity or next-action correctness.",
        },
        {
            "phase": "notification_followup",
            "state": notification_state,
            "seconds": timings.get("notification_followup_seconds", 0.0),
            "required_for_immediate_correctness": False,
            "deferred_safe": True,
            "reason": "Notification scheduling is observability-only and does not affect patchset publication correctness.",
        },
    ]


from .session_route_helpers import (
    _assistant_reply_for_telegram_user_event,
    _compact_dag_worker_live_turn_guard,
    _latest_session_checkpoint,
    _matching_telegram_user_event,
    _maybe_refresh_telegram_checkpoint,
    _normalized_telegram_message_ids,
    _render_turn_analysis_footer,
    _reply_generation_config,
    _reply_generation_events,
    _reply_generation_repo_name,
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


def _reply_text_with_task_dag_progress(text: str, summary: dict[str, Any] | None) -> str:
    return _reply_text_with_task_dag_progress_helper(text, summary)


def _task_dag_progress_summary_for_turn(
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


def create_app() -> FastAPI:
    ctx = _runtime_ctx(force_refresh=True)
    startup_policy = enforce_shared_runtime_policy(ctx, component="ait-server", allow_legacy_override=False)
    if startup_policy.override_active:
        LOGGER.warning(
            "ait-server legacy SQLite shared-deployment override active for %s (root=%s)",
            startup_policy.component,
            ctx.root,
        )
    app = FastAPI(title="ait native server", version="0.10.3")

    @app.get("/healthz")
    def healthz() -> dict:
        ctx = _ctx()
        shared_runtime_policy = evaluate_shared_runtime_policy(ctx, component="ait-server", allow_legacy_override=False)
        live_turn_metrics = normalize_live_turn_metrics(snapshot_live_turn_metrics())
        summary = live_turn_metrics.get("summary") if isinstance(live_turn_metrics.get("summary"), dict) else {}
        payload = {
            "ok": True,
            "db_backend": ctx.db_backend,
            "using_postgres": ctx.using_postgres,
            "runtime_root": str(ctx.root),
            "runtime_root_source": str(ctx.root_source or "explicit"),
            "ci_capabilities": {
                "patchset_run_ci_route": True,
                "repo_run_ci_route": True,
                "patchset_ci_status_route": True,
                "repo_ci_runs_route": True,
                "supported_repo_planes": ["nightly", "release", "post_land_regression"],
                "supported_task_batch_selectors": [
                    "recent_remote_landed",
                    "recent_remote_landed_high_risk",
                    "explicit_task_ids",
                    "curated_corpus",
                ],
            },
            "ci_readiness": {
                "runtime_generation": "native_ci_runtime_v1",
                "stale_runtime_hint": (
                    "If patchset/repo CI commands still return 404 or 405, restart/update the live ait-server process "
                    "so the running runtime matches the checked-in CI routes."
                ),
            },
            "shared_runtime_policy": shared_runtime_policy.as_dict(),
            "queue_mode": _queue_mode(),
            "pressure_metrics_cache_ttl_seconds": _admin_metrics_cache_ttl_seconds(),
            "live_turn_pressure": live_turn_pressure_summary(live_turn_metrics),
            "live_turns": {
                "active_turns": int(summary.get("active_turns") or 0),
                "active_repositories": int(summary.get("active_repositories") or 0),
                "oldest_active_turn_started_at": summary.get("oldest_active_turn_started_at"),
                "oldest_active_turn_age_seconds": summary.get("oldest_active_turn_age_seconds"),
                "recent_completed_turns": int(summary.get("recent_completed_turns") or 0),
                "recent_failed_turns": int(summary.get("recent_failed_turns") or 0),
                "recent_completed_p95_seconds": summary.get("recent_completed_p95_seconds"),
            },
        }
        return _annotated_admin_payload(
            payload,
            cache_state="computed",
            cache_age_seconds=0.0,
            cache_ttl_seconds=_admin_metrics_cache_ttl_seconds(),
            cached_at=utc_now_iso(),
        )


    register_read_routes(
        app,
        ctx_getter=_ctx,
        not_found=_not_found,
        bad_request=_bad_request,
    )
    register_stack_routes(
        app,
        ctx_getter=_ctx,
        not_found=_not_found,
        bad_request=_bad_request,
    )

    @app.get("/v1/native/auth/whoami")
    def api_auth_whoami(request: Request, repo_name: str | None = None) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        return whoami_payload(ctx, actor, repo_name)

    @app.post("/v1/native/admin/repositories/{repo_name}/bindings")
    def api_grant_role_bindings(repo_name: str, req: RoleBindingGrant, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, repo_name)
        rows = grant_role_bindings(ctx, repo_name, req.actor_identity, req.roles)
        return {"repo_name": repo_name, "actor_identity": req.actor_identity, "roles": sorted({row['role'] for row in rows}), "bindings": rows}

    @app.get("/v1/native/admin/repositories/{repo_name}/bindings")
    def api_list_role_bindings(repo_name: str, request: Request) -> list[dict]:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, repo_name)
        return list_role_bindings(ctx, repo_name)

    @app.get("/v1/native/admin/repositories/{repo_name}/storage")
    def api_repository_storage(repo_name: str, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, repo_name)
        try:
            return get_repository_storage(ctx, repo_name)
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/admin/repositories/{repo_name}:pack")
    def api_pack_repo(repo_name: str, req: PackRequest, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, repo_name)
        try:
            if _queue_mode() == "async":
                job = enqueue_async_job(
                    ctx,
                    repo_name,
                    "content.pack",
                    {"repo_name": repo_name, "repack": bool(req.repack), "max_members": req.max_members},
                    max_attempts=3,
                    dedupe_active=True,
                )
                return {"queued": True, "job": job}
            result = pack_repository_storage(ctx, repo_name, repack=bool(req.repack), max_members=req.max_members)
            return {"queued": False, "result": result}
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/admin/repositories/{repo_name}:optimize")
    def api_optimize_repo(repo_name: str, req: OptimizeRequest, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, repo_name)
        try:
            if _queue_mode() == "async":
                job = enqueue_async_job(
                    ctx,
                    repo_name,
                    "content.optimize",
                    {"repo_name": repo_name, "repair": bool(req.repair)},
                    max_attempts=3,
                    dedupe_active=True,
                )
                return {"repo_name": repo_name, "queued": True, "job": job}
            result = optimize_repository_storage(ctx, repo_name, repair=bool(req.repair))
            return {"repo_name": repo_name, "queued": False, "result": result}
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/admin/repositories/{repo_name}:gc")
    def api_gc_repo(repo_name: str, req: GcRequest, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, repo_name)
        try:
            if _queue_mode() == "async":
                job = enqueue_async_job(
                    ctx,
                    repo_name,
                    "content.gc",
                    {
                        "repo_name": repo_name,
                        "prune_unreferenced": bool(req.prune_unreferenced),
                        "prune_orphan_packs": bool(req.prune_orphan_packs),
                    },
                    max_attempts=3,
                    dedupe_active=True,
                )
                return {"queued": True, "job": job}
            result = gc_repository_storage(
                ctx,
                repo_name,
                prune_unreferenced=bool(req.prune_unreferenced),
                prune_orphan_packs=bool(req.prune_orphan_packs),
            )
            return {"queued": False, "result": result}
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.get("/v1/native/admin/repositories/{repo_name}/jobs")
    def api_list_jobs(
        repo_name: str,
        request: Request,
        state: str | None = None,
        limit: int = 50,
        diagnostics: bool = False,
        stale_after_seconds: int = 300,
    ) -> list[dict] | dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, repo_name)
        try:
            if diagnostics:
                return job_diagnostics(ctx, repo_name=repo_name, stale_after_seconds=stale_after_seconds, limit=limit)
            return list_jobs(ctx, repo_name=repo_name, state=state, limit=limit)
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.get("/v1/native/admin/repositories/{repo_name}/workers")
    def api_repository_workers(repo_name: str, request: Request, recent_jobs_limit: int = 20) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, repo_name)
        try:
            return repository_worker_status(ctx, repo_name, recent_jobs_limit=recent_jobs_limit)
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.get("/v1/native/admin/metrics")
    def api_server_metrics(request: Request, recent_jobs_limit: int = 50, stale_after_seconds: int = 300) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        try:
            payload = _cached_admin_payload(
                "server_metrics",
                (int(recent_jobs_limit), int(stale_after_seconds)),
                lambda: server_metrics(ctx, recent_jobs_limit=recent_jobs_limit, stale_after_seconds=stale_after_seconds),
            )
        except ValueError as exc:
            raise _bad_request(exc) from exc
        for repo in payload.get("repositories") or []:
            ensure_admin_action(ctx, actor, str(repo.get("repo_name") or ""))
        return payload

    @app.get("/v1/native/admin/readiness")
    def api_server_readiness(request: Request, recent_jobs_limit: int = 50, stale_after_seconds: int = 300) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        try:
            payload = _cached_admin_payload(
                "server_readiness",
                (int(recent_jobs_limit), int(stale_after_seconds)),
                lambda: server_readiness(ctx, recent_jobs_limit=recent_jobs_limit, stale_after_seconds=stale_after_seconds),
            )
        except ValueError as exc:
            raise _bad_request(exc) from exc
        repository_names = payload.get("repository_names") or [
            str(repo.get("repo_name") or "")
            for repo in repository_index(ctx).get("repositories") or []
            if str(repo.get("repo_name") or "")
        ]
        for repo_name in repository_names:
            ensure_admin_action(ctx, actor, str(repo_name))
        return payload

    @app.get("/v1/native/admin/jobs/{job_id}")
    def api_get_job(job_id: int, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        job = get_job(ctx, job_id)
        ensure_admin_action(ctx, actor, job["repo_name"])
        return job

    @app.post("/v1/native/admin/repositories/{repo_name}:reconcile")
    def api_reconcile_repo(repo_name: str, req: ReconcileRequest, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, repo_name)
        if _queue_mode() == "async":
            job = enqueue_async_job(
                ctx,
                repo_name,
                "reconcile.repo",
                {"repo_name": repo_name, "repair": bool(req.repair)},
                max_attempts=3,
                dedupe_active=True,
            )
            return {"repo_name": repo_name, "queued": True, "job": job}
        result = reconcile_repository(ctx, repo_name, repair=bool(req.repair))
        return {"repo_name": repo_name, "queued": False, "result": result}

    @app.post("/v1/native/admin/repositories/{repo_name}:runCi")
    def api_run_repo_ci(repo_name: str, req: RunRepoCiRequest, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, repo_name)
        suite_ids = [str(item).strip() for item in list(req.suite_ids or []) if str(item).strip()]
        plane = str(req.plane or "").strip() or None
        target_line = str(req.target_line or "main").strip() or "main"
        trigger = str(req.trigger or "manual_rerun").strip() or "manual_rerun"
        selector = str(req.selector or "").strip() or None
        task_ids = [str(item).strip() for item in list(req.task_ids or []) if str(item).strip()]
        curated_corpus = str(req.curated_corpus or "").strip() or None
        count = int(req.count) if req.count is not None else None
        window_days = int(req.window_days) if req.window_days is not None else None
        dependency_evidence = [str(item).strip() for item in list(req.dependency_evidence or []) if str(item).strip()]
        compliance_evidence = [str(item).strip() for item in list(req.compliance_evidence or []) if str(item).strip()]
        try:
            if _queue_mode() == "async":
                job = enqueue_async_job(
                    ctx,
                    repo_name,
                    "repo.ci",
                    {
                        "repo_name": repo_name,
                        "repo_id": str(get_repository(ctx, repo_name).get("repo_id") or ""),
                        "suite_ids": suite_ids or None,
                        "plane": plane,
                        "target_line": target_line,
                        "trigger": trigger,
                        "selector": selector,
                        "task_ids": task_ids or None,
                        "curated_corpus": curated_corpus,
                        "count": count,
                        "window_days": window_days,
                        "dependency_evidence": dependency_evidence or None,
                        "compliance_evidence": compliance_evidence or None,
                    },
                    max_attempts=3,
                    dedupe_active=True,
                )
                return {
                    "repo_name": repo_name,
                    "queued": True,
                    "job": job,
                    "suite_ids": suite_ids,
                    "plane": plane,
                    "target_line": target_line,
                    "trigger": trigger,
                    "selector": selector,
                    "task_ids": task_ids,
                    "curated_corpus": curated_corpus,
                    "count": count,
                    "window_days": window_days,
                    "dependency_evidence": dependency_evidence,
                    "compliance_evidence": compliance_evidence,
                }
            result = run_repo_ci(
                ctx,
                repo_name,
                suite_ids=suite_ids or None,
                plane=plane,
                target_line=target_line,
                trigger=trigger,
                selector=selector,
                task_ids=task_ids or None,
                curated_corpus=curated_corpus,
                count=count,
                window_days=window_days,
                dependency_evidence=dependency_evidence or None,
                compliance_evidence=compliance_evidence or None,
            )
            return result
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/admin/repositories/{repo_name}:migrations")
    def api_run_repo_migrations(repo_name: str, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, repo_name)
        try:
            get_repository(ctx, repo_name)
            initialize(ctx)
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc
        return {
            "repo_name": repo_name,
            "migrations": [
                "control_plane_repo_id_backfill",
                "control_plane_local_key_backfill",
            ],
        }

    register_repository_routes(
        app,
        ctx_getter=_ctx,
        not_found=_not_found,
        bad_request=_bad_request,
    )

    register_planning_routes(
        app,
        ctx_getter=_ctx,
        not_found=_not_found,
        bad_request=_bad_request,
        conflict=_conflict,
    )

    register_task_routes(
        app,
        ctx_getter=_ctx,
        not_found=_not_found,
        bad_request=_bad_request,
        conflict=_conflict,
        attach_notification_followup=_attach_notification_followup,
        trigger_task_notification=_trigger_task_dag_telegram_notifications,
    )

    @app.post("/v1/native/repositories/{repo_name}/sessions")
    def api_create_session(repo_name: str, req: SessionCreate, request: Request) -> dict:
        ctx = _ctx()
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
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _conflict(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/sessions")
    def api_list_sessions(repo_name: str, request: Request, status: str | None = None) -> list[dict]:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            sessions = list_sessions(ctx, repo_name, status=status)
            reply_config = _reply_generation_config(repo_name=repo_name)
            return [_session_with_telegram_runtime_state(ctx, session, reply_config=reply_config) for session in sessions]
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/sessions/{session_id}")
    def api_get_session(session_id: str, request: Request) -> dict:
        ctx = _ctx()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "read")
            reply_config = _reply_generation_config(session=session)
            return _session_with_telegram_runtime_state(ctx, session, reply_config=reply_config)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/sessions/{session_id}")
    def api_get_repo_session(repo_name: str, session_id: str, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            reply_config = _reply_generation_config(session=session)
            return _session_with_telegram_runtime_state(ctx, session, reply_config=reply_config)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.post("/v1/native/sessions/{session_id}/events")
    def api_append_session_event(session_id: str, req: SessionEventAppend, request: Request) -> dict:
        ctx = _ctx()
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
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/sessions/{session_id}/events")
    def api_append_repo_session_event(repo_name: str, session_id: str, req: SessionEventAppend, request: Request) -> dict:
        ctx = _ctx()
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
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/task-dag-runs/{session_id}:advance")
    def api_advance_task_dag_run(session_id: str, req: TaskDagReadinessRequest, request: Request) -> dict:
        ctx = _ctx()
        repo_name = req.graph.get("repo_name")
        if not isinstance(repo_name, str) or not repo_name.strip():
            raise _bad_request(ValueError("Task DAG graph must include repo_name for execute-run advancement."))
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
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/task-dag-runs/{session_id}:advance")
    def api_advance_repo_task_dag_run(repo_name: str, session_id: str, req: TaskDagReadinessRequest, request: Request) -> dict:
        ctx = _ctx()
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
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/sessions/{session_id}:turn")
    def api_session_turn(session_id: str, req: SessionTurnRequest, request: Request) -> dict:
        ctx = _ctx()
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
            _clear_admin_response_cache()
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
                reply = generate_session_reply(
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
                _clear_admin_response_cache()
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
                _clear_admin_response_cache()
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
                _clear_admin_response_cache()
                raise
        except KeyError as exc:
            raise _not_found(exc) from exc
        except AuthzError as exc:
            raise _authz(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/sessions/{session_id}:turn")
    def api_repo_session_turn(repo_name: str, session_id: str, req: SessionTurnRequest, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return api_session_turn(session_id=str(session.get("session_id") or session_id), req=req, request=request)
        except KeyError as exc:
            raise _not_found(exc) from exc


    @app.get("/v1/native/sessions/{session_id}/events")
    def api_list_session_events(
        session_id: str,
        request: Request,
        after_sequence: int = 0,
        limit: int = 200,
    ) -> list[dict]:
        ctx = _ctx()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "read")
            return list_session_events(ctx, session_id, after_sequence=after_sequence, limit=limit)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/sessions/{session_id}/events")
    def api_list_repo_session_events(
        repo_name: str,
        session_id: str,
        request: Request,
        after_sequence: int = 0,
        limit: int = 200,
    ) -> list[dict]:
        ctx = _ctx()
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
            raise _not_found(exc) from exc

    @app.get("/v1/native/sessions/{session_id}/workflow-segments")
    def api_list_session_workflow_segments(
        session_id: str,
        request: Request,
        limit: int = 500,
    ) -> dict:
        ctx = _ctx()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "read")
            return _session_workflow_segmentation_summary(ctx, session, limit=limit)
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/sessions/{session_id}/workflow-segments")
    def api_list_repo_session_workflow_segments(
        repo_name: str,
        session_id: str,
        request: Request,
        limit: int = 500,
    ) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return _session_workflow_segmentation_summary(ctx, session, limit=limit)
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/sessions/{session_id}:telegramTurn")
    def api_telegram_turn(session_id: str, req: TelegramTurnRequest, request: Request) -> dict:
        ctx = _ctx()
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
            _clear_admin_response_cache()
            try:
                checkpoint_before_reply = _latest_session_checkpoint(ctx, session)
                events = _reply_generation_events(
                    ctx,
                    session_id,
                    user_event=user_event,
                    checkpoint_before_reply=checkpoint_before_reply,
                    reply_config=reply_config,
                )
                reply = generate_session_reply(
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
                except Exception as exc:  # pragma: no cover - defensive isolation for checkpoint refresh
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
                _clear_admin_response_cache()
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
                _clear_admin_response_cache()
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
                _clear_admin_response_cache()
                raise
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/sessions/{session_id}/checkpoints")
    def api_create_session_checkpoint(session_id: str, req: SessionCheckpointCreate, request: Request) -> dict:
        ctx = _ctx()
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
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/sessions/{session_id}/checkpoints")
    def api_create_repo_session_checkpoint(repo_name: str, session_id: str, req: SessionCheckpointCreate, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return api_create_session_checkpoint(session_id=str(session.get("session_id") or session_id), req=req, request=request)
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.get("/v1/native/sessions/{session_id}/checkpoints")
    def api_list_session_checkpoints(session_id: str, request: Request) -> list[dict]:
        ctx = _ctx()
        try:
            session = get_session(ctx, session_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "read")
            return list_session_checkpoints(ctx, session_id)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/sessions/{session_id}/checkpoints")
    def api_list_repo_session_checkpoints(repo_name: str, session_id: str, request: Request) -> list[dict]:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return api_list_session_checkpoints(session_id=str(session.get("session_id") or session_id), request=request)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/checkpoints/{checkpoint_id}")
    def api_get_session_checkpoint(checkpoint_id: str, request: Request) -> dict:
        ctx = _ctx()
        try:
            checkpoint = get_session_checkpoint(ctx, checkpoint_id)
            session = get_session(ctx, checkpoint["session_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, session["repo_name"], "read")
            return checkpoint
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.post("/v1/native/sessions/{session_id}:resume")
    def api_resume_session(session_id: str, req: SessionResumeRequest, request: Request) -> dict:
        ctx = _ctx()
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
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/sessions/{session_id}:resume")
    def api_repo_session_resume(repo_name: str, session_id: str, req: SessionResumeRequest, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return api_resume_session(session_id=str(session.get("session_id") or session_id), req=req, request=request)
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/sessions/{session_id}:close")
    def api_close_session(session_id: str, req: SessionCloseRequest, request: Request) -> dict:
        ctx = _ctx()
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
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/sessions/{session_id}:close")
    def api_repo_session_close(repo_name: str, session_id: str, req: SessionCloseRequest, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            session = _resolve_session_for_repo(ctx, repo_name, session_id)
            return api_close_session(session_id=str(session.get("session_id") or session_id), req=req, request=request)
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/changes")
    def api_create_change(repo_name: str, req: ChangeCreate, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            result = create_change(
                ctx,
                repo_name,
                req.task_id,
                req.title,
                req.base_line,
                req.risk_tier,
                change_id=req.change_id,
                fork_snapshot_id=req.fork_snapshot_id,
                forked_from_line=req.forked_from_line,
            )
            _trigger_task_dag_telegram_notifications(ctx, repo_name, event_type="change.created", entity_id=result["change_id"])
            return result
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _conflict(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/changes")
    def api_list_changes(repo_name: str, request: Request) -> list[dict]:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return list_changes(ctx, repo_name)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/changes/{change_ref}")
    def api_get_repo_change(repo_name: str, change_ref: str, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return get_change_for_repo(ctx, repo_name, change_ref)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/changes/{change_id}")
    def api_get_change(change_id: str, request: Request) -> dict:
        ctx = _ctx()
        try:
            change = get_change(ctx, change_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "read")
            return change
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.post("/v1/native/changes/{change_id}:close")
    def api_close_change(change_id: str, req: ChangeCloseRequest, request: Request) -> dict:
        ctx = _ctx()
        try:
            change = get_change(ctx, change_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "contribute")
            result = close_change(ctx, change_id, req.status)
            _trigger_task_dag_telegram_notifications(ctx, change["repo_name"], event_type="change.closed", entity_id=change_id)
            return result
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/releases")
    def api_publish_release(repo_name: str, req: ReleasePublishRequest, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "land")
        try:
            result = publish_release(
                ctx,
                repo_name,
                req.release_id,
                req.version,
                req.line,
                req.snapshot_id,
                req.manifest_hash,
                req.profile,
                package=req.package,
                checks=req.checks,
                artifacts=[artifact.model_dump() if hasattr(artifact, "model_dump") else artifact.dict() for artifact in req.artifacts],
                formula=req.formula,
                metadata=req.metadata,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
            return _release_response_payload(request, result)
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.get("/v1/native/releases/{release_id}")
    def api_get_release(release_id: str, request: Request) -> dict:
        ctx = _ctx()
        try:
            release = get_release(ctx, release_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, release["repo_name"], "read")
            return _release_response_payload(request, release)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/releases/{release_ref}")
    def api_get_repo_release(repo_name: str, release_ref: str, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return _release_response_payload(request, get_release_for_repo(ctx, repo_name, release_ref))
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/releases/{release_id}/artifacts/{artifact_kind}")
    def api_read_release_artifact(release_id: str, artifact_kind: str, request: Request) -> Response:
        ctx = _ctx()
        try:
            release = get_release(ctx, release_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, release["repo_name"], "read")
            payload = read_release_artifact(ctx, release_id, artifact_kind)
            return _release_artifact_response(payload, release_id=release_id, artifact_kind=artifact_kind)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/changes/{change_id}/patchsets")
    def api_list_patchsets(change_id: str, request: Request) -> list[dict]:
        ctx = _ctx()
        try:
            change = get_change(ctx, change_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "read")
            return list_patchsets(ctx, change_id)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/changes/{change_ref}/patchsets")
    def api_list_repo_patchsets(repo_name: str, change_ref: str, request: Request) -> list[dict]:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return list_patchsets_for_repo(ctx, repo_name, change_ref)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.post("/v1/native/changes/{change_id}/patchsets")
    def api_publish_patchset(change_id: str, req: PatchsetPublish, request: Request) -> dict:
        ctx = _ctx()
        try:
            change = get_change(ctx, change_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "contribute")
            timings: dict[str, float] = {}

            phase_started = time.monotonic()
            result = publish_patchset(
                ctx,
                change_id,
                req.base_snapshot_id,
                req.revision_snapshot_id,
                req.summary,
                req.author_mode,
            )
            timings["publish_patchset_seconds"] = round(time.monotonic() - phase_started, 6)

            phase_started = time.monotonic()
            ci_state = _maybe_follow_patchset_publish_with_ci(ctx, result["patchset_id"])
            if isinstance(ci_state, dict):
                result.update(ci_state)
            timings["ci_followup_seconds"] = round(time.monotonic() - phase_started, 6)

            phase_started = time.monotonic()
            result.update(_patchset_publish_policy_followup(result["patchset_id"]))
            timings["policy_followup_seconds"] = round(time.monotonic() - phase_started, 6)

            phase_started = time.monotonic()
            result["notification_followup"] = _schedule_task_dag_telegram_notifications(
                ctx,
                change["repo_name"],
                event_type="patchset.published",
                entity_id=result["patchset_id"],
            )
            timings["notification_followup_seconds"] = round(time.monotonic() - phase_started, 6)
            result["publish_followup"] = {
                "queue_mode": _queue_mode(),
                "timings": timings,
                "phase_outcomes": {
                    "ci_followup": _publish_followup_phase_state(
                        result,
                        deferred_key="ci_followup",
                        queued_key="ci_job",
                        completed_key="ci_result",
                    ),
                    "policy_followup": _publish_followup_phase_state(
                        result,
                        deferred_key="policy_followup",
                        queued_key="policy_job",
                        completed_key="policy",
                    ),
                    "notification_followup": str((result.get("notification_followup") or {}).get("delivery") or "not_applicable"),
                },
                "request_path_audit": _patchset_publish_request_path_audit(result, timings),
                "total_seconds": round(sum(timings.values()), 6),
            }
            return result
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.get("/v1/native/patchsets/{patchset_id}")
    def api_get_patchset(patchset_id: str, request: Request) -> dict:
        ctx = _ctx()
        try:
            patchset = get_patchset(ctx, patchset_id)
            change = get_change(ctx, patchset["change_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "read")
            return patchset
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/patchsets/{patchset_ref}")
    def api_get_repo_patchset(repo_name: str, patchset_ref: str, request: Request, change_ref: str | None = None) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return get_patchset_for_repo(ctx, repo_name, patchset_ref, change_ref=change_ref)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.post("/v1/native/changes/{change_id}:selectPatchset")
    def api_select_patchset(change_id: str, req: SelectPatchsetRequest, request: Request) -> dict:
        ctx = _ctx()
        try:
            change = get_change(ctx, change_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "land")
            result = select_patchset(ctx, change_id, req.patchset_id)
            ci_state = _maybe_start_patchset_ci(ctx, req.patchset_id, trigger="patchset_select")
            if isinstance(ci_state, dict):
                result.update(ci_state)
            policy_job = _maybe_enqueue_policy(ctx, req.patchset_id)
            if policy_job is not None:
                result["policy_job"] = policy_job
            _trigger_task_dag_telegram_notifications(ctx, change["repo_name"], event_type="patchset.selected", entity_id=req.patchset_id)
            return result
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/patchsets/{patchset_id}:runCi")
    def api_run_patchset_ci(patchset_id: str, req: RunPatchsetCiRequest, request: Request) -> dict:
        ctx = _ctx()
        try:
            patchset = get_patchset(ctx, patchset_id)
            change = get_change(ctx, patchset["change_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "contribute")
            if _queue_mode() == "async":
                mark_patchset_ci_pending(ctx, patchset_id, trigger=req.trigger, job_state="queued")
                job = enqueue_async_job(
                    ctx,
                    change["repo_name"],
                    "patchset.ci",
                    _patchset_ci_job_payload(ctx, patchset_id),
                    max_attempts=3,
                    dedupe_active=True,
                )
                return {
                    "patchset_id": patchset_id,
                    "queued": True,
                    "job": job,
                    "trigger": req.trigger,
                }
            result = run_patchset_ci(ctx, patchset_id, trigger=req.trigger)
            return _attach_notification_followup(
                result,
                ctx,
                change["repo_name"],
                event_type="attestation.upserted",
                entity_id=patchset_id,
            )
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/changes/{change_id}:requestReview")
    def api_request_review(change_id: str, req: RequestReviewRequest, request: Request) -> dict:
        ctx = _ctx()
        try:
            change = get_change(ctx, change_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "contribute")
            result = request_review(ctx, change_id, req.patchset_id, req.reviewer_groups, req.note)
            _trigger_task_dag_telegram_notifications(ctx, change["repo_name"], event_type="review.requested", entity_id=change_id)
            return result
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.get("/v1/native/changes/{change_id}/reviews")
    def api_list_reviews(change_id: str, request: Request) -> dict:
        ctx = _ctx()
        try:
            change = get_change(ctx, change_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "read")
            return list_reviews(ctx, change_id)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.post("/v1/native/changes/{change_id}/reviews")
    def api_record_review(change_id: str, req: RecordReviewRequest, request: Request) -> dict:
        ctx = _ctx()
        try:
            change = get_change(ctx, change_id)
            actor = actor_from_request(request)
            ensure_review_action(ctx, actor, change["repo_name"], req.action, change.get("lane"))
            result = record_review(ctx, change_id, req.patchset_id, req.reviewer, req.action, req.comment, req.blocking)
            job = _maybe_enqueue_policy(ctx, req.patchset_id)
            if job is not None:
                result["policy_job"] = job
            return _attach_notification_followup(
                result,
                ctx,
                change["repo_name"],
                event_type="review.recorded",
                entity_id=change_id,
            )
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.put("/v1/native/patchsets/{patchset_id}/attestation")
    def api_put_attestation(patchset_id: str, req: UpsertAttestationRequest, request: Request) -> dict:
        ctx = _ctx()
        try:
            patchset = get_patchset(ctx, patchset_id)
            change = get_change(ctx, patchset["change_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "contribute")
            result = upsert_attestation(ctx, patchset_id, req.author_mode, req.evaluation_summary, req.provenance_summary, req.detail)
            job = _maybe_enqueue_policy(ctx, patchset_id)
            if job is not None:
                result["policy_job"] = job
            _trigger_task_dag_telegram_notifications(ctx, change["repo_name"], event_type="attestation.upserted", entity_id=patchset_id)
            return result
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/patchsets/{patchset_id}/attestation")
    def api_get_attestation(patchset_id: str, request: Request) -> dict:
        ctx = _ctx()
        try:
            patchset = get_patchset(ctx, patchset_id)
            change = get_change(ctx, patchset["change_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "read")
            return get_attestation(ctx, patchset_id)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.post("/v1/native/patchsets/{patchset_id}:evaluatePolicy")
    def api_evaluate_policy(patchset_id: str, request: Request) -> dict:
        ctx = _ctx()
        try:
            patchset = get_patchset(ctx, patchset_id)
            change = get_change(ctx, patchset["change_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "contribute")
            if _queue_mode() == "async":
                job = enqueue_async_job(
                    ctx,
                    change["repo_name"],
                    "policy.evaluate",
                    _policy_job_payload(ctx, patchset_id),
                    max_attempts=5,
                    dedupe_active=True,
                )
                return {"patchset_id": patchset_id, "decision": "pending", "queued": True, "job": job}
            result = evaluate_policy(ctx, patchset_id)
            return _attach_notification_followup(
                result,
                ctx,
                change["repo_name"],
                event_type="policy.evaluated",
                entity_id=patchset_id,
            )
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/patchsets/{patchset_id}/policy")
    def api_get_policy(patchset_id: str, request: Request) -> dict:
        ctx = _ctx()
        try:
            patchset = get_patchset(ctx, patchset_id)
            change = get_change(ctx, patchset["change_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "read")
            return get_policy_status(ctx, patchset_id)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.post("/v1/native/patchsets/{patchset_id}/waivers")
    def api_create_waiver(patchset_id: str, req: CreateWaiverRequest, request: Request) -> dict:
        ctx = _ctx()
        try:
            patchset = get_patchset(ctx, patchset_id)
            change = get_change(ctx, patchset["change_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "waive")
            result = create_waiver(ctx, patchset_id, req.rule_name, req.reason, req.expires_at, inline=_queue_mode() != "async")
            if _queue_mode() == "async":
                job = enqueue_async_job(
                    ctx,
                    change["repo_name"],
                    "policy.evaluate",
                    _policy_job_payload(ctx, patchset_id),
                    max_attempts=5,
                    dedupe_active=True,
                )
                result["policy_job"] = job
            _trigger_task_dag_telegram_notifications(ctx, change["repo_name"], event_type="policy.waived", entity_id=patchset_id)
            return result
        except ValueError as exc:
            raise _bad_request(exc) from exc
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.post("/v1/native/changes/{change_id}:submit")
    def api_submit_land(change_id: str, req: SubmitLandRequest, request: Request) -> dict:
        ctx = _ctx()
        try:
            change = get_change(ctx, change_id)
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "land")
            if _queue_mode() == "async":
                result = submit_land(ctx, change_id, req.patchset_id, req.target_line, req.mode, inline=False)
                job = _maybe_enqueue_land(ctx, result["submission_id"])
                if job is not None:
                    result["land_job"] = job
                return _attach_notification_followup(
                    result,
                    ctx,
                    change["repo_name"],
                    event_type="land.requested",
                    entity_id=result["submission_id"],
                )
            result = submit_land(ctx, change_id, req.patchset_id, req.target_line, req.mode, inline=True)
            return _attach_notification_followup(
                result,
                ctx,
                change["repo_name"],
                event_type="land.processed",
                entity_id=result["submission_id"],
            )
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/changes/{change_ref}:submit")
    def api_submit_repo_land(repo_name: str, change_ref: str, req: SubmitLandRequest, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "land")
        try:
            if _queue_mode() == "async":
                result = submit_land_for_repo(
                    ctx,
                    repo_name,
                    change_ref,
                    req.patchset_id,
                    req.target_line,
                    req.mode,
                    inline=False,
                )
                job = _maybe_enqueue_land(ctx, result["submission_id"])
                if job is not None:
                    result["land_job"] = job
                return _attach_notification_followup(
                    result,
                    ctx,
                    repo_name,
                    event_type="land.requested",
                    entity_id=result["submission_id"],
                )
            result = submit_land_for_repo(
                ctx,
                repo_name,
                change_ref,
                req.patchset_id,
                req.target_line,
                req.mode,
                inline=True,
            )
            return _attach_notification_followup(
                result,
                ctx,
                repo_name,
                event_type="land.processed",
                entity_id=result["submission_id"],
            )
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.get("/v1/native/lands/{submission_id}")
    def api_get_land(submission_id: str, request: Request) -> dict:
        ctx = _ctx()
        try:
            land = get_land_request(ctx, submission_id)
            change = get_change(ctx, land["change_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "read")
            return land
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/lands/{submission_ref}")
    def api_get_repo_land(repo_name: str, submission_ref: str, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return get_land_request_for_repo(ctx, repo_name, submission_ref)
        except KeyError as exc:
            raise _not_found(exc) from exc

    @app.post("/v1/native/lands/{submission_id}:retry")
    def api_retry_land(submission_id: str, req: RetryLandRequest, request: Request) -> dict:
        ctx = _ctx()
        try:
            land = get_land_request(ctx, submission_id)
            change = get_change(ctx, land["change_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "land")
            if _queue_mode() == "async":
                result = retry_land(ctx, submission_id, inline=False)
                job = _maybe_enqueue_land(ctx, submission_id)
                if job is not None:
                    result["land_job"] = job
                return result
            return retry_land(ctx, submission_id, inline=True)
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/lands/{submission_ref}:retry")
    def api_retry_repo_land(repo_name: str, submission_ref: str, req: RetryLandRequest, request: Request) -> dict:
        ctx = _ctx()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "land")
        try:
            land = get_land_request_for_repo(ctx, repo_name, submission_ref)
            if _queue_mode() == "async":
                result = retry_land(ctx, land["submission_id"], inline=False)
                job = _maybe_enqueue_land(ctx, land["submission_id"])
                if job is not None:
                    result["land_job"] = job
                return result
            return retry_land(ctx, land["submission_id"], inline=True)
        except KeyError as exc:
            raise _not_found(exc) from exc
        except ValueError as exc:
            raise _bad_request(exc) from exc

    return app



def main() -> None:
    port = int(os.environ.get("AIT_NATIVE_SERVER_PORT", "8088"))
    host = os.environ.get("AIT_NATIVE_SERVER_HOST", "127.0.0.1")
    repo_root = Path(os.environ.get("AIT_REPO_ROOT") or Path.cwd()).expanduser()
    runtime_ctx = ServerContext.from_env()
    prune_on_start = str(os.environ.get("AIT_CODEX_APP_SERVER_PRUNE_ON_START", "true")).strip().lower()
    if prune_on_start not in {"0", "false", "no", "off"}:
        try:
            pruned = prune_stale_managed_codex_app_servers(repo_root)
        except Exception as exc:  # pragma: no cover - defensive startup hardening
            print(f"ait-server warning: failed to prune stale Codex app-servers: {exc}", file=sys.stderr, flush=True)
        else:
            if pruned:
                print(
                    f"ait-server pruned {len(pruned)} stale AIT-managed Codex app-server process(es).",
                    file=sys.stderr,
                    flush=True,
                )
    print(
        f"ait-server runtime root: {runtime_ctx.root} (source={runtime_ctx.root_source})",
        file=sys.stderr,
        flush=True,
    )
    print(
        f"ait-server startup: {_server_startup_identity_summary()}",
        file=sys.stderr,
        flush=True,
    )
    config = uvicorn.Config("ait_server.app:create_app", factory=True, host=host, port=port, reload=False)
    server = _AitUvicornServer(config)
    with _server_runtime_identity():
        server.run()


if __name__ == "__main__":
    main()
