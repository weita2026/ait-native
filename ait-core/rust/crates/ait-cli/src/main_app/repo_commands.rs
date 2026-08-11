fn run_auth(repo: RepoRuntime, command: AuthCommand) -> Result<(), String> {
    match command {
        AuthCommand::Whoami(args) => {
            let payload = auth_whoami_cmd(
                &repo,
                &AuthRemoteRequest {
                    remote_name: args.remote,
                    repo_name: args.repo,
                },
            )?;
            emit_result(
                "ait-cli auth whoami",
                &payload,
                args.json,
                &[
                    "identity",
                    "actor_type",
                    "mode",
                    "repo_name",
                    "claimed_roles",
                    "claimed_repos",
                    "effective_roles",
                    "effective_repos",
                ],
            )?;
        }
        AuthCommand::Grant(args) => {
            let payload = auth_grant_cmd(
                &repo,
                &AuthGrantRequest {
                    remote_name: args.remote,
                    repo_name: args.repo,
                    actor_identity: args.actor,
                    roles: args.role,
                },
            )?;
            emit_result(
                "ait-cli auth grant",
                &payload,
                args.json,
                &["repo_name", "actor_identity", "roles"],
            )?;
        }
        AuthCommand::Bindings(args) => {
            let payload = auth_bindings_cmd(
                &repo,
                &AuthRemoteRequest {
                    remote_name: args.remote,
                    repo_name: args.repo,
                },
            )?;
            emit_auth_bindings_result(&payload, args.json)?;
        }
    }
    Ok(())
}

fn run_config(repo: RepoRuntime, command: ConfigCommand) -> Result<(), String> {
    match command {
        ConfigCommand::Show(args) => {
            let payload = config_show_cmd(&repo)?;
            emit_result(
                "ait-cli config show",
                &payload,
                args.json,
                &[
                    "repo_name",
                    "workspace_root",
                    "current_line",
                    "default_remote",
                    "effective_actor",
                    "effective_reviewer",
                ],
            )?;
        }
        ConfigCommand::Set(args) => {
            let payload = config_set_cmd(
                &repo,
                &ConfigSetRequest {
                    repository_index: args.repository_index,
                    clear_repository_index: args.clear_repository_index,
                    default_author_mode: args.default_author_mode,
                    clear_default_author_mode: args.clear_default_author_mode,
                    default_model: args.default_model,
                    clear_default_model: args.clear_default_model,
                    task_tracking: args.task_tracking,
                    task_review: args.task_review,
                    command_profiling: args.command_profiling,
                    task_worktree_alias_root: args.task_worktree_alias_root,
                    clear_task_worktree_alias_root: args.clear_task_worktree_alias_root,
                    task_worktree_main_seed_ram_max_bytes: args
                        .task_worktree_main_seed_ram_max_bytes,
                    clear_task_worktree_main_seed_ram_max_bytes: args
                        .clear_task_worktree_main_seed_ram_max_bytes,
                    legacy_task_auto_worktree: args.legacy_task_auto_worktree,
                    legacy_clear_task_auto_worktree: args.legacy_clear_task_auto_worktree,
                    workflow_mode: args.workflow_mode,
                    workflow_default_scope: args.workflow_default_scope,
                    clear_workflow_default_scope: args.clear_workflow_default_scope,
                    task_default_scope: args.task_default_scope,
                    clear_task_default_scope: args.clear_task_default_scope,
                    change_default_scope: args.change_default_scope,
                    clear_change_default_scope: args.clear_change_default_scope,
                    id_namespace_prefix: args.id_namespace_prefix,
                    clear_id_namespace_prefix: args.clear_id_namespace_prefix,
                    sprint: args.sprint,
                    plan_task_binding_mode: args.plan_task_binding_mode,
                    clear_plan_task_binding: args.clear_plan_task_binding,
                    user_name: args.user_name,
                    clear_user_name: args.clear_user_name,
                    user_email: args.user_email,
                    clear_user_email: args.clear_user_email,
                },
            )?;
            emit_result(
                "ait-cli config set",
                &payload,
                args.json,
                &[
                    "repo_name",
                    "workspace_root",
                    "current_line",
                    "default_remote",
                    "effective_actor",
                    "effective_reviewer",
                ],
            )?;
        }
    }
    Ok(())
}

fn run_status(repo: RepoRuntime, args: StatusArgs) -> Result<(), String> {
    let _command_range = perfetto_range!("ait.cli.status.command");
    let payload = {
        let _range = perfetto_range!("ait.cli.status.compute");
        repo_status_cmd(&repo)?
    };
    let _range = perfetto_range!("ait.cli.status.render");
    emit_status_result(&payload, args.json, args.verbose)
}

