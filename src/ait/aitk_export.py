from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from ait_protocol.common import find_plan_item_in_items

from .aitk_layout import layout_active_columns
from .aitk_provenance import build_snapshot_provenance_overlay
from .repo_paths import RepoContext
from .snapshot_diff import DEFAULT_SNAPSHOT_DIFF_MAX_BYTES, snapshot_diff
from .store import (
    current_line,
    get_local_plan_revision,
    get_remote,
    list_lines,
    list_local_changes,
    list_local_plans,
    list_local_sessions,
    list_local_tasks,
    list_snapshots,
    load_config,
)

try:  # Optional remote workflow enrichment for GUI context.
    from .remote_client import (
        RemoteError,
        list_changes as remote_list_changes,
        list_patchsets as remote_list_patchsets,
        list_tasks as remote_list_tasks,
    )
except Exception:  # pragma: no cover - import-time fallback for minimal installs
    RemoteError = RuntimeError  # type: ignore[assignment]
    remote_list_changes = None  # type: ignore[assignment]
    remote_list_patchsets = None  # type: ignore[assignment]
    remote_list_tasks = None  # type: ignore[assignment]


SnapshotId = str

__all__ = [
    "snapshot_chain",
    "snapshot_is_ancestor",
    "snapshot_distance_descendant_to_ancestor",
    "snapshot_health_relation_to_main",
    "attach_graph_layout",
    "attach_provenance_overlays",
    "attach_plan_contexts",
    "attach_snapshot_parent_diffs",
    "build_aitk_payload_from_rows",
    "build_aitk_history_payload",
    "build_line_health_rows",
    "build_markdown_docs",
    "build_plan_links",
]


def _coerce_datetime(value: datetime | str | None) -> datetime:
    """Return a timezone-aware datetime for age calculations."""
    if value is None:
        return datetime(1970, 1, 1, tzinfo=timezone.utc)
    if isinstance(value, datetime):
        return value if value.tzinfo else value.replace(tzinfo=timezone.utc)

    text = str(value).strip()
    if not text:
        return datetime(1970, 1, 1, tzinfo=timezone.utc)
    normalized = text[:-1] + "+00:00" if text.endswith("Z") else text
    dt = datetime.fromisoformat(normalized)
    return dt if dt.tzinfo else dt.replace(tzinfo=timezone.utc)


def _build_parent_map(snapshot_rows: list[dict[str, Any]]) -> dict[SnapshotId, SnapshotId | None]:
    parent_by_id: dict[SnapshotId, SnapshotId | None] = {}
    for row in snapshot_rows:
        snapshot_id = str(row["snapshot_id"])
        raw_parent = row.get("parent_snapshot_id")
        if isinstance(raw_parent, list | tuple):
            raw_parent = raw_parent[0] if raw_parent else None
        parent_by_id[snapshot_id] = str(raw_parent) if raw_parent else None
    return parent_by_id


def snapshot_chain(
    snapshot_id: str | None,
    parent_by_id: dict[SnapshotId, SnapshotId | None],
) -> list[str]:
    """Return ancestry chain from snapshot -> root, newest -> oldest."""
    if not snapshot_id:
        return []

    chain: list[str] = []
    seen: set[str] = set()
    current = snapshot_id
    while current:
        if current in seen:
            raise ValueError(f"Cycle detected in snapshot ancestry at {current}")
        seen.add(current)
        if current not in parent_by_id:
            raise KeyError(f"Unknown snapshot id {current!r}")
        chain.append(current)
        current = parent_by_id[current]

    return chain


def snapshot_is_ancestor(
    ancestor_snapshot_id: str | None,
    descendant_snapshot_id: str | None,
    parent_by_id: dict[SnapshotId, SnapshotId | None],
) -> bool:
    """Return True when `ancestor_snapshot_id` is on `descendant_snapshot_id` chain."""
    if ancestor_snapshot_id is None or descendant_snapshot_id is None:
        return False
    if ancestor_snapshot_id == descendant_snapshot_id:
        return True
    return ancestor_snapshot_id in snapshot_chain(descendant_snapshot_id, parent_by_id)


