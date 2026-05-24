from __future__ import annotations

import base64
import hashlib
import json
from pathlib import Path

from ait_server import authority_store, read_models, server_control, server_queue, server_store
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


def test_workflow_repo_id_writes_propagate_across_shared_workflow_tables(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    repo = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    repo_id = repo["repo_id"]

    plan = server_store.create_plan(
        ctx,
        "repo-a",
        "Repo scoped workflow identity",
        "docs/sprints/test.md",
        None,
        "Repo scoped workflow identity",
        [],
        summary="seed",
        artifact_body="# Repo scoped workflow identity\n",
    )
    planning_session = server_store.create_planning_session(ctx, plan["plan_id"], title="Plan repo scope")
    planning_event = server_store.append_planning_session_event(
        ctx,
        planning_session["planning_session_id"],
        "planning.note",
        {"summary": "capture repo scope"},
    )

    task = server_store.create_task(ctx, "repo-a", "Backfill repo_id", "Cover shared workflow rows", "high")
    change = server_store.create_change(ctx, "repo-a", task["task_id"], "Backfill workflow persistence", "main", "medium")

    base_snapshot = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-BASE",
            parent_snapshot_id=None,
            line_name="main",
            message="base",
            files={"README.md": b"base\n"},
        ),
    )
    revision_snapshot = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-REV",
            parent_snapshot_id=base_snapshot["snapshot_id"],
            line_name="main",
            message="revision",
            files={"README.md": b"base\nrevision\n"},
        ),
    )
    server_store.update_line(ctx, "repo-a", "main", base_snapshot["snapshot_id"])

    patchset = server_store.publish_patchset(
        ctx,
        change["change_id"],
        base_snapshot["snapshot_id"],
        revision_snapshot["snapshot_id"],
        "reviewable backfill",
        "human_only",
    )
    review_request = server_store.request_review(ctx, change["change_id"], patchset["patchset_id"], ["team-repo-a"])
    review = server_store.record_review(ctx, change["change_id"], patchset["patchset_id"], "alice@example.com", "approve", None)
    attestation = server_store.upsert_attestation(
        ctx,
        patchset["patchset_id"],
        "human_only",
        {"tests": "pass"},
        {"policy_readable": True},
    )
    policy = server_store.evaluate_policy(ctx, patchset["patchset_id"])
    waiver = server_store.create_waiver(ctx, patchset["patchset_id"], "lint", "optional lint waiver", inline=False)
    land = server_store.create_land_request(ctx, change["change_id"], patchset["patchset_id"], "main", "direct")

    session = server_store.create_session(
        ctx,
        "repo-a",
        "agent_run",
        task_id=task["task_id"],
        change_id=change["change_id"],
        title="repo scope execution",
    )
    session_event = server_store.append_session_event(ctx, session["session_id"], "agent.progress", {"status": "running"})
    checkpoint = server_store.create_session_checkpoint(
        ctx,
        session["session_id"],
        "repo scope checkpoint",
        snapshot_id=revision_snapshot["snapshot_id"],
    )

    stack = server_store.create_stack(ctx, "repo-a", "repo scope stack", [change["change_id"]])
    job = server_queue.enqueue_job(ctx, "repo-a", "custom.job", {"change_id": change["change_id"]})

    assert review_request["status"] == "requested"
    assert policy["decision"] == "pass"
    assert waiver["change_id"] == change["change_id"]
    assert land["submission_id"]

    conn = server_control.connect(ctx)
    try:
        table_checks = {
            "plans": conn.execute("select repo_id from plans where plan_id = ?", (plan["plan_id"],)).fetchone()["repo_id"],
            "plan_revision_blobs": conn.execute(
                "select repo_id from plan_revision_blobs where plan_revision_id = ?",
                (plan["head_revision"]["plan_revision_id"],),
            ).fetchone()["repo_id"],
            "tasks": conn.execute("select repo_id from tasks where task_id = ?", (task["task_id"],)).fetchone()["repo_id"],
            "changes": conn.execute("select repo_id from changes where change_id = ?", (change["change_id"],)).fetchone()["repo_id"],
            "sessions": conn.execute("select repo_id from sessions where session_id = ?", (session["session_id"],)).fetchone()["repo_id"],
            "session_events": conn.execute(
                "select repo_id from session_events where session_id = ? and sequence = ?",
                (session_event["session_id"], session_event["sequence"]),
            ).fetchone()["repo_id"],
            "session_checkpoints": conn.execute(
                "select repo_id from session_checkpoints where checkpoint_id = ?",
                (checkpoint["checkpoint_id"],),
            ).fetchone()["repo_id"],
            "planning_sessions": conn.execute(
                "select repo_id from planning_sessions where planning_session_id = ?",
                (planning_session["planning_session_id"],),
            ).fetchone()["repo_id"],
            "planning_session_events": conn.execute(
                "select repo_id from planning_session_events where planning_session_id = ? and sequence = ?",
                (planning_event["planning_session_id"], planning_event["sequence"]),
            ).fetchone()["repo_id"],
            "patchsets": conn.execute("select repo_id from patchsets where patchset_id = ?", (patchset["patchset_id"],)).fetchone()["repo_id"],
            "review_requests": conn.execute(
                "select repo_id from review_requests where change_id = ? and patchset_id = ?",
                (change["change_id"], patchset["patchset_id"]),
            ).fetchone()["repo_id"],
            "reviews": conn.execute("select repo_id from reviews where review_id = ?", (review["review_id"],)).fetchone()["repo_id"],
            "attestations": conn.execute(
                "select repo_id from attestations where attestation_id = ?",
                (attestation["attestation_id"],),
            ).fetchone()["repo_id"],
            "policy_decisions": conn.execute(
                "select repo_id from policy_decisions where patchset_id = ? order by policy_decision_id desc limit 1",
                (patchset["patchset_id"],),
            ).fetchone()["repo_id"],
            "waivers": conn.execute("select repo_id from waivers where waiver_id = ?", (waiver["waiver_id"],)).fetchone()["repo_id"],
            "land_requests": conn.execute(
                "select repo_id from land_requests where submission_id = ?",
                (land["submission_id"],),
            ).fetchone()["repo_id"],
            "stacks": conn.execute("select repo_id from stacks where stack_id = ?", (stack["stack_id"],)).fetchone()["repo_id"],
            "stack_changes": conn.execute(
                "select repo_id from stack_changes where stack_id = ? and change_id = ?",
                (stack["stack_id"], change["change_id"]),
            ).fetchone()["repo_id"],
            "jobs": conn.execute("select repo_id from jobs where job_id = ?", (job["job_id"],)).fetchone()["repo_id"],
        }
        assert all(value == repo_id for value in table_checks.values())
    finally:
        conn.close()


