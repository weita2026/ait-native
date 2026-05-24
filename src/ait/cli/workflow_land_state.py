from __future__ import annotations

import json
from typing import Any

from ait_protocol.common import (
    derive_policy_author_class,
    derive_policy_content_class,
    is_structured_code_review_summary,
    normalize_optional_text,
)

from ..remote_client import (
    RemoteError,
    get_attestation as remote_get_attestation,
    get_change as remote_get_change,
    get_patchset as remote_get_patchset,
    get_policy as remote_get_policy,
    get_remote_line,
    get_task as remote_get_task,
    list_reviews as remote_list_reviews,
)
from ..store import (
    RepoContext,
    current_line,
    get_line,
    workspace_status as local_workspace_status,
)
from .remote_repository_defaults import _remote_tuple
from .workflow_land_command_hints import (
    _workflow_land_command_hints,
    _workflow_land_next_action,
    _workflow_land_suggested_commands,
)
from .workflow_land_publish import _current_worktree_retarget_state


def _workflow_land_step(
    code: str,
    label: str,
    status: str,
    detail: str,
    command: str | None = None,
) -> dict[str, Any]:
    payload = {
        "code": code,
        "label": label,
        "status": status,
        "detail": detail,
    }
    if command:
        payload["command"] = command
    return payload


def _workflow_patchset_changed_paths(patchset: dict[str, Any] | None) -> list[str]:
    if not isinstance(patchset, dict):
        return []
    diff_stats = patchset.get("diff_stats")
    if not isinstance(diff_stats, dict):
        raw = patchset.get("diff_stats_json")
        try:
            diff_stats = json.loads(raw) if raw else {}
        except Exception:
            diff_stats = {}
    paths = diff_stats.get("paths") if isinstance(diff_stats.get("paths"), dict) else {}
    changed_paths: list[str] = []
    for key in ("added", "deleted", "modified"):
        for path in paths.get(key) or []:
            text = str(path).strip()
            if text:
                changed_paths.append(text)
    return sorted(dict.fromkeys(changed_paths))


def _workflow_code_review_summary_count(review_summary: dict[str, Any], patchset_id: str | None) -> int:
    current_patchset_id = str(review_summary.get("current_patchset_id") or "").strip()
    reviews = review_summary.get("reviews")
    if isinstance(reviews, list):
        return sum(
            1
            for review in reviews
            if isinstance(review, dict)
            and (not patchset_id or str(review.get("patchset_id") or "").strip() == patchset_id)
            and str(review.get("action") or "") == "code_review_summary"
            and is_structured_code_review_summary(review.get("comment"))
        )
    if "code_review_summaries" in review_summary and (not patchset_id or current_patchset_id == patchset_id):
        return int(review_summary.get("code_review_summaries") or 0)
    return 0


def _workflow_review_decision_lane(action: str) -> str | None:
    if action in {"task_approve", "task_request_changes", "task_defer"}:
        return "task"
    if action in {"approve", "request_changes", "defer"}:
        return "team"
    return None


def _workflow_normalized_reviewer_identity(value: Any | None) -> str | None:
    text = normalize_optional_text(value)
    return text.casefold() if text else None


