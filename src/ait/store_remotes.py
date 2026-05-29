from __future__ import annotations

from typing import Optional

from . import local_control
from .repo_paths import RepoContext
from .store_repo_config import load_config, update_config

__all__ = [
    "add_remote",
    "get_remote",
    "list_remotes",
]


def add_remote(ctx: RepoContext, name: str, url: str, repo_name: Optional[str], make_default: bool = False) -> dict:
    row = local_control.add_remote(ctx, name, url, repo_name, make_default=make_default)
    if make_default:
        update_config(ctx, lambda cfg: cfg.__setitem__("default_remote", name))
    return row


def list_remotes(ctx: RepoContext) -> list[dict]:
    return local_control.list_remotes(ctx)


def get_remote(ctx: RepoContext, name: Optional[str] = None) -> dict:
    resolved_name = name
    if resolved_name is None:
        resolved_name = load_config(ctx).get("default_remote")
    if not resolved_name:
        raise KeyError("No remote configured. Run `ait remote add ... --default` first.")
    return local_control.get_remote(ctx, resolved_name)
