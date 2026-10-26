use super::*;

fn validate_selected_plan_lineage(
    existing_plan: &JsonValue,
    artifact: &SyncArtifact,
) -> Result<(), String> {
    let local_plan_id = LocalPlanId::from_plan(existing_plan)?;
    let existing_path = head_text(existing_plan, "artifact_path").ok_or_else(|| {
        format!(
            "Local Plan {} has no head artifact path; refusing revision.",
            local_plan_id.reference()
        )
    })?;
    let existing_selector = head_text(existing_plan, "artifact_selector");
    if !plan_lineage_identity_matches(
        &existing_path,
        existing_selector.as_deref(),
        &artifact.artifact_path,
        artifact.artifact_selector.as_deref(),
    ) {
        return Err(format!(
            "Local Plan {} tracks {}, but sync selected it for {}; refusing a cross-lineage revision before writing content.",
            local_plan_id.reference(),
            plan_artifact_identity_label(&existing_path, existing_selector.as_deref()),
            plan_artifact_identity_label(
                &artifact.artifact_path,
                artifact.artifact_selector.as_deref()
            )
        ));
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "prune orchestration keeps independently substitutable ports and caches explicit"
)]
pub(super) fn run_prune_phase<W, B, S, I, C>(
    local_writer: &W,
    local_blob_store: &B,
    local_artifact_state_source: &S,
    identity_source: &I,
    request: &SyncRequest,
    sync_target: &SyncTarget,
    local_inventory: &mut LocalInventory,
    remote_inventory: &RemoteInventory,
    mut client: Option<&mut C>,
    remote_revisions_cache: &mut BTreeMap<String, Vec<JsonValue>>,
    remote_revision_detail_cache: &mut BTreeMap<(String, String), JsonValue>,
    synced_artifact_paths: &BTreeSet<String>,
) -> Result<(Vec<JsonValue>, Vec<JsonValue>), String>
where
    W: PlanSyncLocalAdoptionStore
        + PlanSyncLocalIdentityRebindStore
        + PlanSyncLocalInventoryStore
        + ?Sized,
    B: PlanSyncLocalBlobStore + ?Sized,
    S: PlanSyncLocalArtifactStateSource + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
    C: PlanSyncRemoteContinuitySource + ?Sized,
{
    let deleted_artifact_paths = tracked_missing_markdown_artifact_paths(
        local_artifact_state_source,
        request,
        sync_target,
        local_inventory
            .indexed_plans
            .keys()
            .chain(remote_inventory.indexed_plans.keys())
            .cloned()
            .collect(),
        synced_artifact_paths,
    )?;
    let mut adoptions = Vec::new();
    if request.base_url.is_some() {
        for artifact_path in deleted_artifact_paths.iter().filter(|value| {
            remote_inventory
                .indexed_plans
                .contains_key((*value).as_str())
        }) {
            let selectors = remote_inventory
                .indexed_plans
                .get(artifact_path.as_str())
                .into_iter()
                .flat_map(|rows| rows.iter())
                .filter_map(|plan| head_text(plan, "artifact_selector"))
                .collect::<BTreeSet<_>>();
            for selector in selectors {
                let deletion_artifact = SyncArtifact {
                    artifact_path: artifact_path.clone(),
                    artifact_selector: Some(selector.clone()),
                    artifact_heading: String::new(),
                    items: Vec::new(),
                    artifact_body: String::new(),
                    artifact_blob_id: String::new(),
                };
                let (_, adoption, _) = resolve_local_sync_plan_candidate(
                    local_writer,
                    local_blob_store,
                    identity_source,
                    request,
                    &deletion_artifact,
                    local_inventory,
                    &mut remote_inventory.clone(),
                    client.as_deref_mut(),
                    remote_revisions_cache,
                    remote_revision_detail_cache,
                )?;
                if let Some(row) = adoption {
                    adoptions.push(row);
                }
            }
        }
    }
    *local_inventory = load_local_inventory_from_store(local_writer)?;
    let pruned = prune_missing_plan_artifacts(
        local_writer,
        local_artifact_state_source,
        identity_source,
        request,
        sync_target,
        &local_inventory.indexed_plans,
        synced_artifact_paths,
    )?;
    Ok((pruned, adoptions))
}

