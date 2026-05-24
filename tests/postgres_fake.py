from __future__ import annotations

import re
import sqlite3
import importlib
import importlib.util
from pathlib import Path
from types import SimpleNamespace
from urllib.parse import unquote, urlparse


_CREATE_SCHEMA_RE = re.compile(r'^\s*create\s+schema\s+if\s+not\s+exists\s+"?([A-Za-z_][A-Za-z0-9_]*)"?\s*;?\s*$', re.IGNORECASE)
_SET_SEARCH_PATH_RE = re.compile(r'^\s*set\s+search_path\s+to\s+"?([A-Za-z_][A-Za-z0-9_]*)"?\s*,\s*public\s*;?\s*$', re.IGNORECASE)
_SET_TIMEOUT_RE = re.compile(r"^\s*set\s+(?:lock_timeout|statement_timeout)\s*=\s*'?\d+ms'?\s*;?\s*$", re.IGNORECASE)
_RESET_TIMEOUT_RE = re.compile(r"^\s*reset\s+(?:lock_timeout|statement_timeout)\s*;?\s*$", re.IGNORECASE)
_ALTER_TABLE_ADD_COLUMN_IF_NOT_EXISTS_RE = re.compile(
    r'^\s*alter\s+table(?:\s+if\s+exists)?\s+"?([A-Za-z_][A-Za-z0-9_]*)"?\s+add\s+column\s+if\s+not\s+exists\s+"?([A-Za-z_][A-Za-z0-9_]*)"?\b',
    re.IGNORECASE,
)
_ORIGINAL_LOAD_PSYCOPG = None
_ORIGINAL_POSTGRES_SUPPORT_INSTALLED = None


def _remember_real_postgres_driver() -> None:
    global _ORIGINAL_LOAD_PSYCOPG, _ORIGINAL_POSTGRES_SUPPORT_INSTALLED
    if _ORIGINAL_LOAD_PSYCOPG is not None and _ORIGINAL_POSTGRES_SUPPORT_INSTALLED is not None:
        return
    from ait_native import server_db

    _ORIGINAL_LOAD_PSYCOPG = server_db._load_psycopg
    _ORIGINAL_POSTGRES_SUPPORT_INSTALLED = server_db.postgres_support_installed


def install_fake_psycopg(monkeypatch):
    from ait_native import server_db

    _remember_real_postgres_driver()
    fake = FakePsycopg()
    monkeypatch.setattr(server_db, "_load_psycopg", lambda: fake)
    monkeypatch.setattr(server_db, "postgres_support_installed", lambda: True)
    return fake


def fake_postgres_root(data_dir: Path) -> Path:
    return (data_dir / "fake-postgres-runtime").resolve()


def fake_postgres_dsn(data_dir: Path) -> str:
    return f"fake-postgres:///{fake_postgres_root(data_dir)}"


def fake_postgres_schema_db_path(data_dir: Path, schema: str) -> Path:
    return fake_postgres_root(data_dir) / f"{schema}.sqlite3"


def install_fake_psycopg_global() -> None:
    from ait_native import server_db

    _remember_real_postgres_driver()
    server_db._load_psycopg = lambda: FakePsycopg()
    server_db.postgres_support_installed = lambda: True


def restore_real_psycopg() -> None:
    from ait_native import server_db

    def real_postgres_support_installed() -> bool:
        return importlib.util.find_spec("psycopg") is not None

    def real_load_psycopg():
        if not server_db.postgres_support_installed():
            raise server_db.PostgresSupportError(
                "PostgreSQL backend requested but psycopg is not installed. "
                "Install with: pip install 'ait-native[postgres]'"
            )
        return importlib.import_module("psycopg")

    server_db._load_psycopg = real_load_psycopg
    server_db.postgres_support_installed = real_postgres_support_installed
    reset_fake_postgres_runtime()


def reset_fake_postgres_runtime() -> None:
    from ait_native.server_db import close_postgres_connection_pools

    close_postgres_connection_pools()
    try:
        from ait_native.server_content import reset_postgres_schema_ready_cache
    except Exception:
        return
    reset_postgres_schema_ready_cache()


def fake_postgres_context(data_dir: Path):
    from ait_native.server_paths import ServerContext

    install_fake_psycopg_global()
    reset_fake_postgres_runtime()
    return ServerContext.create(data_dir, backend="postgres", postgres_dsn=fake_postgres_dsn(data_dir))


class FakePsycopg:
    def connect(self, dsn: str):
        return FakeConnection(dsn)


class FakeConnection:
    def __init__(self, dsn: str):
        self.root = _dsn_root(dsn)
        self.root.mkdir(parents=True, exist_ok=True)
        self.current_schema = "public"
        self._connections: dict[str, sqlite3.Connection] = {}
        self._lastrowid: int | None = None

    def cursor(self):
        return FakeCursor(self)

    def _schema_conn(self, schema: str | None = None) -> sqlite3.Connection:
        schema_name = schema or self.current_schema
        conn = self._connections.get(schema_name)
        if conn is None:
            db_path = self.root / f"{schema_name}.sqlite3"
            conn = sqlite3.connect(db_path, check_same_thread=False)
            conn.execute("pragma foreign_keys = on")
            self._connections[schema_name] = conn
        return conn

    def commit(self) -> None:
        for conn in self._connections.values():
            conn.commit()

    def rollback(self) -> None:
        for conn in self._connections.values():
            conn.rollback()

    def close(self) -> None:
        for conn in self._connections.values():
            conn.close()
        self._connections.clear()


