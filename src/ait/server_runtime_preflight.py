from __future__ import annotations

import os
from pathlib import Path

from .server_runtime_seam import ServerContext, postgres_preflight, resolve_server_runtime_root

DEFAULT_POSTGRES_BACKEND = "postgres"
DEFAULT_POSTGRES_CONTENT_SCHEMA = "ait_native_content"
DEFAULT_POSTGRES_CONTROL_SCHEMA = "ait_native_control"


def resolve_effective_server_runtime_root(server_data: Path | None) -> Path:
    return server_data or resolve_server_runtime_root()


def _resolved_postgres_runtime_params(
    *,
    backend: str | None,
    dsn: str | None,
    content_schema: str | None,
    control_schema: str | None,
) -> tuple[str, str | None, str, str]:
    resolved_backend = str(backend or os.environ.get("AIT_NATIVE_SERVER_DB_BACKEND") or DEFAULT_POSTGRES_BACKEND).strip().lower() or DEFAULT_POSTGRES_BACKEND
    resolved_dsn = str(dsn or os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN") or "").strip() or None
    resolved_content_schema = (
        str(content_schema or os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA") or DEFAULT_POSTGRES_CONTENT_SCHEMA).strip()
        or DEFAULT_POSTGRES_CONTENT_SCHEMA
    )
    resolved_control_schema = (
        str(control_schema or os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA") or DEFAULT_POSTGRES_CONTROL_SCHEMA).strip()
        or DEFAULT_POSTGRES_CONTROL_SCHEMA
    )
    return (
        resolved_backend,
        resolved_dsn,
        resolved_content_schema,
        resolved_control_schema,
    )


def create_postgres_server_context(
    *,
    server_data: Path | None,
    backend: str | None,
    dsn: str | None,
    content_schema: str | None,
    control_schema: str | None,
) -> ServerContext:
    root = resolve_effective_server_runtime_root(server_data)
    resolved_backend, resolved_dsn, resolved_content_schema, resolved_control_schema = _resolved_postgres_runtime_params(
        backend=backend,
        dsn=dsn,
        content_schema=content_schema,
        control_schema=control_schema,
    )
    return ServerContext.create(
        root,
        backend=resolved_backend,
        postgres_dsn=resolved_dsn,
        content_schema=resolved_content_schema,
        control_schema=resolved_control_schema,
    )


def postgres_preflight_report(
    *,
    server_data: Path | None,
    backend: str | None,
    dsn: str | None,
    content_schema: str | None,
    control_schema: str | None,
    connect: bool,
) -> dict[str, object]:
    ctx = create_postgres_server_context(
        server_data=server_data,
        backend=backend,
        dsn=dsn,
        content_schema=content_schema,
        control_schema=control_schema,
    )
    return postgres_preflight(ctx, attempt_connect=connect)


__all__ = [
    "ServerContext",
    "create_postgres_server_context",
    "postgres_preflight_report",
    "resolve_effective_server_runtime_root",
]
