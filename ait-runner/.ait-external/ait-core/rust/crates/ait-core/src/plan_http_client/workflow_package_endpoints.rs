use super::*;
use crate::workflow_expected_identity::PackageWorkflowExpectedIdentity;
use crate::workflow_package::{EncodedWorkflowPackage, WorkflowPackageManifest};

impl PlanHttpClientManager {
    pub fn read_workflow_package_execution(
        &mut self,
        identity: &PackageWorkflowExpectedIdentity,
    ) -> PlanHttpClientResult<Option<crate::workflow_package::PackageExecution>> {
        identity.validate().map_err(PlanHttpClientError::Invalid)?;
        if self.config.repository_index.map(|index| index.get()) != Some(identity.repository_index)
        {
            return Err(PlanHttpClientError::Invalid(
                "Package execution Repository differs from HTTP authority".into(),
            ));
        }
        let key = crate::server_operational::WorkerJobKey::new(
            crate::server_operational::RepositoryIndex::new(identity.repository_index),
            crate::server_operational::WorkerJobIndex::new(identity.worker_job_index),
        );
        let response = self.get_worker_job(key)?;
        crate::workflow_package::PackageExecution::from_job_response(&response, identity)
            .map_err(PlanHttpClientError::Remote)
    }

    pub fn lookup_workflow_package_completion(
        &mut self,
        identity: &PackageWorkflowExpectedIdentity,
        target_line: &str,
        source: &[crate::workflow_package::PackageMappingExpectation],
        repo_name: &str,
    ) -> crate::workflow_package::PackageCompletion {
        self.observe_workflow_package_completion(identity, target_line, source, repo_name)
            .0
    }

    /// A native Job binding permits one explicit guarded finish replay. It
    /// does not prove full source membership or completed cleanup.
    pub fn observe_workflow_package_completion(
        &mut self,
        identity: &PackageWorkflowExpectedIdentity,
        target_line: &str,
        source: &[crate::workflow_package::PackageMappingExpectation],
        repo_name: &str,
    ) -> (crate::workflow_package::PackageCompletion, bool) {
        use crate::workflow_package::{
            lookup_package_completion, PackageCompletion, PackageReadResource,
        };
        if self.config.repository_index.map(|index| index.get()) != Some(identity.repository_index)
        {
            return (PackageCompletion::unknown(identity), false);
        }
        let mut committed = false;
        let completion =
            lookup_package_completion(identity, target_line, source, |resource| match resource {
                PackageReadResource::Job(index) => {
                    let response =
                        self.get_worker_job(crate::server_operational::WorkerJobKey::new(
                            crate::server_operational::RepositoryIndex::new(
                                identity.repository_index,
                            ),
                            crate::server_operational::WorkerJobIndex::new(index),
                        ))?;
                    if let Ok(data) = crate::workflow_package::package_response_data(&response) {
                        let job = &data["job"];
                        committed = job["repository_index"].as_u64()
                            == Some(u64::from(identity.repository_index))
                            && job["worker_job_index"].as_u64()
                                == Some(u64::from(identity.worker_job_index))
                            && job["tombstoned"].as_bool() == Some(false)
                            && job["patchset_index"]
                                .as_u64()
                                .is_some_and(|index| u32::try_from(index).is_ok());
                    }
                    Ok(response)
                }
                PackageReadResource::Snapshot(id) => {
                    let spec = crate::snapshot_json::SnapshotJson::stateless()
                        .build_get_remote_snapshot_request_spec(
                            &self.config,
                            repo_name,
                            &id,
                            false,
                            None,
                        )?;
                    parse_object_payload(self.execute_json(spec)?)
                }
                PackageReadResource::Task(id) => self.get_task(&id, Some(repo_name)),
                PackageReadResource::Land(id) => self.get_land(&id, Some(repo_name)),
                PackageReadResource::Patchset(id) => self.get_patchset(&id, Some(repo_name), None),
            });
        (completion, committed)
    }
    /// A lost POST response cannot become a fresh ready or another finish.
    /// A later explicit invocation may replay the same finish after native
    /// Job binding proof; transport retries remain disabled.
    fn execute_package_mutation(
        &mut self,
        spec: PlanHttpRequestSpec,
    ) -> PlanHttpClientResult<Value> {
        let retry_attempts = self.config.retry_attempts;
        self.config.retry_attempts = 0;
        let result = self.execute_json(spec).and_then(parse_object_payload);
        self.config.retry_attempts = retry_attempts;
        result
    }

    pub fn prepare_workflow_package(
        &mut self,
        manifest: &WorkflowPackageManifest,
        package: &EncodedWorkflowPackage,
        recovery: Option<&PackageWorkflowExpectedIdentity>,
    ) -> PlanHttpClientResult<Value> {
        let actor = self.ensure_request_actor()?;
        let spec = build_prepare_workflow_package_request_spec(
            &self.config,
            manifest,
            package,
            actor,
            recovery,
        )?;
        self.execute_package_bytes_mutation(spec)
    }

    pub fn reprepare_workflow_package(
        &mut self,
        identity: &PackageWorkflowExpectedIdentity,
    ) -> PlanHttpClientResult<Value> {
        // Validate the selector before actor registration or any mutation.
        build_reprepare_workflow_package_request_spec(&self.config, identity, 0)?;
        let actor = self.ensure_request_actor()?;
        let spec = build_reprepare_workflow_package_request_spec(&self.config, identity, actor)?;
        self.execute_package_bytes_mutation(spec)
    }

    fn execute_package_bytes_mutation(
        &mut self,
        spec: PlanHttpBytesRequestSpec,
    ) -> PlanHttpClientResult<Value> {
        let method = spec.method.clone();
        let url = spec.url.clone();
        let retry_attempts = self.config.retry_attempts;
        self.config.retry_attempts = 0;
        let response = self.execute_bytes(spec);
        self.config.retry_attempts = retry_attempts;
        parse_object_payload(parse_json_bytes_payload(&method, &url, response?)?)
    }

    pub fn finish_workflow_package(
        &mut self,
        identity: &PackageWorkflowExpectedIdentity,
        target_line: &str,
        mode: &str,
    ) -> PlanHttpClientResult<Value> {
        let actor = self.ensure_request_actor()?;
        let spec = build_finish_workflow_package_request_spec(
            &self.config,
            identity,
            target_line,
            mode,
            actor,
        )?;
        self.execute_package_mutation(spec)
    }

    pub fn abandon_workflow_package(
        &mut self,
        identity: &PackageWorkflowExpectedIdentity,
    ) -> PlanHttpClientResult<Value> {
        let actor = self.ensure_request_actor()?;
        let spec = build_abandon_workflow_package_request_spec(&self.config, identity, actor)?;
        self.execute_package_mutation(spec)
    }

    pub fn record_workflow_package_evidence(
        &mut self,
        identity: &PackageWorkflowExpectedIdentity,
        operation: &str,
        idempotency_key: &str,
        payload: Value,
    ) -> PlanHttpClientResult<Value> {
        let actor = self.ensure_request_actor()?;
        let spec = build_workflow_package_evidence_request_spec(
            &self.config,
            identity,
            operation,
            idempotency_key,
            actor,
            payload,
        )?;
        self.execute_package_mutation(spec)
    }
}
