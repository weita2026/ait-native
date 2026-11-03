//! Durable numeric Snapshot ownership, using the protected layout-2 link records.
use super::*;

const LINK_SIZE: u32 = AUTHORITATIVE_SNAPSHOT_LINK_RECORD_SIZE;
const INDEX_SIZE: u32 = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotWorkflowBinding {
    pub task_id: String,
    pub change_id: String,
    pub line_name: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct OwnerIndex {
    latest: u32,
    count: u16,
    next: u8,
}

impl OwnerIndex {
    fn encode(self, task: bool) -> Vec<u8> {
        let mut out = self.latest.to_le_bytes().to_vec();
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&[if task { self.next } else { 0 }, 0]);
        out
    }

    fn decode(raw: &[u8], task: bool) -> StoreResult<Self> {
        require_raw_size(raw, INDEX_SIZE, "Snapshot owner index")?;
        if raw[7] != 0 || (!task && raw[6] != 0) {
            return Err("Snapshot owner index has reserved bytes".into());
        }
        Ok(Self {
            latest: u32::from_le_bytes(raw[..4].try_into().unwrap()),
            count: u16::from_le_bytes(raw[4..6].try_into().unwrap()),
            next: raw[6],
        })
    }
}

type Link = SnapshotLinkRecord;

type SnapshotLinkInventory = (
    Vec<(Link, SnapshotWorkflowBinding)>,
    Vec<OwnerIndex>,
    Vec<OwnerIndex>,
);

impl<B: BinaryDb, const L: u32> BinaryDbWorkflowStore<B, L> {
    pub(super) fn validate_snapshot_link_authority<A: WorkflowReadAccess>(
        &self,
        access: &A,
        rows: &WorkflowRows,
    ) -> StoreResult<()> {
        self.snapshot_links(access, rows, false).map(|_| ())
    }

    fn snapshot_link_file() -> BinaryFileId {
        BinaryFileId::new("snapshot_link.bin", SNAPSHOT_LINK_LAYOUT_ID, LINK_SIZE)
    }
    fn snapshot_owner_file(task: bool) -> BinaryFileId {
        fixed_file::<L>(
            if task {
                "task_snapshot_index.bin"
            } else {
                "change_snapshot_index.bin"
            },
            INDEX_SIZE,
        )
    }

    fn snapshot_link_read_file(&self) -> BinaryFileId {
        if self.legacy_land_reader {
            BinaryFileId::new("snapshot_link.bin", 1, 40)
        } else {
            Self::snapshot_link_file()
        }
    }
    fn snapshot_authoring_line<A: WorkflowReadAccess>(
        access: &A,
        index: u32,
    ) -> StoreResult<String> {
        let row = BinarySnapshotCodec::<1>::decode_record(
            &access.read_record(BinarySnapshotCodec::<1>::record_file(), index)?,
        )?;
        if row.is_tombstone() {
            return Err("Snapshot Link content is tombstoned".into());
        }
        if let Some(line) = row.line_index_plus1.checked_sub(1) {
            return line_name_at(access, line);
        }
        if !row.has_line_name_payload() {
            return Err("Linked Content Snapshot has no authoring Line".into());
        }
        let raw = access.read_payload(
            BinarySnapshotCodec::<1>::payload_file(),
            row.payload_offset,
            u32::from(row.payload_len),
        )?;
        Ok(
            BinarySnapshotCodec::<1>::decode_payload(&raw, true, row.has_additional_parents())?
                .line_name,
        )
    }
    pub(super) fn snapshot_owners_with_access<A: WorkflowReadAccess>(
        &self,
        access: &A,
        rows: &WorkflowRows,
    ) -> StoreResult<Vec<(SnapshotLinkRecord, SnapshotWorkflowBinding)>> {
        self.snapshot_links(access, rows, true)
            .map(|(links, _, _)| links)
    }

