from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence

from ait.cli.workflow_mode_config import _effective_workflow_mode as _shared_effective_workflow_mode
from ait_agent.envelope import build_transport_reply_envelope
from ait_agent.local_runtime_seam import (
    RepoContext,
    append_local_session_event,
    collect_snapshot_chain,
    create_local_checkpoint,
    create_local_session,
    current_line,
    get_line,
    get_local_change,
    get_local_session,
    get_local_task,
    get_remote,
    infer_workflow_context,
    list_local_changes,
    list_local_checkpoints,
    list_local_session_events,
    list_local_tasks,
    load_repo_config,
)
from ait_agent.telegram.runtime import utc_now_iso
from ait_chat.session_reply import ReplyGenerationError, generate_session_reply, load_reply_generation_config, payload_text
from ait_protocol.common import normalize_optional_text
from ait_agent.server_runtime_seam import get_worktree as local_get_worktree, resolve_bound_repo_root

WORKFLOW_SCOPE_LOCAL = "local"
WORKFLOW_SCOPE_REMOTE = "remote"
WORKFLOW_MODE_SOLO_LOCAL = "solo_local"
WORKFLOW_MODE_SOLO_REMOTE = "solo_remote"
WORKFLOW_MODE_TEAM_REMOTE = "team_remote"
_VALID_WORKFLOW_MODES = {
    WORKFLOW_MODE_SOLO_LOCAL,
    WORKFLOW_MODE_SOLO_REMOTE,
    WORKFLOW_MODE_TEAM_REMOTE,
}
_LOCAL_TASK_QUEUE_REVIEWABLE_STATES = {"review", "gated", "approved", "landable"}
_LOCAL_TASK_QUEUE_READY_TO_LAND_STATES = {"approved", "landable"}


class AgentRuntimeConfigError(ValueError):
    pass


@dataclass(frozen=True)
class AgentRuntimeTarget:
    mode: str
    workflow_mode: str
    repo_root: Path
    repo_name: str
    remote_name: str | None = None
    server_url: str | None = None


def _normalize_base_url(value: str | None) -> str | None:
    text = str(value or "").strip()
    if not text:
        return None
    return text.rstrip("/")


def effective_agent_workflow_mode(ctx: RepoContext) -> str:
    configured_mode = normalize_optional_text(load_repo_config(ctx).get("workflow_mode"))
    if configured_mode in _VALID_WORKFLOW_MODES:
        return configured_mode
    workflow_mode = normalize_optional_text(_shared_effective_workflow_mode(ctx).get("value"))
    if workflow_mode in _VALID_WORKFLOW_MODES:
        return workflow_mode
    raise AgentRuntimeConfigError(
        "ait-agent requires a repo workflow preset. Set `ait config set --workflow-mode "
        "solo_local|solo_remote|team_remote` before starting agent workers."
    )


def resolve_agent_runtime_target(repo_root: Path | None = None) -> AgentRuntimeTarget:
    resolved_root = (repo_root or Path(os.environ.get("AIT_REPO_ROOT") or Path.cwd())).expanduser()
    ctx = RepoContext.discover(resolved_root)
    config = load_repo_config(ctx)
    repo_name = str(config.get("repo_name") or ctx.root.name).strip() or ctx.root.name
    workflow_mode = effective_agent_workflow_mode(ctx)
    if workflow_mode == WORKFLOW_MODE_SOLO_LOCAL:
        return AgentRuntimeTarget(
            mode=WORKFLOW_SCOPE_LOCAL,
            workflow_mode=workflow_mode,
            repo_root=ctx.root.resolve(),
            repo_name=repo_name,
        )
    remote_row = get_remote(ctx)
    remote_name = str(remote_row.get("name") or config.get("default_remote") or "").strip() or None
    server_url = _normalize_base_url(str(remote_row.get("url") or ""))
    if server_url is None:
        raise AgentRuntimeConfigError("The default ait remote is missing a server URL.")
    return AgentRuntimeTarget(
        mode=WORKFLOW_SCOPE_REMOTE,
        workflow_mode=workflow_mode,
        repo_root=ctx.root.resolve(),
        repo_name=repo_name,
        remote_name=remote_name,
        server_url=server_url,
    )


