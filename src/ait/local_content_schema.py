from __future__ import annotations

import secrets
import sqlite3
import time
from pathlib import Path
from typing import Callable

from ait_protocol.common import connect_sqlite, utc_now
from ait_storage.revision_trees import build_tree_records

from .local_content_lines import read_ref, write_ref
from .local_content_pack_runtime import (
    _blob_storage_kind_column_present,
    _cleanup_manifest_files,
    _count_stale_manifest_paths,
    _drop_column_if_exists,
    _insert_tree_records,
    _legacy_pack_metadata_columns_present,
    _local_server_catalog_tables_ready_for_drop,
    _sync_tree_pack_metadata,
    _table_columns,
    _tree_entry_blob_size_sql,
    _tree_entry_size_column_present,
)
from .repo_paths import RepoContext

SCHEMA = """
create table if not exists lines (
    line_name text primary key,
    status text not null default 'active',
    archived_at text,
    created_at text not null,
    updated_at text not null
);

create table if not exists blobs (
    blob_id text primary key,
    sha256 text not null unique,
    storage_path text not null,
    size_bytes integer not null,
    pack_id text,
    pack_entry_type text,
    pack_base_blob_id text,
    pack_chain_depth integer,
    pruned_at text,
    created_at text not null
);

create table if not exists snapshots (
    snapshot_id text primary key,
    parent_snapshot_id text,
    root_tree_id text,
    manifest_hash text not null default '',
    manifest_path text not null default '',
    message text,
    line_name text not null,
    snapshot_kind text not null default 'line',
    file_count integer not null,
    total_bytes integer not null,
    created_at text not null
);

create table if not exists stashes (
    stash_id text primary key,
    snapshot_id text not null,
    source_line_name text not null,
    base_snapshot_id text,
    message text,
    workspace_cleared integer not null default 1,
    created_at text not null
);

create table if not exists trees (
    tree_id text primary key,
    entry_count integer not null,
    tree_pack_id text,
    tree_pack_checksum text,
    created_at text not null
);

create table if not exists tree_entries (
    tree_id text not null references trees(tree_id) on delete cascade,
    entry_name text not null,
    entry_type text not null,
    target_id text not null,
    mode text not null,
    primary key (tree_id, entry_name)
);

create table if not exists packs (
    pack_id text primary key,
    status text not null,
    member_count integer not null,
    total_bytes integer not null,
    pack_path text,
    pack_format text not null default 'ait-pack-v1',
    pack_index_entry_name text,
    pack_index_checksum text,
    created_at text not null
);

create table if not exists tree_packs (
    pack_id text primary key,
    status text not null,
    tree_count integer not null,
    total_bytes integer not null,
    pack_path text,
    pack_format text not null default 'ait-tree-pack-v1',
    pack_index_entry_name text,
    pack_index_checksum text,
    created_at text not null
);
"""

LOCAL_CONTENT_SCHEMA_VERSION = 4
_LOCAL_CONTENT_INIT_MIGRATION_BACKUP_PREFIX = "content.db.before-local-content-init-migration"


def _snapshot_files_view_sql(conn) -> str:
    size_expr = _tree_entry_blob_size_sql(conn, "te", "b")
    return f"""
    create view if not exists snapshot_files as
    with recursive snapshot_walk(snapshot_id, prefix, entry_name, entry_type, target_id, size_bytes, mode) as (
        select
            s.snapshot_id,
            '' as prefix,
            te.entry_name,
            te.entry_type,
            te.target_id,
            {size_expr},
            te.mode
        from snapshots s
        join tree_entries te on te.tree_id = s.root_tree_id
        left join blobs b on b.blob_id = te.target_id
      union all
        select
            sw.snapshot_id,
            sw.prefix || sw.entry_name || '/',
            te.entry_name,
            te.entry_type,
            te.target_id,
            {size_expr},
            te.mode
        from snapshot_walk sw
        join tree_entries te on te.tree_id = sw.target_id
        left join blobs b on b.blob_id = te.target_id
        where sw.entry_type = 'tree'
    )
    select
        snapshot_id,
        prefix || entry_name as path,
        target_id as blob_id,
        size_bytes,
        mode
    from snapshot_walk
    where entry_type = 'blob'
    """


def _snapshot_files_object_type(conn) -> str | None:
    row = conn.execute(
        "select type from sqlite_master where name = 'snapshot_files' and type in ('table', 'view')"
    ).fetchone()
    return row["type"] if row else None


def _snapshot_files_view_needs_refresh(conn) -> bool:
    row = conn.execute(
        "select sql from sqlite_master where name = 'snapshot_files' and type = 'view'"
    ).fetchone()
    if row is None:
        return False
    sql = " ".join(str(row["sql"] or "").split()).lower()
    expected_expr = " ".join(_tree_entry_blob_size_sql(conn, "te", "b").split()).lower()
    return (
        "left join blobs b on b.blob_id = te.target_id" not in sql
        or expected_expr not in sql
    )


