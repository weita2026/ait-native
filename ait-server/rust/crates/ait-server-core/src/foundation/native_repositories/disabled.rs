use super::api::{
    LineCloseRequest, LineUpdateRequest, NativeRepositoryError, NativeRepositoryService,
    RepositoryCreateRequest, RetireRepositoryRequest, SnapshotExistsRequest, SnapshotExportQuery,
    SnapshotManifestFileEntry,
};
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DisabledNativeRepositoryService {
    message: String,
}

impl DisabledNativeRepositoryService {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn disabled(&self) -> NativeRepositoryError {
        NativeRepositoryError::internal(self.message.clone())
    }
}

impl NativeRepositoryService for DisabledNativeRepositoryService {
    fn create_repository(
        &self,
        _request: RepositoryCreateRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn list_repositories(&self) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn get_repository(&self, _repo_name: &str) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn get_repository_by_id(&self, _repo_id: &str) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn list_lines(&self, _repo_name: &str) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn get_line(
        &self,
        _repo_name: &str,
        _line_name: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn update_line(
        &self,
        _repo_name: &str,
        _line_name: &str,
        _request: LineUpdateRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn close_line(
        &self,
        _repo_name: &str,
        _line_name: &str,
        _request: LineCloseRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn retire_repository(
        &self,
        _repo_name: &str,
        _request: RetireRepositoryRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn snapshot_existence(
        &self,
        _repo_name: &str,
        _request: SnapshotExistsRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn zstd_bulk_plan(
        &self,
        _repo_name: &str,
        _request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn put_zstd_bulk_object_pack(
        &self,
        _repo_name: &str,
        _pack_id: &str,
        _pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn get_zstd_bulk_object_pack(
        &self,
        _repo_name: &str,
        _pack_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn put_zstd_bulk_tree_pack(
        &self,
        _repo_name: &str,
        _pack_id: &str,
        _pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn get_zstd_bulk_tree_pack(
        &self,
        _repo_name: &str,
        _pack_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn get_zstd_import_manifest(
        &self,
        _repo_name: &str,
        _snapshot_id: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn commit_zstd_bulk(
        &self,
        _repo_name: &str,
        _request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn export_snapshot(
        &self,
        _repo_name: &str,
        _snapshot_id: &str,
        _query: SnapshotExportQuery,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn materialize_snapshot(
        &self,
        _repo_name: &str,
        _snapshot_id: &str,
        _destination: &Path,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn materialize_snapshot_paths(
        &self,
        _repo_name: &str,
        _snapshot_id: &str,
        _destination: &Path,
        _relative_paths: &[PathBuf],
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }

    fn materialize_snapshot_manifest_entries(
        &self,
        _repo_name: &str,
        _snapshot_id: &str,
        _destination: &Path,
        _entries: &[SnapshotManifestFileEntry],
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(self.disabled())
    }
}
