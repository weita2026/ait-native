from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from ait_protocol.runtime_roots import resolve_server_runtime_root


@dataclass(frozen=True)
class ServerContext:
    root: Path

    @classmethod
    def from_env(cls) -> "ServerContext":
        return cls(root=resolve_server_runtime_root())


def get_worktree(*args, **kwargs):
    # Keep the agent/server seam import-safe during runtime_bindings bootstrap.
    from .local_runtime_seam import get_worktree as local_get_worktree

    return local_get_worktree(*args, **kwargs)


def resolve_bound_repo_root(*args, **kwargs):
    # Lazy import avoids pulling `ait` CLI bootstrap into runtime_bindings import time.
    from .local_runtime_seam import resolve_bound_repo_root as local_resolve_bound_repo_root

    return local_resolve_bound_repo_root(*args, **kwargs)

__all__ = [
    "ServerContext",
    "get_worktree",
    "resolve_bound_repo_root",
    "resolve_server_runtime_root",
]
