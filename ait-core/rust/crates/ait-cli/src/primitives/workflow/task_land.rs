use super::*;
use crate::primitives::plan_checklist_closeout::close_task_plan_checklist_item;
use crate::task_land_contract::attach_task_land_contract;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::primitives) enum TaskLandReferenceFamily {
    Task,
    Change,
}

pub(in crate::primitives) fn task_land_reference_family(
    value: &str,
) -> Option<TaskLandReferenceFamily> {
    let text = value.trim().to_ascii_uppercase();
    let (prefix, number) = text.rsplit_once('-')?;
    if number.is_empty() || !number.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    match prefix.chars().last()? {
        'T' => Some(TaskLandReferenceFamily::Task),
        'C' => Some(TaskLandReferenceFamily::Change),
        _ => None,
    }
}

pub(in crate::primitives) fn task_land_remote_change_id(
    repo: &RepoRuntime,
    requested_id: &str,
    remote_name: Option<&str>,
) -> Result<Option<String>, String> {
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    task_land_remote_change_id_with_task_remote(&mut task_remote, &repo_name, requested_id)
}

pub(in crate::primitives) fn task_land_remote_change_id_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    requested_id: &str,
) -> Result<Option<String>, String>
where
    R: TaskWorkflowRemoteChangeReader
        + TaskWorkflowRemoteChangeLister
        + TaskWorkflowRemoteTaskReader
        + ?Sized,
{
    let family = task_land_reference_family(requested_id);
    if family != Some(TaskLandReferenceFamily::Task) {
        if let Ok(change) =
            task_land_remote_change_read_with_task_remote(task_remote, repo_name, requested_id)
        {
            return Ok(Some(change_reference_from_payload(
                &change,
                Some(requested_id),
            )?));
        }
    }
    if family == Some(TaskLandReferenceFamily::Change) {
        return Ok(None);
    }

    let task =
        match task_land_remote_task_read_with_task_remote(task_remote, repo_name, requested_id) {
            Ok(task) => task,
            Err(_) => return Ok(None),
        };
    let task_id = required_string_field(&task, "task_id")?;
    let mut candidates = task_land_remote_change_rows_with_task_remote(task_remote, repo_name)?
        .into_iter()
        .filter(|row| string_field(row, "task_id").as_deref() == Some(task_id.as_str()))
        .filter(|row| {
            !matches!(
                string_field(row, "status").unwrap_or_default().as_str(),
                "archived" | "canceled" | "abandoned"
            )
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|row| {
        (
            match string_field(row, "status").unwrap_or_default().as_str() {
                "active" | "ready" | "review_pending" => 0,
                "draft" => 1,
                "landed" => 2,
                _ => 3,
            },
            string_field(row, "created_at").unwrap_or_default(),
            string_field(row, "change_id").unwrap_or_default(),
        )
    });
    if candidates.len() > 1 {
        let ids = candidates
            .iter()
            .filter_map(|row| {
                change_reference_from_payload(row, None)
                    .ok()
                    .or_else(|| string_field(row, "change_id"))
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Task {task_id} has multiple landable changes ({ids}); run `ait task land <change-id>` for the intended change."
        ));
    }
    candidates
        .first()
        .map(|row| change_reference_from_payload(row, None).map(Some))
        .unwrap_or(Ok(None))
}

pub(in crate::primitives) fn task_land_remote_change_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    change_id: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeReader + ?Sized,
{
    task_remote
        .get_change(change_id, Some(repo_name))
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn task_land_remote_task_read_with_task_remote<R>(
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

pub(in crate::primitives) fn task_land_remote_change_rows_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
) -> Result<Vec<JsonValue>, String>
where
    R: TaskWorkflowRemoteChangeLister + ?Sized,
{
    task_remote
        .list_changes(repo_name)
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn task_land_local_change_id(
    repo: &RepoRuntime,
    requested_id: &str,
) -> Result<Option<String>, String> {
    let change_store = repo.change_store()?;
    task_land_local_change_id_with_change_store(&change_store, requested_id)
}

pub(in crate::primitives) fn task_land_local_change_id_with_change_store<S>(
    change_store: &S,
    requested_id: &str,
) -> Result<Option<String>, String>
where
    S: TaskWorkflowChangeReader + TaskWorkflowChangeLister + ?Sized,
{
    let family = task_land_reference_family(requested_id);
    if family != Some(TaskLandReferenceFamily::Task) {
        if let Ok(change) = workflow_local_change_read_with_change_store(change_store, requested_id)
        {
            return change_reference_from_payload(&change, Some(requested_id)).map(Some);
        }
    }
    if family == Some(TaskLandReferenceFamily::Change) {
        return Ok(None);
    }
    let candidates = workflow_local_change_rows_with_change_store(change_store)?
        .into_iter()
        .filter(|row| string_field(row, "task_id").as_deref() == Some(requested_id))
        .filter(|row| {
            !matches!(
                string_field(row, "status").unwrap_or_default().as_str(),
                "archived" | "canceled" | "abandoned"
            )
        })
        .collect::<Vec<_>>();
    if candidates.len() > 1 {
        let ids = candidates
            .iter()
            .filter_map(|row| change_reference_from_payload(row, None).ok())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Local task {requested_id} has multiple landable changes ({ids}); run `ait task land <change-id>` for the intended change."
        ));
    }
    candidates
        .first()
        .map(|row| change_reference_from_payload(row, None).map(Some))
        .unwrap_or(Ok(None))
}

pub(super) fn resolve_task_land_change_id(
    repo: &RepoRuntime,
    task_or_change_id: &str,
    remote_name: Option<&str>,
) -> Result<String, String> {
    let requested_id = normalized_text(Some(task_or_change_id))
        .ok_or_else(|| "task-or-change-id is required".to_string())?;
    if let Some(change_id) = task_land_remote_change_id(repo, &requested_id, remote_name)? {
        return Ok(change_id);
    }
    if let Some(local_change_id) = task_land_local_change_id(repo, &requested_id)? {
        let change_store = repo.change_store()?;
        let local_change =
            workflow_local_change_read_with_change_store(&change_store, &local_change_id)?;
        if string_field(&local_change, "status").as_deref() == Some("landed")
            && string_field(&local_change, "publication_state").as_deref() != Some("published")
        {
            let remote_name = normalized_text(remote_name).unwrap_or_else(|| "origin".to_string());
            return Err(format!(
                "Local change {local_change_id} is completed but has no ready remote Patchset. Run `ait workflow ready {local_change_id} --apply --remote {remote_name}` to publish its consecutive local workflow history and CI-test the single aggregate Patchset, then hand it to a reviewer running `ait workflow land {local_change_id} --apply --remote {remote_name}`."
            ));
        }
    }
    Err(format!(
        "Could not resolve `{requested_id}` as a remote task or change for `ait task land`."
    ))
}

pub(in crate::primitives) fn task_land_local_payload(
    repo: &RepoRuntime,
    task_or_change_id: &str,
    remote_name: Option<&str>,
) -> Result<Option<JsonValue>, String> {
    if remote_name.is_some() {
        return Ok(None);
    }
    let requested_id = normalized_text(Some(task_or_change_id))
        .ok_or_else(|| "task-or-change-id is required".to_string())?;
    let Some(change_ref) = task_land_local_change_id(repo, &requested_id)? else {
        return Ok(None);
    };
    let change_store = repo.change_store()?;
    let task_store = repo.task_store()?;
    let change = workflow_local_change_read_with_change_store(&change_store, &change_ref)?;
    let change_id = required_string_field(&change, "change_id")?;
    if string_field(&change, "publication_state").as_deref() == Some("published") {
        return Ok(None);
    }
    let change_status = string_field(&change, "status").unwrap_or_default();
    if !matches!(change_status.as_str(), "draft" | "active" | "landed") {
        return Err(format!(
            "Local change {change_id} is {change_status} and cannot be locally landed"
        ));
    }
    let task_id = required_string_field(&change, "task_id")?;
    let task = workflow_local_task_read_with_task_store(&task_store, &task_id)?;
    if string_field(&task, "publication_state").as_deref() == Some("published") {
        return Ok(None);
    }
    let task_status = string_field(&task, "status").unwrap_or_default();
    if !matches!(task_status.as_str(), "active" | "completed") {
        return Err(format!(
            "Local task {task_id} is {task_status} and cannot be locally landed"
        ));
    }
    let target_line = string_field(&change, "target_line")
        .or_else(|| string_field(&change, "base_line"))
        .unwrap_or_else(|| repo.default_line_name());
    let workspace = workflow_workspace_status(repo, None, None)?;
    if change_status == "landed" {
        return Ok(Some(json!({
            "mode": "local",
            "status": "landed_closeout_recovery",
            "apply_status": "preview",
            "already_landed": true,
            "task_id": task_id,
            "change_id": change_id,
            "change_ref": change_ref,
            "target_line": target_line,
            "line_name": target_line,
            "landed_snapshot_id": string_field(&change, "landed_snapshot_id"),
            "task_status": task_status,
            "change_status": change_status,
            "task": task,
            "change": change,
            "workspace": workspace,
            "next_action": {
                "code": "resume_local_task_land_closeout",
                "summary": "Resume the already-landed local Task's Plan closeout and worktree cleanup without landing again.",
                "detail": "Run the same `ait task land <task-or-change-id>` command. The landed Line, Change, and Task state is reused idempotently.",
                "command": format!("ait task land {requested_id}"),
            },
        })));
    }
    Ok(Some(json!({
        "mode": "local",
        "status": "ready",
        "apply_status": "preview",
        "task_id": task_id,
        "change_id": change_id,
        "change_ref": change_ref,
        "target_line": target_line,
        "line_name": target_line,
        "task": task,
        "change": change,
        "workspace": workspace,
        "next_action": {
            "code": "workflow_land_local",
            "summary": "Land the local draft change onto its local target line.",
            "detail": "Run `ait task land <task-or-change-id> --local`, or omit the scope flag when workflow_mode defaults task land to local. Add `--remote <name>` after local land to promote completed local work.",
            "command": format!("ait task land {requested_id}"),
        },
    })))
}

pub(in crate::primitives) fn task_land_apply_local<F>(
    repo: &RepoRuntime,
    task_or_change_id: &str,
    snapshot_message: Option<&str>,
    target: Option<&str>,
    remote_name: Option<&str>,
    _progress: Option<F>,
) -> Result<Option<JsonValue>, String>
where
    F: FnMut(&JsonValue) -> Result<(), String>,
{
    let Some(local_payload) = task_land_local_payload(repo, task_or_change_id, remote_name)? else {
        return Ok(None);
    };
    let closeout_repo = workflow_root_repo(repo)?;
    let change_id = required_string_field(&local_payload, "change_id")?;
    let change_ref = required_string_field(&local_payload, "change_ref")?;
    let task_id = required_string_field(&local_payload, "task_id")?;
    let captured_bound_line = task_land_capture_bound_line(&closeout_repo, &task_id);
    let already_landed = local_payload
        .get("already_landed")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let mut output = if already_landed {
        let task_status = required_string_field(&local_payload, "task_status")?;
        let cleanup = workflow_bound_worktree_cleanup_after_local_land(
            &closeout_repo,
            &task_id,
            &task_status,
            "landed",
        )
        .unwrap_or_else(|error| {
            json!({
                "status": "failed",
                "reason": "recovery_cleanup_failed",
                "error": error,
                "task_id": task_id,
            })
        });
        json!({
            "change_id": change_id,
            "change_ref": change_ref,
            "task_id": task_id,
            "target_line": local_payload["target_line"],
            "line_name": local_payload["line_name"],
            "landed_snapshot_id": local_payload["landed_snapshot_id"],
            "change_status": "landed",
            "task_status": task_status,
            "current_line": closeout_repo.current_line_name()?,
            "workspace_action": "unchanged",
            "repo_root_restore": {
                "status": "skipped",
                "reason": "already_landed_closeout_recovery"
            },
            "bound_worktree_cleanup": cleanup,
            "execution_status": "already_landed",
        })
    } else {
        workflow_land_local(repo, &change_ref, target, None, snapshot_message)?
    };
    if let Some(output_obj) = output.as_object_mut() {
        output_obj.insert(
            "change_id".to_string(),
            JsonValue::String(change_id.clone()),
        );
        output_obj.insert(
            "change_ref".to_string(),
            JsonValue::String(change_ref.clone()),
        );
        output_obj.insert("mode".to_string(), JsonValue::String("local".to_string()));
        output_obj.insert(
            "apply_status".to_string(),
            JsonValue::String("done".to_string()),
        );
        output_obj.insert(
            "task".to_string(),
            local_payload
                .get("task")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
        output_obj.insert(
            "change".to_string(),
            local_payload
                .get("change")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
    }
    task_land_attach_bound_line_closeout(
        &closeout_repo,
        &mut output,
        true,
        None,
        captured_bound_line,
    );
    task_land_attach_plan_checklist_closeout(&closeout_repo, &mut output, true, None);
    Ok(Some(output))
}

pub(in crate::primitives) fn task_land_attach_plan_checklist_closeout(
    repo: &RepoRuntime,
    output: &mut JsonValue,
    use_local_scope: bool,
    remote_name: Option<&str>,
) {
    if output.get("apply_status").and_then(JsonValue::as_str) != Some("done") {
        attach_task_land_contract(output, use_local_scope);
        return;
    }
    if !use_local_scope {
        let result = if task_land_task_status(output).as_deref() != Some("completed") {
            json!({
                "status": "deferred",
                "reason": "task_still_active",
                "scope": "remote",
                "task_id": task_land_task_id(output),
                "task_status": task_land_task_status(output),
                "updated": false,
                "detail": "This Change landed, but peer Changes keep the Task active. Leave the bound Plan item open until the final Change completes the Task.",
            })
        } else {
            task_land_deferred_remote_plan_checklist_closeout(repo, output, remote_name)
        };
        if let Some(object) = output.as_object_mut() {
            object.insert("plan_checklist_closeout".to_string(), result);
        }
        attach_task_land_contract(output, false);
        return;
    }
    if output.get("task_status").and_then(JsonValue::as_str) != Some("completed") {
        let result = json!({
            "status": "deferred",
            "reason": "task_still_active",
            "scope": "local",
            "task_id": task_land_task_id(output),
            "open_peer_change_count": output
                .get("open_peer_change_count")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0),
            "updated": false,
        });
        if let Some(object) = output.as_object_mut() {
            object.insert("plan_checklist_closeout".to_string(), result);
        }
        attach_task_land_contract(output, true);
        return;
    }
    let result = (|| {
        let task = output
            .get("task")
            .filter(|task| task.is_object())
            .cloned()
            .ok_or_else(|| {
                "Local task land output did not preserve its pre-land task binding.".to_string()
            })?;
        run_locked_workspace_command(repo, "ait task land sprint checklist closeout", || {
            close_task_plan_checklist_item(repo, &task, None)
        })
    })()
    .unwrap_or_else(|error| {
        json!({
            "status": "failed",
            "error": error,
            "detail": "Code land and task completion succeeded, but automatic bound sprint checklist closeout did not converge. Run the scope-correct `ait plan sync <artifact>` after reconciling the reported error.",
        })
    });
    if let Some(object) = output.as_object_mut() {
        object.insert("plan_checklist_closeout".to_string(), result);
    }
    attach_task_land_contract(output, true);
}

fn task_land_deferred_remote_plan_checklist_closeout(
    repo: &RepoRuntime,
    output: &JsonValue,
    remote_name: Option<&str>,
) -> JsonValue {
    if output
        .get("history_promotion")
        .filter(|value| value.is_object())
        .is_some()
    {
        return json!({
            "status": "already_synced",
            "reason": "history_promotion_plan_lineage_prepared",
            "scope": "remote",
            "task_id": task_land_task_id(output),
            "updated": false,
            "detail": "Every unique Plan artifact bound to the promoted local history was synchronized before the atomic history prepare request.",
        });
    }
    let task = output.get("task").filter(|task| task.is_object());
    if task.is_some_and(|task| string_field(task, "plan_id").is_none()) {
        return json!({
            "status": "skipped",
            "reason": "no_plan_binding",
            "scope": "remote",
            "task_id": task_land_task_id(output),
        });
    }
    if task.is_some_and(|task| string_field(task, "plan_item_ref").is_none()) {
        return json!({
            "status": "skipped",
            "reason": "no_plan_item_ref",
            "scope": "remote",
            "task_id": task_land_task_id(output),
            "plan_id": task.and_then(|task| string_field(task, "plan_id")),
        });
    }

    let remote = normalized_text(remote_name)
        .or_else(|| repo.default_remote_name())
        .unwrap_or_else(|| "origin".to_string());
    let command = format!("ait plan sync <bound-sprint-card-path> --remote {remote}");
    json!({
        "status": "deferred",
        "reason": "remote_plan_sync_is_separate_from_task_land",
        "scope": "remote",
        "remote": remote,
        "task_id": task_land_task_id(output),
        "plan_id": task.and_then(|task| string_field(task, "plan_id")),
        "origin_plan_revision_id": task.and_then(|task| string_field(task, "origin_plan_revision_id")),
        "plan_item_ref": task.and_then(|task| string_field(task, "plan_item_ref")),
        "updated": false,
        "detail": "Remote task land completed without reading or synchronizing Plan state. Mark the exact bound checklist item complete in its Markdown sprint card, then run the separate Plan sync command.",
        "command": command,
    })
}

pub(in crate::primitives) fn task_land_existing_bound_worktree_cleanup(
    output: &JsonValue,
) -> Option<JsonValue> {
    output
        .get("bound_worktree_cleanup")
        .filter(|cleanup| cleanup.is_object())
        .cloned()
        .or_else(|| {
            output
                .get("applied_actions")
                .and_then(JsonValue::as_array)?
                .iter()
                .rev()
                .find_map(|action| {
                    action
                        .get("result")
                        .and_then(|result| result.get("bound_worktree_cleanup"))
                        .filter(|cleanup| cleanup.is_object())
                        .cloned()
                })
        })
}

pub(in crate::primitives) fn task_land_task_id(output: &JsonValue) -> Option<String> {
    output
        .get("task_id")
        .and_then(JsonValue::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            output
                .get("task")
                .and_then(|task| string_field(task, "task_id"))
        })
        .or_else(|| {
            output
                .get("applied_actions")
                .and_then(JsonValue::as_array)?
                .iter()
                .rev()
                .find_map(|action| {
                    action
                        .get("result")
                        .and_then(|result| string_field(result, "task_id"))
                })
        })
}

pub(in crate::primitives) fn task_land_task_status(output: &JsonValue) -> Option<String> {
    output
        .get("task_status")
        .and_then(JsonValue::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            output
                .get("task")
                .and_then(|task| string_field(task, "status"))
        })
        .or_else(|| {
            output
                .get("applied_actions")
                .and_then(JsonValue::as_array)?
                .iter()
                .rev()
                .find_map(|action| {
                    action
                        .get("result")
                        .and_then(|result| string_field(result, "status"))
                })
        })
}

