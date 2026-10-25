use super::*;
use ait_server_core::foundation::prepared_ci_runtime::{
    ManagedCiPackage, ManagedCiPackagePaths, PreparedCiTarget,
};
use ait_server_core::foundation::remote_binary_db::sync_filesystem_file;
use ait_server_core::foundation::workflow_package::WorkflowPackage;
use ait_server_core::foundation::workflow_package_contract::parse_package_hash;
use ait_server_core::foundation::workflow_package_contract::{
    PackageAbandonRequest, PackageEvidenceRequest, PackageFinishRequest,
};

#[path = "workflow_packages/cleanup.rs"]
mod cleanup;
#[path = "workflow_packages/reset.rs"]
mod reset;

impl OperationalBinaryRuntime {
    pub(crate) fn abandon_workflow_package(
        &self,
        repository_index: u32,
        worker_job_index: u32,
        payload: &JsonValue,
    ) -> StoreResult<(JsonValue, bool)> {
        let request = PackageAbandonRequest::parse(payload).map_err(invalid)?;
        if request.expected_identity.repository_index != repository_index
            || request.expected_identity.worker_job_index != worker_job_index
        {
            return Err(invalid(
                "Package abandonment path differs from expected identity",
            ));
        }
        let _operation = self
            .package_operation_lock
            .lock()
            .map_err(|_| BinaryDbError::other("Package operation lock is poisoned"))?;
        let repository = self.repository(repository_index)?;
        repository
            .jobs
            .recover_prepared_ci_publication(repository.authority.db())?;
        let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(
            repository.authority.db().clone(),
            &namespace_text(repository.entry.record.namespace_ascii),
        )
        .map_err(invalid)?;
        let actor = workflow
            .validate_workflow_package_actor(request.actor_key_index)
            .map_err(invalid)?;
        let now = now_s_store()?;
        self.reconcile(now)?;
        let state = repository.jobs.abandon_prepared_ci_with_cleanup(
            &request,
            actor,
            now,
            |key| {
                Ok(!self.leases.entries()?.iter().any(|lease| {
                    lease.key.repository_index == key.repository_index
                        && lease.key.worker_job_index == key.worker_job_index
                }))
            },
            |source| self.cleanup_owned_package(source),
        )?;
        Ok((
            json!({"expected_identity":request.expected_identity,"state":state}),
            state
                == ait_server_core::foundation::prepared_ci_runtime::PackageAbandonState::Abandoned,
        ))
    }

