use super::*;
use ait_core::workflow_package::{
    PackageAvailability, PackageCiState, PackageCompletion, PackageCompletionOutcome,
    PackageExecution, PreparedPackageExecution,
};

#[derive(Default)]
pub struct PackageReadyOptions<'a> {
    pub summary: Option<&'a str>,
    pub tests: Option<&'a str>,
    pub lint: Option<&'a str>,
    pub security: Option<&'a str>,
    pub license: Option<&'a str>,
    pub author_mode: Option<&'a str>,
    pub model: Option<&'a str>,
}

#[derive(Clone, Copy)]
struct FinishReadScope<'a> {
    target: &'a str,
    repo_name: &'a str,
}

pub(super) fn state(
    reference: &str,
    identity: Option<&PackageWorkflowExpectedIdentity>,
    code: &str,
    detail: &str,
) -> JsonValue {
    let mut result = json!({"contract":"ait-agent-action/v1","task_id":reference,"mode":"workflow_package","apply_status":if code=="done" {"done"} else {"stopped"},"next_action":{"code":code,"summary":detail},"apply_stopped_reason":if code=="done" {JsonValue::Null} else {json!(detail)}});
    if let Some(identity) = identity {
        result["expected_identity"] = json!(identity);
    }
    result
}

fn execution_state(
    reference: &str,
    identity: &PackageWorkflowExpectedIdentity,
    execution: Option<PackageExecution>,
) -> JsonValue {
    let Some(execution) = execution else {
        return state(reference,Some(identity),"fresh_ready_required","The exact package association is unavailable. Keep its selector; only explicit applied ready may request Server-validated retained-source reprepare.");
    };
    let (code,detail) = match execution.ci_state {
        PackageCiState::Queued | PackageCiState::Running if execution.availability==PackageAvailability::Available && execution.recoverable => ("waiting_for_ci","Wait for the original selected package Job."),
        PackageCiState::Passed if execution.ready_passed() => ("done","The selected package Job has passing CI. Review this exact identity before finish."),
        PackageCiState::Failed => ("ci_failed","The selected Job has failing required CI. A new intentional ready is required to rerun."),
        PackageCiState::Interrupted => ("ci_interrupted","The selected execution was interrupted. A new intentional ready is required."),
        PackageCiState::Unknown => ("ci_unknown","The selected execution has no trustworthy CI result. Do not infer success or rerun automatically."),
        _ => ("fresh_ready_required","The exact source or execution is unavailable. A new intentional ready is required."),
    };
    let mut result = state(reference, Some(identity), code, detail);
    result["package_execution"] = json!(execution);
    result
}

fn read_execution(
    client: &mut PlanHttpClientManager,
    reference: &str,
    identity: &PackageWorkflowExpectedIdentity,
) -> (JsonValue, bool) {
    match client.read_workflow_package_execution(identity) {
        Ok(execution) => {
            let retained_check = execution.as_ref().is_none_or(|execution| {
                execution.ci_state == PackageCiState::Interrupted && !execution.recoverable
            });
            (
                execution_state(reference, identity, execution),
                retained_check,
            )
        }
        Err(error) => (
            state(
                reference,
                Some(identity),
                "execution_unknown",
                &format!("Unable to read the original package Job: {error}"),
            ),
            false,
        ),
    }
}

fn ready_author_mode(
    result: &JsonValue,
    prepared_mode: Option<&str>,
    requested: Option<&str>,
) -> Result<Option<String>, String> {
    // Job mode is authoritative only after PackageExecution validated the
    // selected Repository and the complete original five-field identity.
    let read_mode = result["package_execution"]["author_mode"].as_str();
    if let (Some(prepared), Some(read)) = (prepared_mode, read_mode) {
        if prepared != read {
            return Err("Job readback conflicts with the sealed package author mode".into());
        }
    }
    let sealed = prepared_mode.or(read_mode);
    if let (Some(requested), Some(sealed)) = (requested, sealed) {
        if requested != sealed {
            return Err("Explicit author mode conflicts with the original sealed package; keep its mode, package and Job".into());
        }
    }
    Ok(sealed.map(str::to_string))
}