def agent_runtime_summary(repo_root: Path | None = None) -> dict[str, Any]:
    target = resolve_agent_runtime_target(repo_root)
    return {
        "mode": target.mode,
        "workflow_mode": target.workflow_mode,
        "repo_root": str(target.repo_root),
        "repo_name": target.repo_name,
        "remote_name": target.remote_name,
        "server_url": target.server_url,
    }


def _runtime_repo_ctx(repo_root: Path) -> RepoContext:
    return RepoContext.discover(repo_root)


def _reply_generation_repo_root(*, repo_root: Path, session: Mapping[str, Any] | None = None) -> Path:
    metadata = session.get("metadata") if isinstance(session, Mapping) and isinstance(session.get("metadata"), dict) else {}
    resolved_repo_root = resolve_bound_repo_root(
        str((session or {}).get("repo_name") or "").strip() or repo_root.name,
        preferred_workspace_root=(metadata or {}).get("workspace_root"),
        preferred_repo_root=(metadata or {}).get("repo_root"),
        fallback_root=repo_root,
    )
    worktree_name = str((session or {}).get("worktree_name") or "").strip()
    if not worktree_name:
        return resolved_repo_root
    try:
        repo_ctx = _runtime_repo_ctx(resolved_repo_root)
        worktree = local_get_worktree(repo_ctx, worktree_name)
        worktree_path = str(worktree.get("path") or "").strip()
        if not worktree_path:
            return resolved_repo_root
        return RepoContext.discover(Path(worktree_path).expanduser()).root.resolve()
    except (FileNotFoundError, KeyError, ValueError):
        return resolved_repo_root


def _reply_generation_config(*, repo_root: Path, repo_name: str, session: Mapping[str, Any] | None = None):
    resolved_repo_root = _reply_generation_repo_root(repo_root=repo_root, session=session)
    reply_env = dict(os.environ)
    reply_env.pop("AIT_REPO_NAME", None)
    reply_env.pop("AIT_TELEGRAM_REPO_NAME", None)
    reply_env.pop("AIT_DISCORD_REPO_NAME", None)
    reply_env.pop("AIT_SLACK_REPO_NAME", None)
    reply_env.pop("AIT_LINE_REPO_NAME", None)
    reply_env.pop("AIT_CHAT_ENV_PATH", None)
    return load_reply_generation_config(
        repo_name=repo_name,
        repo_root=resolved_repo_root,
        env=reply_env,
    )


def _latest_local_checkpoint(repo_ctx: RepoContext, session: Mapping[str, Any]) -> dict[str, Any] | None:
    checkpoint_id = str(session.get("head_checkpoint_id") or "").strip()
    if checkpoint_id:
        checkpoints = list_local_checkpoints(repo_ctx, str(session.get("session_id") or ""))
        for checkpoint in checkpoints:
            if str(checkpoint.get("checkpoint_id") or "") == checkpoint_id:
                return checkpoint
    checkpoints = list_local_checkpoints(repo_ctx, str(session.get("session_id") or ""))
    return checkpoints[0] if checkpoints else None


def _reply_generation_events(
    repo_ctx: RepoContext,
    session_id: str,
    *,
    user_event: Mapping[str, Any],
    checkpoint_before_reply: Mapping[str, Any] | None,
    reply_config: Any,
) -> list[dict[str, Any]]:
    context_window = max(int(reply_config.history_limit or 0) * 4, 40)
    if checkpoint_before_reply is not None:
        checkpoint_sequence = int(checkpoint_before_reply.get("based_on_sequence") or 0)
        after_sequence = max(checkpoint_sequence, int(user_event.get("sequence") or 0) - context_window)
    else:
        after_sequence = max(int(user_event.get("sequence") or 0) - context_window, 0)
    return list_local_session_events(repo_ctx, session_id, after_sequence=after_sequence, limit=context_window)


