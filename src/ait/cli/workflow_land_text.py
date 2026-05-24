from __future__ import annotations

from typing import Any

from .workflow_land_views import _workflow_land_applied_action_summary


def _render_workflow_land_text(data: dict[str, Any]) -> str:
    change = data.get("change") if isinstance(data.get("change"), dict) else {}
    task = data.get("task") if isinstance(data.get("task"), dict) else {}
    patchset = data.get("patchset") if isinstance(data.get("patchset"), dict) else {}
    workspace = data.get("workspace") if isinstance(data.get("workspace"), dict) else {}
    next_action = data.get("next_action") if isinstance(data.get("next_action"), dict) else {}

    lines = [f"ait workflow land · {change.get('change_id') or '(unknown change)'}", ""]
    lines.append(f"- status: {change.get('status') or 'unknown'}")
    lines.append(f"- task: {task.get('task_id') or 'unknown'}")
    lines.append(f"- base line: {change.get('base_line') or 'unknown'}")
    lines.append(f"- current line: {workspace.get('current_line') or 'unknown'}")
    lines.append(
        f"- workspace: {workspace.get('workspace_status') or 'unknown'} ({workspace.get('changed_count') or 0} changed)"
    )
    lines.append(f"- patchset: {patchset.get('patchset_id') or 'none'}")

    steps = [row for row in data.get("steps") or [] if isinstance(row, dict)]
    if steps:
        lines.extend(["", "Workflow steps"])
        for row in steps:
            status = str(row.get("status") or "").strip() or "unknown"
            label = str(row.get("label") or row.get("code") or "").strip() or "step"
            detail = str(row.get("detail") or "").strip()
            command = str(row.get("command") or "").strip()
            lines.append(f"- [{status}] {label}: {detail}")
            if command:
                lines.append(f"  {command}")

    applied_actions = [row for row in data.get("applied_actions") or [] if isinstance(row, dict)]
    if applied_actions:
        lines.extend(["", "Applied actions"])
        for row in applied_actions:
            lines.append(f"- {row.get('code')}: {_workflow_land_applied_action_summary(row)}")

    summary = str(next_action.get("summary") or "").strip()
    detail = str(next_action.get("detail") or "").strip()
    command = str(next_action.get("command") or "").strip()
    if summary or detail or command:
        lines.extend(["", "Next action"])
        if summary:
            lines.append(f"- {summary}")
        if detail:
            lines.append(f"  {detail}")
        if command:
            lines.append(f"  {command}")

    stopped_reason = str(data.get("apply_stopped_reason") or "").strip()
    if stopped_reason:
        lines.extend(["", "Apply status", f"- {stopped_reason}"])
    return "\n".join(lines)
