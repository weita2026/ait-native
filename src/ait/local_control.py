from __future__ import annotations

import json
import secrets
import sqlite3
import time
from pathlib import Path
from typing import Optional

from ait_protocol.common import connect_sqlite, extract_plan_items, normalize_plan_items, utc_now
from .repo_paths import RepoContext
from .task_statuses import (
    TASK_LOCAL_CLOSE_TARGET_STATUSES,
    TASK_STATUS_COMPLETED,
    TASK_STATUS_LATER_PROMOTION_EXCLUDED,
    is_task_abandoned_status,
    normalize_task_status,
)


from .local_workflow_sessions import (
    append_workflow_session_event,
    close_workflow_session,
    create_workflow_checkpoint,
    create_workflow_session,
    get_workflow_checkpoint,
    get_workflow_session,
    get_workflow_snapshot_provenance,
    list_workflow_checkpoints,
    list_workflow_session_events,
    list_workflow_sessions,
    list_workflow_snapshot_provenance,
    list_workflow_snapshot_provenance_for_change,
    record_workflow_snapshot_provenance,
    resume_workflow_session,
)

from .local_workflow_identity import (
    LOCAL_IDENTITY_SOURCE_LEGACY,
    LOCAL_IDENTITY_SOURCE_SEQUENCE,
    _ensure_local_task_change_identity_schema,
    _normalize_change_identity_metadata,
    _normalize_task_identity_metadata,
    _register_workflow_change_alias,
    _register_workflow_task_alias,
    _resolve_workflow_change_id,
    _resolve_workflow_task_id,
    _set_workflow_sequence_floor,
    _workflow_sequence_from_id,
    allocate_workflow_change_identity,
    allocate_workflow_task_identity,
    get_workflow_sequence_floor,
    workflow_sequence_from_id,
)
from .local_workflow_releases import (
    WORKFLOW_RELEASE_UNSET,
    create_workflow_release,
    get_workflow_release,
    list_workflow_releases,
    update_workflow_release,
)
from .local_workflow_plans import (
    close_workflow_plan,
    create_workflow_plan,
    get_workflow_plan,
    get_workflow_plan_revision,
    get_workflow_plan_revision_by_id,
    list_workflow_plan_revisions,
    list_workflow_plans,
    mark_workflow_plan_published,
    revise_workflow_plan,
)

