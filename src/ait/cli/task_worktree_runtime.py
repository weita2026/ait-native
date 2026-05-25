from __future__ import annotations

import importlib
import shlex
from pathlib import Path
from typing import Any, Mapping

from ..remote_client import RemoteError, get_change as remote_get_change, read_task_audit as remote_read_task_audit
from ..store import (
    RepoContext,
    add_worktree as local_add_worktree,
    bind_worktree as local_bind_worktree,
    current_line,
    get_line,
    get_local_change,
    get_local_task,
    get_worktree as local_get_worktree,
    list_local_tasks,
    load_config,
    remove_worktree as local_remove_worktree,
    save_config,
    sync_worktree as local_sync_worktree,
)
from ..task_worktree_layout import resolve_task_auto_worktree_location
from .remote_repository_defaults import _remote_tuple
from .task_tracking_bindings import _default_line_name, _task_tracking_enabled, _task_worktree_repo_ctx, _tracked_session_binding
from .task_worktree_guidance import _task_worktree_output
from .task_worktree_resolution import (
    _ensure_task_feature_line,
    _find_auto_created_task_worktree,
    _resolve_task_bound_worktree_name,
    _session_bound_worktree,
    _task_feature_line_name,
)
from .workflow_mode_config import _effective_task_worktree, _normalize_text_value


ACTIVE_ROOT_WORKTREE_GUARD_BYPASS_PREFIXES = (
    "land auto sync ",
    "plan sync ",
    "task auto worktree ",
    "worktree ",
)
READONLY_ROOT_MAIN_GUARD_BYPASS_PREFIXES = (
    "plan sync ",
    "land auto sync ",
    "pull",
    "push",
    "task auto worktree ",
    "workspace restore",
    "worktree ",
)


def _app_module():
    return importlib.import_module("ait.cli.app")


def _run_locked_workspace_command(ctx: RepoContext, command_name: str, operation):
    return _app_module()._run_locked_workspace_command(ctx, command_name, operation)


def _set_active_root_worktree_binding(ctx: RepoContext, worktree_name: str | None) -> None:
    if ctx.is_worktree:
        return
    cfg = load_config(ctx)
    resolved_worktree_name = _normalize_text_value(worktree_name)
    if resolved_worktree_name is None:
        if "worktree_name" in cfg:
            cfg.pop("worktree_name", None)
            save_config(ctx, cfg)
        return
    if _normalize_text_value(cfg.get("worktree_name")) == resolved_worktree_name:
        return
    cfg["worktree_name"] = resolved_worktree_name
    save_config(ctx, cfg)


def _clear_active_root_worktree_binding(
    ctx: RepoContext,
    *,
    worktree_name: str | None = None,
) -> None:
    if ctx.is_worktree:
        return
    cfg = load_config(ctx)
    configured_worktree_name = _normalize_text_value(cfg.get("worktree_name"))
    if configured_worktree_name is None:
        return
    resolved_worktree_name = _normalize_text_value(worktree_name)
    if resolved_worktree_name is not None and configured_worktree_name != resolved_worktree_name:
        return
    cfg.pop("worktree_name", None)
    save_config(ctx, cfg)


def _active_root_worktree(ctx: RepoContext) -> dict[str, Any] | None:
    if ctx.is_worktree:
        return None
    configured_worktree_name = _normalize_text_value(load_config(ctx).get("worktree_name"))
    if configured_worktree_name is None:
        return None
    repo_ctx = _task_worktree_repo_ctx(ctx)
    try:
        worktree = local_get_worktree(repo_ctx, configured_worktree_name)
    except KeyError:
        _clear_active_root_worktree_binding(ctx, worktree_name=configured_worktree_name)
        return None
    path_value = _normalize_text_value(worktree.get("path")) or _normalize_text_value(worktree.get("workspace_root"))
    if path_value is None:
        _clear_active_root_worktree_binding(ctx, worktree_name=configured_worktree_name)
        return None
    target_workspace_root = str(Path(path_value).expanduser().resolve())
    if target_workspace_root == str(ctx.root.resolve()):
        _clear_active_root_worktree_binding(ctx, worktree_name=configured_worktree_name)
        return None
    payload = dict(worktree)
    payload["target_workspace_root"] = target_workspace_root
    payload.setdefault("cd_command", f"cd {shlex.quote(target_workspace_root)}")
    return payload


