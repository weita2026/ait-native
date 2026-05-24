from __future__ import annotations

from typing import Any, Mapping

from ..server_paths import ServerContext


def _legacy_read_models_module():
    from .. import read_models as legacy_read_models

    return legacy_read_models


def build_task_graph_progress(*args, **kwargs):
    return _legacy_read_models_module().build_task_graph_progress(*args, **kwargs)


def topological_node_order(*args, **kwargs):
    return _legacy_read_models_module().topological_node_order(*args, **kwargs)


def validate_task_graph(*args, **kwargs):
    return _legacy_read_models_module().validate_task_graph(*args, **kwargs)


def build_task_graph_execution_strategy(*args, **kwargs):
    return _legacy_read_models_module().build_task_graph_execution_strategy(*args, **kwargs)


def compute_task_graph_readiness(*args, **kwargs):
    return _legacy_read_models_module().compute_task_graph_readiness(*args, **kwargs)


def get_change(*args, **kwargs):
    return _legacy_read_models_module().get_change(*args, **kwargs)


def get_plan(*args, **kwargs):
    return _legacy_read_models_module().get_plan(*args, **kwargs)


def get_policy_status(*args, **kwargs):
    return _legacy_read_models_module().get_policy_status(*args, **kwargs)


def list_changes(*args, **kwargs):
    return _legacy_read_models_module().list_changes(*args, **kwargs)


def list_patchsets(*args, **kwargs):
    return _legacy_read_models_module().list_patchsets(*args, **kwargs)


def list_reviews(*args, **kwargs):
    return _legacy_read_models_module().list_reviews(*args, **kwargs)


def list_session_checkpoints(*args, **kwargs):
    return _legacy_read_models_module().list_session_checkpoints(*args, **kwargs)


def list_session_events(*args, **kwargs):
    return _legacy_read_models_module().list_session_events(*args, **kwargs)


def list_sessions(*args, **kwargs):
    return _legacy_read_models_module().list_sessions(*args, **kwargs)


def list_tasks(*args, **kwargs):
    return _legacy_read_models_module().list_tasks(*args, **kwargs)


def _latest_land_summary(*args, **kwargs):
    return _legacy_read_models_module()._latest_land_summary(*args, **kwargs)

def task_dag_readiness_from_facts(
    graph: dict[str, Any],
    workflow: dict[str, Any] | None = None,
    *,
    current_plan_revision_id: str | None = None,
) -> dict[str, Any]:
    return compute_task_graph_readiness(
        graph,
        workflow or {},
        current_plan_revision_id=current_plan_revision_id,
    )


def task_dag_graph_from_facts(
    graph: dict[str, Any],
    workflow: dict[str, Any] | None = None,
    *,
    current_plan_revision_id: str | None = None,
) -> dict[str, Any]:
    graph = validate_task_graph(graph)
    readiness = task_dag_readiness_from_facts(graph, workflow, current_plan_revision_id=current_plan_revision_id)
    readiness_by_id = {row["node_id"]: row for row in readiness.get("nodes", []) if isinstance(row, dict) and row.get("node_id")}
    node_rows = []
    for node_id in topological_node_order(graph):
        row = readiness_by_id.get(node_id, {})
        lineage = row.get("lineage") if isinstance(row.get("lineage"), dict) else {}
        node_rows.append(
            {
                "node_id": node_id,
                "node_kind": row.get("node_kind"),
                "title": row.get("title"),
                "plan_item_ref": row.get("plan_item_ref"),
                "state": row.get("state"),
                "depends_on": row.get("depends_on") or [],
                "lock_keys": row.get("lock_keys") or [],
                "hotspot_keys": row.get("hotspot_keys") or [],
                "task_id": lineage.get("task_id"),
                "change_id": lineage.get("change_id"),
                "session_id": lineage.get("session_id"),
                "task_run_id": lineage.get("task_run_id"),
                "checkpoint_id": lineage.get("checkpoint_id"),
                "patchset_id": lineage.get("patchset_id"),
                "patchset_base_snapshot_id": lineage.get("patchset_base_snapshot_id"),
                "patchset_revision_snapshot_id": lineage.get("patchset_revision_snapshot_id"),
                "land_id": lineage.get("land_id"),
                "landed_snapshot_id": lineage.get("landed_snapshot_id"),
                "session_recommendation": row.get("session_recommendation") or {},
                "surface_bindings": row.get("surface_bindings") or [],
                "reason": row.get("reason"),
            }
        )
    return {
        "schema_version": 1,
        "graph_id": graph.get("graph_id"),
        "source_plan": graph.get("source_plan") or {},
        "summary": readiness.get("summary") or {},
        "nodes": node_rows,
        "edges": graph.get("edges") or [],
    }


