use crate::operational_binary_runtime::{OperationalBinaryRuntime, OperationalDb};
use ait_server_core::foundation::native_repositories::{
    BinaryDbNativeRepositoryService, LineCloseRequest, LineUpdateRequest, NativeRepositoryError,
    NativeRepositoryService, NativeZstdPackKind, NativeZstdPackUpload, RepositoryCreateRequest,
    RetireRepositoryRequest, SnapshotExistsRequest, SnapshotExportQuery, SnapshotManifestFileEntry,
};
use ait_server_core::foundation::remote_binary_db::{BinaryDbError, BinaryDbErrorKind};
use ait_server_core::foundation::server_operational_repository_registry::OperationalRepositoryEntry;
use serde_json::{Number as JsonNumber, Value as JsonValue};
use std::path::{Path, PathBuf};
use std::sync::Arc;

type RepositoryService = BinaryDbNativeRepositoryService<OperationalDb>;

#[derive(Clone)]
pub(crate) struct RoutedBinaryNativeRepositoryService {
    runtime: Arc<OperationalBinaryRuntime>,
}

impl RoutedBinaryNativeRepositoryService {
    pub(crate) fn new(runtime: Arc<OperationalBinaryRuntime>) -> Self {
        Self { runtime }
    }

    pub(crate) fn authority_repositories(
        &self,
    ) -> Result<Vec<(String, String)>, NativeRepositoryError> {
        self.runtime
            .serving_repository_indexes()
            .into_iter()
            .map(|index| {
                let (entry, _) = self
                    .runtime
                    .repository_db(index)
                    .map_err(map_binary_error)?;
                Ok((index.to_string(), entry.repo_name))
            })
            .collect()
    }

    pub(crate) fn resolve_workflow_runtime(
        &self,
        repository_index: &str,
    ) -> Result<(OperationalRepositoryEntry, String, OperationalDb), NativeRepositoryError> {
        let index = parse_repository_index(repository_index)?;
        let (entry, db) = self
            .runtime
            .repository_db(index)
            .map_err(map_binary_error)?;
        let namespace = namespace_text(entry.record.namespace_ascii);
        Ok((entry, namespace, db))
    }

    fn service(
        &self,
        repository_index: &str,
    ) -> Result<(u32, OperationalRepositoryEntry, RepositoryService), NativeRepositoryError> {
        let index = parse_repository_index(repository_index)?;
        let (entry, service) = self
            .runtime
            .repository_native_service(index)
            .map_err(map_binary_error)?;
        Ok((index, entry, service))
    }

    fn project(
        index: u32,
        mut value: JsonValue,
        include_index: bool,
    ) -> Result<JsonValue, NativeRepositoryError> {
        project_repository_identity(&mut value, index);
        if include_index {
            let object = value.as_object_mut().ok_or_else(|| {
                NativeRepositoryError::internal("Binary Repository payload must be an object")
            })?;
            object.insert(
                "repository_index".to_string(),
                JsonValue::Number(JsonNumber::from(index)),
            );
        }
        Ok(value)
    }
}

impl NativeRepositoryService for RoutedBinaryNativeRepositoryService {
    fn create_repository(
        &self,
        _request: RepositoryCreateRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(NativeRepositoryError::conflict(
            "Repository registration is installation-scoped and cannot be routed by name",
        ))
    }

    fn list_repositories(&self) -> Result<JsonValue, NativeRepositoryError> {
        let mut repositories = Vec::new();
        for (index_text, _) in self.authority_repositories()? {
            repositories.push(self.get_repository(&index_text)?);
        }
        Ok(JsonValue::Array(repositories))
    }

