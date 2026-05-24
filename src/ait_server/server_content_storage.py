from __future__ import annotations

import hashlib
import json
import uuid
from pathlib import Path
from typing import Any

from ait_protocol.common import utc_now
from ait_storage.packfiles import (
    build_pack_members,
    build_storage_validation_summary,
    read_pack_index,
    summarize_pack_archives,
    write_pack_archive,
)
from ait_storage.treepacks import (
    read_tree_pack_index,
    read_tree_pack_tree,
    summarize_tree_pack_archives,
)

from .server_paths import ServerContext


def _server_content_module():
    from . import server_content as _server_content

    return _server_content


def snapshot_manifest_map(ctx: ServerContext, snapshot_id: str) -> dict[str, dict[str, Any]]:
    with _server_content_module()._connect(ctx) as conn:
        _server_content_module()._ensure_schema(conn, ctx)
        rows = conn.execute(
            """
            select sf.path, sf.blob_id, sf.size_bytes, sf.mode, b.sha256
            from snapshot_files sf
            join blobs b on b.blob_id = sf.blob_id
            where sf.snapshot_id = ?
            order by sf.path
            """,
            (snapshot_id,),
        ).fetchall()
    return {
        row["path"]: {
            "blob_id": row["blob_id"],
            "size_bytes": row["size_bytes"],
            "mode": row["mode"],
            "sha256": row["sha256"],
        }
        for row in rows
    }



