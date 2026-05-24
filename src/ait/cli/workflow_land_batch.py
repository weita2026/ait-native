from __future__ import annotations

from typing import Any, Callable

from ait_protocol.common import AuthorMode, normalize_optional_text

from ..remote_client import get_change as remote_get_change
from ..store import RepoContext
from .remote_repository_defaults import _sync_remote_repository_defaults
from .runtime_defaults import _normalize_text_value
from .task_tracking_bindings import _task_worktree_repo_ctx
from .workflow_land_completed_local import (
    _workflow_land_apply_completed_local_entry,
    _workflow_land_completed_local_preview_state,
)
from .workflow_land_state import _workflow_land_payload
from .workflow_land_views import _workflow_land_batch_item_status, _workflow_land_preview_item_status


SelectorFn = Callable[..., dict[str, Any]]
ApplyFn = Callable[..., dict[str, Any]]


def _workflow_land_completed_local_payload(
    ctx: RepoContext,
    *,
    change_id: str,
    remote_name: str | None,
    selector_completed_local_fn: SelectorFn,
) -> dict[str, Any]:
    if remote_name is None:
        raise ValueError("Completed local later-promotion requires `--remote <name>`.")
    repo_ctx = _task_worktree_repo_ctx(ctx)
    selector = selector_completed_local_fn(repo_ctx, remote_name=remote_name, local_change_id=change_id)
    entry = next((row for row in selector.get("entries") or [] if isinstance(row, dict)), None)
    if entry is None:
        raise ValueError(f"No completed local landed change is available for `{change_id}`.")
    state = _workflow_land_completed_local_preview_state(repo_ctx, entry=entry, remote_name=remote_name)
    state["selector"] = {
        "mode": "completed_local_single_change",
        "remote": remote_name,
        "change_id": change_id,
    }
    return state


def _workflow_land_completed_local_apply(
    ctx: RepoContext,
    *,
    change_id: str,
    remote_name: str | None,
    summary: str | None,
    tests: str | None,
    lint: str | None,
    security: str | None,
    license: str | None,
    author_mode: AuthorMode | None,
    model: str | None,
    session: str | None,
    checkpoint: str | None,
    reviewer: str | None,
    review_message: str | None,
    target: str | None,
    mode: str,
    selector_completed_local_fn: SelectorFn,
    apply_fn: ApplyFn,
) -> dict[str, Any]:
    if remote_name is None:
        raise ValueError("Completed local later-promotion requires `--remote <name>`.")
    repo_ctx = _task_worktree_repo_ctx(ctx)
    remote_row, repo_name = _sync_remote_repository_defaults(repo_ctx, remote_name)
    selector = selector_completed_local_fn(repo_ctx, remote_name=remote_name, local_change_id=change_id)
    entry = next((row for row in selector.get("entries") or [] if isinstance(row, dict)), None)
    if entry is None:
        raise ValueError(f"No completed local landed change is available for `{change_id}`.")
    state, local_refs = _workflow_land_apply_completed_local_entry(
        repo_ctx,
        entry=entry,
        remote_name=remote_name,
        remote_row=remote_row,
        repo_name=repo_name,
        summary=summary,
        tests=tests,
        lint=lint,
        security=security,
        license=license,
        author_mode=author_mode,
        model=model,
        session=session,
        checkpoint=checkpoint,
        reviewer=reviewer,
        review_message=review_message,
        target=target,
        mode=mode,
        apply_fn=apply_fn,
    )
    state["selector"] = {
        "mode": "completed_local_single_change",
        "remote": remote_name,
        **local_refs,
    }
    return state