pub(super) fn sync_single_plan_artifact<W, B, I>(
    local_writer: &W,
    local_blob_store: &B,
    identity_source: &I,
    request: &SyncRequest,
    artifact: &SyncArtifact,
    existing_plan: Option<&JsonValue>,
    continuity_match: Option<JsonValue>,
) -> Result<JsonValue, String>
where
    W: PlanSyncLocalArtifactWriter + ?Sized,
    B: PlanSyncLocalBlobStore + PlanSyncZstdPackStore + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
{
    if let Some(existing_plan) = existing_plan {
        validate_selected_plan_lineage(existing_plan, artifact)?;
    }
    let local_artifact_blob_id = ensure_blob_bytes_with_plan_sync_local_blob_store(
        local_blob_store,
        artifact.artifact_body.as_bytes(),
        Some(&artifact.artifact_path),
    )?;
    if existing_plan.is_none() {
        let plan_id = workflow_id_with_plan_sync_workflow_identity_source(
            identity_source,
            "PL",
            request.id_namespace_prefix.as_deref(),
        )?;
        let revision_id = workflow_id_with_plan_sync_workflow_identity_source(
            identity_source,
            "PR",
            request.id_namespace_prefix.as_deref(),
        )?;
        let items_json = JsonCodec::encode_value(
            &JsonValue::Array(artifact.items.clone()),
            JsonEncodeOptions::compact(),
        )
        .map_err(String::from)?;
        let now = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
        let artifact_root = prepare_artifact_tree_root_locator_with_plan_sync_zstd_pack_store(
            local_blob_store,
            &revision_id,
            &artifact.artifact_path,
            local_artifact_blob_id.as_str(),
            i64::try_from(artifact.artifact_body.len()).map_err(|_| {
                format!(
                    "Plan sync artifact {} byte count exceeds i64::MAX.",
                    artifact.artifact_path
                )
            })?,
            now.as_str(),
        )?;
        let data = create_plan_with_plan_sync_local_artifact_writer(
            local_writer,
            &PlanSyncLocalPlanCreate {
                plan_id: &plan_id,
                plan_revision_id: &revision_id,
                repo_name: &request.repo_name,
                title: &artifact.artifact_heading,
                artifact_path: &artifact.artifact_path,
                artifact_selector: artifact.artifact_selector.as_deref(),
                artifact_heading: &artifact.artifact_heading,
                items_json: &items_json,
                artifact_blob_id: Some(local_artifact_blob_id.as_str()),
                artifact_root,
                summary: None,
                status: DEFAULT_PLAN_STATUS,
                source_kind: DEFAULT_SOURCE_KIND,
                created_by: request.created_by.as_deref(),
                actor_type: DEFAULT_ACTOR_TYPE,
                publication_state: LOCAL_DRAFT_PUBLICATION_STATE,
                now: now.as_str(),
            },
        )?;
        return Ok(plan_sync_result_row(
            "created",
            &artifact.artifact_path,
            artifact.artifact_selector.as_deref(),
            &data,
            continuity_match,
        ));
    }
    let existing = existing_plan.cloned().unwrap_or(JsonValue::Null);
    if plan_matches_sync_artifact(&existing, &artifact_to_json(artifact), false)
        .map_err(|err| err.to_string())?
    {
        return Ok(plan_sync_result_row(
            "unchanged",
            &artifact.artifact_path,
            artifact.artifact_selector.as_deref(),
            &existing,
            continuity_match,
        ));
    }
    let plan_id = require_plan_id(&existing)?;
    let revision_id = workflow_id_with_plan_sync_workflow_identity_source(
        identity_source,
        "PR",
        request.id_namespace_prefix.as_deref(),
    )?;
    let items_json = JsonCodec::encode_value(
        &JsonValue::Array(artifact.items.clone()),
        JsonEncodeOptions::compact(),
    )
    .map_err(String::from)?;
    let now = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
    let artifact_root = prepare_artifact_tree_root_locator_with_plan_sync_zstd_pack_store(
        local_blob_store,
        &revision_id,
        &artifact.artifact_path,
        local_artifact_blob_id.as_str(),
        i64::try_from(artifact.artifact_body.len()).map_err(|_| {
            format!(
                "Plan sync artifact {} byte count exceeds i64::MAX.",
                artifact.artifact_path
            )
        })?,
        now.as_str(),
    )?;
    let data = revise_plan_with_plan_sync_local_artifact_writer(
        local_writer,
        &PlanSyncLocalPlanRevision {
            plan_id: &plan_id,
            plan_revision_id: &revision_id,
            artifact_path: &artifact.artifact_path,
            artifact_selector: artifact.artifact_selector.as_deref(),
            artifact_heading: &artifact.artifact_heading,
            items_json: &items_json,
            artifact_blob_id: Some(local_artifact_blob_id.as_str()),
            artifact_root,
            title: Some(&artifact.artifact_heading),
            summary: None,
            source_kind: DEFAULT_SOURCE_KIND,
            created_by: request.created_by.as_deref(),
            actor_type: DEFAULT_ACTOR_TYPE,
            now: now.as_str(),
        },
    )?;
    Ok(plan_sync_result_row(
        "updated",
        &artifact.artifact_path,
        artifact.artifact_selector.as_deref(),
        &data,
        continuity_match,
    ))
}

