from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

SESSION_LIST_SUMMARY_FIELDS = (
    "session_id",
    "repo_name",
    "task_id",
    "change_id",
    "title",
    "session_kind",
    "status",
    "line_name",
    "worktree_name",
    "model_name",
    "actor_identity",
    "actor_type",
    "last_event_sequence",
    "head_checkpoint_id",
    "created_at",
    "updated_at",
)


def session_list_summary_row(row: Mapping[str, Any]) -> dict[str, Any]:
    return {field: row.get(field) for field in SESSION_LIST_SUMMARY_FIELDS}


def session_list_summary_rows(rows: Iterable[Mapping[str, Any]]) -> list[dict[str, Any]]:
    return [session_list_summary_row(row) for row in rows]


__all__ = [
    "SESSION_LIST_SUMMARY_FIELDS",
    "session_list_summary_row",
    "session_list_summary_rows",
]
