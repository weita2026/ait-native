from __future__ import annotations

from typing import Any

from ait_protocol.common import AuthorMode
from ait_storage.revision_trees import build_snapshot_id, build_tree_records

from ..remote_client import (
    RemoteError,
    get_patchset as remote_get_patchset,
    get_remote_line,
    get_remote_snapshot,
    publish_patchset as remote_publish_patchset,
    put_remote_snapshot,
    update_remote_line,
)
from ..snapshot_diff import diff_snapshot_file_maps
from ..store import RepoContext, export_snapshot_bundle, get_snapshot, import_snapshot_bundle
from .line_transport_helpers import (
    _remote_snapshot_exists,
    _upload_snapshot_chain,
    _verify_remote_pushed_snapshot,
)
from .runtime_defaults import _effective_author_mode, _normalize_text_value


def _patchset_publish_context(
    *,
    revision_sync: dict[str, Any],
    revision_snapshot_id: str,
    base_snapshot_id: str,
) -> dict[str, Any]:
    line_updated = bool(revision_sync.get("line_updated", True))
    return {
        "pushed_line": revision_sync.get("line") if line_updated else None,
        "revision_line": revision_sync.get("line"),
        "line_updated": line_updated,
        "line_update_skipped_reason": revision_sync.get("line_update_skipped_reason"),
        "revision_snapshot_id": revision_snapshot_id,
        "base_snapshot_id": base_snapshot_id,
        "revision_sync": {
            "checked_snapshots": revision_sync.get("checked_snapshots"),
            "uploaded_snapshots": revision_sync.get("uploaded_snapshots"),
            "skipped_snapshots": revision_sync.get("skipped_snapshots"),
            "head_snapshot_id": revision_sync.get("head_snapshot_id"),
        },
    }


def _workflow_land_batch_ensure_remote_patchset_for_landed_change(
    ctx: RepoContext,
    *,
    remote_name: str,
    remote_row: dict[str, Any],
    repo_name: str,
    remote_change: dict[str, Any],
    local_change: dict[str, Any],
    target_line: str,
    remote_base_snapshot_id: str,
    summary: str | None,
    author_mode: AuthorMode | None,
) -> dict[str, Any]:
    current_patchset_id = _normalize_text_value(remote_change.get("current_patchset_id"))
    if current_patchset_id is not None:
        current_patchset = remote_get_patchset(remote_row["url"], current_patchset_id, repo_name=repo_name)
        current_patchset_base_snapshot_id = _normalize_text_value(current_patchset.get("base_snapshot_id"))
        if current_patchset_base_snapshot_id == remote_base_snapshot_id:
            return current_patchset

    landed_snapshot_id = _normalize_text_value(local_change.get("landed_snapshot_id"))
    if landed_snapshot_id is None:
        raise ValueError(f"Completed local change {local_change['change_id']} is missing `landed_snapshot_id`.")
    landed_snapshot = get_snapshot(ctx, landed_snapshot_id)
    actual_parent_snapshot_id = _normalize_text_value(landed_snapshot.get("parent_snapshot_id"))
    parent_snapshot_id, parent_resolution = _resolve_completed_local_promotion_parent_snapshot_id(
        ctx,
        local_change=local_change,
        landed_snapshot_id=landed_snapshot_id,
    )
    if remote_base_snapshot_id == parent_snapshot_id:
        revision_snapshot_id = landed_snapshot_id
        upload = _upload_snapshot_chain(
            ctx,
            remote_name,
            landed_snapshot_id,
            line_name=target_line,
            reason="completed-local batch promotion snapshot chain",
        )
        promotion_case = "direct_local_landed_snapshot"
    else:
        rebased_bundle = _replay_snapshot_delta_onto_parent_bundle(
            ctx,
            remote_row=remote_row,
            repo_name=repo_name,
            snapshot_id=landed_snapshot_id,
            source_parent_snapshot_id=parent_snapshot_id,
            target_parent_snapshot_id=remote_base_snapshot_id,
            line_name=_normalize_text_value(landed_snapshot.get("line_name")) or target_line,
        )
        revision_snapshot_id = str(rebased_bundle["snapshot_id"])
        already_present = _remote_snapshot_exists(remote_row["url"], repo_name, revision_snapshot_id)
        if not already_present:
            remote_snapshot = put_remote_snapshot(
                remote_row["url"],
                repo_name,
                revision_snapshot_id,
                rebased_bundle,
            )
            _verify_remote_pushed_snapshot(remote_snapshot, repo_name, revision_snapshot_id, rebased_bundle)
        upload = {
            "remote": remote_row.get("name"),
            "repo_name": repo_name,
            "line": target_line,
            "line_updated": False,
            "line_update_skipped_reason": "completed-local batch promotion reparented snapshot",
            "pushed_snapshots": 0 if already_present else 1,
            "checked_snapshots": 1,
            "uploaded_snapshots": 0 if already_present else 1,
            "skipped_snapshots": 1 if already_present else 0,
            "head_snapshot_id": revision_snapshot_id,
            "remote_repository": None,
            "remote_line": None,
        }
        promotion_case = "delta_replayed_to_remote_head"
    patchset_summary = summary or f"Promote completed local change {local_change['change_id']}"
    resolved_author_mode = _effective_author_mode(ctx, author_mode)
    patchset = remote_publish_patchset(
        remote_row["url"],
        str(remote_change["change_id"]),
        remote_base_snapshot_id,
        revision_snapshot_id,
        patchset_summary,
        resolved_author_mode,
        repo_name=repo_name,
        exact_id=True,
    )
    patchset["publish_context"] = _patchset_publish_context(
        revision_sync=upload,
        revision_snapshot_id=revision_snapshot_id,
        base_snapshot_id=remote_base_snapshot_id,
    )
    if current_patchset_id is not None:
        patchset["publish_context"]["refreshed_from_patchset_id"] = current_patchset_id
    patchset["publish_context"]["promotion_case"] = promotion_case
    patchset["publish_context"]["source_landed_snapshot_id"] = landed_snapshot_id
    patchset["publish_context"]["source_parent_snapshot_id"] = parent_snapshot_id
    patchset["publish_context"]["source_parent_resolution"] = parent_resolution
    patchset["publish_context"]["source_landed_parent_snapshot_id"] = actual_parent_snapshot_id
    return patchset


