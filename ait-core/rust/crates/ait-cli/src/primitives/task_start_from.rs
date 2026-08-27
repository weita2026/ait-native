use super::change_flow::validate_short_remote_change_id;
use super::*;
use ait_core::plan_command_execution::execute_plan_inspect_command_request_json;
use ait_core::plan_filesystem::{resolve_repo_artifact_path, PlanFilesystemError};
use ait_core::plan_foundation::parse_plan_markdown;
use ait_core::plan_items::extract_plan_section;
use ait_core::plan_sync_execution::execute_plan_sync_command_request_json;
use ait_core::workflow_primitives::CheckboxState;

const PLAN_BINARY_DB_WRITE_LAYOUT: u32 = 1;
const MAX_TASK_START_FROM_TITLE_CHARS: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq)]
struct TaskStartPlanSource {
    requested: String,
    artifact_path: String,
    resolved_path: PathBuf,
    artifact_selector: String,
    plan_item_ref: String,
    item_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TaskStartPlanBinding {
    scope: String,
    remote_name: Option<String>,
    plan_id: String,
    plan_revision_id: String,
    local_plan_id: String,
    local_plan_revision_id: String,
    sync_action: String,
}

struct RemoteTaskStartContext<'a> {
    remote: &'a RemoteRow,
    debug_probe_override: Option<&'a JsonValue>,
    total_started: Instant,
    plan_source_preflight_elapsed: f64,
}

pub fn task_start_from_with_progress(
    repo: &RepoRuntime,
    source: &str,
    intent: &str,
    local: bool,
    remote_name: Option<&str>,
    debug_probe_override: Option<&JsonValue>,
    mut progress: Option<&mut TaskStartProgressEmitter<'_>>,
) -> Result<JsonValue, String> {
    let total_started = Instant::now();
    let plan_source_preflight_started = Instant::now();
    if !repo.sprint_enabled() {
        return Err(
            "`ait task start --from` is unavailable while sprint mode is off. Use `ait task start --title <title> --intent <intent>` for unbound work, or enable sprint mode first."
                .to_string(),
        );
    }
    task_start_root_preflight(repo)?;
    let source = resolve_task_start_plan_source(repo, source)?;
    let use_local = repo.task_uses_local_scope(local, remote_name)?;
    let remote = if use_local {
        None
    } else {
        Some(repo.remote_row(remote_name)?)
    };
    let plan_source_preflight_elapsed = elapsed_ms(plan_source_preflight_started);

    emit_task_start_progress(
        progress.as_deref_mut(),
        json!({
            "phase": "plan_sync_started",
            "artifact_path": source.artifact_path,
            "artifact_selector": source.artifact_selector,
            "plan_item_ref": source.plan_item_ref,
            "scope": if use_local { "local" } else { "remote" },
            "remote": remote.as_ref().map(|row| row.name.as_str()),
        }),
    )?;
    if !use_local {
        return task_start_from_remote_atomic_with_progress(
            repo,
            &source,
            intent,
            progress,
            RemoteTaskStartContext {
                remote: remote
                    .as_ref()
                    .ok_or_else(|| "Remote task-start context is missing.".to_string())?,
                debug_probe_override,
                total_started,
                plan_source_preflight_elapsed,
            },
        );
    }
    let plan_sync_started = Instant::now();
    let sync = execute_plan_sync_command_request_json(
        &build_task_start_plan_sync_request(repo, &source, use_local, remote.as_ref(), None)?
            .to_string(),
    )?;
    require_plan_sync_success(&sync, &source.artifact_path)?;
    let synchronized_result = require_task_start_plan_sync_result(&sync, &source)?;
    let local_plan_id = required_string_field(synchronized_result, "plan_id")?;
    let plan_sync_elapsed = elapsed_ms(plan_sync_started);
    let plan_binding_started = Instant::now();
    let existing_local_inspect = if remote.is_some()
        && task_start_plan_sync_needs_existing_publication(&sync, &local_plan_id)?
    {
        Some(execute_plan_inspect_command_request_json(
            &build_task_start_local_plan_inspect_request(repo, &local_plan_id)?.to_string(),
        )?)
    } else {
        None
    };
    let binding = resolve_task_start_plan_binding(
        &sync,
        &source,
        remote.as_ref(),
        existing_local_inspect.as_ref(),
    )?;
    let plan_binding_elapsed = elapsed_ms(plan_binding_started);
    emit_task_start_progress(
        progress.as_deref_mut(),
        json!({
            "phase": "plan_synced",
            "artifact_path": source.artifact_path,
            "plan_id": binding.plan_id,
            "plan_revision_id": binding.plan_revision_id,
            "scope": binding.scope,
            "remote": binding.remote_name,
        }),
    )?;

    let after_sync: Result<JsonValue, String> = (|| {
        let plan_item_validation_started = Instant::now();
        let inspect = execute_plan_inspect_command_request_json(
            &build_task_start_plan_inspect_request(repo, &binding, remote.as_ref())?.to_string(),
        )?;
        let item = require_taskable_synchronized_item(&inspect, &binding, &source.plan_item_ref)?;
        let item_text = item
            .get("text")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                format!(
                    "Synchronized Plan item `{}` has no text to derive a Task title from.",
                    source.plan_item_ref
                )
            })?;
        let title = derive_plan_item_task_title(item_text)?;
        let title_source = "plan_item";
        emit_task_start_progress(
            progress.as_deref_mut(),
            json!({
                "phase": "plan_item_validated",
                "plan_item_ref": source.plan_item_ref,
                "title": title,
                "title_source": title_source,
            }),
        )?;
        let plan_item_validation_elapsed = elapsed_ms(plan_item_validation_started);

        let nested_task_start_started = Instant::now();
        let mut payload = task_start_with_progress(
            repo,
            &title,
            intent,
            local,
            remote_name,
            Some(&binding.plan_id),
            Some(&binding.plan_revision_id),
            Some(&source.plan_item_ref),
            debug_probe_override,
            progress.as_deref_mut(),
        )?;
        let nested_task_start_elapsed = elapsed_ms(nested_task_start_started);
        let cd_command = payload
            .pointer("/worktree/cd_command")
            .and_then(JsonValue::as_str)
            .map(str::to_string);
        let object = payload
            .as_object_mut()
            .ok_or_else(|| "task start --from payload must decode to an object.".to_string())?;
        let task_start_timings = object
            .remove("phase_timings_ms")
            .unwrap_or_else(|| json!({}));
        object.insert(
            "title_source".to_string(),
            JsonValue::String(title_source.to_string()),
        );
        object.insert(
            "plan_source".to_string(),
            json!({
                "from": source.requested,
                "artifact_path": source.artifact_path,
                "artifact_selector": source.artifact_selector,
                "plan_item_ref": source.plan_item_ref,
                "scope": binding.scope,
                "remote": binding.remote_name,
                "plan_id": binding.plan_id,
                "plan_revision_id": binding.plan_revision_id,
                "local_plan_id": binding.local_plan_id,
                "local_plan_revision_id": binding.local_plan_revision_id,
                "sync_action": binding.sync_action,
            }),
        );
        if let Some(cd_command) = cd_command {
            object.insert("cd_command".to_string(), JsonValue::String(cd_command));
        }
        object.insert(
            "phase_timings_ms".to_string(),
            json!({
                "plan_source_preflight": plan_source_preflight_elapsed,
                "plan_sync": plan_sync_elapsed,
                "plan_binding": plan_binding_elapsed,
                "plan_item_validation": plan_item_validation_elapsed,
                "task_start": task_start_timings,
                "task_start_call": nested_task_start_elapsed,
                "total": elapsed_ms(total_started),
            }),
        );
        Ok(payload)
    })();

    after_sync.map_err(|error| {
        format!(
            "Plan sync completed for `{}`, but Task start stopped afterward: {error} The synchronized Plan history was kept.",
            source.artifact_path
        )
    })
}

