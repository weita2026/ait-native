from __future__ import annotations

import hashlib
import json
import secrets
import sqlite3
from pathlib import Path
from typing import Any, Iterable

from ait_protocol.common import connect_sqlite, normalize_optional_text, utc_now
from ait_storage.packfiles import (
    build_pack_members,
    build_storage_validation_summary,
    read_pack_entry,
    summarize_pack_archives,
    write_pack_archive,
)
from ait_storage.treepacks import (
    build_tree_pack_members,
    summarize_tree_pack_archives,
    tree_pack_manifest_path,
    write_tree_pack_archive,
)

from .repo_paths import RepoContext

FIRST_WAVE_LOCAL_CONTENT_SCHEMA_VERSION = 1
_LOCAL_SERVER_CATALOG_TABLE_DROP_ORDER = (
    "repository_group_memberships",
    "repository_groups",
    "repositories",
)


def _table_columns(conn, table_name: str) -> set[str]:
    rows = conn.execute(f"pragma table_info({table_name})").fetchall()
    columns: set[str] = set()
    for row in rows:
        if isinstance(row, sqlite3.Row):
            name = row["name"]
        else:
            name = row[1]
        if name is not None:
            columns.add(str(name))
    return columns


def _drop_column_if_exists(conn, table_name: str, column_name: str) -> bool:
    if column_name not in _table_columns(conn, table_name):
        return False
    try:
        conn.execute(f"alter table {table_name} drop column {column_name}")
    except sqlite3.Error:
        return False
    return column_name not in _table_columns(conn, table_name)


def _tree_entry_size_column_present(conn) -> bool:
    return "size_bytes" in _table_columns(conn, "tree_entries")


def _blob_pack_entry_name_column_present(conn) -> bool:
    return "pack_entry_name" in _table_columns(conn, "blobs")


def _blob_packed_at_column_present(conn) -> bool:
    return "packed_at" in _table_columns(conn, "blobs")


def _blob_storage_kind_column_present(conn) -> bool:
    return "storage_kind" in _table_columns(conn, "blobs")


def _tree_pack_entry_name_column_present(conn) -> bool:
    return "tree_pack_entry_name" in _table_columns(conn, "trees")


def _tree_packed_at_column_present(conn) -> bool:
    return "tree_packed_at" in _table_columns(conn, "trees")


def _legacy_pack_metadata_columns_present(conn) -> tuple[tuple[str, str], ...]:
    present: list[tuple[str, str]] = []
    if _blob_pack_entry_name_column_present(conn):
        present.append(("blobs", "pack_entry_name"))
    if _blob_packed_at_column_present(conn):
        present.append(("blobs", "packed_at"))
    if _tree_pack_entry_name_column_present(conn):
        present.append(("trees", "tree_pack_entry_name"))
    if _tree_packed_at_column_present(conn):
        present.append(("trees", "tree_packed_at"))
    return tuple(present)


def _tree_pack_entry_name_sql(conn, tree_alias: str) -> str:
    derived = f"'trees/' || {tree_alias}.tree_id || '.json'"
    if _tree_pack_entry_name_column_present(conn):
        return f"coalesce(nullif({tree_alias}.tree_pack_entry_name, ''), {derived})"
    return derived


def _manifest_path_sql(conn, *, tree_alias: str, tree_pack_alias: str, root_tree_expr: str) -> str:
    return (
        "case "
        f"when coalesce({tree_pack_alias}.pack_path, '') != '' "
        f"then {tree_pack_alias}.pack_path || '#' || {_tree_pack_entry_name_sql(conn, tree_alias)} "
        f"else 'trees/' || {root_tree_expr} "
        "end"
    )


def _tree_entry_blob_size_sql(conn, entry_alias: str = "te", blob_alias: str = "b") -> str:
    if _tree_entry_size_column_present(conn):
        return (
            f"case when {entry_alias}.entry_type = 'blob' "
            f"then coalesce({entry_alias}.size_bytes, {blob_alias}.size_bytes) "
            f"else {entry_alias}.size_bytes end"
        )
    return f"case when {entry_alias}.entry_type = 'blob' then {blob_alias}.size_bytes else null end"


def _tree_reachability_cte_sql(snapshot_predicate: str = "coalesce(root_tree_id, '') != ''") -> str:
    return f"""
    with recursive reachable_trees(tree_id) as (
        select distinct root_tree_id
        from snapshots
        where {snapshot_predicate}
      union
        select te.target_id
        from tree_entries te
        join reachable_trees rt on rt.tree_id = te.tree_id
        where te.entry_type = 'tree'
    )
    """


