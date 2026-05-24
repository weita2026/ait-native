from __future__ import annotations

import base64
import hashlib
from pathlib import Path

import pytest

from ait_server import server_content, server_control, server_store
from ait_server.server_paths import ServerContext
from tests.postgres_fake import fake_postgres_context


def _index_names(conn) -> set[str]:
    rows = conn.execute("select name from sqlite_master where type = 'index'").fetchall()
    return {str(row["name"]) for row in rows if row.get("name")}


def _snapshot_bundle(
    repo_name: str,
    snapshot_id: str,
    *,
    parent_snapshot_id: str | None,
    line_name: str,
    message: str,
    files: dict[str, bytes],
) -> dict:
    file_rows = []
    for path, data in files.items():
        blob_id = f"BLB-{snapshot_id}-{path.replace('/', '_')}"
        file_rows.append(
            {
                "path": path,
                "blob_id": blob_id,
                "size_bytes": len(data),
                "mode": "100644",
                "sha256": hashlib.sha256(data).hexdigest(),
                "content_b64": base64.b64encode(data).decode("ascii"),
            }
        )
    return {
        "snapshot_id": snapshot_id,
        "repo_name": repo_name,
        "parent_snapshot_id": parent_snapshot_id,
        "line_name": line_name,
        "message": message,
        "files": file_rows,
    }


def _publish_seed_patchset(ctx: ServerContext, repo_name: str, change_id: str, *, suffix: str) -> dict:
    base_snapshot = server_store.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            f"SNP-{suffix}-BASE",
            parent_snapshot_id=None,
            line_name="main",
            message="base",
            files={"README.md": b"base\n"},
        ),
    )
    revision_snapshot = server_store.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            f"SNP-{suffix}-REV",
            parent_snapshot_id=base_snapshot["snapshot_id"],
            line_name="main",
            message="revision",
            files={"README.md": f"base\n{suffix}\n".encode("utf-8")},
        ),
    )
    server_store.update_line(ctx, repo_name, "main", base_snapshot["snapshot_id"])
    return server_store.publish_patchset(
        ctx,
        change_id,
        base_snapshot["snapshot_id"],
        revision_snapshot["snapshot_id"],
        f"patchset {suffix}",
        "human_only",
    )


def test_repo_scoped_write_paths_allow_same_local_sequence_across_repositories(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="BBB")

    plan_a = server_store.create_plan(
        ctx,
        "repo-a",
        "Repo A plan",
        "docs/sprints/repo_a.md",
        None,
        "Repo A plan",
        [],
        summary="seed",
        artifact_body="# Repo A\n",
    )
    plan_b = server_store.create_plan(
        ctx,
        "repo-b",
        "Repo B plan",
        "docs/sprints/repo_b.md",
        None,
        "Repo B plan",
        [],
        summary="seed",
        artifact_body="# Repo B\n",
    )

    planning_a = server_store.create_planning_session(ctx, plan_a["plan_id"], planning_session_id="AAAPS-0001")
    planning_b = server_store.create_planning_session(ctx, plan_b["plan_id"], planning_session_id="BBBPS-0001")

    task_a = server_store.create_task(ctx, "repo-a", "Task A", "repo scoped keys", "high")
    task_b = server_store.create_task(ctx, "repo-b", "Task B", "repo scoped keys", "high")
    assert task_a["task_seq"] == 1
    assert task_b["task_seq"] == 1

    change_a = server_store.create_change(ctx, "repo-a", task_a["task_id"], "Change A", "main", "medium")
    change_b = server_store.create_change(ctx, "repo-b", task_b["task_id"], "Change B", "main", "medium")
    assert change_a["change_seq"] == 1
    assert change_b["change_seq"] == 1

    patchset_a = _publish_seed_patchset(ctx, "repo-a", change_a["change_id"], suffix="A1")
    patchset_b = _publish_seed_patchset(ctx, "repo-b", change_b["change_id"], suffix="B1")

    land_a = server_store.create_land_request(ctx, change_a["change_id"], patchset_a["patchset_id"], "main", "direct")
    land_b = server_store.create_land_request(ctx, change_b["change_id"], patchset_b["patchset_id"], "main", "direct")
    assert land_a["land_seq"] == 1
    assert land_b["land_seq"] == 1

    change_a_second = server_store.create_change(ctx, "repo-a", task_a["task_id"], "Change A2", "main", "medium")
    patchset_a_second = _publish_seed_patchset(ctx, "repo-a", change_a_second["change_id"], suffix="A2")
    land_a_second = server_store.create_land_request(
        ctx,
        change_a_second["change_id"],
        patchset_a_second["patchset_id"],
        "main",
        "direct",
    )
    assert land_a_second["land_seq"] == 2

    session_a = server_store.create_session(ctx, "repo-a", "agent_run", task_id=task_a["task_id"], change_id=change_a["change_id"], session_id="AAAS-0001")
    session_b = server_store.create_session(ctx, "repo-b", "agent_run", task_id=task_b["task_id"], change_id=change_b["change_id"], session_id="BBBS-0001")
    checkpoint_a = server_store.create_session_checkpoint(ctx, session_a["session_id"], "checkpoint a", checkpoint_id="AAAK-0001")
    checkpoint_b = server_store.create_session_checkpoint(ctx, session_b["session_id"], "checkpoint b", checkpoint_id="BBBK-0001")

    stack_a = server_store.create_stack(ctx, "repo-a", "Stack A", [change_a["change_id"], change_a_second["change_id"]])
    stack_b = server_store.create_stack(ctx, "repo-b", "Stack B", [change_b["change_id"]])

    assert planning_a["planning_session_local_id"] == "0001"
    assert planning_b["planning_session_local_id"] == "0001"
    assert session_a["session_local_id"] == "0001"
    assert session_b["session_local_id"] == "0001"
    assert checkpoint_a["checkpoint_local_id"] == "0001"
    assert checkpoint_b["checkpoint_local_id"] == "0001"
    assert stack_a["stack_seq"] == 1
    assert stack_b["stack_seq"] == 1


