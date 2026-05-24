from __future__ import annotations

from pathlib import Path
from typing import Any

from ait_protocol.common import normalize_optional_text, utc_now

from . import local_content, local_control
from .remote_client import (
    RemoteError,
    get_change as remote_get_change,
    get_patchset as remote_get_patchset,
)
from .repo_paths import RepoContext
from .store_repo_config import (
    _set_worktree_materialized_snapshot,
    _worktree_materialized_snapshot_id,
    load_config,
)
from .store_worktree_cleanup import (
    _touch_worktree_metadata,
    _update_worktree_registration,
)
from .store_worktree_filesystem import (
    _path_exists_or_directory_link,
    _path_is_directory_link,
    _remove_path_entry,
)
from .store_worktree_layout import (
    _materialize_worktree_alias,
    _materialize_worktree_runtime_layout,
)
from .store_worktree_metadata import (
    _build_worktree_status_cache_payload,
    _load_worktree_metadata,
)
from .store_worktree_runtime import (
    _set_current_line,
    create_line,
    current_line,
    get_remote,
    set_line_head,
    switch_line,
    workspace_status,
)
from .store_worktree_state import (
    _repo_worktree_ctx,
    _worktree_metadata_with_defaults,
)
from .store_worktree_views import (
    _maybe_discover_worktree,
    _resolve_worktree_name,
    get_worktree,
    list_worktrees,
)

__all__ = [
    "recreate_worktree",
    "restore_owned_head",
    "sync_all_worktrees",
    "sync_worktree",
]


def _recreate_remote_binding(
    ctx: RepoContext,
    *,
    metadata: dict[str, Any],
) -> tuple[dict[str, Any], str, dict[str, Any] | None, dict[str, Any] | None] | None:
    repo_ctx = _repo_worktree_ctx(ctx)
    bound_task_id = normalize_optional_text(metadata.get("bound_task_id"))
    bound_change_id = normalize_optional_text(metadata.get("bound_change_id"))
    local_task: dict[str, Any] | None = None
    local_change: dict[str, Any] | None = None
    if bound_task_id is not None:
        try:
            local_task = local_control.get_workflow_task(repo_ctx, bound_task_id)
        except KeyError:
            local_task = None
    if bound_change_id is not None:
        try:
            local_change = local_control.get_workflow_change(repo_ctx, bound_change_id)
        except KeyError:
            local_change = None
    remote_name = (
        normalize_optional_text((local_task or {}).get("published_remote_name"))
        or normalize_optional_text((local_change or {}).get("published_remote_name"))
    )
    if remote_name is None:
        return None
    try:
        remote = get_remote(repo_ctx, remote_name)
    except KeyError:
        return None
    repo_name = (
        normalize_optional_text(remote.get("repo_name"))
        or normalize_optional_text(load_config(repo_ctx).get("repo_name"))
        or repo_ctx.root.name
    )
    return remote, repo_name, local_task, local_change


def _candidate_snapshot_row(
    repo_ctx: RepoContext,
    *,
    source: str,
    snapshot_id: str | None,
    detail: dict[str, Any] | None = None,
) -> dict[str, Any] | None:
    resolved_snapshot_id = normalize_optional_text(snapshot_id)
    if resolved_snapshot_id is None or not local_content.snapshot_exists(repo_ctx, resolved_snapshot_id):
        return None
    payload = {
        "source": source,
        "snapshot_id": resolved_snapshot_id,
        "available_locally": True,
    }
    if detail:
        payload.update(detail)
    return payload


