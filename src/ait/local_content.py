from __future__ import annotations

import hashlib
import json
import secrets
import sqlite3
import time
from contextlib import contextmanager
from contextvars import ContextVar
from pathlib import Path
from typing import Any, Iterable, Optional

from ait_protocol.common import connect_sqlite, encode_ref_name, normalize_optional_text, utc_now
from ait_storage.packfiles import (
    build_pack_members,
    build_storage_validation_summary,
    read_pack_entry,
    summarize_pack_archives,
    write_pack_archive,
)
from ait_storage.revision_trees import build_snapshot_id, build_tree_records
from .repo_paths import RepoContext
from .local_content_workspace import (
    IGNORED_DIRS,
    WORKSPACE_IGNORE_FILE,
    WorkspaceIgnoreRule,
    _load_workspace_ignore_rules,
    _normalize_workspace_restore_path,
    _parse_workspace_ignore_rules,
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
from ait_storage.treepacks import (
    build_tree_pack_members,
    summarize_tree_pack_archives,
    tree_pack_manifest_path,
    write_tree_pack_archive,
)

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


_TASK_WORKTREE_MARKDOWN_BASE_RULES = _parse_workspace_ignore_rules(
    """
/docs/*.md
/docs/**/*.md
""".strip()
)
_LINEAGE_ONLY_MARKDOWN_ALLOWLIST: ContextVar[frozenset[str]] = ContextVar(
    "lineage_only_markdown_allowlist",
    default=frozenset(),
)


def _elapsed_ms(start: float, end: float | None = None) -> float:
    finished = time.perf_counter() if end is None else end
    return round((finished - start) * 1000.0, 3)


def _normalize_markdown_artifact_path(path_value: str | Path) -> str:
    return Path(str(path_value).replace("\\", "/")).as_posix().strip("/")


def _is_markdown_artifact_path(path_value: str | Path) -> bool:
    path = _normalize_markdown_artifact_path(path_value)
    return bool(path) and Path(path).suffix.lower() == ".md"


def _is_line_materialized_markdown_artifact_path(path_value: str | Path) -> bool:
    # Markdown planning artifacts stay on the plan-lineage side only and do not
    # participate in line snapshots or worktree materialization.
    return False


def _is_lineage_only_markdown_artifact_path(path_value: str | Path) -> bool:
    return _is_markdown_artifact_path(path_value) and not _is_line_materialized_markdown_artifact_path(path_value)


def _is_root_lineage_only_sprint_task_graph_path(path_value: str | Path) -> bool:
    path = _normalize_markdown_artifact_path(path_value)
    return path.startswith('docs/sprints/') and path.endswith('.task_graph.json')


def _task_worktree_sprint_markdown_projection_rules(ctx: RepoContext) -> tuple[WorkspaceIgnoreRule, ...]:
    if not ctx.is_worktree:
        return ()
    return _TASK_WORKTREE_MARKDOWN_BASE_RULES


def _merge_workspace_ignore_rules(
    base_rules: tuple[WorkspaceIgnoreRule, ...] | None,
    extra_rules: tuple[WorkspaceIgnoreRule, ...] | None,
) -> tuple[WorkspaceIgnoreRule, ...]:
    base = tuple(base_rules or ())
    extra = tuple(extra_rules or ())
    if not extra:
        return base
    if not base:
        return extra
    return base + extra


def _effective_workspace_ignore_rules(
    ctx: RepoContext,
    ignore_rules: tuple[WorkspaceIgnoreRule, ...] | None = None,
) -> tuple[WorkspaceIgnoreRule, ...]:
    base_rules = _load_workspace_ignore_rules(ctx.root) if ignore_rules is None else tuple(ignore_rules)
    return _merge_workspace_ignore_rules(base_rules, _task_worktree_sprint_markdown_projection_rules(ctx))


def _path_is_projected_out_for_task_worktree(ctx: RepoContext, rel_path: str | Path) -> bool:
    projection_rules = _task_worktree_sprint_markdown_projection_rules(ctx)
    if not projection_rules:
        return False
    return _workspace_path_is_ignored(Path(rel_path), projection_rules)


def _path_is_projected_out_for_workspace(ctx: RepoContext, rel_path: str | Path) -> bool:
    normalized = _normalize_markdown_artifact_path(rel_path)
    if _path_is_projected_out_for_task_worktree(ctx, normalized):
        return True
    if normalized in _LINEAGE_ONLY_MARKDOWN_ALLOWLIST.get():
        return False
    if _is_lineage_only_markdown_artifact_path(normalized):
        return True
    return (not ctx.is_worktree) and _is_root_lineage_only_sprint_task_graph_path(normalized)


@contextmanager
def allow_lineage_only_markdown_paths(paths: Iterable[str | Path]):
    normalized_paths = {
        _normalize_markdown_artifact_path(path)
        for path in paths
        if _normalize_markdown_artifact_path(path)
    }
    if not normalized_paths:
        yield
        return
    current = _LINEAGE_ONLY_MARKDOWN_ALLOWLIST.get()
    token = _LINEAGE_ONLY_MARKDOWN_ALLOWLIST.set(frozenset(set(current) | normalized_paths))
    try:
        yield
    finally:
        _LINEAGE_ONLY_MARKDOWN_ALLOWLIST.reset(token)


def _filter_snapshot_file_map_for_workspace(
    ctx: RepoContext,
    snapshot_files: dict[str, dict[str, Any]],
) -> dict[str, dict[str, Any]]:
    if not snapshot_files:
        return {}
    return {
        path: entry
        for path, entry in snapshot_files.items()
        if not _path_is_projected_out_for_workspace(ctx, path)
    }


def _filter_workspace_state_for_workspace(
    ctx: RepoContext,
    workspace_files: dict[str, dict[str, Any]],
) -> dict[str, dict[str, Any]]:
    if not workspace_files:
        return {}
    return {
        path: entry
        for path, entry in workspace_files.items()
        if not _path_is_projected_out_for_workspace(ctx, path)
    }


def _snapshot_files_view_sql() -> str:
    return """
    create view if not exists snapshot_files as
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


def _snapshot_files_object_type(conn) -> str | None:
    row = conn.execute(
        "select type from sqlite_master where name = 'snapshot_files' and type in ('table', 'view')"
    ).fetchone()
    return row["type"] if row else None


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


def _tree_pack_entry_rows(conn, tree_ids: Iterable[str]) -> list[dict[str, Any]]:
    ids = [str(tree_id) for tree_id in tree_ids if str(tree_id)]
    if not ids:
        return []
    placeholder_sql = ", ".join("?" for _ in ids)
    rows = conn.execute(
        f"""
        select tree_id, entry_name, entry_type, target_id, size_bytes, mode
        from tree_entries
        where tree_id in ({placeholder_sql})
        order by tree_id asc, entry_name asc
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
        select tree_id, entry_count, tree_pack_id, tree_pack_entry_name, tree_pack_checksum
        from trees
        where tree_id in ({placeholder_sql})
        order by tree_id asc
        """,
        tuple(ids),
    ).fetchall()
    return {str(row["tree_id"]): dict(row) for row in rows}


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
    return _tree_pack_row_map(conn, tree_id_list)


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


def _cleanup_manifest_files(ctx: RepoContext | None) -> None:
    if ctx is None or not ctx.manifest_dir.exists():
        return
    for path in ctx.manifest_dir.glob("*.json"):
        path.unlink(missing_ok=True)


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
        conn.execute("drop view if exists snapshot_files")
        conn.execute(_snapshot_files_view_sql())
        _cleanup_manifest_files(ctx)
    elif snapshot_files_type is None:
        conn.execute(_snapshot_files_view_sql())
    elif snapshot_files_type == "view":
        _cleanup_manifest_files(ctx)
    _sync_tree_pack_metadata(conn, ctx)


def _initialize_schema(conn, ctx: RepoContext | None = None) -> None:
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
    _migrate_snapshot_metadata(conn, ctx)
    conn.commit()


def initialize(ctx: RepoContext, default_line: str) -> None:
    conn = connect_sqlite(ctx.content_db_path)
    _initialize_schema(conn, ctx)
    now = utc_now()
    conn.execute(
        "insert or ignore into lines(line_name, status, archived_at, created_at, updated_at) values (?, 'active', null, ?, ?)",
        (default_line, now, now),
    )
    conn.commit()
    conn.close()
    write_ref(ctx, default_line, None)



def _ref_path(ctx: RepoContext, line_name: str) -> Path:
    return ctx.ref_dir / encode_ref_name(line_name)



def read_ref(ctx: RepoContext, line_name: str) -> str | None:
    path = _ref_path(ctx, line_name)
    if not path.exists():
        return None
    text = path.read_text(encoding="utf-8").strip()
    return text or None



def write_ref(ctx: RepoContext, line_name: str, snapshot_id: str | None) -> None:
    path = _ref_path(ctx, line_name)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text((snapshot_id or "") + "\n", encoding="utf-8")



def _blob_path(ctx: RepoContext, blob_id: str) -> Path:
    return ctx.pack_dir / f"{blob_id}.packref"



def _manifest_path(ctx: RepoContext, manifest_hash: str) -> Path:
    return ctx.manifest_dir / f"{manifest_hash}.json"



def _pack_path(ctx: RepoContext, pack_id: str) -> Path:
    return ctx.pack_dir / f"{pack_id}.zip"


def _tree_pack_path(ctx: RepoContext, pack_id: str) -> Path:
    return ctx.tree_pack_dir / f"{pack_id}.zip"



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


def _canonical_snapshot_metadata(
    repo_name: str,
    line_name: str,
    parent_snapshot_id: str | None,
    message: str | None,
    file_entries: list[dict[str, Any]],
    *,
    snapshot_kind: str = "line",
) -> dict[str, Any]:
    root_tree_id, tree_rows, tree_entry_rows = build_tree_records(file_entries)
    snapshot_id, revision_hash = build_snapshot_id(
        repo_name=repo_name,
        line_name=line_name,
        parent_snapshot_id=parent_snapshot_id,
        message=message,
        root_tree_id=root_tree_id,
        snapshot_kind=snapshot_kind,
    )
    return {
        "snapshot_id": snapshot_id,
        "revision_hash": revision_hash,
        "root_tree_id": root_tree_id,
        "manifest_path": f"trees/{root_tree_id}",
        "tree_rows": tree_rows,
        "tree_entry_rows": tree_entry_rows,
    }


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
            blob_item["storage_path"],
            len(data),
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
    conn.commit()
    conn.close()
    return blob_id



def get_line(ctx: RepoContext, line_name: str) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    row = conn.execute("select * from lines where line_name = ?", (line_name,)).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown line: {line_name}")
    out = dict(row)
    out["status"] = out.get("status") or "active"
    out["head_snapshot_id"] = read_ref(ctx, line_name)
    return out



def list_lines(ctx: RepoContext) -> list[dict]:
    conn = connect_sqlite(ctx.content_db_path)
    rows = [dict(r) for r in conn.execute("select * from lines order by line_name")]
    conn.close()
    for row in rows:
        row["status"] = row.get("status") or "active"
        row["head_snapshot_id"] = read_ref(ctx, row["line_name"])
    return rows



def create_line(ctx: RepoContext, line_name: str, from_snapshot_id: Optional[str] = None) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    now = utc_now()
    conn.execute(
        "insert into lines(line_name, status, archived_at, created_at, updated_at) values (?, 'active', null, ?, ?)",
        (line_name, now, now),
    )
    conn.commit()
    conn.close()
    write_ref(ctx, line_name, from_snapshot_id)
    return get_line(ctx, line_name)



def set_line_head(ctx: RepoContext, line_name: str, snapshot_id: Optional[str]) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    row = conn.execute("select * from lines where line_name = ?", (line_name,)).fetchone()
    if row is None:
        now = utc_now()
        conn.execute(
            "insert into lines(line_name, status, archived_at, created_at, updated_at) values (?, 'active', null, ?, ?)",
            (line_name, now, now),
        )
    else:
        if (row["status"] or "active") == "archived":
            conn.close()
            raise ValueError(f"Line {line_name} is archived and cannot move")
        conn.execute("update lines set updated_at = ? where line_name = ?", (utc_now(), line_name))
    conn.commit()
    conn.close()
    write_ref(ctx, line_name, snapshot_id)
    return get_line(ctx, line_name)


def archive_line(ctx: RepoContext, line_name: str) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    row = conn.execute("select * from lines where line_name = ?", (line_name,)).fetchone()
    if row is None:
        conn.close()
        raise KeyError(f"Unknown line: {line_name}")
    if (row["status"] or "active") == "archived":
        conn.close()
        return get_line(ctx, line_name)
    now = utc_now()
    conn.execute(
        "update lines set status = 'archived', archived_at = ?, updated_at = ? where line_name = ?",
        (now, now, line_name),
    )
    conn.commit()
    conn.close()
    return get_line(ctx, line_name)



def snapshot_exists(ctx: RepoContext, snapshot_id: str) -> bool:
    conn = connect_sqlite(ctx.content_db_path)
    row = _snapshot_row(conn, snapshot_id)
    conn.close()
    return row is not None



def create_snapshot(
    ctx: RepoContext,
    repo_name: str,
    line_name: str,
    message: Optional[str],
    *,
    parent_snapshot_id: str | None = None,
    update_line_ref: bool = True,
    snapshot_kind: str = "line",
    touch_line: bool = True,
) -> dict:
    normalized_snapshot_kind = str(snapshot_kind or "line").strip().lower() or "line"
    if normalized_snapshot_kind not in {"line", "stash"}:
        raise ValueError(f"Unsupported snapshot kind: {snapshot_kind}")
    total_started = time.perf_counter()
    conn = connect_sqlite(ctx.content_db_path)
    line_row = conn.execute("select * from lines where line_name = ?", (line_name,)).fetchone()
    if line_row is None:
        conn.close()
        raise KeyError(f"Current line does not exist: {line_name}")
    if (line_row["status"] or "active") == "archived":
        conn.close()
        raise ValueError(f"Current line {line_name} is archived and cannot create snapshots")
    resolved_parent_snapshot_id = parent_snapshot_id if parent_snapshot_id is not None else read_ref(ctx, line_name)

    phase_timings_ms: dict[str, Any] = {}
    entries: list[dict] = []
    new_blob_rows: list[dict[str, Any]] = []
    total_bytes = 0
    effective_ignore_rules = _effective_workspace_ignore_rules(ctx)
    visible_paths = _workspace_visible_files(ctx.root, ignore_rules=effective_ignore_rules, phase_timings_ms=phase_timings_ms)

    projection_started = time.perf_counter()
    materialized_paths: list[tuple[Path, str]] = []
    for path in visible_paths:
        rel = path.relative_to(ctx.root).as_posix()
        if _path_is_projected_out_for_workspace(ctx, rel):
            continue
        materialized_paths.append((path, rel))
    phase_timings_ms["workspace_projection_filter"] = _elapsed_ms(projection_started)

    materialized_state = _workspace_digest_state(
        ctx.root,
        materialized_paths,
        include_data=True,
        phase_timings_ms=phase_timings_ms,
    )
    for rel in sorted(materialized_state):
        path_state = materialized_state[rel]
        digest = str(path_state["sha256"])
        blob_id = f"BLB-{digest[:20]}"
        size = int(path_state["size_bytes"])
        total_bytes += size
        blob_storage = _blob_path(ctx, blob_id)
        existing = _blob_row(conn, blob_id)
        data = path_state.get("data")
        if existing is None:
            if data is None:
                data = Path(path_state["abs_path"]).read_bytes()
            new_blob_rows.append(
                {
                    "blob_id": blob_id,
                    "sha256": digest,
                    "storage_path": str(blob_storage.relative_to(ctx.root)),
                    "size_bytes": size,
                    "data": data,
                    "entry_name": f"blobs/{blob_id}",
                    "path_hint": rel,
                }
            )
        entries.append({
            "path": rel,
            "blob_id": blob_id,
            "size_bytes": size,
            "mode": str(path_state["mode"]),
            "sha256": digest,
        })

    projected_parent_started = time.perf_counter()
    if resolved_parent_snapshot_id is not None:
        existing_paths = {entry["path"] for entry in entries}
        parent_snapshot_files = _snapshot_file_map(conn, resolved_parent_snapshot_id)
        for rel, parent_entry in sorted(parent_snapshot_files.items()):
            if (
                rel in existing_paths
                or not _path_is_projected_out_for_task_worktree(ctx, rel)
                or _is_lineage_only_markdown_artifact_path(rel)
            ):
                continue
            blob = _blob_row(conn, parent_entry["blob_id"])
            if blob is None:
                conn.close()
                raise KeyError(f"Unknown blob: {parent_entry['blob_id']}")
            size_bytes = int(parent_entry["size_bytes"] or 0)
            total_bytes += size_bytes
            entries.append(
                {
                    "path": rel,
                    "blob_id": parent_entry["blob_id"],
                    "size_bytes": size_bytes,
                    "mode": parent_entry["mode"],
                    "sha256": blob["sha256"],
                }
            )
    phase_timings_ms["projected_parent_reuse"] = _elapsed_ms(projected_parent_started)

    snapshot_meta = _canonical_snapshot_metadata(
        repo_name,
        line_name,
        resolved_parent_snapshot_id,
        message,
        entries,
        snapshot_kind=normalized_snapshot_kind,
    )
    snapshot_id = snapshot_meta["snapshot_id"]
    created_at = utc_now()

    blob_pack_ms = 0.0
    if new_blob_rows:
        initial_by_path = _parent_delta_candidates(
            ctx,
            conn,
            resolved_parent_snapshot_id,
            {str(row.get("path_hint") or "") for row in new_blob_rows if row.get("path_hint")},
        )
        blob_pack_started = time.perf_counter()
        pack_id, members_by_blob_id = _write_packed_blobs(
            ctx,
            conn,
            snapshot_id,
            created_at,
            new_blob_rows,
            initial_by_path=initial_by_path,
        )
        blob_pack_ms = _elapsed_ms(blob_pack_started)
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

    tree_record_stage_started = time.perf_counter()
    _insert_tree_records(conn, snapshot_meta["tree_rows"], snapshot_meta["tree_entry_rows"], created_at)
    phase_timings_ms["tree_record_stage"] = _elapsed_ms(tree_record_stage_started)

    tree_pack_started = time.perf_counter()
    _write_tree_pack(
        ctx,
        conn,
        [str(row["tree_id"]) for row in snapshot_meta["tree_rows"]],
        created_at=created_at,
        seed_hint=f"{snapshot_id}|{snapshot_meta['root_tree_id']}",
    )
    tree_pack_ms = _elapsed_ms(tree_pack_started)
    phase_timings_ms["pack_archive_write"] = {
        "blob_pack_write": blob_pack_ms,
        "tree_pack_write": tree_pack_ms,
        "total": round(blob_pack_ms + tree_pack_ms, 3),
    }

    metadata_commit_started = time.perf_counter()
    manifest_path = _manifest_path_for_tree(conn, snapshot_meta["root_tree_id"])
    if _snapshot_row(conn, snapshot_id) is None:
        conn.execute(
            """
            insert into snapshots(
                snapshot_id, parent_snapshot_id, root_tree_id, manifest_hash, manifest_path,
                message, line_name, snapshot_kind, file_count, total_bytes, created_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                snapshot_id,
                resolved_parent_snapshot_id,
                snapshot_meta["root_tree_id"],
                snapshot_meta["revision_hash"],
                manifest_path,
                message,
                line_name,
                normalized_snapshot_kind,
                len(entries),
                total_bytes,
                created_at,
            ),
        )
    if touch_line:
        conn.execute("update lines set updated_at = ? where line_name = ?", (created_at, line_name))
    conn.commit()
    conn.close()
    if update_line_ref:
        write_ref(ctx, line_name, snapshot_id)
    phase_timings_ms["metadata_commit"] = _elapsed_ms(metadata_commit_started)
    phase_timings_ms["total"] = _elapsed_ms(total_started)

    snapshot = get_snapshot(ctx, snapshot_id)
    snapshot["ignore_policy"] = _workspace_ignore_policy_for_rules(ctx.root, effective_ignore_rules)
    snapshot["phase_timings_ms"] = phase_timings_ms
    return snapshot


