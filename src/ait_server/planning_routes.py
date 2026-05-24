from __future__ import annotations

from typing import Any, Callable

from fastapi import FastAPI, Request
from pydantic import BaseModel, Field

from .server_auth import actor_from_request, ensure_repo_action
from .server_store import (
    ServerContext,
    append_planning_session_event,
    close_planning_session,
    create_plan,
    create_planning_session,
    get_plan,
    get_plan_revision,
    get_planning_session,
    join_planning_session,
    list_plan_revisions,
    list_planning_session_events,
    list_planning_sessions,
    list_plans,
    promote_planning_session,
    put_plan_revision_artifacts,
    revise_plan,
    update_plan_status,
)
from .session_route_helpers import _session_with_telegram_runtime_state


class PlanCreate(BaseModel):
    plan_id: str | None = None
    title: str
    artifact_path: str
    artifact_selector: str | None = None
    artifact_heading: str
    items: list[dict[str, Any]] = Field(default_factory=list)
    summary: str | None = None
    status: str = "draft"
    source_kind: str = "manual_edit"
    source_session_id: str | None = None
    artifact_body: str | None = None


class PlanRevisionCreate(BaseModel):
    title: str | None = None
    artifact_path: str
    artifact_selector: str | None = None
    artifact_heading: str
    items: list[dict[str, Any]] = Field(default_factory=list)
    summary: str | None = None
    source_kind: str = "manual_edit"
    source_session_id: str | None = None
    artifact_body: str | None = None
    expected_head_revision_id: str | None = None


class PlanRevisionArtifactPutItem(BaseModel):
    artifact_path: str
    role: str = "supporting_artifact"
    media_type: str = "application/octet-stream"
    encoding: str | None = "utf-8"
    body: str
    metadata: dict[str, Any] = Field(default_factory=dict)


class PlanRevisionArtifactsPut(BaseModel):
    artifacts: list[PlanRevisionArtifactPutItem] = Field(default_factory=list)


class PlanUpdate(BaseModel):
    status: str


class PlanningSessionCreate(BaseModel):
    planning_session_id: str | None = None
    title: str | None = None
    mode: str = "connected_local"
    preferred_agent: str | None = None
    resume_if_active: bool = True


class PlanningSessionEventAppend(BaseModel):
    event_type: str
    payload: dict[str, Any] = Field(default_factory=dict)


class PlanningSessionPromote(BaseModel):
    artifact_path: str
    artifact_selector: str
    artifact_heading: str
    items: list[dict[str, Any]] = Field(default_factory=list)
    title: str | None = None
    summary: str | None = None
    artifact_body: str | None = None


class PlanningSessionCloseRequest(BaseModel):
    status: str = "closed"


class PlanningSessionJoinRequest(BaseModel):
    surface: str = "cli"
    title: str | None = None
    model_name: str | None = None
    resume_if_active: bool = True
    session_id: str | None = None


ErrorBuilder = Callable[[Exception], Exception]


def _model_dump_compatible(value: Any) -> Any:
    if hasattr(value, "model_dump"):
        return value.model_dump()
    if hasattr(value, "dict"):
        return value.dict()
    return value


def _plan_request_context(
    ctx: ServerContext,
    request: Request,
    plan_id: str,
    *,
    action: str,
) -> tuple[dict[str, Any], Any]:
    plan = get_plan(ctx, plan_id)
    actor = actor_from_request(request)
    ensure_repo_action(ctx, actor, plan["repo_name"], action)
    return plan, actor


def _planning_session_request_context(
    ctx: ServerContext,
    request: Request,
    planning_session_id: str,
    *,
    action: str,
) -> tuple[dict[str, Any], Any]:
    planning_session = get_planning_session(ctx, planning_session_id)
    actor = actor_from_request(request)
    ensure_repo_action(ctx, actor, planning_session["repo_name"], action)
    return planning_session, actor