def task_dag_schedule_from_facts(
    graph: dict[str, Any],
    workflow: dict[str, Any] | None = None,
    *,
    current_plan_revision_id: str | None = None,
) -> dict[str, Any]:
    graph = validate_task_graph(graph)
    readiness = task_dag_readiness_from_facts(graph, workflow, current_plan_revision_id=current_plan_revision_id)
    ready_rows = []
    running_rows = []
    blocked_rows = []
    for row in readiness.get("nodes") or []:
        if not isinstance(row, dict):
            continue
        lineage = row.get("lineage") if isinstance(row.get("lineage"), dict) else {}
        base = {
            "node_id": row.get("node_id"),
            "node_kind": row.get("node_kind"),
            "title": row.get("title"),
            "plan_item_ref": row.get("plan_item_ref"),
            "lock_keys": row.get("lock_keys") or [],
            "hotspot_keys": row.get("hotspot_keys") or [],
            "session_recommendation": row.get("session_recommendation") or {},
        }
        if row.get("state") == "ready":
            ready_rows.append(base)
        elif row.get("state") == "running":
            running_rows.append(
                {
                    **base,
                    "task_id": lineage.get("task_id"),
                    "change_id": lineage.get("change_id"),
                    "session_id": lineage.get("session_id"),
                    "task_run_id": lineage.get("task_run_id"),
                    "surface_bindings": row.get("surface_bindings") or [],
                }
            )
        elif row.get("state") == "blocked":
            blocked_rows.append({**base, "reason": row.get("reason"), "blockers": row.get("blockers") or []})
    return {
        "schema_version": 1,
        "graph_id": graph.get("graph_id"),
        "source_plan": graph.get("source_plan") or {},
        "summary": readiness.get("summary") or {},
        "execution_strategy": build_task_graph_execution_strategy(graph, ready_rows),
        "ready": ready_rows,
        "running": running_rows,
        "blocked": blocked_rows,
    }


def task_dag_progress_from_facts(
    graph: dict[str, Any],
    workflow: dict[str, Any] | None = None,
    *,
    current_plan_revision_id: str | None = None,
) -> dict[str, Any]:
    graph = validate_task_graph(graph)
    readiness = task_dag_readiness_from_facts(graph, workflow, current_plan_revision_id=current_plan_revision_id)
    node_states = {
        row["node_id"]: {"state": row.get("state") or "blocked", "reason": row.get("reason")}
        for row in readiness.get("nodes", [])
        if isinstance(row, dict) and row.get("node_id")
    }
    progress = build_task_graph_progress(
        graph,
        node_states,
        next_action=(readiness.get("summary") or {}).get("next_action"),
    )
    return {
        "schema_version": 1,
        "graph_id": graph.get("graph_id"),
        "source_plan": graph.get("source_plan") or {},
        "readiness_summary": readiness.get("summary") or {},
        "progress": progress,
        "blockers": [
            {
                "node_id": row.get("node_id"),
                "title": row.get("title"),
                "reason": row.get("reason"),
                "blockers": row.get("blockers") or [],
            }
            for row in readiness.get("nodes", [])
            if isinstance(row, dict) and row.get("state") == "blocked"
        ],
    }


def task_dag_readiness(
    ctx: ServerContext,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
) -> dict[str, Any]:
    workflow, resolved_revision_id = _task_dag_workflow_facts(ctx, graph, current_plan_revision_id=current_plan_revision_id)
    return task_dag_readiness_from_facts(graph, workflow, current_plan_revision_id=resolved_revision_id)


def task_dag_graph(
    ctx: ServerContext,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
) -> dict[str, Any]:
    workflow, resolved_revision_id = _task_dag_workflow_facts(ctx, graph, current_plan_revision_id=current_plan_revision_id)
    return task_dag_graph_from_facts(graph, workflow, current_plan_revision_id=resolved_revision_id)


def task_dag_schedule(
    ctx: ServerContext,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
) -> dict[str, Any]:
    workflow, resolved_revision_id = _task_dag_workflow_facts(ctx, graph, current_plan_revision_id=current_plan_revision_id)
    return task_dag_schedule_from_facts(graph, workflow, current_plan_revision_id=resolved_revision_id)


