from __future__ import annotations

import json
from typing import Any, Mapping

from ait_protocol.common import workflow_id_matches_any_namespace_prefix

from .ci_status import patchset_ci_status
from ..server_content_repo_lines import read_ref
from ..server_control import connect
from ..server_paths import ServerContext
from ..server_store import (
    get_attestation,
    get_change,
    get_change_for_repo,
    _repo_name_for_repo_id,
    get_land_request,
    get_patchset,
    get_patchset_for_repo,
    get_policy_status,
    get_repository,
    get_stack,
    get_stack_graph,
    get_task,
    get_task_for_repo,
    list_changes,
    list_patchsets,
    list_patchsets_for_repo,
    list_reviews,
)


def _repo_scoped_change(ctx: ServerContext, change_id: str, repo_name: str | None) -> dict[str, Any]:
    if repo_name is None:
        return get_change(ctx, change_id)
    try:
        return get_change_for_repo(ctx, repo_name, change_id)
    except KeyError:
        return get_change(ctx, change_id)


def _repo_scoped_task(ctx: ServerContext, task_id: str, repo_name: str | None) -> dict[str, Any]:
    if repo_name is None:
        return get_task(ctx, task_id)
    try:
        return get_task_for_repo(ctx, repo_name, task_id)
    except KeyError:
        return get_task(ctx, task_id)


def _repo_scoped_patchset(ctx: ServerContext, patchset_id: str, repo_name: str | None) -> dict[str, Any] | None:
    if patchset_id is None:
        return None
    if repo_name is None:
        return get_patchset(ctx, patchset_id)
    try:
        return get_patchset_for_repo(ctx, repo_name, patchset_id)
    except KeyError:
        return get_patchset(ctx, patchset_id)


def _list_patchsets_for_repo_with_fallback(ctx: ServerContext, repo_name: str | None, change_id: str) -> list[dict[str, Any]]:
    if repo_name is None:
        return list_patchsets(ctx, change_id)
    try:
        return list_patchsets_for_repo(ctx, repo_name, change_id)
    except KeyError:
        return list_patchsets(ctx, change_id)

def _legacy_read_models_module():
    from .. import read_models as legacy_read_models

    return legacy_read_models


def _blob_text(ctx: ServerContext, blob_id: str) -> tuple[str | None, bool]:
    return _legacy_read_models_module()._blob_text(ctx, blob_id)


def _snapshot_files(ctx: ServerContext, snapshot_id: str) -> dict[str, dict[str, Any]]:
    return _legacy_read_models_module()._snapshot_files(ctx, snapshot_id)


def _line_stats(old_text: str | None, new_text: str | None) -> tuple[int, int, str, bool]:
    return _legacy_read_models_module()._line_stats(old_text, new_text)


def _workflow_context(target: str, *, focus_type: str, focus_id: str, focus_title: str) -> dict[str, Any]:
    return _legacy_read_models_module()._workflow_context(
        target,
        focus_type=focus_type,
        focus_id=focus_id,
        focus_title=focus_title,
    )

