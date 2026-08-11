use super::*;

pub(super) fn read_task_detail_json(
    workflow: &dyn ServerWorkflowStore,
    repositories: &dyn NativeRepositoryService,
    repo_name: Option<&str>,
    task_ref: &str,
) -> Result<JsonValue, String> {
    let task = json_object(workflow.get_task(repo_name, task_ref)?, "task")?;
    let task_id = required_text(&task, "task_id")?;
    let task_repo = required_text(&task, "repo_name")?;
    let repository = json_object(
        repositories
            .get_repository(&task_repo)
            .map_err(|error| error.to_string())?,
        "repository",
    )?;
    let changes = json_object_rows(workflow.list_changes(&task_repo)?, "changes")?
        .into_iter()
        .filter(|change| object_text(change, "task_id").as_deref() == Some(task_id.as_str()))
        .collect::<Vec<_>>();
    let change_by_ref = changes
        .iter()
        .filter_map(|change| object_text(change, "change_ref").map(|id| (id, change.clone())))
        .collect::<BTreeMap<_, _>>();
    let mut patchsets = Vec::new();
    let mut reviews = Vec::new();
    for change in &changes {
        let change_ref = required_text(change, "change_ref")?;
        patchsets.extend(json_object_rows(
            workflow.list_patchsets(Some(&task_repo), &change_ref)?,
            "patchsets",
        )?);
        reviews.extend(json_object_rows(
            workflow.list_reviews(&change_ref)?,
            "reviews",
        )?);
    }
    let mut attestations = Vec::new();
    let mut policy_decisions = Vec::new();
    let mut patchset_deltas = Vec::new();
    let mut snapshot_file_cache = BTreeMap::new();
    for patchset in &patchsets {
        let patchset_id = required_text(patchset, "patchset_id")?;
        if let Ok(value) = workflow.get_attestation(&patchset_id) {
            attestations.push(json_object(value, "attestation")?);
        }
        if let Ok(value) = workflow.get_policy(&patchset_id) {
            policy_decisions.push(json_object(value, "policy")?);
        }
        if let Some(change_ref) = object_text(patchset, "change_ref") {
            if let Some(change) = change_by_ref.get(&change_ref) {
                if let Ok(delta) = patchset_delta_for_rows_with_cache(
                    workflow,
                    repositories,
                    Some(&task_repo),
                    change,
                    patchset,
                    "base",
                    &mut snapshot_file_cache,
                ) {
                    patchset_deltas.push(json_object(delta, "patchset delta")?);
                }
            }
        }
    }
    let refs = refs_for_changes(repositories, &changes)?;
    task_workflow_detail_read_model_json(&json!({
        "task": task,
        "repository": repository,
        "changes": changes,
        "patchsets": patchsets,
        "reviews": reviews,
        "attestations": attestations,
        "policy_decisions": policy_decisions,
        "land_requests": [],
        "refs": refs,
        "patchset_deltas": patchset_deltas,
        "events": [],
    }))
}

