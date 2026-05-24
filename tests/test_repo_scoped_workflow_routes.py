from __future__ import annotations

import base64
import hashlib
from pathlib import Path

from fastapi.testclient import TestClient

import ait_server.app as server_app
from ait_server import server_store
from ait_server.server_control import connect
from ait_server.server_paths import ServerContext
from tests.postgres_fake import fake_postgres_context, fake_postgres_dsn


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


def _publish_ready_change(ctx: ServerContext, repo_name: str, task_title: str, change_title: str, suffix: str):
    task = server_store.create_task(ctx, repo_name, task_title, "repo scoped routes", "high")
    change = server_store.create_change(ctx, repo_name, task["task_id"], change_title, "main", "medium")
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
    server_store.evaluate_policy(ctx, patchset["patchset_id"])
    return task, change, patchset


def _repo_scoped_task_dag_graph(plan_id: str, plan_revision_id: str, graph_id: str = "demo/task-dag") -> dict:
    return {
        "schema_version": 1,
        "graph_id": graph_id,
        "source_plan": {
            "artifact_path": "docs/sprints/repo-a.md",
            "plan_id": plan_id,
            "plan_revision_id": plan_revision_id,
        },
        "execution_policy": {
            "mode": "guarded_full_dag_convergence",
            "default_mode": "local_execution_dag_with_selective_promotion",
            "dispatch_model": "compact_packet",
            "worker_execution_mode": "worker_only_compact_packet",
            "max_total_sessions": 1,
            "max_worker_sessions": 1,
            "max_batch_sessions": 1,
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Compatibility node",
                "depends_on": [],
                "task_template": {"title": "Compatibility task", "risk_tier": "low"},
            },
        ],
        "edges": [],
    }


def test_repo_scoped_line_route_allows_updates_without_explicit_expected_head(tmp_path: Path, monkeypatch):
    data_dir = tmp_path / "server-data"
    ctx = fake_postgres_context(data_dir)
    server_store.initialize(ctx)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")

    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")

    base_snapshot = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-ROUTE-OPTIONAL-CAS-BASE",
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
            "SNP-ROUTE-OPTIONAL-CAS-CURRENT",
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
            "SNP-ROUTE-OPTIONAL-CAS-NEXT",
            parent_snapshot_id=current_snapshot["snapshot_id"],
            line_name="main",
            message="next",
            files={"README.md": b"base\ncurrent\nnext\n"},
        ),
    )

    with TestClient(server_app.create_app()) as client:
        first = client.put(
            "/v1/native/repositories/repo-a/lines/main",
            json={"head_snapshot_id": base_snapshot["snapshot_id"]},
        )
        assert first.status_code == 200
        assert first.json()["head_snapshot_id"] == base_snapshot["snapshot_id"]

        second = client.put(
            "/v1/native/repositories/repo-a/lines/main",
            json={"head_snapshot_id": current_snapshot["snapshot_id"]},
        )
        assert second.status_code == 200
        assert second.json()["head_snapshot_id"] == current_snapshot["snapshot_id"]

        third = client.put(
            "/v1/native/repositories/repo-a/lines/main",
            json={"head_snapshot_id": next_snapshot["snapshot_id"]},
        )
        assert third.status_code == 200
        assert third.json()["head_snapshot_id"] == next_snapshot["snapshot_id"]


