from __future__ import annotations

import json
import os
import socket
import threading
import time
import urllib.request
from contextlib import contextmanager
from pathlib import Path

import pytest
import uvicorn
from fastapi.testclient import TestClient
from typer.testing import CliRunner

import ait_native.server_db as server_db
import ait_native.server_content as server_content
import ait_native.server_control as server_control
import ait_native.server_store as server_store
from ait_native.cli import app
from ait_native.server import create_app
from ait_native.server_db import (
    PostgresSupportError,
    close_postgres_connection_pools,
    connect_postgres_runtime,
    normalize_sql,
    postgres_preflight,
    postgres_runtime_summary,
    postgres_schema_upgrade_checks,
)
from ait_native.server_paths import ServerContext
from ait_native.web import create_web_app
from ait_server.store.repo_ops import pack_repository_storage
from tests.ait_web.helpers import _create_plan_bound_task, _ensure_task_worktree
from tests.postgres_fake import FakePsycopg, fake_postgres_dsn, install_fake_psycopg
from tests.postgres_live import live_postgres_available, running_live_postgres

runner = CliRunner()
LIVE_POSTGRES = pytest.mark.skipif(not live_postgres_available(), reason="live PostgreSQL binaries or psycopg are unavailable")


@pytest.fixture(autouse=True)
def _reset_postgres_runtime_state():
    close_postgres_connection_pools()
    server_content.reset_postgres_schema_ready_cache()
    yield
    close_postgres_connection_pools()
    server_content.reset_postgres_schema_ready_cache()


@contextmanager
def running_postgres_server(data_dir: Path, dsn: str):
    old_data = os.environ.get("AIT_NATIVE_SERVER_DATA")
    old_backend = os.environ.get("AIT_NATIVE_SERVER_DB_BACKEND")
    old_dsn = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN")
    os.environ["AIT_NATIVE_SERVER_DATA"] = str(data_dir)
    os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = "postgres"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = dsn
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
        raise RuntimeError("postgres test server did not start")
    try:
        yield base_url
    finally:
        server.should_exit = True
        thread.join(timeout=5)
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


def test_server_context_from_env_can_select_postgres(monkeypatch, tmp_path: Path):
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(tmp_path / "server-data"))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", "postgresql://user:pass@localhost:5432/ait_native")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content_test")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control_test")

    ctx = ServerContext.from_env()
    assert ctx.db_backend == "postgres"
    assert ctx.using_postgres is True
    assert ctx.postgres_dsn == "postgresql://user:pass@localhost:5432/ait_native"
    assert ctx.content_schema == "ait_native_content_test"
    assert ctx.control_schema == "ait_native_control_test"
    assert ctx.pack_dir.exists()
    assert ctx.ref_root.exists()


def test_server_context_from_env_requires_explicit_server_data(monkeypatch, tmp_path: Path):
    repo = tmp_path / "repo"
    repo.mkdir()
    monkeypatch.chdir(repo)
    monkeypatch.delenv("AIT_NATIVE_SERVER_DATA", raising=False)
    monkeypatch.setenv("XDG_STATE_HOME", str(tmp_path / "state-home"))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")

    with pytest.raises(RuntimeError, match="AIT_NATIVE_SERVER_DATA is required"):
        ServerContext.from_env()


def test_server_context_from_env_requires_explicit_backend(monkeypatch, tmp_path: Path):
    repo = tmp_path / "repo"
    repo.mkdir()
    monkeypatch.chdir(repo)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(tmp_path / "server-data"))
    monkeypatch.delenv("AIT_NATIVE_SERVER_DB_BACKEND", raising=False)
    monkeypatch.delenv("AIT_NATIVE_SERVER_POSTGRES_DSN", raising=False)

    with pytest.raises(RuntimeError, match="AIT_NATIVE_SERVER_DB_BACKEND is required"):
        ServerContext.from_env()


def test_server_context_from_env_requires_postgres_dsn(monkeypatch, tmp_path: Path):
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(tmp_path / "server-data"))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.delenv("AIT_NATIVE_SERVER_POSTGRES_DSN", raising=False)

    with pytest.raises(RuntimeError, match="AIT_NATIVE_SERVER_POSTGRES_DSN is required"):
        ServerContext.from_env()


def test_local_sqlite_server_runtime_is_removed(monkeypatch, tmp_path: Path):
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(tmp_path / "server-data-local"))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "sqlite")
    monkeypatch.setenv("AIT_NATIVE_SERVER_HOST", "127.0.0.1")
    monkeypatch.delenv("AIT_NATIVE_SHARED_DEPLOYMENT", raising=False)
    monkeypatch.delenv("AIT_NATIVE_ALLOW_SQLITE_SHARED_DEPLOYMENT", raising=False)

    with pytest.raises(RuntimeError, match="sqlite is no longer supported"):
        create_app()