def patchset_delta(
    ctx: ServerContext,
    patchset_id: str,
    against: str = "previous",
    *,
    change: dict[str, Any] | None = None,
    repo_name: str | None = None,
) -> dict[str, Any]:
    patchset = get_patchset(ctx, patchset_id) if repo_name is None else _repo_scoped_patchset(ctx, patchset_id, repo_name)
    if patchset is None:
        raise KeyError(f"Unknown patchset: {patchset_id}")
    resolved_change = change or _repo_scoped_change(ctx, patchset["change_id"], repo_name)
    repository = get_repository(ctx, _repo_name_for_repo_id(ctx, resolved_change.get("repo_id"), resolved_change["repo_name"]))

    left_snapshot_id: str | None
    against_label: str
    if against == "base":
        left_snapshot_id = patchset["base_snapshot_id"]
        against_label = "base"
    elif against == "previous":
        previous_number = patchset["patchset_number"] - 1
        previous = None
        if previous_number > 0:
            patchsets = _list_patchsets_for_repo_with_fallback(ctx, resolved_change["repo_name"], resolved_change["change_id"])
            for row in patchsets:
                if row["patchset_number"] == previous_number:
                    previous = row
                    break
        if previous is None:
            left_snapshot_id = patchset["base_snapshot_id"]
            against_label = "base"
        else:
            left_snapshot_id = previous["revision_snapshot_id"]
            against_label = previous["patchset_id"]
    else:
        if workflow_id_matches_any_namespace_prefix(
            against,
            "P",
            repository.get("id_namespace_prefix"),
            include_task_change_origins=True,
        ):
            other = _repo_scoped_patchset(ctx, against, resolved_change["repo_name"]) if resolved_change["repo_name"] else get_patchset(ctx, against)
            left_snapshot_id = other["revision_snapshot_id"]
            against_label = other["patchset_id"]
        else:
            raise ValueError(f"Unsupported compare target: {against}")

    right_snapshot_id = patchset["revision_snapshot_id"]
    left_files = _snapshot_files(ctx, left_snapshot_id)
    right_files = _snapshot_files(ctx, right_snapshot_id)
    left_paths = set(left_files)
    right_paths = set(right_files)

    file_rows: list[dict[str, Any]] = []
    insertions_total = 0
    deletions_total = 0
    ordered_paths = sorted(left_paths | right_paths)

    for path in ordered_paths:
        left = left_files.get(path)
        right = right_files.get(path)
        if left and not right:
            status = "deleted"
            old_text, old_is_text = _blob_text(ctx, left["blob_id"])
            new_text, new_is_text = (None, True)
        elif right and not left:
            status = "added"
            old_text, old_is_text = (None, True)
            new_text, new_is_text = _blob_text(ctx, right["blob_id"])
        else:
            assert left is not None and right is not None
            if left["sha256"] == right["sha256"] and left["mode"] == right["mode"]:
                continue
            status = "modified"
            old_text, old_is_text = _blob_text(ctx, left["blob_id"])
            new_text, new_is_text = _blob_text(ctx, right["blob_id"])
        insertions, deletions, diff_text, text_renderable = _line_stats(old_text, new_text)
        file_rows.append(
            {
                "path": path,
                "status": status,
                "insertions": insertions,
                "deletions": deletions,
                "diff_text": diff_text,
                "text_renderable": text_renderable and old_is_text and new_is_text,
                "old_blob_id": left["blob_id"] if left else None,
                "new_blob_id": right["blob_id"] if right else None,
            }
        )
        insertions_total += insertions
        deletions_total += deletions

    return {
        "patchset_id": patchset_id,
        "change_id": resolved_change["change_id"],
        "against": against_label,
        "base_snapshot_id": left_snapshot_id,
        "revision_snapshot_id": right_snapshot_id,
        "files_changed": len(file_rows),
        "insertions": insertions_total,
        "deletions": deletions_total,
        "summary": patchset["summary"],
        "files": file_rows,
        "cache_state": "computed",
    }



def _stack_summary(ctx: ServerContext, change: dict[str, Any]) -> dict[str, Any] | None:
    stack_ids = change.get("stack_ids") or []
    if not stack_ids:
        return None
    stack_id = stack_ids[0]
    stack = get_stack(ctx, stack_id)
    graph = get_stack_graph(ctx, stack_id)
    position = None
    for node in graph["nodes"]:
        if node["change_id"] == change["change_id"]:
            position = node["position"]
            break
    return {
        "stack_id": stack_id,
        "title": stack["title"],
        "position": position,
        "size": len(graph["nodes"]),
        "status": graph["status"],
        "graph": graph,
    }



def _latest_land_summary(ctx: ServerContext, change_id: str) -> dict[str, Any] | None:
    with connect(ctx) as conn:
        row = conn.execute(
            "select submission_id from land_requests where change_id = ? order by created_at desc limit 1",
            (change_id,),
        ).fetchone()
    if row is None:
        return None
    land = get_land_request(ctx, row["submission_id"])
    result = land.get("result") or {}
    return {
        "submission_id": land["submission_id"],
        "change_id": land["change_id"],
        "patchset_id": land["patchset_id"],
        "target_line": land["target_line"],
        "status": land["status"],
        "blocker_class": result.get("code"),
        "suggested_action": result.get("message"),
        "updated_at": land["updated_at"],
        "result": result,
    }