def snapshot_distance_descendant_to_ancestor(
    descendant_snapshot_id: str | None,
    ancestor_snapshot_id: str | None,
    parent_by_id: dict[SnapshotId, SnapshotId | None],
) -> int | None:
    """Distance from descendant -> ancestor via parent links, or None if unrelated."""
    if ancestor_snapshot_id is None or descendant_snapshot_id is None:
        return None

    distance = 0
    for snap_id in snapshot_chain(descendant_snapshot_id, parent_by_id):
        if snap_id == ancestor_snapshot_id:
            return distance
        distance += 1

    return None


def snapshot_health_relation_to_main(
    main_head_snapshot_id: str | None,
    line_head_snapshot_id: str | None,
    parent_by_id: dict[SnapshotId, SnapshotId | None],
) -> dict[str, Any]:
    """Calculate line/head relation against main ancestry.

    Returns dict fields:
    - is_contained_in_main
    - ahead_count
    - behind_count
    - base_distance
    - base_snapshot_id
    """
    if not main_head_snapshot_id:
        return {
            "is_contained_in_main": False,
            "ahead_count": None,
            "behind_count": None,
            "base_distance": None,
            "base_snapshot_id": None,
        }

    if line_head_snapshot_id is None:
        return {
            "is_contained_in_main": False,
            "ahead_count": None,
            "behind_count": None,
            "base_distance": None,
            "base_snapshot_id": None,
        }

    try:
        main_chain = snapshot_chain(main_head_snapshot_id, parent_by_id)
        line_chain = snapshot_chain(line_head_snapshot_id, parent_by_id)
    except KeyError as exc:
        return {
            "is_contained_in_main": False,
            "ahead_count": None,
            "behind_count": None,
            "base_distance": None,
            "base_snapshot_id": None,
            "head_snapshot_missing": True,
            "head_snapshot_error": str(exc),
        }

    main_distances = {snapshot_id: distance for distance, snapshot_id in enumerate(main_chain)}
    for ahead_distance, snapshot_id in enumerate(line_chain):
        if snapshot_id not in main_distances:
            continue

        behind = main_distances[snapshot_id]
        is_contained = ahead_distance == 0
        return {
            "is_contained_in_main": is_contained,
            "ahead_count": 0 if is_contained else ahead_distance,
            "behind_count": behind,
            "base_distance": behind if is_contained else ahead_distance,
            "base_snapshot_id": snapshot_id,
            "head_snapshot_missing": False,
            "head_snapshot_error": None,
        }

    return {
        "is_contained_in_main": False,
        "ahead_count": None,
        "behind_count": None,
        "base_distance": None,
        "base_snapshot_id": None,
        "head_snapshot_missing": False,
        "head_snapshot_error": None,
    }


def _is_stale(
    *,
    updated_at: str | None,
    now: datetime,
    stale_days: float,
) -> bool:
    if not updated_at or stale_days <= 0:
        return False
    return (now - _coerce_datetime(updated_at)).total_seconds() >= stale_days * 24 * 60 * 60


def _age_days(value: datetime | str | None, now: datetime) -> float | None:
    if value is None:
        return None
    try:
        seconds = (now - _coerce_datetime(value)).total_seconds()
    except (TypeError, ValueError):
        return None
    return max(0.0, round(seconds / (24 * 60 * 60), 2))


def _find_main_snapshot_id(
    line_rows: list[dict[str, Any]],
    main_line_name: str,
    fallback_line_name: str | None = None,
) -> str | None:
    by_name = {str(row["line_name"]): row for row in line_rows}
    if main_line_name in by_name:
        return by_name[main_line_name].get("head_snapshot_id")
    if fallback_line_name and fallback_line_name in by_name:
        return by_name[fallback_line_name].get("head_snapshot_id")
    if line_rows:
        return line_rows[0].get("head_snapshot_id")
    return None


def attach_graph_layout(payload: dict[str, Any]) -> dict[str, Any]:
    """Attach active-column graph coordinates to each history row."""
    history_rows = payload.get("history_rows", [])
    if not isinstance(history_rows, list):
        return payload

    layout_by_id = {row["snapshot_id"]: row for row in layout_active_columns(history_rows)}
    for row in history_rows:
        snapshot_id = row.get("snapshot_id")
        graph = layout_by_id.get(snapshot_id)
        if graph is None:
            continue
        row["graph_layout"] = graph
        row["graph_column"] = graph["column"]
        row["graph_active_columns"] = graph["active_columns"]
        row["graph_segments"] = graph["segments"]
    return payload


