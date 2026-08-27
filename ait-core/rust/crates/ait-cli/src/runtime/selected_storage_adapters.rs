use super::*;

impl<const WRITE_LAYOUT: u32> RepoBinaryDbStoreFactory<WRITE_LAYOUT> {
    pub fn new(
        repo_root: impl Into<PathBuf>,
        authority_root: impl Into<PathBuf>,
        local_authority_id: AuthorityId,
        current_line_state_scope: LocalStateScope,
    ) -> Self {
        let repo_root = repo_root.into();
        Self {
            pack_root: repo_root.clone(),
            repo_root,
            authority_root: authority_root.into(),
            local_authority_id,
            id_namespace_prefix: String::new(),
            current_line_state_scope,
            admission_error: None,
            generation_guard: None,
        }
    }

    pub fn from_runtime(repo: &RepoRuntime) -> Self {
        let repo_root = repo.authoritative_repo_root();
        let authority_root = repo_root.join(APP_DIR).join(BINARY_DB_DIR);
        let local_authority_id = AuthorityId::new(format!("local:{}", repo.repo_name()));
        let id_namespace_prefix = repo.id_namespace_prefix();
        let current_line_state_scope = if repo.is_worktree() {
            LocalStateScope::Task
        } else {
            LocalStateScope::Repository
        };
        match admit_activated_binary_db_generation_for_runtime(
            &repo_root,
            &authority_root,
            &repo.repo_name(),
        ) {
            Ok((generation, generation_guard)) => Self {
                repo_root,
                authority_root: generation.authority_root,
                pack_root: generation.generation_root,
                local_authority_id,
                id_namespace_prefix,
                current_line_state_scope,
                admission_error: None,
                generation_guard: Some(generation_guard),
            },
            Err(_error) if cfg!(test) && (!authority_root.exists() || authority_root.is_dir()) => {
                Self::new(
                    repo_root,
                    authority_root,
                    local_authority_id,
                    current_line_state_scope,
                )
            }
            Err(error) => Self {
                pack_root: repo_root
                    .join(APP_DIR)
                    .join("unadmitted-binary-db-generation"),
                repo_root,
                authority_root,
                local_authority_id,
                id_namespace_prefix,
                current_line_state_scope,
                admission_error: Some(format!(
                    "Selected Binary DB generation failed activation admission: {error}"
                )),
                generation_guard: None,
            },
        }
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    pub fn authority_root(&self) -> &Path {
        &self.authority_root
    }

    pub fn pack_root(&self) -> &Path {
        &self.pack_root
    }

    pub fn local_authority_id(&self) -> &AuthorityId {
        &self.local_authority_id
    }

    pub fn current_line_state_scope(&self) -> LocalStateScope {
        self.current_line_state_scope
    }

    pub fn local_db(&self) -> LocalBinaryDbFs {
        LocalBinaryDbFs::new(
            StorePath::from(self.authority_root.clone()),
            StorePath::from(self.repo_root.clone()),
            self.local_authority_id.clone(),
            self.current_line_state_scope,
        )
        .with_declared_bin_paths(ait_core::binary_db::REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(ait_core::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS)
        .with_admission_error(self.admission_error.clone())
        .with_generation_guard(self.generation_guard.clone())
    }

    pub fn plans(&self) -> LocalRepositoryPlanStore<WRITE_LAYOUT> {
        LocalRepositoryPlanStore::from_db(
            self.local_authority_id
                .0
                .as_str()
                .strip_prefix("local:")
                .unwrap_or(self.local_authority_id.0.as_str()),
            self.local_db(),
        )
    }

    pub fn status(&self) -> BinaryDbRepoStatusStore<LocalBinaryDbFs, WRITE_LAYOUT> {
        BinaryDbRepoStatusStore::new(self.local_db())
    }

    pub fn lines(&self) -> BinaryDbLineStore<LocalBinaryDbFs, WRITE_LAYOUT> {
        BinaryDbLineStore::new(self.local_db())
    }

    pub fn stashes(&self) -> BinaryDbStashStore<LocalBinaryDbFs, WRITE_LAYOUT> {
        BinaryDbStashStore::new(self.local_db())
    }

    pub fn workflows(&self) -> BinaryDbWorkflowStore<LocalBinaryDbFs, WRITE_LAYOUT> {
        let repo_name = self
            .local_authority_id
            .0
            .strip_prefix("local:")
            .unwrap_or(self.local_authority_id.0.as_str())
            .to_string();
        BinaryDbWorkflowStore::new_with_namespace(
            self.local_db(),
            repo_name,
            self.id_namespace_prefix.clone(),
        )
    }

    pub fn content(&self) -> LocalContentBinaryDb<WRITE_LAYOUT> {
        self.content_for_root(self.repo_root.clone())
    }

    pub fn content_for_root(
        &self,
        workspace_root: impl Into<PathBuf>,
    ) -> LocalContentBinaryDb<WRITE_LAYOUT> {
        LocalContentBinaryDb::from_db_with_roots(
            self.local_db(),
            StorePath::from(workspace_root.into()),
            StorePath::from(self.pack_root.clone()),
        )
    }
}

impl<const WRITE_LAYOUT: u32> RepoRemoteSyncBinaryDbLocalStore<WRITE_LAYOUT> {
    pub(super) fn from_content_and_lines(
        content: LocalContentBinaryDb<WRITE_LAYOUT>,
        lines: BinaryDbLineStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    ) -> Self {
        let blobs = content.blobs().clone();
        let object_packs = content.object_packs().clone();
        let tree_packs = content.tree_packs().clone();
        let trees = content.trees().clone();
        let snapshots = content.snapshots().clone();
        Self {
            import_store: BinaryDbRemoteSyncZstdImportStore::with_content_stores(
                blobs,
                object_packs.clone(),
                tree_packs.clone(),
                trees,
                snapshots.clone(),
            ),
            lines,
            blobs: content.blobs().clone(),
            snapshots,
            object_packs,
            tree_packs,
            trees: content.trees().clone(),
        }
    }

    pub(crate) fn line_by_name(&self, line_name: &str) -> Result<Option<LineRecord>, String> {
        self.lines.line_by_name(line_name)
    }

    pub(crate) fn create_line(
        &self,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        created_at: &str,
    ) -> Result<JsonValue, String> {
        self.lines
            .create_line(line_name, head_snapshot_id, created_at)
            .map(|line| line_record_json(&line))
    }

    pub(crate) fn set_line_head(
        &self,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        updated_at: &str,
    ) -> Result<JsonValue, String> {
        self.lines
            .set_line_head(line_name, head_snapshot_id, updated_at)
            .map(|line| line_record_json(&line))
    }

    #[cfg(test)]
    pub(crate) fn snapshot_exists(&self, snapshot_id: &str) -> SnapshotStoreResult<bool> {
        self.snapshots.snapshot_exists(snapshot_id)
    }

    #[cfg(test)]
    pub(crate) fn snapshot_chain(&self, snapshot_id: &str) -> SnapshotStoreResult<Vec<String>> {
        self.snapshots.snapshot_chain(snapshot_id)
    }
}

impl<const WRITE_LAYOUT: u32> RepoBinaryDbLocalSnapshotOperationStore<WRITE_LAYOUT> {
    pub(super) fn from_content_and_lines(
        content: LocalContentBinaryDb<WRITE_LAYOUT>,
        lines: BinaryDbLineStore<LocalBinaryDbFs, WRITE_LAYOUT>,
        worktree_config_path: Option<PathBuf>,
    ) -> Self {
        Self {
            content,
            lines,
            worktree_config_path,
        }
    }

    pub(super) fn worktree_materialized_snapshot_id(&self, line_name: &str) -> Option<String> {
        let path = self.worktree_config_path.as_ref()?;
        let payload = read_json_object(path);
        let current_line = payload.get("current_line").and_then(as_nonempty_string);
        if current_line.as_deref() != Some(line_name) {
            return None;
        }
        payload
            .get("materialized_snapshot_id")
            .and_then(as_nonempty_string)
    }

    pub(crate) fn ensure_blob_bytes(
        &self,
        data: &[u8],
        path_hint: Option<&str>,
    ) -> Result<String, String> {
        self.content.ensure_blob_bytes_content(data, path_hint)
    }

    pub(crate) fn set_line_head(
        &self,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        updated_at: &str,
    ) -> Result<JsonValue, String> {
        self.lines
            .set_line_head(line_name, head_snapshot_id, updated_at)
            .map(|line| line_record_json(&line))
    }

    pub(crate) fn line_updated_at(&self, line_name: &str) -> Result<Option<String>, String> {
        self.lines.line_updated_at(line_name)
    }

    pub(crate) fn set_line_updated_at(
        &self,
        line_name: &str,
        updated_at: Option<&str>,
    ) -> Result<(), String> {
        self.lines.set_line_updated_at(line_name, updated_at)
    }
}

impl RepoRuntime {
    pub fn control_plane_store_decision(
        &self,
        family: ControlPlaneStoreFamily,
    ) -> ControlPlaneStoreDecision {
        if family == ControlPlaneStoreFamily::CurrentLine {
            return ControlPlaneStoreDecision {
                family,
                mode: ControlPlaneStoreDecisionMode::RepositoryConfig,
                owner_phase: "current-line-config-cutover",
                runtime_accessor: "current_line_name",
                reason: "Current-line selection lives in .ait/config.json or the worktree overlay; it is configuration state, never database authority.",
            };
        }
        if family == ControlPlaneStoreFamily::Remote {
            return ControlPlaneStoreDecision {
                family,
                mode: ControlPlaneStoreDecisionMode::RepositoryConfig,
                owner_phase: "remote-config-json-cutover",
                runtime_accessor: "remote_store",
                reason: "Remote endpoint metadata lives in .ait/config.json; it is configuration state, never database authority.",
            };
        }
        let runtime_accessor = match family {
            ControlPlaneStoreFamily::Line => "line_store",
            ControlPlaneStoreFamily::Stash => "stash_store",
            ControlPlaneStoreFamily::RepoStatus => "repo_status_store",
            ControlPlaneStoreFamily::CurrentLine | ControlPlaneStoreFamily::Remote => {
                unreachable!()
            }
        };
        ControlPlaneStoreDecision {
            family,
            mode: ControlPlaneStoreDecisionMode::SelectedBinaryDb,
            owner_phase: "global-binary-db-only-runtime-closeout",
            runtime_accessor,
            reason: "Repository-authoritative runtime state is stored only in schema-declared layout-1 Binary DB files; no alternate backend selector or fallback exists.",
        }
    }

    pub fn control_plane_store_decisions(&self) -> Vec<ControlPlaneStoreDecision> {
        CONTROL_PLANE_STORE_FAMILIES
            .iter()
            .copied()
            .map(|family| self.control_plane_store_decision(family))
            .collect()
    }

    pub fn control_plane_store_decisions_json(&self) -> JsonValue {
        JsonValue::Array(
            self.control_plane_store_decisions()
                .iter()
                .map(ControlPlaneStoreDecision::to_json)
                .collect(),
        )
    }

    pub fn binary_db_stores<const WRITE_LAYOUT: u32>(
        &self,
    ) -> RepoBinaryDbStoreFactory<WRITE_LAYOUT> {
        RepoBinaryDbStoreFactory::from_runtime(self)
    }

    pub(crate) fn local_plan_head_artifacts(&self) -> Result<Vec<LocalPlanHeadArtifact>, String> {
        let store = self.binary_db_stores::<1>().plans();
        let read = store.begin_read_txn();
        store
            .list_plans(&read, Some(&self.repo_name()), None)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter_map(|plan| {
                plan.head_revision.map(|head| {
                    Ok(LocalPlanHeadArtifact {
                        status: plan.record.status_name().to_string(),
                        artifact_path: head
                            .payload
                            .artifact_path_text()
                            .map_err(|error| error.to_string())?,
                        artifact_blob_id: Some(
                            head.payload
                                .artifact_blob_id_text()
                                .map_err(|error| error.to_string())?,
                        ),
                    })
                })
            })
            .collect()
    }

    pub fn plan_binary_db_storage_request<const WRITE_LAYOUT: u32>(
        &self,
    ) -> Result<JsonValue, String> {
        Ok(self.binary_db_storage_request::<WRITE_LAYOUT>())
    }

    pub fn snapshot_binary_db_storage_request<const WRITE_LAYOUT: u32>(
        &self,
    ) -> Result<JsonValue, String> {
        Ok(self.binary_db_storage_request::<WRITE_LAYOUT>())
    }

    pub fn remote_sync_binary_db_storage_request<const WRITE_LAYOUT: u32>(
        &self,
    ) -> Result<JsonValue, String> {
        Ok(self.binary_db_storage_request::<WRITE_LAYOUT>())
    }

    fn binary_db_storage_request<const WRITE_LAYOUT: u32>(&self) -> JsonValue {
        let stores = self.binary_db_stores::<WRITE_LAYOUT>();
        let activation_pointer = self
            .authoritative_repo_root()
            .join(APP_DIR)
            .join(BINARY_DB_DIR);
        let current_line_scope = match stores.current_line_state_scope() {
            LocalStateScope::Repository => "repository",
            LocalStateScope::Line => "line",
            LocalStateScope::Task => "task",
            LocalStateScope::RemoteCache => "remote_cache",
        };
        JsonValue::Object(JsonMap::from_iter([
            (
                "write_layout".to_string(),
                JsonValue::Number(JsonNumber::from(WRITE_LAYOUT)),
            ),
            (
                "authority_root".to_string(),
                JsonValue::String(stores.authority_root().to_string_lossy().to_string()),
            ),
            (
                "activation_pointer".to_string(),
                JsonValue::String(activation_pointer.to_string_lossy().to_string()),
            ),
            (
                "pack_root".to_string(),
                JsonValue::String(stores.pack_root().to_string_lossy().to_string()),
            ),
            (
                "repo_root".to_string(),
                JsonValue::String(stores.repo_root().to_string_lossy().to_string()),
            ),
            (
                "local_authority_id".to_string(),
                JsonValue::String(stores.local_authority_id().0.clone()),
            ),
            (
                "current_line_state_scope".to_string(),
                JsonValue::String(current_line_scope.to_string()),
            ),
        ]))
    }

    pub fn local_content_maintenance_store<const WRITE_LAYOUT: u32>(
        &self,
    ) -> Result<RepoLocalContentMaintenanceStore<WRITE_LAYOUT>, String> {
        Ok(self.binary_db_stores::<WRITE_LAYOUT>().content())
    }

    pub fn local_snapshot_operation_store<const WRITE_LAYOUT: u32>(
        &self,
        workspace_root: &Path,
    ) -> Result<RepoLocalSnapshotOperationStore<WRITE_LAYOUT>, String> {
        let stores = self.binary_db_stores::<WRITE_LAYOUT>();
        Ok(
            RepoBinaryDbLocalSnapshotOperationStore::from_content_and_lines(
                stores.content_for_root(workspace_root.to_path_buf()),
                stores.lines(),
                self.worktree_config_path.clone(),
            ),
        )
    }

    pub fn remote_sync_local_store<const WRITE_LAYOUT: u32>(
        &self,
    ) -> Result<RepoRemoteSyncLocalStore<WRITE_LAYOUT>, String> {
        let stores = self.binary_db_stores::<WRITE_LAYOUT>();
        Ok(RepoRemoteSyncBinaryDbLocalStore::from_content_and_lines(
            stores.content(),
            stores.lines(),
        ))
    }
}
