from __future__ import annotations

from typing import Any

from ait_protocol.common import connect_sqlite, normalize_optional_text

from . import local_content, local_content_snapshots, local_control
from .local_content_projection import (
    _effective_workspace_ignore_rules,
    _filter_snapshot_file_map_for_workspace,
    _filter_workspace_state_for_workspace,
)
from .repo_paths import RepoContext
from .store_repo_config import _set_worktree_materialized_snapshot, load_config
from .store_worktree_runtime import current_line


def _snapshot_changed_paths(
    ctx: RepoContext,
    *,
    old_snapshot_id: str | None,
    new_snapshot_id: str | None,
) -> list[str]:
    from .snapshot_diff import snapshot_diff as build_snapshot_diff

    diff = build_snapshot_diff(
        ctx,
        old_snapshot_id,
        new_snapshot_id,
        include_text=False,
    )
    return sorted(
        {
            str(row.get("path") or "").strip()
            for row in diff.get("files", [])
            if isinstance(row, dict) and str(row.get("path") or "").strip()
        }
    )


def _current_line_head_snapshot_id(ctx: RepoContext) -> tuple[str, str | None]:
    current_line_name = current_line(ctx)
    current_line_row = local_content.get_line(ctx, current_line_name)
    return current_line_name, normalize_optional_text(current_line_row.get("head_snapshot_id"))


def _require_current_line_head_snapshot(
    ctx: RepoContext,
    *,
    expected_snapshot_id: str,
    command_name: str,
) -> tuple[str, str]:
    current_line_name, current_head_snapshot_id = _current_line_head_snapshot_id(ctx)
    if current_head_snapshot_id is None:
        raise ValueError(f"Current line {current_line_name} has no head snapshot to revert from.")
    if current_head_snapshot_id != expected_snapshot_id:
        raise ValueError(
            f"`{command_name}` currently supports reverting only the current line head snapshot. "
            f"Current line {current_line_name} points at {current_head_snapshot_id}, not {expected_snapshot_id}."
        )
    return current_line_name, current_head_snapshot_id


def _apply_workspace_revert_range(
    ctx: RepoContext,
    *,
    base_snapshot_id: str | None,
    head_snapshot_id: str,
    force: bool = False,
    dry_run: bool = False,
) -> dict[str, Any]:
    affected_paths = _snapshot_changed_paths(
        ctx,
        old_snapshot_id=base_snapshot_id,
        new_snapshot_id=head_snapshot_id,
    )
    result = local_content_snapshots.restore_workspace_paths(
        ctx,
        base_snapshot_id,
        affected_paths,
        baseline_snapshot_id=head_snapshot_id,
        force=force,
        dry_run=dry_run,
    )
    result["affected_paths"] = affected_paths
    result["affected_path_count"] = len(affected_paths)
    if not dry_run:
        if affected_paths and not result.get("dirty_outside_paths"):
            _set_worktree_materialized_snapshot(ctx, base_snapshot_id)
        elif result.get("dirty_outside_paths"):
            _set_worktree_materialized_snapshot(ctx, None)
    return result


def _require_current_line_target(
    ctx: RepoContext,
    *,
    onto_line: str,
    command_name: str,
) -> tuple[str, str | None]:
    current_line_name, current_head_snapshot_id = _current_line_head_snapshot_id(ctx)
    if current_line_name != onto_line:
        raise ValueError(
            f"`{command_name}` currently replays only onto the current line workspace. "
            f"Current line is {current_line_name}, not {onto_line}."
        )
    return current_line_name, current_head_snapshot_id


