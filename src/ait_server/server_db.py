from __future__ import annotations

import atexit
import hashlib
import importlib
import os
import re
import threading
from contextlib import contextmanager
from datetime import date, datetime
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterable, Sequence, TypeVar

from ait_protocol.common import utc_now
from .server_paths import ServerContext

_SCHEMA_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
_INSERT_OR_IGNORE_RE = re.compile(r"(?is)^\s*insert\s+or\s+ignore\s+into\s+")
_INSERT_TARGET_RE = re.compile(r'(?is)^\s*insert(?:\s+or\s+ignore)?\s+into\s+"?([A-Za-z_][A-Za-z0-9_]*)"?')
_POSTGRES_IDENTITY_COLUMNS = {
    "review_requests": "review_request_id",
    "reviews": "review_id",
    "policy_decisions": "policy_decision_id",
    "role_bindings": "binding_id",
    "jobs": "job_id",
    "audit_events": "event_id",
}
_SQLITE_WRITE_LOCKS: dict[str, threading.RLock] = {}
_SQLITE_WRITE_LOCKS_GUARD = threading.Lock()
_DEFAULT_SQLITE_BUSY_TIMEOUT_MS = 15_000
_DEFAULT_POSTGRES_POOL_MAX_SIZE = 16
POSTGRES_SCHEMA_VERSION_TABLE = "schema_versions"
EXPECTED_POSTGRES_SCHEMA_VERSIONS: dict[str, dict[str, Any]] = {
    "content": {
        "version": 3,
        "description": "M6 content schema for repo_id-scoped repositories, groups, refs, blobs, snapshots, trees, and packs.",
    },
    "control": {
        "version": 3,
        "description": "M6 control schema for repo_id-scoped workflow state plus authority-map persistence.",
    },
}
_POSTGRES_ADVISORY_LOCK_PERSON = b"ait-lock"
_POSTGRES_POOLS: dict[tuple[str, str], "_PostgresConnectionPool"] = {}
_POSTGRES_POOLS_GUARD = threading.Lock()
_POSTGRES_TIMEOUTS = threading.local()
_T = TypeVar("_T")


class PostgresSupportError(RuntimeError):
    pass


@dataclass
class DBResult:
    rows: list[dict[str, Any]]
    rowcount: int = -1
    lastrowid: int | None = None

    def fetchone(self) -> dict[str, Any] | None:
        return self.rows[0] if self.rows else None

    def fetchall(self) -> list[dict[str, Any]]:
        return list(self.rows)

    def __iter__(self):
        return iter(self.rows)


