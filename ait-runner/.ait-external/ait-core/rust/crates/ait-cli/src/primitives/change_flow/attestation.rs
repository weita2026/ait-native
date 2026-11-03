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
    ait_core::attest_json::RemoteAttestationStatement::validate_manual_inputs(
        tests, lint, security, license, model,
    )?;
    if patchset_id
        .and_then(|value| normalized_text(Some(value)))
        .is_none()
        && change_id
            .and_then(|value| normalized_text(Some(value)))
            .is_none()
    {
        return Err("Provide a Patchset ID.".to_string());
    }
    guard_no_planning_only_artifact_drift(repo, "ait attest put")?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    let resolved_author_mode = repo.effective_author_mode(author_mode);
    let resolved_model_name = None;
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
        + TaskWorkflowRemoteTaskReader
        + TaskWorkflowRemoteChangeDetailReader
        + TaskWorkflowRemoteChangeLister
        + ?Sized,
    C: TaskWorkflowPatchsetReader
        + TaskWorkflowPatchsetLister
        + TaskWorkflowAttestationWriter
        + TaskWorkflowAttestationReader
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
    R: TaskWorkflowPatchsetReader
        + TaskWorkflowAttestationReader
        + TaskWorkflowAttestationWriter
        + ?Sized,
{
    use ait_core::attest_json::RemoteAttestationStatement;
    // model_name is an automatic default from older internal callers, never an explicit claim.
    let _ = model_name;
    RemoteAttestationStatement::validate_manual_inputs(tests, lint, security, license, None)?;
    let patchset = closeout_remote
        .get_patchset(patchset_id, Some(repo_name), None)
        .map_err(|e| e.to_string())?;
    let existing = match closeout_remote.get_attestation(patchset_id, Some(repo_name), true) {
        Ok(value) => Some(value),
        Err(error) if error.remote_status() == Some(404) => None,
        Err(error) => return Err(error.to_string()),
    };
    let previous = existing
        .as_ref()
        .map(|row| RemoteAttestationStatement::validate_for_patchset(row, &patchset))
        .transpose()?;
    let mut statement = previous.clone().unwrap_or_default();
    statement.verification_state = RemoteAttestationStatement::manual_verification(tests)?;
    attestation_put_with_closeout_remote(
        closeout_remote,
        patchset_id,
        author_mode,
        &statement,
        repo_name,
    )
}

pub(in crate::primitives) fn attestation_put_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    author_mode: &str,
    statement: &ait_core::attest_json::RemoteAttestationStatement,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader
        + TaskWorkflowAttestationReader
        + TaskWorkflowAttestationWriter
        + ?Sized,
{
    use ait_core::attest_json::RemoteAttestationStatement;
    let patchset = closeout_remote
        .get_patchset(patchset_id, Some(repo_name), None)
        .map_err(|e| e.to_string())?;
    if patchset["author_mode"].as_str() != Some(author_mode) {
        return Err("Attestation author_mode must match the exact Patchset".into());
    }
    let mut wanted = statement.clone();
    match closeout_remote.get_attestation(patchset_id, Some(repo_name), true) {
        Ok(existing) => {
            let previous = RemoteAttestationStatement::validate_for_patchset(&existing, &patchset)?;
            wanted.retain_requirements_from(&previous);
            if wanted == previous {
                return Ok(existing);
            }
        }
        Err(error) if error.remote_status() == Some(404) => {}
        Err(error) => return Err(error.to_string()),
    }
    let value = closeout_remote
        .put_attestation(patchset_id, author_mode, &wanted, Some(repo_name), true)
        .map_err(|e| e.to_string())?;
    if RemoteAttestationStatement::validate_for_patchset(&value, &patchset)? != wanted {
        return Err("Remote Attestation readback differs from the requested compact statement; stopped before retrying the write".into());
    }
    Ok(value)
}

pub fn attest_show(
    repo: &RepoRuntime,
    patchset_id: &str,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    let patchset_id = resolve_public_patchset_input(repo, patchset_id, remote_name)?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    attestation_show_with_closeout_remote(&mut closeout_remote, &patchset_id, &repo_name)
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
