from __future__ import annotations

from ..shared import export_app_namespace

export_app_namespace(globals())


@worktree_app.command(
    "show",
    help="Inspect one registered worktree and its local workspace state.",
    short_help="Inspect one worktree.",
)
def worktree_show(name: Optional[str] = typer.Argument(None), json_output: bool = typer.Option(False, "--json")):
    try:
        data = local_get_worktree(_ctx(), name)
    except (KeyError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@worktree_app.command(
    "open",
    help="Print a worktree path or shell-open helpers for entering it.",
    short_help="Print shell-open helpers.",
)
@worktree_app.command(
    "path",
    help="Print a worktree path or shell-open helpers for entering it.",
    short_help="Print a worktree path.",
)
def worktree_path(
    name: Optional[str] = typer.Argument(None),
    shell_output: bool = typer.Option(False, "--shell", help="Print shell commands that enter the worktree and export its runtime environment"),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        ctx = _ctx()
        data = local_get_worktree(ctx, name)
        touched = _touch_worktree_usage_safely(ctx, name=name)
        if touched is not None:
            data = touched
    except (KeyError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    canonical_path = str(data.get("path") or "")
    open_path_value = str(data.get("open_path") or data.get("alias_path") or canonical_path)
    if not canonical_path:
        raise typer.BadParameter("Worktree has no registered path.")
    cd_command = f"cd {shlex.quote(open_path_value)}"
    runtime_paths = _worktree_runtime_paths(open_path_value)
    shell_command = _worktree_shell_command(open_path_value, data)
    if json_output:
        _emit(
            {
                "name": data.get("name"),
                "path": canonical_path,
                "open_path": open_path_value,
                "alias_path": data.get("alias_path"),
                "cd_command": cd_command,
                "shell_command": shell_command,
                "current_line": data.get("current_line"),
                "workspace_status": data.get("workspace_status"),
                "changed_count": data.get("changed_count"),
                "src_path": runtime_paths.get("src_path"),
                "venv_path": runtime_paths.get("venv_path"),
                "venv_bin_path": runtime_paths.get("venv_bin_path"),
            },
            True,
        )
        return
    typer.echo(shell_command if shell_output else open_path_value)


@worktree_app.command(
    "exec",
    help="Run one command inside a named worktree from the repo root.",
    short_help="Run a command in a worktree.",
)
def worktree_exec(
    name: str = typer.Argument(..., help="Registered worktree name"),
    command: list[str] = typer.Argument(..., help="Command to run inside the worktree. Pass it after -- when forwarding flags."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        ctx = _ctx()
        data = local_get_worktree(ctx, name)
    except (KeyError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _touch_worktree_usage_safely(ctx, name=name)
    path_value = str(data.get("path") or "")
    if not path_value:
        raise typer.BadParameter("Worktree has no registered path.")
    worktree_path = Path(path_value)
    if not worktree_path.is_dir():
        raise typer.BadParameter(f"Worktree path is missing: {path_value}")
    env = _worktree_runtime_env(path_value, {"name": data.get("name") or name, "current_line": data.get("current_line")})
    try:
        completed = subprocess.run(command, cwd=path_value, env=env, capture_output=True, text=True)
    except FileNotFoundError as exc:
        raise typer.BadParameter(f"Command not found: {command[0]}") from exc
    payload = {
        "name": data.get("name") or name,
        "path": path_value,
        "current_line": data.get("current_line"),
        "command": command,
        "returncode": completed.returncode,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
    }
    if json_output:
        _emit(payload, True)
    else:
        if completed.stdout:
            typer.echo(completed.stdout, nl=False)
        if completed.stderr:
            typer.echo(completed.stderr, nl=False, err=True)
    if completed.returncode != 0:
        raise typer.Exit(code=completed.returncode)


@worktree_app.command(
    "doctor",
    help="Inspect registered worktree health and stale entries.",
    short_help="Check worktree health.",
)
def worktree_doctor_cmd(json_output: bool = typer.Option(False, "--json")):
    data = local_worktree_doctor(_ctx())
    if json_output:
        _emit(data, True)
        return
    _render_worktree_doctor(data)


@worktree_app.command(
    "cleanup-candidates",
    help="Inspect lifecycle-aware cleanup candidates without deleting anything.",
    short_help="List cleanup candidates.",
)
def worktree_cleanup_candidates(
    older_than: str = typer.Option("7d", "--older-than", help="Idle threshold like 7d, 12h, or 30m."),
    cleanup_policy: Optional[str] = typer.Option(None, "--policy", help="Filter to one cleanup policy."),
    allow_manual_only: bool = typer.Option(
        False,
        "--allow-manual-only",
        help="Also surface clean manual_only worktrees that are only blocked by explicit operator intent.",
    ),
    include_protected: bool = typer.Option(False, "--include-protected", help="Also list protected rows and their reasons."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        data = local_list_worktree_cleanup_candidates(
            _ctx(),
            older_than=older_than,
            cleanup_policy=cleanup_policy,
            include_protected=include_protected,
            allow_manual_only=allow_manual_only,
        )
    except (KeyError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_worktree_cleanup_candidates(data)


@worktree_app.command(
    "cleanup",
    help="Remove lifecycle-approved cleanup candidates after an explicit confirmation.",
    short_help="Clean up worktrees.",
)
def worktree_cleanup(
    older_than: str = typer.Option("7d", "--older-than", help="Idle threshold like 7d, 12h, or 30m."),
    cleanup_policy: Optional[str] = typer.Option(None, "--policy", help="Filter to one cleanup policy."),
    allow_manual_only: bool = typer.Option(
        False,
        "--allow-manual-only",
        help="Explicitly include clean manual_only worktrees that are otherwise safe to remove.",
    ),
    limit: Optional[int] = typer.Option(None, "--limit", min=1, help="Only clean the first N selected candidates."),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview cleanup without deleting anything."),
    yes: bool = typer.Option(False, "--yes", help="Confirm lifecycle cleanup."),
    json_output: bool = typer.Option(False, "--json"),
):
    if not dry_run and not yes:
        raise typer.BadParameter("Pass --yes to apply worktree cleanup, or use --dry-run to preview it.")
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "worktree cleanup",
            lambda: local_cleanup_worktrees(
                ctx,
                older_than=older_than,
                cleanup_policy=cleanup_policy,
                allow_manual_only=allow_manual_only,
                limit=limit,
                dry_run=dry_run,
            ),
        )
    except (KeyError, ValueError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_worktree_cleanup_report(data)


@worktree_app.command(
    "prune-stale",
    help="Remove stale worktree registrations that no longer exist on disk.",
    short_help="Prune stale worktrees.",
)
def worktree_prune_stale(
    dry_run: bool = typer.Option(False, "--dry-run", help="Show which stale worktree registrations would be removed"),
    json_output: bool = typer.Option(False, "--json"),
):
    data = local_prune_stale_worktrees(_ctx(), dry_run=dry_run)
    if json_output:
        _emit(data, True)
        return
    _render_worktree_prune_report(data)


@worktree_app.command(
    "list",
    help="List registered worktrees. Use --refresh to verify current filesystem status before printing.",
    short_help="List worktrees.",
)
def worktree_list(
    json_output: bool = typer.Option(False, "--json"),
    refresh: bool = typer.Option(
        False,
        "--refresh",
        help="Verify each worktree against the live filesystem before printing and refresh the cached status fields.",
    ),
):
    rows = local_list_worktrees(_ctx(), refresh_status=refresh)
    if json_output:
        _emit(rows, True)
        return
    _render_worktrees(rows)


@worktree_app.command(
    "sync",
    help="Restore one worktree, or all live worktrees, to their intended line heads.",
    short_help="Sync worktrees.",
)
def worktree_sync(
    name: Optional[str] = typer.Argument(None),
    all_worktrees: bool = typer.Option(False, "--all", help="Sync every live registered worktree to its own current line"),
    line_name: Optional[str] = typer.Option(None, "--line", help="Restore a specific line head into the target worktree"),
    force: bool = typer.Option(False, "--force", help="Overwrite unsaved changes inside the target worktree"),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview restore actions without writing workspace files"),
    json_output: bool = typer.Option(False, "--json"),
):
    if all_worktrees and name is not None:
        raise typer.BadParameter("Choose either a worktree name or --all")
    if all_worktrees and line_name is not None:
        raise typer.BadParameter("--line cannot be combined with --all; each worktree syncs to its own current line")
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "worktree sync",
            lambda: local_sync_all_worktrees(ctx, force=force, dry_run=dry_run)
            if all_worktrees
            else local_sync_worktree(ctx, name, line_name=line_name, force=force, dry_run=dry_run),
        )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
    elif all_worktrees:
        _render_worktree_sync_report(data)
    else:
        _emit(data, False)
    if all_worktrees and not data.get("ok", True):
        raise typer.Exit(code=2)


@worktree_app.command(
    "recreate",
    help="Recreate one missing task-bound worktree from durable local/remote lineage.",
    short_help="Recreate a missing worktree.",
)
def worktree_recreate(
    name: Optional[str] = typer.Argument(None),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview the recreate source and target paths without writing files"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "worktree recreate",
            lambda: local_recreate_worktree(ctx, name, dry_run=dry_run),
        )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@worktree_app.command(
    "restore-owned-head",
    help="Restore one task-bound worktree to the last clean owned snapshot before foreign lineage contamination.",
    short_help="Restore a clean owned head.",
)
def worktree_restore_owned_head(
    name: Optional[str] = typer.Argument(None),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview the restore-owned-head plan without changing the workspace"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "worktree restore-owned-head",
            lambda: local_restore_owned_head(ctx, name, dry_run=dry_run),
        )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@worktree_app.command(
    "rebase",
    help="Retarget one task worktree onto a newer base line without reopening the worktree path.",
    short_help="Rebase a worktree.",
)
def worktree_rebase(
    name: Optional[str] = typer.Argument(None),
    onto_line: Optional[str] = typer.Option(None, "--onto", help="Base line whose current head should become the new fork point"),
    continue_rebase: bool = typer.Option(False, "--continue", help="Finalize a conflicted worktree rebase after resolving files"),
    abort_rebase: bool = typer.Option(False, "--abort", help="Abort a conflicted worktree rebase and restore the original head"),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview the rebase plan without changing the workspace"),
    json_output: bool = typer.Option(False, "--json"),
):
    if continue_rebase and abort_rebase:
        raise typer.BadParameter("Choose either --continue or --abort")
    if dry_run and (continue_rebase or abort_rebase):
        raise typer.BadParameter("--dry-run cannot be combined with --continue or --abort")
    ctx = _ctx()
    try:
        if continue_rebase:
            data = _run_locked_workspace_command(
                ctx,
                "worktree rebase --continue",
                lambda: local_continue_worktree_rebase(ctx, name),
            )
        elif abort_rebase:
            data = _run_locked_workspace_command(
                ctx,
                "worktree rebase --abort",
                lambda: local_abort_worktree_rebase(ctx, name),
            )
        else:
            data = _run_locked_workspace_command(
                ctx,
                "worktree rebase",
                lambda: local_preview_worktree_rebase(ctx, name, onto_line_name=onto_line)
                if dry_run
                else local_rebase_worktree(ctx, name, onto_line_name=onto_line),
            )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@worktree_app.command(
    "remove",
    help="Remove one or more worktree registrations and optionally delete their paths.",
    short_help="Remove worktrees.",
)
def worktree_remove(
    names: list[str] = typer.Argument(None),
    all_stale: bool = typer.Option(False, "--all-stale", help="Remove every stale worktree registration from the registry"),
    delete_path: bool = typer.Option(False, "--delete-path", help="Also delete the worktree directory after unregistering it"),
    force: bool = typer.Option(False, "--force", help="Allow removing a dirty worktree"),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview the selected worktree removals without writing changes"),
    json_output: bool = typer.Option(False, "--json"),
):
    if all_stale and names:
        raise typer.BadParameter("Choose either one or more worktree names or --all-stale")
    if not all_stale and not names:
        raise typer.BadParameter("Provide one or more worktree names or use --all-stale")
    if all_stale and delete_path:
        raise typer.BadParameter("--delete-path cannot be combined with --all-stale")
    if all_stale and force:
        raise typer.BadParameter("--force cannot be combined with --all-stale")
    ctx = _ctx()
    try:
        data = _run_locked_workspace_command(
            ctx,
            "worktree remove",
            lambda: local_prune_stale_worktrees(ctx, dry_run=dry_run)
            if all_stale
            else local_remove_worktrees(ctx, names, delete_path=delete_path, force=force, dry_run=dry_run)
            if len(names) > 1 or dry_run
            else local_remove_worktree(ctx, names[0], delete_path=delete_path, force=force),
        )
    except (KeyError, ValueError, IsADirectoryError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    if all_stale:
        _render_worktree_prune_report(data)
        return
    _emit(data, False)
