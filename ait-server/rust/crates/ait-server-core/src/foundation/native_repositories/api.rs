use super::service::{
    optional_json_text, repository_pack_storage_capability_json, required_json_text,
};
use super::zstd_bulk::{
    json_text_array, json_value_array, pack_ids_from_array, uploaded_tree_pack_root_index,
    uploaded_zstd_pack_index, validate_pack_id_segment,
    validate_remote_sync_uploaded_zstd_pack_index_metadata, validate_root_tree_locator_index,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

const NATIVE_REPOSITORY_ERROR_PREFIX: &str = "ait-native-repository-error:";
pub const ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE: &str =
    "application/vnd.ait.remote-sync.object-pack+zstd";
pub const ZSTD_BULK_TREE_PACK_MEDIA_TYPE: &str = "application/vnd.ait.remote-sync.tree-pack+zstd";
const REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK: &str = "remote_sync.pack_bulk.zstd.v1";
const REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK_DOWNLOAD: &str =
    "remote_sync.pack_bulk.zstd.download.v1";
const REMOTE_SYNC_CAPABILITY_ZSTD_PULL_MANIFEST: &str =
    "remote_sync.pack_bulk.zstd.pull_manifest.v1";
pub(super) const REMOTE_SYNC_ZSTD_IMPORT_MANIFEST_CONTRACT_V1: &str =
    "ait.remote_sync.zstd_bulk.import_manifest.v1";
pub(super) const REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1: &str =
    "ait.remote_sync.zstd_bulk.pull_manifest.request.v1";
pub(super) const REMOTE_SYNC_ZSTD_PULL_MANIFEST_CONTRACT_V1: &str =
    "ait.remote_sync.zstd_bulk.pull_manifest.v1";
const REPOSITORY_PACK_STORAGE_CAPABILITY_FIELD: &str = "repository_pack_storage";

pub(super) fn default_main_line() -> String {
    "main".to_string()
}

fn default_archived_status() -> String {
    "archived".to_string()
}

fn default_true() -> bool {
    true
}

pub struct RemoteSyncPlanJson<S> {
    store: S,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RemoteSyncZstdBulkPlanRequest {
    pub snapshot_ids: Vec<String>,
    pub object_pack_ids: Vec<String>,
    pub tree_pack_ids: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RemoteSyncZstdPullManifestRequest {
    pub head_snapshot_id: String,
    pub have_snapshot_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct RemoteSyncZstdBulkPlanPresence {
    pub present_snapshot_ids: BTreeSet<String>,
    pub present_object_pack_ids: BTreeSet<String>,
    pub present_tree_pack_ids: BTreeSet<String>,
}

impl<S> RemoteSyncPlanJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl RemoteSyncPlanJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> RemoteSyncPlanJson<S> {
    pub fn capabilities_payload(&self) -> JsonValue {
        let _ = &self.store;
        let mut payload = json!({
            "capabilities": [
                REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK,
                REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK_DOWNLOAD,
                REMOTE_SYNC_CAPABILITY_ZSTD_PULL_MANIFEST,
            ],
            "remote_sync_capabilities": {
                "zstd_pack_bulk": true,
                "zstd_pack_bulk_download": true,
                "zstd_pull_manifest": true,
            },
        });
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                REPOSITORY_PACK_STORAGE_CAPABILITY_FIELD.to_string(),
                repository_pack_storage_capability_json(),
            );
        }
        payload
    }

    pub fn zstd_bulk_plan_request(
        &self,
        request: &JsonValue,
    ) -> Result<RemoteSyncZstdBulkPlanRequest, NativeRepositoryError> {
        let _ = &self.store;
        let request_object = request.as_object().ok_or_else(|| {
            NativeRepositoryError::bad_request("zstd bulk plan payload must be an object")
        })?;
        Ok(RemoteSyncZstdBulkPlanRequest {
            snapshot_ids: json_text_array(request_object.get("snapshot_ids"), "snapshot_ids")?,
            object_pack_ids: pack_ids_from_array(
                request_object.get("object_packs"),
                "object_packs",
            )?,
            tree_pack_ids: pack_ids_from_array(request_object.get("tree_packs"), "tree_packs")?,
        })
    }

    pub fn zstd_pull_manifest_request(
        &self,
        request: &JsonValue,
    ) -> Result<RemoteSyncZstdPullManifestRequest, NativeRepositoryError> {
        let _ = &self.store;
        let request_object = request.as_object().ok_or_else(|| {
            NativeRepositoryError::bad_request("zstd pull manifest payload must be an object")
        })?;
        let contract = required_json_text(request_object, "contract")
            .map_err(NativeRepositoryError::bad_request)?;
        if contract != REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Unsupported zstd pull manifest request contract: {contract}"
            )));
        }
        let head_snapshot_id = required_json_text(request_object, "head_snapshot_id")
            .map_err(NativeRepositoryError::bad_request)?;
        let have_snapshot_ids =
            json_text_array(request_object.get("have_snapshot_ids"), "have_snapshot_ids")?;
        let unique_have_snapshot_ids = have_snapshot_ids.iter().cloned().collect::<BTreeSet<_>>();
        if unique_have_snapshot_ids.len() != have_snapshot_ids.len() {
            return Err(NativeRepositoryError::bad_request(
                "zstd pull manifest have_snapshot_ids must not contain duplicates",
            ));
        }
        if unique_have_snapshot_ids.len() > 100_000 {
            return Err(NativeRepositoryError::bad_request(
                "zstd pull manifest have_snapshot_ids exceeds 100000 entries",
            ));
        }
        Ok(RemoteSyncZstdPullManifestRequest {
            head_snapshot_id,
            have_snapshot_ids: unique_have_snapshot_ids,
        })
    }

    pub fn zstd_bulk_plan_response(
        &self,
        repo_name: &str,
        request: &RemoteSyncZstdBulkPlanRequest,
        presence: &RemoteSyncZstdBulkPlanPresence,
    ) -> JsonValue {
        let _ = &self.store;
        json!({
            "repo_name": repo_name,
            "checked_snapshot_ids": &request.snapshot_ids,
            "present_snapshot_ids": request.snapshot_ids
                .iter()
                .filter(|snapshot_id| presence.present_snapshot_ids.contains(*snapshot_id))
                .cloned()
                .collect::<Vec<_>>(),
            "missing_snapshot_ids": request.snapshot_ids
                .iter()
                .filter(|snapshot_id| !presence.present_snapshot_ids.contains(*snapshot_id))
                .cloned()
                .collect::<Vec<_>>(),
            "present_object_pack_ids": request.object_pack_ids
                .iter()
                .filter(|pack_id| presence.present_object_pack_ids.contains(*pack_id))
                .cloned()
                .collect::<Vec<_>>(),
            "missing_object_pack_ids": request.object_pack_ids
                .iter()
                .filter(|pack_id| !presence.present_object_pack_ids.contains(*pack_id))
                .cloned()
                .collect::<Vec<_>>(),
            "present_tree_pack_ids": request.tree_pack_ids
                .iter()
                .filter(|pack_id| presence.present_tree_pack_ids.contains(*pack_id))
                .cloned()
                .collect::<Vec<_>>(),
            "missing_tree_pack_ids": request.tree_pack_ids
                .iter()
                .filter(|pack_id| !presence.present_tree_pack_ids.contains(*pack_id))
                .cloned()
                .collect::<Vec<_>>(),
        })
    }
}

