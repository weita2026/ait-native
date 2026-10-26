use super::*;

pub(super) fn committed_cleanup_error(error: BinaryDbError) -> BinaryDbError {
    let prefix = if error.kind() == BinaryDbErrorKind::InvalidDomainData {
        "PACKAGE_FINISH_CLEANUP_UNKNOWN"
    } else {
        "PACKAGE_FINISH_CLEANUP_PENDING"
    };
    BinaryDbError::new(error.kind(), format!(
        "{prefix}: publication committed; cleanup incomplete or unknown; retain this identity and retry finish for cleanup only: {error}"
    ))
}

impl OperationalBinaryRuntime {
    /// Only call after proving this exact formal publication. The caller holds
    /// package_operation_lock through lookup, cleanup and response selection.
    pub(super) fn complete_committed_package_cleanup(
        &self,
        repository: &OperationalRepositoryRuntime,
        job: WorkerJobEntry,
    ) -> StoreResult<()> {
        let pending = committed_cleanup_error;
        if self.leases.entries().map_err(pending)?.iter().any(|lease| {
            lease.key.repository_index == job.key.repository_index
                && lease.key.worker_job_index == job.key.worker_job_index
        }) {
            return Err(pending(BinaryDbError::retryable_busy(
                "Exact Job still has a worker lease",
            )));
        }
        let execution = repository.jobs.prepared_ci_runtime();
        if !execution
            .committed_cleanup_complete(job.key)
            .map_err(pending)?
            && execution
                .committed_source(job.key)
                .map_err(pending)?
                .is_some()
        {
            if !execution
                .cleanup_committed_with(job.key, |source| self.cleanup_owned_package(source))
                .map_err(pending)?
            {
                return Err(pending(BinaryDbError::retryable_busy(
                    "Exact package source still has active readers",
                )));
            }
            return Ok(());
        }
        // The bounded empty-container rule supplies physical cleanup only.
        // Formal acceptance remains independent of any missing temporary state.
        let retained = self
            .retained_package_job_paths(job.key.repository_index, job.key.worker_job_index)
            .map_err(pending)?;
        if retained.len() > 1 {
            return Err(pending(invalid(
                "Exact Job has ambiguous retained runtime containers",
            )));
        }
        for (worktree, _) in retained {
            if ait_server_core::foundation::prepared_ci_runtime::remove_empty_managed_runtime_container(
                &self.package_worktrees, &worktree,
            ).map_err(pending)? {
                self.remove_empty_package_hash_parent(worktree.parent().ok_or_else(|| pending(invalid("Package child has no parent")))?).map_err(pending)?;
            }
        }
        // Neither a caller hash nor payload files recreate logical ownership.
        // Search only the exact Job through the existing retained-source lookup.
        if !self
            .retained_package_job_paths(job.key.repository_index, job.key.worker_job_index)
            .map_err(pending)?
            .is_empty()
        {
            return Err(pending(invalid(
                "Retained exact Job resources have no provable cleanup owner",
            )));
        }
        Ok(())
    }

