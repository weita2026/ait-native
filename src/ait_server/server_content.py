from __future__ import annotations

import base64
import hashlib
import json
import time
import threading
import uuid
from pathlib import Path
from typing import Any, Callable, Optional, TypeVar

from ait_protocol.common import (
    DEFAULT_ID_NAMESPACE_PREFIX,
    StorageIngestMode,
    encode_ref_name,
    normalize_id_namespace_prefix,
    normalize_policy,
    normalize_storage_ingest_mode,
    utc_now,
)
from ait_storage.packfiles import (
    build_pack_members,
    build_storage_validation_summary,
    read_pack_entry,
    read_pack_index,
    summarize_pack_archives,
    write_pack_archive,
)
from ait_storage.revision_trees import build_snapshot_id, build_tree_records
from .server_db import (
    connect_server_plane,
    enable_sqlite_wal,
    ensure_schema_version,
    postgres_advisory_lock,
    read_server_plane,
    write_server_plane,
)
from .server_paths import ServerContext
from ait_storage.treepacks import (
    build_tree_pack_members,
    read_tree_pack_index,
    read_tree_pack_tree,
    summarize_tree_pack_archives,
    tree_pack_manifest_path,
    write_tree_pack_archive,
)

_T = TypeVar("_T")


class RepositoryNamespacePrefixConflictError(ValueError):
    pass


def _elapsed_ms(start: float, end: float | None = None) -> float:
    finished = time.perf_counter() if end is None else end
    return round((finished - start) * 1000.0, 3)

SCHEMA_SQLITE = """
create table if not exists repositories (
    repo_name text primary key,
    repo_id text not null unique,
    default_line text not null,
    lifecycle_state text not null default 'active',
    id_namespace_prefix text not null default 'AIT',
    policy_json text not null default '{}',
    created_at text not null,
    updated_at text not null
);

create table if not exists lines (
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    line_name text not null,
    head_snapshot_id text,
    status text not null default 'active',
    archived_at text,
    created_at text not null,
    updated_at text not null,
    primary key (repo_id, line_name)
);
create index if not exists idx_lines_repo on lines(repo_name, line_name);
create index if not exists idx_lines_repo_id on lines(repo_id, line_name);

create table if not exists blobs (
    blob_id text primary key,
    sha256 text not null unique,
    storage_path text not null,
    size_bytes integer not null,
    storage_kind text not null default 'pack_full',
    pack_id text,
    pack_entry_name text,
    pack_entry_type text,
    pack_base_blob_id text,
    pack_chain_depth integer,
    packed_at text,
    pruned_at text,
    created_at text not null
);

create table if not exists snapshots (
    snapshot_id text primary key,
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    parent_snapshot_id text,
    root_tree_id text,
    manifest_hash text not null default '',
    manifest_path text not null default '',
    message text,
    line_name text,
    file_count integer not null,
    total_bytes integer not null,
    created_at text not null
);
create index if not exists idx_snapshots_repo_created on snapshots(repo_name, created_at desc);

create table if not exists trees (
    tree_id text primary key,
    entry_count integer not null,
    tree_pack_id text,
    tree_pack_entry_name text,
    tree_pack_checksum text,
    tree_packed_at text,
    created_at text not null
);

create table if not exists tree_entries (
    tree_id text not null references trees(tree_id) on delete cascade,
    entry_name text not null,
    entry_type text not null,
    target_id text not null,
    size_bytes integer,
    mode text not null,
    primary key (tree_id, entry_name)
);
create index if not exists idx_tree_entries_target on tree_entries(target_id);

create table if not exists packs (
    pack_id text primary key,
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    status text not null,
    member_count integer not null,
    total_bytes integer not null,
    pack_path text,
    pack_format text not null default 'ait-pack-v1',
    pack_index_entry_name text,
    pack_index_checksum text,
    created_at text not null
);
create index if not exists idx_packs_repo on packs(repo_name, created_at desc);

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

create table if not exists repository_groups (
    group_id text primary key,
    title text not null,
    sort_index integer not null,
    system_slug text unique,
    created_at text not null,
    updated_at text not null
);
create index if not exists idx_repository_groups_sort on repository_groups(sort_index, group_id);

create table if not exists repository_group_memberships (
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text primary key,
    group_id text not null references repository_groups(group_id) on delete cascade,
    sort_index integer not null,
    created_at text not null,
    updated_at text not null
);
create index if not exists idx_repository_group_memberships_group on repository_group_memberships(group_id, sort_index, repo_name);
"""

SCHEMA_POSTGRES = """
create table if not exists schema_versions (
    plane text primary key,
    version integer not null,
    description text not null,
    applied_at timestamptz not null,
    checked_at timestamptz not null
);

create table if not exists repositories (
    repo_name text primary key,
    repo_id text not null unique,
    default_line text not null,
    lifecycle_state text not null default 'active',
    id_namespace_prefix text not null default 'AIT',
    policy_json text not null default '{}',
    created_at timestamptz not null,
    updated_at timestamptz not null
);

create table if not exists lines (
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    line_name text not null,
    head_snapshot_id text,
    status text not null default 'active',
    archived_at timestamptz,
    created_at timestamptz not null,
    updated_at timestamptz not null,
    primary key (repo_id, line_name)
);
create index if not exists idx_lines_repo on lines(repo_name, line_name);
create index if not exists idx_lines_repo_id on lines(repo_id, line_name);

create table if not exists blobs (
    blob_id text primary key,
    sha256 text not null unique,
    storage_path text not null,
    size_bytes bigint not null,
    storage_kind text not null default 'pack_full',
    pack_id text,
    pack_entry_name text,
    pack_entry_type text,
    pack_base_blob_id text,
    pack_chain_depth integer,
    packed_at timestamptz,
    pruned_at timestamptz,
    created_at timestamptz not null
);
create index if not exists idx_blobs_pack_id on blobs(pack_id);

create table if not exists snapshots (
    snapshot_id text primary key,
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    parent_snapshot_id text,
    root_tree_id text,
    manifest_hash text not null default '',
    manifest_path text not null default '',
    message text,
    line_name text,
    file_count integer not null,
    total_bytes bigint not null,
    created_at timestamptz not null
);
create index if not exists idx_snapshots_repo_created on snapshots(repo_name, created_at desc);

create table if not exists trees (
    tree_id text primary key,
    entry_count integer not null,
    tree_pack_id text,
    tree_pack_entry_name text,
    tree_pack_checksum text,
    tree_packed_at timestamptz,
    created_at timestamptz not null
);

create table if not exists tree_entries (
    tree_id text not null references trees(tree_id) on delete cascade,
    entry_name text not null,
    entry_type text not null,
    target_id text not null,
    size_bytes bigint,
    mode text not null,
    primary key (tree_id, entry_name)
);
create index if not exists idx_tree_entries_target on tree_entries(target_id);

create table if not exists packs (
    pack_id text primary key,
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    status text not null,
    member_count integer not null,
    total_bytes bigint not null,
    pack_path text,
    pack_format text not null default 'ait-pack-v1',
    pack_index_entry_name text,
    pack_index_checksum text,
    created_at timestamptz not null
);
create index if not exists idx_packs_repo on packs(repo_name, created_at desc);

create table if not exists tree_packs (
    pack_id text primary key,
    status text not null,
    tree_count integer not null,
    total_bytes bigint not null,
    pack_path text,
    pack_format text not null default 'ait-tree-pack-v1',
    pack_index_entry_name text,
    pack_index_checksum text,
    created_at timestamptz not null
);

create table if not exists repository_groups (
    group_id text primary key,
    title text not null,
    sort_index integer not null,
    system_slug text unique,
    created_at timestamptz not null,
    updated_at timestamptz not null
);
create index if not exists idx_repository_groups_sort on repository_groups(sort_index, group_id);

create table if not exists repository_group_memberships (
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text primary key,
    group_id text not null references repository_groups(group_id) on delete cascade,
    sort_index integer not null,
    created_at timestamptz not null,
    updated_at timestamptz not null
);
create index if not exists idx_repository_group_memberships_group on repository_group_memberships(group_id, sort_index, repo_name);
"""