fn task_land_local_sync_target(output: &JsonValue) -> Option<(String, String)> {
    output
        .get("applied_actions")
        .and_then(JsonValue::as_array)?
        .iter()
        .rev()
        .find_map(|action| {
            let local_sync = action.get("result")?.get("local_sync")?;
            let line = string_field(local_sync, "line")?;
            let snapshot_id = string_field(local_sync, "landed_snapshot_id")?;
            Some((line, snapshot_id))
        })
}

pub(in crate::primitives) fn task_land_attach_cli_main_seed_sync(
    repo: &RepoRuntime,
    output: &mut JsonValue,
    fallback_target_line: &str,
    fallback_snapshot_id: Option<&str>,
) {
    if output.get("apply_status").and_then(JsonValue::as_str) != Some("done") {
        return;
    }
    if output
        .get("local_line_sync")
        .and_then(|value| value.get("same_head"))
        .and_then(JsonValue::as_bool)
        == Some(true)
    {
        output["main_seed_sync"] = json!({
            "status": "skipped",
            "reason": "already_at_trusted_local_landed_snapshot",
            "line_name": fallback_target_line,
            "snapshot_id": fallback_snapshot_id,
            "detail": "The remote history promotion landed the Snapshot already held by the local target Line; the prior local Task Land already refreshed the CLI main seed.",
        });
        return;
    }
    let (target_line, target_snapshot_id) = task_land_local_sync_target(output)
        .or_else(|| {
            let snapshot_id = string_field(output, "landed_snapshot_id")
                .or_else(|| normalized_text(fallback_snapshot_id))?;
            let line = string_field(output, "target_line")
                .unwrap_or_else(|| fallback_target_line.to_string());
            Some((line, snapshot_id))
        })
        .unwrap_or_else(|| (fallback_target_line.to_string(), String::new()));
    let task_id = task_land_task_id(output);
    let task_status = task_land_task_status(output);
    let seed_sync = if target_snapshot_id.is_empty() {
        json!({
            "status": "failed",
            "reason": "landed_snapshot_missing",
            "line_name": target_line,
            "detail": "Remote land completed, but CLI main-seed synchronization could not resolve the landed Snapshot.",
        })
    } else {
        sync_main_seed_after_task_land(
            repo,
            task_id.as_deref(),
            task_status.as_deref(),
            &target_line,
            &target_snapshot_id,
        )
    };
    let promoted_cleanup = seed_sync
        .get("worktree_cleanup")
        .filter(|cleanup| {
            cleanup
                .get("removed")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
        })
        .cloned();
    if let Some(output_obj) = output.as_object_mut() {
        output_obj.insert("main_seed_sync".to_string(), seed_sync.clone());
        if let Some(cleanup) = promoted_cleanup {
            output_obj.insert(
                "bound_worktree_cleanup".to_string(),
                json!({
                    "status": "removed",
                    "reason": "promoted_to_cli_main_seed",
                    "task_id": task_id,
                    "removed": true,
                    "worktree": cleanup,
                }),
            );
        }
        if let Some(actions) = output_obj
            .get_mut("applied_actions")
            .and_then(JsonValue::as_array_mut)
        {
            for action in actions.iter_mut().rev() {
                let Some(result) = action.get_mut("result") else {
                    continue;
                };
                let Some(local_sync) = result
                    .get_mut("local_sync")
                    .and_then(JsonValue::as_object_mut)
                else {
                    continue;
                };
                local_sync.insert("main_seed_sync".to_string(), seed_sync.clone());
                if let Some(workspace_restore) = local_sync
                    .get_mut("workspace_restore")
                    .and_then(JsonValue::as_object_mut)
                {
                    workspace_restore.insert("main_seed_sync".to_string(), seed_sync.clone());
                }
                break;
            }
        }
    }
}

