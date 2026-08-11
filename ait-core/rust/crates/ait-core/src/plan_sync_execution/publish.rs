use super::*;

type ExistingRemotePlanHistoryMatch = (JsonValue, Vec<(String, String)>, Vec<JsonValue>);
type CreatedRemotePlanFromLocalSeed = (
    JsonValue,
    String,
    (String, String),
    Option<AtomicTaskStartOutcome>,
);

#[derive(Clone, Debug)]
struct AtomicTaskStartOutcome {
    response: JsonValue,
    remote_plan: JsonValue,
    elapsed_ms: f64,
}

#[expect(
    clippy::too_many_arguments,
    reason = "publication orchestration keeps independently substitutable ports explicit"
)]
pub(super) fn publish_synced_local_results<L, B, A, F, I, C>(
    request: &SyncRequest,
    results: &[JsonValue],
    local_source: &L,
    local_blob_store: &B,
    local_artifact_body_source: &A,
    file_io_store: &F,
    identity_source: &I,
    client: &mut C,
    divergent_retry_mode: Option<&str>,
) -> Result<Vec<JsonValue>, String>
where
    L: PlanSyncLocalPublishSource + ?Sized,
    B: PlanSyncLocalBlobStore + PlanSyncZstdPackStore + ?Sized,
    A: PlanSyncLocalArtifactBodySource + ?Sized,
    F: FileIoStore + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
    C: PlanSyncRemotePublisher + ?Sized,
{
    let mut published = Vec::new();
    let mut seen = BTreeSet::new();
    for row in results {
        let Some(plan_id) = optional_text(value_get(row, "plan_id"))? else {
            continue;
        };
        if !seen.insert(plan_id.clone()) {
            continue;
        }
        let local_plan = get_plan_with_plan_sync_local_store(local_source, &plan_id)?;
        if text_field(row, "action").as_deref() == Some("unchanged")
            && local_plan_fully_published(&local_plan).map_err(|err| err.to_string())?
            && divergent_retry_mode.is_none()
            && request.task_start.is_none()
        {
            continue;
        }
        let publish_result = local_plan_publish(
            request,
            local_source,
            local_blob_store,
            local_artifact_body_source,
            file_io_store,
            identity_source,
            client,
            &plan_id,
            divergent_retry_mode,
        )?;
        if request.task_start.is_none()
            && publish_result
                .get("publish_action")
                .and_then(|value| value.as_str())
                == Some("noop")
        {
            continue;
        }
        published.push(publish_result);
    }
    if request.task_start.is_some() {
        let task_start_results = published
            .iter()
            .filter(|row| value_get(row, "task_start").is_some_and(|value| !value.is_null()))
            .count();
        if task_start_results != 1 {
            return Err(format!(
                "Plan sync task_start expected exactly one composite remote mutation, observed {task_start_results}."
            ));
        }
    }
    Ok(published)
}

