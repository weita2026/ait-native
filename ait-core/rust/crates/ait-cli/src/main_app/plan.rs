const PLAN_BINARY_DB_WRITE_LAYOUT: u32 = 1;

fn run_plan(repo: RepoRuntime, command: PlanCommand) -> Result<(), String> {
    match command {
        PlanCommand::List(args) => {
            let payload = execute_plan_list_command_request_json(&build_query_request(
                &repo,
                &args.scope,
            )?)?;
            if args.scope.json {
                return print_json(&payload);
            }
            let rows = expect_array(&payload)?;
            let all_command = scoped_all_command(
                "ait plan list",
                args.scope.local,
                args.scope.remote.as_deref(),
            );
            print_agent_list(
                &rows,
                &[
                    "plan_id",
                    "status",
                    "publication_state",
                    "title",
                ],
                args.all,
                &["archived"],
                Some("active"),
                &all_command,
            );
            Ok(())
        }
        PlanCommand::Show(args) => {
            let payload =
                execute_plan_show_command_request_json(&build_show_request(&repo, &args)?)?;
            if args.scope.json {
                return print_json(&payload);
            }
            render_show_like("ait-cli plan show", &payload)
        }
        PlanCommand::Revisions(args) => {
            let payload =
                execute_plan_revisions_command_request_json(&build_plan_id_request(&repo, &args)?)?;
            if args.scope.json {
                return print_json(&payload);
            }
            let rows = expect_array(&payload)?;
            let mut all_command = format!("ait plan revisions {} --all", args.plan_id);
            if args.scope.local {
                all_command.push_str(" --local");
            } else if let Some(remote) = args.scope.remote.as_deref() {
                all_command.push_str(&format!(" --remote {remote}"));
            }
            print_key_values(
                &format!("ait plan revisions {}", args.plan_id),
                &[
                    ("revisions", rows.len().to_string()),
                    ("order", "newest first".to_string()),
                ],
            );
            if !rows.is_empty() {
                println!();
                println!("history");
            }
            print_bounded_evidence(
                &rows,
                &[
                    "revision_number",
                    "plan_revision_id",
                    "title_snapshot",
                    "summary",
                    "created_at",
                    "publication_state",
                ],
                args.all,
                DEFAULT_AGENT_TEXT_LIST_LIMIT,
                &all_command,
            );
            Ok(())
        }
        PlanCommand::Items(args) => {
            let payload =
                execute_plan_items_command_request_json(&build_show_request(&repo, &args)?)?;
            if args.scope.json {
                return print_json(&payload);
            }
            render_items_like("ait-cli plan items", &payload)
        }
        PlanCommand::Candidates(args) => {
            let payload = execute_plan_candidates_command_request_json(&build_candidates_request(
                &repo, &args,
            )?)?;
            if args.json {
                return print_json(&payload);
            }
            render_candidates_like(&payload)
        }
        PlanCommand::Inspect(args) => {
            let payload =
                execute_plan_inspect_command_request_json(&build_show_request(&repo, &args)?)?;
            if args.scope.json {
                return print_json(&payload);
            }
            render_inspect_like(&payload)
        }
        PlanCommand::Sync(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli plan sync", || {
                execute_plan_sync_command_request_json(&build_sync_request(&repo, &args)?)
            })?;
            if args.json {
                return print_json(&payload);
            }
            render_sync_like(&payload)
        }
    }
}

fn build_query_request(repo: &RepoRuntime, args: &QueryScopeArgs) -> Result<String, String> {
    validate_scope(args.local, args.remote.as_deref())?;
    if let Some(remote_name) = args.remote.as_deref() {
        let remote = repo.remote_row(Some(remote_name))?;
        return Ok(json!({
            "scope": "remote",
            "base_url": remote.url,
            "repository_index": repo.repository_index(),
            "repo_name": remote.repo_name.unwrap_or_else(|| repo.repo_name()),
            "remote": remote.name,
            "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
        })
        .to_string());
    }
    Ok(json!({
        "scope": "local",
        "repository_index": repo.repository_index(),
        "repo_name": repo.repo_name(),
        "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
    })
    .to_string())
}