def repository_storage_stats(ctx: ServerContext, repo_name: str) -> dict[str, Any]:
    with _server_content_module()._connect(ctx) as conn:
        _server_content_module()._ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = _server_content_module()._repository_scope_params(conn, repo_name)
        referenced = conn.execute(
            """
            select count(*) as c,
                   coalesce(sum(size_bytes), 0) as approx_bytes
            from (
                select distinct b.blob_id, b.size_bytes
                from blobs b
                join snapshot_files sf on sf.blob_id = b.blob_id
                join snapshots s on s.snapshot_id = sf.snapshot_id
                where """
            + _server_content_module()._repository_id_scope_predicate("s")
            + """
            ) referenced_blobs
            """,
            (repo_id, scoped_repo_name),
        ).fetchone()
        counts = conn.execute(
            """
            select
                count(distinct case when b.pack_id is not null then b.blob_id end) as packed_blob_count,
                count(distinct case when b.pack_id is not null and coalesce(b.pack_entry_type, 'full') = 'full' then b.blob_id end) as packed_full_blob_count,
                count(distinct case when b.pack_id is not null and coalesce(b.pack_entry_type, 'full') = 'delta' then b.blob_id end) as packed_delta_blob_count,
                coalesce(sum(case when b.pack_id is not null then b.size_bytes else 0 end), 0) as packed_blob_bytes,
                coalesce(sum(case when b.pack_id is not null and coalesce(b.pack_entry_type, 'full') = 'full' then b.size_bytes else 0 end), 0) as packed_full_blob_bytes,
                coalesce(sum(case when b.pack_id is not null and coalesce(b.pack_entry_type, 'full') = 'delta' then b.size_bytes else 0 end), 0) as packed_delta_blob_bytes
                from blobs b
                join snapshot_files sf on sf.blob_id = b.blob_id
                join snapshots s on s.snapshot_id = sf.snapshot_id
                where """
            + _server_content_module()._repository_id_scope_predicate("s") + """
                """,
            (repo_id, scoped_repo_name),
        ).fetchone()
        snapshot_count = conn.execute(
            "select count(*) as c from snapshots where " + _server_content_module()._repository_id_scope_predicate(),
            (repo_id, scoped_repo_name),
        ).fetchone()["c"]
        repo_tree_stats = _server_content_module()._tree_metadata_stats(conn, repo_name=repo_name, repo_id=repo_id)
        global_tree_stats = _server_content_module()._tree_metadata_stats(conn)
        pack_rows = [
            dict(r)
            for r in conn.execute(
                "select * from packs where " + _server_content_module()._repository_id_scope_predicate() + " order by created_at desc",
                (repo_id, scoped_repo_name),
            )
        ]
        tree_pack_rows = [dict(r) for r in conn.execute("select * from tree_packs order by created_at desc")]
        global_unreferenced = conn.execute(
            "select count(*) as c from blobs b where not exists (select 1 from snapshot_files sf where sf.blob_id = b.blob_id)"
        ).fetchone()["c"]
    pack_summary = summarize_pack_archives(ctx.root, pack_rows)
    tree_pack_summary = summarize_tree_pack_archives(ctx.root, tree_pack_rows)
    logical_tracked_blob_bytes = int(counts["packed_blob_bytes"])
    physical_storage_bytes = int(pack_summary["pack_archive_bytes"])
    storage_savings_bytes = logical_tracked_blob_bytes - physical_storage_bytes
    delta_pre_archive_savings_bytes = int(pack_summary["pack_delta_logical_bytes"]) - int(pack_summary["pack_delta_member_bytes"])
    tracked_blob_count = int(counts["packed_blob_count"])
    optimization_summary = {
        "tracked_blob_count": tracked_blob_count,
        "storage_kind_counts": {
            "pack_full": int(counts["packed_full_blob_count"]),
            "pack_delta": int(counts["packed_delta_blob_count"]),
        },
        "packed_blob_ratio": round((int(counts["packed_blob_count"]) / tracked_blob_count), 4) if tracked_blob_count else 0.0,
        "packed_delta_ratio": round((int(counts["packed_delta_blob_count"]) / tracked_blob_count), 4) if tracked_blob_count else 0.0,
        "delta_within_packed_ratio": round((int(counts["packed_delta_blob_count"]) / int(counts["packed_blob_count"])), 4)
        if int(counts["packed_blob_count"])
        else 0.0,
    }
    efficiency_summary = {
        "logical_tracked_blob_bytes": logical_tracked_blob_bytes,
        "physical_storage_bytes": physical_storage_bytes,
        "storage_savings_bytes": storage_savings_bytes,
        "storage_savings_ratio": round((storage_savings_bytes / logical_tracked_blob_bytes), 4) if logical_tracked_blob_bytes else 0.0,
        "pack_archive_bytes": int(pack_summary["pack_archive_bytes"]),
        "pack_member_bytes": int(pack_summary["pack_member_bytes"]),
        "pack_full_member_bytes": int(pack_summary["pack_full_member_bytes"]),
        "pack_delta_member_bytes": int(pack_summary["pack_delta_member_bytes"]),
        "pack_member_logical_bytes": int(pack_summary["pack_member_logical_bytes"]),
        "pack_delta_logical_bytes": int(pack_summary["pack_delta_logical_bytes"]),
        "delta_pre_archive_savings_bytes": delta_pre_archive_savings_bytes,
        "delta_pre_archive_savings_ratio": round((delta_pre_archive_savings_bytes / int(pack_summary["pack_delta_logical_bytes"])), 4)
        if int(pack_summary["pack_delta_logical_bytes"])
        else 0.0,
        "archive_compression_savings_bytes": int(pack_summary["pack_member_bytes"]) - int(pack_summary["pack_archive_bytes"]),
        "archive_compression_savings_ratio": round(
            ((int(pack_summary["pack_member_bytes"]) - int(pack_summary["pack_archive_bytes"])) / int(pack_summary["pack_member_bytes"])),
            4,
        )
        if int(pack_summary["pack_member_bytes"])
        else 0.0,
        "indexed_pack_count": int(pack_summary["indexed_pack_count"]),
        "pack_indexed_blob_count": int(pack_summary["pack_indexed_blob_count"]),
        "pack_index_error_count": int(pack_summary["index_error_count"]),
    }
    signals = repository_storage_signals(ctx, repo_name)
    validation_summary = build_storage_validation_summary(
        packed_blob_count=int(counts["packed_blob_count"]),
        packed_full_blob_count=int(counts["packed_full_blob_count"]),
        packed_delta_blob_count=int(counts["packed_delta_blob_count"]),
        pack_count=len(pack_rows),
        pack_index_error_count=int(pack_summary["index_error_count"]),
        tree_pack_index_error_count=int(tree_pack_summary["index_error_count"]),
        storage_savings_ratio=float(efficiency_summary["storage_savings_ratio"]),
        unreferenced_blob_count=int(global_unreferenced),
        unreferenced_tree_count=int(global_tree_stats["unreachable_tree_count"]) + int(global_tree_stats["orphan_tree_pack_count"]),
        signals_summary=signals["summary"],
    )
    return {
        "repo_name": repo_name,
        "snapshot_count": snapshot_count,
        "reachable_blob_count": referenced["c"],
        "reachable_tree_count": repo_tree_stats["reachable_tree_count"],
        "reachable_tree_entry_count": repo_tree_stats["reachable_tree_entry_count"],
        "reachable_tree_pack_count": repo_tree_stats["reachable_tree_pack_count"],
        "global_tree_count": global_tree_stats["tree_count"],
        "global_tree_entry_count": global_tree_stats["tree_entry_count"],
        "global_tree_pack_count": global_tree_stats["tree_pack_count"],
        "global_unreachable_tree_count": global_tree_stats["unreachable_tree_count"],
        "global_unreachable_tree_entry_count": global_tree_stats["unreachable_tree_entry_count"],
        "global_orphan_tree_pack_count": global_tree_stats["orphan_tree_pack_count"],
        "global_unreferenced_blob_count": global_unreferenced,
        "packed_blob_count": counts["packed_blob_count"],
        "packed_full_blob_count": counts["packed_full_blob_count"],
        "packed_delta_blob_count": counts["packed_delta_blob_count"],
        "packed_blob_bytes": counts["packed_blob_bytes"],
        "packed_full_blob_bytes": counts["packed_full_blob_bytes"],
        "packed_delta_blob_bytes": counts["packed_delta_blob_bytes"],
        "pack_count": len(pack_rows),
        "pack_archive_bytes": pack_summary["pack_archive_bytes"],
        "tree_pack_archive_bytes": tree_pack_summary["archive_bytes"],
        "optimization_summary": optimization_summary,
        "efficiency_summary": efficiency_summary,
        "metadata_summary": {
            "reachable_tree_count": repo_tree_stats["reachable_tree_count"],
            "reachable_tree_entry_count": repo_tree_stats["reachable_tree_entry_count"],
            "reachable_tree_pack_count": repo_tree_stats["reachable_tree_pack_count"],
            "global_tree_count": global_tree_stats["tree_count"],
            "global_tree_entry_count": global_tree_stats["tree_entry_count"],
            "global_tree_pack_count": global_tree_stats["tree_pack_count"],
            "global_unreachable_tree_count": global_tree_stats["unreachable_tree_count"],
            "global_unreachable_tree_entry_count": global_tree_stats["unreachable_tree_entry_count"],
            "global_orphan_tree_pack_count": global_tree_stats["orphan_tree_pack_count"],
            "tree_pack_archive_bytes": tree_pack_summary["archive_bytes"],
            "tree_pack_index_error_count": tree_pack_summary["index_error_count"],
        },
        "validation_summary": validation_summary,
        "signals_summary": signals["summary"],
        "packs": pack_rows,
        "tree_packs": tree_pack_rows,
    }


