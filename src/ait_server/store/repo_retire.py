from __future__ import annotations

from decimal import Decimal
import hashlib
import json
import os
import re
import shutil
import uuid
from pathlib import Path
from typing import Any

from ait_protocol.common import encode_ref_name, utc_now

from .. import server_content as server_content_module
from ..server_content import (
    connect as connect_content,
    get_repository as get_content_repository,
    read_ref,
    read_blob_bytes,
    set_repository_lifecycle_state,
)
from ..server_control import _table_columns, connect as connect_control, record_event
from ..server_paths import ServerContext
from .repo_ops import export_snapshot, get_repository_storage, list_lines

RETIRE_EXPORT_ROOT_ENV = "AIT_SERVER_RETIRE_EXPORT_ROOT"
_RETIREMENT_STATE_EXPORTED = "exported"
_RETIREMENT_STATE_PURGED = "purged"
_RETIREMENT_STATE_FAILED = "failed"
_BLOB_REFERENCE_SAMPLE_LIMIT = 25

_REPO_SCOPED_CONTROL_TABLES = (
    "tasks",
    "plans",
    "plan_revision_blobs",
    "plan_revision_artifacts",
    "changes",
    "releases",
    "sessions",
    "planning_sessions",
    "planning_session_events",
    "session_events",
    "session_checkpoints",
    "patchsets",
    "review_requests",
    "reviews",
    "attestations",
    "policy_decisions",
    "waivers",
    "land_requests",
    "stacks",
    "stack_changes",
    "role_bindings",
    "jobs",
    "authority_maps",
)

_PURGE_REPO_SCOPED_CONTROL_TABLES = (
    "review_requests",
    "reviews",
    "attestations",
    "policy_decisions",
    "waivers",
    "land_requests",
    "patchsets",
    "session_checkpoints",
    "session_events",
    "planning_session_events",
    "sessions",
    "planning_sessions",
    "stack_changes",
    "stacks",
    "jobs",
    "releases",
    "changes",
    "tasks",
    "plan_revision_artifacts",
    "plan_revision_blobs",
    "plans",
    "role_bindings",
    "authority_maps",
)


def _retirement_id() -> str:
    return f"RTR-{uuid.uuid4().hex[:20].upper()}"


def _safe_name(value: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]+", "_", value).strip("._") or "repo"


def _sha256_bytes(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def _sha256_path(path: Path) -> str:
    return _sha256_bytes(path.read_bytes())


def _json_default(value: Any) -> Any:
    if isinstance(value, Decimal):
        integral = value.to_integral_value()
        return int(value) if value == integral else float(value)
    if isinstance(value, Path):
        return str(value)
    raise TypeError(f"Object of type {type(value).__name__} is not JSON serializable")


def _write_json(path: Path, payload: Any) -> dict[str, Any]:
    path.parent.mkdir(parents=True, exist_ok=True)
    data = json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=True, default=_json_default).encode("utf-8") + b"\n"
    path.write_bytes(data)
    return {"path": path, "sha256": _sha256_bytes(data), "size_bytes": len(data)}


def _write_bytes(path: Path, payload: bytes) -> dict[str, Any]:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(payload)
    return {"path": path, "sha256": _sha256_bytes(payload), "size_bytes": len(payload)}


def _export_root(ctx: ServerContext) -> Path:
    raw = str(os.environ.get(RETIRE_EXPORT_ROOT_ENV) or "").strip()
    if not raw:
        raise ValueError(f"{RETIRE_EXPORT_ROOT_ENV} is required for repository retirement")
    root = Path(raw).expanduser()
    if not root.is_absolute():
        raise ValueError(f"{RETIRE_EXPORT_ROOT_ENV} must be an absolute path")
    root = root.resolve()
    if not root.exists():
        raise ValueError(f"{RETIRE_EXPORT_ROOT_ENV} path does not exist: {root}")
    if not root.is_dir():
        raise ValueError(f"{RETIRE_EXPORT_ROOT_ENV} must point to a directory: {root}")
    try:
        root.relative_to(ctx.root)
    except ValueError:
        pass
    else:
        raise ValueError(f"{RETIRE_EXPORT_ROOT_ENV} must not be inside the active server runtime root {ctx.root}")
    probe = root / f".ait-retire-write-check-{uuid.uuid4().hex}"
    try:
        probe.write_text("ok\n", encoding="utf-8")
        probe.unlink()
    except Exception as exc:
        raise ValueError(f"{RETIRE_EXPORT_ROOT_ENV} is not writable: {root}") from exc
    return root