def _internal_worktree_role(ctx: RepoContext) -> str | None:
    if not ctx.is_worktree:
        return None
    return _normalize_text_value(load_config(ctx).get("internal_role"))


def _guard_internal_worktree_role(ctx: RepoContext, command_name: str) -> None:
    role = _internal_worktree_role(ctx)
    if role != "main_seed_mirror":
        return
    cfg = load_config(ctx)
    worktree_name = _normalize_text_value(cfg.get("worktree_name")) or "main-seed"
    line_name = _normalize_text_value(cfg.get("seed_line_name")) or _normalize_text_value(cfg.get("current_line")) or "main"
    raise ValueError(
        f"Worktree `{worktree_name}` is an internal `{line_name}` seed mirror cache. "
        f"Run `ait {command_name}` from repo root or a task-bound worktree instead of this internal cache workspace."
    )


def _active_root_worktree_allows_task_bootstrap(ctx: RepoContext, command_name: str) -> bool:
    normalized_command_name = str(command_name or "").strip().lower()
    if ctx.is_worktree or normalized_command_name != "task start":
        return False
    return current_line(ctx) == _default_line_name(ctx)


def _task_auto_worktree_bootstrap_allows_dirty_source(ctx: RepoContext, command_name: str | None) -> bool:
    normalized_command_name = str(command_name or "").strip().lower()
    if ctx.is_worktree or normalized_command_name != "task start":
        return False
    return current_line(ctx) == _default_line_name(ctx)


def _guard_active_root_worktree(ctx: RepoContext, command_name: str) -> None:
    normalized_command_name = str(command_name or "").strip().lower()
    if normalized_command_name and any(
        normalized_command_name.startswith(prefix) for prefix in ACTIVE_ROOT_WORKTREE_GUARD_BYPASS_PREFIXES
    ):
        return
    active_worktree = _active_root_worktree(ctx)
    if active_worktree is None:
        return
    if _active_root_worktree_allows_task_bootstrap(ctx, command_name):
        return
    worktree_name = str(active_worktree.get("name") or "unknown")
    task_id = _normalize_text_value(active_worktree.get("bound_task_id"))
    task_fragment = f" for task `{task_id}`" if task_id else ""
    cd_command = str(active_worktree.get("cd_command") or f"cd {shlex.quote(active_worktree['target_workspace_root'])}")
    raise ValueError(
        f"Repo root is pinned to bound worktree `{worktree_name}`{task_fragment}. "
        f"Continue in that task workspace with `{cd_command}` before running `ait {command_name}`."
    )


def _readonly_root_main_enabled(ctx: RepoContext) -> bool:
    if ctx.is_worktree:
        return False
    if not _repository_has_task_workflow_context(ctx):
        return False
    return current_line(ctx) == _default_line_name(ctx)


def _guard_readonly_root_main(ctx: RepoContext, command_name: str) -> None:
    normalized_command_name = str(command_name or "").strip().lower()
    if normalized_command_name and any(
        normalized_command_name.startswith(prefix) for prefix in READONLY_ROOT_MAIN_GUARD_BYPASS_PREFIXES
    ):
        return
    if not _readonly_root_main_enabled(ctx):
        return
    default_line = _default_line_name(ctx)
    raise ValueError(
        f"Repo root line `{default_line}` is a read-only deployment workspace. "
        "Start or continue work inside a task worktree. "
        f"Run `ait {command_name}` there."
    )


