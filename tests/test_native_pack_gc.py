from __future__ import annotations

import base64
import json
import os
import socket
import threading
import time
import urllib.request
import zipfile
from contextlib import contextmanager
from pathlib import Path

import pytest
import uvicorn
from typer.testing import CliRunner

from ait_native.cli import app
from ait_native.common import connect_sqlite
from ait_native.packfiles import PACK_FORMAT_V2, PACK_INDEX_ENTRY_NAME, read_pack_index
from ait_native.server_content import connect as connect_server_content
from ait_native.remote_client import get_change, get_patchset, get_remote_line, get_remote_snapshot
from ait_native.server import create_app
from ait_native.server_store import export_snapshot as export_remote_snapshot, initialize
from ait_native.store import RepoContext, export_snapshot_bundle
from ait_native.worker import process_one
from ait_native.common import utc_now
from tests.postgres_fake import (
    fake_postgres_context,
    fake_postgres_dsn,
    install_fake_psycopg_global,
    reset_fake_postgres_runtime,
)

runner = CliRunner()


@contextmanager
def running_server(
    data_dir: Path,
    auth_mode: str = "open",
    queue_mode: str = "async",
):
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



def _drain_jobs(data_dir: Path, limit: int = 20) -> int:
    ctx = fake_postgres_context(data_dir)
    initialize(ctx)
    processed = 0
    for _ in range(limit):
        job = process_one(ctx, worker_id="test-worker")
        if job is None:
            break
        processed += 1
    return processed


def _seed_orphan_tree_metadata(conn, *, root_tree_id: str = "TRE-ORPHAN-ROOT", child_tree_id: str = "TRE-ORPHAN-CHILD") -> None:
    created_at = utc_now()
    conn.execute("insert into trees(tree_id, entry_count, created_at) values (?, ?, ?)", (root_tree_id, 1, created_at))
    conn.execute("insert into trees(tree_id, entry_count, created_at) values (?, ?, ?)", (child_tree_id, 1, created_at))
    conn.execute(
        "insert into tree_entries(tree_id, entry_name, entry_type, target_id, mode) values (?, ?, ?, ?, ?)",
        (root_tree_id, "nested", "tree", child_tree_id, "tree"),
    )
    conn.execute(
        "insert into tree_entries(tree_id, entry_name, entry_type, target_id, mode) values (?, ?, ?, ?, ?)",
        (child_tree_id, "app.py", "blob", "BLB-ORPHAN-TREE", "0o644"),
    )


def _write_remote_plan(repo: Path, filename: str, *, plan_ref: str, item_ref: str, title: str) -> tuple[str, str]:
    plan_dir = repo / "docs" / "sprints"
    plan_dir.mkdir(parents=True, exist_ok=True)
    plan_file = plan_dir / filename
    plan_file.write_text(
        f"# {title}\n\n## Workflow [plan-ref: {plan_ref}]\n\n- [ ] {title} [ref: {item_ref}]\n",
        encoding="utf-8",
    )
    sync_out = runner.invoke(app, ["plan", "sync", str(plan_file), "--remote", "origin", "--json"], catch_exceptions=False)
    assert sync_out.exit_code == 0, sync_out.stdout
    synced = json.loads(sync_out.stdout)
    publish = synced["publish_results"][0]
    return publish["plan_id"], publish["published_head_revision_id"]



def test_local_pack_and_prune_preserves_snapshot_export(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    readme = repo / "app.py"
    readme.write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)
    initial_stats_out = runner.invoke(app, ["gc", "stats", "--json"], catch_exceptions=False)
    assert initial_stats_out.exit_code == 0, initial_stats_out.stdout
    initial_stats = json.loads(initial_stats_out.stdout)
    assert initial_stats["validation_summary"]["state"] == "packed_full_only"
    assert initial_stats["validation_summary"]["recommended_action"] == "repack"
    assert initial_stats["validation_summary"]["next_actions"] == ["repack"]

    pack_out = runner.invoke(app, ["gc", "pack", "--repack", "--json"], catch_exceptions=False)
    assert pack_out.exit_code == 0, pack_out.stdout
    packed = json.loads(pack_out.stdout)
    assert packed["created"] is True
    assert packed["stats"]["pack_count"] >= 1
    assert packed["stats"]["packed_blob_count"] >= 1
    assert packed["stats"]["packed_full_blob_count"] >= 1
    assert packed["stats"]["packed_delta_blob_count"] == 0
    assert packed["stats"]["validation_summary"]["state"] == "packed_full_only"
    assert packed["stats"]["validation_summary"]["recommended_action"] == "repack"
    assert packed["stats"]["validation_summary"]["next_actions"] == ["repack"]

    ctx = RepoContext.discover(repo)
    conn = connect_sqlite(ctx.content_db_path)
    blob_columns = {row["name"] for row in conn.execute("pragma table_info(blobs)")}
    blob_rows = [dict(row) for row in conn.execute("select pack_entry_type, pack_base_blob_id, pack_chain_depth from blobs order by blob_id")]
    conn.close()
    assert blob_rows
    assert "storage_kind" not in blob_columns
    assert all(row["pack_entry_type"] == "full" for row in blob_rows)
    assert all(row["pack_base_blob_id"] is None for row in blob_rows)
    assert all(row["pack_chain_depth"] == 0 for row in blob_rows)

    bundle = export_snapshot_bundle(ctx, snapshot["snapshot_id"])
    assert bundle["files"]
    assert bundle["files"][0]["path"] == "app.py"
    import base64
    restored = base64.b64decode(bundle["files"][0]["content_b64"]).decode("utf-8")
    assert restored == "base\n"

    pack_files = list((repo / ".ait" / "objects" / "packs").glob("*.zip"))
    assert pack_files, "expected a compacted pack archive"
    pack_index = read_pack_index(pack_files[0])
    assert pack_index["pack_format"] == PACK_FORMAT_V2
    assert pack_index["index_entry_name"] == PACK_INDEX_ENTRY_NAME
    assert pack_index["member_count"] >= 1
    assert any(entry["blob_id"] == bundle["files"][0]["blob_id"] for entry in pack_index["entries"])



