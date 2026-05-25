from __future__ import annotations

import base64
import hashlib
import json
from pathlib import Path

from fastapi.testclient import TestClient

import ait_server.app as server_app
from ait_server import server_store
from ait_server.server_content import set_repository_lifecycle_state
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
    return ctx


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


def test_native_repository_routes_cover_repo_line_and_snapshot_contract(tmp_path: Path, monkeypatch):
    _server_ctx(tmp_path, monkeypatch)

    bundle = _snapshot_bundle(
        "repo-a",
        "SNP-REPO-ROUTES-1",
        parent_snapshot_id=None,
        line_name="main",
        message="initial",
        files={"README.md": b"hello\n"},
    )

    with TestClient(server_app.create_app()) as client:
        create_repo_resp = client.post(
            "/v1/native/repositories",
            json={"repo_name": "repo-a", "default_line": "main", "policy": {}, "id_namespace_prefix": "AAA"},
        )
        assert create_repo_resp.status_code == 200
        repo = create_repo_resp.json()
        assert repo["repo_name"] == "repo-a"
        assert repo["default_line"] == "main"

        put_snapshot_resp = client.put("/v1/native/repositories/repo-a/snapshots/SNP-REPO-ROUTES-1", json=bundle)
        assert put_snapshot_resp.status_code == 200
        snapshot = put_snapshot_resp.json()
        assert snapshot["snapshot_id"] == "SNP-REPO-ROUTES-1"

        exists_resp = client.post(
            "/v1/native/repositories/repo-a/snapshots:exists",
            json={"snapshot_ids": ["SNP-REPO-ROUTES-1", "SNP-MISSING"]},
        )
        assert exists_resp.status_code == 200
        assert exists_resp.json() == {
            "repo_name": "repo-a",
            "checked_snapshots": 2,
            "present": ["SNP-REPO-ROUTES-1"],
            "missing": ["SNP-MISSING"],
        }

        update_line_resp = client.put(
            "/v1/native/repositories/repo-a/lines/main",
            json={"head_snapshot_id": "SNP-REPO-ROUTES-1"},
        )
        assert update_line_resp.status_code == 200
        assert update_line_resp.json()["head_snapshot_id"] == "SNP-REPO-ROUTES-1"

        feature_line_resp = client.put(
            "/v1/native/repositories/repo-a/lines/feature/repo-routes",
            json={"head_snapshot_id": "SNP-REPO-ROUTES-1"},
        )
        assert feature_line_resp.status_code == 200
        assert feature_line_resp.json()["line_name"] == "feature/repo-routes"

        list_lines_resp = client.get("/v1/native/repositories/repo-a/lines")
        assert list_lines_resp.status_code == 200
        assert sorted(row["line_name"] for row in list_lines_resp.json()) == ["feature/repo-routes", "main"]

        get_line_resp = client.get("/v1/native/repositories/repo-a/lines/main")
        assert get_line_resp.status_code == 200
        assert get_line_resp.json()["head_snapshot_id"] == "SNP-REPO-ROUTES-1"

        get_snapshot_resp = client.get("/v1/native/repositories/repo-a/snapshots/SNP-REPO-ROUTES-1")
        assert get_snapshot_resp.status_code == 200
        assert get_snapshot_resp.json()["snapshot_id"] == "SNP-REPO-ROUTES-1"

        get_repo_resp = client.get("/v1/native/repositories/repo-a")
        assert get_repo_resp.status_code == 200
        assert get_repo_resp.json()["repo_name"] == "repo-a"

        close_line_resp = client.post(
            "/v1/native/repositories/repo-a/lines/feature/repo-routes:close",
            json={"status": "archived"},
        )
        assert close_line_resp.status_code == 200
        assert close_line_resp.json()["status"] == "archived"


def test_native_repository_snapshot_route_rejects_path_body_mismatch(tmp_path: Path, monkeypatch):
    _server_ctx(tmp_path, monkeypatch)

    with TestClient(server_app.create_app()) as client:
        client.post(
            "/v1/native/repositories",
            json={"repo_name": "repo-a", "default_line": "main", "policy": {}, "id_namespace_prefix": "AAA"},
        )
        mismatch_resp = client.put(
            "/v1/native/repositories/repo-a/snapshots/SNP-REPO-ROUTES-PATH",
            json={
                "snapshot_id": "SNP-REPO-ROUTES-BODY",
                "repo_name": "repo-a",
                "parent_snapshot_id": None,
                "line_name": "main",
                "message": "mismatch",
                "files": [],
            },
        )
        assert mismatch_resp.status_code == 400
        assert mismatch_resp.json()["detail"] == "snapshot_id path/body mismatch"