def repository_storage_signals(ctx: ServerContext, repo_name: str) -> dict[str, Any]:
    with _server_content_module()._connect(ctx) as conn:
        _server_content_module()._ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = _server_content_module()._repository_scope_params(conn, repo_name)
        signals: list[dict[str, Any]] = []

        def _tree_pack_entries_by_id(pack_index: dict[str, Any]) -> dict[str, dict[str, Any]]:
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

        blob_rows = [
            dict(r)
            for r in conn.execute(
                """
                select distinct b.*
                from blobs b
                join snapshot_files sf on sf.blob_id = b.blob_id
                join snapshots s on s.snapshot_id = sf.snapshot_id
                where """
                + _server_content_module()._repository_id_scope_predicate("s")
                + """
                order by b.blob_id asc
                """,
                (repo_id, scoped_repo_name),
            )
        ]

        pack_rows = [
            dict(r)
            for r in conn.execute(
                "select * from packs where " + _server_content_module()._repository_id_scope_predicate() + " order by created_at desc",
                (repo_id, scoped_repo_name),
            )
        ]
        pack_known_ids = {row["pack_id"] for row in pack_rows}
        referenced_pack_ids = {
            str(row.get("pack_id"))
            for row in blob_rows
            if isinstance(row.get("pack_id"), str) and str(row.get("pack_id")).strip()
        }
        missing_pack_ids = sorted(referenced_pack_ids - pack_known_ids)
        if missing_pack_ids:
            placeholders = ", ".join("?" for _ in missing_pack_ids)
            extra_rows = [
                dict(r)
                for r in conn.execute(
                    f"select * from packs where pack_id in ({placeholders}) order by created_at desc",
                    tuple(missing_pack_ids),
                )
            ]
            seen_pack_ids = set(pack_known_ids)
            for row in extra_rows:
                pack_id = str(row.get("pack_id") or "")
                if not pack_id or pack_id in seen_pack_ids:
                    continue
                pack_rows.append(row)
                seen_pack_ids.add(pack_id)

        pack_index_entries: dict[str, dict[str, dict[str, Any]] | None] = {}
        pack_known_ids = {row["pack_id"] for row in pack_rows}
        for row in pack_rows:
            pack_id = row["pack_id"]
            if not row.get("pack_path"):
                signals.append({"type": "pack_missing_path", "pack_id": pack_id, "repairable": False})
                pack_index_entries[pack_id] = None
                continue
            pack_abs = ctx.root / row["pack_path"]
            if not pack_abs.exists():
                signals.append({"type": "pack_missing_file", "pack_id": pack_id, "pack_path": row["pack_path"], "repairable": False})
                pack_index_entries[pack_id] = None
                continue
            try:
                pack_index = read_pack_index(pack_abs)
            except Exception as exc:
                signals.append({"type": "pack_invalid_index", "pack_id": pack_id, "detail": str(exc), "repairable": False})
                pack_index_entries[pack_id] = None
                continue
            expected_checksum = hashlib.sha256(json.dumps(pack_index, indent=2, sort_keys=True).encode("utf-8")).hexdigest()
            if row.get("pack_index_checksum") and expected_checksum != row["pack_index_checksum"]:
                signals.append(
                    {
                        "type": "pack_index_checksum_mismatch",
                        "pack_id": pack_id,
                        "stored": row["pack_index_checksum"],
                        "expected": expected_checksum,
                        "repairable": False,
                    }
                )
            if row.get("pack_format") and row["pack_format"] != pack_index.get("pack_format"):
                signals.append(
                    {
                        "type": "pack_format_mismatch",
                        "pack_id": pack_id,
                        "stored": row["pack_format"],
                        "expected": pack_index.get("pack_format"),
                        "repairable": False,
                    }
                )
            if row.get("pack_index_entry_name") and row["pack_index_entry_name"] != pack_index.get("index_entry_name"):
                signals.append(
                    {
                        "type": "pack_index_entry_name_mismatch",
                        "pack_id": pack_id,
                        "stored": row["pack_index_entry_name"],
                        "expected": pack_index.get("index_entry_name"),
                        "repairable": False,
                    }
                )
            if int(row.get("member_count") or 0) != int(pack_index.get("member_count") or 0):
                signals.append(
                    {
                        "type": "pack_member_count_mismatch",
                        "pack_id": pack_id,
                        "stored": int(row.get("member_count") or 0),
                        "expected": int(pack_index.get("member_count") or 0),
                        "repairable": False,
                    }
                )
            pack_index_entries[pack_id] = {
                entry["entry_name"]: entry for entry in pack_index.get("entries", []) if isinstance(entry, dict)
            }

        tree_pack_rows = [dict(r) for r in conn.execute("select * from tree_packs order by created_at desc")]
        tree_pack_entries: dict[str, dict[str, dict[str, Any]] | None] = {}
        tree_pack_paths: dict[str, Path] = {}
        tree_pack_known_ids = {row["pack_id"] for row in tree_pack_rows}
        for row in tree_pack_rows:
            pack_id = row["pack_id"]
            if not row.get("pack_path"):
                signals.append({"type": "tree_pack_missing_path", "pack_id": pack_id, "repairable": False})
                tree_pack_entries[pack_id] = None
                continue
            pack_abs = ctx.root / row["pack_path"]
            tree_pack_paths[pack_id] = pack_abs
            if not pack_abs.exists():
                signals.append({"type": "tree_pack_missing_file", "pack_id": pack_id, "pack_path": row["pack_path"], "repairable": False})
                tree_pack_entries[pack_id] = None
                continue
            try:
                pack_index = read_tree_pack_index(pack_abs)
                indexed_trees = _tree_pack_entries_by_id(pack_index)
            except Exception as exc:
                signals.append({"type": "tree_pack_invalid_index", "pack_id": pack_id, "detail": str(exc), "repairable": False})
                tree_pack_entries[pack_id] = None
                continue
            expected_checksum = hashlib.sha256(json.dumps(pack_index, indent=2, sort_keys=True).encode("utf-8")).hexdigest()
            if row.get("pack_index_checksum") and expected_checksum != row["pack_index_checksum"]:
                signals.append(
                    {
                        "type": "tree_pack_index_checksum_mismatch",
                        "pack_id": pack_id,
                        "stored": row["pack_index_checksum"],
                        "expected": expected_checksum,
                        "repairable": False,
                    }
                )
            if row.get("pack_format") and row["pack_format"] != pack_index.get("pack_format"):
                signals.append(
                    {
                        "type": "tree_pack_format_mismatch",
                        "pack_id": pack_id,
                        "stored": row["pack_format"],
                        "expected": pack_index.get("pack_format"),
                        "repairable": False,
                    }
                )
            if row.get("pack_index_entry_name") and row["pack_index_entry_name"] != pack_index.get("index_entry_name"):
                signals.append(
                    {
                        "type": "tree_pack_index_entry_name_mismatch",
                        "pack_id": pack_id,
                        "stored": row["pack_index_entry_name"],
                        "expected": pack_index.get("index_entry_name"),
                        "repairable": False,
                    }
                )
            if int(row.get("tree_count") or 0) != int(pack_index.get("tree_count") or 0):
                signals.append(
                    {
                        "type": "tree_pack_tree_count_mismatch",
                        "pack_id": pack_id,
                        "stored": int(row.get("tree_count") or 0),
                        "expected": int(pack_index.get("tree_count") or 0),
                        "repairable": False,
                    }
                )
            tree_pack_entries[pack_id] = indexed_trees

        root_tree_rows = [
            dict(r)
            for r in conn.execute(
                """
                select
                    s.snapshot_id,
                    s.root_tree_id,
                    t.entry_count,
                    t.tree_pack_id,
                    t.tree_pack_entry_name,
                    t.tree_pack_checksum
                from snapshots s
                left join trees t on t.tree_id = s.root_tree_id
                where """
                + _server_content_module()._repository_id_scope_predicate("s")
                + """
                  and coalesce(s.root_tree_id, '') != ''
                order by s.snapshot_id asc
                """,
                (repo_id, scoped_repo_name),
            )
        ]
        for row in root_tree_rows:
            snapshot_id = str(row["snapshot_id"])
            root_tree_id = str(row["root_tree_id"])
            if row.get("entry_count") is None:
                signals.append(
                    {
                        "type": "snapshot_root_tree_missing",
                        "snapshot_id": snapshot_id,
                        "root_tree_id": root_tree_id,
                        "repairable": False,
                    }
                )
                continue
            tree_pack_id = row.get("tree_pack_id")
            entry_name = row.get("tree_pack_entry_name")
            checksum = row.get("tree_pack_checksum")
            if not tree_pack_id or not entry_name or not checksum:
                signals.append(
                    {
                        "type": "root_tree_missing_pack_metadata",
                        "snapshot_id": snapshot_id,
                        "root_tree_id": root_tree_id,
                        "repairable": False,
                    }
                )
                continue
            if tree_pack_id not in tree_pack_known_ids:
                signals.append(
                    {
                        "type": "root_tree_missing_tree_pack",
                        "snapshot_id": snapshot_id,
                        "root_tree_id": root_tree_id,
                        "tree_pack_id": tree_pack_id,
                        "repairable": False,
                    }
                )
                continue
            indexed_trees = tree_pack_entries.get(str(tree_pack_id))
            if indexed_trees is None:
                continue
            indexed_entry = indexed_trees.get(root_tree_id)
            if indexed_entry is None:
                signals.append(
                    {
                        "type": "root_tree_missing_pack_entry",
                        "snapshot_id": snapshot_id,
                        "root_tree_id": root_tree_id,
                        "tree_pack_id": tree_pack_id,
                        "repairable": False,
                    }
                )
                continue
            if str(entry_name) != str(indexed_entry.get("entry_name") or ""):
                signals.append(
                    {
                        "type": "root_tree_pack_entry_name_mismatch",
                        "snapshot_id": snapshot_id,
                        "root_tree_id": root_tree_id,
                        "stored": entry_name,
                        "expected": indexed_entry.get("entry_name"),
                        "repairable": False,
                    }
                )
            if str(checksum) != str(indexed_entry.get("checksum") or ""):
                signals.append(
                    {
                        "type": "root_tree_pack_checksum_mismatch",
                        "snapshot_id": snapshot_id,
                        "root_tree_id": root_tree_id,
                        "stored": checksum,
                        "expected": indexed_entry.get("checksum"),
                        "repairable": False,
                    }
                )
            if int(row.get("entry_count") or 0) != int(indexed_entry.get("entry_count", 0) or 0):
                signals.append(
                    {
                        "type": "root_tree_pack_entry_count_mismatch",
                        "snapshot_id": snapshot_id,
                        "root_tree_id": root_tree_id,
                        "stored": int(row.get("entry_count") or 0),
                        "expected": int(indexed_entry.get("entry_count", 0) or 0),
                        "repairable": False,
                    }
                )
            pack_abs = tree_pack_paths.get(str(tree_pack_id))
            if pack_abs is None:
                continue
            try:
                read_tree_pack_tree(pack_abs, root_tree_id)
            except Exception as exc:
                signals.append(
                    {
                        "type": "root_tree_pack_payload_invalid",
                        "snapshot_id": snapshot_id,
                        "root_tree_id": root_tree_id,
                        "tree_pack_id": tree_pack_id,
                        "detail": str(exc),
                        "repairable": False,
                    }
                )

        for row in blob_rows:
            blob_id = row["blob_id"]
            storage_kind = row.get("storage_kind") or "pack_full"
            pack_id = row.get("pack_id")
            pack_entry_name = row.get("pack_entry_name")
            pack_entry_type = row.get("pack_entry_type") or ("delta" if storage_kind == "pack_delta" else "full")
            expected_storage_kind = "pack_delta" if pack_entry_type == "delta" else "pack_full"

            if pack_id:
                if pack_id not in pack_known_ids:
                    signals.append({"type": "blob_missing_pack_metadata", "blob_id": blob_id, "pack_id": pack_id, "repairable": False})
                    continue
                entries = pack_index_entries.get(pack_id)
                if entries is None:
                    continue
                if not pack_entry_name or pack_entry_name not in entries:
                    signals.append(
                        {
                            "type": "packed_blob_missing_entry",
                            "blob_id": blob_id,
                            "pack_id": pack_id,
                            "pack_entry_name": pack_entry_name,
                            "repairable": False,
                        }
                    )
                    continue
                entry = entries[pack_entry_name]
                entry_type = entry.get("entry_type") or "full"
                expected_storage_kind = "pack_delta" if entry_type == "delta" else "pack_full"
                if storage_kind != expected_storage_kind:
                    signals.append(
                        {
                            "type": "blob_storage_kind_mismatch",
                            "blob_id": blob_id,
                            "stored_storage_kind": storage_kind,
                            "expected_storage_kind": expected_storage_kind,
                            "expected_pack_entry_type": entry_type,
                            "expected_pack_base_blob_id": entry.get("base_blob_id"),
                            "expected_pack_chain_depth": int(entry.get("chain_depth", 0) or 0),
                            "repairable": True,
                        }
                    )
                if row.get("pack_entry_type") != entry_type:
                    signals.append(
                        {
                            "type": "blob_pack_entry_type_mismatch",
                            "blob_id": blob_id,
                            "stored": row.get("pack_entry_type"),
                            "expected": entry_type,
                            "repairable": True,
                        }
                    )
                if row.get("pack_base_blob_id") != entry.get("base_blob_id"):
                    signals.append(
                        {
                            "type": "blob_pack_base_mismatch",
                            "blob_id": blob_id,
                            "stored": row.get("pack_base_blob_id"),
                            "expected": entry.get("base_blob_id"),
                            "repairable": True,
                        }
                    )
                if int(row.get("pack_chain_depth") or 0) != int(entry.get("chain_depth", 0) or 0):
                    signals.append(
                        {
                            "type": "blob_pack_chain_depth_mismatch",
                            "blob_id": blob_id,
                            "stored": int(row.get("pack_chain_depth") or 0),
                            "expected": int(entry.get("chain_depth", 0) or 0),
                            "repairable": True,
                        }
                    )
                if entry_type == "delta":
                    base_blob_id = entry.get("base_blob_id")
                    if not base_blob_id:
                        signals.append({"type": "packed_delta_missing_base_blob_id", "blob_id": blob_id, "repairable": False})
                    elif _server_content_module()._blob_row(conn, base_blob_id) is None:
                        signals.append(
                            {
                                "type": "packed_delta_missing_base_blob",
                                "blob_id": blob_id,
                                "base_blob_id": base_blob_id,
                                "repairable": False,
                            }
                        )
                continue

            if storage_kind == "loose":
                storage_path_value = str(row.get("storage_path") or "").strip()
                blob_storage = (ctx.root / storage_path_value) if storage_path_value else None
                if blob_storage is not None and blob_storage.exists():
                    continue
                signals.append(
                    {
                        "type": "loose_blob_missing_payload",
                        "blob_id": blob_id,
                        "storage_path": storage_path_value or None,
                        "repairable": False,
                    }
                )
                continue

            signals.append(
                {
                    "type": "blob_storage_kind_missing_pack_ref",
                    "blob_id": blob_id,
                    "stored_storage_kind": storage_kind,
                    "repairable": False,
                }
            )
    by_type: dict[str, int] = {}
    repairable_count = 0
    for signal in signals:
        by_type[signal["type"]] = by_type.get(signal["type"], 0) + 1
        if signal.get("repairable"):
            repairable_count += 1
    return {
        "signals": signals,
        "summary": {
            "drift_count": len(signals),
            "repairable_drift_count": repairable_count,
            "by_type": by_type,
        },
    }



