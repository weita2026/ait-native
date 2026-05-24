from __future__ import annotations

import inspect
from typing import Any, Optional

from ait_protocol.common import utc_now

from ..server_content import (
    archive_line as archive_content_line,
    ensure_repository as ensure_content_repository,
    export_snapshot as export_content_snapshot,
    gc_repository_content as gc_content_repository,
    get_line as get_content_line,
    get_repository as get_content_repository,
    get_snapshot_repo,
    import_snapshot as import_content_snapshot,
    list_lines as list_content_lines,
    pack_repository as pack_content_repository,
    repository_exists,
    repository_storage_stats as get_content_repository_storage,
    snapshot_existence as content_snapshot_existence,
    update_line as update_content_line,
)
from ..server_control import connect, record_event
from ..server_paths import ServerContext

def ensure_repository(
    ctx: ServerContext,
    repo_name: str,
    default_line: str,
    policy: dict[str, Any] | None = None,
    *,
    id_namespace_prefix: str | None = None,
) -> dict:
    row = ensure_content_repository(
        ctx,
        repo_name,
        default_line,
        policy=policy,
        id_namespace_prefix=id_namespace_prefix,
    )
    with connect(ctx) as conn:
        record_event(
            conn,
            "repository.created",
            "repository",
            repo_name,
            {"repo_name": repo_name, "default_line": default_line, "policy_id": row.get("policy", {}).get("policy_id")},
        )
        conn.commit()
    return row

def get_repository(ctx: ServerContext, repo_name: str) -> dict:
    return get_content_repository(ctx, repo_name)

def _repo_id_namespace_prefix(ctx: ServerContext, repo_name: str) -> str:
    repository = get_content_repository(ctx, repo_name)
    value = repository.get("id_namespace_prefix")
    if value is None:
        return "AIT"
    return str(value)


def _repo_id(ctx: ServerContext, repo_name: str) -> str:
    repository = get_content_repository(ctx, repo_name)
    value = str(repository.get("repo_id") or "").strip()
    if not value:
        raise KeyError(f"Repository {repo_name} is missing repo_id")
    return value

def list_lines(ctx: ServerContext, repo_name: str) -> list[dict]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    return list_content_lines(ctx, repo_name)

def get_line(ctx: ServerContext, repo_name: str, line_name: str) -> dict:
    return get_content_line(ctx, repo_name, line_name)

def update_line(
    ctx: ServerContext,
    repo_name: str,
    line_name: str,
    head_snapshot_id: Optional[str],
    *,
    expected_head_snapshot_id: Optional[str] = None,
) -> dict:
    if head_snapshot_id is not None:
        snapshot_repo = get_snapshot_repo(ctx, head_snapshot_id)
        if snapshot_repo is None:
            raise KeyError(f"Unknown snapshot: {head_snapshot_id}")
        if snapshot_repo != repo_name:
            raise KeyError(f"Snapshot {head_snapshot_id} belongs to repository {snapshot_repo}, not {repo_name}")
    row = update_content_line(
        ctx,
        repo_name,
        line_name,
        head_snapshot_id,
        expected_head_snapshot_id=expected_head_snapshot_id,
    )
    with connect(ctx) as conn:
        record_event(
            conn,
            "line.updated",
            "line",
            f"{repo_name}:{line_name}",
            {
                "repo_name": repo_name,
                "line_name": line_name,
                "head_snapshot_id": head_snapshot_id,
                "expected_head_snapshot_id": expected_head_snapshot_id,
            },
        )
        conn.commit()
    return row

def close_line(ctx: ServerContext, repo_name: str, line_name: str, status: str = "archived") -> dict:
    if status != "archived":
        raise ValueError(f"Unsupported line status: {status}")
    repo = get_repository(ctx, repo_name)
    if line_name == repo["default_line"]:
        raise ValueError(f"Default line {line_name} cannot be archived")
    row = archive_content_line(ctx, repo_name, line_name)
    with connect(ctx) as conn:
        record_event(
            conn,
            "line.archived",
            "line",
            f"{repo_name}:{line_name}",
            {"repo_name": repo_name, "line_name": line_name, "status": row["status"], "archived_at": row.get("archived_at")},
        )
        conn.commit()
    return row

def import_snapshot(ctx: ServerContext, repo_name: str, bundle: dict) -> dict:
    row = import_content_snapshot(ctx, repo_name, bundle)
    with connect(ctx) as conn:
        record_event(conn, "snapshot.imported", "snapshot", row["snapshot_id"], {"repo_name": repo_name, "snapshot_id": row["snapshot_id"]})
        conn.commit()
    return row

def export_snapshot(
    ctx: ServerContext,
    repo_name: str,
    snapshot_id: str,
    *,
    include_content: bool = True,
    path: str | None = None,
) -> dict:
    parameters = inspect.signature(export_content_snapshot).parameters
    if "include_content" in parameters or "path" in parameters:
        kwargs: dict[str, Any] = {}
        if "include_content" in parameters:
            kwargs["include_content"] = include_content
        if "path" in parameters:
            kwargs["path"] = path
        return export_content_snapshot(ctx, repo_name, snapshot_id, **kwargs)

    bundle = export_content_snapshot(ctx, repo_name, snapshot_id)
    files = []
    for row in bundle.get("files", []):
        if path is not None and row.get("path") != path:
            continue
        file_row = dict(row)
        if not include_content:
            file_row.pop("content_b64", None)
        files.append(file_row)

    if include_content:
        bundle.setdefault("content_included", True)
    else:
        bundle["content_included"] = False
    return {**bundle, "files": files}

