use super::*;

#[expect(
    clippy::too_many_arguments,
    reason = "arguments mirror the remote attestation command contract"
)]
pub fn attest_put(
    repo: &RepoRuntime,
    patchset_id: Option<&str>,
    change_id: Option<&str>,
    tests: Option<&str>,
    lint: Option<&str>,
    security: Option<&str>,
    license: Option<&str>,
    author_mode: Option<&str>,
    model: Option<&str>,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    if patchset_id
        .and_then(|value| normalized_text(Some(value)))
        .is_none()
        && change_id
            .and_then(|value| normalized_text(Some(value)))
            .is_none()
    {
        return Err(
            "Provide PATCHSET_ID or --change so the primitive can resolve a patchset.".to_string(),
        );
    }
    guard_no_planning_only_artifact_drift(repo, "ait attest put")?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    let resolved_author_mode = repo.effective_author_mode(author_mode);
    let resolved_model_name = repo.effective_model_name(model);
    if patchset_id.is_some() {
        let resolved_patchset_id = resolve_patchset_argument(
            repo,
            &mut closeout_remote,
            patchset_id,
            None,
            Some(&repo_name),
            remote_name,
        )?;
        return attestation_put_payload_with_closeout_remote(
            &mut closeout_remote,
            &resolved_patchset_id,
            tests,
            lint,
            security,
            license,
            &resolved_author_mode,
            resolved_model_name,
            &repo_name,
        );
    }
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    attestation_put_flow_with_task_and_closeout_remotes(
        &mut task_remote,
        &mut closeout_remote,
        None,
        change_id,
        tests,
        lint,
        security,
        license,
        &resolved_author_mode,
        resolved_model_name,
        &repo_name,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn attestation_put_flow_with_task_and_closeout_remotes<T, C>(
    task_remote: &mut T,
    closeout_remote: &mut C,
    patchset_id: Option<&str>,
    change_id: Option<&str>,
    tests: Option<&str>,
    lint: Option<&str>,
    security: Option<&str>,
    license: Option<&str>,
    author_mode: &str,
    model_name: Option<String>,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowRemoteChangeReader
        + TaskWorkflowRemoteChangeDetailReader
        + TaskWorkflowRemoteChangeLister
        + ?Sized,
    C: TaskWorkflowPatchsetReader
        + TaskWorkflowPatchsetLister
        + TaskWorkflowAttestationWriter
        + ?Sized,
{
    let resolved_patchset_id = resolve_patchset_argument_with_task_and_closeout_remotes(
        task_remote,
        closeout_remote,
        patchset_id,
        change_id,
        Some(repo_name),
    )?;
    attestation_put_payload_with_closeout_remote(
        closeout_remote,
        &resolved_patchset_id,
        tests,
        lint,
        security,
        license,
        author_mode,
        model_name,
        repo_name,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn attestation_put_payload_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    tests: Option<&str>,
    lint: Option<&str>,
    security: Option<&str>,
    license: Option<&str>,
    author_mode: &str,
    model_name: Option<String>,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowAttestationWriter + ?Sized,
{
    let attest_json = ait_core::attest_json::AttestJson::stateless();
    let evaluation_summary = attest_json.build_evaluation_summary(tests, lint, security, license);
    let (provenance_summary, detail) =
        attest_json.build_minimum_provenance(author_mode, model_name.as_deref())?;
    attestation_put_with_closeout_remote(
        closeout_remote,
        patchset_id,
        author_mode,
        &evaluation_summary,
        &provenance_summary,
        &detail,
        repo_name,
    )
}

pub(in crate::primitives) fn attestation_put_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    author_mode: &str,
    evaluation_summary: &JsonValue,
    provenance_summary: &JsonValue,
    detail: &JsonValue,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowAttestationWriter + ?Sized,
{
    closeout_remote
        .put_attestation(
            patchset_id,
            author_mode,
            evaluation_summary,
            provenance_summary,
            detail,
            Some(repo_name),
            true,
        )
        .map_err(|err| err.to_string())
}

pub fn attest_show(
    repo: &RepoRuntime,
    patchset_id: &str,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    attestation_show_with_closeout_remote(&mut closeout_remote, patchset_id, &repo_name)
}

pub(in crate::primitives) fn attestation_show_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowAttestationReader + ?Sized,
{
    closeout_remote
        .get_attestation(patchset_id, Some(repo_name), false)
        .map_err(|err| err.to_string())
}