def task_dag_progress(
    ctx: ServerContext,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
) -> dict[str, Any]:
    workflow, resolved_revision_id = _task_dag_workflow_facts(ctx, graph, current_plan_revision_id=current_plan_revision_id)
    payload = task_dag_progress_from_facts(graph, workflow, current_plan_revision_id=resolved_revision_id)
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    latest_graph_run = _task_dag_latest_graph_run_summary(
        ctx,
        workflow.get("sessions") or [],
        plan_id=_task_dag_clean_text(source_plan.get("plan_id") or graph.get("plan_id")),
        graph_id=_task_dag_clean_text(graph.get("graph_id")),
    )
    if latest_graph_run is not None:
        payload["latest_graph_run"] = latest_graph_run
    return payload


def _task_dag_node_state_rows_from_sessions(
    ctx: ServerContext,
    sessions: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for session in sessions:
        if _task_dag_clean_text(session.get("session_kind")) != "task_graph_run":
            continue
        session_id = _task_dag_clean_text(session.get("session_id"))
        if not session_id:
            continue
        graph_run_id = _task_dag_clean_text((session.get("metadata") or {}).get("graph_run_id"))
        for event in list_session_events(ctx, session_id):
            if not isinstance(event, Mapping):
                continue
            event_type = _task_dag_clean_text(event.get("event_type"))
            if event_type not in {"task_graph.node_local_progress", "task_graph.node_completed"}:
                continue
            payload = event.get("payload") if isinstance(event.get("payload"), Mapping) else {}
            node_id = _task_dag_clean_text(payload.get("node_id"))
            if not node_id:
                continue
            status = (
                "completed"
                if event_type == "task_graph.node_completed"
                else (_task_dag_clean_text(payload.get("status")) or "running")
            )
            rows.append(
                {
                    "node_state_id": f"{session_id}:{event.get('sequence') or len(rows) + 1}",
                    "node_id": node_id,
                    "state": status,
                    "status": status,
                    "reason": _task_dag_clean_text(payload.get("summary")) or _task_dag_clean_text(payload.get("reason")),
                    "message": _task_dag_clean_text(payload.get("summary")) or _task_dag_clean_text(payload.get("reason")),
                    "task_id": _task_dag_clean_text(payload.get("task_id")),
                    "change_id": _task_dag_clean_text(payload.get("change_id")),
                    "completion_snapshot_id": _task_dag_clean_text(payload.get("completion_snapshot_id")),
                    "completion_fork_snapshot_id": _task_dag_clean_text(payload.get("completion_fork_snapshot_id")),
                    "completion_line_name": _task_dag_clean_text(payload.get("completion_line_name")),
                    "completion_worktree_name": _task_dag_clean_text(payload.get("completion_worktree_name")),
                    "session_id": session_id,
                    "graph_run_id": graph_run_id,
                    "source_event_type": event_type,
                    "created_at": event.get("created_at"),
                    "updated_at": event.get("created_at") or event.get("updated_at"),
                }
            )
    return rows


def _task_dag_workflow_facts(
    ctx: ServerContext,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
) -> tuple[dict[str, Any], str | None]:
    repo_name = graph.get("repo_name")
    if not isinstance(repo_name, str) or not repo_name.strip():
        raise ValueError("Task DAG graph must include repo_name for server readiness reads.")
    repo_name = repo_name.strip()

    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    plan_id = source_plan.get("plan_id")
    if current_plan_revision_id is None and isinstance(plan_id, str) and plan_id.strip():
        plan = get_plan(ctx, plan_id.strip())
        current_plan_revision_id = (plan.get("head_revision") or {}).get("plan_revision_id") or plan.get("head_revision_id")

    identifiers = _task_dag_graph_identifiers(graph)
    tasks = _task_dag_relevant_tasks(list_tasks(ctx, repo_name), identifiers)
    task_ids = {_task_dag_clean_text(task.get("task_id")) for task in tasks}
    task_ids.discard(None)

    change_summaries = _task_dag_relevant_changes(list_changes(ctx, repo_name), task_ids, identifiers)
    changes = [get_change(ctx, row["change_id"]) for row in change_summaries if _task_dag_clean_text(row.get("change_id"))]
    change_ids = {_task_dag_clean_text(change.get("change_id")) for change in changes}
    change_ids.discard(None)

    sessions = _task_dag_relevant_sessions(list_sessions(ctx, repo_name), task_ids, change_ids, identifiers)
    checkpoints: list[dict[str, Any]] = []
    for session in sessions:
        checkpoints.extend(list_session_checkpoints(ctx, session["session_id"]))

    patchsets: list[dict[str, Any]] = []
    reviews: list[dict[str, Any]] = []
    lands: list[dict[str, Any]] = []
    for change in changes:
        patchsets.extend(list_patchsets(ctx, change["change_id"]))
        review_summary = dict(list_reviews(ctx, change["change_id"]))
        review_summary["change_id"] = change["change_id"]
        reviews.append(review_summary)
        landing_summary = _latest_land_summary(ctx, change["change_id"])
        if landing_summary is not None:
            lands.append(landing_summary)

    policies = []
    for patchset in patchsets:
        policy = dict(get_policy_status(ctx, patchset["patchset_id"]))
        policy["patchset_id"] = patchset["patchset_id"]
        policies.append(policy)
    node_states = _task_dag_node_state_rows_from_sessions(ctx, sessions)

    return (
        {
            "tasks": tasks,
            "changes": changes,
            "sessions": sessions,
            "checkpoints": checkpoints,
            "patchsets": patchsets,
            "review_summaries": reviews,
            "policy_statuses": policies,
            "land_requests": lands,
            "node_states": node_states,
        },
        current_plan_revision_id,
    )


def _task_dag_graph_identifiers(graph: Mapping[str, Any]) -> dict[str, set[str]]:
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), Mapping) else {}
    identifiers = {
        "node_ids": set(),
        "plan_item_refs": set(),
        "task_ids": set(),
        "change_ids": set(),
        "session_ids": set(),
        "plan_ids": set(),
        "graph_ids": set(),
    }
    _task_dag_add_identifier(identifiers["plan_ids"], source_plan.get("plan_id"))
    _task_dag_add_identifier(identifiers["graph_ids"], graph.get("graph_id"))

    nodes = graph.get("nodes") if isinstance(graph.get("nodes"), list) else []
    for node in nodes:
        if not isinstance(node, Mapping):
            continue
        _task_dag_add_identifier(identifiers["node_ids"], node.get("node_id"))
        _task_dag_add_identifier(identifiers["plan_item_refs"], node.get("plan_item_ref"))
        _task_dag_add_identifier(identifiers["task_ids"], node.get("task_id"))
        _task_dag_add_identifier(identifiers["change_ids"], node.get("change_id"))
        _task_dag_add_identifier(identifiers["session_ids"], node.get("session_id"))
        task_template = node.get("task_template")
        if isinstance(task_template, Mapping):
            _task_dag_add_identifier(identifiers["task_ids"], task_template.get("task_id"))
            _task_dag_add_identifier(identifiers["change_ids"], task_template.get("change_id"))
            _task_dag_add_identifier(identifiers["session_ids"], task_template.get("session_id"))
    return identifiers