class DBConnection:
    def __init__(
        self,
        backend: str,
        raw: Any,
        *,
        sqlite_lock: threading.RLock | None = None,
        pool_release: Callable[[Any], None] | None = None,
        pool_discard: Callable[[Any], None] | None = None,
    ):
        self.backend = backend
        self.raw = raw
        self._sqlite_lock = sqlite_lock
        self._sqlite_lock_held = False
        self._pool_release = pool_release
        self._pool_discard = pool_discard
        self._closed = False

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        try:
            if exc_type is not None:
                try:
                    self.rollback()
                except Exception:
                    pass
        finally:
            self.close()
        return False

    def __del__(self):
        try:
            self.close()
        except Exception:
            pass

    def _ensure_sqlite_write_lock(self, sql: str) -> None:
        if self.backend != "sqlite" or self._sqlite_lock is None or self._sqlite_lock_held:
            return
        if not _sqlite_statement_requires_write_lock(sql):
            return
        self._sqlite_lock.acquire()
        self._sqlite_lock_held = True

    def _release_sqlite_write_lock(self) -> None:
        if self.backend != "sqlite" or self._sqlite_lock is None or not self._sqlite_lock_held:
            return
        self._sqlite_lock.release()
        self._sqlite_lock_held = False

    def _cleanup_failed_statement(self) -> None:
        try:
            self.raw.rollback()
        except Exception:
            pass
        finally:
            self._release_sqlite_write_lock()

    def execute(self, sql: str, params: Sequence[Any] | None = None) -> DBResult:
        params = tuple(params or ())
        query = normalize_sql(sql, self.backend)
        cur = None
        try:
            if self.backend == "sqlite":
                self._ensure_sqlite_write_lock(query)
            cur = self.raw.cursor()
            cur.execute(query, params)
            if self.backend == "sqlite":
                rows = _fetch_sqlite_rows(cur)
                return DBResult(rows=rows, rowcount=cur.rowcount, lastrowid=getattr(cur, "lastrowid", None))

            rows = _fetch_pg_rows(cur)
            lastrowid = getattr(cur, "lastrowid", None)
            if lastrowid is None and _looks_like_insert(sql):
                target_table = _insert_target_table(sql)
                identity_column = _POSTGRES_IDENTITY_COLUMNS.get(target_table or "")
                if identity_column is not None:
                    with self.raw.cursor() as aux:
                        aux.execute(
                            "select currval(pg_get_serial_sequence(current_schema() || '.' || %s, %s))",
                            (target_table, identity_column),
                        )
                        row = aux.fetchone()
                        if row:
                            lastrowid = row[0]
            return DBResult(rows=rows, rowcount=cur.rowcount, lastrowid=lastrowid)
        except Exception:
            self._cleanup_failed_statement()
            raise
        finally:
            if cur is not None:
                try:
                    cur.close()
                except Exception:
                    pass

    def executemany(self, sql: str, seq_of_params: Iterable[Sequence[Any]]) -> DBResult:
        query = normalize_sql(sql, self.backend)
        cur = None
        try:
            self._ensure_sqlite_write_lock(query)
            cur = self.raw.cursor()
            cur.executemany(query, list(seq_of_params))
            return DBResult(rows=[], rowcount=cur.rowcount, lastrowid=getattr(cur, "lastrowid", None))
        except Exception:
            self._cleanup_failed_statement()
            raise
        finally:
            if cur is not None:
                try:
                    cur.close()
                except Exception:
                    pass

    def executescript(self, script: str) -> None:
        for statement in split_sql_script(script):
            self.execute(statement)

    def commit(self) -> None:
        try:
            self.raw.commit()
        finally:
            self._release_sqlite_write_lock()

    def rollback(self) -> None:
        try:
            self.raw.rollback()
        finally:
            self._release_sqlite_write_lock()

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            if self.backend == "postgres" and self._pool_release is not None:
                raw = self.raw
                self.raw = None
                try:
                    raw.rollback()
                except Exception:
                    if self._pool_discard is not None:
                        self._pool_discard(raw)
                    else:
                        raw.close()
                else:
                    self._pool_release(raw)
                return
            self.raw.close()
        finally:
            self._release_sqlite_write_lock()


def _resolve_postgres_pool_max_size() -> int:
    raw = (os.environ.get("AIT_NATIVE_SERVER_POSTGRES_POOL_MAX_SIZE") or "").strip()
    if not raw:
        return _DEFAULT_POSTGRES_POOL_MAX_SIZE
    try:
        return max(int(raw), 1)
    except ValueError:
        return _DEFAULT_POSTGRES_POOL_MAX_SIZE


def _current_postgres_timeouts() -> tuple[int | None, int | None]:
    value = getattr(_POSTGRES_TIMEOUTS, "value", None)
    if not isinstance(value, tuple) or len(value) != 2:
        return None, None
    lock_timeout_ms, statement_timeout_ms = value
    return lock_timeout_ms, statement_timeout_ms


def _set_postgres_timeout(cur: Any, name: str, timeout_ms: int | None) -> None:
    if timeout_ms is None:
        cur.execute(f"reset {name}")
        return
    cur.execute(f"set {name} = '{max(int(timeout_ms), 1)}ms'")