def _tree_metadata_stats(conn) -> dict[str, int]:
    reachable_cte = _tree_reachability_cte_sql()
    row = conn.execute(
        reachable_cte
        + """
        select
            (select count(*) from trees) as tree_count,
            (select count(*) from tree_entries) as tree_entry_count,
            (select count(*) from reachable_trees) as reachable_tree_count,
            (select count(*) from tree_entries te join reachable_trees rt on rt.tree_id = te.tree_id) as reachable_tree_entry_count,
            (select count(distinct tree_pack_id) from trees where coalesce(tree_pack_id, '') != '') as tree_pack_count,
            (
                select count(distinct t.tree_pack_id)
                from trees t
                join reachable_trees rt on rt.tree_id = t.tree_id
                where coalesce(t.tree_pack_id, '') != ''
            ) as reachable_tree_pack_count,
            (
                select count(*)
                from tree_packs tp
                where not exists (select 1 from trees t where t.tree_pack_id = tp.pack_id)
            ) as orphan_tree_pack_count
        """
    ).fetchone()
    tree_count = int(row["tree_count"] or 0)
    tree_entry_count = int(row["tree_entry_count"] or 0)
    reachable_tree_count = int(row["reachable_tree_count"] or 0)
    reachable_tree_entry_count = int(row["reachable_tree_entry_count"] or 0)
    tree_pack_count = int(row["tree_pack_count"] or 0)
    reachable_tree_pack_count = int(row["reachable_tree_pack_count"] or 0)
    orphan_tree_pack_count = int(row["orphan_tree_pack_count"] or 0)
    return {
        "tree_count": tree_count,
        "tree_entry_count": tree_entry_count,
        "reachable_tree_count": reachable_tree_count,
        "reachable_tree_entry_count": reachable_tree_entry_count,
        "unreachable_tree_count": max(tree_count - reachable_tree_count, 0),
        "unreachable_tree_entry_count": max(tree_entry_count - reachable_tree_entry_count, 0),
        "tree_pack_count": tree_pack_count,
        "reachable_tree_pack_count": reachable_tree_pack_count,
        "orphan_tree_pack_count": orphan_tree_pack_count,
    }


def _unreachable_tree_ids(conn) -> list[str]:
    reachable_cte = _tree_reachability_cte_sql()
    rows = conn.execute(
        reachable_cte
        + """
        select t.tree_id
        from trees t
        where not exists (
            select 1 from reachable_trees rt where rt.tree_id = t.tree_id
        )
        order by t.tree_id asc
        """
    ).fetchall()
    return [str(row["tree_id"]) for row in rows]


def _insert_tree_records(conn, tree_rows: list[dict[str, Any]], tree_entry_rows: list[dict[str, Any]], created_at: str) -> None:
    if tree_rows:
        conn.executemany(
            "insert or ignore into trees(tree_id, entry_count, created_at) values (?, ?, ?)",
            [(row["tree_id"], int(row["entry_count"]), created_at) for row in tree_rows],
        )
    if tree_entry_rows:
        if _tree_entry_size_column_present(conn):
            conn.executemany(
                """
                insert or ignore into tree_entries(tree_id, entry_name, entry_type, target_id, size_bytes, mode)
                values (?, ?, ?, ?, ?, ?)
                """,
                [
                    (
                        row["tree_id"],
                        row["entry_name"],
                        row["entry_type"],
                        row["target_id"],
                        None if str(row["entry_type"] or "") == "blob" else row["size_bytes"],
                        row["mode"],
                    )
                    for row in tree_entry_rows
                ],
            )
        else:
            conn.executemany(
                """
                insert or ignore into tree_entries(tree_id, entry_name, entry_type, target_id, mode)
                values (?, ?, ?, ?, ?)
                """,
                [
                    (
                        row["tree_id"],
                        row["entry_name"],
                        row["entry_type"],
                        row["target_id"],
                        row["mode"],
                    )
                    for row in tree_entry_rows
                ],
            )


def _tree_pack_entry_rows(conn, tree_ids: Iterable[str]) -> list[dict[str, Any]]:
    ids = [str(tree_id) for tree_id in tree_ids if str(tree_id)]
    if not ids:
        return []
    placeholder_sql = ", ".join("?" for _ in ids)
    size_expr = _tree_entry_blob_size_sql(conn, "te", "b")
    rows = conn.execute(
        f"""
        select
            te.tree_id,
            te.entry_name,
            te.entry_type,
            te.target_id,
            {size_expr} as size_bytes,
            te.mode
        from tree_entries te
        left join blobs b on b.blob_id = te.target_id
        where te.tree_id in ({placeholder_sql})
        order by te.tree_id asc, te.entry_name asc
        """,
        tuple(ids),
    ).fetchall()
    return [dict(row) for row in rows]


