from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

import ait_server.app as server_app
from ait_server import server_store
from ait_server.server_control import connect
from ait_server.server_paths import ServerContext
from tests.postgres_fake import fake_postgres_context, fake_postgres_dsn


def _server_ctx(tmp_path: Path, monkeypatch) -> ServerContext:
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
    return ctx


def test_native_task_routes_cover_create_list_get_close_and_restart(tmp_path: Path, monkeypatch):
    _server_ctx(tmp_path, monkeypatch)

    with TestClient(server_app.create_app()) as client:
        create_resp = client.post(
            "/v1/native/repositories/repo-a/tasks",
            json={"title": "Route task", "intent": "Exercise extracted task routes", "risk_tier": "medium"},
        )
        assert create_resp.status_code == 200
        created_task = create_resp.json()
        task_id = created_task["task_id"]
        assert created_task["tracking"]["session_id"].startswith("AAAS-")

        list_resp = client.get("/v1/native/repositories/repo-a/tasks")
        assert list_resp.status_code == 200
        assert [row["task_id"] for row in list_resp.json()] == [task_id]

        repo_get_resp = client.get(f"/v1/native/repositories/repo-a/tasks/{task_id}")
        assert repo_get_resp.status_code == 200
        assert repo_get_resp.json()["task_id"] == task_id

        global_get_resp = client.get(f"/v1/native/tasks/{task_id}")
        assert global_get_resp.status_code == 200
        assert global_get_resp.json()["task_id"] == task_id

        close_resp = client.post(f"/v1/native/tasks/{task_id}:close", json={"status": "canceled"})
        assert close_resp.status_code == 200
        closed_task = close_resp.json()
        assert closed_task["status"] == "canceled"
        assert closed_task["notification_followup"]["delivery"] == "background"

        restart_resp = client.post(f"/v1/native/tasks/{task_id}:restart")
        assert restart_resp.status_code == 200
        restarted_task = restart_resp.json()
        assert restarted_task["status"] == "active"
        assert restarted_task["notification_followup"]["delivery"] == "background"


def test_native_task_session_recovery_routes_cover_ensure_and_backfill(tmp_path: Path, monkeypatch):
    ctx = _server_ctx(tmp_path, monkeypatch)
    task = server_store.create_task(ctx, "repo-a", "Legacy task", "Recover missing task_run session", "medium")

    with connect(ctx) as conn:
        conn.execute("delete from sessions where task_id = ? and session_kind = 'task_run'", (task["task_id"],))
        conn.execute("update tasks set status = 'completed' where task_id = ?", (task["task_id"],))
        conn.commit()

    with TestClient(server_app.create_app()) as client:
        ensure_resp = client.post(
            f"/v1/native/repositories/repo-a/tasks/{task['task_id']}:ensure-session",
            json={},
        )
        assert ensure_resp.status_code == 200
        ensured_session = ensure_resp.json()
        assert ensured_session["task_id"] == task["task_id"]
        assert ensured_session["session_kind"] == "task_run"
        assert ensured_session["status"] == "completed"

        with connect(ctx) as conn:
            conn.execute("delete from sessions where session_id = ?", (ensured_session["session_id"],))
            conn.commit()

        backfill_resp = client.post(
            "/v1/native/repositories/repo-a/tasks:backfill-sessions",
            json={"task_id": task["task_id"]},
        )
        assert backfill_resp.status_code == 200
        payload = backfill_resp.json()
        assert payload["missing_task_count"] == 1
        assert payload["created_session_count"] == 1
        assert payload["created_sessions"][0]["task_id"] == task["task_id"]
        assert payload["created_sessions"][0]["session_status"] == "completed"