pub(super) fn publish_paired_artifacts<L, C>(
    results: &[JsonValue],
    paired_artifacts_by_markdown_path: &BTreeMap<String, Vec<JsonValue>>,
    local_source: &L,
    client: &mut C,
) -> Result<Vec<JsonValue>, String>
where
    L: PlanSyncLocalPlanStore + ?Sized,
    C: PlanSyncRemoteRevisionArtifactWriter + ?Sized,
{
    if paired_artifacts_by_markdown_path.is_empty() {
        return Ok(Vec::new());
    }
    let mut uploads = Vec::new();
    for row in results {
        let Some(markdown_artifact_path) = optional_text(value_get(row, "artifact_path"))? else {
            continue;
        };
        let Some(plan_id) = optional_text(value_get(row, "plan_id"))? else {
            continue;
        };
        let artifacts = paired_artifacts_by_markdown_path
            .get(markdown_artifact_path.as_str())
            .cloned()
            .unwrap_or_default();
        if artifacts.is_empty() {
            continue;
        }
        let local_plan = get_plan_with_plan_sync_local_store(local_source, &plan_id)?;
        let remote_plan_id = selected_remote_plan_id(&local_plan, &plan_id)?;
        let remote_revision_id =
            optional_text(value_get(&local_plan, "published_head_revision_id"))?
                .or_else(|| head_text(&local_plan, "plan_revision_id"))
                .ok_or_else(|| {
                    format!(
                        "Remote plan {plan_id} has no head revision for paired artifact upload."
                    )
                })?;
        let mut request_artifacts = Vec::new();
        for artifact in &artifacts {
            match optional_text(value_get(artifact, "role"))?.as_deref() {
                Some("public_package_targets_contract_json") => {
                    validate_public_package_targets_contract_artifact_for_revision(
                        artifact,
                        &markdown_artifact_path,
                    )?
                }
                Some("public_future_repo_extraction_prep_contract_json") => {
                    validate_public_future_repo_extraction_prep_contract_artifact_for_revision(
                        artifact,
                        &markdown_artifact_path,
                    )?
                }
                Some("public_future_repo_split_dry_run_contract_json") => {
                    validate_public_future_repo_split_dry_run_contract_artifact_for_revision(
                        artifact,
                        &markdown_artifact_path,
                    )?
                }
                other => {
                    return Err(format!(
                        "Unsupported plan sync paired artifact role: {:?}.",
                        other
                    ))
                }
            }
            request_artifacts.push(JsonValue::Object(JsonMap::from_iter([
                (
                    "artifact_path".to_string(),
                    value_get(artifact, "artifact_path")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "role".to_string(),
                    value_get(artifact, "role")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "media_type".to_string(),
                    value_get(artifact, "media_type")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "encoding".to_string(),
                    value_get(artifact, "encoding")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "body".to_string(),
                    value_get(artifact, "body")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "metadata".to_string(),
                    value_get(artifact, "metadata")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
            ])));
        }
        let uploaded = put_plan_revision_artifacts_with_plan_sync_remote_client(
            client,
            &remote_plan_id,
            &remote_revision_id,
            &request_artifacts,
        )
        .map_err(|err| err.to_string())?;
        for uploaded_artifact in as_array(value_get(&uploaded, "artifacts"))? {
            uploads.push(JsonValue::Object(JsonMap::from_iter([
                ("plan_id".to_string(), JsonValue::String(plan_id.clone())),
                (
                    "plan_revision_id".to_string(),
                    JsonValue::String(remote_revision_id.clone()),
                ),
                (
                    "source_artifact_path".to_string(),
                    JsonValue::String(markdown_artifact_path.clone()),
                ),
                (
                    "artifact_path".to_string(),
                    value_get(uploaded_artifact, "artifact_path")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "role".to_string(),
                    value_get(uploaded_artifact, "role")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "blob_id".to_string(),
                    value_get(uploaded_artifact, "blob_id")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "sha256".to_string(),
                    value_get(uploaded_artifact, "sha256")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "byte_count".to_string(),
                    value_get(uploaded_artifact, "byte_count")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
            ])));
        }
    }
    Ok(uploads)
}

pub(super) fn selected_remote_plan_id(
    local_plan: &JsonValue,
    local_plan_id: &str,
) -> Result<String, String> {
    let observed_local_plan_id = require_plan_id(local_plan)?;
    if observed_local_plan_id != local_plan_id {
        return Err(format!(
            "Local Plan lookup requested {local_plan_id}, but the local store returned {observed_local_plan_id}."
        ));
    }
    Ok(optional_text(value_get(local_plan, "published_plan_id"))?
        .unwrap_or_else(|| local_plan_id.to_string()))
}

fn ensure_missing_remote_create_is_unbound(
    local_plan_id: &str,
    remote_plan_id: &str,
    has_published_remote_plan_receipt: bool,
    lookup_error: &str,
) -> Result<(), String> {
    if !has_published_remote_plan_receipt {
        return Ok(());
    }
    let local_plan_id = LocalPlanId::from_raw(local_plan_id)?;
    let remote_plan_id = RemotePlanId::from_raw(remote_plan_id)?;
    Err(format!(
        "Receipt-bound local Plan {} targets missing remote Plan {}; refusing to create or retarget a replacement: {lookup_error}",
        local_plan_id.reference(),
        remote_plan_id.reference()
    ))
}

pub(super) fn require_selected_remote_plan_identity(
    selected_remote_plan_id: &str,
    remote_plan: &JsonValue,
) -> Result<(), String> {
    let remote_plan_id = text_field(remote_plan, "plan_id");
    if remote_plan_id.as_deref() == Some(selected_remote_plan_id) {
        return Ok(());
    }
    Err(format!(
        "Remote repository returned Plan identity {remote_plan_id:?} for selected remote publication target {selected_remote_plan_id}; refusing to read or mutate a different Plan."
    ))
}

pub(super) fn read_exact_local_plan_revision_artifact_body<B, A>(
    local_blob_store: &B,
    local_artifact_body_source: &A,
    request: &SyncRequest,
    revision: &JsonValue,
) -> Result<Option<String>, String>
where
    B: PlanSyncLocalBlobStore + ?Sized,
    A: PlanSyncLocalArtifactBodySource + ?Sized,
{
    let Some(expected_blob_id) = text_field(revision, "artifact_blob_id") else {
        return Ok(
            read_plan_revision_artifact_body_with_plan_sync_local_artifact_body_source(
                local_artifact_body_source,
                &request.root_path,
                revision,
            ),
        );
    };
    match read_blob_bytes_with_plan_sync_local_blob_store(local_blob_store, &expected_blob_id) {
        Ok(bytes) => {
            let body = String::from_utf8(bytes).map_err(|_| {
                format!(
                    "Plan revision {:?} artifact blob {expected_blob_id} is not valid UTF-8 Markdown.",
                    text_field(revision, "plan_revision_id")
                )
            })?;
            let actual_blob_id = artifact_blob_id(&body);
            if actual_blob_id != expected_blob_id {
                return Err(format!(
                    "Plan revision {:?} read artifact blob {expected_blob_id}, but its bytes resolve to {actual_blob_id}.",
                    text_field(revision, "plan_revision_id")
                ));
            }
            Ok(Some(body))
        }
        Err(blob_error) => {
            let current_body =
                read_plan_revision_artifact_body_with_plan_sync_local_artifact_body_source(
                    local_artifact_body_source,
                    &request.root_path,
                    revision,
                );
            let Some(current_body) = current_body else {
                return Err(format!(
                    "Plan revision {:?} artifact blob {expected_blob_id} is unavailable locally: {blob_error}",
                    text_field(revision, "plan_revision_id")
                ));
            };
            let current_blob_id = artifact_blob_id(&current_body);
            if current_blob_id != expected_blob_id {
                return Err(format!(
                    "Plan revision {:?} artifact blob {expected_blob_id} is unavailable locally ({blob_error}), and the current Markdown bytes resolve to {current_blob_id}.",
                    text_field(revision, "plan_revision_id")
                ));
            }
            Ok(Some(current_body))
        }
    }
}

fn remote_plan_lookup_is_absent(error: &str, plan_id: &str) -> bool {
    if error.contains("out of bounds for file 'plan.bin'") {
        return true;
    }
    let Some((_, missing_plan_id)) = error.rsplit_once(" failed: 404 Unknown plan: ") else {
        return false;
    };
    missing_plan_id == plan_id
}

fn missing_published_remote_replay_is_allowed(reconcile: bool, error: &str, plan_id: &str) -> bool {
    reconcile && remote_plan_lookup_is_absent(error, plan_id)
}

fn missing_remote_replay_reconcile_details(
    local_plan: &JsonValue,
    remote_plan: &JsonValue,
) -> Result<JsonValue, String> {
    let local_head_revision_id = local_head_revision_id(local_plan)
        .ok_or_else(|| "Missing-remote Plan replay has no local head revision.".to_string())?;
    let remote_head_revision_id = remote_head_revision_id(remote_plan)
        .ok_or_else(|| "Missing-remote Plan replay has no remote head revision.".to_string())?;
    Ok(json!({
        "mode": "missing_remote_replay",
        "local_head_revision_id": local_head_revision_id,
        "remote_head_revision_id": remote_head_revision_id,
    }))
}

fn match_existing_remote_plan_history<B, C>(
    local_blob_store: &B,
    plan_id: &str,
    local_revisions: &[JsonValue],
    remote_plan: JsonValue,
    client: &mut C,
    remote_revision_detail_cache: &mut BTreeMap<(String, String), JsonValue>,
) -> Result<Option<ExistingRemotePlanHistoryMatch>, String>
where
    B: PlanSyncLocalBlobStore + ?Sized,
    C: PlanSyncRemoteRevisionLister + PlanSyncRemoteRevisionReader + ?Sized,
{
    require_selected_remote_plan_identity(plan_id, &remote_plan)?;
    let remote_revisions = sort_revisions_ascending(
        list_plan_revisions_with_plan_sync_remote_client(client, plan_id)
            .map_err(|error| error.to_string())?,
    );
    if remote_revisions.is_empty() {
        return Err(format!(
            "Remote plan {plan_id} exists without a revision; dense Plan migration is incomplete."
        ));
    }
    let mut mappings = Vec::with_capacity(remote_revisions.len());
    for (revision_offset, (local_revision, remote_revision)) in
        local_revisions.iter().zip(&remote_revisions).enumerate()
    {
        let comparable_remote_revision = matching_remote_publish_revision(
            local_blob_store,
            plan_id,
            local_revision,
            remote_revision,
            client,
            remote_revision_detail_cache,
        )?;
        let Some(comparable_remote_revision) = comparable_remote_revision else {
            if revision_offset == 0 {
                return Ok(None);
            }
            return Err(format!(
                "Remote plan {plan_id} diverges after {revision_offset} equivalent revision(s); refusing to create a duplicate remote lineage."
            ));
        };
        let remote_revision_id = text_field(&comparable_remote_revision, "plan_revision_id")
            .ok_or_else(|| {
                format!("Remote plan {plan_id} revision is missing plan_revision_id.")
            })?;
        mappings.push((
            require_plan_revision_id(local_revision)?,
            remote_revision_id,
        ));
    }
    if remote_revisions.len() > local_revisions.len() {
        return Err(format!(
            "Remote plan {plan_id} has {} revisions, but local plan.bin lineage has only {}; refusing to discard remote history.",
            remote_revisions.len(),
            local_revisions.len()
        ));
    }
    let remaining = local_revisions[remote_revisions.len()..].to_vec();
    Ok(Some((remote_plan, mappings, remaining)))
}

#[expect(
    clippy::too_many_arguments,
    reason = "remote seed creation coordinates explicit storage and transport boundaries"
)]
fn create_remote_plan_from_local_seed<L, B, A, F, C>(
    request: &SyncRequest,
    local_source: &L,
    local_blob_store: &B,
    local_artifact_body_source: &A,
    file_io_store: &F,
    client: &mut C,
    remote_repo_name: &str,
    local_plan: &JsonValue,
    local_revisions: &[JsonValue],
    local_plan_id: &str,
) -> Result<CreatedRemotePlanFromLocalSeed, String>
where
    L: PlanSyncLocalPublishSource + ?Sized,
    B: PlanSyncLocalBlobStore + PlanSyncZstdPackStore + ?Sized,
    A: PlanSyncLocalArtifactBodySource + ?Sized,
    F: FileIoStore + ?Sized,
    C: PlanSyncRemotePublisher + ?Sized,
{
    let seed_revision = local_revisions
        .first()
        .ok_or_else(|| format!("Local plan {local_plan_id} has no revisions to publish"))?;
    let seed_artifact_body = read_exact_local_plan_revision_artifact_body(
        local_blob_store,
        local_artifact_body_source,
        request,
        seed_revision,
    )?;
    let seed_packed_artifact = match seed_artifact_body.as_deref() {
        Some(body) => Some(publish_plan_revision_packed_artifact(
            file_io_store,
            local_source,
            local_blob_store,
            request,
            client,
            remote_repo_name,
            seed_revision,
            body,
        )?),
        None => None,
    };
    let atomic_outcome = if request.task_start.is_some() && local_revisions.len() == 1 {
        Some(start_atomic_plan_bound_task(
            request,
            client,
            remote_repo_name,
            JsonValue::Object(JsonMap::from_iter([
                (
                    "action".to_string(),
                    JsonValue::String("create".to_string()),
                ),
                (
                    "payload".to_string(),
                    task_start_plan_revision_payload(
                        local_plan,
                        seed_revision,
                        seed_artifact_body.as_deref(),
                        seed_packed_artifact.as_ref(),
                    )?,
                ),
            ])),
        )?)
    } else {
        None
    };
    let created_remote_plan = match atomic_outcome.as_ref() {
        Some(outcome) => outcome.remote_plan.clone(),
        None => create_plan_with_plan_sync_remote_client(
            client,
            remote_repo_name,
            text_field(seed_revision, "title_snapshot")
                .as_deref()
                .unwrap_or(local_plan_id),
            text_field(seed_revision, "artifact_path")
                .as_deref()
                .unwrap_or(""),
            text_field(seed_revision, "artifact_selector").as_deref(),
            text_field(seed_revision, "artifact_heading")
                .as_deref()
                .unwrap_or(local_plan_id),
            as_array(value_get(seed_revision, "items"))?,
            text_field(seed_revision, "summary").as_deref(),
            text_field(local_plan, "status")
                .as_deref()
                .unwrap_or(DEFAULT_PLAN_STATUS),
            None,
            text_field(seed_revision, "source_kind")
                .as_deref()
                .unwrap_or(DEFAULT_SOURCE_KIND),
            seed_artifact_body.as_deref(),
            seed_packed_artifact.as_ref(),
        )
        .map_err(|create_error| create_error.to_string())?,
    };
    let created_remote_plan_id = require_plan_id(&created_remote_plan)?;
    crate::plan_binary_db::parse_repository_plan_id(&created_remote_plan_id).map_err(|error| {
        format!(
            "Remote create for local plan {local_plan_id} returned non-canonical Binary Plan identity {created_remote_plan_id}: {error}"
        )
    })?;
    require_selected_remote_plan_identity(&created_remote_plan_id, &created_remote_plan)?;
    let seed_remote_revision_id =
        remote_head_revision_id(&created_remote_plan).ok_or_else(|| {
            format!(
            "Remote create for local plan {local_plan_id} did not return a head revision identity."
        )
        })?;
    Ok((
        created_remote_plan,
        created_remote_plan_id,
        (
            require_plan_revision_id(seed_revision)?,
            seed_remote_revision_id,
        ),
        atomic_outcome,
    ))
}

