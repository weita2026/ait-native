from __future__ import annotations

from ...command_profiling import _normalize_command_profiling_mode
from ..runtime_inspection_views import _config_summary
from ..shared import export_app_namespace

export_app_namespace(globals())

@config_app.command("show")
def config_show(json_output: bool = typer.Option(False, "--json")):
    """Show the effective workflow mode, actor defaults, and advanced local overrides."""
    ctx = _ctx()
    _emit(_config_summary(ctx), json_output)


@config_app.command(
    "set",
    help="Set the primary workflow-mode preset or advanced local overrides.",
    short_help="Set workflow mode presets or advanced overrides.",
)
def config_set(
    default_author_mode: AuthorMode | None = typer.Option(None, "--default-author-mode", help="Set the default provenance author mode"),
    clear_default_author_mode: bool = typer.Option(False, "--clear-default-author-mode", help="Remove the stored default author mode"),
    default_model: Optional[str] = typer.Option(None, "--default-model", help="Set the default provenance model name"),
    clear_default_model: bool = typer.Option(False, "--clear-default-model", help="Remove the stored default model"),
    task_tracking: Optional[str] = typer.Option(None, "--task-tracking", help="Set task workflow tracking: on or off."),
    command_profiling: Optional[str] = typer.Option(
        None,
        "--command-profiling",
        help="Set command profiling artifact capture: on or off.",
    ),
    task_dag_allow_multi_worker: Optional[str] = typer.Option(
        None,
        "--task-dag-allow-multi-worker",
        help="Allow multi-worker DAG fan-out: on or off.",
    ),
    task_worktree_ephemeral_root: Optional[str] = typer.Option(
        None,
        "--task-worktree-ephemeral-root",
        help="Set an explicit filesystem root for ephemeral task worktrees.",
    ),
    clear_task_worktree_ephemeral_root: bool = typer.Option(
        False,
        "--clear-task-worktree-ephemeral-root",
        help="Remove the stored explicit ephemeral task-worktree root.",
    ),
    task_worktree_alias_root: Optional[str] = typer.Option(
        None,
        "--task-worktree-alias-root",
        help="Set the managed alias root for ephemeral task worktrees.",
    ),
    clear_task_worktree_alias_root: bool = typer.Option(
        False,
        "--clear-task-worktree-alias-root",
        help="Remove the stored managed alias-root override for task worktrees.",
    ),
    legacy_task_auto_worktree: Optional[str] = typer.Option(
        None,
        "--task-auto-worktree",
        help="Deprecated compatibility no-op; task-bound worktree bootstrap is always enabled.",
        hidden=True,
    ),
    legacy_clear_task_auto_worktree: bool = typer.Option(
        False,
        "--clear-task-auto-worktree",
        help="Deprecated compatibility no-op; task-bound worktree bootstrap is always enabled.",
        hidden=True,
    ),
    workflow_mode: Optional[str] = typer.Option(
        None,
        "--workflow-mode",
        help="Primary workflow preset selector: solo_local, solo_remote, or team_remote.",
    ),
    workflow_default_scope: Optional[str] = typer.Option(
        None,
        "--workflow-default-scope",
        help="Advanced override after workflow-mode presets for the default task/change workflow scope: local or remote.",
    ),
    clear_workflow_default_scope: bool = typer.Option(
        False,
        "--clear-workflow-default-scope",
        help="Remove the stored default task/change workflow scope.",
    ),
    task_default_scope: Optional[str] = typer.Option(
        None,
        "--task-default-scope",
        help="Advanced override after workflow-mode presets for the default task command scope: local or remote.",
    ),
    clear_task_default_scope: bool = typer.Option(
        False,
        "--clear-task-default-scope",
        help="Remove the stored default task command scope.",
    ),
    change_default_scope: Optional[str] = typer.Option(
        None,
        "--change-default-scope",
        help="Advanced override after workflow-mode presets for the default change command scope: local or remote.",
    ),
    clear_change_default_scope: bool = typer.Option(
        False,
        "--clear-change-default-scope",
        help="Remove the stored default change command scope.",
    ),
    id_namespace_prefix: Optional[str] = typer.Option(
        None,
        "--id-namespace-prefix",
        help="Set the optional namespace prefix used before workflow type codes such as T/C/P/PL/PR.",
    ),
    clear_id_namespace_prefix: bool = typer.Option(
        False,
        "--clear-id-namespace-prefix",
        help="Remove the stored namespace override and fall back to the default workflow namespace prefix.",
    ),
    plan_task_binding_mode: Optional[str] = typer.Option(
        None,
        "--plan-task-binding-mode",
        help="Advanced override after workflow-mode presets for staged repo-local plan/task binding mode: advisory, strict, or required.",
    ),
    clear_plan_task_binding: bool = typer.Option(
        False,
        "--clear-plan-task-binding",
        help="Remove stored plan/task binding overrides and fall back to staged defaults.",
    ),
    user_name: Optional[str] = typer.Option(None, "--user-name", help="Set the default local user/display name for review actions"),
    clear_user_name: bool = typer.Option(False, "--clear-user-name", help="Remove the stored local user/display name"),
    user_email: Optional[str] = typer.Option(None, "--user-email", help="Set the default local user email for review actions"),
    clear_user_email: bool = typer.Option(False, "--clear-user-email", help="Remove the stored local user email"),
    json_output: bool = typer.Option(False, "--json"),
):
    if default_author_mode is not None and clear_default_author_mode:
        raise typer.BadParameter("Choose either --default-author-mode or --clear-default-author-mode")
    if default_model is not None and clear_default_model:
        raise typer.BadParameter("Choose either --default-model or --clear-default-model")
    if legacy_task_auto_worktree is not None and legacy_clear_task_auto_worktree:
        raise typer.BadParameter("Choose either --task-auto-worktree or --clear-task-auto-worktree")
    if task_worktree_ephemeral_root is not None and clear_task_worktree_ephemeral_root:
        raise typer.BadParameter(
            "Choose either --task-worktree-ephemeral-root or --clear-task-worktree-ephemeral-root"
        )
    if task_worktree_alias_root is not None and clear_task_worktree_alias_root:
        raise typer.BadParameter("Choose either --task-worktree-alias-root or --clear-task-worktree-alias-root")
    if id_namespace_prefix is not None and clear_id_namespace_prefix:
        raise typer.BadParameter("Choose either --id-namespace-prefix or --clear-id-namespace-prefix")
    if workflow_default_scope is not None and clear_workflow_default_scope:
        raise typer.BadParameter("Choose either --workflow-default-scope or --clear-workflow-default-scope")
    if task_default_scope is not None and clear_task_default_scope:
        raise typer.BadParameter("Choose either --task-default-scope or --clear-task-default-scope")
    if change_default_scope is not None and clear_change_default_scope:
        raise typer.BadParameter("Choose either --change-default-scope or --clear-change-default-scope")
    if user_name is not None and clear_user_name:
        raise typer.BadParameter("Choose either --user-name or --clear-user-name")
    if user_email is not None and clear_user_email:
        raise typer.BadParameter("Choose either --user-email or --clear-user-email")
    if clear_plan_task_binding and plan_task_binding_mode is not None:
        raise typer.BadParameter(
            "Choose either --clear-plan-task-binding or explicit --plan-task-binding-* updates"
        )
    if workflow_mode is not None and (
        workflow_default_scope is not None
        or task_default_scope is not None
        or change_default_scope is not None
        or plan_task_binding_mode is not None
        or clear_workflow_default_scope
        or clear_task_default_scope
        or clear_change_default_scope
        or clear_plan_task_binding
    ):
        raise typer.BadParameter(
            "`--workflow-mode` cannot be combined with manual workflow scope or plan/task binding overrides."
        )
    if (
        default_author_mode is None
        and default_model is None
        and task_tracking is None
        and command_profiling is None
        and task_dag_allow_multi_worker is None
        and legacy_task_auto_worktree is None
        and task_worktree_ephemeral_root is None
        and task_worktree_alias_root is None
        and workflow_mode is None
        and workflow_default_scope is None
        and task_default_scope is None
        and change_default_scope is None
        and id_namespace_prefix is None
        and plan_task_binding_mode is None
        and user_name is None
        and user_email is None
        and not clear_default_author_mode
        and not clear_default_model
        and not legacy_clear_task_auto_worktree
        and not clear_task_worktree_ephemeral_root
        and not clear_task_worktree_alias_root
        and not clear_workflow_default_scope
        and not clear_task_default_scope
        and not clear_change_default_scope
        and not clear_id_namespace_prefix
        and not clear_plan_task_binding
        and not clear_user_name
        and not clear_user_email
    ):
        raise typer.BadParameter("No config updates specified")

    ctx = _ctx()
    cfg = load_config(ctx)
    if default_author_mode is not None:
        cfg["default_author_mode"] = default_author_mode.value
    elif clear_default_author_mode:
        cfg.pop("default_author_mode", None)

    if default_model is not None:
        cfg["default_model"] = _normalize_model_name(default_model)
    elif clear_default_model:
        cfg.pop("default_model", None)

    if cfg.get("default_model") is None:
        cfg.pop("default_model", None)

    if task_tracking is not None:
        cfg["task_tracking"] = _normalize_task_tracking_mode(task_tracking)
        if cfg["task_tracking"] == "off":
            for key in TRACKED_SESSION_CONFIG_KEYS:
                cfg.pop(key, None)
    if command_profiling is not None:
        cfg["command_profiling"] = _normalize_command_profiling_mode(command_profiling)

    task_dag_cfg = _task_dag_config(cfg)
    if task_dag_allow_multi_worker is not None:
        task_dag_cfg["allow_multi_worker"] = _normalize_task_dag_allow_multi_worker_mode(task_dag_allow_multi_worker)
    if task_dag_cfg:
        cfg["task_dag"] = task_dag_cfg
    else:
        cfg.pop("task_dag", None)

    if legacy_task_auto_worktree is not None:
        _normalize_toggle_mode(legacy_task_auto_worktree, option_name="`--task-auto-worktree`")

    task_worktree_cfg = _task_worktree_config(cfg)
    task_worktree_cfg.pop("auto_remove_after_remote_land", None)
    task_worktree_cfg.pop("root_mode", None)

    if clear_task_worktree_ephemeral_root:
        task_worktree_cfg.pop("ephemeral_root", None)
    elif task_worktree_ephemeral_root is not None:
        normalized_ephemeral_root = _normalize_text_value(task_worktree_ephemeral_root)
        if normalized_ephemeral_root is None:
            task_worktree_cfg.pop("ephemeral_root", None)
        else:
            task_worktree_cfg["ephemeral_root"] = normalized_ephemeral_root

    if clear_task_worktree_alias_root:
        task_worktree_cfg.pop("alias_root", None)
    elif task_worktree_alias_root is not None:
        normalized_alias_root = _normalize_text_value(task_worktree_alias_root)
        if normalized_alias_root is None:
            task_worktree_cfg.pop("alias_root", None)
        else:
            task_worktree_cfg["alias_root"] = normalized_alias_root

    if task_worktree_cfg:
        cfg["task_worktree"] = task_worktree_cfg
    else:
        cfg.pop("task_worktree", None)

    if workflow_mode is not None:
        resolved_workflow_mode = _normalize_workflow_mode(workflow_mode, option_name="--workflow-mode")
        preset = WORKFLOW_MODE_PRESETS[resolved_workflow_mode]
        cfg["workflow_mode"] = resolved_workflow_mode
        cfg["workflow_default_scope"] = preset["workflow_default_scope"]
        cfg["task_default_scope"] = preset["task_default_scope"]
        cfg["change_default_scope"] = preset["change_default_scope"]
        binding_cfg = _plan_task_binding_config(cfg)
        binding_cfg["mode"] = preset["plan_task_binding_mode"]
        cfg["plan_task_binding"] = binding_cfg
    else:
        if (
            clear_workflow_default_scope
            or workflow_default_scope is not None
            or clear_task_default_scope
            or task_default_scope is not None
            or clear_change_default_scope
            or change_default_scope is not None
            or clear_plan_task_binding
            or plan_task_binding_mode is not None
        ):
            cfg.pop("workflow_mode", None)
        if clear_workflow_default_scope:
            cfg.pop("workflow_default_scope", None)
        elif workflow_default_scope is not None:
            cfg["workflow_default_scope"] = _normalize_workflow_default_scope(
                workflow_default_scope,
                option_name="--workflow-default-scope",
            )

        if clear_task_default_scope:
            cfg.pop("task_default_scope", None)
        elif task_default_scope is not None:
            cfg["task_default_scope"] = _normalize_workflow_default_scope(
                task_default_scope,
                option_name="--task-default-scope",
            )

        if clear_change_default_scope:
            cfg.pop("change_default_scope", None)
        elif change_default_scope is not None:
            cfg["change_default_scope"] = _normalize_workflow_default_scope(
                change_default_scope,
                option_name="--change-default-scope",
            )

    if clear_id_namespace_prefix:
        cfg.pop("id_namespace_prefix", None)
    elif id_namespace_prefix is not None:
        cfg["id_namespace_prefix"] = _normalize_id_namespace_prefix_option(id_namespace_prefix)

    if workflow_mode is None:
        if clear_plan_task_binding:
            cfg.pop("plan_task_binding", None)
        elif plan_task_binding_mode is not None:
            binding_cfg = _plan_task_binding_config(cfg)
            binding_cfg["mode"] = _normalize_plan_task_binding_mode(plan_task_binding_mode)
            cfg["plan_task_binding"] = binding_cfg

    if user_name is not None:
        cfg["user_name"] = _normalize_text_value(user_name)
    elif clear_user_name:
        cfg.pop("user_name", None)

    if user_email is not None:
        cfg["user_email"] = _normalize_text_value(user_email)
    elif clear_user_email:
        cfg.pop("user_email", None)

    if cfg.get("user_name") is None:
        cfg.pop("user_name", None)
    if cfg.get("user_email") is None:
        cfg.pop("user_email", None)
    if cfg.get("id_namespace_prefix") is None:
        cfg.pop("id_namespace_prefix", None)
    for key in ("workflow_default_scope", "task_default_scope", "change_default_scope"):
        if cfg.get(key) is None:
            cfg.pop(key, None)
    if not _task_worktree_config(cfg):
        cfg.pop("task_worktree", None)
    if not _task_dag_config(cfg):
        cfg.pop("task_dag", None)

    save_config(ctx, cfg)
    _emit(_config_summary(ctx), json_output)


