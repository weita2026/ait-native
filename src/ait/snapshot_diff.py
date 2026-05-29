from __future__ import annotations

from collections import defaultdict
from collections.abc import Mapping
from dataclasses import dataclass
from difflib import unified_diff
from pathlib import PurePosixPath
from typing import Any

from ait_protocol.common import connect_sqlite

from . import local_content, local_content_snapshots


DEFAULT_SNAPSHOT_DIFF_MAX_BYTES = 128_000

__all__ = [
    "DEFAULT_SNAPSHOT_DIFF_MAX_BYTES",
    "diff_snapshot_file_maps",
    "snapshot_diff",
]


def _to_mode_int(value: str | int | None) -> int:
    if value is None:
        return 0
    if isinstance(value, int):
        return value
    text = str(value).strip()
    if not text:
        return 0
    try:
        return int(text, 0)
    except ValueError:
        return int(text, 8)


def _coerce_file_map(snapshot_files: Mapping[str, Any] | list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    if isinstance(snapshot_files, Mapping):
        if any(not isinstance(v, Mapping) for v in snapshot_files.values()):
            raise TypeError("snapshot file map mapping values must be row dictionaries")
        out: dict[str, dict[str, Any]] = {}
        for path, row in snapshot_files.items():
            if not isinstance(path, str):
                raise TypeError("snapshot file map keys must be string paths")
            normalized = dict(row)
            normalized.setdefault("path", path)
            out[path] = normalized
        return out
    if not isinstance(snapshot_files, list):
        raise TypeError("snapshot files must be a dict[path, row] map or list of rows")
    out = {}
    for row in snapshot_files:
        if not isinstance(row, Mapping):
            raise TypeError("snapshot file rows must be mapping objects")
        path = row.get("path")
        if not isinstance(path, str):
            raise TypeError("snapshot file row is missing string path")
        out[path] = dict(row)
    return out


@dataclass(frozen=True)
class _TextDiff:
    status: str
    insertions: int = 0
    deletions: int = 0
    text: str | None = None


def _safe_decode_text(data: bytes, *, max_bytes: int) -> tuple[bool, str | None, str | None]:
    if len(data) > max_bytes:
        return False, None, "too_large"
    if b"\x00" in data:
        return False, None, "binary"
    try:
        return True, data.decode("utf-8"), None
    except UnicodeDecodeError:
        return False, None, "binary"


def _build_text_diff(
    *,
    path: str,
    old_text: str,
    new_text: str,
    old_snapshot_id: str | None,
    new_snapshot_id: str | None,
) -> _TextDiff:
    diff_lines = list(
        unified_diff(
            old_text.splitlines(keepends=True),
            new_text.splitlines(keepends=True),
            fromfile=f"{old_snapshot_id or '?'}:{path}",
            tofile=f"{new_snapshot_id or '?'}:{path}",
            lineterm="",
        )
    )
    insertions = 0
    deletions = 0
    for line in diff_lines:
        if line.startswith("+++") or line.startswith("---") or line.startswith("@@"):
            continue
        if line.startswith("+"):
            insertions += 1
        elif line.startswith("-"):
            deletions += 1
    return _TextDiff(
        status="text",
        insertions=insertions,
        deletions=deletions,
        text="\n".join(diff_lines),
    )


def _file_row_payload(row: dict[str, Any] | None) -> dict[str, Any] | None:
    if row is None:
        return {
            "blob_id": None,
            "size_bytes": None,
            "mode": None,
        }
    size = row.get("size_bytes")
    return {
        "blob_id": row.get("blob_id"),
        "size_bytes": int(size) if size is not None else None,
        "mode": row.get("mode"),
    }


def _parent_path(path: str) -> str:
    parent = PurePosixPath(path).parent.as_posix()
    return "." if parent in ("", ".") else parent


def _build_rename_hints(
    *,
    old_map: dict[str, dict[str, Any]],
    new_map: dict[str, dict[str, Any]],
    added: list[str],
    deleted: list[str],
) -> list[dict[str, Any]]:
    deleted_by_blob_id: dict[str, list[str]] = defaultdict(list)
    added_by_blob_id: dict[str, list[str]] = defaultdict(list)

    for path in deleted:
        blob_id = str(old_map[path].get("blob_id") or "").strip()
        if blob_id:
            deleted_by_blob_id[blob_id].append(path)

    for path in added:
        blob_id = str(new_map[path].get("blob_id") or "").strip()
        if blob_id:
            added_by_blob_id[blob_id].append(path)

    rename_hints: list[dict[str, Any]] = []
    for blob_id in sorted(set(deleted_by_blob_id) & set(added_by_blob_id)):
        old_paths = sorted(deleted_by_blob_id[blob_id])
        new_paths = sorted(added_by_blob_id[blob_id])
        if len(old_paths) != 1 or len(new_paths) != 1:
            continue
        old_path = old_paths[0]
        new_path = new_paths[0]
        old_row = old_map[old_path]
        new_row = new_map[new_path]
        rename_hints.append(
            {
                "match_kind": "exact_blob_id",
                "blob_id": blob_id,
                "old_path": old_path,
                "new_path": new_path,
                "old_parent_path": _parent_path(old_path),
                "new_parent_path": _parent_path(new_path),
                "size_bytes": int(new_row.get("size_bytes") or old_row.get("size_bytes") or 0),
            }
        )

    return rename_hints


def _build_directory_move_hints(rename_hints: list[dict[str, Any]]) -> list[dict[str, Any]]:
    old_to_new: dict[str, set[str]] = defaultdict(set)
    new_to_old: dict[str, set[str]] = defaultdict(set)
    pairs: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)

    for hint in rename_hints:
        old_parent = str(hint.get("old_parent_path") or "").strip()
        new_parent = str(hint.get("new_parent_path") or "").strip()
        if not old_parent or not new_parent or old_parent == new_parent:
            continue
        old_to_new[old_parent].add(new_parent)
        new_to_old[new_parent].add(old_parent)
        pairs[(old_parent, new_parent)].append(hint)

    directory_move_hints: list[dict[str, Any]] = []
    for (old_parent, new_parent), matched_hints in sorted(pairs.items()):
        if len(matched_hints) < 2:
            continue
        if len(old_to_new[old_parent]) != 1 or len(new_to_old[new_parent]) != 1:
            continue
        directory_move_hints.append(
            {
                "match_kind": "exact_blob_id_group",
                "old_parent_path": old_parent,
                "new_parent_path": new_parent,
                "rename_count": len(matched_hints),
                "renames": [
                    {
                        "old_path": str(hint["old_path"]),
                        "new_path": str(hint["new_path"]),
                        "blob_id": str(hint["blob_id"]),
                    }
                    for hint in sorted(matched_hints, key=lambda row: (str(row["old_path"]), str(row["new_path"])))
                ],
            }
        )

    return directory_move_hints


