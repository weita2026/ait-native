from __future__ import annotations

from typing import Any, Callable

from fastapi import FastAPI, Request
from pydantic import BaseModel

from .read_models import (
    change_detail,
    patchset_delta,
    repository_detail,
    repository_index,
    reviewer_inbox,
    stack_detail,
    task_audit,
    task_dag_graph,
    task_dag_progress,
    task_dag_readiness,
    task_dag_schedule,
    task_detail,
    task_queue,
)
from .read_models_domains.ci_status import patchset_ci_status, repository_ci_runs
from .server_auth import AuthzError, actor_from_request, ensure_repo_action
from .server_store import (
    ServerContext,
    get_change,
    get_change_for_repo,
    get_patchset,
    get_patchset_for_repo,
    get_stack,
    get_task,
    get_task_for_repo,
)


class TaskDagReadinessRequest(BaseModel):
    graph: dict[str, Any]
    current_plan_revision_id: str | None = None


ErrorBuilder = Callable[[Exception], Exception]


def register_read_routes(
    app: FastAPI,
    *,
    ctx_getter: Callable[[], ServerContext],
    not_found: ErrorBuilder,
    bad_request: ErrorBuilder,
) -> None:
    @app.get("/v1/native/read/reviewer-inbox")
    def api_read_reviewer_inbox(
        request: Request,
        repo_name: str | None = None,
        author_class: str | None = None,
        author_mode: str | None = None,
        tests: str | None = None,
        policy: str | None = None,
        freshness: str | None = None,
        review: str | None = None,
    ) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        if repo_name is not None:
            ensure_repo_action(ctx, actor, repo_name, "read")
        payload = reviewer_inbox(
            ctx,
            repo_name,
            author_class=author_class,
            author_mode=author_mode,
            tests=tests,
            policy=policy,
            freshness=freshness,
            review=review,
        )
        if repo_name is not None:
            payload["repo_name"] = repo_name
        return payload

    @app.get("/v1/native/read/task-queue")
    def api_read_task_queue(request: Request, repo_name: str | None = None, status: str | None = "active") -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        visible_repo_names: list[str] | None = None
        if repo_name is not None:
            ensure_repo_action(ctx, actor, repo_name, "read")
        else:
            visible_repo_names = []
            for repo in repository_index(ctx)["repositories"]:
                try:
                    ensure_repo_action(ctx, actor, repo["repo_name"], "read")
                except AuthzError:
                    continue
                visible_repo_names.append(repo["repo_name"])
        try:
            return task_queue(ctx, repo_name, status=status, repo_names=visible_repo_names)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/read/queue-summary")
    def api_read_queue_summary(request: Request, repo_name: str, status: str | None = "active") -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            cache: dict[str, dict[Any, Any]] = {}
            task_queue_payload = task_queue(ctx, repo_name, status=status, cache=cache)
            reviewer_inbox_payload = reviewer_inbox(ctx, repo_name, cache=cache)
            return {
                "task_queue": task_queue_payload,
                "reviewer_inbox": reviewer_inbox_payload,
            }
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/read/task-dag-readiness")
    def api_read_task_dag_readiness(req: TaskDagReadinessRequest, request: Request) -> dict:
        ctx = ctx_getter()
        repo_name = req.graph.get("repo_name")
        if not isinstance(repo_name, str) or not repo_name.strip():
            raise bad_request(ValueError("Task DAG graph must include repo_name for API reads."))
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name.strip(), "read")
        try:
            return task_dag_readiness(ctx, req.graph, current_plan_revision_id=req.current_plan_revision_id)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/read/task-dag-graph")
    def api_read_task_dag_graph(req: TaskDagReadinessRequest, request: Request) -> dict:
        ctx = ctx_getter()
        repo_name = req.graph.get("repo_name")
        if not isinstance(repo_name, str) or not repo_name.strip():
            raise bad_request(ValueError("Task DAG graph must include repo_name for API reads."))
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name.strip(), "read")
        try:
            return task_dag_graph(ctx, req.graph, current_plan_revision_id=req.current_plan_revision_id)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/read/task-dag-schedule")
    def api_read_task_dag_schedule(req: TaskDagReadinessRequest, request: Request) -> dict:
        ctx = ctx_getter()
        repo_name = req.graph.get("repo_name")
        if not isinstance(repo_name, str) or not repo_name.strip():
            raise bad_request(ValueError("Task DAG graph must include repo_name for API reads."))
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name.strip(), "read")
        try:
            return task_dag_schedule(ctx, req.graph, current_plan_revision_id=req.current_plan_revision_id)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.post("/v1/native/read/task-dag-progress")
    def api_read_task_dag_progress(req: TaskDagReadinessRequest, request: Request) -> dict:
        ctx = ctx_getter()
        repo_name = req.graph.get("repo_name")
        if not isinstance(repo_name, str) or not repo_name.strip():
            raise bad_request(ValueError("Task DAG graph must include repo_name for API reads."))
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name.strip(), "read")
        try:
            return task_dag_progress(ctx, req.graph, current_plan_revision_id=req.current_plan_revision_id)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/read/repositories")
    def api_read_repository_index(request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        if actor.mode == "strict":
            _ = actor.identity
        return repository_index(ctx)

    @app.get("/v1/native/read/repositories/{repo_name}")
    def api_read_repository_detail(repo_name: str, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return repository_detail(ctx, repo_name)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/read/repositories/{repo_name}/ci-runs")
    def api_read_repository_ci_runs(
        repo_name: str,
        request: Request,
        limit: int = 20,
        plane: str | None = None,
        suite_id: str | None = None,
    ) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            return repository_ci_runs(ctx, repo_name, limit=limit, plane=plane, suite_id=suite_id)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/read/patchsets/{patchset_id}/ci-status")
    def api_read_patchset_ci_status(patchset_id: str, request: Request, recent_limit: int = 10) -> dict:
        ctx = ctx_getter()
        try:
            patchset = get_patchset(ctx, patchset_id)
            change = get_change(ctx, patchset["change_id"])
            actor = actor_from_request(request)
            ensure_repo_action(ctx, actor, change["repo_name"], "read")
            return patchset_ci_status(ctx, patchset_id, recent_limit=recent_limit)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/read/changes/{change_id}")
    def api_read_change_detail(change_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        change = get_change(ctx, change_id)
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, change["repo_name"], "read")
        return change_detail(ctx, change_id)

    @app.get("/v1/native/repositories/{repo_name}/read/changes/{change_ref}")
    def api_read_repo_change_detail(repo_name: str, change_ref: str, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            change = get_change_for_repo(ctx, repo_name, change_ref)
            return change_detail(ctx, change["change_id"])
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/read/tasks/{task_id}")
    def api_read_task_detail(task_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        task = get_task(ctx, task_id)
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, task["repo_name"], "read")
        return task_detail(ctx, task_id)

    @app.get("/v1/native/repositories/{repo_name}/read/tasks/{task_ref}")
    def api_read_repo_task_detail(repo_name: str, task_ref: str, request: Request) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            task = get_task_for_repo(ctx, repo_name, task_ref)
            return task_detail(ctx, task["task_id"])
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/read/tasks/{task_id}/audit")
    def api_read_task_audit(task_id: str, request: Request, target_line: str = "main") -> dict:
        ctx = ctx_getter()
        try:
            task = get_task(ctx, task_id)
        except KeyError as exc:
            raise not_found(exc) from exc
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, task["repo_name"], "read")
        return task_audit(ctx, task_id, target_line=target_line)

    @app.get("/v1/native/repositories/{repo_name}/read/tasks/{task_ref}/audit")
    def api_read_repo_task_audit(repo_name: str, task_ref: str, request: Request, target_line: str = "main") -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            task = get_task_for_repo(ctx, repo_name, task_ref)
            return task_audit(ctx, task["task_id"], target_line=target_line)
        except KeyError as exc:
            raise not_found(exc) from exc

    @app.get("/v1/native/read/patchsets/{patchset_id}/delta")
    def api_read_patchset_delta(patchset_id: str, request: Request, against: str = "previous") -> dict:
        ctx = ctx_getter()
        patchset = get_patchset(ctx, patchset_id)
        change = get_change(ctx, patchset["change_id"])
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, change["repo_name"], "read")
        try:
            return patchset_delta(ctx, patchset_id, against)
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/repositories/{repo_name}/read/patchsets/{patchset_ref}/delta")
    def api_read_repo_patchset_delta(
        repo_name: str,
        patchset_ref: str,
        request: Request,
        against: str = "previous",
        change_ref: str | None = None,
    ) -> dict:
        ctx = ctx_getter()
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, repo_name, "read")
        try:
            patchset = get_patchset_for_repo(ctx, repo_name, patchset_ref, change_ref=change_ref)
            return patchset_delta(ctx, patchset["patchset_id"], against)
        except KeyError as exc:
            raise not_found(exc) from exc
        except ValueError as exc:
            raise bad_request(exc) from exc

    @app.get("/v1/native/read/stacks/{stack_id}")
    def api_read_stack_detail(stack_id: str, request: Request) -> dict:
        ctx = ctx_getter()
        stack = get_stack(ctx, stack_id)
        actor = actor_from_request(request)
        ensure_repo_action(ctx, actor, stack["repo_name"], "read")
        return stack_detail(ctx, stack_id)