fn evidence(
    client: &mut PlanHttpClientManager,
    identity: &PackageWorkflowExpectedIdentity,
    operation: &str,
    key: &str,
    payload: JsonValue,
) -> Result<JsonValue, String> {
    let response = client
        .record_workflow_package_evidence(identity, operation, key, payload)
        .map_err(|error| error.to_string())?;
    let data = ait_core::workflow_package::package_response_data(&response)?;
    let returned = PackageWorkflowExpectedIdentity::from_value(&data["expected_identity"])?;
    if returned != *identity
        || data["operation"].as_str() != Some(operation)
        || data["accepted"].as_bool() != Some(true)
        || data["recorded_at_s"].as_u64().is_none()
    {
        return Err(
            "Package evidence response lacks exact accepted identity and Server event time".into(),
        );
    }
    Ok(data.clone())
}

fn review_api_action(
    identity: &PackageWorkflowExpectedIdentity,
    actor_key: Option<u32>,
    key: &str,
    payload: JsonValue,
) -> JsonValue {
    json!({"method":"POST","path":format!("/v1/native/repository-authorities/{}/worker-jobs/{}",identity.repository_index,identity.worker_job_index),
        "body":{"contract":"workflow-package-evidence/v1","expected_identity":identity,"operation":"review","idempotency_key":key,"actor_key_index":actor_key,"payload":payload}})
}