def _workflow_review_lane_counts(review_summary: dict[str, Any], patchset_id: str | None) -> dict[str, int]:
    reviews = review_summary.get("reviews")
    if isinstance(reviews, list):
        latest_by_reviewer_lane: dict[tuple[str, str], dict[str, Any]] = {}
        blocking_count = 0
        for review in reviews:
            if not isinstance(review, dict):
                continue
            if patchset_id and str(review.get("patchset_id") or "").strip() != patchset_id:
                continue
            action = str(review.get("action") or "").strip()
            decision_lane = _workflow_review_decision_lane(action)
            if decision_lane:
                latest_by_reviewer_lane[(str(review.get("reviewer") or ""), decision_lane)] = review
            if action in {"request_changes", "task_request_changes"} or bool(review.get("blocking")):
                blocking_count += 1
        task_approvals = sum(
            1 for review in latest_by_reviewer_lane.values() if str(review.get("action") or "") == "task_approve"
        )
        team_approvals = sum(
            1 for review in latest_by_reviewer_lane.values() if str(review.get("action") or "") == "approve"
        )
        approval_reviewers = {
            str(review.get("reviewer") or "")
            for review in latest_by_reviewer_lane.values()
            if str(review.get("action") or "") in {"task_approve", "approve"}
        }
        return {
            "task_approvals": task_approvals,
            "team_approvals": team_approvals,
            "human_approvals": len(approval_reviewers),
            "eligible_human_approvals": len(approval_reviewers),
            "human_task_approvals": task_approvals,
            "eligible_task_approvals": task_approvals,
            "approvals": len(approval_reviewers),
            "blocking": blocking_count,
        }
    return {
        "task_approvals": int(review_summary.get("task_approvals") or 0),
        "team_approvals": int(review_summary.get("team_approvals") or 0),
        "human_approvals": int(review_summary.get("human_approvals") or review_summary.get("approvals") or 0),
        "eligible_human_approvals": int(review_summary.get("approvals") or 0),
        "human_task_approvals": int(review_summary.get("human_task_approvals") or review_summary.get("task_approvals") or 0),
        "eligible_task_approvals": int(review_summary.get("task_approvals") or 0),
        "approvals": int(review_summary.get("approvals") or 0),
        "blocking": int(review_summary.get("blocking") or 0),
    }


def _workflow_requires_code_review_summary(
    patchset: dict[str, Any] | None,
    attestation: dict[str, Any] | None,
    policy: dict[str, Any] | None,
) -> bool:
    if not isinstance(patchset, dict):
        return False
    effective_requirements = policy.get("effective_requirements") if isinstance(policy, dict) else {}
    if isinstance(effective_requirements, dict) and "require_code_review_summary" in effective_requirements:
        return bool(effective_requirements.get("require_code_review_summary"))
    author_mode = (
        attestation.get("author_mode")
        if isinstance(attestation, dict) and attestation.get("author_mode")
        else patchset.get("author_mode")
    )
    return (
        derive_policy_content_class(_workflow_patchset_changed_paths(patchset)) == "code_change"
        and derive_policy_author_class(str(author_mode or "")) == "ai_related"
    )


