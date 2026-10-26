//! Local Task finish selects the frozen Task head; a Change never selects content.
use super::*;

pub(super) fn payload(
    repo: &RepoRuntime,
    reference: &str,
    remote: Option<&str>,
) -> Result<Option<JsonValue>, String> {
    if remote.is_some() {
        return Ok(None);
    }
    let requested = normalized_text(Some(reference)).ok_or("Task ID is required")?;
    let store = repo.task_store()?;
    let task_id = if is_task_id(&requested) {
        requested.clone()
    } else {
        let Some(change) = task_land_local_change_id(repo, &requested)? else {
            return Ok(None);
        };
        required_string_field(
            &workflow_local_change_read_with_change_store(&repo.change_store()?, &change)?,
            "task_id",
        )?
    };
    let task = workflow_local_task_read_with_task_store(&store, &task_id)?;
    if string_field(&task, "publication_state").as_deref() == Some("published") {
        return Ok(None);
    }
    let receipt = store
        .local_task_receipt(&task_id)
        .map_err(|e| e.to_string())?;
    let status = required_string_field(&task, "status")?;
    if receipt.is_none() && status != "active" {
        return Err(format!(
            "Task {task_id} is {status} and cannot receive another finish"
        ));
    }
    let target = receipt
        .as_ref()
        .and_then(|r| string_field(r, "target_line"))
        .unwrap_or_else(|| "main".into());
    Ok(Some(
        json!({"mode":"local","status":if receipt.is_some(){"finished_closeout_recovery"}else{"ready"},
        "task_id":task_id,"task_status":status,"task":task,"target_line":target,"line_name":target,
        "already_landed":receipt.is_some(),"receipt":receipt,
        "next_action":{"code":"finish_task","command":format!("ait task finish {task_id} --local"),"summary":"Finish the Task's complete frozen Snapshot once, then resume only unfinished closeout phases."}}),
    ))
}