pub(in crate::primitives) fn task_land_force_bound_worktree_cleanup(
    repo: &RepoRuntime,
    output: &JsonValue,
) -> Result<JsonValue, String> {
    let Some(task_id) = task_land_task_id(output) else {
        return Ok(json!({
            "status": "skipped",
            "reason": "no_task_id",
        }));
    };
    let task_status = task_land_task_status(output).unwrap_or_else(|| "completed".to_string());
    if task_status != "completed" {
        return Ok(json!({
            "status": "skipped",
            "reason": "task_not_completed",
            "task_id": task_id,
            "task_status": task_status,
        }));
    }

    let root_repo = workflow_root_repo(repo)?;
    let existing_cleanup = task_land_existing_bound_worktree_cleanup(output);
    let Some(bound_worktree) = workflow_find_bound_task_worktree_metadata(&root_repo, &task_id)?
    else {
        if let Some(existing_cleanup) = existing_cleanup {
            if existing_cleanup
                .get("removed")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
            {
                return Ok(json!({
                    "status": "removed",
                    "reason": "already_removed_by_workflow_land",
                    "task_id": task_id,
                    "worktree": existing_cleanup,
                }));
            }
        }
        return Ok(json!({
            "status": "skipped",
            "reason": "no_bound_worktree",
            "task_id": task_id,
        }));
    };
    let worktree_name = required_string_field(&bound_worktree, "name")?;
    let removed = remove_one_worktree_after_authoritative_task_land(&root_repo, &worktree_name)?;
    Ok(json!({
        "status": "removed",
        "reason": "task_land_force_close",
        "task_id": task_id,
        "worktree": removed,
        "force": true,
    }))
}

