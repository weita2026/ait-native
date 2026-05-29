from __future__ import annotations

import secrets
import sqlite3
import time
from pathlib import Path
from typing import Any

from ait_protocol.common import connect_sqlite, utc_now
from .local_content_lines import (
    _ref_path,
    archive_line,
    create_line,
    get_line,
    list_lines,
    read_ref,
    set_line_head,
    write_ref,
)
from .local_content_projection import (
    _effective_workspace_ignore_rules,
    _filter_snapshot_file_map_for_workspace,
    _filter_workspace_state_for_workspace,
    _is_lineage_only_markdown_artifact_path,
    _normalize_markdown_artifact_path,
    _path_is_projected_out_for_task_worktree,
    _path_is_projected_out_for_workspace,
    allow_lineage_only_markdown_paths,
)
from .repo_paths import RepoContext
from .local_content_workspace import (
    IGNORED_DIRS,
    WORKSPACE_IGNORE_FILE,
    WorkspaceIgnoreRule,
    _normalize_workspace_restore_path,
    _snapshot_workspace_ignore_rules,
    _workspace_digest_state,
    _workspace_state,
    _workspace_ignore_policy_for_rules,
    _workspace_path_is_ignored,
    _workspace_visible_files,
    iter_workspace_files,
    workspace_ignore_policy,
    workspace_path_is_ignored,
    workspace_runtime_root_hygiene,
)
from . import local_content_schema as local_content_schema_helpers
from . import local_content_snapshots as local_content_snapshot_helpers
from .local_content_pack_runtime import (
    _blob_bytes_by_id,
    _blob_bytes_by_row,
    _blob_pack_entry_name,
    _blob_pack_entry_name_column_present,
    _blob_packed_at_column_present,
    _blob_path,
    _blob_row,
    _blob_storage_kind_column_present,
    _cleanup_manifest_files,
    _count_stale_manifest_paths,
    _derived_blob_pack_entry_name,
    _derived_tree_pack_entry_name,
    _drop_column_if_exists,
    _insert_blob_record,
    _insert_tree_records,
    _legacy_pack_metadata_columns_present,
    _local_server_catalog_tables_ready_for_drop,
    _manifest_path_for_tree,
    _manifest_path_sql,
    _pack_path,
    _parent_delta_candidates,
    _read_blob_bytes,
    _repo_status_storage_counts,
    _row_optional_value,
    _schema_cleanup_summary,
    _snapshot_row,
    _sqlite_table_exists,
    _sync_tree_pack_metadata,
    _table_columns,
    _tree_entry_blob_size_sql,
    _tree_entry_size_column_present,
    _tree_metadata_stats,
    _tree_pack_entry_name,
    _tree_pack_entry_name_column_present,
    _tree_pack_entry_rows,
    _tree_pack_entry_name_sql,
    _tree_pack_row_map,
    _tree_packed_at_column_present,
    _unreachable_tree_ids,
    _update_blob_pack_metadata,
    _write_packed_blobs,
    _write_tree_pack,
    create_pack,
    ensure_blob_bytes,
    gc_content,
    storage_stats,
)

SCHEMA = local_content_schema_helpers.SCHEMA
LOCAL_CONTENT_SCHEMA_VERSION = local_content_schema_helpers.LOCAL_CONTENT_SCHEMA_VERSION
_LOCAL_CONTENT_INIT_MIGRATION_BACKUP_PREFIX = (
    local_content_schema_helpers._LOCAL_CONTENT_INIT_MIGRATION_BACKUP_PREFIX
)
_snapshot_files_view_sql = local_content_schema_helpers._snapshot_files_view_sql
_snapshot_files_object_type = local_content_schema_helpers._snapshot_files_object_type
_snapshot_files_view_needs_refresh = local_content_schema_helpers._snapshot_files_view_needs_refresh
_content_db_explicit_init_migration_needed = (
    local_content_schema_helpers._content_db_explicit_init_migration_needed
)
_write_init_migration_backup = local_content_schema_helpers._write_init_migration_backup
_drop_empty_local_server_catalog_tables = local_content_schema_helpers._drop_empty_local_server_catalog_tables
_drop_legacy_pack_metadata_columns = local_content_schema_helpers._drop_legacy_pack_metadata_columns
_backfill_blob_pack_entry_type_from_storage_kind = (
    local_content_schema_helpers._backfill_blob_pack_entry_type_from_storage_kind
)
_drop_redundant_blob_storage_kind_column = (
    local_content_schema_helpers._drop_redundant_blob_storage_kind_column
)
_set_local_content_schema_version = local_content_schema_helpers._set_local_content_schema_version
_migrate_snapshot_metadata = local_content_schema_helpers._migrate_snapshot_metadata