fn task_start_plan_revision_payload(
    local_plan: &JsonValue,
    revision: &JsonValue,
    artifact_body: Option<&str>,
    packed_artifact: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    let mut payload = JsonMap::from_iter([
        (
            "title".to_string(),
            JsonValue::String(
                text_field(revision, "title_snapshot")
                    .or_else(|| text_field(local_plan, "title"))
                    .ok_or_else(|| {
                        "Atomic task-start Plan revision has no title authority.".to_string()
                    })?,
            ),
        ),
        (
            "status".to_string(),
            JsonValue::String(
                text_field(local_plan, "status").unwrap_or_else(|| DEFAULT_PLAN_STATUS.to_string()),
            ),
        ),
        (
            "artifact_path".to_string(),
            JsonValue::String(text_field(revision, "artifact_path").ok_or_else(|| {
                "Atomic task-start Plan revision has no artifact_path.".to_string()
            })?),
        ),
        (
            "artifact_selector".to_string(),
            text_field(revision, "artifact_selector")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "artifact_heading".to_string(),
            JsonValue::String(text_field(revision, "artifact_heading").ok_or_else(|| {
                "Atomic task-start Plan revision has no artifact_heading.".to_string()
            })?),
        ),
        (
            "items".to_string(),
            JsonValue::Array(as_array(value_get(revision, "items"))?.to_vec()),
        ),
        (
            "source_kind".to_string(),
            JsonValue::String(
                text_field(revision, "source_kind")
                    .unwrap_or_else(|| DEFAULT_SOURCE_KIND.to_string()),
            ),
        ),
    ]);
    if let Some(summary) = text_field(revision, "summary") {
        payload.insert("summary".to_string(), JsonValue::String(summary));
    }
    if let Some(artifact_body) = artifact_body {
        payload.insert(
            "artifact_body".to_string(),
            JsonValue::String(artifact_body.to_string()),
        );
    }
    if let Some(packed_artifact) = packed_artifact {
        payload.insert("packed_artifact".to_string(), packed_artifact.clone());
    }
    Ok(JsonValue::Object(payload))
}