def _render_turn_analysis_footer(turn_analysis: Mapping[str, Any] | None) -> str:
    if not isinstance(turn_analysis, Mapping):
        return ""
    command_count = int(turn_analysis.get("command_count") or 0)
    optimization_summary = str(turn_analysis.get("optimization_summary") or "").strip()
    if command_count <= 0 and not optimization_summary:
        return ""
    parts = [f"ran {command_count} commands"]
    if optimization_summary:
        parts.append(optimization_summary)
    return "[turn analysis] " + " · ".join(parts)


def _reply_text_with_turn_analysis(
    text: str,
    *,
    turn_analysis: Mapping[str, Any] | None,
    append_turn_analysis: bool,
) -> str:
    reply_text = str(text or "").strip()
    if not append_turn_analysis:
        return reply_text
    footer = _render_turn_analysis_footer(turn_analysis)
    if not footer:
        return reply_text
    if not reply_text:
        return footer
    return f"{reply_text}\n\n{footer}"


def _compact_dag_worker_live_turn_guard(session: Mapping[str, Any] | None) -> str | None:
    if not isinstance(session, Mapping):
        return None
    metadata = session.get("metadata") if isinstance(session.get("metadata"), dict) else {}
    if str(metadata.get("session_policy") or "").strip() != "task_dag_compact_packet_worker":
        return None
    session_id = str(session.get("session_id") or "").strip() or "<session-id>"
    return (
        f"Session {session_id} is a compact DAG worker session. Compact worker replies are generated "
        "locally and live-turn routes are disabled for this session."
    )


def _build_generic_transport_reply(
    *,
    transport_envelope: Mapping[str, Any],
    assistant_text: str,
    attachments: Sequence[Mapping[str, Any]] = (),
    session_id: str,
    surface: str,
) -> tuple[str, dict[str, Any] | None]:
    transport_name = str(transport_envelope.get("transport") or surface or "transport").strip() or "transport"
    channel = transport_envelope.get("channel") if isinstance(transport_envelope.get("channel"), Mapping) else {}
    message = transport_envelope.get("message") if isinstance(transport_envelope.get("message"), Mapping) else {}
    delivered_via = f"{transport_name}_live"
    reply_envelope = build_transport_reply_envelope(
        transport=transport_name,
        channel_id=channel.get("channel_id") or session_id,
        channel_title=channel.get("channel_title"),
        channel_kind=channel.get("channel_kind"),
        thread_id=channel.get("thread_id"),
        text=assistant_text,
        attachments=attachments,
        reply_to_event_id=str(transport_envelope.get("event_id") or "").strip() or None,
        reply_to_message_id=message.get("message_id"),
        reply_to_message_ids=message.get("message_ids"),
        metadata={"delivered_via": delivered_via},
    )
    return delivered_via, reply_envelope


def _local_task_queue_next_action(workflow_state: str, focus_change: Mapping[str, Any] | None) -> dict[str, Any]:
    if workflow_state == "planning":
        return {"code": "start_change", "label": "Start first change", "detail": "No linked changes exist yet."}
    if workflow_state == "attention_required":
        change_id = str((focus_change or {}).get("change_id") or "").strip()
        detail = str((focus_change or {}).get("reason") or "A linked change needs attention.").strip()
        return {
            "code": "inspect_change",
            "label": "Inspect change",
            "detail": detail,
            **({"target_change_id": change_id} if change_id else {}),
        }
    if workflow_state == "ready_to_land":
        change_id = str((focus_change or {}).get("change_id") or "").strip()
        return {
            "code": "land_change",
            "label": "Land change",
            "detail": "A linked change looks ready to land.",
            **({"target_change_id": change_id} if change_id else {}),
        }
    if workflow_state == "ready_to_complete":
        return {"code": "complete_task", "label": "Complete task", "detail": "All linked changes are landed."}
    if workflow_state == "in_review":
        return {"code": "inspect_review", "label": "Inspect review", "detail": "At least one linked change is in review."}
    return {"code": "continue_change", "label": "Continue change", "detail": "Linked work is still in progress."}


