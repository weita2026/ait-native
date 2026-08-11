use super::{
    http_task_remote, list_remote_names, normalized_text, remote_context, string_field,
    workflow_workspace_status, worktree_doctor,
};
use crate::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonValue};
use ait_core::task_workflow_http_adapter::{
    TaskWorkflowQueueChangeLister, TaskWorkflowQueueSummaryBundleReader,
    TaskWorkflowReviewerInboxReader, TaskWorkflowTaskQueueReader,
};
use std::collections::{BTreeMap, BTreeSet};

fn queue_actionable_local_tasks(local_tasks: &[JsonValue]) -> Vec<JsonValue> {
    local_tasks
        .iter()
        .filter(|row| string_field(row, "publication_state").as_deref() != Some("published"))
        .filter(|row| string_field(row, "status").as_deref() == Some("active"))
        .cloned()
        .collect()
}

fn queue_actionable_local_changes(local_changes: &[JsonValue]) -> Vec<JsonValue> {
    local_changes
        .iter()
        .filter(|row| string_field(row, "publication_state").as_deref() != Some("published"))
        .filter(|row| {
            !matches!(
                string_field(row, "status").as_deref(),
                Some("archived" | "landed")
            )
        })
        .cloned()
        .collect()
}

fn queue_local_summary(local_tasks: &[JsonValue], local_changes: &[JsonValue]) -> JsonValue {
    let unpublished_tasks = local_tasks
        .iter()
        .filter(|row| string_field(row, "publication_state").as_deref() != Some("published"))
        .count() as i64;
    let published_tasks = local_tasks
        .iter()
        .filter(|row| string_field(row, "publication_state").as_deref() == Some("published"))
        .count() as i64;
    let unpublished_changes = local_changes
        .iter()
        .filter(|row| string_field(row, "publication_state").as_deref() != Some("published"))
        .count() as i64;
    let published_changes = local_changes
        .iter()
        .filter(|row| string_field(row, "publication_state").as_deref() == Some("published"))
        .count() as i64;
    let actionable_draft_tasks = queue_actionable_local_tasks(local_tasks);
    let actionable_draft_changes = queue_actionable_local_changes(local_changes);
    json!({
        "task_record_count": local_tasks.len(),
        "change_record_count": local_changes.len(),
        "draft_task_count": actionable_draft_tasks.len(),
        "published_task_count": published_tasks,
        "draft_change_count": actionable_draft_changes.len(),
        "published_change_count": published_changes,
        "unpublished_task_record_count": unpublished_tasks,
        "unpublished_change_record_count": unpublished_changes,
        "active_draft_task_count": actionable_draft_tasks.len(),
        "open_draft_change_count": actionable_draft_changes.len(),
    })
}

fn queue_focus_change_reasons(task_items: &[JsonValue]) -> BTreeMap<String, String> {
    let mut reasons = BTreeMap::new();
    for item in task_items {
        let focus_change = item.get("focus_change").and_then(JsonValue::as_object);
        let next_action = item.get("next_action").and_then(JsonValue::as_object);
        let change_id = focus_change
            .and_then(|value| value.get("change_id"))
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                next_action
                    .and_then(|value| value.get("change_id"))
                    .and_then(JsonValue::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            });
        let Some(change_id) = change_id else {
            continue;
        };
        let reason = focus_change
            .and_then(|value| value.get("reason"))
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                next_action
                    .and_then(|value| value.get("detail"))
                    .and_then(JsonValue::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            });
        if let Some(reason) = reason {
            reasons.insert(change_id.to_string(), reason.to_string());
        }
    }
    reasons
}