def test_repo_scoped_update_line_keeps_explicit_cas_protection(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")

    base_snapshot = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-CAS-BASE",
            parent_snapshot_id=None,
            line_name="main",
            message="base",
            files={"README.md": b"base\n"},
        ),
    )
    current_snapshot = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-CAS-CURRENT",
            parent_snapshot_id=base_snapshot["snapshot_id"],
            line_name="main",
            message="current",
            files={"README.md": b"base\ncurrent\n"},
        ),
    )
    next_snapshot = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-CAS-NEXT",
            parent_snapshot_id=current_snapshot["snapshot_id"],
            line_name="main",
            message="next",
            files={"README.md": b"base\ncurrent\nnext\n"},
        ),
    )

    server_store.update_line(ctx, "repo-a", "main", base_snapshot["snapshot_id"])
    server_store.update_line(
        ctx,
        "repo-a",
        "main",
        current_snapshot["snapshot_id"],
        expected_head_snapshot_id=base_snapshot["snapshot_id"],
    )

    with pytest.raises(ValueError, match="head advanced before update"):
        server_store.update_line(
            ctx,
            "repo-a",
            "main",
            next_snapshot["snapshot_id"],
            expected_head_snapshot_id=base_snapshot["snapshot_id"],
        )


def test_repo_scoped_update_line_allows_repeated_updates_without_explicit_cas(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")

    base_snapshot = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-OPTIONAL-CAS-BASE",
            parent_snapshot_id=None,
            line_name="main",
            message="base",
            files={"README.md": b"base\n"},
        ),
    )
    current_snapshot = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-OPTIONAL-CAS-CURRENT",
            parent_snapshot_id=base_snapshot["snapshot_id"],
            line_name="main",
            message="current",
            files={"README.md": b"base\ncurrent\n"},
        ),
    )
    next_snapshot = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-OPTIONAL-CAS-NEXT",
            parent_snapshot_id=current_snapshot["snapshot_id"],
            line_name="main",
            message="next",
            files={"README.md": b"base\ncurrent\nnext\n"},
        ),
    )

    first = server_store.update_line(ctx, "repo-a", "main", base_snapshot["snapshot_id"])
    assert first["head_snapshot_id"] == base_snapshot["snapshot_id"]

    second = server_store.update_line(ctx, "repo-a", "main", current_snapshot["snapshot_id"])
    assert second["head_snapshot_id"] == current_snapshot["snapshot_id"]

    third = server_store.update_line(ctx, "repo-a", "main", next_snapshot["snapshot_id"])
    assert third["head_snapshot_id"] == next_snapshot["snapshot_id"]