def attach_provenance_overlays(
    payload: dict[str, Any],
    *,
    tasks: list[dict[str, Any]] | None = None,
    changes: list[dict[str, Any]] | None = None,
    patchsets: list[dict[str, Any]] | None = None,
    lands: list[dict[str, Any]] | None = None,
    sessions: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    """Attach read-only task/change/land/session badge overlays."""
    history_rows = payload.get("history_rows", [])
    if not isinstance(history_rows, list):
        return payload

    snapshot_ids = [str(row["snapshot_id"]) for row in history_rows if row.get("snapshot_id")]
    overlay = build_snapshot_provenance_overlay(
        snapshot_ids,
        tasks=tasks,
        changes=changes,
        patchsets=patchsets,
        lands=lands,
        sessions=sessions,
    )
    payload["provenance_overlays"] = overlay
    for row in history_rows:
        snapshot_id = row.get("snapshot_id")
        snapshot_overlay = overlay.get(str(snapshot_id), {"badges": [], "links": [], "items": []})
        row["provenance"] = snapshot_overlay
        row["provenance_badges"] = snapshot_overlay.get("badges", [])
    return payload


def _index_rows(rows: list[dict[str, Any]], *keys: str) -> dict[str, dict[str, Any]]:
    index: dict[str, dict[str, Any]] = {}
    for row in rows:
        row_id = _first_text(row, *keys)
        if row_id:
            index[row_id] = row
    return index


def _plan_links_by_id(plan_links: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    links: dict[str, dict[str, Any]] = {}
    for row in plan_links:
        plan_id = _first_text(row, "plan_id")
        if plan_id and plan_id not in links:
            links[plan_id] = row
    return links


def _plan_item_context(plan_link: dict[str, Any] | None, plan_item_ref: str) -> dict[str, Any]:
    if plan_link is None or not plan_item_ref:
        return {}
    item = find_plan_item_in_items(plan_link.get("items"), plan_item_ref)
    if not item:
        return {}
    context: dict[str, Any] = {}
    text = _first_text(item, "text")
    if text:
        context["plan_item_text"] = text
    heading_path = item.get("heading_path")
    if isinstance(heading_path, list):
        context["plan_item_heading_path"] = [str(value) for value in heading_path if str(value).strip()]
    line_number = item.get("line_number")
    if isinstance(line_number, int) and line_number > 0:
        context["plan_item_line_number"] = line_number
    checkbox_state = _first_text(item, "checkbox_state")
    if checkbox_state:
        context["plan_item_state"] = checkbox_state
    return context


def _plan_context_from_task(
    task: dict[str, Any],
    *,
    item: dict[str, Any],
    change: dict[str, Any] | None,
    patchset: dict[str, Any] | None,
    plan_links: dict[str, dict[str, Any]],
) -> dict[str, Any] | None:
    kind = _first_text(item, "kind")
    item_id = _first_text(item, "id")
    task_id = _first_text(task, "task_id") or _first_text(item, "task_id") or (item_id if kind == "task" else "")
    plan_id = _first_text(task, "plan_id") or _first_text(item, "plan_id")
    plan_item_ref = _first_text(task, "plan_item_ref") or _first_text(item, "plan_item_ref")
    plan_revision_id = (
        _first_text(task, "origin_plan_revision_id", "plan_revision_id")
        or _first_text(item, "origin_plan_revision_id", "plan_revision_id")
    )
    if not any((plan_id, plan_item_ref, plan_revision_id)):
        return None

    plan_link = plan_links.get(plan_id) if plan_id else None
    context: dict[str, Any] = {
        "task_id": task_id,
        "task_title": _first_text(task, "title") or _first_text(item, "title"),
        "task_intent": _first_text(task, "intent"),
        "task_status": _first_text(task, "status"),
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id or _first_text(plan_link or {}, "head_revision_id"),
        "plan_item_ref": plan_item_ref,
    }
    if change:
        context["change_id"] = _first_text(change, "change_id")
        context["change_title"] = _first_text(change, "title")
    elif _first_text(item, "change_id"):
        context["change_id"] = _first_text(item, "change_id")
    if patchset:
        context["patchset_id"] = _first_text(patchset, "patchset_id")
        context["patchset_summary"] = _first_text(patchset, "summary")
    elif kind == "patchset":
        context["patchset_id"] = item_id

    if plan_link is not None:
        for source_key, target_key in (
            ("title", "plan_title"),
            ("status", "plan_status"),
            ("artifact_path", "artifact_path"),
            ("artifact_selector", "artifact_selector"),
            ("artifact_heading", "artifact_heading"),
            ("display_path", "display_path"),
        ):
            value = _first_text(plan_link, source_key)
            if value:
                context[target_key] = value
    if "plan_title" not in context and plan_id:
        context["plan_title"] = plan_id
    if "display_path" not in context:
        context["display_path"] = _plan_display_path(
            _first_text(context, "artifact_path"),
            _first_text(context, "artifact_selector"),
        )
    context.update(_plan_item_context(plan_link, plan_item_ref))

    return {key: value for key, value in context.items() if value not in ("", None, [])}


def attach_plan_contexts(
    payload: dict[str, Any],
    *,
    provenance: dict[str, list[dict[str, Any]]] | None = None,
    plan_links: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    """Attach task-linked plan context keyed by selected snapshot id."""
    history_rows = payload.get("history_rows", [])
    if not isinstance(history_rows, list):
        payload["plan_context_by_snapshot"] = {}
        return payload

    provenance = provenance or {}
    plan_links_by_id = _plan_links_by_id(list(plan_links or payload.get("plan_links") or []))
    tasks_by_id = _index_rows(list(provenance.get("tasks") or []), "task_id")
    changes_by_id = _index_rows(list(provenance.get("changes") or []), "change_id")
    patchsets_by_id = _index_rows(list(provenance.get("patchsets") or []), "patchset_id")
    sessions_by_id = _index_rows(list(provenance.get("sessions") or []), "session_id")

    contexts_by_snapshot: dict[str, list[dict[str, Any]]] = {}
    for row in history_rows:
        snapshot_id = _first_text(row, "snapshot_id")
        contexts: list[dict[str, Any]] = []
        seen: set[tuple[str, str, str, str, str]] = set()
        overlay_items = ((row.get("provenance") or {}).get("items") or []) if isinstance(row.get("provenance"), dict) else []
        for item in overlay_items:
            if not isinstance(item, dict):
                continue
            if _first_text(item, "snapshot_role") == "base":
                continue
            kind = _first_text(item, "kind")
            item_id = _first_text(item, "id")
            task: dict[str, Any] | None = None
            change: dict[str, Any] | None = None
            patchset: dict[str, Any] | None = None

            if kind == "task":
                task = tasks_by_id.get(item_id)
            elif kind == "change":
                change = changes_by_id.get(item_id)
            elif kind == "patchset":
                patchset = patchsets_by_id.get(item_id)
                if patchset is not None:
                    change = changes_by_id.get(_first_text(patchset, "change_id"))
            elif kind == "session":
                session = sessions_by_id.get(item_id)
                if session is not None:
                    task = tasks_by_id.get(_first_text(session, "task_id"))
                    change = changes_by_id.get(_first_text(session, "change_id"))

            task_id = _first_text(item, "task_id")
            change_id = _first_text(item, "change_id")
            if change is None and change_id:
                change = changes_by_id.get(change_id)
            if task is None and task_id:
                task = tasks_by_id.get(task_id)
            if task is None and change is not None:
                task = tasks_by_id.get(_first_text(change, "task_id"))
            task_like = task or item
            context = _plan_context_from_task(
                task_like,
                item=item,
                change=change,
                patchset=patchset,
                plan_links=plan_links_by_id,
            )
            if context is None:
                continue
            key = (
                _first_text(context, "task_id"),
                _first_text(context, "change_id"),
                _first_text(context, "patchset_id"),
                _first_text(context, "plan_id"),
                _first_text(context, "plan_item_ref"),
            )
            if key in seen:
                continue
            seen.add(key)
            contexts.append(context)

        row["plan_contexts"] = contexts
        row["plan_context_count"] = len(contexts)
        if snapshot_id and contexts:
            contexts_by_snapshot[snapshot_id] = contexts

    payload["plan_context_by_snapshot"] = contexts_by_snapshot
    if isinstance(payload.get("summary"), dict):
        payload["summary"]["plan_context_snapshot_count"] = len(contexts_by_snapshot)
    return payload


def attach_snapshot_parent_diffs(
    ctx: RepoContext,
    payload: dict[str, Any],
    *,
    include_text: bool = True,
    max_bytes: int = DEFAULT_SNAPSHOT_DIFF_MAX_BYTES,
) -> dict[str, Any]:
    """Attach bounded parent→snapshot diffs to every aitk history row."""
    for row in payload.get("history_rows", []):
        snapshot_id = row.get("snapshot_id")
        if not snapshot_id:
            continue
        parent_snapshot_id = row.get("parent_snapshot_id") or ""
        try:
            diff = snapshot_diff(
                ctx,
                str(parent_snapshot_id),
                str(snapshot_id),
                include_text=include_text,
                max_bytes=max_bytes,
            )
        except Exception as exc:  # pragma: no cover - defensive UI payload path
            diff = {
                "old_snapshot_id": parent_snapshot_id or None,
                "new_snapshot_id": snapshot_id,
                "files": [],
                "summary": {
                    "old_snapshot_id": parent_snapshot_id or None,
                    "new_snapshot_id": snapshot_id,
                    "files_changed": None,
                    "insertions": 0,
                    "deletions": 0,
                },
                "error": str(exc),
            }
        row["parent_diff"] = diff
        row["changed_files"] = [file_row.get("path") for file_row in diff.get("files", []) if file_row.get("path")]

    return payload


def _first_text(row: dict[str, Any], *keys: str) -> str:
    for key in keys:
        value = row.get(key)
        if value is not None and str(value).strip():
            return str(value).strip()
    return ""


def _plan_display_path(artifact_path: str, artifact_selector: str) -> str:
    if artifact_path and artifact_selector:
        return f"{artifact_path}#{artifact_selector}"
    return artifact_path or artifact_selector


def _local_plan_revision_items(ctx: RepoContext, plan_id: str, plan_revision_id: str) -> list[dict[str, Any]]:
    if not plan_id or not plan_revision_id:
        return []
    try:
        revision = get_local_plan_revision(ctx, plan_id, plan_revision_id)
    except Exception:  # pragma: no cover - optional UI enrichment path
        return []
    items = revision.get("items")
    return list(items) if isinstance(items, list) else []


def _markdown_title(path: Path) -> str:
    try:
        with path.open("r", encoding="utf-8") as handle:
            for line in handle:
                text = line.strip()
                if text.startswith("# "):
                    return text[2:].strip()
    except OSError:
        return ""
    return path.stem.replace("_", " ").replace("-", " ").strip() or path.name


def build_markdown_docs(ctx: RepoContext) -> list[dict[str, Any]]:
    """Return Markdown documents under docs/ for the local aitk browser."""
    try:
        docs_dir = ctx.root / "docs"
    except Exception:
        return []
    if not docs_dir.is_dir():
        return []

    docs: list[dict[str, Any]] = []
    for path in sorted(docs_dir.rglob("*.md")):
        if not path.is_file():
            continue
        try:
            rel_path = str(path.relative_to(ctx.root))
        except ValueError:
            rel_path = str(path)
        try:
            size_bytes = path.stat().st_size
        except OSError:
            size_bytes = 0
        docs.append(
            {
                "kind": "markdown_doc",
                "source": "docs",
                "path": rel_path,
                "display_path": rel_path,
                "title": _markdown_title(path),
                "size_bytes": size_bytes,
            }
        )
    return docs


def _scanned_plan_artifact_links(ctx: RepoContext, seen_artifact_paths: set[str]) -> list[dict[str, Any]]:
    try:
        plan_artifact_dirs = [ctx.root / "docs" / "sprints"]
    except Exception:
        return []

    links: list[dict[str, Any]] = []
    for plans_dir in plan_artifact_dirs:
        if not plans_dir.is_dir():
            continue
        try:
            source = str(plans_dir.relative_to(ctx.root))
        except ValueError:
            source = str(plans_dir)
        for path in sorted(plans_dir.glob("*.md")):
            try:
                artifact_path = str(path.relative_to(ctx.root))
            except ValueError:
                artifact_path = str(path)
            if artifact_path in seen_artifact_paths:
                continue
            seen_artifact_paths.add(artifact_path)
            links.append(
                {
                    "kind": "plan_artifact",
                    "source": source,
                    "plan_id": "",
                    "title": _markdown_title(path),
                    "status": "file",
                    "head_revision_id": "",
                    "head_revision_number": "",
                    "artifact_path": artifact_path,
                    "artifact_selector": "",
                    "artifact_heading": "",
                    "display_path": artifact_path,
                    "items": [],
                }
            )
    return links


def build_plan_links(
    ctx: RepoContext,
    *,
    plans: list[dict[str, Any]] | None = None,
    include_markdown_artifacts: bool = True,
) -> list[dict[str, Any]]:
    """Return right-pane plan links for the local aitk browser."""
    if plans is None:
        try:
            plans = list_local_plans(ctx)
        except Exception:  # pragma: no cover - optional UI enrichment path
            plans = []

    links: list[dict[str, Any]] = []
    seen_artifact_paths: set[str] = set()
    for row in plans:
        artifact_path = _first_text(row, "artifact_path", "head_artifact_path")
        artifact_selector = _first_text(row, "artifact_selector", "head_artifact_selector")
        artifact_heading = _first_text(row, "artifact_heading", "head_artifact_heading")
        if artifact_path:
            seen_artifact_paths.add(artifact_path)

        plan_id = _first_text(row, "plan_id")
        title = _first_text(row, "title", "head_revision_summary", "summary", "artifact_heading", "head_artifact_heading")
        links.append(
            {
                "kind": "plan",
                "source": "local_plan",
                "plan_id": plan_id,
                "title": title or plan_id,
                "status": _first_text(row, "status") or "unknown",
                "head_revision_id": _first_text(row, "head_revision_id"),
                "head_revision_number": _first_text(row, "head_revision_number"),
                "artifact_path": artifact_path,
                "artifact_selector": artifact_selector,
                "artifact_heading": artifact_heading,
                "display_path": _plan_display_path(artifact_path, artifact_selector),
                "items": _local_plan_revision_items(ctx, plan_id, _first_text(row, "head_revision_id")),
            }
        )

    if include_markdown_artifacts:
        links.extend(_scanned_plan_artifact_links(ctx, seen_artifact_paths))

    return links


def _workflow_scope_value(ctx: RepoContext, kind: str) -> str:
    try:
        cfg = load_config(ctx)
    except Exception:
        return "local"
    scope = cfg.get(f"{kind}_default_scope")
    if scope is None:
        scope = cfg.get("workflow_default_scope")
    text = str(scope or "remote").strip().lower()
    return text if text in {"local", "remote"} else "remote"


def _remote_repo_tuple(ctx: RepoContext) -> tuple[str, str] | None:
    try:
        remote = get_remote(ctx)
        cfg = load_config(ctx)
        repo_name = str(remote.get("repo_name") or cfg.get("repo_name") or ctx.root.name).strip()
        url = str(remote.get("url") or "").strip()
    except Exception:
        return None
    if not url or not repo_name:
        return None
    return url, repo_name


def _load_remote_workflow_provenance(
    ctx: RepoContext,
    *,
    snapshot_ids: set[str],
    max_patchset_changes: int,
) -> dict[str, list[dict[str, Any]]]:
    if remote_list_tasks is None or remote_list_changes is None or remote_list_patchsets is None:
        return {}
    if _workflow_scope_value(ctx, "task") != "remote" and _workflow_scope_value(ctx, "change") != "remote":
        return {}
    remote_tuple = _remote_repo_tuple(ctx)
    if remote_tuple is None:
        return {}

    url, repo_name = remote_tuple
    try:
        tasks = remote_list_tasks(url, repo_name)
        changes = remote_list_changes(url, repo_name)
    except (KeyError, RemoteError, OSError, TimeoutError, ValueError):  # pragma: no cover - depends on optional server
        return {}
    if not isinstance(tasks, list) or not isinstance(changes, list):
        return {}

    patchsets: list[dict[str, Any]] = []
    fetched_changes = 0
    for change in changes:
        if fetched_changes >= max_patchset_changes:
            break
        change_id = _first_text(change, "change_id")
        if not change_id:
            continue
        try:
            current_patchset_number = int(change.get("current_patchset_number") or 0)
        except (TypeError, ValueError):
            current_patchset_number = 0
        if current_patchset_number <= 0:
            continue
        try:
            rows = remote_list_patchsets(url, change_id)
        except (KeyError, RemoteError, OSError, TimeoutError, ValueError):  # pragma: no cover - depends on optional server
            continue
        fetched_changes += 1
        for row in rows:
            if not isinstance(row, dict):
                continue
            if not _first_text(row, "change_id"):
                row = {**row, "change_id": change_id}
            if snapshot_ids and not (
                _first_text(row, "revision_snapshot_id") in snapshot_ids
                or _first_text(row, "base_snapshot_id") in snapshot_ids
            ):
                continue
            patchsets.append(row)

    return {"tasks": list(tasks), "changes": list(changes), "patchsets": patchsets}


def _merge_provenance_rows(base: dict[str, list[dict[str, Any]]], extra: dict[str, list[dict[str, Any]]]) -> None:
    for key, rows in extra.items():
        if not rows:
            continue
        base.setdefault(key, [])
        base[key].extend(row for row in rows if isinstance(row, dict))


def build_aitk_payload_from_rows(
    snapshot_rows: list[dict[str, Any]],
    line_rows: list[dict[str, Any]],
    *,
    now: datetime | str | None = None,
    stale_days: float = 3,
    main_line_name: str = "main",
    fallback_main_line_name: str | None = None,
    provenance: dict[str, list[dict[str, Any]]] | None = None,
    plan_links: list[dict[str, Any]] | None = None,
    markdown_docs: list[dict[str, Any]] | None = None,
    repo_root: str | None = None,
) -> dict[str, Any]:
    """Build aitk read-model payload from preloaded rows."""
    now_dt = datetime.now(timezone.utc) if now is None else _coerce_datetime(now)
    parent_by_id = _build_parent_map(snapshot_rows)

    main_head_snapshot_id = _find_main_snapshot_id(line_rows, main_line_name, fallback_main_line_name)

    head_lines_by_snapshot: dict[str, list[str]] = {}
    for row in line_rows:
        head_snapshot_id = row.get("head_snapshot_id")
        if not head_snapshot_id:
            continue
        head_lines_by_snapshot.setdefault(str(head_snapshot_id), []).append(str(row["line_name"]))
    for labels in head_lines_by_snapshot.values():
        labels.sort()

    # Build a light lookup for snapshot rows.
    snapshot_by_id = {str(row["snapshot_id"]): row for row in snapshot_rows}

    history_rows: list[dict[str, Any]] = []
    for row in snapshot_rows:
        snapshot_id = str(row["snapshot_id"])
        head_lines = head_lines_by_snapshot.get(snapshot_id, [])
        is_main_head = bool(main_head_snapshot_id) and snapshot_id == str(main_head_snapshot_id)
        is_head = bool(head_lines)
        marker = "@" if is_main_head else ("*" if is_head else "o")
        history_rows.append(
            {
                **row,
                "age_days": _age_days(row.get("created_at"), now_dt),
                "head_lines": list(head_lines),
                "is_head": is_head,
                "is_main_head": is_main_head,
                "marker": marker,
                "graph": marker,
                "head_line_count": len(head_lines),
            }
        )

    line_health_rows: list[dict[str, Any]] = []
    for row in line_rows:
        line_name = str(row["line_name"])
        head_snapshot_id = row.get("head_snapshot_id")
        relation = snapshot_health_relation_to_main(
            str(main_head_snapshot_id) if main_head_snapshot_id else None,
            str(head_snapshot_id) if head_snapshot_id else None,
            parent_by_id,
        )
        head_snapshot = snapshot_by_id.get(str(head_snapshot_id)) if head_snapshot_id else None
        is_stale = _is_stale(
            updated_at=str(row.get("updated_at") or row.get("created_at") or ""),
            now=now_dt,
            stale_days=stale_days,
        )
        head_label_rows = head_lines_by_snapshot.get(str(head_snapshot_id), [line_name]) if head_snapshot_id else [line_name]
        line_health_rows.append(
            {
                "line_name": line_name,
                "status": row.get("status") or "active",
                "archived_at": row.get("archived_at"),
                "created_at": row.get("created_at"),
                "updated_at": row.get("updated_at"),
                "head_snapshot_id": head_snapshot_id,
                "head_line_labels": head_label_rows,
                "head_snapshot_created_at": head_snapshot.get("created_at") if head_snapshot else None,
                "is_contained_in_main": relation["is_contained_in_main"],
                "ahead_count": relation["ahead_count"],
                "behind_count": relation["behind_count"],
                "base_distance": relation["base_distance"],
                "base_snapshot_id": relation["base_snapshot_id"],
                "head_snapshot_missing": relation.get("head_snapshot_missing", False),
                "head_snapshot_error": relation.get("head_snapshot_error"),
                "is_stale": is_stale,
                "stale_days": stale_days,
                "stale_at": now_dt.isoformat(),
                "is_main_line": line_name == main_line_name,
            }
        )

    contained_lines = [row for row in line_health_rows if row["is_contained_in_main"]]
    active_lines = [row for row in line_health_rows if row.get("status") == "active"]
    uncontained_with_head = [row for row in line_health_rows if row["head_snapshot_id"] and not row["is_contained_in_main"]]
    stale_uncontained = [row for row in uncontained_with_head if row["is_stale"]]

    payload = {
        "repo_root": repo_root,
        "main_line_name": main_line_name,
        "main_head_snapshot_id": main_head_snapshot_id,
        "history_rows": history_rows,
        "line_health_rows": line_health_rows,
        "plan_links": list(plan_links or []),
        "markdown_docs": list(markdown_docs or []),
        "summary": {
            "history_count": len(history_rows),
            "line_count": len(line_health_rows),
            "plan_link_count": len(plan_links or []),
            "markdown_doc_count": len(markdown_docs or []),
            "active_line_count": len(active_lines),
            "contained_line_count": len(contained_lines),
            "uncontained_line_count": len(uncontained_with_head),
            "stale_uncontained_line_count": len(stale_uncontained),
            "stale_threshold_days": stale_days,
            "generated_at": now_dt.isoformat(),
        },
    }
    attach_graph_layout(payload)
    attach_provenance_overlays(payload, **(provenance or {}))
    attach_plan_contexts(payload, provenance=provenance, plan_links=plan_links)
    return payload


def build_aitk_history_payload(
    ctx: RepoContext,
    *,
    now: datetime | str | None = None,
    stale_days: float = 3,
    main_line_name: str = "main",
    include_snapshot_diffs: bool = False,
    snapshot_diff_include_text: bool = True,
    snapshot_diff_max_bytes: int = DEFAULT_SNAPSHOT_DIFF_MAX_BYTES,
    include_provenance: bool = True,
    include_plan_links: bool = True,
    include_markdown_docs: bool = True,
    include_remote_workflow: bool = True,
    remote_patchset_change_limit: int = 75,
) -> dict[str, Any]:
    """Read-only convenience wrapper around preloaded-row builder."""
    snapshot_rows = list_snapshots(ctx)
    line_rows = list_lines(ctx)
    fallback = current_line(ctx)
    provenance = None
    if include_provenance:
        provenance = {}
        for key, loader in (
            ("tasks", list_local_tasks),
            ("changes", list_local_changes),
            ("sessions", list_local_sessions),
        ):
            try:
                provenance[key] = loader(ctx)
            except Exception:  # pragma: no cover - optional overlay path
                provenance[key] = []
        if include_remote_workflow:
            _merge_provenance_rows(
                provenance,
                _load_remote_workflow_provenance(
                    ctx,
                    snapshot_ids={str(row["snapshot_id"]) for row in snapshot_rows if row.get("snapshot_id")},
                    max_patchset_changes=max(0, int(remote_patchset_change_limit)),
                ),
            )
    plan_links = build_plan_links(ctx) if include_plan_links else []
    markdown_docs = build_markdown_docs(ctx) if include_markdown_docs else []
    try:
        repo_root = str(ctx.root)
    except Exception:
        repo_root = None
    payload = build_aitk_payload_from_rows(
        snapshot_rows,
        line_rows,
        now=now,
        stale_days=stale_days,
        main_line_name=main_line_name,
        fallback_main_line_name=fallback,
        provenance=provenance,
        plan_links=plan_links,
        markdown_docs=markdown_docs,
        repo_root=repo_root,
    )
    if include_snapshot_diffs:
        attach_snapshot_parent_diffs(
            ctx,
            payload,
            include_text=snapshot_diff_include_text,
            max_bytes=snapshot_diff_max_bytes,
        )
    return payload


def build_line_health_rows(
    snapshot_rows: list[dict[str, Any]],
    line_rows: list[dict[str, Any]],
    *,
    now: datetime | str | None = None,
    stale_days: float = 3,
    main_line_name: str = "main",
    fallback_main_line_name: str | None = None,
) -> list[dict[str, Any]]:
    """Compatibility helper returning only line health rows."""
    return build_aitk_payload_from_rows(
        snapshot_rows,
        line_rows,
        now=now,
        stale_days=stale_days,
        main_line_name=main_line_name,
        fallback_main_line_name=fallback_main_line_name,
    )["line_health_rows"]