def _content_db_explicit_init_migration_needed(conn) -> bool:
    if _snapshot_files_object_type(conn) == "table":
        return True
    if _tree_entry_size_column_present(conn):
        return True
    if _blob_storage_kind_column_present(conn):
        return True
    if _legacy_pack_metadata_columns_present(conn):
        return True
    if _snapshot_files_view_needs_refresh(conn):
        return True
    if _local_server_catalog_tables_ready_for_drop(conn):
        return True
    return _count_stale_manifest_paths(conn) > 0


def _write_init_migration_backup(conn, ctx: RepoContext) -> Path:
    backup_root = ctx.ait_dir / "repair_backups"
    backup_root.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    backup_path = backup_root / (
        f"{_LOCAL_CONTENT_INIT_MIGRATION_BACKUP_PREFIX}.{stamp}.{secrets.token_hex(2)}.bak"
    )
    backup_conn = sqlite3.connect(backup_path)
    try:
        conn.backup(backup_conn)
    finally:
        backup_conn.close()
    return backup_path


def _drop_empty_local_server_catalog_tables(conn) -> tuple[str, ...]:
    droppable = _local_server_catalog_tables_ready_for_drop(conn)
    for table_name in droppable:
        conn.execute(f"drop table if exists {table_name}")
    return droppable


def _drop_legacy_pack_metadata_columns(conn) -> tuple[tuple[str, str], ...]:
    dropped: list[tuple[str, str]] = []
    for table_name, column_name in _legacy_pack_metadata_columns_present(conn):
        if _drop_column_if_exists(conn, table_name, column_name):
            dropped.append((table_name, column_name))
    return tuple(dropped)


def _backfill_blob_pack_entry_type_from_storage_kind(conn) -> None:
    if not _blob_storage_kind_column_present(conn):
        return
    if "pack_entry_type" not in _table_columns(conn, "blobs"):
        return
    conn.execute(
        """
        update blobs
        set pack_entry_type = case
            when storage_kind = 'pack_delta' then 'delta'
            else 'full'
        end
        where coalesce(pack_id, '') != ''
          and coalesce(pack_entry_type, '') = ''
        """
    )


def _drop_redundant_blob_storage_kind_column(conn) -> bool:
    return _drop_column_if_exists(conn, "blobs", "storage_kind")


def _set_local_content_schema_version(conn, version: int = LOCAL_CONTENT_SCHEMA_VERSION) -> None:
    conn.execute(f"pragma user_version = {int(version)}")


def _migrate_snapshot_metadata(conn, ctx: RepoContext | None = None) -> None:
    snapshot_files_type = _snapshot_files_object_type(conn)
    snapshots = [
        dict(row)
        for row in conn.execute(
            "select snapshot_id from snapshots where coalesce(root_tree_id, '') = '' order by created_at asc, snapshot_id asc"
        ).fetchall()
    ]
    if snapshots:
        if snapshot_files_type != "table":
            raise RuntimeError("Missing legacy snapshot_files table for tree metadata migration")
        migrated_at = utc_now()
        for snap in snapshots:
            rows = [
                dict(row)
                for row in conn.execute(
                    "select path, blob_id, size_bytes, mode from snapshot_files where snapshot_id = ? order by path asc",
                    (snap["snapshot_id"],),
                ).fetchall()
            ]
            root_tree_id, tree_rows, tree_entry_rows = build_tree_records(rows)
            _insert_tree_records(conn, tree_rows, tree_entry_rows, migrated_at)
            conn.execute(
                """
                update snapshots
                set root_tree_id = ?,
                    manifest_hash = case when coalesce(manifest_hash, '') = '' then ? else manifest_hash end,
                    manifest_path = case when coalesce(manifest_path, '') = '' then ? else manifest_path end
                where snapshot_id = ?
                """,
                (root_tree_id, root_tree_id, f"trees/{root_tree_id}", snap["snapshot_id"]),
            )
    remaining = conn.execute(
        "select count(*) as c from snapshots where coalesce(root_tree_id, '') = ''"
    ).fetchone()["c"]
    if snapshot_files_type == "table" and remaining == 0:
        conn.execute("drop table snapshot_files")
        snapshot_files_type = None
    if snapshot_files_type == "view" and (
        _tree_entry_size_column_present(conn) or _snapshot_files_view_needs_refresh(conn)
    ):
        conn.execute("drop view if exists snapshot_files")
        snapshot_files_type = None
    if remaining == 0 and _tree_entry_size_column_present(conn):
        _drop_column_if_exists(conn, "tree_entries", "size_bytes")
    if snapshot_files_type is None:
        conn.execute(_snapshot_files_view_sql(conn))
        _cleanup_manifest_files(ctx)
    elif snapshot_files_type == "view":
        _cleanup_manifest_files(ctx)
    _sync_tree_pack_metadata(conn, ctx)
    _drop_legacy_pack_metadata_columns(conn)