def _recreate_worktree_snapshot_candidates(
    ctx: RepoContext,
    *,
    metadata: dict[str, Any],
) -> list[dict[str, Any]]:
    repo_ctx = _repo_worktree_ctx(ctx)
    candidates: list[dict[str, Any]] = []

    line_name = normalize_optional_text(metadata.get("line_name"))
    if line_name is not None:
        try:
            line_row = local_content.get_line(repo_ctx, line_name)
        except KeyError:
            line_row = None
        if line_row is not None:
            line_candidate = _candidate_snapshot_row(
                repo_ctx,
                source="current_line_head",
                snapshot_id=line_row.get("head_snapshot_id"),
                detail={"line_name": line_name},
            )
            if line_candidate is not None:
                candidates.append(line_candidate)

    fork_snapshot_id = normalize_optional_text(metadata.get("fork_snapshot_id"))
    fork_candidate = _candidate_snapshot_row(
        repo_ctx,
        source="fork_snapshot",
        snapshot_id=fork_snapshot_id,
        detail={"forked_from_line": normalize_optional_text(metadata.get("forked_from_line"))},
    )
    if fork_candidate is not None:
        candidates.append(fork_candidate)

    binding = _recreate_remote_binding(ctx, metadata=metadata)
    if binding is not None:
        remote, repo_name, _local_task, local_change = binding
        base_url = str(remote.get("url") or "").strip()
        if base_url:
            change_ref = (
                normalize_optional_text((local_change or {}).get("published_change_id"))
                or normalize_optional_text(metadata.get("bound_change_id"))
            )
            if change_ref is not None:
                try:
                    remote_change = remote_get_change(base_url, change_ref, repo_name=repo_name)
                except (KeyError, RemoteError, ValueError):
                    remote_change = None
                if remote_change is not None:
                    patchset_id = normalize_optional_text(
                        remote_change.get("selected_patchset_id") or remote_change.get("current_patchset_id")
                    )
                    if patchset_id is not None:
                        try:
                            patchset = remote_get_patchset(
                                base_url,
                                patchset_id,
                                repo_name=repo_name,
                                change_ref=change_ref,
                            )
                        except (KeyError, RemoteError, ValueError):
                            patchset = None
                        if patchset is not None:
                            patchset_candidate = _candidate_snapshot_row(
                                repo_ctx,
                                source="remote_patchset_revision",
                                snapshot_id=patchset.get("revision_snapshot_id"),
                                detail={
                                    "change_id": normalize_optional_text(remote_change.get("change_id")) or change_ref,
                                    "patchset_id": patchset_id,
                                },
                            )
                            if patchset_candidate is not None:
                                candidates.append(patchset_candidate)

    deduped: list[dict[str, Any]] = []
    seen_snapshot_ids: set[str] = set()
    for row in candidates:
        snapshot_id = str(row["snapshot_id"])
        if snapshot_id in seen_snapshot_ids:
            continue
        seen_snapshot_ids.add(snapshot_id)
        deduped.append(row)
    return deduped


def _prepare_worktree_recreate(
    ctx: RepoContext,
    name: str | None = None,
) -> dict[str, Any]:
    repo_ctx = _repo_worktree_ctx(ctx)
    worktree_name = _resolve_worktree_name(ctx, name)
    metadata = _worktree_metadata_with_defaults(_load_worktree_metadata(repo_ctx, worktree_name))
    summary = get_worktree(repo_ctx, worktree_name)
    if str(summary.get("workspace_status") or "") != "missing":
        raise ValueError(f"Worktree {worktree_name} is not missing.")
    if normalize_optional_text(metadata.get("bound_task_id")) is None:
        raise ValueError(f"Worktree {worktree_name} is not task-bound; automatic recreate is only supported for task worktrees.")
    line_name = normalize_optional_text(summary.get("registered_line_name")) or normalize_optional_text(metadata.get("line_name"))
    if line_name is None:
        raise ValueError(f"Worktree {worktree_name} has no registered line to recreate.")
    candidates = _recreate_worktree_snapshot_candidates(repo_ctx, metadata=metadata)
    if not candidates:
        raise ValueError(
            f"Worktree {worktree_name} has no locally available recreate snapshot. "
            "Expected one of the bound line head, fork snapshot, or selected patchset revision to still exist."
        )
    target_path = Path(str(metadata.get("path") or "")).expanduser().resolve()
    alias_path_value = normalize_optional_text(metadata.get("alias_path"))
    alias_path = Path(alias_path_value).expanduser() if alias_path_value is not None else None
    return {
        "repo_ctx": repo_ctx,
        "worktree_name": worktree_name,
        "metadata": metadata,
        "summary": summary,
        "line_name": line_name,
        "target_path": target_path,
        "alias_path": alias_path,
        "candidate": candidates[0],
        "candidates": candidates,
    }