def _tree_pack_row_map(conn, tree_ids: Iterable[str]) -> dict[str, dict[str, Any]]:
    ids = [str(tree_id) for tree_id in tree_ids if str(tree_id)]
    if not ids:
        return {}
    placeholder_sql = ", ".join("?" for _ in ids)
    rows = conn.execute(
        f"""
        select
            tree_id,
            entry_count,
            tree_pack_id,
            {_tree_pack_entry_name_sql(conn, 'trees')} as tree_pack_entry_name,
            tree_pack_checksum
        from trees
        where tree_id in ({placeholder_sql})
        order by tree_id asc
        """,
        tuple(ids),
    ).fetchall()
    return {str(row["tree_id"]): dict(row) for row in rows}


def _derived_blob_pack_entry_name(blob_id: str) -> str:
    return f"blobs/{blob_id}"


def _row_optional_value(row: dict[str, Any] | sqlite3.Row, key: str) -> Any:
    if isinstance(row, sqlite3.Row):
        return row[key] if key in row.keys() else None
    return row.get(key)


def _blob_pack_entry_name(row: dict[str, Any] | sqlite3.Row) -> str:
    explicit_raw = _row_optional_value(row, "pack_entry_name")
    explicit = normalize_optional_text(explicit_raw) if explicit_raw is not None else None
    return explicit or _derived_blob_pack_entry_name(str(row["blob_id"]))


def _derived_tree_pack_entry_name(tree_id: str) -> str:
    return f"trees/{tree_id}.json"


def _tree_pack_entry_name(row: dict[str, Any] | sqlite3.Row) -> str:
    explicit_raw = _row_optional_value(row, "tree_pack_entry_name")
    explicit = normalize_optional_text(explicit_raw) if explicit_raw is not None else None
    return explicit or _derived_tree_pack_entry_name(str(row["tree_id"]))


def _insert_blob_record(
    conn,
    *,
    blob_id: str,
    sha256: str,
    storage_path: str,
    size_bytes: int,
    pack_id: str | None,
    pack_entry_type: str | None,
    pack_base_blob_id: str | None,
    pack_chain_depth: int | None,
    pruned_at: str | None,
    created_at: str,
    pack_entry_name: str | None = None,
    packed_at: str | None = None,
) -> None:
    columns = [
        "blob_id",
        "sha256",
        "storage_path",
        "size_bytes",
        "pack_id",
    ]
    values: list[Any] = [
        blob_id,
        sha256,
        storage_path,
        size_bytes,
        pack_id,
    ]
    if _blob_pack_entry_name_column_present(conn):
        columns.append("pack_entry_name")
        values.append(pack_entry_name)
    columns.extend(
        [
            "pack_entry_type",
            "pack_base_blob_id",
            "pack_chain_depth",
        ]
    )
    values.extend(
        [
            pack_entry_type,
            pack_base_blob_id,
            pack_chain_depth,
        ]
    )
    if _blob_packed_at_column_present(conn):
        columns.append("packed_at")
        values.append(packed_at)
    columns.extend(["pruned_at", "created_at"])
    values.extend([pruned_at, created_at])
    placeholder_sql = ", ".join("?" for _ in columns)
    conn.execute(
        f"insert or ignore into blobs({', '.join(columns)}) values ({placeholder_sql})",
        tuple(values),
    )


def _update_blob_pack_metadata(
    conn,
    *,
    pack_id: str,
    pack_entry_type: str,
    pack_base_blob_id: str | None,
    pack_chain_depth: int,
    blob_id: str,
    pack_entry_name: str | None = None,
    packed_at: str | None = None,
) -> None:
    assignments = [
        "pack_id = ?",
        "pack_entry_type = ?",
        "pack_base_blob_id = ?",
        "pack_chain_depth = ?",
    ]
    values: list[Any] = [
        pack_id,
        pack_entry_type,
        pack_base_blob_id,
        pack_chain_depth,
    ]
    if _blob_pack_entry_name_column_present(conn):
        assignments.append("pack_entry_name = ?")
        values.append(pack_entry_name)
    if _blob_packed_at_column_present(conn):
        assignments.append("packed_at = ?")
        values.append(packed_at)
    values.append(blob_id)
    conn.execute(
        f"update blobs set {', '.join(assignments)} where blob_id = ?",
        tuple(values),
    )


def _blob_path(ctx: RepoContext, blob_id: str) -> Path:
    return ctx.pack_dir / f"{blob_id}.packref"


def _pack_path(ctx: RepoContext, pack_id: str) -> Path:
    return ctx.pack_dir / f"{pack_id}.zip"


def _tree_pack_path(ctx: RepoContext, pack_id: str) -> Path:
    return ctx.tree_pack_dir / f"{pack_id}.zip"