pub fn workflow_package_ready<C>(
    repo: &RepoRuntime,
    reference: &str,
    remote_name: Option<&str>,
    expected: Option<&PackageWorkflowExpectedIdentity>,
    apply: bool,
    options: PackageReadyOptions<'_>,
    mut ci_wait_hook: Option<C>,
) -> Result<JsonValue, String>
where
    C: FnMut(&mut JsonValue) -> Result<bool, String>,
{
    use ait_core::attest_json::{AttestJson, RemoteAttestationStatement, RemoteVerificationState};
    RemoteAttestationStatement::validate_manual_inputs(
        options.tests,
        options.lint,
        options.security,
        options.license,
        options.model,
    )?;
    if options
        .summary
        .is_some_and(|text| text.trim().is_empty() || text.len() > usize::from(u16::MAX))
    {
        return Err("Package summary must contain 1..65535 UTF-8 bytes".into());
    }
    let (remote, repo_name) = remote_context(repo, remote_name, None)?;
    // Validate the completed source before any network mutation.
    workflow_completed_local_entry_if_present(repo,reference,Some(&remote.name))?.ok_or("Unfinished Local Task history requires a real successful receipt before package preparation")?;
    let mut client = package_client(repo, &remote, &repo_name)?;
    let (mut identity, mut sealed_author_mode) = if let Some(identity) = expected {
        if options.summary.is_some() {
            return Err("Package recovery retains its original summary; omit --summary or select an intentional fresh ready".into());
        }
        recover_source_history(repo, reference, &remote.name, identity)?;
        (identity.clone(), None)
    } else {
        if !apply {
            return Ok(state(
                reference,
                None,
                "prepare_package",
                "Apply ready to package existing completed Local history and select a package Job.",
            ));
        }
        guard_no_planning_only_artifact_drift(repo, "ait workflow ready")?;
        let prepared = prepare_source_history(
            repo,
            &remote,
            reference,
            options.author_mode,
            options.summary,
        )?;
        if prepared.mappings.is_empty() {
            return Err("Prepared package has no exact source Task inventory".into());
        }
        let response = client
            .prepare_workflow_package(&prepared.manifest, &prepared.package, None)
            .map_err(|error| error.to_string())?;
        let selected = PreparedPackageExecution::from_value(&response)?;
        let identity = selected.expected_identity;
        if identity.repository_index != prepared.manifest.repository_index
            || identity.package_hash != prepared.package.package_hash
            || identity.base_snapshot_id != prepared.manifest.base_snapshot_id
            || identity.revision_snapshot_id != prepared.manifest.revision_snapshot_id
        {
            return Err("Prepare response selected another package candidate".into());
        }
        (identity, Some(prepared.manifest.author_mode))
    };
    let (mut result, retained_check) = read_execution(&mut client, reference, &identity);
    if apply && expected.is_some() && retained_check {
        sealed_author_mode = ready_author_mode(&result, None, options.author_mode)?;
        if options.author_mode.is_some() && sealed_author_mode.is_none() {
            return Ok(state(reference, Some(&identity), "author_mode_required_action", "The original sealed mode cannot yet be verified for this explicit override. Keep the original selector; do not reprepare merely to discover whether a mode matches."));
        }
        let response = match client.reprepare_workflow_package(&identity) {
            Ok(response) => response,
            Err(error) => return Ok(state(reference, Some(&identity), "execution_unknown", &format!("Retained-source reprepare was not confirmed: {error}. Keep the original selector for an explicit ready retry; no replacement identity is inferred."))),
        };
        let selected = PreparedPackageExecution::from_value(&response)?;
        let replacement = selected.expected_identity;
        if replacement.repository_index != identity.repository_index
            || replacement.package_hash != identity.package_hash
            || replacement.base_snapshot_id != identity.base_snapshot_id
            || replacement.revision_snapshot_id != identity.revision_snapshot_id
            || replacement.worker_job_index == identity.worker_job_index
        {
            return Err("Retained-source reprepare must return a new Job for the same sealed Repository/hash/base/final".into());
        }
        identity = replacement;
        result = read_execution(&mut client, reference, &identity).0;
    }
    let mut author_mode =
        ready_author_mode(&result, sealed_author_mode.as_deref(), options.author_mode)?;
    if apply
        && author_mode.is_none()
        && matches!(
            result["next_action"]["code"].as_str(),
            Some("waiting_for_ci" | "done")
        )
    {
        return Ok(state(reference, Some(&identity), "author_mode_required_action", "The original Job has no verified sealed author mode. This Server may not expose it, or its sealed source is unavailable. Keep the same package and Job; an explicit --author-mode or current default cannot replace that evidence."));
    }
    if apply && result["next_action"]["code"].as_str() == Some("waiting_for_ci") {
        let wait = if let Some(hook) = ci_wait_hook.as_mut() {
            hook(&mut result)?
        } else {
            true
        };
        if wait {
            result = workflow_wait_for_pending_state(repo, &result, "waiting_for_ci", || {
                Ok(read_execution(&mut client, reference, &identity).0)
            })?;
            author_mode = ready_author_mode(
                &result,
                sealed_author_mode.as_deref().or(author_mode.as_deref()),
                options.author_mode,
            )?;
        }
    }
    if apply && result["next_action"]["code"].as_str() == Some("done") {
        let Some(author_mode) = author_mode else {
            return Ok(state(reference, Some(&identity), "author_mode_required_action", "The original Job has no verified sealed author mode. Keep the same package and Job; an explicit --author-mode cannot replace that evidence."));
        };
        let statement = RemoteAttestationStatement {
            verification_state: if options.tests.is_some() {
                RemoteAttestationStatement::manual_verification(options.tests)?
            } else {
                RemoteVerificationState::Pass
            },
            require_human_review: author_mode == "ai_with_human_review",
            ..Default::default()
        };
        let payload = AttestJson::stateless()
            .build_put_attestation_body(&author_mode, &statement)
            .map_err(|error| error.to_string())?;
        // A recovered invocation cannot silently replace an earlier statement.
        let key = format!(
            "workflow-package:{}:{}:ready-attestation",
            identity.package_hash, identity.worker_job_index
        );
        if let Err(error) = evidence(&mut client, &identity, "attestation", &key, payload) {
            result = state(
                reference,
                Some(&identity),
                "attestation_required_action",
                &format!("Exact package attestation was not confirmed: {error}"),
            );
        }
    }
    Ok(result)
}