DEFAULT_REPOSITORY_GROUP_TITLE = "Main group"
DEFAULT_REPOSITORY_GROUP_SYSTEM_SLUG = "main-group"
REPOSITORY_ID_UNIQUE_INDEX = "idx_repositories_repo_id_unique"
REPOSITORY_NAMESPACE_PREFIX_UNIQUE_INDEX = "idx_repositories_namespace_prefix_unique"
LINES_REPO_ID_INDEX = "idx_lines_repo_id"
SNAPSHOTS_REPO_ID_CREATED_INDEX = "idx_snapshots_repo_id_created"
PACKS_REPO_ID_INDEX = "idx_packs_repo_id"
REPOSITORY_GROUP_MEMBERSHIPS_REPO_ID_UNIQUE_INDEX = "idx_repository_group_memberships_repo_id_unique"
REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_ID_INDEX = "idx_repository_group_memberships_group_repo_id"
_POSTGRES_CONTENT_SCHEMA_READY: set[tuple[str, str]] = set()
_POSTGRES_CONTENT_SCHEMA_READY_GUARD = threading.RLock()
REPOSITORY_LIFECYCLE_STATES = {"active", "retiring"}


def _connect(ctx: ServerContext):
    return connect_server_plane(ctx, "content")


def read(
    ctx: ServerContext,
    callback: Callable[[Any], _T],
    *,
    lock_timeout_ms: int | None = None,
    statement_timeout_ms: int | None = None,
) -> _T:
    return read_server_plane(
        ctx,
        "content",
        callback,
        lock_timeout_ms=lock_timeout_ms,
        statement_timeout_ms=statement_timeout_ms,
    )


def write(
    ctx: ServerContext,
    callback: Callable[[Any], _T],
    *,
    lock_timeout_ms: int | None = None,
    statement_timeout_ms: int | None = None,
) -> _T:
    return write_server_plane(
        ctx,
        "content",
        callback,
        lock_timeout_ms=lock_timeout_ms,
        statement_timeout_ms=statement_timeout_ms,
    )


def connect(ctx: ServerContext):
    return _connect(ctx)


def _postgres_schema_ready_key(ctx: ServerContext) -> tuple[str, str] | None:
    if ctx.db_backend != "postgres":
        return None
    dsn = str(ctx.postgres_dsn or "").strip()
    schema = str(ctx.content_schema or "").strip()
    if not dsn or not schema:
        return None
    return (dsn, schema)


def reset_postgres_schema_ready_cache() -> None:
    with _POSTGRES_CONTENT_SCHEMA_READY_GUARD:
        _POSTGRES_CONTENT_SCHEMA_READY.clear()


def _mark_postgres_schema_ready(ctx: ServerContext) -> None:
    key = _postgres_schema_ready_key(ctx)
    if key is None:
        return
    with _POSTGRES_CONTENT_SCHEMA_READY_GUARD:
        _POSTGRES_CONTENT_SCHEMA_READY.add(key)


def _ensure_schema_postgres(conn, ctx: ServerContext) -> None:
    conn.executescript(SCHEMA_POSTGRES)
    _ensure_column(conn, ctx, "lines", "status", "text not null default 'active'")
    _ensure_column(conn, ctx, "lines", "archived_at", "timestamptz")
    _ensure_column(conn, ctx, "lines", "repo_id", "text")
    _ensure_column(conn, ctx, "repositories", "repo_id", "text")
    _ensure_column(conn, ctx, "repositories", "lifecycle_state", "text not null default 'active'")
    _ensure_column(conn, ctx, "repositories", "id_namespace_prefix", "text not null default 'AIT'")
    _ensure_column(conn, ctx, "repositories", "policy_json", "text not null default '{}'")
    _ensure_column(conn, ctx, "snapshots", "repo_id", "text")
    _ensure_column(conn, ctx, "snapshots", "root_tree_id", "text")
    _ensure_column(conn, ctx, "trees", "tree_pack_id", "text")
    _ensure_column(conn, ctx, "trees", "tree_pack_entry_name", "text")
    _ensure_column(conn, ctx, "trees", "tree_pack_checksum", "text")
    _ensure_column(conn, ctx, "trees", "tree_packed_at", "timestamptz")
    _ensure_column(conn, ctx, "blobs", "pack_entry_type", "text")
    _ensure_column(conn, ctx, "blobs", "pack_base_blob_id", "text")
    _ensure_column(conn, ctx, "blobs", "pack_chain_depth", "integer")
    _ensure_column(conn, ctx, "packs", "repo_id", "text")
    _ensure_column(conn, ctx, "packs", "pack_format", "text not null default 'ait-pack-v1'")
    _ensure_column(conn, ctx, "packs", "pack_index_entry_name", "text")
    _ensure_column(conn, ctx, "packs", "pack_index_checksum", "text")
    _ensure_column(conn, ctx, "tree_packs", "pack_format", "text not null default 'ait-tree-pack-v1'")
    _ensure_column(conn, ctx, "tree_packs", "pack_index_entry_name", "text")
    _ensure_column(conn, ctx, "tree_packs", "pack_index_checksum", "text")
    _ensure_column(conn, ctx, "repository_groups", "system_slug", "text")
    _ensure_column(conn, ctx, "repository_group_memberships", "repo_id", "text")
    conn.execute("create index if not exists idx_trees_tree_pack_id on trees(tree_pack_id)")
    conn.execute("create index if not exists idx_repository_groups_sort on repository_groups(sort_index, group_id)")
    conn.execute(
        "create index if not exists idx_repository_group_memberships_group on repository_group_memberships(group_id, sort_index, repo_name)"
    )
    _backfill_missing_repository_ids(conn)
    _backfill_content_plane_repo_ids(conn)
    _ensure_repository_repo_id_unique_index(conn)
    _ensure_content_repo_id_indexes(conn)
    _ensure_repository_namespace_prefix_unique_index(conn, ctx)


def _ensure_schema(conn, ctx: ServerContext) -> None:
    if ctx.db_backend == "postgres":
        ready_key = _postgres_schema_ready_key(ctx)
        if ready_key is not None:
            with _POSTGRES_CONTENT_SCHEMA_READY_GUARD:
                if ready_key in _POSTGRES_CONTENT_SCHEMA_READY:
                    return
                with postgres_advisory_lock(conn, scope=f"{ctx.content_schema}:server-content-initialize"):
                    _ensure_schema_postgres(conn, ctx)
                    _migrate_snapshot_metadata(conn, ctx)
                    ensure_schema_version(conn, plane="content")
                    conn.commit()
                _mark_postgres_schema_ready(ctx)
                return
        _ensure_schema_postgres(conn, ctx)
    else:
        conn.executescript(SCHEMA_SQLITE)
        _ensure_column(conn, ctx, "lines", "status", "text not null default 'active'")
        _ensure_column(conn, ctx, "lines", "archived_at", "text")
        _ensure_column(conn, ctx, "lines", "repo_id", "text")
        _ensure_column(conn, ctx, "repositories", "repo_id", "text")
        _ensure_column(conn, ctx, "repositories", "lifecycle_state", "text not null default 'active'")
        _ensure_column(conn, ctx, "repositories", "id_namespace_prefix", "text not null default 'AIT'")
        _ensure_column(conn, ctx, "repositories", "policy_json", "text not null default '{}'")
        _ensure_column(conn, ctx, "snapshots", "repo_id", "text")
        _ensure_column(conn, ctx, "snapshots", "root_tree_id", "text")
        tree_cols = {row["name"] for row in conn.execute("pragma table_info(trees)")}
        if "tree_pack_id" not in tree_cols:
            conn.execute("alter table trees add column tree_pack_id text")
        if "tree_pack_entry_name" not in tree_cols:
            conn.execute("alter table trees add column tree_pack_entry_name text")
        if "tree_pack_checksum" not in tree_cols:
            conn.execute("alter table trees add column tree_pack_checksum text")
        if "tree_packed_at" not in tree_cols:
            conn.execute("alter table trees add column tree_packed_at text")
        blob_cols = {row["name"] for row in conn.execute("pragma table_info(blobs)")}
        if "storage_kind" not in blob_cols:
            conn.execute("alter table blobs add column storage_kind text not null default 'pack_full'")
        if "pack_id" not in blob_cols:
            conn.execute("alter table blobs add column pack_id text")
        if "pack_entry_name" not in blob_cols:
            conn.execute("alter table blobs add column pack_entry_name text")
        if "pack_entry_type" not in blob_cols:
            conn.execute("alter table blobs add column pack_entry_type text")
        if "pack_base_blob_id" not in blob_cols:
            conn.execute("alter table blobs add column pack_base_blob_id text")
        if "pack_chain_depth" not in blob_cols:
            conn.execute("alter table blobs add column pack_chain_depth integer")
        if "packed_at" not in blob_cols:
            conn.execute("alter table blobs add column packed_at text")
        if "pruned_at" not in blob_cols:
            conn.execute("alter table blobs add column pruned_at text")
        conn.execute("create index if not exists idx_blobs_pack_id on blobs(pack_id)")
        pack_cols = {row["name"] for row in conn.execute("pragma table_info(packs)")}
        if "repo_id" not in pack_cols:
            conn.execute("alter table packs add column repo_id text")
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
        group_cols = {row["name"] for row in conn.execute("pragma table_info(repository_groups)")}
        if "system_slug" not in group_cols:
            conn.execute("alter table repository_groups add column system_slug text")
        membership_cols = {row["name"] for row in conn.execute("pragma table_info(repository_group_memberships)")}
        if "repo_id" not in membership_cols:
            conn.execute("alter table repository_group_memberships add column repo_id text")
        conn.execute("create index if not exists idx_trees_tree_pack_id on trees(tree_pack_id)")
        conn.execute("create index if not exists idx_repository_groups_sort on repository_groups(sort_index, group_id)")
        conn.execute(
            "create index if not exists idx_repository_group_memberships_group on repository_group_memberships(group_id, sort_index, repo_name)"
        )
        _backfill_missing_repository_ids(conn)
        _backfill_content_plane_repo_ids(conn)
        _ensure_repository_repo_id_unique_index(conn)
        _ensure_content_repo_id_indexes(conn)
        _ensure_repository_namespace_prefix_unique_index(conn, ctx)
    _migrate_snapshot_metadata(conn, ctx)
    ensure_schema_version(conn, plane="content")
    conn.commit()