def _repository_has_task_workflow_context(ctx: RepoContext) -> bool:
    repo_ctx = _task_worktree_repo_ctx(ctx)
    if _normalize_text_value((_tracked_session_binding(repo_ctx) or {}).get("task_id")) is not None:
        return True
    try:
        if list_local_tasks(repo_ctx):
            return True
    except Exception:
        pass
    try:
        if ctx.is_worktree:
            worktree = local_get_worktree(ctx)
            if _bound_task_id_for_worktree(ctx, worktree) is not None:
                return True
        else:
            active_worktree = _active_root_worktree(ctx)
            if _bound_task_id_for_worktree(repo_ctx, active_worktree) is not None:
                return True
    except Exception:
        pass
    return False


def _strict_task_bound_authoring_enabled(ctx: RepoContext, *, command_name: str | None = None) -> bool:
    normalized_command_name = str(command_name or "").strip().lower()
    if normalized_command_name in {
        "change create",
        "land submit",
        "patchset publish",
        "workflow land snapshot",
        "workflow land patchset publish",
    }:
        return True
    if normalized_command_name == "snapshot create":
        return _repository_has_task_workflow_context(ctx)
    return _repository_has_task_workflow_context(ctx)


def _bound_task_id_for_worktree(ctx: RepoContext, worktree: dict[str, Any] | None) -> str | None:
    if not isinstance(worktree, dict):
        return None
    task_id = _normalize_text_value(worktree.get("bound_task_id"))
    if task_id is not None:
        return task_id
    change_id = _normalize_text_value(worktree.get("bound_change_id"))
    if change_id is None:
        return None
    try:
        change = get_local_change(_task_worktree_repo_ctx(ctx), change_id)
    except KeyError:
        return None
    return _normalize_text_value(change.get("task_id"))


def _task_identity_aliases(ctx: RepoContext, task_id: str | None) -> set[str]:
    resolved_task_id = _normalize_text_value(task_id)
    if resolved_task_id is None:
        return set()
    aliases = {resolved_task_id}
    try:
        local_task = get_local_task(_task_worktree_repo_ctx(ctx), resolved_task_id)
    except KeyError:
        return aliases
    canonical_task_id = _normalize_text_value(local_task.get("task_id"))
    published_task_id = _normalize_text_value(local_task.get("published_task_id"))
    if canonical_task_id is not None:
        aliases.add(canonical_task_id)
    if published_task_id is not None:
        aliases.add(published_task_id)
    return aliases


def _guard_bound_task_match(ctx: RepoContext, *, task_id: str, command_name: str) -> None:
    if not ctx.is_worktree:
        return
    try:
        worktree = local_get_worktree(ctx)
    except KeyError:
        return
    bound_task_id = _bound_task_id_for_worktree(ctx, worktree)
    if bound_task_id is None:
        return
    bound_aliases = _task_identity_aliases(ctx, bound_task_id)
    requested_aliases = _task_identity_aliases(ctx, task_id)
    if bound_aliases.intersection(requested_aliases):
        return
    worktree_name = str(worktree.get("name") or "current worktree")
    raise ValueError(
        f"Worktree `{worktree_name}` is bound to task `{bound_task_id}`, so `ait {command_name} --task {task_id}` "
        "cannot target a different task. Continue in the other task's worktree or bind a manual worktree to that task first."
    )