def _workflow_landed_payload(
    ctx: RepoContext,
    *,
    change: dict[str, Any],
    task: dict[str, Any],
    patchset: dict[str, Any] | None,
    patchset_source: str | None,
    ignore_workspace_authoring: bool,
) -> dict[str, Any]:
    workspace = local_workspace_status(ctx)
    current_line_name = str(workspace.get("current_line") or current_line(ctx))
    current_line_info = get_line(ctx, current_line_name)
    revision_snapshot_id = str(current_line_info.get("head_snapshot_id") or workspace.get("baseline_snapshot_id") or "").strip() or None
    target_line = str(change.get("base_line") or "main")
    command_hints = _workflow_land_command_hints(
        ctx,
        change_id=str(change.get("change_id") or ""),
        task_id=str(task.get("task_id") or ""),
        patchset=patchset,
        base_line_name=target_line,
        target_line=target_line,
        worktree_retarget=None,
        review_blocking=0,
        requires_code_review_summary=False,
    )
    next_action = _workflow_land_next_action(
        change=change,
        task=task,
        workspace=workspace,
        patchset=patchset,
        base_is_fresh=True,
        workspace_matches_patchset=True,
        patchset_is_authoritative=False,
        attestation=None,
        tests_state="pass",
        requires_code_review_summary=False,
        code_review_summary_count=0,
        review_blocking=0,
        review_approvals=1,
        policy_decision="pass",
        target_line=target_line,
        ignore_workspace_authoring=ignore_workspace_authoring,
        commands=command_hints,
    )
    patchset_label = str((patchset or {}).get("patchset_id") or "").strip()
    patchset_detail = (
        f"Patchset `{patchset_label}` is already part of the landed history for this change."
        if patchset_label
        else "A landed change already implies patchset publication succeeded earlier."
    )
    steps = [
        _workflow_land_step(
            "snapshot",
            "Snapshot",
            "done",
            "No additional authoring snapshot is required because the change is already landed.",
        ),
        _workflow_land_step(
            "patchset",
            "Patchset",
            "done",
            patchset_detail,
        ),
        _workflow_land_step(
            "attestation",
            "Attestation",
            "done",
            "Landing already succeeded, so attestation gating has already cleared.",
        ),
        _workflow_land_step(
            "review",
            "Review",
            "done",
            "Landing already succeeded, so review requirements were already satisfied.",
        ),
        _workflow_land_step(
            "policy",
            "Policy",
            "done",
            "Landing already succeeded, so policy has already cleared.",
        ),
        _workflow_land_step(
            "land",
            "Land",
            "done",
            f"Change `{change.get('change_id') or 'unknown'}` already landed on `{target_line}`.",
        ),
    ]
    suggested_commands = []
    next_command = str(next_action.get("command") or "").strip()
    if next_command:
        suggested_commands.append(next_command)
    return {
        "change": change,
        "task": task,
        "patchset": patchset,
        "patchset_source": patchset_source,
        "workspace": {
            "clean": bool(workspace.get("clean")),
            "changed_count": int(workspace.get("changed_count") or 0),
            "current_line": current_line_name,
            "head_snapshot_id": revision_snapshot_id,
            "workspace_status": "clean" if workspace.get("clean") else "dirty",
            "workspace_matches_patchset": True,
        },
        "base_line": {
            "line_name": target_line,
            "head_snapshot_id": None,
        },
        "review": {
            "approvals": 1,
            "blocking": 0,
            "task_approvals": 1,
            "team_approvals": 0,
            "current_patchset_id": patchset_label or None,
            "reviews": [],
        },
        "attestation": None,
        "policy": {
            "decision": "pass",
            "checks": [],
            "status_source": "landed_fast_path",
        },
        "freshness": {
            "base_is_fresh": True,
            "preflight_state": "not_required",
            "recovery_required": False,
            "worktree_needs_retarget": False,
            "rebase_state": "idle",
            "remote_base_snapshot_id": None,
            "patchset_base_snapshot_id": None,
            "patchset_revision_snapshot_id": str((patchset or {}).get("revision_snapshot_id") or "").strip() or None,
        },
        "steps": steps,
        "next_action": next_action,
        "suggested_commands": suggested_commands,
    }


