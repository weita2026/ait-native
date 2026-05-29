from __future__ import annotations

import json
from typing import Any

from rich import print as rprint
from rich.table import Table


def _render_plan_summary(rows: list[dict]) -> None:
    table = Table(title="ait plans")
    table.add_column("plan_id")
    table.add_column("title")
    table.add_column("status")
    table.add_column("head")
    table.add_column("updated")
    show_publication = any("publication_state" in row for row in rows)
    if show_publication:
        table.add_column("publication")
    for row in rows:
        head = row.get("head_revision_number")
        cells = [
            str(row.get("plan_id") or ""),
            str(row.get("title") or ""),
            str(row.get("status") or ""),
            f"r{head}" if head is not None else "",
            str(row.get("updated_at") or ""),
        ]
        if show_publication:
            cells.append(str(row.get("publication_state") or ""))
        table.add_row(*cells)
    rprint(table)


def _render_plan_detail(plan: dict[str, Any], *, revision: dict[str, Any] | None = None) -> None:
    summary = Table(title=f"ait plan {plan.get('plan_id')}")
    summary.add_column("field")
    summary.add_column("value")
    summary.add_row("title", str(plan.get("title") or ""))
    summary.add_row("status", str(plan.get("status") or ""))
    summary.add_row("repo", str(plan.get("repo_name") or ""))
    summary.add_row("head revision", str(plan.get("head_revision_id") or ""))
    summary.add_row("updated", str(plan.get("updated_at") or ""))
    if "publication_state" in plan:
        summary.add_row("publication", str(plan.get("publication_state") or ""))
        summary.add_row("published at", str(plan.get("published_at") or ""))
    rprint(summary)

    current_revision = revision or plan.get("head_revision")
    if not current_revision:
        return

    detail = Table(title="plan revision")
    detail.add_column("field")
    detail.add_column("value")
    detail.add_row("plan_revision_id", str(current_revision.get("plan_revision_id") or ""))
    detail.add_row("revision_number", str(current_revision.get("revision_number") or ""))
    detail.add_row("title_snapshot", str(current_revision.get("title_snapshot") or ""))
    detail.add_row("summary", str(current_revision.get("summary") or ""))
    detail.add_row("artifact_path", str(current_revision.get("artifact_path") or ""))
    detail.add_row("artifact_selector", str(current_revision.get("artifact_selector") or ""))
    detail.add_row("artifact_heading", str(current_revision.get("artifact_heading") or ""))
    detail.add_row("source_kind", str(current_revision.get("source_kind") or ""))
    detail.add_row("created_at", str(current_revision.get("created_at") or ""))
    rprint(detail)
    items = list(current_revision.get("items") or [])
    if items:
        _render_plan_items(
            {
                "plan_id": plan.get("plan_id"),
                "plan_revision_id": current_revision.get("plan_revision_id"),
                "items": items,
            }
        )


def _render_plan_revisions(plan_id: str, rows: list[dict]) -> None:
    table = Table(title=f"plan revisions for {plan_id}")
    table.add_column("revision")
    table.add_column("plan_revision_id")
    table.add_column("title")
    table.add_column("summary")
    table.add_column("created")
    show_publication = any("publication_state" in row for row in rows)
    if show_publication:
        table.add_column("publication")
    for row in rows:
        cells = [
            f"r{row.get('revision_number')}",
            str(row.get("plan_revision_id") or ""),
            str(row.get("title_snapshot") or ""),
            str(row.get("summary") or ""),
            str(row.get("created_at") or ""),
        ]
        if show_publication:
            cells.append(str(row.get("publication_state") or ""))
        table.add_row(*cells)
    rprint(table)