def _guard_task_bound_authoring(
    ctx: RepoContext,
    command_name: str,
    *,
    local: bool = True,
    remote_name: str | None = None,
    task_id: str | None = None,
    change_id: str | None = None,
    worktree_name: str | None = None,
) -> None:
    _guard_internal_worktree_role(ctx, command_name)
    if not _strict_task_bound_authoring_enabled(ctx, command_name=command_name):
        return
    guidance = (
        f"`ait {command_name}` requires a task-bound worktree. "
        "Start with `ait task start` or continue in the matching task worktree. "
        "If that task worktree is missing, use `ait worktree recreate`."
    )
    if ctx.is_worktree:
        try:
            worktree = local_get_worktree(ctx)
        except KeyError as exc:
            raise ValueError(guidance) from exc
        resolved_task_id = _bound_task_id_for_worktree(ctx, worktree)
        if resolved_task_id is not None:
            return
        worktree_name = str(worktree.get("name") or "current worktree")
        raise ValueError(
            f"Worktree `{worktree_name}` is not bound to a task. {guidance}"
        )
    matching_worktree = _session_bound_worktree(
        ctx,
        local=local,
        remote_name=remote_name,
        task_id=task_id,
        change_id=change_id,
        worktree_name=worktree_name,
    )
    if isinstance(matching_worktree, dict):
        matching_worktree_name = str(matching_worktree.get("name") or "matching worktree")
        bound_task_id = _bound_task_id_for_worktree(_task_worktree_repo_ctx(ctx), matching_worktree)
        if bound_task_id is None:
            bound_task_id = _normalize_text_value(task_id)
        cd_command = str(
            matching_worktree.get("cd_command")
            or f"cd {shlex.quote(str(matching_worktree.get('target_workspace_root') or matching_worktree.get('path') or ''))}"
        ).strip()
        task_fragment = f" for task `{bound_task_id}`" if bound_task_id else ""
        if cd_command:
            raise ValueError(
                f"`ait {command_name}` requires a task-bound worktree. "
                f"Continue in bound worktree `{matching_worktree_name}`{task_fragment} with `{cd_command}`."
            )
        raise ValueError(
            f"`ait {command_name}` requires a task-bound worktree. "
            f"Continue in bound worktree `{matching_worktree_name}`{task_fragment}."
        )
    raise ValueError(guidance)


def _maybe_auto_create_task_worktree(
    ctx: RepoContext,
    *,
    task_id: str,
    title: str,
    base_line_name: str,
    change_id: str | None = None,
) -> dict[str, Any] | None:
    policy = _effective_task_worktree(ctx)
    repo_ctx = _task_worktree_repo_ctx(ctx)
    worktree_name = _resolve_task_bound_worktree_name(repo_ctx, task_id, title)

    def _create_and_bind() -> dict[str, Any]:
        feature_line = _ensure_task_feature_line(
            repo_ctx,
            task_id=task_id,
            base_line_name=base_line_name,
        )
        base_line = get_line(repo_ctx, base_line_name)
        worktree_location = resolve_task_auto_worktree_location(
            repo_ctx,
            worktree_name=worktree_name,
            ephemeral_root=policy["ephemeral_root"]["value"],
            alias_root=policy["alias_root"]["value"],
        )
        created = local_add_worktree(
            repo_ctx,
            worktree_name,
            line_name=str(feature_line.get("line_name") or _task_feature_line_name(task_id)),
            path=str(worktree_location["target_path"]),
            alias_path=str(worktree_location["alias_path"]) if worktree_location.get("alias_path") is not None else None,
            creation_kind="task_auto_created",
            cleanup_policy="after_remote_land",
            root_source=str(worktree_location["root_source"]),
        )
        return local_bind_worktree(
            repo_ctx,
            created["name"],
            task_id=task_id,
            change_id=change_id,
            auto_created_for_task=True,
            fork_snapshot_id=_normalize_text_value(base_line.get("head_snapshot_id")),
            forked_from_line=base_line_name,
            target_base_line=base_line_name,
        )

    worktree = _run_locked_workspace_command(repo_ctx, "task auto worktree add", _create_and_bind)
    if not ctx.is_worktree:
        _set_active_root_worktree_binding(repo_ctx, _normalize_text_value(worktree.get("name")))
    return _task_worktree_output(worktree)


def _task_ready_for_bound_worktree_cleanup(audit: dict[str, Any]) -> bool:
    task = audit.get("task") if isinstance(audit.get("task"), dict) else {}
    workflow = audit.get("workflow") if isinstance(audit.get("workflow"), dict) else {}
    summary = audit.get("summary") if isinstance(audit.get("summary"), dict) else {}
    if str(task.get("status") or "").strip() == "completed":
        return True
    if str(workflow.get("state") or "").strip() == "ready_to_complete":
        return True
    return int(summary.get("open_change_count") or 0) == 0 and not bool(summary.get("stale_workflow_records"))


def _bound_worktree_cleanup_next_step_command(repo_ctx: RepoContext) -> str:
    repo_root = str(repo_ctx.root.resolve())
    return f"cd {shlex.quote(repo_root)} && ait worktree cleanup --yes"