def _local_task_queue_entry(task: Mapping[str, Any], task_changes: list[dict[str, Any]]) -> dict[str, Any]:
    task_changes = sorted(task_changes, key=lambda item: str(item.get("updated_at") or item.get("created_at") or ""), reverse=True)
    total_changes = len(task_changes)
    open_changes = sum(1 for row in task_changes if str(row.get("status") or "") not in {"landed", "archived"})
    landed_changes = sum(1 for row in task_changes if str(row.get("status") or "") == "landed")
    reviewable_changes = sum(1 for row in task_changes if str(row.get("status") or "") in _LOCAL_TASK_QUEUE_REVIEWABLE_STATES)
    ready_to_land_changes = [row for row in task_changes if str(row.get("status") or "") in _LOCAL_TASK_QUEUE_READY_TO_LAND_STATES]
    blocked_changes = [row for row in task_changes if str(row.get("status") or "") == "blocked"]
    focus_change = blocked_changes[0] if blocked_changes else (ready_to_land_changes[0] if ready_to_land_changes else (task_changes[0] if task_changes else None))

    task_status = str(task.get("status") or "")
    if task_status == "completed":
        workflow_state = "completed"
        workflow_reason = "Task is already completed."
    elif task_status == "canceled":
        workflow_state = "canceled"
        workflow_reason = "Task is already canceled."
    elif total_changes == 0:
        workflow_state = "planning"
        workflow_reason = "No linked changes exist yet."
    elif blocked_changes:
        workflow_state = "attention_required"
        workflow_reason = f"Change {blocked_changes[0].get('change_id')} is blocked."
    elif ready_to_land_changes:
        workflow_state = "ready_to_land"
        workflow_reason = f"{len(ready_to_land_changes)} linked change(s) can land now."
    elif open_changes == 0 and landed_changes > 0:
        workflow_state = "ready_to_complete"
        workflow_reason = "All linked changes are landed; the task can complete."
    elif reviewable_changes > 0:
        workflow_state = "in_review"
        workflow_reason = f"{reviewable_changes} linked change(s) are in review."
    else:
        workflow_state = "in_progress"
        workflow_reason = f"{open_changes} linked change(s) are still in progress."

    next_action = _local_task_queue_next_action(workflow_state, focus_change)
    updated_at = max(
        [str(task.get("created_at") or ""), *[str(change.get("updated_at") or change.get("created_at") or "") for change in task_changes]],
        default=str(task.get("created_at") or ""),
    )
    return {
        "task": dict(task),
        "workflow": {"state": workflow_state, "reason": workflow_reason},
        "changes": {
            "total": total_changes,
            "open": open_changes,
            "reviewable": reviewable_changes,
            "landed": landed_changes,
            "patchsets": 0,
        },
        "attention": {
            "blocking_reviews": len(blocked_changes),
            "missing_attestation": 0,
            "tests_pending": 0,
            "stale_base": 0,
            "policy_pending": 0,
            "ready_to_land": len(ready_to_land_changes),
        },
        "focus_change": dict(focus_change) if isinstance(focus_change, dict) else None,
        "next_action": next_action,
        "updated_at": updated_at,
    }


def _local_task_queue_payload(repo_ctx: RepoContext, repo_name: str) -> dict[str, Any]:
    tasks = [row for row in list_local_tasks(repo_ctx) if str(row.get("repo_name") or "") == repo_name and str(row.get("status") or "") == "active"]
    changes = [row for row in list_local_changes(repo_ctx) if str(row.get("repo_name") or "") == repo_name]
    changes_by_task: dict[str, list[dict[str, Any]]] = {}
    for change in changes:
        changes_by_task.setdefault(str(change.get("task_id") or ""), []).append(change)
    items = [_local_task_queue_entry(task, changes_by_task.get(str(task.get("task_id") or ""), [])) for task in tasks]
    items.sort(key=lambda item: str(item.get("updated_at") or ""), reverse=True)
    state_priority = {
        "attention_required": 0,
        "ready_to_land": 1,
        "ready_to_complete": 2,
        "in_review": 3,
        "in_progress": 4,
        "planning": 5,
    }
    items.sort(key=lambda item: state_priority.get(str((item.get("workflow") or {}).get("state") or ""), 99))
    return {
        "items": items,
        "count": len(items),
        "filters": {"repo_name": repo_name, "status": "active"},
        "summary": {
            "active": len(items),
            "completed": 0,
            "canceled": 0,
            "attention_required": sum(1 for item in items if str((item.get("workflow") or {}).get("state") or "") == "attention_required"),
            "ready_to_land": sum(1 for item in items if str((item.get("workflow") or {}).get("state") or "") == "ready_to_land"),
            "ready_to_complete": sum(1 for item in items if str((item.get("workflow") or {}).get("state") or "") == "ready_to_complete"),
        },
    }


