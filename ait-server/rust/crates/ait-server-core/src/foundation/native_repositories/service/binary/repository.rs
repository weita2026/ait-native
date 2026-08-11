use super::*;

const MAX_ZSTD_BULK_MUTATIONS: usize = 100_000;

fn ensure_zstd_bulk_mutation_capacity(
    requested_mutation_count: usize,
) -> Result<(), NativeRepositoryError> {
    if requested_mutation_count > MAX_ZSTD_BULK_MUTATIONS {
        return Err(NativeRepositoryError::bad_request(format!(
            "zstd bulk write set has {requested_mutation_count} mutations, maximum is {MAX_ZSTD_BULK_MUTATIONS}"
        )));
    }
    Ok(())
}

impl<D> BinaryDbNativeRepositoryService<D>
where
    D: ServerRemoteBinaryDb + Clone,
{
    pub fn new(db: D) -> Self {
        Self {
            db,
            default_line: default_main_line(),
            id_namespace_prefix: "BIN".to_string(),
            created_at: now_rfc3339(),
        }
    }

    pub fn with_default_line(mut self, default_line: impl Into<String>) -> Self {
        self.default_line = default_line.into();
        self
    }

    pub fn db(&self) -> &D {
        &self.db
    }

    pub(super) fn repo_name(&self) -> &str {
        self.db.repo_name().as_str()
    }

    pub(super) fn repo_id(&self) -> &str {
        self.db.repo_id().as_str()
    }

    pub(super) fn content_lines(
        &self,
    ) -> ServerBinaryDbLineStore<D, SERVER_CONTENT_BINARY_LAYOUT_ID>
    where
        D: Clone,
    {
        ServerBinaryDbLineStore::new(self.db.clone())
    }

    pub(super) fn content_snapshots(
        &self,
    ) -> ServerBinaryDbSnapshotStore<D, SERVER_CONTENT_BINARY_LAYOUT_ID>
    where
        D: Clone,
    {
        ServerBinaryDbSnapshotStore::new(self.db.clone())
    }

    pub(super) fn repository_content(&self) -> ServerBinaryRepositoryContentStore<D>
    where
        D: Clone,
    {
        ServerBinaryRepositoryContentStore::new(self.db.clone())
    }

    pub(super) fn ensure_repository(&self, repo_name: &str) -> Result<(), NativeRepositoryError> {
        let repo_name = normalize_required_text(repo_name, "repo_name")?;
        if repo_name == self.repo_name() {
            Ok(())
        } else {
            Err(NativeRepositoryError::not_found(format!(
                "Unknown repository: {repo_name}"
            )))
        }
    }

    #[cfg(test)]
    pub(super) fn ensure_test_fixture_authority(&self) -> Result<(), NativeRepositoryError> {
        if self.db.authority_mode().is_test_fixture() {
            Ok(())
        } else {
            Err(NativeRepositoryError::conflict(
                "Binary DB fixture seeding requires test-fixture authority",
            ))
        }
    }

    pub(super) fn repository_payload(&self) -> Result<JsonValue, NativeRepositoryError> {
        let row = RepositoryRow {
            repo_name: self.repo_name().to_string(),
            repo_id: self.repo_id().to_string(),
            default_line: self.default_line.clone(),
            lifecycle_state: "active".to_string(),
            id_namespace_prefix: self.id_namespace_prefix.clone(),
            policy_json: "{}".to_string(),
            created_at: self.created_at.clone(),
            updated_at: self.created_at.clone(),
        };
        let mut payload = repository_json(row);
        let object = payload.as_object_mut().ok_or_else(|| {
            NativeRepositoryError::internal("repository payload must be a JSON object")
        })?;
        object.insert(
            REPOSITORY_PACK_STORAGE_PAYLOAD_FIELD.to_string(),
            binary_repository_pack_storage_payload_json(),
        );
        Ok(payload)
    }

    pub(super) fn unsupported(operation: &str) -> NativeRepositoryError {
        NativeRepositoryError::internal(format!(
            "Binary DB native repository service does not implement {operation} yet"
        ))
    }
}

