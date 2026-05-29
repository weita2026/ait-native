from __future__ import annotations

import json

from ait_protocol.common import connect_sqlite, utc_now

from .repo_paths import RepoContext

WORKFLOW_RELEASE_UNSET = object()


def _connect_control(ctx: RepoContext):
    return connect_sqlite(ctx.control_db_path)


def create_workflow_release(
    ctx: RepoContext,
    release_id: str,
    repo_name: str,
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
    conn = _connect_control(ctx)
    now = utc_now()
    conn.execute(
        """
        insert into workflow_releases(
            release_id, repo_name, version, line_name, snapshot_id, manifest_hash, profile,
            package_name, package_version, package_requires_python, status,
            checks_json, artifacts_json, formula_json, metadata_json, created_at, updated_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            release_id,
            repo_name,
            version,
            line_name,
            snapshot_id,
            manifest_hash,
            profile,
            package_name,
            package_version,
            package_requires_python,
            status,
            json.dumps(checks or [], sort_keys=True),
            json.dumps(artifacts or [], sort_keys=True),
            json.dumps(formula or {}, sort_keys=True),
            json.dumps(metadata or {}, sort_keys=True),
            now,
            now,
        ),
    )
    conn.commit()
    row = conn.execute("select * from workflow_releases where release_id = ?", (release_id,)).fetchone()
    conn.close()
    assert row is not None
    return dict(row)


def list_workflow_releases(ctx: RepoContext) -> list[dict]:
    conn = _connect_control(ctx)
    rows = [dict(r) for r in conn.execute("select * from workflow_releases order by created_at desc, release_id desc")]
    conn.close()
    return rows


def get_workflow_release(ctx: RepoContext, release_id: str) -> dict:
    conn = _connect_control(ctx)
    row = conn.execute("select * from workflow_releases where release_id = ?", (release_id,)).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local release: {release_id}")
    return dict(row)


def update_workflow_release(
    ctx: RepoContext,
    release_id: str,
    *,
    status: str | None = None,
    checks: list[dict] | object = WORKFLOW_RELEASE_UNSET,
    artifacts: list[dict] | object = WORKFLOW_RELEASE_UNSET,
    formula: dict | object = WORKFLOW_RELEASE_UNSET,
    metadata: dict | object = WORKFLOW_RELEASE_UNSET,
) -> dict:
    conn = _connect_control(ctx)
    existing = conn.execute("select * from workflow_releases where release_id = ?", (release_id,)).fetchone()
    if existing is None:
        conn.close()
        raise KeyError(f"Unknown local release: {release_id}")
    assignments: list[str] = []
    params: list[object] = []
    if status is not None:
        assignments.append("status = ?")
        params.append(status)
    if checks is not WORKFLOW_RELEASE_UNSET:
        assignments.append("checks_json = ?")
        params.append(json.dumps(checks, sort_keys=True))
    if artifacts is not WORKFLOW_RELEASE_UNSET:
        assignments.append("artifacts_json = ?")
        params.append(json.dumps(artifacts, sort_keys=True))
    if formula is not WORKFLOW_RELEASE_UNSET:
        assignments.append("formula_json = ?")
        params.append(json.dumps(formula, sort_keys=True))
    if metadata is not WORKFLOW_RELEASE_UNSET:
        assignments.append("metadata_json = ?")
        params.append(json.dumps(metadata, sort_keys=True))
    if not assignments:
        conn.close()
        return dict(existing)
    now = utc_now()
    assignments.append("updated_at = ?")
    params.append(now)
    params.append(release_id)
    conn.execute(f"update workflow_releases set {', '.join(assignments)} where release_id = ?", tuple(params))
    conn.commit()
    row = conn.execute("select * from workflow_releases where release_id = ?", (release_id,)).fetchone()
    conn.close()
    assert row is not None
    return dict(row)
