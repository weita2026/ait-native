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


def _create_change(ctx: ServerContext, repo_name: str, suffix: str) -> dict:
    task = server_store.create_task(
        ctx,
        repo_name,
        f"Task {suffix}",
        f"Route-owned task {suffix}",
        "medium",
    )
    return server_store.create_change(
        ctx,
        repo_name,
        task["task_id"],
        f"Change {suffix}",
        "main",
        "medium",
    )


def test_native_stack_routes_cover_create_update_add_reorder_and_graph(tmp_path: Path, monkeypatch):
    ctx = _server_ctx(tmp_path, monkeypatch)
    change_a = _create_change(ctx, "repo-a", "A")
    change_b = _create_change(ctx, "repo-a", "B")

    with TestClient(server_app.create_app()) as client:
        create_resp = client.post(
            "/v1/native/repositories/repo-a/stacks",
            json={"title": "Route stack", "change_ids": [change_a["change_id"]], "landing_policy": "ordered"},
        )
        assert create_resp.status_code == 200
        stack = create_resp.json()
        stack_id = stack["stack_id"]
        assert stack["change_ids"] == [change_a["change_id"]]

        add_resp = client.post(
            f"/v1/native/stacks/{stack_id}:addChange",
            json={"change_id": change_b["change_id"], "position": 2},
        )
        assert add_resp.status_code == 200
        assert add_resp.json()["change_ids"] == [change_a["change_id"], change_b["change_id"]]

        reorder_resp = client.post(
            f"/v1/native/stacks/{stack_id}:reorderChange",
            json={"change_id": change_b["change_id"], "position": 1},
        )
        assert reorder_resp.status_code == 200
        assert reorder_resp.json()["change_ids"] == [change_b["change_id"], change_a["change_id"]]

        update_resp = client.patch(
            f"/v1/native/stacks/{stack_id}",
            json={"title": "Route stack updated", "landing_policy": "manual", "status": "active"},
        )
        assert update_resp.status_code == 200
        assert update_resp.json()["title"] == "Route stack updated"
        assert update_resp.json()["landing_policy"] == "manual"

        list_resp = client.get("/v1/native/repositories/repo-a/stacks")
        assert list_resp.status_code == 200
        assert [row["stack_id"] for row in list_resp.json()] == [stack_id]

        graph_resp = client.get(f"/v1/native/stacks/{stack_id}/graph")
        assert graph_resp.status_code == 200
        graph = graph_resp.json()
        assert [node["change_id"] for node in graph["nodes"]] == [change_b["change_id"], change_a["change_id"]]
        assert graph["edges"] == [
            {
                "from_change_id": change_b["change_id"],
                "to_change_id": change_a["change_id"],
                "kind": "ordered",
            }
        ]


def test_native_stack_reorder_route_requires_position(tmp_path: Path, monkeypatch):
    ctx = _server_ctx(tmp_path, monkeypatch)
    change_a = _create_change(ctx, "repo-a", "A")
    stack = server_store.create_stack(ctx, "repo-a", "Route stack", [change_a["change_id"]])

    with TestClient(server_app.create_app()) as client:
        reorder_resp = client.post(
            f"/v1/native/stacks/{stack['stack_id']}:reorderChange",
            json={"change_id": change_a["change_id"]},
        )
        assert reorder_resp.status_code == 400
        assert reorder_resp.json()["detail"] == "position is required for reorder"
