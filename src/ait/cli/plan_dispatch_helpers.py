from __future__ import annotations

from typing import Any, Optional

from ..remote_client import (
    RemoteError,
    get_plan as remote_get_plan,
    list_plans as remote_list_plans,
    list_tasks as remote_list_tasks,
)
from ..repo_paths import RepoContext
from ..store import get_local_plan, list_local_plans, list_local_tasks, load_config
from .plan_sync_matching import _plan_status_is_historical
from .workflow_authoring import _plan_uses_local_store
from .remote_repository_defaults import _remote_tuple
from .workflow_mode_config import _normalize_text_value


def _plan_items_payload(plan: dict[str, Any], *, revision: dict[str, Any] | None = None) -> dict[str, Any]:
    current_revision = revision or plan.get("head_revision") or {}
    items = list(current_revision.get("items") or [])
    return {
        "plan_id": plan.get("plan_id"),
        "plan_title": plan.get("title"),
        "plan_revision_id": current_revision.get("plan_revision_id"),
        "revision_number": current_revision.get("revision_number"),
        "identity_only": True,
        "dispatch_validation_required": True,
        "dispatch_validation_hint": (
            "Use `ait plan inspect <plan-id>` or `ait plan candidates` before `ait task start` "
            "to confirm the ref is still taskable."
        ),
        "item_count": len(items),
        "items": items,
    }


def _summarize_plan_linked_task(task: dict[str, Any]) -> dict[str, Any]:
    return {
        "task_id": task.get("task_id"),
        "title": task.get("title"),
        "status": task.get("status"),
        "planning_state": task.get("planning_state"),
        "origin_plan_revision_id": task.get("origin_plan_revision_id"),
        "plan_drift_state": task.get("plan_drift_state"),
    }


def _plan_task_link_indexes(
    tasks: list[dict[str, Any]],
) -> tuple[dict[tuple[str, str], list[dict[str, Any]]], dict[str, list[dict[str, Any]]]]:
    by_item: dict[tuple[str, str], list[dict[str, Any]]] = {}
    by_plan: dict[str, list[dict[str, Any]]] = {}
    for task in tasks:
        plan_id = _normalize_text_value(task.get("plan_id"))
        plan_item_ref = _normalize_text_value(task.get("plan_item_ref"))
        if plan_id is None:
            continue
        summarized = _summarize_plan_linked_task(task)
        by_plan.setdefault(plan_id, []).append(summarized)
        if plan_item_ref is not None:
            by_item.setdefault((plan_id, plan_item_ref), []).append(summarized)
    return by_item, by_plan


def _local_plan_publish_shadow(plan: dict[str, Any] | None) -> dict[str, Any] | None:
    if plan is None:
        return None
    head_revision = plan.get("head_revision") if isinstance(plan.get("head_revision"), dict) else {}
    head_publication_state = head_revision.get("publication_state")
    return {
        "plan_id": plan.get("plan_id"),
        "publication_state": plan.get("publication_state"),
        "head_publication_state": head_publication_state,
        "head_revision_id": plan.get("head_revision_id") or head_revision.get("plan_revision_id"),
        "head_revision_number": head_revision.get("revision_number"),
        "published_plan_id": plan.get("published_plan_id"),
        "published_head_revision_id": plan.get("published_head_revision_id"),
        "unpublished_head": head_publication_state not in {None, "published"},
    }


def _local_plan_publish_shadow_index(ctx: RepoContext) -> dict[str, dict[str, Any]]:
    index: dict[str, dict[str, Any]] = {}
    for row in list_local_plans(ctx):
        plan_id = _normalize_text_value(row.get("plan_id"))
        if plan_id is None:
            continue
        try:
            plan = get_local_plan(ctx, plan_id)
        except KeyError:
            continue
        shadow = _local_plan_publish_shadow(plan)
        if shadow is None:
            continue
        for key in (plan.get("plan_id"), plan.get("published_plan_id")):
            key_text = _normalize_text_value(key)
            if key_text is not None:
                index[key_text] = shadow
    return index


def _plan_revision_for_dispatch(plan: dict[str, Any], revision: dict[str, Any] | None = None) -> dict[str, Any]:
    current_revision = revision or plan.get("head_revision")
    return current_revision if isinstance(current_revision, dict) else {}


