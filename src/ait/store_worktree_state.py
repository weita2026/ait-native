from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from ait_protocol.common import normalize_optional_text, read_json, utc_now

from . import local_content, local_control
from .remote_client import RemoteError, get_change as remote_get_change, get_task as remote_get_task
from .repo_paths import RepoContext
from .store_repo_config import load_config
from .store_worktree_runtime import (
    DEFAULT_WORKTREE_CREATION_KIND,
    _coerce_datetime,
    _default_cleanup_policy_for_creation_kind,
    _normalize_older_than,
    _normalize_worktree_cleanup_policy,
    _normalize_worktree_creation_kind,
    get_remote,
)
from .task_statuses import (
    TASK_CLOSED_STATUSES,
    is_task_abandoned_status,
    is_task_later_promotion_excluded_status,
)

_CHANGE_CLOSED_STATUSES = frozenset({"landed", "archived"})
_PREFETCHED_WORKTREE_SUMMARY_WORKFLOW_ROWS: dict[
    RepoContext,
    dict[str, list[dict[str, Any]]],
] = {}


@dataclass
class _WorktreeSummarySharedState:
    repo_ctx: RepoContext
    active_root_worktree_name: str | None
    active_sessions_by_worktree: dict[str, list[dict[str, Any]]]
    tasks_by_id: dict[str, dict[str, Any]]
    changes_by_id: dict[str, dict[str, Any]]
    line_head_snapshot_ids: dict[str, str | None] = field(default_factory=dict)
    snapshot_chains: dict[str, list[str]] = field(default_factory=dict)


def _cache_worktree_summary_workflow_rows(
    ctx: RepoContext,
    *,
    active_sessions: list[dict[str, Any]],
    changes: list[dict[str, Any]],
) -> None:
    repo_ctx = _repo_worktree_ctx(ctx)
    _PREFETCHED_WORKTREE_SUMMARY_WORKFLOW_ROWS[repo_ctx] = {
        "active_sessions": list(active_sessions),
        "changes": list(changes),
    }


def _consume_worktree_summary_workflow_rows(ctx: RepoContext) -> dict[str, list[dict[str, Any]]] | None:
    repo_ctx = _repo_worktree_ctx(ctx)
    return _PREFETCHED_WORKTREE_SUMMARY_WORKFLOW_ROWS.pop(repo_ctx, None)


def _discard_worktree_summary_workflow_rows(ctx: RepoContext) -> None:
    repo_ctx = _repo_worktree_ctx(ctx)
    _PREFETCHED_WORKTREE_SUMMARY_WORKFLOW_ROWS.pop(repo_ctx, None)


def _normalize_worktree_name(name: str) -> str:
    value = str(name).strip()
    if not value:
        raise ValueError("Worktree name must not be empty.")
    if "/" in value or "\\" in value:
        raise ValueError("Worktree name must not contain path separators.")
    if value in {".", ".."}:
        raise ValueError("Worktree name must not be '.' or '..'.")
    return value


def _repo_worktree_ctx(ctx: RepoContext) -> RepoContext:
    return RepoContext.discover(ctx.repo_root) if ctx.is_worktree else ctx


def _worktree_metadata_with_defaults(payload: dict) -> dict:
    out = dict(payload)
    creation_kind_default = "task_auto_created" if out.get("auto_created_for_task") else DEFAULT_WORKTREE_CREATION_KIND
    try:
        creation_kind = _normalize_worktree_creation_kind(
            out.get("creation_kind"),
            default=creation_kind_default,
        )
    except ValueError:
        creation_kind = creation_kind_default
    assert creation_kind is not None
    cleanup_policy = _normalize_worktree_cleanup_policy(
        out.get("cleanup_policy"),
        default=_default_cleanup_policy_for_creation_kind(creation_kind),
    )
    assert cleanup_policy is not None
    out["creation_kind"] = creation_kind
    out["cleanup_policy"] = cleanup_policy
    out["last_used_at"] = normalize_optional_text(out.get("last_used_at")) or normalize_optional_text(out.get("created_at"))
    out["fork_snapshot_id"] = normalize_optional_text(out.get("fork_snapshot_id"))
    out["forked_from_line"] = normalize_optional_text(out.get("forked_from_line"))
    out["target_base_line"] = normalize_optional_text(out.get("target_base_line")) or out["forked_from_line"]
    out["last_retargeted_at"] = normalize_optional_text(out.get("last_retargeted_at"))
    out["rebase_started_at"] = normalize_optional_text(out.get("rebase_started_at"))
    out["rebase_original_head_snapshot_id"] = normalize_optional_text(out.get("rebase_original_head_snapshot_id"))
    out["rebase_onto_snapshot_id"] = normalize_optional_text(out.get("rebase_onto_snapshot_id"))
    rebase_state = normalize_optional_text(out.get("rebase_state")) or "idle"
    out["rebase_state"] = rebase_state if rebase_state in {"idle", "conflicted"} else "idle"
    conflict_paths = out.get("rebase_conflict_paths")
    if not isinstance(conflict_paths, list):
        conflict_paths = []
    out["rebase_conflict_paths"] = [str(item) for item in conflict_paths if str(item).strip()]
    return out