def test_admin_repository_retire_exports_and_purges_repo(tmp_path: Path, monkeypatch):
    ctx = _server_ctx(tmp_path, monkeypatch)
    export_root = tmp_path / "retired-repos"
    export_root.mkdir()
    monkeypatch.setenv("AIT_SERVER_RETIRE_EXPORT_ROOT", str(export_root))
    monkeypatch.setattr(server_app, "_RUNTIME_CTX", None)

    repo = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    bundle = _snapshot_bundle(
        "repo-a",
        "SNP-RETIRE-1",
        parent_snapshot_id=None,
        line_name="main",
        message="initial",
        files={"README.md": b"hello retire\n"},
    )
    server_store.import_snapshot(ctx, "repo-a", bundle)
    server_store.update_line(ctx, "repo-a", "main", "SNP-RETIRE-1")

    with TestClient(server_app.create_app()) as client:
        retire_resp = client.post(
            "/v1/native/admin/repositories/repo-a:retire",
            json={"expected_repo_id": repo["repo_id"], "require_verified_export": True},
        )
        assert retire_resp.status_code == 200, retire_resp.text
        payload = retire_resp.json()
        assert payload["repo_name"] == "repo-a"
        assert payload["queued"] is False
        result = payload["result"]
        assert result["repo_id"] == repo["repo_id"]
        assert result["verification"]["verified"] is True
        manifest_path = Path(result["manifest_path"])
        assert manifest_path.exists()
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        assert manifest["repo_name"] == "repo-a"
        assert manifest["snapshot_count"] == 1

        get_repo_resp = client.get("/v1/native/repositories/repo-a")
        assert get_repo_resp.status_code == 404

    with server_store.connect(ctx) as conn:
        retirement_row = conn.execute(
            "select * from repository_retirements where repo_id = ?",
            (repo["repo_id"],),
        ).fetchone()
    assert retirement_row is not None
    assert retirement_row["state"] == "purged"


def test_admin_repository_retire_rejects_cross_repo_pack_dependencies(tmp_path: Path, monkeypatch):
    ctx = _server_ctx(tmp_path, monkeypatch)
    export_root = tmp_path / "retired-repos"
    export_root.mkdir()
    monkeypatch.setenv("AIT_SERVER_RETIRE_EXPORT_ROOT", str(export_root))
    monkeypatch.setattr(server_app, "_RUNTIME_CTX", None)

    repo_a = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    repo_b = server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="BBB")

    shared_files = {"README.md": b"shared packed blob\n"}
    server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-RETIRE-SHARED-A",
            parent_snapshot_id=None,
            line_name="main",
            message="shared",
            files=shared_files,
        ),
    )
    server_store.update_line(ctx, "repo-a", "main", "SNP-RETIRE-SHARED-A")
    server_store.pack_repository_storage(ctx, "repo-a", repack=True)

    server_store.import_snapshot(
        ctx,
        "repo-b",
        _snapshot_bundle(
            "repo-b",
            "SNP-RETIRE-SHARED-B",
            parent_snapshot_id=None,
            line_name="main",
            message="shared",
            files=shared_files,
        ),
    )
    server_store.update_line(ctx, "repo-b", "main", "SNP-RETIRE-SHARED-B")

    with TestClient(server_app.create_app()) as client:
        retire_resp = client.post(
            "/v1/native/admin/repositories/repo-a:retire",
            json={"expected_repo_id": repo_a["repo_id"], "require_verified_export": True},
        )
        assert retire_resp.status_code == 400
        assert "shared_pack_refs=1" in retire_resp.json()["detail"]

        get_repo_resp = client.get("/v1/native/repositories/repo-a")
        assert get_repo_resp.status_code == 200
        assert get_repo_resp.json()["repo_id"] == repo_a["repo_id"]

    assert server_store.get_repository(ctx, "repo-b")["repo_id"] == repo_b["repo_id"]


def test_retiring_repository_blocks_mutating_routes_but_keeps_reads(tmp_path: Path, monkeypatch):
    ctx = _server_ctx(tmp_path, monkeypatch)
    monkeypatch.setattr(server_app, "_RUNTIME_CTX", None)
    repo = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    set_repository_lifecycle_state(ctx, "repo-a", "retiring", expected_repo_id=repo["repo_id"])

    with TestClient(server_app.create_app()) as client:
        get_repo_resp = client.get("/v1/native/repositories/repo-a")
        assert get_repo_resp.status_code == 200
        assert get_repo_resp.json()["lifecycle_state"] == "retiring"

        update_line_resp = client.put(
            "/v1/native/repositories/repo-a/lines/main",
            json={"head_snapshot_id": None},
        )
        assert update_line_resp.status_code == 409
        assert "does not accept" in update_line_resp.json()["detail"]
