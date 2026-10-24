//! Task-owned local successful-finish receipts and bounded point projections.
use super::*;

impl<B: BinaryDb, const W: u32> BinaryDbWorkflowStore<B, W> {
    pub(super) fn point_task_index<A: WorkflowReadAccess>(
        &self,
        access: &A,
        id: &str,
    ) -> StoreResult<u32> {
        let index = id
            .rsplit_once('-')
            .and_then(|(_, n)| n.parse::<u32>().ok())
            .and_then(|n| n.checked_sub(1))
            .ok_or("Invalid Task identity")?;
        if self.local_task_id(index)? != id
            || index >= access.record_count(Self::task_record_file())?
        {
            return Err(format!("Unknown Task: {id}").into());
        }
        Ok(index)
    }

    pub(super) fn receipt_at<A: WorkflowReadAccess>(
        &self,
        access: &A,
        task: u32,
    ) -> StoreResult<Option<LocalLandRecord>> {
        let count = access
            .record_count(self.land_file_for_read())
            .map_err(|error| {
                if !self.legacy_land_reader
                    && self.db.layout_id(Self::land_record_file()).ok() == Some(1)
                {
                    crate::binary_db::BinaryDbError::invalid_domain_data(
                        "Local finish receipt upgrade must complete at native repository admission",
                    )
                } else {
                    error
                }
            })?;
        let head =
            TaskLandIndexRecord::decode(&access.read_record(Self::task_land_index_file(), task)?)?;
        let Some(index) = head.latest_land_index_plus1.checked_sub(1) else {
            return Ok(None);
        };
        if index >= count {
            return Err("Task receipt pointer is out of bounds".into());
        }
        let receipt = self.read_land(access, index)?;
        if receipt.task_index != task {
            return Err("Task receipt pointer has a different owner".into());
        }
        Ok(Some(receipt))
    }

    pub(super) fn project_receipt<A: WorkflowReadAccess>(
        &self,
        access: &A,
        task: u32,
    ) -> StoreResult<JsonValue> {
        let Some(receipt) = self.receipt_at(access, task)? else {
            return Ok(JsonValue::Null);
        };
        let task_id = self.local_task_id(task)?;
        let change_ref = receipt
            .change_ordinal_plus1
            .checked_sub(1)
            .map(|ordinal| format!("{task_id}/{}", render_change_id(ordinal)));
        Ok(json!({
            "task_id":task_id, "change_ref":change_ref,
            "land_ordinal":receipt.land_ordinal,
            "target_line":line_name_at(access,receipt.target_line_index_plus1-1)?,
            "pre_land_target_snapshot_id":receipt.pre_land_target_snapshot_index_plus1.checked_sub(1).map(|i|snapshot_id_at(access,i)).transpose()?,
            "landed_snapshot_id":snapshot_id_at(access,receipt.landed_snapshot_index_plus1-1)?,
            "submitted_at":timestamp_json(receipt.submitted_at_s)?,
            "updated_at":timestamp_json(receipt.updated_at_s)?,
            "status":"succeeded"
        }))
    }

    pub fn local_task_receipt(&self, task_id: &str) -> PlanStoreResult<Option<JsonValue>> {
        let read = BinaryDbReadTxn::new(&self.db);
        let index = self
            .point_task_index(&read, task_id)
            .map_err(storage_error)?;
        let value = self.project_receipt(&read, index).map_err(storage_error)?;
        Ok((!value.is_null()).then_some(value))
    }

    pub fn changes_for_task(&self, task_id: &str) -> PlanStoreResult<Vec<JsonValue>> {
        let read = BinaryDbReadTxn::new(&self.db);
        let task = self
            .point_task_index(&read, task_id)
            .map_err(storage_error)?;
        self.change_rows_for_task(&read, task)
            .map_err(storage_error)
    }

    /// Authoring and cancellation must not replace an interrupted apply intent.
    pub fn guard_no_pending_local_finish(
        &self,
        task_id: &str,
        cursor: &std::path::Path,
    ) -> PlanStoreResult<()> {
        if let Some(pending) = read_pending(cursor).map_err(PlanStoreError::Storage)? {
            let read = BinaryDbReadTxn::new(&self.db);
            let task = self
                .point_task_index(&read, task_id)
                .map_err(storage_error)?;
            if pending.task != task {
                return Err(PlanStoreError::Invalid(
                    "Task finish cursor has another owner".into(),
                ));
            }
            return Err(PlanStoreError::Invalid(format!("Task {task_id} has a pending finish. Resume `ait task finish {task_id} --local` before authoring or cancellation; preserve its successful apply evidence.")));
        }
        Ok(())
    }

