from __future__ import annotations

from typing import Any, Optional

from rich import print as rprint
from rich.table import Table


def _queue_actionable_local_tasks(local_tasks: list[dict]) -> list[dict]:
    return [
        row
        for row in local_tasks
        if row.get("publication_state") != "published" and row.get("status") == "active"
    ]


def _queue_actionable_local_changes(local_changes: list[dict]) -> list[dict]:
    return [
        row
        for row in local_changes
        if row.get("publication_state") != "published" and row.get("status") not in {"archived", "landed"}
    ]


def _queue_local_summary(local_tasks: list[dict], local_changes: list[dict]) -> dict:
    unpublished_tasks = [row for row in local_tasks if row.get("publication_state") != "published"]
    published_tasks = [row for row in local_tasks if row.get("publication_state") == "published"]
    unpublished_changes = [row for row in local_changes if row.get("publication_state") != "published"]
    published_changes = [row for row in local_changes if row.get("publication_state") == "published"]
    actionable_draft_tasks = _queue_actionable_local_tasks(local_tasks)
    actionable_draft_changes = _queue_actionable_local_changes(local_changes)
    return {
        "task_record_count": len(local_tasks),
        "change_record_count": len(local_changes),
        "draft_task_count": len(actionable_draft_tasks),
        "published_task_count": len(published_tasks),
        "draft_change_count": len(actionable_draft_changes),
        "published_change_count": len(published_changes),
        "unpublished_task_record_count": len(unpublished_tasks),
        "unpublished_change_record_count": len(unpublished_changes),
        "active_draft_task_count": len(actionable_draft_tasks),
        "open_draft_change_count": len(actionable_draft_changes),
    }


def _queue_focus_change_reasons(task_items: list[dict]) -> dict[str, str]:
    reasons: dict[str, str] = {}
    for item in task_items:
        if not isinstance(item, dict):
            continue
        focus_change = item.get("focus_change") if isinstance(item.get("focus_change"), dict) else {}
        next_action = item.get("next_action") if isinstance(item.get("next_action"), dict) else {}
        change_id = str(focus_change.get("change_id") or next_action.get("change_id") or "").strip()
        if not change_id:
            continue
        reason = str(focus_change.get("reason") or next_action.get("detail") or "").strip()
        if reason:
            reasons[change_id] = reason
    return reasons


def _queue_change_reason(change: dict, reviewer_item: Optional[dict], focus_reason: Optional[str]) -> str:
    if focus_reason:
        return focus_reason
    if int(change.get("current_patchset_number") or 0) <= 0:
        return "No published patchset exists yet."
    if not isinstance(reviewer_item, dict):
        return ""
    review_state = reviewer_item.get("review_state") if isinstance(reviewer_item.get("review_state"), dict) else {}
    freshness = reviewer_item.get("freshness") if isinstance(reviewer_item.get("freshness"), dict) else {}
    attestation = reviewer_item.get("attestation") if isinstance(reviewer_item.get("attestation"), dict) else {}
    policy_state = reviewer_item.get("policy_state") if isinstance(reviewer_item.get("policy_state"), dict) else {}
    missing_requirements = {
        str(item).strip() for item in policy_state.get("missing_requirements", []) if str(item).strip()
    }
    if int(review_state.get("blocking") or 0) > 0:
        return "Blocking review feedback is recorded on this change."
    if freshness.get("base_is_fresh") is False:
        return "The base line moved after this patchset was published."
    attestation_state = str(attestation.get("completeness") or attestation.get("source") or "").strip()
    if attestation_state == "missing":
        return "Attestation is missing for the current patchset."
    if "tests" in missing_requirements or attestation.get("tests") == "pending":
        return "Tests are still pending for the current patchset."
    if "required_human_review" in missing_requirements:
        return "The change still needs a human approval."
    if policy_state.get("decision") == "pass":
        return "Ready to land."
    if policy_state.get("decision") == "pending":
        return "Policy evaluation is still pending."
    return ""