fn task_start_from_remote_atomic_with_progress(
    repo: &RepoRuntime,
    source: &TaskStartPlanSource,
    intent: &str,
    mut progress: Option<&mut TaskStartProgressEmitter<'_>>,
    context: RemoteTaskStartContext<'_>,
) -> Result<JsonValue, String> {
    let RemoteTaskStartContext {
        remote,
        debug_probe_override,
        total_started,
        plan_source_preflight_elapsed,
    } = context;
    let context_preflight_started = Instant::now();
    task_start_from_atomic_context_preflight(repo, source)?;
    let resolved_intent = normalized_text(Some(intent))
        .ok_or_else(|| "Task intent must not be empty.".to_string())?;
    let resolved_base_line = "main".to_string();
    let title_started = Instant::now();
    let resolved_title = derive_plan_item_task_title(&source.item_text)?;
    let title_source = "plan_item";
    let resolved_change_title = resolved_title.clone();
    let context_preflight_elapsed = elapsed_ms(context_preflight_started);
    let title_elapsed = elapsed_ms(title_started);

    let remote_repo_name = remote.repo_name.clone().unwrap_or_else(|| repo.repo_name());
    let remote_base_line_preflight_started = Instant::now();
    let mut task_remote = http_task_remote(repo, remote)?;
    let remote_base_line = task_start_remote_base_line_preflight_with_task_remote(
        repo,
        remote,
        &mut task_remote,
        &remote_repo_name,
        &resolved_base_line,
    )?;
    ensure_remote_base_line_snapshot_with_task_remote(
        repo,
        remote,
        &mut task_remote,
        &remote_repo_name,
        &resolved_base_line,
        &remote_base_line,
    )?;
    let remote_base_line_preflight_elapsed = elapsed_ms(remote_base_line_preflight_started);

    let idempotency_key = task_start_atomic_idempotency_key(
        repo,
        source,
        &resolved_title,
        &resolved_intent,
        &resolved_change_title,
        &resolved_base_line,
    )?;
    let atomic_context = json!({
        "contract": "task-start-atomic/v1",
        "idempotency_key": idempotency_key,
        "plan_item_ref": source.plan_item_ref,
        "task": {
            "title": resolved_title,
            "intent": resolved_intent,
        },
        "change": {
            "title": resolved_change_title,
            "base_line": resolved_base_line,
        },
    });
    let plan_sync_started = Instant::now();
    let sync = execute_plan_sync_command_request_json(
        &build_task_start_plan_sync_request(
            repo,
            source,
            false,
            Some(remote),
            Some(atomic_context),
        )?
        .to_string(),
    )?;
    require_atomic_task_start_plan_sync_success(&sync, &source.artifact_path)?;
    let plan_sync_elapsed = elapsed_ms(plan_sync_started);
    let result = require_task_start_plan_sync_result(&sync, source)?;
    let local_plan_id = required_string_field(result, "plan_id")?;
    let local_plan_revision_id = required_string_field(result, "plan_revision_id")?;
    let sync_action = required_string_field(result, "action")?;
    let publication = require_atomic_task_start_publication(&sync, &local_plan_id)?;
    let atomic = publication
        .get("task_start")
        .ok_or_else(|| "Atomic Plan publication is missing `task_start`.".to_string())?;
    let plan_id = required_string_field(atomic, "plan_id")?;
    let plan_revision_id = required_string_field(atomic, "plan_revision_id")?;
    if publication
        .get("published_plan_id")
        .and_then(JsonValue::as_str)
        != Some(plan_id.as_str())
        || publication
            .get("published_head_revision_id")
            .and_then(JsonValue::as_str)
            != Some(plan_revision_id.as_str())
    {
        return Err(
            "Atomic task-start Plan response does not match the persisted publication receipt."
                .to_string(),
        );
    }
    let binding = TaskStartPlanBinding {
        scope: "remote".to_string(),
        remote_name: Some(remote.name.clone()),
        plan_id,
        plan_revision_id,
        local_plan_id,
        local_plan_revision_id,
        sync_action,
    };
    emit_task_start_progress(
        progress.as_deref_mut(),
        json!({
            "phase": "plan_synced",
            "artifact_path": source.artifact_path,
            "plan_id": binding.plan_id,
            "plan_revision_id": binding.plan_revision_id,
            "scope": binding.scope,
            "remote": binding.remote_name,
        }),
    )?;
    emit_task_start_progress(
        progress.as_deref_mut(),
        json!({
            "phase": "plan_item_validated",
            "plan_item_ref": source.plan_item_ref,
            "title": resolved_title,
            "title_source": title_source,
        }),
    )?;

    let task = atomic
        .get("task")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| "Atomic task-start response is missing `task`.".to_string())?;
    validate_remote_task_create_response(
        &task,
        &remote_repo_name,
        &resolved_title,
        &resolved_intent,
        Some(&binding.plan_id),
        Some(&binding.plan_revision_id),
        Some(&source.plan_item_ref),
    )?;
    let task_id = required_string_field(&task, "task_id")?;
    emit_task_start_progress(
        progress.as_deref_mut(),
        json!({
            "phase": "task_created",
            "task_id": task_id,
        }),
    )?;
    let change = atomic
        .get("change")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| "Atomic task-start response is missing `change`.".to_string())?;
    let change =
        ChangeJson::stateless().normalize_remote_change_payload(&change, Some(&task_id))?;
    validate_short_remote_change_id(&change, &task_id)?;
    for (field, expected) in [
        ("task_id", task_id.as_str()),
        ("title", resolved_change_title.as_str()),
        ("base_line", resolved_base_line.as_str()),
    ] {
        if change.get(field).and_then(JsonValue::as_str) != Some(expected) {
            return Err(format!(
                "Atomic task-start Change returned unexpected {field}; expected {expected:?}."
            ));
        }
    }
    emit_task_start_progress(
        progress.as_deref_mut(),
        json!({
            "phase": "change_created",
            "change_id": string_field(&change, "change_id"),
        }),
    )?;

    let mut payload = task_start_bootstrap_created_records_with_progress(
        repo,
        task,
        Some(change),
        &resolved_base_line,
        false,
        debug_probe_override,
        progress,
    )?;
    let cd_command = payload
        .pointer("/worktree/cd_command")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "Atomic task start payload must decode to an object.".to_string())?;
    let bootstrap_timings = object
        .remove("phase_timings_ms")
        .unwrap_or_else(|| json!({}));
    object.insert(
        "title_source".to_string(),
        JsonValue::String(title_source.to_string()),
    );
    object.insert(
        "plan_source".to_string(),
        json!({
            "from": source.requested,
            "artifact_path": source.artifact_path,
            "artifact_selector": source.artifact_selector,
            "plan_item_ref": source.plan_item_ref,
            "scope": binding.scope,
            "remote": binding.remote_name,
            "plan_id": binding.plan_id,
            "plan_revision_id": binding.plan_revision_id,
            "local_plan_id": binding.local_plan_id,
            "local_plan_revision_id": binding.local_plan_revision_id,
            "sync_action": binding.sync_action,
            "transport_contract": "task-start-atomic/v1",
        }),
    );
    if let Some(cd_command) = cd_command {
        object.insert("cd_command".to_string(), JsonValue::String(cd_command));
    }
    object.insert(
        "phase_timings_ms".to_string(),
        json!({
            "plan_source_preflight": plan_source_preflight_elapsed,
            "context_preflight": context_preflight_elapsed,
            "remote_base_line_preflight": remote_base_line_preflight_elapsed,
            "plan_item_title": title_elapsed,
            "plan_sync": plan_sync_elapsed,
            "atomic_remote_start": publication
                .get("task_start_elapsed_ms")
                .cloned()
                .unwrap_or(JsonValue::Null),
            "worktree_location": bootstrap_timings
                .get("worktree_location")
                .cloned()
                .unwrap_or(JsonValue::Null),
            "worktree_bootstrap": bootstrap_timings
                .get("worktree_bootstrap")
                .cloned()
                .unwrap_or(JsonValue::Null),
            "total": elapsed_ms(total_started),
        }),
    );
    Ok(payload)
}