SCHEMA = """
create table if not exists control_meta (
    key text primary key,
    value text not null
);

create table if not exists remotes (
    remote_id integer primary key autoincrement,
    name text not null unique,
    url text not null,
    repo_name text,
    is_default_push integer not null default 0,
    is_default_pull integer not null default 0,
    created_at text not null
);

create table if not exists events (
    event_id integer primary key autoincrement,
    event_type text not null,
    entity_type text not null,
    entity_id text not null,
    payload_json text not null,
    created_at text not null
);

create table if not exists workflow_tasks (
    task_id text primary key,
    task_seq integer,
    identity_source text not null default 'local_sequence',
    repo_name text not null,
    title text not null,
    intent text not null,
    risk_tier text not null,
    planning_state text not null,
    plan_id text,
    origin_plan_revision_id text,
    plan_item_ref text,
    plan_linked_at text,
    status text not null,
    publication_state text not null,
    published_remote_name text,
    published_task_id text,
    published_at text,
    created_at text not null,
    updated_at text not null
);

create table if not exists workflow_changes (
    change_id text primary key,
    change_seq integer,
    identity_source text not null default 'local_sequence',
    task_id text not null,
    repo_name text not null,
    title text not null,
    base_line text not null,
    fork_snapshot_id text,
    forked_from_line text,
    risk_tier text not null,
    lane text not null,
    status text not null,
    publication_state text not null,
    published_remote_name text,
    published_change_id text,
    published_at text,
    target_line text,
    pre_land_target_snapshot_id text,
    landed_snapshot_id text,
    landed_at text,
    created_at text not null,
    updated_at text not null,
    foreign key(task_id) references workflow_tasks(task_id)
);

create table if not exists workflow_plans (
    plan_id text primary key,
    repo_name text not null,
    title text not null,
    status text not null,
    head_revision_id text not null,
    publication_state text not null,
    published_remote_name text,
    published_plan_id text,
    published_head_revision_id text,
    published_at text,
    created_by text,
    created_at text not null,
    updated_at text not null
);

create table if not exists workflow_plan_revisions (
    plan_revision_id text primary key,
    plan_id text not null,
    revision_number integer not null,
    parent_plan_revision_id text,
    title_snapshot text not null,
    summary text,
    artifact_path text,
    artifact_selector text,
    artifact_heading text,
    artifact_blob_id text,
    items_json text not null,
    source_kind text not null,
    source_session_id text,
    created_by text,
    actor_type text,
    publication_state text not null,
    published_plan_revision_id text,
    published_at text,
    created_at text not null,
    foreign key(plan_id) references workflow_plans(plan_id)
);

create table if not exists workflow_releases (
    release_id text primary key,
    repo_name text not null,
    version text not null,
    line_name text not null,
    snapshot_id text not null,
    manifest_hash text not null,
    profile text not null,
    package_name text,
    package_version text,
    package_requires_python text,
    status text not null,
    checks_json text not null default '[]',
    artifacts_json text not null default '[]',
    formula_json text not null default '{}',
    metadata_json text not null default '{}',
    created_at text not null,
    updated_at text not null
);

create table if not exists workflow_sessions (
    session_id text primary key,
    repo_name text not null,
    task_id text,
    change_id text,
    title text,
    session_kind text not null,
    status text not null,
    line_name text,
    worktree_name text,
    model_name text,
    actor_identity text,
    actor_type text,
    metadata_json text not null,
    last_event_sequence integer not null default 0,
    head_checkpoint_id text,
    created_at text not null,
    updated_at text not null,
    foreign key(task_id) references workflow_tasks(task_id),
    foreign key(change_id) references workflow_changes(change_id)
);

create table if not exists workflow_session_events (
    session_id text not null,
    sequence integer not null,
    event_type text not null,
    payload_json text not null,
    actor_identity text not null,
    actor_type text not null,
    created_at text not null,
    primary key(session_id, sequence),
    foreign key(session_id) references workflow_sessions(session_id)
);

create table if not exists workflow_checkpoints (
    checkpoint_id text primary key,
    session_id text not null,
    based_on_sequence integer not null,
    summary text not null,
    snapshot_id text,
    resume_payload_json text not null,
    created_at text not null,
    foreign key(session_id) references workflow_sessions(session_id)
);

create table if not exists workflow_snapshot_provenance (
    snapshot_id text primary key,
    task_id text,
    change_id text,
    session_id text,
    checkpoint_id text,
    worktree_name text,
    line_name text,
    author_mode text,
    model_name text,
    created_at text not null
);
"""

LOCAL_CONTROL_SCHEMA_VERSION = 1
_LOCAL_CONTROL_INIT_MIGRATION_BACKUP_PREFIX = "control.db.before-local-control-init-migration"
_LEGACY_WORKFLOW_CHANGE_CLOSE_COLUMNS = (
    "closed_reason",
    "superseded_by_change_id",
    "closed_at",
)
_LOCAL_PLAN_LINK_DIFF_METADATA_COLUMNS = (
    "plan_links_surface_hash",
    "plan_links_changed_count_to_prev",
)


def _ensure_schema(conn) -> None:
    conn.executescript(SCHEMA)
    _ensure_column(conn, "workflow_tasks", "planning_state", "text not null default 'unplanned'")
    _ensure_column(conn, "workflow_tasks", "plan_id", "text")
    _ensure_column(conn, "workflow_tasks", "origin_plan_revision_id", "text")
    _ensure_column(conn, "workflow_tasks", "plan_item_ref", "text")
    _ensure_column(conn, "workflow_tasks", "plan_linked_at", "text")
    _ensure_column(conn, "workflow_changes", "target_line", "text")
    _ensure_column(conn, "workflow_changes", "pre_land_target_snapshot_id", "text")
    _ensure_column(conn, "workflow_changes", "landed_snapshot_id", "text")
    _ensure_column(conn, "workflow_changes", "landed_at", "text")
    _ensure_column(conn, "workflow_changes", "fork_snapshot_id", "text")
    _ensure_column(conn, "workflow_changes", "forked_from_line", "text")
    _ensure_column(conn, "workflow_plan_revisions", "artifact_path", "text")
    _ensure_column(conn, "workflow_plan_revisions", "artifact_selector", "text")
    _ensure_column(conn, "workflow_plan_revisions", "artifact_heading", "text")
    _ensure_column(conn, "workflow_plan_revisions", "artifact_blob_id", "text")
    _ensure_column(conn, "workflow_plan_revisions", "items_json", "text not null default '[]'")
    _ensure_column(conn, "workflow_releases", "checks_json", "text not null default '[]'")
    _ensure_column(conn, "workflow_releases", "artifacts_json", "text not null default '[]'")
    _ensure_column(conn, "workflow_releases", "formula_json", "text not null default '{}'")
    _ensure_column(conn, "workflow_releases", "metadata_json", "text not null default '{}'")
    _ensure_local_task_change_identity_schema(conn)
    _migrate_workflow_plan_revisions(conn)
    _remove_local_plan_link_diff_metadata_columns(conn)
    _remove_historical_publication_identity_columns(conn)
    conn.execute(
        "create index if not exists idx_workflow_snapshot_provenance_task on workflow_snapshot_provenance(task_id)"
    )
    conn.execute(
        "create index if not exists idx_workflow_snapshot_provenance_change on workflow_snapshot_provenance(change_id)"
    )
    conn.execute(
        "create index if not exists idx_workflow_snapshot_provenance_session on workflow_snapshot_provenance(session_id)"
    )


