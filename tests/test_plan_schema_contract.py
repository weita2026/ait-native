from __future__ import annotations

import hashlib
from pathlib import Path

from ait_server import server_control, server_store
from ait_server.server_content import read_blob_bytes
from ait_server.server_paths import ServerContext
from tests.postgres_fake import fake_postgres_context, install_fake_psycopg


LANE_A_TASK_COLUMNS = {
    "plan_id",
    "origin_plan_revision_id",
    "plan_item_ref",
    "plan_section_ref",
    "plan_drift_state",
    "plan_linked_at",
    "planning_state",
}

RETIRED_IMPORTED_COMPLETION_TASK_SOURCE_COLUMNS = {
    "source_completion_mode",
    "source_local_task_id",
    "source_local_completed_at",
}

RETIRED_IMPORTED_COMPLETION_CHANGE_SOURCE_COLUMNS = {
    "source_completion_mode",
    "source_local_change_id",
    "source_local_status",
    "source_target_line",
    "source_landed_snapshot_id",
    "source_landed_at",
}

LANE_A_PLAN_COLUMNS = {
    "repo_id",
}

LANE_A_PLANNING_SESSION_COLUMNS = {
    "plan_id",
    "derived_task_id",
    "last_promoted_plan_revision_id",
}

LANE_A_PLAN_REVISION_BLOB_COLUMNS = {
    "plan_revision_id",
    "repo_name",
    "repo_id",
    "blob_id",
    "media_type",
    "encoding",
    "byte_count",
    "created_at",
}

PLAN_REVISION_ARTIFACT_COLUMNS = {
    "plan_revision_id",
    "artifact_path",
    "repo_name",
    "repo_id",
    "role",
    "blob_id",
    "media_type",
    "encoding",
    "byte_count",
    "sha256",
    "metadata_json",
    "created_at",
    "updated_at",
}

LANE_A_PLAN_REVISION_DIFF_COLUMNS = {
    "plan_links_surface_hash",
    "plan_links_changed_count_to_prev",
}

LANE_A_INDEXES = {
    "idx_tasks_repo_created",
    "idx_plans_repo_updated",
    "idx_plans_repo_id_updated",
    "idx_plan_revisions_plan_created",
    "idx_plan_revision_blobs_repo_blob",
    "idx_plan_revision_blobs_repo_id_blob",
    "idx_plan_revision_artifacts_repo",
    "idx_plan_revision_artifacts_repo_id",
    "idx_planning_sessions_repo_plan",
}


def _columns(conn, table_name: str) -> set[str]:
    return server_control._table_columns(conn, table_name)


def _indexes(conn) -> set[str]:
    rows = conn.execute("select name from sqlite_master where type = 'index'").fetchall()
    return {row["name"] for row in rows if row.get("name")}


def _assert_lane_a_schema(conn) -> None:
    task_columns = _columns(conn, "tasks")
    change_columns = _columns(conn, "changes")
    assert LANE_A_TASK_COLUMNS <= task_columns
    assert RETIRED_IMPORTED_COMPLETION_TASK_SOURCE_COLUMNS.isdisjoint(task_columns)
    assert RETIRED_IMPORTED_COMPLETION_CHANGE_SOURCE_COLUMNS.isdisjoint(change_columns)
    assert LANE_A_PLAN_COLUMNS <= _columns(conn, "plans")
    assert LANE_A_PLANNING_SESSION_COLUMNS <= _columns(conn, "planning_sessions")
    assert LANE_A_PLAN_REVISION_DIFF_COLUMNS <= _columns(conn, "plan_revisions")
    assert LANE_A_PLAN_REVISION_BLOB_COLUMNS <= _columns(conn, "plan_revision_blobs")
    assert PLAN_REVISION_ARTIFACT_COLUMNS <= _columns(conn, "plan_revision_artifacts")
    assert LANE_A_INDEXES <= _indexes(conn)


def _table_sql(conn, table_name: str) -> str:
    row = conn.execute(
        "select sql from sqlite_master where type = 'table' and name = ?",
        (table_name,),
    ).fetchone()
    return str((row or {}).get("sql") or "")


