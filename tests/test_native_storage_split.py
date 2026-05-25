from __future__ import annotations

import hashlib
import json
import os
import socket
import sqlite3
import threading
import time
import urllib.request
from contextlib import contextmanager
from pathlib import Path

import pytest
import uvicorn
from fastapi.testclient import TestClient
from typer.testing import CliRunner

import ait_native.server as native_server_module
import ait_native.server_db as native_server_db
import ait_native.server_store as native_server_store
from ait_native.cli import app
from ait_native.packfiles import build_pack_members, write_pack_archive
from ait_native.server import create_app
from ait_native.server_paths import ServerContext
from ait_native.server_store import initialize as initialize_server_store
from tests.postgres_fake import (
    fake_postgres_context,
    fake_postgres_dsn,
    fake_postgres_schema_db_path,
    install_fake_psycopg_global,
    reset_fake_postgres_runtime,
)

runner = CliRunner()


def _sqlite_object_type(db_path: Path, name: str) -> str | None:
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    row = conn.execute(
        "select type from sqlite_master where name = ? and type in ('table', 'view')",
        (name,),
    ).fetchone()
    conn.close()
    return row["type"] if row else None


def _sqlite_table_columns(db_path: Path, table_name: str) -> set[str]:
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    rows = conn.execute(f"pragma table_info({table_name})").fetchall()
    conn.close()
    return {str(row["name"]) for row in rows if row["name"] is not None}


@contextmanager
def running_server(data_dir: Path):
    old = os.environ.get("AIT_NATIVE_SERVER_DATA")
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
        if old is None:
            os.environ.pop("AIT_NATIVE_SERVER_DATA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DATA"] = old
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


def test_server_app_initializes_runtime_once_per_instance(tmp_path: Path, monkeypatch):
    data_dir = tmp_path / "server-data"
    install_fake_psycopg_global()
    reset_fake_postgres_runtime()
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")
    monkeypatch.setattr(native_server_module, "_RUNTIME_CTX", None)

    calls: list[Path] = []
    real_initialize = native_server_module.initialize

    def counted_initialize(ctx):
        calls.append(ctx.root)
        return real_initialize(ctx)

    monkeypatch.setattr(native_server_module, "initialize", counted_initialize)

    with TestClient(native_server_module.create_app()) as client:
        assert client.get("/healthz").status_code == 200
        assert client.get("/healthz").status_code == 200

    assert calls == [data_dir.resolve()]
    reset_fake_postgres_runtime()


def test_sqlite_runtime_initialization_is_rejected_without_creating_db_files(tmp_path: Path, monkeypatch):
    server_data = tmp_path / "server-data"
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(server_data))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "sqlite")
    monkeypatch.setenv("AIT_NATIVE_SQLITE_BUSY_TIMEOUT_MS", "4321")
    monkeypatch.delenv("AIT_NATIVE_SERVER_POSTGRES_DSN", raising=False)
    monkeypatch.delenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", raising=False)
    monkeypatch.delenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", raising=False)

    with pytest.raises(RuntimeError, match="sqlite is no longer supported"):
        ServerContext.from_env()

    assert not (server_data / "content.db").exists()
    assert not (server_data / "control.db").exists()


def test_attestations_use_full_patchset_id_for_unique_ids(tmp_path: Path, monkeypatch):
    server_data = tmp_path / "server-data"
    ctx = fake_postgres_context(server_data)
    initialize_server_store(ctx)

    seed_conn = native_server_db.connect_server_plane(ctx, "control")
    now = "2026-04-26T00:00:00+00:00"
    try:
        for patchset_id, change_id in (("AITP-0001-1", "AITC-0001"), ("ACCP-0001-1", "ACCC-0001")):
            seed_conn.execute(
                "insert into patchsets(patchset_id, change_id, patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (patchset_id, change_id, 1, "SNP-BASE", "SNP-REV", "seed patchset", "human_only", "published", "{}", "pending", now),
            )
        seed_conn.commit()
    finally:
        seed_conn.close()

    first = native_server_store.upsert_attestation(
        ctx,
        "AITP-0001-1",
        "human_only",
        {"tests": "pass"},
        {"policy_readable": True},
    )
    second = native_server_store.upsert_attestation(
        ctx,
        "ACCP-0001-1",
        "human_only",
        {"tests": "pass"},
        {"policy_readable": True},
    )

    assert first["attestation_id"] == "AT-AITP-0001-1"
    assert second["attestation_id"] == "AT-ACCP-0001-1"


