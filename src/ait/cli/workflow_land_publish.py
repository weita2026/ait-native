from __future__ import annotations

import re
from typing import Any

from ait_protocol.common import AuthorMode

from .. import local_control
from ..local_content import collect_snapshot_chain as local_collect_snapshot_chain
from ..remote_client import (
    create_change as remote_create_change,
    get_change as remote_get_change,
    get_remote_line,
    get_task as remote_get_task,
    publish_patchset as remote_publish_patchset,
)
from ..store import (
    RepoContext,
    bind_worktree as local_bind_worktree,
    create_line,
    create_snapshot,
    current_line,
    get_line,
    get_snapshot,
    get_worktree as local_get_worktree,
    load_config,
    rebase_worktree as local_rebase_worktree,
    workspace_status as local_workspace_status,
)
from .line_transport_helpers import (
    _push_line,
    _upload_snapshot_chain,
)
from .local_promotion_stale_base_guard import _bound_task_worktree_retarget_state
from .remote_repository_defaults import _remote_tuple, _sync_remote_repository_defaults
from .runtime_defaults import _effective_author_mode, _normalize_text_value
from .task_tracking_bindings import _task_worktree_repo_ctx
from .workflow_land_snapshot_replay import _patchset_publish_context


def _local_snapshot_chain_segment(
    ctx: RepoContext,
    *,
    base_snapshot_id: str,
    revision_snapshot_id: str,
    command_name: str,
) -> list[str]:
    resolved_base_snapshot_id = _normalize_text_value(base_snapshot_id)
    resolved_revision_snapshot_id = _normalize_text_value(revision_snapshot_id)
    if resolved_base_snapshot_id is None or resolved_revision_snapshot_id is None:
        raise ValueError(f"`{command_name}` requires both a base snapshot and a revision snapshot.")
    if resolved_base_snapshot_id == resolved_revision_snapshot_id:
        return []
    chain = local_collect_snapshot_chain(ctx, resolved_revision_snapshot_id)
    if resolved_base_snapshot_id not in chain:
        raise ValueError(
            f"Current line head `{resolved_revision_snapshot_id}` does not descend from selected base "
            f"`{resolved_base_snapshot_id}`. Rebase, restore, or retarget the bound worktree before "
            f"running `ait {command_name}`."
        )
    base_index = chain.index(resolved_base_snapshot_id)
    return chain[base_index + 1 :]


def _guard_patchset_revision_scope(
    ctx: RepoContext,
    *,
    base_snapshot_id: str,
    revision_snapshot_id: str,
    command_name: str,
    task_id: str | None = None,
    change_id: str | None = None,
) -> None:
    lineage_snapshot_ids = _local_snapshot_chain_segment(
        ctx,
        base_snapshot_id=base_snapshot_id,
        revision_snapshot_id=revision_snapshot_id,
        command_name=command_name,
    )
    if not ctx.is_worktree or not lineage_snapshot_ids:
        return
    try:
        worktree = local_get_worktree(ctx)
    except (KeyError, ValueError):
        return
    expected_task_id = _normalize_text_value(task_id) or _normalize_text_value(worktree.get("bound_task_id"))
    expected_change_id = _normalize_text_value(change_id) or _normalize_text_value(worktree.get("bound_change_id"))
    expected_worktree_name = _normalize_text_value(worktree.get("name"))
    if expected_task_id is None:
        return
    provenance_rows = {
        str(row.get("snapshot_id") or "").strip(): row
        for row in local_control.list_workflow_snapshot_provenance(ctx, snapshot_ids=lineage_snapshot_ids)
        if str(row.get("snapshot_id") or "").strip()
    }
    ownership_issues: list[str] = []
    for snapshot_id in lineage_snapshot_ids:
        provenance = provenance_rows.get(snapshot_id)
        if provenance is None:
            ownership_issues.append(f"{snapshot_id} (missing workflow provenance)")
            continue
        provenance_task_id = _normalize_text_value(provenance.get("task_id"))
        provenance_change_id = _normalize_text_value(provenance.get("change_id"))
        provenance_worktree_name = _normalize_text_value(provenance.get("worktree_name"))
        if provenance_task_id != expected_task_id:
            ownership_issues.append(f"{snapshot_id} (task {provenance_task_id or 'none'})")
            continue
        if expected_change_id is not None and provenance_change_id not in {None, expected_change_id}:
            ownership_issues.append(f"{snapshot_id} (change {provenance_change_id})")
            continue
        if expected_worktree_name is not None and provenance_worktree_name not in {None, expected_worktree_name}:
            ownership_issues.append(f"{snapshot_id} (worktree {provenance_worktree_name})")
    if ownership_issues:
        issue_sample = ", ".join(ownership_issues[:3])
        change_fragment = f" / change `{expected_change_id}`" if expected_change_id is not None else ""
        raise ValueError(
            f"Current line head `{revision_snapshot_id}` includes snapshot lineage that is not owned by "
            f"bound task `{expected_task_id}`{change_fragment} between base `{base_snapshot_id}` and the "
            f"current head: {issue_sample}. Restore or reopen the correct task worktree before running "
            f"`ait {command_name}`."
        )