def _snapshot_chain(
    ctx: RepoContext,
    snapshot_id: str | None,
    *,
    snapshot_chain_cache: dict[str, list[str]] | None = None,
) -> list[str]:
    resolved_snapshot_id = normalize_optional_text(snapshot_id)
    if resolved_snapshot_id is None:
        return []
    if snapshot_chain_cache is not None and resolved_snapshot_id in snapshot_chain_cache:
        return snapshot_chain_cache[resolved_snapshot_id]
    chain = local_content.collect_snapshot_chain(ctx, resolved_snapshot_id)
    if snapshot_chain_cache is not None:
        snapshot_chain_cache[resolved_snapshot_id] = chain
    return chain


def _latest_common_snapshot(
    ctx: RepoContext,
    left_snapshot_id: str | None,
    right_snapshot_id: str | None,
    *,
    snapshot_chain_cache: dict[str, list[str]] | None = None,
) -> str | None:
    if left_snapshot_id is None or right_snapshot_id is None:
        return None
    left_chain = set(_snapshot_chain(ctx, left_snapshot_id, snapshot_chain_cache=snapshot_chain_cache))
    common: str | None = None
    for snapshot_id in _snapshot_chain(ctx, right_snapshot_id, snapshot_chain_cache=snapshot_chain_cache):
        if snapshot_id in left_chain:
            common = snapshot_id
    return common


def _snapshot_distance_if_ancestor(
    ctx: RepoContext,
    ancestor_snapshot_id: str | None,
    snapshot_id: str | None,
    *,
    snapshot_chain_cache: dict[str, list[str]] | None = None,
) -> int | None:
    if ancestor_snapshot_id is None or snapshot_id is None:
        return None
    chain = _snapshot_chain(ctx, snapshot_id, snapshot_chain_cache=snapshot_chain_cache)
    if ancestor_snapshot_id not in chain:
        return None
    return len(chain) - 1 - chain.index(ancestor_snapshot_id)


def _line_head_snapshot_id(
    ctx: RepoContext,
    line_name: str | None,
    *,
    line_head_cache: dict[str, str | None] | None = None,
) -> str | None:
    resolved_line_name = normalize_optional_text(line_name)
    if resolved_line_name is None:
        return None
    if line_head_cache is not None and resolved_line_name in line_head_cache:
        return line_head_cache[resolved_line_name]
    try:
        head_snapshot_id = normalize_optional_text(local_content.get_line(ctx, resolved_line_name).get("head_snapshot_id"))
    except KeyError:
        head_snapshot_id = None
    if line_head_cache is not None:
        line_head_cache[resolved_line_name] = head_snapshot_id
    return head_snapshot_id


def _effective_worktree_target_base_line(
    ctx: RepoContext,
    metadata: dict[str, Any],
    *,
    local_change: dict[str, Any] | None = None,
) -> str | None:
    target_base_line = normalize_optional_text(metadata.get("target_base_line"))
    if target_base_line is not None:
        return target_base_line
    change = local_change if local_change is not None else _local_change_for_worktree(ctx, metadata.get("bound_change_id"))
    return normalize_optional_text((change or {}).get("base_line")) or normalize_optional_text(metadata.get("forked_from_line"))