    pub(crate) fn finish_workflow_package(
        &self,
        repository_index: u32,
        payload: &JsonValue,
    ) -> StoreResult<JsonValue> {
        let request = PackageFinishRequest::parse(payload).map_err(invalid)?;
        if request.expected_identity.repository_index != repository_index {
            return Err(invalid("Package finish Repository differs"));
        }
        let _operation = self
            .package_operation_lock
            .lock()
            .map_err(|_| BinaryDbError::other("Package operation lock is poisoned"))?;
        let repository = self.repository(repository_index)?;
        repository
            .jobs
            .recover_prepared_ci_publication(repository.authority.db())?;
        let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(
            repository.authority.db().clone(),
            &namespace_text(repository.entry.record.namespace_ascii),
        )
        .map_err(invalid)?
        .with_repository_index(repository_index);
        workflow
            .validate_workflow_package_actor(request.actor_key_index)
            .map_err(invalid)?;
        self.reconcile(now_s_store()?)?;
        let job = repository
            .jobs
            .get(request.expected_identity.worker_job_index)?;
        // Immutable acceptance wins over missing temporary state. Hash here is
        // caller input; formal state proves the exact original Job/P/Land.
        if let Some(result) = workflow
            .lookup_workflow_package_finish(job, &request, None)
            .map_err(invalid)?
        {
            let execution = repository.jobs.prepared_ci_runtime();
            let result = match execution
                .committed_result(job.key)
                .map_err(cleanup::committed_cleanup_error)?
            {
                Some(original)
                    if original["expected_identity"] == result["expected_identity"]
                        && original["patchset_id"] == result["patchset_id"]
                        && original["submission_id"] == result["submission_id"] =>
                {
                    original
                }
                Some(_) => {
                    return Err(cleanup::committed_cleanup_error(invalid(
                        "Retained native response differs from immutable publication identity",
                    )))
                }
                None => result,
            };
            self.complete_committed_package_cleanup(&repository, job)?;
            execution
                .release_completed_result(job.key)
                .map_err(cleanup::committed_cleanup_error)?;
            return Ok(result);
        }
        if job.record.patchset_index_plus1 != 0 {
            return Err(invalid(
                "Bound original Job has no provable exact package result; publication will not be repeated",
            ));
        }
        let execution = repository.jobs.prepared_ci_runtime();
        if job.record.state_kind != WORKER_JOB_STATE_SUCCEEDED
            || job.record.outcome_kind != WORKER_JOB_OUTCOME_COMPLETED
            || !execution.recoverable_selection(job.key)
            || ait_server_core::foundation::prepared_ci_runtime::prepared_ci_state(
                execution.evidence(job.key)?,
            ) != "passed"
        {
            return Err(invalid("Package finish requires its selected successful CI attempt; abandoned, superseded or unknown execution cannot publish"));
        }
        let source = repository.jobs.prepared_ci_runtime().source(job.key)?;
        if source.expected_identity(job.key).map_err(invalid)? != request.expected_identity
            || source.target().target_line != request.target_line
        {
            return Err(invalid("Package finish differs from the selected source"));
        }
        source.verify().map_err(|error| {
            invalid(format!(
                "Original package source is unavailable for finish: {error}"
            ))
        })?;
        let raw = fs::read(source.package())
            .map_err(|error| BinaryDbError::io("read sealed finish package", error))?;
        // Re-extract from sealed bytes, never publish a test-mutated working
        // tree or an independently modified unpacked pack.
        let inputs = source.worktree().join("publication-inputs");
        if inputs
            .try_exists()
            .map_err(|error| BinaryDbError::io("inspect derived publication inputs", error))?
        {
            fs::remove_dir_all(&inputs).map_err(|error| {
                BinaryDbError::io("discard prior derived publication inputs", error)
            })?;
        }
        let package = WorkflowPackage::unpack(
            &raw,
            &inputs,
            repository_index,
            &request.expected_identity.package_hash,
            usize::MAX,
        )
        .map_err(invalid)?;
        let history = workflow
            .prepare_workflow_package_history(&package.manifest, &request.expected_identity)
            .map_err(invalid)?;
        let content = repository
            .native
            .prepare_workflow_package_content(&repository.entry.repo_name, &package)
            .map_err(|error| invalid(error.to_string()))?;
        repository
            .native
            .stage_workflow_package_packs(&repository.entry.repo_name, &package)
            .map_err(|error| invalid(error.to_string()))?;
        let committed = repository.jobs.finish_prepared_ci(
            repository.authority.db(),
            job.key.worker_job_index,
            request.expected_identity.hash_bytes().map_err(invalid)?,
            |tx, context| {
                repository
                    .native
                    .apply_workflow_package_content_in_write(
                        tx,
                        &repository.entry.repo_name,
                        content,
                    )
                    .map_err(|error| invalid(error.to_string()))?;
                workflow
                    .publish_workflow_package_in_write(
                        tx,
                        history,
                        &request.expected_identity,
                        request.actor_key_index,
                        context.evidence,
                        &context.actions,
                        &request.mode,
                    )
                    .map_err(invalid)
            },
        )?;
        if let Some(warning) = committed.queue_lock_cleanup_warning {
            eprintln!("warning: committed package finish lock cleanup: {warning}");
        }
        execution
            .retain_committed_result(job.key, committed.result.clone())
            .map_err(cleanup::committed_cleanup_error)?;
        self.complete_committed_package_cleanup(&repository, job)?;
        execution
            .release_completed_result(job.key)
            .map_err(cleanup::committed_cleanup_error)?;
        Ok(committed.result)
    }