fn task_start_from_atomic_context_preflight(
    repo: &RepoRuntime,
    source: &TaskStartPlanSource,
) -> Result<(), String> {
    task_start_root_preflight(repo)?;
    let unexpected = collect_planning_only_artifact_drift_paths(repo)?
        .into_iter()
        .filter(|path| path != &source.artifact_path)
        .collect::<Vec<_>>();
    if unexpected.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Refusing atomic `ait task start --from` while planning-only drift exists outside the selected sprint card. Sync these paths first: {}.",
        unexpected.join(", ")
    ))
}

fn task_start_atomic_idempotency_key(
    repo: &RepoRuntime,
    source: &TaskStartPlanSource,
    title: &str,
    intent: &str,
    change_title: &str,
    base_line: &str,
) -> Result<String, String> {
    let markdown = fs::read(&source.resolved_path).map_err(|error| {
        format!(
            "Failed to read `{}` for atomic task-start identity: {error}",
            source.artifact_path
        )
    })?;
    let mut bytes = Vec::new();
    let repo_name = repo.repo_name();
    for text in [
        repo_name.as_str(),
        source.artifact_path.as_str(),
        source.artifact_selector.as_str(),
        source.plan_item_ref.as_str(),
        title,
        intent,
        change_title,
        base_line,
    ] {
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(0);
    }
    bytes.extend_from_slice(&markdown);
    Ok(format!("task-start-atomic:{}", sha256_hex_bytes(&bytes)))
}