def test_shared_sqlite_server_runtime_requires_postgres_even_if_legacy_override_is_set(monkeypatch, tmp_path: Path):
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(tmp_path / "server-data-shared"))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "sqlite")
    monkeypatch.setenv("AIT_NATIVE_SERVER_HOST", "0.0.0.0")
    monkeypatch.setenv("AIT_NATIVE_SHARED_DEPLOYMENT", "1")
    monkeypatch.delenv("AIT_NATIVE_ALLOW_SQLITE_SHARED_DEPLOYMENT", raising=False)

    with pytest.raises(RuntimeError, match="sqlite is no longer supported"):
        create_app()

    monkeypatch.setenv("AIT_NATIVE_ALLOW_SQLITE_SHARED_DEPLOYMENT", "1")
    with pytest.raises(RuntimeError, match="sqlite is no longer supported"):
        create_app()


def test_shared_sqlite_web_runtime_requires_postgres_even_if_legacy_override_is_set(monkeypatch, tmp_path: Path):
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(tmp_path / "web-runtime"))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "sqlite")
    monkeypatch.setenv("AIT_NATIVE_SERVER_HOST", "0.0.0.0")
    monkeypatch.setenv("AIT_NATIVE_WEB_HOST", "0.0.0.0")
    monkeypatch.setenv("AIT_NATIVE_SHARED_DEPLOYMENT", "1")
    monkeypatch.delenv("AIT_NATIVE_ALLOW_SQLITE_SHARED_DEPLOYMENT", raising=False)

    with pytest.raises(RuntimeError, match="sqlite is no longer supported"):
        create_web_app()

    monkeypatch.setenv("AIT_NATIVE_ALLOW_SQLITE_SHARED_DEPLOYMENT", "1")
    with pytest.raises(RuntimeError, match="sqlite is no longer supported"):
        create_web_app()


def test_normalize_sql_rewrites_insert_or_ignore_for_postgres():
    sql = "insert or ignore into role_bindings(repo_name, actor_identity, role, created_at) values (?, ?, ?, ?)"
    normalized = normalize_sql(sql, "postgres")
    assert "insert into role_bindings" in normalized.lower()
    assert "on conflict do nothing" in normalized.lower()
    assert "%s" in normalized
    assert "?" not in normalized


def test_connect_postgres_runtime_requires_psycopg(monkeypatch):
    monkeypatch.setattr(server_db, "postgres_support_installed", lambda: False)
    with pytest.raises(PostgresSupportError) as excinfo:
        connect_postgres_runtime("postgresql://user:pass@localhost:5432/ait_native", "ait_native_content")
    assert "psycopg" in str(excinfo.value)


def test_postgres_connection_pool_reuses_idle_connections(tmp_path: Path, monkeypatch):
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
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-runtime'}",
        content_schema="ait_native_content_test",
        control_schema="ait_native_control_test",
    )

    conn1 = server_db.connect_server_plane(ctx, "content")
    raw1 = conn1.raw
    conn1.close()

    conn2 = server_db.connect_server_plane(ctx, "content")
    raw2 = conn2.raw
    conn2.close()

    assert fake.connect_calls == 1
    assert raw1 is raw2


def test_postgres_connection_pool_context_manager_releases_connection_after_exception(tmp_path: Path, monkeypatch):
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
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-runtime'}",
        content_schema="ait_native_content_test",
        control_schema="ait_native_control_test",
    )

    with pytest.raises(RuntimeError, match="boom"):
        with server_db.connect_server_plane(ctx, "content") as conn:
            leaked_raw = conn.raw
            conn.execute("select 1")
            raise RuntimeError("boom")

    conn = server_db.connect_server_plane(ctx, "content")
    try:
        assert fake.connect_calls == 1
        assert conn.raw is leaked_raw
    finally:
        conn.close()


def test_write_server_plane_commits_changes(tmp_path: Path, monkeypatch):
    install_fake_psycopg(monkeypatch)
    ctx = ServerContext.create(
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-runtime'}",
        content_schema="ait_native_content_test",
        control_schema="ait_native_control_test",
    )

    server_db.write_server_plane(
        ctx,
        "control",
        lambda conn: conn.execute("create table if not exists facade_commit_test(value text primary key)"),
    )
    server_db.write_server_plane(
        ctx,
        "control",
        lambda conn: conn.execute("insert into facade_commit_test(value) values (?)", ("persisted",)),
    )

    row = server_db.read_server_plane(
        ctx,
        "control",
        lambda conn: conn.execute("select count(*) as c from facade_commit_test where value = ?", ("persisted",)).fetchone(),
    )
    assert row is not None
    assert row["c"] == 1


