from __future__ import annotations

import base64
import hashlib
from typing import Any

from ait_protocol.common import connect_sqlite, utc_now
from ait_storage.revision_trees import build_snapshot_id, build_tree_records

from .local_content_pack_runtime import (
    _blob_bytes_by_row,
    _blob_path,
    _blob_row,
    _insert_blob_record,
    _insert_tree_records,
    _manifest_path_for_tree,
    _parent_delta_candidates,
    _snapshot_row,
    _write_packed_blobs,
    _write_tree_pack,
)
from .repo_paths import RepoContext

def export_snapshot_bundle(ctx: RepoContext, snapshot_id: str, repo_name: str) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    snap = _snapshot_row(conn, snapshot_id)
    if snap is None:
        conn.close()
        raise KeyError(f"Unknown snapshot: {snapshot_id}")
    file_rows = conn.execute(
        """
        select sf.path, sf.blob_id, coalesce(sf.size_bytes, b.size_bytes) as size_bytes, sf.mode,
               b.sha256, b.storage_path, b.pack_id
        from snapshot_files sf
        join blobs b on b.blob_id = sf.blob_id
        where sf.snapshot_id = ?
        order by sf.path
        """,
        (snapshot_id,),
    ).fetchall()

    files = []
    for row in file_rows:
        data = _blob_bytes_by_row(ctx, conn, row)
        files.append(
            {
                "path": row["path"],
                "blob_id": row["blob_id"],
                "size_bytes": row["size_bytes"],
                "mode": row["mode"],
                "sha256": row["sha256"],
                "content_b64": base64.b64encode(data).decode("ascii"),
            }
        )
    conn.close()

    return {
        "snapshot_id": snap["snapshot_id"],
        "repo_name": repo_name,
        "parent_snapshot_id": snap["parent_snapshot_id"],
        "root_tree_id": snap["root_tree_id"],
        "manifest_hash": snap["manifest_hash"],
        "manifest_path": snap["manifest_path"],
        "message": snap["message"],
        "line_name": snap["line_name"],
        "file_count": snap["file_count"],
        "total_bytes": snap["total_bytes"],
        "created_at": snap["created_at"],
        "files": files,
    }



