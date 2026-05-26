from __future__ import annotations

from typing import Any, Callable

from fastapi import FastAPI, HTTPException, Request

from .route_request_models import LineCloseRequest, LineUpdate, RepositoryCreate, SnapshotExistsRequest
from .server_content import RepositoryNamespacePrefixConflictError
from .server_auth import actor_from_request, ensure_admin_action, ensure_line_update, ensure_repo_action
from .server_store import (
    ServerContext,
    close_line,
    ensure_repository,
    export_snapshot,
    get_line,
    get_repository,
    import_snapshot,
    list_lines,
    snapshot_existence,
    update_line,
)

ErrorBuilder = Callable[[Exception], Exception]


def register_repository_routes(
    app: FastAPI,
    *,
    ctx_getter: Callable[[], ServerContext],
    not_found: ErrorBuilder,
    bad_request: ErrorBuilder,
    conflict: ErrorBuilder,
) -> None:
    @app.post("/v1/native/repositories")
    def api_create_repo(req: RepositoryCreate, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_admin_action(ctx, actor, req.repo_name)
        try:
            return ensure_repository(
                ctx,
                req.repo_name,
                req.default_line,
                policy=req.policy,
                id_namespace_prefix=req.id_namespace_prefix,
            )
        except RepositoryNamespacePrefixConflictError as exc:
            raise conflict(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}")
    def api_get_repo(repo_name: str, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return get_repository(ctx, repo_name)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/lines")
    def api_list_repo_lines(repo_name: str, request: Request) -> list[dict]:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return list_lines(ctx, repo_name)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/lines/{line_name:path}")
    def api_get_repo_line(repo_name: str, line_name: str, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return get_line(ctx, repo_name, line_name)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.put("/v1/native/repositories/{repo_name}/lines/{line_name:path}")
    def api_put_repo_line(repo_name: str, line_name: str, req: LineUpdate, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        try:
            repo = get_repository(ctx, repo_name)
            ensure_line_update(ctx, actor, repo_name, line_name, repo["default_line"])
            return update_line(
                ctx,
                repo_name,
                line_name,
                req.head_snapshot_id,
                expected_head_snapshot_id=req.expected_head_snapshot_id,
            )
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/lines/{line_name:path}:close")
    def api_close_repo_line(repo_name: str, line_name: str, req: LineCloseRequest, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        try:
            repo = get_repository(ctx, repo_name)
            ensure_line_update(ctx, actor, repo_name, line_name, repo["default_line"])
            return close_line(ctx, repo_name, line_name, req.status)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/repositories/{repo_name}/snapshots:exists")
    def api_snapshots_exist(repo_name: str, req: SnapshotExistsRequest, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return snapshot_existence(ctx, repo_name, req.snapshot_ids)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.put("/v1/native/repositories/{repo_name}/snapshots/{snapshot_id}")
    def api_put_snapshot(repo_name: str, snapshot_id: str, bundle: dict[str, Any], request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "contribute")
        if bundle.get("snapshot_id") != snapshot_id:
            raise HTTPException(status_code=400, detail="snapshot_id path/body mismatch")
        try:
            return import_snapshot(ctx, repo_name, bundle)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/snapshots/{snapshot_id}")
    def api_get_snapshot(
        repo_name: str,
        snapshot_id: str,
        request: Request,
        include_content: bool = True,
        path: str | None = None,
    ) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return export_snapshot(ctx, repo_name, snapshot_id, include_content=include_content, path=path)
        except KeyError as exc:
            raise not_found(exc) from exc