def _ensure_column(conn, table_name: str, column_name: str, ddl: str) -> None:
    columns = {row[1] for row in conn.execute(f"pragma table_info({table_name})")}
    if column_name in columns:
        return
    conn.execute(f"alter table {table_name} add column {column_name} {ddl}")


def _table_columns(conn, table_name: str) -> set[str]:
    return {row[1] for row in conn.execute(f"pragma table_info({table_name})")}


def _drop_column_if_exists(conn, table_name: str, column_name: str) -> bool:
    if column_name not in _table_columns(conn, table_name):
        return False
    try:
        conn.execute(f"alter table {table_name} drop column {column_name}")
    except sqlite3.Error:
        return False
    return column_name not in _table_columns(conn, table_name)


def _legacy_workflow_change_close_columns_present(conn) -> tuple[str, ...]:
    columns = _table_columns(conn, "workflow_changes")
    return tuple(column_name for column_name in _LEGACY_WORKFLOW_CHANGE_CLOSE_COLUMNS if column_name in columns)


def _local_plan_link_diff_metadata_columns_present(conn) -> tuple[str, ...]:
    columns = _table_columns(conn, "workflow_plan_revisions")
    return tuple(column_name for column_name in _LOCAL_PLAN_LINK_DIFF_METADATA_COLUMNS if column_name in columns)


def _control_db_explicit_init_migration_needed(conn) -> bool:
    return bool(
        _legacy_workflow_change_close_columns_present(conn)
        or _local_plan_link_diff_metadata_columns_present(conn)
    )


def _write_init_migration_backup(conn, ctx: RepoContext) -> Path:
    backup_root = ctx.ait_dir / "repair_backups"
    backup_root.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    backup_path = backup_root / (
        f"{_LOCAL_CONTROL_INIT_MIGRATION_BACKUP_PREFIX}.{stamp}.{secrets.token_hex(2)}.bak"
    )
    backup_conn = sqlite3.connect(backup_path)
    try:
        conn.backup(backup_conn)
    finally:
        backup_conn.close()
    return backup_path


def _set_local_control_schema_version(conn, version: int = LOCAL_CONTROL_SCHEMA_VERSION) -> None:
    conn.execute(f"pragma user_version = {int(version)}")


def _remove_historical_publication_identity_columns(conn) -> None:
    for table_name, column_name in (
        ("workflow_tasks", "historical_publication_remote_name"),
        ("workflow_tasks", "historical_publication_id"),
        ("workflow_tasks", "historical_published_task_id"),
        ("workflow_tasks", "historical_published_at"),
        ("workflow_changes", "historical_publication_remote_name"),
        ("workflow_changes", "historical_publication_id"),
        ("workflow_changes", "historical_published_change_id"),
        ("workflow_changes", "historical_published_at"),
    ):
        _drop_column_if_exists(conn, table_name, column_name)


def _remove_local_plan_link_diff_metadata_columns(conn) -> tuple[str, ...]:
    removed: list[str] = []
    for column_name in _LOCAL_PLAN_LINK_DIFF_METADATA_COLUMNS:
        if _drop_column_if_exists(conn, "workflow_plan_revisions", column_name):
            removed.append(column_name)
    return tuple(removed)


def _remove_legacy_workflow_change_close_columns(conn) -> tuple[str, ...]:
    removed: list[str] = []
    for column_name in _LEGACY_WORKFLOW_CHANGE_CLOSE_COLUMNS:
        if _drop_column_if_exists(conn, "workflow_changes", column_name):
            removed.append(column_name)
    return tuple(removed)


