use super::*;

const ZSTD_OBJECT_INDEX_ENTRY_NAME: &str = "zstd-chunked-object-index";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResolvedPlanPackedContent {
    pub(super) body: String,
    pub(super) blob_id: String,
    pub(super) sha256: String,
    pub(super) size_bytes: u64,
    pub(super) object_pack_id: String,
    pub(super) object_pack_created_at_s: u64,
    pub(super) pack_entry_type: String,
    pub(super) pack_base_blob_id: Option<String>,
    pub(super) pack_chain_depth: u8,
}

impl ResolvedPlanPackedContent {
    pub(super) fn blob_json(
        &self,
        plan_revision_id: &str,
        repo_name: &str,
        repo_id: &str,
        fallback_created_at: &str,
    ) -> JsonMap<String, JsonValue> {
        let created_at = timestamp_string(self.object_pack_created_at_s)
            .unwrap_or_else(|_| fallback_created_at.to_string());
        JsonMap::from_iter([
            ("plan_revision_id".to_string(), json!(plan_revision_id)),
            ("repo_name".to_string(), json!(repo_name)),
            ("repo_id".to_string(), json!(repo_id)),
            ("blob_id".to_string(), json!(self.blob_id)),
            (
                "media_type".to_string(),
                json!("text/markdown; charset=utf-8"),
            ),
            ("encoding".to_string(), json!("utf-8")),
            ("byte_count".to_string(), json!(self.size_bytes)),
            ("created_at".to_string(), json!(created_at)),
            ("storage_authority".to_string(), json!("remote_zstd_pack")),
            ("object_pack_id".to_string(), json!(self.object_pack_id)),
            (
                "object_pack_format".to_string(),
                json!(PACK_FORMAT_ZSTD_CHUNKED_V1),
            ),
            ("sha256".to_string(), json!(self.sha256)),
        ])
    }
}

impl<D, const WRITE_LAYOUT: u32> ServerPlanBinaryDbStore<D, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + Clone,
{
    pub(super) fn resolve_requested_packed_content(
        &self,
        artifact_path: &str,
        artifact_body: Option<&str>,
        packed_artifact: Option<&JsonValue>,
    ) -> Result<Option<ResolvedPlanPackedContent>, String> {
        let Some(packed_artifact) = packed_artifact.filter(|value| !value.is_null()) else {
            if artifact_body.is_some() {
                return Err(
                    "Binary DB Plan writes with artifact_body require the existing same-repository packed_artifact locator."
                        .to_string(),
                );
            }
            return Ok(None);
        };
        let packed = packed_artifact
            .as_object()
            .ok_or_else(|| "Binary DB Plan packed_artifact must be a JSON object.".to_string())?;
        require_packed_text(packed, "storage_authority", "packed_artifact")?
            .eq("remote_zstd_pack")
            .then_some(())
            .ok_or_else(|| {
                "Binary DB Plan packed_artifact storage_authority must be remote_zstd_pack."
                    .to_string()
            })?;
        require_packed_text(packed, "artifact_path", "packed_artifact")?
            .eq(artifact_path)
            .then_some(())
            .ok_or_else(|| {
                format!(
                    "Binary DB Plan packed_artifact path does not match revision artifact_path {artifact_path}."
                )
            })?;
        require_exact_packed_value(
            packed,
            "media_type",
            "text/markdown; charset=utf-8",
            "packed_artifact",
        )?;
        require_exact_packed_value(packed, "encoding", "utf-8", "packed_artifact")?;

        let read = self.read_txn_with_content();
        let resolved = self.resolve_packed_content_blob_with_read(
            &read,
            &require_packed_text(packed, "artifact_blob_id", "packed_artifact")?,
        )?;
        self.validate_packed_request(packed, artifact_body, &resolved)?;
        Ok(Some(resolved))
    }