    fn change_rows_for_task<A: WorkflowReadAccess>(
        &self,
        access: &A,
        task: u32,
    ) -> StoreResult<Vec<JsonValue>> {
        let head = TaskChangeIndexRecord::decode(
            &access.read_record(Self::task_change_index_file(), task)?,
        )?;
        if head.change_count > 64 {
            return Err("Task Change capacity exceeded".into());
        }
        let mut cursor = head.latest_change_index_plus1;
        let mut result = Vec::new();
        for _ in 0..head.change_count {
            let index = cursor
                .checked_sub(1)
                .ok_or("Task Change chain ends early")?;
            let change =
                LocalChangeRecord::decode(&access.read_record(Self::change_record_file(), index)?)?;
            if change.task_index != task {
                return Err("Task Change chain has a different owner".into());
            }
            result.push(self.project_change(access, index, &change)?);
            cursor = change.previous_change_index_plus1;
        }
        if cursor != 0 {
            return Err("Task Change chain has a cycle or incorrect count".into());
        }
        result.reverse();
        Ok(result)
    }

    pub(super) fn point_task(&self, task_id: &str) -> PlanStoreResult<JsonValue> {
        let read = BinaryDbReadTxn::new(&self.db);
        let index = self
            .point_task_index(&read, task_id)
            .map_err(storage_error)?;
        let record = self.read_task_record(&read, index).map_err(storage_error)?;
        self.project_task(&read, index, &record)
            .map_err(storage_error)
    }
}