def test_workflow_repo_id_backfill_recovers_legacy_control_rows_and_indexes(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    repo = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    repo_id = repo["repo_id"]

    conn = server_control.connect(ctx)
    try:
        conn.execute(
            """
            insert into plans(plan_id, repo_name, repo_id, title, status, head_revision_id, created_by, created_at, updated_at)
            values ('AAAPL-LEGACY', 'repo-a', null, 'legacy plan', 'draft', 'AAAPR-LEGACY', 'system', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into plan_revisions(plan_revision_id, plan_id, revision_number, parent_plan_revision_id, title_snapshot, summary, artifact_path, artifact_selector, artifact_heading, items_json, plan_links_surface_hash, plan_links_changed_count_to_prev, source_kind, source_session_id, created_by, actor_type, created_at)
            values ('AAAPR-LEGACY', 'AAAPL-LEGACY', 1, null, 'legacy plan', null, 'docs/execution_plans/legacy.md', null, 'legacy plan', '[]', null, 0, 'manual_edit', null, 'system', 'system_worker', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into plan_revision_blobs(plan_revision_id, repo_name, repo_id, blob_id, media_type, encoding, byte_count, created_at)
            values ('AAAPR-LEGACY', 'repo-a', null, 'BLB-LEGACY-PLAN', 'text/markdown', 'utf-8', 11, '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into tasks(task_id, repo_name, repo_id, title, intent, risk_tier, planning_state, status, created_at)
            values ('AAAT-0001', 'repo-a', null, 'legacy task', 'legacy intent', 'medium', 'planned', 'active', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into changes(change_id, repo_name, repo_id, task_id, title, base_line, risk_tier, lane, status, current_patchset_number, created_at, updated_at)
            values ('AAAC-0001', 'repo-a', null, 'AAAT-0001', 'legacy change', 'main', 'medium', 'assisted', 'review', 1, '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into sessions(session_id, repo_name, repo_id, task_id, change_id, title, session_kind, status, metadata_json, last_event_sequence, created_at, updated_at)
            values ('AAAS-LEGACY', 'repo-a', null, 'AAAT-0001', 'AAAC-0001', 'legacy session', 'agent_run', 'active', '{}', 1, '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into planning_sessions(planning_session_id, repo_name, repo_id, plan_id, title, mode, status, preferred_agent, artifact_status, derived_task_id, last_promoted_plan_revision_id, last_event_sequence, created_by, created_at, updated_at)
            values ('AAAPS-LEGACY', 'repo-a', null, 'AAAPL-LEGACY', 'legacy planning', 'connected_local', 'active', null, 'not_promoted', null, null, 1, 'system', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into planning_session_events(repo_id, planning_session_id, sequence, event_type, payload_json, actor_identity, actor_type, created_at)
            values (null, 'AAAPS-LEGACY', 1, 'planning.note', '{}', 'system', 'system_worker', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into session_events(repo_id, session_id, sequence, event_type, payload_json, actor_identity, actor_type, created_at)
            values (null, 'AAAS-LEGACY', 1, 'session.note', '{}', 'system', 'system_worker', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into session_checkpoints(checkpoint_id, repo_id, session_id, based_on_sequence, summary, snapshot_id, resume_payload_json, created_at)
            values ('AAAK-LEGACY', null, 'AAAS-LEGACY', 1, 'legacy checkpoint', null, '{}', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into patchsets(patchset_id, repo_id, change_id, patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at)
            values ('AAAP-0001-1', null, 'AAAC-0001', 1, 'SNP-BASE', 'SNP-REV', 'legacy patchset', 'human', 'published', '{}', 'pending', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into review_requests(review_request_id, repo_id, change_id, patchset_id, reviewer_group, note, created_at)
            values (1, null, 'AAAC-0001', 'AAAP-0001-1', 'team-repo-a', null, '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into reviews(review_id, repo_id, change_id, patchset_id, reviewer, action, comment, blocking, created_at)
            values (1, null, 'AAAC-0001', 'AAAP-0001-1', 'alice@example.com', 'approve', null, 0, '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into attestations(attestation_id, repo_id, patchset_id, author_mode, evaluation_summary_json, provenance_summary_json, detail_json, created_at, updated_at)
            values ('AT-AAAP-0001-1', null, 'AAAP-0001-1', 'human', '{}', '{}', '{}', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into policy_decisions(policy_decision_id, repo_id, patchset_id, lane, decision, checks_json, created_at)
            values (1, null, 'AAAP-0001-1', 'assisted', 'pending', '[]', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into waivers(waiver_id, repo_id, patchset_id, rule_name, reason, expires_at, created_at)
            values ('W-AAAP-0001-1-1', null, 'AAAP-0001-1', 'lint', 'legacy waiver', null, '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into land_requests(submission_id, repo_id, change_id, patchset_id, target_line, mode, status, result_json, created_at, updated_at)
            values ('LAND-AAAC-0001-0001', null, 'AAAC-0001', 'AAAP-0001-1', 'main', 'direct', 'queued', '{}', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into stacks(stack_id, repo_name, repo_id, title, landing_policy, status, created_at, updated_at)
            values ('AAASK-0001', 'repo-a', null, 'legacy stack', 'ordered', 'active', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """
        )
        conn.execute(
            """
            insert into stack_changes(repo_id, stack_id, change_id, position)
            values (null, 'AAASK-0001', 'AAAC-0001', 1)
            """
        )
        conn.execute(
            """
            insert into jobs(job_id, repo_name, repo_id, job_type, state, payload_json, result_json, attempt_count, max_attempts, available_at, locked_at, locked_by, last_error, created_at, updated_at)
            values (1, 'repo-a', null, 'custom.job', 'queued', '{}', '{}', 0, 5, '2026-04-26T00:00:00Z', null, null, null, '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z')
            """
        )
        conn.commit()
    finally:
        conn.close()

    server_store.initialize(ctx)

    conn = server_control.connect(ctx)
    try:
        checks = [
            ("plans", conn.execute("select repo_id from plans where plan_id = 'AAAPL-LEGACY'").fetchone()["repo_id"]),
            (
                "plan_revision_blobs",
                conn.execute("select repo_id from plan_revision_blobs where plan_revision_id = 'AAAPR-LEGACY'").fetchone()["repo_id"],
            ),
            ("tasks", conn.execute("select repo_id from tasks where task_id = 'AAAT-0001'").fetchone()["repo_id"]),
            ("changes", conn.execute("select repo_id from changes where change_id = 'AAAC-0001'").fetchone()["repo_id"]),
            ("sessions", conn.execute("select repo_id from sessions where session_id = 'AAAS-LEGACY'").fetchone()["repo_id"]),
            ("planning_sessions", conn.execute("select repo_id from planning_sessions where planning_session_id = 'AAAPS-LEGACY'").fetchone()["repo_id"]),
            ("planning_session_events", conn.execute("select repo_id from planning_session_events where planning_session_id = 'AAAPS-LEGACY' and sequence = 1").fetchone()["repo_id"]),
            ("session_events", conn.execute("select repo_id from session_events where session_id = 'AAAS-LEGACY' and sequence = 1").fetchone()["repo_id"]),
            ("session_checkpoints", conn.execute("select repo_id from session_checkpoints where checkpoint_id = 'AAAK-LEGACY'").fetchone()["repo_id"]),
            ("patchsets", conn.execute("select repo_id from patchsets where patchset_id = 'AAAP-0001-1'").fetchone()["repo_id"]),
            ("review_requests", conn.execute("select repo_id from review_requests where review_request_id = 1").fetchone()["repo_id"]),
            ("reviews", conn.execute("select repo_id from reviews where review_id = 1").fetchone()["repo_id"]),
            ("attestations", conn.execute("select repo_id from attestations where attestation_id = 'AT-AAAP-0001-1'").fetchone()["repo_id"]),
            ("policy_decisions", conn.execute("select repo_id from policy_decisions where policy_decision_id = 1").fetchone()["repo_id"]),
            ("waivers", conn.execute("select repo_id from waivers where waiver_id = 'W-AAAP-0001-1-1'").fetchone()["repo_id"]),
            ("land_requests", conn.execute("select repo_id from land_requests where submission_id = 'LAND-AAAC-0001-0001'").fetchone()["repo_id"]),
            ("stacks", conn.execute("select repo_id from stacks where stack_id = 'AAASK-0001'").fetchone()["repo_id"]),
            ("stack_changes", conn.execute("select repo_id from stack_changes where stack_id = 'AAASK-0001' and change_id = 'AAAC-0001'").fetchone()["repo_id"]),
            ("jobs", conn.execute("select repo_id from jobs where job_id = 1").fetchone()["repo_id"]),
        ]
        assert all(value == repo_id for _, value in checks)
        assert "idx_plans_repo_id_updated" in _index_names(conn)
        assert "idx_plan_revision_blobs_repo_id_blob" in _index_names(conn)
        assert "idx_tasks_repo_id_created" in _index_names(conn)
        assert "idx_land_requests_repo_id_target_fifo" in _index_names(conn)
        assert "idx_jobs_repo_id_state" in _index_names(conn)
    finally:
        conn.close()


def test_workflow_repo_id_getters_prefer_repo_id_when_workflow_rows_drift(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    repo = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    repo_id = repo["repo_id"]

    task = server_store.create_task(ctx, "repo-a", "Drift-safe workflow reads", "Verify repo_id-scoped reads", "high")
    change = server_store.create_change(
        ctx,
        "repo-a",
        task["task_id"],
        "Drift-safe readable change",
        "main",
        "medium",
    )

    base_snapshot = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-REPOID-BASE",
            parent_snapshot_id=None,
            line_name="main",
            message="base",
            files={"README.md": b"base\n"},
        ),
    )
    revision_snapshot = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-REPOID-REV",
            parent_snapshot_id=base_snapshot["snapshot_id"],
            line_name="main",
            message="revision",
            files={"README.md": b"base\nrevision\n"},
        ),
    )
    server_store.update_line(ctx, "repo-a", "main", base_snapshot["snapshot_id"])

    patchset = server_store.publish_patchset(
        ctx,
        change["change_id"],
        base_snapshot["snapshot_id"],
        revision_snapshot["snapshot_id"],
        "drift-safe patchset",
        "human_only",
    )
    land = server_store.create_land_request(ctx, change["change_id"], patchset["patchset_id"], "main", "direct")

    with server_control.connect(ctx) as conn:
        drifted_repo_name = "repo-a-legacy"
        conn.execute("update tasks set repo_name = ? where task_id = ?", (drifted_repo_name, task["task_id"]))
        conn.execute("update changes set repo_name = ? where change_id = ?", (drifted_repo_name, change["change_id"]))
        conn.commit()

    drifted_task = server_store.get_task_for_repo(ctx, "repo-a", "1")
    assert drifted_task["task_id"] == task["task_id"]
    assert drifted_task["repo_id"] == repo_id

    drifted_change = server_store.get_change_for_repo(ctx, "repo-a", "1")
    assert drifted_change["change_id"] == change["change_id"]
    assert drifted_change["repo_id"] == repo_id

    drifted_patchset = server_store.get_patchset_for_repo(ctx, "repo-a", "1", change_ref="1")
    assert drifted_patchset["patchset_id"] == patchset["patchset_id"]
    assert drifted_patchset["repo_id"] == repo_id

    drifted_land = server_store.get_land_request_for_repo(ctx, "repo-a", "1")
    assert drifted_land["submission_id"] == land["submission_id"]
    assert drifted_land["repo_id"] == repo_id


