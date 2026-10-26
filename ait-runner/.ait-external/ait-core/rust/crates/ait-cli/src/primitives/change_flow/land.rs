use super::*;

pub fn land_submit(
    repo: &RepoRuntime,
    change_id: &str,
    patchset_id: Option<&str>,
    target_line: &str,
    mode: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    guard_repo_root_pinned_bound_worktree(repo, None, "ait task finish")?;
    guard_no_planning_only_artifact_drift(repo, "ait task finish")?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    let resolve_change_task_id = repo_root_has_bound_worktree_metadata(repo)?;
    land_submit_flow_with_task_and_closeout_remotes(
        &mut task_remote,
        &mut closeout_remote,
        &repo_name,
        change_id,
        patchset_id,
        target_line,
        mode,
        resolve_change_task_id,
        |change_task_id, resolved_change_id| {
            guard_repo_root_bound_task_worktree(
                repo,
                change_task_id,
                Some(resolved_change_id),
                "ait task finish",
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn land_submit_flow_with_task_and_closeout_remotes<T, C, G>(
    task_remote: &mut T,
    closeout_remote: &mut C,
    repo_name: &str,
    change_id: &str,
    patchset_id: Option<&str>,
    target_line: &str,
    mode: &str,
    resolve_change_task_id: bool,
    guard: G,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowRemoteChangeReader
        + TaskWorkflowRemoteChangeDetailReader
        + TaskWorkflowRemoteChangeLister
        + ?Sized,
    C: TaskWorkflowPatchsetReader + TaskWorkflowLandSubmitter + ?Sized,
    G: FnOnce(Option<&str>, &str) -> Result<(), String>,
{
    let _flow_range = perfetto_range!("ait.task_land.remote.land_submit_prepare");
    let change = {
        let _range = perfetto_range!("ait.task_land.remote.submit_change_read");
        change_show_with_task_remote(task_remote, change_id, repo_name)?
    };
    let resolved_change_id =
        string_field(&change, "change_id").unwrap_or_else(|| change_id.to_string());
    let change_task_id = {
        let _range = perfetto_range!("ait.task_land.remote.submit_task_resolve");
        if resolve_change_task_id {
            remote_change_task_id(
                task_remote,
                repo_name,
                &change,
                change_id,
                &resolved_change_id,
            )?
        } else {
            None
        }
    };
    {
        let _range = perfetto_range!("ait.task_land.local.bound_worktree_guard");
        guard(change_task_id.as_deref(), &resolved_change_id)?;
    }
    let resolved_patchset_id = match patchset_id {
        Some(value) => {
            let _range = perfetto_range!("ait.task_land.remote.submit_patchset_read");
            Some(resolve_patchset_id(
                closeout_remote,
                value,
                Some(repo_name),
            )?)
        }
        None => None,
    };
    land_submit_with_closeout_remote(
        closeout_remote,
        change_id,
        resolved_patchset_id.as_deref(),
        target_line,
        mode,
        repo_name,
    )
}

pub(in crate::primitives) fn land_submit_with_closeout_remote<R>(
    closeout_remote: &mut R,
    change_id: &str,
    patchset_id: Option<&str>,
    target_line: &str,
    mode: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLandSubmitter + ?Sized,
{
    let _range = perfetto_range!("ait.task_land.remote.submit_land_http");
    closeout_remote
        .submit_land(change_id, patchset_id, target_line, mode, Some(repo_name))
        .map_err(|err| err.to_string())
}

pub fn land_show(
    repo: &RepoRuntime,
    submission_id: &str,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    guard_no_planning_only_artifact_drift(repo, "ait land retry")?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    land_show_with_closeout_remote(&mut closeout_remote, submission_id, &repo_name)
}

pub(in crate::primitives) fn land_show_with_closeout_remote<R>(
    closeout_remote: &mut R,
    submission_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLandReader + ?Sized,
{
    closeout_remote
        .get_land(submission_id, Some(repo_name))
        .map_err(|err| err.to_string())
}

pub fn land_retry(
    repo: &RepoRuntime,
    submission_id: &str,
    reason: Option<&str>,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    land_retry_with_closeout_remote(&mut closeout_remote, submission_id, reason, &repo_name)
}

pub(in crate::primitives) fn land_retry_with_closeout_remote<R>(
    closeout_remote: &mut R,
    submission_id: &str,
    reason: Option<&str>,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLandRetryer + ?Sized,
{
    closeout_remote
        .retry_land(submission_id, reason, Some(repo_name))
        .map_err(|err| err.to_string())
}