def recreate_worktree(
    ctx: RepoContext,
    name: str | None = None,
    *,
    dry_run: bool = False,
) -> dict[str, Any]:
    prepared = _prepare_worktree_recreate(ctx, name)
    repo_ctx: RepoContext = prepared["repo_ctx"]
    worktree_name = str(prepared["worktree_name"])
    metadata = dict(prepared["metadata"])
    line_name = str(prepared["line_name"])
    target_path: Path = prepared["target_path"]
    alias_path: Path | None = prepared["alias_path"]
    candidate = dict(prepared["candidate"])
    chosen_snapshot_id = str(candidate["snapshot_id"])

    if _path_exists_or_directory_link(target_path):
        raise ValueError(f"Worktree path is no longer missing: {target_path}")
    if alias_path is not None and _path_exists_or_directory_link(alias_path) and not _path_is_directory_link(alias_path):
        raise ValueError(f"Worktree alias path is occupied by a non-link entry: {alias_path}")
    if alias_path is not None and _path_is_directory_link(alias_path):
        alias_target = alias_path.resolve(strict=False)
        if alias_target != target_path:
            raise ValueError(f"Worktree alias path points at a different target and cannot be reclaimed automatically: {alias_path}")

    recreate_payload = {
        "name": worktree_name,
        "path": str(target_path),
        "alias_path": str(alias_path) if alias_path is not None else None,
        "line_name": line_name,
        "workspace_status_before": prepared["summary"].get("workspace_status"),
        "dry_run": dry_run,
        "recreate": {
            "candidate": candidate,
            "candidates": prepared["candidates"],
            "managed_alias_recreated": bool(alias_path is not None),
        },
    }
    if dry_run:
        return recreate_payload

    created_at = normalize_optional_text(metadata.get("created_at")) or utc_now()
    worktree_ctx = _materialize_worktree_runtime_layout(
        repo_ctx,
        worktree_name=worktree_name,
        target_path=target_path,
        line_name=line_name,
        created_at=created_at,
    )
    if alias_path is not None:
        if _path_is_directory_link(alias_path):
            _remove_path_entry(alias_path)
        _materialize_worktree_alias(target_path, alias_path)

    try:
        line_row = local_content.get_line(worktree_ctx, line_name)
    except KeyError:
        create_line(worktree_ctx, line_name, chosen_snapshot_id)
        line_row = local_content.get_line(worktree_ctx, line_name)
    current_head_snapshot_id = normalize_optional_text(line_row.get("head_snapshot_id"))
    if current_head_snapshot_id is None or (
        current_head_snapshot_id != chosen_snapshot_id and not local_content.snapshot_exists(repo_ctx, current_head_snapshot_id)
    ):
        set_line_head(worktree_ctx, line_name, chosen_snapshot_id)
    switch_line(worktree_ctx, line_name)

    local_content.restore_workspace(
        worktree_ctx,
        chosen_snapshot_id,
        baseline_snapshot_id=None,
        force=True,
        dry_run=False,
    )
    _set_worktree_materialized_snapshot(worktree_ctx, chosen_snapshot_id)
    recreated_at = utc_now()
    _update_worktree_registration(
        repo_ctx,
        worktree_name,
        last_used_at=recreated_at,
        workspace_status_cache=_build_worktree_status_cache_payload(
            workspace_status_value="clean",
            clean=True,
            changed_count=0,
            modified_paths=[],
            missing_paths=[],
            untracked_paths=[],
            current_line_name=line_name,
            head_snapshot_id=chosen_snapshot_id,
            status_checked_at=recreated_at,
        ),
    )
    local_control.record_event(
        repo_ctx,
        "worktree.recreated",
        "worktree",
        worktree_name,
        {
            "name": worktree_name,
            "path": str(target_path),
            "alias_path": str(alias_path) if alias_path is not None else None,
            "line_name": line_name,
            "snapshot_id": chosen_snapshot_id,
            "source": candidate.get("source"),
        },
    )
    return {
        **get_worktree(repo_ctx, worktree_name),
        **recreate_payload,
        "workspace_status_after": "clean",
        "head_snapshot_id": chosen_snapshot_id,
    }


def _restore_owned_head_descendancy_error(current_head_snapshot_id: str, fork_snapshot_id: str) -> ValueError:
    return ValueError(
        f"Current line head `{current_head_snapshot_id}` does not descend from registered fork "
        f"`{fork_snapshot_id}`. Rebase, recreate, or retarget the bound worktree before running "
        "`ait worktree restore-owned-head`."
    )