def test_plan_repo_scoped_queries_prefer_repo_id_when_legacy_repo_name_drifts(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")

    plan = server_store.create_plan(
        ctx,
        "repo-a",
        "Repo-id scoped plans",
        "docs/sprints/test.md",
        None,
        "Repo-id scoped plans",
        [],
        summary="seed",
        artifact_body="# Repo-id scoped plans\n",
    )

    conn = server_control.connect(ctx)
    try:
        conn.execute("update plans set repo_name = 'repo-a-legacy' where plan_id = ?", (plan["plan_id"],))
        conn.execute(
            "update plan_revision_blobs set repo_name = 'repo-a-legacy' where plan_revision_id = ?",
            (plan["head_revision"]["plan_revision_id"],),
        )
        conn.commit()
    finally:
        conn.close()

    listed = server_store.list_plans(ctx, "repo-a")
    assert [row["plan_id"] for row in listed] == [plan["plan_id"]]

    task = server_store.create_task(
        ctx,
        "repo-a",
        "Link drifted repo-name plan",
        "Verify plan scope follows repo_id",
        "medium",
        plan_id=plan["plan_id"],
    )
    assert task["plan_id"] == plan["plan_id"]


def test_queue_repo_scoped_queries_prefer_repo_id_and_legacy_fallback(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    repo = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")

    drifted_name_id = repo["repo_id"]
    now = "2026-04-26T00:00:00Z"

    conn = server_control.connect(ctx)
    try:
        conn.execute(
            """
            insert into jobs(
                job_id, repo_name, repo_id, job_type, state, payload_json, result_json, attempt_count, max_attempts, available_at,
                locked_at, locked_by, last_error, created_at, updated_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                1001,
                "repo-a-legacy",
                drifted_name_id,
                "content.pack",
                "queued",
                "{}",
                "{}",
                0,
                3,
                now,
                None,
                None,
                None,
                now,
                now,
            ),
        )
        conn.execute(
            """
            insert into jobs(
                job_id, repo_name, repo_id, job_type, state, payload_json, result_json, attempt_count, max_attempts, available_at,
                locked_at, locked_by, last_error, created_at, updated_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                1002,
                "repo-a",
                None,
                "content.gc",
                "queued",
                "{}",
                "{}",
                0,
                3,
                now,
                None,
                None,
                None,
                now,
                now,
            ),
        )
        conn.commit()
    finally:
        conn.close()

    listed = server_queue.list_jobs(ctx, repo_name="repo-a", state="queued", limit=10)
    listed_ids = {int(job["job_id"]) for job in listed}
    assert listed_ids == {1001, 1002}

    first = server_queue.claim_next_job(ctx, "worker-1", repo_name="repo-a")
    second = server_queue.claim_next_job(ctx, "worker-2", repo_name="repo-a")
    assert first is not None
    assert second is not None
    assert {int(first["job_id"]), int(second["job_id"])} == {1001, 1002}


def test_read_models_prefers_repo_id_when_authority_map_repo_name_drifts(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    repo = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    authority_store.ensure_authority_map(ctx, "repo-a")

    conn = server_control.connect(ctx)
    try:
        conn.execute("update authority_maps set repo_name = ? where repo_id = ?", ("repo-a-legacy", repo["repo_id"]))
        conn.commit()
    finally:
        conn.close()

    payload = read_models.authority_map(ctx, repo_name="repo-a")

    assert payload["repo_name"] == "repo-a"
    assert payload["layer1"]["path"] == "docs/plan.md"


def test_read_models_repository_worker_status_prefers_repo_id_when_job_repo_name_drifts(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    repo = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    now = "2026-04-26T00:00:00Z"

    conn = server_control.connect(ctx)
    try:
        conn.execute(
            """
            insert into jobs(
                job_id, repo_name, repo_id, job_type, state, payload_json, result_json, attempt_count, max_attempts,
                available_at, locked_at, locked_by, last_error, created_at, updated_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                1001,
                "repo-a-legacy",
                repo["repo_id"],
                "content.gc",
                "running",
                "{}",
                "{}",
                0,
                3,
                now,
                now,
                "worker-1",
                None,
                now,
                now,
            ),
        )
        conn.commit()
    finally:
        conn.close()

    payload = read_models.repository_worker_status(ctx, "repo-a")

    assert payload["state_summary"].get("running") == 1
    assert payload["workers"][0]["worker_id"] == "worker-1"
    assert payload["workers"][0]["running_jobs"] == 1
    assert payload["recent_jobs"][0]["job_id"] == 1001


def test_read_models_repository_detail_prefers_repo_id_when_job_repo_name_drifts(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    repo = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    drifted_repo_name = "repo-a-legacy"
    now = "2026-04-26T00:00:00Z"

    conn = server_control.connect(ctx)
    try:
        conn.execute(
            """
            insert into jobs(job_id, repo_name, repo_id, job_type, state, payload_json, result_json, attempt_count, max_attempts, available_at, locked_at, locked_by, last_error, created_at, updated_at)
            values (1001, ?, ?, 'content.gc', 'queued', '{}', '{}', 0, 3, ?, ?, ?, null, ?, ?)
            """,
            (
                drifted_repo_name,
                repo["repo_id"],
                now,
                None,
                None,
                now,
                now,
            ),
        )
        conn.commit()
    finally:
        conn.close()

    payload = read_models.repository_detail(ctx, "repo-a", job_limit=10)

    assert payload["repository"]["repo_name"] == "repo-a"
    job = payload["jobs"][0]
    assert int(job["job_id"]) == 1001
    assert job["repo_name"] == drifted_repo_name
    assert job["state"] == "queued"


def test_read_models_reviewer_inbox_prefers_repo_id_when_change_repo_name_drifts(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    repo = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    now = "2026-04-26T00:00:00Z"

    task = server_store.create_task(ctx, "repo-a", "reviewer inbox drift", "survive repo rename", "medium")
    drifted_change_id = "AAAC-REVIEWER-0001"

    conn = server_control.connect(ctx)
    try:
        conn.execute(
            """
            insert into changes(
                change_id,
                repo_name,
                repo_id,
                task_id,
                title,
                base_line,
                risk_tier,
                lane,
                status,
                current_patchset_number,
                created_at,
                updated_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                drifted_change_id,
                "repo-a-legacy",
                repo["repo_id"],
                task["task_id"],
                "reviewable drifted change",
                "main",
                "medium",
                "assisted",
                "review",
                0,
                now,
                now,
            ),
        )
        conn.commit()
    finally:
        conn.close()

    inbox = read_models.reviewer_inbox(ctx, repo_name="repo-a")

    assert inbox["count"] == 1
    assert inbox["items"][0]["change_id"] == drifted_change_id