def _task_policy_missing_checks(policy: Mapping[str, Any]) -> list[str]:
    missing: list[str] = []
    for check in policy.get("checks") or []:
        status = str(check.get("status") or "").lower()
        if status in {"pending", "hard_fail", "soft_fail"}:
            missing.append(str(check.get("label") or check.get("name") or "unknown"))
    return missing


def _attestation_status(attestation: Mapping[str, Any] | None, key: str) -> str:
    evaluation = (attestation or {}).get("evaluation_summary") or {}
    return str(evaluation.get(key) or "pending").lower()


def _change_has_failed_validation(row: Mapping[str, Any]) -> bool:
    attestation = row.get("attestation_summary")
    policy = row.get("policy_summary") or {}
    review = row.get("review_summary") or {}
    if int(review.get("blocking") or 0) > 0:
        return True
    if str(policy.get("decision") or "pending").lower() in {"hard_fail", "soft_fail", "fail", "failed"}:
        return True
    for key in ("tests", "lint", "security", "security_scan", "license", "license_scan"):
        if _attestation_status(attestation, key) in {"fail", "failed", "hard_fail", "soft_fail"}:
            return True
    return False


def _change_is_landable(row: Mapping[str, Any]) -> bool:
    change = row.get("change") or {}
    if str(change.get("status") or "").lower() in {"landed", "archived"}:
        return True
    if row.get("current_patchset") is None:
        return False
    if row.get("attestation_summary") is None:
        return False
    if _change_has_failed_validation(row):
        return False
    if str((row.get("policy_summary") or {}).get("decision") or "pending").lower() != "pass":
        return False
    review = row.get("review_summary") or {}
    if int(review.get("approvals") or 0) < 1:
        return False
    freshness = row.get("freshness") or {}
    if not bool(freshness.get("base_is_fresh")):
        return False
    return True


def _task_review_packet(
    task: Mapping[str, Any],
    change_rows: list[dict[str, Any]],
    summary: Mapping[str, Any],
    aggregate_diff: Mapping[str, Any],
) -> dict[str, Any]:
    change_count = int(summary.get("change_count") or 0)
    open_change_count = int(summary.get("open_change_count") or 0)
    landed_change_count = int(summary.get("landed_change_count") or 0)
    patchset_count = int(summary.get("patchset_count") or 0)
    shared_boundary_crossed = patchset_count > 0 or landed_change_count > 0
    unresolved_gaps: list[str] = []
    landable_open_changes = 0

    if change_count == 0:
        unresolved_gaps.append("No linked change exists yet.")
    for row in change_rows:
        change = row["change"]
        change_id = str(change["change_id"])
        if str(change["status"]).lower() in {"landed", "archived"}:
            continue
        if _change_is_landable(row):
            landable_open_changes += 1
            continue
        if row["current_patchset"] is None:
            unresolved_gaps.append(f"{change_id} has no published patchset yet.")
            continue
        if row["attestation_summary"] is None:
            unresolved_gaps.append(f"{change_id} is missing attestation evidence.")
        if not bool((row.get("freshness") or {}).get("base_is_fresh")):
            unresolved_gaps.append(f"{change_id} is based on a stale main snapshot.")
        review = row["review_summary"]
        if int(review.get("blocking") or 0) > 0:
            unresolved_gaps.append(f"{change_id} has blocking review feedback.")
        elif int(review.get("approvals") or 0) < 1:
            unresolved_gaps.append(f"{change_id} still needs human approval.")
        policy = row["policy_summary"]
        if str(policy.get("decision") or "pending").lower() != "pass":
            missing = _task_policy_missing_checks(policy)
            detail = ", ".join(missing) if missing else str(policy.get("decision") or "pending")
            unresolved_gaps.append(f"{change_id} is still waiting on policy gates: {detail}.")

    if change_count == 0:
        acceptance_status = "defer"
        completion_summary = "No reviewable implementation has been linked to the task yet."
        suggested_next_action = "revise"
    elif unresolved_gaps:
        acceptance_status = "needs_followup"
        completion_summary = (
            f"{change_count} linked change(s) exist, but {len(unresolved_gaps)} outcome or readiness gaps still block acceptance."
        )
        suggested_next_action = "revise"
    elif open_change_count > 0:
        acceptance_status = "complete"
        completion_summary = f"{landable_open_changes} linked change(s) are ready for the final land path."
        suggested_next_action = "land"
    else:
        acceptance_status = "complete"
        completion_summary = f"All {landed_change_count} linked change(s) are already landed."
        suggested_next_action = "land" if str(task.get("status") or "").lower() != "completed" else "stop"

    effect_summary = (
        f"{aggregate_diff.get('unique_paths', 0)} unique path(s), "
        f"{aggregate_diff.get('insertions', 0)} insertion(s), and "
        f"{aggregate_diff.get('deletions', 0)} deletion(s) are currently visible from the task surface."
        if patchset_count > 0
        else "No published aggregate diff is available yet; the task is still below the shared review surface."
    )
    return {
        "source": "derived_from_workflow_state",
        "intent_summary": str(task.get("intent") or "").strip(),
        "completion_summary": completion_summary,
        "effect_summary": effect_summary,
        "acceptance_status": acceptance_status,
        "unresolved_gaps": unresolved_gaps,
        "suggested_next_action": suggested_next_action,
        "operator_confirmation_required": str(task.get("status") or "").lower() != "completed",
        "shared_boundary": {
            "crossed": shared_boundary_crossed,
            "state": "shared_workflow" if shared_boundary_crossed else "local_execution",
        },
    }