def _maybe_add_text_diff(
    conn,
    ctx,
    *,
    path: str,
    old_row: dict[str, Any] | None,
    new_row: dict[str, Any] | None,
    old_snapshot_id: str | None,
    new_snapshot_id: str | None,
    max_bytes: int,
) -> _TextDiff:
    old_blob_id = old_row.get("blob_id") if old_row else None
    new_blob_id = new_row.get("blob_id") if new_row else None
    if old_blob_id is None or new_blob_id is None:
        return _TextDiff(status="unavailable")
    try:
        old_data = local_content._blob_bytes_by_id(ctx, conn, old_blob_id)
        new_data = local_content._blob_bytes_by_id(ctx, conn, new_blob_id)
    except Exception:
        return _TextDiff(status="unavailable")
    old_is_text, old_text, old_reason = _safe_decode_text(old_data, max_bytes=max_bytes)
    if not old_is_text:
        return _TextDiff(status=old_reason or "binary")
    new_is_text, new_text, new_reason = _safe_decode_text(new_data, max_bytes=max_bytes)
    if not new_is_text:
        return _TextDiff(status=new_reason or "binary")
    return _build_text_diff(
        path=path,
        old_text=old_text,
        new_text=new_text,
        old_snapshot_id=old_snapshot_id,
        new_snapshot_id=new_snapshot_id,
    )