def register_planning_routes(
    app: FastAPI,
    *,
    ctx_getter: Callable[[], ServerContext],
    not_found: ErrorBuilder,
    bad_request: ErrorBuilder,
    conflict: ErrorBuilder,
) -> None:
    @app.post("/v1/native/repositories/{repo_name}/sprints")
    def api_create_plan(repo_name: str, req: PlanCreate, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            return create_plan(
                ctx,
                repo_name,
                req.title,
                req.artifact_path,
                req.artifact_selector,
                req.artifact_heading,
                req.items,
                summary=req.summary,
                status=req.status,
                plan_id=req.plan_id,
                source_kind=req.source_kind,
                source_session_id=req.source_session_id,
                artifact_body=req.artifact_body,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise conflict(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/sprints")
    def api_list_plans(repo_name: str, request: Request) -> list[dict]:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            artifact_path = request.query_params.get("artifact_path")
            return list_plans(ctx, repo_name, artifact_path=artifact_path)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/sprints/{plan_id}")
    def api_get_plan(plan_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            plan, _actor = _plan_request_context(ctx, request, plan_id, action="read")
            return plan
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.patch("/v1/native/sprints/{plan_id}")
    def api_update_plan(plan_id: str, req: PlanUpdate, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            _plan, actor = _plan_request_context(ctx, request, plan_id, action="contribute")
            return update_plan_status(
                ctx,
                plan_id,
                req.status,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/sprints/{plan_id}/revisions")
    def api_list_plan_revisions(plan_id: str, request: Request) -> list[dict]:
        ctx = ctx_getter()
        try:
            _plan, _actor = _plan_request_context(ctx, request, plan_id, action="read")
            return list_plan_revisions(ctx, plan_id)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/sprints/{plan_id}/revisions/{plan_revision_id}")
    def api_get_plan_revision(plan_id: str, plan_revision_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            _plan, _actor = _plan_request_context(ctx, request, plan_id, action="read")
            return get_plan_revision(ctx, plan_id, plan_revision_id)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.put("/v1/native/sprints/{plan_id}/revisions/{plan_revision_id}/artifacts")
    def api_put_plan_revision_artifacts(
        plan_id: str,
        plan_revision_id: str,
        req: PlanRevisionArtifactsPut,
        request: Request,
    ) -> dict:
        ctx = ctx_getter()
        try:
            _plan, actor = _plan_request_context(ctx, request, plan_id, action="contribute")
            return put_plan_revision_artifacts(
                ctx,
                plan_id,
                plan_revision_id,
                [_model_dump_compatible(item) for item in req.artifacts],
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/sprints/{plan_id}/revisions")
    def api_revise_plan(plan_id: str, req: PlanRevisionCreate, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            _plan, actor = _plan_request_context(ctx, request, plan_id, action="contribute")
            return revise_plan(
                ctx,
                plan_id,
                req.artifact_path,
                req.artifact_selector,
                req.artifact_heading,
                req.items,
                title=req.title,
                summary=req.summary,
                source_kind=req.source_kind,
                source_session_id=req.source_session_id,
                artifact_body=req.artifact_body,
                expected_head_revision_id=req.expected_head_revision_id,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            if "head advanced" in str(exc):
                raise conflict(exc) from exc
            raise bad_request(exc) from exc

    @app.post("/v1/native/sprints/{plan_id}/planning-sessions")
    def api_create_planning_session(plan_id: str, req: PlanningSessionCreate, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            _plan, actor = _plan_request_context(ctx, request, plan_id, action="contribute")
            return create_planning_session(
                ctx,
                plan_id,
                title=req.title,
                mode=req.mode,
                preferred_agent=req.preferred_agent,
                resume_if_active=req.resume_if_active,
                planning_session_id=req.planning_session_id,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            if "already exists" in str(exc):
                raise conflict(exc) from exc
            raise bad_request(exc) from exc

    @app.get("/v1/native/sprints/{plan_id}/planning-sessions")
    def api_list_planning_sessions(plan_id: str, request: Request, status: str | None = None) -> list[dict]:
        ctx = ctx_getter()
        try:
            _plan, _actor = _plan_request_context(ctx, request, plan_id, action="read")
            return list_planning_sessions(ctx, plan_id, status=status)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/planning-sessions/{planning_session_id}")
    def api_get_planning_session(planning_session_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            planning_session, _actor = _planning_session_request_context(
                ctx,
                request,
                planning_session_id,
                action="read",
            )
            return planning_session
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.post("/v1/native/planning-sessions/{planning_session_id}/events")
    def api_append_planning_session_event(
        planning_session_id: str,
        req: PlanningSessionEventAppend,
        request: Request,
    ) -> dict:
        ctx = ctx_getter()
        try:
            _planning_session, actor = _planning_session_request_context(
                ctx,
                request,
                planning_session_id,
                action="contribute",
            )
            return append_planning_session_event(
                ctx,
                planning_session_id,
                req.event_type,
                req.payload,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/planning-sessions/{planning_session_id}/events")
    def api_list_planning_session_events(
        planning_session_id: str,
        request: Request,
        after_sequence: int = 0,
        limit: int = 200,
    ) -> list[dict]:
        ctx = ctx_getter()
        try:
            _planning_session, _actor = _planning_session_request_context(
                ctx,
                request,
                planning_session_id,
                action="read",
            )
            return list_planning_session_events(
                ctx,
                planning_session_id,
                after_sequence=after_sequence,
                limit=limit,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/planning-sessions/{planning_session_id}:join")
    def api_join_planning_session(
        planning_session_id: str,
        req: PlanningSessionJoinRequest,
        request: Request,
    ) -> dict:
        ctx = ctx_getter()
        try:
            _planning_session, actor = _planning_session_request_context(
                ctx,
                request,
                planning_session_id,
                action="contribute",
            )
            joined = join_planning_session(
                ctx,
                planning_session_id,
                surface=req.surface,
                title=req.title,
                model_name=req.model_name,
                resume_if_active=req.resume_if_active,
                session_id=req.session_id,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
            return {
                "planning_session": joined["planning_session"],
                "session": _session_with_telegram_runtime_state(ctx, joined["session"]),
            }
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            if "already exists" in str(exc):
                raise conflict(exc) from exc
            raise bad_request(exc) from exc

    @app.post("/v1/native/planning-sessions/{planning_session_id}:promote")
    def api_promote_planning_session(planning_session_id: str, req: PlanningSessionPromote, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            _planning_session, actor = _planning_session_request_context(
                ctx,
                request,
                planning_session_id,
                action="contribute",
            )
            return promote_planning_session(
                ctx,
                planning_session_id,
                req.artifact_path,
                req.artifact_selector,
                req.artifact_heading,
                req.items,
                title=req.title,
                summary=req.summary,
                artifact_body=req.artifact_body,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/planning-sessions/{planning_session_id}:close")
    def api_close_planning_session(
        planning_session_id: str,
        req: PlanningSessionCloseRequest,
        request: Request,
    ) -> dict:
        ctx = ctx_getter()
        try:
            _planning_session, actor = _planning_session_request_context(
                ctx,
                request,
                planning_session_id,
                action="contribute",
            )
            return close_planning_session(
                ctx,
                planning_session_id,
                status=req.status,
                actor_identity=actor.identity,
                actor_type=actor.actor_type,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc


__all__ = [
    "PlanCreate",
    "PlanRevisionArtifactPutItem",
    "PlanRevisionArtifactsPut",
    "PlanRevisionCreate",
    "PlanUpdate",
    "PlanningSessionCloseRequest",
    "PlanningSessionCreate",
    "PlanningSessionEventAppend",
    "PlanningSessionJoinRequest",
    "PlanningSessionPromote",
    "register_planning_routes",
]
