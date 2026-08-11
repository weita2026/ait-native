use super::*;

type RevisionIdMappings = Vec<(String, String)>;
type EquivalentRevisionSuffix = (RevisionIdMappings, Vec<JsonValue>);
type DenseRevisionPrefix = (RevisionIdMappings, Vec<JsonValue>, Option<String>);
type DivergentRetryPublishTarget = (RevisionIdMappings, Vec<JsonValue>, JsonValue);

pub(super) fn validate_materialized_remote_plan_lineage(
    plan_id: &RemotePlanId,
    revisions: &[JsonValue],
) -> Result<(), String> {
    let first_revision = revisions
        .first()
        .ok_or_else(|| format!("Remote Plan {} has no revisions.", plan_id.reference()))?;
    let first_path = text_field(first_revision, "artifact_path").ok_or_else(|| {
        format!(
            "Remote Plan {} first revision has no artifact path.",
            plan_id.reference()
        )
    })?;
    let first_selector = text_field(first_revision, "artifact_selector");
    for revision in revisions {
        if let Some(revision_plan_id) = text_field(revision, "plan_id") {
            if revision_plan_id != plan_id.raw() {
                return Err(format!(
                    "Remote Plan {} revision {:?} belongs to {}; refusing adoption.",
                    plan_id.reference(),
                    text_field(revision, "plan_revision_id"),
                    RemotePlanId::from_raw(revision_plan_id)?.reference()
                ));
            }
        }
        let revision_path = text_field(revision, "artifact_path").ok_or_else(|| {
            format!(
                "Remote Plan {} revision {:?} has no artifact path.",
                plan_id.reference(),
                text_field(revision, "plan_revision_id")
            )
        })?;
        let revision_selector = text_field(revision, "artifact_selector");
        if !plan_lineage_identity_matches(
            &first_path,
            first_selector.as_deref(),
            &revision_path,
            revision_selector.as_deref(),
        ) {
            return Err(format!(
                "Remote Plan {} mixes {} with {}; refusing adoption before local mutation.",
                plan_id.reference(),
                plan_artifact_identity_label(&first_path, first_selector.as_deref()),
                plan_artifact_identity_label(&revision_path, revision_selector.as_deref())
            ));
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "adoption orchestration keeps storage, identity, remote, and cache ports explicit"
)]
pub(super) fn adopt_remote_plan_for_local_sync<W, B, I, C>(
    local_writer: &W,
    local_blob_store: &B,
    identity_source: &I,
    request: &SyncRequest,
    remote_plan: &JsonValue,
    remote_revisions: &[JsonValue],
    mut client: Option<&mut C>,
    remote_revision_detail_cache: &mut BTreeMap<(String, String), JsonValue>,
) -> Result<JsonValue, String>
where
    W: PlanSyncLocalAdoptionStore + ?Sized,
    B: PlanSyncLocalBlobStore + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
    C: PlanSyncRemoteRevisionReader + ?Sized,
{
    let plan_id = RemotePlanId::from_plan(remote_plan)?;
    let revisions = sort_revisions_ascending(remote_revisions.to_vec());
    if revisions.is_empty() {
        return Err(format!(
            "Remote Plan {} has no revisions to adopt locally.",
            plan_id.reference()
        ));
    }
    let mut materialized_revisions = Vec::with_capacity(revisions.len());
    for revision in &revisions {
        materialized_revisions.push(materialize_remote_revision(
            local_blob_store,
            plan_id.raw(),
            revision,
            client.as_deref_mut(),
            remote_revision_detail_cache,
        )?);
    }
    adopt_materialized_remote_plan_for_local_sync(
        local_writer,
        identity_source,
        request,
        remote_plan,
        &materialized_revisions,
    )
}

pub(super) fn adopt_materialized_remote_plan_for_local_sync<W, I>(
    local_writer: &W,
    identity_source: &I,
    request: &SyncRequest,
    remote_plan: &JsonValue,
    revisions: &[JsonValue],
) -> Result<JsonValue, String>
where
    W: PlanSyncLocalAdoptionStore + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
{
    adopt_materialized_remote_plan_for_local_sync_with_identity_policy(
        local_writer,
        identity_source,
        request,
        remote_plan,
        revisions,
    )
}

pub(super) fn adopt_materialized_remote_plan_at_distinct_local_identity<W, I>(
    local_writer: &W,
    identity_source: &I,
    request: &SyncRequest,
    remote_plan: &JsonValue,
    revisions: &[JsonValue],
) -> Result<JsonValue, String>
where
    W: PlanSyncLocalAdoptionStore + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
{
    adopt_materialized_remote_plan_for_local_sync_with_identity_policy(
        local_writer,
        identity_source,
        request,
        remote_plan,
        revisions,
    )
}

fn adopt_materialized_remote_plan_for_local_sync_with_identity_policy<W, I>(
    local_writer: &W,
    identity_source: &I,
    request: &SyncRequest,
    remote_plan: &JsonValue,
    revisions: &[JsonValue],
) -> Result<JsonValue, String>
where
    W: PlanSyncLocalAdoptionStore + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
{
    let plan_id = RemotePlanId::from_plan(remote_plan)?;
    let occupied_local_plan = get_plan_with_plan_sync_local_store(local_writer, plan_id.raw())
        .ok()
        .map(|plan| LocalPlanId::from_plan(&plan))
        .transpose()?;
    if let Some(occupied_local_plan) = occupied_local_plan.as_ref() {
        if !local_writer.remote_adoption_allocates_fresh_local_plan_identity() {
            return Err(format!(
                "Remote Plan {} collides with occupied local Plan {}; the local store cannot allocate a distinct local identity, so adoption is refused before mutation.",
                plan_id.reference(),
                occupied_local_plan.reference()
            ));
        }
    }
    if revisions.is_empty() {
        return Err(format!(
            "Remote Plan {} has no materialized revisions to adopt locally.",
            plan_id.reference()
        ));
    }
    validate_materialized_remote_plan_lineage(&plan_id, revisions)?;
    let mut revision_mappings = Vec::new();
    let first_revision = &revisions[0];
    let first_local_revision_id = workflow_id_with_plan_sync_workflow_identity_source(
        identity_source,
        "PR",
        request.id_namespace_prefix.as_deref(),
    )?;
    let first_blob_id = text_field(first_revision, "artifact_blob_id");
    let remote_title =
        text_field(remote_plan, "title").unwrap_or_else(|| plan_id.raw().to_string());
    let first_title_snapshot = text_field(first_revision, "title_snapshot");
    let first_artifact_heading = text_field(first_revision, "artifact_heading");
    let first_title = first_title_snapshot
        .as_deref()
        .unwrap_or(remote_title.as_str())
        .to_string();
    let first_heading = first_artifact_heading
        .as_deref()
        .or(first_title_snapshot.as_deref())
        .unwrap_or(remote_title.as_str())
        .to_string();
    let first_artifact_path = text_field(first_revision, "artifact_path").unwrap_or_default();
    let first_items_json = JsonCodec::encode_value(
        &JsonValue::Array(as_array(value_get(first_revision, "items"))?.to_vec()),
        JsonEncodeOptions::compact(),
    )
    .map_err(String::from)?;
    let first_now = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
    let first_row = create_plan_with_plan_sync_local_artifact_writer(
        local_writer,
        &PlanSyncLocalPlanCreate {
            plan_id: plan_id.raw(),
            plan_revision_id: &first_local_revision_id,
            repo_name: request
                .remote_repo_name
                .as_deref()
                .unwrap_or(request.repo_name.as_str()),
            title: &first_title,
            artifact_path: &first_artifact_path,
            artifact_selector: text_field(first_revision, "artifact_selector").as_deref(),
            artifact_heading: &first_heading,
            items_json: &first_items_json,
            artifact_blob_id: first_blob_id.as_deref(),
            artifact_root: None,
            summary: text_field(first_revision, "summary").as_deref(),
            status: text_field(remote_plan, "status")
                .as_deref()
                .unwrap_or(DEFAULT_PLAN_STATUS),
            source_kind: text_field(first_revision, "source_kind")
                .as_deref()
                .unwrap_or("remote_adoption"),
            created_by: text_field(first_revision, "created_by").as_deref(),
            actor_type: text_field(first_revision, "actor_type")
                .as_deref()
                .unwrap_or(DEFAULT_ACTOR_TYPE),
            publication_state: LOCAL_DRAFT_PUBLICATION_STATE,
            now: first_now.as_str(),
        },
    )?;
    let local_plan_id = LocalPlanId::from_plan(&first_row)?;
    if occupied_local_plan
        .as_ref()
        .is_some_and(|occupied| occupied.raw() == local_plan_id.raw())
    {
        return Err(format!(
            "Remote Plan {} collided with occupied local Plan {}, and the local writer reused that occupied identity; refusing to append either lineage.",
            plan_id.reference(),
            local_plan_id.reference()
        ));
    }
    let first_local_revision_id =
        local_head_revision_id(&first_row).unwrap_or(first_local_revision_id);
    let remote_revision_id = text_field(first_revision, "plan_revision_id");
    if let Some(remote_revision_id) = remote_revision_id {
        revision_mappings.push((first_local_revision_id, remote_revision_id));
    }

    for revision in revisions.iter().skip(1) {
        let local_revision_id = workflow_id_with_plan_sync_workflow_identity_source(
            identity_source,
            "PR",
            request.id_namespace_prefix.as_deref(),
        )?;
        let revision_title_snapshot = text_field(revision, "title_snapshot");
        let revision_heading = text_field(revision, "artifact_heading")
            .or_else(|| revision_title_snapshot.clone())
            .unwrap_or_else(|| remote_title.clone());
        let revision_artifact_path = text_field(revision, "artifact_path").unwrap_or_default();
        let revision_items_json = JsonCodec::encode_value(
            &JsonValue::Array(as_array(value_get(revision, "items"))?.to_vec()),
            JsonEncodeOptions::compact(),
        )
        .map_err(String::from)?;
        let revision_now = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
        let revision_row = revise_plan_with_plan_sync_local_artifact_writer(
            local_writer,
            &PlanSyncLocalPlanRevision {
                plan_id: local_plan_id.raw(),
                plan_revision_id: &local_revision_id,
                artifact_path: &revision_artifact_path,
                artifact_selector: text_field(revision, "artifact_selector").as_deref(),
                artifact_heading: &revision_heading,
                items_json: &revision_items_json,
                artifact_blob_id: text_field(revision, "artifact_blob_id").as_deref(),
                artifact_root: None,
                title: revision_title_snapshot.as_deref(),
                summary: text_field(revision, "summary").as_deref(),
                source_kind: text_field(revision, "source_kind")
                    .as_deref()
                    .unwrap_or("remote_adoption"),
                created_by: text_field(revision, "created_by").as_deref(),
                actor_type: text_field(revision, "actor_type")
                    .as_deref()
                    .unwrap_or(DEFAULT_ACTOR_TYPE),
                now: revision_now.as_str(),
            },
        )?;
        let local_revision_id = local_head_revision_id(&revision_row).unwrap_or(local_revision_id);
        if let Some(remote_revision_id) = text_field(revision, "plan_revision_id") {
            revision_mappings.push((local_revision_id, remote_revision_id));
        }
    }

    let published_at = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
    mark_plan_published_with_plan_sync_local_store(
        local_writer,
        local_plan_id.raw(),
        request.remote_name.as_deref(),
        plan_id.raw(),
        remote_head_revision_id(remote_plan).as_deref(),
        &revision_mappings,
        published_at.as_str(),
    )
}

pub(super) fn materialize_remote_revision<B, C>(
    local_blob_store: &B,
    plan_id: &str,
    revision: &JsonValue,
    mut client: Option<&mut C>,
    remote_revision_detail_cache: &mut BTreeMap<(String, String), JsonValue>,
) -> Result<JsonValue, String>
where
    B: PlanSyncLocalBlobStore + ?Sized,
    C: PlanSyncRemoteRevisionReader + ?Sized,
{
    let mut payload = revision.clone();
    let declared_blob_id = text_field(&payload, "artifact_blob_id");
    let artifact_body = raw_string_field(&payload, "artifact_body");
    if let Some(body) = artifact_body {
        let computed_blob_id = artifact_blob_id(&body);
        if let Some(expected_blob_id) = declared_blob_id.as_deref() {
            if expected_blob_id != computed_blob_id {
                return Err(format!(
                    "Remote plan revision artifact body does not match its declared artifact_blob_id for plan revision {:?}.",
                    text_field(&payload, "plan_revision_id")
                ));
            }
        }
        let local_blob_id = ensure_blob_bytes_with_plan_sync_local_blob_store(
            local_blob_store,
            body.as_bytes(),
            text_field(&payload, "artifact_path").as_deref(),
        )?;
        if local_blob_id != computed_blob_id {
            return Err(format!(
                "Local blob store materialized {local_blob_id} for remote plan revision {:?}, expected {computed_blob_id}.",
                text_field(&payload, "plan_revision_id")
            ));
        }
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "artifact_blob_id".to_string(),
                JsonValue::String(local_blob_id),
            );
        }
        return Ok(payload);
    }
    let plan_revision_id = text_field(&payload, "plan_revision_id")
        .ok_or_else(|| format!("Remote plan {plan_id} revision is missing a plan_revision_id."))?;
    let cache_key = (plan_id.to_string(), plan_revision_id.clone());
    let detail = if let Some(cached) = remote_revision_detail_cache.get(&cache_key) {
        cached.clone()
    } else {
        let client = client.as_deref_mut().ok_or_else(|| {
            "Remote client is required to materialize remote revision detail.".to_string()
        })?;
        let fetched =
            get_plan_revision_with_plan_sync_remote_client(client, plan_id, &plan_revision_id)
                .map_err(|err| err.to_string())?;
        remote_revision_detail_cache.insert(cache_key, fetched.clone());
        fetched
    };
    if raw_string_field(&detail, "artifact_body").is_some() {
        return materialize_remote_revision(
            local_blob_store,
            plan_id,
            &detail,
            client,
            remote_revision_detail_cache,
        );
    }
    Ok(detail)
}