def _snapshot_matches_bound_slice(
    snapshot_id: str,
    *,
    provenance_rows: dict[str, dict[str, Any]],
    expected_task_id: str,
    expected_change_id: str | None,
    expected_worktree_name: str | None,
) -> dict[str, Any]:
    provenance = provenance_rows.get(snapshot_id)
    if provenance is None:
        return {
            "snapshot_id": snapshot_id,
            "owned": False,
            "reason": "missing workflow provenance",
            "owner_task_id": None,
            "owner_change_id": None,
            "owner_worktree_name": None,
        }
    provenance_task_id = normalize_optional_text(provenance.get("task_id"))
    provenance_change_id = normalize_optional_text(provenance.get("change_id"))
    provenance_worktree_name = normalize_optional_text(provenance.get("worktree_name"))
    if provenance_task_id != expected_task_id:
        return {
            "snapshot_id": snapshot_id,
            "owned": False,
            "reason": f"task {provenance_task_id or 'none'}",
            "owner_task_id": provenance_task_id,
            "owner_change_id": provenance_change_id,
            "owner_worktree_name": provenance_worktree_name,
        }
    if expected_change_id is not None and provenance_change_id not in {None, expected_change_id}:
        return {
            "snapshot_id": snapshot_id,
            "owned": False,
            "reason": f"change {provenance_change_id}",
            "owner_task_id": provenance_task_id,
            "owner_change_id": provenance_change_id,
            "owner_worktree_name": provenance_worktree_name,
        }
    if expected_worktree_name is not None and provenance_worktree_name not in {None, expected_worktree_name}:
        return {
            "snapshot_id": snapshot_id,
            "owned": False,
            "reason": f"worktree {provenance_worktree_name}",
            "owner_task_id": provenance_task_id,
            "owner_change_id": provenance_change_id,
            "owner_worktree_name": provenance_worktree_name,
        }
    return {
        "snapshot_id": snapshot_id,
        "owned": True,
        "reason": None,
        "owner_task_id": provenance_task_id,
        "owner_change_id": provenance_change_id,
        "owner_worktree_name": provenance_worktree_name,
    }


def _prepare_worktree_restore_owned_head(
    ctx: RepoContext,
    name: str | None = None,
) -> dict[str, Any]:
    repo_ctx = _repo_worktree_ctx(ctx)
    worktree_name = _resolve_worktree_name(ctx, name)
    metadata = _worktree_metadata_with_defaults(_load_worktree_metadata(repo_ctx, worktree_name))
    if str(metadata.get("rebase_state") or "idle") == "conflicted":
        raise ValueError(f"Worktree {worktree_name} is in a conflicted rebase. Use `ait worktree rebase --continue` or `--abort`.")
    worktree_path = Path(str(metadata.get("path") or "")).expanduser().resolve()
    worktree_ctx = _maybe_discover_worktree(worktree_path)
    if worktree_ctx is None:
        raise ValueError(f"Worktree is missing or detached: {worktree_name}")
    bound_task_id = normalize_optional_text(metadata.get("bound_task_id"))
    if bound_task_id is None:
        raise ValueError(
            f"Worktree {worktree_name} is not task-bound; `ait worktree restore-owned-head` only supports task worktrees."
        )
    bound_change_id = normalize_optional_text(metadata.get("bound_change_id"))
    current_line_name = current_line(worktree_ctx)
    current_line_row = local_content.get_line(worktree_ctx, current_line_name)
    current_head_snapshot_id = normalize_optional_text(current_line_row.get("head_snapshot_id"))
    if current_head_snapshot_id is None:
        raise ValueError(f"Worktree {worktree_name} has no current line head to restore.")
    fork_snapshot_id = normalize_optional_text(metadata.get("fork_snapshot_id"))
    if fork_snapshot_id is None:
        raise ValueError(f"Worktree {worktree_name} has no registered fork snapshot to restore against.")
    chain = local_content.collect_snapshot_chain(worktree_ctx, current_head_snapshot_id)
    if fork_snapshot_id not in chain:
        raise _restore_owned_head_descendancy_error(current_head_snapshot_id, fork_snapshot_id)
    workspace = workspace_status(worktree_ctx, snapshot_id=current_head_snapshot_id)
    if not workspace.get("clean", True):
        changed_paths = [str(path) for path in (workspace.get("changed_paths") or [])[:5]]
        sample = ", ".join(changed_paths)
        if int(workspace.get("changed_count") or 0) > 5:
            sample += ", ..."
        raise ValueError(
            f"Worktree {worktree_name} has unsaved changes relative to current head `{current_head_snapshot_id}`: {sample}"
        )
    head_segment = chain[chain.index(fork_snapshot_id) + 1 :]
    provenance_rows = {
        str(row.get("snapshot_id") or "").strip(): row
        for row in local_control.list_workflow_snapshot_provenance(worktree_ctx, snapshot_ids=head_segment)
        if str(row.get("snapshot_id") or "").strip()
    }
    return {
        "repo_ctx": repo_ctx,
        "worktree_name": worktree_name,
        "worktree_path": worktree_path,
        "worktree_ctx": worktree_ctx,
        "bound_task_id": bound_task_id,
        "bound_change_id": bound_change_id,
        "current_line_name": current_line_name,
        "current_head_snapshot_id": current_head_snapshot_id,
        "materialized_snapshot_id_before": _worktree_materialized_snapshot_id(worktree_ctx),
        "fork_snapshot_id": fork_snapshot_id,
        "head_segment": head_segment,
        "provenance_rows": provenance_rows,
        "summary": get_worktree(repo_ctx, worktree_name),
    }