fn admitted(
    client: &mut PlanHttpClientManager,
    reference: &str,
    identity: &PackageWorkflowExpectedIdentity,
) -> Option<JsonValue> {
    use ait_core::workflow_admission::{
        FinishAdmissionObservation as Observation, FinishAdmissionTracker,
        FinishAdmissionWaitState as Wait,
    };
    let mut tracker = FinishAdmissionTracker::new(Instant::now(), Duration::from_secs(30));
    let mut previous = None;
    loop {
        let health = match client.get_upgrade_health() {
            Ok(health) => health,
            Err(error) => {
                let disconnected = matches!(
                    error,
                    ait_core::plan_http_client::PlanHttpClientError::Transport(_)
                        | ait_core::plan_http_client::PlanHttpClientError::Closed(_)
                );
                return Some(state(
                    reference,
                    Some(identity),
                    if disconnected {
                        "transport_disconnected"
                    } else {
                        "admission_unavailable"
                    },
                    &format!(
                        "Server admission cannot be established; finish was not sent: {error}"
                    ),
                ));
            }
        };
        let observation = if health.ready {
            Observation::Admitted
        } else {
            health
                .progress()
                .map(Observation::Upgrading)
                .unwrap_or(Observation::ProgressUnavailable)
        };
        match tracker.observe(Instant::now(), observation) {
            Wait::Admitted => return None,
            Wait::Upgrading(progress) => {
                let display = json!({"phase":health.phase,"processed":health.processed,"total":health.total,"last_progress_at":health.last_progress_at});
                if previous.as_ref() != Some(&display) {
                    eprintln!("Server upgrade: {display}");
                    previous = Some(display.clone());
                }
                if progress.stalled {
                    let mut result=state(reference,Some(identity),"upgrade_stalled","Server upgrade has no advancing progress; agent action is required. Finish was not sent.");
                    result["upgrade_progress"] = display;
                    return Some(result);
                }
            }
            Wait::ProgressUnavailable { .. } => {
                let mut result=state(reference,Some(identity),"upgrade_progress_unavailable","Server is upgrading without available progress; agent action is required. Finish was not sent.");
                result["upgrade_progress"] = json!({"phase":health.phase,"processed":health.processed,"total":health.total,"last_progress_at":health.last_progress_at});
                return Some(result);
            }
            Wait::TransportDisconnected { .. } => unreachable!("transport failures return above"),
        }
        sleep(Duration::from_secs(1));
    }
}

