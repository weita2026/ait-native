use super::*;
pub(in crate::foundation::native_repositories) const REPOSITORY_PACK_STORAGE_CONTRACT: &str =
    "ait.repository.pack_storage.v1";
pub(in crate::foundation::native_repositories) const REPOSITORY_PACK_STORAGE_PAYLOAD_FIELD: &str =
    "pack_storage";
pub(in crate::foundation::native_repositories) const REPOSITORY_PACK_STORAGE_MISSING_PAYLOAD_DEFAULT: &str = "zstd_only";

pub(in crate::foundation::native_repositories) fn repository_json(row: RepositoryRow) -> JsonValue {
    json!({
        "repo_name": row.repo_name,
        "repo_id": row.repo_id,
        "default_line": row.default_line,
        "lifecycle_state": row.lifecycle_state,
        "id_namespace_prefix": row.id_namespace_prefix,
        "policy": parse_policy_json(row.policy_json.as_str()),
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "capabilities": remote_sync_capabilities(),
    })
}
pub(in crate::foundation::native_repositories) fn repository_pack_storage_capability_json(
) -> JsonValue {
    json!({
        "contract": REPOSITORY_PACK_STORAGE_CONTRACT,
        "payload_field": REPOSITORY_PACK_STORAGE_PAYLOAD_FIELD,
        "missing_payload_default": REPOSITORY_PACK_STORAGE_MISSING_PAYLOAD_DEFAULT,
    })
}
pub(in crate::foundation::native_repositories) fn normalize_status(value: String) -> String {
    let text = value.trim().to_string();
    if text.is_empty() {
        "active".to_string()
    } else {
        text
    }
}

pub(in crate::foundation::native_repositories) fn remote_sync_capabilities() -> JsonValue {
    RemoteSyncPlanJson::stateless().capabilities_payload()
}

pub(in crate::foundation::native_repositories) fn parse_policy_json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap_or_else(|_| JsonValue::Object(JsonMap::new()))
}