fn run_diff(repo: RepoRuntime, args: DiffArgs) -> Result<(), String> {
    let _command_range = perfetto_range!("ait.cli.diff.command");
    let mut paths = args.paths.clone();
    paths.extend(args.trailing_paths.clone());
    let payload = {
        let _range = perfetto_range!("ait.cli.diff.compute");
        workspace_dirty_diff(&repo, &paths, args.max_bytes)?
    };
    let _range = perfetto_range!("ait.cli.diff.render");
    if args.json {
        return print_json(&payload);
    }
    if args.name_only {
        return emit_diff_name_only(&payload);
    }
    if args.stat {
        return emit_diff_stat(&payload);
    }
    emit_diff_text(&payload)
}

fn emit_diff_name_only(payload: &JsonValue) -> Result<(), String> {
    let paths = payload
        .get("changed_paths")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "diff payload is missing changed_paths".to_string())?;
    for path in paths {
        if let Some(text) = path.as_str() {
            println!("{text}");
        }
    }
    Ok(())
}

fn emit_diff_stat(payload: &JsonValue) -> Result<(), String> {
    let files = payload
        .get("files")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "diff payload is missing files".to_string())?;
    for file in files {
        let Some(file_obj) = file.as_object() else {
            return Err("diff file row must be an object".to_string());
        };
        let path = string_field(file_obj.get("path"));
        let file_status = string_field(file_obj.get("status"));
        let diff = file_obj.get("diff").and_then(JsonValue::as_object);
        let insertions = diff
            .and_then(|value| value.get("insertions"))
            .map(|value| string_field(Some(value)))
            .unwrap_or_else(|| "0".to_string());
        let deletions = diff
            .and_then(|value| value.get("deletions"))
            .map(|value| string_field(Some(value)))
            .unwrap_or_else(|| "0".to_string());
        println!("{path}\t+{insertions}\t-{deletions}\t{file_status}");
    }
    Ok(())
}

fn emit_diff_text(payload: &JsonValue) -> Result<(), String> {
    let files = payload
        .get("files")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "diff payload is missing files".to_string())?;
    for file in files {
        let Some(file_obj) = file.as_object() else {
            return Err("diff file row must be an object".to_string());
        };
        let path = string_field(file_obj.get("path"));
        let file_status = string_field(file_obj.get("status"));
        let diff = file_obj.get("diff").and_then(JsonValue::as_object);
        let diff_status = diff
            .and_then(|value| value.get("status"))
            .map(|value| string_field(Some(value)))
            .unwrap_or_else(|| "unavailable".to_string());
        let text = diff
            .and_then(|value| value.get("text"))
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        if !text.is_empty() {
            println!("{}", text.trim_end());
        } else if !path.is_empty() {
            println!("diff --ait {path}");
            println!("# {file_status}: {diff_status}");
        }
    }
    Ok(())
}

fn run_pull(repo: RepoRuntime, args: PullArgs) -> Result<(), String> {
    let payload = run_locked_workspace_command(&repo, "ait-cli pull", || {
        pull_cmd(
            &repo,
            args.remote.as_deref(),
            args.line.as_deref(),
            args.merge,
            args.restore,
            args.force,
        )
    })?;
    let mut output = payload.clone();
    if args.json && !json_mode_debug_enabled() {
        if let Some(object) = output.as_object_mut() {
            object.remove("phase_timings_ms");
            object.remove("remote_sync_metrics");
        }
    }
    emit_result(
        "ait-cli pull",
        &output,
        args.json,
        &[
            "remote",
            "repo_name",
            "mode",
            "line",
            "relationship",
            "action",
            "local_line_present",
            "local_line_head_snapshot_id",
            "imported_snapshots",
            "head_snapshot_id",
            "line_head_updated",
            "workspace_restored",
            "restore_applied",
        ],
    )
}

fn run_push(repo: RepoRuntime, args: PushArgs) -> Result<(), String> {
    let payload = run_locked_workspace_command(&repo, "ait-cli push", || {
        push_cmd(&repo, args.remote.as_deref(), args.line.as_deref())
    })?;
    let mut output = payload.clone();
    if args.json && !json_mode_debug_enabled() {
        if let Some(object) = output.as_object_mut() {
            object.remove("phase_timings_ms");
            object.remove("remote_sync_metrics");
        }
    }
    emit_result(
        "ait-cli push",
        &output,
        args.json,
        &[
            "remote",
            "repo_name",
            "line",
            "pushed_snapshots",
            "checked_snapshots",
            "uploaded_snapshots",
            "skipped_snapshots",
            "head_snapshot_id",
            "sync_scope",
            "sync_reason",
            "bounded_by_snapshot_id",
        ],
    )
}