def list_snapshots(ctx: RepoContext) -> list[dict]:
    conn = connect_sqlite(ctx.content_db_path)
    rows = [
        dict(r)
        for r in conn.execute(
            "select * from snapshots where coalesce(snapshot_kind, 'line') = 'line' order by created_at desc, rowid desc"
        )
    ]
    conn.close()
    return rows


def _stash_select_sql() -> str:
    return """
        select
            st.stash_id,
            st.snapshot_id,
            st.source_line_name,
            st.base_snapshot_id,
            st.message as stash_message,
            st.workspace_cleared,
            st.created_at as stash_created_at,
            s.parent_snapshot_id,
            s.file_count,
            s.total_bytes,
            s.snapshot_kind,
            s.created_at as snapshot_created_at,
            s.message as snapshot_message
        from stashes st
        join snapshots s on s.snapshot_id = st.snapshot_id
    """


def _stash_view(row: sqlite3.Row | None) -> dict[str, Any]:
    if row is None:
        raise KeyError("Unknown stash")
    payload = dict(row)
    return {
        "stash_id": payload["stash_id"],
        "snapshot_id": payload["snapshot_id"],
        "source_line_name": payload["source_line_name"],
        "base_snapshot_id": payload["base_snapshot_id"],
        "message": payload.get("stash_message") if payload.get("stash_message") is not None else payload.get("snapshot_message"),
        "workspace_cleared": bool(payload.get("workspace_cleared")),
        "created_at": payload["stash_created_at"],
        "snapshot_created_at": payload["snapshot_created_at"],
        "snapshot_kind": payload.get("snapshot_kind") or "line",
        "parent_snapshot_id": payload.get("parent_snapshot_id"),
        "file_count": int(payload.get("file_count") or 0),
        "total_bytes": int(payload.get("total_bytes") or 0),
    }


