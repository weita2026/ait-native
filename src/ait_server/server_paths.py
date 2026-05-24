from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from ait_protocol.runtime_roots import resolve_server_runtime_root, resolve_server_runtime_root_with_source


@dataclass(frozen=True)
class ServerContext:
    root: Path
    content_db_path: Path
    control_db_path: Path
    db_backend: str = "postgres"
    postgres_dsn: str | None = None
    content_schema: str = "ait_native_content"
    control_schema: str = "ait_native_control"
    root_source: str = "explicit"

    @property
    def manifest_dir(self) -> Path:
        return self.root / "objects" / "manifests"

    @property
    def pack_dir(self) -> Path:
        return self.root / "objects" / "packs"

    @property
    def tree_pack_dir(self) -> Path:
        return self.root / "objects" / "tree-packs"

    @property
    def ref_root(self) -> Path:
        return self.root / "refs"

    @property
    def using_postgres(self) -> bool:
        return self.db_backend == "postgres"

    @classmethod
    def create(
        cls,
        root: Path,
        *,
        backend: str = "postgres",
        postgres_dsn: str | None = None,
        content_schema: str = "ait_native_content",
        control_schema: str = "ait_native_control",
        root_source: str = "explicit",
    ) -> "ServerContext":
        root = root.resolve()
        root.mkdir(parents=True, exist_ok=True)
        (root / "objects" / "manifests").mkdir(parents=True, exist_ok=True)
        (root / "objects" / "packs").mkdir(parents=True, exist_ok=True)
        (root / "objects" / "tree-packs").mkdir(parents=True, exist_ok=True)
        (root / "refs").mkdir(parents=True, exist_ok=True)
        backend = (backend or "postgres").strip().lower()
        if backend == "sqlite":
            raise RuntimeError(
                "AIT server SQLite runtime support has been removed; "
                "set AIT_NATIVE_SERVER_DB_BACKEND=postgres and use PostgreSQL for ait-server state."
            )
        if backend != "postgres":
            raise ValueError(f"Unsupported AIT native server database backend: {backend!r}")
        return cls(
            root=root,
            content_db_path=root / "content.db",
            control_db_path=root / "control.db",
            db_backend=backend,
            postgres_dsn=postgres_dsn,
            content_schema=content_schema,
            control_schema=control_schema,
            root_source=root_source,
        )

    @classmethod
    def from_env(cls) -> "ServerContext":
        root, root_source = resolve_server_runtime_root_with_source()
        backend = (os.environ.get("AIT_NATIVE_SERVER_DB_BACKEND") or "").strip().lower()
        if not backend:
            raise RuntimeError(
                "AIT_NATIVE_SERVER_DB_BACKEND is required for server runtime startup; "
                "set it explicitly to 'postgres'."
            )
        if backend == "sqlite":
            raise RuntimeError(
                "AIT_NATIVE_SERVER_DB_BACKEND=sqlite is no longer supported; "
                "ait-server state must use PostgreSQL."
            )
        if backend != "postgres":
            raise RuntimeError(f"Unsupported AIT_NATIVE_SERVER_DB_BACKEND value: {backend!r}")
        postgres_dsn = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN")
        if backend == "postgres" and not (postgres_dsn or "").strip():
            raise RuntimeError("AIT_NATIVE_SERVER_POSTGRES_DSN is required when AIT_NATIVE_SERVER_DB_BACKEND=postgres.")
        content_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
        control_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")
        return cls.create(
            root,
            backend=backend,
            postgres_dsn=postgres_dsn,
            content_schema=content_schema,
            control_schema=control_schema,
            root_source=root_source,
        )