def test_repo_scoped_local_key_backfill_recovers_legacy_rows_and_indexes(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    repo_a = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    repo_b = server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="BBB")

    conn = server_control.connect(ctx)
    try:
        conn.execute(
            """
            insert into tasks(task_id, repo_name, repo_id, task_seq, title, intent, risk_tier, planning_state, status, created_at)
            values
                ('AAAT-0001', 'repo-a', ?, null, 'legacy task a', 'legacy', 'medium', 'planned', 'active', '2026-04-26T00:00:00Z'),
                ('BBBT-0001', 'repo-b', ?, null, 'legacy task b', 'legacy', 'medium', 'planned', 'active', '2026-04-26T00:00:00Z')
            """,
            (repo_a["repo_id"], repo_b["repo_id"]),
        )
        conn.execute(
            """
            insert into changes(change_id, repo_name, repo_id, change_seq, task_id, title, base_line, risk_tier, lane, status, current_patchset_number, created_at, updated_at)
            values
                ('AAAC-0001', 'repo-a', ?, null, 'AAAT-0001', 'legacy change a', 'main', 'medium', 'assisted', 'review', 1, '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
                ('AAAC-0002', 'repo-a', ?, null, 'AAAT-0001', 'legacy change a2', 'main', 'medium', 'assisted', 'review', 1, '2026-04-26T00:01:00Z', '2026-04-26T00:01:00Z'),
                ('BBBC-0001', 'repo-b', ?, null, 'BBBT-0001', 'legacy change b', 'main', 'medium', 'assisted', 'review', 1, '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """,
            (repo_a["repo_id"], repo_a["repo_id"], repo_b["repo_id"]),
        )
        conn.execute(
            """
            insert into planning_sessions(planning_session_id, repo_name, repo_id, planning_session_local_id, plan_id, title, mode, status, preferred_agent, artifact_status, derived_task_id, last_promoted_plan_revision_id, last_event_sequence, created_by, created_at, updated_at)
            values
                ('AAAPS-0001', 'repo-a', ?, null, 'PL-A', 'legacy planning a', 'connected_local', 'active', null, 'not_promoted', null, null, 0, 'system', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
                ('BBBPS-0001', 'repo-b', ?, null, 'PL-B', 'legacy planning b', 'connected_local', 'active', null, 'not_promoted', null, null, 0, 'system', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """,
            (repo_a["repo_id"], repo_b["repo_id"]),
        )
        conn.execute(
            """
            insert into sessions(session_id, repo_name, repo_id, session_local_id, task_id, change_id, title, session_kind, status, metadata_json, last_event_sequence, created_at, updated_at)
            values
                ('AAAS-0001', 'repo-a', ?, null, 'AAAT-0001', 'AAAC-0001', 'legacy session a', 'agent_run', 'active', '{}', 0, '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
                ('BBBS-0001', 'repo-b', ?, null, 'BBBT-0001', 'BBBC-0001', 'legacy session b', 'agent_run', 'active', '{}', 0, '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """,
            (repo_a["repo_id"], repo_b["repo_id"]),
        )
        conn.execute(
            """
            insert into session_checkpoints(checkpoint_id, repo_id, checkpoint_local_id, session_id, based_on_sequence, summary, snapshot_id, resume_payload_json, created_at)
            values
                ('AAAK-0001', ?, null, 'AAAS-0001', 0, 'legacy checkpoint a', null, '{}', '2026-04-26T00:00:00Z'),
                ('BBBK-0001', ?, null, 'BBBS-0001', 0, 'legacy checkpoint b', null, '{}', '2026-04-26T00:00:00Z')
            """,
            (repo_a["repo_id"], repo_b["repo_id"]),
        )
        conn.execute(
            """
            insert into patchsets(patchset_id, repo_id, change_id, patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at)
            values
                ('AAAP-0001-1', ?, 'AAAC-0001', 1, 'SNP-A-BASE', 'SNP-A-REV', 'legacy patchset a', 'human', 'published', '{}', 'pending', '2026-04-26T00:00:00Z'),
                ('AAAP-0002-1', ?, 'AAAC-0002', 1, 'SNP-A2-BASE', 'SNP-A2-REV', 'legacy patchset a2', 'human', 'published', '{}', 'pending', '2026-04-26T00:01:00Z'),
                ('BBBP-0001-1', ?, 'BBBC-0001', 1, 'SNP-B-BASE', 'SNP-B-REV', 'legacy patchset b', 'human', 'published', '{}', 'pending', '2026-04-26T00:00:00Z')
            """,
            (repo_a["repo_id"], repo_a["repo_id"], repo_b["repo_id"]),
        )
        conn.execute(
            """
            insert into land_requests(submission_id, repo_id, land_seq, change_id, patchset_id, target_line, mode, status, result_json, created_at, updated_at)
            values
                ('LAND-AAAC-0001-0001', ?, null, 'AAAC-0001', 'AAAP-0001-1', 'main', 'direct', 'queued', '{}', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
                ('LAND-AAAC-0002-0001', ?, null, 'AAAC-0002', 'AAAP-0002-1', 'main', 'direct', 'queued', '{}', '2026-04-26T00:01:00Z', '2026-04-26T00:01:00Z'),
                ('LAND-BBBC-0001-0001', ?, null, 'BBBC-0001', 'BBBP-0001-1', 'main', 'direct', 'queued', '{}', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """,
            (repo_a["repo_id"], repo_a["repo_id"], repo_b["repo_id"]),
        )
        conn.execute(
            """
            insert into stacks(stack_id, repo_name, repo_id, stack_seq, title, landing_policy, status, created_at, updated_at)
            values
                ('AAASK-0001', 'repo-a', ?, null, 'legacy stack a', 'ordered', 'active', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
                ('BBBSK-0001', 'repo-b', ?, null, 'legacy stack b', 'ordered', 'active', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """,
            (repo_a["repo_id"], repo_b["repo_id"]),
        )
        conn.commit()
    finally:
        conn.close()

    server_store.initialize(ctx)

    conn = server_control.connect(ctx)
    try:
        assert conn.execute("select task_seq from tasks where task_id = 'AAAT-0001'").fetchone()["task_seq"] == 1
        assert conn.execute("select task_seq from tasks where task_id = 'BBBT-0001'").fetchone()["task_seq"] == 1
        assert conn.execute("select change_seq from changes where change_id = 'AAAC-0001'").fetchone()["change_seq"] == 1
        assert conn.execute("select change_seq from changes where change_id = 'AAAC-0002'").fetchone()["change_seq"] == 2
        assert conn.execute("select change_seq from changes where change_id = 'BBBC-0001'").fetchone()["change_seq"] == 1
        assert conn.execute("select planning_session_local_id from planning_sessions where planning_session_id = 'AAAPS-0001'").fetchone()["planning_session_local_id"] == "0001"
        assert conn.execute("select planning_session_local_id from planning_sessions where planning_session_id = 'BBBPS-0001'").fetchone()["planning_session_local_id"] == "0001"
        assert conn.execute("select session_local_id from sessions where session_id = 'AAAS-0001'").fetchone()["session_local_id"] == "0001"
        assert conn.execute("select session_local_id from sessions where session_id = 'BBBS-0001'").fetchone()["session_local_id"] == "0001"
        assert conn.execute("select checkpoint_local_id from session_checkpoints where checkpoint_id = 'AAAK-0001'").fetchone()["checkpoint_local_id"] == "0001"
        assert conn.execute("select checkpoint_local_id from session_checkpoints where checkpoint_id = 'BBBK-0001'").fetchone()["checkpoint_local_id"] == "0001"
        assert conn.execute("select land_seq from land_requests where submission_id = 'LAND-AAAC-0001-0001'").fetchone()["land_seq"] == 1
        assert conn.execute("select land_seq from land_requests where submission_id = 'LAND-AAAC-0002-0001'").fetchone()["land_seq"] == 2
        assert conn.execute("select land_seq from land_requests where submission_id = 'LAND-BBBC-0001-0001'").fetchone()["land_seq"] == 1
        assert conn.execute("select stack_seq from stacks where stack_id = 'AAASK-0001'").fetchone()["stack_seq"] == 1
        assert conn.execute("select stack_seq from stacks where stack_id = 'BBBSK-0001'").fetchone()["stack_seq"] == 1

        index_names = _index_names(conn)
        assert "uq_tasks_repo_id_task_seq" in index_names
        assert "uq_changes_repo_id_change_seq" in index_names
        assert "uq_sessions_repo_id_local_id" in index_names
        assert "uq_planning_sessions_repo_id_local_id" in index_names
        assert "uq_session_checkpoints_repo_id_local_id" in index_names
        assert "uq_land_requests_repo_id_land_seq" in index_names
        assert "uq_stacks_repo_id_stack_seq" in index_names
    finally:
        conn.close()