def _queue_change_ready_to_land(change: dict, reviewer_item: Optional[dict]) -> bool:
    if int(change.get("current_patchset_number") or 0) <= 0:
        return False
    if not isinstance(reviewer_item, dict):
        return False
    review_state = reviewer_item.get("review_state") if isinstance(reviewer_item.get("review_state"), dict) else {}
    freshness = reviewer_item.get("freshness") if isinstance(reviewer_item.get("freshness"), dict) else {}
    policy_state = reviewer_item.get("policy_state") if isinstance(reviewer_item.get("policy_state"), dict) else {}
    return (
        policy_state.get("decision") == "pass"
        and freshness.get("base_is_fresh") is not False
        and int(review_state.get("blocking") or 0) == 0
    )


def _queue_change_inventory(change_rows: list[dict], task_items: list[dict], review_items: list[dict]) -> list[dict]:
    focus_reasons = _queue_focus_change_reasons(task_items)
    reviewer_items_by_change = {
        str(item.get("change_id") or ""): item
        for item in review_items
        if isinstance(item, dict) and str(item.get("change_id") or "").strip()
    }
    inventory: list[dict] = []
    for row in change_rows:
        if not isinstance(row, dict):
            continue
        status = str(row.get("status") or "").strip()
        if status in {"landed", "archived"}:
            continue
        change_id = str(row.get("change_id") or "").strip()
        reviewer_item = reviewer_items_by_change.get(change_id)
        enriched = dict(row)
        enriched["ready_to_land"] = _queue_change_ready_to_land(enriched, reviewer_item)
        enriched["reason"] = _queue_change_reason(enriched, reviewer_item, focus_reasons.get(change_id))
        inventory.append(enriched)
    return inventory