pub(in crate::primitives) fn task_land_attach_forced_cleanup(
    repo: &RepoRuntime,
    output: &mut JsonValue,
) -> Result<(), String> {
    if output.get("apply_status").and_then(JsonValue::as_str) != Some("done") {
        return Ok(());
    }
    let cleanup = task_land_force_bound_worktree_cleanup(repo, output).map_err(|err| {
        format!("Task land completed, but forced bound worktree cleanup failed: {err}")
    })?;
    if let Some(output_obj) = output.as_object_mut() {
        output_obj.insert("bound_worktree_cleanup".to_string(), cleanup.clone());
        if let Some(actions) = output_obj
            .get_mut("applied_actions")
            .and_then(JsonValue::as_array_mut)
        {
            for action in actions.iter_mut().rev() {
                if action.get("code").and_then(JsonValue::as_str) != Some("complete_task") {
                    continue;
                }
                if let Some(result_obj) =
                    action.get_mut("result").and_then(JsonValue::as_object_mut)
                {
                    result_obj.insert("bound_worktree_cleanup".to_string(), cleanup);
                }
                break;
            }
        }
    }
    Ok(())
}

pub fn task_land_payload(
    repo: &RepoRuntime,
    task_or_change_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let change_id = resolve_task_land_change_id(repo, task_or_change_id, remote_name)?;
    workflow_land_payload(repo, &change_id, remote_name)
}

