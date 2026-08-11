use super::*;

impl<D> BinaryDbNativeRepositoryService<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub(in super::super) fn current_lines(
        &self,
        repo_name: &str,
    ) -> Result<Vec<JsonValue>, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let mut values = Vec::new();
        for (_, line_name, record) in self
            .content_lines()
            .all_lines(&read)
            .map_err(binary_native_repository_store_error)?
        {
            values.push(self.canonical_line_value(&read, &line_name, &record)?);
        }
        Ok(values)
    }

    pub(in super::super) fn latest_lines_by_name(
        &self,
        repo_name: &str,
    ) -> Result<BTreeMap<String, JsonValue>, NativeRepositoryError> {
        let mut latest = BTreeMap::new();
        for value in self.current_lines(repo_name)? {
            if let Some(line_name) = binary_line_name(&value, repo_name) {
                latest.insert(line_name, value);
            }
        }
        Ok(latest)
    }

    pub(in super::super) fn latest_line_value(
        &self,
        repo_name: &str,
        line_name: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.latest_line_value_optional(repo_name, line_name)?
            .ok_or_else(|| {
                NativeRepositoryError::not_found(format!(
                    "Unknown line {line_name} for repository {repo_name}"
                ))
            })
    }

    pub(in super::super) fn latest_line_value_optional(
        &self,
        repo_name: &str,
        line_name: &str,
    ) -> Result<Option<JsonValue>, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        self.latest_line_value_optional_with_read(&read, repo_name, line_name)
    }

    pub(in super::super) fn latest_line_value_optional_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        repo_name: &str,
        line_name: &str,
    ) -> Result<Option<JsonValue>, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let Some((_, record)) = self
            .content_lines()
            .line_by_name(read, line_name)
            .map_err(binary_native_repository_store_error)?
        else {
            return Ok(None);
        };
        Ok(Some(self.canonical_line_value(read, line_name, &record)?))
    }

    pub(in super::super) fn canonical_line_value(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        line_name: &str,
        record: &ServerBinaryLineRecord,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let head_snapshot_id = match record.head_snapshot_index() {
            Some(index) => {
                let raw = read
                    .read_record(
                        ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
                        index,
                    )
                    .map_err(binary_native_repository_store_error)?;
                let snapshot =
                    ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::decode_record(
                        &raw,
                    )
                    .map_err(binary_native_repository_store_error)?;
                Some(server_snapshot_id_from_hash48(snapshot.snapshot_hash48))
            }
            None => None,
        };
        let archived_at = if record.is_archived() {
            JsonValue::String(timestamp_string(record.archived_at_s)?)
        } else {
            JsonValue::Null
        };
        Ok(binary_line_payload(
            self.repo_name(),
            self.repo_id(),
            line_name,
            head_snapshot_id.as_deref(),
            if record.is_archived() {
                "archived"
            } else {
                "active"
            },
            archived_at,
            JsonValue::String(timestamp_string(record.created_at_s)?),
            timestamp_string(record.updated_at_s)?,
        ))
    }

    pub(in super::super) fn update_line_in_tx<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        repo_name: &str,
        line_name: &str,
        request: &LineUpdateRequest,
    ) -> Result<bool, NativeRepositoryError>
    where
        F: BinaryDbFsyncPolicy,
    {
        self.ensure_repository(repo_name)?;
        let line_name = normalize_required_text(line_name, "line_name")?;
        let target_snapshot_id = normalize_optional_text(request.head_snapshot_id.clone())
            .ok_or_else(|| {
                NativeRepositoryError::bad_request(
                    "zstd bulk line update requires head_snapshot_id",
                )
            })?;
        let (target_snapshot_index, _) = self
            .content_snapshots()
            .snapshot_by_id_in_write(tx, &target_snapshot_id)
            .map_err(binary_native_repository_store_error)?
            .ok_or_else(|| {
                NativeRepositoryError::not_found(format!("Unknown snapshot: {target_snapshot_id}"))
            })?;
        let current = self
            .content_lines()
            .line_by_name_in_write(tx, &line_name)
            .map_err(binary_native_repository_store_error)?;
        let Some((_, current)) = current else {
            if let Some(expected) =
                normalize_optional_text(request.expected_head_snapshot_id.clone())
            {
                return Err(NativeRepositoryError::conflict(format!(
                    "STALE_REMOTE_SYNC_LINE: {line_name} expected {expected:?}, current None"
                )));
            }
            let head_snapshot_index_plus1 =
                target_snapshot_index.checked_add(1).ok_or_else(|| {
                    NativeRepositoryError::internal("canonical snapshot index exceeds u32")
                })?;
            self.content_lines()
                .create_line_in_tx(tx, &line_name, head_snapshot_index_plus1, now_timestamp_s())
                .map_err(binary_native_repository_store_error)?;
            return Ok(true);
        };
        if current.head_snapshot_index() == Some(target_snapshot_index) {
            return Ok(false);
        }
        let current_head = current
            .head_snapshot_index()
            .map(|index| {
                self.content_snapshots()
                    .snapshot_id_at_in_write(tx, index)
                    .map_err(binary_native_repository_store_error)
            })
            .transpose()?;
        if let Some(expected) =
            normalize_optional_text(request.expected_head_snapshot_id.clone()).as_deref()
        {
            if current_head.as_deref() != Some(expected) {
                return Err(NativeRepositoryError::conflict(format!(
                    "STALE_REMOTE_SYNC_LINE: {line_name} expected {expected:?}, current {current_head:?}"
                )));
            }
        }
        self.content_lines()
            .set_line_head_in_tx(
                tx,
                &line_name,
                current.head_snapshot_index(),
                target_snapshot_index,
                now_timestamp_s(),
            )
            .map_err(binary_native_repository_store_error)?;
        Ok(true)
    }

    pub(in super::super) fn default_line_exists(&self) -> Result<bool, NativeRepositoryError> {
        Ok(self
            .latest_line_value_optional(self.repo_name(), &self.default_line)?
            .is_some())
    }
}
