from __future__ import annotations

import os
from typing import Any

from ..command_profiling import _command_profiling_mode
from ..store import RepoContext, current_line, get_line, list_lines, list_snapshots, load_config
from .runtime_defaults import (
    _detect_actor_identity,
    _detect_model_name,
    _effective_actor_identity,
    _effective_author_mode,
    _effective_model_name,
    _effective_reviewer_identity,
    _normalize_text_value,
)
from .task_tracking_bindings import _task_tracking_mode, _tracked_session_binding
from .workflow_mode_config import (
    _effective_id_namespace_prefix,
    _effective_plan_task_binding,
    _effective_task_dag,
    _effective_task_worktree,
    _effective_workflow_mode,
    _workflow_default_scope_summary,
)


def _local_auth_snapshot(ctx: RepoContext | None = None) -> dict[str, Any]:
    actor = _detect_actor_identity()
    if actor is None and ctx is not None:
        actor = _normalize_text_value(load_config(ctx).get("user_email")) or _normalize_text_value(load_config(ctx).get("user_name"))
    actor = actor or "anonymous"
    actor_type = os.environ.get("AIT_NATIVE_ACTOR_TYPE") or os.environ.get("AIT_ACTOR_TYPE") or "human"
    roles = sorted({item.strip() for item in (os.environ.get("AIT_NATIVE_ROLES") or os.environ.get("AIT_ROLES") or "").split(",") if item.strip()})
    repos = sorted({item.strip() for item in (os.environ.get("AIT_NATIVE_REPOS") or os.environ.get("AIT_REPOS") or "").split(",") if item.strip()})
    return {"identity": actor, "actor_type": actor_type, "claimed_roles": roles, "claimed_repos": repos}


def _storage_validation_view(data: dict[str, Any]) -> dict[str, Any]:
    validation = dict(data.get("validation_summary") or {})
    efficiency = dict(data.get("efficiency_summary") or {})
    optimization = dict(data.get("optimization_summary") or {})
    return {
        "state": validation.get("state"),
        "recommended_action": validation.get("recommended_action"),
        "next_actions": validation.get("next_actions", []),
        "reasons": validation.get("reasons", []),
        "issues": validation.get("issues", []),
        "needs_attention": bool(validation.get("needs_attention", False)),
        "has_pack_optimization": bool(validation.get("has_pack_optimization", False)),
        "has_delta_optimization": bool(validation.get("has_delta_optimization", False)),
        "tracked_blob_count": optimization.get("tracked_blob_count", 0),
        "packed_blob_count": data.get("packed_blob_count", 0),
        "packed_delta_blob_count": data.get("packed_delta_blob_count", 0),
        "pack_count": data.get("pack_count", 0),
        "storage_savings_ratio": efficiency.get("storage_savings_ratio", 0.0),
        "delta_pre_archive_savings_ratio": efficiency.get("delta_pre_archive_savings_ratio", 0.0),
    }