def _apply_workspace_replay_range(
    ctx: RepoContext,
    *,
    source_base_snapshot_id: str,
    source_head_snapshot_id: str,
    baseline_snapshot_id: str | None,
    force: bool = False,
    dry_run: bool = False,
) -> dict[str, Any]:
    from .snapshot_diff import snapshot_diff as build_snapshot_diff

    conn = connect_sqlite(ctx.content_db_path)
    source_base_files = _filter_snapshot_file_map_for_workspace(
        ctx,
        local_content_snapshots._snapshot_file_map(conn, source_base_snapshot_id),
    )
    source_head_files = _filter_snapshot_file_map_for_workspace(
        ctx,
        local_content_snapshots._snapshot_file_map(conn, source_head_snapshot_id),
    )
    delta = build_snapshot_diff(
        ctx,
        source_base_snapshot_id,
        source_head_snapshot_id,
        include_text=False,
    )
    affected_paths = sorted(
        {
            str(row.get("path") or "").strip()
            for row in delta.get("files", [])
            if isinstance(row, dict) and str(row.get("path") or "").strip()
        }
    )
    delta_status_by_path = {
        str(row.get("path") or "").strip(): str(row.get("status") or "").strip() or "unchanged"
        for row in delta.get("files", [])
        if isinstance(row, dict) and str(row.get("path") or "").strip()
    }
    snapshot_ignore_rules = local_content._snapshot_workspace_ignore_rules(
        source_head_snapshot_id,
        source_head_files,
        lambda blob_id: local_content._blob_bytes_by_id(ctx, conn, blob_id).decode("utf-8", errors="replace"),
    )
    effective_ignore_rules = _effective_workspace_ignore_rules(ctx, snapshot_ignore_rules)
    dirty = local_content_snapshots.workspace_delta(ctx, baseline_snapshot_id, ignore_rules=snapshot_ignore_rules)
    workspace_files = _filter_workspace_state_for_workspace(
        ctx,
        local_content._workspace_state(ctx.root, ignore_rules=effective_ignore_rules),
    )

    requested_set = set(affected_paths)
    dirty_selected_paths = sorted(set(dirty["changed_paths"]) & requested_set)
    dirty_outside_paths = sorted(set(dirty["changed_paths"]) - requested_set)

    write_paths: list[str] = []
    remove_paths: list[str] = []
    unchanged_paths: list[str] = []
    for rel in affected_paths:
        status = delta_status_by_path.get(rel, "unchanged")
        source_row = source_head_files.get(rel)
        current = workspace_files.get(rel)
        if status == "deleted":
            if current is None:
                unchanged_paths.append(rel)
            else:
                remove_paths.append(rel)
            continue
        if source_row is None:
            conn.close()
            raise ValueError(
                f"Replay source snapshot `{source_head_snapshot_id}` is missing the changed file `{rel}`."
            )
        if current is None or current["sha256"] != source_row["sha256"] or current["mode"] != source_row["mode"]:
            write_paths.append(rel)
        else:
            unchanged_paths.append(rel)

    result = {
        "source_base_snapshot_id": source_base_snapshot_id,
        "source_head_snapshot_id": source_head_snapshot_id,
        "baseline_snapshot_id": baseline_snapshot_id,
        "force": force,
        "dry_run": dry_run,
        "applied": False,
        "workspace_dirty": not dirty["clean"],
        "would_overwrite_selected_changes": bool(dirty_selected_paths),
        "dirty_workspace": dirty,
        "dirty_selected_paths": dirty_selected_paths,
        "dirty_outside_paths": dirty_outside_paths,
        "affected_paths": affected_paths,
        "affected_path_count": len(affected_paths),
        "delta_summary": {
            "added": list(delta.get("added") or []),
            "deleted": list(delta.get("deleted") or []),
            "modified": list(delta.get("modified") or []),
            "mode_changed": list(delta.get("mode_changed") or []),
        },
        "plan": {
            "write_count": len(write_paths),
            "remove_count": len(remove_paths),
            "unchanged_count": len(unchanged_paths),
            "requested_paths": affected_paths,
            "write_paths": sorted(write_paths),
            "remove_paths": sorted(remove_paths, key=lambda item: (item.count("/"), item), reverse=True),
            "unchanged_paths": unchanged_paths,
        },
    }
    if dirty_selected_paths and not force and not dry_run:
        sample = ", ".join(dirty_selected_paths[:5])
        if len(dirty_selected_paths) > 5:
            sample += ", ..."
        baseline_label = baseline_snapshot_id or "empty workspace"
        conn.close()
        raise ValueError(f"Selected paths have unsaved changes relative to {baseline_label}: {sample}")
    if dry_run:
        conn.close()
        return result

    for rel in result["plan"]["remove_paths"]:
        abs_path = ctx.root / rel
        if abs_path.exists():
            abs_path.unlink()
            local_content_snapshots._prune_empty_parent_dirs(ctx.root, abs_path)

    for rel in sorted(write_paths):
        source_row = source_head_files[rel]
        abs_path = ctx.root / rel
        abs_path.parent.mkdir(parents=True, exist_ok=True)
        if abs_path.exists() and abs_path.is_dir():
            conn.close()
            raise IsADirectoryError(f"Cannot replay file over directory: {rel}")
        data = local_content._blob_bytes_by_id(ctx, conn, source_row["blob_id"])
        abs_path.write_bytes(data)
        abs_path.chmod(local_content_snapshots._parse_mode_bits(source_row["mode"]))

    conn.close()
    result["applied"] = True
    if affected_paths and (write_paths or remove_paths):
        _set_worktree_materialized_snapshot(ctx, None)
    return result