def test_remote_pack_repack_and_gc_preserve_snapshot_access(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote"
    repo.mkdir()
    (repo / "app.py").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-pack-gc", queue_mode="async") as (base_url, data_dir):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        snapshot = json.loads(snap_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main", "--server-storage-mode", "pack_full"], catch_exceptions=False).exit_code == 0

        first_pack = runner.invoke(app, ["repo", "pack", "--repack", "--json"], catch_exceptions=False)
        assert first_pack.exit_code == 0, first_pack.stdout
        queued = json.loads(first_pack.stdout)
        assert queued["queued"] is True
        assert queued["job"]["job_type"] == "content.pack"
        duplicate_pack = runner.invoke(app, ["repo", "pack", "--repack", "--json"], catch_exceptions=False)
        assert duplicate_pack.exit_code == 0, duplicate_pack.stdout
        duplicate_payload = json.loads(duplicate_pack.stdout)
        assert duplicate_payload["queued"] is True
        assert int(duplicate_payload["job"]["job_id"]) == int(queued["job"]["job_id"])
        assert _drain_jobs(data_dir) >= 1

        second_pack = runner.invoke(app, ["repo", "pack", "--repack", "--json"], catch_exceptions=False)
        assert second_pack.exit_code == 0, second_pack.stdout
        queued2 = json.loads(second_pack.stdout)
        assert queued2["queued"] is True
        assert queued2["job"]["job_type"] == "content.pack"
        assert _drain_jobs(data_dir) >= 1

        storage_before_gc = runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False)
        assert storage_before_gc.exit_code == 0, storage_before_gc.stdout
        storage = json.loads(storage_before_gc.stdout)
        assert storage["pack_count"] >= 2
        assert storage["packed_blob_count"] >= 1
        assert storage["packed_full_blob_count"] >= 1
        assert storage["packed_delta_blob_count"] == 0
        assert storage["optimization_summary"]["tracked_blob_count"] >= 1
        assert storage["signals_summary"]["drift_count"] == 0
        assert all(pack["pack_format"] == PACK_FORMAT_V2 for pack in storage["packs"])
        assert all(pack["pack_index_entry_name"] == PACK_INDEX_ENTRY_NAME for pack in storage["packs"])
        assert all(pack["pack_index_checksum"] for pack in storage["packs"])
        ctx = fake_postgres_context(data_dir)
        conn = connect_server_content(ctx)
        blob_rows = [
            dict(row)
            for row in conn.execute(
                "select storage_kind, pack_entry_type, pack_base_blob_id, pack_chain_depth from blobs order by blob_id"
            )
        ]
        conn.close()
        assert blob_rows
        assert all(row["storage_kind"] == "pack_full" for row in blob_rows)
        assert all(row["pack_entry_type"] == "full" for row in blob_rows)
        assert all(row["pack_base_blob_id"] is None for row in blob_rows)
        assert all(row["pack_chain_depth"] == 0 for row in blob_rows)

        gc_out = runner.invoke(app, ["repo", "gc", "--json"], catch_exceptions=False)
        assert gc_out.exit_code == 0, gc_out.stdout
        queued_gc = json.loads(gc_out.stdout)
        assert queued_gc["queued"] is True
        assert queued_gc["job"]["job_type"] == "content.gc"
        assert _drain_jobs(data_dir) >= 1

        storage_after_gc_out = runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False)
        assert storage_after_gc_out.exit_code == 0, storage_after_gc_out.stdout
        storage_after_gc = json.loads(storage_after_gc_out.stdout)
        assert storage_after_gc["pack_count"] == 1
        assert storage_after_gc["packed_full_blob_count"] >= 1
        assert storage_after_gc["packed_delta_blob_count"] == 0
        assert storage_after_gc["signals_summary"]["drift_count"] == 0
        pack_path = data_dir / storage_after_gc["packs"][0]["pack_path"]
        pack_index = read_pack_index(pack_path)
        assert pack_index["pack_format"] == PACK_FORMAT_V2
        assert pack_index["index_entry_name"] == PACK_INDEX_ENTRY_NAME

        bundle = get_remote_snapshot(base_url, "housekeeper", snapshot["snapshot_id"])
        assert bundle["snapshot_id"] == snapshot["snapshot_id"]
        assert bundle["files"][0]["path"] == "app.py"
        assert bundle["files"][0]["content_b64"]


def test_remote_push_can_ingest_directly_as_pack_full(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-direct-pack-full"
    repo.mkdir()
    (repo / "app.py").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-direct-pack-full", queue_mode="inline") as (base_url, data_dir):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-remote-direct-pack-full"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-remote-direct-pack-full", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        snapshot = json.loads(snap_out.stdout)

        push_out = runner.invoke(
            app,
            ["push", "--line", "main", "--server-storage-mode", "pack_full", "--json"],
            catch_exceptions=False,
        )
        assert push_out.exit_code == 0, push_out.stdout
        pushed = json.loads(push_out.stdout)
        assert pushed["server_storage_mode"] == "pack_full"

        storage_out = runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False)
        assert storage_out.exit_code == 0, storage_out.stdout
        storage = json.loads(storage_out.stdout)
        assert storage["packed_blob_count"] >= 1
        assert storage["packed_full_blob_count"] >= 1
        assert storage["packed_delta_blob_count"] == 0
        assert storage["pack_count"] == 1
        assert storage["validation_summary"]["state"] == "packed_full_only"

        ctx = fake_postgres_context(data_dir)
        conn = connect_server_content(ctx)
        blob_rows = [
            dict(row)
            for row in conn.execute(
                "select storage_kind, pack_entry_type, pack_base_blob_id, pack_chain_depth from blobs order by blob_id"
            )
        ]
        conn.close()
        assert blob_rows
        assert all(row["storage_kind"] == "pack_full" for row in blob_rows)
        assert all(row["pack_entry_type"] == "full" for row in blob_rows)
        assert all(row["pack_base_blob_id"] is None for row in blob_rows)
        assert all(row["pack_chain_depth"] == 0 for row in blob_rows)
        assert list((data_dir / "objects" / "packs").glob("*.zip"))

        bundle = get_remote_snapshot(base_url, "housekeeper-remote-direct-pack-full", snapshot["snapshot_id"])
        assert bundle["snapshot_id"] == snapshot["snapshot_id"]
        assert bundle["files"][0]["path"] == "app.py"
        assert base64.b64decode(bundle["files"][0]["content_b64"]).decode("utf-8") == "base\n"