fn confirmed(
    repo: &RepoRuntime,
    reference: &str,
    result: PackageCompletion,
    source: &[PackageMappingExpectation],
) -> JsonValue {
    let mut output = state(
        reference,
        Some(&result.expected_identity),
        "done",
        "The exact package finish and full batch mappings are confirmed.",
    );
    output["completion"] = json!(result);
    output["remote_task_id"] = json!(result.entries.last().map(|row| &row.remote_task_id));
    output["patchset_id"] = json!(result.patchset_id);
    output["submission_id"] = json!(result.submission_id);
    match repo.task_store().and_then(|store| {
        store
            .record_package_publication(&result, source)
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => output["closeout_status"] = json!("complete"),
        Err(error) => {
            output["apply_status"] = json!("partial");
            output["closeout_status"] = json!("partial");
            output["next_action"] = json!({"code":"restore_publication_mappings","summary":error});
        }
    }
    output
}

/// The caller supplies a validated finish response or exact formal read join.
/// Formal acceptance alone never authorizes incomplete mapping writeback.
pub(super) fn accepted_mapping_pending(
    reference: &str,
    result: &PackageCompletion,
) -> Option<JsonValue> {
    if !result.is_accepted() || result.has_complete_mappings() {
        return None;
    }
    let mut output = state(
        reference,
        Some(&result.expected_identity),
        "restore_publication_mappings",
        "The exact Remote package finish is accepted, but complete original mapping evidence is unavailable. Local publication mappings remain pending. Retain this identity for exact readback without resubmitting Remote finish or restarting CI.",
    );
    output["apply_status"] = json!("partial");
    output["closeout_status"] = json!("partial");
    output["completion"] = json!(result);
    output["patchset_id"] = json!(result.patchset_id);
    output["submission_id"] = json!(result.submission_id);
    Some(output)
}

fn finish_response(
    repo: &RepoRuntime,
    client: &mut PlanHttpClientManager,
    reference: &str,
    identity: &PackageWorkflowExpectedIdentity,
    scope: FinishReadScope<'_>,
    source: &[PackageMappingExpectation],
    response: &Result<JsonValue, ait_core::plan_http_client::PlanHttpClientError>,
) -> Option<JsonValue> {
    if let Ok(response) = response {
        if let Ok(completion) = PackageCompletion::from_value(response, identity, source) {
            if completion.has_complete_mappings() {
                return Some(confirmed(repo, reference, completion, source));
            }
            if let Some(result) = accepted_mapping_pending(reference, &completion) {
                return Some(result);
            }
        }
    }
    let detail = response.as_ref().err()?.to_string();
    let code = if detail.contains("PACKAGE_FINISH_CLEANUP_PENDING") {
        "finish_incomplete"
    } else if detail.contains("PACKAGE_FINISH_CLEANUP_UNKNOWN") {
        "completion_unknown"
    } else {
        return None;
    };
    let completion =
        client.lookup_workflow_package_completion(identity, scope.target, source, scope.repo_name);
    if let Some(mut result) = accepted_mapping_pending(reference, &completion) {
        let summary = format!(
            "{} Server cleanup observation: {detail}.",
            result["next_action"]["summary"]
                .as_str()
                .unwrap_or_default()
        );
        result["next_action"]["summary"] = json!(summary);
        result["apply_stopped_reason"] = json!(summary);
        return Some(result);
    }
    let mut result = state(reference, Some(identity), code, &format!(
        "Server reported unresolved cleanup: {detail}. Formal acceptance and complete mappings remain independently unverified. Retain this exact identity and retry the same finish for exact readback; do not run ready or CI."
    ));
    result["completion"] = json!(PackageCompletion::unknown(identity));
    Some(result)
}

fn finish_committed_package(
    repo: &RepoRuntime,
    client: &mut PlanHttpClientManager,
    reference: &str,
    identity: &PackageWorkflowExpectedIdentity,
    scope: FinishReadScope<'_>,
    source: &[PackageMappingExpectation],
    apply: bool,
) -> JsonValue {
    if !apply {
        return state(reference, Some(identity), "finish_incomplete", "The original Job has a formal Patchset binding. Apply the same exact finish to recover required cleanup and its full result; do not run ready or CI.");
    }
    // The Server must resolve the original result before cleanup and may not
    // publish again. This invocation sends exactly one guarded request.
    let response = client.finish_workflow_package(identity, scope.target, "direct");
    if let Some(result) =
        finish_response(repo, client, reference, identity, scope, source, &response)
    {
        return result;
    }
    let detail = match response {
        Ok(_) => "Finish response lacks the exact full original source mappings".into(),
        Err(error) => error.to_string(),
    };
    let mut result = state(reference, Some(identity), "completion_unknown", &format!(
        "{detail}. The exact full result and required cleanup are not confirmed. Finish was not resent; retain this identity, resolve any reported blocker, and retry the same finish. Do not run ready or CI."
    ));
    result["completion"] = json!(PackageCompletion::unknown(identity));
    result
}

pub fn workflow_package_task_review(
    repo: &RepoRuntime,
    reference: &str,
    identity: &PackageWorkflowExpectedIdentity,
    action: &str,
    message: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if !matches!(
        action,
        "task_approve" | "task_request_changes" | "task_comment" | "task_defer"
    ) {
        return Err("Unsupported existing Task review operation for a package selector".into());
    }
    if action == "task_approve" && !workflow_task_review_required(repo) {
        return Err(
            "Task review is configured as automatic; workflow owns automatic Task approval".into(),
        );
    }
    let message = normalized_text(message);
    if action == "task_approve" && message.is_none() {
        return Err("Task approval requires functional validation in --message".into());
    }
    let reviewer = repo.task_review_reviewer_identity().ok_or(
        "Task review requires configured user_name; reviewer identity cannot be overridden",
    )?;
    let (remote, repo_name) = remote_context(repo, remote_name, None)?;
    let (_, source) = recover_source_history(repo, reference, &remote.name, identity)?;
    if source.last().map(|entry| entry.local_task_id.as_str()) != Some(reference) {
        return Err("Task review must select the final aggregate Local source Task".into());
    }
    let mut client = package_client(repo, &remote, &repo_name)?;
    if !client
        .read_workflow_package_execution(identity)
        .map_err(|error| error.to_string())?
        .is_some_and(|execution| execution.ready_passed())
    {
        return Err(
            "Package review requires the exact available Job with passing required CI".into(),
        );
    }
    let payload = json!({"reviewer":reviewer,"reviewer_kind":"user","action":action,"comment":message,"blocking":action=="task_request_changes"});
    // This explicit invocation is a new ordered event. The key remains stable
    // for its one request and supplies no claimed evidence timestamp.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let key = format!(
        "workflow-package:{}:{}:{action}:{nonce}",
        identity.package_hash, identity.worker_job_index
    );
    let mut response = match evidence(&mut client, identity, "review", &key, payload.clone()) {
        Ok(response) => response,
        Err(error) => {
            let mut result=state(reference,Some(identity),"task_review_required_action",&format!("Task review acceptance was not confirmed: {error}. Retry this exact intent through the existing evidence API with its original key; a new CLI invocation is a new explicit decision."));
            result["next_action"]["request"] = review_api_action(
                identity,
                client.require_actor_key_index().ok(),
                &key,
                payload,
            );
            return Ok(result);
        }
    };
    response["idempotency_key"] = json!(key);
    response["task_id"] = json!(reference);
    response["reviewer"] = json!(reviewer);
    response["action"] = json!(action);
    Ok(response)
}

