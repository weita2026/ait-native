use super::*;

pub(in crate::repository_pack_json) fn parse_json_value(
    text: &str,
    label: &str,
) -> Result<JsonValue, String> {
    JsonCodec::parse_value_with_error_prefix(text, &format!("Invalid {label} JSON"))
        .map_err(String::from)
}

pub(in crate::repository_pack_json) fn parse_json_bytes(
    bytes: &[u8],
    label: &str,
) -> Result<JsonValue, String> {
    JsonCodec::parse_slice_with_error_prefix(bytes, &format!("Invalid {label} JSON"))
        .map_err(String::from)
}

pub(in crate::repository_pack_json) fn object_from_value(
    value: JsonValue,
    label: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    match value {
        JsonValue::Null => Ok(JsonMap::new()),
        JsonValue::Object(object) => Ok(object),
        _ => Err(format!("{label} must be a JSON object.")),
    }
}

pub(in crate::repository_pack_json) fn string_value(value: impl Into<String>) -> JsonValue {
    JsonValue::String(value.into())
}

pub(in crate::repository_pack_json) fn number_value(value: i64) -> JsonValue {
    JsonValue::Number(value.into())
}

pub(in crate::repository_pack_json) fn u64_number_value(value: u64) -> Result<JsonValue, String> {
    let number = JsonNumber::from(value);
    Ok(JsonValue::Number(number))
}

pub(in crate::repository_pack_json) fn opt_string(
    object: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("Field `{key}` must be a JSON string or null.")),
    }
}

pub(in crate::repository_pack_json) fn req_string(
    object: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<String, String> {
    opt_string(object, key)?.ok_or_else(|| format!("Missing required field `{key}`."))
}

pub(in crate::repository_pack_json) fn opt_bool(
    object: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<bool>, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("Field `{key}` must be a JSON boolean or null.")),
    }
}

pub(in crate::repository_pack_json) fn opt_i64(
    object: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<i64>, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(value)) => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .map(Some)
            .ok_or_else(|| format!("Field `{key}` must fit in a signed integer.")),
        Some(_) => Err(format!("Field `{key}` must be a JSON integer or null.")),
    }
}

pub(in crate::repository_pack_json) fn opt_u64(
    object: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<u64>, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(value)) => value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
            .map(Some)
            .ok_or_else(|| format!("Field `{key}` must fit in an unsigned integer.")),
        Some(_) => Err(format!("Field `{key}` must be a JSON integer or null.")),
    }
}

pub(in crate::repository_pack_json) fn string_vec_from_field(
    object: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Vec<String>, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("Field `{key}` entries must be JSON strings."))
            })
            .collect(),
        Some(_) => Err(format!("Field `{key}` must be a JSON array.")),
    }
}

pub(in crate::repository_pack_json) fn string_vec_value(values: &[String]) -> JsonValue {
    JsonValue::Array(values.iter().cloned().map(string_value).collect())
}

pub(in crate::repository_pack_json) fn object_vec_from_field<T, F>(
    object: &JsonMap<String, JsonValue>,
    key: &str,
    mut parser: F,
) -> Result<Vec<T>, String>
where
    F: FnMut(JsonMap<String, JsonValue>) -> Result<T, String>,
{
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::Array(values)) => values
            .iter()
            .cloned()
            .map(|value| object_from_value(value, key).and_then(&mut parser))
            .collect(),
        Some(_) => Err(format!("Field `{key}` must be a JSON array.")),
    }
}

pub(in crate::repository_pack_json) fn object_vec_value<T, F>(
    values: &[T],
    builder: F,
) -> Result<JsonValue, String>
where
    F: FnMut(&T) -> Result<JsonValue, String>,
{
    values
        .iter()
        .map(builder)
        .collect::<Result<Vec<_>, _>>()
        .map(JsonValue::Array)
}

pub(in crate::repository_pack_json) fn insert_optional_string(
    object: &mut JsonMap<String, JsonValue>,
    key: &str,
    value: &Option<String>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), string_value(value.clone()));
    }
}

pub(in crate::repository_pack_json) fn insert_optional_i64(
    object: &mut JsonMap<String, JsonValue>,
    key: &str,
    value: Option<i64>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), number_value(value));
    }
}

pub(in crate::repository_pack_json) fn insert_optional_bool(
    object: &mut JsonMap<String, JsonValue>,
    key: &str,
    value: Option<bool>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), JsonValue::Bool(value));
    }
}

