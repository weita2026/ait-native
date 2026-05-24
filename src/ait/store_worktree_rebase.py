from __future__ import annotations

from pathlib import Path
from typing import Any

from ait_protocol.common import normalize_optional_text, utc_now

from . import local_content, local_control
from .repo_paths import RepoContext
from .store_worktree_runtime import (
    create_snapshot,
    current_line,
    set_line_head,
    workspace_status,
)
from .store_repo_config import (
    _set_worktree_materialized_snapshot,
    _worktree_materialized_snapshot_id,
)
from .store_worktree_layout import _prune_empty_worktree_dirs
from .store_worktree_metadata import _load_worktree_metadata
from .store_worktree_state import (
    _effective_worktree_target_base_line,
    _latest_common_snapshot,
    _repo_worktree_ctx,
    _snapshot_distance_if_ancestor,
    _worktree_metadata_with_defaults,
)
from .store_worktree_views import (
    _maybe_discover_worktree,
    _resolve_worktree_name,
    _write_worktree_metadata_fields,
    get_worktree,
)


def _snapshot_file_map_for_id(ctx: RepoContext, snapshot_id: str | None) -> dict[str, dict[str, Any]]:
    if snapshot_id is None:
        return {}
    snapshot = local_content.get_snapshot(ctx, snapshot_id)
    files = snapshot.get("files") if isinstance(snapshot.get("files"), list) else []
    return {
        str(row["path"]): dict(row)
        for row in files
        if isinstance(row, dict) and isinstance(row.get("path"), str)
    }


def _snapshot_rows_equal(left: dict[str, Any] | None, right: dict[str, Any] | None) -> bool:
    if left is None or right is None:
        return left is None and right is None
    return (
        str(left.get("blob_id") or "") == str(right.get("blob_id") or "")
        and str(left.get("mode") or "") == str(right.get("mode") or "")
        and int(left.get("size_bytes") or 0) == int(right.get("size_bytes") or 0)
    )


def _resolve_rebase_worktree(
    ctx: RepoContext,
    name: str | None = None,
) -> tuple[RepoContext, str, dict[str, Any], Path, RepoContext]:
    repo_ctx = _repo_worktree_ctx(ctx)
    worktree_name = _resolve_worktree_name(ctx, name)
    metadata = _worktree_metadata_with_defaults(_load_worktree_metadata(repo_ctx, worktree_name))
    worktree_path = Path(str(metadata.get("path") or "")).expanduser().resolve()
    worktree_ctx = _maybe_discover_worktree(worktree_path)
    if worktree_ctx is None:
        raise ValueError(f"Worktree is missing or detached: {worktree_name}")
    return repo_ctx, worktree_name, metadata, worktree_path, worktree_ctx


def _read_snapshot_blob_bytes(conn, worktree_ctx: RepoContext, row: dict[str, Any] | None) -> bytes:
    if row is None:
        return b""
    blob_id = normalize_optional_text(row.get("blob_id"))
    if blob_id is None:
        return b""
    return local_content._blob_bytes_by_id(worktree_ctx, conn, blob_id)


def _decode_merge_text(data: bytes) -> str | None:
    if b"\x00" in data:
        return None
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return None


def _render_rebase_conflict_text(
    *,
    base_text: str,
    feature_text: str,
    target_text: str,
    feature_label: str,
    target_label: str,
) -> str:
    return (
        f"<<<<<<< {feature_label}\n"
        f"{feature_text}"
        f"||||||| base\n"
        f"{base_text}"
        f"=======\n"
        f"{target_text}"
        f">>>>>>> {target_label}\n"
    )


def _write_workspace_snapshot_row(conn, worktree_ctx: RepoContext, path: str, row: dict[str, Any] | None) -> None:
    abs_path = worktree_ctx.root / path
    if row is None:
        if abs_path.exists():
            abs_path.unlink()
            _prune_empty_worktree_dirs(worktree_ctx.root, abs_path)
        return
    abs_path.parent.mkdir(parents=True, exist_ok=True)
    if abs_path.exists() and abs_path.is_dir():
        raise IsADirectoryError(f"Cannot restore file over directory: {path}")
    abs_path.write_bytes(_read_snapshot_blob_bytes(conn, worktree_ctx, row))
    abs_path.chmod(local_content._parse_mode_bits(str(row.get("mode") or "0o644")))


