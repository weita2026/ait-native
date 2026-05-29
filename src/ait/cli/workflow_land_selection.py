from __future__ import annotations

from typing import Any

from ait_protocol.common import find_plan_item_in_items

from ..remote_client import (
    RemoteError,
    get_change as remote_get_change,
    get_session as remote_get_session,
    get_task as remote_get_task,
    list_session_events as remote_list_session_events,
    list_tasks as remote_list_tasks,
)
from ..store_local_changes import (
    get_local_change,
    list_local_changes,
)
from ..store_local_tasks import (
    get_local_task,
    list_local_tasks,
)
from ..store import (
    RepoContext,
    get_local_plan,
    load_config,
)
from .plan_task_linkage import _normalize_plan_task_linkage
from .remote_repository_defaults import _remote_tuple
from .runtime_defaults import _normalize_text_value
from .task_dag_execute_run_state import _task_dag_execute_run_summary
from .workflow_land_snapshot_replay import _resolve_completed_local_promotion_parent_snapshot_id
from .workflow_land_task_dag import _workflow_batch_task_dag_entry_metadata


def _workflow_batch_local_change_entries(
    ctx: RepoContext,
    *,
    remote_name: str,
    local_change_id: str | None = None,
) -> dict[str, Any]:
    resolved_local_change_id = str(local_change_id or "").strip().upper() or None
    candidate_changes: list[dict[str, Any]]
    if resolved_local_change_id is not None:
        candidate_change = get_local_change(ctx, resolved_local_change_id)
        candidate_task = get_local_task(ctx, str(candidate_change.get("task_id") or "").strip())
        completed_tasks = (
            {str(candidate_task["task_id"]): candidate_task}
            if str(candidate_task.get("status") or "") == "completed"
            else {}
        )
        candidate_changes = [candidate_change]
    else:
        completed_tasks = {
            str(row["task_id"]): row
            for row in list_local_tasks(ctx)
            if str(row.get("status") or "") == "completed"
        }
        candidate_changes = list_local_changes(ctx)
    if not completed_tasks:
        raise ValueError("No completed local tasks are available for batch remote land.")

    entries: list[dict[str, Any]] = []
    skipped_task_ids: list[str] = []
    skipped_change_ids: list[dict[str, Any]] = []
    changes_by_task_id: dict[str, list[dict[str, Any]]] = {}
    for change_row in candidate_changes:
        task_id = str(change_row.get("task_id") or "").strip()
        if task_id not in completed_tasks:
            continue
        changes_by_task_id.setdefault(task_id, []).append(change_row)
    remote_row, repo_name = _remote_tuple(ctx, remote_name)
    remote_tasks_by_item: dict[tuple[str, str], list[dict[str, Any]]] = {}
    needs_remote_task_index = any(
        str(task_row.get("publication_state") or "") != "published"
        for task_row in completed_tasks.values()
    )
    if needs_remote_task_index:
        for remote_task in remote_list_tasks(remote_row["url"], repo_name):
            plan_id = _normalize_text_value(remote_task.get("plan_id"))
            plan_item_ref = _normalize_text_value(remote_task.get("plan_item_ref"))
            if plan_id is None or plan_item_ref is None:
                continue
            remote_tasks_by_item.setdefault((plan_id, plan_item_ref), []).append(remote_task)
        for linked_tasks in remote_tasks_by_item.values():
            linked_tasks.sort(
                key=lambda row: (
                    int(row.get("task_seq") or 0),
                    str(row.get("created_at") or ""),
                    str(row.get("task_id") or ""),
                )
            )

    for task_id, task_row in completed_tasks.items():
        remote_task_status: str | None = None
        published_task_id: str | None = None
        local_plan_id: str | None = None
        published_plan_item_ref: str | None = None
        task_dag: dict[str, Any] | None = None
        task_remote_name = _normalize_text_value(task_row.get("published_remote_name"))
        if str(task_row.get("publication_state") or "") == "published" and task_remote_name not in {None, remote_name}:
            raise ValueError(
                f"Completed local task {task_id} is already published to `{task_remote_name}`, not `{remote_name}`."
            )
        task_changes = changes_by_task_id.get(task_id) or []
        landed_changes = [row for row in task_changes if str(row.get("status") or "") == "landed"]
        if not landed_changes:
            skipped_task_ids.append(task_id)
            continue
        try:
            task_dag = _workflow_batch_task_dag_entry_metadata(
                ctx,
                plan_id=_normalize_text_value(task_row.get("plan_id")),
                plan_revision_id=_normalize_text_value(task_row.get("origin_plan_revision_id")),
                plan_item_ref=_normalize_text_value(task_row.get("plan_item_ref")),
            )
        except ValueError:
            task_dag = None
        if task_dag is not None:
            workflow_boundary = str(task_dag.get("workflow_boundary") or "")
            if workflow_boundary != "reviewable_output":
                detail = (
                    f"Plan item ref `{task_row.get('plan_item_ref')}` is owned by task DAG node "
                    f"`{task_dag['node_id']}` in `{task_dag['graph_path']}` with workflow_boundary={workflow_boundary}. "
                    "Repo-wide `--all-completed-local` only later-promotes final converged DAG outputs."
                )
                for change_row in landed_changes:
                    skipped_change_ids.append(
                        {
                            "task_id": task_id,
                            "change_id": str(change_row.get("change_id") or "").strip(),
                            "reason": "task_dag_execution_only_residue",
                            "detail": detail,
                        }
                    )
                continue
            if not bool(task_dag.get("later_remote_promotion_allowed")):
                detail = (
                    f"Task DAG node `{task_dag['node_id']}` in `{task_dag['graph_path']}` is reviewable, but its graph "
                    "contract does not allow repo-wide later remote promotion after local land."
                )
                for change_row in landed_changes:
                    skipped_change_ids.append(
                        {
                            "task_id": task_id,
                            "change_id": str(change_row.get("change_id") or "").strip(),
                            "reason": "task_dag_repo_batch_remote_promotion_disabled",
                            "detail": detail,
                        }
                    )
                continue
        if str(task_row.get("publication_state") or "") == "published":
            published_task_id = _normalize_text_value(task_row.get("published_task_id")) or task_id
            try:
                remote_task = remote_get_task(remote_row["url"], published_task_id, repo_name=repo_name)
                remote_task_status = _normalize_text_value(remote_task.get("status"))
            except (KeyError, RemoteError, ValueError) as exc:
                detail = str(exc)
                for change_row in landed_changes:
                    skipped_change_ids.append(
                        {
                            "task_id": task_id,
                            "change_id": str(change_row.get("change_id") or "").strip(),
                            "reason": "missing_published_remote_task",
                            "detail": detail,
                        }
                    )
                continue
        else:
            try:
                local_plan_id, _, published_plan_item_ref = _normalize_plan_task_linkage(
                    ctx,
                    plan_id=_normalize_text_value(task_row.get("plan_id")),
                    plan_revision_id=_normalize_text_value(task_row.get("origin_plan_revision_id")),
                    plan_item_ref=_normalize_text_value(task_row.get("plan_item_ref")),
                    require_execution_binding=True,
                )
            except ValueError as exc:
                detail = str(exc)
                reason = (
                    "missing_durable_plan_linkage"
                    if "durable plan linkage" in detail or "Required plan/task binding requires" in detail
                    else "task_publish_preflight_failed"
                )
                if reason == "missing_durable_plan_linkage" and "durable plan linkage" not in detail:
                    detail = f"Completed local task {task_id} is missing durable plan linkage: {detail}"
                for change_row in landed_changes:
                    skipped_change_ids.append(
                        {
                            "task_id": task_id,
                            "change_id": str(change_row.get("change_id") or "").strip(),
                            "reason": reason,
                            "detail": detail,
                        }
                    )
                continue
            local_plan = None
            if local_plan_id is not None:
                try:
                    local_plan = get_local_plan(ctx, local_plan_id)
                except KeyError:
                    local_plan = None
            if local_plan is not None and published_plan_item_ref is not None:
                head_revision = local_plan.get("head_revision") if isinstance(local_plan.get("head_revision"), dict) else {}
                head_revision_id = _normalize_text_value(head_revision.get("plan_revision_id"))
                if find_plan_item_in_items(head_revision.get("items"), published_plan_item_ref) is None:
                    detail = (
                        f"Plan item ref `{published_plan_item_ref}` is no longer present in current plan revision "
                        f"`{head_revision_id or local_plan.get('head_revision_id') or 'unknown'}` for plan `{local_plan_id}`."
                    )
                    for change_row in landed_changes:
                        skipped_change_ids.append(
                            {
                                "task_id": task_id,
                                "change_id": str(change_row.get("change_id") or "").strip(),
                                "reason": "stale_plan_item_ref_residue",
                                "detail": detail,
                            }
                        )
                    continue
            if local_plan_id is not None and published_plan_item_ref is not None:
                linked_remote_tasks = [
                    row
                    for row in remote_tasks_by_item.get((local_plan_id, published_plan_item_ref), [])
                    if str(row.get("task_id") or "").strip() != task_id
                ]
                if linked_remote_tasks:
                    linked_remote_task = linked_remote_tasks[-1]
                    linked_task_id = str(linked_remote_task.get("task_id") or "").strip() or "unknown"
                    linked_status = str(linked_remote_task.get("status") or "unknown").strip() or "unknown"
                    linked_revision_id = _normalize_text_value(linked_remote_task.get("origin_plan_revision_id"))
                    revision_text = f" from revision `{linked_revision_id}`" if linked_revision_id else ""
                    detail = (
                        f"Plan item ref `{published_plan_item_ref}` on plan `{local_plan_id}` is already linked on "
                        f"`{remote_name}` to task `{linked_task_id}` (status: {linked_status}){revision_text}."
                    )
                    for change_row in landed_changes:
                        skipped_change_ids.append(
                            {
                                "task_id": task_id,
                                "change_id": str(change_row.get("change_id") or "").strip(),
                                "reason": "stale_plan_item_ref_residue",
                                "detail": detail,
                            }
                        )
                    continue
        for change_row in landed_changes:
            change_id = str(change_row.get("change_id") or "").strip()
            change_publication_state = str(change_row.get("publication_state") or "")
            remote_change_status: str | None = None
            if (
                remote_task_status not in {None, "", "active"}
                and change_publication_state != "published"
            ):
                skipped_change_ids.append(
                    {
                        "task_id": task_id,
                        "change_id": change_id,
                        "reason": "remote_task_closed_for_new_changes",
                        "detail": (
                            f"Published remote task `{published_task_id or task_id}` is "
                            f"{remote_task_status} and cannot accept new changes."
                        ),
                    }
                )
                continue
            change_remote_name = _normalize_text_value(change_row.get("published_remote_name"))
            if change_publication_state == "published" and change_remote_name not in {None, remote_name}:
                raise ValueError(
                    f"Completed local change {change_id} is already published to `{change_remote_name}`, not `{remote_name}`."
                )
            if change_publication_state == "published":
                published_change_id = _normalize_text_value(change_row.get("published_change_id")) or change_id
                try:
                    remote_change = remote_get_change(remote_row["url"], published_change_id, repo_name=repo_name)
                    remote_change_status = _normalize_text_value(remote_change.get("status"))
                except (KeyError, RemoteError, ValueError) as exc:
                    skipped_change_ids.append(
                        {
                            "task_id": task_id,
                            "change_id": change_id,
                            "reason": "missing_published_remote_change",
                            "detail": str(exc),
                        }
                    )
                    continue
            if (
                str(task_row.get("publication_state") or "") == "published"
                and change_publication_state == "published"
                and remote_task_status == "completed"
                and remote_change_status == "landed"
            ):
                skipped_change_ids.append(
                    {
                        "task_id": task_id,
                        "change_id": change_id,
                        "reason": "published_remote_done_residue",
                        "detail": (
                            f"Published remote task `{published_task_id or task_id}` is completed "
                            f"and remote change `{published_change_id or change_id}` is landed."
                        ),
                    }
                )
                continue
            landed_snapshot_id = _normalize_text_value(change_row.get("landed_snapshot_id"))
            if landed_snapshot_id is None:
                skipped_change_ids.append(
                    {
                        "task_id": task_id,
                        "change_id": change_id,
                        "reason": "missing_landed_snapshot_id",
                    }
                )
                continue
            parent_snapshot_id, parent_resolution = _resolve_completed_local_promotion_parent_snapshot_id(
                ctx,
                local_change=change_row,
                landed_snapshot_id=landed_snapshot_id,
            )
            change_target_line = (
                _normalize_text_value(change_row.get("target_line"))
                or _normalize_text_value(change_row.get("base_line"))
                or "main"
            )
            entries.append(
                {
                    "task": task_row,
                    "change": change_row,
                    "task_id": task_id,
                    "task_publication_state": str(task_row.get("publication_state") or ""),
                    "change_id": change_id,
                    "plan_id": local_plan_id,
                    "plan_item_ref": published_plan_item_ref,
                    "landed_snapshot_id": landed_snapshot_id,
                    "parent_snapshot_id": parent_snapshot_id,
                    "parent_resolution": parent_resolution,
                    "target_line": change_target_line,
                    "landed_at": _normalize_text_value(change_row.get("landed_at")),
                    "created_at": _normalize_text_value(change_row.get("created_at")),
                    "task_dag": task_dag,
                }
            )

    if resolved_local_change_id is not None:
        matching_entries = [
            entry
            for entry in entries
            if str(entry.get("change_id") or "").strip().upper() == resolved_local_change_id
        ]
        matching_skipped_change_ids = [
            row
            for row in skipped_change_ids
            if str(row.get("change_id") or "").strip().upper() == resolved_local_change_id
        ]
        if matching_entries:
            entries = matching_entries
            skipped_change_ids = matching_skipped_change_ids
            skipped_task_ids = []
        elif matching_skipped_change_ids:
            skipped_row = matching_skipped_change_ids[0]
            detail = str(skipped_row.get("detail") or "").strip()
            reason = str(skipped_row.get("reason") or "skipped").strip() or "skipped"
            raise ValueError(
                detail or f"Completed local change {resolved_local_change_id} is not eligible for remote land ({reason})."
            )
        else:
            local_change = get_local_change(ctx, resolved_local_change_id)
            local_status = str(local_change.get("status") or "").strip() or "unknown"
            if local_status != "landed":
                raise ValueError(
                    f"Local change {resolved_local_change_id} is `{local_status}`; "
                    "finish it with `ait workflow land-local` before `ait workflow land`."
                )
            local_task_id = str(local_change.get("task_id") or "").strip()
            local_task = get_local_task(ctx, local_task_id)
            local_task_status = str(local_task.get("status") or "").strip() or "unknown"
            if local_task_status != "completed":
                raise ValueError(
                    f"Local task {local_task_id} is `{local_task_status}`; "
                    f"complete it before promoting landed local change {resolved_local_change_id}."
                )
            raise ValueError(
                f"Completed local change {resolved_local_change_id} is not currently eligible for remote land."
            )

    if not entries:
        skipped_text = f" Skipped completed tasks without landed changes: {', '.join(skipped_task_ids)}." if skipped_task_ids else ""
        skipped_change_text = ""
        if skipped_change_ids:
            sample = ", ".join(
                f"{row['change_id']} ({row.get('reason') or 'skipped'})"
                for row in skipped_change_ids[:5]
            )
            remaining = len(skipped_change_ids) - min(len(skipped_change_ids), 5)
            if remaining > 0:
                sample = f"{sample}, +{remaining} more"
            skipped_change_text = f" Skipped landed changes: {sample}."
        raise ValueError(f"No completed local landed changes are available for batch remote land.{skipped_text}{skipped_change_text}")

    entries.sort(
        key=lambda entry: (
            str(entry.get("landed_at") or entry.get("created_at") or ""),
            str(entry.get("task_id") or ""),
            str(entry.get("change_id") or ""),
        )
    )

    default_line = str(load_config(ctx).get("default_line") or "main")
    target_line_name = next(
        (
            str(entry.get("target_line") or "").strip()
            for entry in entries
            if str(entry.get("target_line") or "").strip() == default_line
        ),
        None,
    )
    if target_line_name is None and entries:
        target_line_name = str(entries[0].get("target_line") or "").strip() or default_line

    same_line_entries: list[dict[str, Any]] = []
    for entry in entries:
        entry_target_line = str(entry.get("target_line") or "").strip() or default_line
        if target_line_name and entry_target_line != target_line_name:
            skipped_change_ids.append(
                {
                    "task_id": str(entry.get("task_id") or "").strip(),
                    "change_id": str(entry.get("change_id") or "").strip(),
                    "reason": "batch_target_line_mismatch",
                    "detail": (
                        f"Repo batch is promoting target line `{target_line_name}`, "
                        f"but this completed local row targets `{entry_target_line}`."
                    ),
                }
            )
            continue
        same_line_entries.append(entry)

    entries = same_line_entries

    deduped_entries: list[dict[str, Any]] = []
    seen_local_plan_item_keys: dict[tuple[str, str], dict[str, Any]] = {}
    for entry in entries:
        entry_plan_id = _normalize_text_value(entry.get("plan_id"))
        entry_plan_item_ref = _normalize_text_value(entry.get("plan_item_ref"))
        entry_task_publication_state = str(entry.get("task_publication_state") or "")
        if (
            entry_task_publication_state != "published"
            and entry_plan_id is not None
            and entry_plan_item_ref is not None
        ):
            plan_item_key = (entry_plan_id, entry_plan_item_ref)
            earlier_entry = seen_local_plan_item_keys.get(plan_item_key)
            if earlier_entry is not None:
                skipped_change_ids.append(
                    {
                        "task_id": str(entry.get("task_id") or "").strip(),
                        "change_id": str(entry.get("change_id") or "").strip(),
                        "reason": "duplicate_local_plan_item_ref_residue",
                        "detail": (
                            f"Plan item ref `{entry_plan_item_ref}` on plan `{entry_plan_id}` is already queued "
                            f"earlier in this batch by task `{earlier_entry['task_id']}` / change "
                            f"`{earlier_entry['change_id']}`."
                        ),
                    }
                )
                continue
            seen_local_plan_item_keys[plan_item_key] = entry
        deduped_entries.append(entry)

    entries = deduped_entries

    return {
        "mode": "all_completed_local",
        "target_line": target_line_name or "main",
        "entries": entries,
        "skipped_task_ids": skipped_task_ids,
        "skipped_change_ids": skipped_change_ids,
    }