pub(super) fn read_change_detail_base_json(
    workflow: &dyn ServerWorkflowStore,
    repositories: &dyn NativeRepositoryService,
    repo_name: Option<&str>,
    change_ref: &str,
) -> Result<JsonValue, String> {
    let change = json_object(workflow.get_change(repo_name, change_ref)?, "change")?;
    let change_id = required_text(&change, "change_id")?;
    let change_ref = required_text(&change, "change_ref")?;
    let change_repo = required_text(&change, "repo_name")?;
    let task_id = required_text(&change, "task_id")?;
    let change_title = object_text(&change, "title").unwrap_or_default();
    let task = json_object(workflow.get_task(Some(&change_repo), &task_id)?, "task")?;
    let repository = json_object(
        repositories
            .get_repository(&change_repo)
            .map_err(|error| error.to_string())?,
        "repository",
    )?;
    let patchsets = json_object_rows(
        workflow.list_patchsets(Some(&change_repo), &change_ref)?,
        "patchsets",
    )?;
    let current_patchset = patchset_for_change(
        &patchsets,
        &change,
        "current_patchset_id",
        "current_patchset_number",
    )
    .or_else(|| patchsets.last().cloned());
    let selected_patchset = patchset_for_change(
        &patchsets,
        &change,
        "selected_patchset_id",
        "selected_patchset_number",
    );
    let current_patchset_id = current_patchset
        .as_ref()
        .and_then(|patchset| object_text(patchset, "patchset_id"));
    let reviews = json_object_rows(workflow.list_reviews(&change_ref)?, "reviews")?;
    let mut review_summary = review_summary_from_rows(&reviews, current_patchset_id.as_deref());
    review_summary.insert("change_id".to_string(), json!(change_id));
    review_summary.insert("change_ref".to_string(), json!(change_ref));
    review_summary.insert(
        "current_patchset_id".to_string(),
        json!(current_patchset_id),
    );
    review_summary.insert(
        "reviews".to_string(),
        JsonValue::Array(reviews.iter().cloned().map(JsonValue::Object).collect()),
    );
    let policy_summary = current_patchset_id
        .as_deref()
        .and_then(|patchset_id| workflow.get_policy(patchset_id).ok())
        .unwrap_or_else(|| {
            json!({
                "patchset_id": current_patchset_id,
                "decision": "pending",
                "checks": [],
            })
        });
    let attestation_summary = current_patchset_id
        .as_deref()
        .and_then(|patchset_id| workflow.get_attestation(patchset_id).ok());
    Ok(json!({
        "change": change,
        "repository": repository,
        "task": task,
        "workflow_context": workflow_context_json("change", "change", &change_ref, &change_title),
        "patchsets": patchsets,
        "current_patchset": current_patchset,
        "selected_patchset": selected_patchset,
        "review_summary": JsonValue::Object(review_summary),
        "policy_summary": policy_summary,
        "attestation_summary": attestation_summary,
        "patchset_ci_status": JsonValue::Null,
        "landing_summary": JsonValue::Null,
        "delta": JsonValue::Null,
        "base_diff": JsonValue::Null,
        "timeline": [],
        "freshness": JsonValue::Null,
    }))
}

pub(super) fn read_change_detail_ci_status_json(
    runtime: &dyn ServerRuntimeService,
    detail: &JsonValue,
) -> JsonValue {
    let current_patchset = detail
        .get("current_patchset")
        .and_then(JsonValue::as_object);
    let change = detail.get("change").and_then(JsonValue::as_object);
    let current_patchset_id =
        current_patchset.and_then(|patchset| object_text(patchset, "patchset_id"));
    current_patchset
        .zip(change)
        .and_then(|(patchset, change)| {
            runtime
                .read_patchset_ci_status_from_workflow_rows(
                    &JsonValue::Object(patchset.clone()),
                    &JsonValue::Object(change.clone()),
                    10,
                    None,
                )
                .unwrap_or_else(|| {
                    runtime.read_patchset_ci_status(
                        current_patchset_id.as_deref().unwrap_or_default(),
                        10,
                        None,
                    )
                })
                .ok()
        })
        .unwrap_or(JsonValue::Null)
}

pub(super) fn read_change_detail_repository_projection_json(
    workflow: &dyn ServerWorkflowStore,
    repositories: &dyn NativeRepositoryService,
    detail: &JsonValue,
) -> Result<(JsonValue, JsonValue, JsonValue), String> {
    let change = detail
        .get("change")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "change detail base is missing change".to_string())?;
    let current_patchset = detail
        .get("current_patchset")
        .and_then(JsonValue::as_object);
    let mut snapshot_file_cache = BTreeMap::new();
    let delta = match current_patchset {
        Some(patchset) => Some(patchset_delta_for_rows_with_cache(
            workflow,
            repositories,
            object_text(change, "repo_name").as_deref(),
            change,
            patchset,
            "previous",
            &mut snapshot_file_cache,
        )?),
        None => None,
    };
    let base_diff = match current_patchset {
        Some(patchset) => Some(patchset_delta_for_rows_with_cache(
            workflow,
            repositories,
            object_text(change, "repo_name").as_deref(),
            change,
            patchset,
            "base",
            &mut snapshot_file_cache,
        )?),
        None => None,
    };
    let freshness = freshness_for_change(repositories, change, current_patchset)?;
    Ok((json!(delta), json!(base_diff), freshness))
}