def _write_workspace_text_file(worktree_ctx: RepoContext, path: str, text: str) -> None:
    abs_path = worktree_ctx.root / path
    abs_path.parent.mkdir(parents=True, exist_ok=True)
    if abs_path.exists() and abs_path.is_dir():
        raise IsADirectoryError(f"Cannot restore file over directory: {path}")
    abs_path.write_text(text, encoding="utf-8")


def _compute_worktree_rebase_plan(
    repo_ctx: RepoContext,
    *,
    line_name: str,
    old_base_snapshot_id: str,
    old_head_snapshot_id: str,
    new_base_snapshot_id: str,
    onto_line_name: str,
) -> dict[str, Any]:
    base_files = _snapshot_file_map_for_id(repo_ctx, old_base_snapshot_id)
    head_files = _snapshot_file_map_for_id(repo_ctx, old_head_snapshot_id)
    target_files = _snapshot_file_map_for_id(repo_ctx, new_base_snapshot_id)
    all_paths = sorted(set(base_files) | set(head_files) | set(target_files))
    files: list[dict[str, Any]] = []
    apply_write_paths: list[str] = []
    apply_remove_paths: list[str] = []
    conflict_paths: list[str] = []
    feature_delta_count = 0

    for path in all_paths:
        base_row = base_files.get(path)
        head_row = head_files.get(path)
        target_row = target_files.get(path)
        feature_changed = not _snapshot_rows_equal(head_row, base_row)
        target_changed = not _snapshot_rows_equal(target_row, base_row)
        if feature_changed:
            feature_delta_count += 1

        if not feature_changed:
            resolution = "target"
            apply_status = "unchanged"
        elif not target_changed:
            resolution = "feature"
            apply_status = "remove" if head_row is None else "write"
        elif _snapshot_rows_equal(head_row, target_row):
            resolution = "same_result"
            apply_status = "unchanged"
        else:
            resolution = "conflict"
            apply_status = "conflict"
            conflict_paths.append(path)

        if apply_status == "write":
            apply_write_paths.append(path)
        elif apply_status == "remove":
            apply_remove_paths.append(path)

        files.append(
            {
                "path": path,
                "feature_changed": feature_changed,
                "target_changed": target_changed,
                "resolution": resolution,
                "apply_status": apply_status,
                "old": base_row,
                "feature": head_row,
                "target": target_row,
            }
        )

    return {
        "line_name": line_name,
        "onto_line_name": onto_line_name,
        "old_base_snapshot_id": old_base_snapshot_id,
        "old_head_snapshot_id": old_head_snapshot_id,
        "new_base_snapshot_id": new_base_snapshot_id,
        "feature_delta_count": feature_delta_count,
        "conflict_count": len(conflict_paths),
        "conflict_paths": conflict_paths,
        "apply_write_paths": apply_write_paths,
        "apply_remove_paths": apply_remove_paths,
        "files": files,
        "would_fast_forward": feature_delta_count == 0,
    }