pub(super) fn materialize_remote_plan_head_for_local_adoption<B, C>(
    local_blob_store: &B,
    remote_plan: &JsonValue,
    client: Option<&mut C>,
    remote_revision_detail_cache: &mut BTreeMap<(String, String), JsonValue>,
) -> Result<JsonValue, String>
where
    B: PlanSyncLocalBlobStore + ?Sized,
    C: PlanSyncRemoteRevisionReader + ?Sized,
{
    let plan_id = require_plan_id(remote_plan)?;
    let head_revision_id = remote_head_revision_id(remote_plan)
        .ok_or_else(|| format!("Remote plan {plan_id} has no head revision to adopt locally."))?;
    let cache_key = (plan_id.clone(), head_revision_id.clone());
    let detail = if let Some(cached) = remote_revision_detail_cache.get(&cache_key) {
        cached.clone()
    } else {
        let client = client.ok_or_else(|| {
            format!(
                "Remote client is required to verify plan {plan_id} head {head_revision_id} before local adoption."
            )
        })?;
        let fetched =
            get_plan_revision_with_plan_sync_remote_client(client, &plan_id, &head_revision_id)
                .map_err(|err| err.to_string())?;
        remote_revision_detail_cache.insert(cache_key, fetched.clone());
        fetched
    };

    let observed_revision_id = require_plan_revision_id(&detail)?;
    if observed_revision_id != head_revision_id {
        return Err(format!(
            "Remote plan {plan_id} head lookup requested {head_revision_id}, but returned {observed_revision_id}; refusing local adoption."
        ));
    }
    if let Some(observed_plan_id) = text_field(&detail, "plan_id") {
        if observed_plan_id != plan_id {
            return Err(format!(
                "Remote plan {plan_id} head {head_revision_id} belongs to {observed_plan_id}; refusing local adoption."
            ));
        }
    }
    let artifact_path = text_field(&detail, "artifact_path").ok_or_else(|| {
        format!(
            "Remote plan {plan_id} head {head_revision_id} is missing its artifact path; refusing local adoption."
        )
    })?;
    if let Some(indexed_artifact_path) = head_text(remote_plan, "artifact_path") {
        if indexed_artifact_path != artifact_path {
            return Err(format!(
                "Remote plan {plan_id} head {head_revision_id} detail tracks {artifact_path}, but inventory tracks {indexed_artifact_path}; refusing local adoption."
            ));
        }
    }
    let artifact_selector = text_field(&detail, "artifact_selector");
    let indexed_artifact_selector = head_text(remote_plan, "artifact_selector");
    if indexed_artifact_selector != artifact_selector {
        return Err(format!(
            "Remote plan {plan_id} head {head_revision_id} detail selector {artifact_selector:?} does not match inventory selector {indexed_artifact_selector:?}; refusing local adoption."
        ));
    }
    let declared_blob_id = text_field(&detail, "artifact_blob_id").ok_or_else(|| {
        format!(
            "Remote plan {plan_id} head {head_revision_id} has no declared artifact blob identity; refusing local adoption."
        )
    })?;
    if let Some(indexed_blob_id) = head_text(remote_plan, "artifact_blob_id") {
        if indexed_blob_id != declared_blob_id {
            return Err(format!(
                "Remote plan {plan_id} head {head_revision_id} detail declares {declared_blob_id}, but inventory declares {indexed_blob_id}; refusing local adoption."
            ));
        }
    }
    if raw_string_field(&detail, "artifact_body").is_none() {
        return Err(format!(
            "Remote plan {plan_id} head {head_revision_id} has no artifact body; refusing local adoption."
        ));
    }
    as_array(value_get(&detail, "items")).map_err(|_| {
        format!(
            "Remote plan {plan_id} head {head_revision_id} has no valid checklist items array; refusing local adoption."
        )
    })?;

    materialize_remote_revision(
        local_blob_store,
        &plan_id,
        &detail,
        Option::<&mut C>::None,
        remote_revision_detail_cache,
    )
}