pub(super) fn prune_missing_plan_artifacts<W, S, I>(
    local_lifecycle_store: &W,
    local_artifact_state_source: &S,
    identity_source: &I,
    request: &SyncRequest,
    sync_target: &SyncTarget,
    indexed_plans: &BTreeMap<String, Vec<JsonValue>>,
    synced_artifact_paths: &BTreeSet<String>,
) -> Result<Vec<JsonValue>, String>
where
    W: PlanSyncLocalLifecycleStore + ?Sized,
    S: PlanSyncLocalArtifactStateSource + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
{
    let candidate_paths = indexed_plans.keys().cloned().collect::<BTreeSet<_>>();
    let missing = plan_sync_missing_artifact_paths(
        &candidate_paths,
        &sync_target.scope,
        &sync_target.target_path,
        synced_artifact_paths,
        &existing_artifact_paths_with_plan_sync_local_artifact_state_source(
            local_artifact_state_source,
            &request.root_path,
            &candidate_paths,
        )?,
        &ignored_artifact_paths_with_plan_sync_local_artifact_state_source(
            local_artifact_state_source,
            &request.root_path,
            &candidate_paths,
        )?,
        false,
    );
    let mut results = Vec::new();
    for artifact_path in missing {
        let candidates = indexed_plans
            .get(artifact_path.as_str())
            .cloned()
            .unwrap_or_default();
        for plan in open_candidates(&candidates) {
            let plan_id = require_plan_id(&plan)?;
            let archived_at = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
            let data = close_plan_with_plan_sync_local_lifecycle_store(
                local_lifecycle_store,
                &plan_id,
                "archived",
                archived_at.as_str(),
            )?;
            results.push(plan_sync_result_row(
                "pruned",
                &artifact_path,
                head_text(&plan, "artifact_selector").as_deref(),
                &data,
                None,
            ));
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected_plan(path: &str, selector: Option<&str>) -> JsonValue {
        json!({
            "plan_id": "PR-52",
            "head_revision": {
                "artifact_path": path,
                "artifact_selector": selector,
            }
        })
    }

    fn sync_artifact(path: &str, selector: Option<&str>) -> SyncArtifact {
        SyncArtifact {
            artifact_path: path.to_string(),
            artifact_selector: selector.map(str::to_string),
            artifact_heading: "Lineage Guard".to_string(),
            items: Vec::new(),
            artifact_body: "# Lineage Guard\n".to_string(),
            artifact_blob_id: "BLB-lineage-guard".to_string(),
        }
    }

    #[test]
    fn selected_plan_guard_rejects_different_unselected_artifact_path() {
        let error = validate_selected_plan_lineage(
            &selected_plan("docs/old.md", None),
            &sync_artifact("docs/new.md", None),
        )
        .expect_err("a path-only Plan identity must not absorb another artifact");

        assert!(error.contains("LPR-52"));
        assert!(error.contains("cross-lineage revision"));
    }

    #[test]
    fn selected_plan_guard_rejects_different_stable_selectors() {
        let error = validate_selected_plan_lineage(
            &selected_plan("docs/plan.md", Some("plan/old/root")),
            &sync_artifact("docs/plan.md", Some("plan/new/root")),
        )
        .expect_err("different stable selectors must remain different Plan lineages");

        assert!(error.contains("LPR-52"));
        assert!(error.contains("plan/old/root"));
        assert!(error.contains("plan/new/root"));
    }

    #[test]
    fn selected_plan_guard_allows_path_move_for_same_stable_selector() {
        validate_selected_plan_lineage(
            &selected_plan("docs/old.md", Some("plan/stable/root")),
            &sync_artifact("docs/new.md", Some("plan/stable/root")),
        )
        .expect("a stable Plan selector remains authoritative across a path move");
    }
}
