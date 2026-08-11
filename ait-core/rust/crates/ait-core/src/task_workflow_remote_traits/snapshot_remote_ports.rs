use crate::json_support::JsonValue as Value;
use crate::repository_pack_json::{
    ZstdBulkCommitRequest, ZstdBulkCommitResponse, ZstdBulkPlanRequest, ZstdBulkPlanResponse,
    ZstdImportManifestPayload, ZstdPackUploadResponse, ZstdPullManifestPayload,
    ZstdPullManifestRequest,
};
use std::collections::BTreeMap;

use super::http_client_types::{TaskWorkflowHttpClientError, TaskWorkflowHttpClientResult};

pub type TaskWorkflowZstdPackPayloads = BTreeMap<String, Vec<u8>>;

pub trait TaskWorkflowZstdPackUploader {
    fn plan_remote_zstd_bulk(
        &mut self,
        _repo_name: &str,
        _request: &ZstdBulkPlanRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdBulkPlanResponse> {
        Err(TaskWorkflowHttpClientError::Remote(
            "ZstdPackBulkRemoteBackend is not supported by this remote.".to_string(),
        ))
    }

    fn put_remote_zstd_object_pack(
        &mut self,
        _repo_name: &str,
        _pack_id: &str,
        _pack_bytes: &[u8],
    ) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse> {
        Err(TaskWorkflowHttpClientError::Remote(
            "ZstdPackBulkRemoteBackend is not supported by this remote.".to_string(),
        ))
    }

    fn put_remote_zstd_tree_pack(
        &mut self,
        _repo_name: &str,
        _pack_id: &str,
        _pack_bytes: &[u8],
    ) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse> {
        Err(TaskWorkflowHttpClientError::Remote(
            "ZstdPackBulkRemoteBackend is not supported by this remote.".to_string(),
        ))
    }

    fn put_remote_zstd_packs_bounded(
        &mut self,
        repo_name: &str,
        object_packs: &[(String, Vec<u8>)],
        tree_packs: &[(String, Vec<u8>)],
        _max_parallelism: usize,
    ) -> TaskWorkflowHttpClientResult<(Vec<ZstdPackUploadResponse>, Vec<ZstdPackUploadResponse>)>
    {
        let mut object_responses = Vec::with_capacity(object_packs.len());
        for (pack_id, pack_bytes) in object_packs {
            object_responses
                .push(self.put_remote_zstd_object_pack(repo_name, pack_id, pack_bytes)?);
        }
        let mut tree_responses = Vec::with_capacity(tree_packs.len());
        for (pack_id, pack_bytes) in tree_packs {
            tree_responses.push(self.put_remote_zstd_tree_pack(repo_name, pack_id, pack_bytes)?);
        }
        Ok((object_responses, tree_responses))
    }

    fn commit_remote_zstd_bulk(
        &mut self,
        _repo_name: &str,
        _request: &ZstdBulkCommitRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdBulkCommitResponse> {
        Err(TaskWorkflowHttpClientError::Remote(
            "ZstdPackBulkRemoteBackend is not supported by this remote.".to_string(),
        ))
    }
}

pub trait TaskWorkflowSnapshotMetadataReader {
    fn get_remote_snapshot(
        &mut self,
        repo_name: &str,
        snapshot_id: &str,
        include_content: bool,
        path: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowZstdPackReader {
    fn get_remote_zstd_import_manifest(
        &mut self,
        _repo_name: &str,
        _snapshot_id: &str,
    ) -> TaskWorkflowHttpClientResult<ZstdImportManifestPayload> {
        Err(TaskWorkflowHttpClientError::Remote(
            "ZstdPackBulkDownloadRemoteBackend is not supported by this remote.".to_string(),
        ))
    }

    fn get_remote_zstd_pull_manifest(
        &mut self,
        _repo_name: &str,
        _request: &ZstdPullManifestRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdPullManifestPayload> {
        Err(TaskWorkflowHttpClientError::Remote(
            "ZstdPullManifestRemoteBackend is not supported by this remote.".to_string(),
        ))
    }

    fn get_remote_zstd_object_pack(
        &mut self,
        _repo_name: &str,
        _pack_id: &str,
    ) -> TaskWorkflowHttpClientResult<Vec<u8>> {
        Err(TaskWorkflowHttpClientError::Remote(
            "ZstdPackBulkDownloadRemoteBackend is not supported by this remote.".to_string(),
        ))
    }

    fn get_remote_zstd_tree_pack(
        &mut self,
        _repo_name: &str,
        _pack_id: &str,
    ) -> TaskWorkflowHttpClientResult<Vec<u8>> {
        Err(TaskWorkflowHttpClientError::Remote(
            "ZstdPackBulkDownloadRemoteBackend is not supported by this remote.".to_string(),
        ))
    }

    fn get_remote_zstd_packs_bounded(
        &mut self,
        repo_name: &str,
        object_pack_ids: &[String],
        tree_pack_ids: &[String],
        _max_parallelism: usize,
    ) -> TaskWorkflowHttpClientResult<(TaskWorkflowZstdPackPayloads, TaskWorkflowZstdPackPayloads)>
    {
        let mut object_packs = BTreeMap::new();
        for pack_id in object_pack_ids {
            object_packs.insert(
                pack_id.clone(),
                self.get_remote_zstd_object_pack(repo_name, pack_id)?,
            );
        }
        let mut tree_packs = BTreeMap::new();
        for pack_id in tree_pack_ids {
            tree_packs.insert(
                pack_id.clone(),
                self.get_remote_zstd_tree_pack(repo_name, pack_id)?,
            );
        }
        Ok((object_packs, tree_packs))
    }
}

pub trait TaskWorkflowSnapshotExistenceReader {
    fn get_remote_snapshots_existence(
        &mut self,
        repo_name: &str,
        snapshot_ids: &[String],
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowSnapshotRemote:
    TaskWorkflowZstdPackUploader
    + TaskWorkflowSnapshotMetadataReader
    + TaskWorkflowZstdPackReader
    + TaskWorkflowSnapshotExistenceReader
{
}

impl<R> TaskWorkflowSnapshotRemote for R where
    R: TaskWorkflowZstdPackUploader
        + TaskWorkflowSnapshotMetadataReader
        + TaskWorkflowZstdPackReader
        + TaskWorkflowSnapshotExistenceReader
        + ?Sized
{
}
