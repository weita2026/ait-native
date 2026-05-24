from __future__ import annotations

from typing import Any

import typer

from ait_protocol.common import DEFAULT_ID_NAMESPACE_PREFIX, normalize_id_namespace_prefix

from ..store import RepoContext, load_config
from ..task_worktree_layout import (
    DEFAULT_TASK_WORKTREE_ALIAS_ROOT,
    DEFAULT_TASK_WORKTREE_ROOT_MODE,
    TASK_WORKTREE_ROOT_MODES,
)

PLAN_TASK_BINDING_MODES = frozenset({"advisory", "strict", "required"})
DEFAULT_PLAN_TASK_BINDING = {"mode": "required"}
WORKFLOW_DEFAULT_SCOPES = frozenset({"local", "remote"})
DEFAULT_WORKFLOW_DEFAULT_SCOPE = "local"
DEFAULT_TASK_WORKFLOW_SCOPE = DEFAULT_WORKFLOW_DEFAULT_SCOPE
DEFAULT_CHANGE_WORKFLOW_SCOPE = DEFAULT_WORKFLOW_DEFAULT_SCOPE
DEFAULT_TASK_DAG = {"allow_multi_worker": "off"}
WORKFLOW_MODE_PRESETS: dict[str, dict[str, str]] = {
    "solo_local": {
        "workflow_default_scope": "local",
        "task_default_scope": "local",
        "change_default_scope": "local",
        "plan_task_binding_mode": "required",
        "dag_default": "local_execution_dag",
        "change_strategy": "promote_reviewable_outputs_late",
    },
    "solo_remote": {
        "workflow_default_scope": "remote",
        "task_default_scope": "remote",
        "change_default_scope": "remote",
        "plan_task_binding_mode": "required",
        "dag_default": "local_execution_dag_with_selective_promotion",
        "change_strategy": "remote_backed_selective_promotion",
    },
    "team_remote": {
        "workflow_default_scope": "remote",
        "task_default_scope": "remote",
        "change_default_scope": "remote",
        "plan_task_binding_mode": "required",
        "dag_default": "shared_workflow_dag",
        "change_strategy": "per_slice_reviewable_changes",
    },
}
WORKFLOW_MODES = frozenset(WORKFLOW_MODE_PRESETS)
LAND_AUTO_REMOVE_BOUND_WORKTREE_MODES = frozenset({"off", "when_task_complete_and_clean"})
DEFAULT_TASK_WORKTREE = {
    "auto_remove_after_remote_land": "off",
    "root_mode": DEFAULT_TASK_WORKTREE_ROOT_MODE,
    "ephemeral_root": None,
    "alias_root": DEFAULT_TASK_WORKTREE_ALIAS_ROOT,
}


def _normalize_text_value(value: str | None) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _normalize_toggle_mode(value: Any, *, option_name: str) -> str | None:
    text = _normalize_text_value(value)
    if text is None:
        return None
    lowered = text.lower()
    if lowered in {"on", "true", "yes"}:
        return "on"
    if lowered in {"off", "false", "no"}:
        return "off"
    raise typer.BadParameter(f"{option_name} must be `on` or `off`.")


def _normalize_task_tracking_mode(value: Any) -> str | None:
    return _normalize_toggle_mode(value, option_name="`--task-tracking`")


def _normalize_task_dag_allow_multi_worker_mode(value: Any) -> str | None:
    return _normalize_toggle_mode(value, option_name="`--task-dag-allow-multi-worker`")


def _normalize_plan_task_binding_mode(value: Any) -> str | None:
    text = _normalize_text_value(value)
    if text is None:
        return None
    lowered = text.lower()
    if lowered not in PLAN_TASK_BINDING_MODES:
        raise typer.BadParameter("`--plan-task-binding-mode` must be `advisory`, `strict`, or `required`.")
    return lowered


def _normalize_workflow_default_scope(value: Any, *, option_name: str) -> str | None:
    text = _normalize_text_value(value)
    if text is None:
        return None
    lowered = text.lower()
    if lowered not in WORKFLOW_DEFAULT_SCOPES:
        raise typer.BadParameter(f"{option_name} must be `local` or `remote`.")
    return lowered


def _normalize_workflow_mode(value: Any, *, option_name: str) -> str | None:
    text = _normalize_text_value(value)
    if text is None:
        return None
    lowered = text.lower()
    if lowered not in WORKFLOW_MODES:
        allowed = "`, `".join(sorted(WORKFLOW_MODES))
        raise typer.BadParameter(f"{option_name} must be `{allowed}`.")
    return lowered


def _normalize_id_namespace_prefix_option(value: Any) -> str | None:
    if value is None:
        return None
    try:
        return normalize_id_namespace_prefix(value, default=DEFAULT_ID_NAMESPACE_PREFIX)
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc


def _configured_id_namespace_prefix(cfg: dict[str, Any]) -> str | None:
    if "id_namespace_prefix" not in cfg:
        return None
    try:
        return normalize_id_namespace_prefix(cfg.get("id_namespace_prefix"), default=DEFAULT_ID_NAMESPACE_PREFIX)
    except ValueError:
        return DEFAULT_ID_NAMESPACE_PREFIX


def _effective_id_namespace_prefix(ctx: RepoContext) -> dict[str, Any]:
    configured = _configured_id_namespace_prefix(load_config(ctx))
    if configured is None:
        return {"value": DEFAULT_ID_NAMESPACE_PREFIX, "source": "default"}
    return {"value": configured, "source": "repo_config"}


def _plan_task_binding_config(cfg: dict[str, Any]) -> dict[str, Any]:
    raw = cfg.get("plan_task_binding")
    if not isinstance(raw, dict):
        return {}
    out: dict[str, Any] = {}
    try:
        mode = _normalize_plan_task_binding_mode(raw.get("mode"))
    except typer.BadParameter:
        mode = None
    if mode is not None:
        out["mode"] = mode
    return out


def _effective_plan_task_binding(ctx: RepoContext) -> dict[str, Any]:
    stored = _plan_task_binding_config(load_config(ctx))
    return {
        "mode": stored.get("mode", DEFAULT_PLAN_TASK_BINDING["mode"]),
        "source": "repo_config" if stored else "staged_default",
    }


def _task_dag_config(cfg: dict[str, Any]) -> dict[str, Any]:
    raw = cfg.get("task_dag")
    if not isinstance(raw, dict):
        return {}
    out: dict[str, Any] = {}
    try:
        allow_multi_worker = _normalize_task_dag_allow_multi_worker_mode(raw.get("allow_multi_worker"))
    except typer.BadParameter:
        allow_multi_worker = None
    if allow_multi_worker is not None:
        out["allow_multi_worker"] = allow_multi_worker
    return out


def _effective_task_dag(ctx: RepoContext) -> dict[str, Any]:
    stored = _task_dag_config(load_config(ctx))
    return {
        "allow_multi_worker": {
            "value": stored.get("allow_multi_worker", DEFAULT_TASK_DAG["allow_multi_worker"]),
            "source": "repo_config" if "allow_multi_worker" in stored else "built_in",
        }
    }


def _task_dag_multi_worker_allowed(ctx: RepoContext) -> bool:
    return str(_effective_task_dag(ctx)["allow_multi_worker"]["value"]) == "on"


def _configured_scope(cfg: dict[str, Any], key: str) -> str | None:
    try:
        return _normalize_workflow_default_scope(cfg.get(key), option_name=f"`{key}`")
    except typer.BadParameter:
        return None


def _configured_workflow_mode(cfg: dict[str, Any]) -> str | None:
    try:
        return _normalize_workflow_mode(cfg.get("workflow_mode"), option_name="`workflow_mode`")
    except typer.BadParameter:
        return None


def _effective_workflow_default_scope(ctx: RepoContext) -> dict[str, Any]:
    configured = _configured_scope(load_config(ctx), "workflow_default_scope")
    if configured is None:
        return {"value": DEFAULT_WORKFLOW_DEFAULT_SCOPE, "source": "built_in"}
    return {"value": configured, "source": "repo_config"}


def _effective_task_default_scope(ctx: RepoContext) -> dict[str, Any]:
    cfg = load_config(ctx)
    configured = _configured_scope(cfg, "task_default_scope")
    if configured is not None:
        return {"value": configured, "source": "repo_config"}
    workflow = _effective_workflow_default_scope(ctx)
    if workflow["source"] == "repo_config":
        return {"value": workflow["value"], "source": "workflow_default_scope"}
    return {"value": DEFAULT_TASK_WORKFLOW_SCOPE, "source": "built_in"}


def _effective_change_default_scope(ctx: RepoContext) -> dict[str, Any]:
    cfg = load_config(ctx)
    configured = _configured_scope(cfg, "change_default_scope")
    if configured is not None:
        return {"value": configured, "source": "repo_config"}
    workflow = _effective_workflow_default_scope(ctx)
    if workflow["source"] == "repo_config":
        return {"value": workflow["value"], "source": "workflow_default_scope"}
    return {"value": DEFAULT_CHANGE_WORKFLOW_SCOPE, "source": "built_in"}


def _workflow_default_scope_summary(ctx: RepoContext) -> dict[str, Any]:
    return {
        "workflow": _effective_workflow_default_scope(ctx),
        "task": _effective_task_default_scope(ctx),
        "change": _effective_change_default_scope(ctx),
    }


