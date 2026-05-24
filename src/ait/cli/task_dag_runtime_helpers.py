from __future__ import annotations

import sys
from pathlib import Path
from typing import Any, Mapping, Optional

from ..remote_client import (
    RemoteError,
    get_change as remote_get_change,
    get_plan as remote_get_plan,
    get_policy as remote_get_policy,
    list_changes as remote_list_changes,
    list_patchsets as remote_list_patchsets,
    list_reviews as remote_list_reviews,
    list_sessions as remote_list_sessions,
    list_session_checkpoints as remote_list_session_checkpoints,
    list_session_events as remote_list_session_events,
    list_tasks as remote_list_tasks,
)
from ..store import (
    RepoContext,
    current_line,
    get_local_plan,
    list_local_changes,
    list_local_session_events,
    list_local_sessions,
    list_local_tasks,
    load_config,
)
from ..task_dag_readiness import compute_task_graph_readiness, task_dag_final_remote_disposition_default
from .remote_ci_readiness_helpers import _remote_error_status_code, _remote_read_task_dag_readiness
from .remote_repository_defaults import _remote_tuple
from .runtime_defaults import _normalize_text_value


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)


def _task_dag_target_line_name(
    ctx: RepoContext,
    graph: dict[str, Any],
    graph_node: Mapping[str, Any],
    *,
    worktree: Mapping[str, Any] | None = None,
) -> str:
    load_config_fn = _app_override("load_config", load_config)
    current_line_fn = _app_override("current_line", current_line)
    normalize_text_fn = _app_override("_normalize_text_value", _normalize_text_value)
    policy = graph.get("execution_policy") if isinstance(graph.get("execution_policy"), Mapping) else {}
    template = graph_node.get("task_template") if isinstance(graph_node.get("task_template"), Mapping) else {}
    config = load_config_fn(ctx)
    candidates = [
        normalize_text_fn(template.get("target_line")),
        normalize_text_fn(graph_node.get("target_line")),
        normalize_text_fn(policy.get("target_line")),
        normalize_text_fn((worktree or {}).get("target_base_line")) if isinstance(worktree, Mapping) else None,
        normalize_text_fn(config.get("default_line")),
        normalize_text_fn(current_line_fn(ctx)),
        "main",
    ]
    return next((candidate for candidate in candidates if candidate), "main")


def _task_dag_relative_path(ctx: RepoContext, path: Path) -> str:
    resolved = path.resolve()
    for base in (ctx.root.resolve(), ctx.repo_root.resolve()):
        try:
            return str(resolved.relative_to(base))
        except ValueError:
            continue
    return str(path)


def _task_dag_graph_run_session_matches(graph: Mapping[str, Any], session: Mapping[str, Any]) -> bool:
    if _normalize_text_value(session.get("session_kind")) != "task_graph_run":
        return False
    metadata = session.get("metadata") if isinstance(session.get("metadata"), Mapping) else {}
    graph_id = _normalize_text_value(graph.get("graph_id"))
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), Mapping) else {}
    plan_id = _normalize_text_value(source_plan.get("plan_id") or graph.get("plan_id"))
    session_graph_id = _normalize_text_value(metadata.get("graph_id"))
    session_plan_id = _normalize_text_value(metadata.get("plan_id"))
    return bool((graph_id and session_graph_id == graph_id) or (plan_id and session_plan_id == plan_id))