fn require_atomic_task_start_plan_sync_success(
    sync: &JsonValue,
    artifact_path: &str,
) -> Result<(), String> {
    if sync.get("status").and_then(JsonValue::as_str) == Some("ok") {
        return Ok(());
    }
    let detail = sync
        .pointer("/error/message")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown atomic Plan/task-start failure");
    Err(format!(
        "Atomic remote task start failed for `{artifact_path}`: {detail} No legacy Task or Change POST was issued; retrying the same command is exact-replay safe."
    ))
}

fn require_atomic_task_start_publication<'a>(
    sync: &'a JsonValue,
    local_plan_id: &str,
) -> Result<&'a JsonValue, String> {
    let publications = sync
        .get("publish_results")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Atomic Plan sync response is missing `publish_results`.".to_string())?;
    let matching = publications
        .iter()
        .filter(|row| {
            row.get("plan_id").and_then(JsonValue::as_str) == Some(local_plan_id)
                && row.get("task_start").is_some_and(|value| value.is_object())
        })
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [publication] => {
            if publication
                .get("head_publication_state")
                .and_then(JsonValue::as_str)
                != Some("published")
            {
                return Err(
                    "Atomic Plan publication did not persist the selected head receipt."
                        .to_string(),
                );
            }
            Ok(*publication)
        }
        [] => Err(format!(
            "Atomic Plan sync returned no task-start publication for local Plan `{local_plan_id}`."
        )),
        _ => Err(format!(
            "Atomic Plan sync returned multiple task-start publications for local Plan `{local_plan_id}`."
        )),
    }
}

fn resolve_task_start_plan_source(
    repo: &RepoRuntime,
    requested: &str,
) -> Result<TaskStartPlanSource, String> {
    let requested = requested.trim();
    let (path_text, plan_item_ref) = requested.rsplit_once('#').ok_or_else(|| {
        "`--from` must use `<repository-relative-markdown-path>#<exact-item-ref>`.".to_string()
    })?;
    let path_text = path_text.trim();
    let plan_item_ref = plan_item_ref.trim();
    if path_text.is_empty() || plan_item_ref.is_empty() {
        return Err(
            "`--from` must include both a Markdown path and an exact item ref.".to_string(),
        );
    }
    let raw_path = Path::new(path_text);
    if raw_path.is_absolute() {
        return Err("`--from` path must be repository-relative, not absolute.".to_string());
    }
    if raw_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("`--from` path must not contain parent-directory traversal.".to_string());
    }
    if raw_path
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|value| !value.eq_ignore_ascii_case("md"))
    {
        return Err("`--from` path must identify a Markdown (`.md`) file.".to_string());
    }

    let resolved = resolve_repo_artifact_path(
        repo.authoritative_repo_root().to_string_lossy().as_ref(),
        path_text,
        false,
    )
    .map_err(plan_filesystem_error_message)?;
    let artifact_path = required_string_field(&resolved, "artifact_path")?;
    let resolved_path = PathBuf::from(required_string_field(&resolved, "resolved_path")?);
    if !resolved_path.is_file() {
        return Err(format!(
            "`--from` path must identify one Markdown file: {artifact_path}"
        ));
    }
    let markdown = fs::read_to_string(&resolved_path).map_err(|error| {
        format!(
            "Failed to read `--from` Markdown file {} as UTF-8: {error}",
            resolved_path.display()
        )
    })?;
    let parsed = parse_plan_markdown(Some(&markdown));
    let matching_items = parsed
        .items
        .iter()
        .filter(|item| item.plan_item_ref == plan_item_ref)
        .collect::<Vec<_>>();
    let item = match matching_items.as_slice() {
        [] => {
            return Err(format!(
                "Plan item ref `{plan_item_ref}` is not present in `{artifact_path}`."
            ))
        }
        [item] => *item,
        _ => {
            return Err(format!(
                "Plan item ref `{plan_item_ref}` is duplicated in `{artifact_path}`."
            ))
        }
    };
    match item.checkbox_state {
        CheckboxState::Open => {}
        CheckboxState::Done => {
            return Err(format!(
                "Plan item `{plan_item_ref}` is already complete in `{artifact_path}`."
            ))
        }
        CheckboxState::None => {
            return Err(format!(
                "Plan item `{plan_item_ref}` is not an open checklist item in `{artifact_path}`."
            ))
        }
    }
    let artifact_selector = parsed
        .plan_refs
        .iter()
        .filter_map(|plan_ref| {
            extract_plan_section(Some(&markdown), Some(&plan_ref.plan_ref)).and_then(|section| {
                section
                    .items
                    .iter()
                    .any(|candidate| candidate.plan_item_ref == plan_item_ref)
                    .then_some((plan_ref.line_number, plan_ref.plan_ref.clone()))
            })
        })
        .max_by_key(|(line_number, _)| *line_number)
        .map(|(_, plan_ref)| plan_ref)
        .ok_or_else(|| {
            format!(
                "Plan item `{plan_item_ref}` is not contained by a `[plan-ref: ...]` section in `{artifact_path}`."
            )
        })?;

    Ok(TaskStartPlanSource {
        requested: requested.to_string(),
        artifact_path,
        resolved_path,
        artifact_selector,
        plan_item_ref: plan_item_ref.to_string(),
        item_text: item.text.clone(),
    })
}

