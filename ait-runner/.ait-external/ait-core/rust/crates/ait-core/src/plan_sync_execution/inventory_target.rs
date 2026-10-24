use super::*;

type LocalSyncPlanCandidate = (Option<JsonValue>, Option<JsonValue>, Option<JsonValue>);

fn validate_rebase_remote_head_artifact_identity(
    remote_plan_id: &str,
    artifact: &SyncArtifact,
    remote_head: &JsonValue,
) -> Result<(), String> {
    let remote_artifact_path = text_field(remote_head, "artifact_path").ok_or_else(|| {
        format!(
            "Remote plan {remote_plan_id} verified head has no artifact path; refusing local rebase adoption."
        )
    })?;
    let remote_artifact_selector = text_field(remote_head, "artifact_selector");
    if remote_artifact_path != artifact.artifact_path
        || remote_artifact_selector != artifact.artifact_selector
    {
        return Err(format!(
            "Remote plan {remote_plan_id} verified head does not match {}; refusing local rebase adoption.",
            plan_artifact_identity_label(
                &artifact.artifact_path,
                artifact.artifact_selector.as_deref()
            )
        ));
    }
    Ok(())
}

fn validate_exact_remote_head_artifact(
    remote_plan_id: &RemotePlanId,
    remote_plan: &JsonValue,
    artifact: &SyncArtifact,
    remote_head: &JsonValue,
) -> Result<(), String> {
    let remote_head_plan = json!({
        "title": text_field(remote_plan, "title"),
        "head_revision": remote_head.clone(),
    });
    if !plan_matches_sync_artifact(&remote_head_plan, &artifact_to_json(artifact), true)? {
        return Err(format!(
            "Remote Plan {} verified head does not exactly match {}; refusing local identity reset.",
            remote_plan_id.reference(),
            plan_artifact_identity_label(
                &artifact.artifact_path,
                artifact.artifact_selector.as_deref()
            )
        ));
    }
    Ok(())
}

fn validate_itemless_current_local_plan_for_identity_reset(
    local_plan_id: &LocalPlanId,
    local_plan: &JsonValue,
    local_revisions: &[JsonValue],
) -> Result<(), String> {
    let ordered = sort_revisions_ascending(local_revisions.to_vec());
    let last_revision = ordered.last().ok_or_else(|| {
        format!(
            "Local Plan {} has no revision history; refusing local identity reset.",
            local_plan_id.reference()
        )
    })?;
    let expected_head = local_head_revision_id(local_plan).ok_or_else(|| {
        format!(
            "Local Plan {} has no head revision; refusing local identity reset.",
            local_plan_id.reference()
        )
    })?;
    let observed_head = require_plan_revision_id(last_revision)?;
    if observed_head != expected_head {
        return Err(format!(
            "Local Plan {} revision inventory ends at {observed_head}, not head {expected_head}; refusing local identity reset.",
            local_plan_id.reference()
        ));
    }
    for revision in &ordered {
        let revision_id = require_plan_revision_id(revision)?;
        if !as_array(value_get(revision, "items"))?.is_empty() {
            return Err(format!(
                "Local Plan {} revision {revision_id} has checklist items; refusing local identity reset.",
                local_plan_id.reference()
            ));
        }
    }
    Ok(())
}

fn validate_populated_revision_receipts(
    plan_id: &LocalPlanId,
    local_revisions: &[JsonValue],
    remote_plan_id: &RemotePlanId,
    remote_revisions: &[JsonValue],
) -> Result<(), String> {
    let mut remote_by_id = BTreeMap::<String, (usize, &JsonValue)>::new();
    for (index, revision) in remote_revisions.iter().enumerate() {
        let revision_id = require_plan_revision_id(revision)?;
        if remote_by_id
            .insert(revision_id.clone(), (index, revision))
            .is_some()
        {
            return Err(format!(
                "Remote Plan {} contains duplicate revision {revision_id}.",
                remote_plan_id.reference()
            ));
        }
    }
    let mut previous_remote_index = None;
    for revision in local_revisions.iter().filter(|revision| {
        text_field(revision, "publication_state").as_deref() == Some("published")
    }) {
        let local_revision_id = require_plan_revision_id(revision)?;
        let Some(remote_revision_id) = text_field(revision, "published_plan_revision_id") else {
            continue;
        };
        let (remote_index, remote_revision) = remote_by_id
            .get(&remote_revision_id)
            .copied()
            .ok_or_else(|| {
                format!(
                    "Local Plan {} revision {local_revision_id} targets missing {} revision {remote_revision_id}.",
                    plan_id.reference(),
                    remote_plan_id.reference()
                )
            })?;
        if previous_remote_index.is_some_and(|previous| remote_index <= previous) {
            return Err(format!(
                "Local Plan {} populated revision receipts are not a strictly increasing history in {}.",
                plan_id.reference(),
                remote_plan_id.reference()
            ));
        }
        if !plan_publish_revision_metadata_matches(revision, remote_revision) {
            return Err(format!(
                "Local Plan {} revision {local_revision_id} does not match {} revision {remote_revision_id}.",
                plan_id.reference(),
                remote_plan_id.reference()
            ));
        }
        previous_remote_index = Some(remote_index);
    }
    Ok(())
}