def _workflow_land_payload(
    ctx: RepoContext,
    *,
    change_id: str | None,
    patchset_id: str | None,
    remote_name: str | None,
    ignore_workspace_authoring: bool = False,
    patchset_is_authoritative: bool = False,
) -> dict[str, Any]:
    if not change_id and not patchset_id:
        raise KeyError("Provide CHANGE_ID so the workflow land helper can resolve a change.")
    remote_row, repo_name = _remote_tuple(ctx, remote_name)
    resolved_patchset_id = str(patchset_id or "").strip().upper() or None
    resolved_change_id = str(change_id or "").strip().upper() or None

    explicit_patchset: dict[str, Any] | None = None
    if resolved_patchset_id:
        explicit_patchset = remote_get_patchset(remote_row["url"], resolved_patchset_id, repo_name=repo_name)
        patchset_change_id = str(explicit_patchset.get("change_id") or "").strip().upper() or None
        if resolved_change_id and patchset_change_id and patchset_change_id != resolved_change_id:
            raise KeyError(f"Patchset {explicit_patchset['patchset_id']} does not belong to change {resolved_change_id}.")
        resolved_change_id = resolved_change_id or patchset_change_id
    if not resolved_change_id:
        raise KeyError("Could not resolve a change for the workflow land helper.")

    change = remote_get_change(remote_row["url"], resolved_change_id, repo_name=repo_name)
    task = remote_get_task(remote_row["url"], str(change["task_id"]), repo_name=repo_name)
    selected_patchset_id = resolved_patchset_id or str(change.get("selected_patchset_id") or change.get("current_patchset_id") or "").strip()
    patchset_source = "explicit" if resolved_patchset_id else ("selected" if change.get("selected_patchset_id") else "current")
    if str(change.get("status") or "") == "landed":
        landed_patchset = explicit_patchset
        if landed_patchset is None and selected_patchset_id:
            landed_patchset = {
                "patchset_id": selected_patchset_id,
                "change_id": resolved_change_id,
            }
        return _workflow_landed_payload(
            ctx,
            change=change,
            task=task,
            patchset=landed_patchset,
            patchset_source=patchset_source if landed_patchset is not None else None,
            ignore_workspace_authoring=ignore_workspace_authoring,
        )
    patchset: dict[str, Any] | None = explicit_patchset
    if selected_patchset_id and patchset is None:
        patchset = remote_get_patchset(remote_row["url"], selected_patchset_id, repo_name=repo_name)
    if patchset is not None and str(patchset.get("change_id") or "").strip().upper() != resolved_change_id:
        raise KeyError(f"Patchset {patchset['patchset_id']} does not belong to change {resolved_change_id}.")

    workspace = local_workspace_status(ctx)
    current_line_name = str(workspace.get("current_line") or current_line(ctx))
    current_line_info = get_line(ctx, current_line_name)
    revision_snapshot_id = str(current_line_info.get("head_snapshot_id") or workspace.get("baseline_snapshot_id") or "").strip() or None

    base_line_name = str(change.get("base_line") or "main")
    base_line = get_remote_line(remote_row["url"], repo_name, base_line_name)
    remote_base_snapshot_id = str(base_line.get("head_snapshot_id") or "").strip() or None

    review_summary = remote_list_reviews(
        remote_row["url"],
        resolved_change_id,
        repo_name=repo_name,
        exact_id=True,
    )
    try:
        attestation = (
            remote_get_attestation(
                remote_row["url"],
                str(patchset["patchset_id"]),
                repo_name=repo_name,
                exact_id=True,
            )
            if patchset is not None
            else None
        )
    except (KeyError, RemoteError):
        attestation = None
    try:
        policy = (
            remote_get_policy(
                remote_row["url"],
                str(patchset["patchset_id"]),
                repo_name=repo_name,
                exact_id=True,
            )
            if patchset is not None
            else None
        )
    except (KeyError, RemoteError):
        policy = None

    tests_state = ""
    if isinstance(attestation, dict):
        evaluation_summary = attestation.get("evaluation_summary") if isinstance(attestation.get("evaluation_summary"), dict) else {}
        tests_state = str(evaluation_summary.get("tests") or "").strip()
    if not tests_state and isinstance(policy, dict):
        for check in policy.get("checks") or []:
            if not isinstance(check, dict) or str(check.get("name") or "") != "tests":
                continue
            tests_state = str(check.get("status") or "").strip()
            break

    patchset_base_snapshot_id = str((patchset or {}).get("base_snapshot_id") or "").strip() or None
    patchset_revision_snapshot_id = str((patchset or {}).get("revision_snapshot_id") or "").strip() or None
    base_is_fresh = True if patchset is None else patchset_base_snapshot_id == remote_base_snapshot_id
    workspace_matches_patchset = (
        True
        if patchset_is_authoritative and patchset is not None
        else None
        if patchset is None or revision_snapshot_id is None
        else revision_snapshot_id == patchset_revision_snapshot_id
    )
    review_lane_counts = _workflow_review_lane_counts(
        review_summary,
        str((patchset or {}).get("patchset_id") or "").strip() or None,
    )
    review_blocking = int(review_lane_counts.get("blocking") or 0)
    review_approvals = int(review_lane_counts.get("approvals") or 0)
    task_review_approvals = int(review_lane_counts.get("task_approvals") or 0)
    team_review_approvals = int(review_lane_counts.get("team_approvals") or 0)
    code_review_summary_count = _workflow_code_review_summary_count(
        review_summary,
        str((patchset or {}).get("patchset_id") or "").strip() or None,
    )
    policy_decision = str((policy or {}).get("decision") or "pending").strip() or "pending"
    requires_code_review_summary = _workflow_requires_code_review_summary(patchset, attestation, policy)

    target_line = base_line_name
    worktree_retarget = _current_worktree_retarget_state(ctx, resolved_change_id)
    command_hints = _workflow_land_command_hints(
        ctx,
        change_id=resolved_change_id,
        task_id=str(task["task_id"]),
        patchset=patchset,
        base_line_name=base_line_name,
        target_line=target_line,
        worktree_retarget=worktree_retarget if isinstance(worktree_retarget, dict) else None,
        review_blocking=review_blocking,
        requires_code_review_summary=requires_code_review_summary,
    )
    publish_command = command_hints["publish_command"]
    patchset_ci_command = command_hints["patchset_ci_command"]
    attestation_command = command_hints["attestation_command"]
    code_review_summary_command = command_hints["code_review_summary_command"]
    review_command = command_hints["review_command"]
    policy_command = command_hints["policy_command"]
    land_command = command_hints["land_command"]
    task_complete_command = command_hints["task_complete_command"]

    steps: list[dict[str, Any]] = []
    if ignore_workspace_authoring:
        steps.append(
            _workflow_land_step(
                "snapshot",
                "Snapshot",
                "done",
                "Batch remote land is using completed local lineage and does not require repo-root authoring state.",
            )
        )
    elif not bool(workspace.get("clean")):
        steps.append(
            _workflow_land_step(
                "snapshot",
                "Snapshot",
                "pending",
                "Workspace changes are still dirty, so publishable land state needs a fresh snapshot first.",
                'ait snapshot create --message "reviewable checkpoint"',
            )
        )
    else:
        steps.append(
            _workflow_land_step(
                "snapshot",
                "Snapshot",
                "done",
                f"The current line `{current_line_name}` is already captured at `{revision_snapshot_id or 'unknown'}`.",
            )
        )

    if patchset is None:
        steps.append(
            _workflow_land_step(
                "patchset",
                "Patchset",
                "pending",
                "No published patchset exists yet for this change.",
                publish_command,
            )
        )
    elif patchset_is_authoritative:
        steps.append(
            _workflow_land_step(
                "patchset",
                "Patchset",
                "done",
                f"Patchset `{patchset['patchset_id']}` is already prepared for batch remote land.",
            )
        )
    elif isinstance(worktree_retarget, dict) and str(worktree_retarget.get("rebase_state") or "idle") == "conflicted":
        steps.append(
            _workflow_land_step(
                "patchset",
                "Patchset",
                "stale",
                f"The bound worktree is paused on conflicted rebase paths: {', '.join((worktree_retarget.get('rebase_conflict_paths') or [])[:5]) or 'resolve conflicts first'}.",
                publish_command,
            )
        )
    elif isinstance(worktree_retarget, dict) and bool(worktree_retarget.get("needs_retarget")):
        steps.append(
            _workflow_land_step(
                "patchset",
                "Patchset",
                "stale",
                f"The bound worktree still forks from `{worktree_retarget.get('fork_snapshot_id')}` while `{base_line_name}` now points at `{worktree_retarget.get('target_base_snapshot_id')}`.",
                publish_command,
            )
        )
    elif not base_is_fresh:
        steps.append(
            _workflow_land_step(
                "patchset",
                "Patchset",
                "stale",
                f"The base line `{base_line_name}` moved to `{remote_base_snapshot_id}` after patchset `{patchset['patchset_id']}` was published.",
                publish_command,
            )
        )
    elif workspace_matches_patchset is False:
        steps.append(
            _workflow_land_step(
                "patchset",
                "Patchset",
                "stale",
                f"The current line head `{revision_snapshot_id}` no longer matches patchset `{patchset['patchset_id']}`.",
                publish_command,
            )
        )
    else:
        steps.append(
            _workflow_land_step(
                "patchset",
                "Patchset",
                "done",
                f"Patchset `{patchset['patchset_id']}` is published for `{resolved_change_id}`.",
            )
        )

    if patchset is None:
        steps.append(_workflow_land_step("attestation", "Attestation", "waiting", "Attestation starts after a patchset exists."))
    elif attestation is None:
        detail = (
            f"No attestation is recorded yet for patchset `{patchset['patchset_id']}`. Run patchset CI so the system can write attestation evidence automatically."
            if patchset_ci_command
            else f"No attestation is recorded yet for patchset `{patchset['patchset_id']}`."
        )
        steps.append(
            _workflow_land_step(
                "attestation",
                "Attestation",
                "pending",
                detail,
                attestation_command,
            )
        )
    elif tests_state and tests_state not in {"pass", "not_required"}:
        step_status = "pending" if tests_state == "pending" else "blocked"
        detail = (
            f"Patchset CI currently reports tests `{tests_state}` for patchset `{patchset['patchset_id']}`."
            if patchset_ci_command
            else f"Tests are currently `{tests_state}` for patchset `{patchset['patchset_id']}`."
        )
        steps.append(
            _workflow_land_step(
                "attestation",
                "Attestation",
                step_status,
                detail,
                attestation_command,
            )
        )
    else:
        steps.append(
            _workflow_land_step(
                "attestation",
                "Attestation",
                "done",
                f"Attestation `{attestation['attestation_id']}` is recorded" + (f" with tests `{tests_state}`." if tests_state else "."),
            )
        )

    if patchset is None:
        steps.append(_workflow_land_step("review", "Review", "waiting", "Review starts after a patchset exists."))
    elif requires_code_review_summary and code_review_summary_count <= 0:
        steps.append(
            _workflow_land_step(
                "review",
                "Review",
                "pending",
                "Code review: pending; Task review: waiting; Team review: not required.",
                code_review_summary_command,
            )
        )
    elif review_blocking > 0:
        steps.append(
            _workflow_land_step(
                "review",
                "Review",
                "blocked",
                f"Code review: {'done' if code_review_summary_count > 0 else 'pending'}; Task review: {'approved' if task_review_approvals > 0 else 'pending'}; Team review: {team_review_approvals} approval(s). Blocking review feedback exists on `{resolved_change_id}`.",
                review_command,
            )
        )
    elif review_approvals <= 0:
        steps.append(
            _workflow_land_step(
                "review",
                "Review",
                "pending",
                f"Code review: {'done' if code_review_summary_count > 0 or not requires_code_review_summary else 'pending'}; Task review: pending; Team review: not required.",
                review_command,
            )
        )
    else:
        steps.append(
            _workflow_land_step(
                "review",
                "Review",
                "done",
                f"Code review: {'done' if code_review_summary_count > 0 or not requires_code_review_summary else 'pending'}; Task review: {'approved' if task_review_approvals > 0 else 'not recorded'}; Team review: {team_review_approvals} approval(s).",
            )
        )

    if patchset is None:
        steps.append(_workflow_land_step("policy", "Policy", "waiting", "Policy evaluation starts after a patchset exists."))
    elif policy is None:
        steps.append(
            _workflow_land_step(
                "policy",
                "Policy",
                "pending",
                f"Policy has not been evaluated yet for patchset `{patchset['patchset_id']}`.",
                policy_command,
            )
        )
    elif policy_decision != "pass":
        steps.append(
            _workflow_land_step(
                "policy",
                "Policy",
                "blocked" if review_blocking > 0 else "pending",
                f"Policy is currently `{policy_decision}` for patchset `{patchset['patchset_id']}`.",
                policy_command,
            )
        )
    else:
        steps.append(
            _workflow_land_step(
                "policy",
                "Policy",
                "done",
                f"Policy passed for patchset `{patchset['patchset_id']}`.",
            )
        )

    if str(change.get("status") or "") == "landed":
        steps.append(
            _workflow_land_step(
                "land",
                "Land",
                "done",
                f"Change `{resolved_change_id}` already landed on `{target_line}`.",
            )
        )
    elif patchset is None:
        steps.append(_workflow_land_step("land", "Land", "waiting", "Landing starts after a patchset exists and clears review/policy."))
    elif review_blocking > 0 or review_approvals <= 0 or policy_decision != "pass":
        steps.append(
            _workflow_land_step(
                "land",
                "Land",
                "waiting",
                "Landing waits for review and policy to clear first.",
                land_command,
            )
        )
    else:
        steps.append(
            _workflow_land_step(
                "land",
                "Land",
                "ready",
                f"Change `{resolved_change_id}` is ready to land onto `{target_line}`.",
                land_command,
            )
        )

    next_action = _workflow_land_next_action(
        change=change,
        task=task,
        workspace=workspace,
        patchset=patchset,
        base_is_fresh=base_is_fresh,
        workspace_matches_patchset=workspace_matches_patchset,
        patchset_is_authoritative=patchset_is_authoritative,
        attestation=attestation,
        tests_state=tests_state,
        requires_code_review_summary=requires_code_review_summary,
        code_review_summary_count=code_review_summary_count,
        review_blocking=review_blocking,
        review_approvals=review_approvals,
        policy_decision=policy_decision,
        target_line=target_line,
        ignore_workspace_authoring=ignore_workspace_authoring,
        commands=command_hints,
    )

    return {
        "change": change,
        "task": task,
        "patchset": patchset,
        "patchset_source": patchset_source if patchset is not None else None,
        "workspace": {
            "clean": bool(workspace.get("clean")),
            "changed_count": int(workspace.get("changed_count") or 0),
            "current_line": current_line_name,
            "head_snapshot_id": revision_snapshot_id,
            "workspace_status": "clean" if workspace.get("clean") else "dirty",
            "workspace_matches_patchset": workspace_matches_patchset,
        },
        "base_line": {
            "line_name": base_line_name,
            "head_snapshot_id": remote_base_snapshot_id,
        },
        "review": review_summary,
        "attestation": attestation,
        "policy": policy,
        "freshness": {
            "base_is_fresh": base_is_fresh,
            "preflight_state": "fresh" if base_is_fresh else "stale",
            "recovery_required": bool(patchset is not None and not base_is_fresh),
            "worktree_needs_retarget": bool(
                isinstance(worktree_retarget, dict) and worktree_retarget.get("needs_retarget")
            ),
            "rebase_state": (
                str(worktree_retarget.get("rebase_state") or "idle")
                if isinstance(worktree_retarget, dict)
                else "idle"
            ),
            "remote_base_snapshot_id": remote_base_snapshot_id,
            "patchset_base_snapshot_id": patchset_base_snapshot_id,
            "patchset_revision_snapshot_id": patchset_revision_snapshot_id,
        },
        "steps": steps,
        "next_action": next_action,
        "suggested_commands": _workflow_land_suggested_commands(
            next_action=next_action,
            change=change,
            patchset=patchset,
            base_is_fresh=base_is_fresh,
            workspace_matches_patchset=workspace_matches_patchset,
            requires_code_review_summary=requires_code_review_summary,
            commands=command_hints,
        ),
    }