def restore_owned_head(
    ctx: RepoContext,
    name: str | None = None,
    *,
    dry_run: bool = False,
) -> dict[str, Any]:
    prepared = _prepare_worktree_restore_owned_head(ctx, name)
    repo_ctx: RepoContext = prepared["repo_ctx"]
    worktree_name = str(prepared["worktree_name"])
    worktree_ctx: RepoContext = prepared["worktree_ctx"]
    current_line_name = str(prepared["current_line_name"])
    current_head_snapshot_id = str(prepared["current_head_snapshot_id"])
    fork_snapshot_id = str(prepared["fork_snapshot_id"])
    expected_task_id = str(prepared["bound_task_id"])
    expected_change_id = prepared["bound_change_id"]
    expected_worktree_name = worktree_name
    restore_anchor_snapshot_id = fork_snapshot_id
    first_foreign_snapshot_id: str | None = None
    dropped_snapshots: list[dict[str, Any]] = []

    for snapshot_id in prepared["head_segment"]:
        ownership = _snapshot_matches_bound_slice(
            snapshot_id,
            provenance_rows=prepared["provenance_rows"],
            expected_task_id=expected_task_id,
            expected_change_id=expected_change_id,
            expected_worktree_name=expected_worktree_name,
        )
        if first_foreign_snapshot_id is not None:
            dropped_snapshots.append(
                {
                    **ownership,
                    "reason": ownership["reason"] or f"descends from foreign snapshot {first_foreign_snapshot_id}",
                    "foreign_root_snapshot_id": first_foreign_snapshot_id,
                }
            )
            continue
        if ownership["owned"]:
            restore_anchor_snapshot_id = snapshot_id
            continue
        first_foreign_snapshot_id = snapshot_id
        dropped_snapshots.append(
            {
                **ownership,
                "foreign_root_snapshot_id": snapshot_id,
            }
        )

    restore_details = {
        "worktree_name": worktree_name,
        "path": str(prepared["worktree_path"]),
        "task_id": expected_task_id,
        "change_id": expected_change_id,
        "line_name": current_line_name,
        "dry_run": dry_run,
        "foreign_detected": bool(dropped_snapshots),
        "fork_snapshot_id": fork_snapshot_id,
        "current_head_snapshot_id_before": current_head_snapshot_id,
        "materialized_snapshot_id_before": prepared["materialized_snapshot_id_before"],
        "restored_snapshot_id": restore_anchor_snapshot_id,
        "dropped_snapshots": dropped_snapshots,
        "noop": not dropped_snapshots,
    }
    if not dropped_snapshots:
        return {
            **prepared["summary"],
            "restore_owned_head": restore_details,
        }

    restore_data = local_content.restore_workspace(
        worktree_ctx,
        restore_anchor_snapshot_id,
        baseline_snapshot_id=current_head_snapshot_id,
        force=False,
        dry_run=dry_run,
    )
    restore_details["restore"] = restore_data

    if dry_run:
        return {
            **prepared["summary"],
            "restore_owned_head": restore_details,
        }

    set_line_head(worktree_ctx, current_line_name, restore_anchor_snapshot_id)
    _set_worktree_materialized_snapshot(worktree_ctx, restore_anchor_snapshot_id)
    restored_at = utc_now()
    _update_worktree_registration(
        repo_ctx,
        worktree_name,
        line_name=current_line_name,
        last_used_at=restored_at,
    )
    local_control.record_event(
        repo_ctx,
        "worktree.restored_owned_head",
        "worktree",
        worktree_name,
        {
            "name": worktree_name,
            "path": str(prepared["worktree_path"]),
            "task_id": expected_task_id,
            "change_id": expected_change_id,
            "line_name": current_line_name,
            "current_head_snapshot_id_before": current_head_snapshot_id,
            "restored_snapshot_id": restore_anchor_snapshot_id,
            "dropped_snapshot_ids": [row["snapshot_id"] for row in dropped_snapshots],
        },
    )
    summary = get_worktree(repo_ctx, worktree_name)
    summary["restore_owned_head"] = restore_details
    return summary