def _new_repository_id() -> str:
    return f"REPO-{uuid.uuid4().hex.upper()}"


def _derived_repository_namespace_prefix(repo_name: str) -> str:
    seed = "".join(ch for ch in str(repo_name or "").upper() if ch.isalnum())
    return normalize_id_namespace_prefix(seed or DEFAULT_ID_NAMESPACE_PREFIX, default=DEFAULT_ID_NAMESPACE_PREFIX)


def _backfill_missing_repository_ids(conn) -> None:
    rows = conn.execute(
        """
        select repo_name
        from repositories
        where repo_id is null or trim(repo_id) = ''
        order by created_at asc, repo_name asc
        """
    ).fetchall()
    if not rows:
        return
    now = utc_now()
    for row in rows:
        conn.execute(
            "update repositories set repo_id = ?, updated_at = ? where repo_name = ?",
            (_new_repository_id(), now, str(row["repo_name"])),
        )


def _ensure_repository_repo_id_unique_index(conn) -> None:
    conn.execute(f"create unique index if not exists {REPOSITORY_ID_UNIQUE_INDEX} on repositories(repo_id)")


def _backfill_content_plane_repo_ids(conn) -> None:
    for table_name in ("lines", "snapshots", "packs", "repository_group_memberships"):
        conn.execute(
            f"""
            update {table_name}
            set repo_id = (
                select repositories.repo_id
                from repositories
                where repositories.repo_name = {table_name}.repo_name
            )
            where repo_id is null or trim(repo_id) = ''
            """
        )


def _ensure_content_repo_id_indexes(conn) -> None:
    conn.execute(f"create index if not exists {LINES_REPO_ID_INDEX} on lines(repo_id, line_name)")
    conn.execute(
        f"create index if not exists {SNAPSHOTS_REPO_ID_CREATED_INDEX} on snapshots(repo_id, created_at desc)"
    )
    conn.execute(f"create index if not exists {PACKS_REPO_ID_INDEX} on packs(repo_id, created_at desc)")
    conn.execute(
        f"create unique index if not exists {REPOSITORY_GROUP_MEMBERSHIPS_REPO_ID_UNIQUE_INDEX} "
        "on repository_group_memberships(repo_id)"
    )
    conn.execute(
        f"create index if not exists {REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_ID_INDEX} "
        "on repository_group_memberships(group_id, sort_index, repo_id)"
    )


def _normalize_head_snapshot_id(value: Any) -> str | None:
    normalized = str(value or "").strip()
    return normalized or None


def _set_line_head_snapshot_id(
    conn,
    *,
    repo_id: Any | None,
    scoped_repo_name: str,
    line_name: str,
    head_snapshot_id: str | None,
) -> None:
    conn.execute(
        "update lines set head_snapshot_id = ? where " + _repository_id_scope_predicate() + " and line_name = ?",
        (head_snapshot_id, repo_id, scoped_repo_name, line_name),
    )


def _migrate_line_head_snapshot_index(conn, ctx: ServerContext) -> None:
    _ensure_column(conn, ctx, "lines", "head_snapshot_id", "text")
    conn.execute("create index if not exists idx_lines_repo_id_head_snapshot on lines(repo_id, head_snapshot_id)")
    rows = conn.execute(
        """
        select repo_name, repo_id, line_name
        from lines
        where head_snapshot_id is null or trim(head_snapshot_id) = ''
        order by repo_name asc, line_name asc
        """
    ).fetchall()
    for row in rows:
        repo_name = str(row["repo_name"] or "").strip()
        repo_id = _normalize_head_snapshot_id(row["repo_id"])
        line_name = str(row["line_name"] or "").strip()
        head_snapshot_id = _read_ref_for_repository(ctx, repo_name, repo_id, line_name)
        _set_line_head_snapshot_id(
            conn,
            repo_id=repo_id,
            scoped_repo_name=repo_name,
            line_name=line_name,
            head_snapshot_id=head_snapshot_id,
        )


def _repository_namespace_prefix_duplicates(conn) -> list[dict[str, Any]]:
    rows = conn.execute(
        "select repo_name, id_namespace_prefix from repositories order by id_namespace_prefix asc, repo_name asc"
    ).fetchall()
    grouped: dict[str, list[str]] = {}
    for row in rows:
        prefix = str(row["id_namespace_prefix"])
        if not prefix:
            continue
        grouped.setdefault(prefix, []).append(str(row["repo_name"]))
    return [
        {
            "id_namespace_prefix": prefix,
            "repo_count": len(repo_names),
            "repo_names": repo_names,
        }
        for prefix, repo_names in grouped.items()
        if len(repo_names) > 1
    ]


def _ensure_repository_namespace_prefix_unique_index(conn, ctx: ServerContext) -> None:
    duplicates = _repository_namespace_prefix_duplicates(conn)
    if duplicates:
        return
    conn.execute(f"drop index if exists {REPOSITORY_NAMESPACE_PREFIX_UNIQUE_INDEX}")
    conn.execute(
        f"create unique index if not exists {REPOSITORY_NAMESPACE_PREFIX_UNIQUE_INDEX} "
        "on repositories(id_namespace_prefix) where id_namespace_prefix <> ''"
    )


def _repository_namespace_prefix_conflict(conn, prefix: str, *, exclude_repo_name: str | None = None) -> dict[str, Any] | None:
    if not str(prefix or "").strip():
        return None
    query = "select repo_name, id_namespace_prefix from repositories where id_namespace_prefix = ?"
    params: list[Any] = [prefix]
    if exclude_repo_name is not None:
        query += " and repo_name <> ?"
        params.append(exclude_repo_name)
    query += " order by repo_name asc limit 1"
    row = conn.execute(query, tuple(params)).fetchone()
    return dict(row) if row is not None else None


def _raise_repository_namespace_prefix_conflict(prefix: str, conflicting_repo_name: str) -> None:
    raise RepositoryNamespacePrefixConflictError(
        f"Repository namespace prefix {prefix!r} is already in use by repository {conflicting_repo_name!r}."
    )


def _ensure_column(conn, ctx: ServerContext, table_name: str, column_name: str, ddl: str) -> None:
    if ctx.db_backend == "postgres":
        row = conn.execute(
            "select 1 from information_schema.columns where table_schema = current_schema() and table_name = ? and column_name = ?",
            (table_name, column_name),
        ).fetchone()
        if row is None:
            try:
                conn.execute(f"alter table {table_name} add column {column_name} {ddl}")
            except Exception as exc:
                message = str(exc).lower()
                if "already exists" not in message and "duplicate column" not in message:
                    raise
        return
    cols = {row["name"] for row in conn.execute(f"pragma table_info({table_name})")}
    if column_name not in cols:
        try:
            conn.execute(f"alter table {table_name} add column {column_name} {ddl}")
        except Exception as exc:
            if "duplicate column name" not in str(exc).lower():
                raise


