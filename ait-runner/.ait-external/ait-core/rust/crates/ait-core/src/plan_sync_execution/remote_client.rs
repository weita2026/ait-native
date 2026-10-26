use super::remote_ports::{
    PlanSyncRemoteInventorySource, PlanSyncRemotePackedArtifactUploader, PlanSyncRemotePlanCreator,
    PlanSyncRemotePlanReader, PlanSyncRemotePlanReviser, PlanSyncRemoteRevisionArtifactWriter,
    PlanSyncRemoteRevisionLister, PlanSyncRemoteRevisionReader, PlanSyncRemoteStatusUpdater,
    PlanSyncRemoteTaskStarter,
};
use crate::json_support::JsonValue;
use crate::plan_http_client::{PlanHttpClientError, PlanHttpClientManager, PlanHttpClientResult};
use crate::repository_pack_json::{
    ZstdBulkCommitRequest, ZstdBulkCommitResponse, ZstdPackUploadResponse,
};

impl PlanSyncRemoteInventorySource for PlanHttpClientManager {
    fn list_plan_summaries(
        &mut self,
        repo_name: &str,
        artifact_path: Option<&str>,
    ) -> Result<Vec<JsonValue>, String> {
        self.list_plans(repo_name, artifact_path)
            .map_err(|err| err.to_string())
    }
}

impl PlanSyncRemotePlanReader for PlanHttpClientManager {
    fn get_plan(&mut self, plan_id: &str) -> Result<JsonValue, String> {
        PlanHttpClientManager::get_plan(self, plan_id).map_err(|err| err.to_string())
    }
}

impl PlanSyncRemoteRevisionLister for PlanHttpClientManager {
    fn list_plan_revisions(&mut self, plan_id: &str) -> Result<Vec<JsonValue>, String> {
        PlanHttpClientManager::list_plan_revisions(self, plan_id).map_err(|err| err.to_string())
    }
}