def _migrate_workflow_plan_revisions(conn) -> None:
    columns = _table_columns(conn, "workflow_plan_revisions")
    if "body_markdown" not in columns:
        return
    rows = conn.execute(
        """
        select plan_revision_id, title_snapshot, body_markdown, items_json, artifact_heading
        from workflow_plan_revisions
        """
    ).fetchall()
    for row in rows:
        items_json = row["items_json"] if "items_json" in row.keys() else None
        if str(items_json or "").strip() not in {"", "[]"}:
            continue
        items = normalize_plan_items(extract_plan_items(row["body_markdown"]))
        conn.execute(
            """
            update workflow_plan_revisions
            set items_json = ?,
                artifact_heading = coalesce(artifact_heading, ?)
            where plan_revision_id = ?
            """,
            (
                json.dumps(items, sort_keys=True),
                row["artifact_heading"] if "artifact_heading" in row.keys() else row["title_snapshot"],
                row["plan_revision_id"],
            ),
        )
    _drop_column_if_exists(conn, "workflow_plan_revisions", "body_markdown")


def _connect_control(ctx: RepoContext):
    return connect_sqlite(ctx.control_db_path)


def initialize(ctx: RepoContext, repo_name: str, default_line: str) -> None:
    existing_db = ctx.control_db_path.exists() and ctx.control_db_path.stat().st_size > 0
    conn = connect_sqlite(ctx.control_db_path)
    if existing_db and _control_db_explicit_init_migration_needed(conn):
        _write_init_migration_backup(conn, ctx)
    _ensure_schema(conn)
    _remove_legacy_workflow_change_close_columns(conn)
    _set_local_control_schema_version(conn)
    meta = {
        "repo_name": repo_name,
        "default_line": default_line,
        "current_line": default_line,
        "storage_profile": "ait-native-split-sqlite",
        "created_at": utc_now(),
    }
    for key, value in meta.items():
        set_meta(ctx, key, value, conn=conn)
    conn.commit()
    conn.close()


def record_event(ctx: RepoContext, event_type: str, entity_type: str, entity_id: str, payload: dict) -> None:
    conn = _connect_control(ctx)
    _record_event(conn, event_type, entity_type, entity_id, payload)
    conn.commit()
    conn.close()


def _record_event(conn, event_type: str, entity_type: str, entity_id: str, payload: dict) -> None:
    conn.execute(
        "insert into events(event_type, entity_type, entity_id, payload_json, created_at) values (?, ?, ?, ?, ?)",
        (event_type, entity_type, entity_id, json.dumps(payload, sort_keys=True), utc_now()),
    )


def get_meta(ctx: RepoContext, key: str, *, conn=None) -> Optional[str]:
    own = False
    if conn is None:
        conn = _connect_control(ctx)
        own = True
    row = conn.execute("select value from control_meta where key = ?", (key,)).fetchone()
    if own:
        conn.close()
    return row["value"] if row else None


def set_meta(ctx: RepoContext, key: str, value: str, *, conn=None) -> None:
    own = False
    if conn is None:
        conn = _connect_control(ctx)
        own = True
    conn.execute("insert or replace into control_meta(key, value) values (?, ?)", (key, value))
    if own:
        conn.commit()
        conn.close()


def add_remote(ctx: RepoContext, name: str, url: str, repo_name: Optional[str], make_default: bool = False) -> dict:
    conn = _connect_control(ctx)
    if make_default:
        conn.execute("update remotes set is_default_push = 0, is_default_pull = 0")
    now = utc_now()
    conn.execute(
        "insert into remotes(name, url, repo_name, is_default_push, is_default_pull, created_at) values (?, ?, ?, ?, ?, ?)",
        (name, url, repo_name, 1 if make_default else 0, 1 if make_default else 0, now),
    )
    _record_event(conn, "remote.added", "remote", name, {"name": name, "url": url, "repo_name": repo_name, "default": make_default})
    conn.commit()
    row = conn.execute("select * from remotes where name = ?", (name,)).fetchone()
    conn.close()
    return dict(row)


def list_remotes(ctx: RepoContext) -> list[dict]:
    conn = _connect_control(ctx)
    rows = [dict(r) for r in conn.execute("select * from remotes order by name")]
    conn.close()
    return rows


def get_remote(ctx: RepoContext, name: str) -> dict:
    conn = _connect_control(ctx)
    row = conn.execute("select * from remotes where name = ?", (name,)).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown remote: {name}")
    return dict(row)


