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
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_auth_whoami(&payload)?);
            }
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
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_auth_grant(&payload)?);
            }
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
            emit_config_show_result(&payload, args.json)?;
        }
        ConfigCommand::Set(args) => {
            let request = ConfigSetRequest {
                agent_teams: args.agent_teams,
                agent_team_size: args.agent_team_size,
                plan_language: args.plan_language,
                plan_style: args.plan_style,
                default_author_mode: args
                    .default_author_mode
                    .map(|value| value.as_str().to_string()),
                default_model: args.default_model,
                task_review: args.task_review.map(|value| value.as_str().to_string()),
                task_worktree_alias_root: args.task_worktree_alias_root,
                task_worktree_main_seed_ram_max_bytes: args.task_worktree_main_seed_ram_max_bytes,
                id_namespace_prefix: args.id_namespace_prefix,
                sprint: args.sprint.map(|value| value.as_str().to_string()),
                user_name: args.user_name,
                user_email: args.user_email,
                post_finish_hooks: args.post_finish_hooks,
            };
            let keys = request.updated_keys();
            let payload = crate::config_surface::config_set_with_team_bootstrap(&repo, &request)?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_configuration_with_keys(&payload, &keys)?
                );
            }
            if let Some(error) = crate::config_surface::team_setup_error(&payload) {
                return Err(error);
            }
        }
        ConfigCommand::Unset(args) => {
            let key = args.key.into_config_key();
            let payload = config_unset_cmd(&repo, key)?;
            emit_config_mutation_result(
                "ait-cli config unset",
                &payload,
                args.json,
                &[key.as_str()],
            )?;
        }
    }
    Ok(())
}

fn emit_config_show_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    println!("{}", config_show_text(payload)?);
    Ok(())
}

pub(crate) fn config_show_text(payload: &JsonValue) -> Result<String, String> {
    let task_review = payload
        .get("task_review")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "ait-cli config show payload is missing task_review.".to_string())?;
    let task_review_mode = string_field(task_review.get("value"));
    let automatic_task_reviewer = if task_review_mode == "automatic" {
        string_field_or_default(task_review.get("automatic_reviewer"), "<unset>")
    } else {
        "not_applicable".to_string()
    };
    let mut output = crate::render::key_values_text(
        "ait-cli config show",
        &[
            ("repo_name", string_field(payload.get("repo_name"))),
            (
                "plan_language",
                config_preference_label(payload, "plan_language"),
            ),
            ("plan_style", config_preference_label(payload, "plan_style")),
            (
                "workspace_root",
                string_field(payload.get("workspace_root")),
            ),
            ("current_line", string_field(payload.get("current_line"))),
            (
                "default_remote",
                string_field(payload.get("default_remote")),
            ),
            (
                "effective_actor",
                string_field(payload.get("effective_actor")),
            ),
            (
                "effective_reviewer",
                string_field(payload.get("effective_reviewer")),
            ),
            ("task_review", task_review_mode),
            (
                "task_review_source",
                string_field(task_review.get("source")),
            ),
            ("automatic_task_reviewer", automatic_task_reviewer),
        ],
    );
    if let Some(count) = payload["agent_team"]["configured_member_count"].as_u64() {
        output.push('\n');
        output.push_str(&crate::render::key_values_text(
            "Agent team",
            &[("members", count.to_string())],
        ));
    }
    Ok(output)
}

fn config_preference_label(payload: &JsonValue, key: &str) -> String {
    format!(
        "{} ({})",
        string_field(payload[key].get("value")),
        string_field(payload[key].get("source"))
    )
}

fn emit_config_mutation_result(
    title: &str,
    payload: &JsonValue,
    json_output: bool,
    keys: &[&str],
) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    println!("{}", config_mutation_text(title, payload, keys)?);
    Ok(())
}

pub(crate) fn config_mutation_text(
    title: &str,
    payload: &JsonValue,
    keys: &[&str],
) -> Result<String, String> {
    let owned_rows = keys
        .iter()
        .map(|key| ((*key).to_string(), config_effective_value(payload, key)))
        .collect::<Vec<_>>();
    let rows = owned_rows
        .iter()
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect::<Vec<_>>();
    Ok(crate::render::key_values_text(title, &rows))
}