def test_local_repo_uses_split_content_and_control_layout(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout

    ait_dir = repo / ".ait"
    assert (ait_dir / "content.db").exists()
    assert (ait_dir / "control.db").exists()
    assert (ait_dir / "objects" / "packs").is_dir()
    assert (ait_dir / "refs" / "lines").is_dir()

    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)

    status_out = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["content_db_path"].endswith("content.db")
    assert status["control_db_path"].endswith("control.db")
    assert status["pack_count"] == 1
    assert status["packed_blob_count"] >= 1

    ref_files = list((ait_dir / "refs" / "lines").iterdir())
    assert len(ref_files) == 1
    assert ref_files[0].read_text(encoding="utf-8").strip() == snapshot["snapshot_id"]


def test_init_repo_creates_blank_governance_bootstrap_docs(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-bootstrap"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout

    plan_path = repo / "docs" / "plan.md"
    milestone_path = repo / "docs" / "milestone.md"
    strategy_path = repo / "docs" / "STRATEGY_INDEX.md"

    assert plan_path.is_file()
    assert milestone_path.is_file()
    assert plan_path.read_text(encoding="utf-8").strip() == ""
    assert milestone_path.read_text(encoding="utf-8").strip() == ""
    assert not strategy_path.exists()


def test_init_repo_preserves_existing_governance_documents(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-bootstrap-preserve"
    (repo / "docs").mkdir(parents=True)
    (repo / "docs" / "plan.md").write_text("# Existing plan\n", encoding="utf-8")
    (repo / "docs" / "milestone.md").write_text("# Existing milestone\n", encoding="utf-8")
    repo.mkdir(exist_ok=True)
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout

    assert (repo / "docs" / "plan.md").read_text(encoding="utf-8") == "# Existing plan\n"
    assert (repo / "docs" / "milestone.md").read_text(encoding="utf-8") == "# Existing milestone\n"


def test_server_uses_split_content_control_and_file_backed_refs(tmp_path: Path, monkeypatch):
    repo = tmp_path / "repo"
    repo.mkdir()
    (repo / "app.py").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data") as (base_url, server_data):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        snapshot = json.loads(snap_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        assert not (server_data / "content.db").exists()
        assert not (server_data / "control.db").exists()
        assert fake_postgres_schema_db_path(server_data, "ait_native_content").exists()
        assert fake_postgres_schema_db_path(server_data, "ait_native_control").exists()
        ref_root = server_data / "refs"
        ref_files = list(ref_root.rglob("*"))
        line_refs = [p for p in ref_files if p.is_file()]
        assert line_refs, "expected at least one server-side line ref file"
        assert any(snapshot["snapshot_id"] in p.read_text(encoding="utf-8") for p in line_refs)


def test_local_snapshot_uses_tree_shared_metadata_view(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-tree-local"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-tree-local"], catch_exceptions=False).exit_code == 0
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)

    content_db = repo / ".ait" / "content.db"
    assert _sqlite_object_type(content_db, "snapshot_files") == "view"

    conn = sqlite3.connect(content_db)
    conn.row_factory = sqlite3.Row
    tree_columns = _sqlite_table_columns(content_db, "trees")
    blob_columns = _sqlite_table_columns(content_db, "blobs")
    snap_row = conn.execute(
        "select snapshot_id, root_tree_id, manifest_hash, manifest_path from snapshots where snapshot_id = ?",
        (snapshot["snapshot_id"],),
    ).fetchone()
    tree_row = conn.execute(
        "select tree_pack_id, tree_pack_checksum from trees where tree_id = ?",
        (snap_row["root_tree_id"],),
    ).fetchone()
    tree_pack_rows = conn.execute("select pack_id, pack_path from tree_packs order by created_at").fetchall()
    tree_rows = conn.execute("select tree_id, entry_count from trees order by tree_id").fetchall()
    file_rows = conn.execute("select path, blob_id from snapshot_files where snapshot_id = ? order by path", (snapshot["snapshot_id"],)).fetchall()
    conn.close()

    assert snap_row is not None
    assert snap_row["root_tree_id"]
    assert snap_row["manifest_hash"]
    assert "#trees/" in snap_row["manifest_path"]
    assert snap_row["manifest_path"].endswith(f"{snap_row['root_tree_id']}.json")
    assert tree_row is not None
    assert tree_row["tree_pack_id"]
    assert tree_row["tree_pack_checksum"]
    assert "tree_pack_entry_name" not in tree_columns
    assert "tree_packed_at" not in tree_columns
    assert "pack_entry_name" not in blob_columns
    assert "packed_at" not in blob_columns
    assert tree_pack_rows
    assert any(row["pack_id"] == tree_row["tree_pack_id"] for row in tree_pack_rows)
    assert tree_rows
    assert [row["path"] for row in file_rows] == ["app.py"]
    assert list((repo / ".ait" / "objects" / "manifests").glob("*.json")) == []


