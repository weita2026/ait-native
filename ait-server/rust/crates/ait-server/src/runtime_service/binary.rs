use super::*;
use std::sync::Arc;

#[derive(Clone)]
pub struct BinaryServerRuntimeService {
    plans: BinaryDbServerPlanService<FilesystemServerRemoteBinaryDb>,
    queue: BinaryDbServerWorkflowReadModelService<FilesystemServerRemoteBinaryDb>,
}

impl BinaryServerRuntimeService {
    pub fn new(db: FilesystemServerRemoteBinaryDb, workflow: Arc<dyn ServerWorkflowStore>) -> Self {
        Self {
            plans: BinaryDbServerPlanService::new(db.clone()),
            queue: BinaryDbServerWorkflowReadModelService::new(db, workflow),
        }
    }
}

impl ServerRuntimeService for BinaryServerRuntimeService {
    fn request_queue_read_models_refresh(&self, _repo_name: Option<&str>) {
        self.queue.request_queue_projection_refresh_after_mutation();
    }

    fn read_repository_queue_summary(
        &self,
        _repository_index: &str,
        status: Option<&str>,
    ) -> Result<JsonValue, String> {
        self.queue.read_queue_summary(None, status, false)
    }

    fn read_repository_task_queue(
        &self,
        _repository_index: &str,
        status: Option<&str>,
    ) -> Result<JsonValue, String> {
        self.queue.read_task_queue(None, status)
    }

    fn read_repository_reviewer_inbox(&self, _repository_index: &str) -> Result<JsonValue, String> {
        self.queue.read_reviewer_inbox(None)
    }

    fn list_plans(
        &self,
        repository_name: &str,
        artifact_path: Option<&str>,
    ) -> Result<JsonValue, String> {
        self.plans.list_plans(repository_name, artifact_path)
    }

    fn create_plan(&self, repository_name: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.plans.create_plan(repository_name, payload)
    }

    fn get_plan(&self, plan_id: &str) -> Result<JsonValue, String> {
        self.plans.get_plan(plan_id)
    }

    fn update_plan_status(&self, plan_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.plans.update_plan_status(plan_id, payload)
    }

    fn list_plan_revisions(&self, plan_id: &str) -> Result<JsonValue, String> {
        self.plans.list_plan_revisions(plan_id)
    }

    fn get_plan_revision(
        &self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String> {
        self.plans.get_plan_revision(plan_id, plan_revision_id)
    }

    fn resolve_task_plan_linkage(
        &self,
        repository_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        resolve_task_plan_linkage_with_runtime(self, repository_name, payload)
    }

    fn list_plan_ids_matching_contains(
        &self,
        repository_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        list_plan_ids_matching_contains_with_runtime(self, repository_name, payload)
    }

    fn revise_plan(&self, plan_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.plans.revise_plan(plan_id, payload)
    }

    fn put_plan_revision_artifacts(
        &self,
        plan_id: &str,
        plan_revision_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.plans
            .put_plan_revision_artifacts(plan_id, plan_revision_id, payload)
    }
}
