from __future__ import annotations

import hashlib
import json
from typing import Any

TREE_ENTRY_BLOB = "blob"
TREE_ENTRY_TREE = "tree"
TREE_ENTRY_TREE_MODE = "tree"


def build_tree_records(file_entries: list[dict[str, Any]]) -> tuple[str, list[dict[str, Any]], list[dict[str, Any]]]:
    root: dict[str, Any] = {}
    for entry in sorted(file_entries, key=lambda item: item["path"]):
        path = str(entry["path"]).strip("/")
        if not path:
            continue
        parts = [part for part in path.split("/") if part]
        cursor = root
        for part in parts[:-1]:
            child = cursor.get(part)
            if child is None:
                child = {"type": TREE_ENTRY_TREE, "children": {}}
                cursor[part] = child
            elif child["type"] != TREE_ENTRY_TREE:
                raise ValueError(f"Path collision while building tree metadata at {path!r}")
            cursor = child["children"]
        leaf = parts[-1]
        cursor[leaf] = {
            "type": TREE_ENTRY_BLOB,
            "blob_id": entry["blob_id"],
            "size_bytes": int(entry["size_bytes"]),
            "mode": str(entry["mode"]),
        }

    tree_rows: dict[str, dict[str, Any]] = {}
    tree_entry_rows: dict[tuple[str, str], dict[str, Any]] = {}

    def _materialize(children: dict[str, Any]) -> str:
        serialized_entries: list[dict[str, Any]] = []
        pending_rows: list[dict[str, Any]] = []
        for name in sorted(children):
            node = children[name]
            if node["type"] == TREE_ENTRY_TREE:
                child_tree_id = _materialize(node["children"])
                serialized_entries.append(
                    {
                        "name": name,
                        "type": TREE_ENTRY_TREE,
                        "target_id": child_tree_id,
                    }
                )
                pending_rows.append(
                    {
                        "entry_name": name,
                        "entry_type": TREE_ENTRY_TREE,
                        "target_id": child_tree_id,
                        "size_bytes": None,
                        "mode": TREE_ENTRY_TREE_MODE,
                    }
                )
                continue
            serialized_entries.append(
                {
                    "name": name,
                    "type": TREE_ENTRY_BLOB,
                    "target_id": node["blob_id"],
                    "size_bytes": int(node["size_bytes"]),
                    "mode": str(node["mode"]),
                }
            )
            pending_rows.append(
                {
                    "entry_name": name,
                    "entry_type": TREE_ENTRY_BLOB,
                    "target_id": node["blob_id"],
                    "size_bytes": int(node["size_bytes"]),
                    "mode": str(node["mode"]),
                }
            )
        digest = hashlib.sha256(json.dumps(serialized_entries, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()
        tree_id = f"TRE-{digest[:20].upper()}"
        if tree_id not in tree_rows:
            tree_rows[tree_id] = {
                "tree_id": tree_id,
                "entry_count": len(serialized_entries),
            }
            for row in pending_rows:
                tree_entry_rows[(tree_id, row["entry_name"])] = {"tree_id": tree_id, **row}
        return tree_id

    root_tree_id = _materialize(root)
    return root_tree_id, list(tree_rows.values()), list(tree_entry_rows.values())


def build_snapshot_id(
    *,
    repo_name: str,
    line_name: str,
    parent_snapshot_id: str | None,
    message: str | None,
    root_tree_id: str,
    snapshot_kind: str = "line",
) -> tuple[str, str]:
    payload = {
        "repo_name": repo_name,
        "line_name": line_name,
        "parent_snapshot_id": parent_snapshot_id,
        "message": message,
        "root_tree_id": root_tree_id,
        "snapshot_kind": snapshot_kind,
    }
    revision_hash = hashlib.sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()
    return f"SNP-{revision_hash[:12].upper()}", revision_hash
