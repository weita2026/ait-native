from __future__ import annotations

from typing import Any

from ..server_paths import ServerContext

_CACHE_MISSING = object()
_CHANGE_BULK_QUERY_CHUNK_SIZE = 500


def _legacy_read_models_module():
    from .. import read_models as legacy_read_models

    return legacy_read_models


def _cache_value(
    cache: dict[str, dict[Any, Any]] | None,
    bucket: str,
    key: Any,
    loader,
):
    if cache is None:
        return loader()
    bucket_map = cache.setdefault(bucket, {})
    if key not in bucket_map:
        bucket_map[key] = loader()
    return bucket_map[key]


def _cache_value_missing_as_none(
    cache: dict[str, dict[Any, Any]] | None,
    bucket: str,
    key: Any,
    loader,
):
    if cache is None:
        try:
            return loader()
        except KeyError:
            return None
    bucket_map = cache.setdefault(bucket, {})
    if key not in bucket_map:
        try:
            bucket_map[key] = loader()
        except KeyError:
            bucket_map[key] = _CACHE_MISSING
    value = bucket_map[key]
    return None if value is _CACHE_MISSING else value


def _change_id_chunks(change_ids: list[str]) -> list[list[str]]:
    return [
        change_ids[index : index + _CHANGE_BULK_QUERY_CHUNK_SIZE]
        for index in range(0, len(change_ids), _CHANGE_BULK_QUERY_CHUNK_SIZE)
    ]