def _write_tree_pack(
    ctx: RepoContext,
    conn,
    tree_ids: Iterable[str],
    *,
    created_at: str,
    seed_hint: str,
) -> dict[str, dict[str, Any]]:
    tree_id_list = [str(tree_id) for tree_id in tree_ids if str(tree_id)]
    if not tree_id_list:
        return {}
    current_rows = _tree_pack_row_map(conn, tree_id_list)
    missing_tree_ids = [tree_id for tree_id in tree_id_list if not current_rows.get(tree_id, {}).get("tree_pack_id")]
    if not missing_tree_ids:
        return current_rows
    placeholder_sql = ", ".join("?" for _ in missing_tree_ids)
    tree_rows = [
        dict(row)
        for row in conn.execute(
            f"select tree_id, entry_count from trees where tree_id in ({placeholder_sql}) order by tree_id asc",
            tuple(missing_tree_ids),
        ).fetchall()
    ]
    if not tree_rows:
        return current_rows
    tree_entry_rows = _tree_pack_entry_rows(conn, missing_tree_ids)
    pack_seed = f"{seed_hint}|{json.dumps(sorted(missing_tree_ids))}"
    pack_id = f"TPK-{hashlib.sha256(pack_seed.encode('utf-8')).hexdigest()[:12].upper()}"
    pack_abs = _tree_pack_path(ctx, pack_id)
    members = build_tree_pack_members(tree_rows, tree_entry_rows)
    archive_stats = write_tree_pack_archive(pack_abs, pack_id, created_at, members)
    conn.execute(
        """
        insert or ignore into tree_packs(
            pack_id, status, tree_count, total_bytes, pack_path, pack_format, pack_index_entry_name, pack_index_checksum, created_at
        ) values (?, 'ready', ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            pack_id,
            archive_stats["tree_count"],
            archive_stats["total_bytes"],
            str(pack_abs.relative_to(ctx.root)),
            archive_stats["pack_format"],
            archive_stats["pack_index_entry_name"],
            archive_stats["pack_index_checksum"],
            created_at,
        ),
    )
    members_by_tree_id = {str(member["tree_id"]): member for member in members}
    for tree_id in missing_tree_ids:
        member = members_by_tree_id.get(tree_id)
        if member is None:
            continue
        assignments = [
            "tree_pack_id = ?",
            "tree_pack_checksum = ?",
        ]
        values: list[Any] = [pack_id, member["checksum"]]
        if _tree_pack_entry_name_column_present(conn):
            assignments.append("tree_pack_entry_name = ?")
            values.append(None)
        if _tree_packed_at_column_present(conn):
            assignments.append("tree_packed_at = ?")
            values.append(None)
        values.append(tree_id)
        conn.execute(
            f"update trees set {', '.join(assignments)} where tree_id = ?",
            tuple(values),
        )
    return _tree_pack_row_map(conn, tree_id_list)


def _manifest_path_for_tree(conn, tree_id: str) -> str:
    row = conn.execute(
        f"""
        select
            tp.pack_path,
            t.tree_id,
            {_tree_pack_entry_name_sql(conn, 't')} as tree_pack_entry_name
        from trees t
        join tree_packs tp on tp.pack_id = t.tree_pack_id
        where t.tree_id = ?
          and coalesce(t.tree_pack_id, '') != ''
        """,
        (tree_id,),
    ).fetchone()
    if row is None:
        return f"trees/{tree_id}"
    return tree_pack_manifest_path(str(row["pack_path"]), _tree_pack_entry_name(row))


def _sync_tree_pack_metadata(conn, ctx: RepoContext | None) -> None:
    if ctx is None:
        return
    tree_ids = [
        str(row["tree_id"])
        for row in conn.execute(
            "select tree_id from trees where coalesce(tree_pack_id, '') = '' order by tree_id asc"
        ).fetchall()
    ]
    if tree_ids:
        _write_tree_pack(ctx, conn, tree_ids, created_at=utc_now(), seed_hint="tree-metadata-migration")
    manifest_path_sql = _manifest_path_sql(
        conn,
        tree_alias="t",
        tree_pack_alias="tp",
        root_tree_expr="snapshots.root_tree_id",
    )
    conn.execute(
        f"""
        update snapshots
        set manifest_path = (
            select {manifest_path_sql}
            from trees t
            left join tree_packs tp on tp.pack_id = t.tree_pack_id
            where t.tree_id = snapshots.root_tree_id
        )
        where coalesce(root_tree_id, '') != ''
        """
    )


def _cleanup_manifest_files(ctx: RepoContext | None) -> None:
    if ctx is None or not ctx.manifest_dir.exists():
        return
    for path in ctx.manifest_dir.glob("*.json"):
        path.unlink(missing_ok=True)


def _sqlite_table_exists(conn, table_name: str) -> bool:
    row = conn.execute(
        "select 1 from sqlite_master where type = 'table' and name = ?",
        (table_name,),
    ).fetchone()
    return row is not None


def _local_server_catalog_tables_ready_for_drop(conn) -> tuple[str, ...]:
    droppable: list[str] = []
    for table_name in _LOCAL_SERVER_CATALOG_TABLE_DROP_ORDER:
        if not _sqlite_table_exists(conn, table_name):
            continue
        row = conn.execute(f"select count(*) as c from {table_name}").fetchone()
        if int(row["c"] or 0) != 0:
            return ()
        droppable.append(table_name)
    return tuple(droppable)


def _count_stale_manifest_paths(conn) -> int:
    try:
        manifest_path_sql = _manifest_path_sql(
            conn,
            tree_alias="t",
            tree_pack_alias="tp",
            root_tree_expr="s.root_tree_id",
        )
        row = conn.execute(
            f"""
            select count(*) as c
            from snapshots s
            left join trees t on t.tree_id = s.root_tree_id
            left join tree_packs tp on tp.pack_id = t.tree_pack_id
            where coalesce(s.root_tree_id, '') != ''
              and coalesce(s.manifest_path, '') != {manifest_path_sql}
            """
        ).fetchone()
    except sqlite3.OperationalError:
        return 0
    return int((row["c"] if row is not None else 0) or 0)


def _schema_cleanup_summary(conn) -> dict[str, Any]:
    schema_version = int(conn.execute("pragma user_version").fetchone()[0] or 0)
    present_tables = [
        table_name
        for table_name in _LOCAL_SERVER_CATALOG_TABLE_DROP_ORDER
        if _sqlite_table_exists(conn, table_name)
    ]
    stale_manifest_count = _count_stale_manifest_paths(conn)
    redundant_blob_storage_kind_present = _blob_storage_kind_column_present(conn)
    legacy_pack_metadata_columns = [
        f"{table_name}.{column_name}"
        for table_name, column_name in _legacy_pack_metadata_columns_present(conn)
    ]
    return {
        "schema_version": schema_version,
        "legacy_local_server_catalog_tables_present": present_tables,
        "legacy_local_server_catalog_table_count": len(present_tables),
        "redundant_blob_storage_kind_present": redundant_blob_storage_kind_present,
        "legacy_pack_metadata_columns_present": legacy_pack_metadata_columns,
        "legacy_pack_metadata_column_count": len(legacy_pack_metadata_columns),
        "stale_manifest_count": stale_manifest_count,
        "first_wave_schema_cleanup_applied": (
            schema_version >= FIRST_WAVE_LOCAL_CONTENT_SCHEMA_VERSION
            and not present_tables
            and stale_manifest_count == 0
        ),
    }


def _blob_row(conn, blob_id: str):
    return conn.execute("select * from blobs where blob_id = ?", (blob_id,)).fetchone()


def _snapshot_row(conn, snapshot_id: str):
    return conn.execute("select * from snapshots where snapshot_id = ?", (snapshot_id,)).fetchone()


def _blob_bytes_by_id(ctx: RepoContext, conn, blob_id: str, *, seen_blob_ids: set[str] | None = None) -> bytes:
    row = _blob_row(conn, blob_id)
    if row is None:
        raise KeyError(f"Unknown blob: {blob_id}")
    return _blob_bytes_by_row(ctx, conn, row, seen_blob_ids=seen_blob_ids)


def _blob_bytes_by_row(ctx: RepoContext, conn, row, *, seen_blob_ids: set[str] | None = None) -> bytes:
    blob_id = row["blob_id"]
    visited = set(seen_blob_ids or ())
    if blob_id in visited:
        raise ValueError(f"Cyclic blob resolution detected for {blob_id}")
    visited.add(blob_id)
    if row["pack_id"]:
        pack_row = conn.execute("select * from packs where pack_id = ?", (row["pack_id"],)).fetchone()
        if pack_row is None:
            raise FileNotFoundError(f"Missing pack metadata for {row['pack_id']}")
        pack_abs = ctx.root / pack_row["pack_path"]
        return read_pack_entry(
            pack_abs,
            _blob_pack_entry_name(row),
            resolve_base_blob=lambda base_blob_id: _blob_bytes_by_id(ctx, conn, base_blob_id, seen_blob_ids=visited),
        )
    raise FileNotFoundError(f"Packed blob payload not available for {row['blob_id']}")


def _parent_delta_candidates(
    ctx: RepoContext,
    conn,
    parent_snapshot_id: str | None,
    paths: set[str],
) -> dict[str, dict[str, Any]]:
    if not parent_snapshot_id or not paths:
        return {}
    placeholders = ", ".join("?" for _ in paths)
    rows = conn.execute(
        f"""
        select sf.path, b.blob_id, b.pack_chain_depth
        from snapshot_files sf
        join blobs b on b.blob_id = sf.blob_id
        where sf.snapshot_id = ?
          and sf.path in ({placeholders})
        order by sf.path
        """,
        (parent_snapshot_id, *sorted(paths)),
    ).fetchall()
    candidates: dict[str, dict[str, Any]] = {}
    for row in rows:
        blob_id = row["blob_id"]
        candidates[row["path"]] = {
            "blob_id": blob_id,
            "data": _blob_bytes_by_id(ctx, conn, blob_id),
            "chain_depth": int(row["pack_chain_depth"] or 0),
        }
    return candidates


def _write_packed_blobs(
    ctx: RepoContext,
    conn,
    snapshot_id: str,
    created_at: str,
    blob_items: list[dict[str, Any]],
    *,
    initial_by_path: dict[str, dict[str, Any]] | None = None,
) -> tuple[str, dict[str, dict[str, Any]]]:
    unique_blob_items: list[dict[str, Any]] = []
    seen_entry_names: set[str] = set()
    for item in blob_items:
        entry_name = str(item["entry_name"])
        if entry_name in seen_entry_names:
            continue
        seen_entry_names.add(entry_name)
        unique_blob_items.append(item)

    pack_seed = f"{snapshot_id}|{json.dumps(sorted(item['blob_id'] for item in unique_blob_items))}"
    pack_id = f"PCK-{hashlib.sha256(pack_seed.encode('utf-8')).hexdigest()[:12].upper()}"
    pack_abs = _pack_path(ctx, pack_id)
    members = build_pack_members(unique_blob_items, initial_by_path=initial_by_path)
    members_by_blob_id = {member["blob_id"]: member for member in members}
    archive_stats = write_pack_archive(pack_abs, pack_id, created_at, members)
    existing_pack = conn.execute("select pack_id from packs where pack_id = ?", (pack_id,)).fetchone()
    if existing_pack is None:
        conn.execute(
            """
            insert into packs(pack_id, status, member_count, total_bytes, pack_path, pack_format, pack_index_entry_name, pack_index_checksum, created_at)
            values (?, 'ready', ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                pack_id,
                archive_stats["member_count"],
                archive_stats["total_bytes"],
                str(pack_abs.relative_to(ctx.root)),
                archive_stats["pack_format"],
                archive_stats["pack_index_entry_name"],
                archive_stats["pack_index_checksum"],
                created_at,
            ),
        )
    return pack_id, members_by_blob_id


