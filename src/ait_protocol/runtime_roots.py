from __future__ import annotations

import os
from pathlib import Path


def resolve_server_runtime_root_with_source(
    path: str | os.PathLike[str] | None = None,
) -> tuple[Path, str]:
    if path is None:
        env_root = (os.environ.get("AIT_NATIVE_SERVER_DATA") or "").strip()
        if env_root:
            return Path(env_root).expanduser(), "env"
        raise RuntimeError(
            "AIT_NATIVE_SERVER_DATA is required for server runtime access; "
            "platform default runtime roots are no longer supported."
        )
    return Path(path).expanduser(), "explicit"


def resolve_server_runtime_root(path: str | os.PathLike[str] | None = None) -> Path:
    return resolve_server_runtime_root_with_source(path)[0]