def list_events(ctx: RepoContext, limit: int = 50) -> list[dict]:
    conn = _connect_control(ctx)
    rows = [dict(r) for r in conn.execute("select * from events order by event_id desc limit ?", (limit,))]
    conn.close()
    return rows


def list_line_events(ctx: RepoContext, line_name: str, limit: int = 20) -> list[dict]:
    conn = _connect_control(ctx)
    rows = [
        dict(row)
        for row in conn.execute(
            """
            select * from events
            where event_type = 'line.moved'
              and entity_type = 'line'
              and entity_id = ?
            order by event_id desc
            limit ?
            """,
            (line_name, limit),
        ).fetchall()
    ]
    conn.close()
    out: list[dict] = []
    for row in rows:
        payload_json = row.get("payload_json")
        payload = {}
        if isinstance(payload_json, str) and payload_json.strip():
            try:
                payload = json.loads(payload_json)
            except json.JSONDecodeError:
                payload = {}
        normalized = dict(row)
        normalized["payload"] = payload
        out.append(normalized)
    return out


def create_workflow_task(
    ctx: RepoContext,
    task_id: str,
    repo_name: str,
    title: str,
    intent: str,
    risk_tier: str,
    *,
    task_seq: int | None = None,
    identity_source: str = LOCAL_IDENTITY_SOURCE_SEQUENCE,
    planning_state: str = "unplanned",
    plan_id: str | None = None,
    origin_plan_revision_id: str | None = None,
    plan_item_ref: str | None = None,
    plan_linked_at: str | None = None,
    status: str = "active",
    publication_state: str = "local_draft",
) -> dict:
    conn = _connect_control(ctx)
    now = utc_now()
    resolved_task_seq, resolved_identity_source = _normalize_task_identity_metadata(task_id, task_seq, identity_source)
    conn.execute(
        """
        insert into workflow_tasks(
            task_id, task_seq, identity_source, repo_name, title, intent, risk_tier, planning_state,
            plan_id, origin_plan_revision_id, plan_item_ref, plan_linked_at,
            status, publication_state, published_remote_name, published_task_id, published_at, created_at, updated_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            task_id,
            resolved_task_seq,
            resolved_identity_source,
            repo_name,
            title,
            intent,
            risk_tier,
            planning_state,
            plan_id,
            origin_plan_revision_id,
            plan_item_ref,
            plan_linked_at,
            status,
            publication_state,
            None,
            None,
            None,
            now,
            now,
        ),
    )
    conn.commit()
    row = conn.execute("select * from workflow_tasks where task_id = ?", (task_id,)).fetchone()
    conn.close()
    return dict(row)


def list_workflow_tasks(ctx: RepoContext) -> list[dict]:
    conn = _connect_control(ctx)
    rows = [dict(r) for r in conn.execute("select * from workflow_tasks order by created_at desc, coalesce(task_seq, 0) desc, task_id desc")]
    conn.close()
    return rows


def get_workflow_task(ctx: RepoContext, task_id: str) -> dict:
    conn = _connect_control(ctx)
    resolved_task_id = _resolve_workflow_task_id(conn, task_id)
    row = conn.execute("select * from workflow_tasks where task_id = ?", (resolved_task_id,)).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local task: {task_id}")
    return dict(row)


def find_latest_workflow_task_for_plan_item(
    ctx: RepoContext,
    *,
    repo_name: str,
    plan_id: str,
    plan_item_ref: str,
) -> dict | None:
    conn = _connect_control(ctx)
    row = conn.execute(
        """
        select *
        from workflow_tasks
        where repo_name = ? and plan_id = ? and plan_item_ref = ?
        order by coalesce(task_seq, 0) desc, created_at desc, task_id desc
        limit 1
        """,
        (repo_name, plan_id, plan_item_ref),
    ).fetchone()
    conn.close()
    return dict(row) if row is not None else None


def close_workflow_task(ctx: RepoContext, task_id: str, status: str) -> dict:
    normalized_status = normalize_task_status(status)
    if normalized_status not in TASK_LOCAL_CLOSE_TARGET_STATUSES:
        raise ValueError(f"Unsupported local task close status: {status}")
    conn = _connect_control(ctx)
    resolved_task_id = _resolve_workflow_task_id(conn, task_id)
    row = conn.execute("select * from workflow_tasks where task_id = ?", (resolved_task_id,)).fetchone()
    if row is None:
        conn.close()
        raise KeyError(f"Unknown local task: {task_id}")
    task = dict(row)
    task_status = normalize_task_status(task.get("status"))
    assert task_status is not None
    if task_status == normalized_status:
        conn.close()
        return task
    if task_status != "active":
        if task_status != TASK_STATUS_COMPLETED or normalized_status != TASK_STATUS_LATER_PROMOTION_EXCLUDED:
            conn.close()
            raise ValueError(f"Local task {task_id} is already {task_status}; reopening is not supported")
        if str(task.get("publication_state") or "") == "published":
            conn.close()
            raise ValueError(
                f"Local task {task_id} has already been published; close the remote task instead."
            )
        terminal_change_statuses = {"archived", "landed", "superseded"}
        change_rows = [
            dict(change_row)
            for change_row in conn.execute(
                "select change_id, status from workflow_changes where task_id = ?",
                (resolved_task_id,),
            ).fetchall()
        ]
        if not change_rows:
            conn.close()
            raise ValueError(
                f"Local task {task_id} is already completed; only completed tasks with landed local changes "
                "can be reclassified as later-promotion-excluded."
            )
        non_terminal_change = next(
            (
                change_row
                for change_row in change_rows
                if str(change_row.get("status") or "").strip() not in terminal_change_statuses
            ),
            None,
        )
        if non_terminal_change is not None:
            conn.close()
            raise ValueError(
                f"Local task {task_id} cannot be reclassified as later-promotion-excluded while local change "
                f"{non_terminal_change['change_id']} is `{non_terminal_change['status']}`."
            )
        if not any(str(change_row.get("status") or "").strip() == "landed" for change_row in change_rows):
            conn.close()
            raise ValueError(
                f"Local task {task_id} is already completed but has no landed local changes to exclude "
                "from later promotion."
            )
    now = utc_now()
    conn.execute("update workflow_tasks set status = ?, updated_at = ? where task_id = ?", (normalized_status, now, resolved_task_id))
    row = conn.execute("select * from workflow_tasks where task_id = ?", (resolved_task_id,)).fetchone()
    conn.commit()
    conn.close()
    assert row is not None
    return dict(row)


def _restart_workflow_change_conn(conn, change_row: dict) -> dict:
    change_status = str(change_row.get("status") or "").strip()
    if change_status == "draft":
        return change_row
    if change_status != "archived":
        raise ValueError(
            f"Local change {change_row['change_id']} is `{change_status or 'unknown'}`; restart only supports archived changes."
        )
    now = utc_now()
    conn.execute(
        "update workflow_changes set status = 'draft', updated_at = ? where change_id = ?",
        (now, str(change_row["change_id"])),
    )
    refreshed = conn.execute(
        "select * from workflow_changes where change_id = ?",
        (str(change_row["change_id"]),),
    ).fetchone()
    assert refreshed is not None
    return dict(refreshed)


def restart_workflow_task(ctx: RepoContext, task_id: str) -> dict:
    conn = _connect_control(ctx)
    resolved_task_id = _resolve_workflow_task_id(conn, task_id)
    row = conn.execute("select * from workflow_tasks where task_id = ?", (resolved_task_id,)).fetchone()
    if row is None:
        conn.close()
        raise KeyError(f"Unknown local task: {task_id}")
    task = dict(row)
    task_status = normalize_task_status(task.get("status"))
    assert task_status is not None
    if str(task.get("publication_state") or "") == "published":
        conn.close()
        raise ValueError(f"Local task {task_id} is published; restart the remote task lineage instead.")
    if not is_task_abandoned_status(task_status):
        conn.close()
        raise ValueError(
            f"Local task {task_id} is `{task_status}`; restart only supports task canceled lineage."
        )
    change_rows = [
        dict(change_row)
        for change_row in conn.execute(
            "select * from workflow_changes where task_id = ? order by created_at asc",
            (resolved_task_id,),
        ).fetchall()
    ]
    landed_change = next((change_row for change_row in change_rows if str(change_row.get("status") or "").strip() == "landed"), None)
    if landed_change is not None:
        conn.close()
        raise ValueError(
            f"Local task {task_id} cannot be restarted because landed change {landed_change['change_id']} already exists."
        )
    archived_changes = [change_row for change_row in change_rows if str(change_row.get("status") or "").strip() == "archived"]
    open_changes = [
        change_row
        for change_row in change_rows
        if str(change_row.get("status") or "").strip() not in {"archived", "superseded"}
    ]
    if not open_changes and len(archived_changes) > 1:
        conn.close()
        archived_ids = ", ".join(str(change_row["change_id"]) for change_row in archived_changes)
        raise ValueError(
            f"Local task {task_id} has multiple archived changes ({archived_ids}); restart only supports one archived change."
        )
    now = utc_now()
    conn.execute(
        "update workflow_tasks set status = 'active', updated_at = ? where task_id = ?",
        (now, resolved_task_id),
    )
    restarted_change: dict | None = None
    if not open_changes and len(archived_changes) == 1:
        restarted_change = _restart_workflow_change_conn(conn, archived_changes[0])
    refreshed = conn.execute("select * from workflow_tasks where task_id = ?", (resolved_task_id,)).fetchone()
    conn.commit()
    conn.close()
    assert refreshed is not None
    payload = dict(refreshed)
    if restarted_change is not None:
        payload["change"] = restarted_change
    return payload


def mark_workflow_task_published(
    ctx: RepoContext,
    task_id: str,
    *,
    remote_name: str | None = None,
    published_task_id: str | None = None,
) -> dict:
    conn = _connect_control(ctx)
    resolved_task_id = _resolve_workflow_task_id(conn, task_id)
    resolved_published_task_id = str(published_task_id or resolved_task_id).strip()
    _register_workflow_task_alias(
        conn,
        resolved_published_task_id,
        resolved_task_id,
        alias_kind="published_remote_id",
    )
    now = utc_now()
    conn.execute(
        """
        update workflow_tasks
        set publication_state = 'published',
            published_remote_name = coalesce(?, published_remote_name),
            published_task_id = ?,
            published_at = coalesce(published_at, ?),
            updated_at = ?
        where task_id = ?
        """,
        (remote_name, resolved_published_task_id, now, now, resolved_task_id),
    )
    row = conn.execute("select * from workflow_tasks where task_id = ?", (resolved_task_id,)).fetchone()
    if row is not None:
        published_sequence = _workflow_sequence_from_id(resolved_published_task_id, family="T")
        repo_name = str(row["repo_name"] or "").strip()
        if published_sequence is not None and repo_name:
            _set_workflow_sequence_floor(conn, repo_name, "T", published_sequence)
    conn.commit()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local task: {task_id}")
    return dict(row)


def create_workflow_change(
    ctx: RepoContext,
    change_id: str,
    task_id: str,
    repo_name: str,
    title: str,
    base_line: str,
    risk_tier: str,
    lane: str,
    *,
    change_seq: int | None = None,
    identity_source: str = LOCAL_IDENTITY_SOURCE_SEQUENCE,
    fork_snapshot_id: str | None = None,
    forked_from_line: str | None = None,
    status: str = "draft",
    publication_state: str = "local_draft",
) -> dict:
    conn = _connect_control(ctx)
    resolved_task_id = _resolve_workflow_task_id(conn, task_id)
    task = conn.execute("select task_id, status from workflow_tasks where task_id = ?", (resolved_task_id,)).fetchone()
    if task is None:
        conn.close()
        raise KeyError(f"Unknown local task: {task_id}")
    if task["status"] != "active":
        conn.close()
        raise ValueError(f"Local task {task_id} is {task['status']} and cannot accept new changes")
    now = utc_now()
    resolved_change_seq, resolved_identity_source = _normalize_change_identity_metadata(change_id, change_seq, identity_source)
    conn.execute(
        """
        insert into workflow_changes(
            change_id, change_seq, identity_source, task_id, repo_name, title, base_line, fork_snapshot_id, forked_from_line,
            risk_tier, lane, status, publication_state, published_remote_name, published_change_id, published_at, created_at, updated_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            change_id,
            resolved_change_seq,
            resolved_identity_source,
            resolved_task_id,
            repo_name,
            title,
            base_line,
            fork_snapshot_id,
            forked_from_line,
            risk_tier,
            lane,
            status,
            publication_state,
            None,
            None,
            None,
            now,
            now,
        ),
    )
    conn.commit()
    row = conn.execute("select * from workflow_changes where change_id = ?", (change_id,)).fetchone()
    conn.close()
    return dict(row)


