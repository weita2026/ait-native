from __future__ import annotations

import sqlite3
from pathlib import Path

import pytest

from ait import local_control
from ait.repo_paths import RepoContext
from ait_protocol.common import connect_sqlite


def _repo_context(tmp_path: Path) -> RepoContext:
    ait_dir = tmp_path / ".ait"
    ait_dir.mkdir()
    return RepoContext(
        root=tmp_path,
        ait_dir=ait_dir,
        content_db_path=ait_dir / "content.db",
        control_db_path=ait_dir / "control.db",
        config_path=ait_dir / "config.json",
    )


def _index_names(conn) -> set[str]:
    rows = conn.execute("select name from sqlite_master where type = 'index'").fetchall()
    return {str(row["name"]) for row in rows if row["name"] is not None}


def _table_columns(conn, table_name: str) -> set[str]:
    rows = conn.execute(f"pragma table_info({table_name})").fetchall()
    return {str(row["name"]) for row in rows if row["name"] is not None}


def test_local_control_initialize_creates_alias_ready_schema(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    conn = connect_sqlite(ctx.control_db_path)
    try:
        assert {
            "task_seq",
            "identity_source",
            "plan_id",
            "origin_plan_revision_id",
            "plan_item_ref",
            "plan_linked_at",
            "published_remote_name",
            "published_task_id",
        } <= _table_columns(conn, "workflow_tasks")
        assert {
            "change_seq",
            "identity_source",
            "fork_snapshot_id",
            "forked_from_line",
            "published_remote_name",
            "published_change_id",
            "target_line",
            "landed_snapshot_id",
            "landed_at",
        } <= _table_columns(conn, "workflow_changes")
        assert "plan_links_surface_hash" not in _table_columns(conn, "workflow_plan_revisions")
        assert "plan_links_changed_count_to_prev" not in _table_columns(conn, "workflow_plan_revisions")

        index_names = _index_names(conn)
        assert "idx_workflow_task_aliases_task_id" in index_names
        assert "idx_workflow_change_aliases_change_id" in index_names
        assert "uq_workflow_tasks_repo_name_task_seq" in index_names
        assert "uq_workflow_changes_repo_name_change_seq" in index_names

        table_names = {str(row["name"]) for row in conn.execute("select name from sqlite_master where type = 'table'").fetchall()}
        assert "workflow_task_aliases" in table_names
        assert "workflow_change_aliases" in table_names
    finally:
        conn.close()


def test_local_control_initialize_drops_historical_publication_identity_columns(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    conn = connect_sqlite(ctx.control_db_path)
    try:
        conn.execute("alter table workflow_tasks add column historical_publication_remote_name text")
        conn.execute("alter table workflow_tasks add column historical_publication_id text")
        conn.execute("alter table workflow_tasks add column historical_published_task_id text")
        conn.execute("alter table workflow_tasks add column historical_published_at text")
        conn.execute("alter table workflow_changes add column historical_publication_remote_name text")
        conn.execute("alter table workflow_changes add column historical_publication_id text")
        conn.execute("alter table workflow_changes add column historical_published_change_id text")
        conn.execute("alter table workflow_changes add column historical_published_at text")
        conn.commit()
    finally:
        conn.close()

    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    conn = connect_sqlite(ctx.control_db_path)
    try:
        assert "historical_publication_remote_name" not in _table_columns(conn, "workflow_tasks")
        assert "historical_publication_id" not in _table_columns(conn, "workflow_tasks")
        assert "historical_published_task_id" not in _table_columns(conn, "workflow_tasks")
        assert "historical_published_at" not in _table_columns(conn, "workflow_tasks")
        assert "historical_publication_remote_name" not in _table_columns(conn, "workflow_changes")
        assert "historical_publication_id" not in _table_columns(conn, "workflow_changes")
        assert "historical_published_change_id" not in _table_columns(conn, "workflow_changes")
        assert "historical_published_at" not in _table_columns(conn, "workflow_changes")
    finally:
        conn.close()


def test_local_control_initialize_drops_local_plan_link_diff_metadata_columns(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    conn = connect_sqlite(ctx.control_db_path)
    try:
        conn.execute("alter table workflow_plan_revisions add column plan_links_surface_hash text")
        conn.execute(
            "alter table workflow_plan_revisions add column plan_links_changed_count_to_prev integer not null default 0"
        )
        conn.commit()
    finally:
        conn.close()

    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    conn = connect_sqlite(ctx.control_db_path)
    try:
        revision_columns = _table_columns(conn, "workflow_plan_revisions")
    finally:
        conn.close()

    assert "plan_links_surface_hash" not in revision_columns
    assert "plan_links_changed_count_to_prev" not in revision_columns


def test_local_control_operations_skip_schema_bootstrap_after_init(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    def fail_ensure_schema(*_args, **_kwargs):
        raise AssertionError("control schema bootstrap should only run during `ait init`")

    monkeypatch.setattr(local_control, "_ensure_schema", fail_ensure_schema)

    task = local_control.create_workflow_task(
        ctx,
        task_id="ABCT-101",
        repo_name="repo-a",
        title="Runtime task",
        intent="prove normal control operations skip request-time schema shaping",
        risk_tier="medium",
    )
    fetched_task = local_control.get_workflow_task(ctx, task["task_id"])
    listed_tasks = local_control.list_workflow_tasks(ctx)

    change = local_control.create_workflow_change(
        ctx,
        change_id="ABCC-101",
        task_id=task["task_id"],
        repo_name="repo-a",
        title="Runtime change",
        base_line="main",
        risk_tier="medium",
        lane="assisted",
    )
    fetched_change = local_control.get_workflow_change(ctx, change["change_id"])
    listed_changes = local_control.list_workflow_changes(ctx)

    assert fetched_task["task_id"] == task["task_id"]
    assert any(row["task_id"] == task["task_id"] for row in listed_tasks)
    assert fetched_change["change_id"] == change["change_id"]
    assert any(row["change_id"] == change["change_id"] for row in listed_changes)


def test_local_control_runtime_operations_do_not_drop_compatibility_columns_after_init(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    conn = connect_sqlite(ctx.control_db_path)
    try:
        conn.execute("alter table workflow_tasks add column historical_publication_remote_name text")
        conn.execute("alter table workflow_tasks add column historical_publication_id text")
        conn.execute("alter table workflow_changes add column historical_publication_remote_name text")
        conn.execute("alter table workflow_changes add column historical_publication_id text")
        conn.commit()
    finally:
        conn.close()

    task = local_control.create_workflow_task(
        ctx,
        task_id="ABCT-102",
        repo_name="repo-a",
        title="Compatibility task",
        intent="prove runtime paths do not perform legacy column cleanup",
        risk_tier="medium",
    )
    change = local_control.create_workflow_change(
        ctx,
        change_id="ABCC-102",
        task_id=task["task_id"],
        repo_name="repo-a",
        title="Compatibility change",
        base_line="main",
        risk_tier="medium",
        lane="assisted",
    )

    conn = connect_sqlite(ctx.control_db_path)
    try:
        task_columns = _table_columns(conn, "workflow_tasks")
        change_columns = _table_columns(conn, "workflow_changes")
    finally:
        conn.close()

    assert task["task_id"] == "ABCT-102"
    assert change["change_id"] == "ABCC-102"
    assert "historical_publication_remote_name" in task_columns
    assert "historical_publication_id" in task_columns
    assert "historical_publication_remote_name" in change_columns
    assert "historical_publication_id" in change_columns


def test_local_plan_operations_do_not_drop_compatibility_columns_after_init(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    conn = connect_sqlite(ctx.control_db_path)
    try:
        conn.execute("alter table workflow_plan_revisions add column plan_links_surface_hash text")
        conn.execute(
            "alter table workflow_plan_revisions add column plan_links_changed_count_to_prev integer not null default 0"
        )
        conn.commit()
    finally:
        conn.close()

    plan = local_control.create_workflow_plan(
        ctx,
        plan_id="PL-local",
        plan_revision_id="PR-local-1",
        repo_name="repo-a",
        title="Compatibility plan",
        items=[
            {
                "plan_item_ref": "compat/runtime-plan-op-cleanup-boundary",
                "title": "Keep runtime cleanup out of plan writes",
                "status": "pending",
            }
        ],
        artifact_path="docs/sprints/compat.md",
        artifact_selector=None,
        artifact_heading="Compatibility plan",
        source_kind="manual_edit",
    )
    revised = local_control.revise_workflow_plan(
        ctx,
        plan_id=plan["plan_id"],
        plan_revision_id="PR-local-2",
        artifact_path="docs/sprints/compat.md",
        artifact_selector=None,
        artifact_heading="Compatibility plan",
        items=[
            {
                "plan_item_ref": "compat/runtime-plan-op-cleanup-boundary",
                "title": "Still present after revise",
                "status": "pending",
            }
        ],
        source_kind="manual_edit",
    )

    conn = connect_sqlite(ctx.control_db_path)
    try:
        revision_columns = _table_columns(conn, "workflow_plan_revisions")
    finally:
        conn.close()

    assert revised["head_revision_id"] == "PR-local-2"
    assert "plan_links_surface_hash" in revision_columns
    assert "plan_links_changed_count_to_prev" in revision_columns


def test_local_control_create_workflow_task_persists_plan_linkage(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    task = local_control.create_workflow_task(
        ctx,
        task_id="ABCT-042",
        repo_name="repo-a",
        title="Plan-linked local task",
        intent="persist local plan lineage on draft tasks",
        risk_tier="medium",
        planning_state="planned",
        plan_id="PL-local",
        origin_plan_revision_id="PR-local",
        plan_item_ref="lane-a/persist-linkage",
        plan_linked_at="2026-01-01T00:00:00Z",
    )

    assert task["planning_state"] == "planned"
    assert task["plan_id"] == "PL-local"
    assert task["origin_plan_revision_id"] == "PR-local"
    assert task["plan_item_ref"] == "lane-a/persist-linkage"
    assert task["plan_linked_at"] == "2026-01-01T00:00:00Z"


def test_local_control_allocates_short_task_and_change_sequence_identity(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    first_identity = local_control.allocate_workflow_task_identity(ctx, "repo-a", "ABC")
    assert first_identity == {"task_id": "ABCT-0001", "task_seq": 1}

    task = local_control.create_workflow_task(
        ctx,
        task_id=first_identity["task_id"],
        task_seq=first_identity["task_seq"],
        repo_name="repo-a",
        title="Short task",
        intent="use repo-scoped sequence identity",
        risk_tier="medium",
    )
    assert task["task_id"] == "ABCT-0001"
    assert task["task_seq"] == 1
    assert task["identity_source"] == local_control.LOCAL_IDENTITY_SOURCE_SEQUENCE

    second_identity = local_control.allocate_workflow_task_identity(ctx, "repo-a", "ABC")
    assert second_identity == {"task_id": "ABCT-0002", "task_seq": 2}

    change_identity = local_control.allocate_workflow_change_identity(ctx, "repo-a", "ABC")
    assert change_identity == {"change_id": "ABCC-0001", "change_seq": 1}
    change = local_control.create_workflow_change(
        ctx,
        change_id=change_identity["change_id"],
        change_seq=change_identity["change_seq"],
        task_id=task["task_id"],
        repo_name="repo-a",
        title="Short change",
        base_line="main",
        risk_tier="medium",
        lane="assisted",
    )
    assert change["change_id"] == "ABCC-0001"
    assert change["change_seq"] == 1
    assert change["identity_source"] == local_control.LOCAL_IDENTITY_SOURCE_SEQUENCE


def test_local_control_publish_updates_sequence_floors_for_future_local_allocations(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    task = local_control.create_workflow_task(
        ctx,
        task_id="ABCT-0001",
        repo_name="repo-a",
        title="Published task floor",
        intent="carry remote task sequence floors back into local allocation",
        risk_tier="medium",
    )
    change = local_control.create_workflow_change(
        ctx,
        change_id="ABCC-0001",
        task_id=task["task_id"],
        repo_name="repo-a",
        title="Published change floor",
        base_line="main",
        risk_tier="medium",
        lane="assisted",
    )

    local_control.mark_workflow_task_published(ctx, task["task_id"], remote_name="origin", published_task_id="ABCT-0042")
    local_control.mark_workflow_change_published(ctx, change["change_id"], remote_name="origin", published_change_id="ABCC-0051")

    assert local_control.get_workflow_sequence_floor(ctx, "repo-a", "T") == 42
    assert local_control.get_workflow_sequence_floor(ctx, "repo-a", "C") == 51
    assert local_control.allocate_workflow_task_identity(ctx, "repo-a", "ABC") == {"task_id": "ABCT-0043", "task_seq": 43}
    assert local_control.allocate_workflow_change_identity(ctx, "repo-a", "ABC") == {"change_id": "ABCC-0052", "change_seq": 52}


def test_local_control_initialize_backfills_sequence_floors_from_existing_published_remote_ids(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)

    conn = connect_sqlite(ctx.control_db_path)
    conn.executescript(
        """
        create table if not exists control_meta (
            key text primary key,
            value text not null
        );
        create table workflow_tasks (
            task_id text primary key,
            task_seq integer,
            identity_source text not null default 'local_sequence',
            repo_name text not null,
            title text not null,
            intent text not null,
            risk_tier text not null,
            planning_state text not null,
            status text not null,
            publication_state text not null,
            published_task_id text,
            published_at text,
            created_at text not null,
            updated_at text not null
        );
        create table workflow_changes (
            change_id text primary key,
            change_seq integer,
            identity_source text not null default 'local_sequence',
            task_id text not null,
            repo_name text not null,
            title text not null,
            base_line text not null,
            risk_tier text not null,
            lane text not null,
            status text not null,
            publication_state text not null,
            published_change_id text,
            published_at text,
            created_at text not null,
            updated_at text not null
        );
        """
    )
    conn.execute(
        """
        insert into workflow_tasks(
            task_id, task_seq, identity_source, repo_name, title, intent, risk_tier, planning_state, status, publication_state, published_task_id, published_at, created_at, updated_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            "ABCT-0001",
            1,
            local_control.LOCAL_IDENTITY_SOURCE_SEQUENCE,
            "repo-a",
            "Published task floor",
            "replay remote task floors during initialization",
            "medium",
            "planned",
            "active",
            "published",
            "ABCT-0042",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ),
    )
    conn.execute(
        """
        insert into workflow_changes(
            change_id, change_seq, identity_source, task_id, repo_name, title, base_line, risk_tier, lane, status, publication_state, published_change_id, published_at, created_at, updated_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            "ABCC-0001",
            1,
            local_control.LOCAL_IDENTITY_SOURCE_SEQUENCE,
            "ABCT-0001",
            "repo-a",
            "Published change floor",
            "main",
            "medium",
            "assisted",
            "draft",
            "published",
            "ABCC-0051",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ),
    )
    conn.commit()
    conn.close()

    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    assert local_control.get_workflow_sequence_floor(ctx, "repo-a", "T") == 42
    assert local_control.get_workflow_sequence_floor(ctx, "repo-a", "C") == 51
    assert local_control.allocate_workflow_task_identity(ctx, "repo-a", "ABC") == {"task_id": "ABCT-0043", "task_seq": 43}
    assert local_control.allocate_workflow_change_identity(ctx, "repo-a", "ABC") == {"change_id": "ABCC-0052", "change_seq": 52}


def test_local_control_publish_keeps_canonical_local_task_id_when_remote_short_id_collides(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    first = local_control.create_workflow_task(
        ctx,
        task_id="ABCT-0001",
        repo_name="repo-a",
        title="Older local task",
        intent="publish with a colliding remote short id",
        risk_tier="medium",
    )
    second = local_control.create_workflow_task(
        ctx,
        task_id="ABCT-0002",
        repo_name="repo-a",
        title="Later canonical local task",
        intent="keep the canonical local id authoritative",
        risk_tier="medium",
    )

    published = local_control.mark_workflow_task_published(
        ctx,
        first["task_id"],
        remote_name="origin",
        published_task_id=second["task_id"],
    )
    assert published["publication_state"] == "published"
    assert published["published_task_id"] == second["task_id"]
    assert local_control.get_workflow_task(ctx, second["task_id"])["task_id"] == second["task_id"]

    conn = connect_sqlite(ctx.control_db_path)
    try:
        aliases = conn.execute(
            "select alias_task_id, task_id from workflow_task_aliases where alias_task_id = ?",
            (second["task_id"],),
        ).fetchall()
        assert aliases == []
    finally:
        conn.close()


def test_local_control_publish_keeps_canonical_local_change_id_when_remote_short_id_collides(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    task = local_control.create_workflow_task(
        ctx,
        task_id="ABCT-0001",
        repo_name="repo-a",
        title="Task for colliding changes",
        intent="publish changes with a colliding remote short id",
        risk_tier="medium",
    )
    first = local_control.create_workflow_change(
        ctx,
        change_id="ABCC-0001",
        task_id=task["task_id"],
        repo_name="repo-a",
        title="Older local change",
        base_line="main",
        risk_tier="medium",
        lane="assisted",
    )
    second = local_control.create_workflow_change(
        ctx,
        change_id="ABCC-0002",
        task_id=task["task_id"],
        repo_name="repo-a",
        title="Later canonical local change",
        base_line="main",
        risk_tier="medium",
        lane="assisted",
    )

    published = local_control.mark_workflow_change_published(
        ctx,
        first["change_id"],
        remote_name="origin",
        published_change_id=second["change_id"],
    )
    assert published["publication_state"] == "published"
    assert published["published_change_id"] == second["change_id"]
    assert local_control.get_workflow_change(ctx, second["change_id"])["change_id"] == second["change_id"]

    conn = connect_sqlite(ctx.control_db_path)
    try:
        aliases = conn.execute(
            "select alias_change_id, change_id from workflow_change_aliases where alias_change_id = ?",
            (second["change_id"],),
        ).fetchall()
        assert aliases == []
    finally:
        conn.close()


def test_local_control_backfills_task_and_change_identity_metadata_and_alias_ready_schema(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)

    conn = connect_sqlite(ctx.control_db_path)
    conn.executescript(
        """
        create table if not exists control_meta (
            key text primary key,
            value text not null
        );
        create table workflow_tasks (
            task_id text primary key,
            repo_name text not null,
            title text not null,
            intent text not null,
            risk_tier text not null,
            planning_state text not null,
            status text not null,
            publication_state text not null,
            published_at text,
            created_at text not null,
            updated_at text not null
        );
        create table workflow_changes (
            change_id text primary key,
            task_id text not null,
            repo_name text not null,
            title text not null,
            base_line text not null,
            risk_tier text not null,
            lane text not null,
            status text not null,
            publication_state text not null,
            published_at text,
            created_at text not null,
            updated_at text not null
        );
        """
    )
    conn.execute(
        """
        insert into workflow_tasks(
            task_id, repo_name, title, intent, risk_tier, planning_state, status, publication_state, published_at, created_at, updated_at
        ) values
            ('ABCT-001', 'repo-a', 'legacy task', 'legacy intent', 'medium', 'planned', 'active', 'published', null, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
            ('LEGACY-TASK', 'repo-a', 'non-sequenced task', 'legacy intent', 'medium', 'planned', 'active', 'local_draft', null, '2026-01-01T00:00:01Z', '2026-01-01T00:00:01Z');
        """
    )
    conn.execute(
        """
        insert into workflow_changes(
            change_id, task_id, repo_name, title, base_line, risk_tier, lane, status, publication_state, published_at, created_at, updated_at
        ) values
            ('ABCC-001', 'ABCT-001', 'repo-a', 'legacy change', 'main', 'medium', 'assisted', 'review', 'published', null, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
            ('LEGACY-CHANGE', 'ABCT-001', 'repo-a', 'non-sequenced change', 'main', 'medium', 'assisted', 'review', 'local_draft', null, '2026-01-01T00:00:01Z', '2026-01-01T00:00:01Z');
        """
    )
    conn.commit()
    conn.close()

    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    conn = connect_sqlite(ctx.control_db_path)
    try:
        task = conn.execute("select * from workflow_tasks where task_id = 'ABCT-001'").fetchone()
        assert task is not None
        assert task["task_seq"] == 1
        assert task["identity_source"] == local_control.LOCAL_IDENTITY_SOURCE_SEQUENCE
        assert task["published_task_id"] == "ABCT-001"

        non_seq_task = conn.execute("select * from workflow_tasks where task_id = 'LEGACY-TASK'").fetchone()
        assert non_seq_task is not None
        assert non_seq_task["task_seq"] is None
        assert non_seq_task["identity_source"] == local_control.LOCAL_IDENTITY_SOURCE_LEGACY
        assert {"plan_id", "origin_plan_revision_id", "plan_item_ref", "plan_linked_at"} <= _table_columns(conn, "workflow_tasks")

        change = conn.execute("select * from workflow_changes where change_id = 'ABCC-001'").fetchone()
        assert change is not None
        assert change["change_seq"] == 1
        assert change["identity_source"] == local_control.LOCAL_IDENTITY_SOURCE_SEQUENCE
        assert change["published_change_id"] == "ABCC-001"

        index_names = _index_names(conn)
        assert "idx_workflow_task_aliases_task_id" in index_names
        assert "idx_workflow_change_aliases_change_id" in index_names
        assert "uq_workflow_tasks_repo_name_task_seq" in index_names
        assert "uq_workflow_changes_repo_name_change_seq" in index_names

        table_names = {str(row["name"]) for row in conn.execute("select name from sqlite_master where type = 'table'").fetchall()}
        assert "workflow_task_aliases" in table_names
        assert "workflow_change_aliases" in table_names
    finally:
        conn.close()


def test_local_control_publish_records_alias_rows_and_resolves_task_change_identity(tmp_path: Path) -> None:
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    task = local_control.create_workflow_task(
        ctx,
        task_id="ABCT-007",
        repo_name="repo-a",
        title="Published task alias",
        intent="backfill behavior",
        risk_tier="medium",
    )
    change = local_control.create_workflow_change(
        ctx,
        change_id="ABCC-007",
        task_id=task["task_id"],
        repo_name="repo-a",
        title="Published change alias",
        base_line="main",
        risk_tier="medium",
        lane="assisted",
    )

    first_task_publish = local_control.mark_workflow_task_published(ctx, task["task_id"], remote_name="origin-1", published_task_id="remote-task-7")
    first_change_publish = local_control.mark_workflow_change_published(
        ctx,
        change["change_id"],
        remote_name="origin-1",
        published_change_id="remote-change-7",
    )

    resolved_task = local_control.get_workflow_task(ctx, "remote-task-7")
    assert resolved_task["task_id"] == task["task_id"]
    assert resolved_task["published_remote_name"] == "origin-1"
    assert resolved_task["published_task_id"] == "remote-task-7"
    assert first_task_publish["published_at"] is not None

    resolved_change = local_control.get_workflow_change(ctx, "remote-change-7")
    assert resolved_change["change_id"] == change["change_id"]
    assert resolved_change["published_remote_name"] == "origin-1"
    assert resolved_change["published_change_id"] == "remote-change-7"
    assert first_change_publish["published_at"] is not None

    second_task_publish = local_control.mark_workflow_task_published(
        ctx,
        task["task_id"],
        remote_name="origin-2",
        published_task_id="remote-task-8",
    )
    assert second_task_publish["published_task_id"] == "remote-task-8"
    assert second_task_publish["published_remote_name"] == "origin-2"
    assert second_task_publish["published_at"] == first_task_publish["published_at"]

    second_change_publish = local_control.mark_workflow_change_published(
        ctx,
        change["change_id"],
        remote_name="origin-2",
        published_change_id="remote-change-8",
    )
    assert second_change_publish["published_change_id"] == "remote-change-8"
    assert second_change_publish["published_remote_name"] == "origin-2"
    assert second_change_publish["published_at"] == first_change_publish["published_at"]

    conn = connect_sqlite(ctx.control_db_path)
    try:
        task_alias_count = conn.execute(
            "select count(*) as count from workflow_task_aliases where task_id = ?",
            (task["task_id"],),
        ).fetchone()["count"]
        assert task_alias_count == 2

        change_alias_count = conn.execute(
            "select count(*) as count from workflow_change_aliases where change_id = ?",
            (change["change_id"],),
        ).fetchone()["count"]
        assert change_alias_count == 2

        task_alias = conn.execute(
            "select alias_kind from workflow_task_aliases where alias_task_id = ?",
            ("remote-task-7",),
        ).fetchone()
        assert task_alias is not None
        assert task_alias["alias_kind"] == "published_remote_id"

        change_alias = conn.execute(
            "select alias_kind from workflow_change_aliases where alias_change_id = ?",
            ("remote-change-7",),
        ).fetchone()
        assert change_alias is not None
        assert change_alias["alias_kind"] == "published_remote_id"
    finally:
        conn.close()