def _table_repo_rows(conn, table_name: str, repo_id: str, repo_name: str, *, order_by: str | None = None) -> list[dict[str, Any]]:
    columns = _table_columns(conn, table_name)
    if "repo_id" in columns and "repo_name" in columns:
        where = "(repo_id = ? or (repo_id is null and repo_name = ?))"
        params: tuple[Any, ...] = (repo_id, repo_name)
    elif "repo_id" in columns:
        where = "repo_id = ?"
        params = (repo_id,)
    elif "repo_name" in columns:
        where = "repo_name = ?"
        params = (repo_name,)
    else:
        raise ValueError(f"Table {table_name} is not repository scoped")
    order_sql = f" order by {order_by}" if order_by else ""
    rows = conn.execute(f"select * from {table_name} where {where}{order_sql}", params).fetchall()
    return [dict(row) for row in rows]


def _delete_repo_rows(conn, table_name: str, repo_id: str, repo_name: str) -> int:
    rows = _table_repo_rows(conn, table_name, repo_id, repo_name)
    if not rows:
        return 0
    columns = _table_columns(conn, table_name)
    if "repo_id" in columns and "repo_name" in columns:
        conn.execute(
            f"delete from {table_name} where repo_id = ? or (repo_id is null and repo_name = ?)",
            (repo_id, repo_name),
        )
    elif "repo_id" in columns:
        conn.execute(f"delete from {table_name} where repo_id = ?", (repo_id,))
    else:
        conn.execute(f"delete from {table_name} where repo_name = ?", (repo_name,))
    return len(rows)


def _rows_by_ids(conn, table_name: str, id_column: str, ids: list[str], *, order_by: str | None = None) -> list[dict[str, Any]]:
    if not ids:
        return []
    placeholders = ", ".join("?" for _ in ids)
    order_sql = f" order by {order_by}" if order_by else ""
    rows = conn.execute(
        f"select * from {table_name} where {id_column} in ({placeholders}){order_sql}",
        tuple(ids),
    ).fetchall()
    return [dict(row) for row in rows]


def _delete_rows_by_ids(conn, table_name: str, id_column: str, ids: list[str]) -> int:
    if not ids:
        return 0
    placeholders = ", ".join("?" for _ in ids)
    conn.execute(f"delete from {table_name} where {id_column} in ({placeholders})", tuple(ids))
    return len(ids)


def _repo_plan_ids(conn, repo_id: str, repo_name: str) -> list[str]:
    return [str(row["plan_id"]) for row in _table_repo_rows(conn, "plans", repo_id, repo_name, order_by="plan_id asc")]


def _repo_authority_map_ids(conn, repo_id: str, repo_name: str) -> list[str]:
    return [
        str(row["authority_map_id"])
        for row in _table_repo_rows(conn, "authority_maps", repo_id, repo_name, order_by="authority_map_id asc")
    ]


def _release_artifact_blob_ids(rows: list[dict[str, Any]]) -> set[str]:
    blob_ids: set[str] = set()
    for row in rows:
        try:
            artifacts = json.loads(row.get("artifacts_json") or "[]")
        except Exception:
            artifacts = []
        for artifact in artifacts if isinstance(artifacts, list) else []:
            if not isinstance(artifact, dict):
                continue
            blob_id = str(artifact.get("blob_id") or "").strip()
            if blob_id:
                blob_ids.add(blob_id)
    return blob_ids


def _control_blob_ids(control_exports: dict[str, list[dict[str, Any]]]) -> list[str]:
    blob_ids = {
        str(row.get("blob_id") or "").strip()
        for table_name in ("plan_revision_blobs", "plan_revision_artifacts")
        for row in control_exports.get(table_name, [])
        if str(row.get("blob_id") or "").strip()
    }
    blob_ids |= _release_artifact_blob_ids(control_exports.get("releases", []))
    return sorted(blob_ids)


def _snapshot_reference_owners(content_conn) -> dict[str, set[str]]:
    rows = content_conn.execute(
        """
        select distinct sf.blob_id, s.repo_name
        from snapshot_files sf
        join snapshots s on s.snapshot_id = sf.snapshot_id
        """
    ).fetchall()
    owners: dict[str, set[str]] = {}
    for row in rows:
        blob_id = str(row["blob_id"] or "").strip()
        repo_name = str(row["repo_name"] or "").strip()
        if blob_id and repo_name:
            owners.setdefault(blob_id, set()).add(repo_name)
    return owners