def _resolve_completed_local_promotion_parent_snapshot_id(
    ctx: RepoContext,
    *,
    local_change: dict[str, Any],
    landed_snapshot_id: str,
) -> tuple[str, str]:
    pre_land_target_snapshot_id = _normalize_text_value(local_change.get("pre_land_target_snapshot_id"))
    if pre_land_target_snapshot_id:
        return pre_land_target_snapshot_id, "pre_land_target_snapshot_id"

    landed_snapshot = get_snapshot(ctx, landed_snapshot_id)
    landed_parent_snapshot_id = _normalize_text_value(landed_snapshot.get("parent_snapshot_id"))
    if landed_parent_snapshot_id is None:
        raise ValueError(
            f"Completed local change {local_change['change_id']} landed at `{landed_snapshot_id}` without a parent snapshot."
        )
    landed_bundle = export_snapshot_bundle(ctx, landed_snapshot_id)
    landed_parent_bundle = export_snapshot_bundle(ctx, landed_parent_snapshot_id)
    landed_delta = diff_snapshot_file_maps(
        landed_parent_bundle.get("files") or [],
        landed_bundle.get("files") or [],
        old_snapshot_id=landed_parent_snapshot_id,
        new_snapshot_id=landed_snapshot_id,
    )
    if any(landed_delta.get(key) for key in ("added", "deleted", "modified", "mode_changed")):
        return landed_parent_snapshot_id, "landed_snapshot_parent"

    fork_snapshot_id = _normalize_text_value(local_change.get("fork_snapshot_id"))
    if fork_snapshot_id:
        return fork_snapshot_id, "fork_snapshot_id_fallback_after_empty_landed_delta"
    return landed_parent_snapshot_id, "landed_snapshot_parent_empty_delta"


def _ensure_snapshot_bundle_available(
    ctx: RepoContext,
    *,
    remote_row: dict[str, Any],
    repo_name: str,
    snapshot_id: str,
) -> dict[str, Any]:
    try:
        return export_snapshot_bundle(ctx, snapshot_id)
    except KeyError:
        bundle = get_remote_snapshot(remote_row["url"], repo_name, snapshot_id)
        import_snapshot_bundle(ctx, bundle)
        return export_snapshot_bundle(ctx, snapshot_id)


