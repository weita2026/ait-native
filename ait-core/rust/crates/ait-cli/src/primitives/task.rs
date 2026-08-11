use super::*;
use crate::primitives::plan_checklist_closeout::inspect_task_plan_checklist_item;
use crate::runtime::SNAPSHOT_BINARY_DB_WRITE_LAYOUT;
use crate::task_land_contract::attach_task_audit_land_contract;
use ait_core::line_store::{LineRecord, LineStore};
use ait_core::snapshot_store::SnapshotStore;
use ait_core::task_workflow_store::list_changes_with_task_workflow_change_store;
use ait_core::task_workflow_store::{
    close_task_with_task_workflow_task_store, create_task_with_task_workflow_task_store,
    get_task_with_task_workflow_task_store, list_tasks_with_task_workflow_task_store,
    mark_task_published_with_task_workflow_task_store,
};

#[expect(
    clippy::too_many_arguments,
    reason = "arguments mirror the public task creation contract"
)]
pub fn task_create(
    repo: &RepoRuntime,
    title: &str,
    intent: &str,
    local: bool,
    remote_name: Option<&str>,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> Result<JsonValue, String> {
    if repo.task_uses_local_scope(local, remote_name) {
        let store = repo.task_store()?;
        return task_local_create_with_task_store(
            &store,
            &repo.repo_name(),
            title,
            intent,
            Some(&repo.id_namespace_prefix()),
            plan_id,
            origin_plan_revision_id,
            plan_item_ref,
        );
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    task_remote_create_flow_with_task_remote(
        repo,
        &mut task_remote,
        &repo_name,
        title,
        intent,
        None,
        plan_id,
        origin_plan_revision_id,
        plan_item_ref,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn task_local_create_with_task_store<S>(
    task_store: &S,
    repo_name: &str,
    title: &str,
    intent: &str,
    namespace_prefix: Option<&str>,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowTaskCreator + ?Sized,
{
    create_task_with_task_workflow_task_store(
        task_store,
        repo_name,
        title,
        intent,
        namespace_prefix,
        plan_id,
        origin_plan_revision_id,
        plan_item_ref,
    )
    .map_err(|err| err.to_string())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn task_remote_create_flow_with_task_remote<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    repo_name: &str,
    title: &str,
    intent: &str,
    task_id: Option<&str>,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRepositoryReader + TaskWorkflowRemoteTaskCreator + ?Sized,
{
    read_remote_repository_authority(repo, task_remote, repo_name)?;
    task_remote_create_with_task_remote(
        task_remote,
        repo_name,
        title,
        intent,
        task_id,
        plan_id,
        origin_plan_revision_id,
        plan_item_ref,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "remote task creation keeps plan linkage fields explicit"
)]
pub(super) fn task_remote_create_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    title: &str,
    intent: &str,
    task_id: Option<&str>,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteTaskCreator + ?Sized,
{
    let created = task_remote
        .create_task(
            repo_name,
            title,
            intent,
            task_id,
            plan_id,
            origin_plan_revision_id,
            plan_item_ref,
        )
        .map_err(|err| err.to_string())?;
    validate_remote_task_create_response(
        &created,
        repo_name,
        title,
        intent,
        plan_id,
        origin_plan_revision_id,
        plan_item_ref,
    )?;
    Ok(created)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_remote_task_create_response(
    created: &JsonValue,
    repo_name: &str,
    title: &str,
    intent: &str,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> Result<(), String> {
    let task_id = required_string_field(created, "task_id")?;
    let expected = [
        ("repo_name", Some(repo_name)),
        ("title", Some(title)),
        ("intent", Some(intent)),
        ("plan_id", plan_id),
        ("origin_plan_revision_id", origin_plan_revision_id),
        ("plan_item_ref", plan_item_ref),
    ];
    let mismatches = expected
        .into_iter()
        .filter_map(|(field, requested)| {
            let requested = normalized_text(requested);
            let returned = string_field(created, field);
            (requested != returned).then(|| {
                format!(
                    "{field}: requested {}, returned {}",
                    requested.as_deref().unwrap_or("<none>"),
                    returned.as_deref().unwrap_or("<none>")
                )
            })
        })
        .collect::<Vec<_>>();
    if mismatches.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Remote task creation returned existing or unrelated task `{task_id}` instead of the requested task ({}). No initial change was created; repair the server Binary task-id allocator before retrying.",
        mismatches.join(", ")
    ))
}

pub fn task_list(
    repo: &RepoRuntime,
    local: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if repo.task_uses_local_scope(local, remote_name) {
        let store = repo.task_store()?;
        return task_local_list_with_task_store(&store).map(JsonValue::Array);
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    task_remote_list_with_task_remote(&mut task_remote, &repo_name).map(JsonValue::Array)
}

pub(super) fn task_local_list_with_task_store<S>(task_store: &S) -> Result<Vec<JsonValue>, String>
where
    S: TaskWorkflowTaskLister + ?Sized,
{
    list_tasks_with_task_workflow_task_store(task_store).map_err(|err| err.to_string())
}

pub(super) fn task_remote_list_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
) -> Result<Vec<JsonValue>, String>
where
    R: TaskWorkflowRemoteTaskLister + ?Sized,
{
    task_remote
        .list_tasks(repo_name)
        .map_err(|err| err.to_string())
}

pub fn task_show(
    repo: &RepoRuntime,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    if repo.task_uses_local_scope(local, remote_name) {
        let store = repo.task_store()?;
        return task_local_read_with_task_store(&store, task_id);
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    task_remote_read_with_task_remote(&mut task_remote, &repo_name, task_id)
}

pub(super) fn task_local_read_with_task_store<S>(
    task_store: &S,
    task_id: &str,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowTaskReader + ?Sized,
{
    get_task_with_task_workflow_task_store(task_store, task_id).map_err(|err| err.to_string())
}

pub(super) fn task_local_close_with_task_store<S>(
    task_store: &S,
    task_id: &str,
    status: &str,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowTaskCloser + ?Sized,
{
    close_task_with_task_workflow_task_store(task_store, task_id, status)
        .map_err(|err| err.to_string())
}

pub(super) fn task_local_mark_published_with_task_store<S>(
    task_store: &S,
    task_id: &str,
    remote_name: Option<&str>,
    published_task_id: Option<&str>,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowTaskPublisher + ?Sized,
{
    mark_task_published_with_task_workflow_task_store(
        task_store,
        task_id,
        remote_name,
        published_task_id,
    )
    .map_err(|err| err.to_string())
}

pub(super) fn task_remote_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    task_id: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteTaskReader + ?Sized,
{
    task_remote
        .get_task(task_id, Some(repo_name))
        .map_err(|err| err.to_string())
}

pub fn task_tokens(
    repo: &RepoRuntime,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    if repo.task_uses_local_scope(local, remote_name) {
        let task_store = repo.task_store()?;
        let task = task_local_read_with_task_store(&task_store, task_id)?;
        let repo_name =
            required_string_field(&task, "repo_name").unwrap_or_else(|_| repo.repo_name());
        return Ok(empty_task_tokens_report(
            &task,
            json!({"mode": "local", "repo_name": repo_name}),
        ));
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    let task = task_remote_read_with_task_remote(&mut task_remote, &repo_name, task_id)?;
    Ok(empty_task_tokens_report(
        &task,
        json!({"mode": "remote", "repo_name": repo_name}),
    ))
}

pub fn task_audit(
    repo: &RepoRuntime,
    task_id: &str,
    target_line: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let task_store = repo.task_store()?;
    let change_store = repo.change_store()?;
    let local_task = task_local_read_with_task_store(&task_store, task_id).ok();
    if remote_name.is_none()
        && local_task
            .as_ref()
            .is_some_and(task_audit_is_unpublished_local)
    {
        let task = local_task
            .as_ref()
            .ok_or_else(|| format!("Unknown local task: {task_id}"))?;
        let target = local_task_audit_target_info(repo, target_line)?;
        return infer_local_task_audit_with_change_store(
            repo,
            &change_store,
            task,
            task_id,
            target_line,
            &target,
            "local_draft",
            "Local draft task audit used local workflow records directly because this task has not been published to the remote workflow yet.",
        );
    }
    let effective_remote_name = normalized_text(remote_name).or_else(|| {
        local_task
            .as_ref()
            .and_then(|task| string_field(task, "published_remote_name"))
    });
    let (remote_row, repo_name) = remote_context(repo, effective_remote_name.as_deref(), None)?;
    let resolved_remote_name = remote_row.name.clone();
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    let output = if remote_name.is_some() {
        let remote_task_id = local_task
            .as_ref()
            .and_then(|task| string_field(task, "published_task_id"))
            .unwrap_or_else(|| task_id.to_string());
        task_remote_audit_read_with_task_remote(
            &mut task_remote,
            &repo_name,
            &remote_task_id,
            target_line,
        )?
    } else {
        task_audit_with_local_stores_and_task_remote(
            repo,
            &task_store,
            &change_store,
            &mut task_remote,
            &repo_name,
            task_id,
            target_line,
        )?
    };
    Ok(attach_remote_task_plan_closeout_evidence(
        repo,
        output,
        &resolved_remote_name,
    ))
}

fn attach_remote_task_plan_closeout_evidence(
    repo: &RepoRuntime,
    mut output: JsonValue,
    remote_name: &str,
) -> JsonValue {
    let remote_scope = output
        .get("task_land_contract")
        .and_then(|value| value.get("scope"))
        .and_then(JsonValue::as_str)
        == Some("remote");
    let task = output.get("task").cloned().unwrap_or(JsonValue::Null);
    if !remote_scope || string_field(&task, "status").as_deref() != Some("completed") {
        return output;
    }
    let evidence =
        inspect_task_plan_checklist_item(repo, &task, remote_name).unwrap_or_else(|error| {
            json!({
                "status": "unavailable",
                "reason": "remote_plan_read_failed",
                "scope": "remote",
                "remote": remote_name,
                "plan_id": string_field(&task, "plan_id"),
                "plan_item_ref": string_field(&task, "plan_item_ref"),
                "error": error,
            })
        });
    if let Some(object) = output.as_object_mut() {
        object.insert("bound_plan_closeout".to_string(), evidence);
    }
    attach_task_audit_land_contract(&mut output, false);
    output
}

pub(super) fn task_audit_with_local_stores_and_task_remote<T, C, R>(
    repo: &RepoRuntime,
    task_store: &T,
    change_store: &C,
    task_remote: &mut R,
    repo_name: &str,
    task_id: &str,
    target_line: &str,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowTaskReader + ?Sized,
    C: TaskWorkflowChangeLister + ?Sized,
    R: TaskWorkflowLineReader
        + TaskWorkflowSnapshotMetadataReader
        + TaskWorkflowRemoteTaskAuditReader
        + TaskWorkflowRemoteTaskReader
        + ?Sized,
{
    let local_task = task_local_read_with_task_store(task_store, task_id).ok();
    let local_draft_only = local_task
        .as_ref()
        .is_some_and(task_audit_is_unpublished_local);
    if local_draft_only {
        let target = local_task_audit_target_info(repo, target_line)?;
        return infer_local_task_audit_with_change_store(
            repo,
            change_store,
            local_task
                .as_ref()
                .ok_or_else(|| format!("Unknown local task: {task_id}"))?,
            task_id,
            target_line,
            &target,
            "local_draft",
            "Local draft task audit used local workflow records directly because this task has not been published to the remote workflow yet.",
        );
    }
    let remote_task_id = local_task
        .as_ref()
        .and_then(|task| string_field(task, "published_task_id"))
        .unwrap_or_else(|| task_id.to_string());
    match task_remote_audit_read_with_task_remote(
        task_remote,
        repo_name,
        &remote_task_id,
        target_line,
    ) {
        Ok(payload) => Ok(payload),
        Err(err) => {
            if !remote_task_missing(task_remote, repo_name, &remote_task_id) {
                return Err(err);
            }
            let local_task = local_task.ok_or(err)?;
            let target = remote_task_audit_target_info(task_remote, repo, repo_name, target_line)?;
            infer_local_task_audit_with_change_store(
                repo,
                change_store,
                &local_task,
                task_id,
                target_line,
                &target,
                "local_fallback",
                "Remote task audit could not load the task, so local workflow records and line ancestry were used as read-only evidence.",
            )
        }
    }
}

fn task_audit_is_unpublished_local(task: &JsonValue) -> bool {
    string_field(task, "publication_state").as_deref() == Some("local_draft")
        && string_field(task, "published_remote_name").is_none()
        && string_field(task, "published_task_id").is_none()
}

pub(super) fn task_remote_audit_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    task_id: &str,
    target_line: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteTaskAuditReader + ?Sized,
{
    let payload = task_remote
        .read_task_audit(repo_name, task_id, target_line)
        .map_err(|err| err.to_string())?;
    Ok(normalize_task_audit_payload(payload, target_line))
}

pub(super) fn normalize_task_audit_payload(payload: JsonValue, target_line: &str) -> JsonValue {
    let Some(mut object) = payload.as_object().cloned() else {
        return payload;
    };

    if object
        .get("workflow")
        .and_then(JsonValue::as_object)
        .is_none()
    {
        let state = object
            .get("verdict")
            .and_then(|verdict| string_field(verdict, "status"))
            .or_else(|| {
                object
                    .get("verdict")
                    .and_then(|verdict| string_field(verdict, "code"))
            })
            .or_else(|| {
                object
                    .get("summary")
                    .and_then(|summary| string_field(summary, "verdict"))
            })
            .unwrap_or_else(|| "unknown".to_string());
        let reason = if let Some(open_changes) = object
            .get("summary")
            .and_then(|summary| summary.get("open_changes"))
            .and_then(JsonValue::as_i64)
        {
            if open_changes > 0 {
                format!("{open_changes} open change(s) are still linked to this task.")
            } else {
                "Remote task audit reported no open linked changes.".to_string()
            }
        } else {
            "Remote task audit read model did not include workflow detail.".to_string()
        };
        object.insert(
            "workflow".to_string(),
            json!({
                "state": state,
                "reason": reason,
            }),
        );
    }

    if object
        .get("queue_workflow")
        .and_then(JsonValue::as_object)
        .is_none()
    {
        if let Some(workflow) = object.get("workflow").cloned() {
            object.insert("queue_workflow".to_string(), workflow);
        }
    }

    if object
        .get("target")
        .and_then(JsonValue::as_object)
        .is_none()
    {
        object.insert(
            "target".to_string(),
            json!({
                "line_name": object
                    .get("target_line")
                    .and_then(JsonValue::as_str)
                    .and_then(|text| normalized_text(Some(text)))
                    .unwrap_or_else(|| target_line.to_string()),
                "head_snapshot_id": object
                    .get("target_line_head")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
                "source": "remote_task_audit",
            }),
        );
    }

    let task_is_completed = object
        .get("task")
        .and_then(|task| task.get("status"))
        .and_then(JsonValue::as_str)
        == Some("completed");
    if task_is_completed {
        let reason = "The remote Task is already completed.";
        let workflow = json!({
            "state": "task_completed",
            "reason": reason,
        });
        let action = json!({
            "code": "none",
            "label": "No action required",
            "detail": reason,
        });
        object.insert("workflow".to_string(), workflow.clone());
        object.insert("queue_workflow".to_string(), workflow);
        object.insert("recommended_action".to_string(), action.clone());
        object.insert("next_action".to_string(), action);
        object.insert(
            "verdict".to_string(),
            json!({"code": "task_completed", "status": "task_completed"}),
        );
        if let Some(summary) = object.get_mut("summary").and_then(JsonValue::as_object_mut) {
            summary.insert(
                "verdict".to_string(),
                JsonValue::String("task_completed".to_string()),
            );
        }
    }

    let mut output = JsonValue::Object(object);
    attach_task_audit_land_contract(&mut output, false);
    output
}

pub(super) fn remote_context(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<(RemoteRow, String), String> {
    let remote_row = repo.remote_row(remote_name)?;
    let repo_name = normalized_text(repo_name_override).unwrap_or_else(|| {
        remote_row
            .repo_name
            .clone()
            .unwrap_or_else(|| repo.repo_name())
    });
    Ok((remote_row, repo_name))
}

pub(super) fn empty_task_tokens_report(task: &JsonValue, scope: JsonValue) -> JsonValue {
    json!({
        "scope": scope,
        "task": task.clone(),
        "summary": {
            "runtime_count": 0,
            "runtimes_with_usage_count": 0,
            "assistant_reply_count": 0,
            "metered_reply_count": 0,
            "usage_last_reply_count": 0,
            "direct_usage_reply_count": 0,
            "payload_usage_reply_count": 0,
            "missing_usage_reply_count": 0,
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0,
            "cached_input_tokens": 0,
            "reasoning_output_tokens": 0,
            "models": [],
        },
        "changes": [],
        "worktrees": [],
        "models": [],
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "local audit projection keeps exact Task, target, and evidence inputs explicit"
)]
pub(super) fn infer_local_task_audit_with_change_store<C>(
    repo: &RepoRuntime,
    change_store: &C,
    task: &JsonValue,
    task_id: &str,
    target_line: &str,
    target: &JsonValue,
    audit_mode: &str,
    audit_detail: &str,
) -> Result<JsonValue, String>
where
    C: TaskWorkflowChangeLister + ?Sized,
{
    let local_draft_mode = audit_mode == "local_draft";
    let target_ancestry = target
        .get("ancestry")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(|text| text.to_string()))
        .collect::<BTreeSet<_>>();
    let mut changes = task_audit_local_change_rows_with_change_store(change_store)?
        .into_iter()
        .filter(|row| string_field(row, "task_id").as_deref() == Some(task_id))
        .collect::<Vec<_>>();
    changes.sort_by_key(|row| string_field(row, "created_at").unwrap_or_default());
    let all_lines = local_task_audit_lines(repo)?;
    let task_tokens = task_audit_id_tokens(task_id);
    let mut change_rows = Vec::new();

    for change in changes {
        let change_id = required_string_field(&change, "change_id")?;
        let candidates = task_audit_candidate_lines(
            &all_lines,
            target_line,
            &task_tokens,
            &task_audit_id_tokens(&change_id),
        );
        let preferred_line = candidates.first().cloned();
        let preferred_snapshot_id = preferred_line
            .as_ref()
            .and_then(|row| string_field(row, "head_snapshot_id"));
        let on_target_candidate_count = candidates
            .iter()
            .filter(|row| {
                string_field(row, "head_snapshot_id")
                    .is_some_and(|snapshot_id| target_ancestry.contains(&snapshot_id))
            })
            .count();
        let inferred_on_target = preferred_snapshot_id
            .as_ref()
            .is_some_and(|snapshot_id| target_ancestry.contains(snapshot_id));
        let effective_on_target = !local_draft_mode && inferred_on_target;
        let stale_workflow_record = !local_draft_mode
            && effective_on_target
            && !matches!(
                string_field(&change, "status").as_deref(),
                Some("landed" | "archived")
            );
        let (target_state, target_reason) = if string_field(&change, "status").as_deref()
            == Some("archived")
        {
            (
                "archived".to_string(),
                "This change is archived and no longer blocks task completion.".to_string(),
            )
        } else if local_draft_mode && string_field(&change, "status").as_deref() == Some("landed") {
            (
                "local_change_landed".to_string(),
                format!(
                    "This local Change is landed; Task closeout should be resumed against {target_line}."
                ),
            )
        } else if local_draft_mode {
            (
                "local_change_not_landed".to_string(),
                format!("This local Change has not been landed onto {target_line}."),
            )
        } else if effective_on_target {
            (
                "merged_on_target_missing_remote".to_string(),
                format!(
                    "The preferred inferred line head is already reachable from {target_line}, but the remote workflow record is missing."
                ),
            )
        } else if on_target_candidate_count > 0 {
            (
                "ambiguous_line_candidates".to_string(),
                format!(
                    "{on_target_candidate_count} lower-confidence candidate line(s) appear on {target_line}, but the preferred inferred line does not."
                ),
            )
        } else if preferred_line.is_none() {
            (
                "no_line_evidence".to_string(),
                "No local line could be linked to this change strongly enough to infer target-line reachability.".to_string(),
            )
        } else if preferred_snapshot_id.is_none() {
            (
                "line_missing_head".to_string(),
                "The preferred inferred line does not currently have a head snapshot.".to_string(),
            )
        } else {
            (
                "not_on_target".to_string(),
                format!("The preferred inferred line head is not reachable from {target_line}."),
            )
        };
        change_rows.push(json!({
            "change": change,
            "current_patchset": JsonValue::Null,
            "selected_patchset": JsonValue::Null,
            "display_patchset": preferred_snapshot_id.as_ref().map(|snapshot_id| {
                json!({
                    "patchset_id": JsonValue::Null,
                    "revision_snapshot_id": snapshot_id,
                })
            }).unwrap_or(JsonValue::Null),
            "landing_summary": JsonValue::Null,
            "effective_on_target": effective_on_target,
            "stale_workflow_record": stale_workflow_record,
            "missing_remote_record": !local_draft_mode,
            "target_state": target_state,
            "target_reason": target_reason,
            "preferred_line": preferred_line.unwrap_or(JsonValue::Null),
            "candidate_lines": candidates,
        }));
    }

    let verdict = build_task_audit_verdict_payload(
        task,
        &JsonValue::Array(change_rows.clone()),
        target_line,
    )?;
    let verdict_obj = verdict
        .as_object()
        .ok_or_else(|| "task audit verdict payload must decode to an object".to_string())?;
    let mut output = json!({
        "task": task.clone(),
        "repository": {
            "repo_name": string_field(task, "repo_name").unwrap_or_else(|| repo.repo_name()),
            "default_line": repo.default_line_name(),
        },
        "workflow": verdict_obj.get("workflow").cloned().unwrap_or(JsonValue::Null),
        "queue_workflow": verdict_obj.get("workflow").cloned().unwrap_or(JsonValue::Null),
        "next_action": verdict_obj.get("recommended_action").cloned().unwrap_or(JsonValue::Null),
        "recommended_action": verdict_obj.get("recommended_action").cloned().unwrap_or(JsonValue::Null),
        "audit_source": {
            "mode": audit_mode,
            "detail": audit_detail,
            "remote_task_missing": !local_draft_mode,
        },
        "target": {
            "line_name": target.get("line_name").cloned().unwrap_or(JsonValue::Null),
            "head_snapshot_id": target.get("head_snapshot_id").cloned().unwrap_or(JsonValue::Null),
            "ancestor_snapshot_count": target.get("ancestor_snapshot_count").cloned().unwrap_or(JsonValue::from(0)),
            "source": target.get("source").cloned().unwrap_or(JsonValue::Null),
        },
        "summary": verdict_obj.get("summary").cloned().unwrap_or(JsonValue::Null),
        "changes": change_rows,
    });
    attach_task_audit_land_contract(&mut output, true);
    Ok(output)
}

pub(super) fn task_audit_local_change_rows_with_change_store<S>(
    change_store: &S,
) -> Result<Vec<JsonValue>, String>
where
    S: TaskWorkflowChangeLister + ?Sized,
{
    list_changes_with_task_workflow_change_store(change_store).map_err(|err| err.to_string())
}

pub(super) fn local_task_audit_lines(repo: &RepoRuntime) -> Result<Vec<JsonValue>, String> {
    let store = task_audit_line_store(repo)?;
    local_task_audit_lines_with_line_store(&store)
}

pub(super) fn local_task_audit_lines_with_line_store<S>(
    line_store: &S,
) -> Result<Vec<JsonValue>, String>
where
    S: LineStore + ?Sized,
{
    line_store
        .list_lines()?
        .into_iter()
        .map(|line| Ok(task_audit_line_record_json(&line)))
        .collect()
}

fn task_audit_line_store(repo: &RepoRuntime) -> Result<impl LineStore, String> {
    repo.line_store()
}

fn task_audit_line_record_json(line: &LineRecord) -> JsonValue {
    json!({
        "line_id": &line.line_id,
        "line_name": &line.line_name,
        "status": &line.status,
        "archived_at": &line.archived_at,
        "created_at": &line.created_at,
        "updated_at": &line.updated_at,
        "head_snapshot_id": &line.head_snapshot_id,
    })
}

pub(super) fn local_task_audit_target_info(
    repo: &RepoRuntime,
    target_line: &str,
) -> Result<JsonValue, String> {
    let line_store = task_audit_line_store(repo)?;
    let snapshot_store = task_audit_snapshot_store(repo)?;
    local_task_audit_target_info_with_stores(&line_store, &snapshot_store, target_line)
}

pub(super) fn local_task_audit_target_info_with_stores<L, S>(
    line_store: &L,
    snapshot_store: &S,
    target_line: &str,
) -> Result<JsonValue, String>
where
    L: LineStore + ?Sized,
    S: SnapshotStore + ?Sized,
{
    let line = line_store
        .line_by_name(target_line)?
        .ok_or_else(|| format!("Unknown line: {target_line}"))?;
    let head_snapshot_id = line.head_snapshot_id.clone();
    let ancestry = match head_snapshot_id.as_deref() {
        Some(snapshot_id) => {
            snapshot_ancestor_closure(
                snapshot_store,
                &[snapshot_id.to_string()],
                &BTreeSet::new(),
                SnapshotParentMode::AllParents,
                SnapshotDagLimits::default(),
            )?
            .topological_snapshot_ids
        }
        None => Vec::new(),
    };
    Ok(json!({
        "line_id": line.line_id,
        "line_name": line.line_name,
        "head_snapshot_id": head_snapshot_id,
        "ancestor_snapshot_count": ancestry.len(),
        "source": "local",
        "ancestry": ancestry,
    }))
}

fn task_audit_snapshot_store(repo: &RepoRuntime) -> Result<impl SnapshotStore, String> {
    let workspace_root = repo.workspace_root();
    repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)
}

pub(super) fn remote_task_audit_target_info<R>(
    task_remote: &mut R,
    repo: &RepoRuntime,
    repo_name: &str,
    target_line: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader + TaskWorkflowSnapshotMetadataReader + ?Sized,
{
    match task_audit_remote_target_line_read_with_task_remote(task_remote, repo_name, target_line) {
        Ok(line) => {
            let head_snapshot_id = string_field(&line, "head_snapshot_id");
            let ancestry =
                remote_snapshot_ancestry(task_remote, repo_name, head_snapshot_id.as_deref())?;
            Ok(json!({
                "line_name": target_line,
                "head_snapshot_id": head_snapshot_id,
                "ancestor_snapshot_count": ancestry.len(),
                "source": "remote",
                "ancestry": ancestry,
            }))
        }
        Err(_) => local_task_audit_target_info(repo, target_line),
    }
}

pub(super) fn task_audit_remote_target_line_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    target_line: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader + ?Sized,
{
    task_remote
        .get_line(repo_name, target_line)
        .map_err(|err| err.to_string())
}

pub(super) fn remote_snapshot_ancestry<R>(
    task_remote: &mut R,
    repo_name: &str,
    snapshot_id: Option<&str>,
) -> Result<Vec<String>, String>
where
    R: TaskWorkflowSnapshotMetadataReader + ?Sized,
{
    let Some(head_snapshot_id) = snapshot_id.map(|value| value.to_string()) else {
        return Ok(Vec::new());
    };
    let limits = SnapshotDagLimits::default();
    let mut pending = VecDeque::from([head_snapshot_id.clone()]);
    let mut queued = BTreeSet::from([head_snapshot_id.clone()]);
    let mut parent_map = BTreeMap::new();
    while let Some(current) = pending.pop_front() {
        let bundle = task_audit_remote_snapshot_metadata_read_with_task_remote(
            task_remote,
            repo_name,
            &current,
        )?;
        let parents = task_audit_remote_parent_snapshot_ids(&bundle, &current)?;
        for parent in &parents {
            if queued.insert(parent.clone()) {
                if queued.len() > limits.max_results {
                    return Err(format!(
                        "Remote task-audit Snapshot DAG exceeded max_results {} at parent {parent} of {current}.",
                        limits.max_results
                    ));
                }
                pending.push_back(parent.clone());
            }
        }
        parent_map.insert(current, parents);
    }
    Ok(snapshot_ancestor_closure_from_parent_map(
        &parent_map,
        &[head_snapshot_id],
        &BTreeSet::new(),
        SnapshotParentMode::AllParents,
        limits,
    )?
    .topological_snapshot_ids)
}

fn task_audit_remote_parent_snapshot_ids(
    snapshot: &JsonValue,
    snapshot_id: &str,
) -> Result<Vec<String>, String> {
    let parent_snapshot_ids = match snapshot.get("parent_snapshot_ids") {
        None => None,
        Some(JsonValue::Array(values)) => Some(
            values
                .iter()
                .enumerate()
                .map(|(ordinal, value)| {
                    value.as_str().map(str::to_string).ok_or_else(|| {
                        format!(
                            "Remote snapshot {snapshot_id} parent ordinal {ordinal} must be text."
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => {
            return Err(format!(
                "Remote snapshot {snapshot_id} parent_snapshot_ids must be an array."
            ))
        }
    };
    normalize_snapshot_parent_set(
        Some(snapshot_id),
        parent_snapshot_ids,
        string_field(snapshot, "primary_parent_snapshot_id"),
        string_field(snapshot, "parent_snapshot_id"),
    )
    .map(|(parents, _, _)| parents)
}

pub(super) fn task_audit_remote_snapshot_metadata_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    snapshot_id: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowSnapshotMetadataReader + ?Sized,
{
    let remote_snapshot = task_remote
        .get_remote_snapshot(repo_name, snapshot_id, false, None)
        .map_err(|err| err.to_string())?;
    let remote_snapshot_id = string_field(&remote_snapshot, "snapshot_id")
        .ok_or_else(|| "Remote snapshot response is missing snapshot_id.".to_string())?;
    if remote_snapshot_id != snapshot_id {
        return Err(format!(
            "Remote snapshot verification returned unexpected snapshot {remote_snapshot_id:?} (expected {snapshot_id:?})"
        ));
    }
    let remote_repo_name = string_field(&remote_snapshot, "repo_name")
        .ok_or_else(|| "Remote snapshot response is missing repo_name.".to_string())?;
    if remote_repo_name != repo_name {
        return Err(format!(
            "Remote snapshot verification returned unexpected repository {remote_repo_name:?} (expected {repo_name:?})"
        ));
    }
    Ok(remote_snapshot)
}

pub(super) fn remote_task_missing<R>(task_remote: &mut R, repo_name: &str, task_id: &str) -> bool
where
    R: TaskWorkflowRemoteTaskReader + ?Sized,
{
    match task_remote_read_with_task_remote(task_remote, repo_name, task_id) {
        Ok(_) => false,
        Err(message) => {
            message.contains(" failed: 404")
                || message.contains("failed: 404 ")
                || message.contains("Unknown task")
                || message.contains("is not a Task in this repository namespace")
        }
    }
}

pub(super) fn task_audit_id_tokens(workflow_id: &str) -> Vec<String> {
    let text = workflow_id.trim().to_ascii_lowercase();
    if text.is_empty() {
        return Vec::new();
    }
    let mut tokens = vec![text.clone()];
    if let Some((prefix, suffix)) = text.split_once('-') {
        if suffix.len() > 8 {
            let short = format!("{prefix}-{}", &suffix[..8]);
            if !tokens.contains(&short) {
                tokens.push(short);
            }
        }
    }
    tokens
}

pub(super) fn task_audit_reason_rank(reason: &str) -> i32 {
    match reason {
        "change_id" => 0,
        "task_id" => 1,
        _ => 99,
    }
}

pub(super) fn task_audit_candidate_lines(
    lines: &[JsonValue],
    target_line: &str,
    task_tokens: &[String],
    change_tokens: &[String],
) -> Vec<JsonValue> {
    let mut candidates = lines
        .iter()
        .filter_map(JsonValue::as_object)
        .filter_map(|line| {
            let line_name = string_field(&JsonValue::Object(line.clone()), "line_name")?;
            if line_name == target_line {
                return None;
            }
            let lowered = line_name.to_ascii_lowercase();
            let mut reasons = Vec::new();
            if change_tokens.iter().any(|token| lowered.contains(token)) {
                reasons.push("change_id".to_string());
            }
            if task_tokens.iter().any(|token| lowered.contains(token)) {
                reasons.push("task_id".to_string());
            }
            if reasons.is_empty() {
                return None;
            }
            let mut candidate = line.clone();
            candidate.insert(
                "match_reasons".to_string(),
                JsonValue::Array(reasons.into_iter().map(JsonValue::String).collect()),
            );
            Some(JsonValue::Object(candidate))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        let left_rank = left
            .get("match_reasons")
            .and_then(JsonValue::as_array)
            .map(|reasons| {
                reasons
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .map(task_audit_reason_rank)
                    .min()
                    .unwrap_or(99)
            })
            .unwrap_or(99);
        let right_rank = right
            .get("match_reasons")
            .and_then(JsonValue::as_array)
            .map(|reasons| {
                reasons
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .map(task_audit_reason_rank)
                    .min()
                    .unwrap_or(99)
            })
            .unwrap_or(99);
        left_rank
            .cmp(&right_rank)
            .then_with(|| string_field(right, "updated_at").cmp(&string_field(left, "updated_at")))
    });
    candidates
}

pub(super) fn http_config(repo: &RepoRuntime, remote_row: &RemoteRow) -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: remote_row.url.clone(),
        repository_index: repo.repository_index(),
        headers: repo.auth_headers(),
        ..PlanHttpClientConfig::default()
    }
}

pub(super) fn http_task_remote(
    repo: &RepoRuntime,
    remote_row: &RemoteRow,
) -> Result<HttpTaskRemote, String> {
    let mut remote =
        HttpTaskRemote::new(http_config(repo, remote_row)).map_err(|err| err.to_string())?;
    if let Some(metadata) = current_worktree_metadata(repo)? {
        remote.set_bound_change_identity_context(
            metadata.bound_task_id.as_deref(),
            metadata.bound_change_id.as_deref(),
            metadata.bound_change_ref.as_deref(),
        )?;
    }
    Ok(remote)
}

pub(super) fn http_closeout_remote(
    repo: &RepoRuntime,
    remote_row: &RemoteRow,
) -> Result<HttpWorkflowCloseoutRemote, String> {
    let mut remote = HttpWorkflowCloseoutRemote::new(http_config(repo, remote_row))
        .map_err(|err| err.to_string())?;
    if let Some(metadata) = current_worktree_metadata(repo)? {
        remote.set_bound_change_identity_context(
            metadata.bound_task_id.as_deref(),
            metadata.bound_change_id.as_deref(),
            metadata.bound_change_ref.as_deref(),
        )?;
    }
    Ok(remote)
}
