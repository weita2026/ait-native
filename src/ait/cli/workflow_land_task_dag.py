from __future__ import annotations

from pathlib import Path
from typing import Any

from ..plan_graph import load_task_graph
from ..remote_client import create_session as remote_create_session
from ..remote_client import list_sessions as remote_list_sessions
from ..store import RepoContext
from ..store_local_sessions import list_local_sessions
from ..task_dag_readiness import task_dag_final_output_later_remote_promotion_allowed
from .plan_task_linkage import _task_dag_node_for_plan_item_ref
from .runtime_defaults import _effective_model_name, _normalize_text_value


def _workflow_batch_task_dag_entry_metadata(
    ctx: RepoContext,
    *,
    plan_id: str | None,
    plan_revision_id: str | None,
    plan_item_ref: str | None,
) -> dict[str, Any] | None:
    resolved_plan_id = _normalize_text_value(plan_id)
    resolved_plan_revision_id = _normalize_text_value(plan_revision_id)
    resolved_plan_item_ref = _normalize_text_value(plan_item_ref)
    if resolved_plan_id is None or resolved_plan_item_ref is None:
        return None
    match = _task_dag_node_for_plan_item_ref(
        ctx,
        plan_id=resolved_plan_id,
        plan_revision_id=resolved_plan_revision_id,
        plan_item_ref=resolved_plan_item_ref,
    )
    if match is None and resolved_plan_revision_id is not None:
        match = _task_dag_node_for_plan_item_ref(
            ctx,
            plan_id=resolved_plan_id,
            plan_revision_id=None,
            plan_item_ref=resolved_plan_item_ref,
        )
    if match is None:
        return None
    graph_id, graph_path, node_id = match
    graph_file = Path(graph_path)
    if not graph_file.is_absolute():
        graph_file = ctx.root / graph_file
    graph = load_task_graph(graph_file)
    graph_node = next(
        (
            node
            for node in graph.get("nodes", [])
            if isinstance(node, dict) and str(node.get("node_id") or "").strip() == node_id
        ),
        None,
    )
    if not isinstance(graph_node, dict):
        return None
    workflow_boundary = str(graph_node.get("workflow_boundary") or "reviewable_output").strip().lower() or "reviewable_output"
    return {
        "graph_id": graph_id,
        "graph_path": str(graph_path),
        "node_id": node_id,
        "workflow_boundary": workflow_boundary,
        "later_remote_promotion_allowed": bool(
            task_dag_final_output_later_remote_promotion_allowed(graph)
        ),
    }


def _workflow_batch_local_task_dag_session_row(
    ctx: RepoContext,
    *,
    task_id: str,
    change_id: str,
) -> dict[str, Any] | None:
    candidate_rows: list[dict[str, Any]] = []
    for row in list_local_sessions(ctx):
        if str(row.get("task_id") or "").strip() != task_id:
            continue
        row_change_id = _normalize_text_value(row.get("change_id"))
        if row_change_id not in {None, change_id}:
            continue
        metadata = row.get("metadata") if isinstance(row.get("metadata"), dict) else {}
        if not str(metadata.get("graph_run_id") or "").strip() and not str(metadata.get("graph_run_session_id") or "").strip():
            continue
        if not bool(metadata.get("single_path_dag")):
            continue
        candidate_rows.append(row)
    if not candidate_rows:
        return None
    candidate_rows.sort(
        key=lambda row: (
            1 if bool((row.get("metadata") or {}).get("dag_shared_boundary_node")) else 0,
            str(row.get("updated_at") or row.get("created_at") or ""),
            str(row.get("session_id") or ""),
        ),
        reverse=True,
    )
    return candidate_rows[0]


def _workflow_land_batch_ensure_remote_task_dag_session(
    ctx: RepoContext,
    *,
    entry: dict[str, Any],
    remote_row: dict[str, Any],
    repo_name: str,
    remote_task_id: str,
    remote_change_id: str,
) -> dict[str, Any] | None:
    task_dag = entry.get("task_dag") if isinstance(entry.get("task_dag"), dict) else {}
    if not task_dag or not bool(task_dag.get("later_remote_promotion_allowed")):
        return None
    if str(task_dag.get("workflow_boundary") or "") != "reviewable_output":
        return None

    candidate_rows = [
        row
        for row in remote_list_sessions(remote_row["url"], repo_name)
        if str(row.get("task_id") or "").strip() == remote_task_id
        and _normalize_text_value(row.get("change_id")) in {None, remote_change_id}
        and bool(((row.get("metadata") or {}) if isinstance(row.get("metadata"), dict) else {}).get("single_path_dag"))
        and bool(((row.get("metadata") or {}) if isinstance(row.get("metadata"), dict) else {}).get("dag_shared_boundary_node"))
    ]
    if candidate_rows:
        candidate_rows.sort(
            key=lambda row: (
                str(row.get("updated_at") or row.get("created_at") or ""),
                str(row.get("session_id") or ""),
            ),
            reverse=True,
        )
        return candidate_rows[0]

    local_task = entry["task"]
    local_change = entry["change"]
    local_session = _workflow_batch_local_task_dag_session_row(
        ctx,
        task_id=str(local_task.get("task_id") or ""),
        change_id=str(local_change.get("change_id") or ""),
    )
    if local_session is None:
        raise ValueError(
            f"Completed local DAG output `{local_change['change_id']}` is missing local graph-run session evidence. "
            "Keep the final reviewable output attached to the same compact DAG worker lineage before later remote promotion."
        )
    metadata = dict(local_session.get("metadata") or {}) if isinstance(local_session.get("metadata"), dict) else {}
    if not str(metadata.get("graph_run_id") or "").strip() and not str(metadata.get("graph_run_session_id") or "").strip():
        raise ValueError(
            f"Completed local DAG output `{local_change['change_id']}` is missing graph-run ids in local session metadata."
        )
    metadata.pop("task_id", None)
    metadata.pop("change_id", None)
    metadata.update(
        {
            "plan_id": _normalize_text_value(local_task.get("plan_id")) or _normalize_text_value(metadata.get("plan_id")),
            "plan_item_ref": _normalize_text_value(local_task.get("plan_item_ref")) or _normalize_text_value(metadata.get("plan_item_ref")),
            "single_path_dag": True,
            "dag_shared_boundary_node": True,
            "workflow_boundary": "reviewable_output",
            "local_later_promotion_source_task_id": str(local_task.get("task_id") or ""),
            "local_later_promotion_source_change_id": str(local_change.get("change_id") or ""),
        }
    )
    return remote_create_session(
        remote_row["url"],
        repo_name,
        "agent_run",
        task_id=remote_task_id,
        change_id=remote_change_id,
        title=str(local_session.get("title") or f"Later promotion for {remote_change_id}"),
        line_name=_normalize_text_value(local_session.get("line_name")) or str(entry.get("target_line") or "main"),
        worktree_name=_normalize_text_value(local_session.get("worktree_name")),
        model_name=_normalize_text_value(local_session.get("model_name")) or _effective_model_name(ctx),
        metadata=metadata,
    )
