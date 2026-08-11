use super::*;

pub(super) const BINARY_ZSTD_OBJECT_PACK_KIND: &str = "zstd_object_pack";
pub(super) const BINARY_ZSTD_TREE_PACK_KIND: &str = "zstd_tree_pack";
pub(super) const BINARY_ZSTD_BLOB_LOCATOR_KIND: &str = "zstd_blob_locator";
pub(super) const BINARY_ZSTD_TREE_LOCATOR_KIND: &str = "zstd_tree_locator";
pub(super) const BINARY_ZSTD_PAYLOAD_KIND_FIELD: &str = "binary_payload_kind";

pub(super) fn binary_native_repository_store_error(error: BinaryDbError) -> NativeRepositoryError {
    match error.kind() {
        BinaryDbErrorKind::MissingData => NativeRepositoryError::not_found(error.to_string()),
        BinaryDbErrorKind::InvalidDomainData => {
            NativeRepositoryError::bad_request(error.to_string())
        }
        BinaryDbErrorKind::RetryableBusy => {
            NativeRepositoryError::service_unavailable(error.to_string())
        }
        BinaryDbErrorKind::Corruption | BinaryDbErrorKind::LayoutMismatch => {
            NativeRepositoryError::internal(error.to_string())
        }
        BinaryDbErrorKind::Io | BinaryDbErrorKind::Unsupported | BinaryDbErrorKind::Other => {
            NativeRepositoryError::internal(error.to_string())
        }
    }
}

pub(in crate::foundation::native_repositories) fn binary_json_text(
    value: &JsonValue,
    field: &str,
) -> Option<String> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

pub(super) fn binary_line_name(value: &JsonValue, repo_name: &str) -> Option<String> {
    binary_json_text(value, "line_name")
        .or_else(|| binary_json_text(value, "line"))
        .or_else(|| binary_json_text(value, "target_line"))
        .or_else(|| binary_json_text(value, "name"))
        .or_else(|| {
            binary_json_text(value, "repo_line_name").and_then(|repo_line_name| {
                repo_line_name
                    .strip_prefix(format!("{repo_name}:").as_str())
                    .map(ToString::to_string)
            })
        })
}

pub(in crate::foundation::native_repositories) fn binary_snapshot_id(
    value: &JsonValue,
) -> Option<String> {
    binary_json_text(value, "snapshot_id")
        .or_else(|| binary_json_text(value, "snapshot"))
        .or_else(|| binary_json_text(value, "id"))
}

pub(super) fn binary_line_head(value: &JsonValue) -> Option<String> {
    binary_json_text(value, "head_snapshot_id")
        .or_else(|| binary_json_text(value, "head"))
        .or_else(|| binary_json_text(value, "target_line_head"))
        .or_else(|| binary_json_text(value, "snapshot_id"))
}

pub(in crate::foundation::native_repositories) fn binary_created_at_value(
    value: &JsonValue,
) -> JsonValue {
    binary_json_text(value, "created_at")
        .or_else(|| binary_json_text(value, "updated_at"))
        .or_else(|| binary_json_text(value, "landed_at"))
        .map(JsonValue::String)
        .unwrap_or(JsonValue::Null)
}

pub(super) fn binary_updated_at_value(value: &JsonValue) -> JsonValue {
    binary_json_text(value, "updated_at")
        .or_else(|| binary_json_text(value, "landed_at"))
        .or_else(|| binary_json_text(value, "created_at"))
        .map(JsonValue::String)
        .unwrap_or(JsonValue::Null)
}

pub(super) fn binary_line_response(
    value: &JsonValue,
    default_repo_id: &str,
    line_name: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo_name = binary_json_text(value, "repo_name").ok_or_else(|| {
        NativeRepositoryError::internal("Binary DB line payload is missing repo_name")
    })?;
    let repo_id = binary_json_text(value, "repo_id").unwrap_or_else(|| default_repo_id.to_string());
    let status = normalize_status(binary_json_text(value, "status").unwrap_or_else(|| {
        if value
            .get("archived_at")
            .and_then(JsonValue::as_str)
            .is_some()
        {
            "archived".to_string()
        } else {
            "active".to_string()
        }
    }));
    Ok(json!({
        "repo_name": repo_name,
        "repo_id": repo_id,
        "line_name": line_name,
        "head_snapshot_id": binary_line_head(value),
        "status": status,
        "archived_at": value.get("archived_at").cloned().unwrap_or(JsonValue::Null),
        "created_at": binary_created_at_value(value),
        "updated_at": binary_updated_at_value(value),
    }))
}

pub(super) fn binary_line_payload(
    repo_name: &str,
    repo_id: &str,
    line_name: &str,
    head_snapshot_id: Option<&str>,
    status: &str,
    archived_at: JsonValue,
    created_at: JsonValue,
    updated_at: String,
) -> JsonValue {
    json!({
        "repo_name": repo_name,
        "repo_id": repo_id,
        "repo_line_name": line_name,
        "line_name": line_name,
        "line": line_name,
        "target_line": line_name,
        "head_snapshot_id": head_snapshot_id,
        "head": head_snapshot_id,
        "target_line_head": head_snapshot_id,
        "status": status,
        "archived_at": archived_at,
        "created_at": created_at,
        "updated_at": updated_at,
    })
}

pub(super) fn binary_repository_pack_storage_payload_json() -> JsonValue {
    json!({
        "contract": REPOSITORY_PACK_STORAGE_CONTRACT,
        "zstd_only_verified": true,
        "object_pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
        "tree_pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        "object_pack_count": 0_u64,
        "tree_pack_count": 0_u64,
        "zstd_object_pack_count": 0_u64,
        "zstd_tree_pack_count": 0_u64,
        "requires_zstd_remote_sync": true,
        "validation": {
            "state": "valid",
            "error_count": 0,
        },
    })
}