def _read_blob_bytes(ctx: RepoContext, blob_id: str) -> bytes:
    conn = connect_sqlite(ctx.content_db_path)
    data = _blob_bytes_by_id(ctx, conn, blob_id)
    conn.close()
    return data


def ensure_blob_bytes(ctx: RepoContext, data: bytes, *, path_hint: str | None = None) -> str:
    digest = hashlib.sha256(data).hexdigest()
    blob_id = f"BLB-{digest[:20]}"
    conn = connect_sqlite(ctx.content_db_path)
    existing = _blob_row(conn, blob_id)
    if existing is not None:
        conn.close()
        return blob_id
    created_at = utc_now()
    blob_storage = _blob_path(ctx, blob_id)
    blob_item = {
        "blob_id": blob_id,
        "sha256": digest,
        "storage_path": str(blob_storage.relative_to(ctx.root)),
        "size_bytes": len(data),
        "data": data,
        "entry_name": f"blobs/{blob_id}",
        "path_hint": str(path_hint or ""),
    }
    pack_id, members_by_blob_id = _write_packed_blobs(
        ctx,
        conn,
        f"BLOB-{blob_id}",
        created_at,
        [blob_item],
    )
    member = members_by_blob_id[blob_id]
    entry_type = member.get("entry_type", "full")
    _insert_blob_record(
        conn,
        blob_id=blob_id,
        sha256=digest,
        storage_path=blob_item["storage_path"],
        size_bytes=len(data),
        pack_id=pack_id,
        pack_entry_type=entry_type,
        pack_base_blob_id=member.get("base_blob_id"),
        pack_chain_depth=int(member.get("chain_depth", 0) or 0),
        pruned_at=None,
        created_at=created_at,
        pack_entry_name=None,
        packed_at=None,
    )
    conn.commit()
    conn.close()
    return blob_id