def _control_blob_owners(control_conn) -> dict[str, set[str]]:
    owners: dict[str, set[str]] = {}
    for table_name in ("plan_revision_blobs", "plan_revision_artifacts"):
        for row in control_conn.execute(f"select repo_name, blob_id from {table_name}").fetchall():
            blob_id = str(row["blob_id"] or "").strip()
            repo_name = str(row["repo_name"] or "").strip()
            if blob_id and repo_name:
                owners.setdefault(blob_id, set()).add(repo_name)
    for row in control_conn.execute("select repo_name, artifacts_json from releases").fetchall():
        repo_name = str(row["repo_name"] or "").strip()
        if not repo_name:
            continue
        try:
            artifacts = json.loads(row["artifacts_json"] or "[]")
        except Exception:
            artifacts = []
        for artifact in artifacts if isinstance(artifacts, list) else []:
            if not isinstance(artifact, dict):
                continue
            blob_id = str(artifact.get("blob_id") or "").strip()
            if blob_id:
                owners.setdefault(blob_id, set()).add(repo_name)
    return owners


def _cross_repo_pack_refs(ctx: ServerContext, repo_name: str, repo_id: str) -> list[dict[str, Any]]:
    with connect_content(ctx) as content_conn, connect_control(ctx) as control_conn:
        pack_rows = content_conn.execute(
            "select pack_id from packs where repo_id = ? or (repo_id is null and repo_name = ?) order by pack_id asc",
            (repo_id, repo_name),
        ).fetchall()
        pack_ids = [str(row["pack_id"]) for row in pack_rows if str(row["pack_id"] or "").strip()]
        if not pack_ids:
            return []
        placeholders = ", ".join("?" for _ in pack_ids)
        blob_rows = content_conn.execute(
            f"select blob_id, pack_id from blobs where pack_id in ({placeholders}) order by blob_id asc",
            tuple(pack_ids),
        ).fetchall()
        if not blob_rows:
            return []
        snapshot_owners = _snapshot_reference_owners(content_conn)
        control_owners = _control_blob_owners(control_conn)
    collisions: list[dict[str, Any]] = []
    for row in blob_rows:
        blob_id = str(row["blob_id"] or "").strip()
        pack_id = str(row["pack_id"] or "").strip()
        owners = set(snapshot_owners.get(blob_id, set()))
        owners |= set(control_owners.get(blob_id, set()))
        other_owners = sorted(owner for owner in owners if owner and owner != repo_name)
        if other_owners:
            collisions.append(
                {
                    "blob_id": blob_id,
                    "pack_id": pack_id,
                    "other_repo_names": other_owners,
                }
            )
        if len(collisions) >= _BLOB_REFERENCE_SAMPLE_LIMIT:
            break
    return collisions