pub fn workflow_package_finish(
    repo: &RepoRuntime,
    reference: &str,
    remote_name: Option<&str>,
    identity: &PackageWorkflowExpectedIdentity,
    apply: bool,
    reviewer: Option<&str>,
    review_message: Option<&str>,
) -> Result<JsonValue, String> {
    let (remote, repo_name) = remote_context(repo, remote_name, None)?;
    identity.validate()?;
    if repo.require_repository_index()?.get() != identity.repository_index {
        return Err("Package identity belongs to another Repository".into());
    }
    let (target, source) = match recover_source_history(repo, reference, &remote.name, identity) {
        Ok(source) => source,
        Err(error) => {
            let mut result=state(reference,Some(identity),"completion_unknown",&format!("The original source set cannot be verified: {error}. This does not establish finish failure and does not authorize a resend."));
            result["completion"] = json!(PackageCompletion::unknown(identity));
            return Ok(result);
        }
    };
    let mut client = package_client(repo, &remote, &repo_name)?;
    if apply {
        if let Some(wait) = admitted(&mut client, reference, identity) {
            return Ok(wait);
        }
    }
    let (completion, committed) =
        client.observe_workflow_package_completion(identity, &target, &source, &repo_name);
    if let Some(result) = accepted_mapping_pending(reference, &completion) {
        return Ok(result);
    }
    if committed {
        return Ok(finish_committed_package(
            repo,
            &mut client,
            reference,
            identity,
            FinishReadScope {
                target: &target,
                repo_name: &repo_name,
            },
            &source,
            apply,
        ));
    }
    if completion.outcome != PackageCompletionOutcome::Incomplete {
        return Ok(state(
            reference,
            Some(identity),
            "completion_unknown",
            "The exact finish outcome is unknown. Do not resend finish or infer missing mappings.",
        ));
    }
    let execution = read_execution(&mut client, reference, identity).0;
    if execution["next_action"]["code"].as_str() != Some("done") {
        return Ok(execution);
    }
    if !apply {
        return Ok(state(reference,Some(identity),"review_and_finish","Apply finish with the acting Reviewer and a structured code review summary for this exact package."));
    }
    guard_no_planning_only_artifact_drift(repo, "ait workflow finish")?;
    let actor = crate::reviewer_identity::ActingReviewer::resolve(repo, reviewer)?;
    let message =
        normalized_text(review_message).ok_or_else(apply_action::review_message_required_error)?;
    let missing = missing_code_review_summary_sections(&message);
    if !missing.is_empty() {
        return Err(apply_action::review_summary_missing_sections_error(
            &missing,
        ));
    }
    use sha2::{Digest, Sha256};
    let key = format!(
        "workflow-package:{}:{}:code-review:{:x}",
        identity.package_hash,
        identity.worker_job_index,
        Sha256::digest(message.as_bytes())
    );
    evidence(
        &mut client,
        identity,
        "review",
        &key,
        json!({"reviewer":actor.name,"reviewer_kind":crate::reviewer_identity::ActingReviewer::ACTOR_KIND,"action":"code_review_summary","comment":message,"blocking":false}),
    )?;
    if !workflow_task_review_required(repo) {
        let key = format!(
            "workflow-package:{}:{}:task-approve",
            identity.package_hash, identity.worker_job_index
        );
        evidence(
            &mut client,
            identity,
            "review",
            &key,
            json!({"reviewer":actor.name,"reviewer_kind":crate::reviewer_identity::ActingReviewer::ACTOR_KIND,"action":"task_approve","comment":AUTOMATIC_TASK_APPROVAL_COMMENT,"blocking":false}),
        )?;
    }
    if let Some(wait) = admitted(&mut client, reference, identity) {
        return Ok(wait);
    }
    // Existing workflow finish uses direct mode. This request carries no archive
    // and is issued once; all subsequent work in this invocation is read-only.
    let response = client.finish_workflow_package(identity, &target, "direct");
    if let Some(result) = finish_response(
        repo,
        &mut client,
        reference,
        identity,
        FinishReadScope {
            target: &target,
            repo_name: &repo_name,
        },
        &source,
        &response,
    ) {
        return Ok(result);
    }
    let completion =
        client.lookup_workflow_package_completion(identity, &target, &source, &repo_name);
    if let Some(result) = accepted_mapping_pending(reference, &completion) {
        return Ok(result);
    }
    let detail = match response {
        Ok(_) => "Finish response has no confirmed exact formal acceptance".to_string(),
        Err(error) => error.to_string(),
    };
    let mut result = state(
        reference,
        Some(identity),
        if completion.outcome == PackageCompletionOutcome::Incomplete {
            "finish_incomplete"
        } else {
            "completion_unknown"
        },
        &format!("{detail}. Finish was not resent; retain this identity for result lookup."),
    );
    if completion.outcome == PackageCompletionOutcome::Incomplete
        && [
            "PACKAGE_REVIEW_BLOCKED",
            "PACKAGE_CODE_REVIEW_REQUIRED",
            "PACKAGE_TASK_APPROVAL_REQUIRED",
            "PACKAGE_HUMAN_REVIEW_REQUIRED",
        ]
        .iter()
        .any(|code| detail.contains(code))
    {
        result["next_action"]["review_required_action"] = json!("Existing CLI summary/automatic approval keys are exact retries, not later decisions. Each blocked principal must make a fresh positive decision in the affected lane through the existing evidence API with a NEW caller-stable key. Keep this Job/CI; ordinary comments and the other lane do not clear objections. Human approval requires an authorized user principal.");
        if detail.contains("PACKAGE_REVIEW_BLOCKED") {
            result["next_action"]["review_required_action"] = json!("The Server did not identify the blocked principal or lane. Select the template matching EACH blocked principal's truthful kind and affected lane, and fill in that principal's exact name, an authorized request Actor key, their fresh explicit decision and a NEW caller-stable key. User principals require their exact authorized Actor identity. A General code_review_summary needs the structured review sections; it does not supply human approval. Keep this Job/CI. Existing retry keys, other principals and the other lane cannot clear the objection.");
            result["next_action"]["request_templates"] = JsonValue::Array([
                ("general", "bot", "code_review_summary"),
                ("general", "user", "code_review_summary"),
                ("task", "bot", "task_approve"),
                ("task", "user", "task_approve"),
            ]
            .into_iter()
            .map(|(lane, kind, action)| {
                let mut request = review_api_action(
                    identity,
                    None,
                    "<new-caller-stable-key-for-this-decision>",
                    json!({"reviewer":"<blocked-reviewer-name>","reviewer_kind":kind,"action":action,"comment":"<that principal's fresh explicit decision>","blocking":false}),
                );
                request["body"]["actor_key_index"] =
                    json!("<authorized-request-Actor-key-index>");
                json!({"lane":lane,"request":request})
            })
            .collect::<Vec<_>>());
        } else {
            let human_required = detail.contains("PACKAGE_HUMAN_REVIEW_REQUIRED");
            let payload = if human_required {
                json!({"reviewer":"<authorized-human-reviewer>","reviewer_kind":"user","action":"approve","comment":"<that reviewer's explicit decision>","blocking":false})
            } else {
                json!({"reviewer":actor.name,"reviewer_kind":crate::reviewer_identity::ActingReviewer::ACTOR_KIND,"action":if detail.contains("PACKAGE_TASK_APPROVAL_REQUIRED") {"task_approve"}else{"code_review_summary"},"comment":message,"blocking":false})
            };
            result["next_action"]["request"] = review_api_action(
                identity,
                if human_required {
                    None
                } else {
                    client.require_actor_key_index().ok()
                },
                "<new-caller-stable-key-for-this-decision>",
                payload,
            );
        }
    }
    Ok(result)
}

