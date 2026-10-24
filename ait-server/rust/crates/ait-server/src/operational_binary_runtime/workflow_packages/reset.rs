use super::*;
use ait_server_core::foundation::operational_binary_v0::WORKER_JOB_OUTCOME_FAILED;
use ait_server_core::foundation::prepared_ci_runtime::remove_empty_managed_runtime_container;

#[cfg(windows)]
type RetainedSourceScope = ait_server_core::foundation::prepared_ci_runtime::WindowsManagedSource;
#[cfg(unix)]
struct RetainedSourceScope {
    _root: fs::File,
}
#[cfg(not(any(unix, windows)))]
struct RetainedSourceScope;

impl OperationalBinaryRuntime {
    fn reset_execution_stopped(&self, job: WorkerJobEntry) -> bool {
        (matches!(
            job.record.state_kind,
            WORKER_JOB_STATE_SUCCEEDED | WORKER_JOB_STATE_FAILED
        ) && self.preformal_stopped_attempts.get(&job.key) == Some(&job.record.attempt_count))
            || (job.record.state_kind == WORKER_JOB_STATE_FAILED
                && job.record.outcome_kind == WORKER_JOB_OUTCOME_FAILED
                && job.record.attempt_count == 0)
    }
    /// Existing empty-body prepare: no archive is exposed to the client and no
    /// original execution evidence is recovered. Caller holds package exclusion.
    pub(super) fn retained_reprepare_input(
        &self,
        repository: &OperationalRepositoryRuntime,
        old_index: u32,
        hash: [u8; 32],
        hash_text: &str,
        manifest_limit: usize,
    ) -> StoreResult<(Vec<u8>, Option<u32>)> {
        let old = repository.jobs.get(old_index)?;
        if !self.preformal_reset_jobs.contains(&old.key)
            || old.record.is_tombstoned()
            || old.record.job_kind != WORKER_JOB_KIND_PATCHSET_CI
            || old.record.patchset_index_plus1 != 0
            || repository
                .jobs
                .prepared_ci_runtime()
                .source(old.key)
                .is_ok()
        {
            return Err(invalid(
                "Original package execution has no confirmed preformal reset",
            ));
        }
        if !self.reset_execution_stopped(old)
            || self.leases.entries()?.iter().any(|lease| {
                lease.key.repository_index == old.key.repository_index
                    && lease.key.worker_job_index == old.key.worker_job_index
            })
        {
            return Err(invalid(
                "Original package execution stop is unavailable; resources retained",
            ));
        }
        let runtime = repository.jobs.prepared_ci_runtime();
        let mut replacement_input = None;
        if let Some(replacement) = runtime.selected_job(old.key.repository_index, hash)? {
            if replacement == old.key
                || self.preformal_reset_jobs.contains(&replacement)
                || !runtime.recoverable_selection(replacement)
            {
                return Err(invalid(
                    "Replacement package execution is unavailable or ambiguous",
                ));
            }
            let actual = repository
                .jobs
                .resume_prepared_ci(replacement.worker_job_index, hash)?;
            let projection = self.package_execution_projection(repository, actual);
            if projection["availability"] != "available"
                || !matches!(
                    projection["ci_state"].as_str(),
                    Some("queued" | "running" | "passed" | "failed" | "interrupted")
                )
            {
                return Err(invalid(
                    "Replacement package execution lacks verified sealed evidence",
                ));
            }
            let source = runtime.source(replacement)?;
            let _scope = self.verify_reprepare_source_scope(source.worktree())?;
            #[cfg(not(windows))]
            let raw = fs::read(source.package())
                .map_err(|e| invalid(format!("Replacement package is unavailable: {e}")))?;
            #[cfg(windows)]
            let raw = _scope
                .read_package()
                .map_err(|e| invalid(format!("Replacement package is unavailable: {e}")))?;
            WorkflowPackage::read_sealed_manifest(
                &raw,
                hash,
                old.key.repository_index,
                manifest_limit,
            )
            .map_err(invalid)?;
            if runtime.is_reprepared_from(replacement, old.key)? {
                return Ok((raw, Some(replacement.worker_job_index)));
            }
            replacement_input = Some((raw, replacement.worker_job_index));
        }
        let retained = self.retained_package_job_paths(old.key.repository_index, old_index)?;
        if retained.len() != 1 || retained[0].1 != hash {
            return Err(invalid(
                "Original package source is unavailable, foreign or ambiguous",
            ));
        }
        let worktree = self
            .package_worktrees
            .join(old.key.repository_index.to_string())
            .join(hash_text)
            .join(old_index.to_string());
        if retained[0].0 != worktree {
            return Err(invalid(
                "Original package source differs from its native managed path",
            ));
        }
        let _scope = self.verify_reprepare_source_scope(&worktree)?;
        for directory in [
            worktree.parent().and_then(Path::parent),
            worktree.parent(),
            Some(worktree.as_path()),
        ] {
            let directory =
                directory.ok_or_else(|| invalid("Original package native path is unavailable"))?;
            if canonical_real_directory(directory)
                .map_err(|e| invalid(format!("Original package path unavailable: {e}")))?
                != directory
            {
                return Err(invalid(
                    "Original package path differs from its native managed scope",
                ));
            }
        }
        let package = worktree.join("package.zst");
        let metadata = fs::symlink_metadata(&package)
            .map_err(|e| invalid(format!("Original sealed package is unavailable: {e}")))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("Original sealed package is foreign or unavailable"));
        }
        #[cfg(not(windows))]
        let raw = fs::read(&package)
            .map_err(|e| invalid(format!("Original sealed package is unavailable: {e}")))?;
        #[cfg(windows)]
        let raw = _scope
            .read_package()
            .map_err(|e| invalid(format!("Original sealed package is unavailable: {e}")))?;
        let manifest = WorkflowPackage::read_sealed_manifest(
            &raw,
            hash,
            old.key.repository_index,
            manifest_limit,
        )
        .map_err(|e| invalid(format!("Original sealed package differs: {e}")))?;
        let source = ManagedCiPackage::open(
            old.key.repository_index,
            ManagedCiPackagePaths {
                managed_root: self.package_worktrees.clone(),
                sources: worktree.join("source"),
                package,
                worktree: worktree.clone(),
            },
            hash,
            PreparedCiTarget {
                target_line: manifest.target_line,
                base_snapshot_id: manifest.base_snapshot_id,
                revision_snapshot_id: manifest.revision_snapshot_id,
            },
        )
        .map_err(|e| invalid(format!("Original package source is unavailable: {e}")))?;
        if source.worktree() != worktree {
            return Err(invalid(
                "Original package source differs from its native binding",
            ));
        }
        source.verify()?;
        if let Some((replacement_raw, replacement)) = replacement_input {
            if replacement_raw != raw {
                return Err(invalid(
                    "Replacement sealed bytes differ from the original reset source",
                ));
            }
            Ok((raw, Some(replacement)))
        } else {
            Ok((raw, None))
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn verify_reprepare_source_scope(&self, _worktree: &Path) -> StoreResult<RetainedSourceScope> {
        Err(invalid(
            "Retained-source ownership verification is unavailable on this platform",
        ))
    }

    #[cfg(windows)]
    fn verify_reprepare_source_scope(&self, worktree: &Path) -> StoreResult<RetainedSourceScope> {
        ait_server_core::foundation::prepared_ci_runtime::verify_managed_runtime_source(
            &self.package_worktrees,
            worktree,
        )
        .map_err(|e| {
            invalid(format!(
                "Retained source Windows ownership/path unavailable: {e}"
            ))
        })
    }

    #[cfg(unix)]
    fn verify_reprepare_source_scope(&self, worktree: &Path) -> StoreResult<RetainedSourceScope> {
        let root =
            ait_server_core::foundation::prepared_ci_runtime::open_owned_managed_runtime_root(
                &self.package_worktrees,
            )?;
        let parent = worktree
            .parent()
            .ok_or_else(|| invalid("Retained source has no managed parent"))?;
        let repository = parent
            .parent()
            .ok_or_else(|| invalid("Retained source has no Repository parent"))?;
        #[cfg(unix)]
        let owner = root
            .metadata()
            .map_err(|e| invalid(format!("Managed source root is unavailable: {e}")))?
            .uid();
        for directory in [
            self.package_worktrees.as_path(),
            repository,
            parent,
            worktree,
            &worktree.join("source"),
        ] {
            let metadata = fs::symlink_metadata(directory)
                .map_err(|e| invalid(format!("Retained source directory is unavailable: {e}")))?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || canonical_real_directory(directory).map_err(invalid)? != directory
            {
                return Err(invalid("Retained source directory is foreign or linked"));
            }
            #[cfg(unix)]
            if metadata.uid() != owner || metadata.mode() & 0o022 != 0 {
                return Err(invalid(
                    "Retained source parent is not exclusively Server-owned",
                ));
            }
        }
        for entry in fs::read_dir(worktree)
            .map_err(|e| invalid(format!("Retained source is unavailable: {e}")))?
        {
            let entry =
                entry.map_err(|e| invalid(format!("Retained source child is unavailable: {e}")))?;
            if !matches!(
                entry.file_name().to_str(),
                Some("package.zst" | "source" | "inputs" | "publication-inputs")
            ) {
                return Err(invalid(
                    "Retained source contains foreign runtime resources",
                ));
            }
        }
        Ok(RetainedSourceScope { _root: root })
    }

    /// A verified new/reuploaded package is retained before considering an old
    /// copy. This establishes only cleanup ownership, never old CI/governance.
    pub(super) fn cleanup_reprepared_source(
        &self,
        repository: &OperationalRepositoryRuntime,
        current: WorkerJobEntry,
        origin: WorkerJobKey,
        hash: [u8; 32],
        raw: &[u8],
        manifest_limit: usize,
    ) -> StoreResult<()> {
        let new_source = repository.jobs.prepared_ci_runtime().source(current.key)?;
        new_source.verify()?;
        let _new_scope = self.verify_reprepare_source_scope(new_source.worktree())?;
        sync_filesystem_directory(new_source.worktree())
            .map_err(|e| BinaryDbError::io("sync replacement sealed package directory", e))?;
        sync_filesystem_directory(
            new_source
                .worktree()
                .parent()
                .ok_or_else(|| invalid("New sealed package has no managed parent"))?,
        )
        .map_err(|e| BinaryDbError::io("sync replacement sealed package parent", e))?;
        let hash_text = new_source
            .expected_identity(current.key)
            .map_err(invalid)?
            .package_hash;
        for old in repository
            .jobs
            .list()?
            .into_iter()
            .filter(|entry| entry.key == origin)
        {
            if old.key == current.key
                || old.record.is_tombstoned()
                || old.record.job_kind != WORKER_JOB_KIND_PATCHSET_CI
                || old.record.patchset_index_plus1 != 0
                || repository
                    .jobs
                    .prepared_ci_runtime()
                    .source(old.key)
                    .is_ok()
            {
                continue;
            }
            let parent = self
                .package_worktrees
                .join(old.key.repository_index.to_string())
                .join(&hash_text);
            let worktree = parent.join(old.key.worker_job_index.to_string());
            let metadata = match fs::symlink_metadata(&worktree) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(BinaryDbError::io("inspect old preformal package", error))
                }
            };
            if !self.reset_execution_stopped(old)
                || self.leases.entries()?.iter().any(|lease| {
                    lease.key.repository_index == old.key.repository_index
                        && lease.key.worker_job_index == old.key.worker_job_index
                })
            {
                return Err(BinaryDbError::retryable_busy(format!(
                    "Old preformal Job {} has no confirmed Runner stop; resources retained",
                    old.key.worker_job_index
                )));
            }
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(invalid(
                    "Old preformal worktree is not its real managed directory",
                ));
            }
            if fs::read_dir(&worktree)
                .map_err(|e| BinaryDbError::io("inspect old preformal contents", e))?
                .next()
                .transpose()
                .map_err(|e| BinaryDbError::io("inspect old preformal child", e))?
                .is_none()
            {
                if !remove_empty_managed_runtime_container(&self.package_worktrees, &worktree)? {
                    return Err(BinaryDbError::retryable_busy(
                        "Old runtime container is not empty",
                    ));
                }
                self.remove_empty_package_hash_parent(&parent)?;
                continue;
            }
            let package_path = worktree.join("package.zst");
            let _old_scope = self.verify_reprepare_source_scope(&worktree)?;
            #[cfg(not(windows))]
            let retained = fs::read(&package_path)
                .map_err(|e| BinaryDbError::io("read old preformal sealed package", e))?;
            #[cfg(windows)]
            let retained = _old_scope
                .read_package()
                .map_err(|e| BinaryDbError::io("read old preformal sealed package", e))?;
            if retained != raw {
                return Err(invalid(
                    "Old preformal sealed package differs; resources retained",
                ));
            }
            let manifest = WorkflowPackage::read_sealed_manifest(
                &retained,
                hash,
                old.key.repository_index,
                manifest_limit,
            )
            .map_err(invalid)?;
            let source = ManagedCiPackage::open(
                old.key.repository_index,
                ManagedCiPackagePaths {
                    managed_root: self.package_worktrees.clone(),
                    sources: worktree.join("source"),
                    package: package_path,
                    worktree,
                },
                hash,
                PreparedCiTarget {
                    target_line: manifest.target_line,
                    base_snapshot_id: manifest.base_snapshot_id,
                    revision_snapshot_id: manifest.revision_snapshot_id,
                },
            )?;
            #[cfg(windows)]
            let _parents = _old_scope.retain_parents();
            if !self.cleanup_owned_package(&source)? {
                return Err(BinaryDbError::retryable_busy(
                    "Old preformal package still has readers",
                ));
            }
        }
        Ok(())
    }
}
