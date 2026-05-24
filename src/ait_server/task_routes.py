from __future__ import annotations

from typing import Any, Callable

from fastapi import FastAPI, Request

from .route_request_models import (
    TaskCloseRequest,
    TaskCreate,
    TaskTrackingBackfillRequest,
    TaskTrackingEnsureRequest,
)
from .server_auth import actor_from_request, ensure_repo_action
from .server_store import (
    ServerContext,
    backfill_task_tracking_sessions,
    close_task,
    create_task,
    ensure_task_tracking_session,
    get_task,
    get_task_for_repo,
    list_tasks,
    restart_task,
)

ErrorBuilder = Callable[[Exception], Exception]
NotificationFollowupBuilder = Callable[..., dict[str, Any]]
TaskNotificationTrigger = Callable[..., None]


def _task_request_context(
    ctx: ServerContext,
    request: Request,
    task_id: str,
    *,
    action: str,
) -> tuple[dict[str, Any], Any]:
    task = get_task(ctx, task_id)
    actor = actor_from_request(request)
    ensure_repo_action(ctx, actor, task["repo_name"], action)
    return task, actor


def register_task_routes(
    app: FastAPI,
    *,
    ctx_getter: Callable[[], ServerContext],
    not_found: ErrorBuilder,
    bad_request: ErrorBuilder,
    conflict: ErrorBuilder,
    attach_notification_followup: NotificationFollowupBuilder,
    trigger_task_notification: TaskNotificationTrigger,
) -> None:
    @app.post("/v1/native/repositories/{repo_name}/tasks")
    def api_create_task(repo_name: str, req: TaskCreate, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            result = create_task(
                ctx,
                repo_name,
                req.title,
                req.intent,
                req.risk_tier,
                task_id=req.task_id,
                plan_id=req.plan_id,
                origin_plan_revision_id=req.origin_plan_revision_id,
                plan_item_ref=req.plan_item_ref,
                tracking_session=req.tracking_session,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
            trigger_task_notification(ctx, repo_name, event_type="task.created", entity_id=result["task_id"])
            return result
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise conflict(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/tasks:backfill-sessions")
    def api_backfill_task_tracking_sessions(repo_name: str, req: TaskTrackingBackfillRequest, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            return backfill_task_tracking_sessions(
                ctx,
                repo_name,
                task_ref=req.task_id,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/tasks/{task_ref}:ensure-session")
    def api_ensure_task_tracking_session(
        repo_name: str,
        task_ref: str,
        req: TaskTrackingEnsureRequest,
        request: Request,
    ) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            return ensure_task_tracking_session(
                ctx,
                repo_name,
                task_ref,
                tracking_session=req.tracking_session,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
                provisioned_by="task.ensure_tracking_session",
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/tasks")
    def api_list_tasks(repo_name: str, request: Request) -> list[dict]:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return list_tasks(ctx, repo_name)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/tasks/{task_ref}")
    def api_get_repo_task(repo_name: str, task_ref: str, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return get_task_for_repo(ctx, repo_name, task_ref)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/tasks/{task_id}")
    def api_get_task(task_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            task, _actor = _task_request_context(ctx, request, task_id, action="read")
            return task
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.post("/v1/native/tasks/{task_id}:close")
    def api_close_task(task_id: str, req: TaskCloseRequest, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            task, _actor = _task_request_context(ctx, request, task_id, action="contribute")
            result = close_task(ctx, task_id, req.status)
            return attach_notification_followup(
                result,
                ctx,
                task["repo_name"],
                event_type="task.closed",
                entity_id=task_id,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/tasks/{task_id}:restart")
    def api_restart_task(task_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            task, _actor = _task_request_context(ctx, request, task_id, action="contribute")
            result = restart_task(ctx, task_id)
            return attach_notification_followup(
                result,
                ctx,
                task["repo_name"],
                event_type="task.restarted",
                entity_id=task_id,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc
