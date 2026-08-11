use super::*;

pub fn policy_eval(
    repo: &RepoRuntime,
    patchset_id: &str,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    guard_no_planning_only_artifact_drift(repo, "ait policy eval")?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    policy_eval_with_closeout_remote(&mut closeout_remote, patchset_id, &repo_name)
}

pub(in crate::primitives) fn policy_eval_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPolicyEvaluator + ?Sized,
{
    closeout_remote
        .evaluate_policy(patchset_id, Some(repo_name), false)
        .map_err(|err| err.to_string())
}

pub fn policy_show(
    repo: &RepoRuntime,
    patchset_id: &str,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    policy_show_with_closeout_remote(&mut closeout_remote, patchset_id, &repo_name)
}

pub(in crate::primitives) fn policy_show_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPolicyReader + ?Sized,
{
    closeout_remote
        .get_policy(patchset_id, Some(repo_name), false)
        .map_err(|err| err.to_string())
}

pub fn policy_waive(
    repo: &RepoRuntime,
    patchset_id: &str,
    rule_name: &str,
    reason: &str,
    expires_at: Option<&str>,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    guard_no_planning_only_artifact_drift(repo, "ait policy waive")?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    policy_waive_with_closeout_remote(
        &mut closeout_remote,
        patchset_id,
        rule_name,
        reason,
        expires_at,
        &repo_name,
    )
}

pub(in crate::primitives) fn policy_waive_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    rule_name: &str,
    reason: &str,
    expires_at: Option<&str>,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPolicyWaiverCreator + ?Sized,
{
    closeout_remote
        .create_waiver(
            patchset_id,
            rule_name,
            reason,
            expires_at,
            Some(repo_name),
            false,
        )
        .map_err(|err| err.to_string())
}
