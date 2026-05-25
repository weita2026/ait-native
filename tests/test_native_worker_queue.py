from __future__ import annotations

import json
import os
import socket
import threading
import time
import urllib.request
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

import pytest
import uvicorn
from typer.testing import CliRunner

import ait_native.read_models as native_read_models
import ait_native.server_db as server_db
from ait_native.cli import app
from ait_native.server import create_app
from ait_native.server_content import connect as connect_content
from ait_native.server_control import connect as connect_control
from ait_native.server_paths import ServerContext
from ait_native.server_queue import (
    async_job_contract,
    enqueue_async_job,
    claim_next_job,
    enqueue_job,
    fail_job,
    get_job,
    job_diagnostics,
    normalize_async_job_payload,
    reclaim_stale_jobs,
)
from ait_native.read_models import server_metrics, server_readiness
from ait_native.server_store import (
    create_plan,
    create_planning_session,
    create_session,
    ensure_repository,
    initialize,
)
from ait_native.worker import app as worker_app, process_one
from tests.ait_web.helpers import _create_plan_bound_task, _ensure_task_worktree
from tests.postgres_fake import (
    FakePsycopg,
    fake_postgres_context,
    fake_postgres_dsn,
    install_fake_psycopg_global,
    reset_fake_postgres_runtime,
    restore_real_psycopg,
)
from tests.postgres_live import live_postgres_available, running_live_postgres

runner = CliRunner()
worker_runner = CliRunner()
LIVE_POSTGRES = pytest.mark.skipif(not live_postgres_available(), reason="live PostgreSQL binaries or psycopg are unavailable")
WORKER_CODE_REVIEW_SUMMARY = (
    "Reviewed files: README.md, queue flow; Findings: no blocking findings; "
    "Risks: low async worker regression risk; Tests: targeted async worker pytest coverage; "
    "Recommendation: safe to land."
)