def _bound_worktree_matches_current_workspace(ctx: RepoContext, *, bound_worktree: Mapping[str, Any]) -> bool:
    bound_path_value = _normalize_text_value(bound_worktree.get("path")) or _normalize_text_value(
        bound_worktree.get("workspace_root")
    )
    if bound_path_value is None:
        return False
    return Path(bound_path_value).expanduser().resolve() == ctx.root.resolve()


def _current_worktree_cleanup_skip_payload(
    repo_ctx: RepoContext,
    *,
    task_id: str,
    bound_worktree: Mapping[str, Any],
) -> dict[str, Any]:
    return {
        "status": "skipped",
        "reason": "current_worktree",
        "task_id": task_id,
        "worktree_name": bound_worktree.get("name"),
        "next_step_command": _bound_worktree_cleanup_next_step_command(repo_ctx),
    }


def _clear_root_binding_for_completed_current_worktree(
    repo_ctx: RepoContext,
    *,
    bound_worktree: Mapping[str, Any],
) -> None:
    worktree_name = _normalize_text_value(bound_worktree.get("name"))
    if worktree_name is None:
        return
    _clear_active_root_worktree_binding(repo_ctx, worktree_name=worktree_name)


def _remove_bound_worktree_after_land(
    repo_ctx: RepoContext,
    *,
    bound_worktree: Mapping[str, Any],
    task_id: str,
) -> dict[str, Any]:
    worktree_name = str(bound_worktree.get("name") or "").strip()
    if not worktree_name:
        return {
            "status": "failed",
            "reason": "missing_worktree_name",
            "task_id": task_id,
        }
    if not bool(bound_worktree.get("clean")):
        try:
            pre_remove_sync = _run_locked_workspace_command(
                repo_ctx,
                "task auto worktree sync",
                lambda: local_sync_worktree(
                    repo_ctx,
                    worktree_name,
                    force=False,
                    dry_run=False,
                ),
            )
            bound_worktree = local_get_worktree(repo_ctx, worktree_name)
        except (KeyError, ValueError) as exc:
            return {
                "status": "skipped",
                "reason": "worktree_not_clean",
                "task_id": task_id,
                "worktree_name": worktree_name,
                "changed_count": bound_worktree.get("changed_count"),
                "detail": str(exc),
            }
        if not bool(bound_worktree.get("clean")):
            return {
                "status": "skipped",
                "reason": "worktree_not_clean",
                "task_id": task_id,
                "worktree_name": worktree_name,
                "changed_count": bound_worktree.get("changed_count"),
            }
    else:
        pre_remove_sync = None

    try:
        removed = _run_locked_workspace_command(
            repo_ctx,
            "task auto worktree remove",
            lambda: local_remove_worktree(
                repo_ctx,
                worktree_name,
                delete_path=True,
                force=False,
            ),
        )
    except (KeyError, ValueError) as exc:
        return {
            "status": "failed",
            "reason": "remove_failed",
            "task_id": task_id,
            "worktree_name": worktree_name,
            "detail": str(exc),
        }

    payload: dict[str, Any] = {
        "status": "removed",
        "task_id": task_id,
        "worktree": removed,
    }
    if pre_remove_sync is not None:
        payload["pre_remove_sync"] = pre_remove_sync
    return payload


def _auto_remove_bound_worktree_after_local_land(
    ctx: RepoContext,
    *,
    task_id: str,
    task_status: str | None,
    change_status: str | None,
) -> dict[str, Any]:
    resolved_task_status = str(task_status or "").strip()
    if resolved_task_status != "completed":
        return {
            "status": "skipped",
            "reason": "task_not_completed",
            "task_id": task_id,
            "task_status": resolved_task_status or None,
        }
    resolved_change_status = str(change_status or "").strip()
    if resolved_change_status != "landed":
        return {
            "status": "skipped",
            "reason": "change_not_landed",
            "task_id": task_id,
            "change_status": resolved_change_status or None,
        }
    repo_ctx = _task_worktree_repo_ctx(ctx)
    bound_worktree = _find_auto_created_task_worktree(repo_ctx, task_id)
    if bound_worktree is None:
        return {
            "status": "skipped",
            "reason": "no_bound_worktree",
            "task_id": task_id,
        }
    return _remove_bound_worktree_after_land(repo_ctx, bound_worktree=bound_worktree, task_id=task_id)