    fn get_repository(&self, repository_index: &str) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        let value = service.get_repository(&entry.repo_name)?;
        Self::project(index, value, true)
    }

    fn get_repository_by_id(
        &self,
        repository_index: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.get_repository(repository_index)
    }

    fn list_lines(&self, repository_index: &str) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        let value = service.list_lines(&entry.repo_name)?;
        Self::project(index, value, false)
    }

    fn get_line(
        &self,
        repository_index: &str,
        line_name: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(index, service.get_line(&entry.repo_name, line_name)?, true)
    }

    fn update_line(
        &self,
        repository_index: &str,
        line_name: &str,
        request: LineUpdateRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.update_line(&entry.repo_name, line_name, request)?,
            true,
        )
    }

    fn close_line(
        &self,
        repository_index: &str,
        line_name: &str,
        request: LineCloseRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.close_line(&entry.repo_name, line_name, request)?,
            true,
        )
    }

    fn retire_repository(
        &self,
        _repository_index: &str,
        _request: RetireRepositoryRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(NativeRepositoryError::conflict(
            "Repository retirement is excluded from Binary DB v0",
        ))
    }

    fn snapshot_existence(
        &self,
        repository_index: &str,
        request: SnapshotExistsRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.snapshot_existence(&entry.repo_name, request)?,
            true,
        )
    }

    fn zstd_bulk_plan(
        &self,
        repository_index: &str,
        request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.zstd_bulk_plan(&entry.repo_name, request)?,
            true,
        )
    }

    fn put_zstd_bulk_object_pack(
        &self,
        repository_index: &str,
        pack_id: &str,
        pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.put_zstd_bulk_object_pack(&entry.repo_name, pack_id, pack_bytes)?,
            true,
        )
    }

    fn begin_zstd_bulk_pack_upload(
        &self,
        repository_index: &str,
        pack_id: &str,
        kind: NativeZstdPackKind,
    ) -> Result<NativeZstdPackUpload, NativeRepositoryError> {
        let (_, entry, service) = self.service(repository_index)?;
        service.begin_zstd_bulk_pack_upload(&entry.repo_name, pack_id, kind)
    }

    fn finish_zstd_bulk_pack_upload(
        &self,
        repository_index: &str,
        upload: NativeZstdPackUpload,
        payload_bytes: u64,
        payload_sha256: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.finish_zstd_bulk_pack_upload(
                &entry.repo_name,
                upload,
                payload_bytes,
                payload_sha256,
            )?,
            true,
        )
    }

    fn get_zstd_bulk_object_pack(
        &self,
        repository_index: &str,
        pack_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError> {
        let (_, entry, service) = self.service(repository_index)?;
        service.get_zstd_bulk_object_pack(&entry.repo_name, pack_id)
    }

    fn put_zstd_bulk_tree_pack(
        &self,
        repository_index: &str,
        pack_id: &str,
        pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.put_zstd_bulk_tree_pack(&entry.repo_name, pack_id, pack_bytes)?,
            true,
        )
    }

    fn get_zstd_bulk_tree_pack(
        &self,
        repository_index: &str,
        pack_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError> {
        let (_, entry, service) = self.service(repository_index)?;
        service.get_zstd_bulk_tree_pack(&entry.repo_name, pack_id)
    }

    fn get_zstd_import_manifest(
        &self,
        repository_index: &str,
        snapshot_id: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.get_zstd_import_manifest(&entry.repo_name, snapshot_id)?,
            true,
        )
    }

    fn get_zstd_pull_manifest(
        &self,
        repository_index: &str,
        request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.get_zstd_pull_manifest(&entry.repo_name, request)?,
            true,
        )
    }

    fn commit_zstd_bulk(
        &self,
        repository_index: &str,
        request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.commit_zstd_bulk(&entry.repo_name, request)?,
            true,
        )
    }

    fn export_snapshot(
        &self,
        repository_index: &str,
        snapshot_id: &str,
        query: SnapshotExportQuery,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.export_snapshot(&entry.repo_name, snapshot_id, query)?,
            true,
        )
    }

    fn materialize_snapshot(
        &self,
        repository_index: &str,
        snapshot_id: &str,
        destination: &Path,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.materialize_snapshot(&entry.repo_name, snapshot_id, destination)?,
            true,
        )
    }

    fn materialize_snapshot_paths(
        &self,
        repository_index: &str,
        snapshot_id: &str,
        destination: &Path,
        relative_paths: &[PathBuf],
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.materialize_snapshot_paths(
                &entry.repo_name,
                snapshot_id,
                destination,
                relative_paths,
            )?,
            true,
        )
    }

    fn materialize_snapshot_manifest_entries(
        &self,
        repository_index: &str,
        snapshot_id: &str,
        destination: &Path,
        entries: &[SnapshotManifestFileEntry],
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (index, entry, service) = self.service(repository_index)?;
        Self::project(
            index,
            service.materialize_snapshot_manifest_entries(
                &entry.repo_name,
                snapshot_id,
                destination,
                entries,
            )?,
            true,
        )
    }
}

fn parse_repository_index(value: &str) -> Result<u32, NativeRepositoryError> {
    let value = value.trim();
    if value.is_empty()
        || value.bytes().any(|byte| !byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(NativeRepositoryError::bad_request(
            "repository_index must be canonical unsigned base-10 without leading zeroes",
        ));
    }
    value
        .parse::<u32>()
        .map_err(|_| NativeRepositoryError::bad_request("repository_index exceeds u32"))
}

fn namespace_text(namespace: [u8; 2]) -> String {
    namespace
        .into_iter()
        .take_while(|byte| *byte != 0)
        .map(char::from)
        .collect()
}

fn project_repository_identity(value: &mut JsonValue, repository_index: u32) {
    match value {
        JsonValue::Array(values) => {
            for value in values {
                project_repository_identity(value, repository_index);
            }
        }
        JsonValue::Object(object) => {
            let had_legacy_id = object.remove("repo_id").is_some();
            for value in object.values_mut() {
                project_repository_identity(value, repository_index);
            }
            if had_legacy_id {
                object.insert(
                    "repository_index".to_string(),
                    JsonValue::Number(JsonNumber::from(repository_index)),
                );
            }
        }
        _ => {}
    }
}

fn map_binary_error(error: BinaryDbError) -> NativeRepositoryError {
    match error.kind() {
        BinaryDbErrorKind::MissingData => NativeRepositoryError::not_found(error.to_string()),
        BinaryDbErrorKind::InvalidDomainData => {
            NativeRepositoryError::bad_request(error.to_string())
        }
        BinaryDbErrorKind::RetryableBusy => {
            NativeRepositoryError::service_unavailable(error.to_string())
        }
        BinaryDbErrorKind::Corruption
        | BinaryDbErrorKind::LayoutMismatch
        | BinaryDbErrorKind::Io
        | BinaryDbErrorKind::Unsupported
        | BinaryDbErrorKind::Other => NativeRepositoryError::internal(error.to_string()),
    }
}