def _prepare_worktree_rebase(
    ctx: RepoContext,
    name: str | None = None,
    *,
    onto_line_name: str | None = None,
    allow_conflicted_state: bool = False,
) -> dict[str, Any]:
    repo_ctx, worktree_name, metadata, worktree_path, worktree_ctx = _resolve_rebase_worktree(ctx, name)
    if str(metadata.get("rebase_state") or "idle") == "conflicted" and not allow_conflicted_state:
        raise ValueError(f"Worktree {worktree_name} is already in a conflicted rebase. Use --continue or --abort.")
    line_name = current_line(worktree_ctx)
    line_row = local_content.get_line(worktree_ctx, line_name)
    old_head_snapshot_id = normalize_optional_text(line_row.get("head_snapshot_id"))
    if old_head_snapshot_id is None:
        raise ValueError(f"Current line {line_name} has no head snapshot to rebase.")
    resolved_onto_line = normalize_optional_text(onto_line_name) or _effective_worktree_target_base_line(repo_ctx, metadata)
    if resolved_onto_line is None:
        raise ValueError(f"Worktree {worktree_name} has no target base line. Pass --onto <line>.")
    onto_line_row = local_content.get_line(repo_ctx, resolved_onto_line)
    new_base_snapshot_id = normalize_optional_text(onto_line_row.get("head_snapshot_id"))
    if new_base_snapshot_id is None:
        raise ValueError(f"Base line {resolved_onto_line} has no head snapshot.")
    old_base_snapshot_id = normalize_optional_text(metadata.get("fork_snapshot_id")) or _latest_common_snapshot(
        repo_ctx,
        old_head_snapshot_id,
        new_base_snapshot_id,
    )
    if old_base_snapshot_id is None:
        raise ValueError(f"Could not infer a fork snapshot for worktree {worktree_name}.")
    if _snapshot_distance_if_ancestor(repo_ctx, old_base_snapshot_id, old_head_snapshot_id) is None:
        raise ValueError(
            f"Fork snapshot {old_base_snapshot_id} is not an ancestor of line head {old_head_snapshot_id}; manual recovery is required."
        )
    if (
        old_base_snapshot_id != new_base_snapshot_id
        and _snapshot_distance_if_ancestor(repo_ctx, old_base_snapshot_id, new_base_snapshot_id) is None
    ):
        raise ValueError(
            f"Base line {resolved_onto_line} no longer descends from fork snapshot {old_base_snapshot_id}; automatic retarget is not safe."
        )
    plan = _compute_worktree_rebase_plan(
        repo_ctx,
        line_name=line_name,
        old_base_snapshot_id=old_base_snapshot_id,
        old_head_snapshot_id=old_head_snapshot_id,
        new_base_snapshot_id=new_base_snapshot_id,
        onto_line_name=resolved_onto_line,
    )
    return {
        "repo_ctx": repo_ctx,
        "worktree_name": worktree_name,
        "metadata": metadata,
        "worktree_path": worktree_path,
        "worktree_ctx": worktree_ctx,
        "line_name": line_name,
        "old_base_snapshot_id": old_base_snapshot_id,
        "old_head_snapshot_id": old_head_snapshot_id,
        "new_base_snapshot_id": new_base_snapshot_id,
        "onto_line_name": resolved_onto_line,
        "plan": plan,
    }


def preview_worktree_rebase(
    ctx: RepoContext,
    name: str | None = None,
    *,
    onto_line_name: str | None = None,
) -> dict[str, Any]:
    prepared = _prepare_worktree_rebase(ctx, name, onto_line_name=onto_line_name)
    summary = get_worktree(prepared["repo_ctx"], prepared["worktree_name"])
    summary["rebase"] = {
        **prepared["plan"],
        "worktree_name": prepared["worktree_name"],
        "path": str(prepared["worktree_path"]),
        "dry_run": True,
    }
    return summary