def _workflow_publish_auto_rebase_if_needed(
    ctx: RepoContext,
    *,
    target_line: str,
) -> dict[str, Any] | None:
    if not ctx.is_worktree:
        return None
    try:
        worktree = local_get_worktree(ctx)
    except (KeyError, ValueError):
        return None
    retarget = worktree.get("retarget") if isinstance(worktree.get("retarget"), dict) else {}
    if not retarget:
        return None
    rebase_state = str(retarget.get("rebase_state") or "idle")
    if rebase_state == "conflicted":
        conflict_paths = ", ".join((retarget.get("rebase_conflict_paths") or [])[:5]) or "resolve conflicts first"
        raise ValueError(f"Current worktree has a conflicted rebase in progress: {conflict_paths}.")
    target_base_line = str(retarget.get("target_base_line") or target_line or "main")
    needs_retarget = bool(retarget.get("needs_retarget")) or target_base_line != target_line
    if not needs_retarget and str(retarget.get("forked_from_line") or target_line) == target_line:
        return None
    result = local_rebase_worktree(
        ctx,
        str(worktree.get("name") or "") or None,
        onto_line_name=target_line,
    )
    rebase = result.get("rebase") if isinstance(result.get("rebase"), dict) else {}
    if str(rebase.get("status") or "") == "conflicted":
        conflict_paths = ", ".join((rebase.get("conflict_paths") or [])[:5]) or "resolve conflicts first"
        raise ValueError(f"Automatic rebase onto `{target_line}` produced conflicts: {conflict_paths}.")
    return result


def _ensure_patchset_not_empty(
    *,
    change_id: str,
    base_snapshot_id: str | None,
    revision_snapshot_id: str | None,
    allow_empty: bool = False,
) -> None:
    if allow_empty:
        return
    if base_snapshot_id and revision_snapshot_id and base_snapshot_id == revision_snapshot_id:
        raise ValueError(
            f"Refusing to publish empty patchset for {change_id}: base and revision both point to {base_snapshot_id}. "
            "Use a review-base line or rerun with --allow-empty if this is intentional."
        )


def _sync_patchset_revision_snapshot(
    ctx: RepoContext,
    *,
    remote_name: str | None,
    line_name: str,
    revision_snapshot_id: str,
    base_line: str,
) -> dict[str, Any]:
    default_line = str(load_config(ctx).get("default_line") or "main")
    if line_name == base_line:
        return _upload_snapshot_chain(
            ctx,
            remote_name,
            revision_snapshot_id,
            line_name=line_name,
            reason="current line is the change base line",
        )
    if line_name == default_line:
        return _upload_snapshot_chain(
            ctx,
            remote_name,
            revision_snapshot_id,
            line_name=line_name,
            reason="current line is the default integration line",
        )
    push_result = _push_line(ctx, remote_name, line_name)
    push_result["line_updated"] = True
    push_result["line_update_skipped_reason"] = None
    return push_result


