from __future__ import annotations

import re
from datetime import datetime, timezone
from typing import Any

from .store import RepoContext
from .store_local_changes import list_local_changes
from .store_local_releases import list_local_releases
from .store_local_tasks import list_local_tasks
from .store_repo_reads import collect_snapshot_chain

_RELEASE_NOTES_START = "<!-- ait-release-notes:start -->"
_RELEASE_NOTES_END = "<!-- ait-release-notes:end -->"
_RELEASE_NOTES_BLOCK_RE = re.compile(
    rf"\n?{re.escape(_RELEASE_NOTES_START)}.*?{re.escape(_RELEASE_NOTES_END)}\n?",
    re.DOTALL,
)
_MAX_RELEASE_NOTES_TASKS = 10


def parse_iso_datetime(value: str | None) -> datetime | None:
    text = str(value or "").strip()
    if not text:
        return None
    normalized = text[:-1] + "+00:00" if text.endswith("Z") else text
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError:
        return None
    return parsed if parsed.tzinfo else parsed.replace(tzinfo=timezone.utc)


def datetime_sort_key(value: str | None) -> tuple[int, str]:
    parsed = parse_iso_datetime(value)
    if parsed is None:
        return (0, "")
    return (1, parsed.isoformat())


def previous_published_release(ctx: RepoContext, record: dict[str, Any]) -> dict[str, Any] | None:
    current_release_id = str(record.get("release_id") or "").strip()
    current_profile = str(record.get("profile") or "").strip()
    current_created_at = parse_iso_datetime(record.get("created_at"))
    candidates: list[tuple[tuple[int, str], dict[str, Any]]] = []
    for release in list_local_releases(ctx):
        if str(release.get("release_id") or "").strip() == current_release_id:
            continue
        if str(release.get("profile") or "").strip() != current_profile:
            continue
        if str(release.get("status") or "").strip() != "published":
            continue
        created_at = parse_iso_datetime(release.get("created_at"))
        if current_created_at is not None and created_at is not None and created_at > current_created_at:
            continue
        candidates.append((datetime_sort_key(release.get("created_at")), release))
    if not candidates:
        return None
    return max(candidates, key=lambda item: item[0])[1]


def collect_release_note_tasks(
    ctx: RepoContext,
    record: dict[str, Any],
) -> dict[str, Any]:
    previous = previous_published_release(ctx, record)
    summary: dict[str, Any] = {
        "version": str(record.get("version") or "").strip(),
        "profile": str(record.get("profile") or "").strip(),
        "mode": "delta",
        "previous_release_id": None,
        "previous_version": None,
        "task_count": 0,
        "shown_task_count": 0,
        "omitted_task_count": 0,
        "tasks": [],
    }
    if previous is None:
        summary["mode"] = "initial_release"
        return summary

    previous_snapshot_id = str(previous.get("snapshot_id") or "").strip()
    previous_version = str(previous.get("version") or "").strip()
    summary["previous_release_id"] = str(previous.get("release_id") or "").strip() or None
    summary["previous_version"] = previous_version or None

    if not previous_snapshot_id:
        summary["mode"] = "baseline_unavailable"
        return summary

    current_snapshot_id = str(record.get("snapshot_id") or "").strip()
    ancestry = collect_snapshot_chain(ctx, current_snapshot_id)
    if previous_snapshot_id not in ancestry:
        summary["mode"] = "baseline_not_ancestor"
        return summary

    cutoff = ancestry.index(previous_snapshot_id)
    snapshot_window = set(ancestry[cutoff + 1 :])
    if not snapshot_window:
        return summary

    task_titles = {
        str(task.get("task_id") or "").strip(): str(task.get("title") or "").strip()
        for task in list_local_tasks(ctx)
        if str(task.get("task_id") or "").strip()
    }
    latest_task_rows: dict[str, dict[str, Any]] = {}
    for change in sorted(
        list_local_changes(ctx),
        key=lambda row: (datetime_sort_key(row.get("landed_at")), str(row.get("change_id") or "")),
    ):
        if str(change.get("status") or "").strip() != "landed":
            continue
        if str(change.get("target_line") or "").strip() != str(record.get("line") or "").strip():
            continue
        landed_snapshot_id = str(change.get("landed_snapshot_id") or "").strip()
        if not landed_snapshot_id or landed_snapshot_id not in snapshot_window:
            continue
        task_id = str(change.get("task_id") or "").strip()
        if not task_id:
            continue
        latest_task_rows[task_id] = {
            "task_id": task_id,
            "title": task_titles.get(task_id) or str(change.get("title") or "").strip() or task_id,
            "landed_at": str(change.get("landed_at") or "").strip() or None,
            "landed_snapshot_id": landed_snapshot_id,
            "change_id": str(change.get("change_id") or "").strip() or None,
        }

    ordered_tasks = sorted(
        latest_task_rows.values(),
        key=lambda row: (datetime_sort_key(row.get("landed_at")), str(row.get("task_id") or "")),
    )
    shown_tasks = ordered_tasks[-_MAX_RELEASE_NOTES_TASKS:]
    summary["task_count"] = len(ordered_tasks)
    summary["shown_task_count"] = len(shown_tasks)
    summary["omitted_task_count"] = max(0, len(ordered_tasks) - len(shown_tasks))
    summary["tasks"] = shown_tasks
    return summary


def render_release_notes(record: dict[str, Any], notes: dict[str, Any]) -> str:
    version = str(record.get("version") or "").strip() or "unknown"
    mode = str(notes.get("mode") or "delta").strip()
    previous_version = str(notes.get("previous_version") or "").strip()
    task_count = int(notes.get("task_count") or 0)
    shown_count = int(notes.get("shown_task_count") or 0)
    omitted_count = int(notes.get("omitted_task_count") or 0)
    tasks = notes.get("tasks") if isinstance(notes.get("tasks"), list) else []
    lines = [
        _RELEASE_NOTES_START,
        "## Release Notes",
        "",
        f"### v{version}",
        "",
    ]
    if mode == "initial_release":
        lines.append("Initial published release for this profile. Task-based delta notes start after the first published baseline.")
    elif mode in {"baseline_unavailable", "baseline_not_ancestor"}:
        if previous_version:
            lines.append(
                f"Previous published release `v{previous_version}` is not an ancestor baseline for this snapshot, so task-based delta notes are unavailable for this build."
            )
        else:
            lines.append("A previous published baseline is unavailable for this profile, so task-based delta notes are unavailable for this build.")
    elif task_count == 0:
        if previous_version:
            lines.append(f"No landed tasks were recorded between this release and the previous published release `v{previous_version}`.")
        else:
            lines.append("No landed tasks were recorded for this release window.")
    else:
        if previous_version:
            summary_line = f"Tasks landed since `v{previous_version}`"
        else:
            summary_line = "Tasks landed for this release window"
        if omitted_count > 0:
            summary_line += f" (showing latest {shown_count} of {task_count})"
        elif task_count == 1:
            summary_line += " (1 task)"
        else:
            summary_line += f" ({task_count} tasks)"
        lines.append(summary_line + ":")
        lines.append("")
        for task in tasks:
            lines.append(f"- `{task['task_id']}` {task['title']}")
    lines.extend(["", _RELEASE_NOTES_END])
    return "\n".join(lines)


def apply_release_notes_to_readme(
    readme_text: str,
    *,
    record: dict[str, Any],
    notes: dict[str, Any],
) -> str:
    base_text = _RELEASE_NOTES_BLOCK_RE.sub("\n", readme_text).rstrip()
    return base_text + "\n\n" + render_release_notes(record, notes) + "\n"