def list_workflow_changes(ctx: RepoContext) -> list[dict]:
    conn = _connect_control(ctx)
    rows = [dict(r) for r in conn.execute("select * from workflow_changes order by created_at desc, coalesce(change_seq, 0) desc, change_id desc")]
    conn.close()
    return rows


def get_workflow_change(ctx: RepoContext, change_id: str) -> dict:
    conn = _connect_control(ctx)
    resolved_change_id = _resolve_workflow_change_id(conn, change_id)
    row = conn.execute("select * from workflow_changes where change_id = ?", (resolved_change_id,)).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local change: {change_id}")
    return dict(row)


def close_workflow_change(ctx: RepoContext, change_id: str, status: str) -> dict:
    if status != "archived":
        raise ValueError(f"Unsupported local change close status: {status}")
    conn = _connect_control(ctx)
    resolved_change_id = _resolve_workflow_change_id(conn, change_id)
    row = conn.execute("select * from workflow_changes where change_id = ?", (resolved_change_id,)).fetchone()
    if row is None:
        conn.close()
        raise KeyError(f"Unknown local change: {change_id}")
    change = dict(row)
    if change["status"] == status:
        conn.close()
        return change
    if change["status"] == "landed":
        conn.close()
        raise ValueError(f"Local change {change_id} is landed and cannot be archived")
    now = utc_now()
    conn.execute("update workflow_changes set status = ?, updated_at = ? where change_id = ?", (status, now, resolved_change_id))
    row = conn.execute("select * from workflow_changes where change_id = ?", (resolved_change_id,)).fetchone()
    conn.commit()
    conn.close()
    assert row is not None
    return dict(row)


