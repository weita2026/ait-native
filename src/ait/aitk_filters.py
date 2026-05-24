from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

LineHealthRow = dict[str, Any]
HistoryRow = dict[str, Any]

__all__ = ["filter_line_health", "filter_history_rows"]


def filter_line_health(
    rows: list[LineHealthRow],
    *,
    query: str | None = None,
    stale_days: int | float | None = None,
    contained: bool | None = None,
    uncontained: bool | None = None,
    dirty_related: bool | None = None,
) -> list[LineHealthRow]:
    """Filter line-health rows by text and status predicates.

    This helper is pure and side-effect free.
    """

    if not isinstance(rows, list):
        raise TypeError("rows must be a list of mapping-like objects")

    stale_threshold = _as_number(stale_days) if stale_days is not None else None
    if stale_days is not None and stale_threshold is None:
        raise TypeError("stale_days must be numeric")

    result: list[LineHealthRow] = []
    for row in rows:
        if not isinstance(row, Mapping):
            continue

        if not _match_line_health_query(row, query):
            continue
        if not _match_contained_filter(row, contained=contained, uncontained=uncontained):
            continue
        if not _match_dirty_related(row, dirty_related):
            continue
        if stale_threshold is not None and not _row_is_stale(row, stale_threshold):
            continue

        result.append(row)

    return result


def filter_history_rows(
    rows: list[HistoryRow],
    *,
    query: str | None = None,
    line: str | None = None,
    path: str | None = None,
) -> list[HistoryRow]:
    """Filter history rows by query, line name and path criteria."""

    if not isinstance(rows, list):
        raise TypeError("rows must be a list of mapping-like objects")

    normalized_line = _normalize_text(line)
    normalized_path = _normalize_text(path)

    result: list[HistoryRow] = []
    for row in rows:
        if not isinstance(row, Mapping):
            continue

        if not _match_history_query(row, query):
            continue
        if not _field_contains(row, "line_name", "line", value=normalized_line):
            continue
        if normalized_path is not None and not _row_paths_contain(row, normalized_path):
            continue

        result.append(row)

    return result


def _match_line_health_query(row: Mapping[str, Any], query: str | None) -> bool:
    normalized_query = _normalize_text(query)
    if normalized_query is None:
        return True

    tokens = [
        row.get("snapshot_id"),
        row.get("line_name"),
        row.get("message"),
        row.get("title"),
        row.get("summary"),
    ]
    tokens.extend(_collect_string_iterables(row, "head_lines", "head_labels", "line_labels"))
    haystack = " ".join(_ensure_str(token).lower() for token in tokens)
    return normalized_query in haystack


def _match_history_query(row: Mapping[str, Any], query: str | None) -> bool:
    normalized_query = _normalize_text(query)
    if normalized_query is None:
        return True

    tokens = [
        row.get("snapshot_id"),
        row.get("line_name"),
        row.get("line"),
        row.get("message"),
        row.get("title"),
    ]
    tokens.extend(_collect_string_iterables(row, "head_lines", "head_labels", "line_labels"))
    haystack = " ".join(_ensure_str(token).lower() for token in tokens)
    return normalized_query in haystack


def _match_contained_filter(
    row: Mapping[str, Any],
    *,
    contained: bool | None,
    uncontained: bool | None,
) -> bool:
    if contained is None and uncontained is None:
        return True

    # If both are set to the same value, interpret as no-op / broad query.
    if contained is not None and uncontained is not None and contained is uncontained:
        return True

    contained_value = _coerce_bool(
        _first_text(
            row,
            "is_contained_in_main",
            "is_contained",
            "contained",
            "contained_in_main",
            "contained_in_mainline",
        )
    )
    if contained_value is None:
        return False

    if contained is not None and contained_value is not contained:
        return False
    if uncontained is not None and contained_value is not (not uncontained):
        return False
    return True