def _local_change_detail(repo_ctx: RepoContext, change_id: str) -> dict[str, Any]:
    change = get_local_change(repo_ctx, change_id)
    task = get_local_task(repo_ctx, str(change.get("task_id") or ""))
    review_blocking = 1 if str(change.get("status") or "") == "blocked" else 0
    return {
        "change": change,
        "task": task,
        "current_patchset": {},
        "policy_summary": {"decision": "pending"},
        "review_summary": {"approvals": 0, "blocking": review_blocking, "comments": 0},
        "freshness": {"base_is_fresh": True, "current_base_head": None},
    }


def _local_task_detail(repo_ctx: RepoContext, task_id: str) -> dict[str, Any]:
    task = get_local_task(repo_ctx, task_id)
    changes = [row for row in list_local_changes(repo_ctx) if str(row.get("task_id") or "") == task_id]
    queue_entry = _local_task_queue_entry(task, changes)
    return {
        "task": task,
        "workflow": queue_entry["workflow"],
        "changes": changes,
        "next_action": queue_entry["next_action"],
    }


def _local_task_audit(repo_ctx: RepoContext, task_id: str, *, target_line: str = "main") -> dict[str, Any]:
    task = get_local_task(repo_ctx, task_id)
    changes = [row for row in list_local_changes(repo_ctx) if str(row.get("task_id") or "") == task_id]
    queue_entry = _local_task_queue_entry(task, changes)
    target = get_line(repo_ctx, target_line)
    target_ancestry = set(collect_snapshot_chain(repo_ctx, target.get("head_snapshot_id")))
    change_rows: list[dict[str, Any]] = []
    effective_on_target_count = 0
    landed_change_count = 0
    for change in changes:
        status = str(change.get("status") or "")
        if status == "landed":
            target_state = "landed_on_target"
            effective_on_target = True
            landed_change_count += 1
            effective_on_target_count += 1
        elif status == "archived":
            target_state = "archived"
            effective_on_target = False
        else:
            target_state = "not_on_target"
            effective_on_target = False
        change_rows.append(
            {
                "change": change,
                "current_patchset": {},
                "selected_patchset": {},
                "display_patchset": {},
                "landing_summary": None,
                "effective_on_target": effective_on_target,
                "stale_workflow_record": False,
                "target_state": target_state,
                "target_reason": "",
                "preferred_line": {
                    "line_name": current_line(repo_ctx),
                    "head_snapshot_id": target.get("head_snapshot_id"),
                },
            }
        )
    open_change_count = sum(1 for change in changes if str(change.get("status") or "") not in {"landed", "archived"})
    if str(task.get("status") or "") == "completed":
        verdict = "completed"
        recommended = {"code": "done", "label": "Done", "detail": "Task is already completed."}
    elif open_change_count == 0 and landed_change_count > 0:
        verdict = "ready_to_complete"
        recommended = {"code": "complete_task", "label": "Complete task", "detail": "All linked changes are landed."}
    elif any(str(change.get("status") or "") in _LOCAL_TASK_QUEUE_READY_TO_LAND_STATES for change in changes):
        verdict = "ready_to_land"
        recommended = {"code": "land_change", "label": "Land change", "detail": "At least one linked change looks ready to land."}
    elif not changes:
        verdict = "planning"
        recommended = {"code": "start_change", "label": "Start first change", "detail": "No linked changes exist yet."}
    else:
        verdict = str(queue_entry["workflow"]["state"] or "in_progress")
        recommended = queue_entry["next_action"]
    return {
        "task": task,
        "workflow": queue_entry["workflow"],
        "summary": {
            "verdict": verdict,
            "open_change_count": open_change_count,
            "landed_change_count": landed_change_count,
            "effective_on_target_change_count": effective_on_target_count,
            "stale_workflow_records": False,
        },
        "target": {
            "line_name": target_line,
            "head_snapshot_id": target.get("head_snapshot_id"),
            "ancestor_snapshot_count": len(target_ancestry),
            "source": "local",
        },
        "recommended_action": recommended,
        "changes": change_rows,
    }