def _render_queue_summary(data: dict[str, Any]) -> None:
    query = data.get("query") if isinstance(data.get("query"), dict) else {}
    remote = data.get("remote") if isinstance(data.get("remote"), dict) else {}
    local = data.get("local") if isinstance(data.get("local"), dict) else {}
    local_summary = local.get("summary") if isinstance(local.get("summary"), dict) else {}
    workspace = (data.get("workspace") or {}).get("status") if isinstance(data.get("workspace"), dict) else {}
    worktrees = (data.get("workspace") or {}).get("worktrees") if isinstance(data.get("workspace"), dict) else {}
    summary = data.get("summary") if isinstance(data.get("summary"), dict) else {}

    table = Table(title="ait queue summary")
    table.add_column("field")
    table.add_column("value")
    table.add_row("repo", str(data.get("repo_name") or ""))
    if remote.get("configured"):
        table.add_row("remote", f"{remote.get('remote_name') or 'default'} ({remote.get('repo_name') or ''})")
    elif remote.get("available_remotes"):
        table.add_row("remote", "configured without default")
    else:
        table.add_row("remote", "none")
    table.add_row("shared tasks", str(summary.get("shared_task_count", 0)))
    table.add_row("attention required", str(summary.get("attention_required_count", 0)))
    table.add_row("ready to land", str(summary.get("ready_to_land_count", 0)))
    table.add_row("ready to complete", str(summary.get("ready_to_complete_count", 0)))
    if query.get("all_changes"):
        table.add_row("open shared changes", str(summary.get("open_shared_change_count", 0)))
    table.add_row("review inbox", str(summary.get("reviewer_inbox_count", 0)))
    table.add_row("local draft tasks", str(summary.get("local_draft_task_count", 0)))
    table.add_row("local draft changes", str(summary.get("local_draft_change_count", 0)))
    table.add_row(
        "workspace",
        f"{'dirty' if summary.get('workspace_dirty') else 'clean'} ({summary.get('workspace_changed_count', 0)} changed)",
    )
    table.add_row("dirty worktrees", str(summary.get("dirty_worktree_count", 0)))
    table.add_row("stale worktrees", str(summary.get("stale_worktree_count", 0)))
    rprint(table)

    if remote.get("error"):
        rprint(f"[yellow]Remote summary unavailable:[/yellow] {remote['error']}")

    task_queue = remote.get("task_queue") if isinstance(remote.get("task_queue"), dict) else {}
    task_items = task_queue.get("items") if isinstance(task_queue.get("items"), list) else []
    if task_items:
        queue_table = Table(title="shared queue")
        queue_table.add_column("task_id")
        queue_table.add_column("title")
        queue_table.add_column("state")
        queue_table.add_column("next_action")
        for item in task_items[:10]:
            task = item.get("task") if isinstance(item.get("task"), dict) else {}
            workflow = item.get("workflow") if isinstance(item.get("workflow"), dict) else {}
            next_action = item.get("next_action") if isinstance(item.get("next_action"), dict) else {}
            queue_table.add_row(
                str(task.get("task_id") or ""),
                str(task.get("title") or ""),
                str(workflow.get("state") or ""),
                str(next_action.get("code") or ""),
            )
        rprint(queue_table)

    reviewer_inbox = remote.get("reviewer_inbox") if isinstance(remote.get("reviewer_inbox"), dict) else {}
    review_items = reviewer_inbox.get("items") if isinstance(reviewer_inbox.get("items"), list) else []
    if review_items:
        review_table = Table(title="review inbox")
        review_table.add_column("change_id")
        review_table.add_column("title")
        review_table.add_column("policy")
        review_table.add_column("tests")
        for item in review_items[:10]:
            policy_state = item.get("policy_state") if isinstance(item.get("policy_state"), dict) else {}
            attestation = item.get("attestation") if isinstance(item.get("attestation"), dict) else {}
            review_table.add_row(
                str(item.get("change_id") or ""),
                str(item.get("title") or ""),
                str(policy_state.get("decision") or ""),
                str(attestation.get("tests") or ""),
            )
        rprint(review_table)

    remote_changes = remote.get("changes") if isinstance(remote.get("changes"), list) else []
    if query.get("all_changes") and remote_changes:
        change_table = Table(title="shared changes")
        change_table.add_column("change_id")
        change_table.add_column("title")
        change_table.add_column("status")
        change_table.add_column("base_line")
        change_table.add_column("patchsets")
        change_table.add_column("next_gate")
        for row in remote_changes:
            change_table.add_row(
                str(row.get("change_id") or ""),
                str(row.get("title") or ""),
                str(row.get("status") or ""),
                str(row.get("base_line") or ""),
                str(row.get("current_patchset_number") or 0),
                str(row.get("reason") or ("ready to land" if row.get("ready_to_land") else "")),
            )
        rprint(change_table)
    elif query.get("all_changes") and remote.get("configured") and not remote.get("error"):
        rprint("[green]No open shared changes detected on the remote.[/green]")

    local_tasks = [row for row in local.get("tasks", []) if isinstance(row, dict) and row.get("publication_state") != "published"]
    if local_tasks:
        local_task_table = Table(title="local draft tasks")
        local_task_table.add_column("task_id")
        local_task_table.add_column("title")
        local_task_table.add_column("status")
        for row in local_tasks[:10]:
            local_task_table.add_row(str(row.get("task_id") or ""), str(row.get("title") or ""), str(row.get("status") or ""))
        rprint(local_task_table)

    local_changes = [row for row in local.get("changes", []) if isinstance(row, dict) and row.get("publication_state") != "published"]
    if local_changes:
        local_change_table = Table(title="local draft changes")
        local_change_table.add_column("change_id")
        local_change_table.add_column("title")
        local_change_table.add_column("status")
        local_change_table.add_column("base_line")
        for row in local_changes[:10]:
            local_change_table.add_row(
                str(row.get("change_id") or ""),
                str(row.get("title") or ""),
                str(row.get("status") or ""),
                str(row.get("base_line") or ""),
            )
        rprint(local_change_table)

    if isinstance(workspace, dict) and workspace.get("clean") is False:
        sample_paths: list[str] = []
        for key in ("modified_paths", "missing_paths", "untracked_paths"):
            sample_paths.extend(str(path) for path in workspace.get(key, [])[:3])
        sample_text = ", ".join(sample_paths[:6])
        if sample_text:
            rprint(f"[yellow]Workspace dirty:[/yellow] {sample_text}")
        else:
            rprint("[yellow]Workspace dirty.[/yellow]")
    elif not local_summary.get("draft_task_count") and not local_summary.get("draft_change_count") and not task_items:
        rprint("[green]No active shared tasks, local drafts, or workspace changes detected.[/green]")

    if isinstance(worktrees, dict) and (worktrees.get("dirty_count") or worktrees.get("stale_count")):
        rprint(
            f"[yellow]Worktree attention:[/yellow] dirty={int(worktrees.get('dirty_count') or 0)}, "
            f"stale={int(worktrees.get('stale_count') or 0)}"
        )