def _effective_workflow_mode(ctx: RepoContext) -> dict[str, Any]:
    cfg = load_config(ctx)
    scope_summary = _workflow_default_scope_summary(ctx)
    binding = _effective_plan_task_binding(ctx)
    workflow_scope = str(scope_summary["workflow"]["value"])
    task_scope = str(scope_summary["task"]["value"])
    change_scope = str(scope_summary["change"]["value"])
    binding_mode = str(binding["mode"])
    configured_mode = _configured_workflow_mode(cfg)

    if configured_mode is not None:
        preset = WORKFLOW_MODE_PRESETS[configured_mode]
        if (
            workflow_scope == preset["workflow_default_scope"]
            and task_scope == preset["task_default_scope"]
            and change_scope == preset["change_default_scope"]
            and binding_mode == preset["plan_task_binding_mode"]
        ):
            return {
                "value": configured_mode,
                "source": "repo_config",
                "dag_default": preset["dag_default"],
                "change_strategy": preset["change_strategy"],
            }

    mode_value = "custom"
    if workflow_scope == task_scope == change_scope == "local" and binding_mode == "required":
        mode_value = "solo_local"
    elif workflow_scope == task_scope == change_scope == "remote" and binding_mode == "advisory":
        mode_value = "solo_remote"
    elif workflow_scope == task_scope == change_scope == "remote" and binding_mode == "required":
        mode_value = "team_remote"

    if mode_value == "custom":
        return {
            "value": "custom",
            "source": "derived_from_effective_config",
            "dag_default": "custom",
            "change_strategy": "custom",
        }

    preset = WORKFLOW_MODE_PRESETS[mode_value]
    return {
        "value": mode_value,
        "source": "derived_from_effective_config",
        "dag_default": preset["dag_default"],
        "change_strategy": preset["change_strategy"],
    }


def _normalize_land_auto_remove_bound_worktree_mode(value: Any, *, option_name: str) -> str | None:
    text = _normalize_text_value(value)
    if text is None:
        return None
    lowered = text.lower()
    if lowered not in LAND_AUTO_REMOVE_BOUND_WORKTREE_MODES:
        raise typer.BadParameter(f"{option_name} must be `off` or `when_task_complete_and_clean`.")
    return lowered


def _normalize_task_worktree_root_mode(value: Any, *, option_name: str) -> str | None:
    text = _normalize_text_value(value)
    if text is None:
        return None
    lowered = text.lower()
    if lowered not in TASK_WORKTREE_ROOT_MODES:
        allowed = "`, `".join(sorted(TASK_WORKTREE_ROOT_MODES))
        raise typer.BadParameter(f"{option_name} must be `{allowed}`.")
    return lowered


def _task_worktree_config(cfg: dict[str, Any]) -> dict[str, Any]:
    raw = cfg.get("task_worktree")
    if not isinstance(raw, dict):
        return {}
    out: dict[str, Any] = {}
    try:
        auto_remove = _normalize_land_auto_remove_bound_worktree_mode(
            raw.get("auto_remove_after_remote_land"),
            option_name="`task_worktree.auto_remove_after_remote_land`",
        )
    except typer.BadParameter:
        auto_remove = None
    if auto_remove is not None:
        out["auto_remove_after_remote_land"] = auto_remove
    try:
        root_mode = _normalize_task_worktree_root_mode(
            raw.get("root_mode"),
            option_name="`task_worktree.root_mode`",
        )
    except typer.BadParameter:
        root_mode = None
    if root_mode is not None:
        out["root_mode"] = root_mode
    ephemeral_root = _normalize_text_value(raw.get("ephemeral_root"))
    if ephemeral_root is not None:
        out["ephemeral_root"] = ephemeral_root
    alias_root = _normalize_text_value(raw.get("alias_root"))
    if alias_root is not None:
        out["alias_root"] = alias_root
    return out


def _effective_task_worktree(ctx: RepoContext) -> dict[str, Any]:
    stored = _task_worktree_config(load_config(ctx))
    return {
        "auto_remove_after_remote_land": {
            "value": stored.get("auto_remove_after_remote_land", DEFAULT_TASK_WORKTREE["auto_remove_after_remote_land"]),
            "source": "repo_config" if "auto_remove_after_remote_land" in stored else "built_in",
        },
        "root_mode": {
            "value": stored.get("root_mode", DEFAULT_TASK_WORKTREE["root_mode"]),
            "source": "repo_config" if "root_mode" in stored else "built_in",
        },
        "ephemeral_root": {
            "value": stored.get("ephemeral_root", DEFAULT_TASK_WORKTREE["ephemeral_root"]),
            "source": "repo_config" if "ephemeral_root" in stored else "built_in",
        },
        "alias_root": {
            "value": stored.get("alias_root", DEFAULT_TASK_WORKTREE["alias_root"]),
            "source": "repo_config" if "alias_root" in stored else "built_in",
        },
    }


def _plan_task_binding_plan_item_error(mode: str) -> str:
    label = "Required" if mode == "required" else "Strict"
    return f"{label} plan/task binding requires `--plan-item-ref` whenever a task is linked to a plan."
