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
    reason = "internal Task record creation keeps canonical Plan linkage explicit"
)]
pub(in crate::primitives) fn task_create(
    repo: &RepoRuntime,
    title: &str,
    intent: &str,
    local: bool,
    remote_name: Option<&str>,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> Result<JsonValue, String> {
    if repo.task_uses_local_scope(local, remote_name)? {
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
    if repo.task_uses_local_scope(local, remote_name)? {
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
) -> Result<JsonValue, String> {
    if repo.task_uses_local_scope(local, remote_name)? {
        let store = repo.task_store()?;
        return task_local_read_with_task_store(&store, task_id);
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
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

pub fn task_audit(
    repo: &RepoRuntime,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    const TARGET_LINE: &str = "main";

    let task_store = repo.task_store()?;
    let change_store = repo.change_store()?;
    let local_task = task_local_read_with_task_store(&task_store, task_id).ok();
    if repo.task_uses_local_scope(local, remote_name)? {
        let task = local_task
            .as_ref()
            .ok_or_else(|| format!("Unknown local task: {task_id}"))?;
        let target = local_task_audit_target_info(repo, TARGET_LINE)?;
        return infer_local_task_audit_with_change_store(
            repo,
            &change_store,
            task,
            task_id,
            TARGET_LINE,
            &target,
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
    let remote_task_id = local_task
        .as_ref()
        .filter(|task| {
            string_field(task, "published_remote_name").as_deref()
                == Some(resolved_remote_name.as_str())
        })
        .and_then(|task| string_field(task, "published_task_id"))
        .unwrap_or_else(|| task_id.to_string());
    let output = task_remote_audit_read_with_task_remote(
        &mut task_remote,
        &repo_name,
        &remote_task_id,
        TARGET_LINE,
    )?;
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

pub(super) fn infer_local_task_audit_with_change_store<C>(
    repo: &RepoRuntime,
    change_store: &C,
    task: &JsonValue,
    task_id: &str,
    target_line: &str,
    target: &JsonValue,
) -> Result<JsonValue, String>
where
    C: TaskWorkflowChangeLister + ?Sized,
{
    let mut changes = task_audit_local_change_rows_with_change_store(change_store)?
        .into_iter()
        .filter(|row| string_field(row, "task_id").as_deref() == Some(task_id))
        .collect::<Vec<_>>();
    changes.sort_by_key(|row| string_field(row, "created_at").unwrap_or_default());
    let all_lines = local_task_audit_lines(repo)?;
    let task_match_terms = task_audit_id_tokens(task_id);
    let mut change_rows = Vec::new();

    for change in changes {
        let change_id = required_string_field(&change, "change_id")?;
        let candidates = task_audit_candidate_lines(
            &all_lines,
            target_line,
            &task_match_terms,
            &task_audit_id_tokens(&change_id),
        );
        let preferred_line = candidates.first().cloned();
        let preferred_snapshot_id = preferred_line
            .as_ref()
            .and_then(|row| string_field(row, "head_snapshot_id"));
        let (target_state, target_reason) = if string_field(&change, "status").as_deref()
            == Some("archived")
        {
            (
                "archived".to_string(),
                "This change is archived and no longer blocks task completion.".to_string(),
            )
        } else if string_field(&change, "status").as_deref() == Some("landed") {
            (
                "local_change_landed".to_string(),
                format!(
                    "This local Change is landed; Task closeout should be resumed against {target_line}."
                ),
            )
        } else {
            (
                "local_change_not_landed".to_string(),
                format!("This local Change has not been landed onto {target_line}."),
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
            "effective_on_target": false,
            "stale_workflow_record": false,
            "missing_remote_record": false,
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
            "mode": "local",
            "detail": "Task audit used only local Task, Change, Line, and Snapshot authority.",
            "remote_task_missing": false,
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
    task_match_terms: &[String],
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
            if task_match_terms.iter().any(|token| lowered.contains(token)) {
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
