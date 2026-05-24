from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any, Iterable
import zipfile

TREE_PACK_FORMAT_V1 = "ait-tree-pack-v1"
TREE_PACK_INDEX_ENTRY_NAME = "tree-pack-index.json"


def tree_pack_manifest_path(pack_path: str, entry_name: str) -> str:
    return f"{pack_path}#{entry_name}"


def _tree_payload_bytes(tree_id: str, tree_entry_rows: Iterable[dict[str, Any]]) -> bytes:
    payload = [
        {
            "entry_name": str(row["entry_name"]),
            "entry_type": str(row["entry_type"]),
            "target_id": str(row["target_id"]),
            "size_bytes": row["size_bytes"],
            "mode": str(row["mode"]),
        }
        for row in sorted(tree_entry_rows, key=lambda item: str(item["entry_name"]))
    ]
    return json.dumps({"tree_id": tree_id, "entries": payload}, sort_keys=True, separators=(",", ":")).encode("utf-8")


def build_tree_pack_members(tree_rows: Iterable[dict[str, Any]], tree_entry_rows: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    entry_rows_by_tree: dict[str, list[dict[str, Any]]] = {}
    for row in tree_entry_rows:
        entry_rows_by_tree.setdefault(str(row["tree_id"]), []).append(dict(row))

    members: list[dict[str, Any]] = []
    for tree_row in sorted(tree_rows, key=lambda item: str(item["tree_id"])):
        tree_id = str(tree_row["tree_id"])
        rows = entry_rows_by_tree.get(tree_id, [])
        data = _tree_payload_bytes(tree_id, rows)
        members.append(
            {
                "tree_id": tree_id,
                "entry_name": f"trees/{tree_id}.json",
                "entry_count": int(tree_row["entry_count"]),
                "data": data,
                "checksum": hashlib.sha256(data).hexdigest(),
            }
        )
    return members


def build_tree_pack_index(pack_id: str, created_at: str, members: Iterable[dict[str, Any]]) -> dict[str, Any]:
    prepared_members = list(members)
    return {
        "pack_format": TREE_PACK_FORMAT_V1,
        "pack_id": pack_id,
        "created_at": created_at,
        "index_entry_name": TREE_PACK_INDEX_ENTRY_NAME,
        "tree_count": len(prepared_members),
        "total_bytes": sum(len(member["data"]) for member in prepared_members),
        "trees": [
            {
                "tree_id": member["tree_id"],
                "entry_name": member["entry_name"],
                "entry_count": int(member["entry_count"]),
                "byte_length": len(member["data"]),
                "checksum": member["checksum"],
            }
            for member in prepared_members
        ],
    }


def write_tree_pack_archive(pack_path: Path, pack_id: str, created_at: str, members: Iterable[dict[str, Any]]) -> dict[str, Any]:
    pack_path.parent.mkdir(parents=True, exist_ok=True)
    prepared_members = list(members)
    pack_index = build_tree_pack_index(pack_id, created_at, prepared_members)
    index_bytes = json.dumps(pack_index, indent=2, sort_keys=True).encode("utf-8")
    index_checksum = hashlib.sha256(index_bytes).hexdigest()
    with zipfile.ZipFile(pack_path, mode="w", compression=zipfile.ZIP_DEFLATED) as zf:
        for member in prepared_members:
            zf.writestr(member["entry_name"], member["data"])
        zf.writestr(TREE_PACK_INDEX_ENTRY_NAME, index_bytes)
    return {
        "tree_count": pack_index["tree_count"],
        "total_bytes": pack_index["total_bytes"],
        "archive_bytes": pack_path.stat().st_size,
        "pack_format": TREE_PACK_FORMAT_V1,
        "pack_index_entry_name": TREE_PACK_INDEX_ENTRY_NAME,
        "pack_index_checksum": index_checksum,
        "pack_index": pack_index,
    }


def _tree_entries_by_id(pack_index: dict[str, Any]) -> dict[str, dict[str, Any]]:
    trees = pack_index.get("trees")
    if not isinstance(trees, list):
        raise ValueError("Invalid tree pack index: missing trees list")
    out: dict[str, dict[str, Any]] = {}
    for entry in trees:
        if not isinstance(entry, dict):
            raise ValueError("Invalid tree pack index: malformed tree entry")
        tree_id = entry.get("tree_id")
        entry_name = entry.get("entry_name")
        if not isinstance(tree_id, str) or not tree_id:
            raise ValueError("Invalid tree pack index: tree entry missing tree_id")
        if not isinstance(entry_name, str) or not entry_name:
            raise ValueError("Invalid tree pack index: tree entry missing entry_name")
        if tree_id in out:
            raise ValueError(f"Invalid tree pack index: duplicate tree_id {tree_id}")
        out[tree_id] = entry
    return out


def read_tree_pack_index(pack_path: Path) -> dict[str, Any]:
    with zipfile.ZipFile(pack_path, mode="r") as zf:
        if TREE_PACK_INDEX_ENTRY_NAME not in zf.namelist():
            raise ValueError(f"Invalid tree pack archive: missing {TREE_PACK_INDEX_ENTRY_NAME}")
        raw = zf.read(TREE_PACK_INDEX_ENTRY_NAME)
    pack_index = json.loads(raw.decode("utf-8"))
    if pack_index.get("pack_format") != TREE_PACK_FORMAT_V1:
        raise ValueError(f"Invalid tree pack index: unsupported pack_format {pack_index.get('pack_format')!r}")
    if pack_index.get("index_entry_name") != TREE_PACK_INDEX_ENTRY_NAME:
        raise ValueError("Invalid tree pack index: incorrect index_entry_name")
    _tree_entries_by_id(pack_index)
    return pack_index


def read_tree_pack_tree(pack_path: Path, tree_id: str) -> list[dict[str, Any]]:
    pack_index = read_tree_pack_index(pack_path)
    tree_entries = _tree_entries_by_id(pack_index)
    entry = tree_entries.get(tree_id)
    if entry is None:
        raise KeyError(tree_id)
    with zipfile.ZipFile(pack_path, mode="r") as zf:
        raw = zf.read(entry["entry_name"])
    checksum = hashlib.sha256(raw).hexdigest()
    if checksum != entry.get("checksum"):
        raise ValueError(f"Tree pack entry checksum mismatch for {tree_id}")
    payload = json.loads(raw.decode("utf-8"))
    if payload.get("tree_id") != tree_id:
        raise ValueError(f"Tree pack entry tree_id mismatch for {tree_id}")
    rows = payload.get("entries")
    if not isinstance(rows, list):
        raise ValueError(f"Tree pack entry payload missing entries for {tree_id}")
    if len(rows) != int(entry.get("entry_count", len(rows))):
        raise ValueError(f"Tree pack entry count mismatch for {tree_id}")
    out: list[dict[str, Any]] = []
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError(f"Tree pack entry row malformed for {tree_id}")
        out.append(
            {
                "tree_id": tree_id,
                "entry_name": row["entry_name"],
                "entry_type": row["entry_type"],
                "target_id": row["target_id"],
                "size_bytes": row.get("size_bytes"),
                "mode": row["mode"],
            }
        )
    return out


def summarize_tree_pack_archives(root: Path, pack_rows: Iterable[dict[str, Any]]) -> dict[str, int]:
    archive_bytes = 0
    indexed_tree_count = 0
    indexed_entry_count = 0
    index_error_count = 0
    pack_count = 0
    for row in pack_rows:
        pack_count += 1
        pack_path = row.get("pack_path")
        if not isinstance(pack_path, str) or not pack_path:
            index_error_count += 1
            continue
        pack_abs = root / pack_path
        if not pack_abs.exists():
            index_error_count += 1
            continue
        archive_bytes += pack_abs.stat().st_size
        try:
            pack_index = read_tree_pack_index(pack_abs)
        except Exception:
            index_error_count += 1
            continue
        indexed_tree_count += int(pack_index.get("tree_count", 0) or 0)
        indexed_entry_count += sum(int(entry.get("entry_count", 0) or 0) for entry in pack_index.get("trees", []))
    return {
        "pack_count": pack_count,
        "archive_bytes": archive_bytes,
        "indexed_tree_count": indexed_tree_count,
        "indexed_entry_count": indexed_entry_count,
        "index_error_count": index_error_count,
    }
