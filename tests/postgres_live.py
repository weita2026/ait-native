from __future__ import annotations

import importlib.util
import os
import re
import shutil
import socket
import subprocess
import time
import uuid
from contextlib import contextmanager
from pathlib import Path


def _candidate_bin_dirs() -> list[Path]:
    dirs: list[Path] = []
    env_dir = os.environ.get("AIT_NATIVE_POSTGRES_BIN_DIR")
    if env_dir:
        dirs.append(Path(env_dir).expanduser())

    which_postgres = shutil.which("postgres")
    if which_postgres:
        dirs.append(Path(which_postgres).resolve().parent)

    for prefix in (Path("/opt/homebrew/opt"), Path("/usr/local/opt")):
        for name in ("postgresql@17", "postgresql@16", "postgresql@15", "postgresql"):
            dirs.append(prefix / name / "bin")

    return dirs


def _external_postgres_dsn() -> str | None:
    for env_name in ("AIT_NATIVE_TEST_POSTGRES_DSN", "AIT_NATIVE_SERVER_POSTGRES_DSN"):
        candidate = os.environ.get(env_name, "").strip()
        if candidate.startswith(("postgresql://", "postgres://")):
            return candidate
    return None


def find_postgres_bin_dir() -> Path | None:
    for candidate in _candidate_bin_dirs():
        postgres = candidate / "postgres"
        initdb = candidate / "initdb"
        if postgres.exists() and initdb.exists():
            return candidate
    return None


def live_postgres_available() -> bool:
    if importlib.util.find_spec("psycopg") is None:
        return False
    return _external_postgres_dsn() is not None or find_postgres_bin_dir() is not None


def live_postgres_unavailable_reason() -> str:
    if importlib.util.find_spec("psycopg") is None:
        return "psycopg is unavailable"
    if _external_postgres_dsn() is not None:
        return ""
    if find_postgres_bin_dir() is None:
        return "postgres binaries are unavailable"
    return ""


def _reserve_port() -> int:
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    port = int(sock.getsockname()[1])
    sock.close()
    return port


def _shared_memory_bootstrap_failure(stderr: str) -> bool:
    lowered = stderr.lower()
    return "could not create shared memory segment" in lowered and "no space left on device" in lowered


def _temporary_database_name(root: Path) -> str:
    slug = re.sub(r"[^a-z0-9]+", "_", root.name.lower()).strip("_")
    if not slug:
        slug = "live_postgres"
    slug = slug[:40]
    return f"ait_live_{slug}_{uuid.uuid4().hex[:8]}"


@contextmanager
def _running_live_postgres_database(root: Path, *, db_name: str = "ait_native"):
    import psycopg
    from psycopg import sql
    from psycopg.conninfo import conninfo_to_dict, make_conninfo

    base_dsn = _external_postgres_dsn()
    if base_dsn is None:
        raise RuntimeError("external postgres DSN is unavailable")

    base_conninfo = conninfo_to_dict(base_dsn)
    temp_db_name = _temporary_database_name(root) if db_name == "ait_native" else db_name
    admin_conninfo = dict(base_conninfo)
    admin_conninfo["dbname"] = "postgres"
    admin_dsn = make_conninfo(**admin_conninfo)
    runtime_conninfo = dict(base_conninfo)
    runtime_conninfo["dbname"] = temp_db_name
    runtime_dsn = make_conninfo(**runtime_conninfo)

    with psycopg.connect(admin_dsn, autocommit=True) as conn:
        with conn.cursor() as cur:
            cur.execute(sql.SQL("create database {}").format(sql.Identifier(temp_db_name)))

    try:
        yield {
            "bin_dir": None,
            "data_dir": None,
            "dsn": runtime_dsn,
            "log_path": None,
            "port": base_conninfo.get("port"),
        }
    finally:
        with psycopg.connect(admin_dsn, autocommit=True) as conn:
            with conn.cursor() as cur:
                cur.execute(
                    "select pg_terminate_backend(pid) from pg_stat_activity where datname = %s and pid <> pg_backend_pid()",
                    (temp_db_name,),
                )
                cur.execute(sql.SQL("drop database {} with (force)").format(sql.Identifier(temp_db_name)))


@contextmanager
def running_live_postgres(root: Path, *, db_name: str = "ait_native", username: str = "postgres"):
    import psycopg
    import pytest
    from psycopg import sql

    external_dsn = _external_postgres_dsn()
    if external_dsn is not None:
        with _running_live_postgres_database(root, db_name=db_name) as runtime:
            yield runtime
        return

    bin_dir = find_postgres_bin_dir()
    if bin_dir is None:
        raise RuntimeError("postgres binaries are unavailable")

    data_dir = root / "pg-data"
    log_path = root / "postgres.log"
    data_dir.parent.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.setdefault("LC_ALL", "C")

    initdb = subprocess.run(
        [
            str(bin_dir / "initdb"),
            "-D",
            str(data_dir),
            "--auth=trust",
            "--username",
            username,
            "--encoding=UTF8",
            "--locale=C",
        ],
        env=env,
        capture_output=True,
        text=True,
    )
    if initdb.returncode != 0:
        if _shared_memory_bootstrap_failure(initdb.stderr):
            detail = initdb.stderr.strip().splitlines()[0]
            pytest.skip(f"live PostgreSQL bootstrap unavailable on this host: {detail}")
        raise subprocess.CalledProcessError(
            initdb.returncode,
            initdb.args,
            output=initdb.stdout,
            stderr=initdb.stderr,
        )

    port = _reserve_port()
    log_handle = log_path.open("w", encoding="utf-8")
    proc = subprocess.Popen(
        [
            str(bin_dir / "postgres"),
            "-D",
            str(data_dir),
            "-h",
            "127.0.0.1",
            "-p",
            str(port),
        ],
        env=env,
        stdout=log_handle,
        stderr=subprocess.STDOUT,
        text=True,
    )

    admin_dsn = f"postgresql://{username}@127.0.0.1:{port}/postgres"
    deadline = time.time() + 20
    try:
        while time.time() < deadline:
            if proc.poll() is not None:
                break
            try:
                with psycopg.connect(admin_dsn, autocommit=True) as conn:
                    with conn.cursor() as cur:
                        cur.execute("select 1")
                break
            except Exception:
                time.sleep(0.1)
        else:
            raise RuntimeError(f"live postgres did not become ready; see {log_path}")

        if proc.poll() is not None:
            log_text = log_path.read_text(encoding="utf-8", errors="replace")
            if _shared_memory_bootstrap_failure(log_text):
                detail = next((line.strip() for line in log_text.splitlines() if line.strip()), str(log_path))
                pytest.skip(f"live PostgreSQL startup unavailable on this host: {detail}")
            raise RuntimeError(f"live postgres exited early; see {log_path}")

        with psycopg.connect(admin_dsn, autocommit=True) as conn:
            with conn.cursor() as cur:
                cur.execute("select 1 from pg_database where datname = %s", (db_name,))
                if cur.fetchone() is None:
                    cur.execute(sql.SQL("create database {}").format(sql.Identifier(db_name)))

        yield {
            "bin_dir": str(bin_dir),
            "data_dir": str(data_dir),
            "dsn": f"postgresql://{username}@127.0.0.1:{port}/{db_name}",
            "log_path": str(log_path),
            "port": port,
        }
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=5)
        log_handle.close()