def sync_worktree(
    ctx: RepoContext,
    name: str | None = None,
    *,
    line_name: str | None = None,
    force: bool = False,
    dry_run: bool = False,
) -> dict[str, Any]:
    worktree_name = _resolve_worktree_name(ctx, name)
    payload = _load_worktree_metadata(ctx, worktree_name)
    worktree_path = Path(payload["path"]).expanduser().resolve()
    worktree_ctx = _maybe_discover_worktree(worktree_path)
    if worktree_ctx is None:
        raise ValueError(f"Worktree is missing or detached: {worktree_name}")
    target_line_name = line_name or current_line(worktree_ctx)
    target_line_row = local_content.get_line(worktree_ctx, target_line_name)
    target_snapshot_id = target_line_row.get("head_snapshot_id")
    baseline_snapshot_id = _worktree_materialized_snapshot_id(worktree_ctx)
    restore_data = local_content.restore_workspace(
        worktree_ctx,
        target_snapshot_id,
        baseline_snapshot_id=baseline_snapshot_id,
        force=force,
        dry_run=dry_run,
    )
    restore_data.update(
        {
            "repo_name": load_config(worktree_ctx).get("repo_name") or worktree_ctx.root.name,
            "current_line_before": current_line(worktree_ctx),
            "current_line": current_line(worktree_ctx),
            "line_name": target_line_name,
            "line_head_snapshot_id": target_snapshot_id,
            "materialized_snapshot_id_before": baseline_snapshot_id,
        }
    )
    if not dry_run:
        _set_worktree_materialized_snapshot(worktree_ctx, target_snapshot_id)
        if current_line(worktree_ctx) != target_line_name:
            _set_current_line(worktree_ctx, target_line_name)
            restore_data["current_line"] = target_line_name
        _update_worktree_registration(
            ctx,
            worktree_name,
            line_name=target_line_name,
            path=str(worktree_path),
            repo_root=str(worktree_ctx.repo_root),
        )
        local_control.record_event(
            ctx,
            "worktree.synced",
            "worktree",
            worktree_name,
            {
                "name": worktree_name,
                "path": str(worktree_path),
                "line_name": target_line_name,
                "write_count": restore_data["plan"]["write_count"],
                "remove_count": restore_data["plan"]["remove_count"],
                "force": force,
            },
        )
        summary = _touch_worktree_metadata(ctx, worktree_name)
    else:
        restore_data["current_line"] = target_line_name
        summary = get_worktree(ctx, worktree_name)
    summary["restore"] = restore_data
    return summary


def sync_all_worktrees(
    ctx: RepoContext,
    *,
    force: bool = False,
    dry_run: bool = False,
) -> dict[str, Any]:
    rows = list_worktrees(ctx)
    synced_rows: list[dict[str, Any]] = []
    skipped_rows: list[dict[str, Any]] = []
    error_rows: list[dict[str, Any]] = []
    for row in rows:
        name = row.get("name")
        if not name:
            continue
        status = row.get("workspace_status")
        if status in {"missing", "detached"}:
            skipped_rows.append(
                {
                    **row,
                    "reason": "stale_registration",
                }
            )
            continue
        try:
            synced_rows.append(sync_worktree(ctx, str(name), force=force, dry_run=dry_run))
        except (KeyError, ValueError, IsADirectoryError) as exc:
            error_rows.append(
                {
                    "name": row.get("name"),
                    "path": row.get("path"),
                    "current_line": row.get("current_line"),
                    "workspace_status": row.get("workspace_status"),
                    "error": str(exc),
                }
            )
    return {
        "dry_run": dry_run,
        "force": force,
        "target": "all",
        "requested_count": len(rows),
        "synced_count": len(synced_rows),
        "skipped_count": len(skipped_rows),
        "error_count": len(error_rows),
        "ok": len(error_rows) == 0,
        "synced_rows": synced_rows,
        "skipped_rows": skipped_rows,
        "error_rows": error_rows,
    }