def _configure_postgres_raw_connection(raw: Any, schema: str, *, ensure_schema: bool) -> None:
    with raw.cursor() as cur:
        if ensure_schema:
            cur.execute(f'create schema if not exists "{schema}"')
        cur.execute(f'set search_path to "{schema}", public')
        lock_timeout_ms, statement_timeout_ms = _current_postgres_timeouts()
        _set_postgres_timeout(cur, "lock_timeout", lock_timeout_ms)
        _set_postgres_timeout(cur, "statement_timeout", statement_timeout_ms)


class _PostgresConnectionPool:
    def __init__(self, dsn: str, schema: str, *, max_size: int):
        self.dsn = dsn
        self.schema = schema
        self.max_size = max_size
        self._idle: list[Any] = []
        self._total = 0
        self._closed = False
        self._condition = threading.Condition()

    def checkout(self) -> DBConnection:
        raw = None
        needs_create = False
        while True:
            with self._condition:
                while True:
                    if self._closed:
                        raise RuntimeError("PostgreSQL connection pool is closed")
                    if self._idle:
                        raw = self._idle.pop()
                        break
                    if self._total < self.max_size:
                        self._total += 1
                        needs_create = True
                        break
                    self._condition.wait()
            if needs_create:
                try:
                    psycopg = _load_psycopg()
                    raw = psycopg.connect(self.dsn)
                    _configure_postgres_raw_connection(raw, self.schema, ensure_schema=True)
                    break
                except Exception:
                    with self._condition:
                        self._total -= 1
                        self._condition.notify()
                    raise
            if raw is None:
                continue
            try:
                try:
                    raw.rollback()
                except Exception:
                    pass
                _configure_postgres_raw_connection(raw, self.schema, ensure_schema=False)
                break
            except Exception:
                self._discard_raw(raw)
                raw = None
                needs_create = False
                continue
        return DBConnection("postgres", raw, pool_release=self._release_raw, pool_discard=self._discard_raw)

    def _release_raw(self, raw: Any) -> None:
        with self._condition:
            if self._closed:
                try:
                    raw.close()
                finally:
                    self._total -= 1
                    self._condition.notify()
                return
            self._idle.append(raw)
            self._condition.notify()

    def _discard_raw(self, raw: Any) -> None:
        try:
            raw.close()
        finally:
            with self._condition:
                self._total -= 1
                self._condition.notify()

    def close(self) -> None:
        with self._condition:
            self._closed = True
            idle = list(self._idle)
            self._idle.clear()
        for raw in idle:
            try:
                raw.close()
            except Exception:
                pass
            finally:
                with self._condition:
                    self._total -= 1
                    self._condition.notify_all()


def close_postgres_connection_pools() -> None:
    with _POSTGRES_POOLS_GUARD:
        pools = list(_POSTGRES_POOLS.values())
        _POSTGRES_POOLS.clear()
    for pool in pools:
        pool.close()


def _postgres_connection_pool(dsn: str, schema: str) -> _PostgresConnectionPool:
    key = (dsn, schema)
    with _POSTGRES_POOLS_GUARD:
        pool = _POSTGRES_POOLS.get(key)
        if pool is None:
            pool = _PostgresConnectionPool(dsn, schema, max_size=_resolve_postgres_pool_max_size())
            _POSTGRES_POOLS[key] = pool
        return pool


def _looks_like_insert(sql: str) -> bool:
    return sql.lstrip().lower().startswith("insert")


def _insert_target_table(sql: str) -> str | None:
    match = _INSERT_TARGET_RE.match(sql)
    if match is None:
        return None
    return match.group(1)


def _sqlite_lock_for(db_path: Path) -> threading.RLock:
    key = str(db_path.resolve())
    with _SQLITE_WRITE_LOCKS_GUARD:
        lock = _SQLITE_WRITE_LOCKS.get(key)
        if lock is None:
            lock = threading.RLock()
            _SQLITE_WRITE_LOCKS[key] = lock
        return lock


def _resolve_sqlite_busy_timeout_ms(value: int | None = None) -> int:
    if value is not None:
        return max(int(value), 0)
    raw = (os.environ.get("AIT_NATIVE_SQLITE_BUSY_TIMEOUT_MS") or "").strip()
    if not raw:
        return _DEFAULT_SQLITE_BUSY_TIMEOUT_MS
    try:
        return max(int(raw), 0)
    except ValueError:
        return _DEFAULT_SQLITE_BUSY_TIMEOUT_MS


def _sqlite_statement_requires_write_lock(sql: str) -> bool:
    normalized = " ".join(str(sql).strip().lower().split())
    if not normalized:
        return False
    if normalized.startswith("select") or normalized.startswith("explain"):
        return False
    if normalized.startswith("pragma"):
        return "=" in normalized
    return True


def enable_sqlite_wal(conn: DBConnection) -> None:
    if conn.backend != "sqlite":
        return
    conn.execute("pragma journal_mode = wal")


def _fetch_sqlite_rows(cur) -> list[dict[str, Any]]:
    if cur.description is None:
        return []
    columns = [col[0] for col in cur.description]
    return [dict(zip(columns, row)) for row in cur.fetchall()]



def _convert_value(value: Any) -> Any:
    if isinstance(value, datetime):
        return value.isoformat()
    if isinstance(value, date):
        return value.isoformat()
    return value


def _fetch_pg_rows(cur) -> list[dict[str, Any]]:
    if cur.description is None:
        return []
    columns = []
    for col in cur.description:
        name = getattr(col, "name", None) or col[0]
        columns.append(name)
    return [dict(zip(columns, (_convert_value(item) for item in row))) for row in cur.fetchall()]



def _ensure_schema_name(schema: str) -> str:
    if not _SCHEMA_RE.match(schema):
        raise ValueError(f"Invalid schema name: {schema}")
    return schema



def postgres_support_installed() -> bool:
    return importlib.util.find_spec("psycopg") is not None



def _load_psycopg():
    if not postgres_support_installed():
        raise PostgresSupportError(
            "PostgreSQL backend requested but psycopg is not installed. "
            "Install with: pip install 'ait-native[postgres]'"
        )
    return importlib.import_module("psycopg")



def connect_sqlite_runtime(db_path: Path, *, busy_timeout_ms: int | None = None) -> DBConnection:
    raise RuntimeError(
        "AIT server SQLite runtime support has been removed; "
        "server runtime planes must use PostgreSQL and must not open content.db/control.db."
    )



def connect_postgres_runtime(dsn: str, schema: str) -> DBConnection:
    schema = _ensure_schema_name(schema)
    return _postgres_connection_pool(dsn, schema).checkout()



def connect_server_plane(ctx: ServerContext, plane: str, *, busy_timeout_ms: int | None = None) -> DBConnection:
    if plane not in {"content", "control"}:
        raise ValueError(f"Unknown plane: {plane}")
    if ctx.db_backend == "postgres":
        if not ctx.postgres_dsn:
            raise PostgresSupportError("PostgreSQL backend requested but AIT_NATIVE_SERVER_POSTGRES_DSN is not configured")
        schema = ctx.content_schema if plane == "content" else ctx.control_schema
        return connect_postgres_runtime(ctx.postgres_dsn, schema)
    raise RuntimeError(
        f"Unsupported AIT server database backend for {plane} plane: {ctx.db_backend!r}. "
        "Only PostgreSQL is supported for ait-server runtime state."
    )


def _run_server_plane(
    ctx: ServerContext,
    plane: str,
    callback: Callable[[DBConnection], _T],
    *,
    write: bool,
    lock_timeout_ms: int | None = None,
    statement_timeout_ms: int | None = None,
) -> _T:
    with postgres_statement_timeouts(
        lock_timeout_ms=lock_timeout_ms,
        statement_timeout_ms=statement_timeout_ms,
    ):
        with connect_server_plane(ctx, plane) as conn:
            result = callback(conn)
            if write:
                conn.commit()
            return result