@contextmanager
def running_server(data_dir: Path, auth_mode: str = "open", queue_mode: str = "async"):
    old_data = os.environ.get("AIT_NATIVE_SERVER_DATA")
    old_mode = os.environ.get("AIT_NATIVE_AUTH_MODE")
    old_queue = os.environ.get("AIT_NATIVE_QUEUE_MODE")
    old_backend = os.environ.get("AIT_NATIVE_SERVER_DB_BACKEND")
    old_dsn = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN")
    old_content_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA")
    old_control_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA")
    install_fake_psycopg_global()
    reset_fake_postgres_runtime()
    os.environ["AIT_NATIVE_SERVER_DATA"] = str(data_dir)
    os.environ["AIT_NATIVE_AUTH_MODE"] = auth_mode
    os.environ["AIT_NATIVE_QUEUE_MODE"] = queue_mode
    os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = "postgres"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = fake_postgres_dsn(data_dir)
    os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA"] = "ait_native_content"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA"] = "ait_native_control"
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    host, port = sock.getsockname()
    sock.close()
    app_obj = create_app()
    config = uvicorn.Config(app_obj, host=host, port=port, log_level="error")
    server = uvicorn.Server(config)
    thread = threading.Thread(target=server.run, daemon=True)
    thread.start()
    base_url = f"http://{host}:{port}"
    deadline = time.time() + 10
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"{base_url}/healthz", timeout=0.5) as resp:
                if resp.status == 200:
                    break
        except Exception:
            time.sleep(0.05)
    else:
        server.should_exit = True
        thread.join(timeout=5)
        raise RuntimeError("native test server did not start")
    try:
        yield base_url, data_dir
    finally:
        server.should_exit = True
        thread.join(timeout=5)
        reset_fake_postgres_runtime()
        if old_data is None:
            os.environ.pop("AIT_NATIVE_SERVER_DATA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DATA"] = old_data
        if old_mode is None:
            os.environ.pop("AIT_NATIVE_AUTH_MODE", None)
        else:
            os.environ["AIT_NATIVE_AUTH_MODE"] = old_mode
        if old_queue is None:
            os.environ.pop("AIT_NATIVE_QUEUE_MODE", None)
        else:
            os.environ["AIT_NATIVE_QUEUE_MODE"] = old_queue
        if old_backend is None:
            os.environ.pop("AIT_NATIVE_SERVER_DB_BACKEND", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = old_backend
        if old_dsn is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_DSN", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = old_dsn
        if old_content_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA"] = old_content_schema
        if old_control_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA"] = old_control_schema


@contextmanager
def running_postgres_server(data_dir: Path, dsn: str, auth_mode: str = "open", queue_mode: str = "async"):
    old_data = os.environ.get("AIT_NATIVE_SERVER_DATA")
    old_mode = os.environ.get("AIT_NATIVE_AUTH_MODE")
    old_queue = os.environ.get("AIT_NATIVE_QUEUE_MODE")
    old_backend = os.environ.get("AIT_NATIVE_SERVER_DB_BACKEND")
    old_dsn = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN")
    old_content_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA")
    old_control_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA")
    restore_real_psycopg()
    os.environ["AIT_NATIVE_SERVER_DATA"] = str(data_dir)
    os.environ["AIT_NATIVE_AUTH_MODE"] = auth_mode
    os.environ["AIT_NATIVE_QUEUE_MODE"] = queue_mode
    os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = "postgres"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = dsn
    os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", None)
    os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", None)
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    host, port = sock.getsockname()
    sock.close()
    app_obj = create_app()
    config = uvicorn.Config(app_obj, host=host, port=port, log_level="error")
    server = uvicorn.Server(config)
    thread = threading.Thread(target=server.run, daemon=True)
    thread.start()
    base_url = f"http://{host}:{port}"
    deadline = time.time() + 10
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"{base_url}/healthz", timeout=0.5) as resp:
                if resp.status == 200:
                    break
        except Exception:
            time.sleep(0.05)
    else:
        server.should_exit = True
        thread.join(timeout=5)
        raise RuntimeError("native postgres test server did not start")
    try:
        yield base_url, data_dir
    finally:
        server.should_exit = True
        thread.join(timeout=5)
        restore_real_psycopg()
        if old_data is None:
            os.environ.pop("AIT_NATIVE_SERVER_DATA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DATA"] = old_data
        if old_mode is None:
            os.environ.pop("AIT_NATIVE_AUTH_MODE", None)
        else:
            os.environ["AIT_NATIVE_AUTH_MODE"] = old_mode
        if old_queue is None:
            os.environ.pop("AIT_NATIVE_QUEUE_MODE", None)
        else:
            os.environ["AIT_NATIVE_QUEUE_MODE"] = old_queue
        if old_backend is None:
            os.environ.pop("AIT_NATIVE_SERVER_DB_BACKEND", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = old_backend
        if old_dsn is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_DSN", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = old_dsn
        if old_content_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA"] = old_content_schema
        if old_control_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA"] = old_control_schema


def _bootstrap_main(repo: Path, base_url: str, monkeypatch) -> None:
    monkeypatch.chdir(repo)
    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
    main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
    assert main_snap_out.exit_code == 0, main_snap_out.stdout
    assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0



def _drain_jobs(data_dir: Path, limit: int = 20, *, backend: str = "postgres", postgres_dsn: str | None = None) -> int:
    if postgres_dsn is None:
        ctx = fake_postgres_context(data_dir)
    else:
        ctx = ServerContext.create(data_dir, backend=backend, postgres_dsn=postgres_dsn)
    initialize(ctx)
    processed = 0
    for _ in range(limit):
        job = process_one(ctx, worker_id="test-worker")
        if job is None:
            break
        processed += 1
    return processed


def _stale_timestamp(seconds_ago: int) -> str:
    return (datetime.now(timezone.utc) - timedelta(seconds=seconds_ago)).isoformat()


def _json_get(base_url: str, path: str) -> dict[str, Any]:
    return json.loads(urllib.request.urlopen(f"{base_url}{path}").read().decode("utf-8"))


def _pressure_payload(payload: dict[str, Any], *, source: str) -> dict[str, Any]:
    pressure = payload.get("live_turn_pressure")
    if not isinstance(pressure, dict):
        pytest.xfail(f"{source} does not expose `live_turn_pressure` until the operator pressure source slice lands.")
    return pressure


def _cache_metadata(payload: dict[str, Any], *, source: str, require_state: bool = True) -> tuple[str | None, float, int]:
    state = payload.get("cache_state")
    age = payload.get("cache_age_seconds")
    ttl = payload.get("cache_ttl_seconds")
    if age is None or ttl is None or (require_state and state is None):
        cache = payload.get("cache")
        if isinstance(cache, dict):
            if state is None:
                state = cache.get("cache_state")
            if age is None:
                age = cache.get("cache_age_seconds")
            if ttl is None:
                ttl = cache.get("cache_ttl_seconds")
    if age is None or ttl is None or (require_state and state is None):
        pytest.xfail(f"{source} does not expose cached admin-read metadata until the operator pressure source slice lands.")
    return (str(state) if state is not None else None), float(age), int(ttl)


def _assert_live_turn_pressure_shape(pressure: dict[str, Any]) -> None:
    for field in ("in_flight_turns", "queued_turns"):
        assert isinstance(pressure.get(field), int)
        assert int(pressure[field]) >= 0
    for field in ("oldest_in_flight_turn_age_seconds", "oldest_queued_turn_age_seconds"):
        value = pressure.get(field)
        assert value is None or float(value) >= 0
    assert str(pressure.get("pressure_state") or "") in {"idle", "ok", "busy", "saturated"}


def test_m4c_async_job_contract_defines_supported_jobs_and_normalizes_payloads():
    contract = async_job_contract()
    by_type = {row["job_type"]: row for row in contract}
    assert sorted(by_type) == [
        "content.gc",
        "content.optimize",
        "content.pack",
        "land.process",
        "patchset.ci",
        "policy.evaluate",
        "reconcile.repo",
        "repo.ci",
    ]
    assert by_type["repo.ci"]["required"] == {"repo_name": "str"}
    assert by_type["repo.ci"]["optional"]["suite_ids"] == {"type": "str_list_or_none", "default": None}
    assert by_type["repo.ci"]["optional"]["selector"] == {"type": "str_or_none", "default": None}
    assert by_type["repo.ci"]["optional"]["task_ids"] == {"type": "str_list_or_none", "default": None}
    assert by_type["repo.ci"]["optional"]["curated_corpus"] == {"type": "str_or_none", "default": None}
    assert by_type["repo.ci"]["optional"]["count"] == {"type": "positive_int_or_none", "default": None}
    assert by_type["repo.ci"]["optional"]["dependency_evidence"] == {"type": "str_list_or_none", "default": None}
    assert by_type["repo.ci"]["optional"]["compliance_evidence"] == {"type": "str_list_or_none", "default": None}
    assert by_type["patchset.ci"]["required"] == {"patchset_id": "str"}
    assert by_type["patchset.ci"]["optional"]["patchset_number"] == {"type": "positive_int_or_none", "default": None}
    assert by_type["policy.evaluate"]["required"] == {"patchset_id": "str"}
    assert by_type["policy.evaluate"]["optional"]["repo_name"] == {"type": "str_or_none", "default": None}
    assert by_type["policy.evaluate"]["optional"]["change_seq"] == {"type": "positive_int_or_none", "default": None}
    assert by_type["policy.evaluate"]["max_attempts"] == 5
    assert by_type["land.process"]["optional"]["land_seq"] == {"type": "positive_int_or_none", "default": None}
    assert by_type["content.pack"]["max_attempts"] == 3

    policy_payload = normalize_async_job_payload(
        "policy.evaluate",
        {
            "patchset_id": "AAAP-0001-1",
            "repo_name": "repo-a",
            "repo_id": "REPO-A",
            "change_id": "AAAC-0001",
            "change_seq": "1",
            "patchset_number": 1,
        },
    )
    assert policy_payload == {
        "patchset_id": "AAAP-0001-1",
        "repo_name": "repo-a",
        "repo_id": "REPO-A",
        "change_id": "AAAC-0001",
        "change_seq": 1,
        "patchset_number": 1,
    }

    ci_payload = normalize_async_job_payload(
        "patchset.ci",
        {
            "patchset_id": "AAAP-0001-1",
            "repo_name": "repo-a",
            "repo_id": "REPO-A",
            "change_id": "AAAC-0001",
            "change_seq": "1",
            "patchset_number": "2",
        },
    )
    assert ci_payload == {
        "patchset_id": "AAAP-0001-1",
        "repo_name": "repo-a",
        "repo_id": "REPO-A",
        "change_id": "AAAC-0001",
        "change_seq": 1,
        "patchset_number": 2,
    }

    repo_ci_payload = normalize_async_job_payload(
        "repo.ci",
        {
            "repo_name": "repo-a",
            "repo_id": "REPO-A",
            "suite_ids": ["full_repo", "release_readiness"],
            "plane": "nightly",
            "target_line": "main",
            "trigger": "manual_rerun",
            "selector": "explicit_task_ids",
            "task_ids": ["T-1", "T-2"],
            "curated_corpus": "demo",
            "count": "2",
            "window_days": 14,
            "dependency_evidence": ["dependency_report"],
            "compliance_evidence": ["compliance_report"],
        },
    )
    assert repo_ci_payload == {
        "repo_name": "repo-a",
        "repo_id": "REPO-A",
        "suite_ids": ["full_repo", "release_readiness"],
        "plane": "nightly",
        "target_line": "main",
        "trigger": "manual_rerun",
        "selector": "explicit_task_ids",
        "task_ids": ["T-1", "T-2"],
        "curated_corpus": "demo",
        "count": 2,
        "window_days": 14,
        "dependency_evidence": ["dependency_report"],
        "compliance_evidence": ["compliance_report"],
    }

    pack_payload = normalize_async_job_payload(
        "content.pack",
        {"repo_name": "housekeeper", "repack": 0, "max_members": "25"},
    )
    assert pack_payload == {
        "repo_name": "housekeeper",
        "repack": False,
        "max_members": 25,
    }

    gc_payload = normalize_async_job_payload("content.gc", {"repo_name": "housekeeper", "prune_unreferenced": "false"})
    assert gc_payload["prune_unreferenced"] is False
    assert gc_payload["prune_orphan_packs"] is True

    land_payload = normalize_async_job_payload(
        "land.process",
        {"submission_id": "LAND-AAAC-0001-0001", "repo_name": "repo-a", "land_seq": "1"},
    )
    assert land_payload["repo_name"] == "repo-a"
    assert land_payload["land_seq"] == 1

    with pytest.raises(ValueError, match="requires payload field `patchset_id`"):
        normalize_async_job_payload("policy.evaluate", {})
    with pytest.raises(ValueError, match="unsupported field"):
        normalize_async_job_payload("content.gc", {"repo_name": "housekeeper", "unknown": True})
    with pytest.raises(ValueError, match="greater than zero"):
        normalize_async_job_payload("content.pack", {"repo_name": "housekeeper", "max_members": 0})
    with pytest.raises(ValueError, match="greater than zero"):
        normalize_async_job_payload("policy.evaluate", {"patchset_id": "AAAP-0001-1", "patchset_number": 0})
    with pytest.raises(ValueError, match="Unsupported async job type"):
        normalize_async_job_payload("demo.job", {"value": 1})


def test_m4c_enqueue_async_job_applies_contract_defaults_and_dedupes(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "job-contract")
    initialize(ctx)

    first = enqueue_async_job(
        ctx,
        "housekeeper",
        "content.pack",
        {"repo_name": "housekeeper"},
        dedupe_active=True,
    )
    duplicate = enqueue_async_job(
        ctx,
        "housekeeper",
        "content.pack",
        {"repo_name": "housekeeper", "repack": False, "max_members": None},
        dedupe_active=True,
    )

    assert int(first["job_id"]) == int(duplicate["job_id"])
    assert first["max_attempts"] == 3
    assert first["payload"] == {
        "repo_name": "housekeeper",
        "repack": False,
        "max_members": None,
    }


def test_m4c_worker_fails_malformed_known_job_payload_without_retry_loop(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "job-contract-invalid")
    initialize(ctx)
    job = enqueue_job(ctx, "housekeeper", "policy.evaluate", {}, max_attempts=5)

    processed = process_one(ctx, worker_id="contract-worker")

    assert processed is not None
    assert processed["job_id"] == job["job_id"]
    assert processed["state"] == "failed"
    assert processed["attempt_count"] == 1
    assert "patchset_id" in (processed["last_error"] or "")





def test_m4d_job_diagnostics_reports_recovery_actions(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "job-diagnostics")
    initialize(ctx)

    stale = enqueue_job(ctx, "housekeeper", "content.optimize", {"repo_name": "housekeeper", "repair": True}, max_attempts=3)
    delayed = enqueue_job(ctx, "housekeeper", "content.gc", {"repo_name": "housekeeper"}, max_attempts=3)
    exhausted = enqueue_job(ctx, "housekeeper", "content.pack", {"repo_name": "housekeeper"}, max_attempts=1)

    stale_claim = claim_next_job(ctx, "stale-worker", repo_name="housekeeper")
    assert stale_claim is not None
    retry_claim = claim_next_job(ctx, "retry-worker", repo_name="housekeeper")
    assert retry_claim is not None
    failed_claim = claim_next_job(ctx, "failed-worker", repo_name="housekeeper")
    assert failed_claim is not None

    conn = connect_control(ctx)
    conn.execute(
        """
        update jobs
        set locked_at = ?, locked_by = ?, updated_at = ?
        where job_id = ?
        """,
        (_stale_timestamp(600), "stale-worker", _stale_timestamp(600), int(stale["job_id"])),
    )
    conn.commit()
    conn.close()

    retry_job = fail_job(ctx, int(delayed["job_id"]), "transient worker failure", retryable=True, retry_delay_seconds=60)
    failed_job = fail_job(ctx, int(exhausted["job_id"]), "permanent worker failure", retryable=True, retry_delay_seconds=60)

    diagnostics = job_diagnostics(ctx, "housekeeper", stale_after_seconds=30)

    assert diagnostics["recommended_action"] == "reclaim_stale"
    assert diagnostics["stale_running_jobs"] == 1
    assert diagnostics["stale_job_ids"] == [int(stale["job_id"])]
    assert diagnostics["delayed_retry_jobs"] == 1
    assert diagnostics["delayed_retry_job_ids"] == [int(delayed["job_id"])]
    assert diagnostics["exhausted_jobs"] == 1
    assert int(exhausted["job_id"]) in diagnostics["exhausted_job_ids"]
    assert diagnostics["failed_jobs"] == 1
    assert failed_job["state"] == "failed"

    delayed_row = next(job for job in diagnostics["recent_jobs"] if int(job["job_id"]) == int(delayed["job_id"]))
    assert delayed_row["attempts_remaining"] == 2
    assert delayed_row["retry_pending"] is True
    assert delayed_row["next_retry_at"] == retry_job["available_at"]

    reclaim = reclaim_stale_jobs(ctx, 30, repo_name="housekeeper")
    assert reclaim["stale_count"] == 1
    assert reclaim["requeued_job_ids"] == [int(stale["job_id"])]
    assert reclaim["reclaimed_jobs"][0]["action"] == "requeued"


def test_m4g_server_metrics_roll_up_storage_workers_and_job_outcomes(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-metrics")
    initialize(ctx)
    ensure_repository(ctx, "housekeeper", "main")
    ensure_repository(ctx, "ops", "main")

    stale = enqueue_job(ctx, "housekeeper", "content.optimize", {"repo_name": "housekeeper", "repair": True}, max_attempts=3)
    delayed = enqueue_job(ctx, "housekeeper", "content.gc", {"repo_name": "housekeeper"}, max_attempts=3)
    failed = enqueue_job(ctx, "ops", "reconcile.repo", {"repo_name": "ops", "repair": False}, max_attempts=1)

    stale_claim = claim_next_job(ctx, "metrics-worker", repo_name="housekeeper")
    retry_claim = claim_next_job(ctx, "retry-worker", repo_name="housekeeper")
    failed_claim = claim_next_job(ctx, "failed-worker", repo_name="ops")
    assert stale_claim is not None
    assert retry_claim is not None
    assert failed_claim is not None

    conn = connect_control(ctx)
    conn.execute(
        "update jobs set locked_at = ?, locked_by = ?, updated_at = ? where job_id = ?",
        (_stale_timestamp(600), "metrics-worker", _stale_timestamp(600), int(stale["job_id"])),
    )
    conn.commit()
    conn.close()

    fail_job(ctx, int(delayed["job_id"]), "retry later", retryable=True, retry_delay_seconds=60)
    fail_job(ctx, int(failed["job_id"]), "permanent failure", retryable=True, retry_delay_seconds=60)

    payload = server_metrics(ctx, recent_jobs_limit=10, stale_after_seconds=30)

    assert payload["summary"]["repo_count"] == 2
    assert payload["storage_metrics"]["repo_count"] == 2
    assert payload["storage_metrics"]["total_lines"] == 2
    assert payload["worker_metrics"]["active_worker_count"] == 1
    assert payload["worker_metrics"]["running_jobs"] == 1
    assert payload["worker_metrics"]["stale_running_jobs"] == 1
    assert payload["worker_metrics"]["delayed_retry_jobs"] == 1
    assert payload["worker_metrics"]["exhausted_jobs"] == 1
    assert payload["job_outcome_metrics"]["total_jobs"] == 3
    assert payload["job_outcome_metrics"]["state_summary"]["running"] == 1
    assert payload["job_outcome_metrics"]["state_summary"]["queued"] == 1
    assert payload["job_outcome_metrics"]["state_summary"]["failed"] == 1
    assert payload["job_outcome_metrics"]["job_type_summary"]["content.optimize"] == 1
    assert payload["job_outcome_metrics"]["recommended_action"] == "reclaim_stale"
    assert payload["summary"]["recommended_action"] == "reclaim_stale"
    assert {repo["repo_name"] for repo in payload["repositories"]} == {"housekeeper", "ops"}


def test_m4h_server_readiness_preflight_reports_runtime_storage_and_jobs(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-readiness")
    initialize(ctx)
    ensure_repository(ctx, "housekeeper", "main")

    healthy = server_readiness(ctx, recent_jobs_limit=5, stale_after_seconds=30)

    assert healthy["ready"] is True
    assert healthy["runtime"]["db_backend"] == "postgres"
    assert healthy["runtime"]["using_postgres"] is True
    assert healthy["runtime"]["sqlite_local_default_preserved"] is True
    assert healthy["recommended_action"] == "none"
    assert {check["name"]: check["status"] for check in healthy["checks"]} == {
        "server_health": "pass",
        "shared_runtime_policy": "pass",
        "storage_integrity": "pass",
        "job_recovery": "pass",
        "postgres_schema_versions": "pass",
    }
    assert healthy["postgres_schema"]["ok"] is True

    stale = enqueue_job(ctx, "housekeeper", "content.optimize", {"repo_name": "housekeeper", "repair": True}, max_attempts=3)
    claimed = claim_next_job(ctx, "readiness-worker", repo_name="housekeeper")
    assert claimed is not None
    conn = connect_control(ctx)
    conn.execute(
        "update jobs set locked_at = ?, locked_by = ?, updated_at = ? where job_id = ?",
        (_stale_timestamp(600), "readiness-worker", _stale_timestamp(600), int(stale["job_id"])),
    )
    conn.commit()
    conn.close()

    attention = server_readiness(ctx, recent_jobs_limit=5, stale_after_seconds=30)
    checks = {check["name"]: check for check in attention["checks"]}
    assert attention["ready"] is False
    assert attention["recommended_action"] == "reclaim_stale"
    assert checks["job_recovery"]["status"] == "fail"
    assert attention["job_summary"]["stale_running_jobs"] == 1


def test_server_readiness_accepts_postgres_shared_deployment_even_with_legacy_override_env(tmp_path: Path, monkeypatch):
    monkeypatch.setenv("AIT_NATIVE_SHARED_DEPLOYMENT", "1")
    monkeypatch.setenv("AIT_NATIVE_ALLOW_SQLITE_SHARED_DEPLOYMENT", "1")
    monkeypatch.setenv("AIT_NATIVE_SERVER_HOST", "0.0.0.0")

    ctx = fake_postgres_context(tmp_path / "server-readiness-shared-override")
    initialize(ctx)
    ensure_repository(ctx, "housekeeper", "main")

    readiness = server_readiness(ctx, recent_jobs_limit=5, stale_after_seconds=30)
    checks = {check["name"]: check for check in readiness["checks"]}

    assert readiness["ready"] is True
    assert readiness["recommended_action"] == "none"
    assert readiness["runtime"]["shared_runtime_policy"]["state"] == "postgres_compliant"
    assert readiness["runtime"]["shared_runtime_policy"]["deployment_scope"] == "shared"
    assert checks["shared_runtime_policy"]["status"] == "pass"


def test_operator_read_models_stage_live_turn_pressure_and_cache_metadata(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-pressure-shape")
    initialize(ctx)
    ensure_repository(ctx, "housekeeper", "main")

    metrics = server_metrics(ctx, recent_jobs_limit=5, stale_after_seconds=30)
    readiness = server_readiness(ctx, recent_jobs_limit=5, stale_after_seconds=30)

    metrics_pressure = _pressure_payload(metrics, source="server_metrics")
    readiness_pressure = _pressure_payload(readiness, source="server_readiness")
    metrics_cache_state, metrics_cache_age, metrics_cache_ttl = _cache_metadata(metrics, source="server_metrics")
    readiness_cache_state, readiness_cache_age, readiness_cache_ttl = _cache_metadata(readiness, source="server_readiness")

    _assert_live_turn_pressure_shape(metrics_pressure)
    _assert_live_turn_pressure_shape(readiness_pressure)
    assert readiness_pressure["in_flight_turns"] == metrics_pressure["in_flight_turns"]
    assert readiness_pressure["queued_turns"] == metrics_pressure["queued_turns"]
    assert readiness_pressure["pressure_state"] == metrics_pressure["pressure_state"]
    assert metrics_cache_state in {"computed", "cached"}
    assert readiness_cache_state in {"computed", "cached"}
    assert metrics_cache_age >= 0
    assert readiness_cache_age >= 0
    assert metrics_cache_ttl >= 0
    assert readiness_cache_ttl >= 0


def test_async_policy_and_land_jobs_run_through_worker(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-async", queue_mode="async") as (base_url, data_dir):
        _bootstrap_main(repo, base_url, monkeypatch)

        task = _create_plan_bound_task(
            repo,
            title="async demo",
            intent="demo",
            risk="medium",
            slug="async-demo",
        )
        worktree_path = _ensure_task_worktree(repo, task["task_id"])
        monkeypatch.chdir(worktree_path)

        assert runner.invoke(app, ["line", "create", "feature/async"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/async"], catch_exceptions=False).exit_code == 0
        (worktree_path / "app.py").write_text("print('async')\n", encoding="utf-8")
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature async", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Async change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "queued patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert "policy_job" not in patchset
        assert patchset["policy_followup"]["state"] == "deferred"
        assert patchset["policy_followup"]["queue_mode"] == "async"
        assert patchset["publish_followup"]["phase_outcomes"]["policy_followup"] == "deferred"
        assert patchset["publish_followup"]["request_path_audit"] == [
            {
                "phase": "publish_patchset",
                "state": "completed",
                "seconds": patchset["publish_followup"]["timings"]["publish_patchset_seconds"],
                "required_for_immediate_correctness": True,
                "deferred_safe": False,
                "reason": "Patchset identity, revision selection, and workflow-land patchset readiness depend on this persistence step.",
            },
            {
                "phase": "ci_followup",
                "state": "not_applicable",
                "seconds": patchset["publish_followup"]["timings"]["ci_followup_seconds"],
                "required_for_immediate_correctness": False,
                "deferred_safe": True,
                "reason": "Patchset CI evidence is required later for land gates, but patchset publication stays correct when CI is queued or deferred off the request path.",
            },
            {
                "phase": "policy_followup",
                "state": "deferred",
                "seconds": patchset["publish_followup"]["timings"]["policy_followup_seconds"],
                "required_for_immediate_correctness": False,
                "deferred_safe": True,
                "reason": "Policy may wait for attestation, review, selection, or waiver evidence without changing patchset identity or next-action correctness.",
            },
            {
                "phase": "notification_followup",
                "state": "background",
                "seconds": patchset["publish_followup"]["timings"]["notification_followup_seconds"],
                "required_for_immediate_correctness": False,
                "deferred_safe": True,
                "reason": "Notification scheduling is observability-only and does not affect patchset publication correctness.",
            },
        ]

        jobs_out = runner.invoke(app, ["repo", "jobs", "--json"], catch_exceptions=False)
        assert jobs_out.exit_code == 0, jobs_out.stdout
        jobs = json.loads(jobs_out.stdout)
        assert not any(job["job_type"] == "policy.evaluate" and job["state"] == "queued" for job in jobs)

        attest_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--lint", "pass", "--security", "pass", "--license", "pass", "--json"],
            catch_exceptions=False,
        )
        assert attest_out.exit_code == 0, attest_out.stdout
        attestation = json.loads(attest_out.stdout)
        initial_policy_job_id = int(attestation["policy_job"]["job_id"])

        approve_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout
        approval = json.loads(approve_out.stdout)
        assert int(approval["policy_job"]["job_id"]) == initial_policy_job_id
        code_review_out = runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                change["change_id"],
                "--reviewer",
                "codex@example.com",
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--message",
                WORKER_CODE_REVIEW_SUMMARY,
                "--json",
            ],
            catch_exceptions=False,
        )
        assert code_review_out.exit_code == 0, code_review_out.stdout
        code_review = json.loads(code_review_out.stdout)
        assert int(code_review["policy_job"]["job_id"]) == initial_policy_job_id

        jobs_out = runner.invoke(app, ["repo", "jobs", "--json"], catch_exceptions=False)
        assert jobs_out.exit_code == 0, jobs_out.stdout
        jobs = json.loads(jobs_out.stdout)
        policy_jobs = [job for job in jobs if job["job_type"] == "policy.evaluate" and job["state"] == "queued"]
        assert len(policy_jobs) == 1
        assert int(policy_jobs[0]["job_id"]) == initial_policy_job_id

        processed = _drain_jobs(data_dir)
        assert processed >= 1

        policy_out = runner.invoke(app, ["policy", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        policy = json.loads(policy_out.stdout)
        assert policy["decision"] == "pass"

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        land = json.loads(land_out.stdout)
        assert land["status"] == "queued"
        assert "land_job" in land

        queued_show = runner.invoke(app, ["land", "show", land["submission_id"], "--json"], catch_exceptions=False)
        assert queued_show.exit_code == 0, queued_show.stdout
        assert json.loads(queued_show.stdout)["status"] == "queued"

        processed = _drain_jobs(data_dir)
        assert processed >= 1

        landed_show = runner.invoke(app, ["land", "show", land["submission_id"], "--json"], catch_exceptions=False)
        assert landed_show.exit_code == 0, landed_show.stdout
        landed = json.loads(landed_show.stdout)
        assert landed["status"] == "succeeded"

        pull_main = runner.invoke(app, ["pull", "--line", "main", "--json"], catch_exceptions=False)
        assert pull_main.exit_code == 0, pull_main.stdout
        main_line = runner.invoke(app, ["line", "show", "main", "--json"], catch_exceptions=False)
        assert main_line.exit_code == 0, main_line.stdout
        line = json.loads(main_line.stdout)
        assert line["head_snapshot_id"] == landed["result"]["landed_snapshot_id"]
        assert landed["result"]["freshness_preflight"]["target_matches_revision_tree"] is True


def _write_patchset_ci_contract(
    repo: Path,
    *,
    preflight_command: str = "python3 -c \"print('preflight ok')\"",
    include_tg1: bool = False,
    tg1_command: str | None = None,
) -> None:
    suite_dir = repo / "ci" / "suites"
    suite_dir.mkdir(parents=True, exist_ok=True)
    suite_dir.joinpath("preflight.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "preflight",
                "display_name": "Preflight",
                "plane": "patchset",
                "default_blocking": True,
                "mode": "gate",
                "purpose": "minimal preflight",
                "runner": {"kind": "command_bundle", "commands": [preflight_command]},
                "artifacts": {"log_path": ".ait/generated/ci/preflight.log"},
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("stable_smoke.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "stable_smoke",
                "display_name": "Stable Smoke",
                "plane": "patchset",
                "default_blocking": True,
                "mode": "gate",
                "purpose": "minimal smoke",
                "runner": {"kind": "pytest", "args": ["tests/test_patchset_ci_smoke.py", "-q"]},
                "artifacts": {"junit_xml": ".ait/generated/ci/stable_smoke.junit.xml"},
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("package_smoke.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "package_smoke",
                "display_name": "Package Smoke",
                "plane": "patchset",
                "default_blocking": True,
                "mode": "gate",
                "purpose": "minimal package smoke",
                "runner": {"kind": "command_bundle", "commands": ["python3 -c \"print('package ok')\""]},
                "artifacts": {"log_path": ".ait/generated/ci/package_smoke.log"},
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("recent_regression.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "recent_regression",
                "display_name": "Recent Regression",
                "plane": "patchset",
                "default_blocking": False,
                "mode": "diagnostic",
                "purpose": "minimal informational regression lane",
                "runner": {"kind": "command_bundle", "commands": ["python3 -c \"print('recent regression known red')\""]},
                "artifacts": {"log_path": ".ait/generated/ci/recent_regression.log"},
            }
        ),
        encoding="utf-8",
    )
    if include_tg1:
        suite_dir.joinpath("tg1_required.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "suite_id": "tg1_required",
                    "display_name": "TG-1 Required",
                    "plane": "patchset",
                    "default_blocking": True,
                    "mode": "gate",
                    "purpose": "test TG-1 summary exposure",
                    "runner": {
                        "kind": "command_bundle",
                        "commands": [tg1_command or "python3 -c \"print('tg1 ok')\""],
                    },
                    "artifacts": {
                        "summary_json": ".ait/generated/ci/tg1_required.json",
                        "log_path": ".ait/generated/ci/tg1_required.log",
                    },
                }
            ),
            encoding="utf-8",
        )
    tests_dir = repo / "tests"
    tests_dir.mkdir(parents=True, exist_ok=True)
    tests_dir.joinpath("test_patchset_ci_smoke.py").write_text(
        "def test_patchset_ci_smoke():\n    assert True\n",
        encoding="utf-8",
    )


def _write_repo_ci_contract(repo: Path) -> None:
    suite_dir = repo / "ci" / "suites"
    suite_dir.mkdir(parents=True, exist_ok=True)
    suite_dir.joinpath("full_repo.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "full_repo",
                "display_name": "Full Repo",
                "plane": "nightly",
                "default_blocking": False,
                "mode": "diagnostic",
                "purpose": "Minimal repo-wide diagnostic suite.",
                "runner": {"kind": "pytest", "args": ["tests/test_repo_ci_smoke.py", "-q"]},
                "triage": {
                    "ownership_rules": [{"test_path_prefix": "tests/", "owner": "repo-ci"}],
                    "suspect_task_selector": "recent_remote_landed",
                },
                "artifacts": {
                    "junit_xml": ".ait/generated/ci/full_repo.junit.xml",
                    "log_path": ".ait/generated/ci/full_repo.log",
                },
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("postgres_preview.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "postgres_preview",
                "display_name": "Postgres Preview",
                "plane": "nightly",
                "default_blocking": False,
                "mode": "diagnostic",
                "purpose": "Minimal postgres preview smoke.",
                "runner": {"kind": "command_bundle", "commands": ["python3 -c \"print('postgres preview ok')\""]},
                "artifacts": {"log_path": ".ait/generated/ci/postgres_preview.log"},
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("release_readiness.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "release_readiness",
                "display_name": "Release Readiness",
                "plane": "nightly",
                "default_blocking": False,
                "mode": "diagnostic",
                "purpose": "Minimal release-readiness smoke.",
                "runner": {"kind": "pytest", "args": ["tests/test_repo_ci_smoke.py", "-q"]},
                "artifacts": {
                    "junit_xml": ".ait/generated/ci/release_readiness.junit.xml",
                    "log_path": ".ait/generated/ci/release_readiness.log",
                },
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("release_readiness_xdist_preview.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "release_readiness_xdist_preview",
                "display_name": "Release Readiness xdist Preview",
                "plane": "post_land_regression",
                "default_blocking": False,
                "mode": "diagnostic",
                "purpose": "Minimal release-readiness two-process preview.",
                "runner": {"kind": "command_bundle", "commands": ["python3 -c \"print('release readiness xdist preview ok')\""]},
                "artifacts": {
                    "log_path": ".ait/generated/ci/release_readiness_xdist_preview.log"
                },
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("safe_parallel_xdist_preview.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "safe_parallel_xdist_preview",
                "display_name": "Safe Parallel xdist Preview",
                "plane": "post_land_regression",
                "default_blocking": False,
                "mode": "diagnostic",
                "purpose": "Minimal eight-worker whitelist preview.",
                "runner": {"kind": "pytest", "args": ["tests/test_repo_ci_config_contract.py", "-q"]},
                "artifacts": {
                    "junit_xml": ".ait/generated/ci/safe_parallel_xdist_preview.junit.xml",
                    "log_path": ".ait/generated/ci/safe_parallel_xdist_preview.log"
                },
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("task_batch.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "task_batch",
                "display_name": "Task Batch",
                "plane": "post_land_regression",
                "default_blocking": False,
                "mode": "diagnostic",
                "purpose": "Minimal task-batch diagnostic suite.",
                "runner": {
                    "kind": "task_batch",
                    "supported_selectors": [
                        "recent_remote_landed",
                        "recent_remote_landed_high_risk",
                        "explicit_task_ids",
                        "curated_corpus",
                    ],
                    "audit_first": True,
                    "behavior_suite_ids": ["task_batch_focus"],
                    "curated_corpus_dir": "ci/task_corpora",
                },
                "defaults": {
                    "selector": "recent_remote_landed",
                    "count": 2,
                    "window_days": 7,
                    "remote": "origin",
                    "target_line": "main",
                    "require_land_status": "succeeded",
                    "blocking": False,
                    "max_parallel": 2,
                    "audit_first": True,
                    "include_lineage_representatives": True,
                },
                "artifacts": {
                    "summary_json": ".ait/generated/ci/task_batch_summary.json",
                    "summary_markdown": ".ait/generated/ci/task_batch_summary.md",
                },
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("task_batch_focus.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "task_batch_focus",
                "display_name": "Task Batch Focus",
                "plane": "task_batch_behavior",
                "default_blocking": False,
                "mode": "diagnostic",
                "purpose": "Minimal focused task-batch regression suite.",
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["python3 -c \"import sys; print('task batch focus failed'); sys.exit(1)\""],
                },
                "artifacts": {"log_path": ".ait/generated/ci/task_batch_focus.log"},
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("release_artifact_smoke.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "release_artifact_smoke",
                "display_name": "Release Artifact Smoke",
                "plane": "release",
                "default_blocking": False,
                "mode": "gate",
                "purpose": "Minimal release artifact smoke.",
                "runner": {"kind": "command_bundle", "commands": ["python3 -c \"print('release smoke ok')\""]},
                "release_gate_evidence": {
                    "required_before_distribution": False,
                    "dependency_keys": ["dependency_report", "dependency_review"],
                    "compliance_keys": ["compliance_report", "license_exception_review"],
                },
                "artifacts": {"log_path": ".ait/generated/ci/release_artifact_smoke.log"},
            }
        ),
        encoding="utf-8",
    )
    corpus_dir = repo / "ci" / "task_corpora"
    corpus_dir.mkdir(parents=True, exist_ok=True)
    corpus_dir.joinpath("demo.json").write_text(
        json.dumps(
            {
                "task_ids": ["T-TASK-BATCH-HIGH", "T-TASK-BATCH-SENTINEL"],
                "lineage_only_task_ids": ["T-TASK-BATCH-SENTINEL"],
                "behavior_suite_ids": ["task_batch_focus"],
            }
        ),
        encoding="utf-8",
    )
    tests_dir = repo / "tests"
    tests_dir.mkdir(parents=True, exist_ok=True)
    tests_dir.joinpath("test_repo_ci_smoke.py").write_text(
        "def test_repo_ci_smoke():\n    assert True\n",
        encoding="utf-8",
    )
    tests_dir.joinpath("test_repo_ci_config_contract.py").write_text(
        "import json\n"
        "from pathlib import Path\n\n"
        "def test_repo_ci_materialized_runtime_preserves_ci_contract():\n"
        "    repo_root = Path(__file__).resolve().parents[1]\n"
        "    config = json.loads((repo_root / '.ait' / 'config.json').read_text(encoding='utf-8'))\n"
        "    ci = config['ci']\n"
        "    assert ci['required_patchset_suites'] == ['preflight', 'stable_smoke', 'package_smoke']\n"
        "    assert ci['nightly_suites'] == ['full_repo', 'postgres_preview', 'release_readiness']\n"
        "    assert ci['release_suites'] == ['release_artifact_smoke']\n",
        encoding="utf-8",
    )


def _write_checked_in_repo_ci_config_contract(repo: Path, ci: dict[str, object]) -> None:
    contract_path = repo / "ci" / "config.contract.json"
    contract_path.parent.mkdir(parents=True, exist_ok=True)
    contract_path.write_text(
        json.dumps({"schema_version": 1, "ci": ci}, indent=2) + "\n",
        encoding="utf-8",
    )


def _write_repo_ci_plane_config(repo: Path) -> None:
    config_path = repo / ".ait" / "config.json"
    config = json.loads(config_path.read_text(encoding="utf-8"))
    ci = dict(config.get("ci") or {})
    ci.update(
        {
            "required_patchset_suites": ["preflight", "stable_smoke", "package_smoke"],
            "informational_patchset_suites": ["recent_regression", "task_batch"],
            "nightly_suites": ["full_repo", "postgres_preview", "release_readiness"],
            "release_suites": ["release_artifact_smoke"],
        }
    )
    config["ci"] = ci
    config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    _write_checked_in_repo_ci_config_contract(repo, ci)


def _write_repo_ci_task_batch_config(repo: Path, *, count: int, selector: str = "recent_remote_landed") -> None:
    config_path = repo / ".ait" / "config.json"
    config = json.loads(config_path.read_text(encoding="utf-8"))
    ci = dict(config.get("ci") or {})
    ci["task_batch"] = {
        "selector": selector,
        "count": count,
        "window_days": 7,
        "remote": "origin",
        "target_line": "main",
        "require_land_status": "succeeded",
        "blocking": False,
        "max_parallel": 2,
        "audit_first": True,
        "include_lineage_representatives": True,
    }
    config["ci"] = ci
    config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    _write_checked_in_repo_ci_config_contract(repo, ci)



def _seed_task_batch_candidate(
    ctx: ServerContext,
    repo_name: str,
    repo_id: str,
    main_snapshot_id: str,
    *,
    task_id: str,
    task_seq: int,
    change_id: str,
    change_seq: int,
    patchset_id: str,
    risk_tier: str,
    landed_at: str,
    with_patchset: bool = True,
) -> None:
    conn = connect_control(ctx)
    try:
        conn.execute(
            """
            insert into tasks(task_id, repo_name, repo_id, task_seq, title, intent, risk_tier, planning_state, status, created_at)
            values (?, ?, ?, ?, ?, ?, ?, 'planned', 'completed', ?)
            """,
            (task_id, repo_name, repo_id, task_seq, f"seed {task_id}", "seed", risk_tier, landed_at),
        )
        conn.execute(
            """
            insert into changes(
                change_id, repo_name, repo_id, change_seq, task_id, title, base_line,
                risk_tier, lane, status, current_patchset_number, selected_patchset_number,
                created_at, updated_at, landed_at
            )
            values (?, ?, ?, ?, ?, ?, 'main', ?, 'assisted', 'landed', ?, ?, ?, ?, ?)
            """,
            (
                change_id,
                repo_name,
                repo_id,
                change_seq,
                task_id,
                f"seed {change_id}",
                risk_tier,
                1 if with_patchset else 0,
                1 if with_patchset else None,
                landed_at,
                landed_at,
                landed_at,
            ),
        )
        if with_patchset:
            conn.execute(
                """
                insert into patchsets(
                    patchset_id, repo_id, change_id, patchset_number, base_snapshot_id,
                    revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json,
                    evaluation_state, created_at
                )
                values (?, ?, ?, 1, ?, ?, ?, 'human', 'published', '{}', 'pending', ?)
                """,
                (patchset_id, repo_id, change_id, main_snapshot_id, main_snapshot_id, f"seed {patchset_id}", landed_at),
            )
        conn.execute(
            """
            insert into land_requests(
                submission_id, repo_id, land_seq, change_id, patchset_id, target_line,
                mode, status, result_json, created_at, updated_at
            )
            values (?, ?, ?, ?, ?, 'main', 'direct', 'succeeded', '{}', ?, ?)
            """,
            (f"LAND-{change_id}-0001", repo_id, task_seq, change_id, patchset_id, landed_at, landed_at),
        )
        conn.commit()
    finally:
        conn.close()


def test_patchset_ci_jobs_publish_select_and_rerun_with_attestation_writeback(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-ci"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _write_patchset_ci_contract(repo)

    with running_server(tmp_path / "server-async-ci", queue_mode="async") as (base_url, data_dir):
        _bootstrap_main(repo, base_url, monkeypatch)

        task = _create_plan_bound_task(
            repo,
            title="ci demo",
            intent="demo",
            risk="medium",
            slug="ci-demo",
        )
        worktree_path = _ensure_task_worktree(repo, task["task_id"])
        monkeypatch.chdir(worktree_path)

        assert runner.invoke(app, ["line", "create", "feature/ci"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/ci"], catch_exceptions=False).exit_code == 0
        (worktree_path / "app.py").write_text("print('ci')\n", encoding="utf-8")
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature ci", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "CI change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "ci patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert patchset["ci_job"]["job_type"] == "patchset.ci"

        jobs_out = runner.invoke(app, ["repo", "jobs", "--json"], catch_exceptions=False)
        jobs = json.loads(jobs_out.stdout)
        assert any(job["job_type"] == "patchset.ci" and job["state"] == "queued" for job in jobs)

        processed = _drain_jobs(data_dir)
        assert processed >= 2

        attestation_out = runner.invoke(app, ["attest", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert attestation_out.exit_code == 0, attestation_out.stdout
        attestation = json.loads(attestation_out.stdout)
        assert attestation["evaluation_summary"]["tests"] == "pass"
        ci_detail = attestation["detail"]["patchset_ci"]
        assert ci_detail["tests_status"] == "pass"
        assert set(ci_detail["selected_suite_ids"]) == {"preflight", "stable_smoke", "package_smoke"}
        assert all(item["status"] == "pass" for item in ci_detail["suite_results"])
        status_out = runner.invoke(app, ["patchset", "ci-status", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert status_out.exit_code == 0, status_out.stdout
        status_payload = json.loads(status_out.stdout)
        assert status_payload["tests_status"] == "pass"
        assert status_payload["latest_job"]["state"] == "succeeded"
        assert set(status_payload["selected_suite_ids"]) == {"preflight", "stable_smoke", "package_smoke"}
        assert status_payload["rerun"]["cli"] == f"ait patchset rerun-ci {patchset['patchset_id']}"

        policy_out = runner.invoke(app, ["policy", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pending"

        rerun_out = runner.invoke(app, ["patchset", "rerun-ci", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert rerun_out.exit_code == 0, rerun_out.stdout
        rerun = json.loads(rerun_out.stdout)
        assert rerun["queued"] is True
        assert rerun["job"]["job_type"] == "patchset.ci"
        assert _drain_jobs(data_dir) >= 1
        rerun_status_out = runner.invoke(app, ["patchset", "ci-status", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert rerun_status_out.exit_code == 0, rerun_status_out.stdout
        rerun_status = json.loads(rerun_status_out.stdout)
        latest_job = rerun_status["latest_job"]
        assert latest_job["state"] == "succeeded"

        (worktree_path / "app.py").write_text("print('ci second patchset')\n", encoding="utf-8")
        second_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature ci second", "--json"], catch_exceptions=False)
        assert second_snap_out.exit_code == 0, second_snap_out.stdout
        second_patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "ci patchset second", "--json"],
            catch_exceptions=False,
        )
        assert second_patchset_out.exit_code == 0, second_patchset_out.stdout
        second_patchset = json.loads(second_patchset_out.stdout)
        assert second_patchset["ci_job"]["job_type"] == "patchset.ci"

        select_out = runner.invoke(
            app,
            ["patchset", "select", patchset["patchset_id"], "--change", change["change_id"], "--json"],
            catch_exceptions=False,
        )
        assert select_out.exit_code == 0, select_out.stdout
        selected = json.loads(select_out.stdout)
        assert selected["selected_patchset_id"] == patchset["patchset_id"]
        assert "ci_job" not in selected
        assert "ci_result" not in selected
        assert selected["policy_job"]["job_type"] == "policy.evaluate"
        assert selected["notification_followup"]["delivery"] == "background"
        post_select_status_out = runner.invoke(app, ["patchset", "ci-status", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert post_select_status_out.exit_code == 0, post_select_status_out.stdout
        post_select_status = json.loads(post_select_status_out.stdout)
        assert post_select_status["latest_job"]["job_id"] == latest_job["job_id"]
        assert post_select_status["attestation_updated_at"] == rerun_status["attestation_updated_at"]


def test_patchset_publish_defers_inline_ci_until_explicit_run_ci(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-ci-inline"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _write_patchset_ci_contract(repo)

    with running_server(tmp_path / "server-inline-ci", queue_mode="inline") as (base_url, _data_dir):
        _bootstrap_main(repo, base_url, monkeypatch)

        task = _create_plan_bound_task(
            repo,
            title="ci inline demo",
            intent="exercise inline patchset ci",
            risk="medium",
            slug="ci-inline-demo",
        )
        worktree_path = _ensure_task_worktree(repo, task["task_id"])
        monkeypatch.chdir(worktree_path)

        assert runner.invoke(app, ["line", "create", "feature/ci-inline"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/ci-inline"], catch_exceptions=False).exit_code == 0
        (worktree_path / "app.py").write_text("print('ci inline')\n", encoding="utf-8")
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature ci inline", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Inline CI change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "inline ci patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert "ci_job" not in patchset
        assert "ci_result" not in patchset
        assert patchset["ci_followup"]["state"] == "deferred"
        assert patchset["ci_followup"]["command"] == f"ait patchset rerun-ci {patchset['patchset_id']}"
        assert patchset["policy_followup"]["state"] == "deferred"
        assert patchset["policy_followup"]["command"] == f"ait policy eval {patchset['patchset_id']}"
        assert patchset["notification_followup"]["delivery"] == "background"
        assert patchset["publish_followup"]["queue_mode"] == "inline"
        assert patchset["publish_followup"]["phase_outcomes"]["policy_followup"] == "deferred"
        assert patchset["publish_followup"]["request_path_audit"] == [
            {
                "phase": "publish_patchset",
                "state": "completed",
                "seconds": patchset["publish_followup"]["timings"]["publish_patchset_seconds"],
                "required_for_immediate_correctness": True,
                "deferred_safe": False,
                "reason": "Patchset identity, revision selection, and workflow-land patchset readiness depend on this persistence step.",
            },
            {
                "phase": "ci_followup",
                "state": "deferred",
                "seconds": patchset["publish_followup"]["timings"]["ci_followup_seconds"],
                "required_for_immediate_correctness": False,
                "deferred_safe": True,
                "reason": "Patchset CI evidence is required later for land gates, but patchset publication stays correct when CI is queued or deferred off the request path.",
            },
            {
                "phase": "policy_followup",
                "state": "deferred",
                "seconds": patchset["publish_followup"]["timings"]["policy_followup_seconds"],
                "required_for_immediate_correctness": False,
                "deferred_safe": True,
                "reason": "Policy may wait for attestation, review, selection, or waiver evidence without changing patchset identity or next-action correctness.",
            },
            {
                "phase": "notification_followup",
                "state": "background",
                "seconds": patchset["publish_followup"]["timings"]["notification_followup_seconds"],
                "required_for_immediate_correctness": False,
                "deferred_safe": True,
                "reason": "Notification scheduling is observability-only and does not affect patchset publication correctness.",
            },
        ]

        rerun_out = runner.invoke(app, ["patchset", "rerun-ci", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert rerun_out.exit_code == 0, rerun_out.stdout
        rerun = json.loads(rerun_out.stdout)
        assert rerun["tests_status"] == "pass"
        assert rerun["attestation"]["evaluation_summary"]["tests"] == "pass"
        rerun_status_out = runner.invoke(app, ["patchset", "ci-status", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert rerun_status_out.exit_code == 0, rerun_status_out.stdout
        rerun_status = json.loads(rerun_status_out.stdout)

        attestation_out = runner.invoke(app, ["attest", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert attestation_out.exit_code == 0, attestation_out.stdout
        attestation = json.loads(attestation_out.stdout)
        assert attestation["evaluation_summary"]["tests"] == "pass"
        assert set(attestation["detail"]["patchset_ci"]["selected_suite_ids"]) == {"preflight", "stable_smoke", "package_smoke"}

        (worktree_path / "app.py").write_text("print('ci inline second patchset')\n", encoding="utf-8")
        second_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature ci inline second", "--json"], catch_exceptions=False)
        assert second_snap_out.exit_code == 0, second_snap_out.stdout
        second_patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "inline ci patchset second", "--json"],
            catch_exceptions=False,
        )
        assert second_patchset_out.exit_code == 0, second_patchset_out.stdout

        select_out = runner.invoke(
            app,
            ["patchset", "select", patchset["patchset_id"], "--change", change["change_id"], "--json"],
            catch_exceptions=False,
        )
        assert select_out.exit_code == 0, select_out.stdout
        selected = json.loads(select_out.stdout)
        assert selected["selected_patchset_id"] == patchset["patchset_id"]
        assert "ci_job" not in selected
        assert "ci_result" not in selected
        assert "policy_job" not in selected
        assert selected["notification_followup"]["delivery"] == "background"

        post_select_status_out = runner.invoke(app, ["patchset", "ci-status", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert post_select_status_out.exit_code == 0, post_select_status_out.stdout
        post_select_status = json.loads(post_select_status_out.stdout)
        assert post_select_status["attestation_updated_at"] == rerun_status["attestation_updated_at"]


def test_patchset_ci_status_exposes_tg1_required_summary(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-ci-tg1-summary"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    tg1_command = (
        "python3 -c \"import json, pathlib; root = pathlib.Path('.ait/generated/ci'); "
        "root.mkdir(parents=True, exist_ok=True); "
        "(root / 'tg1_required.json').write_text(json.dumps({"
        "'status': 'pass', 'validation_status': 'pass', 'minimum_count': 24, 'live_count': 24, "
        "'pytest': {'status': 'pass'}}), encoding='utf-8')\""
    )
    _write_patchset_ci_contract(repo, include_tg1=True, tg1_command=tg1_command)

    with running_server(tmp_path / "server-async-ci-tg1-summary", queue_mode="async") as (base_url, data_dir):
        _bootstrap_main(repo, base_url, monkeypatch)

        task = _create_plan_bound_task(
            repo,
            title="ci tg1 summary demo",
            intent="expose tg1 summary in patchset ci status",
            risk="medium",
            slug="ci-tg1-summary-demo",
        )
        worktree_path = _ensure_task_worktree(repo, task["task_id"])
        monkeypatch.chdir(worktree_path)

        assert runner.invoke(app, ["line", "create", "feature/ci-tg1-summary"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/ci-tg1-summary"], catch_exceptions=False).exit_code == 0
        (worktree_path / "app.py").write_text("print('tg1 summary')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature ci tg1 summary", "--json"], catch_exceptions=False).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "CI TG-1 summary change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "ci tg1 summary patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert _drain_jobs(data_dir) >= 2

        status_out = runner.invoke(app, ["patchset", "ci-status", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert status_out.exit_code == 0, status_out.stdout
        status_payload = json.loads(status_out.stdout)
        assert "tg1_required" in status_payload["selected_suite_ids"]
        assert status_payload["tg1_required"]["status"] == "pass"
        assert status_payload["tg1_required"]["live_count"] == 24
        assert status_payload["tg1_required"]["minimum_count"] == 24

        attestation_out = runner.invoke(app, ["attest", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert attestation_out.exit_code == 0, attestation_out.stdout
        attestation = json.loads(attestation_out.stdout)
        tg1_suite = next(item for item in attestation["detail"]["patchset_ci"]["suite_results"] if item["suite_id"] == "tg1_required")
        assert tg1_suite["tg1_required_summary"]["live_count"] == 24
        assert tg1_suite["tg1_required_summary"]["minimum_count"] == 24


def test_patchset_ci_policy_rollout_surfaces_informational_suites_without_blocking(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-ci-rollout"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _write_patchset_ci_contract(repo)

    with running_server(tmp_path / "server-async-ci-rollout", queue_mode="async") as (base_url, data_dir):
        _bootstrap_main(repo, base_url, monkeypatch)

        task = _create_plan_bound_task(
            repo,
            title="ci rollout demo",
            intent="demo rollout",
            risk="medium",
            slug="ci-rollout-demo",
        )
        worktree_path = _ensure_task_worktree(repo, task["task_id"])
        monkeypatch.chdir(worktree_path)

        assert runner.invoke(app, ["line", "create", "feature/ci-rollout"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/ci-rollout"], catch_exceptions=False).exit_code == 0
        (worktree_path / "app.py").write_text("print('ci rollout')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature ci rollout", "--json"], catch_exceptions=False).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "CI rollout change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "ci rollout patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert _drain_jobs(data_dir) >= 2

        approve_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout
        code_review_out = runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                change["change_id"],
                "--reviewer",
                "codex@example.com",
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--message",
                WORKER_CODE_REVIEW_SUMMARY,
                "--json",
            ],
            catch_exceptions=False,
        )
        assert code_review_out.exit_code == 0, code_review_out.stdout
        assert _drain_jobs(data_dir) >= 1

        policy_out = runner.invoke(app, ["policy", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        policy = json.loads(policy_out.stdout)
        assert policy["decision"] == "pass"
        checks = {row["name"]: row for row in policy["checks"]}
        assert checks["ci_rollout_phase"]["status"] == "pass"
        assert checks["ci_patchset_suite_preflight"]["status"] == "pass"
        assert checks["ci_patchset_suite_stable_smoke"]["status"] == "pass"
        assert checks["ci_patchset_suite_package_smoke"]["status"] == "pass"
        assert checks["ci_patchset_suite_recent_regression"]["status"] == "not_required"
        assert "visible" in str(checks["ci_patchset_suite_recent_regression"]["message"])


def test_required_patchset_ci_failures_cannot_be_waived_and_keep_remote_land_blocked(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-ci-no-waive"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")
    _write_patchset_ci_contract(
        repo,
        preflight_command="python3 -c \"import sys; print('preflight fail'); sys.exit(7)\"",
    )

    with running_server(tmp_path / "server-async-ci-no-waive", queue_mode="async") as (base_url, data_dir):
        _bootstrap_main(repo, base_url, monkeypatch)

        task = _create_plan_bound_task(
            repo,
            title="ci no-waive demo",
            intent="prove failing required CI blocks remote land",
            risk="medium",
            slug="ci-no-waive-demo",
        )
        worktree_path = _ensure_task_worktree(repo, task["task_id"])
        monkeypatch.chdir(worktree_path)

        assert runner.invoke(app, ["line", "create", "feature/ci-no-waive"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/ci-no-waive"], catch_exceptions=False).exit_code == 0
        (worktree_path / "app.py").write_text("print('ci no waive')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature ci no waive", "--json"], catch_exceptions=False).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "CI no-waive change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "ci no-waive patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert _drain_jobs(data_dir) >= 2

        approve_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout
        code_review_out = runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                change["change_id"],
                "--reviewer",
                "codex@example.com",
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--message",
                WORKER_CODE_REVIEW_SUMMARY,
                "--json",
            ],
            catch_exceptions=False,
        )
        assert code_review_out.exit_code == 0, code_review_out.stdout
        assert _drain_jobs(data_dir) >= 1

        policy_out = runner.invoke(app, ["policy", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        policy = json.loads(policy_out.stdout)
        checks = {row["name"]: row["status"] for row in policy["checks"]}
        assert policy["decision"] == "hard_fail"
        assert checks["tests"] == "hard_fail"
        assert checks["ci_patchset_suite_preflight"] == "hard_fail"

        for rule_name in ("tests", "require_tests", "ci_patchset_suite_preflight"):
            waive_out = runner.invoke(
                app,
                ["policy", "waive", patchset["patchset_id"], "--rule", rule_name, "--reason", "try bypass", "--json"],
                catch_exceptions=False,
            )
            assert waive_out.exit_code != 0
            assert "cannot be waived" in (waive_out.stdout + waive_out.stderr)

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        land = json.loads(land_out.stdout)
        assert land["status"] == "queued"
        assert _drain_jobs(data_dir) >= 1

        blocked_show = runner.invoke(app, ["land", "show", land["submission_id"], "--json"], catch_exceptions=False)
        assert blocked_show.exit_code == 0, blocked_show.stdout
        blocked = json.loads(blocked_show.stdout)
        assert blocked["status"] == "blocked"
        assert blocked["result"]["blocker_class"] == "POLICY_BLOCKED"


def test_patchset_ci_rerun_marks_policy_pending_until_fresh_green_results_arrive(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-ci-rerun-pending"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")
    _write_patchset_ci_contract(repo)

    with running_server(tmp_path / "server-async-ci-rerun-pending", queue_mode="async") as (base_url, data_dir):
        _bootstrap_main(repo, base_url, monkeypatch)

        task = _create_plan_bound_task(
            repo,
            title="ci rerun pending demo",
            intent="prove rerun pending blocks remote land until fresh green CI returns",
            risk="medium",
            slug="ci-rerun-pending-demo",
        )
        worktree_path = _ensure_task_worktree(repo, task["task_id"])
        monkeypatch.chdir(worktree_path)

        assert runner.invoke(app, ["line", "create", "feature/ci-rerun-pending"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/ci-rerun-pending"], catch_exceptions=False).exit_code == 0
        (worktree_path / "app.py").write_text("print('ci rerun pending')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature ci rerun pending", "--json"], catch_exceptions=False).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "CI rerun pending change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "ci rerun pending patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert _drain_jobs(data_dir) >= 2

        approve_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout
        code_review_out = runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                change["change_id"],
                "--reviewer",
                "codex@example.com",
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--message",
                WORKER_CODE_REVIEW_SUMMARY,
                "--json",
            ],
            catch_exceptions=False,
        )
        assert code_review_out.exit_code == 0, code_review_out.stdout
        assert _drain_jobs(data_dir) >= 1

        initial_policy_out = runner.invoke(app, ["policy", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert initial_policy_out.exit_code == 0, initial_policy_out.stdout
        assert json.loads(initial_policy_out.stdout)["decision"] == "pass"

        rerun_out = runner.invoke(app, ["patchset", "rerun-ci", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert rerun_out.exit_code == 0, rerun_out.stdout
        assert json.loads(rerun_out.stdout)["queued"] is True

        pending_policy_out = runner.invoke(app, ["policy", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert pending_policy_out.exit_code == 0, pending_policy_out.stdout
        pending_policy = json.loads(pending_policy_out.stdout)
        assert pending_policy["decision"] == "pending"

        pending_ci_out = runner.invoke(app, ["patchset", "ci-status", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert pending_ci_out.exit_code == 0, pending_ci_out.stdout
        pending_ci = json.loads(pending_ci_out.stdout)
        assert pending_ci["tests_status"] == "pending"
        assert set(pending_ci["selected_suite_ids"]) == {"preflight", "stable_smoke", "package_smoke"}

        assert _drain_jobs(data_dir) >= 2

        final_policy_out = runner.invoke(app, ["policy", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert final_policy_out.exit_code == 0, final_policy_out.stdout
        assert json.loads(final_policy_out.stdout)["decision"] == "pass"


def test_repo_ci_jobs_run_nightly_and_release_suites_without_patchset_coupling(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-repo-ci"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _write_repo_ci_contract(repo)

    with running_server(tmp_path / "server-repo-ci", queue_mode="async") as (base_url, data_dir):
        _bootstrap_main(repo, base_url, monkeypatch)
        _write_repo_ci_plane_config(repo)
        config_snap_out = runner.invoke(
            app,
            ["snapshot", "create", "--message", "repo ci plane config", "--json"],
            catch_exceptions=False,
        )
        assert config_snap_out.exit_code == 0, config_snap_out.stdout
        push_out = runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False)
        assert push_out.exit_code == 0, push_out.stdout

        nightly_out = runner.invoke(app, ["repo", "run-ci", "--plane", "nightly", "--json"], catch_exceptions=False)
        assert nightly_out.exit_code == 0, nightly_out.stdout
        nightly = json.loads(nightly_out.stdout)
        assert nightly["queued"] is True
        assert nightly["job"]["job_type"] == "repo.ci"
        assert _drain_jobs(data_dir) >= 1

        nightly_job_out = runner.invoke(
            app,
            ["repo", "jobs", "--job-id", str(nightly["job"]["job_id"]), "--json"],
            catch_exceptions=False,
        )
        assert nightly_job_out.exit_code == 0, nightly_job_out.stdout
        nightly_job = json.loads(nightly_job_out.stdout)
        assert nightly_job["state"] == "succeeded"
        nightly_result = nightly_job["result"]
        assert nightly_result["repo_name"] == "housekeeper"
        assert nightly_result["selected_suite_ids"] == ["full_repo", "postgres_preview", "release_readiness"]
        assert nightly_result["status"] == "pass"
        full_repo = next(item for item in nightly_result["suite_results"] if item["suite_id"] == "full_repo")
        assert full_repo["status"] == "pass"
        assert full_repo["triage"]["failed_test_count"] == 0
        assert full_repo["triage"]["suspect_task_selector"] == "recent_remote_landed"
        assert full_repo["triage"]["suspect_tasks"] == []
        ci_runs_out = runner.invoke(
            app,
            ["repo", "ci-runs", "--plane", "nightly", "--json"],
            catch_exceptions=False,
        )
        assert ci_runs_out.exit_code == 0, ci_runs_out.stdout
        ci_runs = json.loads(ci_runs_out.stdout)
        assert ci_runs["count"] >= 1
        assert ci_runs["items"][0]["selected_planes"] == ["nightly"]
        assert ci_runs["items"][0]["selected_suite_ids"] == [
            "full_repo",
            "postgres_preview",
            "release_readiness",
        ]
        assert ci_runs["items"][0]["status"] == "pass"
        assert ci_runs["items"][0]["rerun"]["cli"] == "ait repo run-ci --plane nightly"

        preview_out = runner.invoke(
            app,
            ["repo", "run-ci", "--suite", "release_readiness_xdist_preview", "--json"],
            catch_exceptions=False,
        )
        assert preview_out.exit_code == 0, preview_out.stdout
        preview = json.loads(preview_out.stdout)
        assert preview["queued"] is True
        preview_job_out = runner.invoke(
            app,
            ["repo", "jobs", "--job-id", str(preview["job"]["job_id"]), "--json"],
            catch_exceptions=False,
        )
        assert preview_job_out.exit_code == 0, preview_job_out.stdout
        preview_job = json.loads(preview_job_out.stdout)
        assert preview_job["state"] == "queued"
        assert _drain_jobs(data_dir) >= 1
        preview_job_out = runner.invoke(
            app,
            ["repo", "jobs", "--job-id", str(preview["job"]["job_id"]), "--json"],
            catch_exceptions=False,
        )
        assert preview_job_out.exit_code == 0, preview_job_out.stdout
        preview_job = json.loads(preview_job_out.stdout)
        assert preview_job["state"] == "succeeded"
        assert preview_job["result"]["selected_suite_ids"] == ["release_readiness_xdist_preview"]

        safe_parallel_out = runner.invoke(
            app,
            ["repo", "run-ci", "--suite", "safe_parallel_xdist_preview", "--json"],
            catch_exceptions=False,
        )
        assert safe_parallel_out.exit_code == 0, safe_parallel_out.stdout
        safe_parallel = json.loads(safe_parallel_out.stdout)
        assert safe_parallel["queued"] is True
        assert _drain_jobs(data_dir) >= 1
        safe_parallel_job_out = runner.invoke(
            app,
            ["repo", "jobs", "--job-id", str(safe_parallel["job"]["job_id"]), "--json"],
            catch_exceptions=False,
        )
        assert safe_parallel_job_out.exit_code == 0, safe_parallel_job_out.stdout
        safe_parallel_job = json.loads(safe_parallel_job_out.stdout)
        assert safe_parallel_job["state"] == "succeeded"
        assert safe_parallel_job["result"]["selected_suite_ids"] == ["safe_parallel_xdist_preview"]

        release_out = runner.invoke(
            app,
            [
                "repo",
                "run-ci",
                "--suite",
                "release_artifact_smoke",
                "--dependency-evidence",
                "dependency_report",
                "--dependency-evidence",
                "dependency_review",
                "--compliance-evidence",
                "compliance_report",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert release_out.exit_code == 0, release_out.stdout
        release = json.loads(release_out.stdout)
        assert release["queued"] is True
        assert release["job"]["job_type"] == "repo.ci"
        assert release["dependency_evidence"] == ["dependency_report", "dependency_review"]
        assert release["compliance_evidence"] == ["compliance_report"]
        assert _drain_jobs(data_dir) >= 1

        release_job_out = runner.invoke(
            app,
            ["repo", "jobs", "--job-id", str(release["job"]["job_id"]), "--json"],
            catch_exceptions=False,
        )
        assert release_job_out.exit_code == 0, release_job_out.stdout
        release_job = json.loads(release_job_out.stdout)
        assert release_job["state"] == "succeeded"
        release_result = release_job["result"]
        assert release_result["selected_suite_ids"] == ["release_artifact_smoke"]
        assert release_result["selected_planes"] == ["release"]
        assert release_result["status"] == "pass"
        assert release_result["dependency_evidence"] == ["dependency_report", "dependency_review"]
        assert release_result["compliance_evidence"] == ["compliance_report"]
        release_gate = release_result["suite_results"][0]["release_gate_evidence"]
        assert release_gate["dependency_keys"] == ["dependency_report", "dependency_review"]
        assert release_gate["compliance_keys"] == ["compliance_report", "license_exception_review"]
        assert release_gate["attached_dependency_evidence"] == ["dependency_report", "dependency_review"]
        assert release_gate["attached_compliance_evidence"] == ["compliance_report"]
        assert release_gate["missing_dependency_keys"] == []
        assert release_gate["missing_compliance_keys"] == ["license_exception_review"]



def test_repo_ci_task_batch_runs_supported_selectors_and_separates_lineage_from_behavior(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-batch"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _write_repo_ci_contract(repo)

    with running_server(tmp_path / "server-task-batch-ci", queue_mode="async") as (base_url, data_dir):
        _bootstrap_main(repo, base_url, monkeypatch)
        _write_repo_ci_task_batch_config(repo, count=2)
        config_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "task batch ci config", "--json"], catch_exceptions=False)
        assert config_snap_out.exit_code == 0, config_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main", "--json"], catch_exceptions=False).exit_code == 0

        line_out = runner.invoke(app, ["line", "show", "main", "--json"], catch_exceptions=False)
        assert line_out.exit_code == 0, line_out.stdout
        main_snapshot_id = json.loads(line_out.stdout)["head_snapshot_id"]

        ctx = ServerContext.create(data_dir, backend="postgres", postgres_dsn=fake_postgres_dsn(data_dir))
        initialize(ctx)
        repo_row = ensure_repository(ctx, "housekeeper", "main")
        recent_high_ts = (datetime.now(timezone.utc) - timedelta(hours=2)).isoformat().replace("+00:00", "Z")
        recent_sentinel_ts = (datetime.now(timezone.utc) - timedelta(hours=1)).isoformat().replace("+00:00", "Z")
        _seed_task_batch_candidate(
            ctx,
            "housekeeper",
            str(repo_row["repo_id"]),
            main_snapshot_id,
            task_id="T-TASK-BATCH-HIGH",
            task_seq=1,
            change_id="C-TASK-BATCH-HIGH",
            change_seq=1,
            patchset_id="P-TASK-BATCH-HIGH-1",
            risk_tier="high",
            landed_at=recent_high_ts,
            with_patchset=True,
        )
        _seed_task_batch_candidate(
            ctx,
            "housekeeper",
            str(repo_row["repo_id"]),
            main_snapshot_id,
            task_id="T-TASK-BATCH-SENTINEL",
            task_seq=2,
            change_id="C-TASK-BATCH-SENTINEL",
            change_seq=2,
            patchset_id="P-TASK-BATCH-SENTINEL-1",
            risk_tier="medium",
            landed_at=recent_sentinel_ts,
            with_patchset=False,
        )

        default_out = runner.invoke(app, ["repo", "run-ci", "--plane", "post_land_regression", "--json"], catch_exceptions=False)
        assert default_out.exit_code == 0, default_out.stdout
        default_run = json.loads(default_out.stdout)
        assert default_run["queued"] is True
        assert default_run["job"]["job_type"] == "repo.ci"
        assert _drain_jobs(data_dir) >= 1
        default_job = json.loads(
            runner.invoke(app, ["repo", "jobs", "--job-id", str(default_run["job"]["job_id"]), "--json"], catch_exceptions=False).stdout
        )
        assert default_job["state"] == "succeeded"
        default_result = default_job["result"]
        assert default_result["selected_suite_ids"] == [
            "release_readiness_xdist_preview",
            "safe_parallel_xdist_preview",
            "task_batch",
        ]
        assert default_result["selected_planes"] == ["post_land_regression"]
        assert default_result["status"] == "fail"
        task_batch = next(item for item in default_result["suite_results"] if item["suite_id"] == "task_batch")
        assert task_batch["selector"] == "recent_remote_landed"
        assert [item["task_id"] for item in task_batch["selected_tasks"]] == [
            "T-TASK-BATCH-SENTINEL",
            "T-TASK-BATCH-HIGH",
        ]
        assert task_batch["lineage_findings"]["problem_count"] == 1
        assert task_batch["lineage_findings"]["problems"][0]["task_id"] == "T-TASK-BATCH-SENTINEL"
        assert task_batch["behavior_regressions"]["status"] == "fail"
        assert task_batch["behavior_regressions"]["failing_suite_ids"] == ["task_batch_focus"]
        assert task_batch["artifacts"]["summary_json"]["exists"] is True
        assert task_batch["artifacts"]["summary_markdown"]["exists"] is True
        ci_runs_out = runner.invoke(
            app,
            ["repo", "ci-runs", "--suite", "task_batch", "--json"],
            catch_exceptions=False,
        )
        assert ci_runs_out.exit_code == 0, ci_runs_out.stdout
        ci_runs = json.loads(ci_runs_out.stdout)
        assert ci_runs["count"] >= 1
        task_batch_run = ci_runs["items"][0]
        assert task_batch_run["selected_suite_ids"] == [
            "release_readiness_xdist_preview",
            "safe_parallel_xdist_preview",
            "task_batch",
        ]
        assert task_batch_run["task_batch"]["selector"] == "recent_remote_landed"
        assert task_batch_run["task_batch"]["selected_task_count"] == 2
        assert task_batch_run["task_batch"]["selected_tasks"][0]["selection_reason"] == "recent_remote_landed"
        assert task_batch_run["task_batch"]["lineage_problem_count"] == 1
        assert task_batch_run["task_batch"]["behavior_status"] == "fail"
        assert task_batch_run["summary_artifacts"][0]["artifact_key"] == "summary_json"
        assert task_batch_run["rerun"]["cli"] == "ait repo run-ci --plane post_land_regression"

        explicit_out = runner.invoke(
            app,
            [
                "repo",
                "run-ci",
                "--suite",
                "task_batch",
                "--selector",
                "explicit_task_ids",
                "--task-id",
                "T-TASK-BATCH-HIGH",
                "--task-id",
                "T-TASK-BATCH-SENTINEL",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert explicit_out.exit_code == 0, explicit_out.stdout
        explicit_run = json.loads(explicit_out.stdout)
        assert explicit_run["selector"] == "explicit_task_ids"
        assert explicit_run["task_ids"] == ["T-TASK-BATCH-HIGH", "T-TASK-BATCH-SENTINEL"]
        assert _drain_jobs(data_dir) >= 1
        explicit_job = json.loads(
            runner.invoke(app, ["repo", "jobs", "--job-id", str(explicit_run["job"]["job_id"]), "--json"], catch_exceptions=False).stdout
        )
        explicit_result = explicit_job["result"]["suite_results"][0]
        assert explicit_result["selector"] == "explicit_task_ids"
        assert {item["task_id"] for item in explicit_result["selected_tasks"]} == {
            "T-TASK-BATCH-HIGH",
            "T-TASK-BATCH-SENTINEL",
        }
        assert explicit_result["lineage_findings"]["problem_count"] == 1
        assert explicit_result["behavior_regressions"]["failing_suite_ids"] == ["task_batch_focus"]

        curated_out = runner.invoke(
            app,
            [
                "repo",
                "run-ci",
                "--suite",
                "task_batch",
                "--selector",
                "curated_corpus",
                "--curated-corpus",
                "demo",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert curated_out.exit_code == 0, curated_out.stdout
        curated_run = json.loads(curated_out.stdout)
        assert curated_run["selector"] == "curated_corpus"
        assert curated_run["curated_corpus"] == "demo"
        assert _drain_jobs(data_dir) >= 1
        curated_job = json.loads(
            runner.invoke(app, ["repo", "jobs", "--job-id", str(curated_run["job"]["job_id"]), "--json"], catch_exceptions=False).stdout
        )
        curated_result = curated_job["result"]["suite_results"][0]
        assert curated_result["selector"] == "curated_corpus"
        assert [item["task_id"] for item in curated_result["selected_tasks"]] == [
            "T-TASK-BATCH-HIGH",
            "T-TASK-BATCH-SENTINEL",
        ]
        assert curated_result["lineage_findings"]["problem_count"] == 1

        high_risk_out = runner.invoke(
            app,
            [
                "repo",
                "run-ci",
                "--suite",
                "task_batch",
                "--selector",
                "recent_remote_landed_high_risk",
                "--count",
                "2",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert high_risk_out.exit_code == 0, high_risk_out.stdout
        high_risk_run = json.loads(high_risk_out.stdout)
        assert high_risk_run["selector"] == "recent_remote_landed_high_risk"
        assert high_risk_run["count"] == 2
        assert _drain_jobs(data_dir) >= 1
        high_risk_job = json.loads(
            runner.invoke(app, ["repo", "jobs", "--job-id", str(high_risk_run["job"]["job_id"]), "--json"], catch_exceptions=False).stdout
        )
        high_risk_result = high_risk_job["result"]["suite_results"][0]
        assert high_risk_result["selector"] == "recent_remote_landed_high_risk"
        assert [item["task_id"] for item in high_risk_result["selected_tasks"]] == ["T-TASK-BATCH-HIGH"]
        assert high_risk_result["lineage_findings"]["problem_count"] == 0
        assert high_risk_result["behavior_regressions"]["failing_suite_ids"] == ["task_batch_focus"]


def test_async_reconcile_job_repairs_current_patchset_pointer(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-reconcile"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-reconcile", queue_mode="async") as (base_url, data_dir):
        monkeypatch.setenv("AIT_ACTOR", "owner@example.com")
        monkeypatch.setenv("AIT_ROLES", "operator")
        monkeypatch.setenv("AIT_REPOS", "*")
        _bootstrap_main(repo, base_url, monkeypatch)

        task = _create_plan_bound_task(
            repo,
            title="reconcile demo",
            intent="demo",
            risk="medium",
            slug="reconcile-demo",
        )
        worktree_path = _ensure_task_worktree(repo, task["task_id"])
        monkeypatch.chdir(worktree_path)
        assert runner.invoke(app, ["line", "create", "feature/reconcile"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/reconcile"], catch_exceptions=False).exit_code == 0
        (worktree_path / "app.py").write_text("print('reconcile')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature reconcile"], catch_exceptions=False).exit_code == 0
        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Reconcile change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        change = json.loads(change_out.stdout)
        patchset_out = runner.invoke(app, ["patchset", "publish", "--change", change["change_id"], "--summary", "ps1", "--json"], catch_exceptions=False)
        patchset = json.loads(patchset_out.stdout)
        assert patchset["patchset_number"] == 1
        _drain_jobs(data_dir)

        ctx = ServerContext.create(data_dir, backend="postgres", postgres_dsn=fake_postgres_dsn(data_dir))
        conn = connect_control(ctx)
        conn.execute("update changes set current_patchset_number = 0 where change_id = ?", (change["change_id"],))
        conn.commit()
        conn.close()

        reconcile_out = runner.invoke(app, ["repo", "reconcile", "--repair", "--json"], catch_exceptions=False)
        assert reconcile_out.exit_code == 0, reconcile_out.stdout
        reconcile = json.loads(reconcile_out.stdout)
        assert reconcile["queued"] is True
        assert reconcile["job"]["job_type"] == "reconcile.repo"
        duplicate_out = runner.invoke(app, ["repo", "reconcile", "--repair", "--json"], catch_exceptions=False)
        assert duplicate_out.exit_code == 0, duplicate_out.stdout
        duplicate = json.loads(duplicate_out.stdout)
        assert int(duplicate["job"]["job_id"]) == int(reconcile["job"]["job_id"])

        jobs_out = runner.invoke(app, ["repo", "jobs", "--json"], catch_exceptions=False)
        jobs = json.loads(jobs_out.stdout)
        reconcile_jobs = [job for job in jobs if job["job_type"] == "reconcile.repo" and job["state"] == "queued"]
        assert len(reconcile_jobs) == 1

        processed = _drain_jobs(data_dir)
        assert processed >= 1

        jobs_out = runner.invoke(app, ["repo", "jobs", "--json"], catch_exceptions=False)
        jobs = json.loads(jobs_out.stdout)
        assert any(job["job_type"] == "reconcile.repo" and job["state"] == "succeeded" for job in jobs)

        change_show = runner.invoke(app, ["change", "show", change["change_id"], "--json"], catch_exceptions=False)
        repaired_change = json.loads(change_show.stdout)
        assert repaired_change["current_patchset_number"] == 1


@LIVE_POSTGRES
def test_live_postgres_async_policy_and_land_jobs_run_through_worker(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-pg-async"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_live_postgres(tmp_path / "live-postgres-async") as runtime:
        with running_postgres_server(tmp_path / "server-async-pg", runtime["dsn"], queue_mode="async") as (base_url, data_dir):
            _bootstrap_main(repo, base_url, monkeypatch)

            task = _create_plan_bound_task(
                repo,
                title="async postgres demo",
                intent="demo",
                risk="high",
                slug="async-postgres-demo",
            )
            worktree_path = _ensure_task_worktree(repo, task["task_id"])
            monkeypatch.chdir(worktree_path)

            assert runner.invoke(app, ["line", "create", "feature/async-postgres"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(app, ["line", "switch", "feature/async-postgres"], catch_exceptions=False).exit_code == 0
            (worktree_path / "app.py").write_text("print('async postgres')\n", encoding="utf-8")
            assert runner.invoke(app, ["snapshot", "create", "--message", "feature async postgres", "--json"], catch_exceptions=False).exit_code == 0

            change_out = runner.invoke(
                app,
                ["change", "create", "--task", task["task_id"], "--title", "Async postgres change", "--base-line", "main", "--risk", "high", "--json"],
                catch_exceptions=False,
            )
            assert change_out.exit_code == 0, change_out.stdout
            change = json.loads(change_out.stdout)

            patchset_out = runner.invoke(
                app,
                ["patchset", "publish", "--change", change["change_id"], "--summary", "queued postgres patchset", "--json"],
                catch_exceptions=False,
            )
            assert patchset_out.exit_code == 0, patchset_out.stdout
            patchset = json.loads(patchset_out.stdout)
            assert "policy_job" not in patchset
            assert patchset["policy_followup"]["state"] == "deferred"

            assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(
                app,
                ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
                catch_exceptions=False,
            ).exit_code == 0
            assert runner.invoke(
                app,
                [
                    "review",
                    "code",
                    "submit",
                    change["change_id"],
                    "--reviewer",
                    "codex@example.com",
                    "--patchset",
                    patchset["patchset_id"],
                    "--verdict",
                    "pass",
                    "--message",
                    WORKER_CODE_REVIEW_SUMMARY,
                    "--json",
                ],
                catch_exceptions=False,
            ).exit_code == 0

            jobs_out = runner.invoke(app, ["repo", "jobs", "--json"], catch_exceptions=False)
            assert jobs_out.exit_code == 0, jobs_out.stdout
            jobs = json.loads(jobs_out.stdout)
            assert any(job["job_type"] == "policy.evaluate" and job["state"] == "queued" for job in jobs)

            processed = _drain_jobs(data_dir, backend="postgres", postgres_dsn=runtime["dsn"])
            assert processed >= 1

            policy_out = runner.invoke(app, ["policy", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
            assert policy_out.exit_code == 0, policy_out.stdout
            assert json.loads(policy_out.stdout)["decision"] == "pass"

            land_out = runner.invoke(
                app,
                ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
                catch_exceptions=False,
            )
            assert land_out.exit_code == 0, land_out.stdout
            land = json.loads(land_out.stdout)
            assert land["status"] == "queued"
            assert "land_job" in land

            processed = _drain_jobs(data_dir, backend="postgres", postgres_dsn=runtime["dsn"])
            assert processed >= 1

            landed_show = runner.invoke(app, ["land", "show", land["submission_id"], "--json"], catch_exceptions=False)
            assert landed_show.exit_code == 0, landed_show.stdout
            assert json.loads(landed_show.stdout)["status"] == "succeeded"


@LIVE_POSTGRES
def test_live_postgres_land_job_requeues_then_succeeds(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-pg-land-retry"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_live_postgres(tmp_path / "live-postgres-land-retry") as runtime:
        with running_postgres_server(tmp_path / "server-land-retry-pg", runtime["dsn"], queue_mode="async") as (base_url, data_dir):
            _bootstrap_main(repo, base_url, monkeypatch)

            task = _create_plan_bound_task(
                repo,
                title="land retry demo",
                intent="demo",
                risk="high",
                slug="land-retry-demo",
            )
            worktree_path = _ensure_task_worktree(repo, task["task_id"])
            monkeypatch.chdir(worktree_path)

            assert runner.invoke(app, ["line", "create", "feature/land-retry"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(app, ["line", "switch", "feature/land-retry"], catch_exceptions=False).exit_code == 0
            (worktree_path / "app.py").write_text("print('land retry')\n", encoding="utf-8")
            assert runner.invoke(app, ["snapshot", "create", "--message", "feature land retry"], catch_exceptions=False).exit_code == 0

            change_out = runner.invoke(
                app,
                ["change", "create", "--task", task["task_id"], "--title", "Land retry change", "--base-line", "main", "--risk", "high", "--json"],
                catch_exceptions=False,
            )
            assert change_out.exit_code == 0, change_out.stdout
            change = json.loads(change_out.stdout)

            patchset_out = runner.invoke(
                app,
                ["patchset", "publish", "--change", change["change_id"], "--summary", "land retry patchset", "--json"],
                catch_exceptions=False,
            )
            assert patchset_out.exit_code == 0, patchset_out.stdout
            patchset = json.loads(patchset_out.stdout)

            assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(
                app,
                ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
                catch_exceptions=False,
            ).exit_code == 0
            assert runner.invoke(
                app,
                [
                    "review",
                    "code",
                    "submit",
                    change["change_id"],
                    "--reviewer",
                    "codex@example.com",
                    "--patchset",
                    patchset["patchset_id"],
                    "--verdict",
                    "pass",
                    "--message",
                    WORKER_CODE_REVIEW_SUMMARY,
                    "--json",
                ],
                catch_exceptions=False,
            ).exit_code == 0

            processed = _drain_jobs(data_dir, backend="postgres", postgres_dsn=runtime["dsn"])
            assert processed >= 1

            land_out = runner.invoke(
                app,
                ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
                catch_exceptions=False,
            )
            assert land_out.exit_code == 0, land_out.stdout
            land = json.loads(land_out.stdout)
            assert land["status"] == "queued"

            ctx = ServerContext.create(data_dir, backend="postgres", postgres_dsn=runtime["dsn"])
            initialize(ctx)
            import ait_native.worker as worker_module

            real_dispatch = worker_module._dispatch
            state = {"failed_once": False}

            def flaky_dispatch(worker_ctx, claimed_job):
                if claimed_job["job_type"] == "land.process" and not state["failed_once"]:
                    state["failed_once"] = True
                    raise RuntimeError("simulated land job failure")
                return real_dispatch(worker_ctx, claimed_job)

            monkeypatch.setattr(worker_module, "_dispatch", flaky_dispatch)

            first_attempt = process_one(ctx, worker_id="pg-worker")
            assert first_attempt is not None
            assert first_attempt["job_type"] == "land.process"
            assert first_attempt["state"] == "queued"
            assert first_attempt["attempt_count"] == 1
            assert "simulated land job failure" in (first_attempt["last_error"] or "")

            conn = connect_control(ctx)
            conn.execute("update jobs set available_at = ? where job_id = ?", (_stale_timestamp(600), int(first_attempt["job_id"])))
            conn.commit()
            conn.close()

            second_attempt = process_one(ctx, worker_id="pg-worker")
            assert second_attempt is not None
            assert second_attempt["job_id"] == first_attempt["job_id"]
            assert second_attempt["state"] == "succeeded"
            assert second_attempt["attempt_count"] == 2

            landed_show = runner.invoke(app, ["land", "show", land["submission_id"], "--json"], catch_exceptions=False)
            assert landed_show.exit_code == 0, landed_show.stdout
            assert json.loads(landed_show.stdout)["status"] == "succeeded"


@LIVE_POSTGRES
def test_live_postgres_reclaim_stale_job_requeues_and_processes(tmp_path: Path, monkeypatch):
    with running_live_postgres(tmp_path / "live-postgres-stale-reclaim") as runtime:
        restore_real_psycopg()
        ctx = ServerContext.create(tmp_path / "worker-stale-requeue-pg", backend="postgres", postgres_dsn=runtime["dsn"])
        initialize(ctx)
        job = enqueue_job(ctx, "demo", "demo.job", {"value": 1}, max_attempts=3)

        conn = connect_control(ctx)
        conn.execute(
            """
            update jobs
            set state = 'running', attempt_count = 1, max_attempts = 3, locked_at = ?, locked_by = ?, updated_at = ?
            where job_id = ?
            """,
            (_stale_timestamp(600), "dead-worker", _stale_timestamp(600), int(job["job_id"])),
        )
        conn.commit()
        conn.close()

        monkeypatch.setattr("ait_native.worker._dispatch", lambda _ctx, claimed_job: {"handled_job_id": claimed_job["job_id"]})

        processed = process_one(ctx, worker_id="replacement-worker", reclaim_stale_seconds=30)
        assert processed is not None
        assert processed["job_id"] == job["job_id"]
        assert processed["state"] == "succeeded"
        assert processed["attempt_count"] == 2
        assert processed["result"]["handled_job_id"] == job["job_id"]


def test_reclaim_stale_job_requeues_and_processes_with_worker(tmp_path: Path, monkeypatch):
    ctx = fake_postgres_context(tmp_path / "worker-stale-requeue")
    initialize(ctx)
    job = enqueue_job(ctx, "demo", "demo.job", {"value": 1}, max_attempts=3)

    conn = connect_control(ctx)
    conn.execute(
        """
        update jobs
        set state = 'running', attempt_count = 1, max_attempts = 3, locked_at = ?, locked_by = ?, updated_at = ?
        where job_id = ?
        """,
        (_stale_timestamp(600), "dead-worker", _stale_timestamp(600), int(job["job_id"])),
    )
    conn.commit()
    conn.close()

    monkeypatch.setattr("ait_native.worker._dispatch", lambda _ctx, claimed_job: {"handled_job_id": claimed_job["job_id"]})

    processed = process_one(ctx, worker_id="replacement-worker", reclaim_stale_seconds=30)
    assert processed is not None
    assert processed["job_id"] == job["job_id"]
    assert processed["state"] == "succeeded"
    assert processed["attempt_count"] == 2
    assert processed["result"]["handled_job_id"] == job["job_id"]


def test_reclaim_stale_job_fails_when_attempts_are_exhausted(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "worker-stale-failed")
    initialize(ctx)
    job = enqueue_job(ctx, "demo", "demo.job", {"value": 1}, max_attempts=2)

    conn = connect_control(ctx)
    conn.execute(
        """
        update jobs
        set state = 'running', attempt_count = 2, max_attempts = 2, locked_at = ?, locked_by = ?, updated_at = ?
        where job_id = ?
        """,
        (_stale_timestamp(600), "dead-worker", _stale_timestamp(600), int(job["job_id"])),
    )
    conn.commit()
    conn.close()

    summary = reclaim_stale_jobs(ctx, 30)
    assert summary["stale_count"] == 1
    assert summary["requeued_job_ids"] == []
    assert summary["failed_job_ids"] == [job["job_id"]]

    stored = get_job(ctx, int(job["job_id"]))
    assert stored["state"] == "failed"
    assert "max attempts" in stored["last_error"]


def test_repository_worker_status_api_reports_running_workers(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()

    with running_server(tmp_path / "worker-status-api") as (base_url, data_dir):
        _bootstrap_main(repo, base_url, monkeypatch)

        ctx = fake_postgres_context(data_dir)
        initialize(ctx)
        queue_job_1 = enqueue_job(ctx, "housekeeper", "content.optimize", {"repo_name": "housekeeper", "repair": True})
        queue_job_2 = enqueue_job(ctx, "housekeeper", "content.gc", {"repo_name": "housekeeper", "prune_unreferenced": False})
        enqueue_job(ctx, "housekeeper", "reconcile.repo", {"repo_name": "housekeeper", "repair": False})

        running_job_1 = claim_next_job(ctx, "worker-1", repo_name="housekeeper")
        running_job_2 = claim_next_job(ctx, "worker-2", repo_name="housekeeper")
        assert running_job_1 is not None
        assert running_job_2 is not None
        claimed_ids = {running_job_1["job_id"], running_job_2["job_id"]}
        assert claimed_ids == {int(queue_job_1["job_id"]), int(queue_job_2["job_id"])}

        conn = connect_control(ctx)
        conn.execute(
            "update jobs set locked_at = ?, updated_at = ? where job_id = ?",
            (_stale_timestamp(600), _stale_timestamp(600), int(running_job_1["job_id"])),
        )
        conn.commit()
        conn.close()

        payload = json.loads(
            urllib.request.urlopen(f"{base_url}/v1/native/admin/repositories/housekeeper/workers").read().decode("utf-8")
        )
        metrics_payload = json.loads(
            urllib.request.urlopen(f"{base_url}/v1/native/admin/metrics?recent_jobs_limit=10").read().decode("utf-8")
        )
        readiness_payload = json.loads(
            urllib.request.urlopen(f"{base_url}/v1/native/admin/readiness?recent_jobs_limit=10").read().decode("utf-8")
        )

        jobs_diag_out = runner.invoke(app, ["repo", "jobs", "--diagnostics", "--json"], catch_exceptions=False)
        assert jobs_diag_out.exit_code == 0, jobs_diag_out.stdout
        jobs_diag = json.loads(jobs_diag_out.stdout)
        metrics_out = runner.invoke(app, ["repo", "metrics", "--recent-jobs-limit", "10", "--json"], catch_exceptions=False)
        assert metrics_out.exit_code == 0, metrics_out.stdout
        metrics_cli = json.loads(metrics_out.stdout)
        readiness_out = runner.invoke(app, ["repo", "readiness", "--recent-jobs-limit", "10", "--json"], catch_exceptions=False)
        assert readiness_out.exit_code == 0, readiness_out.stdout
        readiness_cli = json.loads(readiness_out.stdout)

        assert payload["repo_name"] == "housekeeper"
        assert payload["running_jobs"] == 2
        assert payload["worker_count"] == 2
        assert payload["state_summary"]["running"] == 2
        assert payload["state_summary"]["queued"] == 1
        assert payload["diagnostics"]["recommended_action"] == "reclaim_stale"
        assert payload["diagnostics"]["stale_running_jobs"] == 1
        assert jobs_diag["recommended_action"] == "reclaim_stale"
        assert jobs_diag["stale_running_jobs"] == 1
        assert metrics_payload["summary"]["repo_count"] == 1
        assert metrics_payload["worker_metrics"]["running_jobs"] == 2
        assert metrics_payload["worker_metrics"]["queued_jobs"] == 1
        assert metrics_payload["job_outcome_metrics"]["recommended_action"] == "reclaim_stale"
        assert metrics_cli["summary"]["recommended_action"] == "reclaim_stale"
        assert metrics_cli["repositories"][0]["repo_name"] == "housekeeper"
        assert readiness_payload["ready"] is False
        assert readiness_payload["recommended_action"] == "reclaim_stale"
        assert readiness_cli["job_summary"]["stale_running_jobs"] == 1
        assert {worker["worker_id"] for worker in payload["workers"]} == {"worker-1", "worker-2"}
        assert len(payload["recent_jobs"]) >= 3


def test_queue_and_read_models_release_control_connections_on_empty_or_missing_results(tmp_path: Path, monkeypatch):
    class CountingPsycopg:
        def __init__(self):
            self.inner = FakePsycopg()
            self.connect_calls = 0

        def connect(self, dsn: str):
            self.connect_calls += 1
            return self.inner.connect(dsn)

    fake = CountingPsycopg()
    monkeypatch.setattr(server_db, "_load_psycopg", lambda: fake)
    monkeypatch.setattr(server_db, "postgres_support_installed", lambda: True)
    ctx = ServerContext.create(
        tmp_path / "queue-read-model-server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-runtime'}",
        content_schema="ait_native_content_queue_test",
        control_schema="ait_native_control_queue_test",
    )

    initialize(ctx)
    ensure_repository(ctx, "repo-a", "main")
    baseline_connect_calls = fake.connect_calls

    with pytest.raises(KeyError, match="Unknown job"):
        get_job(ctx, 999999)

    assert native_read_models._latest_land_summary(ctx, "C-999999") is None

    conn = connect_control(ctx)
    try:
        assert fake.connect_calls == baseline_connect_calls
    finally:
        conn.close()


def test_read_models_repository_index_reuses_content_pool_connections(tmp_path: Path, monkeypatch):
    class CountingPsycopg:
        def __init__(self):
            self.inner = FakePsycopg()
            self.connect_calls = 0

        def connect(self, dsn: str):
            self.connect_calls += 1
            return self.inner.connect(dsn)

    fake = CountingPsycopg()
    monkeypatch.setattr(server_db, "_load_psycopg", lambda: fake)
    monkeypatch.setattr(server_db, "postgres_support_installed", lambda: True)
    ctx = ServerContext.create(
        tmp_path / "read-model-index-server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-runtime'}",
        content_schema="ait_native_content_read_models_test",
        control_schema="ait_native_control_read_models_test",
    )

    initialize(ctx)
    ensure_repository(ctx, "repo-a", "main")
    baseline_connect_calls = fake.connect_calls

    payload = native_read_models.repository_index(ctx)
    assert any(row["repo_name"] == "repo-a" for row in payload["repositories"])

    conn = connect_content(ctx)
    try:
        assert fake.connect_calls == baseline_connect_calls
    finally:
        conn.close()


def test_fast_operator_healthz_surface_stages_live_turn_pressure_summary(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-fast-health"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "operator-fast-health") as (base_url, _data_dir):
        _bootstrap_main(repo, base_url, monkeypatch)

        health_payload = _json_get(base_url, "/healthz")
        metrics_payload = _json_get(base_url, "/v1/native/admin/metrics?recent_jobs_limit=5")
        readiness_payload = _json_get(base_url, "/v1/native/admin/readiness?recent_jobs_limit=5")

        health_pressure = _pressure_payload(health_payload, source="/healthz")
        metrics_pressure = _pressure_payload(metrics_payload, source="/v1/native/admin/metrics")
        readiness_pressure = _pressure_payload(readiness_payload, source="/v1/native/admin/readiness")
        health_cache_state, health_cache_age, health_cache_ttl = _cache_metadata(
            health_payload,
            source="/healthz",
            require_state=False,
        )
        metrics_cache_state, metrics_cache_age, metrics_cache_ttl = _cache_metadata(metrics_payload, source="/v1/native/admin/metrics")
        readiness_cache_state, readiness_cache_age, readiness_cache_ttl = _cache_metadata(readiness_payload, source="/v1/native/admin/readiness")

        assert health_payload.get("ok") is True
        assert health_payload["ci_capabilities"]["patchset_run_ci_route"] is True
        assert health_payload["ci_capabilities"]["repo_run_ci_route"] is True
        assert health_payload["ci_readiness"]["runtime_generation"] == "native_ci_runtime_v1"
        _assert_live_turn_pressure_shape(health_pressure)
        _assert_live_turn_pressure_shape(metrics_pressure)
        _assert_live_turn_pressure_shape(readiness_pressure)
        assert health_pressure["in_flight_turns"] == metrics_pressure["in_flight_turns"] == readiness_pressure["in_flight_turns"]
        assert health_pressure["queued_turns"] == metrics_pressure["queued_turns"] == readiness_pressure["queued_turns"]
        assert health_pressure["pressure_state"] == metrics_pressure["pressure_state"] == readiness_pressure["pressure_state"]
        assert health_cache_state in {None, "computed", "cached"}
        assert metrics_cache_state in {"computed", "cached"}
        assert readiness_cache_state in {"computed", "cached"}
        assert health_cache_age >= 0
        assert metrics_cache_age >= 0
        assert readiness_cache_age >= 0
        assert health_cache_ttl >= 0
        assert metrics_cache_ttl >= 0
        assert readiness_cache_ttl >= 0


def test_worker_cli_can_reclaim_stale_jobs(tmp_path: Path):
    data_dir = tmp_path / "worker-cli-reclaim"
    old_data = os.environ.get("AIT_NATIVE_SERVER_DATA")
    old_backend = os.environ.get("AIT_NATIVE_SERVER_DB_BACKEND")
    old_dsn = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN")
    old_content_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA")
    old_control_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA")
    install_fake_psycopg_global()
    reset_fake_postgres_runtime()
    os.environ["AIT_NATIVE_SERVER_DATA"] = str(data_dir)
    os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = "postgres"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = fake_postgres_dsn(data_dir)
    os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA"] = "ait_native_content"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA"] = "ait_native_control"
    try:
        ctx = ServerContext.create(data_dir, backend="postgres", postgres_dsn=fake_postgres_dsn(data_dir))
        initialize(ctx)
        job = enqueue_job(ctx, "demo", "demo.job", {"value": 1}, max_attempts=3)

        conn = connect_control(ctx)
        conn.execute(
            """
            update jobs
            set state = 'running', attempt_count = 1, max_attempts = 3, locked_at = ?, locked_by = ?, updated_at = ?
            where job_id = ?
            """,
            (_stale_timestamp(600), "dead-worker", _stale_timestamp(600), int(job["job_id"])),
        )
        conn.commit()
        conn.close()

        out = worker_runner.invoke(worker_app, ["reclaim-stale", "--stale-seconds", "30"])
        assert out.exit_code == 0, out.stdout

        stored = get_job(ctx, int(job["job_id"]))
        assert stored["state"] == "queued"
        assert stored["locked_by"] is None
    finally:
        reset_fake_postgres_runtime()
        if old_data is None:
            os.environ.pop("AIT_NATIVE_SERVER_DATA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DATA"] = old_data
        if old_backend is None:
            os.environ.pop("AIT_NATIVE_SERVER_DB_BACKEND", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = old_backend
        if old_dsn is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_DSN", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = old_dsn
        if old_content_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA"] = old_content_schema
        if old_control_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA"] = old_control_schema