def test_remote_push_uses_server_default_pack_delta_ingest(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-server-default-pack-delta"
    repo.mkdir()
    (repo / "app.py").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-default-pack-delta", queue_mode="inline") as (base_url, data_dir):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-remote-server-default-pack-delta"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-remote-server-default-pack-delta", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        base_text = "".join(f"line {i:02d} keep same text for compression\n" for i in range(20))
        updated_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
        updated_lines[10] = "line 10 changed text for compression\n"
        updated_lines.append("line 20 keep same text for compression\n")
        updated_text = "".join(updated_lines)

        (repo / "app.py").write_text(base_text, encoding="utf-8")
        first_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
        assert first_out.exit_code == 0, first_out.stdout
        first_snapshot = json.loads(first_out.stdout)
        first_push = runner.invoke(app, ["push", "--line", "main", "--json"], catch_exceptions=False)
        assert first_push.exit_code == 0, first_push.stdout

        (repo / "app.py").write_text(updated_text, encoding="utf-8")
        second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
        assert second_out.exit_code == 0, second_out.stdout
        second_snapshot = json.loads(second_out.stdout)

        push_out = runner.invoke(app, ["push", "--line", "main", "--json"], catch_exceptions=False)
        assert push_out.exit_code == 0, push_out.stdout
        pushed = json.loads(push_out.stdout)
        assert pushed["server_storage_mode"] == "default"

        storage_out = runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False)
        assert storage_out.exit_code == 0, storage_out.stdout
        storage = json.loads(storage_out.stdout)
        assert storage["packed_blob_count"] >= 1
        assert storage["packed_delta_blob_count"] >= 1
        assert storage["packed_full_blob_count"] >= 1
        assert storage["pack_count"] >= 1

        ctx = fake_postgres_context(data_dir)
        conn = connect_server_content(ctx)
        first_blob_id = conn.execute(
            "select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'",
            (first_snapshot["snapshot_id"],),
        ).fetchone()["blob_id"]
        second_blob_row = conn.execute(
            """
            select blob_id, storage_kind, pack_entry_type, pack_base_blob_id, pack_chain_depth
            from blobs
            where blob_id = (
                select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'
            )
            """,
            (second_snapshot["snapshot_id"],),
        ).fetchone()
        conn.close()
        assert second_blob_row["storage_kind"] == "pack_delta"
        assert second_blob_row["pack_entry_type"] == "delta"
        assert second_blob_row["pack_base_blob_id"] == first_blob_id
        assert second_blob_row["pack_chain_depth"] == 1

        bundle = get_remote_snapshot(base_url, "housekeeper-remote-server-default-pack-delta", second_snapshot["snapshot_id"])
        assert bundle["snapshot_id"] == second_snapshot["snapshot_id"]


def test_remote_push_can_ingest_selective_pack_delta(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-direct-pack-delta"
    repo.mkdir()
    readme = repo / "app.py"

    with running_server(tmp_path / "server-direct-pack-delta", queue_mode="inline") as (base_url, data_dir):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-remote-direct-pack-delta"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-remote-direct-pack-delta", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        base_text = "".join(f"line {i:02d} keep same text for compression\n" for i in range(20))
        updated_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
        updated_lines[10] = "line 10 changed text for compression\n"
        updated_lines.append("line 20 keep same text for compression\n")
        updated_text = "".join(updated_lines)

        readme.write_text(base_text, encoding="utf-8")
        first_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
        assert first_out.exit_code == 0, first_out.stdout
        first_snapshot = json.loads(first_out.stdout)
        first_push = runner.invoke(
            app,
            ["push", "--line", "main", "--server-storage-mode", "pack_delta", "--json"],
            catch_exceptions=False,
        )
        assert first_push.exit_code == 0, first_push.stdout

        readme.write_text(updated_text, encoding="utf-8")
        second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
        assert second_out.exit_code == 0, second_out.stdout
        second_snapshot = json.loads(second_out.stdout)
        second_push = runner.invoke(
            app,
            ["push", "--line", "main", "--server-storage-mode", "pack_delta", "--json"],
            catch_exceptions=False,
        )
        assert second_push.exit_code == 0, second_push.stdout
        pushed = json.loads(second_push.stdout)
        assert pushed["server_storage_mode"] == "pack_delta"

        storage_out = runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False)
        assert storage_out.exit_code == 0, storage_out.stdout
        storage = json.loads(storage_out.stdout)
        assert storage["packed_blob_count"] >= 1
        assert storage["packed_full_blob_count"] >= 1
        assert storage["packed_delta_blob_count"] >= 1
        assert storage["validation_summary"]["has_delta_optimization"] is True

        ctx = fake_postgres_context(data_dir)
        conn = connect_server_content(ctx)
        first_blob_id = conn.execute(
            "select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'",
            (first_snapshot["snapshot_id"],),
        ).fetchone()["blob_id"]
        second_blob_row = conn.execute(
            """
            select blob_id, storage_kind, pack_entry_type, pack_base_blob_id, pack_chain_depth
            from blobs
            where blob_id = (
                select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'
            )
            """,
            (second_snapshot["snapshot_id"],),
        ).fetchone()
        conn.close()
        assert second_blob_row["storage_kind"] == "pack_delta"
        assert second_blob_row["pack_entry_type"] == "delta"
        assert second_blob_row["pack_base_blob_id"] == first_blob_id
        assert second_blob_row["pack_chain_depth"] == 1

        bundle = get_remote_snapshot(base_url, "housekeeper-remote-direct-pack-delta", second_snapshot["snapshot_id"])
        assert bundle["snapshot_id"] == second_snapshot["snapshot_id"]
        assert base64.b64decode(bundle["files"][0]["content_b64"]).decode("utf-8") == updated_text


