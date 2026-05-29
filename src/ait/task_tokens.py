from __future__ import annotations

from collections import defaultdict
from typing import Any, Callable

from .remote_client import (
    get_task as remote_get_task,
    list_session_events as remote_list_session_events,
    list_sessions as remote_list_sessions,
)
from .repo_paths import RepoContext
from .store_local_tasks import get_local_task
from .store_local_sessions import list_local_session_events, list_local_sessions

_EVENT_PAGE_LIMIT = 200
_USAGE_TOKEN_KEYS = (
    "prompt_tokens",
    "completion_tokens",
    "total_tokens",
    "cached_input_tokens",
    "reasoning_output_tokens",
)


def _optional_nonnegative_int(value: Any) -> int | None:
    if value is None:
        return None
    try:
        normalized = int(value)
    except (TypeError, ValueError):
        return None
    if normalized < 0:
        return None
    return normalized


def _extract_usage_tokens(payload: dict[str, Any]) -> tuple[dict[str, int | None], str]:
    usage = payload.get("usage")
    if isinstance(usage, dict):
        nested_last = usage.get("last")
        if isinstance(nested_last, dict):
            extracted = _usage_payload_tokens(nested_last)
            if _usage_has_values(extracted):
                return extracted, "usage.last"
        extracted = _usage_payload_tokens(usage)
        if _usage_has_values(extracted):
            return extracted, "usage"
    extracted = _usage_payload_tokens(payload)
    if _usage_has_values(extracted):
        return extracted, "payload"
    return _empty_usage_tokens(), "missing"


def _usage_payload_tokens(payload: dict[str, Any]) -> dict[str, int | None]:
    prompt_tokens = _first_token_int(payload, "prompt_tokens", "input_tokens", "prompt", "input")
    completion_tokens = _first_token_int(payload, "completion_tokens", "output_tokens", "completion", "output")
    total_tokens = _first_token_int(payload, "total_tokens", "total")
    if total_tokens is None and (prompt_tokens is not None or completion_tokens is not None):
        total_tokens = int(prompt_tokens or 0) + int(completion_tokens or 0)
    return {
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": total_tokens,
        "cached_input_tokens": _first_token_int(payload, "cached_input_tokens"),
        "reasoning_output_tokens": _first_token_int(payload, "reasoning_output_tokens"),
    }


def _first_token_int(payload: dict[str, Any], *keys: str) -> int | None:
    for key in keys:
        value = _optional_nonnegative_int(payload.get(key))
        if value is not None:
            return value
    return None


def _usage_has_values(usage: dict[str, int | None]) -> bool:
    return any(value is not None for value in usage.values())


def _empty_usage_tokens() -> dict[str, int | None]:
    return {
        "prompt_tokens": None,
        "completion_tokens": None,
        "total_tokens": None,
        "cached_input_tokens": None,
        "reasoning_output_tokens": None,
    }


def _zero_usage_totals() -> dict[str, int]:
    return {key: 0 for key in _USAGE_TOKEN_KEYS}


def _add_usage_totals(destination: dict[str, int], usage: dict[str, int | None]) -> None:
    for key in _USAGE_TOKEN_KEYS:
        destination[key] += int(usage.get(key) or 0)


def _fetch_all_local_session_events(ctx: RepoContext, session_id: str) -> list[dict[str, Any]]:
    return _fetch_all_session_events(
        lambda after_sequence: list_local_session_events(
            ctx,
            session_id,
            after_sequence=after_sequence,
            limit=_EVENT_PAGE_LIMIT,
        )
    )


def _fetch_all_remote_session_events(base_url: str, repo_name: str, session_id: str) -> list[dict[str, Any]]:
    return _fetch_all_session_events(
        lambda after_sequence: remote_list_session_events(
            base_url,
            session_id,
            after_sequence=after_sequence,
            limit=_EVENT_PAGE_LIMIT,
            repo_name=repo_name,
        )
    )


def _fetch_all_session_events(
    loader: Callable[[int], list[dict[str, Any]]],
) -> list[dict[str, Any]]:
    after_sequence = 0
    rows: list[dict[str, Any]] = []
    while True:
        chunk = list(loader(after_sequence) or [])
        if not chunk:
            return rows
        rows.extend(chunk)
        if len(chunk) < _EVENT_PAGE_LIMIT:
            return rows
        after_sequence = int(chunk[-1].get("sequence") or after_sequence)


def local_task_tokens_report(ctx: RepoContext, task_id: str) -> dict[str, Any]:
    task = get_local_task(ctx, task_id)
    sessions = [row for row in list_local_sessions(ctx) if str(row.get("task_id") or "") == task_id]
    return _build_task_tokens_report(
        task=task,
        sessions=sessions,
        event_loader=lambda session_id: _fetch_all_local_session_events(ctx, session_id),
        scope={"mode": "local", "repo_name": str(task.get("repo_name") or ctx.root.name)},
    )


def remote_task_tokens_report(base_url: str, repo_name: str, task_ref: str) -> dict[str, Any]:
    task = remote_get_task(base_url, task_ref, repo_name=repo_name)
    task_id = str(task.get("task_id") or task_ref)
    sessions = [row for row in remote_list_sessions(base_url, repo_name) if str(row.get("task_id") or "") == task_id]
    return _build_task_tokens_report(
        task=task,
        sessions=sessions,
        event_loader=lambda session_id: _fetch_all_remote_session_events(base_url, repo_name, session_id),
        scope={"mode": "remote", "repo_name": repo_name},
    )