fn build_task_start_plan_sync_request(
    repo: &RepoRuntime,
    source: &TaskStartPlanSource,
    use_local: bool,
    remote: Option<&RemoteRow>,
    task_start: Option<JsonValue>,
) -> Result<JsonValue, String> {
    let mut payload = json!({
        "root_path": repo.authoritative_repo_root(),
        "repo_name": repo.repo_name(),
        "repository_index": repo.repository_index(),
        "id_namespace_prefix": repo.id_namespace_prefix(),
        "created_by": repo.actor_identity(),
        "target": source.artifact_path,
        "plan_ref": source.artifact_selector,
        "prune": false,
        "local": use_local,
        "remote_name": JsonValue::Null,
        "remote_repo_name": JsonValue::Null,
        "base_url": JsonValue::Null,
        "rebase": false,
        "reconcile": false,
        "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
        "task_start": task_start.unwrap_or(JsonValue::Null),
    });
    if let Some(remote) = remote {
        let object = payload
            .as_object_mut()
            .ok_or_else(|| "Task-start Plan sync request must be an object.".to_string())?;
        object.insert(
            "remote_name".to_string(),
            JsonValue::String(remote.name.clone()),
        );
        object.insert(
            "remote_repo_name".to_string(),
            JsonValue::String(remote.repo_name.clone().unwrap_or_else(|| repo.repo_name())),
        );
        object.insert(
            "base_url".to_string(),
            JsonValue::String(remote.url.clone()),
        );
    }
    Ok(payload)
}

fn require_plan_sync_success(sync: &JsonValue, artifact_path: &str) -> Result<(), String> {
    if sync.get("status").and_then(JsonValue::as_str) == Some("ok") {
        return Ok(());
    }
    let detail = sync
        .pointer("/error/message")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown Plan sync failure");
    Err(format!(
        "Plan sync failed for `{artifact_path}` before Task creation: {detail}"
    ))
}

fn require_task_start_plan_sync_result<'a>(
    sync: &'a JsonValue,
    source: &TaskStartPlanSource,
) -> Result<&'a JsonValue, String> {
    let results = sync
        .get("results")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Plan sync response is missing `results`.".to_string())?;
    let matching_results = results
        .iter()
        .filter(|row| {
            row.get("artifact_path").and_then(JsonValue::as_str)
                == Some(source.artifact_path.as_str())
                && row.get("artifact_selector").and_then(JsonValue::as_str)
                    == Some(source.artifact_selector.as_str())
        })
        .collect::<Vec<_>>();
    match matching_results.as_slice() {
        [result] => Ok(*result),
        [] => Err(format!(
            "Plan sync response did not identify `{}` with selector `{}`.",
            source.artifact_path, source.artifact_selector
        )),
        _ => Err(format!(
            "Plan sync response identified `{}` with selector `{}` more than once.",
            source.artifact_path, source.artifact_selector
        )),
    }
}

fn task_start_plan_sync_needs_existing_publication(
    sync: &JsonValue,
    local_plan_id: &str,
) -> Result<bool, String> {
    let publish_results = sync
        .get("publish_results")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Remote Plan sync response is missing `publish_results`.".to_string())?;
    Ok(!publish_results
        .iter()
        .any(|row| row.get("plan_id").and_then(JsonValue::as_str) == Some(local_plan_id)))
}

fn resolve_task_start_plan_binding(
    sync: &JsonValue,
    source: &TaskStartPlanSource,
    remote: Option<&RemoteRow>,
    existing_local_inspect: Option<&JsonValue>,
) -> Result<TaskStartPlanBinding, String> {
    let result = require_task_start_plan_sync_result(sync, source)?;
    let local_plan_id = required_string_field(result, "plan_id")?;
    let local_plan_revision_id = required_string_field(result, "plan_revision_id")?;
    let sync_action = required_string_field(result, "action")?;
    let Some(remote) = remote else {
        return Ok(TaskStartPlanBinding {
            scope: "local".to_string(),
            remote_name: None,
            plan_id: local_plan_id.clone(),
            plan_revision_id: local_plan_revision_id.clone(),
            local_plan_id,
            local_plan_revision_id,
            sync_action,
        });
    };

    let publish_results = sync
        .get("publish_results")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Remote Plan sync response is missing `publish_results`.".to_string())?;
    let matching_publications = publish_results
        .iter()
        .filter(|row| {
            row.get("plan_id").and_then(JsonValue::as_str) == Some(local_plan_id.as_str())
        })
        .collect::<Vec<_>>();
    let (plan_id, plan_revision_id) = match matching_publications.as_slice() {
        [publication] => {
            if publication
                .get("head_publication_state")
                .and_then(JsonValue::as_str)
                != Some("published")
            {
                return Err(format!(
                    "Remote Plan sync did not publish the selected head for local Plan `{local_plan_id}`."
                ));
            }
            (
                required_string_field(publication, "published_plan_id")?,
                required_string_field(publication, "published_head_revision_id")?,
            )
        }
        [] => {
            let inspect = existing_local_inspect.ok_or_else(|| {
                format!(
                    "Remote Plan sync returned no publication for local Plan `{local_plan_id}`, and its existing publication receipt was not inspected."
                )
            })?;
            require_existing_task_start_publication(
                inspect,
                &local_plan_id,
                &local_plan_revision_id,
            )?
        }
        _ => {
            return Err(format!(
                "Remote Plan sync returned multiple publications for local Plan `{local_plan_id}`."
            ))
        }
    };
    Ok(TaskStartPlanBinding {
        scope: "remote".to_string(),
        remote_name: Some(remote.name.clone()),
        plan_id,
        plan_revision_id,
        local_plan_id,
        local_plan_revision_id,
        sync_action,
    })
}

fn require_existing_task_start_publication(
    inspect: &JsonValue,
    local_plan_id: &str,
    local_plan_revision_id: &str,
) -> Result<(String, String), String> {
    let plan_value = inspect
        .get("plan")
        .ok_or_else(|| "Local Plan inspect response is missing `plan`.".to_string())?;
    let plan = plan_value
        .as_object()
        .ok_or_else(|| "Local Plan inspect response is missing `plan`.".to_string())?;
    let inspected_plan_id = plan
        .get("plan_id")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let inspected_revision_id = plan
        .get("plan_revision_id")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    if inspected_plan_id != local_plan_id || inspected_revision_id != local_plan_revision_id {
        return Err(format!(
            "Local Plan head changed while resolving its publication receipt: synchronized {local_plan_id}@{local_plan_revision_id}, inspected {}@{}.",
            if inspected_plan_id.is_empty() {
                "<unknown>"
            } else {
                inspected_plan_id
            },
            if inspected_revision_id.is_empty() {
                "<unknown>"
            } else {
                inspected_revision_id
            },
        ));
    }
    if plan
        .get("head_publication_state")
        .and_then(JsonValue::as_str)
        != Some("published")
    {
        return Err(format!(
            "Remote Plan sync returned no publication for local Plan `{local_plan_id}`, and synchronized head `{local_plan_revision_id}` has no exact published receipt."
        ));
    }
    Ok((
        required_string_field(plan_value, "published_plan_id")?,
        required_string_field(plan_value, "published_head_revision_id")?,
    ))
}