pub(in crate::repository_pack_json) fn insert_optional_value(
    object: &mut JsonMap<String, JsonValue>,
    key: &str,
    value: Option<JsonValue>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), value);
    }
}

pub(in crate::repository_pack_json) fn validate_nonempty(
    value: &str,
    label: &str,
) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("Missing {label}."))
    } else {
        Ok(())
    }
}

pub(in crate::repository_pack_json) fn pack_storage_to_value(
    payload: &RepositoryPackStoragePayload,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert(
        "contract".to_string(),
        string_value(payload.contract.as_str()),
    );
    object.insert(
        "zstd_only_verified".to_string(),
        JsonValue::Bool(payload.zstd_only_verified),
    );
    object.insert(
        "object_pack_format".to_string(),
        string_value(payload.object_pack_format.persisted_name()),
    );
    object.insert(
        "tree_pack_format".to_string(),
        string_value(payload.tree_pack_format.persisted_name()),
    );
    for (key, value) in [
        ("object_pack_count", payload.object_pack_count),
        ("tree_pack_count", payload.tree_pack_count),
        ("zstd_object_pack_count", payload.zstd_object_pack_count),
        ("zstd_tree_pack_count", payload.zstd_tree_pack_count),
    ] {
        object.insert(key.to_string(), u64_number_value(value)?);
    }
    object.insert(
        "requires_zstd_remote_sync".to_string(),
        JsonValue::Bool(payload.requires_zstd_remote_sync),
    );
    let mut validation = JsonMap::new();
    validation.insert(
        "state".to_string(),
        string_value(payload.validation.state.as_str()),
    );
    validation.insert(
        "error_count".to_string(),
        u64_number_value(payload.validation.error_count)?,
    );
    object.insert("validation".to_string(), JsonValue::Object(validation));
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn pack_storage_from_value(
    value: JsonValue,
) -> Result<RepositoryPackStoragePayload, String> {
    let object = object_from_value(value, "repository pack storage payload")?;
    let contract = match req_string(&object, "contract")?.as_str() {
        RepositoryPackStorageContract::NAME => RepositoryPackStorageContract::V1,
        other => {
            return Err(format!(
                "Unsupported repository pack storage contract: {other}"
            ))
        }
    };
    let validation = object
        .get("validation")
        .cloned()
        .map(|value| object_from_value(value, "pack storage validation"))
        .transpose()?
        .unwrap_or_default();
    Ok(RepositoryPackStoragePayload {
        contract,
        zstd_only_verified: opt_bool(&object, "zstd_only_verified")?.unwrap_or(false),
        object_pack_format: PackFormatKind::from_persisted(&req_string(
            &object,
            "object_pack_format",
        )?)?,
        tree_pack_format: TreePackFormatKind::from_persisted(&req_string(
            &object,
            "tree_pack_format",
        )?)?,
        object_pack_count: opt_u64(&object, "object_pack_count")?.unwrap_or(0),
        tree_pack_count: opt_u64(&object, "tree_pack_count")?.unwrap_or(0),
        zstd_object_pack_count: opt_u64(&object, "zstd_object_pack_count")?.unwrap_or(0),
        zstd_tree_pack_count: opt_u64(&object, "zstd_tree_pack_count")?.unwrap_or(0),
        requires_zstd_remote_sync: opt_bool(&object, "requires_zstd_remote_sync")?.unwrap_or(true),
        validation: RepositoryPackStorageValidationPayload {
            state: RepositoryPackStorageValidationState::from_str(
                &opt_string(&validation, "state")?.unwrap_or_else(|| "not_loaded".to_string()),
            )?,
            error_count: opt_u64(&validation, "error_count")?.unwrap_or(0),
        },
    })
}

pub(in crate::repository_pack_json) fn validate_pack_storage(
    payload: &RepositoryPackStoragePayload,
) -> Result<(), String> {
    if payload.contract != RepositoryPackStorageContract::V1 {
        return Err("Repository pack storage payload must use v1 contract.".to_string());
    }
    if payload.object_pack_count != payload.zstd_object_pack_count
        || payload.tree_pack_count != payload.zstd_tree_pack_count
    {
        return Err("Repository pack storage counts must describe zstd packs only.".to_string());
    }
    Ok(())
}

pub(in crate::repository_pack_json) fn pack_inventory_to_value(
    payload: &RepositoryPackInventoryPayload,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert(
        "repo_name".to_string(),
        string_value(payload.repo_name.clone()),
    );
    object.insert(
        "object_packs".to_string(),
        object_vec_value(&payload.object_packs, inventory_object_pack_to_value)?,
    );
    object.insert(
        "tree_packs".to_string(),
        object_vec_value(&payload.tree_packs, inventory_tree_pack_to_value)?,
    );
    object.insert(
        "blob_locators".to_string(),
        object_vec_value(&payload.blob_locators, inventory_blob_locator_to_value)?,
    );
    object.insert(
        "tree_locators".to_string(),
        object_vec_value(&payload.tree_locators, inventory_tree_locator_to_value)?,
    );
    object.insert(
        "snapshots".to_string(),
        object_vec_value(&payload.snapshots, inventory_snapshot_to_value)?,
    );
    object.insert(
        "line_heads".to_string(),
        object_vec_value(&payload.line_heads, inventory_line_head_to_value)?,
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn pack_inventory_from_value(
    value: JsonValue,
) -> Result<RepositoryPackInventoryPayload, String> {
    let object = object_from_value(value, "repository pack inventory payload")?;
    Ok(RepositoryPackInventoryPayload {
        repo_name: req_string(&object, "repo_name")?,
        object_packs: object_vec_from_field(
            &object,
            "object_packs",
            inventory_object_pack_from_object,
        )?,
        tree_packs: object_vec_from_field(&object, "tree_packs", inventory_tree_pack_from_object)?,
        blob_locators: object_vec_from_field(
            &object,
            "blob_locators",
            inventory_blob_locator_from_object,
        )?,
        tree_locators: object_vec_from_field(
            &object,
            "tree_locators",
            inventory_tree_locator_from_object,
        )?,
        snapshots: object_vec_from_field(&object, "snapshots", inventory_snapshot_from_object)?,
        line_heads: object_vec_from_field(&object, "line_heads", inventory_line_head_from_object)?,
    })
}

pub(in crate::repository_pack_json) fn validate_pack_inventory(
    payload: &RepositoryPackInventoryPayload,
) -> Result<(), String> {
    validate_nonempty(&payload.repo_name, "repository name")
}

pub(in crate::repository_pack_json) fn plan_request_to_value(
    request: &ZstdBulkPlanRequest,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert(
        "snapshot_ids".to_string(),
        string_vec_value(&request.snapshot_ids),
    );
    object.insert(
        "object_packs".to_string(),
        object_vec_value(&request.object_packs, object_pack_row_to_value)?,
    );
    object.insert(
        "tree_packs".to_string(),
        object_vec_value(&request.tree_packs, tree_pack_row_to_value)?,
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn plan_request_from_value(
    value: JsonValue,
) -> Result<ZstdBulkPlanRequest, String> {
    let object = object_from_value(value, "zstd bulk plan request")?;
    Ok(ZstdBulkPlanRequest {
        snapshot_ids: string_vec_from_field(&object, "snapshot_ids")?,
        object_packs: object_vec_from_field(&object, "object_packs", object_pack_row_from_object)?,
        tree_packs: object_vec_from_field(&object, "tree_packs", tree_pack_row_from_object)?,
    })
}

pub(in crate::repository_pack_json) fn validate_plan_request(
    request: &ZstdBulkPlanRequest,
) -> Result<(), String> {
    for snapshot_id in &request.snapshot_ids {
        validate_nonempty(snapshot_id, "snapshot id")?;
    }
    for pack in &request.object_packs {
        validate_nonempty(&pack.pack_id, "object pack id")?;
    }
    for pack in &request.tree_packs {
        validate_nonempty(&pack.pack_id, "tree pack id")?;
    }
    Ok(())
}

pub(in crate::repository_pack_json) fn plan_response_to_value(
    response: &ZstdBulkPlanResponse,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    insert_optional_string(&mut object, "repo_name", &response.repo_name);
    object.insert(
        "present_snapshot_ids".to_string(),
        string_vec_value(&response.present_snapshot_ids),
    );
    object.insert(
        "missing_snapshot_ids".to_string(),
        string_vec_value(&response.missing_snapshot_ids),
    );
    object.insert(
        "present_object_pack_ids".to_string(),
        string_vec_value(&response.present_object_pack_ids),
    );
    object.insert(
        "missing_object_pack_ids".to_string(),
        string_vec_value(&response.missing_object_pack_ids),
    );
    object.insert(
        "present_tree_pack_ids".to_string(),
        string_vec_value(&response.present_tree_pack_ids),
    );
    object.insert(
        "missing_tree_pack_ids".to_string(),
        string_vec_value(&response.missing_tree_pack_ids),
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn plan_response_from_value(
    value: JsonValue,
) -> Result<ZstdBulkPlanResponse, String> {
    let object = object_from_value(value, "zstd bulk plan response")?;
    Ok(ZstdBulkPlanResponse {
        repo_name: opt_string(&object, "repo_name")?,
        present_snapshot_ids: string_vec_from_field(&object, "present_snapshot_ids")?,
        missing_snapshot_ids: string_vec_from_field(&object, "missing_snapshot_ids")?,
        present_object_pack_ids: string_vec_from_field(&object, "present_object_pack_ids")?,
        missing_object_pack_ids: string_vec_from_field(&object, "missing_object_pack_ids")?,
        present_tree_pack_ids: string_vec_from_field(&object, "present_tree_pack_ids")?,
        missing_tree_pack_ids: string_vec_from_field(&object, "missing_tree_pack_ids")?,
    })
}

pub(in crate::repository_pack_json) fn validate_plan_response(
    response: &ZstdBulkPlanResponse,
) -> Result<(), String> {
    for pack_id in response
        .present_object_pack_ids
        .iter()
        .chain(response.missing_object_pack_ids.iter())
    {
        validate_nonempty(pack_id, "object pack id")?;
    }
    for pack_id in response
        .present_tree_pack_ids
        .iter()
        .chain(response.missing_tree_pack_ids.iter())
    {
        validate_nonempty(pack_id, "tree pack id")?;
    }
    Ok(())
}

pub(in crate::repository_pack_json) fn backend_payload_to_value(
    payload: &RemoteSyncBackendPayload,
) -> Result<JsonValue, String> {
    let mut capabilities = JsonMap::new();
    capabilities.insert(
        "zstd_pack_bulk".to_string(),
        JsonValue::Bool(payload.capabilities.zstd_pack_bulk),
    );
    capabilities.insert(
        "zstd_pack_bulk_download".to_string(),
        JsonValue::Bool(payload.capabilities.zstd_pack_bulk_download),
    );
    capabilities.insert(
        "zstd_pull_manifest".to_string(),
        JsonValue::Bool(payload.capabilities.zstd_pull_manifest),
    );
    capabilities.insert(
        "snapshot_dag_v2".to_string(),
        JsonValue::Bool(payload.capabilities.snapshot_dag_v2),
    );
    let mut diff = JsonMap::new();
    diff.insert(
        "checked_snapshot_ids".to_string(),
        string_vec_value(&payload.diff.checked_snapshot_ids),
    );
    diff.insert(
        "present_snapshot_ids".to_string(),
        string_vec_value(&payload.diff.present_snapshot_ids),
    );
    diff.insert(
        "missing_snapshot_ids".to_string(),
        string_vec_value(&payload.diff.missing_snapshot_ids),
    );
    let mut object = JsonMap::new();
    object.insert(
        "backend".to_string(),
        string_value(payload.backend.as_str()),
    );
    object.insert("reason".to_string(), string_value(payload.reason.clone()));
    object.insert("capabilities".to_string(), JsonValue::Object(capabilities));
    object.insert("diff".to_string(), JsonValue::Object(diff));
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn backend_payload_from_value(
    value: JsonValue,
) -> Result<RemoteSyncBackendPayload, String> {
    let object = object_from_value(value, "remote sync backend payload")?;
    let capabilities = optional_object_value(&object, "capabilities")?.unwrap_or_default();
    let diff = optional_object_value(&object, "diff")?.unwrap_or_default();
    Ok(RemoteSyncBackendPayload {
        backend: remote_sync_backend_kind_from_name(&req_string(&object, "backend")?)?,
        reason: req_string(&object, "reason")?,
        capabilities: RemoteSyncCapabilities {
            zstd_pack_bulk: opt_bool(&capabilities, "zstd_pack_bulk")?.unwrap_or(false),
            zstd_pack_bulk_download: opt_bool(&capabilities, "zstd_pack_bulk_download")?
                .unwrap_or(false),
            zstd_pull_manifest: opt_bool(&capabilities, "zstd_pull_manifest")?.unwrap_or(false),
            snapshot_dag_v2: opt_bool(&capabilities, "snapshot_dag_v2")?.unwrap_or(false),
        },
        diff: RemoteSyncInventoryDiff {
            checked_snapshot_ids: string_vec_from_field(&diff, "checked_snapshot_ids")?,
            present_snapshot_ids: string_vec_from_field(&diff, "present_snapshot_ids")?,
            missing_snapshot_ids: string_vec_from_field(&diff, "missing_snapshot_ids")?,
        },
    })
}

pub(in crate::repository_pack_json) fn validate_backend_payload(
    payload: &RemoteSyncBackendPayload,
) -> Result<(), String> {
    validate_nonempty(&payload.reason, "remote sync backend reason")
}

pub(in crate::repository_pack_json) fn commit_request_to_value(
    request: &ZstdBulkCommitRequest,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    insert_optional_string(&mut object, "contract", &request.contract);
    insert_optional_string(&mut object, "generation_key", &request.generation_key);
    object.insert(
        "object_packs".to_string(),
        object_vec_value(&request.object_packs, object_pack_row_to_value)?,
    );
    object.insert(
        "tree_packs".to_string(),
        object_vec_value(&request.tree_packs, tree_pack_row_to_value)?,
    );
    object.insert(
        "blob_locators".to_string(),
        object_vec_value(&request.blob_locators, blob_locator_row_to_value)?,
    );
    object.insert(
        "tree_locators".to_string(),
        object_vec_value(&request.tree_locators, tree_locator_row_to_value)?,
    );
    object.insert(
        "snapshots".to_string(),
        object_vec_value(&request.snapshots, snapshot_row_to_value)?,
    );
    object.insert(
        "line_update".to_string(),
        request
            .line_update
            .as_ref()
            .map(line_update_to_value)
            .transpose()?
            .unwrap_or(JsonValue::Null),
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn commit_request_from_value(
    value: JsonValue,
) -> Result<ZstdBulkCommitRequest, String> {
    let object = object_from_value(value, "zstd bulk commit request")?;
    Ok(ZstdBulkCommitRequest {
        contract: opt_string(&object, "contract")?,
        generation_key: opt_string(&object, "generation_key")?,
        object_packs: object_vec_from_field(&object, "object_packs", object_pack_row_from_object)?,
        tree_packs: object_vec_from_field(&object, "tree_packs", tree_pack_row_from_object)?,
        blob_locators: object_vec_from_field(
            &object,
            "blob_locators",
            blob_locator_row_from_object,
        )?,
        tree_locators: object_vec_from_field(
            &object,
            "tree_locators",
            tree_locator_row_from_object,
        )?,
        snapshots: object_vec_from_field(&object, "snapshots", snapshot_row_from_object)?,
        line_update: optional_object_value(&object, "line_update")?
            .map(line_update_from_object)
            .transpose()?,
    })
}

pub(in crate::repository_pack_json) fn validate_commit_request(
    request: &ZstdBulkCommitRequest,
) -> Result<(), String> {
    for pack in &request.object_packs {
        validate_nonempty(&pack.pack_id, "object pack id")?;
    }
    for pack in &request.tree_packs {
        validate_nonempty(&pack.pack_id, "tree pack id")?;
    }
    for snapshot in &request.snapshots {
        validate_nonempty(&snapshot.snapshot_id, "snapshot id")?;
    }
    Ok(())
}

pub(in crate::repository_pack_json) fn commit_response_to_value(
    response: &ZstdBulkCommitResponse,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    insert_optional_string(&mut object, "repo_name", &response.repo_name);
    object.insert(
        "committed_snapshot_ids".to_string(),
        string_vec_value(&response.committed_snapshot_ids),
    );
    object.insert(
        "committed_object_pack_ids".to_string(),
        string_vec_value(&response.committed_object_pack_ids),
    );
    object.insert(
        "committed_tree_pack_ids".to_string(),
        string_vec_value(&response.committed_tree_pack_ids),
    );
    insert_optional_i64(
        &mut object,
        "upserted_snapshots",
        response.upserted_snapshots,
    );
    insert_optional_value(
        &mut object,
        "remote_line",
        response
            .remote_line
            .as_ref()
            .map(remote_line_to_value)
            .transpose()?,
    );
    insert_optional_value(
        &mut object,
        "line_update",
        response
            .line_update
            .as_ref()
            .map(line_update_result_to_value)
            .transpose()?,
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn commit_response_from_value(
    value: JsonValue,
) -> Result<ZstdBulkCommitResponse, String> {
    let object = object_from_value(value, "zstd bulk commit response")?;
    Ok(ZstdBulkCommitResponse {
        repo_name: opt_string(&object, "repo_name")?,
        committed_snapshot_ids: string_vec_from_field(&object, "committed_snapshot_ids")?,
        committed_object_pack_ids: string_vec_from_field(&object, "committed_object_pack_ids")?,
        committed_tree_pack_ids: string_vec_from_field(&object, "committed_tree_pack_ids")?,
        upserted_snapshots: opt_i64(&object, "upserted_snapshots")?,
        remote_line: optional_object_value(&object, "remote_line")?
            .map(remote_line_from_object)
            .transpose()?,
        line_update: optional_object_value(&object, "line_update")?
            .map(line_update_result_from_object)
            .transpose()?,
    })
}

pub(in crate::repository_pack_json) fn validate_commit_response(
    response: &ZstdBulkCommitResponse,
) -> Result<(), String> {
    for snapshot_id in &response.committed_snapshot_ids {
        validate_nonempty(snapshot_id, "snapshot id")?;
    }
    Ok(())
}

pub(in crate::repository_pack_json) fn pack_upload_response_to_value(
    response: &ZstdPackUploadResponse,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    insert_optional_string(&mut object, "repo_name", &response.repo_name);
    object.insert(
        "pack_id".to_string(),
        string_value(response.pack_id.clone()),
    );
    insert_optional_bool(&mut object, "stored", response.stored);
    if let Some(pack_format) = &response.pack_format {
        object.insert(
            "pack_format".to_string(),
            string_value(zstd_pack_format_name(pack_format)),
        );
    }
    insert_optional_string(&mut object, "checksum", &response.checksum);
    insert_optional_i64(&mut object, "pack_bytes", response.pack_bytes);
    insert_optional_bool(&mut object, "raw_binary_upload", response.raw_binary_upload);
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn pack_upload_response_from_value(
    value: JsonValue,
) -> Result<ZstdPackUploadResponse, String> {
    let object = object_from_value(value, "zstd pack upload response")?;
    Ok(ZstdPackUploadResponse {
        repo_name: opt_string(&object, "repo_name")?,
        pack_id: req_string(&object, "pack_id")?,
        stored: opt_bool(&object, "stored")?,
        pack_format: opt_string(&object, "pack_format")?
            .map(|format| zstd_pack_format_from_name(&format))
            .transpose()?,
        checksum: opt_string(&object, "checksum")?,
        pack_bytes: opt_i64(&object, "pack_bytes")?,
        raw_binary_upload: opt_bool(&object, "raw_binary_upload")?,
    })
}

pub(in crate::repository_pack_json) fn validate_pack_upload_response(
    response: &ZstdPackUploadResponse,
) -> Result<(), String> {
    validate_nonempty(&response.pack_id, "pack id")
}

pub(in crate::repository_pack_json) fn optional_object_value(
    object: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<JsonMap<String, JsonValue>>, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("Field `{key}` must be a JSON object or null.")),
    }
}

pub(in crate::repository_pack_json) fn remote_sync_backend_kind_from_name(
    value: &str,
) -> Result<RemoteSyncBackendKind, String> {
    match value.trim() {
        "zstd_pack_bulk" => Ok(RemoteSyncBackendKind::ZstdPackBulk),
        other => Err(format!("Unsupported remote sync backend: {other}")),
    }
}

pub(in crate::repository_pack_json) fn zstd_pack_format_name(
    format: &ZstdPackFormat,
) -> &'static str {
    match format {
        ZstdPackFormat::Object(format) => format.persisted_name(),
        ZstdPackFormat::Tree(format) => format.persisted_name(),
    }
}

pub(in crate::repository_pack_json) fn zstd_pack_format_from_name(
    value: &str,
) -> Result<ZstdPackFormat, String> {
    PackFormatKind::from_persisted(value)
        .map(ZstdPackFormat::Object)
        .or_else(|_| TreePackFormatKind::from_persisted(value).map(ZstdPackFormat::Tree))
}