impl<B: BinaryDb, const W: u32> BinaryDbWorkflowStore<B, W> {
    /// Apply the complete frozen Task once. The optional Change is provenance only.
    /// The caller's managed worktree cursor preserves a pending apply across death.
    #[allow(clippy::too_many_arguments)]
    pub fn finish_local_task(
        &self,
        task_id: &str,
        candidate: &str,
        target: &str,
        expected_pre: Option<&str>,
        change_ref: Option<&str>,
        cursor: Option<&std::path::Path>,
    ) -> PlanStoreResult<JsonValue> {
        if self.legacy_land_reader {
            return Err(PlanStoreError::Invalid(
                "An upgrade source is read-only".into(),
            ));
        }
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let task = self
            .point_task_index(&write, task_id)
            .map_err(storage_error)?;
        let record = self.read_task_record(&write, task).map_err(storage_error)?;
        let line_index = line_index_by_name(&write, target)
            .map_err(storage_error)?
            .ok_or_else(|| unknown_row("Line", target))?;
        let candidate_index = snapshot_index_by_id(&write, candidate)
            .map_err(storage_error)?
            .ok_or_else(|| unknown_row("Snapshot", candidate))?;
        // The receipt is the commit authority. Scan only receipt records under the
        // write lock so an interrupted append cannot be hidden by an old index.
        let mut receipt = None;
        for index in 0..write
            .record_count(Self::land_record_file())
            .map_err(storage_error)?
        {
            let row = self.read_land(&write, index).map_err(storage_error)?;
            if row.task_index == task && receipt.replace((index, row)).is_some() {
                return Err(PlanStoreError::Invalid(
                    "Task owns multiple successful receipts".into(),
                ));
            }
        }
        if let Some((index, row)) = receipt {
            if row.landed_snapshot_index_plus1 != candidate_index + 1
                || row.target_line_index_plus1 != line_index + 1
            {
                return Err(PlanStoreError::Invalid("Task already finished with a different candidate or target; its receipt cannot be replaced".into()));
            }
            self.repair_receipt_indexes(&mut write, index, &row)
                .map_err(storage_error)?;
            write.commit().map_err(storage_error)?;
            self.close_receipted_task(task_id)?;
            return self
                .local_task_receipt(task_id)?
                .ok_or_else(|| unknown_row("Task receipt", task_id));
        }
        if record.is_terminal() || record.is_published() {
            return Err(PlanStoreError::Invalid(
                "A terminal or published Task cannot receive a new local finish".into(),
            ));
        }
        let ancestry = self
            .ancestor_indices(&write, candidate_index)
            .map_err(storage_error)?;
        if let Some(latest) = self
            .required_task_snapshot_index(&write, task)
            .map_err(storage_error)?
        {
            if !ancestry.contains(&latest) {
                let selected = snapshot_id_at(&write, latest).map_err(storage_error)?;
                let line = BinaryLineCodec::<1>::decode_record(
                    &write
                        .read_record(BinaryLineCodec::<1>::record_file(), line_index)
                        .map_err(storage_error)?,
                )
                .map_err(storage_error)?;
                let detail = if line.head_snapshot_index_plus1 == candidate_index + 1 {
                    "The current Line already matches the target; inspect and restore the complete reviewed Task work before finishing."
                } else {
                    "Restore the complete reviewed Task candidate before finishing."
                };
                return Err(PlanStoreError::Invalid(format!("Task {task_id} selected revision `{selected}` is not incorporated in final Snapshot `{candidate}`. {detail}")));
            }
        }
        let changes = self
            .change_rows_for_task(&write, task)
            .map_err(storage_error)?;
        let mut associated = None;
        for change in &changes {
            let reference =
                required_row_text(change, "change_ref", "Change").map_err(storage_error)?;
            let ordinal = change["change_seq"]
                .as_u64()
                .ok_or("Invalid Change ordinal")
                .map_err(|e| PlanStoreError::Invalid(e.into()))? as u8;
            let index = self
                .change_index_in_task(&write, task, ordinal - 1)
                .map_err(storage_error)?;
            let state = string_value(change, "status").unwrap_or_default();
            if matches!(state.as_str(), "draft" | "active" | "review") {
                match self
                    .change_snapshot_index(&write, index)
                    .map_err(storage_error)?
                {
                    Some(head) if !ancestry.contains(&head) => {
                        return Err(PlanStoreError::Invalid(format!(
                            "Final Snapshot does not incorporate revision {reference}"
                        )))
                    }
                    None => {
                        return Err(PlanStoreError::Invalid(format!(
                            "Revision {reference} has no frozen Snapshot; resolve it before finish"
                        )))
                    }
                    _ => {}
                }
            }
            if change_ref == Some(reference.as_str()) {
                associated = Some((index, ordinal));
            }
        }
        if change_ref.is_some() && associated.is_none() {
            return Err(PlanStoreError::Invalid(
                "Receipt revision does not belong to this Task".into(),
            ));
        }
        let mut line = BinaryLineCodec::<1>::decode_record(
            &write
                .read_record(BinaryLineCodec::<1>::record_file(), line_index)
                .map_err(storage_error)?,
        )
        .map_err(storage_error)?;
        if line.is_archived() || line.is_tombstone() {
            return Err(PlanStoreError::Invalid(
                "Finish target is archived or deleted".into(),
            ));
        }
        let supplied_pre = expected_pre
            .map(|id| snapshot_index_by_id(&write, id))
            .transpose()
            .map_err(storage_error)?
            .flatten()
            .map(|i| i + 1)
            .unwrap_or(0);
        if expected_pre.is_some() && supplied_pre == 0 {
            return Err(PlanStoreError::Invalid(
                "Expected target Snapshot is missing".into(),
            ));
        }
        let pending = cursor
            .map(read_pending)
            .transpose()
            .map_err(PlanStoreError::Storage)?
            .flatten();
        let pre = if let Some(pending) = pending {
            if pending.task != task
                || pending.candidate != candidate_index + 1
                || pending.change != associated.map_or(0, |v| v.0 + 1)
            {
                return Err(PlanStoreError::Invalid(
                    "Pending Task finish names a different owner, candidate or revision".into(),
                ));
            }
            pending.pre
        } else {
            supplied_pre
        };
        let already_applied = line.head_snapshot_index_plus1 == candidate_index + 1;
        let recovering_applied = if pending.is_some() && !already_applied {
            line.head_snapshot_index()
                .map(|head| self.ancestor_indices(&write, head))
                .transpose()
                .map_err(storage_error)?
                .is_some_and(|ancestors| ancestors.contains(&candidate_index))
        } else {
            false
        };
        if !already_applied && !recovering_applied && line.head_snapshot_index_plus1 != pre {
            return Err(PlanStoreError::Invalid(
                "Finish target changed; rebase the frozen candidate before retrying".into(),
            ));
        }
        if pre != 0 && !ancestry.contains(&(pre - 1)) {
            return Err(PlanStoreError::Invalid(
                "Finish candidate does not preserve the expected target history".into(),
            ));
        }
        if let Some(path) = cursor {
            write_pending(
                path,
                Pending {
                    task,
                    pre,
                    candidate: candidate_index + 1,
                    change: associated.map_or(0, |v| v.0 + 1),
                },
            )
            .map_err(PlanStoreError::Storage)?;
        }
        if !already_applied && !recovering_applied {
            line.head_snapshot_index_plus1 = candidate_index + 1;
            line.updated_at_s = now_s()?;
            write
                .overwrite_record(
                    BinaryLineCodec::<1>::record_file(),
                    line_index,
                    &BinaryLineCodec::<1>::encode_record(&line).map_err(storage_error)?,
                )
                .map_err(storage_error)?;
            self.db
                .sync_file(&self.db.authority_root().as_path().join("line.bin"))
                .map_err(storage_error)?;
        }
        let now = now_s()?;
        let row = LocalLandRecord {
            land_meta: 2 | LAND_META_HAS_LANDED | if pre != 0 { LAND_META_HAS_PRE_LAND } else { 0 },
            land_ordinal: 0,
            change_ordinal_plus1: associated.map_or(0, |v| v.1),
            failure_kind: 0,
            task_index: task,
            previous_task_land_index_plus1: 0,
            previous_change_land_index_plus1: 0,
            pre_land_target_snapshot_index_plus1: pre,
            landed_snapshot_index_plus1: candidate_index + 1,
            submitted_at_s: now,
            updated_at_s: now,
            target_line_index_plus1: line_index + 1,
        };
        let index = write
            .append_record(
                Self::land_record_file(),
                &row.encode().map_err(storage_error)?,
            )
            .map_err(storage_error)?;
        self.repair_receipt_indexes(&mut write, index, &row)
            .map_err(storage_error)?;
        write.commit().map_err(storage_error)?;
        self.close_receipted_task(task_id)?;
        self.local_task_receipt(task_id)?
            .ok_or_else(|| unknown_row("Task receipt", task_id))
    }