def test_local_repack_can_consolidate_existing_direct_delta_layout(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-repack"
    repo.mkdir()
    readme = repo / "app.py"
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-local-repack"], catch_exceptions=False).exit_code == 0
    base_text = "".join(f"line {i:02d} keep same text for compression\n" for i in range(20))
    updated_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
    updated_lines[10] = "line 10 changed text for compression\n"
    updated_lines.append("line 20 keep same text for compression\n")
    updated_text = "".join(updated_lines)

    readme.write_text(base_text, encoding="utf-8")
    first_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
    assert first_out.exit_code == 0, first_out.stdout
    first_snapshot = json.loads(first_out.stdout)

    first_pack = runner.invoke(app, ["gc", "pack", "--json"], catch_exceptions=False)
    assert first_pack.exit_code == 0, first_pack.stdout
    first_packed = json.loads(first_pack.stdout)
    assert first_packed["stats"]["packed_full_blob_count"] >= 1
    assert first_packed["stats"]["packed_delta_blob_count"] == 0

    readme.write_text(updated_text, encoding="utf-8")
    second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
    assert second_out.exit_code == 0, second_out.stdout
    second_snapshot = json.loads(second_out.stdout)

    second_pack = runner.invoke(app, ["gc", "pack", "--json"], catch_exceptions=False)
    assert second_pack.exit_code == 0, second_pack.stdout
    second_packed = json.loads(second_pack.stdout)
    assert second_packed["created"] is False
    assert second_packed["reason"] == "no_unpacked_reachable_blobs"
    assert second_packed["stats"]["packed_delta_blob_count"] >= 1

    repack_out = runner.invoke(app, ["gc", "pack", "--repack", "--json"], catch_exceptions=False)
    assert repack_out.exit_code == 0, repack_out.stdout
    repacked = json.loads(repack_out.stdout)
    assert repacked["created"] is True
    assert repacked["repack"] is True
    assert repacked["stats"]["packed_full_blob_count"] >= 1
    assert repacked["stats"]["packed_delta_blob_count"] >= 1

    ctx = RepoContext.discover(repo)
    conn = connect_sqlite(ctx.content_db_path)
    first_blob_id = conn.execute(
        "select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'",
        (first_snapshot["snapshot_id"],),
    ).fetchone()["blob_id"]
    second_blob_row = conn.execute(
        """
        select blob_id, pack_entry_type, pack_base_blob_id, pack_chain_depth
        from blobs
        where blob_id = (
            select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'
        )
        """,
        (second_snapshot["snapshot_id"],),
    ).fetchone()
    conn.close()
    assert second_blob_row["pack_entry_type"] == "delta"
    assert second_blob_row["pack_base_blob_id"] == first_blob_id
    assert second_blob_row["pack_chain_depth"] == 1

    gc_out = runner.invoke(app, ["gc", "prune", "--json"], catch_exceptions=False)
    assert gc_out.exit_code == 0, gc_out.stdout
    gc_result = json.loads(gc_out.stdout)
    assert gc_result["removed_orphan_pack_count"] >= 1


def test_remote_repack_can_rewrite_existing_full_pack_into_delta_layout(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-repack"
    repo.mkdir()
    readme = repo / "app.py"

    with running_server(tmp_path / "server-repack-delta", queue_mode="async") as (base_url, data_dir):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-remote-repack"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-remote-repack", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        base_text = "".join(f"line {i:02d} keep same text for compression\n" for i in range(20))
        updated_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
        updated_lines[10] = "line 10 changed text for compression\n"
        updated_lines.append("line 20 keep same text for compression\n")
        updated_text = "".join(updated_lines)

        readme.write_text(base_text, encoding="utf-8")
        first_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
        assert first_out.exit_code == 0, first_out.stdout
        first_snapshot = json.loads(first_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main", "--server-storage-mode", "pack_full"], catch_exceptions=False).exit_code == 0

        first_pack = runner.invoke(app, ["repo", "pack", "--json"], catch_exceptions=False)
        assert first_pack.exit_code == 0, first_pack.stdout
        assert _drain_jobs(data_dir) >= 1

        readme.write_text(updated_text, encoding="utf-8")
        second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
        assert second_out.exit_code == 0, second_out.stdout
        second_snapshot = json.loads(second_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main", "--server-storage-mode", "pack_full"], catch_exceptions=False).exit_code == 0

        second_pack = runner.invoke(app, ["repo", "pack", "--json"], catch_exceptions=False)
        assert second_pack.exit_code == 0, second_pack.stdout
        assert _drain_jobs(data_dir) >= 1

        storage_before = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_before["packed_delta_blob_count"] == 0

        repack_out = runner.invoke(app, ["repo", "pack", "--repack", "--json"], catch_exceptions=False)
        assert repack_out.exit_code == 0, repack_out.stdout
        queued = json.loads(repack_out.stdout)
        assert queued["queued"] is True
        assert _drain_jobs(data_dir) >= 1

        storage_after = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_after["packed_full_blob_count"] >= 1
        assert storage_after["packed_delta_blob_count"] >= 1

        ctx = fake_postgres_context(data_dir)
        conn = connect_server_content(ctx)
        first_blob_id = conn.execute(
            "select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'",
            (first_snapshot["snapshot_id"],),
        ).fetchone()["blob_id"]
        second_blob_row = conn.execute(
            """
            select blob_id, storage_kind, pack_entry_type, pack_base_blob_id, pack_chain_depth
            from blobs
            where blob_id = (
                select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'
            )
            """,
            (second_snapshot["snapshot_id"],),
        ).fetchone()
        conn.close()
        assert second_blob_row["storage_kind"] == "pack_delta"
        assert second_blob_row["pack_entry_type"] == "delta"
        assert second_blob_row["pack_base_blob_id"] == first_blob_id
        assert second_blob_row["pack_chain_depth"] == 1

        gc_out = runner.invoke(app, ["repo", "gc", "--json"], catch_exceptions=False)
        assert gc_out.exit_code == 0, gc_out.stdout
        assert _drain_jobs(data_dir) >= 1
        storage_final = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_final["pack_count"] == 1


def test_local_snapshot_export_reads_direct_pack_delta_by_default(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-delta"
    repo.mkdir()
    readme = repo / "app.py"
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-delta"], catch_exceptions=False).exit_code == 0
    base_text = "".join(f"line {i:02d} keep same text for compression\n" for i in range(20))
    updated_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
    updated_lines[10] = "line 10 changed text for compression\n"
    updated_lines.append("line 20 keep same text for compression\n")
    updated_text = "".join(updated_lines)
    readme.write_text(base_text, encoding="utf-8")
    base_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
    assert base_out.exit_code == 0, base_out.stdout
    base_snapshot = json.loads(base_out.stdout)

    readme.write_text(updated_text, encoding="utf-8")
    target_out = runner.invoke(app, ["snapshot", "create", "--message", "target", "--json"], catch_exceptions=False)
    assert target_out.exit_code == 0, target_out.stdout
    target_snapshot = json.loads(target_out.stdout)

    ctx = RepoContext.discover(repo)
    conn = connect_sqlite(ctx.content_db_path)
    base_blob_id = conn.execute(
        "select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'",
        (base_snapshot["snapshot_id"],),
    ).fetchone()["blob_id"]
    target_blob_id = conn.execute(
        "select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'",
        (target_snapshot["snapshot_id"],),
    ).fetchone()["blob_id"]
    target_blob_row = conn.execute(
        """
        select blob_id, pack_entry_type, pack_base_blob_id, pack_chain_depth
        from blobs
        where blob_id = ?
        """,
        (target_blob_id,),
    ).fetchone()
    conn.close()

    assert target_blob_row["pack_entry_type"] == "delta"
    assert target_blob_row["pack_base_blob_id"] == base_blob_id
    assert target_blob_row["pack_chain_depth"] == 1

    bundle = export_snapshot_bundle(ctx, target_snapshot["snapshot_id"])
    assert bundle["files"][0]["path"] == "app.py"
    import base64
    restored = base64.b64decode(bundle["files"][0]["content_b64"]).decode("utf-8")
    assert restored == updated_text


def test_local_snapshot_create_emits_text_delta_entries_by_default(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-auto-delta"
    repo.mkdir()
    readme = repo / "app.py"
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-auto-delta"], catch_exceptions=False).exit_code == 0
    base_text = "".join(f"line {i:02d} keep same text for compression\n" for i in range(20))
    updated_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
    updated_lines[10] = "line 10 changed text for compression\n"
    updated_lines.append("line 20 keep same text for compression\n")
    updated_text = "".join(updated_lines)
    readme.write_text(base_text, encoding="utf-8")
    first_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
    assert first_out.exit_code == 0, first_out.stdout
    first_snapshot = json.loads(first_out.stdout)

    readme.write_text(updated_text, encoding="utf-8")
    second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
    assert second_out.exit_code == 0, second_out.stdout
    second_snapshot = json.loads(second_out.stdout)

    stats_out = runner.invoke(app, ["gc", "stats", "--json"], catch_exceptions=False)
    assert stats_out.exit_code == 0, stats_out.stdout
    packed = json.loads(stats_out.stdout)
    assert packed["packed_full_blob_count"] >= 1
    assert packed["packed_delta_blob_count"] >= 1
    assert packed["efficiency_summary"]["indexed_pack_count"] == packed["pack_count"]
    assert packed["efficiency_summary"]["pack_delta_logical_bytes"] == packed["packed_delta_blob_bytes"]
    assert packed["efficiency_summary"]["delta_pre_archive_savings_bytes"] > 0
    assert packed["efficiency_summary"]["storage_savings_bytes"] == (
        packed["efficiency_summary"]["logical_tracked_blob_bytes"]
        - packed["efficiency_summary"]["physical_storage_bytes"]
    )
    assert packed["validation_summary"]["state"] == "partially_optimized"
    assert packed["validation_summary"]["recommended_action"] == "repack"
    assert packed["validation_summary"]["next_actions"] == ["repack", "gc"]

    ctx = RepoContext.discover(repo)
    conn = connect_sqlite(ctx.content_db_path)
    first_blob_id = conn.execute(
        "select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'",
        (first_snapshot["snapshot_id"],),
    ).fetchone()["blob_id"]
    second_blob_row = conn.execute(
        """
        select blob_id, pack_entry_type, pack_base_blob_id, pack_chain_depth
        from blobs
        where blob_id = (
            select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'
        )
        """,
        (second_snapshot["snapshot_id"],),
    ).fetchone()
    conn.close()
    assert second_blob_row["pack_entry_type"] == "delta"
    assert second_blob_row["pack_base_blob_id"] == first_blob_id
    assert second_blob_row["pack_chain_depth"] == 1

    bundle = export_snapshot_bundle(ctx, second_snapshot["snapshot_id"])
    restored = base64.b64decode(bundle["files"][0]["content_b64"]).decode("utf-8")
    assert restored == updated_text


def test_snapshot_export_handles_duplicate_blob_content_in_one_snapshot(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-duplicate-content"
    repo.mkdir()
    (repo / "a.txt").write_text("same\n", encoding="utf-8")
    (repo / "b.txt").write_text("same\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-duplicate-content"], catch_exceptions=False).exit_code == 0
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)

    ctx = RepoContext.discover(repo)
    bundle = export_snapshot_bundle(ctx, snapshot["snapshot_id"])
    bundled_paths = {row["path"] for row in bundle["files"]}
    assert {"a.txt", "b.txt"}.issubset(bundled_paths)

    pack_files = list((repo / ".ait" / "objects" / "packs").glob("*.zip"))
    assert len(pack_files) == 1
    pack_index = read_pack_index(pack_files[0])
    entry_names = [entry["entry_name"] for entry in pack_index["entries"]]
    assert len(entry_names) == len(set(entry_names))


def test_remote_pack_can_emit_text_delta_entries(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-delta"
    repo.mkdir()
    readme = repo / "app.py"

    with running_server(tmp_path / "server-pack-delta", queue_mode="async") as (base_url, data_dir):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-remote-delta"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-remote-delta", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        base_text = "".join(f"line {i:02d} keep same text for compression\n" for i in range(20))
        updated_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
        updated_lines[10] = "line 10 changed text for compression\n"
        updated_lines.append("line 20 keep same text for compression\n")
        updated_text = "".join(updated_lines)
        readme.write_text(base_text, encoding="utf-8")
        first_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
        assert first_out.exit_code == 0, first_out.stdout
        first_snapshot = json.loads(first_out.stdout)

        readme.write_text(updated_text, encoding="utf-8")
        second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
        assert second_out.exit_code == 0, second_out.stdout
        second_snapshot = json.loads(second_out.stdout)

        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        pack_out = runner.invoke(app, ["repo", "pack", "--json"], catch_exceptions=False)
        assert pack_out.exit_code == 0, pack_out.stdout
        queued = json.loads(pack_out.stdout)
        assert queued["queued"] is True
        assert _drain_jobs(data_dir) >= 1

        storage_out = runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False)
        assert storage_out.exit_code == 0, storage_out.stdout
        storage = json.loads(storage_out.stdout)
        assert storage["packed_full_blob_count"] >= 1
        assert storage["packed_delta_blob_count"] >= 1
        assert storage["efficiency_summary"]["indexed_pack_count"] == storage["pack_count"]
        assert storage["efficiency_summary"]["pack_delta_logical_bytes"] == storage["packed_delta_blob_bytes"]
        assert storage["efficiency_summary"]["delta_pre_archive_savings_bytes"] > 0
        assert storage["efficiency_summary"]["storage_savings_bytes"] == (
            storage["efficiency_summary"]["logical_tracked_blob_bytes"]
            - storage["efficiency_summary"]["physical_storage_bytes"]
        )
        assert storage["validation_summary"]["state"] == "partially_optimized"
        assert storage["validation_summary"]["recommended_action"] == "repack"
        assert storage["validation_summary"]["next_actions"] == ["repack", "gc"]

        ctx = fake_postgres_context(data_dir)
        conn = connect_server_content(ctx)
        first_blob_id = conn.execute(
            """
            select blob_id from snapshot_files
            where snapshot_id = ? and path = 'app.py'
            """,
            (first_snapshot["snapshot_id"],),
        ).fetchone()["blob_id"]
        second_blob_row = conn.execute(
            """
            select blob_id, storage_kind, pack_entry_type, pack_base_blob_id, pack_chain_depth
            from blobs
            where blob_id = (
                select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'
            )
            """,
            (second_snapshot["snapshot_id"],),
        ).fetchone()
        conn.close()
        assert second_blob_row["storage_kind"] == "pack_delta"
        assert second_blob_row["pack_entry_type"] == "delta"
        assert second_blob_row["pack_base_blob_id"] == first_blob_id
        assert second_blob_row["pack_chain_depth"] == 1

        bundle = get_remote_snapshot(base_url, "housekeeper-remote-delta", second_snapshot["snapshot_id"])
        restored = base64.b64decode(bundle["files"][0]["content_b64"]).decode("utf-8")
        assert restored == updated_text


def test_remote_reconcile_reports_and_repairs_storage_signals(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-signals"
    repo.mkdir()
    readme = repo / "app.py"

    with running_server(tmp_path / "server-storage-signals", queue_mode="inline") as (base_url, data_dir):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-remote-signals"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-remote-signals", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        base_text = "".join(f"line {i:02d} keep same text for compression\n" for i in range(20))
        updated_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
        updated_lines[10] = "line 10 changed text for compression\n"
        updated_lines.append("line 20 keep same text for compression\n")
        updated_text = "".join(updated_lines)

        readme.write_text(base_text, encoding="utf-8")
        first_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
        assert first_out.exit_code == 0, first_out.stdout
        first_snapshot = json.loads(first_out.stdout)

        readme.write_text(updated_text, encoding="utf-8")
        second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
        assert second_out.exit_code == 0, second_out.stdout
        second_snapshot = json.loads(second_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        pack_out = runner.invoke(app, ["repo", "pack", "--json"], catch_exceptions=False)
        assert pack_out.exit_code == 0, pack_out.stdout
        packed = json.loads(pack_out.stdout)
        assert packed["queued"] is False

        storage_before = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_before["signals_summary"]["drift_count"] == 0

        ctx = fake_postgres_context(data_dir)
        conn = connect_server_content(ctx)
        second_blob_id = conn.execute(
            "select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'",
            (second_snapshot["snapshot_id"],),
        ).fetchone()["blob_id"]
        conn.execute(
            """
            update blobs
            set storage_kind = 'pack_full',
                pack_entry_type = 'full',
                pack_base_blob_id = null,
                pack_chain_depth = 0
            where blob_id = ?
            """,
            (second_blob_id,),
        )
        conn.commit()
        conn.close()

        storage_drift = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_drift["signals_summary"]["drift_count"] >= 1
        assert storage_drift["signals_summary"]["by_type"]["blob_storage_kind_mismatch"] >= 1
        assert storage_drift["validation_summary"]["state"] == "attention_required"
        assert storage_drift["validation_summary"]["recommended_action"] == "optimize"
        assert storage_drift["validation_summary"]["next_actions"] == ["optimize"]

        reconcile_out = runner.invoke(app, ["repo", "reconcile", "--repair", "--json"], catch_exceptions=False)
        assert reconcile_out.exit_code == 0, reconcile_out.stdout
        reconcile_payload = json.loads(reconcile_out.stdout)
        assert reconcile_payload["queued"] is False
        reconcile = reconcile_payload["result"]
        assert reconcile["storage_signals_summary"]["drift_count"] >= 1
        assert reconcile["repaired_count"] >= 1

        storage_after = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_after["signals_summary"]["drift_count"] == 0
        assert storage_after["validation_summary"]["state"] == "partially_optimized"
        assert storage_after["validation_summary"]["recommended_action"] == "repack"
        assert storage_after["validation_summary"]["next_actions"] == ["repack", "gc"]


def test_remote_optimize_job_repairs_and_compacts_storage(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-optimize"
    repo.mkdir()
    readme = repo / "app.py"

    with running_server(tmp_path / "server-storage-optimize", queue_mode="async") as (base_url, data_dir):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-remote-optimize"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-remote-optimize", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        base_text = "".join(f"line {i:02d} keep same text for compression\n" for i in range(20))
        updated_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
        updated_lines[10] = "line 10 changed text for compression\n"
        updated_lines.append("line 20 keep same text for compression\n")
        updated_text = "".join(updated_lines)

        readme.write_text(base_text, encoding="utf-8")
        first_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
        assert first_out.exit_code == 0, first_out.stdout
        first_snapshot = json.loads(first_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        first_pack = runner.invoke(app, ["repo", "pack", "--json"], catch_exceptions=False)
        assert first_pack.exit_code == 0, first_pack.stdout
        assert _drain_jobs(data_dir) >= 1

        readme.write_text(updated_text, encoding="utf-8")
        second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
        assert second_out.exit_code == 0, second_out.stdout
        second_snapshot = json.loads(second_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        second_pack = runner.invoke(app, ["repo", "pack", "--json"], catch_exceptions=False)
        assert second_pack.exit_code == 0, second_pack.stdout
        assert _drain_jobs(data_dir) >= 1

        storage_before = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_before["pack_count"] >= 2
        assert storage_before["packed_delta_blob_count"] >= 1

        ctx = fake_postgres_context(data_dir)
        conn = connect_server_content(ctx)
        second_blob_id = conn.execute(
            "select blob_id from snapshot_files where snapshot_id = ? and path = 'app.py'",
            (second_snapshot["snapshot_id"],),
        ).fetchone()["blob_id"]
        conn.execute(
            """
            update blobs
            set storage_kind = 'pack_full',
                pack_entry_type = 'full',
                pack_base_blob_id = null,
                pack_chain_depth = 0
            where blob_id = ?
            """,
            (second_blob_id,),
        )
        conn.commit()
        conn.close()

        optimize_out = runner.invoke(app, ["repo", "optimize", "--json"], catch_exceptions=False)
        assert optimize_out.exit_code == 0, optimize_out.stdout
        optimize = json.loads(optimize_out.stdout)
        assert optimize["queued"] is True
        assert optimize["job"]["job_type"] == "content.optimize"
        assert _drain_jobs(data_dir) >= 1

        jobs = json.loads(runner.invoke(app, ["repo", "jobs", "--json"], catch_exceptions=False).stdout)
        assert any(job["job_type"] == "content.optimize" and job["state"] == "succeeded" for job in jobs)

        storage_after = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_after["signals_summary"]["drift_count"] == 0
        assert storage_after["pack_count"] == 1
        assert storage_after["packed_delta_blob_count"] >= 1
        assert storage_after["efficiency_summary"]["indexed_pack_count"] == storage_after["pack_count"]
        assert storage_after["efficiency_summary"]["delta_pre_archive_savings_bytes"] > 0
        assert storage_after["efficiency_summary"]["logical_tracked_blob_bytes"] > storage_after["efficiency_summary"]["physical_storage_bytes"]
        assert storage_after["validation_summary"]["state"] == "delta_optimized"
        assert storage_after["validation_summary"]["recommended_action"] == "none"
        assert storage_after["validation_summary"]["next_actions"] == []

        bundle = get_remote_snapshot(base_url, "housekeeper-remote-optimize", second_snapshot["snapshot_id"])
        restored = base64.b64decode(bundle["files"][0]["content_b64"]).decode("utf-8")
        assert restored == updated_text


def test_remote_optimize_job_compacts_unreachable_tree_metadata(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-tree-metadata-optimize"
    repo.mkdir()
    readme = repo / "app.py"

    with running_server(tmp_path / "server-tree-metadata-optimize", queue_mode="async") as (base_url, data_dir):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-remote-tree-metadata-optimize"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-remote-tree-metadata-optimize", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        base_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
        updated_lines = list(base_lines)
        updated_lines[10] = "line 10 changed text for compression\n"
        updated_lines.append("line 20 keep same text for compression\n")

        readme.write_text("".join(base_lines), encoding="utf-8")
        first_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
        assert first_out.exit_code == 0, first_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        readme.write_text("".join(updated_lines), encoding="utf-8")
        second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
        assert second_out.exit_code == 0, second_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        optimize_out = runner.invoke(app, ["repo", "optimize", "--json"], catch_exceptions=False)
        assert optimize_out.exit_code == 0, optimize_out.stdout
        initial_optimize = json.loads(optimize_out.stdout)
        assert initial_optimize["queued"] is True
        assert initial_optimize["job"]["job_type"] == "content.optimize"
        assert _drain_jobs(data_dir) >= 1

        ctx = fake_postgres_context(data_dir)
        conn = connect_server_content(ctx)
        _seed_orphan_tree_metadata(conn, root_tree_id="TRE-REMOTE-ORPHAN-ROOT", child_tree_id="TRE-REMOTE-ORPHAN-CHILD")
        conn.commit()
        conn.close()

        storage_before = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_before["global_unreachable_tree_count"] == 2
        assert storage_before["global_unreachable_tree_entry_count"] == 2
        assert storage_before["global_tree_pack_count"] >= 1
        assert storage_before["global_orphan_tree_pack_count"] == 0
        assert storage_before["validation_summary"]["state"] == "partially_optimized"
        assert storage_before["validation_summary"]["recommended_action"] == "gc"
        assert storage_before["validation_summary"]["next_actions"] == ["gc"]

        second_optimize_out = runner.invoke(app, ["repo", "optimize", "--json"], catch_exceptions=False)
        assert second_optimize_out.exit_code == 0, second_optimize_out.stdout
        second_optimize = json.loads(second_optimize_out.stdout)
        assert second_optimize["queued"] is True
        assert second_optimize["job"]["job_type"] == "content.optimize"
        assert _drain_jobs(data_dir) >= 1

        storage_after = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_after["global_unreachable_tree_count"] == 0
        assert storage_after["global_unreachable_tree_entry_count"] == 0
        assert storage_after["global_orphan_tree_pack_count"] == 0
        assert storage_after["validation_summary"]["state"] == "delta_optimized"
        assert storage_after["validation_summary"]["recommended_action"] == "none"
        assert storage_after["validation_summary"]["next_actions"] == []


def test_local_validation_summary_reports_packed_storage_state(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-mixed-state"
    repo.mkdir()
    readme = repo / "app.py"
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-local-mixed-state"], catch_exceptions=False).exit_code == 0
    readme.write_text("base\n", encoding="utf-8")
    first_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
    assert first_out.exit_code == 0, first_out.stdout

    first_pack = runner.invoke(app, ["gc", "pack", "--json"], catch_exceptions=False)
    assert first_pack.exit_code == 0, first_pack.stdout
    first_packed = json.loads(first_pack.stdout)
    assert first_packed["stats"]["validation_summary"]["state"] == "packed_full_only"

    readme.write_text("base\nnext\n", encoding="utf-8")
    second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
    assert second_out.exit_code == 0, second_out.stdout

    mixed_stats_out = runner.invoke(app, ["gc", "stats", "--json"], catch_exceptions=False)
    assert mixed_stats_out.exit_code == 0, mixed_stats_out.stdout
    mixed_stats = json.loads(mixed_stats_out.stdout)
    assert mixed_stats["packed_blob_count"] >= 1
    assert mixed_stats["validation_summary"]["state"] == "packed_full_only"
    assert mixed_stats["validation_summary"]["recommended_action"] == "repack"
    assert mixed_stats["validation_summary"]["next_actions"] == ["repack"]


def test_local_gc_reports_and_compacts_unreachable_tree_metadata(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-tree-metadata-gc"
    repo.mkdir()
    readme = repo / "app.py"
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-local-tree-metadata-gc"], catch_exceptions=False).exit_code == 0
    base_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
    updated_lines = list(base_lines)
    updated_lines[10] = "line 10 changed text for compression\n"
    updated_lines.append("line 20 keep same text for compression\n")

    readme.write_text("".join(base_lines), encoding="utf-8")
    assert runner.invoke(app, ["snapshot", "create", "--message", "base"], catch_exceptions=False).exit_code == 0
    readme.write_text("".join(updated_lines), encoding="utf-8")
    assert runner.invoke(app, ["snapshot", "create", "--message", "update"], catch_exceptions=False).exit_code == 0

    optimize_out = runner.invoke(app, ["gc", "optimize", "--json"], catch_exceptions=False)
    assert optimize_out.exit_code == 0, optimize_out.stdout
    optimized = json.loads(optimize_out.stdout)
    assert optimized["final_storage"]["validation_summary"]["state"] == "delta_optimized"

    ctx = RepoContext.discover(repo)
    conn = connect_sqlite(ctx.content_db_path)
    _seed_orphan_tree_metadata(conn, root_tree_id="TRE-LOCAL-ORPHAN-ROOT", child_tree_id="TRE-LOCAL-ORPHAN-CHILD")
    conn.commit()
    conn.close()

    stats_out = runner.invoke(app, ["gc", "stats", "--json"], catch_exceptions=False)
    assert stats_out.exit_code == 0, stats_out.stdout
    stats = json.loads(stats_out.stdout)
    assert stats["unreachable_tree_count"] == 2
    assert stats["unreachable_tree_entry_count"] == 2
    assert stats["tree_pack_count"] >= 1
    assert stats["orphan_tree_pack_count"] == 0
    assert stats["validation_summary"]["state"] == "partially_optimized"
    assert stats["validation_summary"]["recommended_action"] == "gc"
    assert stats["validation_summary"]["next_actions"] == ["gc"]

    prune_out = runner.invoke(app, ["gc", "prune", "--json"], catch_exceptions=False)
    assert prune_out.exit_code == 0, prune_out.stdout
    pruned = json.loads(prune_out.stdout)
    assert pruned["removed_unreachable_tree_count"] == 2
    assert pruned["removed_unreachable_tree_entry_count"] == 2
    assert pruned["removed_orphan_tree_pack_count"] >= 1
    assert pruned["stats"]["tree_pack_count"] == 1
    assert pruned["stats"]["unreachable_tree_count"] == 0
    assert pruned["stats"]["unreachable_tree_entry_count"] == 0
    assert pruned["stats"]["orphan_tree_pack_count"] == 0
    assert pruned["stats"]["validation_summary"]["state"] == "delta_optimized"
    assert pruned["stats"]["validation_summary"]["recommended_action"] == "none"
    assert pruned["stats"]["validation_summary"]["next_actions"] == []


def test_local_gc_validate_inspects_corrupt_tree_pack_archive(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-tree-pack-validate"
    repo.mkdir()
    (repo / "app.py").write_text("hello tree pack\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-local-tree-pack-validate"], catch_exceptions=False).exit_code == 0
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout

    ctx = RepoContext.discover(repo)
    conn = connect_sqlite(ctx.content_db_path)
    tree_pack_row = conn.execute("select pack_path from tree_packs order by created_at desc limit 1").fetchone()
    conn.close()
    assert tree_pack_row is not None
    tree_pack_abs = ctx.root / tree_pack_row["pack_path"]
    assert tree_pack_abs.exists()
    tree_pack_abs.write_bytes(b"not-a-zip")

    validate_out = runner.invoke(app, ["gc", "validate", "--json"], catch_exceptions=False)
    assert validate_out.exit_code == 0, validate_out.stdout
    payload = json.loads(validate_out.stdout)
    assert payload["state"] == "attention_required"
    assert payload["recommended_action"] == "inspect"
    assert payload["next_actions"] == ["inspect"]
    assert "tree_pack_index_errors" in payload["issues"]
    assert payload["needs_attention"] is True


def test_m7_storage_acceptance_preserves_identity_and_surfaces_storage_failures(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-m7-acceptance"
    repo.mkdir()
    readme = repo / "app.py"
    base_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
    updated_lines = list(base_lines)
    updated_lines[10] = "line 10 changed text for compression\n"
    updated_lines.append("line 20 keep same text for compression\n")
    readme.write_text("".join(base_lines), encoding="utf-8")

    with running_server(tmp_path / "server-m7-acceptance", queue_mode="inline") as (base_url, data_dir):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-m7-acceptance"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-m7-acceptance", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        plan_id, plan_revision_id = _write_remote_plan(
            repo,
            "storage_m7_acceptance.md",
            plan_ref="storage-m7-acceptance",
            item_ref="storage-m7-acceptance/validate-storage-milestone-behavior",
            title="Storage M7 acceptance",
        )

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "M7 acceptance demo",
                "--intent",
                "validate storage milestone behavior",
                "--change-title",
                "M7 acceptance change",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--plan",
                plan_id,
                "--revision",
                plan_revision_id,
                "--plan-item-ref",
                "storage-m7-acceptance/validate-storage-milestone-behavior",
                "--remote",
                "origin",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        started = json.loads(start_out.stdout)
        change = started["change"]
        feature_line_name = started["worktree"]["current_line"]
        worktree_path = Path(started["worktree"]["path"])
        monkeypatch.chdir(worktree_path)
        readme = worktree_path / "app.py"

        readme.write_text("".join(updated_lines), encoding="utf-8")

        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature m7 acceptance", "--json"], catch_exceptions=False)
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
        push_feature_out = runner.invoke(
            app,
            ["push", "--line", feature_line_name, "--server-storage-mode", "pack_delta", "--json"],
            catch_exceptions=False,
        )
        assert push_feature_out.exit_code == 0, push_feature_out.stdout
        storage_before_drift = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_before_drift["packed_delta_blob_count"] >= 1

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "m7 acceptance patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        main_line_before = get_remote_line(base_url, "housekeeper-m7-acceptance", "main")
        feature_line_before = get_remote_line(base_url, "housekeeper-m7-acceptance", feature_line_name)
        change_before = get_change(base_url, change["change_id"])
        patchset_before = get_patchset(base_url, patchset["patchset_id"])

        ctx = fake_postgres_context(data_dir)
        conn = connect_server_content(ctx)
        revision_blob_id = conn.execute(
            """
            select sf.blob_id
            from snapshot_files sf
            where sf.snapshot_id = ? and sf.path = 'app.py'
            """,
            (patchset_before["revision_snapshot_id"],),
        ).fetchone()["blob_id"]
        conn.execute(
            """
            update blobs
            set storage_kind = 'pack_full',
                pack_entry_type = 'full',
                pack_base_blob_id = null,
                pack_chain_depth = 0
            where blob_id = ?
            """,
            (revision_blob_id,),
        )
        conn.commit()
        conn.close()

        validate_drift = json.loads(runner.invoke(app, ["repo", "validate", "--json"], catch_exceptions=False).stdout)
        assert validate_drift["state"] == "attention_required"
        assert validate_drift["recommended_action"] == "optimize"
        assert validate_drift["next_actions"] == ["optimize"]

        optimize_out = runner.invoke(app, ["repo", "optimize", "--json"], catch_exceptions=False)
        assert optimize_out.exit_code == 0, optimize_out.stdout
        optimize_payload = json.loads(optimize_out.stdout)
        assert optimize_payload["queued"] is False
        assert optimize_payload["result"]["executed_step_count"] >= 1

        storage_after_optimize = json.loads(runner.invoke(app, ["repo", "storage", "--json"], catch_exceptions=False).stdout)
        assert storage_after_optimize["signals_summary"]["drift_count"] == 0

        main_line_after_optimize = get_remote_line(base_url, "housekeeper-m7-acceptance", "main")
        feature_line_after_optimize = get_remote_line(base_url, "housekeeper-m7-acceptance", feature_line_name)
        change_after_optimize = get_change(base_url, change["change_id"])
        patchset_after_optimize = get_patchset(base_url, patchset["patchset_id"])
        assert main_line_after_optimize["head_snapshot_id"] == main_line_before["head_snapshot_id"]
        assert feature_line_after_optimize["head_snapshot_id"] == feature_line_before["head_snapshot_id"]
        assert change_after_optimize["current_patchset_id"] == change_before["current_patchset_id"]
        assert patchset_after_optimize["base_snapshot_id"] == patchset_before["base_snapshot_id"]
        assert patchset_after_optimize["revision_snapshot_id"] == patchset_before["revision_snapshot_id"]

        conn = connect_server_content(ctx)
        pack_row = conn.execute(
            """
            select b.pack_id, p.pack_path, b.pack_entry_name
            from blobs b
            join packs p on p.pack_id = b.pack_id
            where b.blob_id = ?
            """,
            (revision_blob_id,),
        ).fetchone()
        assert pack_row is not None
        pack_abs = ctx.root / pack_row["pack_path"]
        with zipfile.ZipFile(pack_abs, mode="r") as zf:
            members = {name: zf.read(name) for name in zf.namelist()}
        members[pack_row["pack_entry_name"]] = members[pack_row["pack_entry_name"]] + b"CORRUPT"
        with zipfile.ZipFile(pack_abs, mode="w", compression=zipfile.ZIP_DEFLATED) as zf:
            for name, payload in members.items():
                zf.writestr(name, payload)
        conn.close()

        with pytest.raises(ValueError, match="checksum mismatch|Invalid pack delta payload"):
            export_remote_snapshot(ctx, "housekeeper-m7-acceptance", patchset_before["revision_snapshot_id"])

        main_line_after_failure = get_remote_line(base_url, "housekeeper-m7-acceptance", "main")
        feature_line_after_failure = get_remote_line(base_url, "housekeeper-m7-acceptance", feature_line_name)
        change_after_failure = get_change(base_url, change["change_id"])
        patchset_after_failure = get_patchset(base_url, patchset["patchset_id"])
        assert main_line_after_failure["head_snapshot_id"] == main_line_before["head_snapshot_id"]
        assert feature_line_after_failure["head_snapshot_id"] == feature_line_before["head_snapshot_id"]
        assert change_after_failure["current_patchset_id"] == change_before["current_patchset_id"]
        assert patchset_after_failure["base_snapshot_id"] == patchset_before["base_snapshot_id"]
        assert patchset_after_failure["revision_snapshot_id"] == patchset_before["revision_snapshot_id"]