    fn snapshot_links<A: WorkflowReadAccess>(
        &self,
        access: &A,
        rows: &WorkflowRows,
        verify_namespace: bool,
    ) -> StoreResult<SnapshotLinkInventory> {
        use super::legacy_snapshot_links::{LegacySnapshotLink, LegacySnapshotLinkPayload};
        let mut tasks = vec![OwnerIndex::default(); rows.tasks.len()];
        let mut changes = vec![OwnerIndex::default(); rows.changes.len()];
        let mut links = Vec::new();
        let mut seen = BTreeMap::new();
        for index in 0..access.record_count(self.snapshot_link_read_file())? {
            let raw = access.read_record(self.snapshot_link_read_file(), index)?;
            let legacy = if self.legacy_land_reader {
                Some(LegacySnapshotLink::decode(&raw)?)
            } else {
                None
            };
            let link = if let Some(old) = &legacy {
                Link {
                    meta: old.meta & !0x38,
                    ordinal: old.ordinal,
                    task: old.task,
                    change_plus1: old.change_plus1,
                    snapshot: old.snapshot,
                    created_at_s: old.created_at_s,
                }
            } else {
                Link::decode(&raw)?
            };
            let task_id = self.local_task_id(link.task)?;
            let task = tasks
                .get_mut(link.task as usize)
                .ok_or("Snapshot Link Task is missing")?;
            if task.count >= 256 || u16::from(link.ordinal) != task.count {
                return Err("Snapshot Link Task ordinal is inconsistent".into());
            }
            let mut change_id = String::new();
            if let Some(change_index) = link.change_plus1.checked_sub(1) {
                let record = LocalChangeRecord::decode(
                    &access.read_record(Self::change_record_file(), change_index)?,
                )?;
                if record.task_index != link.task {
                    return Err("Snapshot Link Change belongs to another Task".into());
                }
                change_id = render_change_id(record.change_ordinal);
                let change = changes
                    .get_mut(change_index as usize)
                    .ok_or("Snapshot Link Change is missing")?;
                if legacy
                    .as_ref()
                    .is_some_and(|old| old.previous_change != change.latest)
                {
                    return Err("Legacy Snapshot Link Change chain is inconsistent".into());
                }
                change.latest = index + 1;
                change.count = change
                    .count
                    .checked_add(1)
                    .ok_or("Snapshot Link Change count overflow")?;
            }
            if let Some(old) = &legacy {
                let payload = LegacySnapshotLinkPayload::decode(&access.read_payload(
                    BinaryPayloadFileId::new("snapshot_link_payload.bin", 1),
                    old.payload_offset,
                    u32::from(old.payload_len),
                )?)?;
                let task_matches = if verify_namespace {
                    payload.task_id == task_id
                } else {
                    let (prefix, _) = payload
                        .task_id
                        .split_once("T-")
                        .ok_or("Invalid legacy Snapshot Link Task")?;
                    let namespace = prefix
                        .strip_prefix('L')
                        .ok_or("Non-local legacy Snapshot Link Task")?;
                    payload.task_id
                        == format!(
                            "{}T-{:04}",
                            workflow_origin_namespace_prefix("L", Some(namespace))?,
                            u64::from(link.task) + 1
                        )
                };
                if !task_matches
                    || payload.change_id != change_id
                    || old.previous_task != task.latest
                    || (link.change_plus1 == 0 && old.previous_change != 0)
                    || (old.meta & 8 != 0) == payload.worktree_name.is_empty()
                    || (old.meta & 16 != 0) == payload.line_name.is_empty()
                    || (old.meta & 32 != 0)
                        != (!payload.author_mode.is_empty() || !payload.model_name.is_empty())
                {
                    return Err(
                        "Legacy Snapshot Link fixed/payload ownership or flags disagree".into(),
                    );
                }
            }
            task.latest = index + 1;
            task.count += 1;
            task.next = task.count.min(255) as u8;
            let snapshot_id = snapshot_id_at(access, link.snapshot)?;
            if link.meta & 0x80 == 0 && seen.insert(snapshot_id, index).is_some() {
                return Err("Snapshot has multiple live ownership links".into());
            }
            let line_name = Self::snapshot_authoring_line(access, link.snapshot)?;
            links.push((
                link,
                SnapshotWorkflowBinding {
                    task_id,
                    change_id,
                    line_name,
                },
            ));
        }
        for (is_task, expected) in [(true, &tasks), (false, &changes)] {
            let file = Self::snapshot_owner_file(is_task);
            let actual_count = access.record_count(file.clone())?;
            if actual_count as usize > expected.len() {
                return Err("Snapshot owner index has extra rows".into());
            }
            for (index, expected) in expected.iter().enumerate() {
                let actual = if index < actual_count as usize {
                    OwnerIndex::decode(&access.read_record(file.clone(), index as u32)?, is_task)?
                } else {
                    OwnerIndex::default()
                };
                if actual != *expected {
                    return Err(
                        "Snapshot owner index disagrees with fixed ownership and ordinals".into(),
                    );
                }
            }
        }
        Ok((links, tasks, changes))
    }

    fn validate_snapshot_owner(
        &self,
        rows: &WorkflowRows,
        binding: &SnapshotWorkflowBinding,
    ) -> PlanStoreResult<(u32, Option<u32>)> {
        let task_index = self.task_index_for_id(rows, &binding.task_id)?;
        let change_ref = format!("{}/{}", binding.task_id, binding.change_id);
        let change_index = if binding.change_id.is_empty() {
            None
        } else {
            Some(self.change_index_for_id(rows, &change_ref)?)
        };
        let task = &rows.tasks[&binding.task_id];
        if task["status"].as_str() != Some("active") || !task["local_finish"].is_null() {
            return Err(PlanStoreError::Invalid(
                "Snapshot authoring requires an active Task".to_string(),
            ));
        }
        if change_index.is_some() {
            let change = &rows.changes[&change_ref];
            if !matches!(
                change["status"].as_str(),
                Some("draft" | "active" | "review")
            ) || change["task_id"].as_str() != Some(binding.task_id.as_str())
            {
                return Err(PlanStoreError::Invalid(
                    "Snapshot authoring requires a writable Change owned by the Task".to_string(),
                ));
            }
        } else if rows.changes.values().any(|change| {
            change["task_id"] == binding.task_id
                && matches!(
                    change["status"].as_str(),
                    Some("draft" | "active" | "review")
                )
        }) {
            return Err(PlanStoreError::Invalid(
                "Select the exact active revision Change before authoring".into(),
            ));
        }
        if binding.line_name.trim().is_empty() {
            return Err(PlanStoreError::Invalid(
                "Snapshot authoring requires a bound worktree and Line".to_string(),
            ));
        }
        Ok((task_index, change_index))
    }

    pub fn validate_snapshot_binding(
        &self,
        binding: &SnapshotWorkflowBinding,
    ) -> PlanStoreResult<()> {
        let read = BinaryDbReadTxn::new(&self.db);
        let rows = self.read_rows_with_access(&read).map_err(storage_error)?;
        let (task_index, _) = self.validate_snapshot_owner(&rows, binding)?;
        let (_, tasks, _) = self
            .snapshot_links(&read, &rows, true)
            .map_err(storage_error)?;
        if tasks[task_index as usize].count >= 256 {
            return Err(PlanStoreError::Invalid(
                "Task has exhausted its 256 Snapshot links".to_string(),
            ));
        }
        Ok(())
    }