def _hydrate_change_rows(
    ctx: ServerContext,
    change_rows: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    if not change_rows:
        return []
    rm = _legacy_read_models_module()
    change_ids = [str(row["change_id"]) for row in change_rows]
    patchsets_by_change: dict[str, list[dict[str, Any]]] = {}
    stack_ids_by_change: dict[str, list[str]] = {}

    with rm.connect(ctx) as conn:
        for chunk in _change_id_chunks(change_ids):
            placeholders = ", ".join("?" for _ in chunk)
            patchset_rows = conn.execute(
                f"select * from patchsets where change_id in ({placeholders}) order by change_id, patchset_number",
                tuple(chunk),
            ).fetchall()
            for row in patchset_rows:
                patchsets_by_change.setdefault(str(row["change_id"]), []).append(dict(row))

            stack_rows = conn.execute(
                f"select change_id, stack_id from stack_changes where change_id in ({placeholders}) order by change_id, stack_id",
                tuple(chunk),
            ).fetchall()
            for row in stack_rows:
                stack_ids_by_change.setdefault(str(row["change_id"]), []).append(str(row["stack_id"]))

    hydrated: list[dict[str, Any]] = []
    for row in change_rows:
        change_id = str(row["change_id"])
        patchsets = patchsets_by_change.get(change_id, [])
        current_patchset = patchsets[-1] if patchsets else None
        selected_patchset_number = row.get("selected_patchset_number")
        selected_patchset = next(
            (patchset for patchset in patchsets if patchset["patchset_number"] == selected_patchset_number),
            None,
        )
        hydrated_row = dict(row)
        hydrated_row["current_patchset_id"] = current_patchset["patchset_id"] if current_patchset is not None else None
        hydrated_row["selected_patchset_id"] = selected_patchset["patchset_id"] if selected_patchset is not None else None
        hydrated_row["stack_ids"] = stack_ids_by_change.get(change_id, [])
        hydrated.append(hydrated_row)
    return hydrated


def _scoped_hydrated_changes(
    ctx: ServerContext,
    repo_name: str | None,
    *,
    cache: dict[str, dict[Any, Any]] | None = None,
) -> list[dict[str, Any]]:
    rm = _legacy_read_models_module()

    def _load() -> list[dict[str, Any]]:
        if repo_name is not None:
            change_rows = rm.list_changes(ctx, repo_name)
        else:
            with rm.connect(ctx) as conn:
                change_rows = [dict(row) for row in conn.execute("select * from changes order by updated_at desc").fetchall()]
        hydrated_changes = _hydrate_change_rows(ctx, change_rows)
        if cache is not None:
            change_bucket = cache.setdefault("change", {})
            for change in hydrated_changes:
                change_bucket[(repo_name, change["change_id"])] = change
        return hydrated_changes

    return _cache_value(cache, "changes_for_scope", repo_name, _load)


def _task_focus_candidate(
    change: dict[str, Any],
    current_patchset: dict[str, Any] | None,
    review_summary: dict[str, Any],
    policy_summary: dict[str, Any],
    attestation_summary: dict[str, Any] | None,
    freshness: dict[str, Any],
) -> dict[str, Any]:
    rm = _legacy_read_models_module()
    tests_state = rm._effective_validation_state(
        policy_summary,
        attestation_summary,
        key="tests",
        requirement_key="require_tests",
    )
    missing_requirements = rm._missing_requirements(policy_summary)
    decision = str(policy_summary.get("decision") or "pending")

    if change["status"] == "draft" and current_patchset is None:
        return {"rank": 0, "action": "publish_patchset", "reason": "No published patchset exists yet.", "primary_gate": None}
    if int(review_summary.get("blocking") or 0) > 0:
        return {
            "rank": 1,
            "action": "address_blocking_review",
            "reason": "Blocking review feedback is recorded on this change.",
            "primary_gate": "review",
        }
    if current_patchset is None:
        return {
            "rank": 2,
            "action": "publish_patchset",
            "reason": "Publish a patchset so the task has a reviewable surface.",
            "primary_gate": None,
        }
    if attestation_summary is None:
        return {
            "rank": 3,
            "action": "record_attestation",
            "reason": "Attestation is missing for the current patchset.",
            "primary_gate": "attestation",
        }
    if tests_state not in {"pass", "not_required"}:
        return {
            "rank": 4,
            "action": "complete_validation",
            "reason": f"Tests are {tests_state or 'pending'} for the current patchset.",
            "primary_gate": "ci",
        }
    if not freshness.get("base_is_fresh", True):
        return {
            "rank": 5,
            "action": "refresh_patchset",
            "reason": "The base line moved after this patchset was published.",
            "primary_gate": "freshness",
        }
    if decision != "pass":
        if "required_human_review" in missing_requirements and int(review_summary.get("approvals") or 0) <= 0:
            return {
                "rank": 6,
                "action": "review_change",
                "reason": "The change still needs a human approval.",
                "primary_gate": "review",
            }
        return {
            "rank": 7,
            "action": "satisfy_policy",
            "reason": ", ".join(missing_requirements) or "Policy evaluation is still pending.",
            "primary_gate": "policy",
        }
    if change["status"] in {"approved", "landable"} or (
        decision == "pass" and int(review_summary.get("blocking") or 0) == 0 and change["status"] in {"review", "gated"}
    ):
        return {"rank": 8, "action": "land_change", "reason": "This change is ready for landing.", "primary_gate": None}
    if change["status"] in rm.REVIEWABLE_CHANGE_STATES:
        return {"rank": 9, "action": "review_change", "reason": "This change is ready for nested review.", "primary_gate": "review"}
    return {"rank": 10, "action": "inspect_change", "reason": "Inspect the linked change from the task page.", "primary_gate": None}


def _task_next_action(
    task: dict[str, Any],
    workflow_state: str,
    focus_change: dict[str, Any] | None,
    *,
    total_changes: int,
) -> dict[str, Any]:
    rm = _legacy_read_models_module()
    if task["status"] == "completed":
        return {"code": "open_history", "label": "Open task history", "detail": "Review the completed task context.", "change_id": None}
    if rm.is_task_abandoned_status(task["status"]):
        return {"code": "open_history", "label": "Open task history", "detail": "Review the abandoned task context.", "change_id": None}
    if rm.is_task_later_promotion_excluded_status(task["status"]):
        return {
            "code": "open_history",
            "label": "Open task history",
            "detail": "Review why this task was excluded from later promotion.",
            "change_id": None,
        }
    if total_changes == 0:
        return {
            "code": "create_change",
            "label": "Open task and create the first change",
            "detail": "This task does not have any linked changes yet.",
            "change_id": None,
        }
    if workflow_state == "ready_to_complete":
        return {
            "code": "complete_task",
            "label": "Open task and complete it",
            "detail": "All linked changes are already landed.",
            "change_id": None,
        }
    if focus_change is None:
        return {"code": "open_task", "label": "Open task", "detail": "Use the task as the main workflow home.", "change_id": None}

    action_labels = {
        "publish_patchset": "Open task and publish a patchset",
        "address_blocking_review": "Open task and address requested changes",
        "record_attestation": "Open task and record attestation",
        "complete_validation": "Open task and complete validation",
        "refresh_patchset": "Open task and refresh the patchset",
        "satisfy_policy": "Open task and satisfy policy",
        "land_change": "Open task and land the focus change",
        "review_change": "Open task and review the focus change",
        "inspect_change": "Open task and inspect the linked change",
    }
    return {
        "code": focus_change["action"],
        "label": action_labels.get(focus_change["action"], "Open task"),
        "detail": focus_change["reason"],
        "change_id": focus_change["change_id"],
    }


def _task_queue_entry(
    ctx: ServerContext,
    task: dict[str, Any],
    task_changes: list[dict[str, Any]],
    *,
    cache: dict[str, dict[Any, Any]] | None = None,
) -> dict[str, Any]:
    rm = _legacy_read_models_module()
    task_changes = sorted(task_changes, key=lambda item: item["updated_at"], reverse=True)
    total_changes = len(task_changes)
    open_changes = 0
    reviewable_changes = 0
    landed_changes = 0
    patchset_ids: set[str] = set()
    blocking_reviews = 0
    missing_attestation = 0
    tests_pending = 0
    stale_base = 0
    policy_pending = 0
    ready_to_land = 0
    focus_change: dict[str, Any] | None = None

    for change in task_changes:
        current_patchset = _cache_value(
            cache,
            "patchset",
            (change["repo_name"], change.get("current_patchset_id")),
            lambda: rm._repo_scoped_patchset(ctx, change.get("current_patchset_id"), change["repo_name"]),
        )
        review_summary = _cache_value(
            cache,
            "reviews",
            change["change_id"],
            lambda: rm.list_reviews(ctx, change["change_id"]),
        )
        policy_summary = (
            _cache_value(
                cache,
                "policy",
                current_patchset["patchset_id"],
                lambda: rm.get_policy_status(ctx, current_patchset["patchset_id"]),
            )
            if current_patchset
            else {"decision": "pending", "checks": [], "lane": change["lane"]}
        )
        attestation_summary = (
            _cache_value_missing_as_none(
                cache,
                "attestation",
                current_patchset["patchset_id"],
                lambda: rm.get_attestation(ctx, current_patchset["patchset_id"]),
            )
            if current_patchset
            else None
        )
        base_head = (
            _cache_value(
                cache,
                "ref",
                (change["repo_name"], change["base_line"]),
                lambda: rm.read_ref(ctx, change["repo_name"], change["base_line"]),
            )
            if current_patchset
            else None
        )
        freshness = {
            "base_is_fresh": bool(current_patchset and base_head == current_patchset["base_snapshot_id"]),
            "current_base_head": base_head,
        }
        tests_state = rm._effective_validation_state(
            policy_summary,
            attestation_summary,
            key="tests",
            requirement_key="require_tests",
        )
        ci_summary = rm._task_ci_summary(
            ctx,
            change,
            current_patchset=current_patchset,
            review_summary=review_summary,
            policy_summary=policy_summary,
            freshness=freshness,
            tests_state=tests_state,
        )

        if change["status"] not in {"landed", "archived"}:
            open_changes += 1
        if change["status"] == "landed":
            landed_changes += 1
        if change["status"] in rm.REVIEWABLE_CHANGE_STATES:
            reviewable_changes += 1
        if current_patchset is not None:
            patchset_ids.add(current_patchset["patchset_id"])
        actionable_change = change["status"] not in {"landed", "archived"}
        if actionable_change and int(review_summary.get("blocking") or 0) > 0:
            blocking_reviews += 1
        if actionable_change and current_patchset is not None and attestation_summary is None:
            missing_attestation += 1
        if actionable_change and current_patchset is not None and tests_state not in {"pass", "not_required"}:
            tests_pending += 1
        if actionable_change and current_patchset is not None and not freshness["base_is_fresh"]:
            stale_base += 1
        if actionable_change and current_patchset is not None and str(policy_summary.get("decision") or "pending") != "pass":
            policy_pending += 1
        if (
            actionable_change
            and current_patchset is not None
            and str(policy_summary.get("decision") or "pending") == "pass"
            and int(review_summary.get("blocking") or 0) == 0
            and change["status"] in {"review", "gated", "approved", "landable"}
        ):
            ready_to_land += 1

        if actionable_change:
            candidate = _task_focus_candidate(change, current_patchset, review_summary, policy_summary, attestation_summary, freshness)
            candidate.update(
                {
                    "change_id": change["change_id"],
                    "title": change["title"],
                    "status": change["status"],
                    "updated_at": change["updated_at"],
                    "patchset_id": current_patchset["patchset_id"] if current_patchset else None,
                    "patchset_number": current_patchset["patchset_number"] if current_patchset else None,
                    "policy_decision": policy_summary.get("decision", "pending"),
                    "tests": tests_state,
                    "ci_summary": ci_summary,
                }
            )
            if focus_change is None or candidate["rank"] < focus_change["rank"]:
                focus_change = candidate

    if task["status"] == "completed":
        workflow_state = "completed"
        workflow_reason = "Task is already completed."
    elif rm.is_task_abandoned_status(task["status"]):
        workflow_state = "abandoned"
        workflow_reason = "Task is already abandoned."
    elif rm.is_task_later_promotion_excluded_status(task["status"]):
        workflow_state = rm.TASK_STATUS_LATER_PROMOTION_EXCLUDED
        workflow_reason = "Task is already excluded from later promotion."
    elif total_changes == 0:
        workflow_state = "planning"
        workflow_reason = "No linked changes exist yet."
    elif any(value > 0 for value in (blocking_reviews, missing_attestation, tests_pending, stale_base, policy_pending)):
        workflow_state = "attention_required"
        workflow_reason = focus_change["reason"] if focus_change is not None else "At least one linked change needs attention."
    elif ready_to_land > 0:
        workflow_state = "ready_to_land"
        workflow_reason = f"{ready_to_land} linked change(s) can land now."
    elif open_changes == 0 and landed_changes > 0:
        workflow_state = "ready_to_complete"
        workflow_reason = "All linked changes are landed; the task can complete."
    elif reviewable_changes > 0:
        workflow_state = "in_review"
        workflow_reason = f"{reviewable_changes} linked change(s) are in review."
    else:
        workflow_state = "in_progress"
        workflow_reason = f"{open_changes} linked change(s) are still in progress."

    updated_at = max([task["created_at"], *[change["updated_at"] for change in task_changes]])
    return {
        "task": task,
        "workflow": {"state": workflow_state, "reason": workflow_reason},
        "primary_gate": focus_change.get("primary_gate") if workflow_state == "attention_required" and focus_change else None,
        "primary_reason": focus_change.get("reason") if workflow_state == "attention_required" and focus_change else None,
        "changes": {
            "total": total_changes,
            "open": open_changes,
            "reviewable": reviewable_changes,
            "landed": landed_changes,
            "patchsets": len(patchset_ids),
        },
        "attention": {
            "blocking_reviews": blocking_reviews,
            "missing_attestation": missing_attestation,
            "tests_pending": tests_pending,
            "stale_base": stale_base,
            "policy_pending": policy_pending,
            "ready_to_land": ready_to_land,
        },
        "focus_change": focus_change,
        "ci_summary": focus_change.get("ci_summary") if focus_change else None,
        "next_action": _task_next_action(task, workflow_state, focus_change, total_changes=total_changes),
        "updated_at": updated_at,
    }


def _snapshot_ancestry(content_conn, head_snapshot_id: str | None) -> set[str]:
    ancestry: set[str] = set()
    current = head_snapshot_id
    while current:
        if current in ancestry:
            break
        ancestry.add(current)
        row = content_conn.execute(
            "select parent_snapshot_id from snapshots where snapshot_id = ?",
            (current,),
        ).fetchone()
        if row is None:
            break
        current = row["parent_snapshot_id"]
    return ancestry


def _task_audit_change_entry(
    ctx: ServerContext,
    change: dict[str, Any],
    *,
    target_line: str,
    target_ancestry: set[str],
) -> dict[str, Any]:
    rm = _legacy_read_models_module()
    current_patchset = rm._repo_scoped_patchset(ctx, change.get("current_patchset_id"), change["repo_name"])
    selected_patchset = rm._repo_scoped_patchset(ctx, change.get("selected_patchset_id"), change["repo_name"])
    display_patchset = selected_patchset or current_patchset
    landing_summary = rm._latest_land_summary(ctx, change["change_id"])
    revision_snapshot_id = display_patchset["revision_snapshot_id"] if display_patchset else None
    landed_snapshot_id = None
    if landing_summary is not None:
        result = landing_summary.get("result") or {}
        landed_snapshot_id = result.get("landed_snapshot_id")
    effective_snapshot_id = landed_snapshot_id if change["status"] == "landed" and landed_snapshot_id else revision_snapshot_id
    effective_on_target = bool(effective_snapshot_id and effective_snapshot_id in target_ancestry)
    stale_workflow_record = effective_on_target and change["status"] not in {"landed", "archived"}

    if change["status"] == "archived":
        target_state = "archived"
        target_reason = "This change is archived and no longer blocks task completion."
    elif effective_on_target and change["status"] == "landed":
        target_state = "landed_on_target"
        if landed_snapshot_id and landed_snapshot_id != revision_snapshot_id:
            target_reason = (
                f"The landed target snapshot is already reachable from {target_line}, "
                "including equivalent-tree land outcomes that reused an existing target snapshot."
            )
        else:
            target_reason = f"The selected patchset revision is already reachable from {target_line}."
    elif effective_on_target:
        target_state = "merged_on_target"
        target_reason = f"The selected patchset revision is already reachable from {target_line}, but workflow state is still {change['status']}."
    elif display_patchset is None:
        target_state = "no_patchset"
        target_reason = "This change does not have a published patchset yet."
    else:
        target_state = "not_on_target"
        target_reason = f"The selected patchset revision is not reachable from {target_line}."

    return {
        "change": change,
        "current_patchset": current_patchset,
        "selected_patchset": selected_patchset,
        "display_patchset": display_patchset,
        "landing_summary": landing_summary,
        "effective_on_target": effective_on_target,
        "stale_workflow_record": stale_workflow_record,
        "target_state": target_state,
        "target_reason": target_reason,
    }


def task_queue(
    ctx: ServerContext,
    repo_name: str | None = None,
    *,
    status: str | None = "active",
    repo_names: list[str] | None = None,
    cache: dict[str, dict[Any, Any]] | None = None,
) -> dict[str, Any]:
    rm = _legacy_read_models_module()
    normalized_status = rm._normalize_inbox_filter(status) or "active"
    if normalized_status not in {"active", "completed", "abandoned", rm.TASK_STATUS_LATER_PROMOTION_EXCLUDED, "canceled", "all"}:
        raise ValueError(f"Unsupported task queue status filter: {normalized_status}")
    if repo_name is not None:
        candidate_repos = [repo_name]
    elif repo_names is not None:
        candidate_repos = [name for name in repo_names if name]
    else:
        candidate_repos = [row["repo_name"] for row in rm.repository_index(ctx)["repositories"]]

    items: list[dict[str, Any]] = []
    for current_repo in candidate_repos:
        tasks = rm.list_tasks(ctx, current_repo)
        changes = _scoped_hydrated_changes(ctx, current_repo, cache=cache)
        changes_by_task: dict[str, list[dict[str, Any]]] = {}
        for change in changes:
            changes_by_task.setdefault(change["task_id"], []).append(change)
        for task in tasks:
            if not rm.task_status_matches_filter(task["status"], normalized_status):
                continue
            items.append(_task_queue_entry(ctx, task, changes_by_task.get(task["task_id"], []), cache=cache))

    items.sort(key=lambda item: item["updated_at"], reverse=True)
    items.sort(key=lambda item: rm.TASK_QUEUE_STATE_PRIORITY.get(item["workflow"]["state"], 99))
    return {
        "items": items,
        "count": len(items),
        "filters": {"repo_name": repo_name, "status": normalized_status},
        "summary": {
            "active": sum(1 for item in items if item["task"]["status"] == "active"),
            "completed": sum(1 for item in items if item["task"]["status"] == "completed"),
            "abandoned": sum(1 for item in items if str(item["task"]["status"] or "").lower() == "abandoned"),
            "later_promotion_excluded": sum(
                1 for item in items if str(item["task"]["status"] or "").lower() == rm.TASK_STATUS_LATER_PROMOTION_EXCLUDED
            ),
            "canceled": sum(1 for item in items if rm.is_task_abandoned_status(item["task"]["status"])),
            "attention_required": sum(1 for item in items if item["workflow"]["state"] == "attention_required"),
            "ready_to_land": sum(1 for item in items if item["workflow"]["state"] == "ready_to_land"),
            "ready_to_complete": sum(1 for item in items if item["workflow"]["state"] == "ready_to_complete"),
        },
    }


def task_audit(ctx: ServerContext, task_id: str, *, target_line: str = "main") -> dict[str, Any]:
    rm = _legacy_read_models_module()
    task = rm.get_task(ctx, task_id)
    repo = rm.get_repository(ctx, task["repo_name"])
    task_changes = [change for change in _scoped_hydrated_changes(ctx, task["repo_name"]) if change["task_id"] == task_id]
    task_changes.sort(key=lambda item: item["created_at"])
    queue_entry = _task_queue_entry(ctx, task, task_changes)

    target_head_snapshot_id = rm.read_ref(ctx, task["repo_name"], target_line)
    with rm.connect_content(ctx) as content_conn:
        target_ancestry = _snapshot_ancestry(content_conn, target_head_snapshot_id)

    change_rows = [
        _task_audit_change_entry(ctx, change, target_line=target_line, target_ancestry=target_ancestry)
        for change in task_changes
    ]
    open_change_count = queue_entry["changes"]["open"]
    landed_change_count = queue_entry["changes"]["landed"]
    effective_on_target_count = sum(1 for row in change_rows if row["effective_on_target"])
    open_on_target_count = sum(
        1
        for row in change_rows
        if row["effective_on_target"] and row["change"]["status"] not in {"landed", "archived"}
    )
    stale_workflow_count = sum(1 for row in change_rows if row["stale_workflow_record"])
    ready_to_complete = open_change_count == 0 and landed_change_count > 0
    effectively_complete_on_target = bool(change_rows) and all(
        row["change"]["status"] == "archived" or row["effective_on_target"] for row in change_rows
    ) and any(row["effective_on_target"] for row in change_rows)

    if task["status"] == "completed":
        verdict = "task_completed"
    elif rm.is_task_abandoned_status(task["status"]):
        verdict = "task_abandoned"
    elif rm.is_task_later_promotion_excluded_status(task["status"]):
        verdict = "task_later_promotion_excluded"
    elif not change_rows:
        verdict = "no_changes"
    elif ready_to_complete:
        verdict = "ready_to_complete"
    elif effectively_complete_on_target and stale_workflow_count:
        verdict = "workflow_stale_on_target"
    elif effectively_complete_on_target:
        verdict = "landed_on_target"
    elif open_on_target_count:
        verdict = "partially_on_target"
    else:
        verdict = "not_landed_on_target"

    workflow = dict(queue_entry["workflow"])
    if ready_to_complete:
        workflow = {
            "state": "ready_to_complete",
            "reason": "All linked changes are already landed.",
        }
    elif verdict == "workflow_stale_on_target":
        workflow = {
            "state": "workflow_stale_on_target",
            "reason": f"Linked changes already appear on {target_line}, but workflow records are still open.",
        }
    elif verdict == "landed_on_target":
        workflow = {
            "state": "landed_on_target",
            "reason": f"Linked changes already appear on {target_line}.",
        }

    recommended_action = queue_entry["next_action"]
    if ready_to_complete:
        recommended_action = {
            "code": "complete_task",
            "label": "Open task and complete it",
            "detail": "All linked changes are already landed.",
            "change_id": None,
        }
    elif verdict == "workflow_stale_on_target":
        recommended_action = {
            "code": "repair_workflow_state",
            "label": "Repair workflow state",
            "detail": f"{stale_workflow_count} linked change(s) already appear on {target_line}, but workflow records are still open.",
            "change_id": None,
        }
    elif verdict == "partially_on_target" and stale_workflow_count:
        recommended_action = {
            "code": "inspect_stale_workflow",
            "label": "Inspect stale workflow state",
            "detail": f"{stale_workflow_count} linked change(s) already appear on {target_line}, while other linked changes still need work.",
            "change_id": None,
        }

    return {
        "task": task,
        "repository": repo,
        "workflow": workflow,
        "queue_workflow": queue_entry["workflow"],
        "next_action": queue_entry["next_action"],
        "recommended_action": recommended_action,
        "target": {
            "line_name": target_line,
            "head_snapshot_id": target_head_snapshot_id,
            "ancestor_snapshot_count": len(target_ancestry),
        },
        "summary": {
            "change_count": len(change_rows),
            "open_change_count": open_change_count,
            "landed_change_count": landed_change_count,
            "patchset_count": queue_entry["changes"]["patchsets"],
            "effective_on_target_change_count": effective_on_target_count,
            "open_on_target_change_count": open_on_target_count,
            "stale_workflow_change_count": stale_workflow_count,
            "ready_to_complete": ready_to_complete,
            "effectively_complete_on_target": effectively_complete_on_target,
            "stale_workflow_records": stale_workflow_count > 0,
            "verdict": verdict,
        },
        "changes": change_rows,
    }