def _initialize_schema(conn, ctx: RepoContext | None = None) -> None:
    local_content_schema_helpers._initialize_schema(
        conn,
        ctx,
        migrate_snapshot_metadata_fn=_migrate_snapshot_metadata,
    )


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


_canonical_snapshot_metadata = local_content_snapshot_helpers._canonical_snapshot_metadata
snapshot_exists = local_content_snapshot_helpers.snapshot_exists
create_snapshot = local_content_snapshot_helpers.create_snapshot
list_snapshots = local_content_snapshot_helpers.list_snapshots
_stash_select_sql = local_content_snapshot_helpers._stash_select_sql
_stash_view = local_content_snapshot_helpers._stash_view
create_stash = local_content_snapshot_helpers.create_stash
list_stashes = local_content_snapshot_helpers.list_stashes
get_stash = local_content_snapshot_helpers.get_stash
drop_stash = local_content_snapshot_helpers.drop_stash
get_snapshot = local_content_snapshot_helpers.get_snapshot
_parse_mode_bits = local_content_snapshot_helpers._parse_mode_bits
_blob_sha256_map = local_content_snapshot_helpers._blob_sha256_map
workspace_delta = local_content_snapshot_helpers.workspace_delta
_prune_empty_parent_dirs = local_content_snapshot_helpers._prune_empty_parent_dirs
restore_workspace = local_content_snapshot_helpers.restore_workspace
restore_workspace_paths = local_content_snapshot_helpers.restore_workspace_paths


def _snapshot_file_map(conn, snapshot_id: str | None) -> dict[str, dict]:
    return local_content_snapshot_helpers._snapshot_file_map(
        conn,
        snapshot_id,
        snapshot_row_fn=_snapshot_row,
        snapshot_file_map_via_view_fn=_snapshot_file_map_via_view,
        snapshot_file_map_via_tree_traversal_fn=_snapshot_file_map_via_tree_traversal,
    )


def _snapshot_file_map_via_view(conn, snapshot_id: str) -> dict[str, dict]:
    return local_content_snapshot_helpers._snapshot_file_map_via_view(conn, snapshot_id)


def _snapshot_file_map_via_tree_traversal(conn, root_tree_id: str) -> dict[str, dict]:
    return local_content_snapshot_helpers._snapshot_file_map_via_tree_traversal(conn, root_tree_id)


def repo_status(ctx: RepoContext, repo_name: str, current_line: str, remote_count: int) -> dict:
    stats = _repo_status_storage_counts(ctx)
    head_snapshot_id = read_ref(ctx, current_line)
    workspace = workspace_delta(ctx, head_snapshot_id)
    return {
        "repo_name": repo_name,
        "current_line": current_line,
        "head_snapshot_id": head_snapshot_id,
        "snapshot_count": stats["snapshot_count"],
        "pack_count": stats["pack_count"],
        "packed_blob_count": stats["packed_blob_count"],
        "remote_count": remote_count,
        "workspace_status": "clean" if workspace["clean"] else "dirty",
        "workspace_dirty": not workspace["clean"],
        "workspace_changed_count": workspace["changed_count"],
        "workspace_modified_count": len(workspace["modified_paths"]),
        "workspace_missing_count": len(workspace["missing_paths"]),
        "workspace_untracked_count": len(workspace["untracked_paths"]),
        "workspace_changed_paths_sample": workspace["changed_paths"][:10],
        "ignore_policy": workspace["ignore_policy"],
        "phase_timings_ms": workspace.get("phase_timings_ms"),
        "workspace_root": str(ctx.root),
        "content_db_path": str(ctx.content_db_path),
        "control_db_path": str(ctx.control_db_path),
        "refs_path": str(ctx.ref_dir),
        "objects_path": str(ctx.ait_dir / "objects"),
    }

from .local_content_bundle import (
    collect_snapshot_chain,
    ensure_snapshot_chain,
    export_snapshot_bundle,
    import_snapshot_bundle,
)