def _status_counts(rows: list[dict[str, Any]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for row in rows:
        status = str(row.get("status") or "unknown")
        counts[status] = counts.get(status, 0) + 1
    return counts


def _plan_dispatch_summary(
    plan: dict[str, Any],
    *,
    revision: dict[str, Any] | None = None,
    task_links_by_item: dict[tuple[str, str], list[dict[str, Any]]] | None = None,
    tasks_by_plan: dict[str, list[dict[str, Any]]] | None = None,
    local_shadow: dict[str, Any] | None = None,
) -> dict[str, Any]:
    plan_id = str(plan.get("plan_id") or "")
    current_revision = _plan_revision_for_dispatch(plan, revision=revision)
    raw_items = list(current_revision.get("items") or [])
    task_links_by_item = task_links_by_item or {}
    tasks_by_plan = tasks_by_plan or {}
    linked_plan_tasks = list(tasks_by_plan.get(plan_id, []))
    enriched_items: list[dict[str, Any]] = []
    open_items: list[dict[str, Any]] = []
    taskable_items: list[dict[str, Any]] = []
    unref_open_item_count = 0
    linked_open_item_count = 0

    for item in raw_items:
        plan_item_ref = _normalize_text_value(item.get("plan_item_ref"))
        linked_tasks = list(task_links_by_item.get((plan_id, plan_item_ref), [])) if plan_item_ref else []
        checkbox_state = str(item.get("checkbox_state") or "")
        taskable_blocker = None
        if checkbox_state != "open":
            taskable_blocker = "not_open"
        elif plan_item_ref is None:
            taskable_blocker = "missing_plan_item_ref"
            unref_open_item_count += 1
        elif linked_tasks:
            taskable_blocker = "linked_task_exists"
            linked_open_item_count += 1

        enriched = dict(item)
        enriched["linked_tasks"] = linked_tasks
        enriched["taskable"] = taskable_blocker is None
        enriched["taskable_blocker"] = taskable_blocker
        enriched_items.append(enriched)
        if checkbox_state == "open":
            open_items.append(enriched)
        if enriched["taskable"]:
            taskable_items.append(enriched)

    done_item_count = sum(1 for item in enriched_items if item.get("checkbox_state") == "done")
    local_shadow = local_shadow or _local_plan_publish_shadow(plan)
    return {
        "plan_id": plan.get("plan_id"),
        "title": plan.get("title"),
        "status": plan.get("status"),
        "repo_name": plan.get("repo_name"),
        "artifact_path": current_revision.get("artifact_path"),
        "artifact_selector": current_revision.get("artifact_selector"),
        "artifact_heading": current_revision.get("artifact_heading"),
        "plan_revision_id": current_revision.get("plan_revision_id"),
        "revision_number": current_revision.get("revision_number"),
        "publication_state": plan.get("publication_state"),
        "head_publication_state": current_revision.get("publication_state"),
        "published_plan_id": plan.get("published_plan_id"),
        "published_head_revision_id": plan.get("published_head_revision_id"),
        "local_publication": local_shadow,
        "local_unpublished_head": bool(local_shadow and local_shadow.get("unpublished_head")),
        "item_count": len(enriched_items),
        "open_item_count": len(open_items),
        "done_item_count": done_item_count,
        "unref_open_item_count": unref_open_item_count,
        "linked_open_item_count": linked_open_item_count,
        "taskable_item_count": len(taskable_items),
        "linked_task_count": len(linked_plan_tasks),
        "linked_task_status_counts": _status_counts(linked_plan_tasks),
        "items": enriched_items,
        "open_items": open_items,
        "taskable_items": taskable_items,
    }


def _plan_sync_task_start_advisory(
    ctx: RepoContext,
    *,
    plan_ids: list[str],
) -> dict[str, Any]:
    resolved_plan_ids: list[str] = []
    seen_plan_ids: set[str] = set()
    for plan_id in plan_ids:
        normalized = _normalize_text_value(plan_id)
        if normalized is None or normalized in seen_plan_ids:
            continue
        seen_plan_ids.add(normalized)
        resolved_plan_ids.append(normalized)

    if not resolved_plan_ids:
        return {
            "advisory_only": True,
            "dispatch_validation_still_required": True,
            "task_start_validation_hint": (
                "`ait task start` still performs final plan-item validation even when "
                "`ait plan sync` shows advisory taskable refs."
            ),
            "summary": {
                "touched_plan_count": 0,
                "taskable_plan_count": 0,
                "non_taskable_plan_count": 0,
                "taskable_item_count": 0,
            },
            "plans": [],
        }

    task_links_by_item, tasks_by_plan = _plan_task_link_indexes(list_local_tasks(ctx))
    plans: list[dict[str, Any]] = []
    for plan_id in resolved_plan_ids:
        try:
            plan = get_local_plan(ctx, plan_id)
        except KeyError:
            continue
        summary = _plan_dispatch_summary(
            plan,
            task_links_by_item=task_links_by_item,
            tasks_by_plan=tasks_by_plan,
        )
        blocked_open_items = [item for item in summary.get("open_items") or [] if not item.get("taskable")]
        taskable_items = list(summary.get("taskable_items") or [])
        taskable_refs = [
            str(item.get("plan_item_ref") or "")
            for item in taskable_items
            if str(item.get("plan_item_ref") or "").strip()
        ]
        advisory_plan = {
            "plan_id": summary.get("plan_id"),
            "plan_title": summary.get("title"),
            "plan_revision_id": summary.get("plan_revision_id"),
            "revision_number": summary.get("revision_number"),
            "artifact_path": summary.get("artifact_path"),
            "artifact_selector": summary.get("artifact_selector"),
            "item_count": summary.get("item_count"),
            "open_item_count": summary.get("open_item_count"),
            "taskable_item_count": summary.get("taskable_item_count"),
            "linked_task_count": summary.get("linked_task_count"),
            "linked_open_item_count": summary.get("linked_open_item_count"),
            "unref_open_item_count": summary.get("unref_open_item_count"),
            "taskable_items": taskable_items,
            "blocked_open_items": blocked_open_items,
            "taskable_refs": taskable_refs,
        }
        if taskable_refs:
            advisory_plan["task_start_command_hint"] = (
                f'ait task start --plan {plan_id} --plan-item-ref {taskable_refs[0]} '
                '--title "..." --intent "..."'
            )
        plans.append(advisory_plan)

    plans.sort(
        key=lambda row: (
            -int(row.get("taskable_item_count") or 0),
            -int(row.get("open_item_count") or 0),
            str(row.get("artifact_path") or ""),
            str(row.get("plan_id") or ""),
        )
    )
    return {
        "advisory_only": True,
        "dispatch_validation_still_required": True,
        "task_start_validation_hint": (
            "`ait task start` still performs final plan-item validation even when "
            "`ait plan sync` shows advisory taskable refs."
        ),
        "summary": {
            "touched_plan_count": len(plans),
            "taskable_plan_count": sum(1 for row in plans if int(row.get("taskable_item_count") or 0) > 0),
            "non_taskable_plan_count": sum(1 for row in plans if int(row.get("taskable_item_count") or 0) <= 0),
            "taskable_item_count": sum(int(row.get("taskable_item_count") or 0) for row in plans),
        },
        "plans": plans,
    }


def _plan_dispatch_scope_payload(
    ctx: RepoContext,
    *,
    local: bool,
    remote: Optional[str],
    include_all: bool,
) -> dict[str, Any]:
    use_local = _plan_uses_local_store(local, remote)
    cfg = load_config(ctx)
    if use_local:
        plan_ids = [
            str(row.get("plan_id") or "")
            for row in list_local_plans(ctx)
            if not _plan_status_is_historical(row.get("status"))
        ]
        plans = [get_local_plan(ctx, plan_id) for plan_id in plan_ids if plan_id]
        tasks = list_local_tasks(ctx)
        remote_name = None
        repo_name = cfg.get("repo_name") or ctx.root.name
        local_shadow_index: dict[str, dict[str, Any]] = {}
    else:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        plan_ids = [
            str(row.get("plan_id") or "")
            for row in remote_list_plans(remote_row["url"], repo_name)
            if not _plan_status_is_historical(row.get("status"))
        ]
        plans = [remote_get_plan(remote_row["url"], plan_id) for plan_id in plan_ids if plan_id]
        tasks = remote_list_tasks(remote_row["url"], repo_name)
        remote_name = remote_row.get("name") or remote
        local_shadow_index = _local_plan_publish_shadow_index(ctx)

    task_links_by_item, tasks_by_plan = _plan_task_link_indexes(tasks)
    scanned: list[dict[str, Any]] = []
    candidates: list[dict[str, Any]] = []
    for plan in plans:
        local_shadow = local_shadow_index.get(str(plan.get("plan_id") or "")) if not use_local else None
        summary = _plan_dispatch_summary(
            plan,
            task_links_by_item=task_links_by_item,
            tasks_by_plan=tasks_by_plan,
            local_shadow=local_shadow,
        )
        scanned.append(summary)
        if include_all or int(summary.get("taskable_item_count") or 0) > 0:
            candidates.append(summary)

    return {
        "scope": "local" if use_local else "remote",
        "remote": remote_name,
        "repo_name": repo_name,
        "summary": {
            "scanned_plan_count": len(scanned),
            "candidate_plan_count": len(candidates),
            "open_item_count": sum(int(row.get("open_item_count") or 0) for row in scanned),
            "taskable_item_count": sum(int(row.get("taskable_item_count") or 0) for row in scanned),
            "linked_task_count": sum(int(row.get("linked_task_count") or 0) for row in scanned),
            "local_unpublished_head_count": sum(1 for row in scanned if row.get("local_unpublished_head")),
        },
        "candidates": sorted(
            candidates,
            key=lambda row: (
                -int(row.get("taskable_item_count") or 0),
                -int(row.get("open_item_count") or 0),
                str(row.get("artifact_path") or ""),
                str(row.get("title") or ""),
            ),
        ),
    }