def _publish_patchset_from_current_line(
    ctx: RepoContext,
    *,
    change_id: str,
    summary: str,
    remote_name: str | None,
    author_mode: AuthorMode | None,
    allow_empty: bool = False,
) -> dict[str, Any]:
    line_name = current_line(ctx)
    line_row = get_line(ctx, line_name)
    revision_snapshot_id = line_row["head_snapshot_id"]
    if not revision_snapshot_id:
        raise KeyError(f"Current line {line_name} has no snapshot to publish")

    remote_row, repo_name = _sync_remote_repository_defaults(ctx, remote_name)
    change_info = remote_get_change(remote_row["url"], change_id, repo_name=repo_name)
    resolved_change_id = str(change_info.get("change_id") or change_id)
    worktree_retarget = _current_worktree_retarget_state(ctx, change_id)
    if isinstance(worktree_retarget, dict):
        if str(worktree_retarget.get("rebase_state") or "idle") == "conflicted":
            raise ValueError("Current worktree has a conflicted rebase in progress. Run `ait worktree rebase --continue` or `--abort` before publishing.")
        if bool(worktree_retarget.get("needs_retarget")):
            target_base_line = str(worktree_retarget.get("target_base_line") or change_info.get("base_line") or "main")
            raise ValueError(
                f"Current worktree is still based on `{worktree_retarget.get('fork_snapshot_id')}` while `{target_base_line}` moved to `{worktree_retarget.get('target_base_snapshot_id')}`. "
                f"Run `ait worktree rebase --onto {target_base_line}` before publishing."
            )
    base_line = change_info["base_line"]
    base_line_info = get_remote_line(remote_row["url"], repo_name, base_line)
    base_snapshot_id = base_line_info["head_snapshot_id"]
    if not base_snapshot_id:
        raise KeyError(f"Base line {base_line} has no head snapshot on remote")
    _guard_patchset_revision_scope(
        ctx,
        base_snapshot_id=base_snapshot_id,
        revision_snapshot_id=revision_snapshot_id,
        command_name="patchset publish",
        change_id=change_id,
    )
    _ensure_patchset_not_empty(
        change_id=change_id,
        base_snapshot_id=base_snapshot_id,
        revision_snapshot_id=revision_snapshot_id,
        allow_empty=allow_empty,
    )

    revision_sync = _sync_patchset_revision_snapshot(
        ctx,
        remote_name=remote_name,
        line_name=line_name,
        revision_snapshot_id=revision_snapshot_id,
        base_line=base_line,
    )
    resolved_author_mode = _effective_author_mode(ctx, author_mode)
    data = remote_publish_patchset(
        remote_row["url"],
        resolved_change_id,
        base_snapshot_id,
        revision_snapshot_id,
        summary,
        resolved_author_mode,
        repo_name=repo_name,
        exact_id=True,
    )
    data["publish_context"] = _patchset_publish_context(
        revision_sync=revision_sync,
        revision_snapshot_id=revision_snapshot_id,
        base_snapshot_id=base_snapshot_id,
    )
    return data


def _current_worktree_retarget_state(ctx: RepoContext, change_id: str) -> dict[str, Any] | None:
    return _bound_task_worktree_retarget_state(ctx, change_id=change_id)


def _workflow_refresh_patchset_for_land(
    ctx: RepoContext,
    *,
    change_id: str,
    summary: str,
    remote_name: str | None,
    author_mode: AuthorMode | None,
) -> dict[str, Any]:
    remote_row, repo_name = _sync_remote_repository_defaults(ctx, remote_name)
    change = remote_get_change(remote_row["url"], change_id, repo_name=repo_name)
    auto_rebase = _workflow_publish_auto_rebase_if_needed(
        ctx,
        target_line=str(change.get("base_line") or "main"),
    )
    result = _publish_patchset_from_current_line(
        ctx,
        change_id=change_id,
        summary=summary,
        remote_name=remote_name,
        author_mode=author_mode,
    )
    if auto_rebase is not None:
        result["auto_rebase"] = auto_rebase
    return result