def audit_repository_namespace_prefix_duplicates(ctx: ServerContext) -> list[dict[str, Any]]:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        return _repository_namespace_prefix_duplicates(conn)


def _snapshot_files_view_sql() -> str:
    return """
    create view snapshot_files as
    with recursive snapshot_walk(snapshot_id, prefix, entry_name, entry_type, target_id, size_bytes, mode) as (
        select
            s.snapshot_id,
            '' as prefix,
            te.entry_name,
            te.entry_type,
            te.target_id,
            te.size_bytes,
            te.mode
        from snapshots s
        join tree_entries te on te.tree_id = s.root_tree_id
      union all
        select
            sw.snapshot_id,
            sw.prefix || sw.entry_name || '/',
            te.entry_name,
            te.entry_type,
            te.target_id,
            te.size_bytes,
            te.mode
        from snapshot_walk sw
        join tree_entries te on te.tree_id = sw.target_id
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


def _snapshot_files_object_type(conn, ctx: ServerContext) -> str | None:
    def _sqlite_catalog_type() -> str | None:
        row = conn.execute(
            "select type from sqlite_master where name = 'snapshot_files' and type in ('table', 'view')"
        ).fetchone()
        return row["type"] if row else None

    if ctx.db_backend == "postgres":
        try:
            table = conn.execute(
                """
                select table_type
                from information_schema.tables
                where table_schema = current_schema()
                  and table_name = 'snapshot_files'
                """
            ).fetchone()
        except Exception:
            return _sqlite_catalog_type()
        if table is not None:
            return "view" if str(table.get("table_type") or "").upper() == "VIEW" else "table"
        view = conn.execute(
            "select 1 from information_schema.views where table_schema = current_schema() and table_name = 'snapshot_files'"
        ).fetchone()
        return "view" if view is not None else None
    return _sqlite_catalog_type()


def _tree_reachability_cte_sql(
    repo_name: str | None = None,
    repo_id: str | None = None,
) -> tuple[str, tuple[Any, ...]]:
    predicate = "coalesce(root_tree_id, '') != ''"
    params: tuple[Any, ...] = ()
    if repo_name is not None:
        predicate += f" and {_repository_id_scope_predicate('s')}"
        params = (repo_id, repo_name)
    return (
        f"""
        with recursive reachable_trees(tree_id) as (
            select distinct s.root_tree_id
            from snapshots s
            where {predicate}
          union
            select te.target_id
            from tree_entries te
            join reachable_trees rt on rt.tree_id = te.tree_id
            where te.entry_type = 'tree'
        )
        """,
        params,
    )


def _tree_metadata_stats(
    conn,
    repo_name: str | None = None,
    repo_id: str | None = None,
) -> dict[str, int]:
    if repo_name is not None and repo_id is None:
        repo_id, _ = _repository_scope_params(conn, repo_name)
    reachable_cte, params = _tree_reachability_cte_sql(repo_name, repo_id)
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
        """,
        params,
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
    reachable_cte, params = _tree_reachability_cte_sql()
    rows = conn.execute(
        reachable_cte
        + """
        select t.tree_id
        from trees t
        where not exists (
            select 1 from reachable_trees rt where rt.tree_id = t.tree_id
        )
        order by t.tree_id asc
        """,
        params,
    ).fetchall()
    return [str(row["tree_id"]) for row in rows]


def _insert_tree_records(conn, tree_rows: list[dict[str, Any]], tree_entry_rows: list[dict[str, Any]], created_at: str) -> None:
    if tree_rows:
        conn.executemany(
            "insert or ignore into trees(tree_id, entry_count, created_at) values (?, ?, ?)",
            [(row["tree_id"], int(row["entry_count"]), created_at) for row in tree_rows],
        )
    if tree_entry_rows:
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
                    row["size_bytes"],
                    row["mode"],
                )
                for row in tree_entry_rows
            ],
        )


def _tree_pack_entry_rows(conn, tree_ids: list[str]) -> list[dict[str, Any]]:
    if not tree_ids:
        return []
    placeholder_sql = ", ".join("?" for _ in tree_ids)
    rows = conn.execute(
        f"""
        select tree_id, entry_name, entry_type, target_id, size_bytes, mode
        from tree_entries
        where tree_id in ({placeholder_sql})
        order by tree_id asc, entry_name asc
        """,
        tuple(tree_ids),
    ).fetchall()
    return [dict(row) for row in rows]


def _tree_pack_row_map(conn, tree_ids: list[str]) -> dict[str, dict[str, Any]]:
    if not tree_ids:
        return {}
    placeholder_sql = ", ".join("?" for _ in tree_ids)
    rows = conn.execute(
        f"""
        select tree_id, entry_count, tree_pack_id, tree_pack_entry_name, tree_pack_checksum
        from trees
        where tree_id in ({placeholder_sql})
        order by tree_id asc
        """,
        tuple(tree_ids),
    ).fetchall()
    return {str(row["tree_id"]): dict(row) for row in rows}


def _write_tree_pack(
    ctx: ServerContext,
    conn,
    tree_ids: list[str],
    *,
    created_at: str,
    seed_hint: str,
) -> dict[str, dict[str, Any]]:
    if not tree_ids:
        return {}
    current_rows = _tree_pack_row_map(conn, tree_ids)
    missing_tree_ids = [tree_id for tree_id in tree_ids if not current_rows.get(tree_id, {}).get("tree_pack_id")]
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
        insert into tree_packs(
            pack_id, status, tree_count, total_bytes, pack_path, pack_format, pack_index_entry_name, pack_index_checksum, created_at
        ) values (?, 'ready', ?, ?, ?, ?, ?, ?, ?)
        on conflict(pack_id) do nothing
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
        conn.execute(
            """
            update trees
            set tree_pack_id = ?,
                tree_pack_entry_name = ?,
                tree_pack_checksum = ?,
                tree_packed_at = ?
            where tree_id = ?
            """,
            (pack_id, member["entry_name"], member["checksum"], created_at, tree_id),
        )
    return _tree_pack_row_map(conn, tree_ids)


def _manifest_path_for_tree(conn, tree_id: str) -> str:
    row = conn.execute(
        """
        select tp.pack_path, t.tree_pack_entry_name
        from trees t
        join tree_packs tp on tp.pack_id = t.tree_pack_id
        where t.tree_id = ?
          and coalesce(t.tree_pack_id, '') != ''
          and coalesce(t.tree_pack_entry_name, '') != ''
        """,
        (tree_id,),
    ).fetchone()
    if row is None:
        return f"trees/{tree_id}"
    return tree_pack_manifest_path(str(row["pack_path"]), str(row["tree_pack_entry_name"]))


def _sync_tree_pack_metadata(conn, ctx: ServerContext) -> None:
    tree_ids = [
        str(row["tree_id"])
        for row in conn.execute(
            "select tree_id from trees where coalesce(tree_pack_id, '') = '' order by tree_id asc"
        ).fetchall()
    ]
    if tree_ids:
        _write_tree_pack(ctx, conn, tree_ids, created_at=utc_now(), seed_hint="tree-metadata-migration")
    conn.execute(
        """
        update snapshots
        set manifest_path = (
            select case
                when coalesce(tp.pack_path, '') != '' and coalesce(t.tree_pack_entry_name, '') != ''
                then tp.pack_path || '#' || t.tree_pack_entry_name
                else 'trees/' || snapshots.root_tree_id
            end
            from trees t
            left join tree_packs tp on tp.pack_id = t.tree_pack_id
            where t.tree_id = snapshots.root_tree_id
        )
        where coalesce(root_tree_id, '') != ''
        """
    )


def _cleanup_manifest_files(ctx: ServerContext) -> None:
    if not ctx.manifest_dir.exists():
        return
    for path in ctx.manifest_dir.glob("*.json"):
        path.unlink(missing_ok=True)


def _recreate_snapshot_files_view(conn, ctx: ServerContext) -> None:
    conn.execute("drop view if exists snapshot_files")
    conn.execute(_snapshot_files_view_sql())