def _build_task_tokens_report(
    *,
    task: dict[str, Any],
    sessions: list[dict[str, Any]],
    event_loader: Callable[[str], list[dict[str, Any]]],
    scope: dict[str, Any],
) -> dict[str, Any]:
    summary = _zero_usage_totals()
    summary_counts = {
        "session_count": len(sessions),
        "sessions_with_usage_count": 0,
        "assistant_reply_count": 0,
        "metered_reply_count": 0,
        "usage_last_reply_count": 0,
        "direct_usage_reply_count": 0,
        "payload_usage_reply_count": 0,
        "missing_usage_reply_count": 0,
    }
    session_rows: list[dict[str, Any]] = []
    change_rollups: dict[str, dict[str, Any]] = {}
    worktree_rollups: dict[str, dict[str, Any]] = {}
    model_rollups: dict[str, dict[str, Any]] = {}
    models_seen: set[str] = set()

    for session in sessions:
        session_id = str(session.get("session_id") or "")
        usage_totals = _zero_usage_totals()
        assistant_reply_count = 0
        metered_reply_count = 0
        usage_last_count = 0
        direct_usage_count = 0
        payload_usage_count = 0
        missing_usage_count = 0
        session_models: set[str] = set()

        for event in event_loader(session_id):
            if str(event.get("event_type") or "") != "assistant.reply":
                continue
            assistant_reply_count += 1
            payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
            usage, usage_source = _extract_usage_tokens(payload)
            model_name = str(payload.get("model") or session.get("model_name") or "").strip()
            if model_name:
                session_models.add(model_name)
                models_seen.add(model_name)
            if usage_source == "missing":
                missing_usage_count += 1
                continue
            metered_reply_count += 1
            if usage_source == "usage.last":
                usage_last_count += 1
            elif usage_source == "usage":
                direct_usage_count += 1
            else:
                payload_usage_count += 1
            _add_usage_totals(usage_totals, usage)
            if model_name:
                model_rollup = model_rollups.setdefault(
                    model_name,
                    {
                        "model_name": model_name,
                        "assistant_reply_count": 0,
                        "metered_reply_count": 0,
                        **_zero_usage_totals(),
                    },
                )
                model_rollup["assistant_reply_count"] += 1
                model_rollup["metered_reply_count"] += 1
                _add_usage_totals(model_rollup, usage)

        if metered_reply_count > 0:
            summary_counts["sessions_with_usage_count"] += 1
        summary_counts["assistant_reply_count"] += assistant_reply_count
        summary_counts["metered_reply_count"] += metered_reply_count
        summary_counts["usage_last_reply_count"] += usage_last_count
        summary_counts["direct_usage_reply_count"] += direct_usage_count
        summary_counts["payload_usage_reply_count"] += payload_usage_count
        summary_counts["missing_usage_reply_count"] += missing_usage_count
        _add_usage_totals(summary, usage_totals)

        session_row = {
            "session_id": session_id,
            "title": session.get("title"),
            "session_kind": session.get("session_kind"),
            "status": session.get("status"),
            "task_id": session.get("task_id"),
            "change_id": session.get("change_id"),
            "line_name": session.get("line_name"),
            "worktree_name": session.get("worktree_name"),
            "model_name": session.get("model_name"),
            "assistant_reply_count": assistant_reply_count,
            "metered_reply_count": metered_reply_count,
            "usage_last_reply_count": usage_last_count,
            "direct_usage_reply_count": direct_usage_count,
            "payload_usage_reply_count": payload_usage_count,
            "missing_usage_reply_count": missing_usage_count,
            "models": sorted(session_models),
            **usage_totals,
        }
        session_rows.append(session_row)
        _add_session_rollup(
            change_rollups,
            key=str(session.get("change_id") or "").strip() or "(task-only)",
            label_field="change_id",
            session_row=session_row,
        )
        _add_session_rollup(
            worktree_rollups,
            key=str(session.get("worktree_name") or "").strip() or "(none)",
            label_field="worktree_name",
            session_row=session_row,
        )

    return {
        "scope": scope,
        "task": task,
        "summary": {**summary_counts, **summary, "models": sorted(models_seen)},
        "sessions": sorted(session_rows, key=lambda row: (str(row.get("session_id") or ""))),
        "changes": sorted(change_rollups.values(), key=lambda row: str(row.get("change_id") or "")),
        "worktrees": sorted(worktree_rollups.values(), key=lambda row: str(row.get("worktree_name") or "")),
        "models": sorted(model_rollups.values(), key=lambda row: str(row.get("model_name") or "")),
    }


def _add_session_rollup(
    destination: dict[str, dict[str, Any]],
    *,
    key: str,
    label_field: str,
    session_row: dict[str, Any],
) -> None:
    rollup = destination.setdefault(
        key,
        {
            label_field: key,
            "session_count": 0,
            "assistant_reply_count": 0,
            "metered_reply_count": 0,
            **_zero_usage_totals(),
        },
    )
    rollup["session_count"] += 1
    rollup["assistant_reply_count"] += int(session_row.get("assistant_reply_count") or 0)
    rollup["metered_reply_count"] += int(session_row.get("metered_reply_count") or 0)
    _add_usage_totals(rollup, session_row)