def test_read_server_plane_rolls_back_uncommitted_changes(tmp_path: Path, monkeypatch):
    install_fake_psycopg(monkeypatch)
    ctx = ServerContext.create(
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-runtime'}",
        content_schema="ait_native_content_test",
        control_schema="ait_native_control_test",
    )

    server_db.write_server_plane(
        ctx,
        "control",
        lambda conn: conn.execute("create table if not exists facade_read_rollback_test(value text primary key)"),
    )
    server_db.read_server_plane(
        ctx,
        "control",
        lambda conn: conn.execute("insert into facade_read_rollback_test(value) values (?)", ("transient",)),
    )

    row = server_db.read_server_plane(
        ctx,
        "control",
        lambda conn: conn.execute("select count(*) as c from facade_read_rollback_test where value = ?", ("transient",)).fetchone(),
    )
    assert row is not None
    assert row["c"] == 0


def test_postgres_runtime_summary_reports_backend(tmp_path: Path):
    ctx = ServerContext.create(
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn="postgresql://user:pass@localhost:5432/ait_native",
        content_schema="ait_native_content",
        control_schema="ait_native_control",
    )
    summary = postgres_runtime_summary(ctx)
    assert summary["backend"] == "postgres"
    assert summary["server_data_root"] == str(ctx.root)
    assert summary["postgres_dsn_configured"] is True
    assert summary["content_schema"] == "ait_native_content"
    assert summary["control_schema"] == "ait_native_control"


def test_postgres_preflight_reports_missing_dependency_and_dsn(tmp_path: Path, monkeypatch):
    monkeypatch.setattr(server_db, "postgres_support_installed", lambda: False)
    ctx = ServerContext.create(tmp_path / "server-data", backend="postgres")
    result = postgres_preflight(ctx, attempt_connect=False)
    assert result["backend"] == "postgres"
    assert result["ready"] is False
    assert result["psycopg_installed"] is False
    assert "AIT_NATIVE_SERVER_POSTGRES_DSN is not configured." in result["issues"]
    assert any("psycopg" in issue for issue in result["issues"])
    assert result["content_schema_valid"] is True
    assert result["control_schema_valid"] is True


def test_postgres_preflight_reports_invalid_schema_names(tmp_path: Path):
    ctx = ServerContext.create(
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn="postgresql://user:pass@localhost:5432/ait_native",
        content_schema="ait-native-content",
        control_schema="9bad",
    )
    result = postgres_preflight(ctx, attempt_connect=False)
    assert result["ready"] is False
    assert result["content_schema_valid"] is False
    assert result["control_schema_valid"] is False
    assert any("Invalid schema name" in issue for issue in result["issues"])


def test_doctor_postgres_connect_succeeds_with_fake_driver(tmp_path: Path, monkeypatch):
    install_fake_psycopg(monkeypatch)
    server_data = tmp_path / "server-data"
    dsn = f"fake-postgres:///{tmp_path / 'fake-runtime'}"
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(server_data))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", dsn)

    result = runner.invoke(app, ["doctor", "postgres", "--connect", "--json"], catch_exceptions=False)
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["backend"] == "postgres"
    assert payload["ready"] is True
    assert payload["live_connection_ok"] is True
    assert payload["schema_upgrade_checks"]["ok"] is True
    expected_versions = server_db.expected_postgres_schema_versions()
    assert payload["schema_upgrade_checks"]["checks"]["content"]["version"] == expected_versions["content"]["version"]
    assert payload["schema_upgrade_checks"]["checks"]["control"]["version"] == expected_versions["control"]["version"]
    assert payload["issues"] == []