fn start_atomic_plan_bound_task<C>(
    request: &SyncRequest,
    client: &mut C,
    remote_repo_name: &str,
    plan_operation: JsonValue,
) -> Result<AtomicTaskStartOutcome, String>
where
    C: PlanSyncRemotePublisher + ?Sized,
{
    let task_start = request
        .task_start
        .as_ref()
        .ok_or_else(|| "Atomic task-start request context is missing.".to_string())?;
    let payload = JsonValue::Object(JsonMap::from_iter([
        (
            "contract".to_string(),
            JsonValue::String(task_start.contract.clone()),
        ),
        (
            "idempotency_key".to_string(),
            JsonValue::String(task_start.idempotency_key.clone()),
        ),
        (
            "plan_item_ref".to_string(),
            JsonValue::String(task_start.plan_item_ref.clone()),
        ),
        ("plan".to_string(), plan_operation),
        ("task".to_string(), task_start.task.clone()),
        ("change".to_string(), task_start.change.clone()),
    ]));
    let started = Instant::now();
    let response =
        start_plan_bound_task_with_plan_sync_remote_client(client, remote_repo_name, &payload)?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    if text_field(&response, "contract").as_deref() != Some(task_start.contract.as_str()) {
        return Err("Atomic task-start response returned an unexpected contract.".to_string());
    }
    if text_field(&response, "repo_name").as_deref() != Some(remote_repo_name) {
        return Err(format!(
            "Atomic task-start response belongs to repository {:?}, not {remote_repo_name}.",
            text_field(&response, "repo_name")
        ));
    }
    if text_field(&response, "plan_item_ref").as_deref() != Some(task_start.plan_item_ref.as_str())
    {
        return Err("Atomic task-start response returned a different Plan item ref.".to_string());
    }
    let remote_plan = value_get(&response, "plan")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| "Atomic task-start response is missing its Plan projection.".to_string())?;
    let remote_plan_id = require_plan_id(&response)?;
    require_selected_remote_plan_identity(&remote_plan_id, &remote_plan)?;
    let response_revision_id = text_field(&response, "plan_revision_id")
        .ok_or_else(|| "Atomic task-start response is missing plan_revision_id.".to_string())?;
    if remote_head_revision_id(&remote_plan).as_deref() != Some(response_revision_id.as_str()) {
        return Err(
            "Atomic task-start response Plan head does not match plan_revision_id.".to_string(),
        );
    }
    if !value_get(&response, "task").is_some_and(JsonValue::is_object)
        || !value_get(&response, "change").is_some_and(JsonValue::is_object)
    {
        return Err(
            "Atomic task-start response must include Task and Change projections.".to_string(),
        );
    }
    Ok(AtomicTaskStartOutcome {
        response,
        remote_plan,
        elapsed_ms,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "local publication coordinates explicit storage, identity, and remote ports"
)]
pub(super) fn local_plan_publish<L, B, A, F, I, C>(
    request: &SyncRequest,
    local_source: &L,
    local_blob_store: &B,
    local_artifact_body_source: &A,
    file_io_store: &F,
    identity_source: &I,
    client: &mut C,
    plan_id: &str,
    divergent_retry_mode: Option<&str>,
) -> Result<JsonValue, String>
where
    L: PlanSyncLocalPublishSource + ?Sized,
    B: PlanSyncLocalBlobStore + PlanSyncZstdPackStore + ?Sized,
    A: PlanSyncLocalArtifactBodySource + ?Sized,
    F: FileIoStore + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
    C: PlanSyncRemotePublisher + ?Sized,
{
    let local_plan = get_plan_with_plan_sync_local_store(local_source, plan_id)?;
    let mut remote_plan_id = selected_remote_plan_id(&local_plan, plan_id)?;
    let remote_repo_name = request
        .remote_repo_name
        .as_deref()
        .ok_or_else(|| "Remote repo_name is required for remote publish.".to_string())?;
    if text_field(&local_plan, "repo_name").as_deref() != Some(remote_repo_name) {
        return Err(format!(
            "Local plan {plan_id} belongs to repository {}, not {}",
            text_field(&local_plan, "repo_name").unwrap_or_default(),
            remote_repo_name
        ));
    }
    let local_revisions = sort_revisions_ascending(list_plan_revisions_with_plan_sync_local_store(
        local_source,
        plan_id,
    )?);
    if local_revisions.is_empty() {
        return Err(format!("Local plan {plan_id} has no revisions to publish"));
    }

    let mut revision_mappings = Vec::new();
    let mut remote_revision_detail_cache = BTreeMap::new();
    let mut remote_plan: Option<JsonValue> = None;
    let mut published_revision_count = 0usize;
    let mut rebase_details: Option<JsonValue> = None;
    let mut reconcile_details: Option<JsonValue> = None;
    let mut replayed_missing_published_remote = false;
    let mut atomic_task_start: Option<AtomicTaskStartOutcome> = None;
    let has_published_remote_plan_receipt = text_field(&local_plan, "published_plan_id").is_some();
    let published_revisions = local_revisions
        .iter()
        .filter(|row| {
            text_field(row, "publication_state").as_deref() == Some("published")
                && text_field(row, "published_plan_revision_id").is_some()
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut unpublished_revisions = local_revisions
        .iter()
        .filter(|row| {
            text_field(row, "publication_state").as_deref() != Some("published")
                || text_field(row, "published_plan_revision_id").is_none()
        })
        .cloned()
        .collect::<Vec<_>>();

    if text_field(&local_plan, "publication_state").as_deref() != Some("published") {
        let mut create_remote = false;
        let mut occupied_remote_plan_id = None;
        match get_plan_with_plan_sync_remote_client(client, &remote_plan_id) {
            Ok(existing_remote_plan) => {
                match match_existing_remote_plan_history(
                    local_blob_store,
                    &remote_plan_id,
                    &local_revisions,
                    existing_remote_plan,
                    client,
                    &mut remote_revision_detail_cache,
                )? {
                    Some((existing_remote_plan, existing_mappings, remaining)) => {
                        revision_mappings.extend(existing_mappings);
                        remote_plan = Some(existing_remote_plan);
                        unpublished_revisions = remaining;
                    }
                    None => {
                        occupied_remote_plan_id = Some(remote_plan_id.clone());
                        create_remote = true;
                    }
                }
            }
            Err(error) if remote_plan_lookup_is_absent(&error, &remote_plan_id) => {
                ensure_missing_remote_create_is_unbound(
                    plan_id,
                    &remote_plan_id,
                    has_published_remote_plan_receipt,
                    &error,
                )?;
                create_remote = true;
            }
            Err(error) => return Err(error),
        }
        if create_remote {
            let (created_remote_plan, created_remote_plan_id, seed_mapping, create_task_start) =
                create_remote_plan_from_local_seed(
                    request,
                    local_source,
                    local_blob_store,
                    local_artifact_body_source,
                    file_io_store,
                    client,
                    remote_repo_name,
                    &local_plan,
                    &local_revisions,
                    plan_id,
                )?;
            if occupied_remote_plan_id.as_deref() == Some(created_remote_plan_id.as_str()) {
                return Err(format!(
                    "Remote create for colliding local plan {plan_id} reused occupied Binary Plan identity {created_remote_plan_id}; refusing to overwrite either lineage."
                ));
            }
            remote_plan_id = created_remote_plan_id;
            revision_mappings.push(seed_mapping);
            published_revision_count += 1;
            remote_plan = Some(created_remote_plan);
            atomic_task_start = create_task_start;
            unpublished_revisions = local_revisions[1..].to_vec();
        }
    } else {
        let fetched_remote_plan =
            match get_plan_with_plan_sync_remote_client(client, &remote_plan_id) {
                Ok(fetched_remote_plan) => Some(fetched_remote_plan),
                Err(error)
                    if missing_published_remote_replay_is_allowed(
                        request.reconcile,
                        &error,
                        &remote_plan_id,
                    ) =>
                {
                    None
                }
                Err(error) => return Err(error),
            };
        if let Some(fetched_remote_plan) = fetched_remote_plan {
            require_selected_remote_plan_identity(&remote_plan_id, &fetched_remote_plan)?;
            let mut recovered_published_head = None;
            if published_revisions.is_empty() {
                let remote_revisions = sort_revisions_ascending(
                    list_plan_revisions_with_plan_sync_remote_client(client, &remote_plan_id)
                        .map_err(|err| err.to_string())?,
                );
                let (recovered_mappings, remaining, recovered_head) =
                    map_exact_dense_remote_plan_revision_prefix(
                        &local_revisions,
                        &remote_revisions,
                    )?;
                if recovered_mappings.is_empty() {
                    return Err(format!(
                        "Local published plan {plan_id} has no publication receipts and no exact canonical remote revision prefix; refusing to replay historical revisions."
                    ));
                }
                revision_mappings.extend(recovered_mappings);
                unpublished_revisions = remaining;
                recovered_published_head = recovered_head;
            }
            let latest_published = published_revisions.last().cloned();
            let latest_published_number = latest_published
                .as_ref()
                .and_then(|row| value_get(row, "revision_number"))
                .and_then(|value| value.as_i64())
                .unwrap_or(0);
            if latest_published.is_some() {
                unpublished_revisions = local_revisions
                    .iter()
                    .filter(|row| {
                        value_get(row, "revision_number")
                            .and_then(|value| value.as_i64())
                            .unwrap_or(0)
                            > latest_published_number
                            && (text_field(row, "publication_state").as_deref()
                                != Some("published")
                                || text_field(row, "published_plan_revision_id").is_none())
                    })
                    .cloned()
                    .collect();
            }
            let expected_remote_head = latest_published
                .as_ref()
                .and_then(|row| text_field(row, "published_plan_revision_id"))
                .or(recovered_published_head);
            let actual_remote_head = remote_head_revision_id(&fetched_remote_plan);
            if expected_remote_head.is_some() && actual_remote_head != expected_remote_head {
                let remote_revisions = sort_revisions_ascending(
                    list_plan_revisions_with_plan_sync_remote_client(client, &remote_plan_id)
                        .map_err(|err| err.to_string())?,
                );
                if let Some((suffix_mappings, remaining_unpublished)) =
                    map_equivalent_remote_plan_revision_suffix(
                        &unpublished_revisions,
                        &remote_revisions,
                        expected_remote_head.as_deref(),
                    )?
                {
                    revision_mappings.extend(suffix_mappings);
                    unpublished_revisions = remaining_unpublished;
                } else {
                    let Some(mode) = divergent_retry_mode else {
                        return Err(format!(
                            "Remote plan {remote_plan_id} for local {plan_id} has advanced to {}; retry the shared publish with `--rebase` or the legacy `--reconcile` retry path.",
                            actual_remote_head.unwrap_or_default()
                        ));
                    };
                    let (retry_mappings, remaining_unpublished, retry_details) =
                        select_divergent_retry_publish_target(
                            &local_revisions,
                            &remote_revisions,
                            actual_remote_head.as_deref(),
                        )?;
                    revision_mappings.extend(retry_mappings);
                    unpublished_revisions = remaining_unpublished;
                    if mode == "rebase" {
                        rebase_details = Some(retry_details);
                    } else {
                        reconcile_details = Some(retry_details);
                    }
                }
            }
            remote_plan = Some(fetched_remote_plan);
        } else {
            replayed_missing_published_remote = true;
            let (created_remote_plan, created_remote_plan_id, seed_mapping, create_task_start) =
                create_remote_plan_from_local_seed(
                    request,
                    local_source,
                    local_blob_store,
                    local_artifact_body_source,
                    file_io_store,
                    client,
                    remote_repo_name,
                    &local_plan,
                    &local_revisions,
                    plan_id,
                )?;
            remote_plan_id = created_remote_plan_id;
            revision_mappings.push(seed_mapping);
            published_revision_count += 1;
            remote_plan = Some(created_remote_plan);
            atomic_task_start = create_task_start;
            unpublished_revisions = local_revisions[1..].to_vec();
        }
    }

    let unpublished_revision_count = unpublished_revisions.len();
    for (revision_offset, revision) in unpublished_revisions.into_iter().enumerate() {
        let expected_head_revision_id = remote_plan.as_ref().and_then(remote_head_revision_id);
        let revision_artifact_body = read_exact_local_plan_revision_artifact_body(
            local_blob_store,
            local_artifact_body_source,
            request,
            &revision,
        )?;
        let revision_packed_artifact = match revision_artifact_body.as_deref() {
            Some(body) => Some(publish_plan_revision_packed_artifact(
                file_io_store,
                local_source,
                local_blob_store,
                request,
                client,
                remote_repo_name,
                &revision,
                body,
            )?),
            None => None,
        };
        let use_atomic_task_start = request.task_start.is_some()
            && atomic_task_start.is_none()
            && revision_offset + 1 == unpublished_revision_count;
        let revised_remote = if use_atomic_task_start {
            let outcome = start_atomic_plan_bound_task(
                request,
                client,
                remote_repo_name,
                JsonValue::Object(JsonMap::from_iter([
                    (
                        "action".to_string(),
                        JsonValue::String("revise".to_string()),
                    ),
                    (
                        "plan_id".to_string(),
                        JsonValue::String(remote_plan_id.clone()),
                    ),
                    (
                        "expected_head_revision_id".to_string(),
                        expected_head_revision_id
                            .as_ref()
                            .map(|value| JsonValue::String(value.clone()))
                            .unwrap_or(JsonValue::Null),
                    ),
                    (
                        "payload".to_string(),
                        task_start_plan_revision_payload(
                            &local_plan,
                            &revision,
                            revision_artifact_body.as_deref(),
                            revision_packed_artifact.as_ref(),
                        )?,
                    ),
                ])),
            )?;
            let remote_plan = outcome.remote_plan.clone();
            atomic_task_start = Some(outcome);
            remote_plan
        } else {
            revise_plan_with_plan_sync_remote_client(
                client,
                &remote_plan_id,
                text_field(&revision, "artifact_path")
                    .as_deref()
                    .unwrap_or(""),
                text_field(&revision, "artifact_selector").as_deref(),
                text_field(&revision, "artifact_heading")
                    .as_deref()
                    .unwrap_or(plan_id),
                as_array(value_get(&revision, "items"))?,
                text_field(&revision, "title_snapshot").as_deref(),
                text_field(&revision, "summary").as_deref(),
                text_field(&revision, "source_kind")
                    .as_deref()
                    .unwrap_or(DEFAULT_SOURCE_KIND),
                revision_artifact_body.as_deref(),
                expected_head_revision_id.as_deref(),
                revision_packed_artifact.as_ref(),
            )
            .map_err(|err| err.to_string())?
        };
        require_selected_remote_plan_identity(&remote_plan_id, &revised_remote)?;
        let remote_revision_id = remote_head_revision_id(&revised_remote).ok_or_else(|| {
            format!(
                "Remote plan {remote_plan_id} for local {plan_id} revise response did not include a head revision id"
            )
        })?;
        revision_mappings.push((require_plan_revision_id(&revision)?, remote_revision_id));
        published_revision_count += 1;
        remote_plan = Some(revised_remote);
    }

    let mut final_remote_plan = remote_plan.unwrap_or(JsonValue::Null);
    if final_remote_plan.is_null() {
        final_remote_plan = get_plan_with_plan_sync_remote_client(client, &remote_plan_id)
            .map_err(|err| err.to_string())?;
        require_selected_remote_plan_identity(&remote_plan_id, &final_remote_plan)?;
    }
    if text_field(&final_remote_plan, "status") != text_field(&local_plan, "status") {
        if request.task_start.is_some() {
            return Err(format!(
                "Atomic task-start requires local Plan {plan_id} status {:?} to match remote Plan {remote_plan_id} status {:?}; publish the status change before starting the Task.",
                text_field(&local_plan, "status"),
                text_field(&final_remote_plan, "status"),
            ));
        }
        final_remote_plan = update_plan_status_with_plan_sync_remote_client(
            client,
            &remote_plan_id,
            text_field(&local_plan, "status")
                .as_deref()
                .unwrap_or(DEFAULT_PLAN_STATUS),
        )
        .map_err(|err| err.to_string())?;
        require_selected_remote_plan_identity(&remote_plan_id, &final_remote_plan)?;
    }
    if request.task_start.is_some() && atomic_task_start.is_none() {
        let remote_head_revision_id =
            remote_head_revision_id(&final_remote_plan).ok_or_else(|| {
                format!(
                "Remote Plan {remote_plan_id} has no head revision for atomic task-start binding."
            )
            })?;
        let outcome = start_atomic_plan_bound_task(
            request,
            client,
            remote_repo_name,
            JsonValue::Object(JsonMap::from_iter([
                (
                    "action".to_string(),
                    JsonValue::String("existing".to_string()),
                ),
                (
                    "plan_id".to_string(),
                    JsonValue::String(remote_plan_id.clone()),
                ),
                (
                    "plan_revision_id".to_string(),
                    JsonValue::String(remote_head_revision_id),
                ),
            ])),
        )?;
        final_remote_plan = outcome.remote_plan.clone();
        atomic_task_start = Some(outcome);
    }
    let remote_head_revision_id_value = remote_head_revision_id(&final_remote_plan);
    if replayed_missing_published_remote {
        reconcile_details = Some(missing_remote_replay_reconcile_details(
            &local_plan,
            &final_remote_plan,
        )?);
    }
    let published_at = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
    let plan_row = mark_plan_published_with_plan_sync_local_store(
        local_source,
        plan_id,
        request.remote_name.as_deref(),
        &remote_plan_id,
        remote_head_revision_id_value.as_deref(),
        &revision_mappings,
        published_at.as_str(),
    )?;
    let publish_action = if rebase_details.is_some() {
        if published_revision_count > 0 {
            "rebased"
        } else {
            "rebased_mapping"
        }
    } else if reconcile_details.is_some() {
        if published_revision_count > 0 {
            "reconciled"
        } else {
            "reconciled_mapping"
        }
    } else if published_revision_count == 0 {
        "mapped"
    } else {
        "published"
    };
    let mut payload = JsonMap::from_iter([
        (
            "plan_id".to_string(),
            value_get(&plan_row, "plan_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "status".to_string(),
            value_get(&plan_row, "status")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "publication_state".to_string(),
            value_get(&plan_row, "publication_state")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "head_revision_id".to_string(),
            value_get(&plan_row, "head_revision_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "published_plan_id".to_string(),
            value_get(&plan_row, "published_plan_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "published_head_revision_id".to_string(),
            value_get(&plan_row, "published_head_revision_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "head_publication_state".to_string(),
            value_get(&plan_row, "head_revision")
                .and_then(|value| value.as_object())
                .and_then(|value| value.get("publication_state"))
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "publish_action".to_string(),
            JsonValue::String(publish_action.to_string()),
        ),
        (
            "published_revision_count".to_string(),
            JsonValue::Number(Number::from(published_revision_count as u64)),
        ),
        (
            "rebased".to_string(),
            JsonValue::Bool(rebase_details.is_some()),
        ),
        (
            "reconciled".to_string(),
            JsonValue::Bool(reconcile_details.is_some()),
        ),
    ]);
    if let Some(details) = rebase_details {
        payload.insert(
            "rebase_mode".to_string(),
            value_get(&details, "mode")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
        payload.insert(
            "rebase_remote_head_revision_id".to_string(),
            value_get(&details, "remote_head_revision_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
        payload.insert(
            "rebase_local_head_revision_id".to_string(),
            value_get(&details, "local_head_revision_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
    }
    if let Some(details) = reconcile_details {
        payload.insert(
            "reconcile_mode".to_string(),
            value_get(&details, "mode")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
        payload.insert(
            "reconcile_remote_head_revision_id".to_string(),
            value_get(&details, "remote_head_revision_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
        payload.insert(
            "reconcile_local_head_revision_id".to_string(),
            value_get(&details, "local_head_revision_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
    }
    if let Some(outcome) = atomic_task_start {
        payload.insert("task_start".to_string(), outcome.response);
        payload.insert(
            "task_start_elapsed_ms".to_string(),
            json!(outcome.elapsed_ms),
        );
    }
    Ok(JsonValue::Object(payload))
}

pub(super) fn matching_remote_publish_revision<B, C>(
    local_blob_store: &B,
    plan_id: &str,
    local_revision: &JsonValue,
    remote_revision: &JsonValue,
    client: &mut C,
    remote_revision_detail_cache: &mut BTreeMap<(String, String), JsonValue>,
) -> Result<Option<JsonValue>, String>
where
    B: PlanSyncLocalBlobStore + ?Sized,
    C: PlanSyncRemoteRevisionReader + ?Sized,
{
    if plan_publish_revision_metadata_matches(local_revision, remote_revision) {
        return Ok(Some(remote_revision.clone()));
    }
    let materialized_remote_revision = materialize_remote_revision(
        local_blob_store,
        plan_id,
        remote_revision,
        Some(client),
        remote_revision_detail_cache,
    )?;
    if !plan_publish_revision_metadata_matches(local_revision, &materialized_remote_revision) {
        return Ok(None);
    }
    Ok(Some(materialized_remote_revision))
}

pub(super) fn resolve_paired_artifacts(
    request: &SyncRequest,
    sync_target: &SyncTarget,
    markdown_artifacts: &[SyncArtifact],
) -> Result<BTreeMap<String, Vec<JsonValue>>, String> {
    let markdown_paths = markdown_artifacts
        .iter()
        .map(|artifact| artifact.artifact_path.clone())
        .collect::<BTreeSet<_>>();
    if markdown_paths.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut candidates = Vec::new();
    if let Some(candidate) =
        public_package_targets_contract_candidate(&request.root_path, sync_target)
    {
        candidates.push(candidate);
    }
    if let Some(candidate) =
        public_future_repo_extraction_prep_contract_candidate(&request.root_path, sync_target)
    {
        candidates.push(candidate);
    }
    if let Some(candidate) =
        public_future_repo_split_dry_run_contract_candidate(&request.root_path, sync_target)
    {
        candidates.push(candidate);
    }
    candidates.sort();
    candidates.dedup();
    let mut grouped = BTreeMap::<String, Vec<JsonValue>>::new();
    for path in candidates {
        let payload = resolve_paired_artifact(request, &path)?;
        let source_artifact_path = optional_text(value_get(&payload, "source_artifact_path"))?
            .ok_or_else(|| "Paired artifact is missing source_artifact_path.".to_string())?;
        if !markdown_paths.contains(source_artifact_path.as_str()) {
            let artifact_label = optional_text(value_get(&payload, "artifact_label"))?
                .unwrap_or_else(|| "Paired artifact".to_string());
            return Err(format!(
                "{} {} points at {}, which is not part of this plan sync target.",
                artifact_label,
                optional_text(value_get(&payload, "artifact_path"))?.unwrap_or_default(),
                source_artifact_path
            ));
        }
        grouped
            .entry(source_artifact_path)
            .or_default()
            .push(payload);
    }
    Ok(grouped)
}

pub(super) fn public_package_targets_contract_candidate(
    repo_root: &str,
    sync_target: &SyncTarget,
) -> Option<PathBuf> {
    let contract = Path::new(repo_root).join(PUBLIC_PACKAGE_TARGETS_CONTRACT_PATH);
    if !contract.exists() || !contract.is_file() {
        return None;
    }
    if sync_target.resolved_target.is_file() {
        let guide = Path::new(repo_root).join(PUBLIC_PACKAGE_TARGETS_GUIDE_PATH);
        return if sync_target.resolved_target == guide {
            Some(contract)
        } else {
            None
        };
    }
    let guide = Path::new(repo_root).join(PUBLIC_PACKAGE_TARGETS_GUIDE_PATH);
    if guide.starts_with(&sync_target.resolved_target) {
        Some(contract)
    } else {
        None
    }
}

pub(super) fn public_future_repo_extraction_prep_contract_candidate(
    repo_root: &str,
    sync_target: &SyncTarget,
) -> Option<PathBuf> {
    let contract = Path::new(repo_root).join(PUBLIC_FUTURE_REPO_PREP_CONTRACT_PATH);
    if !contract.exists() || !contract.is_file() {
        return None;
    }
    if sync_target.resolved_target.is_file() {
        let guide = Path::new(repo_root).join(PUBLIC_FUTURE_REPO_PREP_GUIDE_PATH);
        return if sync_target.resolved_target == guide {
            Some(contract)
        } else {
            None
        };
    }
    let guide = Path::new(repo_root).join(PUBLIC_FUTURE_REPO_PREP_GUIDE_PATH);
    if guide.starts_with(&sync_target.resolved_target) {
        Some(contract)
    } else {
        None
    }
}

pub(super) fn public_future_repo_split_dry_run_contract_candidate(
    repo_root: &str,
    sync_target: &SyncTarget,
) -> Option<PathBuf> {
    let contract = Path::new(repo_root).join(PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_CONTRACT_PATH);
    if !contract.exists() || !contract.is_file() {
        return None;
    }
    if sync_target.resolved_target.is_file() {
        let guide = Path::new(repo_root).join(PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_GUIDE_PATH);
        return if sync_target.resolved_target == guide {
            Some(contract)
        } else {
            None
        };
    }
    let guide = Path::new(repo_root).join(PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_GUIDE_PATH);
    if guide.starts_with(&sync_target.resolved_target) {
        Some(contract)
    } else {
        None
    }
}

pub(super) fn resolve_paired_artifact(
    request: &SyncRequest,
    path: &Path,
) -> Result<JsonValue, String> {
    let payload =
        resolve_repo_artifact_path(&request.root_path, path.to_string_lossy().as_ref(), false)
            .map_err(plan_fs_error)?;
    let artifact_path = required_string_field(&payload, "artifact_path")?;
    if artifact_path == PUBLIC_PACKAGE_TARGETS_CONTRACT_PATH {
        return resolve_public_package_targets_contract_artifact(request, path);
    }
    if artifact_path == PUBLIC_FUTURE_REPO_PREP_CONTRACT_PATH {
        return resolve_public_future_repo_extraction_prep_contract_artifact(request, path);
    }
    if artifact_path == PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_CONTRACT_PATH {
        return resolve_public_future_repo_split_dry_run_contract_artifact(request, path);
    }
    Err(format!(
        "Unsupported plan sync paired artifact: {artifact_path}."
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_missing_remote_create_is_unbound, json, missing_published_remote_replay_is_allowed,
        missing_remote_replay_reconcile_details, remote_plan_lookup_is_absent,
    };

    #[test]
    fn remote_plan_lookup_accepts_only_the_exact_canonical_not_found_identity() {
        let error =
            "GET http://server.test/v1/native/repository-authorities/4/sprints/PR-1 failed: 404 Unknown plan: PR-1";
        assert!(remote_plan_lookup_is_absent(error, "PR-1"));
        assert!(!remote_plan_lookup_is_absent(error, "PR-2"));
    }

    #[test]
    fn remote_plan_lookup_keeps_unrelated_failures_closed() {
        for error in [
            "GET http://server.test/sprints/PR-1 failed: 503 Unknown plan: PR-1",
            "GET http://server.test/sprints/PR-1 failed: 404 Unknown plan revision: PR-1",
            "GET http://server.test/sprints/PR-1 failed: 404 Unknown plan: PR-1 trailing",
            "transport connection refused",
        ] {
            assert!(!remote_plan_lookup_is_absent(error, "PR-1"), "{error}");
        }
    }

    #[test]
    fn remote_plan_lookup_retains_bounded_legacy_out_of_bounds_compatibility() {
        assert!(remote_plan_lookup_is_absent(
            "record index 4 is out of bounds for file 'plan.bin' with 4 records",
            "PR-4",
        ));
    }

    #[test]
    fn missing_published_remote_replay_requires_explicit_reconcile() {
        let missing =
            "GET http://server.test/v1/native/repository-authorities/4/sprints/PR-1 failed: 404 Unknown plan: PR-1";
        assert!(!missing_published_remote_replay_is_allowed(
            false, missing, "PR-1",
        ));
        assert!(missing_published_remote_replay_is_allowed(
            true, missing, "PR-1",
        ));
        assert!(!missing_published_remote_replay_is_allowed(
            true, missing, "PR-2",
        ));
        assert!(!missing_published_remote_replay_is_allowed(
            true,
            "GET http://server.test/sprints/PR-1 failed: 503 Unknown plan: PR-1",
            "PR-1",
        ));
    }

    #[test]
    fn equal_raw_local_and_remote_ordinals_do_not_erase_an_explicit_receipt_boundary() {
        let error = ensure_missing_remote_create_is_unbound(
            "PR-52",
            "PR-52",
            true,
            "404 Unknown plan: PR-52",
        )
        .expect_err("an explicit remote receipt must remain authoritative across equal ordinals");
        assert!(error.contains("LPR-52"));
        assert!(error.contains("RPR-52"));

        ensure_missing_remote_create_is_unbound("PR-52", "PR-52", false, "404 Unknown plan: PR-52")
            .expect("an unbound local draft may create a missing remote Plan");
    }

    #[test]
    fn missing_remote_replay_projects_exact_final_heads() {
        let details = missing_remote_replay_reconcile_details(
            &json!({"head_revision_id": "LPR-3"}),
            &json!({"head_revision_id": "RPR-3"}),
        )
        .unwrap();
        assert_eq!(details["mode"], "missing_remote_replay");
        assert_eq!(details["local_head_revision_id"], "LPR-3");
        assert_eq!(details["remote_head_revision_id"], "RPR-3");

        let error = missing_remote_replay_reconcile_details(
            &json!({}),
            &json!({"head_revision_id": "RPR-3"}),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "Missing-remote Plan replay has no local head revision."
        );
    }
}