def test_repo_scoped_routes_resolve_local_refs_without_global_id_assumptions(tmp_path: Path, monkeypatch):
    data_dir = tmp_path / "server-data"
    ctx = fake_postgres_context(data_dir)
    server_store.initialize(ctx)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")

    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="BBB")

    task_a1, change_a1, patchset_a1 = _publish_ready_change(ctx, "repo-a", "Task A1", "Change A1", "A1")
    task_b1, change_b1, patchset_b1 = _publish_ready_change(ctx, "repo-b", "Task B1", "Change B1", "B1")
    land_a1 = server_store.create_land_request(ctx, change_a1["change_id"], patchset_a1["patchset_id"], "main", "direct")
    land_b1 = server_store.create_land_request(ctx, change_b1["change_id"], patchset_b1["patchset_id"], "main", "direct")
    task_a2, change_a2, patchset_a2 = _publish_ready_change(ctx, "repo-a", "Task A2", "Change A2", "A2")

    with TestClient(server_app.create_app()) as client:
        task_resp = client.get("/v1/native/repositories/repo-a/tasks/1")
        assert task_resp.status_code == 200
        assert task_resp.json()["task_id"] == task_a1["task_id"]

        other_task_resp = client.get("/v1/native/repositories/repo-b/tasks/1")
        assert other_task_resp.status_code == 200
        assert other_task_resp.json()["task_id"] == task_b1["task_id"]

        change_resp = client.get("/v1/native/repositories/repo-a/changes/1")
        assert change_resp.status_code == 200
        assert change_resp.json()["change_id"] == change_a1["change_id"]

        patchset_resp = client.get("/v1/native/repositories/repo-a/patchsets/1", params={"change_ref": "1"})
        assert patchset_resp.status_code == 200
        assert patchset_resp.json()["patchset_id"] == patchset_a1["patchset_id"]

        land_resp = client.get("/v1/native/repositories/repo-a/lands/1")
        assert land_resp.status_code == 200
        assert land_resp.json()["submission_id"] == land_a1["submission_id"]

        other_land_resp = client.get("/v1/native/repositories/repo-b/lands/1")
        assert other_land_resp.status_code == 200
        assert other_land_resp.json()["submission_id"] == land_b1["submission_id"]

        read_task_resp = client.get("/v1/native/repositories/repo-a/read/tasks/1")
        assert read_task_resp.status_code == 200
        assert read_task_resp.json()["task"]["task_id"] == task_a1["task_id"]

        read_task_audit_resp = client.get("/v1/native/repositories/repo-a/read/tasks/1/audit")
        assert read_task_audit_resp.status_code == 200
        assert read_task_audit_resp.json()["task"]["task_id"] == task_a1["task_id"]

        read_change_resp = client.get("/v1/native/repositories/repo-a/read/changes/1")
        assert read_change_resp.status_code == 200
        assert read_change_resp.json()["change"]["change_id"] == change_a1["change_id"]

        read_patchset_delta_resp = client.get(
            "/v1/native/repositories/repo-a/read/patchsets/1/delta",
            params={"change_ref": "1"},
        )
        assert read_patchset_delta_resp.status_code == 200
        assert read_patchset_delta_resp.json()["patchset_id"] == patchset_a1["patchset_id"]

        other_read_task_resp = client.get("/v1/native/repositories/repo-b/read/tasks/1")
        assert other_read_task_resp.status_code == 200
        assert other_read_task_resp.json()["task"]["task_id"] == task_b1["task_id"]

        other_read_change_resp = client.get("/v1/native/repositories/repo-b/read/changes/1")
        assert other_read_change_resp.status_code == 200
        assert other_read_change_resp.json()["change"]["change_id"] == change_b1["change_id"]

        other_read_patchset_delta_resp = client.get(
            "/v1/native/repositories/repo-b/read/patchsets/1/delta",
            params={"change_ref": "1"},
        )
        assert other_read_patchset_delta_resp.status_code == 200
        assert other_read_patchset_delta_resp.json()["patchset_id"] == patchset_b1["patchset_id"]

        submit_resp = client.post(
            "/v1/native/repositories/repo-a/changes/2:submit",
            json={"patchset_id": "1", "target_line": "main", "mode": "direct"},
        )
        assert submit_resp.status_code == 200
        payload = submit_resp.json()
        assert payload["change_id"] == change_a2["change_id"]
        assert payload["status"] == "succeeded"
        assert payload["submission_id"].startswith("LAND-")
        assert payload["result"]["target_line"] == "main"


