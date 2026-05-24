from __future__ import annotations

import base64
import hashlib
from pathlib import Path

import pytest

from ait_server import server_store
from ait_server.server_paths import ServerContext
from tests.postgres_fake import fake_postgres_context, restore_real_psycopg
from ait_server.server_queue import enqueue_async_job
from ait_server.worker import process_one
from tests.postgres_live import live_postgres_available, running_live_postgres


LIVE_POSTGRES = pytest.mark.skipif(not live_postgres_available(), reason="live PostgreSQL binaries or psycopg are unavailable")


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


def _publish_ready_change(ctx: ServerContext, repo_name: str, suffix: str) -> tuple[dict, dict, dict]:
    task = server_store.create_task(ctx, repo_name, f"Task {suffix}", "repo scoped validation", "high")
    change = server_store.create_change(ctx, repo_name, task["task_id"], f"Change {suffix}", "main", "medium")
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
    patchset = server_store.publish_patchset(
        ctx,
        change["change_id"],
        base_snapshot["snapshot_id"],
        revision_snapshot["snapshot_id"],
        f"patchset {suffix}",
        "human_only",
    )
    server_store.upsert_attestation(
        ctx,
        patchset["patchset_id"],
        "human_only",
        {"tests": "pass"},
        {"policy_readable": True},
    )
    server_store.record_review(ctx, change["change_id"], patchset["patchset_id"], "alice@example.com", "approve", None)
    return task, change, patchset


def test_repo_scoped_async_worker_paths_handle_overlapping_local_refs(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    repo_a = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    repo_b = server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="BBB")

    task_a, change_a, patchset_a = _publish_ready_change(ctx, "repo-a", "A1")
    task_b, change_b, patchset_b = _publish_ready_change(ctx, "repo-b", "B1")

    assert task_a["task_seq"] == 1
    assert task_b["task_seq"] == 1
    assert change_a["change_seq"] == 1
    assert change_b["change_seq"] == 1

    policy_job_a = enqueue_async_job(
        ctx,
        "repo-a",
        "policy.evaluate",
        {
            "patchset_id": patchset_a["patchset_id"],
            "repo_name": "repo-a",
            "repo_id": repo_a["repo_id"],
            "change_id": change_a["change_id"],
            "change_seq": change_a["change_seq"],
            "patchset_number": patchset_a["patchset_number"],
        },
    )
    policy_job_b = enqueue_async_job(
        ctx,
        "repo-b",
        "policy.evaluate",
        {
            "patchset_id": patchset_b["patchset_id"],
            "repo_name": "repo-b",
            "repo_id": repo_b["repo_id"],
            "change_id": change_b["change_id"],
            "change_seq": change_b["change_seq"],
            "patchset_number": patchset_b["patchset_number"],
        },
    )

    processed_a = process_one(ctx, worker_id="validation-worker")
    processed_b = process_one(ctx, worker_id="validation-worker")
    assert processed_a is not None and processed_a["state"] == "succeeded"
    assert processed_b is not None and processed_b["state"] == "succeeded"
    assert {processed_a["job_id"], processed_b["job_id"]} == {policy_job_a["job_id"], policy_job_b["job_id"]}
    assert server_store.get_policy_status(ctx, patchset_a["patchset_id"])["decision"] == "pass"
    assert server_store.get_policy_status(ctx, patchset_b["patchset_id"])["decision"] == "pass"

    land_a = server_store.create_land_request(ctx, change_a["change_id"], patchset_a["patchset_id"], "main", "direct")
    land_b = server_store.create_land_request(ctx, change_b["change_id"], patchset_b["patchset_id"], "main", "direct")
    assert land_a["land_seq"] == 1
    assert land_b["land_seq"] == 1

    land_job_a = enqueue_async_job(
        ctx,
        "repo-a",
        "land.process",
        {
            "submission_id": land_a["submission_id"],
            "repo_name": "repo-a",
            "repo_id": repo_a["repo_id"],
            "change_id": change_a["change_id"],
            "change_seq": change_a["change_seq"],
            "patchset_id": patchset_a["patchset_id"],
            "land_seq": land_a["land_seq"],
        },
    )
    land_job_b = enqueue_async_job(
        ctx,
        "repo-b",
        "land.process",
        {
            "submission_id": land_b["submission_id"],
            "repo_name": "repo-b",
            "repo_id": repo_b["repo_id"],
            "change_id": change_b["change_id"],
            "change_seq": change_b["change_seq"],
            "patchset_id": patchset_b["patchset_id"],
            "land_seq": land_b["land_seq"],
        },
    )

    processed_land_a = process_one(ctx, worker_id="validation-worker")
    processed_land_b = process_one(ctx, worker_id="validation-worker")
    assert processed_land_a is not None and processed_land_a["state"] == "succeeded"
    assert processed_land_b is not None and processed_land_b["state"] == "succeeded"
    assert {processed_land_a["job_id"], processed_land_b["job_id"]} == {land_job_a["job_id"], land_job_b["job_id"]}
    assert server_store.get_land_request_for_repo(ctx, "repo-a", "1")["status"] == "succeeded"
    assert server_store.get_land_request_for_repo(ctx, "repo-b", "1")["status"] == "succeeded"


