from __future__ import annotations

from ait_protocol.common import generate_namespaced_workflow_id

from . import local_control
from .repo_paths import RepoContext
from .store_local_views import _local_release_view
from .store_repo_config import effective_id_namespace_prefix, load_config

_LOCAL_RELEASE_UNSET = object()

__all__ = [
    "create_local_release",
    "list_local_releases",
    "get_local_release",
    "update_local_release",
]


def create_local_release(
    ctx: RepoContext,
    version: str,
    line_name: str,
    snapshot_id: str,
    manifest_hash: str,
    profile: str,
    *,
    package_name: str | None = None,
    package_version: str | None = None,
    package_requires_python: str | None = None,
    status: str = "candidate",
    checks: list[dict] | None = None,
    artifacts: list[dict] | None = None,
    formula: dict | None = None,
    metadata: dict | None = None,
) -> dict:
    repo_name = load_config(ctx).get("repo_name") or ctx.root.name
    row = local_control.create_workflow_release(
        ctx,
        generate_namespaced_workflow_id("R", effective_id_namespace_prefix(ctx)),
        repo_name,
        version,
        line_name,
        snapshot_id,
        manifest_hash,
        profile,
        package_name=package_name,
        package_version=package_version,
        package_requires_python=package_requires_python,
        status=status,
        checks=checks,
        artifacts=artifacts,
        formula=formula,
        metadata=metadata,
    )
    payload = _local_release_view(row)
    assert payload is not None
    local_control.record_event(
        ctx,
        "release.local_created",
        "release",
        payload["release_id"],
        {
            "release_id": payload["release_id"],
            "repo_name": repo_name,
            "version": version,
            "line": line_name,
            "snapshot_id": snapshot_id,
            "profile": profile,
            "status": payload["status"],
        },
    )
    return payload


def list_local_releases(ctx: RepoContext) -> list[dict]:
    return [_local_release_view(row) or {} for row in local_control.list_workflow_releases(ctx)]


def get_local_release(ctx: RepoContext, release_id: str) -> dict:
    return _local_release_view(local_control.get_workflow_release(ctx, release_id)) or {}


def update_local_release(
    ctx: RepoContext,
    release_id: str,
    *,
    status: str | None = None,
    checks: list[dict] | object = _LOCAL_RELEASE_UNSET,
    artifacts: list[dict] | object = _LOCAL_RELEASE_UNSET,
    formula: dict | object = _LOCAL_RELEASE_UNSET,
    metadata: dict | object = _LOCAL_RELEASE_UNSET,
    event_type: str = "release.updated",
) -> dict:
    row = _local_release_view(
        local_control.update_workflow_release(
            ctx,
            release_id,
            status=status,
            checks=checks if checks is not _LOCAL_RELEASE_UNSET else local_control._UNSET,
            artifacts=artifacts if artifacts is not _LOCAL_RELEASE_UNSET else local_control._UNSET,
            formula=formula if formula is not _LOCAL_RELEASE_UNSET else local_control._UNSET,
            metadata=metadata if metadata is not _LOCAL_RELEASE_UNSET else local_control._UNSET,
        )
    )
    assert row is not None
    local_control.record_event(
        ctx,
        event_type,
        "release",
        release_id,
        {
            "release_id": release_id,
            "status": row["status"],
            "check_count": len(row.get("checks") or []),
            "artifact_count": len(row.get("artifacts") or []),
            "has_formula": bool(row.get("formula")),
        },
    )
    return row