def _render_plan_items(payload: dict[str, Any]) -> None:
    plan_id = str(payload.get("plan_id") or "")
    plan_revision_id = str(payload.get("plan_revision_id") or "")
    items = list(payload.get("items") or [])
    if not items:
        rprint(
            f"[yellow]No explicit `[ref: ...]` plan items found for {plan_id} ({plan_revision_id or 'no revision'}).[/yellow]"
        )
        return
    table = Table(title=f"plan items for {plan_id}")
    table.add_column("ref")
    table.add_column("state")
    table.add_column("title")
    table.add_column("heading")
    table.add_column("line")
    for item in items:
        table.add_row(
            str(item.get("plan_item_ref") or ""),
            str(item.get("checkbox_state") or ""),
            str(item.get("text") or ""),
            " / ".join(str(part) for part in item.get("heading_path") or []),
            str(item.get("line_number") or ""),
        )
    rprint(table)
    if payload.get("dispatch_validation_required"):
        rprint(
            "[yellow]Identity-only view. Use `ait plan inspect <plan-id>` or "
            "`ait plan candidates` before `ait task start` to confirm the ref is still taskable.[/yellow]"
        )


def _render_plan_candidates(payload: dict[str, Any]) -> None:
    summary = payload.get("summary") if isinstance(payload.get("summary"), dict) else {}
    candidates = payload.get("candidates") if isinstance(payload.get("candidates"), list) else []
    header = Table(title="ait plan candidates")
    header.add_column("field")
    header.add_column("value")
    header.add_row("scope", str(payload.get("scope") or ""))
    if payload.get("remote"):
        header.add_row("remote", str(payload.get("remote") or ""))
    header.add_row("repo", str(payload.get("repo_name") or ""))
    header.add_row("scanned plans", str(summary.get("scanned_plan_count", 0)))
    header.add_row("candidate plans", str(summary.get("candidate_plan_count", 0)))
    header.add_row("taskable items", str(summary.get("taskable_item_count", 0)))
    header.add_row("linked tasks", str(summary.get("linked_task_count", 0)))
    header.add_row("local unpublished heads", str(summary.get("local_unpublished_head_count", 0)))
    rprint(header)

    if not candidates:
        rprint("[green]No taskable plan items found.[/green]")
        return

    table = Table(title="taskable plan roots")
    table.add_column("plan_id")
    table.add_column("title")
    table.add_column("path")
    table.add_column("selector")
    table.add_column("open")
    table.add_column("taskable")
    table.add_column("linked")
    table.add_column("local head")
    table.add_column("sample refs")
    for row in candidates:
        refs = [str(item.get("plan_item_ref") or "") for item in row.get("taskable_items") or []]
        local_head = "unpublished" if row.get("local_unpublished_head") else ""
        table.add_row(
            str(row.get("plan_id") or ""),
            str(row.get("title") or ""),
            str(row.get("artifact_path") or ""),
            str(row.get("artifact_selector") or ""),
            str(row.get("open_item_count") or 0),
            str(row.get("taskable_item_count") or 0),
            str(row.get("linked_task_count") or 0),
            local_head,
            ", ".join(refs[:3]),
        )
    rprint(table)


def _render_plan_inspect(payload: dict[str, Any]) -> None:
    plan = payload.get("plan") if isinstance(payload.get("plan"), dict) else {}
    table = Table(title=f"ait plan inspect {plan.get('plan_id') or ''}")
    table.add_column("field")
    table.add_column("value")
    table.add_row("title", str(plan.get("title") or ""))
    table.add_row("status", str(plan.get("status") or ""))
    table.add_row("repo", str(plan.get("repo_name") or payload.get("repo_name") or ""))
    table.add_row("scope", str(payload.get("scope") or ""))
    if payload.get("remote"):
        table.add_row("remote", str(payload.get("remote") or ""))
    table.add_row("artifact_path", str(plan.get("artifact_path") or ""))
    table.add_row("artifact_selector", str(plan.get("artifact_selector") or ""))
    table.add_row("plan_revision_id", str(plan.get("plan_revision_id") or ""))
    table.add_row("revision", str(plan.get("revision_number") or ""))
    table.add_row("publication", str(plan.get("publication_state") or ""))
    table.add_row("head publication", str(plan.get("head_publication_state") or ""))
    table.add_row("local unpublished head", "yes" if plan.get("local_unpublished_head") else "no")
    table.add_row("items", str(plan.get("item_count") or 0))
    table.add_row("open items", str(plan.get("open_item_count") or 0))
    table.add_row("taskable items", str(plan.get("taskable_item_count") or 0))
    table.add_row("linked tasks", str(plan.get("linked_task_count") or 0))
    rprint(table)

    items = plan.get("items") if isinstance(plan.get("items"), list) else []
    if not items:
        rprint("[yellow]No explicit `[ref: ...]` plan items found.[/yellow]")
        return
    item_table = Table(title="plan item workflow state")
    item_table.add_column("ref")
    item_table.add_column("state")
    item_table.add_column("taskable")
    item_table.add_column("linked tasks")
    item_table.add_column("line")
    item_table.add_column("title")
    for item in items:
        linked_tasks = item.get("linked_tasks") if isinstance(item.get("linked_tasks"), list) else []
        linked_task_ids = ", ".join(str(task.get("task_id") or "") for task in linked_tasks[:3])
        item_table.add_row(
            str(item.get("plan_item_ref") or ""),
            str(item.get("checkbox_state") or ""),
            "yes" if item.get("taskable") else str(item.get("taskable_blocker") or "no"),
            linked_task_ids,
            str(item.get("line_number") or ""),
            str(item.get("text") or ""),
        )
    rprint(item_table)


