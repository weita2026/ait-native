from __future__ import annotations

from typing import Any


HistoryRow = dict[str, Any]
GraphSegment = dict[str, Any]


def layout_active_columns(history_rows: list[HistoryRow]) -> list[dict[str, Any]]:
    """
    Compute a gitk-style active-column layout for newest-first snapshot history rows.

    Each input row represents a snapshot and may contain:
    - snapshot_id (required)
    - parent_snapshot_id (single parent string or list of parent strings)

    Output rows include:
    - snapshot_id
    - column (active column index for this snapshot)
    - active_columns (columns currently in use at this row)
    - segments (parent edges from this snapshot to its parents)
    - labels (line labels, copied from head_lines when available)
    """

    if not isinstance(history_rows, list):
        raise TypeError("history_rows must be a list")

    active_parent_column: dict[str, int] = {}
    layout_rows: list[dict[str, Any]] = []

    for index, row in enumerate(history_rows):
        if not isinstance(row, dict):
            raise TypeError(f"history_rows[{index}] must be a dict")

        snapshot_id = row.get("snapshot_id")
        if not isinstance(snapshot_id, str) or not snapshot_id:
            raise ValueError(f"history_rows[{index}] must contain snapshot_id")

        parent_ids = _extract_parent_ids(row.get("parent_snapshot_id"))
        labels = row.get("head_lines", [])
        if not isinstance(labels, list):
            labels = []

        assigned_column = active_parent_column.pop(snapshot_id, None)
        reserved_columns = set(active_parent_column.values())
        if assigned_column is None:
            assigned_column = _next_free_column(reserved_columns)

        used_columns = set(reserved_columns)
        used_columns.add(assigned_column)

        segments: list[GraphSegment] = []
        taken_for_multi_parent = set()
        for parent_idx, parent_id in enumerate(parent_ids):
            if not isinstance(parent_id, str) or not parent_id:
                continue

            parent_column = active_parent_column.get(parent_id)
            if parent_column is None:
                if parent_idx == 0 and parent_ids:
                    parent_column = assigned_column
                else:
                    parent_column = _next_free_column(used_columns | taken_for_multi_parent)

            active_parent_column[parent_id] = parent_column
            used_columns.add(parent_column)
            taken_for_multi_parent.add(parent_column)
            segments.append(
                {
                    "to_snapshot_id": parent_id,
                    "from_column": assigned_column,
                    "to_column": parent_column,
                    "kind": "parent",
                }
            )

        layout_rows.append(
            {
                "snapshot_id": snapshot_id,
                "parent_snapshot_id": row.get("parent_snapshot_id"),
                "column": assigned_column,
                "active_columns": sorted(used_columns),
                "segments": segments,
                "labels": labels,
                "row_index": index,
            }
        )

    return layout_rows


def _extract_parent_ids(parent_snapshot_id: Any) -> list[str]:
    if parent_snapshot_id is None:
        return []
    if isinstance(parent_snapshot_id, str):
        return [parent_snapshot_id]
    if isinstance(parent_snapshot_id, (list, tuple)):
        parents: list[str] = []
        for raw_parent in parent_snapshot_id:
            if isinstance(raw_parent, str) and raw_parent:
                parents.append(raw_parent)
        return parents
    return []


def _next_free_column(used: set[int]) -> int:
    column = 0
    while column in used:
        column += 1
    return column