def _migrate_snapshot_metadata(conn, ctx: ServerContext) -> None:
    snapshot_files_type = _snapshot_files_object_type(conn, ctx)
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
        _recreate_snapshot_files_view(conn, ctx)
        _cleanup_manifest_files(ctx)
    elif snapshot_files_type is None:
        _recreate_snapshot_files_view(conn, ctx)
    elif snapshot_files_type == "view":
        _cleanup_manifest_files(ctx)
    _sync_tree_pack_metadata(conn, ctx)


def initialize(ctx: ServerContext) -> None:
    with _connect(ctx) as conn:
        enable_sqlite_wal(conn)
        with postgres_advisory_lock(conn, scope=f"{ctx.content_schema}:server-content-initialize"):
            _ensure_schema(conn, ctx)
            _migrate_line_head_snapshot_index(conn, ctx)
        conn.commit()



def _repo_ref_path(ctx: ServerContext, repo_token: str, line_name: str) -> Path:
    return ctx.ref_root / encode_ref_name(repo_token) / "lines" / encode_ref_name(line_name)


def _ref_paths(ctx: ServerContext, repo_name: str, repo_id: str | None, line_name: str) -> tuple[Path, Path | None]:
    current = _repo_ref_path(ctx, repo_id or repo_name, line_name)
    legacy = None if repo_id is None or repo_id == repo_name else _repo_ref_path(ctx, repo_name, line_name)
    return current, legacy



def _read_ref_from_paths(current_path: Path, legacy_path: Path | None) -> str | None:
    for path in (current_path, legacy_path):
        if path is None or not path.exists():
            continue
        text = path.read_text(encoding="utf-8").strip()
        return text or None
    return None



def _read_ref_for_repository(ctx: ServerContext, repo_name: str, repo_id: str | None, line_name: str) -> str | None:
    current_path, legacy_path = _ref_paths(ctx, repo_name, repo_id, line_name)
    return _read_ref_from_paths(current_path, legacy_path)



def _write_ref_for_repository(
    ctx: ServerContext,
    repo_name: str,
    repo_id: str | None,
    line_name: str,
    snapshot_id: str | None,
) -> None:
    path, legacy_path = _ref_paths(ctx, repo_name, repo_id, line_name)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text((snapshot_id or "") + "\n", encoding="utf-8")
    if legacy_path is not None and legacy_path.exists():
        legacy_path.unlink()



def read_ref(ctx: ServerContext, repo_name: str, line_name: str) -> str | None:
    try:
        repo = get_repository(ctx, repo_name)
        repo_id = str(repo.get("repo_id") or "").strip() or None
    except KeyError:
        repo_id = None
    return _read_ref_for_repository(ctx, repo_name, repo_id, line_name)



def write_ref(ctx: ServerContext, repo_name: str, line_name: str, snapshot_id: str | None) -> None:
    repo = get_repository(ctx, repo_name)
    repo_id = str(repo.get("repo_id") or "").strip() or None
    _write_ref_for_repository(ctx, repo_name, repo_id, line_name, snapshot_id)
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        scoped_repo_id, scoped_repo_name = _repository_scope_params(conn, repo_name)
        _set_line_head_snapshot_id(
            conn,
            repo_id=scoped_repo_id,
            scoped_repo_name=scoped_repo_name,
            line_name=line_name,
            head_snapshot_id=_normalize_head_snapshot_id(snapshot_id),
        )
        conn.commit()



def _blob_path(ctx: ServerContext, blob_id: str) -> Path:
    return ctx.pack_dir / f"{blob_id}.packref"



def _manifest_path(ctx: ServerContext, manifest_hash: str) -> Path:
    return ctx.manifest_dir / f"{manifest_hash}.json"



def _pack_path(ctx: ServerContext, pack_id: str) -> Path:
    return ctx.pack_dir / f"{pack_id}.zip"


def _tree_pack_path(ctx: ServerContext, pack_id: str) -> Path:
    return ctx.tree_pack_dir / f"{pack_id}.zip"



def _snapshot_row(conn, snapshot_id: str):
    return conn.execute("select * from snapshots where snapshot_id = ?", (snapshot_id,)).fetchone()



def _blob_row(conn, blob_id: str):
    return conn.execute("select * from blobs where blob_id = ?", (blob_id,)).fetchone()



def _blob_bytes_by_id(ctx: ServerContext, conn, blob_id: str, *, seen_blob_ids: set[str] | None = None) -> bytes:
    row = _blob_row(conn, blob_id)
    if row is None:
        raise KeyError(f"Unknown blob: {blob_id}")
    return _blob_bytes_by_row(ctx, conn, row, seen_blob_ids=seen_blob_ids)


def _blob_bytes_by_row(ctx: ServerContext, conn, row, *, seen_blob_ids: set[str] | None = None) -> bytes:
    blob_id = row["blob_id"]
    visited = set(seen_blob_ids or ())
    if blob_id in visited:
        raise ValueError(f"Cyclic blob resolution detected for {blob_id}")
    visited.add(blob_id)
    if row["pack_id"] and row["pack_entry_name"]:
        pack_row = conn.execute("select * from packs where pack_id = ?", (row["pack_id"],)).fetchone()
        if pack_row is None:
            raise FileNotFoundError(f"Missing pack metadata for {row['pack_id']}")
        pack_abs = ctx.root / pack_row["pack_path"]
    return read_pack_entry(
        pack_abs,
        row["pack_entry_name"],
        resolve_base_blob=lambda base_blob_id: _blob_bytes_by_id(ctx, conn, base_blob_id, seen_blob_ids=visited),
    )


def _bootstrap_authority_files_not_seeded(_: ServerContext, repo_name: str) -> None:
    # Remote repository bootstrap is metadata-only; governance documents remain
    # repository-scoped and are intentionally materialized by local/new-repo flow.
    # Keep this hook explicit to preserve compatibility when future server-side
    # bootstrap seeds are added.
    return None



def read_blob_bytes(ctx: ServerContext, blob_id: str) -> bytes:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        return _blob_bytes_by_id(ctx, conn, blob_id)


def write_blob_bytes(
    ctx: ServerContext,
    repo_name: str,
    data: bytes,
    *,
    path_hint: str | None = None,
    created_at: str | None = None,
) -> dict[str, Any]:
    """Store standalone blob bytes in the shared content store.

    Snapshot import already writes file blobs, but plan revisions need a
    revision-level Markdown payload without manufacturing a repository snapshot.
    Keep the blob row compatible with snapshot blobs so later pack/gc layers can
    reason about the payload through the same content table.
    """

    ensure_repository(ctx, repo_name, "main")
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        repo_id, _ = _repository_scope_params(conn, repo_name)
        digest = hashlib.sha256(data).hexdigest()
        blob_id = f"BLB-{digest[:20]}"
        size = len(data)
        now = created_at or utc_now()
        blob_storage = _blob_path(ctx, blob_id)
        existing = _blob_row(conn, blob_id)
        if existing is None:
            pack_id, members_by_blob_id = _write_packed_blobs(
                ctx,
                conn,
                repo_name,
                repo_id,
                f"standalone:{blob_id}:{now}",
                now,
                [
                    {
                        "blob_id": blob_id,
                        "sha256": digest,
                        "storage_path": str(blob_storage.relative_to(ctx.root)),
                        "size_bytes": size,
                        "data": data,
                        "entry_name": f"blobs/{blob_id}",
                        "path_hint": path_hint,
                    }
                ],
            )
            member = members_by_blob_id[blob_id]
            entry_type = member.get("entry_type", "full")
            target_storage_kind = "pack_delta" if entry_type == "delta" else "pack_full"
            conn.execute(
                """
                insert or ignore into blobs(
                    blob_id, sha256, storage_path, size_bytes, storage_kind, pack_id, pack_entry_name,
                    pack_entry_type, pack_base_blob_id, pack_chain_depth, packed_at, pruned_at, created_at
                ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, null, ?)
                """,
                (
                    blob_id,
                    digest,
                    str(blob_storage.relative_to(ctx.root)),
                    size,
                    target_storage_kind,
                    pack_id,
                    member["entry_name"],
                    entry_type,
                    member.get("base_blob_id"),
                    int(member.get("chain_depth", 0) or 0),
                    now,
                    now,
                ),
            )
        else:
            stored = dict(existing)
            if stored.get("sha256") != digest or int(stored.get("size_bytes") or 0) != size:
                raise ValueError(f"Blob {blob_id} already exists with different metadata")
        conn.commit()
        row = conn.execute("select * from blobs where blob_id = ?", (blob_id,)).fetchone()
    assert row is not None
    out = dict(row)
    if path_hint is not None:
        out["path_hint"] = path_hint
    return out


