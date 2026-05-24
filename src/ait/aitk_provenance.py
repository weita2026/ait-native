from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

SNAPSHOT_REFERENCE_FIELDS = (
    "snapshot_id",
    "head_snapshot_id",
    "target_snapshot_id",
    "revision_snapshot_id",
    "base_snapshot_id",
)

_PROVENANCE_KIND_LABELS = {
    "tasks": "task",
    "changes": "change",
    "patchsets": "patchset",
    "lands": "land",
    "sessions": "session",
}

_ID_FIELDS_BY_KIND = {
    "tasks": ("task_id",),
    "changes": ("change_id",),
    "patchsets": ("patchset_id",),
    "lands": ("submission_id", "land_id"),
    "sessions": ("session_id",),
}

_CONTEXT_FIELDS = (
    "task_id",
    "change_id",
    "patchset_id",
    "plan_id",
    "origin_plan_revision_id",
    "plan_revision_id",
    "plan_item_ref",
    "plan_section_ref",
    "status",
    "intent",
    "base_snapshot_id",
    "revision_snapshot_id",
    "target_snapshot_id",
    "head_snapshot_id",
)


def build_snapshot_provenance_overlay(
    snapshot_ids: Iterable[str],
    *,
    tasks: Iterable[Mapping[str, Any]] | None = None,
    changes: Iterable[Mapping[str, Any]] | None = None,
    patchsets: Iterable[Mapping[str, Any]] | None = None,
    lands: Iterable[Mapping[str, Any]] | None = None,
    sessions: Iterable[Mapping[str, Any]] | None = None,
) -> dict[str, dict[str, Any]]:
    """Build read-only aitk provenance badges keyed by snapshot id.

    The function is deliberately row-oriented and side-effect free. Callers can
    feed rows from CLI JSON, server read models, or future local read models.
    Unknown fields are ignored so the overlay can evolve without making the GUI
    tightly coupled to one database shape.
    """

    overlay: dict[str, dict[str, Any]] = {
        snapshot_id: {"snapshot_id": snapshot_id, "badges": [], "links": [], "items": []}
        for snapshot_id in snapshot_ids
        if isinstance(snapshot_id, str) and snapshot_id.strip()
    }
    if not overlay:
        return overlay

    for collection_name, rows in (
        ("tasks", tasks),
        ("changes", changes),
        ("patchsets", patchsets),
        ("lands", lands),
        ("sessions", sessions),
    ):
        if rows is None:
            continue
        _attach_rows(overlay, collection_name, rows)

    for row in overlay.values():
        row["badges"] = sorted(set(row["badges"]))
    return overlay


def _attach_rows(
    overlay: dict[str, dict[str, Any]],
    collection_name: str,
    rows: Iterable[Mapping[str, Any]],
) -> None:
    kind = _PROVENANCE_KIND_LABELS[collection_name]
    for source_row in rows:
        if not isinstance(source_row, Mapping):
            continue
        row_id = _first_text(source_row, _ID_FIELDS_BY_KIND[collection_name])
        if row_id is None:
            continue
        for snapshot_id, role in _snapshot_references(source_row):
            target = overlay.get(snapshot_id)
            if target is None:
                continue
            target["badges"].append(kind)
            target["links"].append(_link_for(kind, row_id))
            item = {
                "kind": kind,
                "id": row_id,
                "snapshot_id": snapshot_id,
                "snapshot_role": role,
            }
            title = _first_text(source_row, ("title", "summary", "message", "status"))
            if title is not None:
                item["title"] = title
            for field in _CONTEXT_FIELDS:
                value = _first_text(source_row, (field,))
                if value is not None:
                    item[field] = value
            target["items"].append(item)


def _snapshot_references(row: Mapping[str, Any]) -> list[tuple[str, str]]:
    refs: list[tuple[str, str]] = []
    seen: set[tuple[str, str]] = set()
    for field in SNAPSHOT_REFERENCE_FIELDS:
        value = row.get(field)
        if not isinstance(value, str) or not value.strip():
            continue
        role = "snapshot" if field == "snapshot_id" else field.removesuffix("_snapshot_id")
        key = (value.strip(), role)
        if key in seen:
            continue
        refs.append(key)
        seen.add(key)
    return refs


def _first_text(row: Mapping[str, Any], fields: Iterable[str]) -> str | None:
    for field in fields:
        value = row.get(field)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return None


def _link_for(kind: str, row_id: str) -> str:
    if kind == "task":
        return f"/tasks/{row_id}"
    if kind == "change":
        return f"/changes/{row_id}"
    if kind == "patchset":
        return f"patchset:{row_id}"
    if kind == "land":
        return f"land:{row_id}"
    if kind == "session":
        return f"/sessions/{row_id}"
    return row_id


__all__ = ["SNAPSHOT_REFERENCE_FIELDS", "build_snapshot_provenance_overlay"]