pub struct RemoteSyncCommitJson<S> {
    store: S,
}

pub struct RemoteSyncZstdImportManifestJson<S> {
    store: S,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RemoteSyncZstdBulkCommitResponse {
    pub repo_name: String,
    pub repo_id: String,
    pub upserted_object_packs: i64,
    pub skipped_object_packs: i64,
    pub upserted_tree_packs: i64,
    pub skipped_tree_packs: i64,
    pub upserted_blobs: i64,
    pub upserted_trees: i64,
    pub upserted_snapshots: i64,
    pub skipped_snapshots: i64,
    pub remote_line: JsonValue,
    pub line_head_updated_after_ingest: bool,
}

impl<S> RemoteSyncCommitJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl RemoteSyncCommitJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> RemoteSyncZstdImportManifestJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl RemoteSyncZstdImportManifestJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> RemoteSyncCommitJson<S> {
    pub fn zstd_bulk_commit_object<'a>(
        &self,
        request: &'a JsonValue,
    ) -> Result<&'a JsonMap<String, JsonValue>, NativeRepositoryError> {
        let _ = &self.store;
        request.as_object().ok_or_else(|| {
            NativeRepositoryError::bad_request("zstd bulk commit payload must be an object")
        })
    }

    pub fn zstd_bulk_commit_values<'a>(
        &self,
        request_object: &'a JsonMap<String, JsonValue>,
        field: &str,
    ) -> Result<Vec<&'a JsonValue>, NativeRepositoryError> {
        let _ = &self.store;
        json_value_array(request_object.get(field), field)
    }

    pub fn line_update_request(
        &self,
        request_object: &JsonMap<String, JsonValue>,
    ) -> Result<Option<(String, LineUpdateRequest)>, NativeRepositoryError> {
        let _ = &self.store;
        match request_object.get("line_update") {
            Some(JsonValue::Object(line_update)) => {
                let line_name = required_json_text(line_update, "line_name")
                    .map_err(NativeRepositoryError::bad_request)?;
                let head_snapshot_id = optional_json_text(line_update, "head_snapshot_id");
                let expected_head_snapshot_id =
                    optional_json_text(line_update, "expected_head_snapshot_id");
                Ok(Some((
                    line_name,
                    LineUpdateRequest {
                        head_snapshot_id,
                        expected_head_snapshot_id,
                    },
                )))
            }
            Some(JsonValue::Null) | None => Ok(None),
            _ => Err(NativeRepositoryError::bad_request(
                "zstd bulk commit line_update must be an object or null",
            )),
        }
    }

    pub fn validate_zstd_pack_id_segment(
        &self,
        pack_id: &str,
    ) -> Result<(), NativeRepositoryError> {
        let _ = &self.store;
        validate_pack_id_segment(pack_id)
    }

    pub fn uploaded_zstd_pack_index(&self, pack_bytes: &[u8]) -> Option<JsonValue> {
        let _ = &self.store;
        uploaded_zstd_pack_index(pack_bytes)
    }

    pub fn uploaded_tree_pack_root_index(&self, pack_bytes: &[u8]) -> Option<JsonValue> {
        let _ = &self.store;
        uploaded_tree_pack_root_index(pack_bytes)
    }

    pub fn validate_uploaded_zstd_pack_index_metadata(
        &self,
        index: &JsonValue,
        object: &JsonMap<String, JsonValue>,
        pack_id: &str,
        tree_pack: bool,
    ) -> Result<(), NativeRepositoryError> {
        let _ = &self.store;
        validate_remote_sync_uploaded_zstd_pack_index_metadata(
            index, object, pack_id, tree_pack, None,
        )
    }

    pub fn validate_uploaded_root_tree_locator(
        &self,
        index: &JsonValue,
        pack_id: &str,
        root_entry_ordinal: i64,
    ) -> Result<(), NativeRepositoryError> {
        let _ = &self.store;
        let Ok(root_entry_ordinal) = usize::try_from(root_entry_ordinal) else {
            return Err(NativeRepositoryError::bad_request(format!(
                "Tree pack {pack_id} is missing root entry ordinal {root_entry_ordinal}"
            )));
        };
        validate_root_tree_locator_index(index, pack_id, root_entry_ordinal)
    }

    pub fn zstd_bulk_commit_response(
        &self,
        response: RemoteSyncZstdBulkCommitResponse,
    ) -> JsonValue {
        let _ = &self.store;
        json!({
            "repo_name": response.repo_name,
            "repo_id": response.repo_id,
            "upserted_object_packs": response.upserted_object_packs,
            "skipped_object_packs": response.skipped_object_packs,
            "upserted_tree_packs": response.upserted_tree_packs,
            "skipped_tree_packs": response.skipped_tree_packs,
            "upserted_blobs": response.upserted_blobs,
            "upserted_trees": response.upserted_trees,
            "upserted_snapshots": response.upserted_snapshots,
            "skipped_snapshots": response.skipped_snapshots,
            "remote_line": response.remote_line,
            "line_head_updated_after_ingest": response.line_head_updated_after_ingest,
            "raw_binary_upload": true,
        })
    }
}

