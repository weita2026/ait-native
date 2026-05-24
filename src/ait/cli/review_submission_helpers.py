from __future__ import annotations

from typing import Any, Optional

import typer

from ..remote_client import (
    RemoteError,
    list_patchsets as remote_list_patchsets,
    record_review as remote_record_review,
    request_review as remote_request_review,
)
from ..store import RepoContext
from .remote_repository_defaults import _remote_tuple
from .runtime_defaults import _effective_reviewer_identity


def _latest_patchset_id(base_url: str, change_id: str, repo_name: str | None) -> str:
    rows = remote_list_patchsets(base_url, change_id, repo_name=repo_name)
    if not rows:
        raise KeyError(f"Change {change_id} has no patchsets")
    return rows[0]["patchset_id"]


def _request_team_review_result(
    ctx: RepoContext,
    *,
    change_id: str,
    group: list[str],
    patchset: Optional[str],
    note: Optional[str],
    remote: Optional[str],
) -> dict[str, Any]:
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        patchset_id = patchset or _latest_patchset_id(remote_row["url"], change_id, repo_name=repo_name)
        return remote_request_review(remote_row["url"], change_id, patchset_id, group, note, repo_name=repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc


def _review_action_result(
    ctx: RepoContext,
    *,
    change_id: str,
    reviewer: Optional[str],
    action: str,
    blocking: bool,
    patchset: Optional[str],
    message: Optional[str],
    remote: Optional[str],
) -> dict[str, Any]:
    resolved_reviewer = _effective_reviewer_identity(ctx, reviewer)
    if not resolved_reviewer:
        raise typer.BadParameter(
            "No reviewer identity configured. Pass --reviewer or run `ait config set --user-name ... --user-email ...`."
        )
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        patchset_id = patchset or _latest_patchset_id(remote_row["url"], change_id, repo_name=repo_name)
        return remote_record_review(
            remote_row["url"],
            change_id,
            patchset_id,
            resolved_reviewer,
            action,
            message,
            blocking,
            repo_name=repo_name,
        )
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
