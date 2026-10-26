use super::*;

#[derive(Debug, Clone)]
pub(in crate::primitives) struct PatchsetPublishRemoteContext {
    pub(in crate::primitives) resolved_change_id: String,
    pub(in crate::primitives) resolved_change_ref: String,
    pub(in crate::primitives) change_task_id: Option<String>,
    pub(in crate::primitives) base_line: String,
    pub(in crate::primitives) base_snapshot_id: String,
}

const PUBLIC_PATCHSET_CI_RECENT_LIMIT: i64 = 10;
const PUBLIC_PATCHSET_CI_RERUN_TRIGGER: &str = "manual_rerun";

fn publication_task_preflight<R: TaskWorkflowRemoteTaskReader + ?Sized>(
    remote: &mut R,
    task_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String> {
    let task = remote
        .get_task(task_id, Some(repo_name))
        .map_err(|error| error.to_string())?;
    if task["status"].as_str() != Some("active")
        || ait_core::land_json::LandJson::stateless()
            .task_successful_receipt(&task, task_id, None)?
            .is_some()
    {
        return Err(
            "Patchset publication requires the exact active Task without an accepted receipt"
                .into(),
        );
    }
    match task.get("current_patchset_id") {
        Some(JsonValue::Null)=>{},
        Some(JsonValue::String(id)) if !id.is_empty() && id.trim()==id=>{},
        _=>return Err("Task publication requires nullable current_patchset_id; update the server before publication".into()),
    }
    Ok(task)
}

pub(super) fn exact_patchset_id(value: &str) -> Result<String, String> {
    let patchset_id =
        normalized_text(Some(value)).ok_or_else(|| "Patchset ID must be non-empty.".to_string())?;
    if patchset_id.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("Public Patchset reference required; use TASK_ID/P-##.".to_string());
    }
    Ok(patchset_id)
}