fn build_task_start_local_plan_inspect_request(
    repo: &RepoRuntime,
    plan_id: &str,
) -> Result<JsonValue, String> {
    Ok(json!({
        "scope": "local",
        "repository_index": repo.repository_index(),
        "repo_name": repo.repo_name(),
        "plan_id": plan_id,
        "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
    }))
}

fn build_task_start_plan_inspect_request(
    repo: &RepoRuntime,
    binding: &TaskStartPlanBinding,
    remote: Option<&RemoteRow>,
) -> Result<JsonValue, String> {
    let Some(remote) = remote else {
        return Ok(json!({
            "scope": "local",
            "repository_index": repo.repository_index(),
            "repo_name": repo.repo_name(),
            "plan_id": binding.plan_id,
            "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
        }));
    };
    Ok(json!({
        "scope": "remote",
        "base_url": remote.url,
        "repository_index": repo.repository_index(),
        "repo_name": remote.repo_name.clone().unwrap_or_else(|| repo.repo_name()),
        "remote": remote.name,
        "plan_id": binding.plan_id,
        "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
    }))
}

fn require_taskable_synchronized_item<'a>(
    inspect: &'a JsonValue,
    binding: &TaskStartPlanBinding,
    plan_item_ref: &str,
) -> Result<&'a JsonValue, String> {
    let plan = inspect
        .get("plan")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Plan inspect response is missing `plan`.".to_string())?;
    let inspected_plan_id = plan
        .get("plan_id")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let inspected_revision_id = plan
        .get("plan_revision_id")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    if inspected_plan_id != binding.plan_id || inspected_revision_id != binding.plan_revision_id {
        return Err(format!(
            "Plan head changed during task start: synchronized {}@{}, inspected {}@{}. Retry from the current Plan head.",
            binding.plan_id,
            binding.plan_revision_id,
            if inspected_plan_id.is_empty() { "<unknown>" } else { inspected_plan_id },
            if inspected_revision_id.is_empty() { "<unknown>" } else { inspected_revision_id },
        ));
    }
    let items = plan
        .get("items")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Plan inspect response is missing `plan.items`.".to_string())?;
    let matching = items
        .iter()
        .filter(|item| item.get("plan_item_ref").and_then(JsonValue::as_str) == Some(plan_item_ref))
        .collect::<Vec<_>>();
    let item = match matching.as_slice() {
        [item] => *item,
        [] => {
            return Err(format!(
                "Synchronized Plan does not contain item `{plan_item_ref}`."
            ))
        }
        _ => {
            return Err(format!(
                "Synchronized Plan contains duplicate item ref `{plan_item_ref}`."
            ))
        }
    };
    if item.get("checkbox_state").and_then(JsonValue::as_str) != Some("open") {
        return Err(format!(
            "Synchronized Plan item `{plan_item_ref}` is not open."
        ));
    }
    if item.get("taskable").and_then(JsonValue::as_bool) != Some(true) {
        let blocker = item
            .get("taskable_blocker")
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown_blocker");
        return Err(format!(
            "Synchronized Plan item `{plan_item_ref}` is not taskable: {blocker}."
        ));
    }
    Ok(item)
}

fn derive_plan_item_task_title(item_text: &str) -> Result<String, String> {
    let normalized = normalize_title_whitespace(item_text)
        .trim_end_matches('.')
        .trim()
        .to_string();
    validate_task_title(&normalized)
}

fn validate_task_title(title: &str) -> Result<String, String> {
    if title.is_empty() {
        return Err(
            "Plan item text cannot produce an empty Task title. Rewrite the checklist summary."
                .to_string(),
        );
    }
    let char_count = title.chars().count();
    if char_count > MAX_TASK_START_FROM_TITLE_CHARS {
        return Err(format!(
            "Task title has {char_count} characters; the limit is {MAX_TASK_START_FROM_TITLE_CHARS}. Rewrite the checklist as a concise outcome with nested acceptance criteria."
        ));
    }
    if is_generic_task_title(title) {
        return Err(format!(
            "Plan item text `{title}` is too generic for a Task title. Rewrite the checklist summary."
        ));
    }
    Ok(title.to_string())
}

fn normalize_title_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_generic_task_title(title: &str) -> bool {
    let normalized = title
        .trim_matches(|character: char| character.is_ascii_punctuation())
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "todo" | "tbd" | "task" | "work item" | "do work" | "implement" | "fix" | "fix issue"
    )
}