class FakeCursor:
    def __init__(self, connection: FakeConnection):
        self.connection = connection
        self._cursor: sqlite3.Cursor | None = None
        self._rows: list[tuple] = []
        self.description = None
        self.rowcount = -1
        self.lastrowid = None

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        self.close()
        return False

    def execute(self, sql: str, params=()):
        self._rows = []
        self.description = None
        self.rowcount = -1
        self.lastrowid = None
        text = sql.strip()

        match = _CREATE_SCHEMA_RE.match(text)
        if match:
            self.connection._schema_conn(match.group(1))
            return self

        match = _SET_SEARCH_PATH_RE.match(text)
        if match:
            schema = match.group(1)
            self.connection.current_schema = schema
            self.connection._schema_conn(schema)
            return self

        if _SET_TIMEOUT_RE.match(text) or _RESET_TIMEOUT_RE.match(text):
            return self

        if text.lower() == "select lastval()":
            self._rows = [(self.connection._lastrowid,)]
            self.description = [SimpleNamespace(name="lastval")]
            return self

        if re.match(r"^\s*select\s+pg_advisory_(?:lock|unlock)\s*\(", text, re.IGNORECASE):
            self._rows = [(True,)]
            self.description = [SimpleNamespace(name="pg_advisory_lock")]
            return self

        if "information_schema.columns" in text.lower():
            values = tuple(params or ())
            if len(values) == 1:
                table_name = values[0]
                pragma = self.connection._schema_conn().execute(f"pragma table_info({table_name})")
                self._rows = [(row[1],) for row in pragma.fetchall()]
                self.description = [SimpleNamespace(name="column_name")]
                return self
            table_name, column_name = values
            pragma = self.connection._schema_conn().execute(f"pragma table_info({table_name})")
            names = {row[1] for row in pragma.fetchall()}
            if column_name in names:
                self._rows = [(1,)]
                self.description = [SimpleNamespace(name="?column?")]
            return self

        if "information_schema.tables" in text.lower():
            values = tuple(params or ())
            table_name = values[0] if values else None
            if table_name:
                pragma = self.connection._schema_conn().execute(
                    "select name from sqlite_master where type in ('table', 'view') and name = ?",
                    (table_name,),
                )
                if pragma.fetchone() is not None:
                    self._rows = [(1,)]
                    self.description = [SimpleNamespace(name="?column?")]
            return self

        if "information_schema.views" in text.lower():
            values = tuple(params or ())
            table_name = values[0] if values else None
            if not table_name:
                view_match = re.search(r"table_name\s*=\s*'([^']+)'", text, re.IGNORECASE)
                table_name = view_match.group(1) if view_match is not None else None
            if table_name:
                pragma = self.connection._schema_conn().execute(
                    "select name from sqlite_master where type = 'view' and name = ?",
                    (table_name,),
                )
                if pragma.fetchone() is not None:
                    self._rows = [(1,)]
                    self.description = [SimpleNamespace(name="?column?")]
            return self

        match = _ALTER_TABLE_ADD_COLUMN_IF_NOT_EXISTS_RE.match(text)
        if match:
            table_name, column_name = match.groups()
            pragma = self.connection._schema_conn().execute(f"pragma table_info({table_name})")
            names = {row[1] for row in pragma.fetchall()}
            if column_name in names:
                return self

        sqlite_sql = _translate_sql(text)
        sqlite_params = tuple(params or ())
        cur = self.connection._schema_conn().cursor()
        cur.execute(sqlite_sql, sqlite_params)
        self._cursor = cur
        self.rowcount = cur.rowcount
        self.lastrowid = getattr(cur, "lastrowid", None)
        if self.lastrowid is not None:
            self.connection._lastrowid = self.lastrowid
        if cur.description is not None:
            self._rows = cur.fetchall()
            self.description = [SimpleNamespace(name=col[0]) for col in cur.description]
        return self

    def executemany(self, sql: str, seq_of_params):
        self._rows = []
        self.description = None
        sqlite_sql = _translate_sql(sql)
        cur = self.connection._schema_conn().cursor()
        cur.executemany(sqlite_sql, list(seq_of_params))
        self._cursor = cur
        self.rowcount = cur.rowcount
        self.lastrowid = getattr(cur, "lastrowid", None)
        if self.lastrowid is not None:
            self.connection._lastrowid = self.lastrowid
        return self

    def fetchall(self):
        return list(self._rows)

    def fetchone(self):
        if not self._rows:
            return None
        return self._rows.pop(0)

    def close(self):
        if self._cursor is not None:
            self._cursor.close()
            self._cursor = None


def _dsn_root(dsn: str) -> Path:
    parsed = urlparse(dsn)
    if parsed.scheme != "fake-postgres":
        raise ValueError(f"Unsupported fake postgres DSN: {dsn}")
    return Path(unquote(parsed.path)).resolve()


def _translate_sql(sql: str) -> str:
    out = sql.strip().rstrip(";")
    out = re.sub(r"\balter\s+table\s+if\s+exists\b", "alter table", out, flags=re.IGNORECASE)
    out = re.sub(r"\badd\s+column\s+if\s+not\s+exists\b", "add column", out, flags=re.IGNORECASE)
    out = re.sub(r"\bdrop\s+column\s+if\s+exists\b", "drop column", out, flags=re.IGNORECASE)
    out = re.sub(r"\s+for\s+update\s+skip\s+locked\b", "", out, flags=re.IGNORECASE)
    out = re.sub(r"\bbigserial\s+primary\s+key\b", "integer primary key autoincrement", out, flags=re.IGNORECASE)
    out = re.sub(r"\bbigserial\b", "integer", out, flags=re.IGNORECASE)
    out = re.sub(r"\bbigint\b", "integer", out, flags=re.IGNORECASE)
    out = re.sub(r"\btimestamptz\b", "text", out, flags=re.IGNORECASE)
    out = re.sub(r"\bboolean\b", "integer", out, flags=re.IGNORECASE)
    out = out.replace("%s", "?")
    return out