def test_postgres_schema_upgrade_checks_are_repeatable_with_fake_driver(tmp_path: Path, monkeypatch):
    install_fake_psycopg(monkeypatch)
    ctx = ServerContext.create(
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-runtime'}",
        content_schema="ait_native_content_test",
        control_schema="ait_native_control_test",
    )

    for _ in range(3):
        checks = postgres_schema_upgrade_checks(ctx, apply=True)

    expected_versions = server_db.expected_postgres_schema_versions()
    assert checks["ok"] is True
    assert checks["applied"] is True
    assert checks["checks"]["content"]["table_present"] is True
    assert checks["checks"]["content"]["expected_version"] == expected_versions["content"]["version"]
    assert checks["checks"]["content"]["version"] == expected_versions["content"]["version"]
    assert checks["checks"]["control"]["table_present"] is True
    assert checks["checks"]["control"]["expected_version"] == expected_versions["control"]["version"]
    assert checks["checks"]["control"]["version"] == expected_versions["control"]["version"]

    # Direct initializers should also be idempotent and leave the same version rows.
    server_content.initialize(ctx)
    server_control.initialize(ctx)
    verify = postgres_schema_upgrade_checks(ctx, apply=False)
    assert verify["ok"] is True
    assert verify["checks"]["content"]["version"] == expected_versions["content"]["version"]
    assert verify["checks"]["control"]["version"] == expected_versions["control"]["version"]


def test_postgres_schema_upgrade_checks_second_batch_reuses_pooled_connections(tmp_path: Path, monkeypatch):
    class TrackingConnection:
        def __init__(self, inner):
            self.inner = inner
            self.close_calls = 0

        def close(self):
            self.close_calls += 1
            return self.inner.close()

        def __getattr__(self, name: str):
            return getattr(self.inner, name)

    class CountingPsycopg:
        def __init__(self):
            self.inner = FakePsycopg()
            self.connect_calls = 0
            self.connections: list[TrackingConnection] = []

        def connect(self, dsn: str):
            self.connect_calls += 1
            tracked = TrackingConnection(self.inner.connect(dsn))
            self.connections.append(tracked)
            return tracked

    fake = CountingPsycopg()
    monkeypatch.setattr(server_db, "_load_psycopg", lambda: fake)
    monkeypatch.setattr(server_db, "postgres_support_installed", lambda: True)

    ctx = ServerContext.create(
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-runtime'}",
        content_schema="ait_native_content_test",
        control_schema="ait_native_control_test",
    )

    first_batch = postgres_schema_upgrade_checks(ctx, apply=True)
    assert first_batch["ok"] is True
    first_connect_calls = fake.connect_calls
    assert len(fake.connections) == 2
    content_connection = fake.connections[0]
    control_connection = fake.connections[1]

    second_batch = postgres_schema_upgrade_checks(ctx, apply=True)
    assert second_batch["ok"] is True
    assert fake.connect_calls == first_connect_calls
    assert content_connection.close_calls == 0
    assert control_connection.close_calls == 0

    # Re-connect one more time through both paths to ensure pooled connections are returned per-plane.
    content_conn = server_db.connect_server_plane(ctx, "content")
    control_conn = server_db.connect_server_plane(ctx, "control")
    try:
        assert content_conn.raw is content_connection
        assert control_conn.raw is control_connection
    finally:
        content_conn.close()
        control_conn.close()


def test_postgres_request_paths_skip_content_migrations_after_initialize(tmp_path: Path, monkeypatch):
    install_fake_psycopg(monkeypatch)
    ctx = ServerContext.create(
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-runtime'}",
        content_schema="ait_native_content_test",
        control_schema="ait_native_control_test",
    )
    server_content.initialize(ctx)

    def fail(*args, **kwargs):
        raise AssertionError("request-time content paths should not rerun content migrations after initialize")

    monkeypatch.setattr(server_content, "_migrate_snapshot_metadata", fail)
    monkeypatch.setattr(server_content, "_migrate_line_head_snapshot_index", fail)
    repo = server_content.ensure_repository(ctx, "ready-repo", "main")
    assert repo["repo_name"] == "ready-repo"
    assert server_content.list_lines(ctx, "missing-repo") == []
    server_content.write_ref(ctx, "ready-repo", "main", "SNP-READY")
    assert server_content.read_ref(ctx, "ready-repo", "main") == "SNP-READY"


def test_postgres_initializers_take_advisory_locks(tmp_path: Path, monkeypatch):
    install_fake_psycopg(monkeypatch)
    ctx = ServerContext.create(
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-runtime'}",
        content_schema="ait_native_content_test",
        control_schema="ait_native_control_test",
    )

    class RecordingConn:
        def __init__(self, inner):
            self._inner = inner
            self.backend = inner.backend
            self.queries: list[str] = []

        def __enter__(self):
            self._inner.__enter__()
            return self

        def __exit__(self, exc_type, exc, tb):
            return self._inner.__exit__(exc_type, exc, tb)

        def execute(self, sql: str, params=()):
            self.queries.append(str(sql))
            return self._inner.execute(sql, params)

        def __getattr__(self, name: str):
            return getattr(self._inner, name)

    content_conn = RecordingConn(server_db.connect_server_plane(ctx, "content"))
    monkeypatch.setattr(server_content, "_connect", lambda _: content_conn)
    server_content.initialize(ctx)
    assert any("pg_advisory_lock" in query for query in content_conn.queries)
    assert any("pg_advisory_unlock" in query for query in content_conn.queries)

    control_conn = RecordingConn(server_db.connect_server_plane(ctx, "control"))
    monkeypatch.setattr(server_control, "connect", lambda _: control_conn)
    server_control.initialize(ctx)
    assert any("pg_advisory_lock" in query for query in control_conn.queries)
    assert any("pg_advisory_unlock" in query for query in control_conn.queries)