    pub(crate) fn accept_workflow_package_evidence(
        &self,
        repository_index: u32,
        worker_job_index: u32,
        payload: &JsonValue,
    ) -> StoreResult<JsonValue> {
        let request = PackageEvidenceRequest::parse(payload).map_err(invalid)?;
        if request.expected_identity.repository_index != repository_index
            || request.expected_identity.worker_job_index != worker_job_index
        {
            return Err(invalid(
                "Package evidence path differs from its expected identity",
            ));
        }
        let repository = self.repository(repository_index)?;
        let job = repository.jobs.get(worker_job_index)?;
        if job.record.state_kind != WORKER_JOB_STATE_SUCCEEDED
            || job.record.outcome_kind != WORKER_JOB_OUTCOME_COMPLETED
            || job.record.patchset_index_plus1 != 0
        {
            return Err(invalid(
                "Package evidence requires the successful original prefinish Job",
            ));
        }
        let runtime = repository.jobs.prepared_ci_runtime();
        let source = runtime.source(job.key)?;
        source.verify()?;
        let raw = fs::read(source.package())
            .map_err(|error| BinaryDbError::io("read sealed package evidence identity", error))?;
        let manifest = WorkflowPackage::read_sealed_manifest(
            &raw,
            source.sha256(),
            repository_index,
            usize::MAX,
        )
        .map_err(invalid)?;
        let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(
            repository.authority.db().clone(),
            &namespace_text(repository.entry.record.namespace_ascii),
        )
        .map_err(invalid)?;
        let actor = workflow
            .validate_package_evidence(&request, &manifest.author_mode)
            .map_err(invalid)?;
        let event = runtime.accept_evidence_with_capacity(
            job.key,
            request,
            actor,
            now_s_store()?,
            || {
                workflow
                    .package_available_attestations(&manifest)
                    .map_err(invalid)
            },
        )?;
        Ok(
            json!({"expected_identity":event.request.expected_identity,"operation":event.request.operation,"accepted":true,"recorded_at_s":event.recorded_at_s}),
        )
    }