def _code_review_packet(
    change_rows: list[dict[str, Any]],
    summary: Mapping[str, Any],
    aggregate_diff: Mapping[str, Any],
) -> dict[str, Any]:
    change_count = int(summary.get("change_count") or 0)
    open_change_count = int(summary.get("open_change_count") or 0)
    file_entries = int(aggregate_diff.get("file_entries") or 0)
    touched_components: list[str] = []
    for file_row in aggregate_diff.get("files") or []:
        path = str(file_row.get("path") or "")
        if path and path not in touched_components:
            touched_components.append(path)
        if len(touched_components) >= 8:
            break

    regression_concerns: list[str] = []
    if change_count == 0:
        regression_concerns.append("No linked change exists yet.")
    for row in change_rows:
        change = row["change"]
        change_id = str(change["change_id"])
        if str(change["status"]).lower() in {"landed", "archived"}:
            continue
        if row["current_patchset"] is None:
            regression_concerns.append(f"{change_id} has no published patchset.")
            continue
        if row["attestation_summary"] is None:
            regression_concerns.append(f"{change_id} is missing attestation evidence.")
        if not bool((row.get("freshness") or {}).get("base_is_fresh")):
            regression_concerns.append(f"{change_id} is based on a stale base snapshot.")
        review = row["review_summary"]
        if int(review.get("blocking") or 0) > 0:
            regression_concerns.append(f"{change_id} has blocking review feedback.")
        policy = row["policy_summary"]
        decision = str(policy.get("decision") or "pending").lower()
        if decision != "pass":
            missing = _task_policy_missing_checks(policy)
            detail = ", ".join(missing) if missing else decision
            regression_concerns.append(f"{change_id} is waiting on policy gates: {detail}.")
        attestation = row["attestation_summary"]
        for key, label in (("tests", "tests"), ("lint", "lint"), ("security", "security"), ("license", "license")):
            status = _attestation_status(attestation, key)
            if status in {"fail", "failed", "hard_fail", "soft_fail"}:
                regression_concerns.append(f"{change_id} has failing {label} evidence.")

    approvals = sum(int((row["review_summary"] or {}).get("approvals") or 0) for row in change_rows)
    blocking = sum(int((row["review_summary"] or {}).get("blocking") or 0) for row in change_rows)
    passing_policy_rows = sum(1 for row in change_rows if str((row["policy_summary"] or {}).get("decision") or "").lower() == "pass")
    passing_tests_rows = sum(1 for row in change_rows if _attestation_status(row.get("attestation_summary"), "tests") == "pass")
    coverage_summary = (
        f"{passing_tests_rows}/{change_count} change(s) report passing tests; "
        f"{passing_policy_rows}/{change_count} change(s) currently pass policy; "
        f"approvals={approvals}; blocking={blocking}; file_entries={file_entries}."
        if change_count
        else "No reviewable implementation is available yet."
    )

    if change_count == 0:
        verdict = "needs_fix"
    elif any(_change_has_failed_validation(row) for row in change_rows):
        verdict = "high_risk"
    elif any(
        row["current_patchset"] is None or row["attestation_summary"] is None
        for row in change_rows
        if str(row["change"]["status"]).lower() not in {"landed", "archived"}
    ):
        verdict = "needs_fix"
    elif regression_concerns:
        verdict = "safe_with_minor_followup"
    else:
        verdict = "safe_to_promote"

    if verdict == "safe_to_promote" and open_change_count > 0:
        promotion_recommendation = "land when the task outcome is accepted"
    elif verdict == "safe_to_promote":
        promotion_recommendation = "complete the task record after confirming the landed outcome"
    elif verdict == "safe_with_minor_followup":
        promotion_recommendation = "clear the remaining follow-up before shared landing"
    else:
        promotion_recommendation = "revise the implementation before promotion"

    return {
        "source": "derived_from_change_patchset_policy_state",
        "touched_components": touched_components,
        "risk_summary": coverage_summary,
        "regression_concerns": regression_concerns,
        "coverage_summary": coverage_summary,
        "promotion_recommendation": promotion_recommendation,
        "verdict": verdict,
        "human_checkable": True,
    }


