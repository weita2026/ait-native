from __future__ import annotations

import json
from typing import Any, Mapping

from .plans import _get_plan_row, _plan_revision_has_task_graph_artifact


def _task_graph_land_lineage_error(task: Mapping[str, Any] | None, change: Mapping[str, Any] | None) -> str | None:
    if not isinstance(task, Mapping) or not isinstance(change, Mapping):
        return None
    task_id = str(task.get("task_id") or "").strip()
    change_id = str(change.get("change_id") or "").strip()
    if not task_id or not change_id:
        return None
    if str(task.get("plan_item_ref") or "").strip():
        return None
    return (
        f"Change {change_id} is linked to task {task_id} without a plan_item_ref. "
        "Remote land for node-bound tasks requires explicit plan lineage; rebind the task with `--plan-item-ref` "
        "and keep the change attached to the same compact DAG worker lineage."
    )


def _session_has_task_graph_run_evidence(session_row: Mapping[str, Any], *, task: Mapping[str, Any]) -> bool:
    try:
        metadata = json.loads(str(session_row.get("metadata_json") or "{}"))
    except json.JSONDecodeError:
        return False
    if not isinstance(metadata, Mapping):
        return False
    if not str(metadata.get("graph_run_id") or "").strip() and not str(metadata.get("graph_run_session_id") or "").strip():
        return False
    task_plan_id = str(task.get("plan_id") or "").strip()
    task_plan_item_ref = str(task.get("plan_item_ref") or "").strip()
    metadata_plan_id = str(metadata.get("plan_id") or "").strip()
    metadata_plan_item_ref = str(metadata.get("plan_item_ref") or "").strip()
    if task_plan_id and metadata_plan_id and metadata_plan_id != task_plan_id:
        return False
    if task_plan_item_ref and metadata_plan_item_ref and metadata_plan_item_ref != task_plan_item_ref:
        return False
    return True


def _session_task_graph_metadata(session_row: Mapping[str, Any]) -> Mapping[str, Any]:
    try:
        metadata = json.loads(str(session_row.get("metadata_json") or "{}"))
    except json.JSONDecodeError:
        return {}
    return metadata if isinstance(metadata, Mapping) else {}


def _validate_task_graph_change_land_request(conn, change: Mapping[str, Any]) -> None:
    task_id = str(change.get("task_id") or "").strip()
    if not task_id:
        return
    task_row = conn.execute("select * from tasks where task_id = ?", (task_id,)).fetchone()
    if task_row is None:
        return
    task = dict(task_row)
    plan_id = str(task.get("plan_id") or "").strip()
    if not plan_id:
        return
    plan_revision_id = str(task.get("origin_plan_revision_id") or "").strip()
    if not plan_revision_id:
        plan_row = _get_plan_row(conn, plan_id)
        plan_revision_id = str(plan_row.get("head_revision_id") or "").strip()
    if not _plan_revision_has_task_graph_artifact(conn, plan_revision_id or None):
        return
    lineage_error = _task_graph_land_lineage_error(task, change)
    if lineage_error is not None:
        raise ValueError(lineage_error)
    session_rows = conn.execute(
        """
        select metadata_json
        from sessions
        where task_id = ?
          and (change_id is null or change_id = ?)
        order by updated_at desc, created_at desc, session_id desc
        """,
        (task_id, change["change_id"]),
    ).fetchall()
    evidenced_metadata = [
        _session_task_graph_metadata(dict(row))
        for row in session_rows
        if _session_has_task_graph_run_evidence(dict(row), task=task)
    ]
    if evidenced_metadata:
        if any(bool(metadata.get("single_path_dag")) for metadata in evidenced_metadata) and not any(
            bool(metadata.get("dag_shared_boundary_node")) for metadata in evidenced_metadata
        ):
            raise ValueError(
                f"Change {change['change_id']} belongs to a non-final DAG node. "
                "Only the final converged DAG node may clear remote review / policy / land gates."
            )
        return
    raise ValueError(
        f"Change {change['change_id']} is linked to task-graph work but has no graph-run session evidence. "
        "Create or continue the compact DAG worker lineage with `ait plan execute ... --auto-compact-worker --yes` before remote land."
    )


__all__ = ["_validate_task_graph_change_land_request"]