    pub(crate) fn prepare_workflow_package(
        &self,
        repository_index: u32,
        actor_key_index: u32,
        hash_text: &str,
        selected_job: Option<u32>,
        raw: &[u8],
        manifest_limit: usize,
    ) -> StoreResult<(JsonValue, bool)> {
        // Use the same ordering as finish/abandon: operation, then Queue.
        // This also protects hash-parent removal against parent/child creation.
        let _operation = self
            .package_operation_lock
            .lock()
            .map_err(|_| BinaryDbError::other("Package operation lock is poisoned"))?;
        let hash = parse_package_hash(hash_text).map_err(invalid)?;
        if !raw.is_empty() && <[u8; 32]>::from(Sha256::digest(raw)) != hash {
            return Err(invalid("Package SHA-256 differs from its upload header"));
        }
        let repository = self.repository(repository_index)?;
        repository
            .jobs
            .recover_prepared_ci_publication(repository.authority.db())?;
        let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(
            repository.authority.db().clone(),
            &namespace_text(repository.entry.record.namespace_ascii),
        )
        .map_err(invalid)?
        .with_repository_index(repository_index);
        workflow
            .validate_workflow_package_actor(actor_key_index)
            .map_err(invalid)?;
        for job in repository.jobs.list()? {
            if workflow
                .lookup_accepted_workflow_package(job, hash_text, actor_key_index)
                .map_err(invalid)?
                .is_some()
            {
                return Err(invalid("Final Remote Task is terminal; whole published batch requires exact readback, not new CI/finish"));
            }
        }
        let reprepare_origin = if raw.is_empty() {
            Some(WorkerJobKey {
                repository_index,
                worker_job_index: selected_job.ok_or_else(|| {
                    invalid("Retained reprepare requires the original Worker Job selector")
                })?,
            })
        } else {
            None
        };
        let retained = if raw.is_empty() {
            Some(self.retained_reprepare_input(
                &repository,
                selected_job.ok_or_else(|| {
                    invalid("Retained reprepare requires the original Worker Job selector")
                })?,
                hash,
                hash_text,
                manifest_limit,
            )?)
        } else {
            None
        };
        let (raw, selected_job) = retained
            .as_ref()
            .map(|(bytes, replacement)| (bytes.as_slice(), *replacement))
            .unwrap_or((raw, selected_job));
        let (job, reused) = if let Some(index) = selected_job {
            (repository.jobs.resume_prepared_ci(index, hash).map_err(|error|invalid(format!("Original package execution is unavailable; run fresh ready after resolving that attempt: {error}")))?, true)
        } else {
            repository.jobs.enqueue_prepared_ci_with(
                hash,
                1,
                now_s_store()?,
                |_| Err(invalid("Previous package execution lost its Server association; rerun existing ready after Server reset")),
                |key| {
                    let parent = self
                        .package_worktrees
                        .join(repository_index.to_string())
                        .join(hash_text);
                    let new_parent = match fs::symlink_metadata(&parent) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                        Err(error) => return Err(BinaryDbError::io("inspect package worktree parent", error)),
                        Ok(_) => false,
                    };
                    fs::create_dir_all(&parent).map_err(|error| {
                        BinaryDbError::io("prepare package worktree parent", error)
                    })?;
                    if canonical_real_directory(&parent).map_err(invalid)? != parent {
                        return Err(invalid("Package worktree parent is not its managed real directory"));
                    }
                    let worktree = parent.join(key.worker_job_index.to_string());
                    if let Err(error) = fs::create_dir(&worktree) {
                        return Err(self.preparation_cleanup_error(
                            BinaryDbError::io("create package Job worktree", error),
                            if new_parent {
                                self.remove_empty_package_hash_parent(&parent)
                            } else {
                                Ok(())
                            },
                        ));
                    }
                    let result = (|| {
                        let package = WorkflowPackage::unpack(
                            raw,
                            &worktree.join("inputs"),
                            repository_index,
                            hash_text,
                            manifest_limit,
                        )
                        .map_err(invalid)?;
                        let identity = ait_server_core::foundation::workflow_package_contract::PackageExpectedIdentity {
                            repository_index, worker_job_index: key.worker_job_index,
                            package_hash: hash_text.to_string(), base_snapshot_id: package.manifest.base_snapshot_id.clone(),
                            revision_snapshot_id: package.manifest.revision_snapshot_id.clone(),
                        };
                        let content_result = repository.native.prepare_workflow_package_content(&repository.entry.repo_name, &package);
                        let history_result = workflow.preflight_workflow_package_history(&package.manifest, &identity);
                        let mut failures = Vec::new();
                        if let Err(error) = content_result { failures.push(format!("content: {error}")); }
                        if let Err(error) = history_result { failures.push(format!("history: {error}")); }
                        if !failures.is_empty() { return Err(invalid(failures.join("; "))); }
                        let source = worktree.join("source");
                        repository
                            .native
                            .materialize_workflow_package(
                                &repository.entry.repo_name,
                                &package,
                                &source,
                            )
                            .map_err(invalid)?;
                        let package_path = worktree.join("package.zst");
                        fs::write(&package_path, raw)
                            .map_err(|error| BinaryDbError::io("retain original package", error))?;
                        sync_filesystem_file(&package_path)
                            .map_err(|error| BinaryDbError::io("sync original package", error))?;
                        ManagedCiPackage::open(
                            repository_index,
                            ManagedCiPackagePaths {
                                managed_root: self.package_worktrees.clone(),
                                worktree: worktree.clone(),
                                sources: source,
                                package: package_path,
                            },
                            hash,
                            PreparedCiTarget {
                                target_line: package.manifest.target_line,
                                base_snapshot_id: package.manifest.base_snapshot_id,
                                revision_snapshot_id: package.manifest.revision_snapshot_id,
                            },
                        )
                    })();
                    result.map_err(|error| {
                        self.preparation_cleanup_error(
                            error,
                            self.cleanup_failed_package_preparation(&worktree, &parent),
                        )
                    })
                },
            )?
        };
        if let Some(origin) = reprepare_origin {
            repository
                .jobs
                .prepared_ci_runtime()
                .remember_reprepare_origin(job.key, origin)?;
            if let Err(error) =
                self.cleanup_reprepared_source(&repository, job, origin, hash, raw, manifest_limit)
            {
                eprintln!("warning: prior preformal package cleanup pending: {error}");
            }
        }
        let projection = self.package_execution_projection(&repository, job);
        let state = projection["ci_state"].as_str().ok_or_else(|| {
            invalid("Package execution association is unavailable; run fresh ready")
        })?;
        if !matches!(
            state,
            "queued" | "running" | "passed" | "failed" | "interrupted"
        ) {
            return Err(invalid(
                "Original package execution has no trustworthy result; run fresh ready",
            ));
        }
        if matches!(state, "queued" | "running" | "passed") && projection["recoverable"] != true {
            return Err(invalid(
                "Original package source is unavailable; run fresh ready",
            ));
        }
        Ok((
            json!({"expected_identity":projection["expected_identity"],"ci_state":state,"reused":reused}),
            matches!(state, "queued" | "running"),
        ))
    }

    pub(super) fn observe_committed_package_cleanup(
        &self,
        repository: &OperationalRepositoryRuntime,
        job: WorkerJobEntry,
    ) {
        let result = self
            .package_operation_lock
            .lock()
            .map_err(|_| BinaryDbError::other("Package operation lock is poisoned"))
            .and_then(|_operation| self.complete_committed_package_cleanup(repository, job));
        if let Err(error) = result {
            eprintln!("warning: committed package cleanup Repository {} Job {} failed; exact lifecycle observation will retry: {error}", job.key.repository_index, job.key.worker_job_index);
        }
    }
}