def pack_repository(ctx: ServerContext, repo_name: str, *, repack: bool = False, max_members: int | None = None) -> dict[str, Any]:
    with _server_content_module()._connect(ctx) as conn:
        _server_content_module()._ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = _server_content_module()._repository_scope_params(conn, repo_name)
        where_clause = "1 = 1" if repack else "b.pack_id is null"
        rows = [
            dict(r)
            for r in conn.execute(
                """
                select
                    b.blob_id,
                    b.sha256,
                    b.storage_path,
                    b.size_bytes,
                    b.storage_kind,
                    b.pack_id,
                b.pack_entry_name,
                min(sf.path) as path_hint,
                min(s.created_at) as first_seen_at
                from blobs b
                join snapshot_files sf on sf.blob_id = b.blob_id
                join snapshots s on s.snapshot_id = sf.snapshot_id
                where """
                + _server_content_module()._repository_id_scope_predicate("s")
                + " and "
                + where_clause
                + """
                group by b.blob_id, b.sha256, b.storage_path, b.size_bytes, b.storage_kind, b.pack_id, b.pack_entry_name
                order by path_hint asc, first_seen_at asc, b.blob_id asc
                """,
                (repo_id, scoped_repo_name),
            )
        ]
        if max_members is not None:
            rows = rows[: max(0, max_members)]
        if not rows:
            stats = repository_storage_stats(ctx, repo_name)
            return {
                "created": False,
                "reason": "no_reachable_blobs" if repack else "no_unpacked_reachable_blobs",
                "stats": stats,
                "repack": repack,
            }

        pack_seed = repo_name + "|" + "|".join(row["blob_id"] for row in rows) + "|" + utc_now() + "|" + uuid.uuid4().hex
        pack_id = f"PCK-{hashlib.sha256(pack_seed.encode('utf-8')).hexdigest()[:12].upper()}"
        pack_abs = _server_content_module()._pack_path(ctx, pack_id)
        blob_items: list[dict[str, object]] = []
        now = utc_now()
        for row in rows:
            data = _server_content_module()._blob_bytes_by_row(ctx, conn, row)
            blob_items.append(
                {
                    "entry_name": f"blobs/{row['blob_id']}",
                    "blob_id": row["blob_id"],
                    "data": data,
                    "path_hint": row.get("path_hint"),
                }
            )
        members = build_pack_members(blob_items)
        members_by_blob_id = {member["blob_id"]: member for member in members}
        archive_stats = write_pack_archive(pack_abs, pack_id, now, members)
        conn.execute(
            "insert into packs(pack_id, repo_name, repo_id, status, member_count, total_bytes, pack_path, pack_format, pack_index_entry_name, pack_index_checksum, created_at) values (?, ?, ?, 'ready', ?, ?, ?, ?, ?, ?, ?)",
            (
                pack_id,
                repo_name,
                repo_id,
                archive_stats["member_count"],
                archive_stats["total_bytes"],
                str(pack_abs.relative_to(ctx.root)),
                archive_stats["pack_format"],
                archive_stats["pack_index_entry_name"],
                archive_stats["pack_index_checksum"],
                now,
            ),
        )
        for row in rows:
            member = members_by_blob_id[row["blob_id"]]
            entry_name = member["entry_name"]
            entry_type = member.get("entry_type", "full")
            base_blob_id = member.get("base_blob_id")
            chain_depth = int(member.get("chain_depth", 0) or 0)
            target_storage_kind = "pack_delta" if entry_type == "delta" else "pack_full"
            conn.execute(
                """
                update blobs
                set pack_id = ?,
                    pack_entry_name = ?,
                    pack_entry_type = ?,
                    pack_base_blob_id = ?,
                    pack_chain_depth = ?,
                    storage_kind = ?,
                    packed_at = ?
                where blob_id = ?
                """,
                (pack_id, entry_name, entry_type, base_blob_id, chain_depth, target_storage_kind, now, row["blob_id"]),
            )
        conn.commit()
    return {
        "created": True,
        "repo_name": repo_name,
        "pack_id": pack_id,
        "pack_path": str(pack_abs.relative_to(ctx.root)),
        "pack_format": archive_stats["pack_format"],
        "pack_index_entry_name": archive_stats["pack_index_entry_name"],
        "pack_index_checksum": archive_stats["pack_index_checksum"],
        "member_count": archive_stats["member_count"],
        "total_bytes": archive_stats["total_bytes"],
        "archive_bytes": archive_stats["archive_bytes"],
        "repack": repack,
        "stats": repository_storage_stats(ctx, repo_name),
    }



