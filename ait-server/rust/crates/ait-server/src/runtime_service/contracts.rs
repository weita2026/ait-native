use super::*;

pub trait ServerRuntimeService: Send + Sync {
    fn complete_post_land_delivery(&self, _land: &mut JsonValue) {}

    fn run_repo_ci(
        &self,
        repository_index: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Repository {repository_index} CI dispatch requires the numeric Binary runtime"
        ))
    }

    fn run_patchset_ci(
        &self,
        patchset_id: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Patchset {patchset_id} CI dispatch is unavailable in this runtime"
        ))
    }

    fn run_repository_patchset_ci(
        &self,
        _repo_name: &str,
        patchset_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.run_patchset_ci(patchset_id, payload)
    }

    fn run_repository_authority_patchset_ci(
        &self,
        _repository_index: &str,
        patchset_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.run_patchset_ci(patchset_id, payload)
    }

    #[cfg(test)]
    fn run_patchset_ci_from_workflow_rows(
        &self,
        _patchset: &JsonValue,
        _change: &JsonValue,
        _payload: &JsonValue,
    ) -> Option<Result<JsonValue, String>> {
        None
    }

    /// Invalidates the derived workflow queue projection after a durable
    /// workflow mutation. Worker Job indexes remain transactionally updated
    /// authority and do not use this hook.
    fn request_queue_read_models_refresh(&self, _repo_name: Option<&str>) {}

    fn read_repository_queue_summary(
        &self,
        repository_index: &str,
        _status: Option<&str>,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Repository authority {repository_index} does not support queue-summary reads"
        ))
    }

    fn read_repository_task_queue(
        &self,
        repository_index: &str,
        _status: Option<&str>,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Repository authority {repository_index} does not support task-queue reads"
        ))
    }

    fn read_repository_reviewer_inbox(&self, repository_index: &str) -> Result<JsonValue, String> {
        Err(format!(
            "Repository authority {repository_index} does not support reviewer-inbox reads"
        ))
    }

    fn read_patchset_ci_status(
        &self,
        patchset_id: &str,
        _recent_limit: i64,
        _projection: Option<&str>,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Patchset {patchset_id} CI status requires an explicit numeric repository_index"
        ))
    }

    fn read_repository_patchset_ci_status(
        &self,
        _repo_name: &str,
        patchset_id: &str,
        recent_limit: i64,
        projection: Option<&str>,
    ) -> Result<JsonValue, String> {
        self.read_patchset_ci_status(patchset_id, recent_limit, projection)
    }

    fn read_repository_authority_patchset_ci_status(
        &self,
        _repository_index: &str,
        patchset_id: &str,
        recent_limit: i64,
        projection: Option<&str>,
    ) -> Result<JsonValue, String> {
        self.read_patchset_ci_status(patchset_id, recent_limit, projection)
    }

    fn read_patchset_ci_status_from_workflow_rows(
        &self,
        _patchset: &JsonValue,
        _change: &JsonValue,
        _recent_limit: i64,
        _projection: Option<&str>,
    ) -> Option<Result<JsonValue, String>> {
        None
    }

    fn plan_repository_zstd_bulk(
        &self,
        repository_index: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Repository authority {repository_index} does not support numeric zstd planning"
        ))
    }

    fn put_repository_zstd_object_pack(
        &self,
        repository_index: &str,
        _pack_id: &str,
        _pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Repository authority {repository_index} does not support numeric object-pack upload"
        ))
    }

    fn get_repository_zstd_object_pack(
        &self,
        repository_index: &str,
        _pack_id: &str,
    ) -> Result<Vec<u8>, String> {
        Err(format!(
            "Repository authority {repository_index} does not support numeric object-pack download"
        ))
    }

    fn put_repository_zstd_tree_pack(
        &self,
        repository_index: &str,
        _pack_id: &str,
        _pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Repository authority {repository_index} does not support numeric tree-pack upload"
        ))
    }

    fn get_repository_zstd_tree_pack(
        &self,
        repository_index: &str,
        _pack_id: &str,
    ) -> Result<Vec<u8>, String> {
        Err(format!(
            "Repository authority {repository_index} does not support numeric tree-pack download"
        ))
    }

    fn commit_repository_zstd_bulk(
        &self,
        repository_index: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Repository authority {repository_index} does not support numeric zstd commit"
        ))
    }

    fn get_repository_zstd_import_manifest(
        &self,
        repository_index: &str,
        _snapshot_id: &str,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Repository authority {repository_index} does not support numeric zstd import manifests"
        ))
    }

    fn get_repository_zstd_pull_manifest(
        &self,
        repository_index: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Repository authority {repository_index} does not support numeric zstd pull manifests"
        ))
    }

    fn list_plans(&self, repo_name: &str, artifact_path: Option<&str>)
        -> Result<JsonValue, String>;

    fn create_plan(&self, repo_name: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn list_repository_plans(
        &self,
        repository_index: &str,
        artifact_path: Option<&str>,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Repository authority {repository_index} does not support numeric Plan routing for artifact {artifact_path:?}"
        ))
    }

    fn create_repository_plan(
        &self,
        repository_index: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "Repository authority {repository_index} does not support numeric Plan routing"
        ))
    }

    fn get_plan(&self, plan_id: &str) -> Result<JsonValue, String>;

    fn get_repository_plan(&self, _repo_id: &str, plan_id: &str) -> Result<JsonValue, String> {
        self.get_plan(plan_id)
    }

    fn update_plan_status(&self, plan_id: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn update_repository_plan_status(
        &self,
        _repo_id: &str,
        plan_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.update_plan_status(plan_id, payload)
    }

    fn list_plan_revisions(&self, plan_id: &str) -> Result<JsonValue, String>;

    fn list_repository_plan_revisions(
        &self,
        _repo_id: &str,
        plan_id: &str,
    ) -> Result<JsonValue, String> {
        self.list_plan_revisions(plan_id)
    }

    fn get_plan_revision(&self, plan_id: &str, plan_revision_id: &str)
        -> Result<JsonValue, String>;

    fn get_repository_plan_revision(
        &self,
        _repo_id: &str,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String> {
        self.get_plan_revision(plan_id, plan_revision_id)
    }

    fn resolve_task_plan_linkage(
        &self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String>;

    fn list_plan_ids_matching_contains(
        &self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String>;

    fn revise_plan(&self, plan_id: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn revise_repository_plan(
        &self,
        _repo_id: &str,
        plan_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.revise_plan(plan_id, payload)
    }

    fn put_plan_revision_artifacts(
        &self,
        plan_id: &str,
        plan_revision_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String>;

    fn put_repository_plan_revision_artifacts(
        &self,
        _repo_id: &str,
        plan_id: &str,
        plan_revision_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.put_plan_revision_artifacts(plan_id, plan_revision_id, payload)
    }
}