pub(super) fn bind_equivalent_local_plan_to_remote_identity<L, I>(
    local_identity_store: &L,
    identity_source: &I,
    request: &SyncRequest,
    local_plan: &JsonValue,
    remote_plan: &JsonValue,
    remote_revisions: &[JsonValue],
) -> Result<Option<JsonValue>, String>
where
    L: PlanSyncLocalIdentityRebindStore + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
{
    let local_plan_id = LocalPlanId::from_plan(local_plan)?;
    let remote_plan_id = RemotePlanId::from_plan(remote_plan)?;
    let local_revisions =
        list_plan_revisions_with_plan_sync_local_store(local_identity_store, local_plan_id.raw())?;
    let Some(revision_mappings) = equivalent_revision_mappings(&local_revisions, remote_revisions)?
    else {
        return Ok(None);
    };
    let local_receipt_plan_id = if local_identity_store
        .remote_adoption_preserves_local_plan_identity()
        || local_plan_id.raw() == remote_plan_id.raw()
    {
        local_plan_id.raw().to_string()
    } else {
        let rekeyed_at = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
        rekey_plan_with_plan_sync_local_lifecycle_store(
            local_identity_store,
            local_plan_id.raw(),
            remote_plan_id.raw(),
            rekeyed_at.as_str(),
        )?;
        remote_plan_id.raw().to_string()
    };
    let published_at = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
    let rebound = mark_plan_published_with_plan_sync_local_store(
        local_identity_store,
        &local_receipt_plan_id,
        request.remote_name.as_deref(),
        remote_plan_id.raw(),
        remote_head_revision_id(remote_plan).as_deref(),
        &revision_mappings,
        published_at.as_str(),
    )?;
    Ok(Some(rebound))
}