def read_server_plane(
    ctx: ServerContext,
    plane: str,
    callback: Callable[[DBConnection], _T],
    *,
    lock_timeout_ms: int | None = None,
    statement_timeout_ms: int | None = None,
) -> _T:
    """Run a callback inside an auto-cleaned server-plane read scope."""

    return _run_server_plane(
        ctx,
        plane,
        callback,
        write=False,
        lock_timeout_ms=lock_timeout_ms,
        statement_timeout_ms=statement_timeout_ms,
    )


def write_server_plane(
    ctx: ServerContext,
    plane: str,
    callback: Callable[[DBConnection], _T],
    *,
    lock_timeout_ms: int | None = None,
    statement_timeout_ms: int | None = None,
) -> _T:
    """Run a callback inside an auto-committed server-plane write scope."""

    return _run_server_plane(
        ctx,
        plane,
        callback,
        write=True,
        lock_timeout_ms=lock_timeout_ms,
        statement_timeout_ms=statement_timeout_ms,
    )


def expected_postgres_schema_versions() -> dict[str, dict[str, Any]]:
    return {
        plane: {"version": int(metadata["version"]), "description": str(metadata["description"])}
        for plane, metadata in EXPECTED_POSTGRES_SCHEMA_VERSIONS.items()
    }


def _expected_schema_metadata(plane: str) -> dict[str, Any]:
    metadata = EXPECTED_POSTGRES_SCHEMA_VERSIONS.get(plane)
    if metadata is None:
        raise ValueError(f"Unknown schema plane: {plane}")
    return metadata


def postgres_advisory_lock_key(scope: str) -> tuple[int, int]:
    text = str(scope or "").strip()
    if not text:
        raise ValueError("scope is required")
    digest = hashlib.blake2b(text.encode("utf-8"), digest_size=8, person=_POSTGRES_ADVISORY_LOCK_PERSON).digest()
    value = int.from_bytes(digest, "big", signed=False)
    return ((value >> 32) & 0x7FFFFFFF, value & 0x7FFFFFFF)


@contextmanager
def postgres_advisory_lock(conn: DBConnection, *, scope: str):
    if conn.backend != "postgres":
        yield
        return
    key_hi, key_lo = postgres_advisory_lock_key(scope)
    conn.execute("select pg_advisory_lock(?, ?)", (key_hi, key_lo))
    try:
        yield
    finally:
        try:
            conn.execute("select pg_advisory_unlock(?, ?)", (key_hi, key_lo))
        except Exception:
            pass


@contextmanager
def postgres_statement_timeouts(
    *,
    lock_timeout_ms: int | None = None,
    statement_timeout_ms: int | None = None,
):
    previous = getattr(_POSTGRES_TIMEOUTS, "value", None)
    _POSTGRES_TIMEOUTS.value = (lock_timeout_ms, statement_timeout_ms)
    try:
        yield
    finally:
        if previous is None:
            try:
                delattr(_POSTGRES_TIMEOUTS, "value")
            except AttributeError:
                pass
        else:
            _POSTGRES_TIMEOUTS.value = previous