def test_repo_scoped_routes_prefer_repo_id_when_workflow_rows_drift(tmp_path: Path, monkeypatch):
    data_dir = tmp_path / "server-data"
    ctx = fake_postgres_context(data_dir)
    server_store.initialize(ctx)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")

    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    task, change, patchset = _publish_ready_change(ctx, "repo-a", "Task Drift", "Change Drift", "DRIFT")
    land = server_store.create_land_request(ctx, change["change_id"], patchset["patchset_id"], "main", "direct")

    with connect(ctx) as conn:
        drifted_repo_name = "repo-a-legacy"
        conn.execute("update tasks set repo_name = ? where task_id = ?", (drifted_repo_name, task["task_id"]))
        conn.execute("update changes set repo_name = ? where change_id = ?", (drifted_repo_name, change["change_id"]))
        conn.commit()

    with TestClient(server_app.create_app()) as client:
        task_resp = client.get("/v1/native/repositories/repo-a/tasks/1")
        assert task_resp.status_code == 200
        assert task_resp.json()["task_id"] == task["task_id"]

        change_resp = client.get("/v1/native/repositories/repo-a/changes/1")
        assert change_resp.status_code == 200
        assert change_resp.json()["change_id"] == change["change_id"]

        patchset_resp = client.get("/v1/native/repositories/repo-a/patchsets/1", params={"change_ref": "1"})
        assert patchset_resp.status_code == 200
        assert patchset_resp.json()["patchset_id"] == patchset["patchset_id"]

        land_resp = client.get("/v1/native/repositories/repo-a/lands/1")
        assert land_resp.status_code == 200
        assert land_resp.json()["submission_id"] == land["submission_id"]

        submit_resp = client.post(
            "/v1/native/repositories/repo-a/changes/1:submit",
            json={"patchset_id": "1", "target_line": "main", "mode": "direct"},
        )
        assert submit_resp.status_code == 200
        payload = submit_resp.json()
        assert payload["status"] == "succeeded"
        assert payload["submission_id"].startswith("LAND-")
        assert payload["result"]["target_line"] == "main"


def test_repo_scoped_session_routes_resolve_local_refs(tmp_path: Path, monkeypatch):
    data_dir = tmp_path / "server-data-sessions"
    ctx = fake_postgres_context(data_dir)
    server_store.initialize(ctx)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")

    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    task, change, _ = _publish_ready_change(ctx, "repo-a", "Task Session", "Change Session", "SES")
    session = server_store.create_session(
        ctx,
        "repo-a",
        "task_graph_run",
        task_id=task["task_id"],
        change_id=change["change_id"],
        title="Repo session",
    )
    event = server_store.append_session_event(ctx, session["session_id"], "session.note", {"text": "hello"})

    with TestClient(server_app.create_app()) as client:
        session_resp = client.get(f"/v1/native/repositories/repo-a/sessions/{session['session_local_id']}")
        assert session_resp.status_code == 200
        assert session_resp.json()["session_id"] == session["session_id"]

        events_resp = client.get(
            f"/v1/native/repositories/repo-a/sessions/{session['session_local_id']}/events",
            params={"after_sequence": 0, "limit": 20},
        )
        assert events_resp.status_code == 200
        events_payload = events_resp.json()
        assert [row["sequence"] for row in events_payload] == [event["sequence"]]

        checkpoint_resp = client.post(
            f"/v1/native/repositories/repo-a/sessions/{session['session_local_id']}/checkpoints",
            json={"summary": "repo checkpoint", "resume_payload": {"step": "continue"}},
        )
        assert checkpoint_resp.status_code == 200
        checkpoint_payload = checkpoint_resp.json()
        assert checkpoint_payload["session_id"] == session["session_id"]

        checkpoints_resp = client.get(
            f"/v1/native/repositories/repo-a/sessions/{session['session_local_id']}/checkpoints"
        )
        assert checkpoints_resp.status_code == 200
        checkpoints_payload = checkpoints_resp.json()
        assert checkpoints_payload[0]["checkpoint_id"] == checkpoint_payload["checkpoint_id"]

        close_resp = client.post(
            f"/v1/native/repositories/repo-a/sessions/{session['session_local_id']}:close",
            json={"status": "paused"},
        )
        assert close_resp.status_code == 200
        assert close_resp.json()["status"] == "paused"

        resume_resp = client.post(
            f"/v1/native/repositories/repo-a/sessions/{session['session_local_id']}:resume",
            json={"limit": 20},
        )
        assert resume_resp.status_code == 200
        assert resume_resp.json()["session"]["session_id"] == session["session_id"]
        assert resume_resp.json()["session"]["status"] == "active"


def test_task_create_route_records_request_actor_on_task_created_event(tmp_path: Path, monkeypatch):
    data_dir = tmp_path / "server-data"
    ctx = fake_postgres_context(data_dir)
    server_store.initialize(ctx)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            "/v1/native/repositories/repo-a/tasks",
            headers={"X-AIT-Actor": "alice@example.com", "X-AIT-Actor-Type": "human"},
            json={"title": "Route task", "intent": "Verify task.created actor provenance", "risk_tier": "medium"},
        )

    assert response.status_code == 200
    task_id = response.json()["task_id"]
    with connect(ctx) as conn:
        event_row = conn.execute(
            """
            select actor_identity, actor_type
            from events
            where entity_type = 'task' and entity_id = ? and event_type = 'task.created'
            order by event_id desc
            limit 1
            """,
            (task_id,),
        ).fetchone()
    assert event_row is not None
    assert event_row["actor_identity"] == "alice@example.com"
    assert event_row["actor_type"] == "human"