def _repository_out(row) -> dict[str, Any]:
    out = dict(row)
    repo_id = str(out.get("repo_id") or "").strip()
    if not repo_id:
        raise ValueError(f"Repository {out.get('repo_name')!r} is missing repo_id")
    out["repo_id"] = repo_id
    lifecycle_state = str(out.get("lifecycle_state") or "active").strip().lower() or "active"
    if lifecycle_state not in REPOSITORY_LIFECYCLE_STATES:
        lifecycle_state = "active"
    out["lifecycle_state"] = lifecycle_state
    try:
        raw_policy = json.loads(out.get("policy_json") or "{}")
    except Exception:
        raw_policy = {}
    out["policy"] = normalize_policy(raw_policy)
    try:
        out["id_namespace_prefix"] = normalize_id_namespace_prefix(
            out.get("id_namespace_prefix"),
            default=DEFAULT_ID_NAMESPACE_PREFIX,
        )
    except ValueError:
        out["id_namespace_prefix"] = DEFAULT_ID_NAMESPACE_PREFIX
    out.pop("policy_json", None)
    return out


def _repository_id_scope_predicate(alias: str | None = None) -> str:
    prefix = f"{alias}." if alias is not None else ""
    return f"({prefix}repo_id = ? or ({prefix}repo_id is null and {prefix}repo_name = ?))"


def _repository_scope_params(conn, repo_name: str) -> tuple[Any | None, str]:
    row = conn.execute("select repo_id from repositories where repo_name = ?", (repo_name,)).fetchone()
    if row is None:
        return (None, repo_name)
    repo_id = row["repo_id"]
    return (repo_id if repo_id is not None else None, repo_name)


from .server_content_groups import (
    _repository_group_row_out,
    _new_repository_group_id,
    _list_repository_group_rows,
    _normalize_repository_group_order,
    _normalize_repository_group_memberships,
    _ensure_default_repository_group,
    _sync_repository_group_memberships,
    list_repository_groups,
    create_repository_group,
    replace_repository_group_layout
)


from .server_content_storage import (
    snapshot_manifest_map,
    repository_storage_stats,
    repository_storage_signals,
    pack_repository,
    gc_repository_content,
)