def _preflight_blockers(ctx: ServerContext, repo_name: str, repo_id: str) -> dict[str, Any]:
    blockers: dict[str, Any] = {}
    with connect_control(ctx) as conn:
        active_jobs = [
            dict(row)
            for row in conn.execute(
                """
                select job_id, job_type, state
                from jobs
                where (repo_id = ? or (repo_id is null and repo_name = ?))
                  and state in ('queued', 'running')
                order by job_id asc
                limit 25
                """,
                (repo_id, repo_name),
            ).fetchall()
        ]
        if active_jobs:
            blockers["active_jobs"] = active_jobs
        active_tasks = [
            dict(row)
            for row in conn.execute(
                """
                select task_id, status
                from tasks
                where (repo_id = ? or (repo_id is null and repo_name = ?))
                  and status = 'active'
                order by created_at asc, task_id asc
                limit 25
                """,
                (repo_id, repo_name),
            ).fetchall()
        ]
        if active_tasks:
            blockers["active_tasks"] = active_tasks
        open_changes = [
            dict(row)
            for row in conn.execute(
                """
                select change_id, status
                from changes
                where (repo_id = ? or (repo_id is null and repo_name = ?))
                  and status not in ('archived', 'landed', 'superseded')
                order by updated_at asc, change_id asc
                limit 25
                """,
                (repo_id, repo_name),
            ).fetchall()
        ]
        if open_changes:
            blockers["open_changes"] = open_changes
        active_sessions = [
            dict(row)
            for row in conn.execute(
                """
                select session_id, session_kind, status
                from sessions
                where (repo_id = ? or (repo_id is null and repo_name = ?))
                  and status = 'active'
                order by updated_at asc, session_id asc
                limit 25
                """,
                (repo_id, repo_name),
            ).fetchall()
        ]
        if active_sessions:
            blockers["active_sessions"] = active_sessions
        active_planning_sessions = [
            dict(row)
            for row in conn.execute(
                """
                select planning_session_id, status
                from planning_sessions
                where (repo_id = ? or (repo_id is null and repo_name = ?))
                  and status = 'active'
                order by updated_at asc, planning_session_id asc
                limit 25
                """,
                (repo_id, repo_name),
            ).fetchall()
        ]
        if active_planning_sessions:
            blockers["active_planning_sessions"] = active_planning_sessions
        pending_lands = [
            dict(row)
            for row in conn.execute(
                """
                select submission_id, status, target_line
                from land_requests
                where repo_id = ?
                  and status in ('queued', 'running')
                order by created_at asc, submission_id asc
                limit 25
                """,
                (repo_id,),
            ).fetchall()
        ]
        if pending_lands:
            blockers["pending_lands"] = pending_lands
    shared_pack_refs = _cross_repo_pack_refs(ctx, repo_name, repo_id)
    if shared_pack_refs:
        blockers["shared_pack_refs"] = shared_pack_refs
    return blockers


def _raise_blockers(repo_name: str, blockers: dict[str, Any]) -> None:
    details = ", ".join(f"{key}={len(value)}" for key, value in blockers.items())
    raise ValueError(f"Repository {repo_name} cannot be retired while blockers remain: {details}")


def _repo_content_pack_rows(content_conn, repo_id: str, repo_name: str) -> list[dict[str, Any]]:
    rows = content_conn.execute(
        """
        select pack_id, pack_path
        from packs
        where repo_id = ? or (repo_id is null and repo_name = ?)
        order by created_at asc, pack_id asc
        """,
        (repo_id, repo_name),
    ).fetchall()
    return [dict(row) for row in rows]


def _repo_snapshot_ids(content_conn, repo_id: str, repo_name: str) -> list[str]:
    rows = content_conn.execute(
        """
        select snapshot_id
        from snapshots
        where repo_id = ? or (repo_id is null and repo_name = ?)
        order by created_at asc, snapshot_id asc
        """,
        (repo_id, repo_name),
    ).fetchall()
    return [str(row["snapshot_id"]) for row in rows]


def _repo_content_rows(content_conn, repo_id: str, repo_name: str) -> dict[str, list[dict[str, Any]]]:
    tables = {
        "repositories": [dict(content_conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone())],
        "lines": [
            dict(row)
            for row in content_conn.execute(
                "select * from lines where repo_id = ? or (repo_id is null and repo_name = ?) order by line_name asc",
                (repo_id, repo_name),
            ).fetchall()
        ],
        "snapshots": [
            dict(row)
            for row in content_conn.execute(
                "select * from snapshots where repo_id = ? or (repo_id is null and repo_name = ?) order by created_at asc, snapshot_id asc",
                (repo_id, repo_name),
            ).fetchall()
        ],
        "packs": _repo_content_pack_rows(content_conn, repo_id, repo_name),
        "repository_group_memberships": [
            dict(row)
            for row in content_conn.execute(
                "select * from repository_group_memberships where repo_id = ? or (repo_id is null and repo_name = ?) order by sort_index asc, repo_name asc",
                (repo_id, repo_name),
            ).fetchall()
        ],
    }
    return tables