pub fn workflow_package_abandon(
    repo: &RepoRuntime,
    remote_name: &str,
    identity: &PackageWorkflowExpectedIdentity,
) -> Result<JsonValue, String> {
    use ait_core::workflow_package::{PackageAbandonment, PackageAbandonmentState};
    identity.validate()?;
    if remote_name.trim().is_empty() {
        return Err("Package abandonment requires an explicit nonempty remote name".into());
    }
    if repo.require_repository_index()?.get() != identity.repository_index {
        return Err("Package identity belongs to another Repository".into());
    }
    let (remote, repo_name) = remote_context(repo, Some(remote_name), None)?;
    let mut client = package_client(repo, &remote, &repo_name)?;
    match client.abandon_workflow_package(identity) {
        Ok(response) => {
            let result = PackageAbandonment::from_value(&response, identity)?;
            Ok(
                json!({"contract":"ait-agent-action/v1","expected_identity":identity,"state":result.state,"complete":result.state==PackageAbandonmentState::Abandoned}),
            )
        }
        Err(error) => Ok(
            json!({"contract":"ait-agent-action/v1","expected_identity":identity,"state":"unknown","complete":false,"detail":format!("Abandonment was not confirmed: {error}. Keep the same identity for explicit recovery; no different Job was selected.")}),
        ),
    }
}