def test_lane_a_schema_contract_sqlite_is_idempotent(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")

    for _ in range(3):
        server_control.initialize(ctx)

    conn = server_control.connect(ctx)
    try:
        _assert_lane_a_schema(conn)
    finally:
        conn.close()


def test_lane_a_schema_contract_migrates_legacy_sqlite_tables(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    ctx.root.mkdir(parents=True, exist_ok=True)
    conn = server_control.connect(ctx)
    conn.executescript(
        """
        create table tasks (
            task_id text primary key,
            repo_name text not null,
            title text not null,
            intent text not null,
            risk_tier text not null,
            status text not null,
            created_at text not null
        );
        insert into tasks(task_id, repo_name, title, intent, risk_tier, status, created_at)
        values ('T-LEGACY', 'ait', 'legacy task', 'keep row', 'low', 'active', '2026-04-20T00:00:00Z');

        create table planning_sessions (
            planning_session_id text primary key,
            repo_name text not null,
            title text,
            mode text not null,
            status text not null,
            preferred_agent text,
            artifact_status text not null,
            last_event_sequence integer not null default 0,
            created_by text not null,
            created_at text not null,
            updated_at text not null
        );
        insert into planning_sessions(
            planning_session_id, repo_name, title, mode, status, artifact_status,
            last_event_sequence, created_by, created_at, updated_at
        ) values (
            'PS-LEGACY', 'ait', 'legacy planning', 'connected_local', 'active', 'not_promoted',
            0, 'system', '2026-04-20T00:00:00Z', '2026-04-20T00:00:00Z'
        );

        create table plan_revision_blobs (
            plan_revision_id text primary key,
            repo_name text not null,
            blob_id text not null
        );
        insert into plan_revision_blobs(plan_revision_id, repo_name, blob_id)
        values ('PR-LEGACY', 'ait', 'BLB-LEGACY');
        """
    )
    conn.commit()
    conn.close()

    for _ in range(3):
        server_control.initialize(ctx)

    conn = server_control.connect(ctx)
    try:
        _assert_lane_a_schema(conn)
        assert "blob_id text not null unique" not in _table_sql(conn, "plan_revision_blobs").lower()
        assert conn.execute("select task_id from tasks where task_id = ?", ("T-LEGACY",)).fetchone() is not None
        assert (
            conn.execute(
                "select planning_session_id from planning_sessions where planning_session_id = ?",
                ("PS-LEGACY",),
            ).fetchone()
            is not None
        )
        assert (
            conn.execute(
                "select plan_revision_id from plan_revision_blobs where plan_revision_id = ?",
                ("PR-LEGACY",),
            ).fetchone()
            is not None
        )
        conn.execute(
            """
            insert into plan_revision_blobs(
                plan_revision_id, repo_name, blob_id, media_type, encoding, byte_count, created_at
            ) values (?, ?, ?, 'text/markdown', 'utf-8', ?, ?)
            """,
            ("PR-LEGACY-2", "ait", "BLB-LEGACY", 4, "2026-04-20T00:00:01Z"),
        )
        conn.commit()
        assert (
            len(
                conn.execute(
                    "select plan_revision_id from plan_revision_blobs where blob_id = ?",
                    ("BLB-LEGACY",),
                ).fetchall()
            )
            == 2
        )
    finally:
        conn.close()


def test_lane_a_schema_contract_fake_postgres_is_idempotent(monkeypatch, tmp_path: Path):
    install_fake_psycopg(monkeypatch)
    ctx = ServerContext.create(
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-pg'}",
        control_schema="ait_native_control_test",
    )

    for _ in range(3):
        server_control.initialize(ctx)

    conn = server_control.connect(ctx)
    try:
        _assert_lane_a_schema(conn)
    finally:
        conn.close()


def test_lane_a_postgres_sql_file_matches_runtime_contract():
    sql = Path("sql/ait_native_postgres_control_schema.sql").read_text(encoding="utf-8")

    assert "create table if not exists plan_revision_blobs" in server_control.SCHEMA_POSTGRES
    assert "create table if not exists plan_revision_blobs" in sql
    assert "create table if not exists plan_revision_artifacts" in server_control.SCHEMA_POSTGRES
    assert "create table if not exists plan_revision_artifacts" in sql
    for column in sorted(LANE_A_TASK_COLUMNS):
        assert column in server_control.SCHEMA_POSTGRES
        assert column in sql
    for column in sorted(RETIRED_IMPORTED_COMPLETION_TASK_SOURCE_COLUMNS | RETIRED_IMPORTED_COMPLETION_CHANGE_SOURCE_COLUMNS):
        assert column not in server_control.SCHEMA_POSTGRES
        assert column not in sql
    for column in sorted(LANE_A_PLAN_COLUMNS):
        assert column in server_control.SCHEMA_POSTGRES
        assert column in sql
    for column in sorted(LANE_A_PLAN_REVISION_BLOB_COLUMNS):
        assert column in server_control.SCHEMA_POSTGRES
        assert column in sql
    for column in sorted(PLAN_REVISION_ARTIFACT_COLUMNS):
        assert column in server_control.SCHEMA_POSTGRES
        assert column in sql
    for index_name in sorted(LANE_A_INDEXES):
        assert index_name in server_control.SCHEMA_POSTGRES
        assert index_name in sql


def test_lane_a_plan_revision_artifact_blobs_round_trip_through_plan_runtime(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main")

    items = [
        {
            "plan_item_ref": "lane-a/blob-contract",
            "text": "persist revision artifact blobs",
            "state": "open",
            "level": 1,
        }
    ]
    artifact_body = (
        "# Lane A Contract\n\n"
        "## Blob Contract [plan-ref: lane-a/contract]\n\n"
        "- [ ] persist revision artifact blobs [ref: lane-a/blob-contract]\n"
    )
    expected_blob_id = f"BLB-{hashlib.sha256(artifact_body.encode('utf-8')).hexdigest()[:20]}"

    plan = server_store.create_plan(
        ctx,
        "repo-a",
        "Lane A blob contract",
        "docs/sprints/lane_a.md",
        "lane-a/contract",
        "Blob Contract",
        items,
        summary="seed contract",
        artifact_body=artifact_body,
    )
    head_revision = plan["head_revision"]

    assert head_revision["artifact_blob_id"] == expected_blob_id
    assert head_revision["artifact_media_type"] == "text/markdown"
    assert head_revision["artifact_encoding"] == "utf-8"
    assert head_revision["artifact_byte_count"] == len(artifact_body.encode("utf-8"))
    assert head_revision["plan_links_changed_count_to_prev"] == 0
    assert head_revision["plan_links_surface_hash"]
    assert read_blob_bytes(ctx, expected_blob_id).decode("utf-8") == artifact_body

    listed_revision = server_store.list_plan_revisions(ctx, plan["plan_id"])[0]
    fetched_revision = server_store.get_plan_revision(ctx, plan["plan_id"], head_revision["plan_revision_id"])
    assert listed_revision["artifact_blob_id"] == expected_blob_id
    assert fetched_revision["artifact_blob_id"] == expected_blob_id

    revised_body = artifact_body + "\n- [ ] expose read surfaces [ref: lane-a/read-surface]\n"
    revised_blob_id = f"BLB-{hashlib.sha256(revised_body.encode('utf-8')).hexdigest()[:20]}"
    revised = server_store.revise_plan(
        ctx,
        plan["plan_id"],
        "docs/sprints/lane_a.md",
        "lane-a/contract",
        "Blob Contract",
        [
            *items,
            {
                "plan_item_ref": "lane-a/read-surface",
                "text": "expose read surfaces",
                "state": "open",
                "level": 1,
            },
        ],
        summary="revise contract",
        artifact_body=revised_body,
    )
    assert revised["head_revision"]["artifact_blob_id"] == revised_blob_id
    assert revised["head_revision"]["plan_links_changed_count_to_prev"] == 1
    assert revised["head_revision"]["plan_links_surface_hash"] != head_revision["plan_links_surface_hash"]
    assert read_blob_bytes(ctx, revised_blob_id).decode("utf-8") == revised_body

    repeated = server_store.revise_plan(
        ctx,
        plan["plan_id"],
        "docs/sprints/lane_a.md",
        "lane-a/contract",
        "Blob Contract",
        revised["head_revision"]["items"],
        summary="record same body again",
        artifact_body=revised_body,
    )
    assert repeated["head_revision"]["artifact_blob_id"] == revised_blob_id
    assert repeated["head_revision"]["plan_links_changed_count_to_prev"] == 0
    assert repeated["head_revision"]["plan_links_surface_hash"] == revised["head_revision"]["plan_links_surface_hash"]
    repeated_blob_rows = server_control.connect(ctx)
    try:
        assert (
            len(
                repeated_blob_rows.execute(
                    "select plan_revision_id from plan_revision_blobs where blob_id = ?",
                    (revised_blob_id,),
                ).fetchall()
            )
            == 2
        )
    finally:
        repeated_blob_rows.close()

    planning_session = server_store.create_planning_session(ctx, plan["plan_id"], title="Lane A promotion")
    promoted_body = revised_body.replace("[ ] expose", "[x] expose")
    promoted_blob_id = f"BLB-{hashlib.sha256(promoted_body.encode('utf-8')).hexdigest()[:20]}"
    promoted = server_store.promote_planning_session(
        ctx,
        planning_session["planning_session_id"],
        "docs/sprints/lane_a.md",
        "lane-a/contract",
        "Blob Contract",
        revised["head_revision"]["items"],
        summary="promoted contract",
        artifact_body=promoted_body,
    )

    assert promoted["promoted_revision"]["artifact_blob_id"] == promoted_blob_id
    assert promoted["planning_session"]["artifact_status"] == "promoted"
    assert promoted["planning_session"]["last_promoted_plan_revision_id"] == promoted["promoted_revision"]["plan_revision_id"]
    assert read_blob_bytes(ctx, promoted_blob_id).decode("utf-8") == promoted_body