    fn repair_receipt_indexes<F: BinaryDbFsyncPolicy>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        index: u32,
        row: &LocalLandRecord,
    ) -> StoreResult<()> {
        write.overwrite_record(
            Self::task_land_index_file(),
            row.task_index,
            &TaskLandIndexRecord {
                latest_land_index_plus1: index + 1,
                land_count: 1,
                reserved0: 0,
            }
            .encode(),
        )?;
        if let Some(ordinal) = row.change_ordinal_plus1.checked_sub(1) {
            let change = self.change_index_in_task(write, row.task_index, ordinal)?;
            write.overwrite_record(
                Self::change_land_index_file(),
                change,
                &ChangeLandIndexRecord {
                    latest_land_index_plus1: index + 1,
                    land_count: 1,
                    next_land_ordinal: row.land_ordinal + 1,
                    reserved0: 0,
                }
                .encode(),
            )?;
        }
        Ok(())
    }

    pub(super) fn change_index_in_task<A: WorkflowReadAccess>(
        &self,
        access: &A,
        task: u32,
        ordinal: u8,
    ) -> StoreResult<u32> {
        let head = TaskChangeIndexRecord::decode(
            &access.read_record(Self::task_change_index_file(), task)?,
        )?;
        let mut cursor = head.latest_change_index_plus1;
        for _ in 0..head.change_count.min(64) {
            let index = cursor.checked_sub(1).ok_or("Broken Task Change chain")?;
            let row =
                LocalChangeRecord::decode(&access.read_record(Self::change_record_file(), index)?)?;
            if row.task_index != task {
                return Err("Change chain owner differs".into());
            }
            if row.change_ordinal == ordinal {
                return Ok(index);
            }
            cursor = row.previous_change_index_plus1;
        }
        Err("Unknown revision Change within Task".into())
    }

    fn required_task_snapshot_index<A: WorkflowReadAccess>(
        &self,
        access: &A,
        task: u32,
    ) -> StoreResult<Option<u32>> {
        self.latest_owned_snapshot_index(access, task, None, true)
    }
    fn change_snapshot_index<A: WorkflowReadAccess>(
        &self,
        access: &A,
        change: u32,
    ) -> StoreResult<Option<u32>> {
        let owner =
            LocalChangeRecord::decode(&access.read_record(Self::change_record_file(), change)?)?;
        self.latest_owned_snapshot_index(access, owner.task_index, Some(change), false)
    }

    fn ancestor_indices<A: WorkflowReadAccess>(
        &self,
        access: &A,
        root: u32,
    ) -> StoreResult<std::collections::BTreeSet<u32>> {
        let mut result = std::collections::BTreeSet::new();
        let mut active = std::collections::BTreeSet::new();
        let mut todo = vec![(root, false)];
        while let Some((index, leaving)) = todo.pop() {
            if leaving {
                active.remove(&index);
                continue;
            }
            if active.contains(&index) {
                return Err("Snapshot ancestry contains a cycle".into());
            }
            if !result.insert(index) {
                continue;
            }
            if result.len() > 1_048_576 {
                return Err("Snapshot ancestry budget exceeded".into());
            }
            let record = BinarySnapshotCodec::<1>::decode_record(
                &access.read_record(BinarySnapshotCodec::<1>::record_file(), index)?,
            )?;
            if record.snapshot_meta & 0x80 != 0 {
                return Err("Finish ancestry contains a deleted Snapshot".into());
            }
            let additional = if record.has_additional_parents() {
                let raw_payload = access.read_payload(
                    BinarySnapshotCodec::<1>::payload_file(),
                    record.payload_offset,
                    u32::from(record.payload_len),
                )?;
                BinarySnapshotCodec::<1>::decode_payload(
                    &raw_payload,
                    record.has_line_name_payload(),
                    true,
                )?
                .additional_parent_snapshot_indices
            } else {
                Vec::new()
            };
            active.insert(index);
            todo.push((index, true));
            todo.extend(
                record
                    .parent_snapshot_index_plus1
                    .checked_sub(1)
                    .into_iter()
                    .chain(additional)
                    .map(|parent| (parent, false)),
            );
        }
        Ok(result)
    }

    fn close_receipted_task(&self, task_id: &str) -> PlanStoreResult<()> {
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let task = self
            .point_task_index(&write, task_id)
            .map_err(storage_error)?;
        let receipt = self
            .receipt_at(&write, task)
            .map_err(storage_error)?
            .ok_or_else(|| unknown_row("Task receipt", task_id))?;
        let mut record = self.read_task_record(&write, task).map_err(storage_error)?;
        if record.is_terminal() && record.task_meta & 0x40 == 0 {
            return Err(PlanStoreError::Invalid(
                "A closed Task cannot reopen during finish recovery".into(),
            ));
        }
        let ancestry = self
            .ancestor_indices(&write, receipt.landed_snapshot_index_plus1 - 1)
            .map_err(storage_error)?;
        for value in self
            .change_rows_for_task(&write, task)
            .map_err(storage_error)?
        {
            let ordinal = value["change_seq"]
                .as_u64()
                .ok_or_else(|| PlanStoreError::Invalid("Invalid revision ordinal".into()))?
                as u8;
            let index = self
                .change_index_in_task(&write, task, ordinal - 1)
                .map_err(storage_error)?;
            let mut change = LocalChangeRecord::decode(
                &write
                    .read_record(Self::change_record_file(), index)
                    .map_err(storage_error)?,
            )
            .map_err(storage_error)?;
            if change.change_meta & 3 == 3 {
                continue;
            }
            match self.change_snapshot_index(&write,index).map_err(storage_error)? {
                Some(head) if !ancestry.contains(&head)=>{
                    return Err(PlanStoreError::Invalid("Successful receipt does not incorporate an unfinished revision; preserve it for recovery".into()));
                }
                None if receipt.change_ordinal_plus1!=ordinal=>return Err(PlanStoreError::Invalid("A later unfinished revision has no frozen Snapshot; resolve it explicitly before closeout recovery".into())),
                _=>{},
            }
            change.change_meta = if receipt.change_ordinal_plus1 == ordinal {
                (change.change_meta & !3) | 2
            } else {
                (change.change_meta & !3) | 3
            };
            change.archived_at_s = if change.change_meta & 3 == 3 {
                receipt.updated_at_s
            } else {
                0
            };
            change.updated_at_s = receipt.updated_at_s;
            write
                .overwrite_record(
                    Self::change_record_file(),
                    index,
                    &change.encode().map_err(storage_error)?,
                )
                .map_err(storage_error)?;
        }
        if record.task_meta & 0x40 == 0 {
            let marker = (record.task_meta | 0x40) & !0x80;
            record.closed_at_s = now_s()?;
            record.updated_at_s = record.closed_at_s;
            write
                .overwrite_record(
                    Self::task_record_file(),
                    task,
                    &record.encode().map_err(storage_error)?,
                )
                .map_err(storage_error)?;
            write
                .commit_with_record_marker(Self::task_record_file(), task, marker)
                .map_err(storage_error)?;
        } else {
            write.commit().map_err(storage_error)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Pending {
    task: u32,
    pre: u32,
    candidate: u32,
    change: u32,
}
fn read_pending(path: &std::path::Path) -> Result<Option<Pending>, String> {
    use std::io::Read;
    if path.parent().is_some_and(|parent| {
        std::fs::symlink_metadata(parent).is_ok_and(|metadata| !metadata.file_type().is_dir())
    }) {
        return Err("Task finish cursor directory is aliased".into());
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.file_type().is_file() || metadata.len() != 32 {
        return Err("Invalid Task finish cursor".into());
    }
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
            return Err("Task finish cursor changed while opening".into());
        }
    }
    if !opened.file_type().is_file() || opened.len() != 32 {
        return Err("Task finish cursor changed while opening".into());
    }
    let mut raw = Vec::new();
    file.take(33)
        .read_to_end(&mut raw)
        .map_err(|error| error.to_string())?;
    if raw.len() != 32 {
        return Err("Task finish cursor is truncated or changed".into());
    }
    if raw[..4] != 1u32.to_le_bytes() || raw[4] & !7 != 0 || raw[5..8] != [0; 3] {
        return Err("Unsupported Task finish cursor".into());
    }
    let field = |at| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
    let value = Pending {
        task: field(8),
        change: field(12),
        pre: field(16),
        candidate: field(20),
    };
    if (raw[4] & 1 != 0) != (value.change != 0)
        || (raw[4] & 2 != 0) != (value.pre != 0)
        || (raw[4] & 4 != 0) != (value.candidate != 0)
    {
        return Err("Task cursor presence flags differ".into());
    }
    Ok((value.candidate != 0).then_some(value))
}
fn write_pending(path: &std::path::Path, value: Pending) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("Task cursor has no parent")?;
    if path.file_name().and_then(|n| n.to_str()) != Some("cursor.bin")
        || parent.file_name().and_then(|n| n.to_str()) != Some(".ait-worktree")
    {
        return Err("Task finish cursor must use the existing worktree cursor path".into());
    }
    if std::fs::symlink_metadata(parent).is_ok_and(|m| !m.file_type().is_dir()) {
        return Err("Task cursor directory is aliased".into());
    }
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut raw = 1u32.to_le_bytes().to_vec();
    raw.extend_from_slice(&[
        (value.change != 0) as u8 | (((value.pre != 0) as u8) << 1) | 4,
        0,
        0,
        0,
    ]);
    for field in [value.task, value.change, value.pre, value.candidate] {
        raw.extend_from_slice(&field.to_le_bytes());
    }
    raw.extend_from_slice(&now_s().map_err(|e| e.to_string())?.to_le_bytes());
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    staged
        .write_all(&raw)
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    staged.persist(path).map_err(|e| e.to_string())?;
    crate::file_io::sync_filesystem_directory(parent).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::tests::{fixture, seed_line_and_snapshots};
    use tempfile::TempDir;

    fn task(store: &BinaryDbWorkflowStore<crate::binary_db::LocalBinaryDbFs, 1>) -> String {
        TaskStore::create_task(
            store,
            "fixture",
            "Initial work",
            "Implement the Task",
            Some("C"),
            None,
            None,
            None,
        )
        .unwrap()["task_id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn bind(
        store: &BinaryDbWorkflowStore<crate::binary_db::LocalBinaryDbFs, 1>,
        task: &str,
        change: &str,
        snapshot: &str,
    ) {
        let mut write =
            BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
        store
            .record_snapshot_link_in_write(
                &mut write,
                &SnapshotWorkflowBinding {
                    task_id: task.into(),
                    change_id: change.into(),
                    line_name: "main".into(),
                },
                snapshot,
                1,
            )
            .unwrap();
        write.commit().unwrap();
    }
    fn head(store: &BinaryDbWorkflowStore<crate::binary_db::LocalBinaryDbFs, 1>, value: u32) {
        let mut write = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::General).unwrap();
        let file = BinaryLineCodec::<1>::record_file();
        let mut record =
            BinaryLineCodec::<1>::decode_record(&write.read_record(file.clone(), 0).unwrap())
                .unwrap();
        record.head_snapshot_index_plus1 = value;
        write
            .overwrite_record(
                file,
                0,
                &BinaryLineCodec::<1>::encode_record(&record).unwrap(),
            )
            .unwrap();
        write.commit().unwrap();
    }
    #[test]
    fn task_only_snapshots_finish_once_and_retries_preserve_a_newer_target() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (base, candidate) = seed_line_and_snapshots(&store);
        let id = task(&store);
        bind(&store, &id, "", &base);
        bind(&store, &id, "", &candidate);
        assert!(store.changes_for_task(&id).unwrap().is_empty());
        let owners = store
            .snapshot_ownership_rows(&[base.clone(), candidate.clone()])
            .unwrap();
        assert_eq!(owners.len(), 2);
        assert!(owners.iter().all(|owner| owner["change_id"].is_null()));
        let receipt = store
            .finish_local_task(&id, &candidate, "main", Some(&base), None, None)
            .unwrap();
        assert!(receipt["change_ref"].is_null());
        assert_eq!(receipt["task_id"], id);
        assert_eq!(
            TaskStore::get_task(&store, &id).unwrap()["status"],
            "completed"
        );
        assert_eq!(
            TaskStore::list_completed_tasks_with_landed_changes(&store)
                .unwrap()
                .len(),
            1
        );
        assert!(TaskStore::close_task(&store, &id, "canceled").is_err());
        assert!(store
            .finish_local_task(&id, &base, "main", Some(&base), None, None)
            .is_err());
        // A committed receipt remains authoritative even after target history advances.
        head(&store, 1);
        assert_eq!(
            store
                .finish_local_task(&id, &candidate, "main", Some(&base), None, None)
                .unwrap(),
            receipt
        );
        let read = BinaryDbReadTxn::new(store.db());
        assert_eq!(
            store
                .db()
                .record_count(
                    BinaryDbWorkflowStore::<crate::binary_db::LocalBinaryDbFs, 1>::land_record_file(
                    )
                )
                .unwrap(),
            1
        );
        let raw = read
            .read_record(BinaryLineCodec::<1>::record_file(), 0)
            .unwrap();
        assert_eq!(
            BinaryLineCodec::<1>::decode_record(&raw)
                .unwrap()
                .head_snapshot_index_plus1,
            1
        );
        store.validate_detached_authority().unwrap();
    }
    #[test]
    fn pending_apply_recovers_in_a_fresh_store_and_repairs_receipt_indexes() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (base, candidate) = seed_line_and_snapshots(&store);
        let id = task(&store);
        let cursor = temp.path().join(".ait-worktree/cursor.bin");
        write_pending(
            &cursor,
            Pending {
                task: 0,
                pre: 1,
                candidate: 2,
                change: 0,
            },
        )
        .unwrap();
        head(&store, 2);
        let fresh = fixture(&temp);
        fresh
            .finish_local_task(
                &id,
                &candidate,
                "main",
                Some(&candidate),
                None,
                Some(&cursor),
            )
            .unwrap();
        let mut write = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::General).unwrap();
        write
            .overwrite_record(
                BinaryDbWorkflowStore::<crate::binary_db::LocalBinaryDbFs, 1>::task_land_index_file(
                ),
                0,
                &[0; 8],
            )
            .unwrap();
        write.commit().unwrap();
        assert!(store.local_task_receipt(&id).unwrap().is_none());
        let repaired = store
            .finish_local_task(&id, &candidate, "main", Some(&base), None, Some(&cursor))
            .unwrap();
        assert_eq!(repaired["pre_land_target_snapshot_id"], base);
        store.validate_detached_authority().unwrap();
    }
    #[test]
    fn revisions_require_frozen_incorporated_work_and_one_task_receipt() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (base, candidate) = seed_line_and_snapshots(&store);
        let id = task(&store);
        bind(&store, &id, "", &base);
        let change = ChangeStore::create_change(
            &store,
            &id,
            "fixture",
            "Correct reviewed baseline",
            "main",
            Some("C"),
            Some(&base),
        )
        .unwrap();
        assert_eq!(change["change_id"], "C-01");
        assert_eq!(change["fork_snapshot_id"], base);
        assert!(store
            .finish_local_task(
                &id,
                &candidate,
                "main",
                Some(&base),
                Some(&format!("{id}/C-01")),
                None
            )
            .is_err());
        bind(&store, &id, "C-01", &candidate);
        assert!(store
            .finish_local_task(&id, &base, "main", Some(&base), None, None)
            .is_err());
        let second = ChangeStore::create_change(
            &store,
            &id,
            "fixture",
            "Independent later revision",
            "main",
            Some("C"),
            Some(&candidate),
        )
        .unwrap();
        assert_eq!(second["change_id"], "C-02");
        ChangeStore::close_change(&store, &format!("{id}/C-02"), "archived").unwrap();
        let receipt = store
            .finish_local_task(
                &id,
                &candidate,
                "main",
                Some(&base),
                Some(&format!("{id}/C-01")),
                None,
            )
            .unwrap();
        assert_eq!(receipt["change_ref"], format!("{id}/C-01"));
        assert!(ChangeStore::create_change(
            &store,
            &id,
            "fixture",
            "Too late",
            "main",
            Some("C"),
            Some(&candidate)
        )
        .is_err());
        assert_eq!(
            store.changes_for_task(&id).unwrap()[1]["status"],
            "archived"
        );
        store.validate_detached_authority().unwrap();
    }
    #[test]
    fn point_task_query_does_not_decode_an_unrelated_task() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let _ = seed_line_and_snapshots(&store);
        let id = task(&store);
        let _other = task(&store);
        let mut write = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::General).unwrap();
        let file =
            BinaryDbWorkflowStore::<crate::binary_db::LocalBinaryDbFs, 1>::task_record_file();
        let mut raw = write.read_record(file.clone(), 1).unwrap();
        raw[2..4].copy_from_slice(&0u16.to_le_bytes());
        write.overwrite_record(file, 1, &raw).unwrap();
        write.commit().unwrap();
        assert_eq!(TaskStore::get_task(&store, &id).unwrap()["task_id"], id);
        assert!(TaskStore::list_tasks(&store).is_err());
    }
    #[test]
    fn receipt_recovers_status_last_close_and_never_reopens_a_canceled_task() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (base, candidate) = seed_line_and_snapshots(&store);
        let id = task(&store);
        let receipt = store
            .finish_local_task(&id, &candidate, "main", Some(&base), None, None)
            .unwrap();
        let mut write = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::General).unwrap();
        let file =
            BinaryDbWorkflowStore::<crate::binary_db::LocalBinaryDbFs, 1>::task_record_file();
        let mut raw = write.read_record(file.clone(), 0).unwrap();
        raw[0] &= !0x40;
        write.overwrite_record(file, 0, &raw).unwrap();
        write.commit().unwrap();
        let pending = TaskStore::get_task(&store, &id).unwrap();
        assert_eq!(pending["status"], "active");
        assert!(pending["closed_at"].is_null());
        assert_eq!(
            store
                .finish_local_task(&id, &candidate, "main", Some(&base), None, None)
                .unwrap(),
            receipt
        );
        assert_eq!(
            TaskStore::get_task(&store, &id).unwrap()["status"],
            "completed"
        );
        let canceled = task(&store);
        TaskStore::close_task(&store, &canceled, "canceled").unwrap();
        assert!(store
            .finish_local_task(&canceled, &candidate, "main", Some(&candidate), None, None)
            .is_err());
        assert!(store.local_task_receipt(&canceled).unwrap().is_none());
        store.validate_detached_authority().unwrap();
    }
    #[test]
    fn stale_target_and_duplicate_receipts_fail_before_target_mutation() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (base, candidate) = seed_line_and_snapshots(&store);
        let id = task(&store);
        head(&store, 2);
        assert!(store
            .finish_local_task(&id, &base, "main", Some(&base), None, None)
            .is_err());
        assert!(store.local_task_receipt(&id).unwrap().is_none());
        store
            .finish_local_task(&id, &candidate, "main", Some(&candidate), None, None)
            .unwrap();
        let file =
            BinaryDbWorkflowStore::<crate::binary_db::LocalBinaryDbFs, 1>::land_record_file();
        let mut write = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::General).unwrap();
        let raw = write.read_record(file.clone(), 0).unwrap();
        write.append_record(file, &raw).unwrap();
        write.commit().unwrap();
        assert!(store
            .finish_local_task(&id, &candidate, "main", Some(&candidate), None, None)
            .unwrap_err()
            .to_string()
            .contains("multiple"));
        assert!(store.validate_detached_authority().is_err());
        let raw = store
            .db()
            .read_record(BinaryLineCodec::<1>::record_file(), 0)
            .unwrap();
        assert_eq!(
            BinaryLineCodec::<1>::decode_record(&raw)
                .unwrap()
                .head_snapshot_index_plus1,
            2
        );
    }
    #[test]
    fn pending_cursor_blocks_new_work_and_rejects_aliases_or_truncation() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let _ = seed_line_and_snapshots(&store);
        let id = task(&store);
        let cursor = temp.path().join(".ait-worktree/cursor.bin");
        write_pending(
            &cursor,
            Pending {
                task: 0,
                pre: 1,
                candidate: 2,
                change: 0,
            },
        )
        .unwrap();
        assert!(store
            .guard_no_pending_local_finish(&id, &cursor)
            .unwrap_err()
            .to_string()
            .contains("pending finish"));
        std::fs::write(&cursor, [0; 3]).unwrap();
        assert!(read_pending(&cursor).is_err());
        #[cfg(unix)]
        {
            std::fs::remove_file(&cursor).unwrap();
            let outside = temp.path().join("outside");
            std::fs::write(&outside, [0; 32]).unwrap();
            std::os::unix::fs::symlink(&outside, &cursor).unwrap();
            assert!(read_pending(&cursor).is_err());
            assert_eq!(std::fs::read(&outside).unwrap(), [0; 32]);
        }
    }
}
