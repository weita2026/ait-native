use super::*;

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
