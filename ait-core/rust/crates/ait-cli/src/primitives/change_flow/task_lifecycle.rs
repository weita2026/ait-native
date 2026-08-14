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