impl<S> RemoteSyncZstdImportManifestJson<S> {
    pub fn zstd_import_manifest_response(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        snapshot_row: JsonValue,
        object_packs: Vec<JsonValue>,
        tree_packs: Vec<JsonValue>,
        blob_locators: Vec<JsonValue>,
        tree_locators: Vec<JsonValue>,
    ) -> JsonValue {
        let _ = &self.store;
        json!({
            "contract": REMOTE_SYNC_ZSTD_IMPORT_MANIFEST_CONTRACT_V1,
            "repo_name": repo_name,
            "snapshot_id": snapshot_id,
            "snapshots": [snapshot_row],
            "object_packs": object_packs,
            "tree_packs": tree_packs,
            "blob_locators": blob_locators,
            "tree_locators": tree_locators,
            "line_update": JsonValue::Null,
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the typed pull-manifest response keeps each deduplicated inventory explicit"
    )]
    pub fn zstd_pull_manifest_response(
        &self,
        repo_name: &str,
        head_snapshot_id: &str,
        boundary_snapshot_ids: Vec<String>,
        snapshot_rows: Vec<JsonValue>,
        object_packs: Vec<JsonValue>,
        tree_packs: Vec<JsonValue>,
        blob_locators: Vec<JsonValue>,
        tree_locators: Vec<JsonValue>,
    ) -> JsonValue {
        let _ = &self.store;
        json!({
            "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_CONTRACT_V1,
            "repo_name": repo_name,
            "head_snapshot_id": head_snapshot_id,
            "boundary_snapshot_ids": boundary_snapshot_ids,
            "snapshots": snapshot_rows,
            "object_packs": object_packs,
            "tree_packs": tree_packs,
            "blob_locators": blob_locators,
            "tree_locators": tree_locators,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRepositoryErrorKind {
    BadRequest,
    NotFound,
    Conflict,
    ServiceUnavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeRepositoryError {
    pub kind: NativeRepositoryErrorKind,
    pub message: String,
}

impl NativeRepositoryError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: NativeRepositoryErrorKind::BadRequest,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: NativeRepositoryErrorKind::NotFound,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: NativeRepositoryErrorKind::Conflict,
            message: message.into(),
        }
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: NativeRepositoryErrorKind::ServiceUnavailable,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: NativeRepositoryErrorKind::Internal,
            message: message.into(),
        }
    }

    pub fn from_wrapped_string(text: String) -> Self {
        if let Some(rest) = text.strip_prefix(NATIVE_REPOSITORY_ERROR_PREFIX) {
            if let Some((kind_text, message)) = rest.split_once(':') {
                let kind = match kind_text {
                    "bad_request" => NativeRepositoryErrorKind::BadRequest,
                    "not_found" => NativeRepositoryErrorKind::NotFound,
                    "conflict" => NativeRepositoryErrorKind::Conflict,
                    "service_unavailable" => NativeRepositoryErrorKind::ServiceUnavailable,
                    _ => NativeRepositoryErrorKind::Internal,
                };
                return Self {
                    kind,
                    message: message.to_string(),
                };
            }
        }
        Self::internal(text)
    }

    fn kind_tag(&self) -> &'static str {
        match self.kind {
            NativeRepositoryErrorKind::BadRequest => "bad_request",
            NativeRepositoryErrorKind::NotFound => "not_found",
            NativeRepositoryErrorKind::Conflict => "conflict",
            NativeRepositoryErrorKind::ServiceUnavailable => "service_unavailable",
            NativeRepositoryErrorKind::Internal => "internal",
        }
    }
}

impl fmt::Display for NativeRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}{}:{}",
            NATIVE_REPOSITORY_ERROR_PREFIX,
            self.kind_tag(),
            self.message
        )
    }
}