use crate::json_support::parse_value_error_string;

fn build_plan_id_request(repo: &RepoRuntime, args: &PlanIdArgs) -> Result<String, String> {
    let mut payload = parse_value_error_string(&build_query_request(repo, &args.scope)?)?;
    let obj = payload
        .as_object_mut()
        .ok_or("Plan request payload must be an object.")?;
    obj.insert(
        "plan_id".to_string(),
        JsonValue::String(args.plan_id.clone()),
    );
    Ok(payload.to_string())
}

fn build_show_request(repo: &RepoRuntime, args: &ShowArgs) -> Result<String, String> {
    let mut payload = parse_value_error_string(&build_query_request(repo, &args.scope)?)?;
    let obj = payload
        .as_object_mut()
        .ok_or("Plan request payload must be an object.")?;
    obj.insert(
        "plan_id".to_string(),
        JsonValue::String(args.plan_id.clone()),
    );
    obj.insert(
        "revision".to_string(),
        args.revision
            .clone()
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    Ok(payload.to_string())
}

fn build_candidates_request(repo: &RepoRuntime, args: &CandidatesArgs) -> Result<String, String> {
    validate_scope(args.local, args.remote.as_deref())?;
    let contains_terms = args
        .contains
        .as_deref()
        .map(parse_contains_terms)
        .transpose()?
        .unwrap_or_default();
    let mut payload = if let Some(remote_name) = args.remote.as_deref() {
        let remote = repo.remote_row(Some(remote_name))?;
        json!({
            "scope": "remote",
            "base_url": remote.url,
            "repository_index": repo.repository_index(),
            "repo_name": remote.repo_name.unwrap_or_else(|| repo.repo_name()),
            "remote": remote.name,
        })
    } else {
        json!({
            "scope": "local",
            "repository_index": repo.repository_index(),
            "repo_name": repo.repo_name(),
        })
    };
    let obj = payload
        .as_object_mut()
        .ok_or("Candidates request payload must be an object.")?;
    obj.insert(
        "plan_storage".to_string(),
        repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
    );
    obj.insert("include_all".to_string(), JsonValue::Bool(args.include_all));
    obj.insert(
        "contains_terms".to_string(),
        JsonValue::Array(contains_terms.into_iter().map(JsonValue::String).collect()),
    );
    Ok(payload.to_string())
}

fn build_sync_request(repo: &RepoRuntime, args: &SyncArgs) -> Result<String, String> {
    validate_scope(args.local, args.remote.as_deref())?;
    if args.rebase && args.reconcile {
        return Err("--rebase cannot be combined with --reconcile".to_string());
    }
    let mut payload = json!({
        "root_path": repo.root,
        "repo_name": repo.repo_name(),
        "repository_index": repo.repository_index(),
        "id_namespace_prefix": repo.id_namespace_prefix(),
        "created_by": repo.actor_identity(),
        "target": args.target,
        "plan_ref": args.plan_ref,
        "prune": args.prune,
        "local": args.local,
        "remote_name": JsonValue::Null,
        "remote_repo_name": JsonValue::Null,
        "base_url": JsonValue::Null,
        "rebase": args.rebase,
        "reconcile": args.reconcile,
        "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
    });
    if let Some(remote_name) = args.remote.as_deref() {
        let remote = repo.remote_row(Some(remote_name))?;
        let obj = payload
            .as_object_mut()
            .ok_or("Sync request payload must be an object.")?;
        obj.insert("remote_name".to_string(), JsonValue::String(remote.name));
        obj.insert(
            "remote_repo_name".to_string(),
            remote
                .repo_name
                .map(JsonValue::String)
                .unwrap_or_else(|| JsonValue::String(repo.repo_name())),
        );
        obj.insert("base_url".to_string(), JsonValue::String(remote.url));
    }
    Ok(payload.to_string())
}

fn validate_scope(local: bool, remote_name: Option<&str>) -> Result<(), String> {
    if local && remote_name.is_some() {
        return Err("--local cannot be combined with --remote".to_string());
    }
    Ok(())
}

fn resolve_task_land_scope(
    repo: &RepoRuntime,
    local: bool,
    remote_name: Option<&str>,
) -> Result<(bool, Option<String>), String> {
    validate_scope(local, remote_name)?;
    let use_local_scope = repo.task_uses_local_scope(local, remote_name);
    if use_local_scope {
        return Ok((true, None));
    }
    let remote = repo.remote_row(remote_name)?;
    Ok((false, Some(remote.name)))
}

fn parse_contains_terms(raw: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for value in raw.split(',') {
        let term = value.trim();
        if term.is_empty() {
            continue;
        }
        if !out.iter().any(|existing| existing == term) {
            out.push(term.to_string());
        }
    }
    if out.is_empty() {
        return Err(
            "--contains must include at least one non-empty comma-delimited term.".to_string(),
        );
    }
    Ok(out)
}

fn expect_array(value: &JsonValue) -> Result<Vec<JsonValue>, String> {
    value
        .as_array()
        .cloned()
        .ok_or_else(|| "Expected an array payload from plan command execution.".to_string())
}

fn render_show_like(title: &str, payload: &JsonValue) -> Result<(), String> {
    let plan = payload
        .get("plan")
        .and_then(JsonValue::as_object)
        .or_else(|| payload.as_object())
        .ok_or("Plan show payload must decode to an object.")?;
    print_key_values(
        title,
        &[
            ("plan_id", string_field(plan.get("plan_id"))),
            ("title", string_field(plan.get("title"))),
            ("status", string_field(plan.get("status"))),
            ("repo", string_field(plan.get("repo_name"))),
            (
                "head_revision_id",
                string_field(plan.get("head_revision_id")),
            ),
            ("updated_at", string_field(plan.get("updated_at"))),
            (
                "publication_state",
                string_field(plan.get("publication_state")),
            ),
        ],
    );
    if let Some(revision) = payload
        .get("revision")
        .and_then(JsonValue::as_object)
        .or_else(|| payload.get("head_revision").and_then(JsonValue::as_object))
    {
        println!();
        print_key_values(
            "revision",
            &[
                (
                    "plan_revision_id",
                    string_field(revision.get("plan_revision_id")),
                ),
                (
                    "revision_number",
                    string_field(revision.get("revision_number")),
                ),
                (
                    "title_snapshot",
                    string_field(revision.get("title_snapshot")),
                ),
                ("summary", string_field(revision.get("summary"))),
                ("artifact_path", string_field(revision.get("artifact_path"))),
                (
                    "artifact_selector",
                    string_field(revision.get("artifact_selector")),
                ),
                (
                    "artifact_heading",
                    string_field(revision.get("artifact_heading")),
                ),
                ("created_at", string_field(revision.get("created_at"))),
            ],
        );
    }
    Ok(())
}

fn render_items_like(title: &str, payload: &JsonValue) -> Result<(), String> {
    let items = payload
        .get("items")
        .and_then(JsonValue::as_array)
        .ok_or("Plan items payload is missing `items`.")?;
    print_key_values(
        title,
        &[
            ("plan_id", string_field(payload.get("plan_id"))),
            (
                "plan_revision_id",
                string_field(payload.get("plan_revision_id")),
            ),
            ("item_count", items.len().to_string()),
        ],
    );
    println!();
    print_list(
        items,
        &["plan_item_ref", "checkbox_state", "text", "line_number"],
    );
    Ok(())
}

fn render_candidates_like(payload: &JsonValue) -> Result<(), String> {
    let summary = payload
        .get("summary")
        .and_then(JsonValue::as_object)
        .ok_or("Plan candidates payload is missing `summary`.")?;
    print_key_values(
        "ait-cli plan candidates",
        &[
            ("scope", string_field(payload.get("scope"))),
            ("remote", string_field(payload.get("remote"))),
            ("repo", string_field(payload.get("repo_name"))),
            (
                "candidate_plan_count",
                string_field(summary.get("candidate_plan_count")),
            ),
            (
                "taskable_item_count",
                string_field(summary.get("taskable_item_count")),
            ),
            (
                "linked_task_count",
                string_field(summary.get("linked_task_count")),
            ),
        ],
    );
    println!();
    let rows = payload
        .get("candidates")
        .and_then(JsonValue::as_array)
        .ok_or("Plan candidates payload is missing `candidates`.")?;
    print_list(
        rows,
        &[
            "plan_id",
            "title",
            "artifact_path",
            "artifact_selector",
            "open_item_count",
            "taskable_item_count",
            "linked_task_count",
        ],
    );
    Ok(())
}

fn render_inspect_like(payload: &JsonValue) -> Result<(), String> {
    let plan = payload
        .get("plan")
        .and_then(JsonValue::as_object)
        .ok_or("Plan inspect payload is missing `plan`.")?;
    print_key_values(
        "ait-cli plan inspect",
        &[
            ("plan_id", string_field(plan.get("plan_id"))),
            ("title", string_field(plan.get("title"))),
            ("status", string_field(plan.get("status"))),
            (
                "repo",
                string_field(payload.get("repo_name").or_else(|| plan.get("repo_name"))),
            ),
            ("scope", string_field(payload.get("scope"))),
            ("remote", string_field(payload.get("remote"))),
            (
                "plan_revision_id",
                string_field(plan.get("plan_revision_id")),
            ),
            (
                "taskable_item_count",
                string_field(plan.get("taskable_item_count")),
            ),
            (
                "linked_task_count",
                string_field(plan.get("linked_task_count")),
            ),
        ],
    );
    println!();
    let rows = plan
        .get("items")
        .and_then(JsonValue::as_array)
        .ok_or("Plan inspect payload is missing `items`.")?;
    print_list(
        rows,
        &[
            "plan_item_ref",
            "checkbox_state",
            "taskable",
            "taskable_blocker",
            "line_number",
            "text",
        ],
    );
    Ok(())
}

fn render_sync_like(payload: &JsonValue) -> Result<(), String> {
    if payload.get("status").and_then(JsonValue::as_str) == Some("failed") {
        let message = payload
            .get("error")
            .and_then(JsonValue::as_object)
            .and_then(|error| error.get("message"))
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown plan sync failure");
        return Err(format!("Plan sync failed: {message}"));
    }
    let summary = payload
        .get("summary")
        .and_then(JsonValue::as_object);
    print_key_values(
        "ait-cli plan sync",
        &[
            ("target", string_field(payload.get("target"))),
            ("scope", string_field(payload.get("scope"))),
            ("mode", string_field(payload.get("mode"))),
            ("status", string_field(payload.get("status"))),
            (
                "created_count",
                summary
                    .map(|summary| string_field(summary.get("created_count")))
                    .unwrap_or_default(),
            ),
            (
                "updated_count",
                summary
                    .map(|summary| string_field(summary.get("updated_count")))
                    .unwrap_or_default(),
            ),
            (
                "unchanged_count",
                summary
                    .map(|summary| string_field(summary.get("unchanged_count")))
                    .unwrap_or_default(),
            ),
            (
                "published_count",
                summary
                    .map(|summary| string_field(summary.get("published_count")))
                    .unwrap_or_default(),
            ),
        ],
    );
    println!();
    let results = payload
        .get("results")
        .and_then(JsonValue::as_array)
        .ok_or("Plan sync payload is missing `results`.")?;
    print_list(
        results,
        &[
            "action",
            "artifact_path",
            "plan_id",
            "local_plan_ref",
            "plan_revision_id",
            "status",
        ],
    );
    Ok(())
}