def snapshot_existence(ctx: ServerContext, repo_name: str, snapshot_ids: list[str]) -> dict[str, Any]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    return content_snapshot_existence(ctx, repo_name, snapshot_ids)

def get_repository_storage(ctx: ServerContext, repo_name: str) -> dict[str, Any]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    return get_content_repository_storage(ctx, repo_name)

def pack_repository_storage(ctx: ServerContext, repo_name: str, *, repack: bool = False, max_members: int | None = None) -> dict[str, Any]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    row = pack_content_repository(ctx, repo_name, repack=repack, max_members=max_members)
    with connect(ctx) as conn:
        record_event(conn, "content.pack.created" if row.get("created") else "content.pack.skipped", "repository", repo_name, {
            "repo_name": repo_name,
            "created": bool(row.get("created")),
            "pack_id": row.get("pack_id"),
            "member_count": row.get("member_count", 0),
            "repack": repack,
        })
        conn.commit()
    return row

def optimize_repository_storage(ctx: ServerContext, repo_name: str, *, repair: bool = True) -> dict[str, Any]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    initial_storage = get_content_repository_storage(ctx, repo_name)
    current_storage = initial_storage
    steps: list[dict[str, Any]] = []

    if repair and current_storage["signals_summary"]["repairable_drift_count"] > 0:
        from .. import server_store as server_store_module

        reconcile_result = server_store_module.reconcile_repository(ctx, repo_name, repair=True)
        current_storage = get_content_repository_storage(ctx, repo_name)
        steps.append(
            {
                "action": "reconcile",
                "reason": "repairable_storage_drifts",
                "result": {
                    "drift_count": reconcile_result["drift_count"],
                    "repaired_count": reconcile_result["repaired_count"],
                    "storage_signals_summary": reconcile_result["storage_signals_summary"],
                },
            }
        )

    if current_storage["pack_count"] > 1:
        repack_result = pack_repository_storage(ctx, repo_name, repack=True)
        current_storage = repack_result["stats"]
        steps.append(
            {
                "action": "repack",
                "reason": "multi_pack_layout",
                "result": {
                    "created": repack_result.get("created", False),
                    "pack_id": repack_result.get("pack_id"),
                    "member_count": repack_result.get("member_count", 0),
                },
            }
        )

    if (
        current_storage["pack_count"] > 1
        or current_storage["global_unreferenced_blob_count"] > 0
        or current_storage.get("global_unreachable_tree_count", 0) > 0
    ):
        gc_result = gc_repository_storage(ctx, repo_name)
        current_storage = gc_result["stats"]
        steps.append(
            {
                "action": "gc",
                "reason": "pack_cleanup_or_unreferenced_storage_metadata",
                "result": {
                    "removed_unreferenced_blob_count": gc_result.get("removed_unreferenced_blob_count", 0),
                    "removed_unreachable_tree_count": gc_result.get("removed_unreachable_tree_count", 0),
                    "removed_unreachable_tree_entry_count": gc_result.get("removed_unreachable_tree_entry_count", 0),
                    "removed_orphan_pack_count": gc_result.get("removed_orphan_pack_count", 0),
                },
            }
        )

    with connect(ctx) as conn:
        record_event(
            conn,
            "content.optimize.completed",
            "repository",
            repo_name,
            {
                "repo_name": repo_name,
                "repair": repair,
                "step_count": len(steps),
                "initial_pack_count": initial_storage["pack_count"],
                "final_pack_count": current_storage["pack_count"],
                "initial_global_unreachable_tree_count": initial_storage.get("global_unreachable_tree_count", 0),
                "final_global_unreachable_tree_count": current_storage.get("global_unreachable_tree_count", 0),
                "final_packed_delta_blob_count": current_storage["packed_delta_blob_count"],
            },
        )
        conn.commit()
    return {
        "repo_name": repo_name,
        "repair": repair,
        "initial_storage": initial_storage,
        "final_storage": current_storage,
        "steps": steps,
        "executed_step_count": len(steps),
    }

def gc_repository_storage(
    ctx: ServerContext,
    repo_name: str,
    *,
    prune_unreferenced: bool = True,
    prune_orphan_packs: bool = True,
) -> dict[str, Any]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    row = gc_content_repository(
        ctx,
        repo_name,
        prune_unreferenced=prune_unreferenced,
        prune_orphan_packs=prune_orphan_packs,
    )
    with connect(ctx) as conn:
        record_event(conn, "content.gc.completed", "repository", repo_name, {
            "repo_name": repo_name,
            "removed_unreferenced_blob_count": row.get("removed_unreferenced_blob_count", 0),
            "removed_unreachable_tree_count": row.get("removed_unreachable_tree_count", 0),
            "removed_unreachable_tree_entry_count": row.get("removed_unreachable_tree_entry_count", 0),
            "removed_orphan_pack_count": row.get("removed_orphan_pack_count", 0),
        })
        conn.commit()
    return row