pub(in crate::primitives) fn patchset_publish_remote_context_with_task_remote<R>(
    task_remote: &mut R,
    change_id: &str,
    repo_name: &str,
    resolve_change_task_id: bool,
) -> Result<PatchsetPublishRemoteContext, String>
where
    R: TaskWorkflowRemoteChangeReader
        + TaskWorkflowRemoteTaskReader
        + TaskWorkflowRemoteChangeDetailReader
        + TaskWorkflowRemoteChangeLister
        + TaskWorkflowLineReader
        + ?Sized,
{
    if is_task_id(change_id) {
        let task = publication_task_preflight(task_remote, change_id, repo_name)?;
        let base_line = string_field(&task, "base_line").unwrap_or_else(|| "main".into());
        let base_snapshot_id =
            change_base_line_head_with_task_remote(task_remote, repo_name, &base_line)?;
        return Ok(PatchsetPublishRemoteContext {
            resolved_change_id: change_id.into(),
            resolved_change_ref: change_id.into(),
            change_task_id: Some(change_id.into()),
            base_line,
            base_snapshot_id,
        });
    }
    let change = change_show_with_task_remote(task_remote, change_id, repo_name)?;
    let resolved_change_id =
        string_field(&change, "change_id").unwrap_or_else(|| change_id.to_string());
    let resolved_change_ref = change_reference_from_payload(&change, Some(change_id))?;
    if let Some(task_id) = string_field(&change, "task_id").filter(|task| is_task_id(task)) {
        publication_task_preflight(task_remote, &task_id, repo_name)?;
    }
    let change_task_id = if resolve_change_task_id {
        remote_change_task_id(
            task_remote,
            repo_name,
            &change,
            change_id,
            &resolved_change_id,
        )?
    } else {
        None
    };
    let base_line = required_string_field(&change, "base_line")?;
    let base_snapshot_id =
        change_base_line_head_with_task_remote(task_remote, repo_name, &base_line)?;
    Ok(PatchsetPublishRemoteContext {
        resolved_change_id,
        resolved_change_ref,
        change_task_id,
        base_line,
        base_snapshot_id,
    })
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn patchset_publish_flow_with_task_and_closeout_remotes<T, C>(
    repo: &RepoRuntime,
    task_remote: &mut T,
    closeout_remote: &mut C,
    remote_name: &str,
    repo_name: &str,
    change_id: &str,
    summary: &str,
    author_mode: &str,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowRemoteChangeReader
        + TaskWorkflowRemoteTaskReader
        + TaskWorkflowRemoteChangeDetailReader
        + TaskWorkflowRemoteChangeLister
        + TaskWorkflowLineReader
        + TaskWorkflowLineHeadUpdater
        + TaskWorkflowRepositoryReader
        + TaskWorkflowSnapshotExistenceReader
        + TaskWorkflowZstdPackUploader
        + ?Sized,
    C: TaskWorkflowPatchsetPublisher + ?Sized,
{
    let _range = perfetto_range!("ait.workflow_ready.publish.remote_flow.detail");
    guard_capture_publication_target_name(repo, remote_name)?;
    let (line_name, revision_snapshot_id) = {
        let _range = perfetto_range!("ait.workflow_ready.publish.local_context");
        guard_no_planning_only_artifact_drift(repo, "ait patchset publish")?;
        let line_name = repo.current_line_name()?;
        let line_row = local_line_row(repo, &line_name)?;
        let revision_snapshot_id = required_string_field(&line_row, "head_snapshot_id")?;
        (line_name, revision_snapshot_id)
    };
    let mut publish_context = {
        let _range = perfetto_range!("ait.workflow_ready.publish.remote_context");
        patchset_publish_remote_context_with_task_remote(
            task_remote,
            change_id,
            repo_name,
            repo_root_has_bound_worktree_metadata(repo)?,
        )?
    };
    if is_task_id(change_id) {
        let owners = resolved_remote_snapshot_ownership_rows(
            repo,
            std::slice::from_ref(&revision_snapshot_id),
            Some(remote_name),
        )?;
        let owners = owners
            .iter()
            .filter(|owner| owner["snapshot_id"].as_str() == Some(&revision_snapshot_id))
            .collect::<Vec<_>>();
        let [owner] = owners.as_slice() else {
            return Err("Publication requires unique numeric capture ownership".into());
        };
        if string_field(owner, "task_id").as_deref() != Some(change_id)
            || owner["ownership_source"].as_str() != Some("durable_remote_snapshot_binding")
        {
            return Err("Publication Snapshot is not captured by this exact Task".into());
        }
        if let Some(child) = string_field(owner, "change_id") {
            publish_context.resolved_change_ref =
                change_reference_for_context(Some(change_id), &child)?;
            publish_context.resolved_change_id = child;
        }
    }
    {
        let _range = perfetto_range!("ait.workflow_ready.publish.scope_guards");
        {
            let _range = perfetto_range!("ait.workflow_ready.publish.worktree_guard");
            guard_repo_root_bound_task_worktree(
                repo,
                publish_context.change_task_id.as_deref(),
                (!is_task_id(&publish_context.resolved_change_id))
                    .then_some(publish_context.resolved_change_id.as_str()),
                "ait patchset publish",
            )?;
        }
        guard_patchset_worktree_retarget(
            repo,
            &publish_context.base_line,
            &publish_context.base_snapshot_id,
            &revision_snapshot_id,
        )?;
        guard_patchset_revision_scope(
            repo,
            &publish_context.base_snapshot_id,
            &revision_snapshot_id,
            &publish_context.resolved_change_ref,
            &line_name,
            remote_name,
        )?;
        ensure_patchset_not_empty(
            &publish_context.resolved_change_ref,
            &publish_context.base_snapshot_id,
            &revision_snapshot_id,
        )?;
    }
    let snapshot_sync = {
        let _range = perfetto_range!("ait.workflow_ready.publish.snapshot_sync");
        super::remote_sync::sync_patchset_revision_snapshot_with_task_remote(
            repo,
            task_remote,
            remote_name,
            repo_name,
            &line_name,
            &revision_snapshot_id,
            &publish_context.base_line,
        )?
    };
    let _range = perfetto_range!("ait.workflow_ready.publish.patchset_http");
    patchset_publish_payload_with_closeout_remote(
        closeout_remote,
        &publish_context.resolved_change_ref,
        &publish_context.base_snapshot_id,
        &revision_snapshot_id,
        summary,
        author_mode,
        repo_name,
        Some(line_name),
        Some(snapshot_sync),
    )
}

pub fn patchset_publish(
    repo: &RepoRuntime,
    change_id: &str,
    summary: &str,
    author_mode: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    guard_no_planning_only_artifact_drift(repo, "ait patchset publish")?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    guard_capture_publication_target(repo, &remote_row)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    let resolved_author_mode = repo.effective_author_mode(author_mode);
    patchset_publish_flow_with_task_and_closeout_remotes(
        repo,
        &mut task_remote,
        &mut closeout_remote,
        &remote_row.name,
        &repo_name,
        change_id,
        summary,
        &resolved_author_mode,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "arguments mirror the patchset publication contract"
)]
pub fn patchset_publish_explicit(
    repo: &RepoRuntime,
    change_id: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    summary: &str,
    author_mode: Option<&str>,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    guard_no_planning_only_artifact_drift(repo, "ait patchset publish")?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    guard_capture_publication_target(repo, &remote_row)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    let resolved_author_mode = repo.effective_author_mode(author_mode);
    patchset_publish_payload_with_closeout_remote(
        &mut closeout_remote,
        change_id,
        base_snapshot_id,
        revision_snapshot_id,
        summary,
        &resolved_author_mode,
        &repo_name,
        None,
        None,
    )
}

pub(in crate::primitives) fn patchset_publish_with_closeout_remote<R>(
    closeout_remote: &mut R,
    change_id: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    summary: &str,
    author_mode: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetPublisher + ?Sized,
{
    ensure_patchset_not_empty(change_id, base_snapshot_id, revision_snapshot_id)?;
    closeout_remote
        .publish_patchset(
            change_id,
            base_snapshot_id,
            revision_snapshot_id,
            summary,
            author_mode,
            Some(repo_name),
            true,
        )
        .map_err(|err| err.to_string())
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn patchset_publish_payload_with_closeout_remote<R>(
    closeout_remote: &mut R,
    change_id: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    summary: &str,
    author_mode: &str,
    repo_name: &str,
    current_line: Option<String>,
    snapshot_sync: Option<JsonValue>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetPublisher + ?Sized,
{
    let patchset = patchset_publish_with_closeout_remote(
        closeout_remote,
        change_id,
        base_snapshot_id,
        revision_snapshot_id,
        summary,
        author_mode,
        repo_name,
    )?;
    Ok(patchset_publish_payload(
        patchset,
        current_line,
        change_id,
        base_snapshot_id,
        revision_snapshot_id,
        author_mode,
        snapshot_sync,
    ))
}

fn patchset_publish_payload(
    patchset: JsonValue,
    current_line: Option<String>,
    change_id: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    author_mode: &str,
    snapshot_sync: Option<JsonValue>,
) -> JsonValue {
    let mut payload = patchset.as_object().cloned().unwrap_or_default();
    let canonical_change_id =
        canonical_change_id(change_id).unwrap_or_else(|_| change_id.to_string());
    payload.insert(
        "change_id".to_string(),
        if is_task_id(change_id) {
            JsonValue::Null
        } else {
            JsonValue::String(canonical_change_id.clone())
        },
    );
    if change_id != canonical_change_id && !payload.contains_key("change_ref") {
        payload.insert(
            "change_ref".to_string(),
            JsonValue::String(change_id.to_string()),
        );
    }
    if let Some(value) = current_line {
        payload.insert("current_line".to_string(), JsonValue::String(value));
    }
    payload.insert(
        "base_snapshot_id".to_string(),
        JsonValue::String(base_snapshot_id.to_string()),
    );
    payload.insert(
        "revision_snapshot_id".to_string(),
        JsonValue::String(revision_snapshot_id.to_string()),
    );
    payload.insert(
        "author_mode".to_string(),
        JsonValue::String(author_mode.to_string()),
    );
    if let Some(value) = snapshot_sync {
        payload.insert("snapshot_sync".to_string(), value);
    }
    payload.insert("patchset".to_string(), patchset);
    JsonValue::Object(payload)
}

pub fn patchset_list(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    patchset_list_with_closeout_remote(&mut closeout_remote, change_id, Some(&repo_name))
}

pub(in crate::primitives) fn patchset_list_with_closeout_remote<R>(
    closeout_remote: &mut R,
    change_id: &str,
    repo_name: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetLister + ?Sized,
{
    let rows = closeout_remote
        .list_patchsets(change_id, repo_name)
        .map_err(|err| err.to_string())?;
    Ok(JsonValue::Array(rows))
}

pub fn patchset_show(
    repo: &RepoRuntime,
    patchset_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let patchset_id = exact_patchset_id(patchset_id)?;
    let patchset_id = resolve_public_patchset_input(repo, &patchset_id, remote_name)?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    patchset_show_with_closeout_remote(&mut closeout_remote, &patchset_id, Some(&repo_name), None)
}

pub(in crate::primitives) fn patchset_show_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    repo_name: Option<&str>,
    change_ref: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + ?Sized,
{
    closeout_remote
        .get_patchset(patchset_id, repo_name, change_ref)
        .map_err(|err| err.to_string())
}

pub fn patchset_select(
    repo: &RepoRuntime,
    patchset_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let patchset_id = exact_patchset_id(patchset_id)?;
    let patchset_id = resolve_public_patchset_input(repo, &patchset_id, remote_name)?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    patchset_select_by_id_with_closeout_remote(&mut closeout_remote, &patchset_id, &repo_name)
}

pub(in crate::primitives) fn patchset_select_by_id_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowPatchsetSelector + ?Sized,
{
    let patchset =
        patchset_show_with_closeout_remote(closeout_remote, patchset_id, Some(repo_name), None)?;
    let resolved_patchset_id =
        string_field(&patchset, "patchset_id").unwrap_or_else(|| patchset_id.to_string());
    let owning_change_ref = required_string_field(&patchset, "task_id")?;
    patchset_select_with_closeout_remote(
        closeout_remote,
        &owning_change_ref,
        &resolved_patchset_id,
        repo_name,
    )
}

pub(in crate::primitives) fn patchset_select_with_closeout_remote<R>(
    closeout_remote: &mut R,
    change_id: &str,
    patchset_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetSelector + ?Sized,
{
    closeout_remote
        .select_patchset(change_id, patchset_id, Some(repo_name), false)
        .map_err(|err| err.to_string())
}

pub fn patchset_ci_status(
    repo: &RepoRuntime,
    patchset_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let patchset_id = exact_patchset_id(patchset_id)?;
    let patchset_id = resolve_public_patchset_input(repo, &patchset_id, remote_name)?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    patchset_ci_status_with_closeout_remote(
        &mut closeout_remote,
        &patchset_id,
        &repo_name,
        PUBLIC_PATCHSET_CI_RECENT_LIMIT,
    )
}

pub(in crate::primitives) fn patchset_ci_status_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    repo_name: &str,
    recent_limit: i64,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowPatchsetCiStatusReader + ?Sized,
{
    let patchset = get_patchset_for_ci_status(closeout_remote, patchset_id, repo_name)?;
    let resolved_patchset_id =
        string_field(&patchset, "patchset_id").unwrap_or_else(|| patchset_id.to_string());
    closeout_remote
        .read_patchset_ci_status(&resolved_patchset_id, recent_limit, None, true)
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn patchset_ci_readiness_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    repo_name: &str,
    recent_limit: i64,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowPatchsetCiStatusReader + ?Sized,
{
    let patchset = get_patchset_for_ci_status(closeout_remote, patchset_id, repo_name)?;
    let resolved_patchset_id =
        string_field(&patchset, "patchset_id").unwrap_or_else(|| patchset_id.to_string());
    let payload = closeout_remote
        .read_patchset_ci_readiness(&resolved_patchset_id, recent_limit, None, true)
        .map_err(|err| err.to_string())?;
    validate_patchset_ci_readiness_payload(&payload, &resolved_patchset_id, recent_limit)?;
    if string_field(&patchset, "task_id").is_none()
        || string_field(&patchset, "task_id") != string_field(&payload, "task_id")
        || string_field(&patchset, "change_ref") != string_field(&payload, "change_ref")
        || patchset.get("change_id") != payload.get("change_id")
    {
        return Err("Patchset CI readiness owner differs from the exact Patchset.".into());
    }
    Ok(payload)
}

pub(in crate::primitives) fn get_patchset_for_ci_status<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + ?Sized,
{
    patchset_ci_status_patchset_read_with_closeout_remote(closeout_remote, patchset_id, repo_name)
}

pub(in crate::primitives) fn patchset_ci_status_patchset_read_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + ?Sized,
{
    match closeout_remote.get_patchset(patchset_id, Some(repo_name), None) {
        Ok(patchset) => Ok(patchset),
        Err(original_err) => {
            let Some(alias) = legacy_repo_patchset_alias(patchset_id) else {
                return Err(original_err.to_string());
            };
            closeout_remote
                .get_patchset(&alias, Some(repo_name), None)
                .map_err(|_| original_err.to_string())
        }
    }
}

pub(in crate::primitives) fn legacy_repo_patchset_alias(patchset_id: &str) -> Option<String> {
    patchset_id
        .strip_prefix("P-RCC-")
        .map(|suffix| format!("RCP-{suffix}"))
        .or_else(|| {
            patchset_id
                .strip_prefix("P-LCC-")
                .map(|suffix| format!("LCP-{suffix}"))
        })
}

fn validate_patchset_ci_readiness_payload(
    payload: &JsonValue,
    patchset_id: &str,
    recent_limit: i64,
) -> Result<(), String> {
    let object = payload.as_object().ok_or_else(|| {
        "Patchset CI readiness response must be a JSON object; refusing repository-job fallback."
            .to_string()
    })?;
    let required_text = |key: &str| {
        object
            .get(key)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!(
                    "Patchset CI readiness response is missing non-empty {key}; refusing repository-job fallback."
                )
            })
    };
    if required_text("contract")? != "ait.server.patchset_ci.readiness.v2" {
        return Err(
            "Patchset CI readiness response has an unsupported contract; refusing repository-job fallback."
                .to_string(),
        );
    }
    if required_text("projection")? != "readiness" {
        return Err(
            "Patchset CI returned an unexpected response type; repository-job fallback was not started."
                .to_string(),
        );
    }
    if required_text("patchset_id")? != patchset_id {
        return Err(format!(
            "Patchset CI readiness response is for a different patchset; expected {patchset_id}."
        ));
    }
    let task_id = required_text("task_id")?;
    let change_ref = match object.get("change_ref") {
        Some(JsonValue::Null) => None,
        Some(JsonValue::String(text)) if !text.is_empty() && text.trim() == text => {
            Some(text.as_str())
        }
        _ => return Err("Patchset CI readiness Change must be exact text or null.".into()),
    };
    let change_id = match object.get("change_id") {
        Some(JsonValue::Null) => None,
        Some(JsonValue::String(text)) if !text.is_empty() && text.trim() == text => {
            Some(text.as_str())
        }
        _ => return Err("Patchset CI readiness requires nullable exact change_id.".into()),
    };
    if change_ref.and_then(|reference| reference.rsplit_once('/').map(|(_, child)| child))
        != change_id
    {
        return Err("Patchset CI readiness optional Change fields disagree.".into());
    }
    if patchset_id.contains("/P-") {
        ait_core::workflow_expected_identity::validate_patchset_reference_owner(
            patchset_id,
            task_id,
            change_ref,
        )?;
    }
    if let Some(change_ref) = change_ref {
        if change_ref.rsplit_once('/').map(|(owner, _)| owner) != Some(task_id) {
            return Err("Patchset CI readiness Change belongs to another Task.".into());
        }
    }
    required_text("repo_name")?;
    required_text("tests_status")?;
    for key in ["available", "has_runnable_evidence"] {
        if object.get(key).and_then(JsonValue::as_bool).is_none() {
            return Err(format!(
                "Patchset CI readiness response is missing boolean {key}; refusing repository-job fallback."
            ));
        }
    }
    if !object
        .get("selected_suite_ids")
        .is_some_and(JsonValue::is_array)
    {
        return Err(
            "Patchset CI readiness response is missing selected_suite_ids array; refusing repository-job fallback."
                .to_string(),
        );
    }
    for key in ["suite_result_count", "blocking_failure_count"] {
        if object
            .get(key)
            .and_then(JsonValue::as_i64)
            .is_none_or(|value| value < 0)
        {
            return Err(format!(
                "Patchset CI readiness response is missing non-negative integer {key}; refusing repository-job fallback."
            ));
        }
    }
    let expected_recent_limit = recent_limit.clamp(1, 20);
    if object
        .get("recent_limit_applied")
        .and_then(JsonValue::as_i64)
        != Some(expected_recent_limit)
    {
        return Err(format!(
            "Patchset CI readiness response did not apply bounded recent_limit {expected_recent_limit}; refusing repository-job fallback."
        ));
    }
    if !object
        .get("latest_job")
        .is_some_and(|value| value.is_null() || value.is_object())
    {
        return Err(
            "Patchset CI readiness response has malformed latest_job; refusing repository-job fallback."
                .to_string(),
        );
    }
    if !object.get("recent_jobs").is_some_and(JsonValue::is_array) {
        return Err(
            "Patchset CI readiness response is missing recent_jobs array; refusing repository-job fallback."
                .to_string(),
        );
    }
    Ok(())
}

pub fn patchset_rerun_ci(
    repo: &RepoRuntime,
    patchset_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let patchset_id = exact_patchset_id(patchset_id)?;
    let patchset_id = resolve_public_patchset_input(repo, &patchset_id, remote_name)?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    patchset_run_ci_with_closeout_remote(
        &mut closeout_remote,
        &patchset_id,
        PUBLIC_PATCHSET_CI_RERUN_TRIGGER,
        None,
        &repo_name,
    )
}

pub(in crate::primitives) fn patchset_run_ci_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset_id: &str,
    trigger: &str,
    execution_profile: Option<&str>,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetCiRunner + ?Sized,
{
    closeout_remote
        .run_patchset_ci(
            patchset_id,
            trigger,
            execution_profile,
            Some(repo_name),
            false,
        )
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod patchset_reference_tests {
    use super::*;

    #[test]
    fn exact_reference_guard_rejects_numeric_ordinals() {
        let error = exact_patchset_id(" 12 ").expect_err("numeric Patchset ref must be rejected");
        assert!(error.contains("TASK_ID/P-##"));
        assert_eq!(
            exact_patchset_id(" RCT-1/P-02 ").expect("public Patchset ID"),
            "RCT-1/P-02"
        );
    }
}
