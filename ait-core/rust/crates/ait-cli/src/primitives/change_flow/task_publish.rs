use super::*;

pub(in crate::primitives) fn task_publish_remote_create_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    local_task: &JsonValue,
    requested_task_id: &str,
    published_plan_id: Option<&str>,
    published_revision_id: Option<&str>,
    published_plan_item_ref: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteTaskCreator + ?Sized,
{
    let remote_task = task_remote
        .create_task(
            repo_name,
            &required_string_field(local_task, "title")?,
            &required_string_field(local_task, "intent")?,
            Some(requested_task_id),
            published_plan_id,
            published_revision_id,
            published_plan_item_ref,
        )
        .map_err(|err| err.to_string())?;
    let remote_task_id = required_string_field(&remote_task, "task_id")?;
    if remote_task_id != requested_task_id {
        return Err(format!(
            "Remote server returned task_id {remote_task_id:?} while publishing local task {requested_task_id}. Shared publish must preserve the requested canonical id."
        ));
    }
    Ok(remote_task)
}

pub fn task_close(
    repo: &RepoRuntime,
    task_id: &str,
    status: &str,
    local: bool,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    if repo.task_uses_local_scope(local, remote_name) {
        let store = repo.task_store()?;
        return task_local_close_with_task_store(&store, task_id, status);
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    task_close_with_closeout_remote(&mut closeout_remote, task_id, status, &repo_name)
}

pub(in crate::primitives) fn task_close_with_closeout_remote<R>(
    closeout_remote: &mut R,
    task_id: &str,
    status: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteTaskCloser + ?Sized,
{
    closeout_remote
        .close_task(task_id, status, Some(repo_name))
        .map_err(|err| err.to_string())
}

pub fn task_complete(
    repo: &RepoRuntime,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    if repo.task_uses_local_scope(local, remote_name) {
        let store = repo.task_store()?;
        return task_local_close_with_task_store(&store, task_id, "completed");
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    task_complete_with_closeout_remote(&mut closeout_remote, task_id, &repo_name)
}

pub(in crate::primitives) fn task_complete_with_closeout_remote<R>(
    closeout_remote: &mut R,
    task_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteTaskCloser + ?Sized,
{
    task_close_with_closeout_remote(closeout_remote, task_id, "completed", repo_name)
}

pub fn task_restart(
    repo: &RepoRuntime,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    if repo.task_uses_local_scope(local, remote_name) {
        return restart_local_task(repo, task_id);
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    task_restart_with_closeout_remote(&mut closeout_remote, task_id, &repo_name)
}

pub(in crate::primitives) fn task_restart_with_closeout_remote<R>(
    closeout_remote: &mut R,
    task_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteTaskRestarter + ?Sized,
{
    closeout_remote
        .restart_task(task_id, Some(repo_name))
        .map_err(|err| err.to_string())
}

pub fn task_publish(
    repo: &RepoRuntime,
    task_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    task_publish_inner(repo, task_id, remote_name, false)
}

fn task_publish_inner(
    repo: &RepoRuntime,
    task_id: &str,
    remote_name: Option<&str>,
    allow_completed_local: bool,
) -> Result<JsonValue, String> {
    let task_store = repo.task_store()?;
    let change_store = repo.change_store()?;
    let local_task = task_local_read_with_task_store(&task_store, task_id)?;
    task_publish_completed_local_guard_with_change_store(
        &change_store,
        &local_task,
        task_id,
        allow_completed_local,
    )?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let (published_plan_id, published_revision_id, published_plan_item_ref) =
        published_local_task_plan_linkage(repo, &local_task)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    task_publish_with_local_stores_and_task_remote(
        &task_store,
        &change_store,
        &mut task_remote,
        &local_task,
        task_id,
        &repo_name,
        remote_row.name.as_str(),
        allow_completed_local,
        published_plan_id.as_deref(),
        published_revision_id.as_deref(),
        published_plan_item_ref.as_deref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn task_publish_with_local_stores_and_task_remote<T, C, R>(
    task_store: &T,
    change_store: &C,
    task_remote: &mut R,
    local_task: &JsonValue,
    task_id: &str,
    repo_name: &str,
    remote_name: &str,
    allow_completed_local: bool,
    published_plan_id: Option<&str>,
    published_revision_id: Option<&str>,
    published_plan_item_ref: Option<&str>,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowTaskPublisher + ?Sized,
    C: TaskWorkflowChangeLister + ?Sized,
    R: TaskWorkflowRemoteTaskCreator + ?Sized,
{
    let local_task_id = required_string_field(local_task, "task_id")?;
    if local_task_id != task_id {
        return Err(format!(
            "Local task payload id {local_task_id} does not match requested task {task_id}"
        ));
    }
    task_publish_completed_local_guard_with_change_store(
        change_store,
        local_task,
        task_id,
        allow_completed_local,
    )?;
    let local_repo_name = required_string_field(local_task, "repo_name")?;
    if local_repo_name != repo_name {
        return Err(format!(
            "Local task {task_id} belongs to repository {local_repo_name}, not {repo_name}"
        ));
    }
    let remote_task = task_publish_remote_create_with_task_remote(
        task_remote,
        repo_name,
        local_task,
        &local_task_id,
        published_plan_id,
        published_revision_id,
        published_plan_item_ref,
    )?;
    let remote_task_id = required_string_field(&remote_task, "task_id")?;
    let published_task_id =
        string_field(&remote_task, "published_task_id").unwrap_or_else(|| remote_task_id.clone());
    task_local_mark_published_with_task_store(
        task_store,
        task_id,
        Some(remote_name),
        Some(&published_task_id),
    )
}

pub(in crate::primitives) fn task_publish_completed_local_guard_with_change_store<C>(
    change_store: &C,
    local_task: &JsonValue,
    task_id: &str,
    allow_completed_local: bool,
) -> Result<(), String>
where
    C: TaskWorkflowChangeLister + ?Sized,
{
    let local_task_id = required_string_field(local_task, "task_id")?;
    let local_task_status = required_string_field(local_task, "status")?;
    let landed_local_changes = change_local_list_with_change_store(change_store)?
        .into_iter()
        .filter(|row| {
            string_field(row, "task_id").as_deref() == Some(local_task_id.as_str())
                && string_field(row, "status").as_deref() == Some("landed")
        })
        .filter_map(|row| string_field(&row, "change_id"))
        .collect::<Vec<_>>();
    if local_task_status == "completed"
        && !landed_local_changes.is_empty()
        && !allow_completed_local
    {
        let landed_preview = landed_local_changes
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let overflow = if landed_local_changes.len() > 3 {
            format!(" (+{} more)", landed_local_changes.len() - 3)
        } else {
            String::new()
        };
        let detail = if landed_preview.is_empty() {
            " landed local change lineage".to_string()
        } else {
            format!(" landed local change(s) {landed_preview}{overflow}")
        };
        return Err(format!(
            "Local task {task_id} already has{detail}. {COMPLETED_LOCAL_FINAL_SNAPSHOT_PROMOTION_GUIDANCE}"
        ));
    }
    Ok(())
}

pub(in crate::primitives) fn restart_local_task_read_with_task_store<S>(
    task_store: &S,
    task_id: &str,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowTaskReader + ?Sized,
{
    task_store.get_task(task_id).map_err(|err| err.to_string())
}

pub(in crate::primitives) fn restart_local_change_read_with_change_store<S>(
    change_store: &S,
    change_id: &str,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowChangeReader + ?Sized,
{
    change_store
        .get_change(change_id)
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn restart_local_change_rows_with_change_store<S>(
    change_store: &S,
    task_id: &str,
) -> Result<Vec<JsonValue>, String>
where
    S: ChangeStore + ?Sized,
{
    list_changes_for_task_with_change_store(change_store, task_id).map_err(|err| err.to_string())
}

pub(in crate::primitives) fn restart_local_task_reactivate_with_task_store<S>(
    task_store: &S,
    task_id: &str,
) -> Result<JsonValue, String>
where
    S: TaskStore + ?Sized,
{
    restart_task_with_task_store(task_store, task_id).map_err(|err| err.to_string())
}

pub(in crate::primitives) fn restart_local_change_reopen_with_change_store<S>(
    change_store: &S,
    change_id: &str,
) -> Result<JsonValue, String>
where
    S: ChangeStore + ?Sized,
{
    reopen_change_as_draft_with_change_store(change_store, change_id).map_err(|err| err.to_string())
}

fn restart_local_task(repo: &RepoRuntime, task_id: &str) -> Result<JsonValue, String> {
    let task_store = repo.task_store()?;
    let change_store = repo.change_store()?;
    let task = restart_local_task_read_with_task_store(&task_store, task_id)?;
    let resolved_task_id = required_string_field(&task, "task_id")?;
    if string_field(&task, "publication_state").as_deref() == Some("published") {
        return Err(format!(
            "Local task {task_id} is published; restart the remote task lineage instead."
        ));
    }
    let task_status = required_string_field(&task, "status")?;
    if !matches!(task_status.as_str(), "abandoned" | "canceled") {
        return Err(format!(
            "Local task {task_id} is `{task_status}`; restart only supports task canceled lineage."
        ));
    }
    let change_rows =
        restart_local_change_rows_with_change_store(&change_store, resolved_task_id.as_str())?;
    if let Some(landed_change_id) = change_rows.iter().find_map(|row| {
        (string_field(row, "status").as_deref() == Some("landed"))
            .then(|| string_field(row, "change_id"))
            .flatten()
    }) {
        return Err(format!(
            "Local task {task_id} cannot be restarted because landed change {landed_change_id} already exists."
        ));
    }
    let archived_changes = change_rows
        .iter()
        .filter(|row| string_field(row, "status").as_deref() == Some("archived"))
        .filter_map(|row| string_field(row, "change_id"))
        .collect::<Vec<_>>();
    let open_changes = change_rows
        .iter()
        .filter(|row| {
            !matches!(
                string_field(row, "status").as_deref(),
                Some("archived" | "superseded")
            )
        })
        .filter_map(|row| string_field(row, "change_id"))
        .collect::<Vec<_>>();
    if open_changes.is_empty() && archived_changes.len() > 1 {
        return Err(format!(
            "Local task {task_id} has multiple archived changes ({}); restart only supports one archived change.",
            archived_changes.join(", ")
        ));
    }
    let refreshed =
        restart_local_task_reactivate_with_task_store(&task_store, resolved_task_id.as_str())?;
    let restarted_change = if open_changes.is_empty() && archived_changes.len() == 1 {
        let change_id = &archived_changes[0];
        restart_local_change_reopen_with_change_store(&change_store, change_id)?;
        Some(restart_local_change_read_with_change_store(
            &change_store,
            change_id,
        )?)
    } else {
        None
    };
    let mut object = refreshed
        .as_object()
        .cloned()
        .ok_or_else(|| "task restart payload must decode to an object.".to_string())?;
    if let Some(change) = restarted_change {
        object.insert("change".to_string(), change);
    }
    Ok(JsonValue::Object(object))
}

type PublishedTaskPlanLinkage = (Option<String>, Option<String>, Option<String>);

pub(in crate::primitives) fn published_local_task_plan_linkage(
    repo: &RepoRuntime,
    task: &JsonValue,
) -> Result<PublishedTaskPlanLinkage, String> {
    let resolved_plan_id = string_field(task, "plan_id");
    let resolved_revision_id = string_field(task, "origin_plan_revision_id");
    let resolved_plan_item_ref = string_field(task, "plan_item_ref");
    let mode = repo
        .config
        .get("plan_task_binding_mode")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
        .unwrap_or_default();
    if resolved_plan_id.is_none() && resolved_revision_id.is_none() {
        if mode == "required" {
            return Err(
                "Required plan/task binding requires local draft tasks to carry durable plan linkage before remote publish.".to_string(),
            );
        }
        if resolved_plan_item_ref.is_some() {
            return Err(
                "Local task plan metadata is incomplete: `plan_item_ref` requires plan linkage."
                    .to_string(),
            );
        }
        return Ok((None, None, None));
    }
    if matches!(mode.as_str(), "strict" | "required") && resolved_plan_item_ref.is_none() {
        return Err(
            "Strict or required plan/task binding requires `plan_item_ref` for remote publish."
                .to_string(),
        );
    }
    let plan_store = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .plans();
    let linkage = resolve_reconciled_plan_publish_linkage_with_plan_store(
        &plan_store,
        resolved_plan_id.as_deref(),
        resolved_revision_id.as_deref(),
    )
    .map_err(|err| err.to_string())?;
    let resolved_plan_id = linkage.plan_id;
    let published_plan_id = linkage.published_plan_id.ok_or_else(|| {
        format!(
            "Local task {} is linked to unpublished local plan {resolved_plan_id}. Publish the plan first.",
            required_string_field(task, "task_id").unwrap_or_else(|_| "unknown task".to_string())
        )
    })?;
    let resolved_revision_id = linkage.plan_revision_id.ok_or_else(|| {
        format!(
            "Local task {} is linked to local plan {resolved_plan_id} without a stored revision id.",
            required_string_field(task, "task_id").unwrap_or_else(|_| "unknown task".to_string())
        )
    })?;
    let published_revision_id = linkage.published_plan_revision_id.ok_or_else(|| {
        format!(
            "Local task {} is linked to unpublished local plan revision {resolved_revision_id}. Publish the plan revision first.",
            required_string_field(task, "task_id").unwrap_or_else(|_| "unknown task".to_string())
        )
    })?;
    Ok((
        Some(published_plan_id),
        Some(published_revision_id),
        resolved_plan_item_ref,
    ))
}
