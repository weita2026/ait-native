use super::*;

pub(in crate::primitives) fn resolve_patchset_argument_with_task_and_closeout_remotes<T, C>(
    task_remote: &mut T,
    closeout_remote: &mut C,
    patchset_id: Option<&str>,
    change_id: Option<&str>,
    repo_name: Option<&str>,
) -> Result<String, String>
where
    T: TaskWorkflowRemoteChangeReader + TaskWorkflowRemoteTaskReader + ?Sized,
    C: TaskWorkflowPatchsetReader + TaskWorkflowPatchsetLister + ?Sized,
{
    if let Some(patchset_id) = patchset_id {
        return resolve_patchset_id(closeout_remote, patchset_id, repo_name);
    }
    let Some(change_id) = change_id else {
        return Err("Provide a Patchset ID.".to_string());
    };
    let (resolved_change_ref, current_patchset_id) =
        change_identity_with_task_remote(task_remote, change_id, repo_name)?;
    let current = current_patchset_id
        .ok_or_else(|| format!("Task for {resolved_change_ref} has no current Patchset"))?;
    let row = closeout_remote
        .get_patchset(&current, repo_name, None)
        .map_err(|err| err.to_string())?;
    let task = if is_task_id(&resolved_change_ref) {
        resolved_change_ref.as_str()
    } else {
        resolved_change_ref
            .rsplit_once('/')
            .ok_or("Change has no Task")?
            .0
    };
    let association =
        ait_core::workflow_expected_identity::validate_patchset_owner_fields(&row, task)?;
    if !is_task_id(&resolved_change_ref)
        && association.as_deref() != Some(resolved_change_ref.as_str())
    {
        return Err("Current Task candidate belongs to another Change".into());
    }
    let expected = ait_core::workflow_expected_identity::WorkflowExpectedIdentity {
        repository_index: 0,
        patchset_id: current,
        base_snapshot_id: required_string_field(&row, "base_snapshot_id")?,
        revision_snapshot_id: required_string_field(&row, "revision_snapshot_id")?,
    };
    expected.validate_patchset(0, &row)?;
    required_string_field(&row, "patchset_id")
}

pub(in crate::primitives) fn resolve_patchset_argument<R>(
    repo: &RepoRuntime,
    closeout_remote: &mut R,
    patchset_id: Option<&str>,
    change_id: Option<&str>,
    repo_name: Option<&str>,
    remote_name: Option<&str>,
) -> Result<String, String>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowPatchsetLister + ?Sized,
{
    if let Some(patchset_id) = patchset_id {
        let patchset_id = resolve_public_patchset_input(repo, patchset_id, remote_name)?;
        return resolve_patchset_id(closeout_remote, &patchset_id, repo_name);
    }
    let Some(change_id) = change_id else {
        return Err("Provide a Patchset ID.".to_string());
    };
    let remote_row = repo.remote_row(remote_name)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    resolve_patchset_argument_with_task_and_closeout_remotes(
        &mut task_remote,
        closeout_remote,
        None,
        Some(change_id),
        repo_name,
    )
}

pub(in crate::primitives) fn resolve_patchset_id<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    repo_name: Option<&str>,
) -> Result<String, String>
where
    R: TaskWorkflowPatchsetReader + ?Sized,
{
    closeout_remote
        .get_patchset(patchset_id, repo_name, None)
        .map_err(|err| err.to_string())
        .and_then(|patchset| required_string_field(&patchset, "patchset_id"))
}

pub(in crate::primitives) fn change_identity_with_task_remote<R>(
    task_remote: &mut R,
    change_id: &str,
    repo_name: Option<&str>,
) -> Result<(String, Option<String>), String>
where
    R: TaskWorkflowRemoteChangeReader + TaskWorkflowRemoteTaskReader + ?Sized,
{
    let owner = if is_task_id(change_id) {
        change_id.to_string()
    } else {
        let change = task_remote
            .get_change(change_id, repo_name)
            .map_err(|err| err.to_string())?;
        change_reference_from_payload(&change, Some(change_id))?
    };
    let task_id = if is_task_id(&owner) {
        owner.as_str()
    } else {
        owner.rsplit_once('/').ok_or("Change has no Task owner")?.0
    };
    let task = task_remote
        .get_task(task_id, repo_name)
        .map_err(|err| err.to_string())?;
    if task["task_id"].as_str() != Some(task_id) {
        return Err("Task readback owner mismatch".into());
    }
    ait_core::land_json::LandJson::stateless().task_successful_receipt(&task, task_id, None)?;
    let current = match task.get("current_patchset_id") {
        Some(JsonValue::Null) => None,
        Some(JsonValue::String(id)) if !id.is_empty() && id.trim() == id => Some(id.clone()),
        _ => return Err("Task requires nullable current_patchset_id".into()),
    };
    Ok((owner, current))
}
