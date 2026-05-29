from __future__ import annotations

from typing import Any

import click

from ..remote_client import RemoteError, list_sessions as remote_list_sessions
from ..repo_paths import RepoContext
from ..store import (
    current_line,
    load_config,
)
from ..store_local_sessions import (
    append_local_session_event,
    close_local_session,
    create_local_checkpoint,
    create_local_session,
    get_local_session,
    list_local_session_events,
    list_local_sessions,
    resume_local_session,
)
from ..task_statuses import task_close_session_status
from .remote_repository_defaults import _remote_tuple, _sync_remote_repository_defaults
from .remote_session_wrappers import (
    remote_append_session_event,
    remote_close_session,
    remote_create_session_checkpoint,
    remote_get_session,
    remote_list_session_events,
    remote_resume_session,
)
from .runtime_defaults import (
    _effective_actor_identity,
    _effective_author_mode,
    _effective_model_name,
    _normalize_text_value,
)
from .session_command_analysis import (
    AIT_TASK_CLOSE_COMMAND_PATHS,
    _analyze_session_ait_usage,
    _session_command_phase,
)
from .task_dag_node_bootstrap import remote_create_session
from .task_tracking_bindings import (
    _clear_tracked_session_binding_if_matches,
    _set_tracked_session_binding,
    _task_tracking_enabled,
    _tracked_session_binding,
)
from .task_worktree_resolution import _session_bound_worktree
from .workflow_boundary_sessions import (
    _apply_session_workspace_metadata,
    _remote_task_tracking_session_seed,
    remote_ensure_task_tracking_session,
)


def _task_tracking_session_title(task: dict[str, Any]) -> str:
    task_id = _normalize_text_value(task.get("task_id"))
    title = _normalize_text_value(task.get("title"))
    if task_id and title:
        return f"{task_id}: {title}"
    return title or task_id or "Tracked task session"


def _task_tracking_session_metadata(ctx: RepoContext, task: dict[str, Any]) -> dict[str, Any]:
    metadata: dict[str, Any] = {
        "author_mode": _effective_author_mode(ctx),
        "tracking_policy": "task_tracking",
    }
    task_id = _normalize_text_value(task.get("task_id"))
    task_intent = _normalize_text_value(task.get("intent"))
    if task_id:
        metadata["task_id"] = task_id
    if task_intent:
        metadata["objective"] = task_intent
    return metadata


def _auto_track_created_task(
    ctx: RepoContext,
    task: dict[str, Any],
    *,
    local: bool,
    remote_name: str | None = None,
    worktree: dict[str, Any] | None = None,
) -> dict[str, Any]:
    task_id = str(task["task_id"])
    bound_worktree = worktree or _session_bound_worktree(
        ctx,
        local=local,
        remote_name=remote_name,
        task_id=task_id,
    )
    resolved_line = (
        _normalize_text_value((bound_worktree or {}).get("current_line"))
        or _normalize_text_value((bound_worktree or {}).get("registered_line_name"))
        or current_line(ctx)
    )
    resolved_model = _effective_model_name(ctx)
    worktree_name = _normalize_text_value((bound_worktree or {}).get("name")) or _normalize_text_value(
        load_config(ctx).get("worktree_name")
    )
    metadata = _apply_session_workspace_metadata(
        ctx,
        _task_tracking_session_metadata(ctx, task),
        worktree=bound_worktree,
    )
    if local:
        session = create_local_session(
            ctx,
            "task_run",
            task_id=task_id,
            title=_task_tracking_session_title(task),
            line_name=resolved_line,
            worktree_name=worktree_name,
            model_name=resolved_model,
            metadata=metadata,
        )
        binding = _set_tracked_session_binding(
            ctx,
            task_id=task_id,
            session_id=session["session_id"],
            scope="local",
        )
    else:
        remote_row, repo_name = _sync_remote_repository_defaults(ctx, remote_name)
        session = remote_create_session(
            remote_row["url"],
            repo_name,
            "task_run",
            task_id=task_id,
            title=_task_tracking_session_title(task),
            line_name=resolved_line,
            worktree_name=worktree_name,
            model_name=resolved_model,
            metadata=metadata,
        )
        binding = _set_tracked_session_binding(
            ctx,
            task_id=task_id,
            session_id=session["session_id"],
            scope="remote",
            remote_name=str(remote_row.get("name") or remote_name or ""),
        )
    return {
        "mode": "on",
        "task_id": task_id,
        "session_id": session["session_id"],
        "session_scope": binding["scope"],
        "session_remote": binding.get("remote_name"),
        "session_status": session.get("status"),
        "worktree_name": worktree_name,
        "workspace_root": metadata.get("workspace_root"),
        "capture_mode": "config.task_tracking",
    }