def _combined_review_recommendation(
    task: Mapping[str, Any],
    task_review: Mapping[str, Any],
    code_review: Mapping[str, Any],
    summary: Mapping[str, Any],
) -> dict[str, Any]:
    task_verdict = str(task_review.get("acceptance_status") or "defer")
    code_verdict = str(code_review.get("verdict") or "needs_fix")
    shared_boundary = task_review.get("shared_boundary") or {"crossed": False, "state": "local_execution"}
    open_change_count = int(summary.get("open_change_count") or 0)
    landed_change_count = int(summary.get("landed_change_count") or 0)

    if task_verdict == "redirect":
        action = "change direction"
        reason = "The task outcome needs redirection even before the technical slice is promoted."
    elif task_verdict == "not_accepted":
        action = "stop"
        reason = "The current task outcome is not accepted, so the implementation should not advance to landing."
    elif code_verdict in {"needs_fix", "high_risk"}:
        action = "revise"
        reason = "Technical safety issues still block promotion or landing."
    elif task_verdict == "complete" and code_verdict == "safe_to_promote":
        action = "land" if open_change_count > 0 else ("stop" if str(task.get("status") or "").lower() == "completed" else "land")
        reason = "The task outcome appears complete and the current technical slice looks safe to promote."
    elif task_verdict == "complete":
        action = "split follow-up task"
        reason = "The outcome appears complete, but a minor technical follow-up should stay visible."
    else:
        action = "revise"
        reason = "The task outcome still needs follow-up before it should be treated as accepted."

    if not bool(shared_boundary.get("crossed")):
        boundary_summary = "The task is still in local execution space; no shared patchset has been published yet."
    elif open_change_count > 0:
        boundary_summary = "The task has crossed into shared workflow and is waiting on the final review/policy/land path."
    elif landed_change_count > 0:
        boundary_summary = "The shared workflow path has already landed; finish the remaining task-level cleanup honestly."
    else:
        boundary_summary = "The task is in a shared workflow state."

    return {
        "task_review_verdict": task_verdict,
        "code_review_verdict": code_verdict,
        "action": action,
        "reason": reason,
        "shared_boundary": {
            "crossed": bool(shared_boundary.get("crossed")),
            "state": str(shared_boundary.get("state") or "local_execution"),
            "summary": boundary_summary,
        },
    }



def _timeline(ctx: ServerContext, change_id: str) -> list[dict[str, Any]]:
    change = _repo_scoped_change(ctx, change_id, None)
    patchsets = _list_patchsets_for_repo_with_fallback(ctx, change["repo_name"], change_id)
    patchset_ids = {row["patchset_id"] for row in patchsets}
    task_id = change["task_id"]
    with connect(ctx) as conn:
        rows = [
            dict(r)
            for r in conn.execute(
                "select event_type, entity_type, entity_id, payload_json, actor_identity, actor_type, created_at from events order by event_id asc"
            ).fetchall()
        ]
    out: list[dict[str, Any]] = []
    for row in rows:
        if row["entity_id"] in {change_id, task_id} or row["entity_id"] in patchset_ids:
            payload = json.loads(row["payload_json"])
            out.append({
                "event_type": row["event_type"],
                "entity_type": row["entity_type"],
                "entity_id": row["entity_id"],
                "payload": payload,
                "actor_identity": row["actor_identity"],
                "actor_type": row["actor_type"],
                "created_at": row["created_at"],
            })
    return out