def gc_repository_content(
    ctx: ServerContext,
    repo_name: str,
    *,
    prune_unreferenced: bool = True,
    prune_orphan_packs: bool = True,
) -> dict[str, Any]:
    with _server_content_module()._connect(ctx) as conn:
        _server_content_module()._ensure_schema(conn, ctx)
        removed_unreferenced = 0
        removed_unreachable_trees = 0
        removed_unreachable_tree_entries = 0
        removed_orphan_packs = 0
        removed_orphan_tree_packs = 0
        repo_id, scoped_repo_name = _server_content_module()._repository_scope_params(conn, repo_name)

        if prune_unreferenced:
            rows = [
                dict(r)
                for r in conn.execute(
                    """
                    select * from blobs b
                    where not exists (
                        select 1 from snapshot_files sf where sf.blob_id = b.blob_id
                    )
                    order by blob_id asc
                    """
                )
            ]
            for row in rows:
                conn.execute("delete from blobs where blob_id = ?", (row["blob_id"],))
                removed_unreferenced += 1

            unreachable_tree_ids = _server_content_module()._unreachable_tree_ids(conn)
            if unreachable_tree_ids:
                placeholder_sql = ", ".join("?" for _ in unreachable_tree_ids)
                removed_unreachable_tree_entries = int(
                    conn.execute(
                        f"select count(*) as c from tree_entries where tree_id in ({placeholder_sql})",
                        tuple(unreachable_tree_ids),
                    ).fetchone()["c"]
                    or 0
                )
                conn.executemany("delete from trees where tree_id = ?", [(tree_id,) for tree_id in unreachable_tree_ids])
                removed_unreachable_trees = len(unreachable_tree_ids)

        if prune_orphan_packs:
            pack_rows = [
                dict(r)
                for r in conn.execute(
                    "select * from packs where " + _server_content_module()._repository_id_scope_predicate() + " order by created_at asc",
                    (repo_id, scoped_repo_name),
                )
            ]
            for row in pack_rows:
                ref_count = conn.execute("select count(*) as c from blobs where pack_id = ?", (row["pack_id"],)).fetchone()["c"]
                if ref_count != 0:
                    continue
                pack_abs = ctx.root / row["pack_path"]
                if pack_abs.exists():
                    pack_abs.unlink()
                conn.execute("delete from packs where pack_id = ?", (row["pack_id"],))
                removed_orphan_packs += 1
            tree_pack_rows = [dict(r) for r in conn.execute("select * from tree_packs order by created_at asc")]
            for row in tree_pack_rows:
                ref_count = conn.execute("select count(*) as c from trees where tree_pack_id = ?", (row["pack_id"],)).fetchone()["c"]
                if ref_count != 0:
                    continue
                pack_abs = ctx.root / row["pack_path"]
                if pack_abs.exists():
                    pack_abs.unlink()
                conn.execute("delete from tree_packs where pack_id = ?", (row["pack_id"],))
                removed_orphan_tree_packs += 1

        conn.commit()
    return {
        "repo_name": repo_name,
        "removed_unreferenced_blob_count": removed_unreferenced,
        "removed_unreachable_tree_count": removed_unreachable_trees,
        "removed_unreachable_tree_entry_count": removed_unreachable_tree_entries,
        "removed_orphan_pack_count": removed_orphan_packs,
        "removed_orphan_tree_pack_count": removed_orphan_tree_packs,
        "stats": repository_storage_stats(ctx, repo_name),
    }