def test_server_snapshot_import_uses_tree_shared_metadata_view(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-tree-server"
    repo.mkdir()
    (repo / "app.py").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-tree-data") as (base_url, server_data):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-tree-server"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-tree-server", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        snapshot = json.loads(snap_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        content_db = fake_postgres_schema_db_path(server_data, "ait_native_content")
        assert _sqlite_object_type(content_db, "snapshot_files") == "view"

        conn = sqlite3.connect(content_db)
        conn.row_factory = sqlite3.Row
        snap_row = conn.execute(
            "select snapshot_id, root_tree_id, manifest_path from snapshots where snapshot_id = ?",
            (snapshot["snapshot_id"],),
        ).fetchone()
        tree_row = conn.execute(
            "select tree_pack_id, tree_pack_entry_name, tree_pack_checksum from trees where tree_id = ?",
            (snap_row["root_tree_id"],),
        ).fetchone()
        tree_pack_rows = conn.execute("select pack_id, pack_path from tree_packs order by created_at").fetchall()
        file_rows = conn.execute("select path from snapshot_files where snapshot_id = ? order by path", (snapshot["snapshot_id"],)).fetchall()
        conn.close()

        assert snap_row is not None
        assert snap_row["root_tree_id"]
        assert "#trees/" in snap_row["manifest_path"]
        assert snap_row["manifest_path"].endswith(f"{snap_row['root_tree_id']}.json")
        assert tree_row is not None
        assert tree_row["tree_pack_id"]
        assert tree_row["tree_pack_entry_name"] in (None, f"trees/{snap_row['root_tree_id']}.json")
        assert tree_row["tree_pack_checksum"]
        assert tree_pack_rows
        assert [row["path"] for row in file_rows] == ["app.py"]
        assert list((server_data / "objects" / "manifests").glob("*.json")) == []


def test_local_snapshot_ignores_repo_root_ait_server_runtime(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    (repo / ".ait-server").mkdir()
    (repo / ".ait-server" / "content.db").write_text("runtime-data\n", encoding="utf-8")
    (repo / ".ait-server" / "refs").mkdir()
    (repo / ".ait-server" / "refs" / "main").write_text("SNP-EXAMPLE\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout

    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)

    show_out = runner.invoke(app, ["snapshot", "show", snapshot["snapshot_id"], "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    body = json.loads(show_out.stdout)
    paths = [row["path"] for row in body["files"]]
    assert paths == ["app.py"]


def test_local_snapshot_ignores_repo_root_aitignore_patterns(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-aitignore"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    (repo / ".aitignore").write_text("local-secrets/.env\n", encoding="utf-8")
    (repo / "local-secrets").mkdir()
    (repo / "local-secrets" / ".env").write_text("secret\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout

    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)

    show_out = runner.invoke(app, ["snapshot", "show", snapshot["snapshot_id"], "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    body = json.loads(show_out.stdout)
    paths = [row["path"] for row in body["files"]]
    assert paths == [".aitignore", "app.py"]
    assert snapshot["ignore_policy"]["rule_files"] == [".aitignore"]
    assert snapshot["ignore_policy"]["custom_patterns"] == ["local-secrets/.env"]


def test_local_snapshot_ignores_repo_local_runtime_root_from_env(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-env-runtime"
    repo.mkdir()
    runtime_root = repo / "var" / "server-data"
    runtime_root.mkdir(parents=True)
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    (runtime_root / "content.db").write_text("runtime-data\n", encoding="utf-8")
    (runtime_root / "refs").mkdir()
    (runtime_root / "refs" / "main").write_text("SNP-EXAMPLE\n", encoding="utf-8")
    monkeypatch.chdir(repo)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", "var/server-data")

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout

    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)

    show_out = runner.invoke(app, ["snapshot", "show", snapshot["snapshot_id"], "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    body = json.loads(show_out.stdout)
    paths = [row["path"] for row in body["files"]]
    assert paths == ["app.py"]

    assert snapshot["ignore_policy"]["runtime_roots"] == ["var/server-data"]
    assert snapshot["ignore_policy"]["operational_roots"] == [".ait", ".ait-server", "var/server-data"]


def test_local_snapshot_ignores_repo_local_runtime_root_from_env(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-xdg-runtime"
    repo.mkdir()
    runtime_root = repo / "var" / "state" / "ait" / "server-data"
    runtime_root.mkdir(parents=True)
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    (runtime_root / "content.db").write_text("runtime-data\n", encoding="utf-8")
    (runtime_root / "logs").mkdir()
    (runtime_root / "logs" / "server.log").write_text("runtime-log\n", encoding="utf-8")
    monkeypatch.chdir(repo)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", "var/state/ait/server-data")

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout

    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)

    show_out = runner.invoke(app, ["snapshot", "show", snapshot["snapshot_id"], "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    body = json.loads(show_out.stdout)
    paths = [row["path"] for row in body["files"]]
    assert paths == ["app.py"]
    assert snapshot["ignore_policy"]["runtime_roots"] == ["var/state/ait/server-data"]


def test_workspace_status_ignores_repo_local_runtime_root_from_env(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-env-dirty"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout

    runtime_root = repo / "var" / "server-data"
    runtime_root.mkdir(parents=True)
    (runtime_root / "content.db").write_text("runtime-data\n", encoding="utf-8")
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", "var/server-data")

    status_out = runner.invoke(app, ["workspace", "status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["clean"] is True
    assert status["changed_paths"] == []
    assert status["ignore_policy"]["runtime_roots"] == ["var/server-data"]
    assert status["ignore_policy"]["operational_roots"] == [".ait", ".ait-server", "var/server-data"]


def test_workspace_status_ignores_repo_local_aitignore_patterns(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-aitignore-dirty"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    (repo / ".aitignore").write_text("local-secrets/.env\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout

    (repo / "local-secrets").mkdir()
    (repo / "local-secrets" / ".env").write_text("secret\n", encoding="utf-8")

    status_out = runner.invoke(app, ["workspace", "status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["clean"] is True
    assert status["changed_paths"] == []
    assert status["ignore_policy"]["rule_files"] == [".aitignore"]
    assert status["ignore_policy"]["custom_patterns"] == ["local-secrets/.env"]
    assert status["phase_timings_ms"]["workspace_scan"] >= 0
    assert status["phase_timings_ms"]["ignore_filtering"] >= 0
    assert status["phase_timings_ms"]["hashing"] >= 0
    assert status["phase_timings_ms"]["compare_manifest"] >= 0
    assert status["phase_timings_ms"]["total"] >= 0


def test_workspace_status_ignores_local_deploy_runtime_artifacts_from_aitignore(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-aitignore-deploy-runtime"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    (repo / ".aitignore").write_text(
        "deploy/site/.env\n"
        "deploy/site/macos-nginx/site.env\n"
        "deploy/site/nginx-rendered/\n",
        encoding="utf-8",
    )
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout

    (repo / "deploy" / "site").mkdir(parents=True)
    (repo / "deploy" / "site" / ".env").write_text("SITE_ENV=local\n", encoding="utf-8")
    (repo / "deploy" / "site" / "macos-nginx").mkdir(parents=True)
    (repo / "deploy" / "site" / "macos-nginx" / "site.env").write_text("SITE_DOMAIN=ait-native.dev\n", encoding="utf-8")
    (repo / "deploy" / "site" / "nginx-rendered").mkdir(parents=True)
    (repo / "deploy" / "site" / "nginx-rendered" / "official-site-http.conf").write_text("server {}\n", encoding="utf-8")

    status_out = runner.invoke(app, ["workspace", "status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["clean"] is True
    assert status["changed_paths"] == []
    assert status["ignore_policy"]["rule_files"] == [".aitignore"]
    assert status["ignore_policy"]["custom_patterns"] == [
        "deploy/site/.env",
        "deploy/site/macos-nginx/site.env",
        "deploy/site/nginx-rendered/",
    ]


def test_snapshot_create_json_exposes_phase_timings_for_ignore_policy_runs(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-phase-timing-ignore-policy"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    (repo / ".aitignore").write_text("local-secrets/.env\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout

    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)

    assert snapshot["phase_timings_ms"]["workspace_scan"] >= 0
    assert snapshot["phase_timings_ms"]["ignore_filtering"] >= 0
    assert snapshot["phase_timings_ms"]["hashing"] >= 0
    assert snapshot["phase_timings_ms"]["pack_archive_write"]["total"] >= 0
    assert snapshot["phase_timings_ms"]["metadata_commit"] >= 0
    assert snapshot["phase_timings_ms"]["total"] >= 0


def test_repo_status_reports_ignored_operational_roots(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-status-ignore"
    repo.mkdir()
    runtime_root = repo / "var" / "server-data"
    runtime_root.mkdir(parents=True)
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    (runtime_root / "jobs").mkdir()
    (runtime_root / "jobs" / "job-0001.json").write_text("{\"state\":\"queued\"}\n", encoding="utf-8")
    monkeypatch.chdir(repo)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", "var/server-data")

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout

    status_out = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["workspace_dirty"] is False
    assert status["ignore_policy"]["runtime_roots"] == ["var/server-data"]
    assert status["ignore_policy"]["operational_roots"] == [".ait", ".ait-server", "var/server-data"]




def test_doctor_runtime_root_passes_for_external_root(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-runtime-doctor-external"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)
    monkeypatch.delenv("AIT_NATIVE_SERVER_DATA", raising=False)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout

    external_root = tmp_path / "external-server-data"
    out = runner.invoke(app, ["doctor", "runtime-root", "--server-data", str(external_root), "--json"], catch_exceptions=False)
    assert out.exit_code == 0, out.stdout
    payload = json.loads(out.stdout)
    assert payload["state"] == "pass"
    assert payload["recommended_action"] == "none"
    assert payload["inside_repo"] is False
    assert payload["protected_from_snapshots"] is True


def test_doctor_runtime_root_warns_for_repo_local_ignored_root(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-runtime-doctor-local"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    runtime_root = repo / "var" / "server-data"
    runtime_root.mkdir(parents=True)
    (runtime_root / "content.db").write_text("runtime-data\n", encoding="utf-8")
    monkeypatch.chdir(repo)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", "var/server-data")

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout

    out = runner.invoke(app, ["doctor", "runtime-root", "--json"], catch_exceptions=False)
    assert out.exit_code == 0, out.stdout
    payload = json.loads(out.stdout)
    assert payload["state"] == "warn"
    assert payload["recommended_action"] == "prefer_external_runtime_root"
    assert payload["inside_repo"] is True
    assert payload["runtime_root_relative_to_repo"] == "var/server-data"
    assert payload["snapshot_ignored"] is True
    assert payload["protected_from_snapshots"] is True
    assert payload["ignore_policy"]["runtime_roots"] == ["var/server-data"]


def test_runtime_root_equal_repo_root_is_fail_and_not_silently_ignored(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-runtime-doctor-root"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout

    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", ".")
    (repo / "content.db").write_text("runtime-data-in-root\n", encoding="utf-8")

    status_out = runner.invoke(app, ["workspace", "status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["clean"] is False
    assert "content.db" in status["untracked_paths"]
    assert status["ignore_policy"]["runtime_roots"] == []
    assert status["ignore_policy"]["operational_roots"] == [".ait", ".ait-server"]

    doctor_out = runner.invoke(app, ["doctor", "runtime-root", "--json"], catch_exceptions=False)
    assert doctor_out.exit_code == 0, doctor_out.stdout
    payload = json.loads(doctor_out.stdout)
    assert payload["state"] == "fail"
    assert payload["recommended_action"] == "move_runtime_root_outside_repo"
    assert payload["inside_repo"] is True
    assert payload["equals_repo_root"] is True
    assert payload["snapshot_ignored"] is False
    assert payload["protected_from_snapshots"] is False

def test_repo_status_ignores_repo_local_aitignore_patterns(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-status-aitignore"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    (repo / ".aitignore").write_text("local-secrets/.env\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout

    (repo / "local-secrets").mkdir()
    (repo / "local-secrets" / ".env").write_text("secret\n", encoding="utf-8")

    status_out = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["workspace_dirty"] is False
    assert status["workspace_changed_count"] == 0
    assert status["ignore_policy"]["rule_files"] == [".aitignore"]
    assert status["ignore_policy"]["custom_patterns"] == ["local-secrets/.env"]
