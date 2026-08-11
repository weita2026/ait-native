use crate::json_support::{json, JsonMap, JsonValue};
use crate::local_snapshot::SnapshotFileRow;
use crate::object_diff::{self, BlobReader, ObjectReader, SnapshotReader};
use crate::plan_http_client::{
    build_plan_http_request_spec, configured_repository_authority_path_segment,
    encode_path_segment, PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientResult,
    PlanHttpRequestSpec,
};
use crate::snapshot_store::SnapshotRecord;
use reqwest::Method;
use std::collections::BTreeMap;

pub struct SnapshotJson<S> {
    store: S,
}

impl<S> SnapshotJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl SnapshotJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> SnapshotJson<S> {
    pub fn snapshot_record_payload(&self, snapshot: &SnapshotRecord) -> JsonValue {
        let _ = &self.store;
        json!({
            "snapshot_id": &snapshot.snapshot_id,
            "parent_snapshot_ids": &snapshot.parent_snapshot_ids,
            "primary_parent_snapshot_id": &snapshot.primary_parent_snapshot_id,
            "parent_snapshot_id": &snapshot.parent_snapshot_id,
            "root_tree_pack_id": &snapshot.root_tree_pack_id,
            "root_entry_ordinal": &snapshot.root_entry_ordinal,
            "manifest_hash": &snapshot.manifest_hash,
            "message": &snapshot.message,
            "line_name": &snapshot.line_name,
            "snapshot_kind": &snapshot.snapshot_kind,
            "file_count": snapshot.file_count,
            "total_bytes": snapshot.total_bytes,
            "created_at": &snapshot.created_at,
        })
    }

    pub fn snapshot_file_row_payload(&self, row: &SnapshotFileRow) -> JsonValue {
        let _ = &self.store;
        json!({
            "path": &row.path,
            "blob_id": &row.blob_id,
            "size_bytes": row.size_bytes,
            "mode": &row.mode,
            "sha256": &row.sha256,
        })
    }

    pub fn build_get_remote_snapshot_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        _repo_name: &str,
        snapshot_id: &str,
        include_content: bool,
        path: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let repository_index = configured_repository_authority_path_segment(config)?;
        let snapshot_id = encode_path_segment(&require_plan_http_non_empty_text(
            snapshot_id,
            "snapshot_id",
        )?);
        build_plan_http_request_spec(
            config,
            Method::GET,
            &format!(
                "/v1/native/repository-authorities/{repository_index}/snapshots/{snapshot_id}"
            ),
            snapshot_query_pairs(include_content, path),
            None,
        )
    }

    pub fn build_get_remote_snapshots_existence_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        _repo_name: &str,
        snapshot_ids: &[String],
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let repository_index = configured_repository_authority_path_segment(config)?;
        let mut body = JsonMap::new();
        body.insert(
            "snapshot_ids".to_string(),
            JsonValue::Array(
                snapshot_ids
                    .iter()
                    .filter_map(|value| normalize_optional_str(Some(value.as_str())))
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
        build_plan_http_request_spec(
            config,
            Method::POST,
            &format!("/v1/native/repository-authorities/{repository_index}/snapshots:exists"),
            Vec::new(),
            Some(JsonValue::Object(body)),
        )
    }

    pub fn snapshot_manifest_from_object_reader<R: ObjectReader + ?Sized>(
        &self,
        object_reader: &R,
        snapshot_id: &str,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        object_diff::snapshot_manifest_from_object_reader_impl(object_reader, snapshot_id)
    }

    pub fn snapshot_diff_from_readers<R: SnapshotReader + ?Sized, B: BlobReader + ?Sized>(
        &self,
        snapshot_reader: &R,
        blob_reader: Option<&B>,
        old_snapshot_id: Option<&str>,
        new_snapshot_id: Option<&str>,
        include_text: bool,
        max_bytes: usize,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        object_diff::snapshot_diff_from_readers_impl(
            snapshot_reader,
            blob_reader,
            old_snapshot_id,
            new_snapshot_id,
            include_text,
            max_bytes,
        )
    }

    pub fn snapshot_diff_from_object_reader<O: ObjectReader + ?Sized, B: BlobReader + ?Sized>(
        &self,
        object_reader: &O,
        blob_reader: Option<&B>,
        old_snapshot_id: Option<&str>,
        new_snapshot_id: Option<&str>,
        include_text: bool,
        max_bytes: usize,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        object_diff::snapshot_diff_from_object_reader_impl(
            object_reader,
            blob_reader,
            old_snapshot_id,
            new_snapshot_id,
            include_text,
            max_bytes,
        )
    }

    pub fn diff_snapshot_manifests(
        &self,
        old_files: &JsonValue,
        new_files: &JsonValue,
        old_snapshot_id: Option<&str>,
        new_snapshot_id: Option<&str>,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        object_diff::diff_snapshot_manifests_impl(
            old_files,
            new_files,
            old_snapshot_id,
            new_snapshot_id,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "arguments mirror the stable snapshot diff request surface"
    )]
    pub fn snapshot_diff_from_manifests(
        &self,
        old_files: &JsonValue,
        new_files: &JsonValue,
        blob_bytes_by_id: &BTreeMap<String, Vec<u8>>,
        old_snapshot_id: Option<&str>,
        new_snapshot_id: Option<&str>,
        include_text: bool,
        max_bytes: usize,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        object_diff::snapshot_diff_from_manifests_impl(
            old_files,
            new_files,
            blob_bytes_by_id,
            old_snapshot_id,
            new_snapshot_id,
            include_text,
            max_bytes,
        )
    }
}

fn snapshot_query_pairs(include_content: bool, path: Option<&str>) -> Vec<(String, String)> {
    let mut query_pairs = Vec::new();
    if !include_content {
        query_pairs.push(("include_content".to_string(), "false".to_string()));
    }
    if let Some(path_value) = normalize_optional_str(path) {
        query_pairs.push(("path".to_string(), path_value));
    }
    query_pairs
}

fn require_plan_http_non_empty_text(value: &str, field: &str) -> PlanHttpClientResult<String> {
    normalize_optional_str(Some(value)).ok_or_else(|| {
        PlanHttpClientError::Invalid(format!("Plan HTTP {field} must not be empty."))
    })
}

fn normalize_optional_str(value: Option<&str>) -> Option<String> {
    let text = value?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

#[cfg(test)]
mod tests;