def _replay_snapshot_delta_onto_parent_bundle(
    ctx: RepoContext,
    *,
    remote_row: dict[str, Any],
    repo_name: str,
    snapshot_id: str,
    source_parent_snapshot_id: str,
    target_parent_snapshot_id: str | None,
    line_name: str,
) -> dict[str, Any]:
    source_bundle = export_snapshot_bundle(ctx, snapshot_id)
    source_parent_bundle = export_snapshot_bundle(ctx, source_parent_snapshot_id)
    target_parent_bundle = (
        _ensure_snapshot_bundle_available(
            ctx,
            remote_row=remote_row,
            repo_name=repo_name,
            snapshot_id=target_parent_snapshot_id,
        )
        if target_parent_snapshot_id
        else {"files": []}
    )

    source_files = {
        str(file_row["path"]): dict(file_row)
        for file_row in source_bundle.get("files") or []
        if isinstance(file_row, dict) and file_row.get("path")
    }
    source_parent_files = {
        str(file_row["path"]): dict(file_row)
        for file_row in source_parent_bundle.get("files") or []
        if isinstance(file_row, dict) and file_row.get("path")
    }
    replay_base_files = {
        str(file_row["path"]): dict(file_row)
        for file_row in target_parent_bundle.get("files") or []
        if isinstance(file_row, dict) and file_row.get("path")
    }
    delta = diff_snapshot_file_maps(
        source_parent_files,
        source_files,
        old_snapshot_id=source_parent_snapshot_id,
        new_snapshot_id=snapshot_id,
    )
    for path in delta.get("deleted") or []:
        replay_base_files.pop(str(path), None)
    for path in [*(delta.get("added") or []), *(delta.get("modified") or []), *(delta.get("mode_changed") or [])]:
        row = source_files.get(str(path))
        if row is None:
            raise ValueError(
                f"Completed local change snapshot `{snapshot_id}` is missing replay row for `{path}`."
            )
        replay_base_files[str(path)] = row

    merged_files = [replay_base_files[path] for path in sorted(replay_base_files)]
    file_rows = [
        {
            "path": file_row["path"],
            "blob_id": file_row["blob_id"],
            "size_bytes": file_row["size_bytes"],
            "mode": file_row["mode"],
            "sha256": file_row["sha256"],
        }
        for file_row in merged_files
    ]
    root_tree_id, _, _ = build_tree_records(file_rows)
    rebased_snapshot_id, revision_hash = build_snapshot_id(
        repo_name=repo_name,
        line_name=line_name,
        parent_snapshot_id=target_parent_snapshot_id,
        message=source_bundle.get("message"),
        root_tree_id=root_tree_id,
    )
    rebased_bundle = dict(source_bundle)
    rebased_bundle["snapshot_id"] = rebased_snapshot_id
    rebased_bundle["parent_snapshot_id"] = target_parent_snapshot_id
    rebased_bundle["root_tree_id"] = root_tree_id
    rebased_bundle["manifest_hash"] = revision_hash
    rebased_bundle["manifest_path"] = None
    rebased_bundle["line_name"] = line_name
    rebased_bundle["file_count"] = len(merged_files)
    rebased_bundle["total_bytes"] = sum(int(file_row.get("size_bytes") or 0) for file_row in merged_files)
    rebased_bundle["files"] = merged_files
    return rebased_bundle


def _workflow_land_batch_ensure_remote_target_line_base(
    ctx: RepoContext,
    *,
    remote_name: str,
    remote_row: dict[str, Any],
    repo_name: str,
    target_line: str,
    initial_parent_snapshot_id: str,
) -> str:
    remote_line = _ensure_remote_line_initialized(
        ctx,
        remote_name,
        remote_row=remote_row,
        repo_name=repo_name,
        line_name=target_line,
        head_snapshot_id=initial_parent_snapshot_id,
    )
    remote_head_snapshot_id = _normalize_text_value(remote_line.get("head_snapshot_id"))
    if remote_head_snapshot_id is None:
        raise ValueError(f"Remote line `{target_line}` has no head snapshot after initialization.")
    return remote_head_snapshot_id


def _ensure_remote_line_initialized(
    ctx: RepoContext,
    remote_name: str,
    *,
    remote_row: dict[str, Any],
    repo_name: str,
    line_name: str,
    head_snapshot_id: str | None,
) -> dict[str, Any]:
    try:
        remote_line = get_remote_line(remote_row["url"], repo_name, line_name)
    except (KeyError, RemoteError, ValueError):
        remote_line = None

    remote_head_snapshot_id = (
        _normalize_text_value(remote_line.get("head_snapshot_id"))
        if isinstance(remote_line, dict)
        else None
    )
    if remote_line is not None and (remote_head_snapshot_id is not None or head_snapshot_id is None):
        return remote_line

    if head_snapshot_id is not None:
        _upload_snapshot_chain(
            ctx,
            remote_name,
            head_snapshot_id,
            line_name=line_name,
            reason="batch workflow land remote line initialization",
        )
    remote_line = update_remote_line(
        remote_row["url"],
        repo_name,
        line_name,
        head_snapshot_id,
        expected_head_snapshot_id=remote_head_snapshot_id,
    )
    return remote_line