def create_stash(
    ctx: RepoContext,
    *,
    snapshot_id: str,
    source_line_name: str,
    base_snapshot_id: str | None,
    message: str | None,
    workspace_cleared: bool,
) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    snapshot_row = _snapshot_row(conn, snapshot_id)
    if snapshot_row is None:
        conn.close()
        raise KeyError(f"Unknown snapshot: {snapshot_id}")
    if (snapshot_row["snapshot_kind"] or "line") != "stash":
        conn.close()
        raise ValueError(f"Snapshot {snapshot_id} is not a stash snapshot.")
    stash_id = f"STH-{secrets.token_hex(10).upper()}"
    created_at = utc_now()
    conn.execute(
        """
        insert into stashes(
            stash_id, snapshot_id, source_line_name, base_snapshot_id, message, workspace_cleared, created_at
        ) values (?, ?, ?, ?, ?, ?, ?)
        """,
        (
            stash_id,
            snapshot_id,
            source_line_name,
            base_snapshot_id,
            message,
            1 if workspace_cleared else 0,
            created_at,
        ),
    )
    row = conn.execute(f"{_stash_select_sql()} where st.stash_id = ?", (stash_id,)).fetchone()
    conn.commit()
    conn.close()
    return _stash_view(row)


def list_stashes(ctx: RepoContext) -> list[dict]:
    conn = connect_sqlite(ctx.content_db_path)
    rows = [
        _stash_view(row)
        for row in conn.execute(f"{_stash_select_sql()} order by st.created_at desc, st.stash_id desc")
    ]
    conn.close()
    return rows