@LIVE_POSTGRES
def test_doctor_postgres_connect_succeeds_with_live_runtime(tmp_path: Path, monkeypatch):
    server_data = tmp_path / "server-data-live"
    with running_live_postgres(tmp_path / "live-postgres-doctor") as runtime:
        monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(server_data))
        monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
        monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", runtime["dsn"])

        result = runner.invoke(app, ["doctor", "postgres", "--connect", "--json"], catch_exceptions=False)
        assert result.exit_code == 0, result.stdout
        payload = json.loads(result.stdout)
        assert payload["backend"] == "postgres"
        assert payload["ready"] is True
        assert payload["live_connection_ok"] is True
        assert payload["schema_upgrade_checks"]["ok"] is True
        assert payload["issues"] == []


@LIVE_POSTGRES
def test_live_postgres_content_initialize_serializes_concurrent_calls(tmp_path: Path, monkeypatch):
    with running_live_postgres(tmp_path / "live-postgres-init-lock") as runtime:
        ctx = ServerContext.create(
            tmp_path / "server-data-live",
            backend="postgres",
            postgres_dsn=runtime["dsn"],
            content_schema="ait_native_content_test",
            control_schema="ait_native_control_test",
        )
        server_content.initialize(ctx)

        active = 0
        max_active = 0
        active_lock = threading.Lock()
        errors: list[BaseException] = []
        original_ensure_schema = server_content._ensure_schema

        def wrapped_ensure_schema(conn, inner_ctx):
            nonlocal active, max_active
            with active_lock:
                active += 1
                max_active = max(max_active, active)
            try:
                time.sleep(0.2)
                return original_ensure_schema(conn, inner_ctx)
            finally:
                with active_lock:
                    active -= 1

        monkeypatch.setattr(server_content, "_ensure_schema", wrapped_ensure_schema)
        start_barrier = threading.Barrier(3)

        def worker():
            try:
                start_barrier.wait(timeout=5)
                server_content.initialize(ctx)
            except BaseException as exc:  # pragma: no cover - failure path captured for assertion
                errors.append(exc)

        threads = [threading.Thread(target=worker, daemon=True) for _ in range(2)]
        for thread in threads:
            thread.start()
        start_barrier.wait(timeout=5)
        for thread in threads:
            thread.join(timeout=10)

        assert errors == []
        assert all(not thread.is_alive() for thread in threads)
        assert max_active == 1