impl PlanSyncRemoteRevisionReader for PlanHttpClientManager {
    fn get_plan_revision(
        &mut self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String> {
        PlanHttpClientManager::get_plan_revision(self, plan_id, plan_revision_id)
            .map_err(|err| err.to_string())
    }
}

impl PlanSyncRemotePlanCreator for PlanHttpClientManager {
    fn create_plan(
        &mut self,
        repo_name: &str,
        title: &str,
        artifact_path: &str,
        artifact_selector: Option<&str>,
        artifact_heading: &str,
        items: &[JsonValue],
        summary: Option<&str>,
        status: &str,
        plan_id: Option<&str>,
        source_kind: &str,
        artifact_body: Option<&str>,
        packed_artifact: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        PlanHttpClientManager::create_plan_with_packed_artifact(
            self,
            repo_name,
            title,
            artifact_path,
            artifact_selector,
            artifact_heading,
            items,
            summary,
            status,
            plan_id,
            source_kind,
            artifact_body,
            packed_artifact,
        )
        .map_err(|err| err.to_string())
    }
}

impl PlanSyncRemotePlanReviser for PlanHttpClientManager {
    fn revise_plan(
        &mut self,
        plan_id: &str,
        artifact_path: &str,
        artifact_selector: Option<&str>,
        artifact_heading: &str,
        items: &[JsonValue],
        title: Option<&str>,
        summary: Option<&str>,
        source_kind: &str,
        artifact_body: Option<&str>,
        expected_head_revision_id: Option<&str>,
        packed_artifact: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        PlanHttpClientManager::revise_plan_with_packed_artifact(
            self,
            plan_id,
            artifact_path,
            artifact_selector,
            artifact_heading,
            items,
            title,
            summary,
            source_kind,
            artifact_body,
            expected_head_revision_id,
            packed_artifact,
        )
        .map_err(|err| err.to_string())
    }
}

impl PlanSyncRemotePackedArtifactUploader for PlanHttpClientManager {
    fn get_remote_zstd_object_pack_if_present(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> Result<Option<Vec<u8>>, String> {
        optional_repository_pack_download(PlanHttpClientManager::get_remote_zstd_object_pack(
            self, repo_name, pack_id,
        ))
    }

    fn get_remote_zstd_tree_pack_if_present(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> Result<Option<Vec<u8>>, String> {
        optional_repository_pack_download(PlanHttpClientManager::get_remote_zstd_tree_pack(
            self, repo_name, pack_id,
        ))
    }

    fn put_remote_zstd_object_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> Result<ZstdPackUploadResponse, String> {
        PlanHttpClientManager::put_remote_zstd_object_pack(self, repo_name, pack_id, pack_bytes)
            .map_err(|err| err.to_string())
    }

    fn put_remote_zstd_tree_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> Result<ZstdPackUploadResponse, String> {
        PlanHttpClientManager::put_remote_zstd_tree_pack(self, repo_name, pack_id, pack_bytes)
            .map_err(|err| err.to_string())
    }

    fn commit_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkCommitRequest,
    ) -> Result<ZstdBulkCommitResponse, String> {
        PlanHttpClientManager::commit_remote_zstd_bulk(self, repo_name, request)
            .map_err(|err| err.to_string())
    }
}

fn optional_repository_pack_download(
    result: PlanHttpClientResult<Vec<u8>>,
) -> Result<Option<Vec<u8>>, String> {
    match result {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if repository_pack_is_absent_for_probe(&error) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn repository_pack_is_absent_for_probe(error: &PlanHttpClientError) -> bool {
    error.remote_status() == Some(404)
        || (error.remote_status() == Some(400)
            && error
                .remote_detail()
                .is_some_and(repository_pack_is_typed_not_found))
        || (error.remote_status() == Some(409)
            && error
                .remote_detail()
                .is_some_and(|detail| detail.contains("belongs to repository")))
}

fn repository_pack_is_typed_not_found(detail: &str) -> bool {
    let Some(message) = detail.strip_prefix("ait-native-repository-error:not_found:") else {
        return false;
    };
    message.starts_with("Unknown zstd object pack ")
        || message.starts_with("Unknown zstd tree pack ")
}

impl PlanSyncRemoteStatusUpdater for PlanHttpClientManager {
    fn update_plan_status(&mut self, plan_id: &str, status: &str) -> Result<JsonValue, String> {
        PlanHttpClientManager::update_plan_status(self, plan_id, status)
            .map_err(|err| err.to_string())
    }
}

impl PlanSyncRemoteTaskStarter for PlanHttpClientManager {
    fn start_plan_bound_task(
        &mut self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        PlanHttpClientManager::start_plan_bound_task(self, repo_name, payload)
            .map_err(|err| err.to_string())
    }
}

impl PlanSyncRemoteRevisionArtifactWriter for PlanHttpClientManager {
    fn put_plan_revision_artifacts(
        &mut self,
        plan_id: &str,
        plan_revision_id: &str,
        artifacts: &[JsonValue],
    ) -> Result<JsonValue, String> {
        PlanHttpClientManager::put_plan_revision_artifacts(
            self,
            plan_id,
            plan_revision_id,
            artifacts,
        )
        .map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_error(status: u16, detail: &str) -> PlanHttpClientError {
        PlanHttpClientError::RemoteResponse {
            method: "GET".to_string(),
            url: "http://server.test/packs/PCK-1".to_string(),
            status,
            detail: detail.to_string(),
        }
    }

    #[test]
    fn remote_pack_probe_treats_only_missing_or_repository_scope_conflict_as_absent() {
        assert_eq!(
            optional_repository_pack_download(Ok(vec![1, 2, 3])).unwrap(),
            Some(vec![1, 2, 3])
        );
        assert_eq!(
            optional_repository_pack_download(Err(response_error(404, "unknown pack"))).unwrap(),
            None
        );
        for detail in [
            "ait-native-repository-error:not_found:Unknown zstd object pack PCK-1 for repository repo",
            "ait-native-repository-error:not_found:Unknown zstd tree pack TPK-1 for repository repo",
        ] {
            assert_eq!(
                optional_repository_pack_download(Err(response_error(400, detail))).unwrap(),
                None
            );
        }
        assert_eq!(
            optional_repository_pack_download(Err(response_error(
                409,
                "Object pack PCK-1 belongs to repository other, not repo"
            )))
            .unwrap(),
            None
        );
        for error in [
            response_error(400, "identity PCK-NOTFOUND is not hex"),
            response_error(
                400,
                "ait-native-repository-error:not_found:Unknown blob BLB-1 for repository repo",
            ),
            response_error(409, "Object pack PCK-1 has conflicting content"),
            response_error(500, "storage unavailable"),
        ] {
            assert!(optional_repository_pack_download(Err(error)).is_err());
        }
    }
}