fn populated_receipt_issue(
    local_plan_id: &LocalPlanId,
    local_plan: &JsonValue,
    local_revisions: &[JsonValue],
    remote_plan_id: &RemotePlanId,
    remote_revisions: &[JsonValue],
) -> Result<Option<String>, String> {
    validate_materialized_remote_plan_lineage(remote_plan_id, remote_revisions)?;
    if let Err(detail) = validate_populated_revision_receipts(
        local_plan_id,
        local_revisions,
        remote_plan_id,
        remote_revisions,
    ) {
        return Ok(Some(detail));
    }
    let published_revisions = local_revisions
        .iter()
        .filter(|revision| {
            text_field(revision, "publication_state").as_deref() == Some("published")
        })
        .collect::<Vec<_>>();
    if published_revisions.is_empty()
        || published_revisions
            .iter()
            .any(|revision| text_field(revision, "published_plan_revision_id").is_none())
    {
        return Ok(None);
    }
    let Some(recorded_head) = text_field(local_plan, "published_head_revision_id") else {
        return Ok(None);
    };
    let latest_revision_head = published_revisions
        .last()
        .and_then(|revision| text_field(revision, "published_plan_revision_id"));
    if latest_revision_head.as_deref() != Some(recorded_head.as_str()) {
        return Ok(Some(format!(
            "Local Plan {} populated Plan-level head {recorded_head} does not equal its latest populated revision receipt {:?} in {}.",
            local_plan_id.reference(),
            latest_revision_head,
            remote_plan_id.reference()
        )));
    }
    Ok(None)
}

pub(super) fn resolve_sync_target(
    request: &SyncRequest,
    allow_missing: bool,
) -> Result<SyncTarget, String> {
    let payload = resolve_repo_artifact_path(&request.root_path, &request.target, allow_missing)
        .map_err(plan_fs_error)?;
    let resolved_target = required_string_field(&payload, "resolved_path")?;
    let artifact_path = required_string_field(&payload, "artifact_path")?;
    let resolved_path = PathBuf::from(&resolved_target);
    if resolved_path.exists() && resolved_path.is_dir() {
        let mut files = Vec::new();
        let target_prefix = artifact_path.trim_end_matches('/');
        for rel_path in list_visible_markdown_artifact_paths(&request.root_path, None, None)
            .map_err(plan_fs_error)?
        {
            if target_prefix != "."
                && rel_path != target_prefix
                && !rel_path.starts_with(&format!("{target_prefix}/"))
            {
                continue;
            }
            if is_forbidden_sync_markdown_path(&rel_path) {
                continue;
            }
            files.push(Path::new(&request.root_path).join(rel_path));
        }
        files.sort();
        return Ok(SyncTarget {
            scope: "directory".to_string(),
            target_path: artifact_path.trim_end_matches('/').to_string(),
            resolved_target: resolved_path,
            files,
        });
    }
    if resolved_path.exists() && resolved_path.is_file() {
        if resolved_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| !value.eq_ignore_ascii_case("md"))
            .unwrap_or(true)
        {
            return Err(format!(
                "Plan sync target must be a Markdown file or directory: {}",
                request.target
            ));
        }
        if is_forbidden_sync_markdown_path(&artifact_path) {
            return Err(forbidden_sync_markdown_message(&artifact_path));
        }
        return Ok(SyncTarget {
            scope: "file".to_string(),
            target_path: artifact_path,
            resolved_target: resolved_path.clone(),
            files: vec![resolved_path],
        });
    }
    if allow_missing && artifact_path.ends_with(".md") {
        if is_forbidden_sync_markdown_path(&artifact_path) {
            return Err(forbidden_sync_markdown_message(&artifact_path));
        }
        return Ok(SyncTarget {
            scope: "file".to_string(),
            target_path: artifact_path,
            resolved_target: resolved_path,
            files: Vec::new(),
        });
    }
    if allow_missing {
        return Err(format!(
            "Missing sync target {}. Use an existing directory, or point plan sync at one specific Markdown path when publishing or pruning a deletion.",
            request.target
        ));
    }
    Err(format!(
        "Plan sync target does not exist: {}",
        request.target
    ))
}