@app.command("status")
def status_cmd(json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    data = repo_status(ctx)
    try:
        data["default_remote"] = get_remote(ctx)["name"]
    except Exception:
        data["default_remote"] = None
    if json_output:
        _emit(data, True)
        return
    _render_repo_status(data)


@app.command("history")
def history_cmd(
    line_name: Optional[str] = typer.Option(None, "--line", help="Show history reachable from a specific line head."),
    limit: int = typer.Option(30, "--limit", help="Maximum number of snapshots to show. Use 0 for all."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        rows = _history_rows(ctx, None if limit <= 0 else limit, line_name=line_name)
    except KeyError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return

    current = current_line(ctx)
    title = f"ait history ({line_name})" if line_name else "ait history"
    table = Table(title=title)
    table.add_column("graph")
    table.add_column("snapshot_id")
    table.add_column("heads")
    table.add_column("line")
    table.add_column("parent")
    table.add_column("message")
    for row in rows:
        heads = [
            f"{line_name}*" if row["is_current_head"] and line_name == current else line_name
            for line_name in row["head_lines"]
        ]
        table.add_row(
            row["graph"],
            row["snapshot_id"],
            ", ".join(heads),
            row["line_name"],
            row["parent_snapshot_id"] or "",
            row.get("message") or "",
        )
    rprint(table)