pub(super) fn validate_exact_mixed_local_plan_lineage_split(
    artifact: &SyncArtifact,
    local_plan: &JsonValue,
    local_revisions: &[JsonValue],
    bound_remote_plan_id: &str,
    bound_remote_revisions: &[JsonValue],
) -> Result<(), String> {
    let local_plan_id = require_plan_id(local_plan)?;
    if text_field(local_plan, "publication_state").as_deref() != Some("published") {
        return Err(format!(
            "Local plan {local_plan_id} is not published; refusing mixed-lineage split recovery."
        ));
    }
    let recorded_remote_plan_id = text_field(local_plan, "published_plan_id").ok_or_else(|| {
        format!(
            "Local plan {local_plan_id} has no published remote Plan receipt; refusing mixed-lineage split recovery."
        )
    })?;
    if recorded_remote_plan_id != bound_remote_plan_id {
        return Err(format!(
            "Local plan {local_plan_id} records remote {recorded_remote_plan_id}, but recovery loaded {bound_remote_plan_id}; refusing mixed-lineage split recovery."
        ));
    }
    let published_head_revision_id = text_field(local_plan, "published_head_revision_id")
        .ok_or_else(|| {
            format!(
                "Local plan {local_plan_id} has no published remote head receipt; refusing mixed-lineage split recovery."
            )
        })?;
    let local_head_artifact_path = head_text(local_plan, "artifact_path").ok_or_else(|| {
        format!(
            "Local plan {local_plan_id} has no head artifact path; refusing mixed-lineage split recovery."
        )
    })?;
    let local_head_artifact_selector = head_text(local_plan, "artifact_selector");
    if local_head_artifact_path != artifact.artifact_path
        || local_head_artifact_selector != artifact.artifact_selector
    {
        return Err(format!(
            "Local plan {local_plan_id} head does not match {}; refusing mixed-lineage split recovery.",
            plan_artifact_identity_label(
                &artifact.artifact_path,
                artifact.artifact_selector.as_deref()
            )
        ));
    }

    let ordered_local = sort_revisions_ascending(local_revisions.to_vec());
    if ordered_local.is_empty() {
        return Err(format!(
            "Local plan {local_plan_id} has no revisions; refusing mixed-lineage split recovery."
        ));
    }
    let local_head_revision_id = local_head_revision_id(local_plan).ok_or_else(|| {
        format!(
            "Local plan {local_plan_id} has no head revision; refusing mixed-lineage split recovery."
        )
    })?;
    if require_plan_revision_id(ordered_local.last().unwrap_or(&JsonValue::Null))?
        != local_head_revision_id
    {
        return Err(format!(
            "Local plan {local_plan_id} revision inventory does not end at head {local_head_revision_id}; refusing mixed-lineage split recovery."
        ));
    }

    let ordered_remote = sort_revisions_ascending(bound_remote_revisions.to_vec());
    if ordered_remote.is_empty() {
        return Err(format!(
            "Bound remote plan {bound_remote_plan_id} has no revisions; refusing mixed-lineage split recovery."
        ));
    }
    let mut remote_revision_indexes = BTreeMap::new();
    for (index, remote_revision) in ordered_remote.iter().enumerate() {
        let remote_plan_id = text_field(remote_revision, "plan_id").ok_or_else(|| {
            format!(
                "Bound remote plan {bound_remote_plan_id} revision is missing plan_id; refusing mixed-lineage split recovery."
            )
        })?;
        if remote_plan_id != bound_remote_plan_id {
            return Err(format!(
                "Bound remote plan {bound_remote_plan_id} revision belongs to {remote_plan_id}; refusing mixed-lineage split recovery."
            ));
        }
        let remote_revision_id = require_plan_revision_id(remote_revision)?;
        if remote_revision_indexes
            .insert(remote_revision_id.clone(), index)
            .is_some()
        {
            return Err(format!(
                "Bound remote plan {bound_remote_plan_id} contains duplicate revision {remote_revision_id}; refusing mixed-lineage split recovery."
            ));
        }
    }

    let mut saw_local_draft = false;
    let mut published_revision_count = 0usize;
    let mut previous_remote_index = None;
    let mut last_published_local_revision = None;
    let mut last_published_remote_revision_id = None;
    for local_revision in &ordered_local {
        let publication_state = text_field(local_revision, "publication_state").ok_or_else(|| {
            format!(
                "Local plan {local_plan_id} revision {} has no publication_state; refusing mixed-lineage split recovery.",
                require_plan_revision_id(local_revision).unwrap_or_else(|_| "<unknown>".to_string())
            )
        })?;
        if publication_state != "published" {
            let draft_path = text_field(local_revision, "artifact_path").ok_or_else(|| {
                format!(
                    "Local plan {local_plan_id} draft revision {} has no artifact path; refusing mixed-lineage split recovery.",
                    require_plan_revision_id(local_revision)
                        .unwrap_or_else(|_| "<unknown>".to_string())
                )
            })?;
            let draft_selector = text_field(local_revision, "artifact_selector");
            if !plan_lineage_identity_matches(
                &local_head_artifact_path,
                local_head_artifact_selector.as_deref(),
                &draft_path,
                draft_selector.as_deref(),
            ) {
                return Err(format!(
                    "Local plan {local_plan_id} draft tail mixes {} with {}; refusing mixed-lineage split recovery.",
                    plan_artifact_identity_label(
                        &local_head_artifact_path,
                        local_head_artifact_selector.as_deref()
                    ),
                    plan_artifact_identity_label(&draft_path, draft_selector.as_deref())
                ));
            }
            saw_local_draft = true;
            continue;
        }
        if saw_local_draft {
            return Err(format!(
                "Local plan {local_plan_id} has a published revision after a local draft; refusing mixed-lineage split recovery."
            ));
        }
        let local_revision_id = require_plan_revision_id(local_revision)?;
        let remote_revision_id = text_field(local_revision, "published_plan_revision_id")
            .ok_or_else(|| {
                format!(
                    "Local plan {local_plan_id} published revision {local_revision_id} has no exact remote revision receipt; refusing mixed-lineage split recovery."
                )
            })?;
        let remote_index = *remote_revision_indexes
            .get(&remote_revision_id)
            .ok_or_else(|| {
                format!(
                    "Local plan {local_plan_id} published revision {local_revision_id} targets missing bound remote revision {remote_revision_id}; refusing mixed-lineage split recovery."
                )
            })?;
        if previous_remote_index.is_some_and(|previous| remote_index != previous + 1) {
            return Err(format!(
                "Local plan {local_plan_id} published revision receipts are not a contiguous ordered boundary in remote {bound_remote_plan_id}; refusing mixed-lineage split recovery."
            ));
        }
        let remote_revision = &ordered_remote[remote_index];
        if !plan_publish_revision_metadata_matches(local_revision, remote_revision) {
            return Err(format!(
                "Local plan {local_plan_id} published revision {local_revision_id} does not match bound remote revision {remote_revision_id}; refusing mixed-lineage split recovery."
            ));
        }
        previous_remote_index = Some(remote_index);
        published_revision_count += 1;
        last_published_local_revision = Some(local_revision);
        last_published_remote_revision_id = Some(remote_revision_id);
    }
    if published_revision_count == 0 {
        return Err(format!(
            "Local plan {local_plan_id} has no published revisions to preserve; refusing mixed-lineage split recovery."
        ));
    }
    if !saw_local_draft {
        return Err(format!(
            "Local plan {local_plan_id} has no local draft tail to split; refusing mixed-lineage split recovery."
        ));
    }
    if last_published_remote_revision_id.as_deref() != Some(&published_head_revision_id) {
        return Err(format!(
            "Local plan {local_plan_id} published revision receipts do not end at recorded remote head {published_head_revision_id}; refusing mixed-lineage split recovery."
        ));
    }
    let published_artifact_path =
        last_published_local_revision.and_then(|revision| text_field(revision, "artifact_path"));
    let published_artifact_selector = last_published_local_revision
        .and_then(|revision| text_field(revision, "artifact_selector"));
    if published_artifact_path.as_deref() == Some(local_head_artifact_path.as_str())
        && published_artifact_selector == local_head_artifact_selector
    {
        return Err(format!(
            "Local plan {local_plan_id} published and draft heads track the same artifact; refusing mixed-lineage split recovery."
        ));
    }
    Ok(())
}