    /// Read-only budget check before history replay/Plan publication. Fixed
    /// owner-link counts include tombstones and are not reusable ordinals.
    pub fn validate_replay_snapshot_capacity(
        &self,
        additions: &std::collections::BTreeMap<String, usize>,
    ) -> PlanStoreResult<()> {
        let read = BinaryDbReadTxn::new(&self.db);
        let rows = self.read_rows_with_access(&read).map_err(storage_error)?;
        let (_, tasks, _) = self
            .snapshot_links(&read, &rows, true)
            .map_err(storage_error)?;
        for (task_id, additional) in additions {
            let index = self.task_index_for_id(&rows, task_id)?;
            if usize::from(tasks[index as usize].count).saturating_add(*additional) > 256 {
                return Err(PlanStoreError::Invalid(format!("Task {task_id} has insufficient capacity for replay within its 256 Snapshot links")));
            }
        }
        Ok(())
    }

    pub(crate) fn record_snapshot_link_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, B, F>,
        binding: &SnapshotWorkflowBinding,
        snapshot_id: &str,
        created_at_s: u64,
    ) -> StoreResult<()> {
        let rows = self.read_rows_with_access(tx)?;
        let (task_index, change_index) = self
            .validate_snapshot_owner(&rows, binding)
            .map_err(|e| e.to_string())?;
        self.append_snapshot_link_in_write(
            tx,
            &rows,
            binding,
            (task_index, change_index),
            snapshot_id,
            created_at_s,
        )
    }

    /// Restricted history replay admission. Derive the exact owner from a live
    /// source link, never caller-supplied attribution or receipt inference.
    /// Ordinary capture continues to require an active Task/writable Change.
    pub(crate) fn record_replayed_snapshot_link_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, B, F>,
        source_snapshot_id: &str,
        snapshot_id: &str,
        created_at_s: u64,
    ) -> StoreResult<()> {
        if source_snapshot_id == snapshot_id {
            return Err("History replay may only bind a new Snapshot".into());
        }
        let rows = self.read_rows_with_access(tx)?;
        let (links, _, _) = self.snapshot_links(tx, &rows, true)?;
        let source_index = snapshot_index_by_id(tx, source_snapshot_id)?
            .ok_or("History replay source Snapshot is missing")?;
        let (source, binding) = links
            .iter()
            .find(|(link, _)| link.snapshot == source_index && link.meta & 0x80 == 0)
            .ok_or("History replay requires exact live source Snapshot ownership")?;
        let task = &rows.tasks[&binding.task_id];
        if task["status"].as_str() != Some("completed")
            || !self
                .receipt_at(tx, source.task)?
                .is_some_and(|receipt| receipt.is_succeeded())
        {
            return Err(
                "History replay requires the completed owner's original successful receipt".into(),
            );
        }
        let snapshot_index = snapshot_index_by_id(tx, snapshot_id)?
            .ok_or("History replay output Snapshot is missing")?;
        if links
            .iter()
            .any(|(link, _)| link.snapshot == snapshot_index)
        {
            return Err("History replay cannot rebind an existing Snapshot link".into());
        }
        self.append_snapshot_link_in_write(
            tx,
            &rows,
            binding,
            (source.task, source.change_plus1.checked_sub(1)),
            snapshot_id,
            created_at_s,
        )
    }

    fn append_snapshot_link_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, B, F>,
        rows: &WorkflowRows,
        binding: &SnapshotWorkflowBinding,
        owner: (u32, Option<u32>),
        snapshot_id: &str,
        created_at_s: u64,
    ) -> StoreResult<()> {
        let (task_index, change_index) = owner;
        let (links, mut tasks, mut changes) = self.snapshot_links(tx, rows, true)?;
        let snapshot_index =
            snapshot_index_by_id(tx, snapshot_id)?.ok_or("Snapshot Link content is missing")?;
        if let Some((_, existing)) = links
            .iter()
            .find(|(link, _)| link.snapshot == snapshot_index && link.meta & 0x80 == 0)
        {
            return if existing == binding {
                Ok(())
            } else {
                Err("Snapshot ownership is immutable and conflicts with this Task/Change".into())
            };
        }
        let task = &mut tasks[task_index as usize];
        let ordinal =
            u8::try_from(task.count).map_err(|_| "Task has exhausted its 256 Snapshot links")?;
        if binding.line_name != Self::snapshot_authoring_line(tx, snapshot_index)? {
            return Err("Snapshot binding Line differs from immutable Content Snapshot".into());
        }
        let link = Link {
            meta: u8::from(change_index.is_some()),
            ordinal,
            task: task_index,
            change_plus1: change_index.map_or(0, |index| index + 1),
            snapshot: snapshot_index,
            created_at_s,
        };
        let index = tx.append_record(Self::snapshot_link_file(), &link.encode()?)?;
        task.latest = index + 1;
        task.count += 1;
        task.next = if task.count == 256 {
            255
        } else {
            task.count as u8
        };
        if let Some(change_index) = change_index {
            let change = &mut changes[change_index as usize];
            change.latest = index + 1;
            change.count = change
                .count
                .checked_add(1)
                .ok_or("Snapshot Link Change count overflow")?;
        }
        for (is_task, owners, modified) in [
            (true, tasks, Some(task_index)),
            (false, changes, change_index),
        ] {
            let file = Self::snapshot_owner_file(is_task);
            let old_count = tx.record_count(file.clone())?;
            for (index, owner) in owners.iter().enumerate() {
                if index >= old_count as usize {
                    tx.append_record(file.clone(), &owner.encode(is_task))?;
                } else if Some(index as u32) == modified {
                    tx.overwrite_record(file.clone(), index as u32, &owner.encode(is_task))?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn latest_owned_snapshot_index<A: WorkflowReadAccess>(
        &self,
        access: &A,
        task: u32,
        change: Option<u32>,
        skip_archived_revisions: bool,
    ) -> StoreResult<Option<u32>> {
        let is_task = change.is_none();
        let owner = change.unwrap_or(task);
        let file = Self::snapshot_owner_file(is_task);
        let head = if owner < access.record_count(file.clone())? {
            OwnerIndex::decode(&access.read_record(file, owner)?, is_task)?
        } else {
            OwnerIndex::default()
        };
        if head.count > 256 || (is_task && head.next != head.count.min(255) as u8) {
            return Err("Invalid Snapshot owner index".into());
        }
        let mut count = 0u16;
        let mut latest = None::<(u8, u32)>;
        let mut live = None::<(u8, u32)>;
        let mut ordinals = [false; 256];
        // Without predecessor slots, inspect fixed owner refs. Global neighbors
        // are never interpreted as an owner's predecessor.
        for index in 0..access.record_count(Self::snapshot_link_file())? {
            let link = Link::decode(&access.read_record(Self::snapshot_link_file(), index)?)?;
            if link.task != task || change.is_some_and(|change| link.change_plus1 != change + 1) {
                continue;
            }
            if ordinals[link.ordinal as usize] {
                return Err("Duplicate Task-scoped Snapshot ordinal".into());
            }
            ordinals[link.ordinal as usize] = true;
            count = count
                .checked_add(1)
                .ok_or("Snapshot owner count overflow")?;
            if latest.is_none_or(|(ordinal, _)| link.ordinal > ordinal) {
                latest = Some((link.ordinal, index + 1));
            }
            let archived = if skip_archived_revisions {
                link.change_plus1
                    .checked_sub(1)
                    .map(|index| -> StoreResult<bool> {
                        let row = LocalChangeRecord::decode(
                            &access.read_record(Self::change_record_file(), index)?,
                        )?;
                        if row.task_index != task {
                            return Err("Snapshot revision belongs to another Task".into());
                        }
                        Ok(row.change_meta & 3 == 3 || row.change_state != 0)
                    })
                    .transpose()?
                    .unwrap_or(false)
            } else {
                false
            };
            if link.meta & 0x80 == 0
                && !archived
                && live.is_none_or(|(ordinal, _)| link.ordinal > ordinal)
            {
                live = Some((link.ordinal, link.snapshot));
            }
        }
        if count != head.count
            || latest.map_or(0, |(_, index)| index) != head.latest
            || (is_task && (0..count).any(|ordinal| !ordinals[ordinal as usize]))
        {
            return Err("Snapshot owner index disagrees with fixed ownership and ordinals".into());
        }
        Ok(live.map(|(_, snapshot)| snapshot))
    }

    pub fn latest_task_snapshot_id(&self, task_id: &str) -> PlanStoreResult<Option<String>> {
        let read = BinaryDbReadTxn::new(&self.db);
        let task = self
            .point_task_index(&read, task_id)
            .map_err(storage_error)?;
        self.latest_owned_snapshot_index(&read, task, None, false)
            .map_err(storage_error)?
            .map(|index| snapshot_id_at(&read, index))
            .transpose()
            .map_err(storage_error)
    }

    /// Resolve the latest live checkpoint of this exact Task-owned Change.
    /// The declared owner chain supplies ordering, including after worktree cleanup.
    pub fn latest_change_snapshot_id(&self, change_ref: &str) -> PlanStoreResult<Option<String>> {
        let read = BinaryDbReadTxn::new(&self.db);
        let rows = self.read_rows_with_access(&read).map_err(storage_error)?;
        let change_index = self.change_index_for_id(&rows, change_ref)?;
        let task = LocalChangeRecord::decode(
            &read
                .read_record(Self::change_record_file(), change_index)
                .map_err(storage_error)?,
        )
        .map_err(storage_error)?
        .task_index;
        self.latest_owned_snapshot_index(&read, task, Some(change_index), false)
            .map_err(storage_error)?
            .map(|index| snapshot_id_at(&read, index))
            .transpose()
            .map_err(storage_error)
    }

    pub fn snapshot_ownership_rows(
        &self,
        snapshot_ids: &[String],
    ) -> PlanStoreResult<Vec<JsonValue>> {
        let read = BinaryDbReadTxn::new(&self.db);
        let rows = self.read_rows_with_access(&read).map_err(storage_error)?;
        let (links, _, _) = self
            .snapshot_links(&read, &rows, true)
            .map_err(storage_error)?;
        let requested = snapshot_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        let mut output = Vec::new();
        for (link, binding) in links {
            if link.meta & 0x80 != 0 {
                continue;
            }
            let snapshot_id = snapshot_id_at(&read, link.snapshot).map_err(storage_error)?;
            if !requested.contains(&snapshot_id) {
                continue;
            }
            output.push(json!({
                "snapshot_id": snapshot_id, "task_id": binding.task_id,
                "change_id": nonempty(Some(&binding.change_id)),
                "line_name": nonempty(Some(&binding.line_name)), "created_at": link.created_at_s.to_string(),
                "ownership_source": "durable_snapshot_binding",
            }));
        }
        // A successful Task receipt proves exactly its accepted Snapshot. No
        // ancestor inference survives removal of a legacy initial Change.
        for snapshot_id in snapshot_ids {
            let receipts = rows
                .tasks
                .values()
                .filter(|task| {
                    task["local_finish"]["landed_snapshot_id"].as_str() == Some(snapshot_id)
                })
                .collect::<Vec<_>>();
            // A fixed live ownership link is authoring authority. Another
            // Task may accept that same Snapshot in a no-op finish; its receipt
            // does not erase or replace the explicit authoring binding.
            if output
                .iter()
                .any(|row| row["snapshot_id"].as_str() == Some(snapshot_id))
            {
                continue;
            }
            if receipts.len() == 1
                && !output
                    .iter()
                    .any(|row| row["snapshot_id"].as_str() == Some(snapshot_id))
            {
                let receipt = &receipts[0]["local_finish"];
                let line = snapshot_index_by_id(&read, snapshot_id)
                    .map_err(storage_error)?
                    .map(|index| Self::snapshot_authoring_line(&read, index))
                    .transpose()
                    .map_err(storage_error)?;
                let change = receipt["change_ref"]
                    .as_str()
                    .and_then(|id| id.rsplit('/').next());
                output.push(json!({
                    "snapshot_id":snapshot_id,"task_id":receipts[0]["task_id"],
                    "change_id":change,"line_name":line,"ownership_source":"derived_from_recorded_task_receipt",
                }));
            } else if receipts.len() > 1 {
                let accepted_tasks = receipts
                    .iter()
                    .map(|task| task["task_id"].clone())
                    .collect::<Vec<_>>();
                output.push(json!({
                    "snapshot_id":snapshot_id,"task_id":null,"change_id":null,
                    "ownership_source":"ambiguous_recorded_task_receipts",
                    "ownership_conflict":true,"accepted_task_ids":accepted_tasks,
                }));
            }
        }
        Ok(output)
    }

    /// The original governed source is the fixed owner-link prefix through
    /// the real successful receipt Snapshot. Restricted replays append later
    /// ordinals and cannot change this set, including after a process restart.
    pub fn completed_task_source_snapshots(
        &self,
        task_id: &str,
    ) -> PlanStoreResult<Vec<(String, SnapshotWorkflowBinding)>> {
        let read = BinaryDbReadTxn::new(&self.db);
        let rows = self.read_rows_with_access(&read).map_err(storage_error)?;
        self.completed_task_source_snapshots_with_access(&read, &rows, task_id)
    }

    pub(super) fn completed_task_source_snapshots_with_access<A: WorkflowReadAccess>(
        &self,
        read: &A,
        rows: &WorkflowRows,
        task_id: &str,
    ) -> PlanStoreResult<Vec<(String, SnapshotWorkflowBinding)>> {
        let index = self.task_index_for_id(rows, task_id)?;
        if rows.tasks[task_id]["status"].as_str() != Some("completed")
            || !self
                .receipt_at(read, index)
                .map_err(storage_error)?
                .is_some_and(|receipt| receipt.is_succeeded())
        {
            return Err(PlanStoreError::Invalid(
                "Original source selection requires a completed Task with its successful receipt"
                    .into(),
            ));
        }
        let receipt_id = rows.tasks[task_id]["local_finish"]["landed_snapshot_id"]
            .as_str()
            .ok_or_else(|| {
                PlanStoreError::Invalid("Original successful Snapshot identity is missing".into())
            })?;
        let (links, _, _) = self
            .snapshot_links(read, rows, true)
            .map_err(storage_error)?;
        let receipt_index = snapshot_index_by_id(read, receipt_id)
            .map_err(storage_error)?
            .ok_or_else(|| {
                PlanStoreError::Invalid("Original receipt Snapshot is unavailable".into())
            })?;
        let cutoff = links
            .iter()
            .find(|(link, _)| {
                link.task == index && link.snapshot == receipt_index && link.meta & 0x80 == 0
            })
            .map(|(link, _)| link.ordinal)
            .ok_or_else(|| {
                PlanStoreError::Invalid("Original receipt has no exact fixed source owner".into())
            })?;
        if links
            .iter()
            .any(|(link, _)| link.task == index && link.ordinal <= cutoff && link.meta & 0x80 != 0)
        {
            return Err(PlanStoreError::Invalid(
                "Original source contains a deleted Snapshot link; its exact set is unavailable"
                    .into(),
            ));
        }
        links
            .into_iter()
            .filter(|(link, _)| {
                link.task == index && link.ordinal <= cutoff && link.meta & 0x80 == 0
            })
            .map(|(link, owner)| {
                Ok((
                    snapshot_id_at(read, link.snapshot).map_err(storage_error)?,
                    owner,
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod replay_tests;

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture, seed_line_and_snapshots};
    use super::*;
    use crate::binary_db::LocalBinaryDbFs;
    use tempfile::TempDir;

    fn owner(
        store: &BinaryDbWorkflowStore<LocalBinaryDbFs, 1>,
        base: &str,
    ) -> SnapshotWorkflowBinding {
        let task = TaskStore::create_task(
            store,
            "fixture",
            "Ownership",
            "Preserve provenance",
            Some("C"),
            None,
            None,
            None,
        )
        .unwrap();
        let task_id = task["task_id"].as_str().unwrap().to_string();
        ChangeStore::create_change(
            store,
            &task_id,
            "fixture",
            "Checkpoint",
            "main",
            Some("C"),
            Some(base),
        )
        .unwrap();
        SnapshotWorkflowBinding {
            task_id,
            change_id: "C-01".to_string(),
            line_name: "main".to_string(),
        }
    }

    fn bind(
        store: &BinaryDbWorkflowStore<LocalBinaryDbFs, 1>,
        binding: &SnapshotWorkflowBinding,
        id: &str,
    ) -> StoreResult<()> {
        let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite)?;
        store.record_snapshot_link_in_write(&mut tx, binding, id, 1)?;
        tx.commit()?;
        Ok(())
    }

    #[test]
    fn latest_change_checkpoint_uses_exact_owner_chain_and_skips_tombstones() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (first, second) = seed_line_and_snapshots(&store);
        let one = owner(&store, &first);
        ChangeStore::create_change(
            &store,
            &one.task_id,
            "fixture",
            "Sibling",
            "main",
            Some("C"),
            Some(&first),
        )
        .unwrap();
        let two = SnapshotWorkflowBinding {
            change_id: "C-02".into(),
            ..one.clone()
        };
        let first_ref = format!("{}/C-01", one.task_id);
        let second_ref = format!("{}/C-02", one.task_id);
        assert_eq!(store.latest_change_snapshot_id(&second_ref).unwrap(), None);
        bind(&store, &one, &first).unwrap();
        bind(&store, &two, &second).unwrap();
        TaskStore::close_task(&store, &one.task_id, "completed").unwrap();
        let reopened = fixture(&temp);
        assert_eq!(
            reopened.latest_change_snapshot_id(&first_ref).unwrap(),
            Some(first)
        );
        assert_eq!(
            reopened.latest_change_snapshot_id(&second_ref).unwrap(),
            Some(second)
        );
        assert!(reopened
            .latest_change_snapshot_id(&format!("{}/C-03", one.task_id))
            .is_err());
        let file = BinaryDbWorkflowStore::<LocalBinaryDbFs, 1>::snapshot_link_file();
        let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::General).unwrap();
        let mut last = tx.read_record(file.clone(), 1).unwrap();
        last[0] |= 0x80;
        tx.overwrite_record(file, 1, &last).unwrap();
        tx.commit().unwrap();
        assert_eq!(store.latest_change_snapshot_id(&second_ref).unwrap(), None);
    }

    #[test]
    fn durable_snapshot_links_survive_closeout_and_detached_validation() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (first, second) = seed_line_and_snapshots(&store);
        let binding = owner(&store, &first);
        bind(&store, &binding, &first).unwrap();
        bind(&store, &binding, &first).unwrap(); // A retry cannot append another ordinal.
        bind(&store, &binding, &second).unwrap();
        TaskStore::close_task(&store, &binding.task_id, "completed").unwrap();
        let reopened = fixture(&temp);
        let rows = reopened.snapshot_ownership_rows(&[first, second]).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row["task_id"] == binding.task_id
            && row["ownership_source"] == "durable_snapshot_binding"));
        assert_eq!(
            std::fs::metadata(temp.path().join("binary-db/snapshot_link.bin"))
                .unwrap()
                .len(),
            4 + 2 * 22
        );
        assert_eq!(
            std::fs::metadata(temp.path().join("binary-db/task_snapshot_index.bin"))
                .unwrap()
                .len(),
            12
        );
        BinaryDbWorkflowStore::<_, 1>::new(reopened.db().clone(), "fixture")
            .validate_detached_authority()
            .unwrap();
        assert!(reopened.validate_snapshot_binding(&binding).is_err());
    }

    #[test]
    fn conflicting_binding_and_corrupt_owner_indexes_fail_closed() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (first, _) = seed_line_and_snapshots(&store);
        let binding = owner(&store, &first);
        let foreign = owner(&store, &first);
        bind(&store, &binding, &first).unwrap();
        let path = temp.path().join("binary-db/snapshot_link.bin");
        let before = std::fs::read(&path).unwrap();
        assert!(bind(&store, &foreign, &first)
            .unwrap_err()
            .contains("immutable"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            store
                .snapshot_ownership_rows(std::slice::from_ref(&first))
                .unwrap()[0]["task_id"],
            binding.task_id
        );
        let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::General).unwrap();
        tx.overwrite_record(
            BinaryDbWorkflowStore::<LocalBinaryDbFs, 1>::snapshot_owner_file(true),
            0,
            &[0; 8],
        )
        .unwrap();
        tx.commit().unwrap();
        assert!(store
            .snapshot_ownership_rows(&[first])
            .unwrap_err()
            .to_string()
            .contains("index"));
        assert!(store.validate_detached_authority().is_err());
    }

    #[test]
    fn failed_link_rolls_back_content_in_the_same_transaction() {
        use crate::content_binary_db::BinarySnapshotCodec;
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (first, _) = seed_line_and_snapshots(&store);
        let mut binding = owner(&store, &first);
        binding.change_id = "C-02".to_string();
        let file = BinarySnapshotCodec::<1>::record_file();
        let before = store.db().record_count(file.clone()).unwrap();
        {
            let mut tx =
                BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
            let mut record =
                BinarySnapshotCodec::<1>::decode_record(&tx.read_record(file.clone(), 0).unwrap())
                    .unwrap();
            record.snapshot_hash48 = snapshot_hash48_from_id("SNP-000000000003").unwrap();
            tx.append_record(
                file.clone(),
                &BinarySnapshotCodec::<1>::encode_record(&record).unwrap(),
            )
            .unwrap();
            assert!(store
                .record_snapshot_link_in_write(&mut tx, &binding, "SNP-000000000003", 1)
                .is_err());
        }
        assert_eq!(store.db().record_count(file).unwrap(), before);
    }

    #[test]
    fn legacy_attribution_requires_one_exact_recorded_acceptance() {
        // Seed converted historical evidence, which predates Snapshot links.
        fn historical(store: &BinaryDbWorkflowStore<LocalBinaryDbFs, 1>, task: u32, change: u32) {
            let mut tx =
                BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::General).unwrap();
            let row = LocalLandRecord {
                land_meta: 2 | LAND_META_HAS_PRE_LAND | LAND_META_HAS_LANDED,
                land_ordinal: 0,
                change_ordinal_plus1: 1,
                failure_kind: 0,
                task_index: task,
                previous_task_land_index_plus1: 0,
                previous_change_land_index_plus1: 0,
                pre_land_target_snapshot_index_plus1: 1,
                landed_snapshot_index_plus1: 2,
                submitted_at_s: 1,
                updated_at_s: 1,
                target_line_index_plus1: 1,
            };
            let index = tx
                .append_record(
                    BinaryDbWorkflowStore::<LocalBinaryDbFs, 1>::land_record_file(),
                    &row.encode().unwrap(),
                )
                .unwrap();
            tx.overwrite_record(
                BinaryDbWorkflowStore::<LocalBinaryDbFs, 1>::task_land_index_file(),
                task,
                &TaskLandIndexRecord {
                    latest_land_index_plus1: index + 1,
                    land_count: 1,
                    reserved0: 0,
                }
                .encode(),
            )
            .unwrap();
            tx.overwrite_record(
                BinaryDbWorkflowStore::<LocalBinaryDbFs, 1>::change_land_index_file(),
                change,
                &ChangeLandIndexRecord {
                    latest_land_index_plus1: index + 1,
                    land_count: 1,
                    next_land_ordinal: 1,
                    reserved0: 0,
                }
                .encode(),
            )
            .unwrap();
            let file = BinaryDbWorkflowStore::<LocalBinaryDbFs, 1>::change_record_file();
            let mut raw = tx.read_record(file.clone(), change).unwrap();
            raw[0] = (raw[0] & !3) | 2;
            tx.overwrite_record(file, change, &raw).unwrap();
            tx.commit().unwrap();
        }
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (base, accepted) = seed_line_and_snapshots(&store);
        let _binding = owner(&store, &base);
        historical(&store, 0, 0);
        let rows = store
            .snapshot_ownership_rows(&[base.clone(), accepted.clone()])
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["snapshot_id"], accepted);
        assert_eq!(
            rows[0]["ownership_source"],
            "derived_from_recorded_task_receipt"
        );
        let _other = owner(&store, &base);
        historical(&store, 1, 1);
        let ambiguous = store.snapshot_ownership_rows(&[base, accepted]).unwrap();
        assert_eq!(ambiguous.len(), 1);
        assert!(ambiguous[0]["task_id"].is_null());
        assert_eq!(ambiguous[0]["ownership_conflict"], true);
        assert_eq!(
            ambiguous[0]["accepted_task_ids"].as_array().unwrap().len(),
            2
        );
        assert!(store
            .local_task_receipt(&_binding.task_id)
            .unwrap()
            .is_some());
        assert!(store.local_task_receipt(&_other.task_id).unwrap().is_some());
    }

    #[test]
    fn layout_two_offsets_and_retired_flags_are_exact() {
        let row = Link {
            meta: 1,
            ordinal: 255,
            task: 0,
            change_plus1: 7,
            snapshot: 11,
            created_at_s: u64::from(u32::MAX) + 5,
        };
        let bytes = row.encode().unwrap();
        assert_eq!(bytes.len(), 22);
        assert_eq!(&bytes[2..6], &0u32.to_le_bytes());
        assert_eq!(&bytes[6..10], &7u32.to_le_bytes());
        assert_eq!(&bytes[10..14], &11u32.to_le_bytes());
        assert_eq!(&bytes[14..22], &row.created_at_s.to_le_bytes());
        assert_eq!(Link::decode(&bytes).unwrap(), row);
        for bit in [8, 16, 32, 64] {
            let mut invalid = bytes.clone();
            invalid[0] |= bit;
            assert!(Link::decode(&invalid).is_err());
        }
        let mirror = RemoteMirrorSnapshotLinkRecord {
            link: row,
            remote_meta: 15,
            fetched_at_s: u64::from(u32::MAX) + 9,
        };
        let bytes = mirror.encode().unwrap();
        assert_eq!(bytes.len(), 34);
        assert_eq!(&bytes[6..10], &0u32.to_le_bytes());
        assert_eq!(&bytes[26..34], &mirror.fetched_at_s.to_le_bytes());
        assert_eq!(
            RemoteMirrorSnapshotLinkRecord::decode(&bytes).unwrap(),
            mirror
        );
        let mut invalid = bytes;
        invalid[5] = 1;
        assert!(RemoteMirrorSnapshotLinkRecord::decode(&invalid).is_err());
    }

    #[test]
    fn snapshot_ordinal_uses_all_256_slots_without_reuse() {
        use crate::content_binary_db::BinarySnapshotCodec;
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (first, _) = seed_line_and_snapshots(&store);
        let binding = owner(&store, &first);
        let file = BinarySnapshotCodec::<1>::record_file();
        let mut tx =
            BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
        let original =
            BinarySnapshotCodec::<1>::decode_record(&tx.read_record(file.clone(), 0).unwrap())
                .unwrap();
        for i in 1..=256 {
            let id = format!("SNP-{i:012X}");
            if i > 2 {
                let mut record = original.clone();
                record.snapshot_hash48 = snapshot_hash48_from_id(&id).unwrap();
                tx.append_record(
                    file.clone(),
                    &BinarySnapshotCodec::<1>::encode_record(&record).unwrap(),
                )
                .unwrap();
            }
            store
                .record_snapshot_link_in_write(&mut tx, &binding, &id, 1)
                .unwrap();
        }
        tx.commit().unwrap();
        assert!(store
            .validate_snapshot_binding(&binding)
            .unwrap_err()
            .to_string()
            .contains("256"));
        assert_eq!(
            store
                .latest_task_snapshot_id(&binding.task_id)
                .unwrap()
                .as_deref(),
            Some("SNP-000000000100")
        );
        let rows = store.read_rows().unwrap();
        let read = BinaryDbReadTxn::new(store.db());
        let (links, tasks, _) = store.snapshot_links(&read, &rows, true).unwrap();
        assert_eq!(links.last().unwrap().0.ordinal, 255);
        assert_eq!(tasks[0].count, 256);
        assert_eq!(links.len(), 256);
        store
            .validate_replay_snapshot_capacity(&BTreeMap::from([(binding.task_id.clone(), 0)]))
            .unwrap();
        assert!(store
            .validate_replay_snapshot_capacity(&BTreeMap::from([(binding.task_id, 1)]))
            .unwrap_err()
            .to_string()
            .contains("256"));
    }
    #[test]
    fn interleaved_owners_reopen_and_skip_their_latest_tombstone() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (first, second) = seed_line_and_snapshots(&store);
        let a = owner(&store, &first);
        let b = owner(&store, &first);
        bind(&store, &a, &first).unwrap();
        bind(&store, &b, &second).unwrap();
        let third = "SNP-000000000003";
        let file = BinarySnapshotCodec::<1>::record_file();
        let mut tx =
            BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
        let mut content =
            BinarySnapshotCodec::<1>::decode_record(&tx.read_record(file.clone(), 1).unwrap())
                .unwrap();
        content.snapshot_hash48 = snapshot_hash48_from_id(third).unwrap();
        tx.append_record(
            file,
            &BinarySnapshotCodec::<1>::encode_record(&content).unwrap(),
        )
        .unwrap();
        tx.commit().unwrap();
        bind(&store, &a, third).unwrap();
        let file = BinaryDbWorkflowStore::<LocalBinaryDbFs, 1>::snapshot_link_file();
        let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::General).unwrap();
        let mut raw = tx.read_record(file.clone(), 2).unwrap();
        raw[0] |= 0x80;
        tx.overwrite_record(file, 2, &raw).unwrap();
        tx.commit().unwrap();
        let reopened = fixture(&temp);
        assert_eq!(
            reopened
                .latest_task_snapshot_id(&a.task_id)
                .unwrap()
                .as_deref(),
            Some(first.as_str())
        );
        assert_eq!(
            reopened
                .latest_change_snapshot_id(&format!("{}/C-01", a.task_id))
                .unwrap()
                .as_deref(),
            Some(first.as_str())
        );
        assert_eq!(
            reopened
                .latest_task_snapshot_id(&b.task_id)
                .unwrap()
                .as_deref(),
            Some(second.as_str())
        );
        assert!(!temp
            .path()
            .join("binary-db/snapshot_link_payload.bin")
            .exists());
    }
    #[test]
    fn numeric_change_and_content_line_survive_base_changes_and_other_task_acceptance() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (first, _) = seed_line_and_snapshots(&store);
        let a = owner(&store, &first);
        bind(&store, &a, &first).unwrap();
        let _b = owner(&store, &first);
        crate::line_binary_db::BinaryDbLineStore::<_, 1>::new(store.db().clone())
            .append_line_for_bootstrap(
                "other-base",
                "active",
                Some("2026-07-24T00:00:00Z"),
                Some("2026-07-24T00:00:00Z"),
                None,
                None,
            )
            .unwrap();
        let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::General).unwrap();
        let file = BinaryDbWorkflowStore::<LocalBinaryDbFs, 1>::change_record_file();
        let mut raw = tx.read_record(file.clone(), 0).unwrap();
        raw[56..60].copy_from_slice(&2u32.to_le_bytes());
        tx.overwrite_record(file, 0, &raw).unwrap();
        let receipt = LocalLandRecord {
            land_meta: 2 | LAND_META_HAS_LANDED,
            land_ordinal: 0,
            change_ordinal_plus1: 0,
            failure_kind: 0,
            task_index: 1,
            previous_task_land_index_plus1: 0,
            previous_change_land_index_plus1: 0,
            pre_land_target_snapshot_index_plus1: 0,
            landed_snapshot_index_plus1: 1,
            submitted_at_s: 1,
            updated_at_s: 1,
            target_line_index_plus1: 1,
        };
        tx.append_record(
            BinaryDbWorkflowStore::<LocalBinaryDbFs, 1>::land_record_file(),
            &receipt.encode().unwrap(),
        )
        .unwrap();
        tx.overwrite_record(
            BinaryDbWorkflowStore::<LocalBinaryDbFs, 1>::task_land_index_file(),
            1,
            &TaskLandIndexRecord {
                latest_land_index_plus1: 1,
                land_count: 1,
                reserved0: 0,
            }
            .encode(),
        )
        .unwrap();
        tx.commit().unwrap();
        let reopened = fixture(&temp);
        let rows = reopened.snapshot_ownership_rows(&[first]).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["task_id"], a.task_id);
        assert_eq!(rows[0]["change_id"], "C-01");
        assert_eq!(rows[0]["line_name"], "main");
        for field in ["worktree_name", "author_mode", "model_name"] {
            assert!(rows[0].get(field).is_none());
        }
    }
}