fn config_effective_value(payload: &JsonValue, key: &str) -> String {
    let value = match key {
        "plan-language" => payload
            .get("plan_language")
            .and_then(|value| value.get("value")),
        "plan-style" => payload
            .get("plan_style")
            .and_then(|value| value.get("value")),
        "sprint" => payload.get("sprint").and_then(|value| value.get("value")),
        "default-author-mode" => payload.get("effective_author_mode"),
        "default-model" => payload.get("effective_model"),
        "task-review" => payload
            .get("task_review")
            .and_then(|value| value.get("value")),
        "task-worktree-alias-root" => payload
            .get("task_worktree")
            .and_then(|value| value.get("alias_root"))
            .and_then(|value| value.get("value")),
        "task-worktree-main-seed-ram-max-bytes" => payload
            .get("task_worktree")
            .and_then(|value| value.get("main_seed_ram_max_bytes"))
            .and_then(|value| value.get("value")),
        "id-namespace-prefix" => payload
            .get("id_namespace_prefix")
            .and_then(|value| value.get("value")),
        "user-name" => payload.get("user_name"),
        "user-email" => payload.get("user_email"),
        "post-finish-hooks" => payload
            .get("hooks")
            .and_then(|value| value.get("post_finish_count")),
        _ => None,
    };
    match value {
        None | Some(JsonValue::Null) => "<unset>".to_string(),
        Some(JsonValue::String(text)) if text.is_empty() => "<empty>".to_string(),
        Some(value) => string_field(Some(value)),
    }
}

fn run_status(repo: RepoRuntime, args: StatusArgs) -> Result<(), String> {
    let _command_range = perfetto_range!("ait.cli.status.command");
    let payload = {
        let _range = perfetto_range!("ait.cli.status.compute");
        repo_status_cmd(&repo)?
    };
    let _range = perfetto_range!("ait.cli.status.render");
    emit_status_result(&payload, args.json, args.full)
}

fn run_diff(repo: RepoRuntime, args: DiffArgs) -> Result<(), String> {
    let _command_range = perfetto_range!("ait.cli.diff.command");
    let payload = {
        let _range = perfetto_range!("ait.cli.diff.compute");
        workspace_dirty_diff(&repo, &args.paths, DEFAULT_SNAPSHOT_DIFF_MAX_BYTES)?
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
    print!("{}", workspace_diff_name_only_stdout(payload)?);
    Ok(())
}

pub(crate) fn workspace_diff_name_only_stdout(payload: &JsonValue) -> Result<String, String> {
    let mut output = String::new();
    let paths = payload
        .get("changed_paths")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "diff payload is missing changed_paths".to_string())?;
    for path in paths {
        if let Some(text) = path.as_str() {
            output.push_str(text);
            output.push('\n');
        }
    }
    Ok(output)
}

fn emit_diff_stat(payload: &JsonValue) -> Result<(), String> {
    print!("{}", workspace_diff_stat_stdout(payload)?);
    Ok(())
}

pub(crate) fn workspace_diff_stat_stdout(payload: &JsonValue) -> Result<String, String> {
    let mut output = String::new();
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
        output.push_str(&format!(
            "{path}\t+{insertions}\t-{deletions}\t{file_status}\n"
        ));
    }
    Ok(output)
}

fn emit_diff_text(payload: &JsonValue) -> Result<(), String> {
    print!("{}", workspace_diff_text_stdout(payload)?);
    Ok(())
}

pub(crate) fn workspace_diff_text_stdout(payload: &JsonValue) -> Result<String, String> {
    let mut output = String::new();
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
            output.push_str(text.trim_end());
            output.push('\n');
        } else if !path.is_empty() {
            output.push_str(&format!("diff --ait {path}\n"));
            output.push_str(&format!("# {file_status}: {diff_status}\n"));
        }
    }
    Ok(output)
}
fn run_pull(repo: RepoRuntime, args: PullArgs) -> Result<(), String> {
    let payload = crate::binding_services::pull_repository(
        &repo,
        &crate::binding_services::PullRequest {
            remote_name: args.remote,
            line_name: args.line,
            merge: args.merge,
            restore: args.restore,
            force: args.force,
        },
    )?;
    if args.json {
        print_json(&crate::binding_output::project_remote_sync(&payload))
    } else {
        println!("{}", crate::binding_output::format_pull(&payload)?);
        Ok(())
    }
}

fn run_push(repo: RepoRuntime, args: PushArgs) -> Result<(), String> {
    let payload = crate::binding_services::push_repository(
        &repo,
        &crate::binding_services::RemoteSyncRequest {
            remote_name: args.remote,
            line_name: args.line,
        },
    )?;
    if args.json {
        print_json(&crate::binding_output::project_remote_sync(&payload))
    } else {
        println!("{}", crate::binding_output::format_push(&payload)?);
        Ok(())
    }
}