def diff_snapshot_file_maps(
    old_files: Mapping[str, Any] | list[dict[str, Any]],
    new_files: Mapping[str, Any] | list[dict[str, Any]],
    *,
    old_snapshot_id: str | None = None,
    new_snapshot_id: str | None = None,
) -> dict[str, Any]:
    old_map = _coerce_file_map(old_files)
    new_map = _coerce_file_map(new_files)
    old_keys = set(old_map.keys())
    new_keys = set(new_map.keys())

    added = sorted(new_keys - old_keys)
    deleted = sorted(old_keys - new_keys)
    modified: list[str] = []
    mode_changed: list[str] = []
    file_entries: list[dict[str, Any]] = []

    for path in sorted(added):
        new_row = new_map[path]
        file_entries.append(
            {
                "path": path,
                "status": "added",
                "old": _file_row_payload(None),
                "new": _file_row_payload(new_row),
            }
        )

    for path in sorted(deleted):
        old_row = old_map[path]
        file_entries.append(
            {
                "path": path,
                "status": "deleted",
                "old": _file_row_payload(old_row),
                "new": _file_row_payload(None),
            }
        )

    for path in sorted(old_keys & new_keys):
        old_row = old_map[path]
        new_row = new_map[path]
        old_mode = _to_mode_int(old_row.get("mode"))
        new_mode = _to_mode_int(new_row.get("mode"))
        mode_only = old_mode != new_mode and old_row.get("blob_id") == new_row.get("blob_id")
        content_changed = (
            old_row.get("blob_id") != new_row.get("blob_id")
            or old_row.get("size_bytes") != new_row.get("size_bytes")
        )

        if mode_only:
            mode_changed.append(path)
            status = "mode_changed"
        elif content_changed:
            modified.append(path)
            status = "modified"
        else:
            continue

        file_entries.append(
            {
                "path": path,
                "status": status,
                "old": _file_row_payload(old_row),
                "new": _file_row_payload(new_row),
            }
        )

    files_changed = len(added) + len(deleted) + len(modified) + len(mode_changed)
    rename_hints = _build_rename_hints(
        old_map=old_map,
        new_map=new_map,
        added=added,
        deleted=deleted,
    )
    directory_move_hints = _build_directory_move_hints(rename_hints)
    return {
        "old_snapshot_id": old_snapshot_id,
        "new_snapshot_id": new_snapshot_id,
        "added": added,
        "deleted": deleted,
        "modified": modified,
        "mode_changed": mode_changed,
        "rename_hints": rename_hints,
        "directory_move_hints": directory_move_hints,
        "files": file_entries,
        "summary": {
            "files_changed": files_changed,
            "insertions": 0,
            "deletions": 0,
        },
    }


def snapshot_diff(
    ctx,
    old_snapshot_id: str,
    new_snapshot_id: str,
    *,
    include_text: bool = False,
    max_bytes: int = DEFAULT_SNAPSHOT_DIFF_MAX_BYTES,
) -> dict[str, Any]:
    conn = connect_sqlite(ctx.content_db_path)
    try:
        old_files = local_content_snapshots._snapshot_file_map(conn, old_snapshot_id) if old_snapshot_id else {}
        new_files = local_content_snapshots._snapshot_file_map(conn, new_snapshot_id) if new_snapshot_id else {}
    finally:
        conn.close()

    result = diff_snapshot_file_maps(
        old_files,
        new_files,
        old_snapshot_id=old_snapshot_id,
        new_snapshot_id=new_snapshot_id,
    )
    result["summary"]["old_snapshot_id"] = old_snapshot_id
    result["summary"]["new_snapshot_id"] = new_snapshot_id

    if not include_text:
        return result

    conn = connect_sqlite(ctx.content_db_path)
    try:
        total_insertions = 0
        total_deletions = 0
        for file_row in result["files"]:
            if file_row["status"] != "modified":
                file_row["diff"] = {
                    "status": "metadata_only",
                    "insertions": 0,
                    "deletions": 0,
                    "text": None,
                }
                continue
            old_row = file_row["old"]
            text_diff = _maybe_add_text_diff(
                conn,
                ctx=ctx,
                path=file_row["path"],
                old_row=old_row and {
                    "blob_id": old_row["blob_id"],
                    "size_bytes": old_row["size_bytes"],
                    "mode": old_row["mode"],
                },
                new_row=file_row["new"] and {
                    "blob_id": file_row["new"]["blob_id"],
                    "size_bytes": file_row["new"]["size_bytes"],
                    "mode": file_row["new"]["mode"],
                },
                old_snapshot_id=old_snapshot_id,
                new_snapshot_id=new_snapshot_id,
                max_bytes=max_bytes,
            )
            file_row["diff"] = {
                "status": text_diff.status,
                "insertions": text_diff.insertions,
                "deletions": text_diff.deletions,
                "text": text_diff.text,
            }
            total_insertions += text_diff.insertions
            total_deletions += text_diff.deletions
    finally:
        conn.close()

    result["summary"]["insertions"] = total_insertions
    result["summary"]["deletions"] = total_deletions
    return result