def _materialize_worktree_rebase_conflicts(prepared: dict[str, Any]) -> list[dict[str, Any]]:
    worktree_ctx: RepoContext = prepared["worktree_ctx"]
    plan = prepared["plan"]
    line_name = str(prepared["line_name"])
    onto_line_name = str(prepared["onto_line_name"])
    conflict_entries = [row for row in plan["files"] if row["resolution"] == "conflict"]
    rendered_conflicts: list[dict[str, Any]] = []
    conn = local_content.connect_sqlite(worktree_ctx.content_db_path)
    try:
        for entry in conflict_entries:
            path = str(entry["path"])
            base_text = _decode_merge_text(_read_snapshot_blob_bytes(conn, worktree_ctx, entry.get("old"))) or ""
            feature_bytes = _read_snapshot_blob_bytes(conn, worktree_ctx, entry.get("feature"))
            target_bytes = _read_snapshot_blob_bytes(conn, worktree_ctx, entry.get("target"))
            feature_text = _decode_merge_text(feature_bytes)
            target_text = _decode_merge_text(target_bytes)
            if feature_text is not None and target_text is not None:
                _write_workspace_text_file(
                    worktree_ctx,
                    path,
                    _render_rebase_conflict_text(
                        base_text=base_text,
                        feature_text=feature_text,
                        target_text=target_text,
                        feature_label=line_name,
                        target_label=onto_line_name,
                    ),
                )
                rendered_conflicts.append({"path": path, "kind": "text_markers"})
                continue
            if entry.get("feature") is not None:
                _write_workspace_snapshot_row(conn, worktree_ctx, path, entry.get("feature"))
                rendered_conflicts.append({"path": path, "kind": "binary_or_non_utf8_feature_version"})
            else:
                _write_workspace_snapshot_row(conn, worktree_ctx, path, entry.get("target"))
                rendered_conflicts.append({"path": path, "kind": "binary_or_non_utf8_target_version"})
    finally:
        conn.close()
    return rendered_conflicts