def ensure_schema_version(conn: DBConnection, *, plane: str) -> dict[str, Any]:
    """Create/update the per-schema version row for a PostgreSQL runtime plane."""

    if conn.backend != "postgres":
        return {
            "plane": plane,
            "backend": conn.backend,
            "table_present": False,
            "expected_version": None,
            "version": None,
            "ok": True,
            "skipped": True,
        }

    metadata = _expected_schema_metadata(plane)
    expected_version = int(metadata["version"])
    description = str(metadata["description"])
    now = utc_now()
    conn.execute(
        f"""
        create table if not exists {POSTGRES_SCHEMA_VERSION_TABLE} (
            plane text primary key,
            version integer not null,
            description text not null,
            applied_at timestamptz not null,
            checked_at timestamptz not null
        )
        """
    )
    row = conn.execute(
        f"select plane, version, description, applied_at, checked_at from {POSTGRES_SCHEMA_VERSION_TABLE} where plane = ?",
        (plane,),
    ).fetchone()
    if row is None:
        conn.execute(
            f"""
            insert into {POSTGRES_SCHEMA_VERSION_TABLE}(plane, version, description, applied_at, checked_at)
            values (?, ?, ?, ?, ?)
            """,
            (plane, expected_version, description, now, now),
        )
    else:
        current_version = int(row["version"])
        if current_version > expected_version:
            raise RuntimeError(
                f"PostgreSQL {plane} schema version {current_version} is newer than this server supports "
                f"({expected_version})."
            )
        if current_version < expected_version:
            conn.execute(
                f"""
                update {POSTGRES_SCHEMA_VERSION_TABLE}
                set version = ?, description = ?, applied_at = ?, checked_at = ?
                where plane = ?
                """,
                (expected_version, description, now, now, plane),
            )
        else:
            conn.execute(
                f"""
                update {POSTGRES_SCHEMA_VERSION_TABLE}
                set description = ?, checked_at = ?
                where plane = ?
                """,
                (description, now, plane),
            )
    return schema_version_status(conn, plane=plane)


def schema_version_status(conn: DBConnection, *, plane: str) -> dict[str, Any]:
    metadata = _expected_schema_metadata(plane)
    expected_version = int(metadata["version"])
    try:
        row = conn.execute(
            f"select plane, version, description, applied_at, checked_at from {POSTGRES_SCHEMA_VERSION_TABLE} where plane = ?",
            (plane,),
        ).fetchone()
    except Exception as exc:
        return {
            "plane": plane,
            "backend": conn.backend,
            "table_present": False,
            "expected_version": expected_version,
            "version": None,
            "ok": False,
            "error": str(exc),
        }
    if row is None:
        return {
            "plane": plane,
            "backend": conn.backend,
            "table_present": True,
            "expected_version": expected_version,
            "version": None,
            "ok": False,
            "error": "schema version row is missing",
        }
    version = int(row["version"])
    return {
        "plane": plane,
        "backend": conn.backend,
        "table_present": True,
        "expected_version": expected_version,
        "version": version,
        "description": row.get("description"),
        "applied_at": row.get("applied_at"),
        "checked_at": row.get("checked_at"),
        "ok": version == expected_version,
        "error": None if version == expected_version else f"expected schema version {expected_version}, found {version}",
    }


def postgres_schema_upgrade_checks(ctx: ServerContext, *, apply: bool = True) -> dict[str, Any]:
    if ctx.db_backend != "postgres":
        return {
            "backend": ctx.db_backend,
            "applied": False,
            "ok": False,
            "checks": {},
            "error": "PostgreSQL schema upgrade checks require the postgres backend.",
        }

    if apply:
        from . import server_content, server_control

        server_content.initialize(ctx)
        server_control.initialize(ctx)

    checks: dict[str, dict[str, Any]] = {}
    for plane in ("content", "control"):
        conn = connect_server_plane(ctx, plane)
        try:
            checks[plane] = schema_version_status(conn, plane=plane)
        finally:
            conn.close()
    return {
        "backend": ctx.db_backend,
        "applied": bool(apply),
        "expected_versions": expected_postgres_schema_versions(),
        "checks": checks,
        "ok": all(check.get("ok") for check in checks.values()),
    }



def normalize_sql(sql: str, backend: str) -> str:
    if backend == "sqlite":
        return sql
    out = sql.strip()
    semicolon = out.endswith(";")
    out = out.rstrip(";\n \t")
    if _INSERT_OR_IGNORE_RE.match(out):
        out = _INSERT_OR_IGNORE_RE.sub("insert into ", out, count=1)
        if " on conflict " not in out.lower():
            out = f"{out} on conflict do nothing"
    out = out.replace("?", "%s")
    if semicolon:
        out += ";"
    return out