fn queue_change_reason(
    change: &JsonValue,
    reviewer_item: Option<&JsonValue>,
    focus_reason: Option<&str>,
) -> String {
    if let Some(reason) = normalized_text(focus_reason) {
        return reason;
    }
    if change
        .get("current_patchset_number")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        <= 0
    {
        return "No published patchset exists yet.".to_string();
    }
    let Some(reviewer_item) = reviewer_item else {
        return String::new();
    };
    let review_state = reviewer_item
        .get("review_state")
        .and_then(JsonValue::as_object);
    let freshness = reviewer_item
        .get("freshness")
        .and_then(JsonValue::as_object);
    let attestation = reviewer_item
        .get("attestation")
        .and_then(JsonValue::as_object);
    let policy_state = reviewer_item
        .get("policy_state")
        .and_then(JsonValue::as_object);
    let missing_requirements = policy_state
        .and_then(|value| value.get("missing_requirements"))
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    if review_state
        .and_then(|value| value.get("blocking"))
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        > 0
    {
        return "Blocking review feedback is recorded on this change.".to_string();
    }
    if freshness
        .and_then(|value| value.get("base_is_fresh"))
        .and_then(JsonValue::as_bool)
        == Some(false)
    {
        return "The base line moved after this patchset was published.".to_string();
    }
    let attestation_state = attestation
        .and_then(|value| value.get("completeness"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            attestation
                .and_then(|value| value.get("source"))
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        });
    if attestation_state == Some("missing") {
        return "Attestation is missing for the current patchset.".to_string();
    }
    if missing_requirements.contains("tests")
        || attestation
            .and_then(|value| value.get("tests"))
            .and_then(JsonValue::as_str)
            == Some("pending")
    {
        return "Tests are still pending for the current patchset.".to_string();
    }
    if missing_requirements.contains("required_human_review") {
        return "The change still needs a human approval.".to_string();
    }
    match policy_state
        .and_then(|value| value.get("decision"))
        .and_then(JsonValue::as_str)
    {
        Some("pass") => "Ready to land.".to_string(),
        Some("pending") => "Policy evaluation is still pending.".to_string(),
        _ => String::new(),
    }
}

fn queue_change_ready_to_land(change: &JsonValue, reviewer_item: Option<&JsonValue>) -> bool {
    if change
        .get("current_patchset_number")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        <= 0
    {
        return false;
    }
    let Some(reviewer_item) = reviewer_item else {
        return false;
    };
    let review_state = reviewer_item
        .get("review_state")
        .and_then(JsonValue::as_object);
    let freshness = reviewer_item
        .get("freshness")
        .and_then(JsonValue::as_object);
    let policy_state = reviewer_item
        .get("policy_state")
        .and_then(JsonValue::as_object);
    policy_state
        .and_then(|value| value.get("decision"))
        .and_then(JsonValue::as_str)
        == Some("pass")
        && freshness
            .and_then(|value| value.get("base_is_fresh"))
            .and_then(JsonValue::as_bool)
            != Some(false)
        && review_state
            .and_then(|value| value.get("blocking"))
            .and_then(JsonValue::as_i64)
            .unwrap_or(0)
            == 0
}