def _task_timeline(ctx: ServerContext, task_id: str, change_ids: set[str], patchset_ids: set[str]) -> list[dict[str, Any]]:
    with connect(ctx) as conn:
        rows = [
            dict(r)
            for r in conn.execute(
                "select event_type, entity_type, entity_id, payload_json, actor_identity, actor_type, created_at from events order by event_id asc"
            ).fetchall()
        ]
    out: list[dict[str, Any]] = []
    tracked_ids = set(change_ids) | set(patchset_ids) | {task_id}
    for row in rows:
        if row["entity_id"] in tracked_ids:
            payload = json.loads(row["payload_json"])
            out.append({
                "event_type": row["event_type"],
                "entity_type": row["entity_type"],
                "entity_id": row["entity_id"],
                "payload": payload,
                "actor_identity": row["actor_identity"],
                "actor_type": row["actor_type"],
                "created_at": row["created_at"],
            })
    return out


def task_detail(ctx: ServerContext, task_id: str) -> dict[str, Any]:
    task = get_task(ctx, task_id)
    repo = get_repository(ctx, task["repo_name"])
    task_changes = [
        _repo_scoped_change(ctx, change["change_id"], task["repo_name"])
        for change in list_changes(ctx, task["repo_name"])
        if change["task_id"] == task_id
    ]
    task_changes.sort(key=lambda item: item["created_at"])

    change_rows: list[dict[str, Any]] = []
    aggregate_files: list[dict[str, Any]] = []
    patchset_ids: set[str] = set()
    change_ids = {change["change_id"] for change in task_changes}
    insertions_total = 0
    deletions_total = 0
    unique_paths: set[str] = set()

    for change in task_changes:
        current_patchset = _repo_scoped_patchset(ctx, change["current_patchset_id"], change["repo_name"])
        selected_patchset = _repo_scoped_patchset(ctx, change["selected_patchset_id"], change["repo_name"])
        display_patchset = selected_patchset or current_patchset
        review_summary = list_reviews(ctx, change["change_id"])
        policy_summary = get_policy_status(ctx, current_patchset["patchset_id"]) if current_patchset else {"decision": "pending", "checks": [], "lane": change["lane"]}
        try:
            attestation_summary = get_attestation(ctx, current_patchset["patchset_id"]) if current_patchset else None
        except KeyError:
            attestation_summary = None
        landing_summary = _latest_land_summary(ctx, change["change_id"])
        stack = _stack_summary(ctx, change)
        base_head = read_ref(ctx, change["repo_name"], change["base_line"]) if current_patchset else None
        freshness = {
            "base_is_fresh": bool(current_patchset and base_head == current_patchset["base_snapshot_id"]),
            "current_base_head": base_head,
        }
        delta = (
            patchset_delta(
                ctx,
                display_patchset["patchset_id"],
                "base",
                change=change,
                repo_name=change["repo_name"],
            )
            if display_patchset
            else None
        )
        if display_patchset is not None:
            patchset_ids.add(display_patchset["patchset_id"])
        if delta is not None:
            for file_row in delta["files"]:
                aggregate_files.append({
                    **file_row,
                    "change_id": change["change_id"],
                    "change_title": change["title"],
                    "patchset_id": display_patchset["patchset_id"],
                    "patchset_number": display_patchset["patchset_number"],
                })
                insertions_total += file_row["insertions"]
                deletions_total += file_row["deletions"]
                unique_paths.add(file_row["path"])
        change_rows.append({
            "change": change,
            "current_patchset": current_patchset,
            "selected_patchset": selected_patchset,
            "display_patchset": display_patchset,
            "review_summary": review_summary,
            "policy_summary": policy_summary,
            "attestation_summary": attestation_summary,
            "landing_summary": landing_summary,
            "stack": stack,
            "freshness": freshness,
            "delta": delta,
        })

    summary = {
        "change_count": len(change_rows),
        "open_change_count": sum(1 for row in change_rows if row["change"]["status"] not in {"landed", "archived"}),
        "landed_change_count": sum(1 for row in change_rows if row["change"]["status"] == "landed"),
        "patchset_count": len(patchset_ids),
    }
    aggregate_diff = {
        "change_count": len(change_rows),
        "patchset_count": len(patchset_ids),
        "file_entries": len(aggregate_files),
        "unique_paths": len(unique_paths),
        "insertions": insertions_total,
        "deletions": deletions_total,
        "files": aggregate_files,
    }
    task_review = _task_review_packet(task, change_rows, summary, aggregate_diff)
    code_review = _code_review_packet(change_rows, summary, aggregate_diff)
    combined_recommendation = _combined_review_recommendation(task, task_review, code_review, summary)

    return {
        "task": task,
        "repository": repo,
        "changes": change_rows,
        "workflow_context": _workflow_context("task", focus_type="task", focus_id=task_id, focus_title=task["title"]),
        "summary": summary,
        "aggregate_diff": aggregate_diff,
        "task_review": task_review,
        "code_review": code_review,
        "combined_recommendation": combined_recommendation,
        "timeline": _task_timeline(ctx, task_id, change_ids, patchset_ids),
    }