class LocalAitRuntime:
    def __init__(self, target: AgentRuntimeTarget):
        if target.mode != WORKFLOW_SCOPE_LOCAL:
            raise AgentRuntimeConfigError("LocalAitRuntime requires a local workflow target.")
        self.target = target

    def _ctx(self) -> RepoContext:
        return _runtime_repo_ctx(self.target.repo_root)

    def create_session(self, *, session_kind: str, title: str, metadata: dict[str, Any]) -> dict[str, Any]:
        repo_ctx = self._ctx()
        return create_local_session(
            repo_ctx,
            session_kind,
            title=title,
            line_name=current_line(repo_ctx),
            metadata=metadata,
        )

    def get_session(self, session_id: str) -> dict[str, Any]:
        return get_local_session(self._ctx(), session_id)

    def append_session_event(
        self,
        session_id: str,
        *,
        event_type: str,
        payload: dict[str, Any],
        actor_identity: str,
        actor_type: str,
    ) -> dict[str, Any]:
        return append_local_session_event(
            self._ctx(),
            session_id,
            event_type,
            payload,
            actor_identity=actor_identity,
            actor_type=actor_type,
        )

    def list_session_events(self, session_id: str, *, after_sequence: int, limit: int = 50) -> list[dict[str, Any]]:
        return list_local_session_events(self._ctx(), session_id, after_sequence=after_sequence, limit=limit)

    def create_surface_turn(
        self,
        session_id: str,
        *,
        text: str,
        surface: str,
        title: str,
        actor_identity: str,
        actor_type: str,
        actor_display_name: str | None,
        transport_envelope: dict[str, Any] | None,
    ) -> dict[str, Any]:
        repo_ctx = self._ctx()
        session = get_local_session(repo_ctx, session_id)
        if (guard_message := _compact_dag_worker_live_turn_guard(session)) is not None:
            raise ValueError(guard_message)
        normalized_text = str(text or "").strip()
        if not normalized_text:
            raise ValueError("text is required")
        workflow_context = infer_workflow_context(text=normalized_text, session=session)
        user_payload = {
            "source": surface,
            "surface_title": title,
            "text": normalized_text,
            "ingested_at": utc_now_iso(),
        }
        if actor_display_name:
            user_payload["actor_display_name"] = actor_display_name
        if transport_envelope:
            user_payload["transport_envelope"] = dict(transport_envelope)
        if workflow_context:
            user_payload["workflow_context"] = workflow_context
        user_event = append_local_session_event(
            repo_ctx,
            session_id,
            "session.message",
            user_payload,
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        reply_config = _reply_generation_config(repo_root=self.target.repo_root, repo_name=self.target.repo_name, session=session)
        checkpoint_before_reply = _latest_local_checkpoint(repo_ctx, session)
        events = _reply_generation_events(
            repo_ctx,
            session_id,
            user_event=user_event,
            checkpoint_before_reply=checkpoint_before_reply,
            reply_config=reply_config,
        )
        try:
            reply = generate_session_reply(
                reply_config,
                session=session,
                events=events,
                chat_id=session_id,
                chat_title=title,
                checkpoint=checkpoint_before_reply,
                surface=surface,
                actor_identity=actor_identity,
            )
        except ReplyGenerationError as exc:
            return {
                "ok": False,
                "session_id": session_id,
                "user_event": user_event,
                "assistant_event": None,
                "reply_text": None,
                "error": str(exc),
                "surface": surface,
            }
        assistant_text = str(reply.text or "").strip()
        reply_text = _reply_text_with_turn_analysis(
            assistant_text,
            turn_analysis=reply.turn_analysis,
            append_turn_analysis=getattr(reply_config, "telegram_append_turn_analysis", False),
        )
        delivered_via = "session_live"
        transport_reply_envelope = None
        if transport_envelope:
            delivered_via, transport_reply_envelope = _build_generic_transport_reply(
                transport_envelope=transport_envelope,
                assistant_text=assistant_text,
                attachments=reply.attachments,
                session_id=session_id,
                surface=surface,
            )
        assistant_payload = {
            "source": reply.source,
            "generated_via": "ait_agent_local_backend",
            "text": assistant_text,
            "turn_analysis": reply.turn_analysis or {},
            "model": reply.model,
            "response_id": reply.response_id,
            "usage": reply.usage or {},
            "reply_to_sequence": int(user_event.get("sequence") or 0),
            "delivered_via": delivered_via,
            "session_surface": surface,
            "surface_title": title,
            "generated_at": utc_now_iso(),
        }
        if transport_reply_envelope is not None:
            assistant_payload["transport_reply_envelope"] = transport_reply_envelope
        assistant_event = append_local_session_event(
            repo_ctx,
            session_id,
            "assistant.reply",
            assistant_payload,
            actor_identity="ait-agent-local",
            actor_type="ai_assistant",
        )
        return {
            "ok": True,
            "session_id": session_id,
            "user_event": user_event,
            "assistant_event": assistant_event,
            "reply_text": reply_text,
            "turn_analysis": reply.turn_analysis or {},
            "surface": surface,
        }

    def create_telegram_turn(
        self,
        session_id: str,
        *,
        text: str,
        chat_id: str | int,
        chat_title: str,
        chat_type: str | None,
        telegram_message_id: int | None,
        telegram_message_ids: list[int] | tuple[int, ...] | None,
        transport_envelope: dict[str, Any] | None,
        actor_identity: str,
    ) -> dict[str, Any]:
        repo_ctx = self._ctx()
        session = get_local_session(repo_ctx, session_id)
        if (guard_message := _compact_dag_worker_live_turn_guard(session)) is not None:
            raise ValueError(guard_message)
        normalized_text = str(text or "").strip()
        if not normalized_text:
            raise ValueError("text is required")
        normalized_message_ids = [int(value) for value in (telegram_message_ids or []) if int(value) > 0]
        if telegram_message_id is not None and int(telegram_message_id) > 0 and int(telegram_message_id) not in normalized_message_ids:
            normalized_message_ids.append(int(telegram_message_id))
        if not normalized_message_ids and telegram_message_id is not None and int(telegram_message_id) > 0:
            normalized_message_ids = [int(telegram_message_id)]
        workflow_context = infer_workflow_context(text=normalized_text, session=session)
        user_payload = {
            "source": "telegram",
            "text": normalized_text,
            "telegram_chat_id": str(chat_id),
            "telegram_chat_title": chat_title,
            "telegram_chat_type": chat_type,
            "telegram_message_id": telegram_message_id,
            "telegram_message_ids": normalized_message_ids,
            "logical_turn_message_count": len(normalized_message_ids),
            "ingested_at": utc_now_iso(),
        }
        if transport_envelope:
            user_payload["transport_envelope"] = dict(transport_envelope)
        if workflow_context:
            user_payload["workflow_context"] = workflow_context
        user_event = append_local_session_event(
            repo_ctx,
            session_id,
            "telegram.user_message",
            user_payload,
            actor_identity=actor_identity,
            actor_type="telegram_user",
        )
        reply_config = _reply_generation_config(repo_root=self.target.repo_root, repo_name=self.target.repo_name, session=session)
        checkpoint_before_reply = _latest_local_checkpoint(repo_ctx, session)
        events = _reply_generation_events(
            repo_ctx,
            session_id,
            user_event=user_event,
            checkpoint_before_reply=checkpoint_before_reply,
            reply_config=reply_config,
        )
        try:
            reply = generate_session_reply(
                reply_config,
                session=session,
                events=events,
                chat_id=chat_id,
                chat_title=chat_title or str(session.get("title") or chat_id),
                checkpoint=checkpoint_before_reply,
                actor_identity=actor_identity,
            )
        except ReplyGenerationError as exc:
            refreshed_session = get_local_session(repo_ctx, session_id)
            return {
                "ok": False,
                "session_id": session_id,
                "user_event": user_event,
                "assistant_event": None,
                "reply_text": None,
                "error": str(exc),
                "checkpoint": None,
                "telegram_context_runtime": {
                    "reply_context_mode": "recent_tail",
                    "has_checkpoint": bool(_latest_local_checkpoint(repo_ctx, refreshed_session)),
                },
            }
        assistant_text = str(reply.text or "").strip()
        reply_text = _reply_text_with_turn_analysis(
            assistant_text,
            turn_analysis=reply.turn_analysis,
            append_turn_analysis=getattr(reply_config, "telegram_append_turn_analysis", False),
        )
        assistant_payload = {
            "source": reply.source,
            "generated_via": "ait_agent_local_backend",
            "text": assistant_text,
            "turn_analysis": reply.turn_analysis or {},
            "model": reply.model,
            "response_id": reply.response_id,
            "usage": reply.usage or {},
            "telegram_chat_id": str(chat_id),
            "telegram_chat_title": chat_title,
            "reply_to_sequence": int(user_event.get("sequence") or 0),
            "delivered_via": "telegram_live",
            "generated_at": utc_now_iso(),
            "transport_reply_envelope": build_transport_reply_envelope(
                transport="telegram",
                channel_id=chat_id,
                channel_title=chat_title,
                channel_kind=chat_type,
                text=assistant_text,
                reply_to_event_id=(str(transport_envelope.get("event_id") or "").strip() if transport_envelope else None),
                reply_to_message_id=telegram_message_id,
                reply_to_message_ids=normalized_message_ids,
                metadata={"delivered_via": "telegram_live"},
            ),
        }
        assistant_event = append_local_session_event(
            repo_ctx,
            session_id,
            "assistant.reply",
            assistant_payload,
            actor_identity="ait-agent-local",
            actor_type="ai_assistant",
        )
        checkpoint = create_local_checkpoint(
            repo_ctx,
            session_id,
            summary=f"Telegram checkpoint after reply to {payload_text(user_payload)[:80]}",
            resume_payload={
                "source": "telegram",
                "objective": payload_text(user_payload),
                "latest_user_request": payload_text(user_payload),
                "latest_assistant_reply": payload_text(assistant_payload),
                "context": {
                    "chat_id": str(chat_id),
                    "chat_title": chat_title,
                    "chat_type": chat_type,
                    "last_event_sequence": int(assistant_event.get("sequence") or 0),
                    "last_user_sequence": int(user_event.get("sequence") or 0),
                    "last_assistant_sequence": int(assistant_event.get("sequence") or 0),
                    "logical_turn_message_count": len(normalized_message_ids),
                    "checkpoint_reason": "local_live_turn",
                },
            },
            based_on_sequence=int(assistant_event.get("sequence") or 0),
        )
        return {
            "ok": True,
            "session_id": session_id,
            "user_event": user_event,
            "assistant_event": assistant_event,
            "reply_text": reply_text,
            "turn_analysis": reply.turn_analysis or {},
            "checkpoint": checkpoint,
            "telegram_context_runtime": {
                "reply_context_mode": "checkpoint_delta",
                "has_checkpoint": True,
                "checkpoint_id": checkpoint.get("checkpoint_id"),
                "checkpoint_based_on_sequence": checkpoint.get("based_on_sequence"),
                "last_event_sequence": int(assistant_event.get("sequence") or 0),
                "delta_event_count": 0,
                "checkpoint_freshness": "fresh",
                "refresh_recommended": False,
            },
        }

    def read_task_queue(self) -> dict[str, Any]:
        return _local_task_queue_payload(self._ctx(), self.target.repo_name)

    def read_task(self, task_id: str) -> dict[str, Any]:
        return _local_task_detail(self._ctx(), task_id)

    def read_change(self, change_id: str) -> dict[str, Any]:
        return _local_change_detail(self._ctx(), change_id)

    def read_task_audit(self, task_id: str, *, target_line: str = "main") -> dict[str, Any]:
        return _local_task_audit(self._ctx(), task_id, target_line=target_line)

    def read_task_dag_progress(self, graph: dict[str, Any]) -> dict[str, Any]:
        raise AgentRuntimeConfigError("Task DAG progress notifications require a remote/shared workflow mode.")