def _render_plan_sync_summary(payload: dict[str, Any]) -> None:
    summary = payload.get("summary") or {}
    table = Table(title=f"ait plan sync {payload.get('target')}")
    table.add_column("field")
    table.add_column("value")
    table.add_row("scope", str(payload.get("scope") or ""))
    table.add_row("created", str(summary.get("created_count") or 0))
    table.add_row("updated", str(summary.get("updated_count") or 0))
    table.add_row("unchanged", str(summary.get("unchanged_count") or 0))
    table.add_row("pruned", str(summary.get("pruned_count") or 0))
    if "adopted_count" in summary:
        table.add_row("adopted", str(summary.get("adopted_count") or 0))
    table.add_row("processed", str(summary.get("processed_count") or 0))
    if "published_count" in summary:
        table.add_row("published", str(summary.get("published_count") or 0))
    if "artifact_count" in summary:
        table.add_row("paired artifacts", str(summary.get("artifact_count") or 0))
    rprint(table)

    rows = list(payload.get("results") or [])
    if not rows:
        return
    detail = Table(title="plan sync results")
    detail.add_column("action")
    detail.add_column("artifact_path")
    detail.add_column("plan_id")
    detail.add_column("plan_revision_id")
    detail.add_column("status")
    for row in rows:
        detail.add_row(
            str(row.get("action") or ""),
            str(row.get("artifact_path") or ""),
            str(row.get("plan_id") or ""),
            str(row.get("plan_revision_id") or ""),
            str(row.get("status") or ""),
        )
    rprint(detail)
    artifact_rows = list(payload.get("artifact_results") or [])
    if artifact_rows:
        artifact_detail = Table(title="paired artifact uploads")
        artifact_detail.add_column("role")
        artifact_detail.add_column("artifact_path")
        artifact_detail.add_column("plan_id")
        artifact_detail.add_column("plan_revision_id")
        artifact_detail.add_column("blob_id")
        for row in artifact_rows:
            artifact_detail.add_row(
                str(row.get("role") or ""),
                str(row.get("artifact_path") or ""),
                str(row.get("plan_id") or ""),
                str(row.get("plan_revision_id") or ""),
                str(row.get("blob_id") or ""),
            )
        rprint(artifact_detail)
    _render_plan_sync_task_start_advisory(payload.get("task_start_advisory"))


def _format_plan_sync_blocker_summary(plan: dict[str, Any]) -> str:
    blocked_open_items = list(plan.get("blocked_open_items") or [])
    if int(plan.get("item_count") or 0) <= 0:
        return "no explicit [ref] items"
    if int(plan.get("open_item_count") or 0) <= 0:
        return "no open items"
    if blocked_open_items:
        counts: dict[str, int] = {}
        for item in blocked_open_items:
            blocker = str(item.get("taskable_blocker") or "blocked")
            counts[blocker] = counts.get(blocker, 0) + 1
        return ", ".join(f"{name}:{counts[name]}" for name in sorted(counts))
    return "no taskable refs"