pub(super) fn complete_change_detail_json(
    mut detail: JsonValue,
    patchset_ci_status: JsonValue,
    delta: JsonValue,
    base_diff: JsonValue,
    freshness: JsonValue,
) -> Result<JsonValue, String> {
    let object = detail
        .as_object_mut()
        .ok_or_else(|| "change detail base must be an object".to_string())?;
    object.insert("patchset_ci_status".to_string(), patchset_ci_status);
    object.insert("delta".to_string(), delta);
    object.insert("base_diff".to_string(), base_diff);
    object.insert("freshness".to_string(), freshness);
    Ok(detail)
}

pub(super) fn read_patchset_delta_json(
    workflow: &dyn ServerWorkflowStore,
    repositories: &dyn NativeRepositoryService,
    repo_name: Option<&str>,
    patchset_ref: &str,
    against: &str,
    change_ref: Option<&str>,
) -> Result<JsonValue, String> {
    let patchset = resolve_patchset(workflow, repo_name, patchset_ref, change_ref)?;
    let change_ref = required_text(&patchset, "change_ref")?;
    let change = json_object(workflow.get_change(repo_name, &change_ref)?, "change")?;
    patchset_delta_for_rows(
        workflow,
        repositories,
        repo_name,
        &change,
        &patchset,
        against,
    )
}

pub(super) fn patchset_delta_for_rows(
    workflow: &dyn ServerWorkflowStore,
    repositories: &dyn NativeRepositoryService,
    repo_name: Option<&str>,
    change: &JsonMap<String, JsonValue>,
    patchset: &JsonMap<String, JsonValue>,
    against: &str,
) -> Result<JsonValue, String> {
    patchset_delta_for_rows_with_cache(
        workflow,
        repositories,
        repo_name,
        change,
        patchset,
        against,
        &mut BTreeMap::new(),
    )
}

fn patchset_delta_for_rows_with_cache(
    workflow: &dyn ServerWorkflowStore,
    repositories: &dyn NativeRepositoryService,
    repo_name: Option<&str>,
    change: &JsonMap<String, JsonValue>,
    patchset: &JsonMap<String, JsonValue>,
    against: &str,
    snapshot_file_cache: &mut BTreeMap<
        (String, String),
        BTreeMap<String, JsonMap<String, JsonValue>>,
    >,
) -> Result<JsonValue, String> {
    let patchset_id = required_text(patchset, "patchset_id")?;
    let change_id = required_text(change, "change_id")?;
    let change_ref = required_text(change, "change_ref")?;
    let change_repo = required_text(change, "repo_name")?;
    let repository = json_object(
        repositories
            .get_repository(&change_repo)
            .map_err(|error| error.to_string())?,
        "repository",
    )?;
    let left_snapshot_id;
    let against_label;
    match against {
        "base" => {
            left_snapshot_id = required_text(patchset, "base_snapshot_id")?;
            against_label = "base".to_string();
        }
        "previous" => {
            let previous_number = int_field(patchset, "patchset_number").unwrap_or(0) - 1;
            let previous = if previous_number > 0 {
                json_object_rows(
                    workflow.list_patchsets(Some(&change_repo), change_ref.as_str())?,
                    "patchsets",
                )?
                .into_iter()
                .find(|row| int_field(row, "patchset_number") == Some(previous_number))
            } else {
                None
            };
            if let Some(previous) = previous {
                left_snapshot_id = required_text(&previous, "revision_snapshot_id")?;
                against_label = required_text(&previous, "patchset_id")?;
            } else {
                left_snapshot_id = required_text(patchset, "base_snapshot_id")?;
                against_label = "base".to_string();
            }
        }
        other => {
            let other_patchset = resolve_patchset(
                workflow,
                repo_name.or(Some(change_repo.as_str())),
                other,
                Some(change_ref.as_str()),
            )?;
            left_snapshot_id = required_text(&other_patchset, "revision_snapshot_id")?;
            against_label = required_text(&other_patchset, "patchset_id")?;
        }
    }
    let right_snapshot_id = required_text(patchset, "revision_snapshot_id")?;
    let left_files = snapshot_file_map_cached(
        repositories,
        &change_repo,
        &left_snapshot_id,
        snapshot_file_cache,
    )?;
    let right_files = snapshot_file_map_cached(
        repositories,
        &change_repo,
        &right_snapshot_id,
        snapshot_file_cache,
    )?;
    let mut paths = left_files.keys().cloned().collect::<BTreeSet<_>>();
    paths.extend(right_files.keys().cloned());
    let mut files = Vec::new();
    for path in paths {
        let left = left_files.get(&path);
        let right = right_files.get(&path);
        let status = match (left, right) {
            (Some(_), None) => "deleted",
            (None, Some(_)) => "added",
            (Some(left), Some(right)) => {
                if object_text(left, "sha256") == object_text(right, "sha256")
                    && object_text(left, "mode") == object_text(right, "mode")
                {
                    continue;
                }
                "modified"
            }
            (None, None) => continue,
        };
        files.push(json!({
            "path": path,
            "status": status,
            "insertions": 0,
            "deletions": 0,
            "diff_text": "",
            "text_renderable": false,
            "old_blob_id": left.and_then(|row| object_text(row, "blob_id")),
            "new_blob_id": right.and_then(|row| object_text(row, "blob_id")),
        }));
    }
    Ok(json!({
        "patchset_id": patchset_id,
        "change_id": change_id,
        "change_ref": change_ref,
        "against": against_label,
        "base_snapshot_id": left_snapshot_id,
        "revision_snapshot_id": right_snapshot_id,
        "files_changed": files.len(),
        "insertions": 0,
        "deletions": 0,
        "summary": patchset.get("summary").cloned().unwrap_or(JsonValue::Null),
        "files": files,
        "cache_state": "computed",
        "content_diff": {
            "available": false,
            "reason": "native snapshot export provides manifest rows for server-side read models; inline text diff is not required for Python removal",
            "repository_pack_storage": repository.get("pack_storage").cloned().unwrap_or(JsonValue::Null),
        },
    }))
}