def _worktree_retarget_summary(
    ctx: RepoContext,
    metadata: dict[str, Any],
    *,
    current_line_name: str | None,
    head_snapshot_id: str | None,
    shared_state: _WorktreeSummarySharedState | None = None,
) -> dict[str, Any]:
    repo_ctx = shared_state.repo_ctx if shared_state is not None else _repo_worktree_ctx(ctx)
    bound_change_id = normalize_optional_text(metadata.get("bound_change_id"))
    local_change = (
        shared_state.changes_by_id.get(bound_change_id)
        if shared_state is not None and bound_change_id is not None
        else None
    )
    target_base_line = _effective_worktree_target_base_line(ctx, metadata, local_change=local_change)
    line_head_cache = shared_state.line_head_snapshot_ids if shared_state is not None else None
    snapshot_chain_cache = shared_state.snapshot_chains if shared_state is not None else None
    target_base_snapshot_id = _line_head_snapshot_id(repo_ctx, target_base_line, line_head_cache=line_head_cache)
    fork_snapshot_id = normalize_optional_text(metadata.get("fork_snapshot_id"))
    if fork_snapshot_id is None:
        fork_snapshot_id = _latest_common_snapshot(
            repo_ctx,
            head_snapshot_id,
            target_base_snapshot_id,
            snapshot_chain_cache=snapshot_chain_cache,
        )
    needs_retarget = bool(
        target_base_line
        and fork_snapshot_id
        and target_base_snapshot_id
        and fork_snapshot_id != target_base_snapshot_id
    )
    return {
        "target_base_line": target_base_line,
        "target_base_snapshot_id": target_base_snapshot_id,
        "fork_snapshot_id": fork_snapshot_id,
        "forked_from_line": normalize_optional_text(metadata.get("forked_from_line")),
        "line_name": current_line_name,
        "needs_retarget": needs_retarget,
        "feature_ahead_count": _snapshot_distance_if_ancestor(
            repo_ctx,
            fork_snapshot_id,
            head_snapshot_id,
            snapshot_chain_cache=snapshot_chain_cache,
        ),
        "base_behind_count": _snapshot_distance_if_ancestor(
            repo_ctx,
            fork_snapshot_id,
            target_base_snapshot_id,
            snapshot_chain_cache=snapshot_chain_cache,
        ),
        "rebase_state": str(metadata.get("rebase_state") or "idle"),
        "rebase_started_at": normalize_optional_text(metadata.get("rebase_started_at")),
        "rebase_original_head_snapshot_id": normalize_optional_text(metadata.get("rebase_original_head_snapshot_id")),
        "rebase_onto_snapshot_id": normalize_optional_text(metadata.get("rebase_onto_snapshot_id")),
        "rebase_conflict_paths": list(metadata.get("rebase_conflict_paths") or []),
        "last_retargeted_at": normalize_optional_text(metadata.get("last_retargeted_at")),
    }


def _active_root_worktree_binding_name(ctx: RepoContext) -> str | None:
    repo_ctx = _repo_worktree_ctx(ctx)
    payload = read_json(repo_ctx.config_path, default={}) or {}
    if not isinstance(payload, dict):
        return None
    worktree_name = normalize_optional_text(payload.get("worktree_name"))
    if worktree_name is None:
        return None
    try:
        return _normalize_worktree_name(worktree_name)
    except ValueError:
        return None


def _active_sessions_for_worktree(ctx: RepoContext, worktree_name: str) -> list[dict[str, Any]]:
    repo_ctx = _repo_worktree_ctx(ctx)
    return [
        row
        for row in local_control.list_workflow_sessions(repo_ctx, status="active")
        if normalize_optional_text(row.get("worktree_name")) == worktree_name
    ]


def _local_task_for_worktree(ctx: RepoContext, task_id: str | None) -> dict[str, Any] | None:
    resolved = normalize_optional_text(task_id)
    if resolved is None:
        return None
    try:
        return local_control.get_workflow_task(_repo_worktree_ctx(ctx), resolved)
    except KeyError:
        return None


def _local_change_for_worktree(ctx: RepoContext, change_id: str | None) -> dict[str, Any] | None:
    resolved = normalize_optional_text(change_id)
    if resolved is None:
        return None
    try:
        return local_control.get_workflow_change(_repo_worktree_ctx(ctx), resolved)
    except KeyError:
        return None


