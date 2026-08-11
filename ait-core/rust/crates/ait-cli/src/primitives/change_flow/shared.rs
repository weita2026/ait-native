use super::*;

pub(in crate::primitives) fn resolve_patchset_argument_with_task_and_closeout_remotes<T, C>(
    task_remote: &mut T,
    closeout_remote: &mut C,
    patchset_id: Option<&str>,
    change_id: Option<&str>,
    repo_name: Option<&str>,
) -> Result<String, String>
where
    T: TaskWorkflowRemoteChangeReader + ?Sized,
    C: TaskWorkflowPatchsetReader + TaskWorkflowPatchsetLister + ?Sized,
{
    if let Some(patchset_id) = patchset_id {
        return resolve_patchset_id(closeout_remote, patchset_id, repo_name);
    }
    let Some(change_id) = change_id else {
        return Err(
            "Provide PATCHSET_ID or --change so the primitive can resolve a patchset.".to_string(),
        );
    };
    let (resolved_change_ref, selected_patchset_id) =
        change_identity_with_task_remote(task_remote, change_id, repo_name)?;
    if let Some(selected) = selected_patchset_id {
        return Ok(selected);
    }
    let mut rows = closeout_remote
        .list_patchsets(&resolved_change_ref, repo_name)
        .map_err(|err| err.to_string())?;
    rows.sort_by_key(|row| {
        row.get("patchset_number")
            .and_then(JsonValue::as_i64)
            .unwrap_or_default()
    });
    rows.last()
        .and_then(|row| string_field(row, "patchset_id"))
        .ok_or_else(|| format!("Change {resolved_change_ref} has no patchsets"))
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
        return resolve_patchset_id(closeout_remote, patchset_id, repo_name);
    }
    let Some(change_id) = change_id else {
        return Err(
            "Provide PATCHSET_ID or --change so the primitive can resolve a patchset.".to_string(),
        );
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
    R: TaskWorkflowRemoteChangeReader + ?Sized,
{
    let change = task_remote
        .get_change(change_id, repo_name)
        .map_err(|err| err.to_string())?;
    Ok((
        change_reference_from_payload(&change, Some(change_id))?,
        string_field(&change, "selected_patchset_id"),
    ))
}