def _task_dag_relevant_tasks(tasks: list[dict[str, Any]], identifiers: Mapping[str, set[str]]) -> list[dict[str, Any]]:
    node_ids = identifiers.get("node_ids", set())
    plan_item_refs = identifiers.get("plan_item_refs", set())
    task_ids = identifiers.get("task_ids", set())
    plan_ids = identifiers.get("plan_ids", set())
    relevant = []
    for task in tasks:
        metadata = task.get("metadata") if isinstance(task.get("metadata"), Mapping) else {}
        task_id = _task_dag_clean_text(task.get("task_id"))
        node_id = _task_dag_clean_text(task.get("node_id")) or _task_dag_clean_text(metadata.get("node_id"))
        plan_item_ref = _task_dag_clean_text(task.get("plan_item_ref")) or _task_dag_clean_text(metadata.get("plan_item_ref"))
        plan_id = _task_dag_clean_text(task.get("plan_id")) or _task_dag_clean_text(metadata.get("plan_id"))
        if task_id and task_id in task_ids:
            relevant.append(task)
        elif node_id and node_id in node_ids:
            relevant.append(task)
        elif plan_item_ref and plan_item_ref in plan_item_refs:
            if not plan_id or not plan_ids or plan_id in plan_ids:
                relevant.append(task)
    return relevant


def _task_dag_relevant_changes(
    changes: list[dict[str, Any]],
    task_ids: set[str],
    identifiers: Mapping[str, set[str]],
) -> list[dict[str, Any]]:
    explicit_change_ids = identifiers.get("change_ids", set())
    return [
        change
        for change in changes
        if _task_dag_clean_text(change.get("task_id")) in task_ids
        or _task_dag_clean_text(change.get("change_id")) in explicit_change_ids
    ]