pub(super) fn resolve_patchset(
    workflow: &dyn ServerWorkflowStore,
    repo_name: Option<&str>,
    patchset_ref: &str,
    change_ref: Option<&str>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let direct_error = match workflow.get_patchset(repo_name, patchset_ref) {
        Ok(value) => return json_object(value, "patchset"),
        Err(error) => error,
    };
    let (Some(repo_name), Some(change_ref)) = (repo_name, change_ref) else {
        return Err(direct_error);
    };
    json_object_rows(
        workflow.list_patchsets(Some(repo_name), change_ref)?,
        "patchsets",
    )?
    .into_iter()
    .find(|patchset| object_text(patchset, "patchset_id").as_deref() == Some(patchset_ref))
    .ok_or_else(|| format!("Unknown patchset {patchset_ref} for {repo_name}/{change_ref}"))
}

pub(super) fn patchset_for_change(
    patchsets: &[JsonMap<String, JsonValue>],
    change: &JsonMap<String, JsonValue>,
    id_field: &str,
    number_field: &str,
) -> Option<JsonMap<String, JsonValue>> {
    if let Some(patchset_id) = object_text(change, id_field) {
        if let Some(found) = patchsets.iter().find(|patchset| {
            object_text(patchset, "patchset_id").as_deref() == Some(patchset_id.as_str())
        }) {
            return Some(found.clone());
        }
    }
    let number = int_field(change, number_field)?;
    patchsets
        .iter()
        .find(|patchset| int_field(patchset, "patchset_number") == Some(number))
        .cloned()
}

