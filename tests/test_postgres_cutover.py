from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient
from typer.testing import CliRunner

from ait_native.cli import app
from ait_native.server import create_app
from ait_native.server_db import connect_server_plane, connect_sqlite_runtime
from ait_native.server_paths import ServerContext
from tests.postgres_fake import install_fake_psycopg

runner = CliRunner()


def test_server_context_rejects_sqlite_runtime(tmp_path: Path) -> None:
    with pytest.raises(RuntimeError, match="SQLite runtime support has been removed"):
        ServerContext.create(tmp_path / "server-data", backend="sqlite")


def test_server_db_rejects_sqlite_open_without_creating_files(tmp_path: Path) -> None:
    db_path = tmp_path / "server-data" / "content.db"

    with pytest.raises(RuntimeError, match="SQLite runtime support has been removed"):
        connect_sqlite_runtime(db_path)

    assert not db_path.exists()


def test_connect_server_plane_rejects_non_postgres_context_without_creating_files(tmp_path: Path) -> None:
    ctx = ServerContext(
        root=tmp_path / "server-data",
        content_db_path=tmp_path / "server-data" / "content.db",
        control_db_path=tmp_path / "server-data" / "control.db",
        db_backend="sqlite",
    )

    with pytest.raises(RuntimeError, match="Only PostgreSQL is supported"):
        connect_server_plane(ctx, "content")

    assert not ctx.content_db_path.exists()
    assert not ctx.control_db_path.exists()


def test_cutover_cli_commands_are_removed() -> None:
    doctor = runner.invoke(app, ["doctor", "postgres-cutover", "--json"])
    repo_inventory = runner.invoke(app, ["repo", "sqlite-inventory", "--json"])
    repo_parity = runner.invoke(app, ["repo", "postgres-parity", "--json"])

    assert doctor.exit_code != 0
    assert repo_inventory.exit_code != 0
    assert repo_parity.exit_code != 0
    assert "No such command" in doctor.output
    assert "No such command" in repo_inventory.output
    assert "No such command" in repo_parity.output


def test_cutover_admin_endpoints_are_removed(monkeypatch, tmp_path: Path) -> None:
    install_fake_psycopg(monkeypatch)
    runtime_root = tmp_path / "server-data"
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(runtime_root))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", f"fake-postgres:///{tmp_path / 'pg'}")
    monkeypatch.setenv("AIT_NATIVE_AUTH_MODE", "open")

    with TestClient(create_app()) as client:
        assert client.get("/v1/native/admin/sqlite-inventory").status_code == 404
        assert client.get("/v1/native/admin/postgres-cutover-parity").status_code == 404

    assert not (runtime_root / "content.db").exists()
    assert not (runtime_root / "control.db").exists()
