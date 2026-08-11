use super::*;
#[cfg(feature = "legacy-postgres-runtime")]
use ::postgres as pg;

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

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn repository_json_with_pack_storage(
    client: &mut pg::Client,
    row: RepositoryRow,
) -> Result<JsonValue, NativeRepositoryError> {
    let pack_storage = repository_pack_storage_payload_json(client, &row)?;
    let mut payload = repository_json(row);
    let object = payload.as_object_mut().ok_or_else(|| {
        NativeRepositoryError::internal("repository payload must be a JSON object")
    })?;
    object.insert(
        REPOSITORY_PACK_STORAGE_PAYLOAD_FIELD.to_string(),
        pack_storage,
    );
    Ok(payload)
}

#[cfg(feature = "legacy-postgres-runtime")]
fn repository_pack_storage_payload_json(
    client: &mut pg::Client,
    row: &RepositoryRow,
) -> Result<JsonValue, NativeRepositoryError> {
    let object_formats = pack_format_counts(
        client,
        "select pack_format, count(*)::bigint as count from packs where repo_id = $1 group by pack_format",
        &[&row.repo_id],
    )?;
    let tree_formats = pack_format_counts(
        client,
        "select pack_format, count(*)::bigint as count from tree_packs where repo_id = $1 or (repo_id is null and repo_name = $2) group by pack_format",
        &[&row.repo_id, &row.repo_name],
    )?;
    let object_pack_count = count_total(&object_formats);
    let tree_pack_count = count_total(&tree_formats);
    let zstd_object_pack_count = count_for_format(&object_formats, PACK_FORMAT_ZSTD_CHUNKED_V1);
    let zstd_tree_pack_count = count_for_format(&tree_formats, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1);
    let zstd_only = (object_pack_count == 0 && tree_pack_count == 0)
        || (zstd_object_pack_count == object_pack_count && zstd_tree_pack_count == tree_pack_count);
    if !zstd_only {
        return Err(NativeRepositoryError::bad_request(format!(
            "Repository {} contains unsupported non-current pack formats; only zstd-chunked object and tree packs are accepted.",
            row.repo_name
        )));
    }
    Ok(json!({
        "contract": REPOSITORY_PACK_STORAGE_CONTRACT,
        "zstd_only_verified": true,
        "object_pack_format": object_pack_format_summary(&object_formats),
        "tree_pack_format": tree_pack_format_summary(&tree_formats),
        "object_pack_count": object_pack_count,
        "tree_pack_count": tree_pack_count,
        "zstd_object_pack_count": zstd_object_pack_count,
        "zstd_tree_pack_count": zstd_tree_pack_count,
        "requires_zstd_remote_sync": true,
        "validation": {
            "state": "valid",
            "error_count": 0,
        },
    }))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::foundation::native_repositories) enum ZstdOnlyRepositoryFlow {
    ZstdBulkPlan,
    ZstdBulkCommit,
    ZstdImportManifest,
    SnapshotMaterialize,
    LineUpdate,
}

impl ZstdOnlyRepositoryFlow {
    fn label(self) -> &'static str {
        match self {
            Self::ZstdBulkPlan => "zstd bulk plan",
            Self::ZstdBulkCommit => "zstd bulk commit",
            Self::ZstdImportManifest => "zstd import manifest export",
            Self::SnapshotMaterialize => "snapshot materialize",
            Self::LineUpdate => "line update",
        }
    }
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn ensure_zstd_only_repository_flow_allowed(
    client: &mut pg::Client,
    repo_name: &str,
    repo: &RepositoryRow,
    flow: ZstdOnlyRepositoryFlow,
) -> Result<(), NativeRepositoryError> {
    let pack_storage = repository_pack_storage_payload_json(client, repo)?;
    ensure_zstd_only_repository_flow_allowed_for_pack_storage(repo_name, &pack_storage, flow)
}

pub(in crate::foundation::native_repositories) fn ensure_zstd_only_repository_flow_allowed_for_pack_storage(
    repo_name: &str,
    pack_storage: &JsonValue,
    flow: ZstdOnlyRepositoryFlow,
) -> Result<(), NativeRepositoryError> {
    let zstd_only_verified = pack_storage
        .get("zstd_only_verified")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    if zstd_only_verified {
        return Ok(());
    }
    Err(NativeRepositoryError::bad_request(format!(
        "Repository {repo_name} has not passed current zstd-only pack validation; {} is unavailable.",
        flow.label()
    )))
}