pub(super) fn refs_for_changes(
    repositories: &dyn NativeRepositoryService,
    changes: &[JsonMap<String, JsonValue>],
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    let mut seen = BTreeSet::new();
    let mut refs = Vec::new();
    for change in changes {
        let Some(repo_name) = object_text(change, "repo_name") else {
            continue;
        };
        let Some(line_name) = object_text(change, "base_line") else {
            continue;
        };
        if !seen.insert((repo_name.clone(), line_name.clone())) {
            continue;
        }
        if let Ok(line) = repositories.get_line(&repo_name, &line_name) {
            let line = json_object(line, "line")?;
            refs.push(JsonMap::from_iter([
                ("repo_name".to_string(), json!(repo_name)),
                ("line_name".to_string(), json!(line_name)),
                (
                    "head_snapshot_id".to_string(),
                    line.get("head_snapshot_id")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
            ]));
        }
    }
    Ok(refs)
}

pub(super) fn freshness_for_change(
    repositories: &dyn NativeRepositoryService,
    change: &JsonMap<String, JsonValue>,
    current_patchset: Option<&JsonMap<String, JsonValue>>,
) -> Result<JsonValue, String> {
    let Some(patchset) = current_patchset else {
        return Ok(json!({
            "base_is_fresh": false,
            "current_base_head": JsonValue::Null,
        }));
    };
    let repo_name = required_text(change, "repo_name")?;
    let base_line = object_text(change, "base_line").unwrap_or_else(|| "main".to_string());
    let base_head = repositories
        .get_line(&repo_name, &base_line)
        .ok()
        .and_then(|value| value.get("head_snapshot_id").cloned());
    let base_snapshot_id = patchset
        .get("base_snapshot_id")
        .cloned()
        .unwrap_or(JsonValue::Null);
    Ok(json!({
        "base_is_fresh": base_head.as_ref() == Some(&base_snapshot_id),
        "current_base_head": base_head,
    }))
}

pub(super) fn snapshot_file_map(
    repositories: &dyn NativeRepositoryService,
    repo_name: &str,
    snapshot_id: &str,
) -> Result<BTreeMap<String, JsonMap<String, JsonValue>>, String> {
    let snapshot = repositories
        .export_snapshot(
            repo_name,
            snapshot_id,
            SnapshotExportQuery {
                include_content: false,
                path: None,
            },
        )
        .map_err(|error| error.to_string())?;
    let mut out = BTreeMap::new();
    for file in snapshot
        .get("files")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let object = file
            .as_object()
            .cloned()
            .ok_or_else(|| "snapshot file row must be a JSON object.".to_string())?;
        let path = required_text(&object, "path")?;
        out.insert(path, object);
    }
    Ok(out)
}

fn snapshot_file_map_cached(
    repositories: &dyn NativeRepositoryService,
    repo_name: &str,
    snapshot_id: &str,
    cache: &mut BTreeMap<(String, String), BTreeMap<String, JsonMap<String, JsonValue>>>,
) -> Result<BTreeMap<String, JsonMap<String, JsonValue>>, String> {
    let key = (repo_name.to_string(), snapshot_id.to_string());
    if let Some(existing) = cache.get(&key) {
        return Ok(existing.clone());
    }
    let files = snapshot_file_map(repositories, repo_name, snapshot_id)?;
    cache.insert(key, files.clone());
    Ok(files)
}

pub(super) fn workflow_context_json(
    target: &str,
    focus_type: &str,
    focus_id: &str,
    focus_title: &str,
) -> JsonValue {
    json!({
        "target": target,
        "focus": {
            "type": focus_type,
            "id": focus_id,
            "title": focus_title,
        },
        "summary": {
            "document_count": 0,
            "diagram_count": 0,
            "layers": [],
        },
        "entries": [],
    })
}

pub(super) fn json_object(
    value: JsonValue,
    context: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{context} payload must be a JSON object."))
}

pub(super) fn json_object_rows(
    value: JsonValue,
    context: &str,
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    let rows = value
        .as_array()
        .or_else(|| {
            value
                .as_object()
                .and_then(|object| object.get("items").and_then(JsonValue::as_array))
        })
        .or_else(|| {
            value
                .as_object()
                .and_then(|object| object.get("reviews").and_then(JsonValue::as_array))
        })
        .ok_or_else(|| format!("{context} payload must be a JSON array."))?;
    rows.iter()
        .map(|row| {
            row.as_object()
                .cloned()
                .ok_or_else(|| format!("{context} row must be a JSON object."))
        })
        .collect()
}

pub(super) fn required_text(
    row: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<String, String> {
    object_text(row, field).ok_or_else(|| format!("{field} is required."))
}

pub(super) fn object_text(row: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    row.get(field)
        .and_then(|value| match value {
            JsonValue::String(text) => Some(text.as_str()),
            _ => None,
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn int_field(row: &JsonMap<String, JsonValue>, field: &str) -> Option<i64> {
    row.get(field).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
            .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
    })
}