pub fn task_land_payload_scoped(
    repo: &RepoRuntime,
    task_or_change_id: &str,
    use_local_scope: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let mut output = if use_local_scope {
        task_land_local_payload(repo, task_or_change_id, None)?.ok_or_else(|| {
            format!(
                "`ait task land {task_or_change_id}` is using local scope, but no unpublished local draft is ready to land. Pass `--remote <name>` for shared remote closeout."
            )
        })?
    } else {
        task_land_payload(repo, task_or_change_id, remote_name)?
    };
    attach_task_land_contract(&mut output, use_local_scope);
    Ok(output)
}

pub(in crate::primitives) fn task_land_exact_atomic_reference(
    repo: &RepoRuntime,
    requested: &str,
) -> Result<String, String> {
    let requested = normalized_text(Some(requested))
        .ok_or_else(|| "task-or-change-id is required.".to_string())?;
    match task_land_reference_family(&requested) {
        Some(TaskLandReferenceFamily::Task) => {
            if let Ok(task_store) = repo.task_store() {
                if let Ok(task) = workflow_local_task_read_with_task_store(&task_store, &requested)
                {
                    if string_field(&task, "publication_state").as_deref() == Some("published") {
                        if let Some(remote_task_id) = string_field(&task, "published_task_id") {
                            return Ok(remote_task_id);
                        }
                    }
                }
            }
        }
        Some(TaskLandReferenceFamily::Change) => {
            if let Ok(change_store) = repo.change_store() {
                if let Ok(change) =
                    workflow_local_change_read_with_change_store(&change_store, &requested)
                {
                    if string_field(&change, "publication_state").as_deref() == Some("published") {
                        if let Some(remote_change_ref) =
                            string_field(&change, "published_change_id")
                        {
                            return Ok(remote_change_ref);
                        }
                    }
                }
            }
        }
        None => {}
    }
    if task_land_reference_family(&requested) != Some(TaskLandReferenceFamily::Change)
        || requested.contains('/')
    {
        return Ok(requested);
    }
    let metadata = if repo.is_worktree() {
        current_worktree_metadata(repo)?
    } else {
        bound_task_worktree_metadata(repo, None, Some(&requested))?
    };
    let Some(metadata) = metadata else {
        return Err(format!(
            "Atomic remote Task Land requires an exact Change reference; `{requested}` has no local Task binding. Use `<task-id>/{requested}` or pass the Task ID."
        ));
    };
    if metadata.bound_change_id.as_deref() != Some(requested.as_str()) {
        return Err(format!(
            "Bound worktree `{}` does not own Change `{requested}`.",
            metadata.name
        ));
    }
    metadata.bound_change_ref.ok_or_else(|| {
        format!(
            "Bound worktree `{}` cannot derive an exact Change reference for `{requested}`.",
            metadata.name
        )
    })
}