impl std::error::Error for NativeRepositoryError {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepositoryCreateRequest {
    pub repo_name: String,
    #[serde(default = "default_main_line")]
    pub default_line: String,
    #[serde(default)]
    pub policy: JsonValue,
    #[serde(default)]
    pub id_namespace_prefix: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LineUpdateRequest {
    #[serde(default)]
    pub head_snapshot_id: Option<String>,
    #[serde(default)]
    pub expected_head_snapshot_id: Option<String>,
}

pub fn require_remote_sync_line_update_authority(
    default_line: &str,
    line_name: &str,
    current_head_snapshot_id: Option<&str>,
    requested_head_snapshot_id: Option<&str>,
) -> Result<(), NativeRepositoryError> {
    let current_head_snapshot_id = current_head_snapshot_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let requested_head_snapshot_id = requested_head_snapshot_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if line_name != default_line
        || current_head_snapshot_id.is_none()
        || current_head_snapshot_id == requested_head_snapshot_id
    {
        return Ok(());
    }
    Err(NativeRepositoryError::conflict(format!(
        "GOVERNED_TARGET_LINE_REQUIRES_LAND: public remote sync cannot move initialized default \
         Line `{line_name}` from `{current}` to `{requested}`. Upload immutable repository \
         content without a Line update, prepare the history promotion, and use authoritative \
         remote Task Land.",
        current = current_head_snapshot_id.unwrap_or("none"),
        requested = requested_head_snapshot_id.unwrap_or("none"),
    )))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LineCloseRequest {
    #[serde(default = "default_archived_status")]
    pub status: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RetireRepositoryRequest {
    pub expected_repo_id: String,
    #[serde(default = "default_true")]
    pub require_verified_export: bool,
    #[serde(default)]
    pub actor_identity: Option<String>,
    #[serde(default)]
    pub actor_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SnapshotExistsRequest {
    #[serde(default)]
    pub snapshot_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SnapshotExportQuery {
    #[serde(default = "default_true")]
    pub include_content: bool,
    #[serde(default)]
    pub path: Option<String>,
}

impl Default for SnapshotExportQuery {
    fn default() -> Self {
        Self {
            include_content: true,
            path: None,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SnapshotManifestFileEntry {
    pub path: String,
    pub blob_id: String,
    pub size_bytes: i64,
    pub mode: String,
    pub sha256: String,
}

pub trait NativeRepositoryService: Send + Sync {
    fn create_repository(
        &self,
        request: RepositoryCreateRequest,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn create_repository_metadata(
        &self,
        _request: RepositoryCreateRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(NativeRepositoryError::service_unavailable(
            "This repository service does not support PostgreSQL metadata-only repository creation",
        ))
    }
    fn list_repositories(&self) -> Result<JsonValue, NativeRepositoryError>;
    fn get_repository(&self, repo_name: &str) -> Result<JsonValue, NativeRepositoryError>;
    fn get_repository_by_id(&self, repo_id: &str) -> Result<JsonValue, NativeRepositoryError>;
    fn list_lines(&self, repo_name: &str) -> Result<JsonValue, NativeRepositoryError>;
    fn get_line(
        &self,
        repo_name: &str,
        line_name: &str,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn update_line(
        &self,
        repo_name: &str,
        line_name: &str,
        request: LineUpdateRequest,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn close_line(
        &self,
        repo_name: &str,
        line_name: &str,
        request: LineCloseRequest,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn retire_repository(
        &self,
        repo_name: &str,
        request: RetireRepositoryRequest,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn snapshot_existence(
        &self,
        repo_name: &str,
        request: SnapshotExistsRequest,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn zstd_bulk_plan(
        &self,
        repo_name: &str,
        request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn put_zstd_bulk_object_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn get_zstd_bulk_object_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError>;
    fn put_zstd_bulk_tree_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn get_zstd_bulk_tree_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError>;
    fn get_zstd_import_manifest(
        &self,
        repo_name: &str,
        snapshot_id: &str,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn get_zstd_pull_manifest(
        &self,
        _repo_name: &str,
        _request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(NativeRepositoryError::service_unavailable(
            "This repository service does not support bulk zstd pull manifests",
        ))
    }
    fn commit_zstd_bulk(
        &self,
        repo_name: &str,
        request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn export_snapshot(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        query: SnapshotExportQuery,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn materialize_snapshot(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn materialize_snapshot_paths(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
        relative_paths: &[PathBuf],
    ) -> Result<JsonValue, NativeRepositoryError>;
    fn materialize_snapshot_manifest_entries(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
        entries: &[SnapshotManifestFileEntry],
    ) -> Result<JsonValue, NativeRepositoryError>;
}