def get_stash(ctx: RepoContext, stash_id: str) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    row = conn.execute(f"{_stash_select_sql()} where st.stash_id = ?", (stash_id,)).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown stash: {stash_id}")
    return _stash_view(row)


def drop_stash(ctx: RepoContext, stash_id: str) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    row = conn.execute(f"{_stash_select_sql()} where st.stash_id = ?", (stash_id,)).fetchone()
    if row is None:
        conn.close()
        raise KeyError(f"Unknown stash: {stash_id}")
    stash = _stash_view(row)
    conn.execute("delete from stashes where stash_id = ?", (stash_id,))
    remaining = conn.execute("select count(*) as c from stashes where snapshot_id = ?", (stash["snapshot_id"],)).fetchone()["c"]
    if remaining == 0:
        conn.execute(
            "delete from snapshots where snapshot_id = ? and coalesce(snapshot_kind, 'line') = 'stash'",
            (stash["snapshot_id"],),
        )
    conn.commit()
    conn.close()
    return {
        **stash,
        "dropped": True,
        "snapshot_deleted": remaining == 0,
    }



def get_snapshot(ctx: RepoContext, snapshot_id: str) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    snap = _snapshot_row(conn, snapshot_id)
    if snap is None:
        conn.close()
        raise KeyError(f"Unknown snapshot: {snapshot_id}")
    files = [
        dict(r)
        for r in conn.execute(
            "select path, blob_id, size_bytes, mode from snapshot_files where snapshot_id = ? order by path",
            (snapshot_id,),
        )
    ]
    conn.close()
    out = dict(snap)
    out["files"] = files
    return out