def test_fake_postgres_backend_supports_core_remote_workflow(tmp_path: Path, monkeypatch):
    install_fake_psycopg(monkeypatch)
    repo = tmp_path / "housekeeper-pg"
    repo.mkdir()
    (repo / "notes.txt").write_text("base\n", encoding="utf-8")
    dsn = f"fake-postgres:///{tmp_path / 'fake-runtime'}"

    with running_postgres_server(tmp_path / "server-data", dsn) as base_url:
        monkeypatch.chdir(repo)
        health = json.loads(urllib.request.urlopen(f"{base_url}/healthz", timeout=5).read().decode("utf-8"))
        assert health["using_postgres"] is True

        assert runner.invoke(app, ["init", "--name", "housekeeper-pg"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-pg", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        main_snapshot = json.loads(main_snap_out.stdout)
        push_out = runner.invoke(app, ["push", "--line", "main", "--json"], catch_exceptions=False)
        assert push_out.exit_code == 0, push_out.stdout

        task = _create_plan_bound_task(
            repo,
            title="postgres workflow",
            intent="validate postgres deployment path",
            risk="medium",
            slug="postgres-workflow",
        )
        worktree_path = _ensure_task_worktree(repo, task["task_id"])
        monkeypatch.chdir(worktree_path)

        assert runner.invoke(app, ["line", "create", "feature/postgres-smoke"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/postgres-smoke"], catch_exceptions=False).exit_code == 0
        (worktree_path / "notes.txt").write_text("base\npostgres\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "postgres feature", "--json"], catch_exceptions=False)
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
        feature_snapshot = json.loads(feature_snap_out.stdout)

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "postgres smoke change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "postgres smoke patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert patchset["base_snapshot_id"] == main_snapshot["snapshot_id"]
        assert patchset["revision_snapshot_id"] == feature_snapshot["snapshot_id"]

        attest_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--author-mode", "human_only", "--json"],
            catch_exceptions=False,
        )
        assert attest_out.exit_code == 0, attest_out.stdout

        review_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        )
        assert review_out.exit_code == 0, review_out.stdout

        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pass"

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        land = json.loads(land_out.stdout)
        assert land["status"] == "succeeded"
        assert land["result"]["landed_snapshot_id"] == feature_snapshot["snapshot_id"]


def test_repo_storage_validation_accepts_shared_pack_metadata_from_other_repositories(tmp_path: Path, monkeypatch):
    install_fake_psycopg(monkeypatch)
    repo_a = tmp_path / "housekeeper-shared-pack-a"
    repo_b = tmp_path / "housekeeper-shared-pack-b"
    repo_a.mkdir()
    repo_b.mkdir()
    (repo_a / "notes.txt").write_text("base\n", encoding="utf-8")
    (repo_b / "notes.txt").write_text("base\n", encoding="utf-8")
    dsn = f"fake-postgres:///{tmp_path / 'fake-runtime-shared-pack'}"

    with running_postgres_server(tmp_path / "server-data-shared-pack", dsn) as base_url:
        for repo, name, prefix in (
            (repo_a, "housekeeper-shared-pack-a", "SPA"),
            (repo_b, "housekeeper-shared-pack-b", "SPB"),
        ):
            monkeypatch.chdir(repo)
            assert runner.invoke(app, ["init", "--name", name], catch_exceptions=False).exit_code == 0
            assert runner.invoke(app, ["config", "set", "--id-namespace-prefix", prefix], catch_exceptions=False).exit_code == 0
            assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", name, "--default"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
            seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
            assert seed_out.exit_code == 0, seed_out.stdout
            push_out = runner.invoke(app, ["push", "--line", "main", "--json"], catch_exceptions=False)
            assert push_out.exit_code == 0, push_out.stdout

        ctx = ServerContext.from_env()
        server_content.initialize(ctx)
        server_control.initialize(ctx)

        pack_result = pack_repository_storage(ctx, "housekeeper-shared-pack-b", repack=True)
        assert pack_result["stats"]["packed_blob_count"] >= 1

        reconcile_a = server_store.reconcile_repository(ctx, "housekeeper-shared-pack-a", repair=False)
        assert reconcile_a["storage_signals_summary"]["drift_count"] == 0
        assert reconcile_a["drift_count"] == 0


def test_repo_storage_validation_accepts_legacy_loose_blob_payloads(tmp_path: Path, monkeypatch):
    install_fake_psycopg(monkeypatch)
    repo = tmp_path / "housekeeper-legacy-loose"
    repo.mkdir()
    (repo / "notes.txt").write_text("base\n", encoding="utf-8")
    dsn = f"fake-postgres:///{tmp_path / 'fake-runtime-legacy-loose'}"

    with running_postgres_server(tmp_path / "server-data-legacy-loose", dsn) as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-legacy-loose"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-legacy-loose", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert seed_out.exit_code == 0, seed_out.stdout
        push_out = runner.invoke(app, ["push", "--line", "main", "--json"], catch_exceptions=False)
        assert push_out.exit_code == 0, push_out.stdout

        ctx = ServerContext.from_env()
        server_content.initialize(ctx)
        server_control.initialize(ctx)

        def _downgrade_one_blob(conn):
            row = conn.execute(
                "select blob_id, storage_path from blobs where pack_id is not null order by created_at limit 1"
            ).fetchone()
            assert row is not None
            loose_storage_path = f"objects/blobs/{row['blob_id']}"
            conn.execute(
                """
                update blobs
                   set storage_kind = 'loose',
                       storage_path = ?,
                       pack_id = null,
                       pack_entry_name = null,
                       pack_entry_type = null,
                       pack_base_blob_id = null,
                       pack_chain_depth = null,
                       packed_at = null,
                       pruned_at = null
                 where blob_id = ?
                """,
                (loose_storage_path, row["blob_id"]),
            )
            return {"blob_id": row["blob_id"], "storage_path": loose_storage_path, "payload": "base\n"}

        legacy_blob = server_db.write_server_plane(ctx, "content", _downgrade_one_blob)
        loose_path = ctx.root / legacy_blob["storage_path"]
        loose_path.parent.mkdir(parents=True, exist_ok=True)
        loose_path.write_text(str(legacy_blob["payload"]), encoding="utf-8")
        assert loose_path.exists()

        reconcile = server_store.reconcile_repository(ctx, "housekeeper-legacy-loose", repair=False)
        assert reconcile["storage_signals_summary"]["drift_count"] == 0
        assert reconcile["drift_count"] == 0


@LIVE_POSTGRES
def test_live_postgres_backend_supports_server_and_web_review_flow(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-pg-live"
    repo.mkdir()
    (repo / "notes.txt").write_text("base\n", encoding="utf-8")

    with running_live_postgres(tmp_path / "live-postgres-runtime") as runtime:
        server_data = tmp_path / "server-data-live"
        with running_postgres_server(server_data, runtime["dsn"]) as base_url:
            monkeypatch.chdir(repo)
            health = json.loads(urllib.request.urlopen(f"{base_url}/healthz", timeout=5).read().decode("utf-8"))
            assert health["using_postgres"] is True

            assert runner.invoke(app, ["init", "--name", "housekeeper-pg-live"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-pg-live", "--default"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

            main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
            assert main_snap_out.exit_code == 0, main_snap_out.stdout
            main_snapshot = json.loads(main_snap_out.stdout)
            push_out = runner.invoke(app, ["push", "--line", "main", "--json"], catch_exceptions=False)
            assert push_out.exit_code == 0, push_out.stdout
            assert json.loads(push_out.stdout)["head_snapshot_id"] == main_snapshot["snapshot_id"]

            task = _create_plan_bound_task(
                repo,
                title="live postgres workflow",
                intent="validate live postgres lifecycle path",
                risk="high",
                slug="live-postgres-workflow",
            )
            worktree_path = _ensure_task_worktree(repo, task["task_id"])
            monkeypatch.chdir(worktree_path)

            assert runner.invoke(app, ["line", "create", "feature/postgres-live"], catch_exceptions=False).exit_code == 0
            assert runner.invoke(app, ["line", "switch", "feature/postgres-live"], catch_exceptions=False).exit_code == 0
            (worktree_path / "notes.txt").write_text("base\npostgres live\n", encoding="utf-8")
            feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "postgres live feature", "--json"], catch_exceptions=False)
            assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
            feature_snapshot = json.loads(feature_snap_out.stdout)

            change_out = runner.invoke(
                app,
                ["change", "create", "--task", task["task_id"], "--title", "live postgres change", "--base-line", "main", "--risk", "high", "--json"],
                catch_exceptions=False,
            )
            assert change_out.exit_code == 0, change_out.stdout
            change = json.loads(change_out.stdout)

            patchset_out = runner.invoke(
                app,
                ["patchset", "publish", "--change", change["change_id"], "--summary", "live postgres patchset", "--json"],
                catch_exceptions=False,
            )
            assert patchset_out.exit_code == 0, patchset_out.stdout
            patchset = json.loads(patchset_out.stdout)
            assert patchset["base_snapshot_id"] == main_snapshot["snapshot_id"]
            assert patchset["revision_snapshot_id"] == feature_snapshot["snapshot_id"]

            assert (
                runner.invoke(
                    app,
                    ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--author-mode", "human_only", "--json"],
                    catch_exceptions=False,
                ).exit_code
                == 0
            )
            assert runner.invoke(
                app,
                ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
                catch_exceptions=False,
            ).exit_code == 0

            policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
            assert policy_out.exit_code == 0, policy_out.stdout
            assert json.loads(policy_out.stdout)["decision"] == "pass"

            with TestClient(create_web_app()) as client:
                inbox = client.get("/inbox")
                assert inbox.status_code == 200
                assert "Reviewer Inbox" in inbox.text
                assert "housekeeper-pg-live" in inbox.text
                assert change["change_id"] in inbox.text

                change_page = client.get(f"/changes/{change['change_id']}")
                assert change_page.status_code == 200
                assert "live postgres change" in change_page.text
                assert patchset["patchset_id"] in change_page.text

            land_out = runner.invoke(
                app,
                ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
                catch_exceptions=False,
            )
            assert land_out.exit_code == 0, land_out.stdout
        land = json.loads(land_out.stdout)
        assert land["status"] == "succeeded"
        assert land["result"]["landed_snapshot_id"] == feature_snapshot["snapshot_id"]


def test_server_content_initialize_backfills_legacy_line_head_snapshot_ids(tmp_path: Path, monkeypatch):
    install_fake_psycopg(monkeypatch)
    ctx = ServerContext.create(tmp_path / "server-data", backend="postgres", postgres_dsn=fake_postgres_dsn(tmp_path / "server-data"))
    legacy_now = "2026-05-21T00:00:00+00:00"

    with server_content.connect(ctx) as conn:
        conn.execute(
            """
            create table repositories (
                repo_name text primary key,
                repo_id text not null unique,
                default_line text not null,
                id_namespace_prefix text not null default 'AIT',
                policy_json text not null default '{}',
                created_at text not null,
                updated_at text not null
            )
            """
        )
        conn.execute(
            """
            create table lines (
                repo_name text not null references repositories(repo_name) on delete cascade,
                repo_id text not null,
                line_name text not null,
                status text not null default 'active',
                archived_at text,
                created_at text not null,
                updated_at text not null,
                primary key (repo_id, line_name)
            )
            """
        )
        conn.execute(
            """
            insert into repositories(repo_name, repo_id, default_line, id_namespace_prefix, policy_json, created_at, updated_at)
            values (?, ?, ?, ?, ?, ?, ?)
            """,
            ("legacy", "REPO-LEGACY", "main", "LEG", "{}", legacy_now, legacy_now),
        )
        conn.execute(
            """
            insert into lines(repo_name, repo_id, line_name, status, archived_at, created_at, updated_at)
            values (?, ?, ?, 'active', null, ?, ?)
            """,
            ("legacy", "REPO-LEGACY", "feature/legacy", legacy_now, legacy_now),
        )
        conn.commit()

    server_content._write_ref_for_repository(ctx, "legacy", "REPO-LEGACY", "feature/legacy", "SNP-LEGACY")
    server_content.initialize(ctx)

    with server_content.connect(ctx) as conn:
        line_columns = {row["name"] for row in conn.execute("pragma table_info(lines)")}
        line_indexes = {row["name"] for row in conn.execute("pragma index_list(lines)")}
        row = conn.execute(
            "select head_snapshot_id from lines where repo_id = ? and line_name = ?",
            ("REPO-LEGACY", "feature/legacy"),
        ).fetchone()

    assert "head_snapshot_id" in line_columns
    assert "idx_lines_repo_id_head_snapshot" in line_indexes
    assert row["head_snapshot_id"] == "SNP-LEGACY"


def test_postgres_schema_files_exist_and_define_expected_tables():
    root = Path(__file__).resolve().parent.parent
    content_sql = (root / "sql" / "ait_native_postgres_content_schema.sql").read_text(encoding="utf-8")
    control_sql = (root / "sql" / "ait_native_postgres_control_schema.sql").read_text(encoding="utf-8")
    assert "create table if not exists snapshots" in content_sql.lower()
    assert "create table if not exists repository_groups" in content_sql.lower()
    assert "create table if not exists repository_group_memberships" in content_sql.lower()
    assert "create table if not exists schema_versions" in content_sql.lower()
    assert "head_snapshot_id text" in content_sql.lower()
    assert "idx_lines_repo_id_head_snapshot" in content_sql.lower()
    assert "policy_json" in content_sql.lower()
    assert "create table if not exists patchsets" in control_sql.lower()
    assert "create table if not exists test_case_inventory" in control_sql.lower()
    assert "create table if not exists remote_task_inventory" in control_sql.lower()
    assert "create table if not exists remote_change_inventory" in control_sql.lower()
    assert "create table if not exists remote_patchset_inventory" in control_sql.lower()
    assert "create table if not exists task_test_case_links" in control_sql.lower()
    assert "create table if not exists schema_versions" in control_sql.lower()
    assert "repo_id text" in control_sql.lower()
    assert "primary key (repo_id, test_case_id)" in control_sql.lower()
    assert "primary key (repo_id, task_id, test_case_id)" in control_sql.lower()
    assert "pytest_node_id text not null" in control_sql.lower()
    assert "task_seq integer" in control_sql.lower()
    assert "alter table if exists tasks add column if not exists repo_id text" in control_sql.lower()
    assert "alter table if exists tasks add column if not exists task_seq integer" in control_sql.lower()
    assert "uq_tasks_repo_id_task_seq" in control_sql.lower()
    assert "uq_test_case_inventory_repo_id_test_case_id" in control_sql.lower()
    assert "uq_test_case_inventory_repo_id_node" in control_sql.lower()
    assert "set search_path" in content_sql.lower()
    assert "set search_path" in control_sql.lower()