def _maybe_auto_remove_bound_worktree_after_task_complete(
    ctx: RepoContext,
    *,
    task_id: str,
    task_status: str | None,
) -> dict[str, Any] | None:
    resolved_task_status = str(task_status or "").strip()
    if resolved_task_status != "completed":
        return None

    repo_ctx = _task_worktree_repo_ctx(ctx)
    bound_worktree = _find_auto_created_task_worktree(repo_ctx, task_id)
    if bound_worktree is None:
        return {
            "status": "skipped",
            "reason": "no_bound_worktree",
            "task_id": task_id,
        }
    if _bound_worktree_matches_current_workspace(ctx, bound_worktree=bound_worktree):
        _clear_root_binding_for_completed_current_worktree(
            repo_ctx,
            bound_worktree=bound_worktree,
        )
        return _current_worktree_cleanup_skip_payload(
            repo_ctx,
            task_id=task_id,
            bound_worktree=bound_worktree,
        )

    binding_summary = bound_worktree.get("binding_summary") if isinstance(bound_worktree.get("binding_summary"), dict) else {}
    return _auto_remove_bound_worktree_after_local_land(
        ctx,
        task_id=task_id,
        task_status=resolved_task_status,
        change_status=_normalize_text_value(binding_summary.get("change_status")),
    )


def _maybe_auto_remove_bound_worktree_after_land(
    ctx: RepoContext,
    *,
    remote_name: str | None,
    change_id: str,
    land_result: dict[str, Any],
) -> dict[str, Any]:
    result = dict(land_result)
    if str(result.get("status") or "").strip() != "succeeded":
        return result

    remote_row, repo_name = _remote_tuple(ctx, remote_name)
    landed = result.get("result") if isinstance(result.get("result"), dict) else {}
    target_line = str(landed.get("target_line") or "main").strip() or "main"

    try:
        change = remote_get_change(remote_row["url"], change_id, repo_name=repo_name)
        task_id = str(change.get("task_id") or "").strip()
        if not task_id:
            raise KeyError(f"Change {change_id} has no linked task.")
        audit = remote_read_task_audit(remote_row["url"], task_id, target_line=target_line, repo_name=repo_name)
    except (KeyError, RemoteError, ValueError) as exc:
        result["bound_worktree_cleanup"] = {
            "status": "skipped",
            "reason": "task_audit_unavailable",
            "detail": str(exc),
        }
        return result

    if not _task_ready_for_bound_worktree_cleanup(audit):
        summary = audit.get("summary") if isinstance(audit.get("summary"), dict) else {}
        workflow = audit.get("workflow") if isinstance(audit.get("workflow"), dict) else {}
        result["bound_worktree_cleanup"] = {
            "status": "skipped",
            "reason": "task_not_ready",
            "task_id": task_id,
            "workflow_state": workflow.get("state"),
            "open_change_count": summary.get("open_change_count"),
        }
        return result

    repo_ctx = _task_worktree_repo_ctx(ctx)
    bound_worktree = _find_auto_created_task_worktree(repo_ctx, task_id)
    if bound_worktree is None:
        result["bound_worktree_cleanup"] = {
            "status": "skipped",
            "reason": "no_bound_worktree",
            "task_id": task_id,
        }
        return result

    if _bound_worktree_matches_current_workspace(ctx, bound_worktree=bound_worktree):
        result["bound_worktree_cleanup"] = _current_worktree_cleanup_skip_payload(
            repo_ctx,
            task_id=task_id,
            bound_worktree=bound_worktree,
        )
        return result
    result["bound_worktree_cleanup"] = _remove_bound_worktree_after_land(
        repo_ctx,
        bound_worktree=bound_worktree,
        task_id=task_id,
    )
    return result