def _parse_mode_bits(mode: str | None) -> int:
    text = str(mode or "0o644").strip() or "0o644"
    try:
        return int(text, 0)
    except ValueError:
        return int(text, 8)


def _snapshot_file_map(conn, snapshot_id: str | None) -> dict[str, dict]:
    if snapshot_id is None:
        return {}
    snap = _snapshot_row(conn, snapshot_id)
    if snap is None:
        raise KeyError(f"Unknown snapshot: {snapshot_id}")
    root_tree_id = normalize_optional_text(snap["root_tree_id"])
    if root_tree_id is not None:
        return _snapshot_file_map_via_tree_traversal(conn, root_tree_id)
    return _snapshot_file_map_via_view(conn, snapshot_id)


def _snapshot_file_map_via_view(conn, snapshot_id: str) -> dict[str, dict]:
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
            "path": row["path"],
            "blob_id": row["blob_id"],
            "sha256": row["sha256"],
            "size_bytes": row["size_bytes"],
            "mode": row["mode"],
        }
        for row in rows
    }


def _blob_sha256_map(conn, blob_ids: list[str]) -> dict[str, str]:
    if not blob_ids:
        return {}
    rows_by_blob: dict[str, str] = {}
    chunk_size = 400
    for start in range(0, len(blob_ids), chunk_size):
        chunk = blob_ids[start:start + chunk_size]
        placeholders = ", ".join("?" for _ in chunk)
        rows = conn.execute(
            f"select blob_id, sha256 from blobs where blob_id in ({placeholders})",
            tuple(chunk),
        ).fetchall()
        for row in rows:
            blob_id = str(row["blob_id"] or "").strip()
            sha256 = str(row["sha256"] or "").strip()
            if blob_id:
                rows_by_blob[blob_id] = sha256
    return rows_by_blob


