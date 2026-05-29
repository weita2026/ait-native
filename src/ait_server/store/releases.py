from __future__ import annotations

import base64
import hashlib
import json
from pathlib import Path
from typing import Any

from ait_protocol.common import utc_now

from ..server_content import get_snapshot_repo
from ..server_content_repo_lines import get_line as get_content_line
from ..server_content_repo_lines import repository_exists
from ..server_content_storage import read_blob_bytes, write_blob_bytes
from ..server_control import connect, record_event
from ..server_paths import ServerContext
from .plans import _normalize_nonempty_text, _normalize_optional_text
from .repo_ops import _repo_id
from .workflow_artifacts import _release_artifact_media_type, _release_row, _sanitize_release_artifact_path


def _assert_repo_scope(ctx: ServerContext, repo_name: str) -> str:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    return _repo_id(ctx, repo_name)


def _store_release_artifact(
    ctx: ServerContext,
    repo_name: str,
    release_id: str,
    artifact: dict[str, Any],
    *,
    created_at: str,
) -> dict[str, Any]:
    kind = _normalize_nonempty_text(artifact.get("kind"), field="Release artifact kind").lower()
    artifact_path = _sanitize_release_artifact_path(_normalize_nonempty_text(artifact.get("path"), field="Release artifact path"))
    content_b64 = _normalize_nonempty_text(artifact.get("content_b64"), field=f"Release artifact {kind} content_b64")
    try:
        data = base64.b64decode(content_b64, validate=True)
    except Exception as exc:
        raise ValueError(f"Release artifact {kind} content_b64 is not valid base64") from exc
    sha256 = hashlib.sha256(data).hexdigest()
    expected_sha = _normalize_optional_text(artifact.get("sha256"))
    if expected_sha and expected_sha != sha256:
        raise ValueError(f"Release artifact {kind} sha256 mismatch: expected {expected_sha}, got {sha256}")
    expected_size = artifact.get("size_bytes")
    if expected_size is not None and int(expected_size) != len(data):
        raise ValueError(f"Release artifact {kind} size mismatch: expected {expected_size}, got {len(data)}")
    blob = write_blob_bytes(
        ctx,
        repo_name,
        data,
        path_hint=f"releases/{release_id}/{artifact_path}",
        created_at=created_at,
    )
    return {
        "kind": kind,
        "path": artifact_path,
        "size_bytes": len(data),
        "sha256": sha256,
        "blob_id": str(blob["blob_id"]),
        "download_name": Path(artifact_path).name or f"{release_id}-{kind}",
        "media_type": _release_artifact_media_type(kind, artifact_path),
    }


def _release_formula_payload(
    formula: dict[str, Any] | None,
    artifacts: list[dict[str, Any]],
) -> dict[str, Any]:
    payload = dict(formula or {})
    if not payload:
        return {}
    artifact_kind = str(payload.get("artifact_kind") or "sdist")
    source_artifact = next((artifact for artifact in artifacts if str(artifact.get("kind") or "") == artifact_kind), None)
    formula_artifact = next((artifact for artifact in artifacts if str(artifact.get("kind") or "") == "formula"), None)
    return {
        "name": _normalize_optional_text(payload.get("name")),
        "class_name": _normalize_optional_text(payload.get("class_name")),
        "artifact_kind": artifact_kind,
        "path": formula_artifact.get("path") if formula_artifact is not None else None,
        "sha256": source_artifact.get("sha256") if source_artifact is not None else _normalize_optional_text(payload.get("sha256")),
    }