def ensure_repository(
    ctx: ServerContext,
    repo_name: str,
    default_line: str,
    policy: dict[str, Any] | None = None,
    *,
    id_namespace_prefix: str | None = None,
) -> dict:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
        now = utc_now()
        normalized_policy = normalize_policy(policy) if policy is not None else None
        normalized_prefix = (
            normalize_id_namespace_prefix(id_namespace_prefix, default=DEFAULT_ID_NAMESPACE_PREFIX)
            if id_namespace_prefix is not None
            else None
        )
        if row is None:
            stored_policy = normalized_policy or normalize_policy(None)
            stored_prefix = normalized_prefix if normalized_prefix is not None else _derived_repository_namespace_prefix(repo_name)
            repo_id = _new_repository_id()
            conflict = _repository_namespace_prefix_conflict(conn, stored_prefix)
            if conflict is not None:
                _raise_repository_namespace_prefix_conflict(stored_prefix, str(conflict["repo_name"]))
            conn.execute(
                """
                insert into repositories(repo_name, repo_id, default_line, id_namespace_prefix, policy_json, created_at, updated_at)
                values (?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    repo_name,
                    repo_id,
                    default_line,
                    stored_prefix,
                    json.dumps(stored_policy, sort_keys=True),
                    now,
                    now,
                ),
            )
            conn.execute(
                "insert into lines(repo_name, repo_id, line_name, created_at, updated_at) values (?, ?, ?, ?, ?)",
                (repo_name, repo_id, default_line, now, now),
            )
            _bootstrap_authority_files_not_seeded(ctx, repo_name)
            conn.commit()
            row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
            write_ref(ctx, repo_name, default_line, None)
        else:
            existing = _repository_out(row)
            updates: list[str] = []
            parameters: list[Any] = []
            if normalized_policy is not None and existing["policy"] != normalized_policy:
                updates.append("policy_json = ?")
                parameters.append(json.dumps(normalized_policy, sort_keys=True))
            if normalized_prefix is not None and existing.get("id_namespace_prefix") != normalized_prefix:
                conflict = _repository_namespace_prefix_conflict(conn, normalized_prefix, exclude_repo_name=repo_name)
                if conflict is not None:
                    _raise_repository_namespace_prefix_conflict(normalized_prefix, str(conflict["repo_name"]))
                updates.append("id_namespace_prefix = ?")
                parameters.append(normalized_prefix)
            if updates:
                updates.append("updated_at = ?")
                parameters.append(now)
                parameters.append(repo_name)
                conn.execute(
                    f"update repositories set {', '.join(updates)} where repo_name = ?",
                    tuple(parameters),
                )
                conn.commit()
                row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
        _ensure_default_repository_group(conn)
        _sync_repository_group_memberships(conn)
        conn.commit()
    return _repository_out(row)



def repository_exists(ctx: ServerContext, repo_name: str) -> bool:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        row = conn.execute("select 1 from repositories where repo_name = ?", (repo_name,)).fetchone()
    return row is not None



def get_repository(ctx: ServerContext, repo_name: str) -> dict:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
    if row is None:
        raise KeyError(f"Unknown repository: {repo_name}")
    return _repository_out(row)


def set_repository_lifecycle_state(
    ctx: ServerContext,
    repo_name: str,
    lifecycle_state: str,
    *,
    expected_repo_id: str | None = None,
) -> dict:
    normalized_state = str(lifecycle_state or "").strip().lower()
    if normalized_state not in REPOSITORY_LIFECYCLE_STATES:
        raise ValueError(f"Unsupported repository lifecycle state: {lifecycle_state!r}")
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown repository: {repo_name}")
        current = _repository_out(row)
        normalized_expected_repo_id = str(expected_repo_id or "").strip()
        if normalized_expected_repo_id and normalized_expected_repo_id != current["repo_id"]:
            raise ValueError(
                f"Repository scope mismatch for {repo_name}: repo_id {normalized_expected_repo_id} does not match {current['repo_id']}"
            )
        if current["lifecycle_state"] == normalized_state:
            return current
        conn.execute(
            "update repositories set lifecycle_state = ?, updated_at = ? where repo_name = ?",
            (normalized_state, utc_now(), repo_name),
        )
        conn.commit()
        row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
    assert row is not None
    return _repository_out(row)



def list_lines(ctx: ServerContext, repo_name: str) -> list[dict]:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = _repository_scope_params(conn, repo_name)
        rows = [
            dict(r)
            for r in conn.execute(
                "select * from lines where " + _repository_id_scope_predicate() + " order by line_name",
                (repo_id, scoped_repo_name),
            )
        ]
    for row in rows:
        row["status"] = row.get("status") or "active"
        resolved_repo_id = str(row.get("repo_id") or repo_id or "").strip() or None
        row["head_snapshot_id"] = _read_ref_for_repository(ctx, repo_name, resolved_repo_id, row["line_name"])
    return rows



def get_line(ctx: ServerContext, repo_name: str, line_name: str) -> dict:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = _repository_scope_params(conn, repo_name)
        row = conn.execute(
            "select * from lines where " + _repository_id_scope_predicate() + " and line_name = ?",
            (repo_id, scoped_repo_name, line_name),
        ).fetchone()
    if row is None:
        raise KeyError(f"Unknown line {line_name} for repository {repo_name}")
    out = dict(row)
    out["status"] = out.get("status") or "active"
    resolved_repo_id = str(out.get("repo_id") or "").strip() or None
    out["head_snapshot_id"] = _read_ref_for_repository(ctx, repo_name, resolved_repo_id, line_name)
    return out


def list_lines_by_head_snapshot_ids(
    ctx: ServerContext,
    repo_name: str,
    head_snapshot_ids: set[str] | list[str] | tuple[str, ...],
    *,
    exclude_line_names: set[str] | list[str] | tuple[str, ...] | None = None,
) -> list[dict]:
    normalized_snapshot_ids = sorted(
        item for item in {_normalize_head_snapshot_id(item) for item in head_snapshot_ids if item} if item is not None
    )
    if not normalized_snapshot_ids:
        return []
    normalized_excluded_line_names = sorted({str(item).strip() for item in exclude_line_names or [] if str(item).strip()})
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = _repository_scope_params(conn, repo_name)
        snapshot_placeholders = ", ".join("?" for _ in normalized_snapshot_ids)
        exclude_clause = ""
        query_params: list[Any] = [repo_id, scoped_repo_name, *normalized_snapshot_ids]
        if normalized_excluded_line_names:
            exclude_placeholders = ", ".join("?" for _ in normalized_excluded_line_names)
            exclude_clause = f" and line_name not in ({exclude_placeholders})"
            query_params.extend(normalized_excluded_line_names)
        rows = [
            dict(r)
            for r in conn.execute(
                "select * from lines where "
                + _repository_id_scope_predicate()
                + f" and head_snapshot_id in ({snapshot_placeholders}){exclude_clause} order by line_name",
                tuple(query_params),
            )
        ]
    for row in rows:
        row["status"] = row.get("status") or "active"
        row["head_snapshot_id"] = _normalize_head_snapshot_id(row.get("head_snapshot_id"))
    return rows


def update_line(
    ctx: ServerContext,
    repo_name: str,
    line_name: str,
    head_snapshot_id: Optional[str],
    *,
    expected_head_snapshot_id: Optional[str] = None,
    timings: Optional[dict[str, Any]] = None,
) -> dict:
    def _update(conn):
        _ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = _repository_scope_params(conn, repo_name)
        resolved_repo_id = str(repo_id or "").strip() or None
        lock_scope = f"{ctx.content_schema or 'content'}:line:{resolved_repo_id or scoped_repo_name}:{line_name}"
        total_started = time.perf_counter()
        lock_wait_started = total_started
        lock_hold_started: float | None = None
        try:
            with postgres_advisory_lock(conn, scope=lock_scope):
                lock_hold_started = time.perf_counter()
                row = conn.execute(
                    "select * from lines where " + _repository_id_scope_predicate() + " and line_name = ?",
                    (repo_id, scoped_repo_name, line_name),
                ).fetchone()
                current_head_snapshot_id = _read_ref_for_repository(ctx, repo_name, resolved_repo_id, line_name)
                if expected_head_snapshot_id is not None and current_head_snapshot_id != expected_head_snapshot_id:
                    raise ValueError(
                        f"Line {line_name} head advanced before update: "
                        f"expected {expected_head_snapshot_id!r}, got {current_head_snapshot_id!r}"
                    )
                now = utc_now()
                if row is None:
                    conn.execute(
                        "insert into lines(repo_name, repo_id, line_name, head_snapshot_id, status, archived_at, created_at, updated_at) "
                        "values (?, ?, ?, ?, 'active', null, ?, ?)",
                        (repo_name, repo_id, line_name, head_snapshot_id, now, now),
                    )
                else:
                    if (row["status"] or "active") == "archived":
                        raise ValueError(f"Line {line_name} is archived and cannot move")
                    conn.execute(
                        "update lines set head_snapshot_id = ?, updated_at = ? where "
                        + _repository_id_scope_predicate()
                        + " and line_name = ?",
                        (head_snapshot_id, now, repo_id, scoped_repo_name, line_name),
                    )
                _write_ref_for_repository(ctx, repo_name, resolved_repo_id, line_name, head_snapshot_id)
                refreshed = conn.execute(
                    "select * from lines where " + _repository_id_scope_predicate() + " and line_name = ?",
                    (repo_id, scoped_repo_name, line_name),
                ).fetchone()
                assert refreshed is not None
                out = dict(refreshed)
                out["status"] = out.get("status") or "active"
                out["head_snapshot_id"] = head_snapshot_id
                return out
        finally:
            if timings is not None:
                finished = time.perf_counter()
                if lock_hold_started is None:
                    timings["advisory_lock_wait"] = _elapsed_ms(lock_wait_started, finished)
                else:
                    timings["advisory_lock_wait"] = _elapsed_ms(lock_wait_started, lock_hold_started)
                    timings["advisory_lock_hold"] = _elapsed_ms(lock_hold_started, finished)
                timings["total"] = _elapsed_ms(total_started, finished)

    return write(ctx, _update)


def archive_line(ctx: ServerContext, repo_name: str, line_name: str) -> dict:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = _repository_scope_params(conn, repo_name)
        row = conn.execute(
            "select * from lines where " + _repository_id_scope_predicate() + " and line_name = ?",
            (repo_id, scoped_repo_name, line_name),
        ).fetchone()
        if row is None:
            raise KeyError(f"Unknown line {line_name} for repository {repo_name}")
        if (row["status"] or "active") == "archived":
            return get_line(ctx, repo_name, line_name)
        now = utc_now()
        conn.execute(
            "update lines set status = 'archived', archived_at = ?, updated_at = ? "
            "where " + _repository_id_scope_predicate() + " and line_name = ?",
            (now, now, repo_id, scoped_repo_name, line_name),
        )
        conn.commit()
    return get_line(ctx, repo_name, line_name)



def snapshot_exists(ctx: ServerContext, snapshot_id: str) -> bool:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        row = conn.execute("select 1 from snapshots where snapshot_id = ?", (snapshot_id,)).fetchone()
    return row is not None



def snapshot_existence(ctx: ServerContext, repo_name: str, snapshot_ids: list[str]) -> dict[str, Any]:
    normalized = [str(snapshot_id).strip() for snapshot_id in snapshot_ids if str(snapshot_id).strip()]
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        if normalized:
            unique_snapshot_ids = list(dict.fromkeys(normalized))
            placeholders = ",".join("?" for _ in unique_snapshot_ids)
            rows = conn.execute(
                f"""
                select s.snapshot_id
                from snapshots s
                where """
                + _repository_id_scope_predicate("s")
                + f" and s.snapshot_id in ({placeholders})",
                (*_repository_scope_params(conn, repo_name), *unique_snapshot_ids),
            ).fetchall()
            present_set = {row["snapshot_id"] for row in rows}
        else:
            present_set = set()
    present = [snapshot_id for snapshot_id in normalized if snapshot_id in present_set]
    missing = [snapshot_id for snapshot_id in normalized if snapshot_id not in present_set]
    return {
        "repo_name": repo_name,
        "checked_snapshots": len(normalized),
        "present": present,
        "missing": missing,
    }



def get_snapshot_repo(ctx: ServerContext, snapshot_id: str) -> str | None:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        row = conn.execute(
            """
            select coalesce(r.repo_name, s.repo_name) as repo_name
            from snapshots s
            left join repositories r on r.repo_id = s.repo_id
            where s.snapshot_id = ?
            """,
            (snapshot_id,),
        ).fetchone()
    return row["repo_name"] if row else None



def _snapshot_storage_ingest_mode(bundle: dict[str, Any]) -> str:
    explicit = bundle.get("storage_ingest_mode")
    return normalize_storage_ingest_mode(explicit, allow_default=False)


def _parent_delta_candidates(
    ctx: ServerContext,
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
    ctx: ServerContext,
    conn,
    repo_name: str,
    repo_id: str | None,
    snapshot_id: str,
    created_at: str,
    blob_items: list[dict[str, Any]],
    *,
    initial_by_path: dict[str, dict[str, Any]] | None = None,
) -> tuple[str, dict[str, dict[str, Any]]]:
    pack_seed = f"{repo_name}|{snapshot_id}|{json.dumps(sorted(item['blob_id'] for item in blob_items))}"
    pack_id = f"PCK-{hashlib.sha256(pack_seed.encode('utf-8')).hexdigest()[:12].upper()}"
    pack_abs = _pack_path(ctx, pack_id)
    members = build_pack_members(blob_items, initial_by_path=initial_by_path)
    members_by_blob_id = {member["blob_id"]: member for member in members}
    archive_stats = write_pack_archive(pack_abs, pack_id, created_at, members)
    existing_pack = conn.execute("select pack_id from packs where pack_id = ?", (pack_id,)).fetchone()
    if existing_pack is None:
        conn.execute(
            """
            insert into packs(pack_id, repo_name, repo_id, status, member_count, total_bytes, pack_path, pack_format, pack_index_entry_name, pack_index_checksum, created_at)
            values (?, ?, ?, 'ready', ?, ?, ?, ?, ?, ?, ?)
            """,
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
                created_at,
            ),
        )
    return pack_id, members_by_blob_id


def _canonical_snapshot_metadata(
    repo_name: str,
    line_name: str,
    parent_snapshot_id: str | None,
    message: str | None,
    file_entries: list[dict[str, Any]],
) -> dict[str, Any]:
    root_tree_id, tree_rows, tree_entry_rows = build_tree_records(file_entries)
    snapshot_id, revision_hash = build_snapshot_id(
        repo_name=repo_name,
        line_name=line_name,
        parent_snapshot_id=parent_snapshot_id,
        message=message,
        root_tree_id=root_tree_id,
    )
    return {
        "snapshot_id": snapshot_id,
        "revision_hash": revision_hash,
        "root_tree_id": root_tree_id,
        "manifest_path": f"trees/{root_tree_id}",
        "tree_rows": tree_rows,
        "tree_entry_rows": tree_entry_rows,
    }


def import_snapshot(ctx: ServerContext, repo_name: str, bundle: dict) -> dict:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        repo_id, _ = _repository_scope_params(conn, repo_name)
        snapshot_id = bundle["snapshot_id"]
        bundle_repo_name = bundle.get("repo_name")
        if bundle_repo_name and bundle_repo_name != repo_name:
            raise ValueError(
                f"Snapshot bundle repository mismatch for {snapshot_id}: "
                f"body={bundle_repo_name!r}, request={repo_name!r}"
            )
        expected_line_name = bundle.get("line_name") or "main"
        expected_parent_snapshot_id = bundle.get("parent_snapshot_id")
        expected_message = bundle.get("message")
        expected_file_count = bundle.get("file_count") or len(bundle["files"])
        expected_total_bytes = bundle.get("total_bytes")
        if expected_total_bytes is None:
            expected_total_bytes = sum(int(file_entry.get("size_bytes") or 0) for file_entry in bundle["files"])
        existing = conn.execute("select * from snapshots where snapshot_id = ?", (snapshot_id,)).fetchone()
        if existing is not None:
            row = dict(existing)
            mismatches: list[str] = []
            if row.get("repo_name") != repo_name:
                mismatches.append(f"repository={row.get('repo_name')!r}")
            if row.get("line_name") != expected_line_name:
                mismatches.append(f"line_name={row.get('line_name')!r}")
            if row.get("parent_snapshot_id") != expected_parent_snapshot_id:
                mismatches.append(f"parent_snapshot_id={row.get('parent_snapshot_id')!r}")
            if row.get("message") != expected_message:
                mismatches.append(f"message={row.get('message')!r}")
            if row.get("file_count") != expected_file_count:
                mismatches.append(f"file_count={row.get('file_count')!r}")
            if row.get("total_bytes") != expected_total_bytes:
                mismatches.append(f"total_bytes={row.get('total_bytes')!r}")
            if mismatches:
                raise ValueError(
                    f"Snapshot {snapshot_id} already exists with different canonical fields: "
                    + ", ".join(mismatches)
                )
            return row
        ensure_repository(ctx, repo_name, bundle.get("line_name") or "main")
        file_rows: list[dict[str, Any]] = []
        total_bytes = 0
        created_at = bundle.get("created_at") or utc_now()
        storage_ingest_mode = _snapshot_storage_ingest_mode(bundle)
        new_blob_rows: list[dict[str, Any]] = []
        for file_entry in bundle["files"]:
            data = base64.b64decode(file_entry["content_b64"])
            digest = hashlib.sha256(data).hexdigest()
            if digest != file_entry["sha256"]:
                raise ValueError(f"Snapshot blob digest mismatch for {file_entry['path']}")
            blob_id = file_entry["blob_id"]
            canonical_blob_id = blob_id
            size = len(data)
            total_bytes += size
            blob_storage = _blob_path(ctx, blob_id)
            existing_blob = _blob_row(conn, blob_id)
            if existing_blob is None:
                existing_blob = conn.execute(
                    "select * from blobs where sha256 = ?",
                    (digest,),
                ).fetchone()
                if existing_blob is not None:
                    canonical_blob_id = str(existing_blob["blob_id"])
            if existing_blob is None:
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
                    "blob_id": canonical_blob_id,
                    "size_bytes": file_entry["size_bytes"],
                    "mode": file_entry["mode"],
                    "sha256": file_entry["sha256"],
                }
            )

        if new_blob_rows:
            initial_by_path = None
            if storage_ingest_mode == StorageIngestMode.PACK_DELTA.value:
                initial_by_path = _parent_delta_candidates(
                    ctx,
                    conn,
                    bundle.get("parent_snapshot_id"),
                    {str(row.get("path_hint") or "") for row in new_blob_rows if row.get("path_hint")},
                )
            pack_id, members_by_blob_id = _write_packed_blobs(
                ctx,
                conn,
                repo_name,
                repo_id,
                snapshot_id,
                created_at,
                new_blob_rows,
                initial_by_path=initial_by_path,
            )
            for row in new_blob_rows:
                member = members_by_blob_id[row["blob_id"]]
                entry_type = member.get("entry_type", "full")
                target_storage_kind = "pack_delta" if entry_type == "delta" else "pack_full"
                conn.execute(
                    """
                    insert or ignore into blobs(
                        blob_id, sha256, storage_path, size_bytes, storage_kind, pack_id, pack_entry_name,
                        pack_entry_type, pack_base_blob_id, pack_chain_depth, packed_at, pruned_at, created_at
                    ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, null, ?)
                    """,
                    (
                        row["blob_id"],
                        row["sha256"],
                        row["storage_path"],
                        row["size_bytes"],
                        target_storage_kind,
                        pack_id,
                        member["entry_name"],
                        entry_type,
                        member.get("base_blob_id"),
                        int(member.get("chain_depth", 0) or 0),
                        created_at,
                        created_at,
                    ),
                )

        root_tree_id, tree_rows, tree_entry_rows = build_tree_records(file_rows)
        bundle_root_tree_id = bundle.get("root_tree_id")
        if bundle_root_tree_id and bundle_root_tree_id != root_tree_id:
            raise ValueError(
                f"Snapshot tree metadata mismatch for {snapshot_id}: "
                f"bundle={bundle_root_tree_id!r}, computed={root_tree_id!r}"
            )
        computed_snapshot_id, revision_hash = build_snapshot_id(
            repo_name=repo_name,
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
            seed_hint=f"{repo_name}|{snapshot_id}|{root_tree_id}",
        )
        manifest_path = _manifest_path_for_tree(conn, root_tree_id)

        conn.execute(
            """
            insert into snapshots(
                snapshot_id, repo_name, repo_id, parent_snapshot_id, root_tree_id, manifest_hash, manifest_path,
                message, line_name, file_count, total_bytes, created_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                snapshot_id,
                repo_name,
                repo_id,
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
    return dict(row)



def export_snapshot(
    ctx: ServerContext,
    repo_name: str,
    snapshot_id: str,
    *,
    include_content: bool = True,
    path: str | None = None,
) -> dict:
    with _connect(ctx) as conn:
        _ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = _repository_scope_params(conn, repo_name)
        snap = conn.execute(
            "select * from snapshots s where s.snapshot_id = ? and " + _repository_id_scope_predicate("s"),
            (snapshot_id, repo_id, scoped_repo_name),
        ).fetchone()
        if snap is None:
            raise KeyError(f"Unknown snapshot {snapshot_id} for repository {repo_name}")
        where_clause = "where sf.snapshot_id = ?"
        params: list[Any] = [snapshot_id]
        if path is not None:
            where_clause += " and sf.path = ?"
            params.append(path)
        if include_content:
            query = f"""
            select sf.path, sf.blob_id, sf.size_bytes, sf.mode, b.sha256, b.storage_path, b.storage_kind, b.pack_id, b.pack_entry_name
            from snapshot_files sf
            join blobs b on b.blob_id = sf.blob_id
            {where_clause}
            order by sf.path
            """
        else:
            query = f"""
            select sf.path, sf.blob_id, sf.size_bytes, sf.mode, b.sha256
            from snapshot_files sf
            join blobs b on b.blob_id = sf.blob_id
            {where_clause}
            order by sf.path
            """
        file_rows = conn.execute(query, params).fetchall()
        files = []
        for row in file_rows:
            file_row = {
                "path": row["path"],
                "blob_id": row["blob_id"],
                "size_bytes": row["size_bytes"],
                "mode": row["mode"],
                "sha256": row["sha256"],
            }
            if include_content:
                data = _blob_bytes_by_row(ctx, conn, row)
                file_row["content_b64"] = base64.b64encode(data).decode("ascii")
            files.append(file_row)
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
        "content_included": include_content,
        "files": files,
    }