def _snapshot_file_map_via_tree_traversal(conn, root_tree_id: str) -> dict[str, dict]:
    tree_rows: list[dict[str, Any]] = []
    stack: list[tuple[str, str]] = [("", root_tree_id)]
    while stack:
        prefix, tree_id = stack.pop()
        rows = conn.execute(
            """
            select entry_name, entry_type, target_id, size_bytes, mode
            from tree_entries
            where tree_id = ?
            order by entry_name asc
            """,
            (tree_id,),
        ).fetchall()
        for row in rows:
            entry_name = str(row["entry_name"] or "")
            path = f"{prefix}{entry_name}"
            entry_type = str(row["entry_type"] or "")
            target_id = str(row["target_id"] or "")
            if entry_type == "blob":
                tree_rows.append(
                    {
                        "path": path,
                        "blob_id": target_id,
                        "size_bytes": row["size_bytes"],
                        "mode": row["mode"],
                    }
                )
            elif entry_type == "tree" and target_id:
                stack.append((path + "/", target_id))

    sha256_by_blob = _blob_sha256_map(conn, sorted({str(row["blob_id"]) for row in tree_rows}))
    return {
        row["path"]: {
            "path": row["path"],
            "blob_id": row["blob_id"],
            "sha256": sha256_by_blob.get(str(row["blob_id"]), ""),
            "size_bytes": row["size_bytes"],
            "mode": row["mode"],
        }
        for row in sorted(tree_rows, key=lambda item: str(item["path"]))
    }