fn task_land_reference_task_hint(repo: &RepoRuntime, reference: &str) -> Option<String> {
    reference
        .split_once('/')
        .map(|(task_id, _)| task_id.to_string())
        .or_else(|| {
            (task_land_reference_family(reference) == Some(TaskLandReferenceFamily::Task))
                .then(|| reference.to_string())
        })
        .or_else(|| {
            current_worktree_metadata(repo)
                .ok()
                .flatten()
                .and_then(|metadata| metadata.bound_task_id)
        })
}

fn task_land_atomic_idempotency_key(
    repo: &RepoRuntime,
    task_or_change_ref: &str,
    target: Option<&str>,
    mode: &str,
) -> Result<String, String> {
    let repository_index = repo.require_repository_index()?.to_string();
    let mut bytes = Vec::new();
    for part in [
        "task-land-atomic/v1",
        repository_index.as_str(),
        task_or_change_ref,
        normalized_text(target)
            .as_deref()
            .unwrap_or("<server-base-line>"),
        mode,
    ] {
        bytes.extend_from_slice(part.as_bytes());
        bytes.push(0);
    }
    Ok(format!("task-land-atomic:{}", sha256_hex_bytes(&bytes)))
}

pub(in crate::primitives) fn task_land_atomic_action_result(
    atomic_response: &JsonValue,
    local_sync_result: JsonValue,
) -> Result<JsonValue, String> {
    let mut result = atomic_response
        .get("land")
        .and_then(JsonValue::as_object)
        .cloned()
        .ok_or_else(|| "Atomic Task Land response is missing Land projection.".to_string())?;
    for key in [
        "task_id",
        "change_id",
        "change_ref",
        "patchset_id",
        "target_line",
        "landed_snapshot_id",
    ] {
        if let Some(value) = atomic_response.get(key) {
            result.insert(key.to_string(), value.clone());
        }
    }
    result.insert(
        "atomic_task_land".to_string(),
        json!({
            "contract": atomic_response["contract"].clone(),
            "idempotency_key": atomic_response["idempotency_key"].clone(),
            "replayed": atomic_response["replayed"].clone(),
            "remote_mutation_count": 1,
        }),
    );
    if let Some(local_sync) = local_sync_result.get("local_sync") {
        result.insert("local_sync".to_string(), local_sync.clone());
    }
    if let Some(cleanup) = local_sync_result.get("bound_worktree_cleanup") {
        result.insert("bound_worktree_cleanup".to_string(), cleanup.clone());
    }
    if let Some(nested_result) = local_sync_result.get("result") {
        result.insert("result".to_string(), nested_result.clone());
    }
    Ok(JsonValue::Object(result))
}

pub(in crate::primitives) fn task_land_atomic_output(
    atomic_response: &JsonValue,
    land_action_result: JsonValue,
) -> Result<JsonValue, String> {
    let task = atomic_response
        .get("task")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| "Atomic Task Land response is missing Task projection.".to_string())?;
    let change = atomic_response
        .get("change")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| "Atomic Task Land response is missing Change projection.".to_string())?;
    let patchset = atomic_response
        .get("patchset")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| "Atomic Task Land response is missing Patchset projection.".to_string())?;
    let land = atomic_response
        .get("land")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| "Atomic Task Land response is missing Land projection.".to_string())?;
    let task_id = required_string_field(atomic_response, "task_id")?;
    let change_id = required_string_field(atomic_response, "change_id")?;
    let change_ref = required_string_field(atomic_response, "change_ref")?;
    let patchset_id = required_string_field(atomic_response, "patchset_id")?;
    let target_line = required_string_field(atomic_response, "target_line")?;
    let landed_snapshot_id = required_string_field(atomic_response, "landed_snapshot_id")?;
    let submission_id = land
        .get("submission_id")
        .and_then(JsonValue::as_str)
        .map(ToString::to_string);
    let complete_task_result = task.clone();
    Ok(json!({
        "contract": atomic_response["contract"].clone(),
        "repo_name": atomic_response["repo_name"].clone(),
        "repository_index": atomic_response["repository_index"].clone(),
        "task_id": task_id,
        "task_status": "completed",
        "change_id": change_id,
        "change_ref": change_ref,
        "change_status": "landed",
        "patchset_id": patchset_id,
        "target_line": target_line,
        "landed_snapshot_id": landed_snapshot_id,
        "task": task,
        "change": change,
        "patchset": patchset,
        "land": land,
        "history_promotion": atomic_response.get("history_promotion").cloned().unwrap_or(JsonValue::Null),
        "landing_summary": land_action_result.clone(),
        "patchset_is_authoritative": true,
        "workspace": {
            "clean": JsonValue::Null,
            "changed_count": JsonValue::Null,
            "changed_paths": [],
            "evaluation": "skipped",
            "reason": "ready_patchset_is_authoritative",
            "read_scope": "line_and_bound_worktree_metadata_only",
        },
        "atomic_task_land": {
            "contract": "task-land-atomic/v1",
            "idempotency_key": atomic_response["idempotency_key"].clone(),
            "replayed": atomic_response["replayed"].clone(),
            "remote_mutation_count": 1,
        },
        "applied_actions": [
            {
                "code": "submit_land",
                "result": land_action_result,
                "delivery": "atomic_task_land"
            },
            {
                "code": "complete_task",
                "result": complete_task_result,
                "delivery": "atomic_task_land"
            }
        ],
        "mutation_receipts": [
            {
                "action": "submit_land",
                "source_action": "atomic_task_land",
                "delivery": "atomic_response",
                "change_id": atomic_response["change_id"].clone(),
                "patchset_id": atomic_response["patchset_id"].clone(),
                "submission_id": submission_id,
                "status": atomic_response["status"].clone()
            },
            {
                "action": "complete_task",
                "source_action": "atomic_task_land",
                "delivery": "atomic_response",
                "task_id": atomic_response["task_id"].clone(),
                "status": atomic_response["task_status"].clone()
            }
        ],
        "next_action": {
            "code": "done",
            "summary": "Atomic remote Task Land completed."
        },
        "apply_status": "done",
        "apply_phase": workflow_apply_phase_payload_json(
            if atomic_response.get("replayed").and_then(JsonValue::as_bool) == Some(true) {
                "authoritative_resume"
            } else {
                "done"
            },
            "done",
            Some("Land, Change, target Line, and Task completion were committed by one atomic server mutation."),
            atomic_response.get("replayed").and_then(JsonValue::as_bool) == Some(true),
        ),
    }))
}