pub(in crate::foundation::native_repositories) fn repository_pack_storage_capability_json(
) -> JsonValue {
    json!({
        "contract": REPOSITORY_PACK_STORAGE_CONTRACT,
        "payload_field": REPOSITORY_PACK_STORAGE_PAYLOAD_FIELD,
        "missing_payload_default": REPOSITORY_PACK_STORAGE_MISSING_PAYLOAD_DEFAULT,
    })
}

#[cfg(feature = "legacy-postgres-runtime")]
fn pack_format_counts(
    client: &mut pg::Client,
    sql: &str,
    params: &[&(dyn pg::types::ToSql + Sync)],
) -> Result<Vec<(String, u64)>, NativeRepositoryError> {
    let rows = client.query(sql, params).map_err(db_internal)?;
    rows.into_iter()
        .map(|row| {
            let count = row.get::<_, i64>("count");
            if count < 0 {
                return Err(NativeRepositoryError::internal(
                    "pack format count cannot be negative",
                ));
            }
            Ok((row.get::<_, String>("pack_format"), count as u64))
        })
        .collect()
}

fn count_total(values: &[(String, u64)]) -> u64 {
    values.iter().map(|(_, count)| *count).sum()
}

fn count_for_format(values: &[(String, u64)], expected: &str) -> u64 {
    values
        .iter()
        .filter(|(format, _)| format == expected)
        .map(|(_, count)| *count)
        .sum()
}

pub(in crate::foundation::native_repositories) fn object_pack_format_summary(
    values: &[(String, u64)],
) -> String {
    pack_format_summary(values, PACK_FORMAT_ZSTD_CHUNKED_V1)
}

pub(in crate::foundation::native_repositories) fn tree_pack_format_summary(
    values: &[(String, u64)],
) -> String {
    pack_format_summary(values, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1)
}

fn pack_format_summary(values: &[(String, u64)], empty_default: &str) -> String {
    match values {
        [] => empty_default.to_string(),
        [(format, _)] => format.clone(),
        _ => "mixed".to_string(),
    }
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn line_json(
    row: pg::Row,
) -> Result<JsonValue, NativeRepositoryError> {
    let line_name: String = row.get("line_name");
    let head_snapshot_id: Option<String> = row.get("head_snapshot_id");
    Ok(json!({
        "repo_name": row.get::<_, String>("repo_name"),
        "repo_id": row.get::<_, String>("repo_id"),
        "line_name": line_name,
        "head_snapshot_id": head_snapshot_id,
        "status": normalize_status(row.get::<_, String>("status")),
        "archived_at": row.get::<_, Option<String>>("archived_at_text"),
        "created_at": row.get::<_, String>("created_at_text"),
        "updated_at": row.get::<_, String>("updated_at_text"),
    }))
}

pub(in crate::foundation::native_repositories) fn normalize_status(value: String) -> String {
    let text = value.trim().to_string();
    if text.is_empty() {
        "active".to_string()
    } else {
        text
    }
}

pub(in crate::foundation::native_repositories) fn normalize_policy_json(
    value: &JsonValue,
) -> Result<JsonValue, NativeRepositoryError> {
    match value {
        JsonValue::Null => Ok(JsonValue::Object(JsonMap::new())),
        JsonValue::Object(_) => Ok(value.clone()),
        _ => Err(NativeRepositoryError::bad_request(
            "policy must be a JSON object when provided",
        )),
    }
}

pub(in crate::foundation::native_repositories) fn remote_sync_capabilities() -> JsonValue {
    RemoteSyncPlanJson::stateless().capabilities_payload()
}

pub(in crate::foundation::native_repositories) fn parse_policy_json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap_or_else(|_| JsonValue::Object(JsonMap::new()))
}

pub(in crate::foundation::native_repositories) fn normalize_namespace_prefix(
    value: Option<String>,
) -> Result<String, NativeRepositoryError> {
    let text = value.unwrap_or_default().trim().to_string();
    if text.is_empty() {
        return Ok(String::new());
    }
    if text
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Ok(text)
    } else {
        Err(NativeRepositoryError::bad_request(
            "id_namespace_prefix must contain only ASCII alphanumeric characters, `_`, or `-`",
        ))
    }
}