def storage_stats(ctx: RepoContext) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    counts = conn.execute(
        """
        select
            count(*) as total_blobs,
            coalesce(sum(case when pack_id is not null then 1 else 0 end), 0) as packed_blob_count,
            coalesce(sum(case when pack_id is not null and coalesce(pack_entry_type, 'full') = 'full' then 1 else 0 end), 0) as packed_full_blob_count,
            coalesce(sum(case when pack_id is not null and coalesce(pack_entry_type, 'full') = 'delta' then 1 else 0 end), 0) as packed_delta_blob_count,
            coalesce(sum(size_bytes), 0) as total_blob_bytes,
            coalesce(sum(case when pack_id is not null then size_bytes else 0 end), 0) as packed_blob_bytes,
            coalesce(sum(case when pack_id is not null and coalesce(pack_entry_type, 'full') = 'full' then size_bytes else 0 end), 0) as packed_full_blob_bytes,
            coalesce(sum(case when pack_id is not null and coalesce(pack_entry_type, 'full') = 'delta' then size_bytes else 0 end), 0) as packed_delta_blob_bytes
        from blobs
        """
    ).fetchone()
    snapshot_count = conn.execute("select count(*) as c from snapshots").fetchone()["c"]
    reachable = conn.execute("select count(distinct blob_id) as c from snapshot_files").fetchone()["c"]
    unreachable = conn.execute(
        "select count(*) as c from blobs b where not exists (select 1 from snapshot_files sf where sf.blob_id = b.blob_id)"
    ).fetchone()["c"]
    tree_stats = _tree_metadata_stats(conn)
    pack_rows = [dict(r) for r in conn.execute("select * from packs order by created_at desc")]
    tree_pack_rows = [dict(r) for r in conn.execute("select * from tree_packs order by created_at desc")]
    schema_cleanup_summary = _schema_cleanup_summary(conn)
    conn.close()
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
    validation_summary = build_storage_validation_summary(
        packed_blob_count=int(counts["packed_blob_count"]),
        packed_full_blob_count=int(counts["packed_full_blob_count"]),
        packed_delta_blob_count=int(counts["packed_delta_blob_count"]),
        pack_count=len(pack_rows),
        pack_index_error_count=int(pack_summary["index_error_count"]),
        tree_pack_index_error_count=int(tree_pack_summary["index_error_count"]),
        storage_savings_ratio=float(efficiency_summary["storage_savings_ratio"]),
        unreferenced_blob_count=int(unreachable),
        unreferenced_tree_count=int(tree_stats["unreachable_tree_count"]) + int(tree_stats["orphan_tree_pack_count"]),
    )
    return {
        "snapshot_count": snapshot_count,
        "reachable_blob_count": reachable,
        "unreachable_blob_count": unreachable,
        "tree_count": tree_stats["tree_count"],
        "tree_entry_count": tree_stats["tree_entry_count"],
        "reachable_tree_count": tree_stats["reachable_tree_count"],
        "reachable_tree_entry_count": tree_stats["reachable_tree_entry_count"],
        "unreachable_tree_count": tree_stats["unreachable_tree_count"],
        "unreachable_tree_entry_count": tree_stats["unreachable_tree_entry_count"],
        "tree_pack_count": tree_stats["tree_pack_count"],
        "reachable_tree_pack_count": tree_stats["reachable_tree_pack_count"],
        "orphan_tree_pack_count": tree_stats["orphan_tree_pack_count"],
        "total_blobs": counts["total_blobs"],
        "packed_blob_count": counts["packed_blob_count"],
        "packed_full_blob_count": counts["packed_full_blob_count"],
        "packed_delta_blob_count": counts["packed_delta_blob_count"],
        "total_blob_bytes": counts["total_blob_bytes"],
        "packed_blob_bytes": counts["packed_blob_bytes"],
        "packed_full_blob_bytes": counts["packed_full_blob_bytes"],
        "packed_delta_blob_bytes": counts["packed_delta_blob_bytes"],
        "pack_count": len(pack_rows),
        "pack_archive_bytes": pack_summary["pack_archive_bytes"],
        "tree_pack_archive_bytes": tree_pack_summary["archive_bytes"],
        "schema_cleanup_summary": schema_cleanup_summary,
        "optimization_summary": optimization_summary,
        "efficiency_summary": efficiency_summary,
        "metadata_summary": {
            **tree_stats,
            "tree_pack_archive_bytes": tree_pack_summary["archive_bytes"],
            "tree_pack_index_error_count": tree_pack_summary["index_error_count"],
        },
        "validation_summary": validation_summary,
        "packs": pack_rows,
        "tree_packs": tree_pack_rows,
    }