def _build_worktree_summary_shared_state(ctx: RepoContext) -> _WorktreeSummarySharedState:
    repo_ctx = _repo_worktree_ctx(ctx)
    prefetched_rows = _consume_worktree_summary_workflow_rows(repo_ctx)
    active_session_rows = (
        list(prefetched_rows.get("active_sessions") or [])
        if isinstance(prefetched_rows, dict)
        else list(local_control.list_workflow_sessions(repo_ctx, status="active"))
    )
    change_rows = (
        list(prefetched_rows.get("changes") or [])
        if isinstance(prefetched_rows, dict)
        else list(local_control.list_workflow_changes(repo_ctx))
    )
    active_sessions_by_worktree: dict[str, list[dict[str, Any]]] = {}
    for row in active_session_rows:
        worktree_name = normalize_optional_text(row.get("worktree_name"))
        if worktree_name is None:
            continue
        active_sessions_by_worktree.setdefault(worktree_name, []).append(row)
    tasks_by_id = {
        str(row.get("task_id")): row
        for row in local_control.list_workflow_tasks(repo_ctx)
        if normalize_optional_text(row.get("task_id")) is not None
    }
    changes_by_id = {
        str(row.get("change_id")): row
        for row in change_rows
        if normalize_optional_text(row.get("change_id")) is not None
    }
    return _WorktreeSummarySharedState(
        repo_ctx=repo_ctx,
        active_root_worktree_name=_active_root_worktree_binding_name(ctx),
        active_sessions_by_worktree=active_sessions_by_worktree,
        tasks_by_id=tasks_by_id,
        changes_by_id=changes_by_id,
    )


def _remote_workflow_binding(ctx: RepoContext, *, local_task: dict[str, Any] | None, local_change: dict[str, Any] | None) -> tuple[dict[str, Any], str] | None:
    repo_ctx = _repo_worktree_ctx(ctx)
    remote_name = (
        normalize_optional_text((local_task or {}).get("published_remote_name"))
        or normalize_optional_text((local_change or {}).get("published_remote_name"))
    )
    try:
        remote = get_remote(repo_ctx, remote_name)
    except KeyError:
        return None
    repo_name = (
        normalize_optional_text(remote.get("repo_name"))
        or normalize_optional_text(load_config(repo_ctx).get("repo_name"))
        or repo_ctx.root.name
    )
    return remote, repo_name


def _workflow_statuses_for_worktree(
    ctx: RepoContext,
    *,
    metadata: dict[str, Any],
    bound_task_id: str | None,
    bound_change_id: str | None,
    local_task: dict[str, Any] | None,
    local_change: dict[str, Any] | None,
) -> tuple[str | None, str | None]:
    task_status = normalize_optional_text((local_task or {}).get("status"))
    change_status = normalize_optional_text((local_change or {}).get("status"))
    task_status_hint = normalize_optional_text(metadata.get("bound_task_status"))
    change_status_hint = normalize_optional_text(metadata.get("bound_change_status"))
    should_fetch_task = bound_task_id is not None and (
        local_task is None or str((local_task or {}).get("publication_state") or "").strip() == "published"
    )
    should_fetch_change = bound_change_id is not None and (
        local_change is None or str((local_change or {}).get("publication_state") or "").strip() == "published"
    )
    remote_task_status: str | None = None
    remote_change_status: str | None = None
    if should_fetch_task or should_fetch_change:
        binding = _remote_workflow_binding(ctx, local_task=local_task, local_change=local_change)
        if binding is not None:
            remote, repo_name = binding
            base_url = str(remote.get("url") or "").strip()
            if base_url:
                if should_fetch_task:
                    task_ref = normalize_optional_text((local_task or {}).get("published_task_id")) or bound_task_id
                    if task_ref is not None:
                        try:
                            remote_task_status = normalize_optional_text(
                                remote_get_task(base_url, task_ref, repo_name=repo_name).get("status")
                            )
                        except (KeyError, RemoteError, ValueError):
                            remote_task_status = None
                if should_fetch_change:
                    change_ref = normalize_optional_text((local_change or {}).get("published_change_id")) or bound_change_id
                    if change_ref is not None:
                        try:
                            remote_change_status = normalize_optional_text(
                                remote_get_change(base_url, change_ref, repo_name=repo_name).get("status")
                            )
                        except (KeyError, RemoteError, ValueError):
                            remote_change_status = None
    return (
        remote_task_status or task_status or task_status_hint,
        remote_change_status or change_status or change_status_hint,
    )