def test_repo_scoped_async_worker_rejects_mismatched_repo_scope(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    repo_a = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    repo_b = server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="BBB")
    _, change_a, patchset_a = _publish_ready_change(ctx, "repo-a", "A1")

    bad_job = enqueue_async_job(
        ctx,
        "repo-a",
        "policy.evaluate",
        {
            "patchset_id": patchset_a["patchset_id"],
            "repo_name": "repo-a",
            "repo_id": repo_b["repo_id"],
            "change_id": change_a["change_id"],
            "change_seq": change_a["change_seq"],
            "patchset_number": patchset_a["patchset_number"],
        },
    )

    processed = process_one(ctx, worker_id="validation-worker")
    assert processed is not None
    assert processed["job_id"] == bad_job["job_id"]
    assert processed["state"] == "failed"
    assert "Repository scope mismatch" in str(processed.get("last_error") or "")

    assert server_store.get_policy_status(ctx, patchset_a["patchset_id"])["decision"] == "pending"
    assert repo_a["repo_id"] != repo_b["repo_id"]


@LIVE_POSTGRES
def test_live_postgres_repo_scoped_local_sequences_validate_per_repository(tmp_path: Path):
    with running_live_postgres(tmp_path / "live-postgres-validation") as runtime:
        restore_real_psycopg()
        ctx = ServerContext.create(
            tmp_path / "server-data-pg",
            backend="postgres",
            postgres_dsn=runtime["dsn"],
        )
        server_store.initialize(ctx)

        server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
        server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="BBB")

        task_a, change_a, patchset_a = _publish_ready_change(ctx, "repo-a", "PA1")
        task_b, change_b, patchset_b = _publish_ready_change(ctx, "repo-b", "PB1")
        land_a = server_store.create_land_request(ctx, change_a["change_id"], patchset_a["patchset_id"], "main", "direct")
        land_b = server_store.create_land_request(ctx, change_b["change_id"], patchset_b["patchset_id"], "main", "direct")

        assert task_a["task_seq"] == 1
        assert task_b["task_seq"] == 1
        assert change_a["change_seq"] == 1
        assert change_b["change_seq"] == 1
        assert land_a["land_seq"] == 1
        assert land_b["land_seq"] == 1
        assert server_store.get_task_for_repo(ctx, "repo-a", "1")["task_id"] == task_a["task_id"]
        assert server_store.get_change_for_repo(ctx, "repo-b", "1")["change_id"] == change_b["change_id"]
        assert server_store.get_land_request_for_repo(ctx, "repo-a", "1")["submission_id"] == land_a["submission_id"]