def publish_release(
    ctx: ServerContext,
    repo_name: str,
    release_id: str,
    version: str,
    line_name: str,
    snapshot_id: str,
    manifest_hash: str,
    profile: str,
    *,
    package: dict[str, Any] | None = None,
    checks: list[dict[str, Any]] | None = None,
    artifacts: list[dict[str, Any]] | None = None,
    formula: dict[str, Any] | None = None,
    metadata: dict[str, Any] | None = None,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
) -> dict:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    get_content_line(ctx, repo_name, line_name)
    snapshot_repo = get_snapshot_repo(ctx, snapshot_id)
    if snapshot_repo != repo_name:
        raise KeyError(f"Unknown snapshot {snapshot_id} for repository {repo_name}")
    if not artifacts:
        raise ValueError("Release publish requires built artifacts.")
    normalized_release_id = _normalize_nonempty_text(release_id, field="Release release_id")
    normalized_version = _normalize_nonempty_text(version, field="Release version")
    normalized_line_name = _normalize_nonempty_text(line_name, field="Release line")
    normalized_snapshot_id = _normalize_nonempty_text(snapshot_id, field="Release snapshot_id")
    normalized_manifest_hash = _normalize_nonempty_text(manifest_hash, field="Release manifest_hash")
    normalized_profile = _normalize_nonempty_text(profile, field="Release profile")
    normalized_package = package if isinstance(package, dict) else {}
    normalized_checks = [dict(item) for item in (checks or []) if isinstance(item, dict)]
    normalized_metadata = dict(metadata or {}) if isinstance(metadata or {}, dict) else {}
    now = utc_now()
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        existing = conn.execute("select * from releases where release_id = ?", (normalized_release_id,)).fetchone()
        version_owner = conn.execute(
            "select release_id from releases where repo_id = ? and version = ?",
            (repo_id, normalized_version),
        ).fetchone()
    if version_owner is not None and str(version_owner["release_id"]) != normalized_release_id:
        raise ValueError(
            f"Release version {normalized_version} is already published in {repo_name} as {version_owner['release_id']}."
        )
    if existing is not None:
        existing_row = dict(existing)
        immutable_fields = {
            "repo_name": repo_name,
            "version": normalized_version,
            "line_name": normalized_line_name,
            "snapshot_id": normalized_snapshot_id,
            "manifest_hash": normalized_manifest_hash,
            "profile": normalized_profile,
        }
        for field_name, expected in immutable_fields.items():
            actual = str(existing_row.get(field_name) or "")
            if actual != expected:
                raise ValueError(
                    f"Release {normalized_release_id} already exists with {field_name}={actual!r}, not {expected!r}."
                )
    stored_artifacts: list[dict[str, Any]] = []
    seen_kinds: set[str] = set()
    for artifact in artifacts:
        if not isinstance(artifact, dict):
            continue
        stored = _store_release_artifact(
            ctx,
            repo_name,
            normalized_release_id,
            artifact,
            created_at=now,
        )
        kind = str(stored.get("kind") or "")
        if kind in seen_kinds:
            raise ValueError(f"Release publish received duplicate artifact kind {kind!r}.")
        seen_kinds.add(kind)
        stored_artifacts.append(stored)
    stored_artifacts.sort(key=lambda item: str(item.get("kind") or ""))
    stored_formula = _release_formula_payload(formula, stored_artifacts)
    normalized_metadata["publication"] = {
        "surface": "ait_server",
        "repo_name": repo_name,
        "published_at": now,
    }
    package_name = _normalize_optional_text(normalized_package.get("name"))
    package_version = _normalize_optional_text(normalized_package.get("version"))
    package_requires_python = _normalize_optional_text(normalized_package.get("requires_python"))
    with connect(ctx) as conn:
        try:
            existing = conn.execute("select created_by, actor_type, created_at from releases where release_id = ?", (normalized_release_id,)).fetchone()
            if existing is None:
                conn.execute(
                    """
                    insert into releases(
                        release_id, repo_name, repo_id, version, line_name, snapshot_id, manifest_hash, profile,
                        package_name, package_version, package_requires_python, status,
                        checks_json, artifacts_json, formula_json, metadata_json,
                        created_by, actor_type, created_at, updated_at
                    ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        normalized_release_id,
                        repo_name,
                        repo_id,
                        normalized_version,
                        normalized_line_name,
                        normalized_snapshot_id,
                        normalized_manifest_hash,
                        normalized_profile,
                        package_name,
                        package_version,
                        package_requires_python,
                        "published",
                        json.dumps(normalized_checks, sort_keys=True),
                        json.dumps(stored_artifacts, sort_keys=True),
                        json.dumps(stored_formula, sort_keys=True),
                        json.dumps(normalized_metadata, sort_keys=True),
                        actor_identity,
                        actor_type,
                        now,
                        now,
                    ),
                )
                event_type = "release.published"
            else:
                conn.execute(
                    """
                    update releases
                    set package_name = ?, package_version = ?, package_requires_python = ?, status = ?,
                        checks_json = ?, artifacts_json = ?, formula_json = ?, metadata_json = ?, updated_at = ?
                    where release_id = ?
                    """,
                    (
                        package_name,
                        package_version,
                        package_requires_python,
                        "published",
                        json.dumps(normalized_checks, sort_keys=True),
                        json.dumps(stored_artifacts, sort_keys=True),
                        json.dumps(stored_formula, sort_keys=True),
                        json.dumps(normalized_metadata, sort_keys=True),
                        now,
                        normalized_release_id,
                    ),
                )
                event_type = "release.updated"
            record_event(
                conn,
                event_type,
                "release",
                normalized_release_id,
                {
                    "repo_name": repo_name,
                    "version": normalized_version,
                    "line_name": normalized_line_name,
                    "snapshot_id": normalized_snapshot_id,
                    "status": "published",
                    "artifact_count": len(stored_artifacts),
                },
                actor_identity=actor_identity,
                actor_type=actor_type,
            )
            conn.commit()
        except Exception:
            conn.rollback()
            raise
    return get_release(ctx, normalized_release_id)


def get_release(ctx: ServerContext, release_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select * from releases where release_id = ?", (release_id,)).fetchone()
    if row is None:
        raise KeyError(f"Unknown release: {release_id}")
    return _release_row(row)


def get_release_for_repo(ctx: ServerContext, repo_name: str, release_ref: str) -> dict:
    repo_id = _assert_repo_scope(ctx, repo_name)
    with connect(ctx) as conn:
        row = conn.execute(
            "select * from releases where repo_id = ? and release_id = ?",
            (repo_id, release_ref),
        ).fetchone()
        if row is None:
            row = conn.execute(
                "select * from releases where repo_id = ? and version = ?",
                (repo_id, str(release_ref or "").strip()),
            ).fetchone()
    if row is None:
        raise KeyError(f"Unknown release {release_ref} for repository {repo_name}")
    return _release_row(row)


def read_release_artifact(ctx: ServerContext, release_id: str, artifact_kind: str) -> dict[str, Any]:
    release = get_release(ctx, release_id)
    normalized_kind = str(artifact_kind or "").strip().lower()
    artifact = next(
        (
            item
            for item in release.get("artifacts", [])
            if isinstance(item, dict) and str(item.get("kind") or "").strip().lower() == normalized_kind
        ),
        None,
    )
    if artifact is None:
        raise KeyError(f"Release {release_id} does not include artifact kind {artifact_kind!r}")
    blob_id = _normalize_optional_text(artifact.get("blob_id"))
    if blob_id is None:
        raise KeyError(f"Release {release_id} artifact {artifact_kind!r} is missing blob storage.")
    return {
        "release": release,
        "artifact": artifact,
        "data": read_blob_bytes(ctx, blob_id),
    }