    /// The operation lock serializes this final parent removal with creation.
    pub(super) fn cleanup_owned_package(&self, source: &ManagedCiPackage) -> StoreResult<bool> {
        let hash = source
            .sha256()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let parent = self
            .package_worktrees
            .join(source.repository_index().to_string())
            .join(hash);
        if source.worktree().parent() != Some(parent.as_path()) {
            return Err(invalid(
                "Owned package hash parent differs from its managed path",
            ));
        }
        let cleaned = match source.request_cleanup() {
            Ok(cleaned) => cleaned,
            Err(error)
                if error.kind() == BinaryDbErrorKind::InvalidDomainData
                    && error.contains("recreated after owned child cleanup") =>
            {
                // The original cleanup already fenced readers. Product permits
                // only this physical empty-slot retry; payload remains unknown.
                if ait_server_core::foundation::prepared_ci_runtime::remove_empty_managed_runtime_container(
                    &self.package_worktrees, source.worktree(),
                )? {
                    true
                } else {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        };
        if !cleaned {
            return Ok(false);
        }
        self.remove_empty_package_hash_parent(&parent)?;
        Ok(true)
    }

    pub(super) fn remove_empty_package_hash_parent(&self, parent: &Path) -> StoreResult<()> {
        if parent.parent().and_then(Path::parent) != Some(self.package_worktrees.as_path())
            || parent
                .file_name()
                .and_then(|n| n.to_str())
                .is_none_or(|n| parse_package_hash(n).is_err())
        {
            return Err(invalid(
                "Package hash parent is outside its native runtime scope",
            ));
        }
        #[cfg(any(unix, windows))]
        ait_server_core::foundation::prepared_ci_runtime::remove_empty_managed_runtime_container(
            &self.package_worktrees,
            parent,
        )
        .map_err(|error| {
            BinaryDbError::new(
                error.kind(),
                format!("remove exact empty package hash parent: {error}"),
            )
        })?;
        #[cfg(not(any(unix, windows)))]
        {
            // Preserve the existing owned-source path on other platforms.
            // Unknown empty-container recovery still requires safe handles.
            match fs::symlink_metadata(parent) {
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
                Ok(metadata)
                    if metadata.is_dir()
                        && !metadata.file_type().is_symlink()
                        && canonical_real_directory(parent).map_err(invalid)? == parent => {}
                Err(error) => {
                    return Err(BinaryDbError::io(
                        "inspect owned package hash parent",
                        error,
                    ))
                }
                _ => {
                    return Err(invalid(
                        "Owned package hash parent is not its real directory",
                    ))
                }
            }
            match fs::remove_dir(parent) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => {
                    return Err(BinaryDbError::io("remove owned package hash parent", error))
                }
            }
        }
        Ok(())
    }

    pub(super) fn cleanup_failed_package_preparation(
        &self,
        worktree: &Path,
        parent: &Path,
    ) -> StoreResult<()> {
        match fs::symlink_metadata(worktree) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(BinaryDbError::io(
                    "inspect failed package preparation",
                    error,
                ))
            }
            Ok(metadata)
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && canonical_real_directory(worktree).map_err(invalid)? == worktree =>
            {
                fs::remove_dir_all(worktree).map_err(|error| {
                    BinaryDbError::io("remove failed package preparation", error)
                })?;
            }
            _ => {
                return Err(invalid(
                    "Failed package preparation no longer owns its directory",
                ))
            }
        }
        self.remove_empty_package_hash_parent(parent)
    }

    pub(super) fn preparation_cleanup_error(
        &self,
        original: BinaryDbError,
        cleanup: StoreResult<()>,
    ) -> BinaryDbError {
        match cleanup {
            Ok(()) => original,
            Err(error) => BinaryDbError::new(
                original.kind(),
                format!("{original}; exact package preparation cleanup incomplete: {error}"),
            ),
        }
    }

    pub(super) fn retained_package_job_paths(
        &self,
        repository_index: u32,
        worker_job_index: u32,
    ) -> StoreResult<Vec<(PathBuf, [u8; 32])>> {
        let root = self.package_worktrees.join(repository_index.to_string());
        let directories = match fs::read_dir(&root) {
            Ok(directories) => directories,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(BinaryDbError::io(
                    "inspect retained exact Job sources",
                    error,
                ))
            }
        };
        if canonical_real_directory(&root).map_err(invalid)? != root {
            return Err(invalid(
                "Retained package Repository root is not its real directory",
            ));
        }
        let mut retained = Vec::new();
        for directory in directories {
            let directory = directory
                .map_err(|error| BinaryDbError::io("read retained package hash", error))?;
            let Some(hash) = directory
                .file_name()
                .to_str()
                .and_then(|name| parse_package_hash(name).ok())
            else {
                continue;
            };
            if !directory
                .file_type()
                .map_err(|error| BinaryDbError::io("inspect retained package hash", error))?
                .is_dir()
            {
                return Err(invalid(
                    "Retained package hash parent is not its real directory",
                ));
            }
            let worktree = directory.path().join(worker_job_index.to_string());
            match fs::symlink_metadata(&worktree) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(BinaryDbError::io(
                        "inspect retained exact Job source",
                        error,
                    ))
                }
                Ok(_) => retained.push((worktree, hash)),
            }
        }
        Ok(retained)
    }
}