def land_workflow_change(
    ctx: RepoContext,
    change_id: str,
    *,
    target_line: str,
    landed_snapshot_id: str,
    pre_land_target_snapshot_id: str | None = None,
) -> dict:
    conn = _connect_control(ctx)
    resolved_change_id = _resolve_workflow_change_id(conn, change_id)
    row = conn.execute("select * from workflow_changes where change_id = ?", (resolved_change_id,)).fetchone()
    if row is None:
        conn.close()
        raise KeyError(f"Unknown local change: {change_id}")
    change = dict(row)
    if change["status"] == "landed":
        conn.close()
        raise ValueError(f"Local change {change_id} is already landed")
    if change["status"] not in {"draft", "active"}:
        conn.close()
        raise ValueError(f"Local change {change_id} is {change['status']} and only open changes can be landed")
    now = utc_now()
    conn.execute(
        """
        update workflow_changes
        set status = 'landed',
            target_line = ?,
            pre_land_target_snapshot_id = ?,
            landed_snapshot_id = ?,
            landed_at = ?,
            updated_at = ?
        where change_id = ?
        """,
        (target_line, pre_land_target_snapshot_id, landed_snapshot_id, now, now, resolved_change_id),
    )
    row = conn.execute("select * from workflow_changes where change_id = ?", (resolved_change_id,)).fetchone()
    conn.commit()
    conn.close()
    assert row is not None
    return dict(row)


