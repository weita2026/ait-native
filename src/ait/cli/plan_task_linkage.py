from __future__ import annotations

from pathlib import Path
from typing import Any

from .. import local_control
from ..plan_graph import load_task_graph
from ..repo_paths import RepoContext
from ..task_dag_conversation import discover_task_dag_graph_paths
from ..store import get_local_plan, get_local_plan_revision
from .workflow_mode_config import (
    _effective_plan_task_binding,
    _task_dag_multi_worker_allowed,
    _normalize_text_value,
    _plan_task_binding_plan_item_error,
)


def _normalize_plan_task_linkage(
    ctx: RepoContext,
    *,
    plan_id: str | None = None,
    plan_revision_id: str | None = None,
    plan_item_ref: str | None = None,
    local: bool = False,
    require_execution_binding: bool = False,
    guard_dag_task_bootstrap: bool = False,
) -> tuple[str | None, str | None, str | None]:
    del local  # retained for CLI call-site compatibility while extraction stays behavior-neutral
    resolved_plan_id = _normalize_text_value(plan_id)
    resolved_revision_id = _normalize_text_value(plan_revision_id)
    resolved_plan_item_ref = _normalize_text_value(plan_item_ref)
    mode = _effective_plan_task_binding(ctx)["mode"]
    if resolved_plan_id is None and resolved_revision_id is None:
        if resolved_plan_item_ref is not None:
            raise ValueError("`--plan-item-ref` requires `--plan` or `--revision`.")
        if require_execution_binding and mode == "required":
            raise ValueError(
                "Required plan/task binding requires `--plan` (or `--revision`) and `--plan-item-ref` for execution tasks."
            )
        return None, None, None
    if mode in {"strict", "required"} and resolved_plan_item_ref is None:
        raise ValueError(_plan_task_binding_plan_item_error(mode))
    if guard_dag_task_bootstrap:
        _guard_task_dag_plan_item_task_bootstrap(
            ctx,
            plan_id=resolved_plan_id,
            plan_revision_id=resolved_revision_id,
            plan_item_ref=resolved_plan_item_ref,
        )
    return resolved_plan_id, resolved_revision_id, resolved_plan_item_ref


def _guard_task_dag_plan_item_task_bootstrap(
    ctx: RepoContext,
    *,
    plan_id: str | None,
    plan_revision_id: str | None,
    plan_item_ref: str | None,
) -> None:
    if plan_id is None or plan_item_ref is None or _task_dag_multi_worker_allowed(ctx):
        return
    match = _task_dag_node_for_plan_item_ref(
        ctx,
        plan_id=plan_id,
        plan_revision_id=plan_revision_id,
        plan_item_ref=plan_item_ref,
    )
    if match is None:
        return
    graph_id, graph_path, node_id = match
    raise ValueError(
        f"Plan item ref {plan_item_ref!r} on plan {plan_id} is owned by task DAG node {node_id} in {graph_path} "
        f"({graph_id}). Compact DAG stays on one fresh worker session by default, so manual task bootstrap is blocked. "
        f"Use `ait plan execute {plan_id} --from-json {graph_path} --auto-compact-worker --yes`. "
        "If you intentionally need multi-worker DAG fan-out, first run "
        "`ait config set --task-dag-allow-multi-worker on`."
    )


def _task_dag_node_for_plan_item_ref(
    ctx: RepoContext,
    *,
    plan_id: str,
    plan_revision_id: str | None,
    plan_item_ref: str,
) -> tuple[str, str, str] | None:
    for candidate in discover_task_dag_graph_paths(ctx.root):
        try:
            graph = load_task_graph(candidate)
        except ValueError:
            continue
        source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
        graph_plan_id = _normalize_text_value(source_plan.get("plan_id"))
        if graph_plan_id != plan_id:
            continue
        graph_plan_revision_id = _normalize_text_value(source_plan.get("plan_revision_id"))
        if plan_revision_id is not None and graph_plan_revision_id is not None and graph_plan_revision_id != plan_revision_id:
            continue
        for node in graph.get("nodes", []):
            if not isinstance(node, dict):
                continue
            if _normalize_text_value(node.get("plan_item_ref")) != plan_item_ref:
                continue
            graph_id = _normalize_text_value(graph.get("graph_id")) or "task-dag"
            node_id = _normalize_text_value(node.get("node_id")) or "unknown"
            return graph_id, _repo_relative_path(ctx, candidate), node_id
    return None


def _repo_relative_path(ctx: RepoContext, path: Path) -> str:
    try:
        return str(path.resolve().relative_to(ctx.root.resolve()))
    except ValueError:
        return str(path)


def _published_local_task_plan_linkage(ctx: RepoContext, task: dict[str, Any]) -> tuple[str | None, str | None, str | None]:
    resolved_plan_id = _normalize_text_value(task.get("plan_id"))
    resolved_revision_id = _normalize_text_value(task.get("origin_plan_revision_id"))
    resolved_plan_item_ref = _normalize_text_value(task.get("plan_item_ref"))
    mode = str(_effective_plan_task_binding(ctx).get("mode") or "")
    if resolved_plan_id is None and resolved_revision_id is None:
        if mode == "required":
            raise ValueError(
                "Required plan/task binding requires local draft tasks to carry durable plan linkage before remote publish."
            )
        if resolved_plan_item_ref is not None:
            raise ValueError("Local task plan metadata is incomplete: `plan_item_ref` requires plan linkage.")
        return None, None, None
    if mode in {"strict", "required"} and resolved_plan_item_ref is None:
        raise ValueError(_plan_task_binding_plan_item_error(mode))
    if resolved_plan_id is None:
        assert resolved_revision_id is not None
        revision_row = local_control.get_workflow_plan_revision_by_id(ctx, resolved_revision_id)
        resolved_plan_id = _normalize_text_value(revision_row.get("plan_id"))
    if resolved_plan_id is None:
        raise ValueError("Local task plan metadata is incomplete: missing `plan_id`.")
    local_plan = get_local_plan(ctx, resolved_plan_id)
    published_plan_id = _normalize_text_value(local_plan.get("published_plan_id"))
    if published_plan_id is None:
        raise ValueError(f"Local task {task['task_id']} is linked to unpublished local plan {resolved_plan_id}. Publish the plan first.")
    if resolved_revision_id is None:
        resolved_revision_id = _normalize_text_value(local_plan.get("head_revision_id"))
    if resolved_revision_id is None:
        raise ValueError(f"Local task {task['task_id']} is linked to local plan {resolved_plan_id} without a stored revision id.")
    local_revision = get_local_plan_revision(ctx, resolved_plan_id, resolved_revision_id)
    published_revision_id = _normalize_text_value(local_revision.get("published_plan_revision_id"))
    if published_revision_id is None:
        raise ValueError(
            f"Local task {task['task_id']} is linked to unpublished local plan revision {resolved_revision_id}. Publish the plan revision first."
        )
    return published_plan_id, published_revision_id, resolved_plan_item_ref