pub(super) fn select_existing_plan_with_continuity(
    repo_root: &Path,
    artifact: &SyncArtifact,
    indexed_by_identity: &BTreeMap<(String, Option<String>), Vec<JsonValue>>,
    plans: &[JsonValue],
) -> Result<(Option<JsonValue>, Option<JsonValue>), String> {
    let key = (
        artifact.artifact_path.clone(),
        artifact.artifact_selector.clone(),
    );
    let direct = select_sync_existing_plan(
        &plan_artifact_identity_label(
            &artifact.artifact_path,
            artifact.artifact_selector.as_deref(),
        ),
        indexed_by_identity.get(&key).cloned().unwrap_or_default(),
    )?;
    if direct.is_some() {
        return Ok((direct, None));
    }
    if let Some(selector) = &artifact.artifact_selector {
        let selector_matches = open_plans_matching_selector(plans, selector);
        if selector_matches.is_empty() {
            return Ok((None, None));
        }
        if selector_matches.len() > 1 {
            return Err(format!(
                "Multiple open plans already expose selector {}: {}",
                selector,
                selector_matches
                    .iter()
                    .filter_map(|row| text_field(row, "plan_id"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let matched = selector_matches[0].clone();
        let previous_artifact_path = head_text(&matched, "artifact_path");
        if let Some(previous_artifact_path) = previous_artifact_path {
            if repo_root.join(&previous_artifact_path).exists() {
                return Err(continuity_conflict_due_to_existing_source_path(
                    text_field(&matched, "plan_id").unwrap_or_default().as_str(),
                    &previous_artifact_path,
                    &artifact.artifact_path,
                    Some(selector.as_str()),
                ));
            }
            return Ok((
                Some(matched),
                Some(json!({
                    "match_kind": "artifact_selector_move",
                    "previous_artifact_path": previous_artifact_path,
                    "new_artifact_path": artifact.artifact_path,
                    "artifact_selector": selector,
                })),
            ));
        }
        return Ok((Some(matched), None));
    }
    if artifact.artifact_blob_id.is_empty() {
        return Ok((None, None));
    }
    let blob_matches = open_generic_plans_matching_blob_id(plans, &artifact.artifact_blob_id);
    if blob_matches.is_empty() {
        return Ok((None, None));
    }
    if blob_matches.len() > 1 {
        return Err(format!(
            "Multiple open generic Markdown plans share exact blob {}; rename/move continuity for {} is ambiguous.",
            artifact.artifact_blob_id, artifact.artifact_path
        ));
    }
    let matched = blob_matches[0].clone();
    let previous_artifact_path = head_text(&matched, "artifact_path");
    if let Some(previous_artifact_path) = previous_artifact_path.clone() {
        if repo_root.join(&previous_artifact_path).exists() {
            return Err(continuity_conflict_due_to_existing_source_path(
                text_field(&matched, "plan_id").unwrap_or_default().as_str(),
                &previous_artifact_path,
                &artifact.artifact_path,
                None,
            ));
        }
    }
    Ok((
        Some(matched),
        previous_artifact_path.map(|previous_artifact_path| {
            json!({
                "match_kind": "exact_blob_move",
                "previous_artifact_path": previous_artifact_path,
                "new_artifact_path": artifact.artifact_path,
                "artifact_blob_id": artifact.artifact_blob_id,
            })
        }),
    ))
}

pub(super) fn select_sync_existing_plan(
    artifact_label: &str,
    candidates: Vec<JsonValue>,
) -> Result<Option<JsonValue>, String> {
    let current_candidates = open_candidates(&candidates);
    if current_candidates.len() > 1 {
        return Err(format!(
            "Multiple current plans already track {}: {}",
            artifact_label,
            current_candidates
                .iter()
                .filter_map(|row| text_field(row, "plan_id"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(current_candidates.into_iter().next())
}

pub(super) fn open_candidates(plans: &[JsonValue]) -> Vec<JsonValue> {
    plans
        .iter()
        .filter(|row| !is_historical_status(value_get(row, "status")))
        .cloned()
        .collect()
}

pub(super) fn open_plans_matching_selector(plans: &[JsonValue], selector: &str) -> Vec<JsonValue> {
    plans
        .iter()
        .filter(|row| {
            !is_historical_status(value_get(row, "status"))
                && head_text(row, "artifact_selector").as_deref() == Some(selector)
        })
        .cloned()
        .collect()
}

pub(super) fn open_generic_plans_matching_blob_id(
    plans: &[JsonValue],
    blob_id: &str,
) -> Vec<JsonValue> {
    plans
        .iter()
        .filter(|row| {
            !is_historical_status(value_get(row, "status"))
                && head_text(row, "artifact_selector").is_none()
                && head_text(row, "artifact_blob_id").as_deref() == Some(blob_id)
        })
        .cloned()
        .collect()
}

pub(super) fn index_plans_by_path(plans: &[JsonValue]) -> BTreeMap<String, Vec<JsonValue>> {
    let mut indexed = BTreeMap::new();
    for plan in plans {
        let Some(artifact_path) = head_text(plan, "artifact_path") else {
            continue;
        };
        indexed
            .entry(artifact_path)
            .or_insert_with(Vec::new)
            .push(plan.clone());
    }
    indexed
}

pub(super) fn index_plans_by_identity(
    plans: &[JsonValue],
) -> BTreeMap<(String, Option<String>), Vec<JsonValue>> {
    let mut indexed = BTreeMap::new();
    for plan in plans {
        let Some(artifact_path) = head_text(plan, "artifact_path") else {
            continue;
        };
        let key = (artifact_path, head_text(plan, "artifact_selector"));
        indexed
            .entry(key)
            .or_insert_with(Vec::new)
            .push(plan.clone());
    }
    indexed
}

pub(super) fn continuity_conflict_due_to_existing_source_path(
    plan_id: &str,
    previous_artifact_path: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
) -> String {
    let selector_detail = artifact_selector
        .map(|value| format!(" [{value}]"))
        .unwrap_or_default();
    format!(
        "Tracked plan {plan_id} still points at {previous_artifact_path}{selector_detail}; rename/move continuity to {artifact_path} is only allowed after the previously tracked Markdown path disappears."
    )
}

pub(super) fn tracked_missing_markdown_artifact_paths<S>(
    local_artifact_state_source: &S,
    request: &SyncRequest,
    sync_target: &SyncTarget,
    artifact_paths: BTreeSet<String>,
    synced_artifact_paths: &BTreeSet<String>,
) -> Result<BTreeSet<String>, String>
where
    S: PlanSyncLocalArtifactStateSource + ?Sized,
{
    Ok(plan_sync_missing_artifact_paths(
        &artifact_paths,
        &sync_target.scope,
        &sync_target.target_path,
        synced_artifact_paths,
        &existing_artifact_paths_with_plan_sync_local_artifact_state_source(
            local_artifact_state_source,
            &request.root_path,
            &artifact_paths,
        )?,
        &ignored_artifact_paths_with_plan_sync_local_artifact_state_source(
            local_artifact_state_source,
            &request.root_path,
            &artifact_paths,
        )?,
        true,
    ))
}

pub(super) fn plan_sync_missing_artifact_paths(
    artifact_paths: &BTreeSet<String>,
    scope: &str,
    target_path: &str,
    synced_artifact_paths: &BTreeSet<String>,
    existing_artifact_paths: &BTreeSet<String>,
    ignored_artifact_paths: &BTreeSet<String>,
    markdown_only: bool,
) -> BTreeSet<String> {
    let mut deleted = BTreeSet::new();
    for artifact_path in artifact_paths {
        if synced_artifact_paths.contains(artifact_path) {
            continue;
        }
        if markdown_only && !artifact_path.ends_with(".md") {
            continue;
        }
        if !artifact_path_in_scope(
            artifact_path,
            scope,
            target_path,
            ignored_artifact_paths.contains(artifact_path),
        ) {
            continue;
        }
        if existing_artifact_paths.contains(artifact_path) {
            continue;
        }
        deleted.insert(artifact_path.clone());
    }
    deleted
}

pub(super) fn artifact_path_in_scope(
    artifact_path: &str,
    scope: &str,
    target_path: &str,
    ignored: bool,
) -> bool {
    if scope == "file" {
        return artifact_path == target_path;
    }
    let normalized_target = target_path.trim_end_matches('/');
    if normalized_target.is_empty() || normalized_target == "." {
        return !ignored;
    }
    artifact_path == normalized_target
        || artifact_path.starts_with(&format!("{normalized_target}/"))
}

pub(super) fn map_equivalent_remote_plan_revision_suffix(
    local_revisions: &[JsonValue],
    remote_revisions: &[JsonValue],
    expected_remote_head: Option<&str>,
) -> Result<Option<EquivalentRevisionSuffix>, String> {
    if local_revisions.is_empty() {
        return Ok(Some((Vec::new(), Vec::new())));
    }
    let Some(expected_remote_head) = expected_remote_head else {
        return Ok(None);
    };
    let Some(remote_head_index) = remote_revisions.iter().position(|revision| {
        text_field(revision, "plan_revision_id").as_deref() == Some(expected_remote_head)
    }) else {
        return Ok(None);
    };
    let remote_suffix = &remote_revisions[remote_head_index + 1..];
    let mut mappings = Vec::new();
    let mut next_local_index = 0usize;
    let mut last_matched_local_index: Option<usize> = None;
    for remote_revision in remote_suffix {
        let Some((matched_index, local_revision)) = local_revisions
            .iter()
            .enumerate()
            .skip(next_local_index)
            .find(|(_, local_revision)| {
                plan_publish_revision_metadata_matches(local_revision, remote_revision)
            })
        else {
            return Ok(None);
        };
        let remote_revision_id = text_field(remote_revision, "plan_revision_id")
            .ok_or_else(|| "Remote plan revision is missing plan_revision_id.".to_string())?;
        mappings.push((
            require_plan_revision_id(local_revision)?,
            remote_revision_id,
        ));
        next_local_index = matched_index + 1;
        last_matched_local_index = Some(matched_index);
    }
    let remaining = match last_matched_local_index {
        Some(index) => local_revisions[index + 1..].to_vec(),
        None => local_revisions.to_vec(),
    };
    Ok(Some((mappings, remaining)))
}

pub(super) fn map_exact_dense_remote_plan_revision_prefix(
    local_revisions: &[JsonValue],
    remote_revisions: &[JsonValue],
) -> Result<DenseRevisionPrefix, String> {
    let mut mappings = Vec::new();
    let mut mapped_count = 0usize;
    for local_revision in local_revisions {
        if text_field(local_revision, "publication_state").as_deref() != Some("published") {
            break;
        }
        let local_revision_id = require_plan_revision_id(local_revision)?;
        let mut exact_remote_matches = remote_revisions.iter().filter(|remote_revision| {
            text_field(remote_revision, "plan_revision_id").as_deref()
                == Some(local_revision_id.as_str())
        });
        let Some(remote_revision) = exact_remote_matches.next() else {
            break;
        };
        if exact_remote_matches.next().is_some() {
            return Err(format!(
                "Remote Plan history contains duplicate canonical revision ID {local_revision_id}; refusing publication receipt recovery."
            ));
        }
        if !plan_publish_revision_metadata_matches(local_revision, remote_revision) {
            return Err(format!(
                "Remote canonical revision {local_revision_id} does not match local artifact/checklist metadata; refusing publication receipt recovery."
            ));
        }
        mappings.push((local_revision_id.clone(), local_revision_id));
        mapped_count += 1;
    }
    let published_head_revision_id = mappings.last().map(|(_, remote_id)| remote_id.clone());
    Ok((
        mappings,
        local_revisions[mapped_count..].to_vec(),
        published_head_revision_id,
    ))
}

pub(super) fn select_divergent_retry_publish_target(
    local_revisions: &[JsonValue],
    remote_revisions: &[JsonValue],
    actual_remote_head: Option<&str>,
) -> Result<DivergentRetryPublishTarget, String> {
    if local_revisions.is_empty() {
        return Err("Reconcile requires at least one local plan revision".to_string());
    }
    let local_head = local_revisions.last().cloned().unwrap_or(JsonValue::Null);
    let local_head_revision_id = require_plan_revision_id(&local_head)?;
    let remote_head = actual_remote_head.and_then(|target_id| {
        remote_revisions
            .iter()
            .find(|revision| text_field(revision, "plan_revision_id").as_deref() == Some(target_id))
            .cloned()
    });
    if let Some(remote_head) = remote_head {
        if plan_publish_revision_metadata_matches(&local_head, &remote_head) {
            return Ok((
                vec![(
                    local_head_revision_id.clone(),
                    actual_remote_head.unwrap_or_default().to_string(),
                )],
                Vec::new(),
                json!({
                    "mode": "mapped_head",
                    "local_head_revision_id": local_head_revision_id,
                    "remote_head_revision_id": actual_remote_head.unwrap_or_default(),
                }),
            ));
        }
    }
    Ok((
        Vec::new(),
        vec![local_head],
        json!({
            "mode": "publish_head",
            "local_head_revision_id": local_head_revision_id,
            "remote_head_revision_id": actual_remote_head.unwrap_or_default(),
        }),
    ))
}

pub(super) fn equivalent_revision_mappings(
    local_revisions: &[JsonValue],
    remote_revisions: &[JsonValue],
) -> Result<Option<Vec<(String, String)>>, String> {
    let ordered_local = sort_revisions_ascending(local_revisions.to_vec());
    let ordered_remote = sort_revisions_ascending(remote_revisions.to_vec());
    if ordered_local.len() != ordered_remote.len() {
        return Ok(None);
    }
    let mut mappings = Vec::new();
    for (local_revision, remote_revision) in ordered_local.iter().zip(ordered_remote.iter()) {
        if !plan_publish_revision_metadata_matches(local_revision, remote_revision) {
            return Ok(None);
        }
        mappings.push((
            require_plan_revision_id(local_revision)?,
            text_field(remote_revision, "plan_revision_id")
                .ok_or_else(|| "Remote plan revision is missing plan_revision_id.".to_string())?,
        ));
    }
    Ok(Some(mappings))
}

pub(super) fn plan_publish_revision_metadata_matches(
    local_revision: &JsonValue,
    remote_revision: &JsonValue,
) -> bool {
    text_field(local_revision, "artifact_path") == text_field(remote_revision, "artifact_path")
        && text_field(local_revision, "artifact_selector")
            == text_field(remote_revision, "artifact_selector")
        && text_field(local_revision, "artifact_heading")
            == text_field(remote_revision, "artifact_heading")
        && text_field(local_revision, "title_snapshot")
            == text_field(remote_revision, "title_snapshot")
        && text_field(local_revision, "artifact_blob_id")
            == text_field(remote_revision, "artifact_blob_id")
        && plan_publish_item_metadata(local_revision) == plan_publish_item_metadata(remote_revision)
}

fn plan_publish_item_metadata(revision: &JsonValue) -> Vec<JsonValue> {
    const PUBLISHED_ITEM_FIELDS: [&str; 5] = [
        "plan_item_ref",
        "text",
        "checkbox_state",
        "heading_path",
        "line_number",
    ];

    as_array(value_get(revision, "items"))
        .unwrap_or_default()
        .iter()
        .map(|item| {
            let Some(object) = item.as_object() else {
                return item.clone();
            };
            let mut published = JsonMap::new();
            for field in PUBLISHED_ITEM_FIELDS {
                if let Some(value) = object.get(field) {
                    published.insert(field.to_string(), value.clone());
                }
            }
            JsonValue::Object(published)
        })
        .collect()
}

pub(super) fn load_remote_revisions_cached<C>(
    client: Option<&mut C>,
    cache: &mut BTreeMap<String, Vec<JsonValue>>,
    plan_id: &str,
) -> Result<Vec<JsonValue>, String>
where
    C: PlanSyncRemoteRevisionLister + ?Sized,
{
    if let Some(rows) = cache.get(plan_id) {
        return Ok(rows.clone());
    }
    let client =
        client.ok_or_else(|| "Remote client is required to load plan revisions.".to_string())?;
    let rows = list_plan_revisions_with_plan_sync_remote_client(client, plan_id)
        .map_err(|err| err.to_string())?;
    cache.insert(plan_id.to_string(), rows.clone());
    Ok(rows)
}

pub(super) fn sort_revisions_ascending(mut rows: Vec<JsonValue>) -> Vec<JsonValue> {
    rows.sort_by_key(|row| {
        value_get(row, "revision_number")
            .and_then(|value| value.as_i64())
            .unwrap_or(0)
    });
    rows
}

pub(super) fn replace_plan_in_inventory(
    inventory: &mut LocalInventory,
    previous_plan_id: &str,
    replacement: &JsonValue,
) {
    inventory.plans = inventory
        .plans
        .iter()
        .filter(|row| text_field(row, "plan_id").as_deref() != Some(previous_plan_id))
        .cloned()
        .collect();
    inventory.plans.push(replacement.clone());
    inventory.indexed_by_identity = index_plans_by_identity(&inventory.plans);
    inventory.indexed_plans = index_plans_by_path(&inventory.plans);
}