def _match_dirty_related(row: Mapping[str, Any], dirty_related: bool | None) -> bool:
    if dirty_related is None:
        return True

    dirty_value = _coerce_bool(
        _first_text(
            row,
            "dirty_related",
            "is_dirty_related",
            "workspace_dirty_related",
            "related_to_dirty_workspace",
        )
    )
    if dirty_value is None:
        return False
    return dirty_value is dirty_related


def _row_is_stale(row: Mapping[str, Any], stale_threshold: float) -> bool:
    stale_age = _as_number(_first_text(row, "stale_age_days", "age_days", "days_since_update"))
    if stale_age is None:
        return False
    return stale_age >= stale_threshold


def _field_contains(row: Mapping[str, Any], *fields: str, value: str | None) -> bool:
    if value is None:
        return True

    for field in fields:
        raw = row.get(field)
        if isinstance(raw, str) and value in raw.lower():
            return True
        if isinstance(raw, Iterable) and not isinstance(raw, (bytes, str, dict)):
            for item in raw:
                if isinstance(item, str) and value in item.lower():
                    return True
    return False


def _row_paths_contain(row: Mapping[str, Any], path_query: str) -> bool:
    for path in _row_paths(row):
        if path_query in path:
            return True
    return False


def _row_paths(row: Mapping[str, Any]) -> list[str]:
    paths: list[str] = []
    for field in (
        "changed_paths",
        "changed_files",
        "paths",
        "files",
        "touched_paths",
        "path_changes",
    ):
        raw = row.get(field)
        if raw is None:
            continue

        if isinstance(raw, str):
            paths.extend(_split_path_text(raw))
            continue

        if isinstance(raw, Mapping):
            p = _extract_path(raw)
            if p:
                paths.append(p)
            continue

        if isinstance(raw, Iterable) and not isinstance(raw, (bytes, str, dict)):
            for item in raw:
                if isinstance(item, str):
                    paths.append(item)
                elif isinstance(item, Mapping):
                    p = _extract_path(item)
                    if p:
                        paths.append(p)

    return [p.lower() for p in paths if isinstance(p, str) and p.strip()]


def _collect_string_iterables(row: Mapping[str, Any], *fields: str) -> list[str]:
    values: list[str] = []
    for field in fields:
        raw = row.get(field)
        if isinstance(raw, str):
            values.append(raw)
            continue
        if isinstance(raw, Iterable) and not isinstance(raw, (bytes, str, dict)):
            for item in raw:
                if isinstance(item, str):
                    values.append(item)
                elif isinstance(item, Mapping):
                    item_label = _first_text(item, "name", "line", "label")
                    if isinstance(item_label, str):
                        values.append(item_label)
    return values


def _first_text(row: Mapping[str, Any], *fields: str) -> Any:
    for field in fields:
        if field in row:
            return row.get(field)
    return None


def _extract_path(item: Mapping[str, Any]) -> str | None:
    path = item.get("path")
    return path if isinstance(path, str) else None


def _split_path_text(value: str) -> list[str]:
    return [part.strip() for part in value.split(";") if part.strip()]


def _ensure_str(value: Any) -> str:
    return value if isinstance(value, str) else ""


def _as_number(value: Any) -> float | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int | float):
        return float(value)
    if isinstance(value, str):
        try:
            return float(value)
        except ValueError:
            return None
    return None


def _coerce_bool(value: Any) -> bool | None:
    if isinstance(value, bool):
        return value
    if isinstance(value, int):
        if value in (0, 1):
            return bool(value)
        return None
    if isinstance(value, str):
        normalized = value.strip().lower()
        if normalized in {"1", "true", "yes", "y", "on"}:
            return True
        if normalized in {"0", "false", "no", "n", "off"}:
            return False
    return None


def _normalize_text(value: str | None) -> str | None:
    if not isinstance(value, str):
        return None
    normalized = value.strip().lower()
    return normalized or None