def _repo_status_storage_counts(ctx: RepoContext) -> dict[str, int]:
    conn = connect_sqlite(ctx.content_db_path)
    row = conn.execute(
        """
        select
            (select count(*) from snapshots) as snapshot_count,
            (select count(*) from packs) as pack_count,
            coalesce(sum(case when pack_id is not null then 1 else 0 end), 0) as packed_blob_count
        from blobs
        """
    ).fetchone()
    conn.close()
    return {
        "snapshot_count": int(row["snapshot_count"] or 0),
        "pack_count": int(row["pack_count"] or 0),
        "packed_blob_count": int(row["packed_blob_count"] or 0),
    }


def create_pack(ctx: RepoContext, *, max_members: int | None = None, repack: bool = False) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
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
                b.pack_id,
                min(sf.path) as path_hint,
                min(s.created_at) as first_seen_at
            from blobs b
            join snapshot_files sf on sf.blob_id = b.blob_id
            join snapshots s on s.snapshot_id = sf.snapshot_id
            where """
            + where_clause
            + """
            group by b.blob_id, b.sha256, b.storage_path, b.size_bytes, b.pack_id
            order by path_hint asc, first_seen_at asc, b.blob_id asc
            """
        )
    ]
    if max_members is not None:
        rows = rows[: max(0, max_members)]
    if not rows:
        stats = storage_stats(ctx)
        conn.close()
        return {"created": False, "reason": "no_reachable_blobs" if repack else "no_unpacked_reachable_blobs", "stats": stats, "repack": repack}

    pack_seed = "|".join(row["blob_id"] for row in rows) + "|" + utc_now() + "|" + secrets.token_hex(8)
    pack_id = f"PCK-{hashlib.sha256(pack_seed.encode('utf-8')).hexdigest()[:12].upper()}"
    pack_abs = _pack_path(ctx, pack_id)
    blob_items: list[dict[str, object]] = []
    for row in rows:
        data = _blob_bytes_by_row(ctx, conn, row)
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
    archive_stats = write_pack_archive(pack_abs, pack_id, now := utc_now(), members)
    conn.execute(
        "insert into packs(pack_id, status, member_count, total_bytes, pack_path, pack_format, pack_index_entry_name, pack_index_checksum, created_at) values (?, 'ready', ?, ?, ?, ?, ?, ?, ?)",
        (
            pack_id,
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
        entry_type = member.get("entry_type", "full")
        base_blob_id = member.get("base_blob_id")
        chain_depth = int(member.get("chain_depth", 0) or 0)
        _update_blob_pack_metadata(
            conn,
            pack_id=pack_id,
            pack_entry_type=entry_type,
            pack_base_blob_id=base_blob_id,
            pack_chain_depth=chain_depth,
            blob_id=row["blob_id"],
            pack_entry_name=None,
            packed_at=None,
        )
    conn.commit()
    conn.close()
    return {
        "created": True,
        "pack_id": pack_id,
        "pack_path": str(pack_abs.relative_to(ctx.root)),
        "pack_format": archive_stats["pack_format"],
        "pack_index_entry_name": archive_stats["pack_index_entry_name"],
        "pack_index_checksum": archive_stats["pack_index_checksum"],
        "member_count": archive_stats["member_count"],
        "total_bytes": archive_stats["total_bytes"],
        "archive_bytes": archive_stats["archive_bytes"],
        "repack": repack,
        "stats": storage_stats(ctx),
    }


def gc_content(
    ctx: RepoContext,
    *,
    prune_unreferenced: bool = True,
    prune_orphan_packs: bool = True,
) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    removed_unreferenced = 0
    removed_unreachable_trees = 0
    removed_unreachable_tree_entries = 0
    removed_orphan_packs = 0
    removed_orphan_tree_packs = 0

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

        unreachable_tree_ids = _unreachable_tree_ids(conn)
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
            remaining_tree_ids = [
                str(row["tree_id"])
                for row in conn.execute("select tree_id from trees order by tree_id asc").fetchall()
            ]
            if remaining_tree_ids:
                conn.execute(
                    "update trees set tree_pack_id = null, tree_pack_checksum = null"
                )
                if _tree_pack_entry_name_column_present(conn):
                    conn.execute("update trees set tree_pack_entry_name = null")
                if _tree_packed_at_column_present(conn):
                    conn.execute("update trees set tree_packed_at = null")
                _write_tree_pack(
                    ctx,
                    conn,
                    remaining_tree_ids,
                    created_at=utc_now(),
                    seed_hint="tree-metadata-gc-repack",
                )

    if prune_orphan_packs:
        pack_rows = [dict(r) for r in conn.execute("select * from packs order by created_at asc")]
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
    conn.close()
    return {
        "removed_unreferenced_blob_count": removed_unreferenced,
        "removed_unreachable_tree_count": removed_unreachable_trees,
        "removed_unreachable_tree_entry_count": removed_unreachable_tree_entries,
        "removed_orphan_pack_count": removed_orphan_packs,
        "removed_orphan_tree_pack_count": removed_orphan_tree_packs,
        "stats": storage_stats(ctx),
    }