pub(super) fn resolve_plan_artifact(
    request: &SyncRequest,
    body_file: &Path,
    plan_ref: Option<&str>,
    allow_generic_markdown: bool,
) -> Result<SyncArtifact, String> {
    let payload = resolve_repo_artifact_path(
        &request.root_path,
        body_file.to_string_lossy().as_ref(),
        false,
    )
    .map_err(plan_fs_error)?;
    let artifact_path = required_string_field(&payload, "artifact_path")?;
    let resolved_path = required_string_field(&payload, "resolved_path")?;
    let file_path = PathBuf::from(resolved_path);
    if !file_path.is_file() {
        return Err(format!("Plan file must be a file: {}", body_file.display()));
    }
    if file_path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| !value.eq_ignore_ascii_case("md"))
        .unwrap_or(true)
    {
        return Err(format!(
            "Plan file must be a Markdown file: {}",
            body_file.display()
        ));
    }
    let markdown =
        read_utf8_text_file(file_path.to_string_lossy().as_ref()).map_err(plan_fs_error)?;
    let known_refs = list_plan_section_refs(Some(markdown.as_str()));
    let mut normalized_plan_ref = plan_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if normalized_plan_ref.is_none() {
        if known_refs.len() == 1 {
            normalized_plan_ref = Some(known_refs[0].plan_ref.clone());
        } else if known_refs.is_empty() {
            if allow_generic_markdown {
                return Ok(SyncArtifact {
                    artifact_path: artifact_path.clone(),
                    artifact_selector: None,
                    artifact_heading: default_markdown_artifact_heading(&markdown, &artifact_path),
                    items: Vec::new(),
                    artifact_body: markdown.clone(),
                    artifact_blob_id: artifact_blob_id(&markdown),
                });
            }
            return Err(format!(
                "{artifact_path} does not expose any `[plan-ref: ...]` section headings yet."
            ));
        } else {
            return Err(format!(
                "`--plan-ref` is required for {artifact_path} because it exposes multiple `[plan-ref: ...]` sections. Known refs: {}",
                known_refs
                    .iter()
                    .map(|entry| entry.plan_ref.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let selected_ref = normalized_plan_ref.unwrap_or_default();
    let section = extract_plan_section(Some(markdown.as_str()), Some(selected_ref.as_str()))
        .ok_or_else(|| {
            if known_refs.is_empty() {
                format!(
                    "{artifact_path} does not expose any `[plan-ref: ...]` section headings yet."
                )
            } else {
                format!(
                    "Plan ref {:?} is not present in {}. Known refs: {}",
                    selected_ref,
                    artifact_path,
                    known_refs
                        .iter()
                        .map(|entry| entry.plan_ref.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        })?;
    Ok(SyncArtifact {
        artifact_path,
        artifact_selector: Some(section.plan_ref.clone()),
        artifact_heading: section.heading_title.clone(),
        items: section.items.iter().map(plan_item_to_json).collect(),
        artifact_body: markdown.clone(),
        artifact_blob_id: artifact_blob_id(&markdown),
    })
}

pub(super) fn load_local_inventory_from_store<S>(store: &S) -> Result<LocalInventory, String>
where
    S: PlanSyncLocalStore + ?Sized,
{
    let mut plans = Vec::new();
    if let Some(details) = store.list_plan_inventory_details()? {
        plans.extend(
            details
                .into_iter()
                .filter(|row| !is_historical_status(value_get(row, "status"))),
        );
        return Ok(LocalInventory {
            indexed_plans: index_plans_by_path(&plans),
            indexed_by_identity: index_plans_by_identity(&plans),
            plans,
        });
    }
    for row in list_plan_summaries_with_plan_sync_local_store(store)? {
        if is_historical_status(value_get(&row, "status")) {
            continue;
        }
        if let Some(plan_id) = optional_text(value_get(&row, "plan_id"))? {
            plans.push(get_plan_with_plan_sync_local_store(
                store,
                plan_id.as_str(),
            )?);
        }
    }
    Ok(LocalInventory {
        indexed_plans: index_plans_by_path(&plans),
        indexed_by_identity: index_plans_by_identity(&plans),
        plans,
    })
}

pub(super) fn load_remote_inventory<C>(
    client: &mut C,
    request: &SyncRequest,
    sync_target: &SyncTarget,
) -> Result<RemoteInventory, String>
where
    C: PlanSyncRemoteInventorySource + ?Sized,
{
    load_remote_inventory_from_source(client, request, sync_target)
}

pub(super) fn load_remote_inventory_from_source<S>(
    source: &mut S,
    request: &SyncRequest,
    sync_target: &SyncTarget,
) -> Result<RemoteInventory, String>
where
    S: PlanSyncRemoteInventorySource + ?Sized,
{
    let repo_name = request
        .remote_repo_name
        .as_deref()
        .ok_or_else(|| "Remote repo_name is required for remote sync.".to_string())?;
    let scoped_artifact_path = if sync_target.scope == "file" {
        Some(sync_target.target_path.as_str())
    } else {
        None
    };
    let plans = remote_plan_summaries_to_open_plans(
        list_plan_summaries_with_plan_sync_remote_inventory_source(
            source,
            repo_name,
            scoped_artifact_path,
        )?,
    );
    Ok(RemoteInventory {
        indexed_plans: index_plans_by_path(&plans),
        indexed_by_identity: index_plans_by_identity(&plans),
        scoped_artifact_path: scoped_artifact_path.map(str::to_string),
        full_loaded: scoped_artifact_path.is_none(),
        plans,
    })
}

pub(super) fn ensure_full_remote_inventory<C>(
    client: Option<&mut C>,
    request: &SyncRequest,
    inventory: &mut RemoteInventory,
) -> Result<(), String>
where
    C: PlanSyncRemoteInventorySource + ?Sized,
{
    ensure_full_remote_inventory_from_source(client, request, inventory)
}

pub(super) fn ensure_full_remote_inventory_from_source<S>(
    client: Option<&mut S>,
    request: &SyncRequest,
    inventory: &mut RemoteInventory,
) -> Result<(), String>
where
    S: PlanSyncRemoteInventorySource + ?Sized,
{
    if inventory.full_loaded {
        return Ok(());
    }
    let repo_name = request
        .remote_repo_name
        .as_deref()
        .ok_or_else(|| "Remote repo_name is required for remote sync.".to_string())?;
    let source = client.ok_or_else(|| {
        "Remote client is required to load full remote plan inventory.".to_string()
    })?;
    let plans = remote_plan_summaries_to_open_plans(
        list_plan_summaries_with_plan_sync_remote_inventory_source(source, repo_name, None)?,
    );
    inventory.indexed_plans = index_plans_by_path(&plans);
    inventory.indexed_by_identity = index_plans_by_identity(&plans);
    inventory.plans = plans;
    inventory.scoped_artifact_path = None;
    inventory.full_loaded = true;
    Ok(())
}

pub(super) fn remote_plan_summaries_to_open_plans(rows: Vec<JsonValue>) -> Vec<JsonValue> {
    rows.into_iter()
        .filter(|row| !is_historical_status(value_get(row, "status")))
        .map(remote_plan_summary_to_plan)
        .collect()
}

pub(super) fn remote_plan_summary_to_plan(row: JsonValue) -> JsonValue {
    let mut plan = match row {
        JsonValue::Object(object) => object,
        other => return other,
    };
    let items = value_get(&JsonValue::Object(plan.clone()), "head_revision_items_json")
        .and_then(JsonValue::as_str)
        .and_then(|text| JsonCodec::parse_value(text, "head revision items").ok())
        .and_then(|value| value.as_array().cloned())
        .map(JsonValue::Array)
        .unwrap_or_else(|| JsonValue::Array(Vec::new()));
    let row_value = JsonValue::Object(plan.clone());
    plan.insert(
        "head_revision".to_string(),
        JsonValue::Object(JsonMap::from_iter([
            (
                "plan_revision_id".to_string(),
                text_field(&row_value, "head_revision_id")
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "revision_number".to_string(),
                value_get(&row_value, "head_revision_number")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "artifact_path".to_string(),
                text_field(&row_value, "head_artifact_path")
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "artifact_selector".to_string(),
                text_field(&row_value, "head_artifact_selector")
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "artifact_heading".to_string(),
                text_field(&row_value, "head_artifact_heading")
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "artifact_blob_id".to_string(),
                text_field(&row_value, "head_artifact_blob_id")
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            ("items".to_string(), items),
            (
                "summary".to_string(),
                text_field(&row_value, "head_revision_summary")
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "created_at".to_string(),
                text_field(&row_value, "head_revision_created_at")
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
        ])),
    );
    JsonValue::Object(plan)
}

fn duplicate_current_updated_at_s(plan_id: &str, plan: &JsonValue) -> Result<u64, String> {
    let updated_at = raw_string_field(plan, "updated_at").ok_or_else(|| {
        format!(
            "Duplicate current plan {plan_id} has no updated_at ordering authority; refusing local reconciliation."
        )
    })?;
    let malformed = || {
        format!(
            "Duplicate current plan {plan_id} has malformed updated_at ordering authority; refusing local reconciliation."
        )
    };
    if updated_at.is_empty() || updated_at.trim() != updated_at {
        return Err(malformed());
    }
    if updated_at.bytes().all(|byte| byte.is_ascii_digit()) {
        let seconds = updated_at.parse::<u64>().map_err(|_| malformed())?;
        if seconds.to_string() != updated_at {
            return Err(malformed());
        }
        return Ok(seconds);
    }
    let parsed = chrono::DateTime::parse_from_rfc3339(&updated_at).map_err(|_| malformed())?;
    if parsed.offset().local_minus_utc() != 0 || parsed.timestamp_subsec_nanos() != 0 {
        return Err(malformed());
    }
    let seconds = u64::try_from(parsed.timestamp()).map_err(|_| malformed())?;
    let canonical = chrono::DateTime::<Utc>::from_timestamp(
        i64::try_from(seconds).map_err(|_| malformed())?,
        0,
    )
    .ok_or_else(malformed)?;
    let canonical_offset = canonical.to_rfc3339_opts(SecondsFormat::Secs, false);
    let canonical_z = canonical.to_rfc3339_opts(SecondsFormat::Secs, true);
    if updated_at != canonical_offset && updated_at != canonical_z {
        return Err(malformed());
    }
    Ok(seconds)
}

pub(super) fn select_or_reconcile_local_sync_plan_candidate<W, I>(
    local_writer: &W,
    identity_source: &I,
    request: &SyncRequest,
    artifact: &SyncArtifact,
    local_inventory: &mut LocalInventory,
) -> Result<(Option<JsonValue>, Option<JsonValue>), String>
where
    W: PlanSyncLocalLifecycleStore + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
{
    let artifact_key = (
        artifact.artifact_path.clone(),
        artifact.artifact_selector.clone(),
    );
    let direct_current = open_candidates(
        &local_inventory
            .indexed_by_identity
            .get(&artifact_key)
            .cloned()
            .unwrap_or_default(),
    );
    if direct_current.len() <= 1 || !request.reconcile {
        return select_existing_plan_with_continuity(
            Path::new(&request.root_path),
            artifact,
            &local_inventory.indexed_by_identity,
            &local_inventory.plans,
        );
    }
    if artifact.artifact_selector.is_some() {
        return Err(format!(
            "Explicit duplicate-current reconciliation is limited to generic Markdown artifact {}; structured selectors remain fail closed.",
            artifact.artifact_path
        ));
    }

    let mut candidates = Vec::with_capacity(direct_current.len());
    let mut published_plan_id = None;
    for plan in direct_current {
        let plan_id = require_plan_id(&plan)?;
        if !local_plan_fully_published(&plan)? {
            return Err(format!(
                "Duplicate current plan {plan_id} is not fully published; refusing local reconciliation."
            ));
        }
        let items = head_value(&plan, "items").ok_or_else(|| {
            format!(
                "Duplicate current plan {plan_id} has no exact head items array; refusing local reconciliation."
            )
        })?;
        let items = items.as_array().ok_or_else(|| {
            format!(
                "Duplicate current plan {plan_id} head items are not an array; refusing local reconciliation."
            )
        })?;
        if !items.is_empty() {
            return Err(format!(
                "Duplicate current plan {plan_id} has Plan items; refusing local reconciliation."
            ));
        }
        let target_plan_id = text_field(&plan, "published_plan_id").ok_or_else(|| {
            format!(
                "Duplicate current plan {plan_id} has no published remote Plan target; refusing local reconciliation."
            )
        })?;
        match published_plan_id.as_deref() {
            Some(expected) if expected != target_plan_id.as_str() => {
                return Err(format!(
                    "Duplicate current plans do not share one published remote Plan target: {expected}, {target_plan_id}."
                ))
            }
            None => published_plan_id = Some(target_plan_id),
            _ => {}
        }
        let updated_at = duplicate_current_updated_at_s(&plan_id, &plan)?;
        candidates.push((updated_at, plan_id, plan));
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    let greatest_updated_at = candidates
        .last()
        .map(|candidate| candidate.0)
        .ok_or_else(|| "Duplicate current Plan reconciliation has no candidates.".to_string())?;
    if candidates
        .iter()
        .filter(|candidate| candidate.0 == greatest_updated_at)
        .count()
        != 1
    {
        return Err(format!(
            "Duplicate current plans have a tied greatest updated_at {greatest_updated_at}; refusing local reconciliation."
        ));
    }
    let retained = candidates
        .last()
        .map(|candidate| candidate.2.clone())
        .ok_or_else(|| "Duplicate current Plan reconciliation has no retained Plan.".to_string())?;
    let retained_plan_id = require_plan_id(&retained)?;
    let mut archived_plan_ids = candidates
        .iter()
        .filter(|candidate| candidate.1 != retained_plan_id)
        .map(|candidate| candidate.1.clone())
        .collect::<Vec<_>>();
    archived_plan_ids.sort();
    let archived_at = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
    for archived_plan_id in &archived_plan_ids {
        let archived = close_plan_with_plan_sync_local_lifecycle_store(
            local_writer,
            archived_plan_id,
            "archived",
            archived_at.as_str(),
        )?;
        replace_plan_in_inventory(local_inventory, archived_plan_id, &archived);
    }
    Ok((
        Some(retained),
        Some(json!({
            "match_kind": "duplicate_current_reconcile",
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "retained_plan_id": retained_plan_id,
            "archived_plan_ids": archived_plan_ids,
            "published_plan_id": published_plan_id,
        })),
    ))
}

#[expect(
    clippy::too_many_arguments,
    reason = "candidate resolution coordinates explicit local, remote, identity, and cache ports"
)]
pub(super) fn resolve_local_sync_plan_candidate<W, B, I, C>(
    local_writer: &W,
    local_blob_store: &B,
    identity_source: &I,
    request: &SyncRequest,
    artifact: &SyncArtifact,
    local_inventory: &mut LocalInventory,
    remote_inventory: &mut RemoteInventory,
    mut client: Option<&mut C>,
    remote_revisions_cache: &mut BTreeMap<String, Vec<JsonValue>>,
    remote_revision_detail_cache: &mut BTreeMap<(String, String), JsonValue>,
) -> Result<LocalSyncPlanCandidate, String>
where
    W: PlanSyncLocalAdoptionStore + PlanSyncLocalIdentityRebindStore + ?Sized,
    B: PlanSyncLocalBlobStore + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
    C: PlanSyncRemoteContinuitySource + ?Sized,
{
    let (local_plan, local_continuity) = select_or_reconcile_local_sync_plan_candidate(
        local_writer,
        identity_source,
        request,
        artifact,
        local_inventory,
    )?;
    let local_fully_published = local_plan
        .as_ref()
        .map(|plan| local_plan_fully_published(plan).unwrap_or(false))
        .unwrap_or(false);
    let mut remote_plan = None;
    let mut remote_continuity = None;
    if request.base_url.is_some()
        && (local_plan.is_none() || !local_fully_published || request.reconcile)
    {
        let (mut candidate, mut continuity) = select_existing_plan_with_continuity(
            Path::new(&request.root_path),
            artifact,
            &remote_inventory.indexed_by_identity,
            &remote_inventory.plans,
        )?;
        if candidate.is_none() && remote_inventory.scoped_artifact_path.is_some() {
            ensure_full_remote_inventory(client.as_deref_mut(), request, remote_inventory)?;
            (candidate, continuity) = select_existing_plan_with_continuity(
                Path::new(&request.root_path),
                artifact,
                &remote_inventory.indexed_by_identity,
                &remote_inventory.plans,
            )?;
        }
        remote_plan = candidate;
        remote_continuity = continuity;
    }
    let continuity = local_continuity.or(remote_continuity);
    let Some(remote_plan_value) = remote_plan.clone() else {
        return Ok((local_plan, None, continuity));
    };
    let remote_plan_id = require_plan_id(&remote_plan_value)?;
    if local_plan.is_none() {
        let remote_revisions = if request.rebase {
            vec![materialize_remote_plan_head_for_local_adoption(
                local_blob_store,
                &remote_plan_value,
                client.as_deref_mut(),
                remote_revision_detail_cache,
            )?]
        } else {
            load_remote_revisions_cached(
                client.as_deref_mut(),
                remote_revisions_cache,
                &remote_plan_id,
            )?
        };
        let adopted_plan = if request.rebase {
            adopt_materialized_remote_plan_for_local_sync(
                local_writer,
                identity_source,
                request,
                &remote_plan_value,
                &remote_revisions,
            )?
        } else {
            adopt_remote_plan_for_local_sync(
                local_writer,
                local_blob_store,
                identity_source,
                request,
                &remote_plan_value,
                &remote_revisions,
                client.as_deref_mut(),
                remote_revision_detail_cache,
            )?
        };
        local_inventory.plans.push(adopted_plan.clone());
        local_inventory.indexed_by_identity = index_plans_by_identity(&local_inventory.plans);
        local_inventory.indexed_plans = index_plans_by_path(&local_inventory.plans);
        return Ok((
            Some(adopted_plan.clone()),
            Some(plan_sync_adoption_row(
                adopted_plan
                    .get("plan_id")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
                &remote_plan_id,
                &artifact.artifact_path,
                artifact.artifact_selector.as_deref(),
                remote_head_revision_id(&remote_plan_value),
                local_head_revision_id(&adopted_plan),
                None,
                None,
            )),
            continuity,
        ));
    }

    let local_plan_value = local_plan.clone().unwrap_or(JsonValue::Null);
    let local_plan_id = require_plan_id(&local_plan_value)?;
    let bound_remote_plan_id = selected_remote_plan_id(&local_plan_value, &local_plan_id)?;
    let local_plan_ref = LocalPlanId::from_raw(local_plan_id.clone())?;
    let remote_plan_ref = RemotePlanId::from_raw(remote_plan_id.clone())?;
    let bound_remote_plan_ref = RemotePlanId::from_raw(bound_remote_plan_id.clone())?;
    let local_publication_state = optional_text(value_get(&local_plan_value, "publication_state"))?;

    if local_publication_state.as_deref() == Some("published") && request.reconcile {
        let local_revisions =
            list_plan_revisions_with_plan_sync_local_store(local_writer, &local_plan_id)?;
        let mixed_lineage = !plan_revisions_share_lineage(&local_revisions)?;
        let receipt_issue = if bound_remote_plan_ref == remote_plan_ref {
            let remote_revisions = load_remote_revisions_cached(
                client.as_deref_mut(),
                remote_revisions_cache,
                &remote_plan_id,
            )?;
            populated_receipt_issue(
                &local_plan_ref,
                &local_plan_value,
                &local_revisions,
                &remote_plan_ref,
                &remote_revisions,
            )?
        } else {
            None
        };
        if mixed_lineage || receipt_issue.is_some() {
            validate_itemless_current_local_plan_for_identity_reset(
                &local_plan_ref,
                &local_plan_value,
                &local_revisions,
            )?;
            let artifact_value = artifact_to_json(artifact);
            let local_head_exact =
                plan_matches_sync_artifact(&local_plan_value, &artifact_value, true)?;
            let remote_head_exact =
                plan_matches_sync_artifact(&remote_plan_value, &artifact_value, true)?;
            if local_head_exact && remote_head_exact {
                if !local_writer.remote_adoption_allocates_fresh_local_plan_identity() {
                    return Err(format!(
                        "Local store cannot allocate a fresh identity for {}; refusing exact-head identity reset before lifecycle mutation.",
                        local_plan_ref.reference()
                    ));
                }
                let remote_head = materialize_remote_plan_head_for_local_adoption(
                    local_blob_store,
                    &remote_plan_value,
                    client.as_deref_mut(),
                    remote_revision_detail_cache,
                )?;
                validate_exact_remote_head_artifact(
                    &remote_plan_ref,
                    &remote_plan_value,
                    artifact,
                    &remote_head,
                )?;
                let archived_at =
                    timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
                let archived = close_plan_with_plan_sync_local_lifecycle_store(
                    local_writer,
                    &local_plan_id,
                    "archived",
                    archived_at.as_str(),
                )?;
                replace_plan_in_inventory(local_inventory, &local_plan_id, &archived);
                let adopted = adopt_materialized_remote_plan_at_distinct_local_identity(
                    local_writer,
                    identity_source,
                    request,
                    &remote_plan_value,
                    std::slice::from_ref(&remote_head),
                )?;
                let adopted_plan_id = require_plan_id(&adopted)?;
                if adopted_plan_id == local_plan_id {
                    return Err(format!(
                        "Exact-head identity reset reused occupied local Plan {local_plan_id}; refusing to overwrite immutable Plan history."
                    ));
                }
                local_inventory.plans.push(adopted.clone());
                local_inventory.indexed_by_identity =
                    index_plans_by_identity(&local_inventory.plans);
                local_inventory.indexed_plans = index_plans_by_path(&local_inventory.plans);
                return Ok((
                    Some(adopted.clone()),
                    Some(plan_sync_adoption_row(
                        adopted.get("plan_id").cloned().unwrap_or(JsonValue::Null),
                        &remote_plan_id,
                        &artifact.artifact_path,
                        artifact.artifact_selector.as_deref(),
                        remote_head_revision_id(&remote_plan_value),
                        local_head_revision_id(&adopted),
                        None,
                        archived.get("plan_id").cloned(),
                    )),
                    continuity,
                ));
            }
            if let Some(detail) = receipt_issue {
                return Err(format!(
                    "{detail} Current local Markdown and {} head are not an exact match, so explicit reconciliation cannot reset the local identity.",
                    remote_plan_ref.reference()
                ));
            }
        }
    }
    if bound_remote_plan_id == remote_plan_id {
        return Ok((Some(local_plan_value), None, continuity));
    }
    if local_publication_state.as_deref() == Some("published") && request.rebase {
        let remote_head = materialize_remote_plan_head_for_local_adoption(
            local_blob_store,
            &remote_plan_value,
            client.as_deref_mut(),
            remote_revision_detail_cache,
        )?;
        validate_rebase_remote_head_artifact_identity(&remote_plan_id, artifact, &remote_head)?;
        let archived_at = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
        let archived = close_plan_with_plan_sync_local_lifecycle_store(
            local_writer,
            &local_plan_id,
            "archived",
            archived_at.as_str(),
        )?;
        replace_plan_in_inventory(local_inventory, &local_plan_id, &archived);
        let adopted = adopt_materialized_remote_plan_at_distinct_local_identity(
            local_writer,
            identity_source,
            request,
            &remote_plan_value,
            std::slice::from_ref(&remote_head),
        )?;
        let adopted_plan_id = require_plan_id(&adopted)?;
        if adopted_plan_id == local_plan_id {
            return Err(format!(
                "Published Plan rebase reused occupied local plan {local_plan_id}; refusing to overwrite immutable Plan history."
            ));
        }
        local_inventory.plans.push(adopted.clone());
        local_inventory.indexed_by_identity = index_plans_by_identity(&local_inventory.plans);
        local_inventory.indexed_plans = index_plans_by_path(&local_inventory.plans);
        return Ok((
            Some(adopted.clone()),
            Some(plan_sync_adoption_row(
                adopted.get("plan_id").cloned().unwrap_or(JsonValue::Null),
                &remote_plan_id,
                &artifact.artifact_path,
                artifact.artifact_selector.as_deref(),
                remote_head_revision_id(&remote_plan_value),
                local_head_revision_id(&adopted),
                None,
                archived.get("plan_id").cloned(),
            )),
            continuity,
        ));
    }
    if local_publication_state.as_deref() == Some("published") && request.reconcile {
        let local_revisions =
            list_plan_revisions_with_plan_sync_local_store(local_writer, &local_plan_id)?;
        if !plan_revisions_share_lineage(&local_revisions)? {
            let remote_head_artifact_path =
                head_text(&remote_plan_value, "artifact_path").ok_or_else(|| {
                    format!(
                        "Remote plan {remote_plan_id} has no head artifact path; refusing mixed-lineage split recovery."
                    )
                })?;
            let remote_head_artifact_selector = head_text(&remote_plan_value, "artifact_selector");
            if remote_head_artifact_path != artifact.artifact_path
                || remote_head_artifact_selector != artifact.artifact_selector
            {
                return Err(format!(
                    "Remote plan {remote_plan_id} does not match {}; refusing mixed-lineage split recovery.",
                    plan_artifact_identity_label(
                        &artifact.artifact_path,
                        artifact.artifact_selector.as_deref()
                    )
                ));
            }
            let bound_remote_revisions = load_remote_revisions_cached(
                client.as_deref_mut(),
                remote_revisions_cache,
                &bound_remote_plan_id,
            )?;
            validate_exact_mixed_local_plan_lineage_split(
                artifact,
                &local_plan_value,
                &local_revisions,
                &bound_remote_plan_id,
                &bound_remote_revisions,
            )?;
            let remote_head = materialize_remote_plan_head_for_local_adoption(
                local_blob_store,
                &remote_plan_value,
                client.as_deref_mut(),
                remote_revision_detail_cache,
            )?;
            let archived_at = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
            let archived = close_plan_with_plan_sync_local_lifecycle_store(
                local_writer,
                &local_plan_id,
                "archived",
                archived_at.as_str(),
            )?;
            replace_plan_in_inventory(local_inventory, &local_plan_id, &archived);
            let adopted = adopt_materialized_remote_plan_at_distinct_local_identity(
                local_writer,
                identity_source,
                request,
                &remote_plan_value,
                std::slice::from_ref(&remote_head),
            )?;
            let adopted_plan_id = require_plan_id(&adopted)?;
            if adopted_plan_id == local_plan_id {
                return Err(format!(
                    "Mixed-lineage split recovery reused occupied local plan {local_plan_id}; refusing to overwrite immutable Plan history."
                ));
            }
            local_inventory.plans.push(adopted.clone());
            local_inventory.indexed_by_identity = index_plans_by_identity(&local_inventory.plans);
            local_inventory.indexed_plans = index_plans_by_path(&local_inventory.plans);
            return Ok((
                Some(adopted.clone()),
                Some(plan_sync_adoption_row(
                    adopted.get("plan_id").cloned().unwrap_or(JsonValue::Null),
                    &remote_plan_id,
                    &artifact.artifact_path,
                    artifact.artifact_selector.as_deref(),
                    remote_head_revision_id(&remote_plan_value),
                    local_head_revision_id(&adopted),
                    None,
                    archived.get("plan_id").cloned(),
                )),
                continuity,
            ));
        }
    }

    if local_publication_state.as_deref() != Some("published") && request.rebase {
        let remote_head = materialize_remote_plan_head_for_local_adoption(
            local_blob_store,
            &remote_plan_value,
            client.as_deref_mut(),
            remote_revision_detail_cache,
        )?;
        validate_rebase_remote_head_artifact_identity(&remote_plan_id, artifact, &remote_head)?;
        let archived_at = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
        let archived = close_plan_with_plan_sync_local_lifecycle_store(
            local_writer,
            &local_plan_id,
            "archived",
            archived_at.as_str(),
        )?;
        replace_plan_in_inventory(local_inventory, &local_plan_id, &archived);
        let adopted = adopt_materialized_remote_plan_for_local_sync(
            local_writer,
            identity_source,
            request,
            &remote_plan_value,
            std::slice::from_ref(&remote_head),
        )?;
        local_inventory.plans.push(adopted.clone());
        local_inventory.indexed_by_identity = index_plans_by_identity(&local_inventory.plans);
        local_inventory.indexed_plans = index_plans_by_path(&local_inventory.plans);
        return Ok((
            Some(adopted.clone()),
            Some(plan_sync_adoption_row(
                adopted.get("plan_id").cloned().unwrap_or(JsonValue::Null),
                &remote_plan_id,
                &artifact.artifact_path,
                artifact.artifact_selector.as_deref(),
                remote_head_revision_id(&remote_plan_value),
                local_head_revision_id(&adopted),
                None,
                archived.get("plan_id").cloned(),
            )),
            continuity,
        ));
    }

    if (local_publication_state.as_deref() != Some("published") || request.reconcile)
        && plan_heads_equivalent(&local_plan_value, &remote_plan_value)
            .map_err(|err| err.to_string())?
    {
        let remote_revisions = load_remote_revisions_cached(
            client.as_deref_mut(),
            remote_revisions_cache,
            &remote_plan_id,
        )?;
        if let Some(rebound) = bind_equivalent_local_plan_to_remote_identity(
            local_writer,
            identity_source,
            request,
            &local_plan_value,
            &remote_plan_value,
            &remote_revisions,
        )? {
            let rebound_plan_id = require_plan_id(&rebound)?;
            let previous_plan_id = (rebound_plan_id != local_plan_id)
                .then(|| JsonValue::String(local_plan_id.clone()));
            replace_plan_in_inventory(local_inventory, &local_plan_id, &rebound);
            return Ok((
                Some(rebound.clone()),
                Some(plan_sync_adoption_row(
                    rebound.get("plan_id").cloned().unwrap_or(JsonValue::Null),
                    &remote_plan_id,
                    &artifact.artifact_path,
                    artifact.artifact_selector.as_deref(),
                    remote_head_revision_id(&remote_plan_value),
                    local_head_revision_id(&rebound),
                    previous_plan_id,
                    None,
                )),
                continuity,
            ));
        }
        let archived_at = timestamp_with_plan_sync_workflow_identity_source(identity_source)?;
        let archived = close_plan_with_plan_sync_local_lifecycle_store(
            local_writer,
            &local_plan_id,
            "archived",
            archived_at.as_str(),
        )?;
        replace_plan_in_inventory(local_inventory, &local_plan_id, &archived);
        let adopted = adopt_remote_plan_for_local_sync(
            local_writer,
            local_blob_store,
            identity_source,
            request,
            &remote_plan_value,
            &remote_revisions,
            client,
            remote_revision_detail_cache,
        )?;
        local_inventory.plans.push(adopted.clone());
        local_inventory.indexed_by_identity = index_plans_by_identity(&local_inventory.plans);
        local_inventory.indexed_plans = index_plans_by_path(&local_inventory.plans);
        return Ok((
            Some(adopted.clone()),
            Some(plan_sync_adoption_row(
                adopted.get("plan_id").cloned().unwrap_or(JsonValue::Null),
                &remote_plan_id,
                &artifact.artifact_path,
                artifact.artifact_selector.as_deref(),
                remote_head_revision_id(&remote_plan_value),
                local_head_revision_id(&adopted),
                None,
                archived.get("plan_id").cloned(),
            )),
            continuity,
        ));
    }

    let selector_suffix = artifact
        .artifact_selector
        .as_ref()
        .map(|value| format!(" [{value}]"))
        .unwrap_or_default();
    Err(format!(
        "Remote plan {remote_plan_id} already tracks {}{}; local plan {local_plan_id} is bound to remote {bound_remote_plan_id} and would publish a duplicate.",
        artifact.artifact_path, selector_suffix
    ))
}