def _workflow_land_batch_payload(
    ctx: RepoContext,
    *,
    all_completed_local: bool,
    graph_run_session_id: str | None,
    remote_name: str | None,
    target: str | None,
    selector_completed_local_fn: SelectorFn,
    graph_run_selector_fn: SelectorFn,
) -> dict[str, Any]:
    if all_completed_local and graph_run_session_id:
        raise ValueError("Choose either --all-completed-local or --graph-run-session, not both.")
    if not all_completed_local and not graph_run_session_id:
        raise ValueError("Choose --all-completed-local or --graph-run-session for batch workflow land.")
    if remote_name is None:
        raise ValueError("Batch workflow land requires `--remote <name>`.")

    repo_ctx = _task_worktree_repo_ctx(ctx)
    remote_row, repo_name = _sync_remote_repository_defaults(repo_ctx, remote_name)
    if all_completed_local:
        selector = selector_completed_local_fn(repo_ctx, remote_name=remote_name)
    else:
        selector = graph_run_selector_fn(
            repo_ctx,
            remote_name=remote_name,
            graph_run_session_id=str(graph_run_session_id),
        )
    entries = [row for row in selector.get("entries") or [] if isinstance(row, dict)]
    if not entries:
        raise ValueError("No batch workflow land items are available.")

    mode_name = str(selector.get("mode") or "batch")
    completed_items = 0
    ready_items = 0
    item_results: list[dict[str, Any]] = []
    for index, entry in enumerate(entries, start=1):
        if mode_name == "all_completed_local":
            state = _workflow_land_completed_local_preview_state(repo_ctx, entry=entry, remote_name=remote_name)
            target_line = str(entry.get("target_line") or target or "main")
            routing = state.get("routing") if isinstance(state.get("routing"), dict) else {}
            local_refs = {
                "task_id": str(entry["task"]["task_id"]),
                "change_id": str(entry["change"]["change_id"]),
                "remote_task_id": str(routing.get("remote_task_id") or routing.get("published_task_id") or ""),
                "remote_change_id": str(
                    routing.get("remote_change_id")
                    or routing.get("published_change_id")
                    or entry["change"]["change_id"]
                ),
            }
            item_status = "ready"
        else:
            state = _workflow_land_payload(
                repo_ctx,
                change_id=str(entry["change_id"]),
                patchset_id=None,
                remote_name=remote_name,
            )
            target_line = normalize_optional_text(target) or str((state.get("change") or {}).get("base_line") or "main")
            local_refs = {}
            item_status = _workflow_land_preview_item_status(state)
        if item_status == "completed":
            completed_items += 1
        else:
            ready_items += 1
        next_action = state.get("next_action") if isinstance(state.get("next_action"), dict) else {}
        item_results.append(
            {
                "index": index,
                "status": item_status,
                "percent_complete": int((completed_items * 100) / len(entries)),
                "current_step": str(next_action.get("code") or "done"),
                "target_line": target_line,
                "remote_change_id": str(
                    local_refs.get("remote_change_id")
                    or (state.get("change") or {}).get("change_id")
                    or entry.get("change_id")
                    or ""
                ),
                "remote_task_id": str(
                    local_refs.get("remote_task_id")
                    or (state.get("task") or {}).get("task_id")
                    or ""
                ),
                "state": state,
                **local_refs,
            }
        )

    status = "completed" if completed_items == len(entries) else "ready"
    return {
        "status": status,
        "mode": mode_name,
        "remote": _normalize_text_value(remote_row.get("name")) or remote_name,
        "repo_name": repo_name,
        "current_item": len(item_results),
        "total_items": len(entries),
        "completed_items": completed_items,
        "ready_items": ready_items,
        "blocked_items": 0,
        "percent_complete": int((completed_items * 100) / len(entries)),
        "items": item_results,
        "stopped_reason": None,
        "selector": {
            "all_completed_local": all_completed_local,
            "graph_run_session_id": graph_run_session_id,
        },
        "skipped_task_ids": selector.get("skipped_task_ids") or [],
        "skipped_change_ids": selector.get("skipped_change_ids") or [],
        "graph_run_summary": selector.get("summary"),
    }