def workspace_delta(
    ctx: RepoContext,
    snapshot_id: str | None,
    *,
    ignore_rules: tuple[WorkspaceIgnoreRule, ...] | None = None,
) -> dict:
    total_started = time.perf_counter()
    conn = connect_sqlite(ctx.content_db_path)
    baseline_read_started = time.perf_counter()
    snapshot_files = _filter_snapshot_file_map_for_workspace(ctx, _snapshot_file_map(conn, snapshot_id))
    conn.close()

    effective_ignore_rules = _effective_workspace_ignore_rules(ctx, ignore_rules)
    snapshot_files = {
        path: snapshot_file
        for path, snapshot_file in snapshot_files.items()
        if not _workspace_path_is_ignored(Path(path), effective_ignore_rules)
    }
    phase_timings_ms: dict[str, Any] = {
        "baseline_snapshot_read": _elapsed_ms(baseline_read_started),
    }
    workspace_files = _workspace_state(
        ctx.root,
        ignore_rules=effective_ignore_rules,
        phase_timings_ms=phase_timings_ms,
    )
    projection_started = time.perf_counter()
    workspace_files = _filter_workspace_state_for_workspace(ctx, workspace_files)
    phase_timings_ms["workspace_projection_filter"] = _elapsed_ms(projection_started)

    compare_started = time.perf_counter()
    modified_paths: list[str] = []
    missing_paths: list[str] = []
    for path, snapshot_file in snapshot_files.items():
        current = workspace_files.pop(path, None)
        if current is None:
            missing_paths.append(path)
            continue
        if current["sha256"] != snapshot_file["sha256"] or current["mode"] != snapshot_file["mode"]:
            modified_paths.append(path)
    untracked_paths = sorted(workspace_files)
    modified_paths = sorted(modified_paths)
    missing_paths = sorted(missing_paths)
    changed_paths = sorted({*modified_paths, *missing_paths, *untracked_paths})
    phase_timings_ms["compare_manifest"] = _elapsed_ms(compare_started)
    phase_timings_ms["total"] = _elapsed_ms(total_started)
    return {
        "snapshot_id": snapshot_id,
        "clean": len(changed_paths) == 0,
        "changed_count": len(changed_paths),
        "changed_paths": changed_paths,
        "modified_paths": modified_paths,
        "missing_paths": missing_paths,
        "untracked_paths": untracked_paths,
        "ignore_policy": _workspace_ignore_policy_for_rules(ctx.root, effective_ignore_rules),
        "phase_timings_ms": phase_timings_ms,
    }


def _prune_empty_parent_dirs(root: Path, path: Path) -> None:
    cur = path.parent
    while cur != root and cur.exists():
        try:
            cur.rmdir()
        except OSError:
            break
        cur = cur.parent


def restore_workspace(
    ctx: RepoContext,
    target_snapshot_id: str | None,
    *,
    baseline_snapshot_id: str | None = None,
    force: bool = False,
    dry_run: bool = False,
) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    target_files = _filter_snapshot_file_map_for_workspace(ctx, _snapshot_file_map(conn, target_snapshot_id))
    snapshot_ignore_rules = _snapshot_workspace_ignore_rules(
        target_snapshot_id,
        target_files,
        lambda blob_id: _blob_bytes_by_id(ctx, conn, blob_id).decode("utf-8", errors="replace"),
    )
    effective_ignore_rules = _effective_workspace_ignore_rules(ctx, snapshot_ignore_rules)
    dirty = workspace_delta(ctx, baseline_snapshot_id, ignore_rules=snapshot_ignore_rules)
    workspace_files = _filter_workspace_state_for_workspace(
        ctx,
        _workspace_state(ctx.root, ignore_rules=effective_ignore_rules),
    )

    write_paths: list[str] = []
    for path, target in target_files.items():
        current = workspace_files.pop(path, None)
        if current is None or current["sha256"] != target["sha256"] or current["mode"] != target["mode"]:
            write_paths.append(path)
    remove_paths = sorted(workspace_files, key=lambda item: (item.count("/"), item), reverse=True)
    result = {
        "target_snapshot_id": target_snapshot_id,
        "baseline_snapshot_id": baseline_snapshot_id,
        "force": force,
        "dry_run": dry_run,
        "applied": False,
        "workspace_dirty": not dirty["clean"],
        "would_overwrite_workspace_changes": not dirty["clean"],
        "dirty_workspace": dirty,
        "plan": {
            "write_count": len(write_paths),
            "remove_count": len(remove_paths),
            "unchanged_count": max(len(target_files) - len(write_paths), 0),
            "write_paths": sorted(write_paths),
            "remove_paths": remove_paths,
        },
    }
    if dirty["changed_count"] and not force and not dry_run:
        sample = ", ".join(dirty["changed_paths"][:5])
        if len(dirty["changed_paths"]) > 5:
            sample += ", ..."
        baseline_label = baseline_snapshot_id or "empty workspace"
        conn.close()
        raise ValueError(f"Workspace has unsaved changes relative to {baseline_label}: {sample}")
    if dry_run:
        conn.close()
        return result

    for rel in remove_paths:
        abs_path = ctx.root / rel
        if abs_path.exists():
            abs_path.unlink()
            _prune_empty_parent_dirs(ctx.root, abs_path)

    for rel in sorted(write_paths):
        target = target_files[rel]
        abs_path = ctx.root / rel
        abs_path.parent.mkdir(parents=True, exist_ok=True)
        if abs_path.exists() and abs_path.is_dir():
            conn.close()
            raise IsADirectoryError(f"Cannot restore file over directory: {rel}")
        data = _blob_bytes_by_id(ctx, conn, target["blob_id"])
        abs_path.write_bytes(data)
        abs_path.chmod(_parse_mode_bits(target["mode"]))

    conn.close()
    result["applied"] = True
    return result