fn plan_filesystem_error_message(error: PlanFilesystemError) -> String {
    match error {
        PlanFilesystemError::Invalid(message)
        | PlanFilesystemError::NotFound(message)
        | PlanFilesystemError::MissingEntry(message)
        | PlanFilesystemError::Io(message) => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_surface::{init_repo, InitRequest};
    use tempfile::TempDir;

    fn fixture_repo(temp: &TempDir) -> RepoRuntime {
        fs::create_dir_all(temp.path().join(".ait")).unwrap();
        fs::write(
            temp.path().join(".ait/config.json"),
            r#"{"repo_name":"fixture","default_line":"main","task_default_scope":"local"}"#,
        )
        .unwrap();
        RepoRuntime::discover_from_path(temp.path()).unwrap()
    }

    fn write_card(temp: &TempDir, markdown: &str) {
        let path = temp.path().join("docs/sprints/card.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, markdown).unwrap();
    }

    #[test]
    fn source_resolution_selects_the_nearest_plan_section_and_open_item() {
        let temp = TempDir::new().unwrap();
        let repo = fixture_repo(&temp);
        write_card(
            &temp,
            "# Root [plan-ref: root]\n\n## Child [plan-ref: child]\n\n- [ ] Add bounded task start. [ref: child/start]\n",
        );

        let source =
            resolve_task_start_plan_source(&repo, "docs/sprints/card.md#child/start").unwrap();

        assert_eq!(source.artifact_path, "docs/sprints/card.md");
        assert_eq!(source.artifact_selector, "child");
        assert_eq!(source.plan_item_ref, "child/start");
    }

    #[test]
    fn source_resolution_rejects_unsafe_missing_duplicate_and_closed_inputs() {
        let temp = TempDir::new().unwrap();
        let repo = fixture_repo(&temp);
        write_card(
            &temp,
            "# Root [plan-ref: root]\n\n- [x] Closed. [ref: root/closed]\n- [ ] First. [ref: root/duplicate]\n- [ ] Second. [ref: root/duplicate]\n",
        );

        for (source, expected) in [
            ("/tmp/card.md#root/closed", "repository-relative"),
            ("../card.md#root/closed", "parent-directory traversal"),
            ("docs/sprints/card.txt#root/closed", "Markdown"),
            ("docs/sprints/card.md", "must use"),
            ("docs/sprints/card.md#root/missing", "is not present"),
            ("docs/sprints/card.md#root/duplicate", "is duplicated"),
            ("docs/sprints/card.md#root/closed", "already complete"),
        ] {
            let error = resolve_task_start_plan_source(&repo, source).unwrap_err();
            assert!(error.contains(expected), "{source}: {error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn source_resolution_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let repo = fixture_repo(&temp);
        fs::write(
            outside.path().join("card.md"),
            "# Outside [plan-ref: outside]\n- [ ] Escape. [ref: outside/item]\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("docs/sprints")).unwrap();
        symlink(
            outside.path().join("card.md"),
            temp.path().join("docs/sprints/escape.md"),
        )
        .unwrap();

        let error = resolve_task_start_plan_source(&repo, "docs/sprints/escape.md#outside/item")
            .unwrap_err();
        assert!(error.contains("inside the repository root"), "{error}");
    }

    #[test]
    fn title_derivation_is_deterministic_and_rejects_weak_summaries() {
        assert_eq!(
            derive_plan_item_task_title("  Add   Plan-derived task start. ").unwrap(),
            "Add Plan-derived task start"
        );
        assert!(derive_plan_item_task_title("TODO").is_err());
    }

    #[test]
    fn title_derivation_accepts_500_unicode_characters_and_rejects_501() {
        assert_eq!(MAX_TASK_START_FROM_TITLE_CHARS, 500);
        let ordinary_long_title = "x".repeat(155);
        assert_eq!(
            derive_plan_item_task_title(&ordinary_long_title).unwrap(),
            ordinary_long_title
        );

        let maximum_title = "界".repeat(500);
        assert_eq!(maximum_title.chars().count(), 500);
        assert!(maximum_title.len() > 500);
        assert_eq!(
            derive_plan_item_task_title(&maximum_title).unwrap(),
            maximum_title
        );
        let oversized_title = "界".repeat(501);
        let error = derive_plan_item_task_title(&oversized_title).unwrap_err();
        assert!(error.contains("501 characters"), "{error}");
        assert!(error.contains("limit is 500"), "{error}");
    }

    #[test]
    fn remote_binding_uses_published_ids_not_local_source_ids() {
        let source = TaskStartPlanSource {
            requested: "docs/sprints/card.md#root/item".to_string(),
            artifact_path: "docs/sprints/card.md".to_string(),
            resolved_path: PathBuf::from("/repo/docs/sprints/card.md"),
            artifact_selector: "root".to_string(),
            plan_item_ref: "root/item".to_string(),
            item_text: "Implement root item".to_string(),
        };
        let remote = RemoteRow {
            name: "origin".to_string(),
            url: "https://example.test/repo".to_string(),
            repo_name: Some("fixture".to_string()),
        };
        let sync = json!({
            "results": [{
                "action": "updated",
                "artifact_path": "docs/sprints/card.md",
                "artifact_selector": "root",
                "plan_id": "PR-LOCAL",
                "plan_revision_id": "plan-revision:local"
            }],
            "publish_results": [{
                "plan_id": "PR-LOCAL",
                "head_publication_state": "published",
                "published_plan_id": "PR-REMOTE",
                "published_head_revision_id": "plan-revision:remote"
            }]
        });
        assert!(!task_start_plan_sync_needs_existing_publication(&sync, "PR-LOCAL").unwrap());
        let binding = resolve_task_start_plan_binding(&sync, &source, Some(&remote), None).unwrap();

        assert_eq!(binding.plan_id, "PR-REMOTE");
        assert_eq!(binding.plan_revision_id, "plan-revision:remote");
        assert_eq!(binding.local_plan_id, "PR-LOCAL");
    }

    #[test]
    fn remote_binding_reuses_an_exact_existing_publication_receipt() {
        let source = TaskStartPlanSource {
            requested: "docs/sprints/card.md#root/item".to_string(),
            artifact_path: "docs/sprints/card.md".to_string(),
            resolved_path: PathBuf::from("/repo/docs/sprints/card.md"),
            artifact_selector: "root".to_string(),
            plan_item_ref: "root/item".to_string(),
            item_text: "Implement root item".to_string(),
        };
        let remote = RemoteRow {
            name: "origin".to_string(),
            url: "https://example.test/repo".to_string(),
            repo_name: Some("fixture".to_string()),
        };
        let sync = json!({
            "results": [{
                "action": "unchanged",
                "artifact_path": "docs/sprints/card.md",
                "artifact_selector": "root",
                "plan_id": "PR-LOCAL",
                "plan_revision_id": "plan-revision:local"
            }],
            "publish_results": []
        });
        assert!(task_start_plan_sync_needs_existing_publication(&sync, "PR-LOCAL").unwrap());
        let inspect = json!({"plan": {
            "plan_id": "PR-LOCAL",
            "plan_revision_id": "plan-revision:local",
            "head_publication_state": "published",
            "published_plan_id": "PR-REMOTE",
            "published_head_revision_id": "plan-revision:remote"
        }});

        let binding =
            resolve_task_start_plan_binding(&sync, &source, Some(&remote), Some(&inspect)).unwrap();

        assert_eq!(binding.plan_id, "PR-REMOTE");
        assert_eq!(binding.plan_revision_id, "plan-revision:remote");
        assert_eq!(binding.local_plan_revision_id, "plan-revision:local");

        let mismatched = json!({"plan": {
            "plan_id": "PR-LOCAL",
            "plan_revision_id": "plan-revision:newer",
            "head_publication_state": "published",
            "published_plan_id": "PR-REMOTE",
            "published_head_revision_id": "plan-revision:remote-newer"
        }});
        let error =
            resolve_task_start_plan_binding(&sync, &source, Some(&remote), Some(&mismatched))
                .unwrap_err();
        assert!(error.contains("Local Plan head changed"), "{error}");

        let unpublished = json!({"plan": {
            "plan_id": "PR-LOCAL",
            "plan_revision_id": "plan-revision:local",
            "head_publication_state": "local_draft",
            "published_plan_id": "PR-REMOTE",
            "published_head_revision_id": "plan-revision:older"
        }});
        let error =
            resolve_task_start_plan_binding(&sync, &source, Some(&remote), Some(&unpublished))
                .unwrap_err();
        assert!(error.contains("no exact published receipt"), "{error}");
    }

    #[test]
    fn synchronized_item_validation_rejects_head_race_and_taskability_blocker() {
        let binding = TaskStartPlanBinding {
            scope: "remote".to_string(),
            remote_name: Some("origin".to_string()),
            plan_id: "PR-1".to_string(),
            plan_revision_id: "plan-revision:2".to_string(),
            local_plan_id: "PR-L".to_string(),
            local_plan_revision_id: "plan-revision:L".to_string(),
            sync_action: "updated".to_string(),
        };
        let race = require_taskable_synchronized_item(
            &json!({"plan": {
                "plan_id": "PR-1",
                "plan_revision_id": "plan-revision:3",
                "items": []
            }}),
            &binding,
            "root/item",
        )
        .unwrap_err();
        assert!(race.contains("Plan head changed"));

        let blocked = require_taskable_synchronized_item(
            &json!({"plan": {
                "plan_id": "PR-1",
                "plan_revision_id": "plan-revision:2",
                "items": [{
                    "plan_item_ref": "root/item",
                    "checkbox_state": "open",
                    "taskable": false,
                    "taskable_blocker": "linked_task_exists"
                }]
            }}),
            &binding,
            "root/item",
        )
        .unwrap_err();
        assert!(blocked.contains("linked_task_exists"));
    }

    #[test]
    fn local_orchestration_syncs_validates_and_reuses_task_change_worktree_bootstrap() {
        let temp = TempDir::new().unwrap();
        init_repo(&InitRequest {
            root: temp.path().to_path_buf(),
            name: Some("fixture".to_string()),
            default_line: "main".to_string(),
            policy_profile: "prototype".to_string(),
            default_author_mode: "ai_with_human_review".to_string(),
            default_model: None,
            repair_existing: false,
        })
        .unwrap();
        write_card(
            &temp,
            "# Card [plan-ref: card]\n\n- [ ] Add deterministic Plan start. [ref: card/start]\n",
        );
        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        let memory = TempDir::new().unwrap();
        let debug_probe = json!({
            "platform": "linux",
            "linux_detected_memory_roots": [memory.path().to_string_lossy().to_string()],
        });
        let mut phases = Vec::new();
        let mut progress = |event: &JsonValue| {
            phases.push(
                event
                    .get("phase")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
            );
            Ok(())
        };

        let payload = task_start_from_with_progress(
            &repo,
            "docs/sprints/card.md#card/start",
            "Exercise the complete local Plan-derived task bootstrap",
            true,
            None,
            Some(&debug_probe),
            Some(&mut progress),
        )
        .unwrap();

        assert_eq!(payload["title"], "Add deterministic Plan start");
        assert_eq!(payload["title_source"], "plan_item");
        assert_eq!(payload["plan_source"]["scope"], "local");
        assert_eq!(payload["plan_source"]["plan_item_ref"], "card/start");
        assert!(payload["change"].get("change_id").is_some());
        assert!(payload["worktree"].get("open_path").is_some());
        assert_eq!(payload["cd_command"], payload["worktree"]["cd_command"]);
        assert!(payload
            .pointer("/phase_timings_ms/plan_sync")
            .and_then(JsonValue::as_f64)
            .is_some());
        assert!(payload
            .pointer("/phase_timings_ms/task_start/worktree_bootstrap")
            .and_then(JsonValue::as_f64)
            .is_some());
        let plan_sync_index = phases
            .iter()
            .position(|phase| phase == "plan_sync_started")
            .unwrap();
        let validated_index = phases
            .iter()
            .position(|phase| phase == "plan_item_validated")
            .unwrap();
        let task_index = phases
            .iter()
            .position(|phase| phase == "task_created")
            .unwrap();
        let worktree_index = phases
            .iter()
            .position(|phase| phase == "worktree_ready")
            .unwrap();
        assert!(plan_sync_index < validated_index);
        assert!(validated_index < task_index);
        assert!(task_index < worktree_index);
    }
}