def mark_workflow_change_published(
    ctx: RepoContext,
    change_id: str,
    *,
    remote_name: str | None = None,
    published_change_id: str | None = None,
    allow_landed: bool = False,
) -> dict:
    conn = _connect_control(ctx)
    resolved_change_id = _resolve_workflow_change_id(conn, change_id)
    existing = conn.execute("select status from workflow_changes where change_id = ?", (resolved_change_id,)).fetchone()
    if existing is None:
        conn.close()
        raise KeyError(f"Unknown local change: {change_id}")
    allowed_statuses = {"draft", "landed"} if allow_landed else {"draft"}
    if existing["status"] not in allowed_statuses:
        conn.close()
        raise ValueError(f"Local change {change_id} is {existing['status']} and cannot be published")
    resolved_published_change_id = str(published_change_id or resolved_change_id).strip()
    _register_workflow_change_alias(
        conn,
        resolved_published_change_id,
        resolved_change_id,
        alias_kind="published_remote_id",
    )
    now = utc_now()
    conn.execute(
        """
        update workflow_changes
        set publication_state = 'published',
            published_remote_name = coalesce(?, published_remote_name),
            published_change_id = ?,
            published_at = coalesce(published_at, ?),
            updated_at = ?
        where change_id = ?
        """,
        (remote_name, resolved_published_change_id, now, now, resolved_change_id),
    )
    row = conn.execute("select * from workflow_changes where change_id = ?", (resolved_change_id,)).fetchone()
    if row is not None:
        published_sequence = _workflow_sequence_from_id(resolved_published_change_id, family="C")
        repo_name = str(row["repo_name"] or "").strip()
        if published_sequence is not None and repo_name:
            _set_workflow_sequence_floor(conn, repo_name, "C", published_sequence)
    conn.commit()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local change: {change_id}")
    return dict(row)


_UNSET = WORKFLOW_RELEASE_UNSET
