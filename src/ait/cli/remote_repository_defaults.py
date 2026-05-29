from __future__ import annotations

import importlib
from typing import Any, Optional

from ..remote_client import (
    RemoteError,
    ensure_repository as _default_ensure_repository,
    get_repository as _default_remote_get_repository,
)
from ..store import (
    RepoContext,
    effective_id_namespace_prefix as repo_id_namespace_prefix,
    load_config,
    load_policy,
)
from ..store_remotes import (
    get_remote,
)


def _remote_tuple(ctx: RepoContext, remote_name: Optional[str]) -> tuple[dict[str, Any], str]:
    remote = get_remote(ctx, remote_name)
    cfg = load_config(ctx)
    repo_name = remote.get("repo_name") or cfg["repo_name"]
    return remote, repo_name


def _remote_error_status_code(exc: RemoteError) -> int | None:
    message = str(exc)
    marker = " failed: "
    if marker not in message:
        return None
    status_text = message.split(marker, 1)[1].split(" ", 1)[0]
    return int(status_text) if status_text.isdigit() else None


def _cli_app_override(name: str, default: Any) -> Any:
    cli_app = importlib.import_module("ait.cli.app")

    return getattr(cli_app, name, default)


def _verify_remote_repository(base_url: str, repo_name: str, expected_default_line: str) -> dict[str, Any]:
    remote_get_repository = _cli_app_override("remote_get_repository", _default_remote_get_repository)
    try:
        remote_repo = remote_get_repository(base_url, repo_name)
    except RemoteError as exc:
        raise RemoteError(f"Remote repository {repo_name} could not be verified after ensure/create: {exc}") from exc
    if remote_repo.get("repo_name") != repo_name:
        raise RemoteError(
            f"Remote repository verification returned unexpected repository {remote_repo.get('repo_name')!r} "
            f"(expected {repo_name!r})"
        )
    remote_default_line = remote_repo.get("default_line")
    if remote_default_line and remote_default_line != expected_default_line:
        raise RemoteError(
            f"Remote repository {repo_name} default line mismatch: local={expected_default_line!r} "
            f"remote={remote_default_line!r}"
        )
    return remote_repo


def _sync_remote_repository_defaults(ctx: RepoContext, remote_name: str | None) -> tuple[dict[str, Any], str]:
    remote, repo_name = _remote_tuple(ctx, remote_name)
    cfg = load_config(ctx)
    expected_prefix = repo_id_namespace_prefix(ctx)
    expected_policy = load_policy(ctx)
    ensure_repository = _cli_app_override("ensure_repository", _default_ensure_repository)
    try:
        remote_repo = _verify_remote_repository(remote["url"], repo_name, cfg["default_line"])
    except RemoteError as exc:
        if "failed: 404" not in str(exc):
            raise
        ensure_repository(
            remote["url"],
            repo_name,
            cfg["default_line"],
            policy=expected_policy,
            id_namespace_prefix=expected_prefix,
        )
        _verify_remote_repository(remote["url"], repo_name, cfg["default_line"])
    else:
        if remote_repo.get("id_namespace_prefix") != expected_prefix or remote_repo.get("policy") != expected_policy:
            ensure_repository(
                remote["url"],
                repo_name,
                cfg["default_line"],
                policy=expected_policy,
                id_namespace_prefix=expected_prefix,
            )
            _verify_remote_repository(remote["url"], repo_name, cfg["default_line"])
    return remote, repo_name