def _task_dag_node_state_rows_from_session_events(
    graph: Mapping[str, Any],
    sessions: list[dict[str, Any]],
    event_loader: Any,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for session in sessions:
        if not isinstance(session, Mapping) or not _task_dag_graph_run_session_matches(graph, session):
            continue
        session_id = _normalize_text_value(session.get("session_id"))
        if session_id is None:
            continue
        metadata = session.get("metadata") if isinstance(session.get("metadata"), Mapping) else {}
        graph_run_id = _normalize_text_value(metadata.get("graph_run_id"))
        try:
            events = event_loader(session_id)
        except KeyError:
            continue
        for event in events:
            if not isinstance(event, Mapping):
                continue
            event_type = _normalize_text_value(event.get("event_type"))
            if event_type not in {"task_graph.node_local_progress", "task_graph.node_completed"}:
                continue
            payload = event.get("payload") if isinstance(event.get("payload"), Mapping) else {}
            node_id = _normalize_text_value(payload.get("node_id"))
            if node_id is None:
                continue
            status = (
                "completed"
                if event_type == "task_graph.node_completed"
                else (_normalize_text_value(payload.get("status")) or "running")
            )
            sequence = event.get("sequence") or len(rows) + 1
            rows.append(
                {
                    "node_state_id": f"{session_id}:{sequence}",
                    "node_id": node_id,
                    "state": status,
                    "status": status,
                    "reason": _normalize_text_value(payload.get("summary")) or _normalize_text_value(payload.get("reason")),
                    "message": _normalize_text_value(payload.get("summary")) or _normalize_text_value(payload.get("reason")),
                    "task_id": _normalize_text_value(payload.get("task_id")),
                    "change_id": _normalize_text_value(payload.get("change_id")),
                    "completion_snapshot_id": _normalize_text_value(payload.get("completion_snapshot_id")),
                    "completion_fork_snapshot_id": _normalize_text_value(payload.get("completion_fork_snapshot_id")),
                    "completion_line_name": _normalize_text_value(payload.get("completion_line_name")),
                    "completion_worktree_name": _normalize_text_value(payload.get("completion_worktree_name")),
                    "session_id": session_id,
                    "graph_run_id": graph_run_id,
                    "source_event_type": event_type,
                    "created_at": event.get("created_at"),
                    "updated_at": event.get("created_at") or event.get("updated_at"),
                }
            )
    return rows


def _task_dag_local_readiness_payload(ctx: RepoContext, graph: dict[str, Any]) -> dict[str, Any]:
    compute_readiness_fn = _app_override("compute_task_graph_readiness", compute_task_graph_readiness)
    get_local_plan_fn = _app_override("get_local_plan", get_local_plan)
    list_local_tasks_fn = _app_override("list_local_tasks", list_local_tasks)
    list_local_changes_fn = _app_override("list_local_changes", list_local_changes)
    list_local_sessions_fn = _app_override("list_local_sessions", list_local_sessions)
    list_local_session_events_fn = _app_override("list_local_session_events", list_local_session_events)
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    current_plan_revision_id = None
    plan_id = source_plan.get("plan_id")
    if isinstance(plan_id, str) and plan_id.strip():
        try:
            plan = get_local_plan_fn(ctx, plan_id.strip())
        except KeyError:
            plan = None
        if isinstance(plan, dict):
            head_revision = plan.get("head_revision") if isinstance(plan.get("head_revision"), dict) else {}
            current_plan_revision_id = head_revision.get("plan_revision_id") or plan.get("head_revision_id")
    sessions = list_local_sessions_fn(ctx)
    return compute_readiness_fn(
        graph,
        {
            "tasks": list_local_tasks_fn(ctx),
            "changes": list_local_changes_fn(ctx),
            "sessions": sessions,
            "checkpoints": [],
            "node_states": _task_dag_node_state_rows_from_session_events(
                graph,
                sessions,
                lambda session_id: list_local_session_events_fn(ctx, session_id),
            ),
        },
        current_plan_revision_id=current_plan_revision_id,
    )


def _task_dag_readiness_payload(ctx: RepoContext, graph: dict[str, Any], remote_name: Optional[str]) -> dict[str, Any]:
    if not task_dag_final_remote_disposition_default(graph):
        return _task_dag_local_readiness_payload(ctx, graph)
    remote_tuple_fn = _app_override("_remote_tuple", _remote_tuple)
    remote_readiness_fn = _app_override("_remote_read_task_dag_readiness", _remote_read_task_dag_readiness)
    remote_error_status_code_fn = _app_override("_remote_error_status_code", _remote_error_status_code)
    remote_row, repo_name = remote_tuple_fn(ctx, remote_name)
    try:
        return remote_readiness_fn(remote_row["url"], graph, repo_name=repo_name)
    except RemoteError as exc:
        if remote_error_status_code_fn(exc) != 404:
            raise
    remote_row, repo_name = remote_tuple_fn(ctx, remote_name)
    return _task_dag_readiness_from_remote_inventory(remote_row, repo_name, graph)


def _task_dag_readiness_from_remote_inventory(remote_row: dict[str, Any], repo_name: str, graph: dict[str, Any]) -> dict[str, Any]:
    base_url = remote_row["url"]
    remote_get_plan_fn = _app_override("remote_get_plan", remote_get_plan)
    remote_list_tasks_fn = _app_override("remote_list_tasks", remote_list_tasks)
    remote_list_changes_fn = _app_override("remote_list_changes", remote_list_changes)
    remote_get_change_fn = _app_override("remote_get_change", remote_get_change)
    remote_list_sessions_fn = _app_override("remote_list_sessions", remote_list_sessions)
    remote_list_session_checkpoints_fn = _app_override("remote_list_session_checkpoints", remote_list_session_checkpoints)
    remote_list_session_events_fn = _app_override("remote_list_session_events", remote_list_session_events)
    remote_list_patchsets_fn = _app_override("remote_list_patchsets", remote_list_patchsets)
    remote_list_reviews_fn = _app_override("remote_list_reviews", remote_list_reviews)
    remote_get_policy_fn = _app_override("remote_get_policy", remote_get_policy)
    remote_error_status_code_fn = _app_override("_remote_error_status_code", _remote_error_status_code)
    compute_readiness_fn = _app_override("compute_task_graph_readiness", compute_task_graph_readiness)

    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    current_plan_revision_id = None
    plan_id = source_plan.get("plan_id")
    if isinstance(plan_id, str) and plan_id.strip():
        try:
            plan = remote_get_plan_fn(base_url, plan_id.strip())
            head_revision = plan.get("head_revision") if isinstance(plan.get("head_revision"), dict) else {}
            current_plan_revision_id = head_revision.get("plan_revision_id") or plan.get("head_revision_id")
        except RemoteError:
            current_plan_revision_id = None

    tasks = remote_list_tasks_fn(base_url, repo_name)
    change_rows = remote_list_changes_fn(base_url, repo_name)
    changes = [
        remote_get_change_fn(base_url, str(row["change_id"]), repo_name=repo_name)
        for row in change_rows
        if row.get("change_id")
    ]
    sessions = remote_list_sessions_fn(base_url, repo_name)
    checkpoints: list[dict[str, Any]] = []
    for session in sessions:
        session_id = session.get("session_id")
        if session_id:
            checkpoints.extend(
                remote_list_session_checkpoints_fn(base_url, str(session_id), repo_name=repo_name),
            )
    node_states = _task_dag_node_state_rows_from_session_events(
        graph,
        sessions,
        lambda session_id: remote_list_session_events_fn(base_url, session_id, repo_name=repo_name),
    )

    patchsets: list[dict[str, Any]] = []
    reviews: list[dict[str, Any]] = []
    policies: list[dict[str, Any]] = []
    for change in changes:
        change_id = change.get("change_id")
        if not change_id:
            continue
        change_patchsets = remote_list_patchsets_fn(base_url, str(change_id), repo_name=repo_name)
        patchsets.extend(change_patchsets)
        review_summary = dict(remote_list_reviews_fn(base_url, str(change_id), repo_name=repo_name))
        review_summary["change_id"] = change_id
        reviews.append(review_summary)
    for patchset in patchsets:
        patchset_id = patchset.get("patchset_id")
        if not patchset_id:
            continue
        try:
            policy = dict(remote_get_policy_fn(base_url, str(patchset_id), repo_name=repo_name))
        except RemoteError as exc:
            if remote_error_status_code_fn(exc) != 404:
                raise
            continue
        policy["patchset_id"] = patchset_id
        policies.append(policy)

    return compute_readiness_fn(
        graph,
        {
            "tasks": tasks,
            "changes": changes,
            "sessions": sessions,
            "checkpoints": checkpoints,
            "patchsets": patchsets,
            "review_summaries": reviews,
            "policy_statuses": policies,
            "node_states": node_states,
        },
        current_plan_revision_id=current_plan_revision_id,
    )


def _task_dag_graph_for_remote(ctx: RepoContext, graph: dict[str, Any], graph_path: Path) -> dict[str, Any]:
    dispatch_artifacts = dict(graph.get("dispatch_artifacts") or {}) if isinstance(graph.get("dispatch_artifacts"), dict) else {}
    dispatch_artifacts["task_graph_json"] = _task_dag_relative_path(ctx, graph_path)
    payload = dict(graph)
    payload["dispatch_artifacts"] = dispatch_artifacts
    return payload
