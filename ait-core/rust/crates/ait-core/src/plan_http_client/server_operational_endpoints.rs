use super::*;
use crate::server_operational::{
    RepositoryIndex, ServerOperationalCapabilities, WorkerJobKey, WorkerLeaseProof,
};
use crate::server_repo_retire::RemoteExportManifest;

impl PlanHttpClientManager {
    pub fn get_server_operational_capabilities(
        &mut self,
    ) -> PlanHttpClientResult<ServerOperationalCapabilities> {
        let spec = build_get_server_operational_capabilities_request_spec(&self.config)?;
        let payload = parse_object_payload(self.execute_json(spec)?)?;
        Ok(ServerOperationalCapabilities::from_server_payload(Some(
            &payload,
        )))
    }

    pub fn get_repository_by_index(
        &mut self,
        repository_index: RepositoryIndex,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_repository_by_index_request_spec(&self.config, repository_index)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_repository_authority_by_index(
        &mut self,
        repository_index: RepositoryIndex,
    ) -> PlanHttpClientResult<Value> {
        let mut repository = self.get_repository_by_index(repository_index)?;
        let handshake = self.get_server_handshake()?;
        let repository_object = repository.as_object_mut().ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "Repository authority response must decode to an object.".to_string(),
            )
        })?;
        for field in ["ci_capabilities", "operational_capabilities"] {
            if let Some(value) = handshake.get(field) {
                repository_object.insert(field.to_string(), value.clone());
            }
        }
        Ok(repository)
    }

    pub fn begin_repository_retirement(
        &mut self,
        repository_index: RepositoryIndex,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_begin_repository_retirement_request_spec(&self.config, repository_index)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn abort_repository_retirement(
        &mut self,
        repository_index: RepositoryIndex,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_abort_repository_retirement_request_spec(&self.config, repository_index)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_repository_retirement(
        &mut self,
        repository_index: RepositoryIndex,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_repository_retirement_request_spec(&self.config, repository_index)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_repository_retirement_file(
        &mut self,
        repository_index: RepositoryIndex,
        file_path: &str,
    ) -> PlanHttpClientResult<Vec<u8>> {
        let spec = build_get_repository_retirement_file_request_spec(
            &self.config,
            repository_index,
            file_path,
        )?;
        self.execute_bytes(spec)
    }

    pub fn purge_repository_retirement(
        &mut self,
        repository_index: RepositoryIndex,
        manifest: &RemoteExportManifest,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_purge_repository_retirement_request_spec(
            &self.config,
            repository_index,
            manifest,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn begin_repository_restore(
        &mut self,
        manifest: &RemoteExportManifest,
        policy_flags: u8,
    ) -> PlanHttpClientResult<Value> {
        let spec =
            build_begin_repository_restore_request_spec(&self.config, manifest, policy_flags)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn upload_repository_restore_file(
        &mut self,
        restore_token: &str,
        file_path: &str,
        bytes: Vec<u8>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_upload_repository_restore_file_request_spec(
            &self.config,
            restore_token,
            file_path,
            bytes,
        )?;
        let method = spec.method.clone();
        let url = spec.url.clone();
        let response_bytes = self.execute_bytes(spec)?;
        parse_object_payload(parse_json_bytes_payload(&method, &url, response_bytes)?)
    }

    pub fn commit_repository_restore(
        &mut self,
        restore_token: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_commit_repository_restore_request_spec(&self.config, restore_token)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn list_worker_jobs(
        &mut self,
        repository_index: RepositoryIndex,
        state_kind: Option<u8>,
        limit: u32,
    ) -> PlanHttpClientResult<Value> {
        let spec =
            build_list_worker_jobs_request_spec(&self.config, repository_index, state_kind, limit)?;
        parse_any_payload(self.execute_json(spec)?)
    }

    pub fn get_worker_job(&mut self, key: WorkerJobKey) -> PlanHttpClientResult<Value> {
        let spec = build_get_worker_job_request_spec(&self.config, key)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn claim_worker_job(&mut self, key: WorkerJobKey) -> PlanHttpClientResult<Value> {
        let spec = build_claim_worker_job_request_spec(&self.config, key)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn heartbeat_worker_job(
        &mut self,
        proof: &WorkerLeaseProof,
    ) -> PlanHttpClientResult<Value> {
        let spec =
            build_worker_job_lease_operation_request_spec(&self.config, "heartbeat", proof, None)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn complete_worker_job(
        &mut self,
        proof: &WorkerLeaseProof,
        detail: Value,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_worker_job_lease_operation_request_spec(
            &self.config,
            "complete",
            proof,
            Some(detail),
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn fail_worker_job(
        &mut self,
        proof: &WorkerLeaseProof,
        detail: Value,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_worker_job_lease_operation_request_spec(
            &self.config,
            "fail",
            proof,
            Some(detail),
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }
}