def _maybe_attach_task_tracking(
    ctx: RepoContext,
    payload: dict[str, Any],
    *,
    local: bool,
    remote_name: str | None,
    worktree: dict[str, Any] | None = None,
) -> dict[str, Any]:
    if not _task_tracking_enabled(ctx):
        return payload
    existing_tracking = payload.get("tracking") if isinstance(payload.get("tracking"), dict) else None
    if not local and existing_tracking is not None:
        task_id = str(payload["task_id"])
        remote_row, _ = _sync_remote_repository_defaults(ctx, remote_name)
        session = remote_ensure_task_tracking_session(
            remote_row["url"],
            str(payload.get("repo_name") or ""),
            task_id,
            tracking_session=_remote_task_tracking_session_seed(
                ctx,
                title=str(payload.get("title") or ""),
                intent=str(payload.get("intent") or ""),
                remote_name=remote_name,
                worktree=worktree,
                source_surface="cli.task.attach_tracking",
            ),
        )
        binding = _set_tracked_session_binding(
            ctx,
            task_id=task_id,
            session_id=str(existing_tracking["session_id"]),
            scope="remote",
            remote_name=str(remote_row.get("name") or remote_name or ""),
        )
        tracked_payload = dict(payload)
        tracking = dict(existing_tracking)
        tracking.update(
            {
                "mode": "on",
                "task_id": task_id,
                "session_scope": binding["scope"],
                "session_remote": binding.get("remote_name"),
                "session_status": session.get("status") or tracking.get("session_status"),
                "capture_mode": "config.task_tracking",
                "worktree_name": _normalize_text_value(session.get("worktree_name"))
                or _normalize_text_value((worktree or {}).get("name"))
                or _normalize_text_value(tracking.get("worktree_name")),
                "workspace_root": _normalize_text_value((session.get("metadata") or {}).get("workspace_root"))
                or _normalize_text_value((worktree or {}).get("path"))
                or _normalize_text_value((worktree or {}).get("workspace_root"))
                or _normalize_text_value(tracking.get("workspace_root")),
            }
        )
        tracked_payload["tracking"] = tracking
        return tracked_payload
    try:
        tracking = _auto_track_created_task(
            ctx,
            payload,
            local=local,
            remote_name=remote_name,
            worktree=worktree,
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise click.ClickException(
            f"Task {payload['task_id']} was created, but automatic task tracking could not start a session: {exc}"
        ) from exc
    tracked_payload = dict(payload)
    tracked_payload["tracking"] = tracking
    return tracked_payload


_SESSION_STATUS_PRIORITY = {
    "active": 0,
    "paused": 1,
    "completed": 2,
    "canceled": 3,
}


def _latest_task_session_row(rows: list[dict[str, Any]], task_id: str) -> dict[str, Any] | None:
    candidates = [row for row in rows if _normalize_text_value(row.get("task_id")) == task_id]
    if not candidates:
        return None
    candidates.sort(
        key=lambda row: str(row.get("updated_at") or row.get("created_at") or row.get("session_id") or ""),
        reverse=True,
    )
    candidates.sort(key=lambda row: _SESSION_STATUS_PRIORITY.get(str(row.get("status") or ""), 99))
    return candidates[0]


def _resolve_task_review_session(
    ctx: RepoContext,
    task_id: str,
    *,
    local: bool,
    remote_name: str | None,
) -> dict[str, Any] | None:
    binding = _tracked_session_binding(ctx)
    if binding is not None and binding.get("task_id") == task_id:
        try:
            if binding["scope"] == "local":
                return {
                    "scope": "local",
                    "session_id": binding["session_id"],
                    "session": get_local_session(ctx, binding["session_id"]),
                    "remote_name": None,
                    "remote_row": None,
                }
            bound_remote_name = binding.get("remote_name") or remote_name
            remote_row, repo_name = _remote_tuple(ctx, bound_remote_name)
            return {
                "scope": "remote",
                "session_id": binding["session_id"],
                "session": remote_get_session(remote_row["url"], binding["session_id"], repo_name=repo_name),
                "remote_name": remote_row.get("name") or bound_remote_name,
                "remote_row": remote_row,
                "repo_name": repo_name,
            }
        except (KeyError, RemoteError, ValueError):
            pass
    if local:
        candidate = _latest_task_session_row(list_local_sessions(ctx), task_id)
        if candidate is None:
            return None
        return {
            "scope": "local",
            "session_id": candidate["session_id"],
            "session": candidate,
            "remote_name": None,
            "remote_row": None,
        }
    remote_row, repo_name = _remote_tuple(ctx, remote_name)
    candidate = _latest_task_session_row(remote_list_sessions(remote_row["url"], repo_name), task_id)
    if candidate is None:
        return None
    return {
        "scope": "remote",
        "session_id": candidate["session_id"],
        "session": candidate,
        "remote_name": remote_row.get("name") or remote_name,
        "remote_row": remote_row,
        "repo_name": repo_name,
    }


def _ensure_task_review_session_active(ctx: RepoContext, review_session: dict[str, Any]) -> dict[str, Any]:
    session = review_session["session"]
    status = str(session.get("status") or "")
    if status != "paused":
        return review_session
    if review_session["scope"] == "local":
        resumed = resume_local_session(ctx, review_session["session_id"], limit=1)
    else:
        resumed = remote_resume_session(
            review_session["remote_row"]["url"],
            review_session["session_id"],
            limit=1,
            repo_name=review_session.get("repo_name"),
        )
    review_session["session"] = resumed["session"]
    return review_session


def _list_task_review_session_events(ctx: RepoContext, review_session: dict[str, Any]) -> list[dict[str, Any]]:
    session = review_session["session"]
    limit = max(int(session.get("last_event_sequence") or 0), 1)
    if review_session["scope"] == "local":
        return list_local_session_events(ctx, review_session["session_id"], limit=limit)
    return remote_list_session_events(
        review_session["remote_row"]["url"],
        review_session["session_id"],
        limit=limit,
        repo_name=review_session.get("repo_name"),
    )


def _trim_current_task_close_event(events: list[dict[str, Any]], task_id: str) -> list[dict[str, Any]]:
    if not events:
        return events
    last = events[-1]
    if str(last.get("event_type") or "") != "tool.command":
        return events
    payload = last.get("payload") if isinstance(last.get("payload"), dict) else {}
    if _session_command_phase(payload) != "started":
        return events
    if _normalize_text_value(payload.get("command_path")) not in AIT_TASK_CLOSE_COMMAND_PATHS:
        return events
    if _normalize_text_value(payload.get("target")) != task_id:
        return events
    return events[:-1]


def _task_improvement_plan(analysis: dict[str, Any]) -> list[str]:
    plan: list[str] = []
    merge_opportunities = analysis.get("merge_opportunities") or []
    optimization_hints = analysis.get("optimization_hints") or []
    by_code = {str(row.get("code") or ""): row for row in optimization_hints}

    inventory_merge = next(
        (row for row in merge_opportunities if row.get("code") == "queue_summary_inventory_merge"),
        None,
    )
    if inventory_merge is not None:
        plan.append(
            f"Start workflow inventory turns with `{inventory_merge.get('suggested_command')}` instead of rebuilding the queue from separate task/change commands."
        )

    if "duplicate_inventory_reads" in by_code:
        plan.append(
            "Do not rerun the same queue, task-list, change-list, or task-audit read in one turn unless workflow state actually changed."
        )

    task_start_merge = next(
        (row for row in merge_opportunities if row.get("code") == "task_start_bootstrap_merge"),
        None,
    )
    if task_start_merge is not None:
        plan.append(
            f"Open a new task plus its first change with `{task_start_merge.get('suggested_command')}` instead of separate bootstrap commands."
        )

    task_audit_merge = next(
        (row for row in merge_opportunities if row.get("code") == "task_audit_read_merge"),
        None,
    )
    if task_audit_merge is not None:
        plan.append(
            f"Use `{task_audit_merge.get('suggested_command')}` when you need one task's readiness instead of separate task/show and change/list reads."
        )

    if "prefer_workflow_guide" in by_code:
        plan.append(
            "Start repeated help/discovery turns with `ait workflow guide inventory` or `ait workflow guide land` before reopening many narrower help screens."
        )

    if "prefer_workflow_land" in by_code:
        plan.append(
            "When one turn keeps hopping across patchset, attestation, review, policy, and land status for one change, start with `ait workflow land <change-id>`."
        )

    if any(row.get("code") == "dedupe_repeated_command" for row in merge_opportunities):
        plan.append("Do not rerun the same `ait` command unless repository state has actually changed.")

    if any(row.get("code") == "reuse_show_result" for row in merge_opportunities):
        plan.append("Reuse previously loaded task, change, or session detail instead of calling `show` on the same object again.")

    if "reduce_commands_per_turn" in by_code:
        plan.append("Keep each conversation turn to one summary command first, then drill down only where the summary shows a real gap.")

    if "promote_repeated_shell_workflow" in by_code:
        plan.append(
            "When the same shell-heavy Codex inspection pattern keeps recurring, capture it in a small Python helper or dedicated `ait` command."
        )

    if not plan:
        plan.append("Keep the current command plan; this session did not show avoidable `ait` command churn.")
    return plan


def _build_task_retrospective(
    task_id: str,
    session_id: str,
    analysis: dict[str, Any],
) -> tuple[dict[str, Any], list[str]]:
    merge_opportunities = analysis.get("merge_opportunities") or []
    turns = analysis.get("conversation_turns") or []
    avoidable_count = sum(int(row.get("avoidable_count") or 0) for row in merge_opportunities)
    heavy_turn_count = sum(1 for row in turns if int(row.get("ait_command_count") or 0) >= 4)
    dominant_paths = [
        str(row.get("command_path") or "")
        for row in (analysis.get("command_paths") or [])[:3]
        if row.get("command_path")
    ]
    retrospective = {
        "task_id": task_id,
        "session_id": session_id,
        "ait_command_count": int(analysis.get("ait_command_count") or 0),
        "distinct_command_paths": int(analysis.get("distinct_command_paths") or 0),
        "conversation_turn_count": len(turns),
        "merge_opportunity_count": len(merge_opportunities),
        "avoidable_command_count": avoidable_count,
        "repeated_command_run_count": len(analysis.get("repeated_command_runs") or []),
        "heavy_turn_count": heavy_turn_count,
        "dominant_command_paths": dominant_paths,
        "top_merge_opportunities": merge_opportunities[:3],
        "top_hints": (analysis.get("optimization_hints") or [])[:3],
    }
    return retrospective, _task_improvement_plan(analysis)


def _append_task_retrospective_event(
    ctx: RepoContext,
    review_session: dict[str, Any],
    payload: dict[str, Any],
) -> dict[str, Any]:
    if review_session["scope"] == "local":
        return append_local_session_event(
            ctx,
            review_session["session_id"],
            "task.retrospective",
            payload,
            actor_identity=_effective_actor_identity(ctx),
            actor_type="tool",
        )
    return remote_append_session_event(
        review_session["remote_row"]["url"],
        review_session["session_id"],
        "task.retrospective",
        payload,
        repo_name=review_session.get("repo_name"),
    )


def _create_task_retrospective_checkpoint(
    ctx: RepoContext,
    review_session: dict[str, Any],
    task_id: str,
    task_status: str,
    retrospective: dict[str, Any],
    improvement_plan: list[str],
) -> dict[str, Any]:
    summary = f"Task {task_status} retrospective for {task_id}"
    resume_payload = {
        "task_id": task_id,
        "task_status": task_status,
        "analysis_summary": {
            "ait_command_count": retrospective["ait_command_count"],
            "distinct_command_paths": retrospective["distinct_command_paths"],
            "merge_opportunity_count": retrospective["merge_opportunity_count"],
            "avoidable_command_count": retrospective["avoidable_command_count"],
        },
        "improvement_plan": improvement_plan,
    }
    if review_session["scope"] == "local":
        return create_local_checkpoint(
            ctx,
            review_session["session_id"],
            summary,
            resume_payload=resume_payload,
        )
    return remote_create_session_checkpoint(
        review_session["remote_row"]["url"],
        review_session["session_id"],
        summary,
        resume_payload=resume_payload,
        repo_name=review_session.get("repo_name"),
    )


def _close_task_review_session(ctx: RepoContext, review_session: dict[str, Any], status: str) -> dict[str, Any]:
    if review_session["scope"] == "local":
        return close_local_session(ctx, review_session["session_id"], status)
    return remote_close_session(
        review_session["remote_row"]["url"],
        review_session["session_id"],
        status=status,
        repo_name=review_session.get("repo_name"),
    )


def _finalize_task_close_tracking(
    ctx: RepoContext,
    review_session: dict[str, Any],
    *,
    task_id: str,
    task_status: str,
) -> dict[str, Any]:
    review_session = _ensure_task_review_session_active(ctx, review_session)
    events = _trim_current_task_close_event(_list_task_review_session_events(ctx, review_session), task_id)
    analysis = _analyze_session_ait_usage(
        review_session["session_id"],
        events,
        after_sequence=0,
        limit=max(len(events), 1),
    )
    retrospective, improvement_plan = _build_task_retrospective(task_id, review_session["session_id"], analysis)
    event_payload = {
        "task_id": task_id,
        "task_status": task_status,
        "session_id": review_session["session_id"],
        "analysis_summary": {
            "ait_command_count": retrospective["ait_command_count"],
            "distinct_command_paths": retrospective["distinct_command_paths"],
            "merge_opportunity_count": retrospective["merge_opportunity_count"],
            "avoidable_command_count": retrospective["avoidable_command_count"],
        },
        "retrospective": retrospective,
        "improvement_plan": improvement_plan,
    }

    checkpoint = None
    retrospective_event = None
    session = review_session["session"]
    session_status = str(session.get("status") or "")
    if session_status in {"active", "paused"}:
        retrospective_event = _append_task_retrospective_event(ctx, review_session, event_payload)
        checkpoint = _create_task_retrospective_checkpoint(
            ctx,
            review_session,
            task_id,
            task_status,
            retrospective,
            improvement_plan,
        )
        session_close_status = task_close_session_status(task_status)
        assert session_close_status is not None
        session = _close_task_review_session(
            ctx,
            review_session,
            session_close_status,
        )
        review_session["session"] = session

    _clear_tracked_session_binding_if_matches(
        ctx,
        task_id=task_id,
        session_id=review_session["session_id"],
    )
    return {
        "mode": "on",
        "task_id": task_id,
        "session_id": review_session["session_id"],
        "session_scope": review_session["scope"],
        "session_remote": review_session.get("remote_name"),
        "session_status": session.get("status"),
        "retrospective_event_sequence": None if retrospective_event is None else retrospective_event.get("sequence"),
        "checkpoint_id": None if checkpoint is None else checkpoint.get("checkpoint_id"),
        "retrospective": retrospective,
        "improvement_plan": improvement_plan,
    }