def revert_snapshot(
    ctx: RepoContext,
    snapshot_id: str,
    *,
    force: bool = False,
    dry_run: bool = False,
) -> dict[str, Any]:
    snapshot = local_content_snapshots.get_snapshot(ctx, snapshot_id)
    snapshot_kind = str(snapshot.get("snapshot_kind") or "line").strip() or "line"
    if snapshot_kind != "line":
        raise ValueError(
            f"Snapshot {snapshot_id} is `{snapshot_kind}`. Use the matching first-class surface instead of `snapshot revert`."
        )
    parent_snapshot_id = normalize_optional_text(snapshot.get("parent_snapshot_id"))
    current_line_name, current_head_snapshot_id = _require_current_line_head_snapshot(
        ctx,
        expected_snapshot_id=snapshot_id,
        command_name="snapshot revert",
    )
    result = _apply_workspace_revert_range(
        ctx,
        base_snapshot_id=parent_snapshot_id,
        head_snapshot_id=current_head_snapshot_id,
        force=force,
        dry_run=dry_run,
    )
    payload = {
        **result,
        "repo_name": load_config(ctx).get("repo_name") or ctx.root.name,
        "snapshot_id": snapshot_id,
        "parent_snapshot_id": parent_snapshot_id,
        "current_line": current_line_name,
        "current_line_head_snapshot_id": current_head_snapshot_id,
        "mutation_scope": "workspace_only",
        "moves_line_head": False,
        "creates_snapshot": False,
    }
    if not dry_run:
        local_control.record_event(
            ctx,
            "snapshot.reverted",
            "snapshot",
            snapshot_id,
            {
                "snapshot_id": snapshot_id,
                "parent_snapshot_id": parent_snapshot_id,
                "current_line": current_line_name,
                "affected_path_count": payload["affected_path_count"],
                "force": force,
            },
        )
    return payload