def _render_plan_sync_task_start_advisory(advisory: dict[str, Any] | None) -> None:
    if not isinstance(advisory, dict):
        return
    plans = list(advisory.get("plans") or [])
    if not plans:
        return

    summary = advisory.get("summary") or {}
    header = Table(title="task-start advisory")
    header.add_column("field")
    header.add_column("value")
    header.add_row("touched plans", str(summary.get("touched_plan_count") or 0))
    header.add_row("taskable plans", str(summary.get("taskable_plan_count") or 0))
    header.add_row("taskable refs", str(summary.get("taskable_item_count") or 0))
    if advisory.get("dispatch_validation_still_required"):
        header.add_row("validation", str(advisory.get("task_start_validation_hint") or ""))
    rprint(header)

    table = Table(title="next task candidates")
    table.add_column("plan_id")
    table.add_column("title")
    table.add_column("taskable refs")
    table.add_column("blocked open")
    table.add_column("next ref")
    for plan in plans:
        refs = list(plan.get("taskable_refs") or [])
        if refs:
            shown_refs = refs[:3]
            suffix = f" (+{len(refs) - len(shown_refs)} more)" if len(refs) > len(shown_refs) else ""
            refs_text = ", ".join(shown_refs) + suffix
        else:
            refs_text = "none"
        table.add_row(
            str(plan.get("plan_id") or ""),
            str(plan.get("plan_title") or ""),
            refs_text,
            _format_plan_sync_blocker_summary(plan),
            refs[0] if refs else "",
        )
    rprint(table)

    command_hints = [
        (str(plan.get("plan_id") or ""), str(plan.get("task_start_command_hint") or ""))
        for plan in plans
        if str(plan.get("task_start_command_hint") or "").strip()
    ]
    if len(command_hints) == 1:
        rprint(f"Suggested task start: {command_hints[0][1]}")
    elif command_hints:
        rprint("Suggested task starts:")
        for plan_id, command in command_hints:
            rprint(f"{plan_id}: {command}")

    if len(plans) == 1 and int(plans[0].get("taskable_item_count") or 0) > 0:
        ref_table = Table(title=f"taskable refs for {plans[0].get('plan_id') or ''}")
        ref_table.add_column("ref")
        ref_table.add_column("line")
        ref_table.add_column("title")
        for item in plans[0].get("taskable_items") or []:
            ref_table.add_row(
                str(item.get("plan_item_ref") or ""),
                str(item.get("line_number") or ""),
                str(item.get("text") or ""),
            )
        rprint(ref_table)


def _render_planning_sessions(plan_id: str, rows: list[dict[str, Any]]) -> None:
    table = Table(title=f"planning sessions for {plan_id}")
    table.add_column("planning_session_id")
    table.add_column("status")
    table.add_column("mode")
    table.add_column("artifact")
    table.add_column("title")
    table.add_column("updated")
    for row in rows:
        table.add_row(
            str(row.get("planning_session_id") or ""),
            str(row.get("status") or ""),
            str(row.get("mode") or ""),
            str(row.get("artifact_status") or ""),
            str(row.get("title") or ""),
            str(row.get("updated_at") or ""),
        )
    rprint(table)


def _render_planning_session_detail(session: dict[str, Any]) -> None:
    table = Table(title=f"planning session {session.get('planning_session_id')}")
    table.add_column("field")
    table.add_column("value")
    for field in (
        "plan_id",
        "status",
        "mode",
        "preferred_agent",
        "artifact_status",
        "derived_task_id",
        "last_promoted_plan_revision_id",
        "last_event_sequence",
        "created_by",
        "created_at",
        "updated_at",
    ):
        table.add_row(field, str(session.get(field) or ""))
    if session.get("title"):
        table.add_row("title", str(session.get("title") or ""))
    rprint(table)


def _render_planning_session_events(planning_session_id: str, rows: list[dict[str, Any]]) -> None:
    table = Table(title=f"planning session events for {planning_session_id}")
    table.add_column("seq")
    table.add_column("type")
    table.add_column("actor")
    table.add_column("created")
    table.add_column("payload")
    for row in rows:
        table.add_row(
            str(row.get("sequence") or ""),
            str(row.get("event_type") or ""),
            str(row.get("actor_identity") or ""),
            str(row.get("created_at") or ""),
            json.dumps(row.get("payload") or {}, sort_keys=True),
        )
    rprint(table)