def _export_bundle(ctx: ServerContext, repo_name: str, repo_id: str) -> dict[str, Any]:
    export_root = _export_root(ctx)
    stamp = utc_now().replace(":", "").replace("+00:00", "Z").replace("+08:00", "+0800")
    export_dir = export_root / f"{stamp}__{_safe_name(repo_name)}__{repo_id}"
    export_dir.mkdir(parents=True, exist_ok=False)
    files: list[dict[str, Any]] = []

    repository = get_content_repository(ctx, repo_name)
    lines = list_lines(ctx, repo_name)
    refs = {str(line["line_name"]): read_ref(ctx, repo_name, str(line["line_name"])) for line in lines}
    storage = get_repository_storage(ctx, repo_name)
    files.append(_write_json(export_dir / "repository.json", repository))
    files.append(_write_json(export_dir / "content" / "lines.json", lines))
    files.append(_write_json(export_dir / "content" / "refs.json", refs))
    files.append(_write_json(export_dir / "content" / "storage.json", storage))

    with connect_content(ctx) as content_conn:
        content_tables = _repo_content_rows(content_conn, repo_id, repo_name)
        snapshot_ids = _repo_snapshot_ids(content_conn, repo_id, repo_name)
    for table_name, rows in content_tables.items():
        files.append(_write_json(export_dir / "content" / "tables" / f"{table_name}.json", rows))

    control_exports: dict[str, list[dict[str, Any]]] = {}
    with connect_control(ctx) as control_conn:
        plan_ids = _repo_plan_ids(control_conn, repo_id, repo_name)
        authority_map_ids = _repo_authority_map_ids(control_conn, repo_id, repo_name)
        for table_name in _REPO_SCOPED_CONTROL_TABLES:
            control_exports[table_name] = _table_repo_rows(control_conn, table_name, repo_id, repo_name, order_by="1 asc")
        control_exports["plan_revisions"] = _rows_by_ids(
            control_conn,
            "plan_revisions",
            "plan_id",
            plan_ids,
            order_by="plan_id asc, revision_number asc",
        )
        control_exports["authority_nodes"] = _rows_by_ids(
            control_conn,
            "authority_nodes",
            "authority_map_id",
            authority_map_ids,
            order_by="authority_map_id asc, sort_index asc, authority_node_id asc",
        )
        control_exports["authority_mutations"] = _rows_by_ids(
            control_conn,
            "authority_mutations",
            "authority_map_id",
            authority_map_ids,
            order_by="authority_map_id asc, created_at asc, mutation_id asc",
        )
    for table_name, rows in control_exports.items():
        files.append(_write_json(export_dir / "control" / "tables" / f"{table_name}.json", rows))

    control_blob_ids = _control_blob_ids(control_exports)
    control_blob_manifest: list[dict[str, Any]] = []
    for blob_id in control_blob_ids:
        payload = read_blob_bytes(ctx, blob_id)
        blob_path = export_dir / "control" / "blobs" / f"{blob_id}.bin"
        written = _write_bytes(blob_path, payload)
        files.append(written)
        control_blob_manifest.append(
            {
                "blob_id": blob_id,
                "path": str(blob_path.relative_to(export_dir)),
                "sha256": written["sha256"],
                "size_bytes": written["size_bytes"],
            }
        )
    files.append(_write_json(export_dir / "control" / "blob_manifest.json", control_blob_manifest))

    snapshot_manifest: list[dict[str, Any]] = []
    for snapshot_id in snapshot_ids:
        bundle = export_snapshot(ctx, repo_name, snapshot_id, include_content=True)
        path = export_dir / "content" / "snapshots" / f"{snapshot_id}.json"
        written = _write_json(path, bundle)
        files.append(written)
        snapshot_manifest.append(
            {
                "snapshot_id": snapshot_id,
                "path": str(path.relative_to(export_dir)),
                "sha256": written["sha256"],
                "size_bytes": written["size_bytes"],
            }
        )
    files.append(_write_json(export_dir / "content" / "snapshot_manifest.json", snapshot_manifest))

    manifest_entries = [
        {
            "path": str(Path(item["path"]).relative_to(export_dir)),
            "sha256": item["sha256"],
            "size_bytes": item["size_bytes"],
        }
        for item in files
    ]
    manifest_entries.sort(key=lambda item: item["path"])
    manifest_payload = {
        "repo_name": repo_name,
        "repo_id": repo_id,
        "generated_at": utc_now(),
        "export_root": str(export_root),
        "export_path": str(export_dir),
        "snapshot_count": len(snapshot_ids),
        "control_blob_count": len(control_blob_ids),
        "content_table_counts": {table_name: len(rows) for table_name, rows in content_tables.items()},
        "control_table_counts": {table_name: len(rows) for table_name, rows in control_exports.items()},
        "files": manifest_entries,
    }
    manifest_written = _write_json(export_dir / "manifest.json", manifest_payload)
    manifest_sha = manifest_written["sha256"]
    checksum_path = export_dir / "manifest.sha256"
    checksum_path.write_text(f"{manifest_sha}  manifest.json\n", encoding="utf-8")
    return {
        "export_root": export_root,
        "export_path": export_dir,
        "manifest_path": export_dir / "manifest.json",
        "manifest_sha256": manifest_sha,
        "manifest": manifest_payload,
    }