def _history_rows(ctx: RepoContext, limit: int | None = None, line_name: str | None = None) -> list[dict[str, Any]]:
    current = current_line(ctx)
    line_rows = list_lines(ctx)
    current_head_snapshot_id = None
    head_lines_by_snapshot: dict[str, list[str]] = {}
    for row in line_rows:
        head_snapshot_id = row.get("head_snapshot_id")
        if not head_snapshot_id:
            continue
        head_lines_by_snapshot.setdefault(head_snapshot_id, []).append(row["line_name"])
        if row["line_name"] == current:
            current_head_snapshot_id = head_snapshot_id

    snapshot_rows = list_snapshots(ctx)
    snapshot_by_id = {row["snapshot_id"]: row for row in snapshot_rows}
    if line_name is not None:
        line_row = get_line(ctx, line_name)
        selected_head_snapshot_id = line_row.get("head_snapshot_id")
        snapshots: list[dict[str, Any]] = []
        seen_snapshot_ids: set[str] = set()
        cur = selected_head_snapshot_id
        while cur:
            row = snapshot_by_id.get(cur)
            if row is None or cur in seen_snapshot_ids:
                break
            snapshots.append(row)
            seen_snapshot_ids.add(cur)
            cur = row.get("parent_snapshot_id")
            if limit is not None and len(snapshots) >= limit:
                break
    else:
        snapshots = snapshot_rows[:limit] if limit is not None else snapshot_rows

    out: list[dict[str, Any]] = []
    for row in snapshots:
        snapshot_id = row["snapshot_id"]
        head_lines = sorted(head_lines_by_snapshot.get(snapshot_id, []))
        is_current_head = snapshot_id == current_head_snapshot_id
        is_selected_line_head = bool(line_name and line_name in head_lines)
        if line_name is not None:
            graph = "@" if is_current_head and is_selected_line_head else "#" if is_selected_line_head else "|"
            marker = graph
        else:
            graph = "@" if is_current_head else "*" if head_lines else "o"
            marker = graph
        out.append(
            {
                **row,
                "head_lines": head_lines,
                "is_head": bool(head_lines),
                "is_current_head": is_current_head,
                "is_selected_line_head": is_selected_line_head,
                "marker": marker,
                "graph": graph,
            }
        )
    return out


def _config_summary(ctx: RepoContext) -> dict[str, Any]:
    cfg = load_config(ctx)
    web_inbox_defaults = cfg.get("web_inbox_defaults") if isinstance(cfg.get("web_inbox_defaults"), dict) else {}
    tracked_session = _tracked_session_binding(ctx)
    try:
        from ait_agent.runtime_backend import agent_runtime_summary

        agent_runtime = agent_runtime_summary(ctx.root)
    except Exception as exc:
        agent_runtime = {"error": str(exc)}
    return {
        "repo_root": str(ctx.repo_root),
        "workspace_root": str(ctx.root),
        "is_worktree": ctx.is_worktree,
        "worktree_name": _normalize_text_value(cfg.get("worktree_name")),
        "repo_name": cfg.get("repo_name") or ctx.root.name,
        "default_line": cfg.get("default_line"),
        "current_line": current_line(ctx),
        "default_remote": cfg.get("default_remote"),
        "policy_profile": cfg.get("policy_profile"),
        "default_author_mode": cfg.get("default_author_mode"),
        "default_model": cfg.get("default_model"),
        "detected_model": _detect_model_name(),
        "detected_actor": _detect_actor_identity(),
        "user_name": _normalize_text_value(cfg.get("user_name")),
        "user_email": _normalize_text_value(cfg.get("user_email")),
        "effective_actor": _effective_actor_identity(ctx),
        "effective_reviewer": _effective_reviewer_identity(ctx),
        "effective_author_mode": _effective_author_mode(ctx),
        "effective_model": _effective_model_name(ctx),
        "task_tracking": _task_tracking_mode(ctx),
        "command_profiling": _command_profiling_mode(ctx),
        "id_namespace_prefix": _effective_id_namespace_prefix(ctx),
        "workflow_mode": _effective_workflow_mode(ctx),
        "workflow_default_scope": _workflow_default_scope_summary(ctx),
        "agent_runtime": agent_runtime,
        "task_worktree": _effective_task_worktree(ctx),
        "task_dag": _effective_task_dag(ctx),
        "plan_task_binding": _effective_plan_task_binding(ctx),
        "tracked_session": None
        if tracked_session is None
        else {
            "task_id": tracked_session.get("task_id"),
            "session_id": tracked_session.get("session_id"),
            "scope": tracked_session.get("scope"),
            "remote_name": tracked_session.get("remote_name"),
        },
        "web_inbox_defaults": {
            "repo": _normalize_text_value(web_inbox_defaults.get("repo")),
            "author_class": _normalize_text_value(web_inbox_defaults.get("author_class")),
            "author_mode": _normalize_text_value(web_inbox_defaults.get("author_mode")),
            "tests": _normalize_text_value(web_inbox_defaults.get("tests")),
        },
    }