def restore_workspace_paths(
    ctx: RepoContext,
    target_snapshot_id: str | None,
    paths: Iterable[str | Path],
    *,
    baseline_snapshot_id: str | None = None,
    force: bool = False,
    dry_run: bool = False,
) -> dict:
    requested_paths = sorted({_normalize_workspace_restore_path(path) for path in paths})
    conn = connect_sqlite(ctx.content_db_path)
    target_files = _filter_snapshot_file_map_for_workspace(ctx, _snapshot_file_map(conn, target_snapshot_id))
    snapshot_ignore_rules = _snapshot_workspace_ignore_rules(
        target_snapshot_id,
        target_files,
        lambda blob_id: _blob_bytes_by_id(ctx, conn, blob_id).decode("utf-8", errors="replace"),
    )
    effective_ignore_rules = _effective_workspace_ignore_rules(ctx, snapshot_ignore_rules)
    dirty = workspace_delta(ctx, baseline_snapshot_id, ignore_rules=snapshot_ignore_rules)
    workspace_files = _filter_workspace_state_for_workspace(
        ctx,
        _workspace_state(ctx.root, ignore_rules=effective_ignore_rules),
    )

    requested_set = set(requested_paths)
    dirty_selected_paths = sorted(set(dirty["changed_paths"]) & requested_set)
    dirty_outside_paths = sorted(set(dirty["changed_paths"]) - requested_set)

    write_paths: list[str] = []
    remove_paths: list[str] = []
    unchanged_paths: list[str] = []
    for rel in requested_paths:
        target = target_files.get(rel)
        current = workspace_files.get(rel)
        if target is None:
            if current is None:
                unchanged_paths.append(rel)
            else:
                remove_paths.append(rel)
            continue
        if current is None or current["sha256"] != target["sha256"] or current["mode"] != target["mode"]:
            write_paths.append(rel)
        else:
            unchanged_paths.append(rel)

    result = {
        "target_snapshot_id": target_snapshot_id,
        "baseline_snapshot_id": baseline_snapshot_id,
        "force": force,
        "dry_run": dry_run,
        "applied": False,
        "workspace_dirty": not dirty["clean"],
        "would_overwrite_selected_changes": bool(dirty_selected_paths),
        "dirty_workspace": dirty,
        "dirty_selected_paths": dirty_selected_paths,
        "dirty_outside_paths": dirty_outside_paths,
        "plan": {
            "write_count": len(write_paths),
            "remove_count": len(remove_paths),
            "unchanged_count": len(unchanged_paths),
            "requested_paths": requested_paths,
            "write_paths": sorted(write_paths),
            "remove_paths": sorted(remove_paths, key=lambda item: (item.count("/"), item), reverse=True),
            "unchanged_paths": unchanged_paths,
        },
    }
    if dirty_selected_paths and not force and not dry_run:
        sample = ", ".join(dirty_selected_paths[:5])
        if len(dirty_selected_paths) > 5:
            sample += ", ..."
        baseline_label = baseline_snapshot_id or "empty workspace"
        conn.close()
        raise ValueError(f"Selected paths have unsaved changes relative to {baseline_label}: {sample}")
    if dry_run:
        conn.close()
        return result

    for rel in result["plan"]["remove_paths"]:
        abs_path = ctx.root / rel
        if abs_path.exists():
            abs_path.unlink()
            _prune_empty_parent_dirs(ctx.root, abs_path)

    for rel in sorted(write_paths):
        target = target_files[rel]
        abs_path = ctx.root / rel
        abs_path.parent.mkdir(parents=True, exist_ok=True)
        if abs_path.exists() and abs_path.is_dir():
            conn.close()
            raise IsADirectoryError(f"Cannot restore file over directory: {rel}")
        data = _blob_bytes_by_id(ctx, conn, target["blob_id"])
        abs_path.write_bytes(data)
        abs_path.chmod(_parse_mode_bits(target["mode"]))

    conn.close()
    result["applied"] = True
    return result


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
                b.storage_kind,
                b.pack_id,
                b.pack_entry_name,
                min(sf.path) as path_hint,
                min(s.created_at) as first_seen_at
            from blobs b
            join snapshot_files sf on sf.blob_id = b.blob_id
            join snapshots s on s.snapshot_id = sf.snapshot_id
            where """
            + where_clause
            + """
            group by b.blob_id, b.sha256, b.storage_path, b.size_bytes, b.storage_kind, b.pack_id, b.pack_entry_name
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
                # Repack the surviving tree metadata so stale archives do not linger
                # after GC removes unreachable trees.
                conn.execute(
                    """
                    update trees
                    set tree_pack_id = null,
                        tree_pack_entry_name = null,
                        tree_pack_checksum = null,
                        tree_packed_at = null
                    """
                )
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