def _verify_export(export_dir: Path, manifest_path: Path, manifest_sha256: str) -> dict[str, Any]:
    actual_manifest_sha = _sha256_path(manifest_path)
    if actual_manifest_sha != manifest_sha256:
        raise ValueError(f"Manifest checksum mismatch for {manifest_path}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    verified_files = 0
    total_bytes = 0
    for entry in manifest.get("files", []):
        path = export_dir / str(entry["path"])
        if not path.exists():
            raise ValueError(f"Export verification failed; missing file {path}")
        actual_sha = _sha256_path(path)
        if actual_sha != str(entry["sha256"]):
            raise ValueError(f"Export verification failed; checksum mismatch for {path}")
        size_bytes = path.stat().st_size
        if size_bytes != int(entry["size_bytes"]):
            raise ValueError(f"Export verification failed; size mismatch for {path}")
        verified_files += 1
        total_bytes += size_bytes
    return {
        "verified": True,
        "verified_file_count": verified_files,
        "verified_total_bytes": total_bytes,
    }


def _insert_retirement_record(
    ctx: ServerContext,
    retirement_id: str,
    *,
    repo_name: str,
    repo_id: str,
    actor_identity: str,
    actor_type: str,
    export_path: Path,
    manifest_path: Path,
    manifest_sha256: str,
    summary: dict[str, Any],
) -> None:
    now = utc_now()
    with connect_control(ctx) as conn:
        conn.execute(
            """
            insert into repository_retirements(
                retirement_id, repo_name, repo_id, state, actor_identity, actor_type, export_path,
                manifest_path, manifest_sha256, summary_json, created_at, exported_at, verified_at,
                purged_at, updated_at, last_error
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, null, ?, null)
            """,
            (
                retirement_id,
                repo_name,
                repo_id,
                _RETIREMENT_STATE_EXPORTED,
                actor_identity,
                actor_type,
                str(export_path),
                str(manifest_path),
                manifest_sha256,
                json.dumps(summary, sort_keys=True),
                now,
                now,
                now,
                now,
            ),
        )
        record_event(
            conn,
            "repository.retire_exported",
            "repository",
            repo_name,
            {
                "repo_id": repo_id,
                "retirement_id": retirement_id,
                "export_path": str(export_path),
                "manifest_path": str(manifest_path),
                "manifest_sha256": manifest_sha256,
            },
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        conn.commit()


def _update_retirement_record(
    ctx: ServerContext,
    retirement_id: str,
    *,
    state: str,
    last_error: str | None = None,
    summary_patch: dict[str, Any] | None = None,
) -> None:
    now = utc_now()
    with connect_control(ctx) as conn:
        row = conn.execute(
            "select repo_name, repo_id, summary_json from repository_retirements where retirement_id = ?",
            (retirement_id,),
        ).fetchone()
        if row is None:
            return
        try:
            summary = json.loads(row["summary_json"] or "{}")
        except Exception:
            summary = {}
        if summary_patch:
            summary.update(summary_patch)
        purged_at = now if state == _RETIREMENT_STATE_PURGED else None
        conn.execute(
            """
            update repository_retirements
            set state = ?, summary_json = ?, purged_at = coalesce(?, purged_at), updated_at = ?, last_error = ?
            where retirement_id = ?
            """,
            (
                state,
                json.dumps(summary, sort_keys=True),
                purged_at,
                now,
                last_error,
                retirement_id,
            ),
        )
        event_type = "repository.retired" if state == _RETIREMENT_STATE_PURGED else "repository.retire_failed"
        record_event(
            conn,
            event_type,
            "repository",
            str(row["repo_name"]),
            {
                "repo_id": str(row["repo_id"]),
                "retirement_id": retirement_id,
                "state": state,
                "last_error": last_error,
            },
        )
        conn.commit()


def _remaining_control_blob_ids(control_conn) -> set[str]:
    remaining = {
        str(row["blob_id"])
        for table_name in ("plan_revision_blobs", "plan_revision_artifacts")
        for row in control_conn.execute(f"select blob_id from {table_name}").fetchall()
        if str(row["blob_id"] or "").strip()
    }
    remaining |= _release_artifact_blob_ids(
        [dict(row) for row in control_conn.execute("select artifacts_json from releases").fetchall()]
    )
    return remaining


def _cleanup_content_after_repo_delete(
    ctx: ServerContext,
    content_conn,
    control_conn,
    *,
    repo_name: str,
    repo_id: str,
    repo_pack_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    snapshot_blob_ids = {
        str(row["blob_id"])
        for row in content_conn.execute("select distinct blob_id from snapshot_files").fetchall()
        if str(row["blob_id"] or "").strip()
    }
    remaining_control_blob_ids = _remaining_control_blob_ids(control_conn)
    retained_blob_ids = snapshot_blob_ids | remaining_control_blob_ids
    blob_rows = [dict(row) for row in content_conn.execute("select blob_id, pack_id from blobs").fetchall()]
    removed_blob_count = 0
    for row in blob_rows:
        blob_id = str(row["blob_id"] or "").strip()
        if not blob_id or blob_id in retained_blob_ids:
            continue
        content_conn.execute("delete from blobs where blob_id = ?", (blob_id,))
        removed_blob_count += 1

    unreachable_tree_ids = server_content_module._unreachable_tree_ids(content_conn)
    removed_tree_entry_count = 0
    if unreachable_tree_ids:
        placeholders = ", ".join("?" for _ in unreachable_tree_ids)
        removed_tree_entry_count = int(
            content_conn.execute(
                f"select count(*) as c from tree_entries where tree_id in ({placeholders})",
                tuple(unreachable_tree_ids),
            ).fetchone()["c"]
            or 0
        )
        content_conn.executemany("delete from trees where tree_id = ?", [(tree_id,) for tree_id in unreachable_tree_ids])

    removed_pack_count = 0
    for row in repo_pack_rows:
        pack_id = str(row.get("pack_id") or "").strip()
        pack_path = str(row.get("pack_path") or "").strip()
        if not pack_id or not pack_path:
            continue
        remaining = int(
            content_conn.execute("select count(*) as c from blobs where pack_id = ?", (pack_id,)).fetchone()["c"] or 0
        )
        if remaining != 0:
            raise ValueError(
                f"Repository {repo_name} cannot remove pack {pack_id}; {remaining} blob(s) remain referenced outside {repo_name}"
            )
        pack_abs = ctx.root / pack_path
        if pack_abs.exists():
            pack_abs.unlink()
        removed_pack_count += 1

    removed_tree_pack_count = 0
    tree_pack_rows = [dict(row) for row in content_conn.execute("select pack_id, pack_path from tree_packs").fetchall()]
    for row in tree_pack_rows:
        pack_id = str(row.get("pack_id") or "").strip()
        pack_path = str(row.get("pack_path") or "").strip()
        if not pack_id or not pack_path:
            continue
        remaining = int(
            content_conn.execute("select count(*) as c from trees where tree_pack_id = ?", (pack_id,)).fetchone()["c"] or 0
        )
        if remaining != 0:
            continue
        pack_abs = ctx.root / pack_path
        if pack_abs.exists():
            pack_abs.unlink()
        content_conn.execute("delete from tree_packs where pack_id = ?", (pack_id,))
        removed_tree_pack_count += 1

    for repo_token in {repo_id, repo_name}:
        ref_dir = ctx.ref_root / encode_ref_name(repo_token)
        if ref_dir.exists():
            shutil.rmtree(ref_dir, ignore_errors=True)

    return {
        "removed_unreferenced_blob_count": removed_blob_count,
        "removed_unreachable_tree_count": len(unreachable_tree_ids),
        "removed_unreachable_tree_entry_count": removed_tree_entry_count,
        "removed_pack_count": removed_pack_count,
        "removed_tree_pack_count": removed_tree_pack_count,
    }


def _purge_repository(ctx: ServerContext, repo_name: str, repo_id: str) -> dict[str, Any]:
    control_counts: dict[str, int] = {}
    with connect_control(ctx) as control_conn:
        plan_ids = _repo_plan_ids(control_conn, repo_id, repo_name)
        authority_map_ids = _repo_authority_map_ids(control_conn, repo_id, repo_name)
        control_counts["plan_revisions"] = _delete_rows_by_ids(control_conn, "plan_revisions", "plan_id", plan_ids)
        control_counts["authority_nodes"] = _delete_rows_by_ids(
            control_conn,
            "authority_nodes",
            "authority_map_id",
            authority_map_ids,
        )
        control_counts["authority_mutations"] = _delete_rows_by_ids(
            control_conn,
            "authority_mutations",
            "authority_map_id",
            authority_map_ids,
        )
        for table_name in _PURGE_REPO_SCOPED_CONTROL_TABLES:
            control_counts[table_name] = _delete_repo_rows(control_conn, table_name, repo_id, repo_name)
        control_conn.commit()

        with connect_content(ctx) as content_conn:
            repo_pack_rows = _repo_content_pack_rows(content_conn, repo_id, repo_name)
            repo_row = content_conn.execute(
                "select repo_name from repositories where repo_name = ? and repo_id = ?",
                (repo_name, repo_id),
            ).fetchone()
            if repo_row is None:
                raise KeyError(f"Unknown repository: {repo_name}")
            content_conn.execute("delete from repositories where repo_name = ? and repo_id = ?", (repo_name, repo_id))
            content_cleanup = _cleanup_content_after_repo_delete(
                ctx,
                content_conn,
                control_conn,
                repo_name=repo_name,
                repo_id=repo_id,
                repo_pack_rows=repo_pack_rows,
            )
            content_conn.commit()

    return {
        "control_deleted": control_counts,
        "content_cleanup": content_cleanup,
    }


def retire_repository(
    ctx: ServerContext,
    repo_name: str,
    *,
    expected_repo_id: str,
    actor_identity: str,
    actor_type: str,
    require_verified_export: bool = True,
) -> dict[str, Any]:
    repository = get_content_repository(ctx, repo_name)
    repo_id = str(repository.get("repo_id") or "").strip()
    normalized_expected_repo_id = str(expected_repo_id or "").strip()
    if not normalized_expected_repo_id:
        raise ValueError("expected_repo_id is required")
    if normalized_expected_repo_id != repo_id:
        raise ValueError(
            f"Repository scope mismatch for {repo_name}: repo_id {normalized_expected_repo_id} does not match {repo_id}"
        )
    blockers = _preflight_blockers(ctx, repo_name, repo_id)
    if blockers:
        _raise_blockers(repo_name, blockers)

    set_repository_lifecycle_state(ctx, repo_name, "retiring", expected_repo_id=repo_id)
    retirement_id: str | None = None
    try:
        export_result = _export_bundle(ctx, repo_name, repo_id)
        verification = (
            _verify_export(
                export_result["export_path"],
                export_result["manifest_path"],
                export_result["manifest_sha256"],
            )
            if require_verified_export
            else {"verified": False, "verified_file_count": 0, "verified_total_bytes": 0}
        )
        summary = {
            "repo_name": repo_name,
            "repo_id": repo_id,
            "require_verified_export": bool(require_verified_export),
            "verification": verification,
            "manifest": {
                "snapshot_count": int(export_result["manifest"]["snapshot_count"]),
                "control_blob_count": int(export_result["manifest"]["control_blob_count"]),
                "file_count": len(export_result["manifest"]["files"]),
            },
        }
        retirement_id = _retirement_id()
        _insert_retirement_record(
            ctx,
            retirement_id,
            repo_name=repo_name,
            repo_id=repo_id,
            actor_identity=actor_identity,
            actor_type=actor_type,
            export_path=export_result["export_path"],
            manifest_path=export_result["manifest_path"],
            manifest_sha256=export_result["manifest_sha256"],
            summary=summary,
        )
        purge_result = _purge_repository(ctx, repo_name, repo_id)
        _update_retirement_record(
            ctx,
            retirement_id,
            state=_RETIREMENT_STATE_PURGED,
            summary_patch={"purge": purge_result},
        )
        return {
            "retirement_id": retirement_id,
            "repo_name": repo_name,
            "repo_id": repo_id,
            "export_path": str(export_result["export_path"]),
            "manifest_path": str(export_result["manifest_path"]),
            "manifest_sha256": str(export_result["manifest_sha256"]),
            "verification": verification,
            "purge": purge_result,
        }
    except Exception as exc:
        if retirement_id is None:
            try:
                set_repository_lifecycle_state(ctx, repo_name, "active", expected_repo_id=repo_id)
            except Exception:
                pass
        else:
            _update_retirement_record(ctx, retirement_id, state=_RETIREMENT_STATE_FAILED, last_error=str(exc))
        raise