def _workflow_publish_slug(value: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")
    return slug[:48].strip("-") or "review-base"


def _ensure_local_line_at_snapshot(ctx: RepoContext, line_name: str, snapshot_id: str) -> dict[str, Any]:
    try:
        return create_line(ctx, line_name, snapshot_id)
    except ValueError:
        existing = get_line(ctx, line_name)
        if existing.get("head_snapshot_id") != snapshot_id:
            raise
        payload = dict(existing)
        payload["existing"] = True
        return payload


def _workflow_publish_payload(
    ctx: RepoContext,
    *,
    task_id: str,
    summary: str,
    remote_name: str | None,
    base_snapshot_id: str | None,
    base_line_name: str | None,
    target_line: str | None,
    change_title: str | None,
    snapshot_message: str | None,
    author_mode: AuthorMode | None,
) -> dict[str, Any]:
    remote_row, repo_name = _remote_tuple(ctx, remote_name)
    task = remote_get_task(remote_row["url"], task_id, repo_name=repo_name)
    if task.get("repo_name") != repo_name:
        raise KeyError(f"Remote task {task_id} belongs to repository {task.get('repo_name')}, not {repo_name}")
    if task.get("status") != "active":
        raise ValueError(f"Task {task_id} is {task.get('status')} and cannot accept a publish change")

    workspace_before = local_workspace_status(ctx)
    selected_base_snapshot = _normalize_text_value(base_snapshot_id)
    snapshot: dict[str, Any] | None = None
    if not bool(workspace_before.get("clean")):
        snapshot = create_snapshot(ctx, snapshot_message or summary)

    revision_line_name = current_line(ctx)
    revision_line = get_line(ctx, revision_line_name)
    revision_snapshot_id = _normalize_text_value(revision_line.get("head_snapshot_id"))
    if revision_snapshot_id is None:
        raise KeyError(f"Current line {revision_line_name} has no snapshot to publish")

    resolved_target_line = _normalize_text_value(target_line)
    if resolved_target_line is not None:
        auto_rebase = _workflow_publish_auto_rebase_if_needed(ctx, target_line=resolved_target_line)
        revision_line_name = current_line(ctx)
        revision_line = get_line(ctx, revision_line_name)
        revision_snapshot_id = _normalize_text_value(revision_line.get("head_snapshot_id"))
        if revision_snapshot_id is None:
            raise KeyError(f"Current line {revision_line_name} has no snapshot to publish")
        target_line_info = get_remote_line(remote_row["url"], repo_name, resolved_target_line)
        selected_base_snapshot = _normalize_text_value(target_line_info.get("head_snapshot_id"))
        if selected_base_snapshot is None:
            raise KeyError(f"Target line {resolved_target_line} has no remote head snapshot")
        _guard_patchset_revision_scope(
            ctx,
            base_snapshot_id=selected_base_snapshot,
            revision_snapshot_id=revision_snapshot_id,
            command_name="workflow publish",
            task_id=task_id,
        )
        _ensure_patchset_not_empty(
            change_id=task_id,
            base_snapshot_id=selected_base_snapshot,
            revision_snapshot_id=revision_snapshot_id,
        )
        revision_sync = _sync_patchset_revision_snapshot(
            ctx,
            remote_name=remote_name,
            line_name=revision_line_name,
            revision_snapshot_id=revision_snapshot_id,
            base_line=resolved_target_line,
        )
        change = remote_create_change(
            remote_row["url"],
            repo_name,
            task_id,
            _normalize_text_value(change_title) or str(task.get("title") or task_id),
            resolved_target_line,
            str(task.get("risk_tier") or "medium"),
            fork_snapshot_id=selected_base_snapshot,
            forked_from_line=resolved_target_line,
        )
        if ctx.is_worktree:
            try:
                worktree = local_get_worktree(ctx)
            except (KeyError, ValueError):
                worktree = None
            if isinstance(worktree, dict):
                local_bind_worktree(
                    _task_worktree_repo_ctx(ctx),
                    str(worktree.get("name") or ""),
                    task_id=task_id,
                    change_id=str(change["change_id"]),
                    auto_created_for_task=bool(worktree.get("auto_created_for_task")),
                    target_base_line=resolved_target_line,
                )
        resolved_author_mode = _effective_author_mode(ctx, author_mode)
        patchset = remote_publish_patchset(
            remote_row["url"],
            str(change["change_id"]),
            selected_base_snapshot,
            revision_snapshot_id,
            summary,
            resolved_author_mode,
            repo_name=repo_name,
            exact_id=True,
        )
        patchset["publish_context"] = {
            "pushed_line": revision_sync.get("line") if revision_sync.get("line_updated", True) else None,
            "revision_line": revision_line_name,
            "line_updated": bool(revision_sync.get("line_updated", True)),
            "line_update_skipped_reason": revision_sync.get("line_update_skipped_reason"),
            "base_line": resolved_target_line,
            "base_snapshot_id": selected_base_snapshot,
            "revision_snapshot_id": revision_snapshot_id,
            "promotion_mode": "local_first_final_remote_land",
        }
        return {
            "task": task,
            "change": change,
            "patchset": patchset,
            "snapshot": snapshot,
            "base_line": {"line_name": resolved_target_line, "head_snapshot_id": selected_base_snapshot},
            "base_push": None,
            "revision_push": revision_sync,
            "auto_rebase": auto_rebase,
            "remote": remote_row.get("name"),
            "repo_name": repo_name,
            "promotion_mode": "local_first_final_remote_land",
        }

    if selected_base_snapshot is None:
        baseline_snapshot_id = _normalize_text_value(workspace_before.get("baseline_snapshot_id"))
        if baseline_snapshot_id is not None and baseline_snapshot_id != revision_snapshot_id:
            selected_base_snapshot = baseline_snapshot_id
        else:
            revision_snapshot = get_snapshot(ctx, revision_snapshot_id)
            selected_base_snapshot = _normalize_text_value(revision_snapshot.get("parent_snapshot_id"))
    if selected_base_snapshot is None:
        raise ValueError("Could not infer a review base snapshot. Pass --base-snapshot explicitly.")
    _guard_patchset_revision_scope(
        ctx,
        base_snapshot_id=selected_base_snapshot,
        revision_snapshot_id=revision_snapshot_id,
        command_name="workflow publish",
        task_id=task_id,
    )
    _ensure_patchset_not_empty(
        change_id=task_id,
        base_snapshot_id=selected_base_snapshot,
        revision_snapshot_id=revision_snapshot_id,
    )

    resolved_base_line = _normalize_text_value(base_line_name)
    if resolved_base_line is None:
        resolved_base_line = f"review-base/{_workflow_publish_slug(task_id)}-{selected_base_snapshot[-12:].lower()}"
    base_line = _ensure_local_line_at_snapshot(ctx, resolved_base_line, selected_base_snapshot)
    base_push = _push_line(ctx, remote_name, resolved_base_line)
    revision_sync = _sync_patchset_revision_snapshot(
        ctx,
        remote_name=remote_name,
        line_name=revision_line_name,
        revision_snapshot_id=revision_snapshot_id,
        base_line=resolved_base_line,
    )

    change = remote_create_change(
        remote_row["url"],
        repo_name,
        task_id,
        _normalize_text_value(change_title) or str(task.get("title") or task_id),
        resolved_base_line,
        str(task.get("risk_tier") or "medium"),
        fork_snapshot_id=selected_base_snapshot,
        forked_from_line=resolved_base_line,
    )
    resolved_author_mode = _effective_author_mode(ctx, author_mode)
    patchset = remote_publish_patchset(
        remote_row["url"],
        str(change["change_id"]),
        selected_base_snapshot,
        revision_snapshot_id,
        summary,
        resolved_author_mode,
        repo_name=repo_name,
        exact_id=True,
    )
    patchset["publish_context"] = {
        "pushed_line": revision_sync.get("line") if revision_sync.get("line_updated", True) else None,
        "revision_line": revision_line_name,
        "line_updated": bool(revision_sync.get("line_updated", True)),
        "line_update_skipped_reason": revision_sync.get("line_update_skipped_reason"),
        "base_line": resolved_base_line,
        "base_snapshot_id": selected_base_snapshot,
        "revision_snapshot_id": revision_snapshot_id,
    }
    return {
        "task": task,
        "change": change,
        "patchset": patchset,
        "snapshot": snapshot,
        "base_line": base_line,
        "base_push": base_push,
        "revision_push": revision_sync,
        "remote": remote_row.get("name"),
        "repo_name": repo_name,
    }