def _workflow_land_batch_run(
    ctx: RepoContext,
    *,
    all_completed_local: bool,
    graph_run_session_id: str | None,
    remote_name: str | None,
    summary: str | None,
    tests: str | None,
    lint: str | None,
    security: str | None,
    license: str | None,
    author_mode: AuthorMode | None,
    model: str | None,
    session: str | None,
    checkpoint: str | None,
    reviewer: str | None,
    review_message: str | None,
    target: str | None,
    mode: str,
    selector_completed_local_fn: SelectorFn,
    graph_run_selector_fn: SelectorFn,
    apply_fn: ApplyFn,
) -> dict[str, Any]:
    if all_completed_local and graph_run_session_id:
        raise ValueError("Choose either --all-completed-local or --graph-run-session, not both.")
    if not all_completed_local and not graph_run_session_id:
        raise ValueError("Choose --all-completed-local or --graph-run-session for batch workflow land.")
    if remote_name is None:
        raise ValueError("Batch workflow land requires `--remote <name>`.")
    if graph_run_session_id and session:
        raise ValueError("Use either --graph-run-session or --session, not both.")

    repo_ctx = _task_worktree_repo_ctx(ctx)
    remote_row, repo_name = _sync_remote_repository_defaults(repo_ctx, remote_name)
    if all_completed_local:
        selector = selector_completed_local_fn(repo_ctx, remote_name=remote_name)
    else:
        selector = graph_run_selector_fn(
            repo_ctx,
            remote_name=remote_name,
            graph_run_session_id=str(graph_run_session_id),
        )
    entries = [row for row in selector.get("entries") or [] if isinstance(row, dict)]
    if not entries:
        raise ValueError("No batch workflow land items are available.")

    mode_name = str(selector.get("mode") or "batch")
    completed_items = 0
    blocked_items = 0
    item_results: list[dict[str, Any]] = []
    stopped_reason: str | None = None

    for index, entry in enumerate(entries, start=1):
        if mode_name == "all_completed_local":
            state, local_refs = _workflow_land_apply_completed_local_entry(
                repo_ctx,
                entry=entry,
                remote_name=remote_name,
                remote_row=remote_row,
                repo_name=repo_name,
                summary=summary,
                tests=tests,
                lint=lint,
                security=security,
                license=license,
                author_mode=author_mode,
                model=model,
                session=session,
                checkpoint=checkpoint,
                reviewer=reviewer,
                review_message=review_message,
                target=target,
                mode=mode,
                apply_fn=apply_fn,
            )
            target_line = str(entry.get("target_line") or target or "main")
        else:
            remote_change = remote_get_change(remote_row["url"], str(entry["change_id"]), repo_name=repo_name)
            target_line = normalize_optional_text(target) or str(remote_change.get("base_line") or "main")
            state = apply_fn(
                repo_ctx,
                change_id=str(remote_change["change_id"]),
                patchset_id=None,
                remote_name=remote_name,
                snapshot_message=None,
                patchset_summary=summary,
                tests=tests,
                lint=lint,
                security=security,
                license=license,
                author_mode=author_mode,
                model=model,
                session=str(graph_run_session_id),
                checkpoint=checkpoint,
                reviewer=reviewer,
                review_message=review_message,
                target=target_line,
                mode=mode,
            )
            local_refs = {}

        item_status = _workflow_land_batch_item_status(state)
        if item_status == "completed":
            completed_items += 1
        else:
            blocked_items += 1
            stopped_reason = (
                f"Batch workflow land stopped at item {index}/{len(entries)} "
                f"({str(state.get('change', {}).get('change_id') or entry.get('change_id') or 'unknown')})."
            )
        next_action = state.get("next_action") if isinstance(state.get("next_action"), dict) else {}
        item_result = {
            "index": index,
            "status": item_status,
            "percent_complete": int((completed_items * 100) / len(entries)),
            "current_step": str(next_action.get("code") or "done"),
            "target_line": target_line,
            "remote_change_id": str((state.get("change") or {}).get("change_id") or entry.get("change_id") or ""),
            "remote_task_id": str((state.get("task") or {}).get("task_id") or ""),
            "state": state,
            **local_refs,
        }
        item_results.append(item_result)
        if item_status != "completed":
            break

    status = "completed" if completed_items == len(entries) and blocked_items == 0 else "blocked"
    return {
        "status": status,
        "mode": mode_name,
        "remote": _normalize_text_value(remote_row.get("name")) or remote_name,
        "repo_name": repo_name,
        "current_item": len(item_results),
        "total_items": len(entries),
        "completed_items": completed_items,
        "blocked_items": blocked_items,
        "percent_complete": int((completed_items * 100) / len(entries)),
        "items": item_results,
        "stopped_reason": stopped_reason,
        "selector": {
            "all_completed_local": all_completed_local,
            "graph_run_session_id": graph_run_session_id,
        },
        "skipped_task_ids": selector.get("skipped_task_ids") or [],
        "skipped_change_ids": selector.get("skipped_change_ids") or [],
        "graph_run_summary": selector.get("summary"),
    }
