from __future__ import annotations

from typing import Any

from .runtime_defaults import _normalize_text_value


def _workflow_land_batch_item_status(state: dict[str, Any]) -> str:
    change = state.get("change") if isinstance(state.get("change"), dict) else {}
    task = state.get("task") if isinstance(state.get("task"), dict) else {}
    if str(change.get("status") or "") == "landed" and str(task.get("status") or "") == "completed":
        return "completed"
    next_action = state.get("next_action") if isinstance(state.get("next_action"), dict) else {}
    if str(next_action.get("code") or "") == "done":
        return "completed"
    return "blocked"


def _workflow_land_completed_local_route_metadata(
    *,
    local_task: dict[str, Any],
    local_change: dict[str, Any],
    remote_name: str,
    target_line: str,
    remote_task_id: str | None = None,
    remote_change_id: str | None = None,
) -> dict[str, Any]:
    return {
        "kind": "completed_local",
        "local_task_id": str(local_task.get("task_id") or "").strip(),
        "local_change_id": str(local_change.get("change_id") or "").strip(),
        "published_task_id": _normalize_text_value(local_task.get("published_task_id")),
        "published_change_id": _normalize_text_value(local_change.get("published_change_id")),
        "remote_task_id": _normalize_text_value(remote_task_id),
        "remote_change_id": _normalize_text_value(remote_change_id),
        "remote_name": remote_name,
        "target_line": target_line,
    }


def _workflow_land_preview_item_status(state: dict[str, Any]) -> str:
    return "completed" if _workflow_land_batch_item_status(state) == "completed" else "ready"


def _workflow_land_applied_action_summary(action: dict[str, Any]) -> str:
    code = str(action.get("code") or "").strip() or "action"
    result = action.get("result") if isinstance(action.get("result"), dict) else {}
    if code == "snapshot_create":
        return f"created snapshot `{result.get('snapshot_id') or 'unknown'}`"
    if code in {"publish_patchset", "refresh_patchset"}:
        auto_rebase = result.get("auto_rebase") if isinstance(result.get("auto_rebase"), dict) else {}
        rebase = auto_rebase.get("rebase") if isinstance(auto_rebase.get("rebase"), dict) else {}
        if rebase:
            return (
                f"published patchset `{result.get('patchset_id') or 'unknown'}` "
                f"after auto-rebase `{rebase.get('status') or 'applied'}`"
            )
        return f"published patchset `{result.get('patchset_id') or 'unknown'}`"
    if code == "run_patchset_ci":
        if result.get("queued") is True:
            return f"queued patchset CI for `{result.get('patchset_id') or 'unknown'}`"
        return f"updated patchset CI evidence for `{result.get('patchset_id') or 'unknown'}`"
    if code == "record_attestation":
        return f"recorded attestation `{result.get('attestation_id') or 'unknown'}`"
    if code == "record_review":
        return f"recorded review `{result.get('review_id') or 'unknown'}`"
    if code == "evaluate_policy":
        return f"policy is now `{result.get('decision') or 'unknown'}`"
    if code == "submit_land":
        cleanup = result.get("bound_worktree_cleanup") if isinstance(result.get("bound_worktree_cleanup"), dict) else {}
        cleanup_status = str(cleanup.get("status") or "").strip()
        cleanup_worktree = cleanup.get("worktree") if isinstance(cleanup.get("worktree"), dict) else {}
        if cleanup_status == "removed":
            return (
                f"land request `{result.get('submission_id') or 'unknown'}` is `{result.get('status') or 'unknown'}`"
                f" and removed bound worktree `{cleanup_worktree.get('name') or cleanup.get('worktree_name') or 'unknown'}`"
            )
        return f"land request `{result.get('submission_id') or 'unknown'}` is `{result.get('status') or 'unknown'}`"
    if code == "complete_task":
        return f"task `{result.get('task_id') or 'unknown'}` is `{result.get('status') or 'unknown'}`"
    return code