def rebase_worktree(
    ctx: RepoContext,
    name: str | None = None,
    *,
    onto_line_name: str | None = None,
) -> dict[str, Any]:
    prepared = _prepare_worktree_rebase(ctx, name, onto_line_name=onto_line_name)
    repo_ctx: RepoContext = prepared["repo_ctx"]
    worktree_name = str(prepared["worktree_name"])
    worktree_ctx: RepoContext = prepared["worktree_ctx"]
    line_name = str(prepared["line_name"])
    old_head_snapshot_id = str(prepared["old_head_snapshot_id"])
    new_base_snapshot_id = str(prepared["new_base_snapshot_id"])
    onto_line_name = str(prepared["onto_line_name"])
    plan = prepared["plan"]

    if not workspace_status(worktree_ctx)["clean"]:
        raise ValueError(f"Worktree {worktree_name} has unsaved changes. Snapshot or clean it before rebasing.")
    materialized_snapshot_id = _worktree_materialized_snapshot_id(worktree_ctx)
    if materialized_snapshot_id != old_head_snapshot_id:
        local_content.restore_workspace(
            worktree_ctx,
            old_head_snapshot_id,
            baseline_snapshot_id=materialized_snapshot_id,
            force=False,
            dry_run=False,
        )
        _set_worktree_materialized_snapshot(worktree_ctx, old_head_snapshot_id)

    if str(prepared["old_base_snapshot_id"]) == new_base_snapshot_id:
        _write_worktree_metadata_fields(
            repo_ctx,
            worktree_name,
            fork_snapshot_id=new_base_snapshot_id,
            forked_from_line=onto_line_name,
            target_base_line=onto_line_name,
            last_retargeted_at=utc_now(),
            rebase_state="idle",
            rebase_conflict_paths=[],
            clear_keys=("rebase_started_at", "rebase_original_head_snapshot_id", "rebase_onto_snapshot_id"),
        )
        summary = get_worktree(repo_ctx, worktree_name)
        summary["rebase"] = {**plan, "worktree_name": worktree_name, "path": str(prepared["worktree_path"]), "status": "noop"}
        return summary

    local_content.restore_workspace(
        worktree_ctx,
        new_base_snapshot_id,
        baseline_snapshot_id=old_head_snapshot_id,
        force=False,
        dry_run=False,
    )
    _set_worktree_materialized_snapshot(worktree_ctx, new_base_snapshot_id)

    conn = local_content.connect_sqlite(worktree_ctx.content_db_path)
    try:
        for entry in plan["files"]:
            if entry["resolution"] == "conflict":
                continue
            if entry["apply_status"] == "write":
                _write_workspace_snapshot_row(conn, worktree_ctx, str(entry["path"]), entry.get("feature"))
            elif entry["apply_status"] == "remove":
                _write_workspace_snapshot_row(conn, worktree_ctx, str(entry["path"]), None)
    finally:
        conn.close()

    if plan["conflict_count"]:
        rendered_conflicts = _materialize_worktree_rebase_conflicts(prepared)
        _write_worktree_metadata_fields(
            repo_ctx,
            worktree_name,
            target_base_line=onto_line_name,
            rebase_state="conflicted",
            rebase_started_at=utc_now(),
            rebase_original_head_snapshot_id=old_head_snapshot_id,
            rebase_onto_snapshot_id=new_base_snapshot_id,
            rebase_conflict_paths=plan["conflict_paths"],
        )
        local_control.record_event(
            repo_ctx,
            "worktree.rebase_conflicted",
            "worktree",
            worktree_name,
            {
                "name": worktree_name,
                "line_name": line_name,
                "onto_line_name": onto_line_name,
                "old_base_snapshot_id": prepared["old_base_snapshot_id"],
                "old_head_snapshot_id": old_head_snapshot_id,
                "new_base_snapshot_id": new_base_snapshot_id,
                "conflict_paths": plan["conflict_paths"],
            },
        )
        summary = get_worktree(repo_ctx, worktree_name)
        summary["rebase"] = {
            **plan,
            "worktree_name": worktree_name,
            "path": str(prepared["worktree_path"]),
            "status": "conflicted",
            "rendered_conflicts": rendered_conflicts,
        }
        return summary

    new_status = workspace_status(worktree_ctx)
    if new_status["clean"]:
        new_head_snapshot_id = new_base_snapshot_id
    else:
        new_snapshot = create_snapshot(
            worktree_ctx,
            f"Rebase {line_name} onto {onto_line_name}",
            parent_snapshot_id=new_base_snapshot_id,
        )
        new_head_snapshot_id = str(new_snapshot["snapshot_id"])
    set_line_head(worktree_ctx, line_name, new_head_snapshot_id)
    _set_worktree_materialized_snapshot(worktree_ctx, new_head_snapshot_id)
    _write_worktree_metadata_fields(
        repo_ctx,
        worktree_name,
        fork_snapshot_id=new_base_snapshot_id,
        forked_from_line=onto_line_name,
        target_base_line=onto_line_name,
        last_retargeted_at=utc_now(),
        rebase_state="idle",
        rebase_conflict_paths=[],
        clear_keys=("rebase_started_at", "rebase_original_head_snapshot_id", "rebase_onto_snapshot_id"),
    )
    local_control.record_event(
        repo_ctx,
        "worktree.rebased",
        "worktree",
        worktree_name,
        {
            "name": worktree_name,
            "line_name": line_name,
            "onto_line_name": onto_line_name,
            "old_base_snapshot_id": prepared["old_base_snapshot_id"],
            "old_head_snapshot_id": old_head_snapshot_id,
            "new_base_snapshot_id": new_base_snapshot_id,
            "new_head_snapshot_id": new_head_snapshot_id,
        },
    )
    summary = get_worktree(repo_ctx, worktree_name)
    summary["rebase"] = {
        **plan,
        "worktree_name": worktree_name,
        "path": str(prepared["worktree_path"]),
        "status": "applied",
        "new_head_snapshot_id": new_head_snapshot_id,
    }
    return summary