fn task_land_main_seed_failed(output: &JsonValue) -> bool {
    output
        .get("main_seed_sync")
        .and_then(|value| string_field(value, "status"))
        .as_deref()
        == Some("failed")
}

fn task_land_local_sync_failed(output: &JsonValue) -> bool {
    output
        .get("local_line_sync")
        .and_then(|value| string_field(value, "status"))
        .as_deref()
        == Some("failed")
}

pub(in crate::primitives) fn task_land_defer_bound_cleanup(
    output: &mut JsonValue,
    reason: &str,
    detail: &str,
    error: Option<&str>,
) {
    let cleanup = json!({
        "status": "deferred",
        "reason": reason,
        "task_id": task_land_task_id(output),
        "detail": detail,
        "error": error,
        "removed": false,
    });
    if let Some(object) = output.as_object_mut() {
        object.insert("bound_worktree_cleanup".to_string(), cleanup.clone());
        if let Some(actions) = object
            .get_mut("applied_actions")
            .and_then(JsonValue::as_array_mut)
        {
            if let Some(result) = actions
                .iter_mut()
                .find(|action| {
                    action.get("code").and_then(JsonValue::as_str) == Some("submit_land")
                })
                .and_then(|action| action.get_mut("result"))
                .and_then(JsonValue::as_object_mut)
            {
                result.insert("bound_worktree_cleanup".to_string(), cleanup);
            }
        }
    }
}