def _worktree_cleanup_reason(creation_kind: str, cleanup_policy: str, *, older_than_label: str) -> str:
    if cleanup_policy == "after_idle":
        noun = "helper worktree" if creation_kind in {"bootstrap_helper", "land_helper"} else "worktree"
        return f"clean {noun} idle for {older_than_label}"
    if cleanup_policy == "after_task_complete":
        return "clean task-complete worktree eligible for explicit cleanup"
    if cleanup_policy == "after_remote_land":
        return "clean task-bound worktree eligible for auto-remove after remote land"
    return f"cleanup policy {cleanup_policy}"


def _worktree_cleanup_decision(
    ctx: RepoContext,
    payload: dict,
    *,
    status_label: str,
    is_current: bool,
    older_than: str | None = None,
    allow_manual_only: bool = False,
    workflow_status_resolver=None,
    shared_state: _WorktreeSummarySharedState | None = None,
) -> dict[str, Any]:
    metadata = _worktree_metadata_with_defaults(payload)
    creation_kind = str(metadata["creation_kind"])
    cleanup_policy = str(metadata["cleanup_policy"])
    last_used_at = normalize_optional_text(metadata.get("last_used_at"))
    older_than_delta, older_than_label = _normalize_older_than(older_than)

    bound_task_id = normalize_optional_text(metadata.get("bound_task_id"))
    bound_change_id = normalize_optional_text(metadata.get("bound_change_id"))
    binding_summary: dict[str, Any] = {
        "active_root_binding": False,
        "active_session_count": 0,
        "active_session_ids": [],
        "task_id": bound_task_id,
        "task_status": None,
        "change_id": bound_change_id,
        "change_status": None,
    }

    if status_label == "missing":
        return {
            "creation_kind": creation_kind,
            "cleanup_policy": cleanup_policy,
            "last_used_at": last_used_at,
            "cleanup_class": "stale",
            "cleanup_candidate": False,
            "cleanup_reason": "missing worktree path",
            "protected_reason": None,
            "manual_review_candidate": False,
            "manual_review_reason": None,
            "older_than": older_than_label,
            "binding_summary": binding_summary,
        }
    if status_label == "detached":
        return {
            "creation_kind": creation_kind,
            "cleanup_policy": cleanup_policy,
            "last_used_at": last_used_at,
            "cleanup_class": "stale",
            "cleanup_candidate": False,
            "cleanup_reason": "detached worktree layout",
            "protected_reason": None,
            "manual_review_candidate": False,
            "manual_review_reason": None,
            "older_than": older_than_label,
            "binding_summary": binding_summary,
        }

    worktree_name = _normalize_worktree_name(str(metadata.get("name") or payload.get("name") or ""))
    active_root_binding = (
        shared_state.active_root_worktree_name == worktree_name
        if shared_state is not None
        else _active_root_worktree_binding_name(ctx) == worktree_name
    )
    active_sessions = (
        list(shared_state.active_sessions_by_worktree.get(worktree_name, []))
        if shared_state is not None
        else _active_sessions_for_worktree(ctx, worktree_name)
    )
    local_task = (
        shared_state.tasks_by_id.get(bound_task_id)
        if shared_state is not None and bound_task_id is not None
        else _local_task_for_worktree(ctx, bound_task_id)
    )
    local_change = (
        shared_state.changes_by_id.get(bound_change_id)
        if shared_state is not None and bound_change_id is not None
        else _local_change_for_worktree(ctx, bound_change_id)
    )
    status_resolver = workflow_status_resolver or _workflow_statuses_for_worktree
    task_status, change_status = status_resolver(
        ctx,
        metadata=metadata,
        bound_task_id=bound_task_id,
        bound_change_id=bound_change_id,
        local_task=local_task,
        local_change=local_change,
    )
    binding_summary.update(
        {
            "active_root_binding": active_root_binding,
            "active_session_count": len(active_sessions),
            "active_session_ids": [str(row.get("session_id") or "") for row in active_sessions],
            "task_status": task_status,
            "change_status": change_status,
        }
    )
    task_closed = task_status in TASK_CLOSED_STATUSES
    change_closed = change_status in _CHANGE_CLOSED_STATUSES
    active_task_binding = bound_task_id is not None and not task_closed
    active_change_binding = bound_change_id is not None and not change_closed
    closed_task_cleanup_candidate = (
        bound_task_id is not None
        and (
            is_task_abandoned_status(task_status)
            or is_task_later_promotion_excluded_status(task_status)
        )
    )

    clean = payload.get("clean")
    protected_reason: str | None = None
    manual_review_candidate = False
    manual_review_reason: str | None = None
    force_remove_dirty = False
    if is_current:
        protected_reason = "current worktree"
    elif clean is None:
        protected_reason = "workspace status not verified"
    elif closed_task_cleanup_candidate:
        force_remove_dirty = clean is False
    elif active_root_binding:
        protected_reason = "active root-worktree binding target"
    elif not clean:
        protected_reason = "dirty worktree"
    elif active_task_binding:
        protected_reason = "active task-bound worktree"
    elif active_change_binding and not closed_task_cleanup_candidate:
        protected_reason = "active change-bound worktree"
    elif cleanup_policy == "manual_only":
        manual_review_candidate = True
        manual_review_reason = "clean manual worktree requires explicit manual-only cleanup opt-in"
        if not allow_manual_only:
            protected_reason = "cleanup policy manual_only"
    elif cleanup_policy == "never":
        protected_reason = "cleanup policy never"

    cleanup_class = "protected"
    cleanup_candidate = False
    cleanup_reason: str | None = None

    if protected_reason is None:
        idle_long_enough = (_coerce_datetime(utc_now()) - _coerce_datetime(last_used_at)) >= older_than_delta
        if closed_task_cleanup_candidate:
            cleanup_class = "safe_auto_remove"
            cleanup_candidate = True
            if is_task_later_promotion_excluded_status(task_status):
                cleanup_reason = "later-promotion-excluded task-bound worktree eligible for auto-remove"
            else:
                cleanup_reason = "abandoned task-bound worktree eligible for auto-remove"
        elif cleanup_policy == "after_idle":
            if idle_long_enough:
                cleanup_class = "safe_cleanup_candidate"
                cleanup_candidate = True
                cleanup_reason = _worktree_cleanup_reason(
                    creation_kind,
                    cleanup_policy,
                    older_than_label=older_than_label,
                )
            else:
                protected_reason = f"idle threshold {older_than_label} not reached"
        elif cleanup_policy == "after_task_complete":
            if bound_task_id is None:
                protected_reason = "cleanup policy after_task_complete requires task-bound worktree"
            elif task_closed and (bound_change_id is None or change_closed):
                cleanup_class = "safe_cleanup_candidate"
                cleanup_candidate = True
                cleanup_reason = _worktree_cleanup_reason(
                    creation_kind,
                    cleanup_policy,
                    older_than_label=older_than_label,
                )
            else:
                protected_reason = "task completion cleanup is not ready yet"
        elif cleanup_policy == "after_remote_land":
            if creation_kind != "task_auto_created" and not bool(metadata.get("auto_created_for_task")):
                protected_reason = "cleanup policy after_remote_land only applies to auto-created task worktrees"
            elif bound_task_id is not None and task_closed and (bound_change_id is None or change_closed):
                cleanup_class = "safe_auto_remove"
                cleanup_candidate = True
                cleanup_reason = _worktree_cleanup_reason(
                    creation_kind,
                    cleanup_policy,
                    older_than_label=older_than_label,
                )
            else:
                protected_reason = "waiting for remote land cleanup event"
        elif cleanup_policy == "manual_only" and allow_manual_only:
            cleanup_class = "safe_cleanup_candidate"
            cleanup_candidate = True
            cleanup_reason = "clean manual worktree selected for explicit manual-only cleanup"
            protected_reason = None
        else:
            protected_reason = f"cleanup policy {cleanup_policy} is not a candidate path"

    return {
        "creation_kind": creation_kind,
        "cleanup_policy": cleanup_policy,
        "last_used_at": last_used_at,
        "cleanup_class": cleanup_class,
        "cleanup_candidate": cleanup_candidate,
        "cleanup_reason": cleanup_reason,
        "protected_reason": protected_reason,
        "manual_review_candidate": manual_review_candidate,
        "manual_review_reason": manual_review_reason,
        "force_remove_dirty": force_remove_dirty,
        "older_than": older_than_label,
        "binding_summary": binding_summary,
    }