pub(super) fn apply(
    repo: &RepoRuntime,
    reference: &str,
    message: Option<&str>,
    target: Option<&str>,
    remote: Option<&str>,
) -> Result<Option<JsonValue>, String> {
    let Some(initial) = payload(repo, reference, remote)? else {
        return Ok(None);
    };
    let task_id = required_string_field(&initial, "task_id")?;
    let root = workflow_root_repo(repo)?;
    let store = repo.task_store()?;
    let old = store
        .local_task_receipt(&task_id)
        .map_err(|e| e.to_string())?;
    let captured = task_land_capture_bound_line(&root, &task_id);
    let mut automatic = None;
    let (candidate, target, pre, association) = if let Some(receipt) = old.as_ref() {
        if let Some(requested) = normalized_text(target) {
            if string_field(receipt, "target_line").as_deref() != Some(requested.as_str()) {
                return Err("Task already finished onto a different target".into());
            }
        }
        if repo.is_worktree()
            && !workflow_workspace_status(repo, None, None)?["clean"]
                .as_bool()
                .unwrap_or(false)
        {
            return Err("Task apply already succeeded, but its worktree has new changes; preserve them before closeout recovery".into());
        }
        if repo.is_worktree() {
            let head = string_field(
                &local_line_row(repo, &repo.current_line_name()?)?,
                "head_snapshot_id",
            );
            if head != string_field(receipt, "landed_snapshot_id") {
                return Err("Task apply already succeeded, but its worktree Line has a different Snapshot; preserve it before closeout recovery".into());
            }
        }
        (
            required_string_field(receipt, "landed_snapshot_id")?,
            required_string_field(receipt, "target_line")?,
            string_field(receipt, "pre_land_target_snapshot_id"),
            string_field(receipt, "change_ref"),
        )
    } else {
        guard_repo_root_pinned_bound_worktree(repo, Some(&task_id), "ait task finish")?;
        guard_repo_root_bound_task_worktree(repo, Some(&task_id), None, "ait task finish")?;
        let metadata = current_worktree_metadata(repo)?
            .ok_or("Task finish requires its registered worktree")?;
        if metadata.bound_task_id.as_deref() != Some(task_id.as_str()) {
            return Err("Task finish is outside its exact bound worktree".into());
        }
        let clean = workflow_workspace_status(repo, None, None)?["clean"]
            .as_bool()
            .unwrap_or(false);
        if !clean {
            let message=normalized_text(message).ok_or_else(||format!("Workspace is dirty; pass --message to `ait task finish {task_id} --local` or capture a Snapshot first"))?;
            automatic = Some(snapshot_create_for_reference(
                repo,
                reference,
                Some(&message),
            )?);
        }
        let line = repo.current_line_name()?;
        let candidate = string_field(&local_line_row(repo, &line)?, "head_snapshot_id")
            .ok_or("Task Line has no final Snapshot")?;
        let target = normalized_text(target).unwrap_or_else(|| "main".into());
        if metadata
            .target_base_line
            .as_deref()
            .is_some_and(|line| line != target)
        {
            return Err("Finish target differs from the bound worktree target; rebase onto that target before finish".into());
        }
        let pre = string_field(&local_line_row(repo, &target)?, "head_snapshot_id");
        let ownership = store
            .snapshot_ownership_rows(std::slice::from_ref(&candidate))
            .map_err(|e| e.to_string())?;
        let association = ownership
            .iter()
            .find(|row| string_field(row, "task_id").as_deref() == Some(task_id.as_str()))
            .and_then(|row| string_field(row, "change_id"))
            .filter(|id| !id.is_empty())
            .map(|id| format!("{task_id}/{id}"));
        (candidate, target, pre, association)
    };
    let pending = repo
        .is_worktree()
        .then(|| repo.workspace_root().join(".ait-worktree/cursor.bin"));
    let receipt = match store.finish_local_task(
        &task_id,
        &candidate,
        &target,
        pre.as_deref(),
        association.as_deref(),
        pending.as_deref(),
    ) {
        Ok(receipt) => receipt,
        Err(error) => {
            if let Some(receipt) = store
                .local_task_receipt(&task_id)
                .map_err(|e| e.to_string())?
            {
                if string_field(&receipt, "landed_snapshot_id").as_deref()
                    != Some(candidate.as_str())
                    || string_field(&receipt, "target_line").as_deref() != Some(target.as_str())
                {
                    return Err(error.to_string());
                }
                return Ok(Some(
                    json!({"mode":"local","apply_status":"partial","execution_status":"applied",
                    "task_id":task_id,"task_status":workflow_local_task_read_with_task_store(&store,&task_id)?["status"],
                    "local_finish":receipt,"task_closeout":{"status":"failed","error":error.to_string()},
                    "next_action":{"code":"resume_closeout","command":format!("ait task finish {task_id} --local"),
                    "summary":"Apply succeeded. Retry only unfinished Task closeout; preserve the successful receipt."}}),
                ));
            }
            let detail = error.to_string();
            if detail.contains("target changed") || detail.contains("expected target history") {
                let line = repo.current_line_name()?;
                let target_head = string_field(&local_line_row(repo, &target)?, "head_snapshot_id")
                    .unwrap_or_else(|| "<empty>".into());
                return Err(format!("{detail}\nCandidate selection: {}\nCurrent Line `{line}` head: {candidate}\nTarget Line `{target}` head: {target_head}\nRun `ait worktree rebase --onto {target}` before retrying the frozen Task candidate.",if automatic.is_some(){"new workspace Snapshot"}else{"complete Task Line Snapshot"}));
            }
            return Err(detail);
        }
    };
    let task = workflow_local_task_read_with_task_store(&store, &task_id)?;
    let change = association
        .as_ref()
        .map(|reference| {
            workflow_local_change_read_with_change_store(&repo.change_store()?, reference)
        })
        .transpose()?;
    let target_head = string_field(&local_line_row(repo, &target)?, "head_snapshot_id");
    let restore = if target_head.as_deref() != Some(candidate.as_str()) {
        json!({"status":"skipped","reason":"target_advanced_after_successful_apply","target_snapshot_id":target_head})
    } else {
        workflow_repo_root_restore_after_land(repo, &target, pre.as_deref(), Some(&candidate))
            .unwrap_or_else(|error| json!({"status":"failed","error":error}))
    };
    let cleanup = if string_field(&restore, "status").as_deref() == Some("failed") {
        json!({"status":"skipped","reason":"repo_root_restore_failed"})
    } else {
        workflow_bound_worktree_cleanup_after_local_land(
            &root,
            &task_id,
            "completed",
            if association.is_some() {
                "landed"
            } else {
                "task_receipt"
            },
        )
        .unwrap_or_else(|error| json!({"status":"failed","error":error,"task_id":task_id}))
    };
    let mut output = json!({"mode":"local","apply_status":"done","execution_status":if old.is_some(){"already_landed"}else{"applied"},
        "task_id":task_id,"task_status":"completed","task_status_before":initial["task_status"],"task":task,
        "change_id":association.as_deref().and_then(|r|r.rsplit_once('/').map(|(_,id)|id)),"change_ref":association,
        "change_status":change.as_ref().and_then(|row|string_field(row,"status")),"change":change,
        "target_line":target,"line_name":target,"landed_snapshot_id":candidate,"previous_target_head_snapshot_id":pre,
        "local_finish":receipt,"auto_snapshot":automatic,"current_line":root.current_line_name()?,
        "workspace_action":restore["status"],"repo_root_restore":restore,"bound_worktree_cleanup":cleanup,"open_peer_change_count":0});
    task_land_attach_bound_line_closeout(&root, &mut output, true, None, captured);
    if output["bound_worktree_cleanup"]["status"].as_str() != Some("failed")
        && output["repo_root_restore"]["status"].as_str() != Some("failed")
    {
        task_land_attach_cli_main_seed_sync(&root, &mut output, &target, Some(&candidate));
    } else {
        output["main_seed_sync"] = json!({"status":"pending","reason":"task_closeout_recovery_pending","line_name":target});
    }
    task_land_attach_plan_markdown_materialization(&root, &mut output);
    task_land_attach_plan_checklist_closeout(&root, &mut output, true, None);
    Ok(Some(output))
}