def replay_snapshot(
    ctx: RepoContext,
    snapshot_id: str,
    *,
    onto_line: str,
    force: bool = False,
    dry_run: bool = False,
) -> dict[str, Any]:
    snapshot = local_content_snapshots.get_snapshot(ctx, snapshot_id)
    snapshot_kind = str(snapshot.get("snapshot_kind") or "line").strip() or "line"
    if snapshot_kind != "line":
        raise ValueError(
            f"Snapshot {snapshot_id} is `{snapshot_kind}`. Use the matching first-class surface instead of `snapshot replay`."
        )
    parent_snapshot_id = normalize_optional_text(snapshot.get("parent_snapshot_id"))
    if parent_snapshot_id is None:
        raise ValueError(
            f"Snapshot {snapshot_id} has no parent snapshot, so `snapshot replay` cannot compute a replay delta."
        )
    current_line_name, current_head_snapshot_id = _require_current_line_target(
        ctx,
        onto_line=onto_line,
        command_name="snapshot replay",
    )
    result = _apply_workspace_replay_range(
        ctx,
        source_base_snapshot_id=parent_snapshot_id,
        source_head_snapshot_id=snapshot_id,
        baseline_snapshot_id=current_head_snapshot_id,
        force=force,
        dry_run=dry_run,
    )
    payload = {
        **result,
        "repo_name": load_config(ctx).get("repo_name") or ctx.root.name,
        "snapshot_id": snapshot_id,
        "parent_snapshot_id": parent_snapshot_id,
        "source_line": normalize_optional_text(snapshot.get("line_name")),
        "onto_line": current_line_name,
        "onto_line_head_snapshot_id": current_head_snapshot_id,
        "mutation_scope": "workspace_only",
        "moves_line_head": False,
        "creates_snapshot": False,
    }
    if not dry_run:
        local_control.record_event(
            ctx,
            "snapshot.replayed",
            "snapshot",
            snapshot_id,
            {
                "snapshot_id": snapshot_id,
                "parent_snapshot_id": parent_snapshot_id,
                "source_line": normalize_optional_text(snapshot.get("line_name")),
                "onto_line": current_line_name,
                "onto_line_head_snapshot_id": current_head_snapshot_id,
                "affected_path_count": payload["affected_path_count"],
                "force": force,
            },
        )
    return payload


def revert_change(
    ctx: RepoContext,
    change_id: str,
    *,
    task_id: str | None = None,
    fork_snapshot_id: str | None = None,
    force: bool = False,
    dry_run: bool = False,
) -> dict[str, Any]:
    resolved_change_id = str(change_id or "").strip()
    if not resolved_change_id:
        raise ValueError("change_id is required")
    change_snapshot_rows = local_control.list_workflow_snapshot_provenance_for_change(ctx, resolved_change_id)
    latest_snapshot_id = normalize_optional_text((change_snapshot_rows[0] if change_snapshot_rows else {}).get("snapshot_id"))
    if latest_snapshot_id is None:
        raise ValueError(
            f"Change {resolved_change_id} has no recorded local snapshot lineage to revert. "
            "Create a snapshot from the bound task worktree before retrying `ait change revert`."
        )
    resolved_fork_snapshot_id = normalize_optional_text(fork_snapshot_id)
    if resolved_fork_snapshot_id is None:
        raise ValueError(
            f"Change {resolved_change_id} is missing fork_snapshot_id lineage metadata, so `ait change revert` cannot compute a safe base."
        )
    if not local_content_snapshots.snapshot_exists(ctx, latest_snapshot_id):
        raise KeyError(f"Latest recorded change snapshot is not available locally: {latest_snapshot_id}")
    if not local_content_snapshots.snapshot_exists(ctx, resolved_fork_snapshot_id):
        raise KeyError(f"Change fork snapshot is not available locally: {resolved_fork_snapshot_id}")
    latest_chain = local_content.collect_snapshot_chain(ctx, latest_snapshot_id)
    if resolved_fork_snapshot_id not in latest_chain:
        raise ValueError(
            f"Change {resolved_change_id} fork snapshot {resolved_fork_snapshot_id} is not an ancestor of latest recorded change snapshot {latest_snapshot_id}."
        )
    current_line_name, current_head_snapshot_id = _require_current_line_head_snapshot(
        ctx,
        expected_snapshot_id=latest_snapshot_id,
        command_name="change revert",
    )
    result = _apply_workspace_revert_range(
        ctx,
        base_snapshot_id=resolved_fork_snapshot_id,
        head_snapshot_id=current_head_snapshot_id,
        force=force,
        dry_run=dry_run,
    )
    payload = {
        **result,
        "repo_name": load_config(ctx).get("repo_name") or ctx.root.name,
        "change_id": resolved_change_id,
        "task_id": normalize_optional_text(task_id),
        "fork_snapshot_id": resolved_fork_snapshot_id,
        "latest_change_snapshot_id": latest_snapshot_id,
        "current_line": current_line_name,
        "current_line_head_snapshot_id": current_head_snapshot_id,
        "mutation_scope": "workspace_only",
        "moves_line_head": False,
        "creates_snapshot": False,
    }
    if not dry_run:
        local_control.record_event(
            ctx,
            "change.reverted",
            "change",
            resolved_change_id,
            {
                "change_id": resolved_change_id,
                "task_id": normalize_optional_text(task_id),
                "fork_snapshot_id": resolved_fork_snapshot_id,
                "latest_change_snapshot_id": latest_snapshot_id,
                "current_line": current_line_name,
                "affected_path_count": payload["affected_path_count"],
                "force": force,
            },
        )
    return payload