def _task_dag_relevant_sessions(
    sessions: list[dict[str, Any]],
    task_ids: set[str],
    change_ids: set[str],
    identifiers: Mapping[str, set[str]],
) -> list[dict[str, Any]]:
    explicit_session_ids = identifiers.get("session_ids", set())
    explicit_plan_ids = identifiers.get("plan_ids", set())
    explicit_graph_ids = identifiers.get("graph_ids", set())
    return [
        session
        for session in sessions
        if _task_dag_clean_text(session.get("task_id")) in task_ids
        or _task_dag_clean_text(session.get("change_id")) in change_ids
        or _task_dag_clean_text(session.get("session_id")) in explicit_session_ids
        or (
            _task_dag_clean_text(session.get("session_kind")) == "task_graph_run"
            and (
                _task_dag_clean_text((session.get("metadata") or {}).get("plan_id")) in explicit_plan_ids
                or _task_dag_clean_text((session.get("metadata") or {}).get("graph_id")) in explicit_graph_ids
            )
        )
    ]


def _task_dag_latest_graph_run_summary(
    ctx: ServerContext,
    sessions: list[dict[str, Any]],
    *,
    plan_id: str | None = None,
    graph_id: str | None = None,
) -> dict[str, Any] | None:
    graph_runs = [
        session
        for session in sessions
        if _task_dag_clean_text(session.get("session_kind")) == "task_graph_run"
        and _task_dag_session_matches_graph_run_identity(session, plan_id=plan_id, graph_id=graph_id)
    ]
    if not graph_runs:
        return None
    latest = sorted(
        graph_runs,
        key=lambda row: (
            str(row.get("updated_at") or row.get("created_at") or ""),
            str(row.get("created_at") or ""),
            str(row.get("session_id") or ""),
        ),
        reverse=True,
    )[0]
    events = list_session_events(ctx, str(latest.get("session_id") or ""))
    latest_event = events[-1] if events else None
    state_snapshot = None
    for event in reversed(events):
        if str(event.get("event_type") or "").strip() == "task_graph.state_snapshot":
            state_snapshot = event
            break
    metadata = latest.get("metadata") if isinstance(latest.get("metadata"), dict) else {}
    state_payload = state_snapshot.get("payload") if isinstance(state_snapshot, dict) and isinstance(state_snapshot.get("payload"), dict) else {}
    latest_payload = latest_event.get("payload") if isinstance(latest_event, dict) and isinstance(latest_event.get("payload"), dict) else {}
    workflow_summary = latest_payload.get("workflow_summary") if isinstance(latest_payload.get("workflow_summary"), dict) else {}
    if not workflow_summary and isinstance(state_payload.get("workflow_summary"), dict):
        workflow_summary = state_payload.get("workflow_summary")
    gate_handoff = state_payload.get("gate_handoff") if isinstance(state_payload.get("gate_handoff"), dict) else {}
    return {
        "session_id": latest.get("session_id"),
        "session_local_id": latest.get("session_local_id"),
        "repo_name": latest.get("repo_name"),
        "repo_id": latest.get("repo_id"),
        "graph_run_id": metadata.get("graph_run_id"),
        "execution_state": state_payload.get("execution_state") or metadata.get("execution_state") or "active",
        "pause_reason": state_payload.get("pause_reason"),
        "next_action": state_payload.get("next_action") or (workflow_summary or {}).get("next_action"),
        "gate_handoff": {
            "kind": gate_handoff.get("kind"),
            "candidate_node_ids": gate_handoff.get("candidate_node_ids") or [],
            "candidate_change_ids": gate_handoff.get("candidate_change_ids") or [],
            "required_gates": gate_handoff.get("required_gates") or [],
            "promotion_required": bool(gate_handoff.get("promotion_required")),
        }
        if gate_handoff
        else None,
        "latest_event_type": latest_event.get("event_type") if isinstance(latest_event, dict) else None,
        "latest_event_sequence": latest_event.get("sequence") if isinstance(latest_event, dict) else None,
        "event_count": len(events),
        "workflow_summary": workflow_summary or None,
    }


def _task_dag_session_matches_graph_run_identity(
    session: Mapping[str, Any],
    *,
    plan_id: str | None,
    graph_id: str | None,
) -> bool:
    metadata = session.get("metadata") if isinstance(session.get("metadata"), Mapping) else {}
    session_plan_id = _task_dag_clean_text(metadata.get("plan_id"))
    session_graph_id = _task_dag_clean_text(metadata.get("graph_id"))
    if plan_id is not None and session_plan_id != plan_id:
        return False
    if graph_id is not None and session_graph_id != graph_id:
        return False
    return True


def _task_dag_add_identifier(values: set[str], value: Any) -> None:
    text = _task_dag_clean_text(value)
    if text:
        values.add(text)


def _task_dag_clean_text(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None
