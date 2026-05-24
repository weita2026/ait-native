from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

import ait_server.app as server_app
from ait_server import server_store
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


def _plan_create_payload() -> dict:
    return {
        "title": "Route-owned plan",
        "artifact_path": "docs/sprints/route_owned_plan.md",
        "artifact_heading": "Route Owned Plan",
        "items": [],
        "summary": "Seed the first route-owned sprint plan.",
        "artifact_body": "# Route Owned Plan\n",
    }


def _plan_revision_payload(*, expected_head_revision_id: str | None = None) -> dict:
    payload = {
        "title": "Route-owned revision",
        "artifact_path": "docs/sprints/route_owned_plan.md",
        "artifact_heading": "Route Owned Plan",
        "items": [],
        "summary": "Add another revision through the native planning routes.",
        "artifact_body": "# Route Owned Plan\n\nRevised.\n",
    }
    if expected_head_revision_id is not None:
        payload["expected_head_revision_id"] = expected_head_revision_id
    return payload


def test_native_planning_routes_put_revision_artifacts_and_join_relay_session(tmp_path: Path, monkeypatch):
    _server_ctx(tmp_path, monkeypatch)

    with TestClient(server_app.create_app()) as client:
        create_resp = client.post("/v1/native/repositories/repo-a/sprints", json=_plan_create_payload())
        assert create_resp.status_code == 200
        created_plan = create_resp.json()
        plan_id = created_plan["plan_id"]
        head_revision_id = created_plan["head_revision_id"]

        artifacts_resp = client.put(
            f"/v1/native/sprints/{plan_id}/revisions/{head_revision_id}/artifacts",
            json={
                "artifacts": [
                    {
                        "artifact_path": "docs/sprints/route_owned_plan.task_graph.json",
                        "role": "supporting_artifact",
                        "media_type": "application/json",
                        "encoding": "utf-8",
                        "body": '{\"graph_id\":\"planning-routes/demo\"}',
                        "metadata": {"kind": "task_graph"},
                    }
                ]
            },
        )
        assert artifacts_resp.status_code == 200
        artifacts_payload = artifacts_resp.json()
        assert artifacts_payload["plan_id"] == plan_id
        assert artifacts_payload["plan_revision_id"] == head_revision_id
        assert artifacts_payload["artifacts"][0]["artifact_path"] == "docs/sprints/route_owned_plan.task_graph.json"

        planning_session_resp = client.post(
            f"/v1/native/sprints/{plan_id}/planning-sessions",
            json={"title": "Route planning relay", "preferred_agent": "codex", "resume_if_active": True},
        )
        assert planning_session_resp.status_code == 200
        planning_session = planning_session_resp.json()
        planning_session_id = planning_session["planning_session_id"]

        join_resp = client.post(
            f"/v1/native/planning-sessions/{planning_session_id}:join",
            json={"surface": "cli", "title": "Route relay session", "model_name": "gpt-5-codex"},
        )
        assert join_resp.status_code == 200
        joined_payload = join_resp.json()
        assert joined_payload["planning_session"]["planning_session_id"] == planning_session_id
        assert joined_payload["session"]["session_kind"] == "planning_session_relay"
        assert joined_payload["session"]["metadata"]["planning_session_id"] == planning_session_id
        assert joined_payload["session"]["metadata"]["plan_id"] == plan_id


def test_native_planning_revise_route_maps_stale_expected_head_to_conflict(tmp_path: Path, monkeypatch):
    _server_ctx(tmp_path, monkeypatch)

    with TestClient(server_app.create_app()) as client:
        create_resp = client.post("/v1/native/repositories/repo-a/sprints", json=_plan_create_payload())
        assert create_resp.status_code == 200
        head_revision_id = create_resp.json()["head_revision_id"]
        plan_id = create_resp.json()["plan_id"]

        first_revise_resp = client.post(
            f"/v1/native/sprints/{plan_id}/revisions",
            json=_plan_revision_payload(expected_head_revision_id=head_revision_id),
        )
        assert first_revise_resp.status_code == 200
        assert first_revise_resp.json()["head_revision"]["parent_plan_revision_id"] == head_revision_id

        stale_revise_resp = client.post(
            f"/v1/native/sprints/{plan_id}/revisions",
            json=_plan_revision_payload(expected_head_revision_id=head_revision_id),
        )
        assert stale_revise_resp.status_code == 409
        assert "head advanced" in stale_revise_resp.json()["detail"]
