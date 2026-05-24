from __future__ import annotations

from typing import Any


_REMOVED_MESSAGE = (
    "AIT server SQLite cutover tooling has been removed; "
    "ait-server runtime state is PostgreSQL-only."
)


def assess_sqlite_runtime_quiesce(*_: Any, **__: Any) -> dict[str, Any]:
    raise RuntimeError(_REMOVED_MESSAGE)


def migrate_sqlite_runtime_to_postgres(*_: Any, **__: Any) -> dict[str, Any]:
    raise RuntimeError(_REMOVED_MESSAGE)


def validate_sqlite_postgres_parity(*_: Any, **__: Any) -> dict[str, Any]:
    raise RuntimeError(_REMOVED_MESSAGE)