def _initialize_schema(
    conn,
    ctx: RepoContext | None = None,
    *,
    migrate_snapshot_metadata_fn: Callable[[object, RepoContext | None], None] = _migrate_snapshot_metadata,
) -> None:
    conn.executescript(SCHEMA)
    line_cols = {row["name"] for row in conn.execute("pragma table_info(lines)")}
    if "status" not in line_cols:
        conn.execute("alter table lines add column status text not null default 'active'")
    if "archived_at" not in line_cols:
        conn.execute("alter table lines add column archived_at text")
    snapshot_cols = {row["name"] for row in conn.execute("pragma table_info(snapshots)")}
    if "root_tree_id" not in snapshot_cols:
        conn.execute("alter table snapshots add column root_tree_id text")
    if "snapshot_kind" not in snapshot_cols:
        conn.execute("alter table snapshots add column snapshot_kind text not null default 'line'")
    stash_cols = {row["name"] for row in conn.execute("pragma table_info(stashes)")}
    if "workspace_cleared" not in stash_cols:
        conn.execute("alter table stashes add column workspace_cleared integer not null default 1")
    tree_cols = {row["name"] for row in conn.execute("pragma table_info(trees)")}
    if "tree_pack_id" not in tree_cols:
        conn.execute("alter table trees add column tree_pack_id text")
    if "tree_pack_checksum" not in tree_cols:
        conn.execute("alter table trees add column tree_pack_checksum text")
    blob_cols = {row["name"] for row in conn.execute("pragma table_info(blobs)")}
    if "pack_id" not in blob_cols:
        conn.execute("alter table blobs add column pack_id text")
    if "pack_entry_type" not in blob_cols:
        conn.execute("alter table blobs add column pack_entry_type text")
    if "pack_base_blob_id" not in blob_cols:
        conn.execute("alter table blobs add column pack_base_blob_id text")
    if "pack_chain_depth" not in blob_cols:
        conn.execute("alter table blobs add column pack_chain_depth integer")
    if "pruned_at" not in blob_cols:
        conn.execute("alter table blobs add column pruned_at text")
    conn.execute("create index if not exists idx_blobs_pack_id on blobs(pack_id)")
    conn.execute("create index if not exists idx_snapshots_kind_created_at on snapshots(snapshot_kind, created_at)")
    conn.execute("create index if not exists idx_stashes_created_at on stashes(created_at desc, stash_id desc)")
    conn.execute("create index if not exists idx_stashes_snapshot_id on stashes(snapshot_id)")
    pack_cols = {row["name"] for row in conn.execute("pragma table_info(packs)")}
    if "pack_format" not in pack_cols:
        conn.execute("alter table packs add column pack_format text not null default 'ait-pack-v1'")
    if "pack_index_entry_name" not in pack_cols:
        conn.execute("alter table packs add column pack_index_entry_name text")
    if "pack_index_checksum" not in pack_cols:
        conn.execute("alter table packs add column pack_index_checksum text")
    tree_pack_cols = {row["name"] for row in conn.execute("pragma table_info(tree_packs)")}
    if "pack_format" not in tree_pack_cols:
        conn.execute("alter table tree_packs add column pack_format text not null default 'ait-tree-pack-v1'")
    if "pack_index_entry_name" not in tree_pack_cols:
        conn.execute("alter table tree_packs add column pack_index_entry_name text")
    if "pack_index_checksum" not in tree_pack_cols:
        conn.execute("alter table tree_packs add column pack_index_checksum text")
    conn.execute("create index if not exists idx_tree_entries_target on tree_entries(target_id)")
    conn.execute("create index if not exists idx_trees_tree_pack_id on trees(tree_pack_id)")
    migrate_snapshot_metadata_fn(conn, ctx)
    _drop_empty_local_server_catalog_tables(conn)
    _backfill_blob_pack_entry_type_from_storage_kind(conn)
    _drop_redundant_blob_storage_kind_column(conn)
    _set_local_content_schema_version(conn)
    conn.commit()


def initialize(ctx: RepoContext, default_line: str) -> None:
    existing_db = ctx.content_db_path.exists() and ctx.content_db_path.stat().st_size > 0
    existing_line_head = read_ref(ctx, default_line)
    conn = connect_sqlite(ctx.content_db_path)
    if existing_db and _content_db_explicit_init_migration_needed(conn):
        _write_init_migration_backup(conn, ctx)
    _initialize_schema(conn, ctx)
    now = utc_now()
    conn.execute(
        "insert or ignore into lines(line_name, status, archived_at, created_at, updated_at) values (?, 'active', null, ?, ?)",
        (default_line, now, now),
    )
    conn.commit()
    conn.close()
    write_ref(ctx, default_line, existing_line_head)