def import_snapshot_bundle(ctx: RepoContext, bundle: dict) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    snapshot_id = bundle["snapshot_id"]
    if _snapshot_row(conn, snapshot_id) is not None:
        row = dict(_snapshot_row(conn, snapshot_id))
        conn.close()
        return row

    file_rows: list[dict[str, Any]] = []
    new_blob_rows: list[dict[str, Any]] = []
    total_bytes = 0
    created_at = bundle.get("created_at") or utc_now()
    for file_entry in bundle["files"]:
        data = base64.b64decode(file_entry["content_b64"])
        digest = hashlib.sha256(data).hexdigest()
        if digest != file_entry["sha256"]:
            conn.close()
            raise ValueError(f"Snapshot blob digest mismatch for {file_entry['path']}")
        blob_id = file_entry["blob_id"]
        size = len(data)
        total_bytes += size
        blob_storage = _blob_path(ctx, blob_id)
        existing = _blob_row(conn, blob_id)
        if existing is None:
            new_blob_rows.append(
                {
                    "blob_id": blob_id,
                    "sha256": digest,
                    "storage_path": str(blob_storage.relative_to(ctx.root)),
                    "size_bytes": size,
                    "data": data,
                    "entry_name": f"blobs/{blob_id}",
                    "path_hint": file_entry["path"],
                }
            )
        file_rows.append(
            {
                "path": file_entry["path"],
                "blob_id": blob_id,
                "size_bytes": file_entry["size_bytes"] if file_entry.get("size_bytes") is not None else size,
                "mode": file_entry["mode"],
                "sha256": file_entry["sha256"],
            }
        )

    if new_blob_rows:
        initial_by_path = _parent_delta_candidates(
            ctx,
            conn,
            bundle.get("parent_snapshot_id"),
            {str(row.get("path_hint") or "") for row in new_blob_rows if row.get("path_hint")},
        )
        pack_id, members_by_blob_id = _write_packed_blobs(
            ctx,
            conn,
            snapshot_id,
            created_at,
            new_blob_rows,
            initial_by_path=initial_by_path,
        )
        for row in new_blob_rows:
            member = members_by_blob_id[row["blob_id"]]
            entry_type = member.get("entry_type", "full")
            _insert_blob_record(
                conn,
                blob_id=row["blob_id"],
                sha256=row["sha256"],
                storage_path=row["storage_path"],
                size_bytes=row["size_bytes"],
                pack_id=pack_id,
                pack_entry_type=entry_type,
                pack_base_blob_id=member.get("base_blob_id"),
                pack_chain_depth=int(member.get("chain_depth", 0) or 0),
                pruned_at=None,
                created_at=created_at,
                pack_entry_name=None,
                packed_at=None,
            )

    root_tree_id, tree_rows, tree_entry_rows = build_tree_records(file_rows)
    bundle_root_tree_id = bundle.get("root_tree_id")
    if bundle_root_tree_id and bundle_root_tree_id != root_tree_id:
        conn.close()
        raise ValueError(
            f"Snapshot tree metadata mismatch for {snapshot_id}: "
            f"bundle={bundle_root_tree_id!r}, computed={root_tree_id!r}"
        )
    computed_snapshot_id, revision_hash = build_snapshot_id(
        repo_name=bundle.get("repo_name") or "",
        line_name=bundle.get("line_name") or "main",
        parent_snapshot_id=bundle.get("parent_snapshot_id"),
        message=bundle.get("message"),
        root_tree_id=root_tree_id,
    )
    manifest_hash = revision_hash if snapshot_id == computed_snapshot_id else (bundle.get("manifest_hash") or revision_hash)
    _insert_tree_records(conn, tree_rows, tree_entry_rows, created_at)
    _write_tree_pack(
        ctx,
        conn,
        [str(row["tree_id"]) for row in tree_rows],
        created_at=created_at,
        seed_hint=f"{snapshot_id}|{root_tree_id}",
    )
    manifest_path = _manifest_path_for_tree(conn, root_tree_id)

    conn.execute(
        """
        insert into snapshots(
            snapshot_id, parent_snapshot_id, root_tree_id, manifest_hash, manifest_path,
            message, line_name, file_count, total_bytes, created_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            snapshot_id,
            bundle.get("parent_snapshot_id"),
            root_tree_id,
            manifest_hash,
            manifest_path,
            bundle.get("message"),
            bundle.get("line_name") or "main",
            bundle.get("file_count") or len(bundle["files"]),
            bundle.get("total_bytes") or total_bytes,
            created_at,
        ),
    )
    conn.commit()
    row = conn.execute("select * from snapshots where snapshot_id = ?", (snapshot_id,)).fetchone()
    conn.close()
    return dict(row)



def collect_snapshot_chain(ctx: RepoContext, snapshot_id: str) -> list[str]:
    conn = connect_sqlite(ctx.content_db_path)
    seen: set[str] = set()
    ordered: list[str] = []
    cur = snapshot_id
    while cur:
        if cur in seen:
            conn.close()
            raise ValueError(f"Cycle detected in snapshot chain at {cur}")
        row = _snapshot_row(conn, cur)
        if row is None:
            conn.close()
            raise KeyError(f"Unknown snapshot: {cur}")
        seen.add(cur)
        ordered.append(cur)
        cur = row["parent_snapshot_id"]
    conn.close()
    ordered.reverse()
    return ordered



def ensure_snapshot_chain(ctx: RepoContext, bundles: list[dict]) -> list[dict]:
    imported = []
    for bundle in bundles:
        imported.append(import_snapshot_bundle(ctx, bundle))
    return imported