def change_detail(ctx: ServerContext, change_id: str) -> dict[str, Any]:
    change = _repo_scoped_change(ctx, change_id, None)
    task = _repo_scoped_task(ctx, change["task_id"], change["repo_name"])
    patchsets = _list_patchsets_for_repo_with_fallback(ctx, change["repo_name"], change_id)
    current_patchset = _repo_scoped_patchset(ctx, change["current_patchset_id"], change["repo_name"])
    selected_patchset = _repo_scoped_patchset(ctx, change["selected_patchset_id"], change["repo_name"])
    review_summary = list_reviews(ctx, change_id)
    policy_summary = get_policy_status(ctx, current_patchset["patchset_id"]) if current_patchset else {"decision": "pending", "checks": [], "lane": change["lane"]}
    try:
        attestation_summary = get_attestation(ctx, current_patchset["patchset_id"]) if current_patchset else None
    except KeyError:
        attestation_summary = None
    stack = _stack_summary(ctx, change)
    landing_summary = _latest_land_summary(ctx, change_id)
    delta = (
        patchset_delta(
            ctx,
            current_patchset["patchset_id"],
            "previous",
            change=change,
            repo_name=change["repo_name"],
        )
        if current_patchset
        else None
    )
    base_diff = (
        patchset_delta(
            ctx,
            current_patchset["patchset_id"],
            "base",
            change=change,
            repo_name=change["repo_name"],
        )
        if current_patchset
        else None
    )
    base_head = read_ref(ctx, change["repo_name"], change["base_line"]) if current_patchset else None
    freshness = {
        "base_is_fresh": bool(current_patchset and base_head == current_patchset["base_snapshot_id"]),
        "current_base_head": base_head,
    }
    return {
        "change": change,
        "repository": get_repository(ctx, _repo_name_for_repo_id(ctx, change.get("repo_id"), change["repo_name"])),
        "task": task,
        "workflow_context": _workflow_context("change", focus_type="change", focus_id=change_id, focus_title=change["title"]),
        "patchsets": patchsets,
        "current_patchset": current_patchset,
        "selected_patchset": selected_patchset,
        "review_summary": review_summary,
        "policy_summary": policy_summary,
        "attestation_summary": attestation_summary,
        "patchset_ci_status": patchset_ci_status(ctx, current_patchset["patchset_id"]) if current_patchset else None,
        "stack": stack,
        "landing_summary": landing_summary,
        "delta": delta,
        "base_diff": base_diff,
        "timeline": _timeline(ctx, change_id),
        "freshness": freshness,
    }



def stack_detail(ctx: ServerContext, stack_id: str) -> dict[str, Any]:
    stack = get_stack(ctx, stack_id)
    graph = get_stack_graph(ctx, stack_id)
    change_details = [_repo_scoped_change(ctx, node["change_id"], stack["repo_name"]) for node in graph["nodes"]]
    return {"stack": stack, "graph": graph, "changes": change_details}