def test_task_create_route_returns_server_tracking_session(tmp_path: Path, monkeypatch):
    data_dir = tmp_path / "server-data-tracking-route"
    ctx = fake_postgres_context(data_dir)
    server_store.initialize(ctx)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            "/v1/native/repositories/repo-a/tasks",
            json={"title": "Route task", "intent": "Verify server-guaranteed tracking", "risk_tier": "medium"},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["tracking"]["session_id"].startswith("AAAS-")
    sessions = [
        row
        for row in server_store.list_sessions(ctx, "repo-a")
        if row["task_id"] == payload["task_id"] and row["session_kind"] == "task_run"
    ]
    assert len(sessions) == 1
    assert sessions[0]["session_id"] == payload["tracking"]["session_id"]
    assert sessions[0]["metadata"]["objective"] == "Verify server-guaranteed tracking"


def test_repo_scoped_task_dag_run_advance_route_injects_repo_name(tmp_path: Path, monkeypatch):
    data_dir = tmp_path / "server-data-task-dag-run"
    ctx = fake_postgres_context(data_dir)
    server_store.initialize(ctx)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")

    repo_name = "repo-a"
    server_store.ensure_repository(ctx, repo_name, "main", id_namespace_prefix="AAA")
    plan = server_store.create_plan(
        ctx,
        repo_name,
        "Repo scoped task DAG run plan",
        "docs/sprints/repo_a.md",
        None,
        "Repo scoped task DAG run plan",
        [],
        summary="seed",
        artifact_body="# Repo scoped plan\n",
    )
    plan_id = plan["plan_id"]
    plan_revision_id = plan["head_revision"]["plan_revision_id"]
    graph = _repo_scoped_task_dag_graph(plan_id, plan_revision_id)
    session = server_store.create_session(
        ctx,
        repo_name,
        "task_graph_run",
        title="Task DAG execute: demo/task-dag",
        metadata={
            "session_policy": "task_dag_execute_run",
            "plan_id": plan_id,
            "plan_revision_id": plan_revision_id,
            "graph_id": graph["graph_id"],
        },
        session_id="S-RUN-1",
    )

    with TestClient(server_app.create_app()) as client:
        missing_repo_name_resp = client.post(
            f"/v1/native/task-dag-runs/{session['session_id']}:advance",
            json={"graph": graph},
        )
        assert missing_repo_name_resp.status_code == 400

        response = client.get(f"/v1/native/repositories/{repo_name}/sessions/{session['session_id']}")
        assert response.status_code == 200
        assert response.json()["session_id"] == session["session_id"]

        response = client.post(
            f"/v1/native/repositories/{repo_name}/task-dag-runs/{session['session_id']}:advance",
            json={"graph": graph},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["session_id"] == session["session_id"]
    assert payload["advanced"] is True
    assert payload["latest_state_snapshot"]["execution_state"] == "active"


def test_backfill_task_sessions_route_recovers_legacy_completed_task(tmp_path: Path, monkeypatch):
    data_dir = tmp_path / "server-data-backfill-route"
    ctx = fake_postgres_context(data_dir)
    server_store.initialize(ctx)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    task = server_store.create_task(ctx, "repo-a", "Legacy task", "Recover missing task_run session", "medium")
    with connect(ctx) as conn:
        conn.execute("delete from sessions where task_id = ? and session_kind = 'task_run'", (task["task_id"],))
        conn.execute("update tasks set status = 'completed' where task_id = ?", (task["task_id"],))
        conn.commit()

    with TestClient(server_app.create_app()) as client:
        response = client.post(
            "/v1/native/repositories/repo-a/tasks:backfill-sessions",
            json={"task_id": task["task_id"]},
        )

    assert response.status_code == 200
    payload = response.json()
    assert payload["missing_task_count"] == 1
    assert payload["created_session_count"] == 1
    sessions = [
        row
        for row in server_store.list_sessions(ctx, "repo-a")
        if row["task_id"] == task["task_id"] and row["session_kind"] == "task_run"
    ]
    assert len(sessions) == 1
    assert sessions[0]["status"] == "completed"