fn queue_change_inventory(
    change_rows: &[JsonValue],
    task_items: &[JsonValue],
    review_items: &[JsonValue],
) -> Vec<JsonValue> {
    let focus_reasons = queue_focus_change_reasons(task_items);
    let reviewer_items_by_change = review_items
        .iter()
        .filter_map(|item| {
            let change_id = item
                .get("change_id")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            Some((change_id.to_string(), item.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut inventory = Vec::new();
    for row in change_rows {
        if matches!(
            string_field(row, "status").as_deref(),
            Some("landed" | "archived")
        ) {
            continue;
        }
        let change_id = string_field(row, "change_id").unwrap_or_default();
        let reviewer_item = reviewer_items_by_change.get(&change_id);
        let mut enriched = row.as_object().cloned().unwrap_or_default();
        enriched.insert(
            "ready_to_land".to_string(),
            JsonValue::Bool(queue_change_ready_to_land(row, reviewer_item)),
        );
        enriched.insert(
            "reason".to_string(),
            JsonValue::String(queue_change_reason(
                row,
                reviewer_item,
                focus_reasons.get(&change_id).map(String::as_str),
            )),
        );
        inventory.push(JsonValue::Object(enriched));
    }
    inventory
}

fn queue_summary_bundle_missing(err: &str) -> bool {
    err.contains("/v1/native/repository-authorities/")
        && err.contains("/read/queue-summary")
        && err.contains("404")
}

pub(super) fn queue_remote_reads_with_task_remote<R>(
    task_remote: &mut R,
    mut remote_section: JsonValue,
    repo_name: &str,
    status: &str,
    include_all_changes: bool,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowQueueSummaryBundleReader
        + TaskWorkflowTaskQueueReader
        + TaskWorkflowReviewerInboxReader
        + TaskWorkflowQueueChangeLister
        + ?Sized,
{
    let bundle_result = {
        let _range = perfetto_range!("ait.cli.queue.remote.summary_bundle_http");
        queue_remote_summary_bundle_with_task_remote(task_remote, repo_name, status)
    };
    match bundle_result {
        Ok(bundle) => {
            remote_section["task_queue"] =
                bundle.get("task_queue").cloned().unwrap_or(JsonValue::Null);
            remote_section["reviewer_inbox"] = bundle
                .get("reviewer_inbox")
                .cloned()
                .unwrap_or(JsonValue::Null);
        }
        Err(err) if queue_summary_bundle_missing(&err.to_string()) => {
            match queue_remote_task_queue_with_task_remote(task_remote, repo_name, status) {
                Ok(task_queue) => remote_section["task_queue"] = task_queue,
                Err(read_err) => {
                    remote_section["error"] = JsonValue::String(read_err.to_string());
                    return Ok(remote_section);
                }
            }
            match queue_remote_reviewer_inbox_with_task_remote(task_remote, repo_name) {
                Ok(reviewer_inbox) => remote_section["reviewer_inbox"] = reviewer_inbox,
                Err(read_err) => {
                    remote_section["task_queue"] = JsonValue::Null;
                    remote_section["reviewer_inbox"] = JsonValue::Null;
                    remote_section["error"] = JsonValue::String(read_err.to_string());
                    return Ok(remote_section);
                }
            }
        }
        Err(err) => {
            remote_section["error"] = JsonValue::String(err.to_string());
            return Ok(remote_section);
        }
    }

    if include_all_changes {
        let task_items = remote_section
            .get("task_queue")
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("items"))
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        let review_items = remote_section
            .get("reviewer_inbox")
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("items"))
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        let change_rows_result = {
            let _range = perfetto_range!("ait.cli.queue.remote.change_list_http");
            queue_remote_change_rows_with_task_remote(task_remote, repo_name)
        };
        match change_rows_result {
            Ok(change_rows) => {
                let _range = perfetto_range!("ait.cli.queue.remote.change_inventory");
                remote_section["changes"] = JsonValue::Array(queue_change_inventory(
                    &change_rows,
                    &task_items,
                    &review_items,
                ));
            }
            Err(err) => {
                remote_section["error"] = JsonValue::String(err.to_string());
            }
        }
    }

    Ok(remote_section)
}

pub(super) fn queue_remote_summary_bundle_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    status: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowQueueSummaryBundleReader + ?Sized,
{
    task_remote
        .read_queue_summary_bundle(repo_name, Some(status))
        .map_err(|err| err.to_string())
}

pub(super) fn queue_remote_task_queue_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    status: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowTaskQueueReader + ?Sized,
{
    task_remote
        .read_task_queue(repo_name, Some(status))
        .map_err(|err| err.to_string())
}

pub(super) fn queue_remote_reviewer_inbox_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowReviewerInboxReader + ?Sized,
{
    task_remote
        .read_reviewer_inbox(repo_name)
        .map_err(|err| err.to_string())
}

pub(super) fn queue_remote_change_rows_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
) -> Result<Vec<JsonValue>, String>
where
    R: TaskWorkflowQueueChangeLister + ?Sized,
{
    task_remote
        .list_changes(repo_name)
        .map_err(|err| err.to_string())
}

fn queue_remote_section(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    status: &str,
    include_all_changes: bool,
) -> Result<JsonValue, String> {
    let _section_range = perfetto_range!("ait.cli.queue.remote");
    let available_remotes = list_remote_names(repo)?;
    let mut remote_section = json!({
        "configured": false,
        "remote_name": JsonValue::Null,
        "repo_name": repo.repo_name(),
        "url": JsonValue::Null,
        "status_filter": status,
        "available_remotes": available_remotes,
        "task_queue": JsonValue::Null,
        "reviewer_inbox": JsonValue::Null,
        "changes": JsonValue::Null,
        "error": JsonValue::Null,
    });
    let should_attempt_remote =
        normalized_text(remote_name).is_some() || repo.default_remote_name().is_some();
    if !should_attempt_remote {
        if !available_remotes.is_empty() {
            remote_section["error"] = JsonValue::String(
                "No default remote configured. Set one first, or pass --remote <name> for this queue read."
                    .to_string(),
            );
        }
        return Ok(remote_section);
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    remote_section["configured"] = JsonValue::Bool(true);
    remote_section["remote_name"] = JsonValue::String(remote_row.name.clone());
    remote_section["repo_name"] = JsonValue::String(
        remote_row
            .repo_name
            .clone()
            .unwrap_or_else(|| repo_name.clone()),
    );
    remote_section["url"] = JsonValue::String(remote_row.url.clone());

    let mut task_remote = http_task_remote(repo, &remote_row)?;
    queue_remote_reads_with_task_remote(
        &mut task_remote,
        remote_section,
        &repo_name,
        status,
        include_all_changes,
    )
}

pub fn queue_summary(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    status: &str,
    include_all_changes: bool,
) -> Result<JsonValue, String> {
    let _summary_range = perfetto_range!("ait.cli.queue.summary");
    let workspace = {
        let _range = perfetto_range!("ait.cli.queue.workspace_status");
        workflow_workspace_status(repo, None, None)?
    };
    let worktrees = {
        let _range = perfetto_range!("ait.cli.queue.worktree_doctor");
        worktree_doctor(repo, false)?
    };
    let local_store = repo.binary_db_stores::<1>().workflows();
    let local_tasks = ait_core::task_store::TaskStore::list_tasks(&local_store)
        .map_err(|error| error.to_string())?;
    let local_changes = ait_core::change_store::ChangeStore::list_changes(&local_store)
        .map_err(|error| error.to_string())?;
    let actionable_local_tasks = queue_actionable_local_tasks(&local_tasks);
    let actionable_local_changes = queue_actionable_local_changes(&local_changes);
    let local_summary = queue_local_summary(&local_tasks, &local_changes);
    let remote_section = {
        let _range = perfetto_range!("ait.cli.queue.remote_section");
        queue_remote_section(repo, remote_name, status, include_all_changes)?
    };
    let task_queue = remote_section
        .get("task_queue")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let reviewer_inbox = remote_section
        .get("reviewer_inbox")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let task_queue_summary = task_queue
        .get("summary")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let remote_changes = remote_section
        .get("changes")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let _range = perfetto_range!("ait.cli.queue.assemble");
    Ok(json!({
        "repo_name": repo.repo_name(),
        "query": {
            "all_changes": include_all_changes,
            "status": status,
        },
        "remote": remote_section,
        "local": {
            "available": true,
            "authority": "local_binary_v0",
            "tasks": actionable_local_tasks,
            "changes": actionable_local_changes,
            "all_tasks": local_tasks,
            "all_changes": local_changes,
            "summary": local_summary,
        },
        "workspace": {
            "status": workspace,
            "worktrees": worktrees,
        },
        "summary": {
            "shared_task_count": task_queue.get("count").and_then(JsonValue::as_i64).unwrap_or(0),
            "attention_required_count": task_queue_summary.get("attention_required").and_then(JsonValue::as_i64).unwrap_or(0),
            "ready_to_land_count": task_queue_summary.get("ready_to_land").and_then(JsonValue::as_i64).unwrap_or(0),
            "ready_to_complete_count": task_queue_summary.get("ready_to_complete").and_then(JsonValue::as_i64).unwrap_or(0),
            "open_shared_change_count": remote_changes.len(),
            "reviewer_inbox_count": reviewer_inbox.get("count").and_then(JsonValue::as_i64).unwrap_or(0),
            "local_draft_task_count": local_summary.get("draft_task_count").and_then(JsonValue::as_i64).unwrap_or(0),
            "local_draft_change_count": local_summary.get("draft_change_count").and_then(JsonValue::as_i64).unwrap_or(0),
            "workspace_dirty": workspace.get("clean").and_then(JsonValue::as_bool) == Some(false),
            "workspace_changed_count": workspace.get("changed_count").and_then(JsonValue::as_i64).unwrap_or(0),
            "dirty_worktree_count": worktrees.get("dirty_count").and_then(JsonValue::as_i64).unwrap_or(0),
            "stale_worktree_count": worktrees.get("stale_count").and_then(JsonValue::as_i64).unwrap_or(0),
        },
    }))
}

#[cfg(test)]
mod tests;
