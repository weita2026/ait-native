from __future__ import annotations

from typing import Any, Callable

from fastapi import FastAPI, Request

from .route_request_models import StackChangeOp, StackCreate, StackUpdate
from .server_auth import actor_from_request, ensure_repo_action
from .server_store import (
    ServerContext,
    add_change_to_stack,
    create_stack,
    get_stack,
    get_stack_graph,
    list_stacks,
    remove_change_from_stack,
    reorder_stack_change,
    update_stack,
)

ErrorBuilder = Callable[[Exception], Exception]


def _stack_request_context(
    ctx: ServerContext,
    request: Request,
    stack_id: str,
    *,
    action: str,
) -> tuple[dict[str, Any], Any]:
    stack = get_stack(ctx, stack_id)
    actor = actor_from_request(request)
    ensure_repo_action(ctx, actor, stack["repo_name"], action)
    return stack, actor


def register_stack_routes(
    app: FastAPI,
    *,
    ctx_getter: Callable[[], ServerContext],
    not_found: ErrorBuilder,
    bad_request: ErrorBuilder,
) -> None:
    @app.post("/v1/native/repositories/{repo_name}/stacks")
    def api_create_stack(repo_name: str, req: StackCreate, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        try:
            return create_stack(ctx, repo_name, req.title, req.change_ids, req.landing_policy)
        except (KeyError, ValueError) as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/stacks")
    def api_list_stacks(repo_name: str, request: Request) -> list[dict]:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return list_stacks(ctx, repo_name)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/stacks/{stack_id}")
    def api_get_stack(stack_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            stack, _actor = _stack_request_context(ctx, request, stack_id, action="read")
            return stack
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.patch("/v1/native/stacks/{stack_id}")
    def api_update_stack(stack_id: str, req: StackUpdate, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            _stack, _actor = _stack_request_context(ctx, request, stack_id, action="contribute")
            return update_stack(ctx, stack_id, req.title, req.landing_policy, req.status)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/stacks/{stack_id}:addChange")
    def api_stack_add_change(stack_id: str, req: StackChangeOp, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            _stack, _actor = _stack_request_context(ctx, request, stack_id, action="contribute")
            return add_change_to_stack(ctx, stack_id, req.change_id, req.position)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/stacks/{stack_id}:removeChange")
    def api_stack_remove_change(stack_id: str, req: StackChangeOp, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            _stack, _actor = _stack_request_context(ctx, request, stack_id, action="contribute")
            return remove_change_from_stack(ctx, stack_id, req.change_id)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.post("/v1/native/stacks/{stack_id}:reorderChange")
    def api_stack_reorder_change(stack_id: str, req: StackChangeOp, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            _stack, _actor = _stack_request_context(ctx, request, stack_id, action="contribute")
            if req.position is None:
                raise ValueError("position is required for reorder")
            return reorder_stack_change(ctx, stack_id, req.change_id, req.position)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/stacks/{stack_id}/graph")
    def api_get_stack_graph(stack_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        try:
            _stack, _actor = _stack_request_context(ctx, request, stack_id, action="read")
            return get_stack_graph(ctx, stack_id)
        except KeyError as exc:
            raise not_found(exc) from exc