def split_sql_script(script: str) -> list[str]:
    statements: list[str] = []
    current: list[str] = []
    in_single = False
    in_double = False
    prev = ""
    for ch in script:
        if ch == "'" and not in_double and prev != "\\":
            in_single = not in_single
        elif ch == '"' and not in_single and prev != "\\":
            in_double = not in_double
        if ch == ";" and not in_single and not in_double:
            statement = "".join(current).strip()
            if statement:
                statements.append(statement)
            current = []
        else:
            current.append(ch)
        prev = ch
    tail = "".join(current).strip()
    if tail:
        statements.append(tail)
    return statements



def postgres_runtime_summary(ctx: ServerContext) -> dict[str, Any]:
    return {
        "backend": ctx.db_backend,
        "server_data_root": str(ctx.root),
        "content_db_path": str(ctx.content_db_path),
        "control_db_path": str(ctx.control_db_path),
        "postgres_dsn_configured": bool(ctx.postgres_dsn),
        "content_schema": ctx.content_schema,
        "control_schema": ctx.control_schema,
        "timestamp": utc_now(),
    }


def postgres_preflight(ctx: ServerContext, *, attempt_connect: bool = False) -> dict[str, Any]:
    summary = postgres_runtime_summary(ctx)
    issues: list[str] = []
    warnings: list[str] = []

    if ctx.db_backend != "postgres":
        warnings.append(
            f"Server backend is configured as {ctx.db_backend!r}; "
            "set AIT_NATIVE_SERVER_DB_BACKEND=postgres or use --backend postgres to validate the PostgreSQL path."
        )

    if not ctx.postgres_dsn:
        issues.append("AIT_NATIVE_SERVER_POSTGRES_DSN is not configured.")

    content_schema_valid = True
    control_schema_valid = True
    try:
        _ensure_schema_name(ctx.content_schema)
    except ValueError as exc:
        content_schema_valid = False
        issues.append(str(exc))
    try:
        _ensure_schema_name(ctx.control_schema)
    except ValueError as exc:
        control_schema_valid = False
        issues.append(str(exc))

    psycopg_installed = postgres_support_installed()
    if not psycopg_installed:
        issues.append("psycopg is not installed. Install with: pip install 'ait-native[postgres]'")

    repo_root = Path(__file__).resolve().parent.parent.parent
    content_schema_file = repo_root / "sql" / "ait_native_postgres_content_schema.sql"
    control_schema_file = repo_root / "sql" / "ait_native_postgres_control_schema.sql"
    if not content_schema_file.exists():
        issues.append(f"Missing PostgreSQL content schema file: {content_schema_file}")
    if not control_schema_file.exists():
        issues.append(f"Missing PostgreSQL control schema file: {control_schema_file}")

    live_connection_ok: bool | None = None
    live_connection_error: str | None = None
    schema_upgrade_checks: dict[str, Any] | None = None
    if attempt_connect:
        if issues:
            live_connection_ok = False
            live_connection_error = "Skipped live PostgreSQL connection attempt because preflight issues were already detected."
        else:
            try:
                schema_upgrade_checks = postgres_schema_upgrade_checks(ctx, apply=True)
                if not schema_upgrade_checks.get("ok"):
                    raise RuntimeError("PostgreSQL schema upgrade checks did not reach the expected versions.")
                live_connection_ok = True
            except Exception as exc:
                live_connection_ok = False
                live_connection_error = str(exc)
                issues.append(f"Live PostgreSQL connection failed: {exc}")

    return {
        **summary,
        "psycopg_installed": psycopg_installed,
        "content_schema_valid": content_schema_valid,
        "control_schema_valid": control_schema_valid,
        "schema_files": {
            "content": {"path": str(content_schema_file), "exists": content_schema_file.exists()},
            "control": {"path": str(control_schema_file), "exists": control_schema_file.exists()},
        },
        "attempted_live_connect": bool(attempt_connect),
        "live_connection_ok": live_connection_ok,
        "live_connection_error": live_connection_error,
        "schema_upgrade_checks": schema_upgrade_checks,
        "issues": issues,
        "warnings": warnings,
        "ready": not issues,
    }


atexit.register(close_postgres_connection_pools)