def replay_change(
    ctx: RepoContext,
    change_id: str,
    *,
    onto_line: str,
    task_id: str | None = None,
    fork_snapshot_id: str | None = None,
    force: bool = False,
    dry_run: bool = False,
) -> dict[str, Any]:
    resolved_change_id = str(change_id or "").strip()
    if not resolved_change_id:
        raise ValueError("change_id is required")
    change_snapshot_rows = local_control.list_workflow_snapshot_provenance_for_change(ctx, resolved_change_id)
    latest_snapshot_id = normalize_optional_text((change_snapshot_rows[0] if change_snapshot_rows else {}).get("snapshot_id"))
    if latest_snapshot_id is None:
        raise ValueError(
            f"Change {resolved_change_id} has no recorded local snapshot lineage to replay. "
            "Create a snapshot from the source worktree before retrying `ait change replay`."
        )
    resolved_fork_snapshot_id = normalize_optional_text(fork_snapshot_id)
    if resolved_fork_snapshot_id is None:
        raise ValueError(
            f"Change {resolved_change_id} is missing fork_snapshot_id lineage metadata, so `ait change replay` cannot compute a safe base."
        )
    if not local_content_snapshots.snapshot_exists(ctx, latest_snapshot_id):
        raise KeyError(f"Latest recorded change snapshot is not available locally: {latest_snapshot_id}")
    if not local_content_snapshots.snapshot_exists(ctx, resolved_fork_snapshot_id):
        raise KeyError(f"Change fork snapshot is not available locally: {resolved_fork_snapshot_id}")
    latest_chain = local_content.collect_snapshot_chain(ctx, latest_snapshot_id)
    if resolved_fork_snapshot_id not in latest_chain:
        raise ValueError(
            f"Change {resolved_change_id} fork snapshot {resolved_fork_snapshot_id} is not an ancestor of latest recorded change snapshot {latest_snapshot_id}."
        )
    current_line_name, current_head_snapshot_id = _require_current_line_target(
        ctx,
        onto_line=onto_line,
        command_name="change replay",
    )
    result = _apply_workspace_replay_range(
        ctx,
        source_base_snapshot_id=resolved_fork_snapshot_id,
        source_head_snapshot_id=latest_snapshot_id,
        baseline_snapshot_id=current_head_snapshot_id,
        force=force,
        dry_run=dry_run,
    )
    payload = {
        **result,
        "repo_name": load_config(ctx).get("repo_name") or ctx.root.name,
        "change_id": resolved_change_id,
        "task_id": normalize_optional_text(task_id),
        "fork_snapshot_id": resolved_fork_snapshot_id,
        "latest_change_snapshot_id": latest_snapshot_id,
        "onto_line": current_line_name,
        "onto_line_head_snapshot_id": current_head_snapshot_id,
        "mutation_scope": "workspace_only",
        "moves_line_head": False,
        "creates_snapshot": False,
    }
    if not dry_run:
        local_control.record_event(
            ctx,
            "change.replayed",
            "change",
            resolved_change_id,
            {
                "change_id": resolved_change_id,
                "task_id": normalize_optional_text(task_id),
                "fork_snapshot_id": resolved_fork_snapshot_id,
                "latest_change_snapshot_id": latest_snapshot_id,
                "onto_line": current_line_name,
                "onto_line_head_snapshot_id": current_head_snapshot_id,
                "affected_path_count": payload["affected_path_count"],
                "force": force,
            },
        )
    return payload