    pub(super) fn resolve_packed_content_blob_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        artifact_blob_id: &str,
    ) -> Result<ResolvedPlanPackedContent, String> {
        if artifact_blob_id.trim().is_empty() {
            return Err("Binary DB Plan packed content requires artifact_blob_id.".to_string());
        }
        let content = ServerBinaryRepositoryContentStore::new(self.db.clone());
        let blob = content
            .blob_with_read(read, artifact_blob_id)
            .map_err(binary_error)?
            .ok_or_else(|| {
                format!("Binary DB Plan artifact blob {artifact_blob_id} is missing.")
            })?;
        let sha256 = hex_lower(&blob.record.sha256);
        let bytes = content
            .blob_bytes_with_read(read, artifact_blob_id)
            .map_err(binary_error)?
            .ok_or_else(|| {
                format!("Binary DB Plan artifact blob {artifact_blob_id} has no pack bytes.")
            })?;
        let body = String::from_utf8(bytes).map_err(|error| {
            format!("Binary DB Plan artifact blob {artifact_blob_id} is not UTF-8: {error}")
        })?;
        let member_kind = blob.member.member_meta & 0b0000_0011;
        let pack_entry_type = match member_kind {
            0 => "full",
            1 => "delta",
            other => {
                return Err(format!(
                    "Binary DB Plan artifact blob {artifact_blob_id} has unsupported pack member kind {other}."
                ))
            }
        }
        .to_string();
        Ok(ResolvedPlanPackedContent {
            body,
            blob_id: blob.blob_id,
            sha256,
            size_bytes: blob.record.size_bytes,
            object_pack_id: blob.pack_id,
            object_pack_created_at_s: blob.pack.created_at_s,
            pack_entry_type,
            pack_base_blob_id: blob.base_blob_id,
            pack_chain_depth: blob.member.delta_chain_depth,
        })
    }

    fn validate_packed_request(
        &self,
        packed: &JsonMap<String, JsonValue>,
        artifact_body: Option<&str>,
        resolved: &ResolvedPlanPackedContent,
    ) -> Result<(), String> {
        require_packed_u64(packed, "byte_count", "packed_artifact")?
            .eq(&resolved.size_bytes)
            .then_some(())
            .ok_or_else(|| "Binary DB Plan packed_artifact byte_count mismatch.".to_string())?;
        if artifact_body.is_some_and(|body| body != resolved.body) {
            return Err(
                "Binary DB Plan artifact_body disagrees with the committed packed blob."
                    .to_string(),
            );
        }

        let object_pack = require_packed_object(packed, "object_pack", "packed_artifact")?;
        require_matching_id(
            require_packed_text(object_pack, "pack_id", "object_pack")?,
            &resolved.object_pack_id,
            "object_pack.pack_id",
        )?;
        require_exact_packed_value(
            object_pack,
            "pack_format",
            PACK_FORMAT_ZSTD_CHUNKED_V1,
            "object_pack",
        )?;
        require_exact_packed_value(
            object_pack,
            "pack_index_entry_name",
            ZSTD_OBJECT_INDEX_ENTRY_NAME,
            "object_pack",
        )?;
        let content = ServerBinaryRepositoryContentStore::new(self.db.clone());
        let object_pack_path = content.object_pack_path(&resolved.object_pack_id);
        let actual_object_index_checksum = read_pack_index_checksum_with_format(
            path_text(&object_pack_path)?,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .map_err(|error| {
            format!(
                "Binary DB Plan cannot read object-pack index for {}: {error}",
                resolved.object_pack_id
            )
        })?;
        require_matching_checksum(
            require_packed_text(object_pack, "pack_index_checksum", "object_pack")?,
            &actual_object_index_checksum,
            "object_pack.pack_index_checksum",
        )?;

        let locator = require_packed_object(packed, "blob_locator", "packed_artifact")?;
        require_matching_id(
            require_packed_text(locator, "blob_id", "blob_locator")?,
            &resolved.blob_id,
            "blob_locator.blob_id",
        )?;
        require_matching_checksum(
            require_packed_text(locator, "sha256", "blob_locator")?,
            &resolved.sha256,
            "blob_locator.sha256",
        )?;
        if require_packed_u64(locator, "size_bytes", "blob_locator")? != resolved.size_bytes {
            return Err("Binary DB Plan blob_locator.size_bytes mismatch.".to_string());
        }
        require_matching_id(
            require_packed_text(locator, "pack_id", "blob_locator")?,
            &resolved.object_pack_id,
            "blob_locator.pack_id",
        )?;
        require_exact_packed_value(
            locator,
            "pack_entry_type",
            &resolved.pack_entry_type,
            "blob_locator",
        )?;
        let requested_base = optional_packed_text(locator, "pack_base_blob_id", "blob_locator")?;
        if requested_base.as_deref() != resolved.pack_base_blob_id.as_deref() {
            return Err("Binary DB Plan blob_locator.pack_base_blob_id mismatch.".to_string());
        }
        if require_packed_u64(locator, "pack_chain_depth", "blob_locator")?
            != u64::from(resolved.pack_chain_depth)
        {
            return Err("Binary DB Plan blob_locator.pack_chain_depth mismatch.".to_string());
        }

        Ok(())
    }
}

fn require_packed_object<'a>(
    object: &'a JsonMap<String, JsonValue>,
    field: &str,
    context: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    object
        .get(field)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Binary DB Plan {context}.{field} must be an object."))
}

fn require_packed_text(
    object: &JsonMap<String, JsonValue>,
    field: &str,
    context: &str,
) -> Result<String, String> {
    object
        .get(field)
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Binary DB Plan {context}.{field} must be non-empty text."))
}

fn optional_packed_text(
    object: &JsonMap<String, JsonValue>,
    field: &str,
    context: &str,
) -> Result<Option<String>, String> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        _ => Err(format!(
            "Binary DB Plan {context}.{field} must be non-empty text or null."
        )),
    }
}

fn require_packed_u64(
    object: &JsonMap<String, JsonValue>,
    field: &str,
    context: &str,
) -> Result<u64, String> {
    object
        .get(field)
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| format!("Binary DB Plan {context}.{field} must be a u64 integer."))
}

fn require_exact_packed_value(
    object: &JsonMap<String, JsonValue>,
    field: &str,
    expected: &str,
    context: &str,
) -> Result<(), String> {
    let actual = require_packed_text(object, field, context)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "Binary DB Plan {context}.{field} is {actual}, expected {expected}."
        ))
    }
}

fn require_matching_id(actual: String, expected: &str, field: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "Binary DB Plan {field} is {actual}, expected {expected}."
        ))
    }
}

fn require_matching_checksum(actual: String, expected: &str, field: &str) -> Result<(), String> {
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!("Binary DB Plan {field} checksum mismatch."))
    }
}

fn path_text(path: &std::path::Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("Binary DB pack path is not UTF-8: {}", path.display()))
}