pub fn task_land_apply<F>(
    repo: &RepoRuntime,
    task_or_change_id: &str,
    remote_name: Option<&str>,
    mut progress: Option<F>,
) -> Result<JsonValue, String>
where
    F: FnMut(&JsonValue) -> Result<(), String>,
{
    let _task_land_range = perfetto_range!("ait.task_land.apply");
    let total_started = Instant::now();
    let preflight_started = Instant::now();
    let task_or_change_ref = task_land_exact_atomic_reference(repo, task_or_change_id)?;
    let task_hint = task_land_reference_task_hint(repo, &task_or_change_ref);
    guard_repo_root_pinned_bound_worktree(repo, task_hint.as_deref(), "ait task land")?;
    guard_repo_root_bound_task_worktree(
        repo,
        task_hint.as_deref(),
        (task_land_reference_family(&task_or_change_ref) == Some(TaskLandReferenceFamily::Change))
            .then_some(task_or_change_ref.as_str()),
        "ait task land",
    )?;
    guard_no_planning_only_artifact_drift(repo, "ait task land")?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    let idempotency_key =
        task_land_atomic_idempotency_key(repo, &task_or_change_ref, Some("main"), "direct")?;
    let preflight_elapsed = elapsed_ms(preflight_started);

    workflow_progress_emit(
        &mut progress,
        "starting",
        "atomic_task_land",
        (task_land_reference_family(&task_or_change_ref) == Some(TaskLandReferenceFamily::Change))
            .then_some(task_or_change_ref.as_str()),
        None,
        Some(1),
        Some("Submitting one atomic already-ready Task Land mutation."),
        Some("mutation_started"),
        None,
        None,
        None,
    )?;
    let atomic_remote_started = Instant::now();
    let atomic_response = {
        let _range = perfetto_range!("ait.task_land.atomic_remote_closeout");
        closeout_remote
            .submit_task_land(
                &task_or_change_ref,
                Some("main"),
                "direct",
                &idempotency_key,
                Some(&repo_name),
            )
            .map_err(|error| error.to_string())?
    };
    let atomic_remote_elapsed = elapsed_ms(atomic_remote_started);
    let response_task_id = required_string_field(&atomic_response, "task_id")?;
    let response_change_id = required_string_field(&atomic_response, "change_id")?;
    let response_patchset_id = required_string_field(&atomic_response, "patchset_id")?;
    let response_target_line = required_string_field(&atomic_response, "target_line")?;
    let response_snapshot_id = required_string_field(&atomic_response, "landed_snapshot_id")?;
    workflow_progress_emit(
        &mut progress,
        "completed",
        "atomic_task_land",
        Some(&response_change_id),
        Some(&response_patchset_id),
        Some(1),
        None,
        Some("mutation_accepted"),
        None,
        None,
        Some("Land and Task completion committed atomically."),
    )?;
    let captured_bound_line = task_land_capture_bound_line(repo, &response_task_id);

    let local_sync_started = Instant::now();
    let local_sync_result = workflow_attach_local_land_sync_from_atomic_response(
        repo,
        &response_task_id,
        atomic_response
            .get("land")
            .ok_or_else(|| "Atomic Task Land response is missing Land projection.".to_string())?,
        &response_target_line,
        &response_snapshot_id,
    )
    .unwrap_or_else(|error| {
        json!({
            "status": "succeeded",
            "target_line": response_target_line,
            "landed_snapshot_id": response_snapshot_id,
            "result": {
                "target_line": response_target_line,
                "landed_snapshot_id": response_snapshot_id,
            },
            "local_sync": {
                "status": "failed",
                "reason": "atomic_response_local_line_sync_failed",
                "error": error,
                "line": response_target_line,
                "landed_snapshot_id": response_snapshot_id,
            },
            "bound_worktree_cleanup": {
                "status": "deferred",
                "reason": "local_line_sync_failed",
                "task_id": response_task_id,
                "removed": false,
            }
        })
    });
    let local_sync_elapsed = elapsed_ms(local_sync_started);
    let land_action_result =
        task_land_atomic_action_result(&atomic_response, local_sync_result.clone())?;
    let mut output = task_land_atomic_output(&atomic_response, land_action_result)?;
    if let Some(local_sync) = local_sync_result.get("local_sync") {
        output["local_line_sync"] = local_sync.clone();
    }
    if let Some(cleanup) = local_sync_result.get("bound_worktree_cleanup") {
        output["bound_worktree_cleanup"] = cleanup.clone();
    }

    let main_seed_started = Instant::now();
    if !task_land_local_sync_failed(&output) {
        let _range = perfetto_range!("ait.task_land.cli_main_seed_sync");
        task_land_attach_cli_main_seed_sync(
            repo,
            &mut output,
            &response_target_line,
            Some(&response_snapshot_id),
        );
    }
    let main_seed_elapsed = elapsed_ms(main_seed_started);

    let cleanup_started = Instant::now();
    let local_closeout_failed =
        task_land_local_sync_failed(&output) || task_land_main_seed_failed(&output);
    if local_closeout_failed {
        let (reason, detail, error) = if task_land_main_seed_failed(&output) {
            (
                "main_seed_sync_failed",
                "Remote land is authoritative, but the CLI main seed was not updated. Repair the reported local path or permission issue, then rerun the same Task Land command; the server will replay without a second Land.",
                output
                    .get("main_seed_sync")
                    .and_then(|value| string_field(value, "error")),
            )
        } else {
            (
                "local_line_sync_failed",
                "Remote land is authoritative, but local target-Line synchronization failed. Repair the reported local state, then rerun the same Task Land command.",
                output
                    .get("local_line_sync")
                    .and_then(|value| string_field(value, "error")),
            )
        };
        task_land_defer_bound_cleanup(&mut output, reason, detail, error.as_deref());
        output["bound_line_closeout"] = json!({
            "status": "deferred",
            "reason": reason,
            "task_id": response_task_id,
            "detail": detail,
        });
    } else {
        let _range = perfetto_range!("ait.task_land.force_worktree_cleanup");
        if let Err(error) = task_land_attach_forced_cleanup(repo, &mut output) {
            output["bound_worktree_cleanup"] = json!({
                "status": "failed",
                "reason": "post_land_cleanup_failed",
                "error": error,
            });
        }
        let _range = perfetto_range!("ait.task_land.bound_line_closeout");
        let closeout_repo = workflow_root_repo(repo)?;
        task_land_attach_bound_line_closeout(
            &closeout_repo,
            &mut output,
            false,
            remote_name,
            captured_bound_line,
        );
    }
    {
        let _range = perfetto_range!("ait.task_land.plan_closeout_projection");
        task_land_attach_plan_checklist_closeout(repo, &mut output, false, remote_name);
    }
    let cleanup_elapsed = elapsed_ms(cleanup_started);
    output["phase_timings_ms"] = json!({
        "preflight": preflight_elapsed,
        "atomic_remote_closeout": atomic_remote_elapsed,
        "local_line_sync": local_sync_elapsed,
        "main_seed_sync": main_seed_elapsed,
        "remaining_cleanup": cleanup_elapsed,
        "total": elapsed_ms(total_started),
    });
    attach_task_land_contract(&mut output, false);
    Ok(output)
}

pub fn task_land_apply_scoped<F>(
    repo: &RepoRuntime,
    task_or_change_id: &str,
    use_local_scope: bool,
    remote_name: Option<&str>,
    progress: Option<F>,
) -> Result<JsonValue, String>
where
    F: FnMut(&JsonValue) -> Result<(), String>,
{
    if use_local_scope {
        return task_land_apply_local(
            repo,
            task_or_change_id,
            None,
            Some("main"),
            None,
            None::<fn(&JsonValue) -> Result<(), String>>,
        )?
        .ok_or_else(|| {
            format!(
                "`ait task land {task_or_change_id}` is using local scope, but no unpublished local draft is ready to land. Pass `--remote <name>` for shared remote closeout."
            )
        });
    }
    task_land_apply(repo, task_or_change_id, remote_name, progress)
}