def continue_worktree_rebase(ctx: RepoContext, name: str | None = None) -> dict[str, Any]:
    repo_ctx, worktree_name, metadata, worktree_path, worktree_ctx = _resolve_rebase_worktree(ctx, name)
    if str(metadata.get("rebase_state") or "idle") != "conflicted":
        raise ValueError(f"Worktree {worktree_name} has no conflicted rebase to continue.")
    line_name = current_line(worktree_ctx)
    new_base_snapshot_id = normalize_optional_text(metadata.get("rebase_onto_snapshot_id"))
    old_head_snapshot_id = normalize_optional_text(metadata.get("rebase_original_head_snapshot_id"))
    onto_line_name = normalize_optional_text(metadata.get("target_base_line")) or normalize_optional_text(
        metadata.get("forked_from_line")
    )
    if new_base_snapshot_id is None or old_head_snapshot_id is None or onto_line_name is None:
        raise ValueError(f"Worktree {worktree_name} is missing rebase state metadata; abort and retry.")
    current_delta = local_content.workspace_delta(worktree_ctx, new_base_snapshot_id)
    if current_delta["clean"]:
        new_head_snapshot_id = new_base_snapshot_id
    else:
        new_snapshot = create_snapshot(
            worktree_ctx,
            f"Rebase {line_name} onto {onto_line_name}",
            parent_snapshot_id=new_base_snapshot_id,
        )
        new_head_snapshot_id = str(new_snapshot["snapshot_id"])
    set_line_head(worktree_ctx, line_name, new_head_snapshot_id)
    _set_worktree_materialized_snapshot(worktree_ctx, new_head_snapshot_id)
    _write_worktree_metadata_fields(
        repo_ctx,
        worktree_name,
        fork_snapshot_id=new_base_snapshot_id,
        forked_from_line=onto_line_name,
        target_base_line=onto_line_name,
        last_retargeted_at=utc_now(),
        rebase_state="idle",
        rebase_conflict_paths=[],
        clear_keys=("rebase_started_at", "rebase_original_head_snapshot_id", "rebase_onto_snapshot_id"),
    )
    local_control.record_event(
        repo_ctx,
        "worktree.rebase_continued",
        "worktree",
        worktree_name,
        {
            "name": worktree_name,
            "line_name": line_name,
            "onto_line_name": onto_line_name,
            "old_head_snapshot_id": old_head_snapshot_id,
            "new_base_snapshot_id": new_base_snapshot_id,
            "new_head_snapshot_id": new_head_snapshot_id,
        },
    )
    summary = get_worktree(repo_ctx, worktree_name)
    summary["rebase"] = {
        "worktree_name": worktree_name,
        "path": str(worktree_path),
        "status": "continued",
        "old_head_snapshot_id": old_head_snapshot_id,
        "new_base_snapshot_id": new_base_snapshot_id,
        "new_head_snapshot_id": new_head_snapshot_id,
    }
    return summary


def abort_worktree_rebase(ctx: RepoContext, name: str | None = None) -> dict[str, Any]:
    repo_ctx, worktree_name, metadata, worktree_path, worktree_ctx = _resolve_rebase_worktree(ctx, name)
    if str(metadata.get("rebase_state") or "idle") != "conflicted":
        raise ValueError(f"Worktree {worktree_name} has no conflicted rebase to abort.")
    old_head_snapshot_id = normalize_optional_text(metadata.get("rebase_original_head_snapshot_id"))
    if old_head_snapshot_id is None:
        raise ValueError(f"Worktree {worktree_name} is missing the original head snapshot; manual recovery is required.")
    baseline_snapshot_id = _worktree_materialized_snapshot_id(worktree_ctx)
    local_content.restore_workspace(
        worktree_ctx,
        old_head_snapshot_id,
        baseline_snapshot_id=baseline_snapshot_id,
        force=True,
        dry_run=False,
    )
    set_line_head(worktree_ctx, current_line(worktree_ctx), old_head_snapshot_id)
    _set_worktree_materialized_snapshot(worktree_ctx, old_head_snapshot_id)
    _write_worktree_metadata_fields(
        repo_ctx,
        worktree_name,
        rebase_state="idle",
        rebase_conflict_paths=[],
        clear_keys=("rebase_started_at", "rebase_original_head_snapshot_id", "rebase_onto_snapshot_id"),
    )
    local_control.record_event(
        repo_ctx,
        "worktree.rebase_aborted",
        "worktree",
        worktree_name,
        {
            "name": worktree_name,
            "restored_snapshot_id": old_head_snapshot_id,
        },
    )
    summary = get_worktree(repo_ctx, worktree_name)
    summary["rebase"] = {
        "worktree_name": worktree_name,
        "path": str(worktree_path),
        "status": "aborted",
        "restored_snapshot_id": old_head_snapshot_id,
    }
    return summary