def _workflow_land_batch_graph_run_selector(
    ctx: RepoContext,
    *,
    remote_name: str,
    graph_run_session_id: str,
) -> dict[str, Any]:
    remote_row, repo_name = _remote_tuple(ctx, remote_name)
    session = remote_get_session(remote_row["url"], graph_run_session_id, repo_name=repo_name)
    session_kind = str(session.get("session_kind") or "").strip()
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    if session_kind != "task_graph_run":
        owning_graph_run_session_id = str(metadata.get("graph_run_session_id") or "").strip() or None
        if owning_graph_run_session_id:
            raise ValueError(
                f"Session {graph_run_session_id} is `{session_kind}`, not `task_graph_run`; use `{owning_graph_run_session_id}` instead."
            )
        raise ValueError(f"Session {graph_run_session_id} is `{session_kind}`, not `task_graph_run`.")
    events = remote_list_session_events(remote_row["url"], graph_run_session_id, repo_name=repo_name)
    summary = _task_dag_execute_run_summary(session, events)
    latest_state_snapshot = (
        summary.get("latest_state_snapshot")
        if isinstance(summary.get("latest_state_snapshot"), dict)
        else {}
    )
    gate_handoff = (
        latest_state_snapshot.get("gate_handoff")
        if isinstance(latest_state_snapshot.get("gate_handoff"), dict)
        else {}
    )
    handoff_kind = str(gate_handoff.get("kind") or "").strip()
    if handoff_kind != "converged_gate_bundle":
        raise ValueError(
            f"Graph-run session {graph_run_session_id} is not paused at a converged gate bundle (kind={handoff_kind or 'none'})."
        )
    if not bool(gate_handoff.get("final_remote_land_ready")):
        raise ValueError(
            f"Graph-run session {graph_run_session_id} is not ready for final remote land yet."
        )
    candidate_change_ids = [
        str(value).strip()
        for value in gate_handoff.get("candidate_change_ids") or []
        if str(value).strip()
    ]
    if not candidate_change_ids:
        raise ValueError(
            f"Graph-run session {graph_run_session_id} has no converged candidate changes to land."
        )
    return {
        "mode": "graph_run_session",
        "graph_run_session_id": graph_run_session_id,
        "remote_row": remote_row,
        "repo_name": repo_name,
        "session": session,
        "summary": summary,
        "entries": [{"change_id": change_id} for change_id in candidate_change_ids],
    }