impl<D> NativeRepositoryService for BinaryDbNativeRepositoryService<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    fn create_repository(
        &self,
        request: RepositoryCreateRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = normalize_required_text(&request.repo_name, "repo_name")?;
        if repo_name != self.repo_name() {
            return Err(NativeRepositoryError::conflict(format!(
                "Binary DB native repository service is configured for repository {}, not {repo_name}",
                self.repo_name()
            )));
        }
        let default_line = normalize_required_text(&request.default_line, "default_line")?;
        if default_line != self.default_line {
            return Err(NativeRepositoryError::bad_request(format!(
                "Binary DB native repository service default line is {}, not {default_line}",
                self.default_line
            )));
        }
        if !self.default_line_exists()? {
            self.content_lines()
                .create_line(&self.default_line, 0, now_timestamp_s())
                .map_err(binary_native_repository_store_error)?;
        }
        self.repository_payload()
    }

    fn list_repositories(&self) -> Result<JsonValue, NativeRepositoryError> {
        Ok(JsonValue::Array(vec![self.repository_payload()?]))
    }

    fn get_repository(&self, repo_name: &str) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        self.repository_payload()
    }

    fn get_repository_by_id(&self, repo_id: &str) -> Result<JsonValue, NativeRepositoryError> {
        let repo_id = normalize_required_text(repo_id, "repo_id")?;
        if repo_id != self.repo_id() {
            return Err(NativeRepositoryError::not_found(format!(
                "Unknown repository id: {repo_id}"
            )));
        }
        self.repository_payload()
    }

    fn list_lines(&self, repo_name: &str) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let lines = self
            .latest_lines_by_name(repo_name)?
            .into_iter()
            .map(|(line_name, value)| binary_line_response(&value, self.repo_id(), &line_name))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(JsonValue::Array(lines))
    }

    fn get_line(
        &self,
        repo_name: &str,
        line_name: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let line_name = normalize_required_text(line_name, "line_name")?;
        let value = self.latest_line_value(repo_name, &line_name)?;
        binary_line_response(&value, self.repo_id(), &line_name)
    }

    fn update_line(
        &self,
        repo_name: &str,
        line_name: &str,
        request: LineUpdateRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let line_name = normalize_required_text(line_name, "line_name")?;
        let head_snapshot_id = normalize_optional_text(request.head_snapshot_id);
        let expected_head_snapshot_id = normalize_optional_text(request.expected_head_snapshot_id);

        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let head_snapshot_index_plus1 = match head_snapshot_id.as_deref() {
            Some(snapshot_id) => self
                .content_snapshots()
                .snapshot_by_id(&read, snapshot_id)
                .map_err(binary_native_repository_store_error)?
                .map(|(index, _)| index.saturating_add(1))
                .ok_or_else(|| {
                    NativeRepositoryError::not_found(format!("Unknown snapshot: {snapshot_id}"))
                })?,
            None => 0,
        };
        let current = self
            .content_lines()
            .line_by_name(&read, &line_name)
            .map_err(binary_native_repository_store_error)?;
        let current_value = match current.as_ref() {
            Some((_, record)) => Some(self.canonical_line_value(&read, &line_name, record)?),
            None => None,
        };
        let current_head = current_value.as_ref().and_then(binary_line_head);
        if let Some(expected) = expected_head_snapshot_id.as_deref() {
            if current_head.as_deref() != Some(expected) {
                return Err(NativeRepositoryError::bad_request(format!(
                    "Line {line_name} head advanced before update: expected {expected:?}, got {:?}",
                    current_head
                )));
            }
        }
        if current
            .as_ref()
            .is_some_and(|(_, record)| record.is_archived())
        {
            return Err(NativeRepositoryError::bad_request(format!(
                "Line {line_name} is archived and cannot move"
            )));
        }

        drop(read);
        let timestamp_s = now_timestamp_s();
        match current {
            Some((_, record)) => {
                self.content_lines()
                    .set_line_head_if_current(
                        &line_name,
                        &record,
                        head_snapshot_index_plus1,
                        timestamp_s,
                    )
                    .map_err(binary_native_repository_store_error)?;
            }
            None => {
                self.content_lines()
                    .create_line(&line_name, head_snapshot_index_plus1, timestamp_s)
                    .map_err(binary_native_repository_store_error)?;
            }
        }
        self.get_line(repo_name, &line_name)
    }

    fn close_line(
        &self,
        repo_name: &str,
        line_name: &str,
        request: LineCloseRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        if request.status.trim() != "archived" {
            return Err(NativeRepositoryError::bad_request(format!(
                "Unsupported line status: {:?}",
                request.status
            )));
        }
        let line_name = normalize_required_text(line_name, "line_name")?;
        if line_name == self.default_line {
            return Err(NativeRepositoryError::bad_request(format!(
                "Default line {line_name} cannot be archived"
            )));
        }
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let (_, current) = self
            .content_lines()
            .line_by_name(&read, &line_name)
            .map_err(binary_native_repository_store_error)?
            .ok_or_else(|| {
                NativeRepositoryError::not_found(format!(
                    "Unknown line {line_name} for repository {repo_name}"
                ))
            })?;
        if current.is_archived() {
            return self.get_line(repo_name, &line_name);
        }
        drop(read);
        self.content_lines()
            .archive_line_if_current(&line_name, &current, now_timestamp_s())
            .map_err(binary_native_repository_store_error)?;
        self.get_line(repo_name, &line_name)
    }

    fn retire_repository(
        &self,
        _repo_name: &str,
        _request: RetireRepositoryRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Err(Self::unsupported("retire_repository"))
    }

    fn snapshot_existence(
        &self,
        repo_name: &str,
        request: SnapshotExistsRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let normalized = request
            .snapshot_ids
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let mut present_set = BTreeSet::new();
        for snapshot_id in &normalized {
            if self
                .content_snapshots()
                .snapshot_by_id(&read, snapshot_id)
                .map_err(binary_native_repository_store_error)?
                .is_some()
            {
                present_set.insert(snapshot_id.clone());
            }
        }
        let present = normalized
            .iter()
            .filter(|value| present_set.contains(value.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let missing = normalized
            .iter()
            .filter(|value| !present_set.contains(value.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        Ok(json!({
            "repo_name": repo_name,
            "checked_snapshots": normalized.len(),
            "present": present,
            "missing": missing,
        }))
    }

    fn zstd_bulk_plan(
        &self,
        repo_name: &str,
        request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let contract = RemoteSyncPlanJson::stateless();
        let plan_request = contract.zstd_bulk_plan_request(&request)?;
        let mut present_snapshot_ids = BTreeSet::new();
        {
            let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
            for snapshot_id in &plan_request.snapshot_ids {
                if self
                    .content_snapshots()
                    .snapshot_by_id(&read, snapshot_id)
                    .map_err(binary_native_repository_store_error)?
                    .is_some()
                {
                    present_snapshot_ids.insert(snapshot_id.clone());
                }
            }
        }
        let mut present_object_pack_ids = BTreeSet::new();
        let mut present_tree_pack_ids = BTreeSet::new();
        {
            let content = self.repository_content();
            for pack_id in &plan_request.object_pack_ids {
                let ready = content
                    .object_pack(pack_id)
                    .map_err(binary_native_repository_store_error)?
                    .is_some();
                if ready {
                    present_object_pack_ids.insert(pack_id.clone());
                }
            }
            for pack_id in &plan_request.tree_pack_ids {
                let ready = content
                    .tree_pack(pack_id)
                    .map_err(binary_native_repository_store_error)?
                    .is_some();
                if ready {
                    present_tree_pack_ids.insert(pack_id.clone());
                }
            }
        }
        Ok(contract.zstd_bulk_plan_response(
            repo_name,
            &plan_request,
            &RemoteSyncZstdBulkPlanPresence {
                present_snapshot_ids,
                present_object_pack_ids,
                present_tree_pack_ids,
            },
        ))
    }

    fn put_zstd_bulk_object_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        self.put_binary_zstd_pack(repo_name, pack_id, pack_bytes, false)
    }

    fn get_zstd_bulk_object_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        validate_pack_id_segment(pack_id)?;
        let value = self.latest_binary_zstd_record(
            repo_name,
            BINARY_ZSTD_OBJECT_PACK_KIND,
            "pack_id",
            pack_id,
            "object pack",
        )?;
        if binary_json_text(&value, "pack_format").as_deref()
            != Some(REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1)
        {
            return Err(NativeRepositoryError::bad_request(format!(
                "Object pack {pack_id} has unsupported pack_format {:?}",
                binary_json_text(&value, "pack_format").unwrap_or_default()
            )));
        }
        let bytes = self.read_zstd_pack_payload(&value, false, pack_id)?;
        let (index, detected_checksum) = zstd_pack_index_from_bytes(&bytes, pack_id, false)?;
        let metadata = binary_zstd_pack_metadata_object(&value, false);
        validate_remote_sync_uploaded_zstd_pack_index_metadata(
            &index,
            &metadata,
            pack_id,
            false,
            detected_checksum.as_deref(),
        )?;
        Ok(bytes)
    }

    fn put_zstd_bulk_tree_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        self.put_binary_zstd_pack(repo_name, pack_id, pack_bytes, true)
    }

    fn get_zstd_bulk_tree_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        validate_pack_id_segment(pack_id)?;
        let value = self.latest_binary_zstd_record(
            repo_name,
            BINARY_ZSTD_TREE_PACK_KIND,
            "pack_id",
            pack_id,
            "tree pack",
        )?;
        if binary_json_text(&value, "pack_format").as_deref()
            != Some(REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1)
        {
            return Err(NativeRepositoryError::bad_request(format!(
                "Tree pack {pack_id} has unsupported pack_format {:?}",
                binary_json_text(&value, "pack_format").unwrap_or_default()
            )));
        }
        let bytes = self.read_zstd_pack_payload(&value, true, pack_id)?;
        let (index, detected_checksum) = zstd_pack_index_from_bytes(&bytes, pack_id, true)?;
        let metadata = binary_zstd_pack_metadata_object(&value, true);
        validate_remote_sync_uploaded_zstd_pack_index_metadata(
            &index,
            &metadata,
            pack_id,
            true,
            detected_checksum.as_deref(),
        )?;
        Ok(bytes)
    }

    fn get_zstd_import_manifest(
        &self,
        repo_name: &str,
        snapshot_id: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let snapshot_id = normalize_required_text(snapshot_id, "snapshot_id")?;
        let snapshot = self.latest_snapshot_value(repo_name, &snapshot_id)?;
        let snapshot_row = binary_zstd_import_manifest_snapshot_row(&snapshot)?;
        let content = self.binary_zstd_import_manifest_content(repo_name, &snapshot)?;
        Ok(
            RemoteSyncZstdImportManifestJson::stateless().zstd_import_manifest_response(
                repo_name,
                &snapshot_id,
                snapshot_row,
                content.object_packs,
                content.tree_packs,
                content.blob_locators,
                content.tree_locators,
            ),
        )
    }

    fn get_zstd_pull_manifest(
        &self,
        repo_name: &str,
        request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let request = RemoteSyncPlanJson::stateless().zstd_pull_manifest_request(&request)?;
        let (snapshots, boundary_snapshot_ids) = self.binary_zstd_pull_manifest_snapshots(
            repo_name,
            &request.head_snapshot_id,
            &request.have_snapshot_ids,
        )?;
        let snapshot_rows = snapshots
            .iter()
            .map(binary_zstd_import_manifest_snapshot_row)
            .collect::<Result<Vec<_>, _>>()?;
        let content =
            self.binary_zstd_import_manifest_content_for_snapshots(repo_name, &snapshots)?;
        Ok(
            RemoteSyncZstdImportManifestJson::stateless().zstd_pull_manifest_response(
                repo_name,
                &request.head_snapshot_id,
                boundary_snapshot_ids,
                snapshot_rows,
                content.object_packs,
                content.tree_packs,
                content.blob_locators,
                content.tree_locators,
            ),
        )
    }

    fn commit_zstd_bulk(
        &self,
        repo_name: &str,
        request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        #[cfg(feature = "perfetto-tracing")]
        let _trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.remote_sync.zstd_commit");
        let started = std::time::Instant::now();
        let commit_once = || -> Result<JsonValue, NativeRepositoryError> {
            #[cfg(feature = "perfetto-tracing")]
            let ensure_repository_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.ensure_repository",
            );
            self.ensure_repository(repo_name)?;
            #[cfg(feature = "perfetto-tracing")]
            drop(ensure_repository_trace);

            #[cfg(feature = "perfetto-tracing")]
            let decode_validate_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.decode_validate",
            );
            let contract = RemoteSyncCommitJson::stateless();
            let request_object = contract.zstd_bulk_commit_object(&request)?;
            let object_pack_values =
                contract.zstd_bulk_commit_values(request_object, "object_packs")?;
            let tree_pack_values =
                contract.zstd_bulk_commit_values(request_object, "tree_packs")?;
            let blob_locator_values =
                contract.zstd_bulk_commit_values(request_object, "blob_locators")?;
            let tree_locator_values =
                contract.zstd_bulk_commit_values(request_object, "tree_locators")?;
            let snapshot_values = contract.zstd_bulk_commit_values(request_object, "snapshots")?;
            let requested_mutation_count = object_pack_values
                .len()
                .saturating_add(tree_pack_values.len())
                .saturating_add(blob_locator_values.len())
                .saturating_add(tree_locator_values.len())
                .saturating_add(snapshot_values.len())
                .saturating_add(usize::from(request_object.contains_key("line_update")));
            ensure_zstd_bulk_mutation_capacity(requested_mutation_count)?;
            let line_update = contract.line_update_request(request_object)?;
            if let Some((line_name, request)) = line_update.as_ref() {
                let current_line = self.latest_line_value_optional(repo_name, line_name)?;
                require_remote_sync_line_update_authority(
                    &self.default_line,
                    line_name,
                    current_line.as_ref().and_then(binary_line_head).as_deref(),
                    request.head_snapshot_id.as_deref(),
                )?;
            }
            #[cfg(feature = "perfetto-tracing")]
            drop(decode_validate_trace);

            #[cfg(feature = "perfetto-tracing")]
            let prefetch_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.prefetch_read_set",
            );
            let read_set = self.prefetch_binary_zstd_commit_read_set(
                repo_name,
                &object_pack_values,
                &tree_pack_values,
                &blob_locator_values,
                &tree_locator_values,
                &snapshot_values,
                line_update.as_ref(),
            )?;
            #[cfg(feature = "perfetto-tracing")]
            drop(prefetch_trace);

            #[cfg(feature = "perfetto-tracing")]
            let prepare_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.prepare",
            );
            let mut object_pack_indexes = BTreeMap::new();
            let mut tree_pack_indexes = BTreeMap::new();
            let mut request_identities = BTreeSet::new();
            let mut metadata_mutations = Vec::new();
            let mut snapshot_mutations = Vec::new();
            let mut upserted_object_packs = 0_i64;
            let mut skipped_object_packs = 0_i64;
            #[cfg(feature = "perfetto-tracing")]
            let prepare_object_packs_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.prepare.object_packs",
            );
            for value in object_pack_values {
                let object = json_object(value, "object_packs[]")?;
                let pack_id = required_json_text(object, "pack_id")
                    .map_err(NativeRepositoryError::bad_request)?;
                if !request_identities.insert(format!("{BINARY_ZSTD_OBJECT_PACK_KIND}:{pack_id}")) {
                    return Err(NativeRepositoryError::bad_request(format!(
                        "duplicate object pack {pack_id} in zstd bulk request"
                    )));
                }
                let (index, mutation) = self
                    .prepare_binary_zstd_pack_from_commit(repo_name, &object, false, &read_set)?;
                object_pack_indexes.insert(pack_id, index);
                if let Some(mutation) = mutation {
                    metadata_mutations.push(mutation);
                    upserted_object_packs += 1;
                } else {
                    skipped_object_packs += 1;
                }
            }
            #[cfg(feature = "perfetto-tracing")]
            drop(prepare_object_packs_trace);

            let mut upserted_tree_packs = 0_i64;
            let mut skipped_tree_packs = 0_i64;
            #[cfg(feature = "perfetto-tracing")]
            let prepare_tree_packs_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.prepare.tree_packs",
            );
            for value in tree_pack_values {
                let object = json_object(value, "tree_packs[]")?;
                let pack_id = required_json_text(object, "pack_id")
                    .map_err(NativeRepositoryError::bad_request)?;
                if !request_identities.insert(format!("{BINARY_ZSTD_TREE_PACK_KIND}:{pack_id}")) {
                    return Err(NativeRepositoryError::bad_request(format!(
                        "duplicate tree pack {pack_id} in zstd bulk request"
                    )));
                }
                let (index, mutation) =
                    self.prepare_binary_zstd_pack_from_commit(repo_name, &object, true, &read_set)?;
                tree_pack_indexes.insert(pack_id, index);
                if let Some(mutation) = mutation {
                    metadata_mutations.push(mutation);
                    upserted_tree_packs += 1;
                } else {
                    skipped_tree_packs += 1;
                }
            }
            #[cfg(feature = "perfetto-tracing")]
            drop(prepare_tree_packs_trace);

            let mut upserted_blobs = 0_i64;
            #[cfg(feature = "perfetto-tracing")]
            let prepare_blob_locators_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.prepare.blob_locators",
            );
            for value in blob_locator_values {
                let object = json_object(value, "blob_locators[]")?;
                let blob_id = required_json_text(object, "blob_id")
                    .map_err(NativeRepositoryError::bad_request)?;
                if !request_identities.insert(format!("{BINARY_ZSTD_BLOB_LOCATOR_KIND}:{blob_id}"))
                {
                    return Err(NativeRepositoryError::bad_request(format!(
                        "duplicate blob locator {blob_id} in zstd bulk request"
                    )));
                }
                if let Some(mutation) = self.prepare_binary_zstd_blob_locator(
                    repo_name,
                    &object,
                    &mut object_pack_indexes,
                    &read_set,
                )? {
                    metadata_mutations.push(mutation);
                    upserted_blobs += 1;
                }
            }
            #[cfg(feature = "perfetto-tracing")]
            drop(prepare_blob_locators_trace);

            let mut upserted_trees = 0_i64;
            #[cfg(feature = "perfetto-tracing")]
            let prepare_tree_locators_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.prepare.tree_locators",
            );
            for value in tree_locator_values {
                let object = json_object(value, "tree_locators[]")?;
                let tree_id = required_json_text(object, "tree_id")
                    .map_err(NativeRepositoryError::bad_request)?;
                if !request_identities.insert(format!("{BINARY_ZSTD_TREE_LOCATOR_KIND}:{tree_id}"))
                {
                    return Err(NativeRepositoryError::bad_request(format!(
                        "duplicate tree locator {tree_id} in zstd bulk request"
                    )));
                }
                if let Some(mutation) = self.prepare_binary_zstd_tree_locator(
                    repo_name,
                    &object,
                    &mut tree_pack_indexes,
                    &read_set,
                )? {
                    metadata_mutations.push(mutation);
                    upserted_trees += 1;
                }
            }
            #[cfg(feature = "perfetto-tracing")]
            drop(prepare_tree_locators_trace);

            let mut incoming_seen = BTreeSet::new();
            let mut upserted_snapshots = 0_i64;
            let mut skipped_snapshots = 0_i64;
            #[cfg(feature = "perfetto-tracing")]
            let prepare_snapshots_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.prepare.snapshots",
            );
            for value in snapshot_values {
                let object = json_object(value, "snapshots[]")?;
                let snapshot_id = required_json_text(object, "snapshot_id")
                    .map_err(NativeRepositoryError::bad_request)?;
                if !request_identities.insert(format!("snapshot:{snapshot_id}")) {
                    return Err(NativeRepositoryError::bad_request(format!(
                        "duplicate snapshot {snapshot_id} in zstd bulk request"
                    )));
                }
                if let Some(mutation) = self.prepare_binary_zstd_snapshot(
                    repo_name,
                    &object,
                    &mut tree_pack_indexes,
                    &incoming_seen,
                    &read_set,
                )? {
                    snapshot_mutations.push(mutation);
                    upserted_snapshots += 1;
                } else {
                    skipped_snapshots += 1;
                }
                incoming_seen.insert(snapshot_id);
            }
            #[cfg(feature = "perfetto-tracing")]
            drop(prepare_snapshots_trace);

            #[cfg(feature = "perfetto-tracing")]
            let prepare_line_guard_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.prepare.line_guard",
            );
            let line_write = match line_update.as_ref() {
                Some((line_name, line_request)) => {
                    let target = normalize_optional_text(line_request.head_snapshot_id.clone())
                        .ok_or_else(|| {
                            NativeRepositoryError::bad_request(
                                "zstd bulk line update requires head_snapshot_id",
                            )
                        })?;
                    if !incoming_seen.contains(&target) && read_set.snapshot(&target)?.is_none() {
                        return Err(NativeRepositoryError::not_found(format!(
                            "Unknown snapshot: {target}"
                        )));
                    }
                    let current_line = read_set.line(line_name)?;
                    let current_head = current_line.and_then(binary_line_head);
                    require_remote_sync_line_update_authority(
                        &self.default_line,
                        line_name,
                        current_head.as_deref(),
                        Some(&target),
                    )?;
                    if current_head.as_deref() == Some(target.as_str()) {
                        None
                    } else {
                        if let Some(expected) =
                            normalize_optional_text(line_request.expected_head_snapshot_id.clone())
                        {
                            if current_head.as_deref() != Some(expected.as_str()) {
                                return Err(NativeRepositoryError::conflict(format!(
                                    "STALE_REMOTE_SYNC_LINE: {line_name} expected {expected:?}, current {current_head:?}"
                                )));
                            }
                        }
                        Some(if current_line.is_some() {
                            ServerBinaryRemoteSyncLineWrite::Update
                        } else {
                            ServerBinaryRemoteSyncLineWrite::Create
                        })
                    }
                }
                None => None,
            };
            #[cfg(feature = "perfetto-tracing")]
            drop(prepare_line_guard_trace);
            #[cfg(feature = "perfetto-tracing")]
            drop(prepare_trace);

            #[cfg(feature = "perfetto-tracing")]
            let writer_admission_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.writer_admission",
            );
            let mut tx = BinaryDbWriteTxn::begin_serving(
                &self.db,
                BinaryDbCommandScope::ServerRemoteSyncCommit,
            )
            .map_err(binary_native_repository_store_error)?;
            #[cfg(feature = "perfetto-tracing")]
            drop(writer_admission_trace);

            #[cfg(feature = "perfetto-tracing")]
            let transaction_mutation_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.transaction_mutation",
            );
            let mut object_packs = Vec::new();
            let mut tree_packs = Vec::new();
            let mut blob_locators = Vec::new();
            let mut tree_locators = Vec::new();
            for mutation in metadata_mutations {
                match mutation.kind {
                    BINARY_ZSTD_OBJECT_PACK_KIND => object_packs.push(mutation.value),
                    BINARY_ZSTD_TREE_PACK_KIND => tree_packs.push(mutation.value),
                    BINARY_ZSTD_BLOB_LOCATOR_KIND => blob_locators.push(mutation.value),
                    BINARY_ZSTD_TREE_LOCATOR_KIND => tree_locators.push(mutation.value),
                    other => {
                        return Err(NativeRepositoryError::internal(format!(
                            "unsupported typed Binary DB content mutation {other}"
                        )))
                    }
                }
            }
            let content = self.repository_content();
            #[cfg(feature = "perfetto-tracing")]
            let prepare_write_set_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.transaction_prepare_write_set",
            );
            content
                .prepare_remote_sync_write_set(
                    &mut tx,
                    !object_packs.is_empty(),
                    !tree_packs.is_empty(),
                    !snapshot_mutations.is_empty(),
                    line_write,
                )
                .map_err(binary_native_repository_store_error)?;
            #[cfg(feature = "perfetto-tracing")]
            drop(prepare_write_set_trace);

            #[cfg(feature = "perfetto-tracing")]
            let content_append_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.transaction_mutation.content_append",
            );
            for pack in &object_packs {
                let pack_id = binary_json_text(pack, "pack_id").ok_or_else(|| {
                    NativeRepositoryError::internal("object pack mutation is missing pack_id")
                })?;
                let locators = blob_locators
                    .iter()
                    .filter(|locator| {
                        binary_json_text(locator, "pack_id").as_deref() == Some(pack_id.as_str())
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                content
                    .append_object_pack_in_tx(&mut tx, pack, &locators)
                    .map_err(binary_native_repository_store_error)?;
            }
            for pack in &tree_packs {
                let pack_id = binary_json_text(pack, "pack_id").ok_or_else(|| {
                    NativeRepositoryError::internal("tree pack mutation is missing pack_id")
                })?;
                let locators = tree_locators
                    .iter()
                    .filter(|locator| {
                        binary_json_text(locator, "tree_pack_id").as_deref()
                            == Some(pack_id.as_str())
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                content
                    .append_tree_pack_in_tx(&mut tx, pack, &locators)
                    .map_err(binary_native_repository_store_error)?;
            }
            #[cfg(feature = "perfetto-tracing")]
            drop(content_append_trace);

            #[cfg(feature = "perfetto-tracing")]
            let snapshot_append_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.transaction_mutation.snapshot_append",
            );
            for mutation in snapshot_mutations {
                self.upsert_snapshot_value_in_tx(
                    &mut tx,
                    repo_name,
                    &mutation.snapshot_id,
                    &mutation.value,
                )?;
            }
            #[cfg(feature = "perfetto-tracing")]
            drop(snapshot_append_trace);

            #[cfg(feature = "perfetto-tracing")]
            let line_update_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.transaction_mutation.line_update",
            );
            let line_head_updated_after_ingest = match line_update.as_ref() {
                Some((line_name, line_request)) => {
                    self.update_line_in_tx(&mut tx, repo_name, line_name, line_request)?
                }
                None => false,
            };
            #[cfg(feature = "perfetto-tracing")]
            drop(line_update_trace);
            #[cfg(feature = "perfetto-tracing")]
            drop(transaction_mutation_trace);

            #[cfg(feature = "perfetto-tracing")]
            let transaction_commit_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.transaction_commit",
            );
            tx.commit().map_err(binary_native_repository_store_error)?;
            #[cfg(feature = "perfetto-tracing")]
            drop(transaction_commit_trace);

            #[cfg(feature = "perfetto-tracing")]
            let response_line_read_trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.remote_sync.zstd_commit.response_line_read",
            );
            let remote_line = match line_update {
                Some((line_name, _)) => self.get_line(repo_name, &line_name)?,
                None => JsonValue::Null,
            };
            #[cfg(feature = "perfetto-tracing")]
            drop(response_line_read_trace);
            Ok(
                contract.zstd_bulk_commit_response(RemoteSyncZstdBulkCommitResponse {
                    repo_name: repo_name.to_string(),
                    repo_id: self.repo_id().to_string(),
                    upserted_object_packs,
                    skipped_object_packs,
                    upserted_tree_packs,
                    skipped_tree_packs,
                    upserted_blobs,
                    upserted_trees,
                    upserted_snapshots,
                    skipped_snapshots,
                    remote_line,
                    line_head_updated_after_ingest,
                }),
            )
        };
        loop {
            match commit_once() {
                Err(error)
                    if error.kind == NativeRepositoryErrorKind::ServiceUnavailable
                        && error.message.contains("Binary DB writer is active")
                        && started.elapsed() < std::time::Duration::from_secs(30) =>
                {
                    #[cfg(feature = "perfetto-tracing")]
                    let _retry_sleep_trace = crate::perfetto_trace::PerfettoRange::new(
                        "ait.server.remote_sync.zstd_commit.retry_sleep",
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                result => return result,
            }
        }
    }

    fn export_snapshot(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        query: SnapshotExportQuery,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let snapshot_id = normalize_required_text(snapshot_id, "snapshot_id")?;
        let value = self.latest_snapshot_value(repo_name, &snapshot_id)?;
        let files = self.snapshot_files_for_value(&value)?;
        binary_snapshot_export_json(&value, files, &query)
    }

    fn materialize_snapshot(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let snapshot_id = normalize_required_text(snapshot_id, "snapshot_id")?;
        self.materialize_canonical_snapshot(repo_name, &snapshot_id, destination, None)
    }

    fn materialize_snapshot_paths(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
        relative_paths: &[PathBuf],
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let snapshot_id = normalize_required_text(snapshot_id, "snapshot_id")?;
        self.materialize_canonical_snapshot(
            repo_name,
            &snapshot_id,
            destination,
            Some(relative_paths),
        )
    }

    fn materialize_snapshot_manifest_entries(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
        entries: &[SnapshotManifestFileEntry],
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let snapshot_id = normalize_required_text(snapshot_id, "snapshot_id")?;
        self.materialize_canonical_manifest_entries(repo_name, &snapshot_id, destination, entries)
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::*;

    #[test]
    fn zstd_bulk_capacity_admits_release_history_and_retains_upper_bound() {
        assert!(ensure_zstd_bulk_mutation_capacity(22_016).is_ok());
        assert!(ensure_zstd_bulk_mutation_capacity(MAX_ZSTD_BULK_MUTATIONS).is_ok());

        let error = ensure_zstd_bulk_mutation_capacity(MAX_ZSTD_BULK_MUTATIONS + 1)
            .expect_err("a write set above the bounded maximum must fail closed");
        assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
        assert_eq!(
            error.message,
            "zstd bulk write set has 100001 mutations, maximum is 100000"
        );
    }
}
