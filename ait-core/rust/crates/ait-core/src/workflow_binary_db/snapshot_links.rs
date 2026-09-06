//! Durable local Snapshot ownership, using the protected layout-1 link records.
use super::*;

const LINK_SIZE: u32 = 40;
const INDEX_SIZE: u32 = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotWorkflowBinding {
    pub task_id: String,
    pub change_id: String,
    pub worktree_name: String,
    pub line_name: String,
    pub author_mode: String,
    pub model_name: String,
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

#[derive(Clone, Debug)]
struct Link {
    meta: u8,
    ordinal: u8,
    payload_len: u16,
    payload_offset: u64,
    task: u32,
    change_plus1: u32,
    snapshot: u32,
    previous_task: u32,
    previous_change: u32,
    created_at_s: u64,
}

impl Link {
    fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.meta, self.ordinal];
        out.extend_from_slice(&self.payload_len.to_le_bytes());
        out.extend_from_slice(&self.payload_offset.to_le_bytes());
        for value in [
            self.task,
            self.change_plus1,
            self.snapshot,
            self.previous_task,
            self.previous_change,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&self.created_at_s.to_le_bytes());
        out
    }

    fn decode(raw: &[u8]) -> StoreResult<Self> {
        require_raw_size(raw, LINK_SIZE, "Snapshot Link")?;
        if raw[0] & 0x40 != 0 {
            return Err("Snapshot Link has reserved metadata".into());
        }
        let u32_at = |offset| u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap());
        let value = Self {
            meta: raw[0],
            ordinal: raw[1],
            payload_len: u16::from_le_bytes(raw[2..4].try_into().unwrap()),
            payload_offset: u64::from_le_bytes(raw[4..12].try_into().unwrap()),
            task: u32_at(12),
            change_plus1: u32_at(16),
            snapshot: u32_at(20),
            previous_task: u32_at(24),
            previous_change: u32_at(28),
            created_at_s: u64::from_le_bytes(raw[32..40].try_into().unwrap()),
        };
        if (value.meta & 1 != 0) != (value.change_plus1 != 0) {
            return Err("Snapshot Link Change flag disagrees with its pointer".into());
        }
        Ok(value)
    }
}

impl SnapshotWorkflowBinding {
    fn encode(&self) -> StoreResult<Vec<u8>> {
        let fields = [
            &self.worktree_name,
            &self.line_name,
            &self.task_id,
            &self.change_id,
            &self.author_mode,
            &self.model_name,
        ];
        let mut out = Vec::new();
        for field in &fields[..5] {
            out.extend_from_slice(
                &u16::try_from(field.len())
                    .map_err(|_| "Snapshot Link field exceeds u16")?
                    .to_le_bytes(),
            );
        }
        for field in fields {
            out.extend_from_slice(field.as_bytes());
        }
        u16::try_from(out.len()).map_err(|_| "Snapshot Link payload exceeds u16")?;
        Ok(out)
    }

    fn decode(raw: &[u8]) -> StoreResult<Self> {
        if raw.len() < 10 {
            return Err("Snapshot Link payload is truncated".into());
        }
        let mut offset = 10;
        let mut fields = Vec::new();
        for index in 0..5 {
            let size = usize::from(u16::from_le_bytes(
                raw[index * 2..index * 2 + 2].try_into().unwrap(),
            ));
            let end = offset + size;
            let bytes = raw
                .get(offset..end)
                .ok_or("Snapshot Link payload field is truncated")?;
            fields.push(
                std::str::from_utf8(bytes)
                    .map_err(|_| "Snapshot Link payload is not UTF-8")?
                    .to_string(),
            );
            offset = end;
        }
        fields.push(
            std::str::from_utf8(&raw[offset..])
                .map_err(|_| "Snapshot Link model is not UTF-8")?
                .to_string(),
        );
        Ok(Self {
            worktree_name: fields[0].clone(),
            line_name: fields[1].clone(),
            task_id: fields[2].clone(),
            change_id: fields[3].clone(),
            author_mode: fields[4].clone(),
            model_name: fields[5].clone(),
        })
    }
}

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
        fixed_file::<L>("snapshot_link.bin", LINK_SIZE)
    }
    fn snapshot_link_payload_file() -> BinaryPayloadFileId {
        payload_file::<L>("snapshot_link_payload.bin")
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

    fn snapshot_links<A: WorkflowReadAccess>(
        &self,
        access: &A,
        rows: &WorkflowRows,
        verify_namespace: bool,
    ) -> StoreResult<SnapshotLinkInventory> {
        let mut tasks = vec![OwnerIndex::default(); rows.tasks.len()];
        let mut changes = vec![OwnerIndex::default(); rows.changes.len()];
        let count = access.record_count(Self::snapshot_link_file())?;
        let mut links = Vec::new();
        let mut seen = BTreeMap::new();
        for index in 0..count {
            let link = Link::decode(&access.read_record(Self::snapshot_link_file(), index)?)?;
            let payload = SnapshotWorkflowBinding::decode(&access.read_payload(
                Self::snapshot_link_payload_file(),
                link.payload_offset,
                u32::from(link.payload_len),
            )?)?;
            let task_id = self.local_task_id(link.task)?;
            let payload_id_matches = if verify_namespace {
                payload.task_id == task_id
            } else {
                // Detached generation admission has no repository configuration.
                // Check the encoded local namespace and exact physical identity.
                let (prefix, _) = payload
                    .task_id
                    .split_once("T-")
                    .ok_or("Invalid Snapshot Link Task ID")?;
                let namespace = prefix
                    .strip_prefix('L')
                    .ok_or("Snapshot Link has a non-local Task")?;
                let prefix = workflow_origin_namespace_prefix("L", Some(namespace))?;
                payload.task_id == format!("{prefix}T-{:04}", u64::from(link.task) + 1)
            };
            if !payload_id_matches || !rows.tasks.contains_key(&task_id) {
                return Err("Snapshot Link Task identity disagrees with its fixed owner".into());
            }
            let task = tasks
                .get_mut(link.task as usize)
                .ok_or("Snapshot Link Task is missing")?;
            if task.count >= 256
                || link.ordinal != task.count as u8
                || link.previous_task != task.latest
            {
                return Err("Snapshot Link Task chain or ordinal is inconsistent".into());
            }
            task.latest = index + 1;
            task.count += 1;
            // count=256 proves exhaustion; the last representable next slot is
            // never allocated again. No Snapshot ordinal wraps or is reused.
            task.next = if task.count == 256 {
                255
            } else {
                task.count as u8
            };
            if let Some(change_index) = link.change_plus1.checked_sub(1) {
                let change_record = LocalChangeRecord::decode(
                    &access.read_record(Self::change_record_file(), change_index)?,
                )?;
                if change_record.task_index != link.task
                    || render_change_id(change_record.change_ordinal) != payload.change_id
                {
                    return Err("Snapshot Link Change disagrees with its Task or payload".into());
                }
                let change = changes
                    .get_mut(change_index as usize)
                    .ok_or("Snapshot Link Change is missing")?;
                if link.previous_change != change.latest {
                    return Err("Snapshot Link Change chain is inconsistent".into());
                }
                change.latest = index + 1;
                change.count = change
                    .count
                    .checked_add(1)
                    .ok_or("Snapshot Link Change count overflow")?;
            } else if link.previous_change != 0 || !payload.change_id.is_empty() {
                return Err("Snapshot Link has Change data without a Change".into());
            }
            let snapshot_id = snapshot_id_at(access, link.snapshot)?;
            if link.meta & 0x80 == 0 && seen.insert(snapshot_id, index).is_some() {
                return Err("Snapshot has multiple live ownership links".into());
            }
            if (link.meta & 8 != 0) == payload.worktree_name.is_empty()
                || (link.meta & 16 != 0) == payload.line_name.is_empty()
                || (link.meta & 32 != 0)
                    != (!payload.author_mode.is_empty() || !payload.model_name.is_empty())
            {
                return Err("Snapshot Link flags disagree with its payload".into());
            }
            links.push((link, payload));
        }
        for (is_task, expected) in [(true, &tasks), (false, &changes)] {
            let file = Self::snapshot_owner_file(is_task);
            let actual_count = access.record_count(file.clone())?;
            // Older generations have no link families. A shorter aligned
            // inventory is only valid for the newly appended owners with no links.
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
                    return Err("Snapshot owner index disagrees with its durable chain".into());
                }
            }
        }
        Ok((links, tasks, changes))
    }

    fn validate_snapshot_owner(
        &self,
        rows: &WorkflowRows,
        binding: &SnapshotWorkflowBinding,
    ) -> PlanStoreResult<(u32, u32)> {
        let task_index = self.task_index_for_id(rows, &binding.task_id)?;
        let change_ref = format!("{}/{}", binding.task_id, binding.change_id);
        let change_index = self.change_index_for_id(rows, &change_ref)?;
        let task = &rows.tasks[&binding.task_id];
        let change = &rows.changes[&change_ref];
        if task["status"].as_str() != Some("active") {
            return Err(PlanStoreError::Invalid(
                "Snapshot authoring requires an active Task".to_string(),
            ));
        }
        if !matches!(
            change["status"].as_str(),
            Some("draft" | "active" | "review")
        ) || change["task_id"].as_str() != Some(binding.task_id.as_str())
        {
            return Err(PlanStoreError::Invalid(
                "Snapshot authoring requires a writable Change owned by the Task".to_string(),
            ));
        }
        if binding.worktree_name.trim().is_empty() || binding.line_name.trim().is_empty() {
            return Err(PlanStoreError::Invalid(
                "Snapshot authoring requires a bound worktree and Line".to_string(),
            ));
        }
        binding.encode().map_err(storage_error)?;
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
        let (links, mut tasks, mut changes) = self.snapshot_links(tx, &rows, true)?;
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
        let change = &mut changes[change_index as usize];
        let payload = binding.encode()?;
        let locator = tx.append_payload(Self::snapshot_link_payload_file(), &payload)?;
        let link = Link {
            meta: 1
                | 8
                | 16
                | if binding.author_mode.is_empty() && binding.model_name.is_empty() {
                    0
                } else {
                    32
                },
            ordinal,
            payload_len: payload.len() as u16,
            payload_offset: locator.payload_offset,
            task: task_index,
            change_plus1: change_index + 1,
            snapshot: snapshot_index,
            previous_task: task.latest,
            previous_change: change.latest,
            created_at_s,
        };
        let index = tx.append_record(Self::snapshot_link_file(), &link.encode())?;
        task.latest = index + 1;
        task.count += 1;
        task.next = if task.count == 256 {
            255
        } else {
            task.count as u8
        };
        change.latest = index + 1;
        change.count = change
            .count
            .checked_add(1)
            .ok_or("Snapshot Link Change count overflow")?;
        for (is_task, owners, modified) in
            [(true, tasks, task_index), (false, changes, change_index)]
        {
            let file = Self::snapshot_owner_file(is_task);
            let old_count = tx.record_count(file.clone())?;
            for (index, owner) in owners.iter().enumerate() {
                if index >= old_count as usize {
                    tx.append_record(file.clone(), &owner.encode(is_task))?;
                } else if index == modified as usize {
                    tx.overwrite_record(file.clone(), index as u32, &owner.encode(is_task))?;
                }
            }
        }
        Ok(())
    }

    /// Resolve the latest live checkpoint of this exact Task-owned Change.
    /// The declared owner chain supplies ordering, including after worktree cleanup.
    pub fn latest_change_snapshot_id(&self, change_ref: &str) -> PlanStoreResult<Option<String>> {
        let read = BinaryDbReadTxn::new(&self.db);
        let rows = self.read_rows_with_access(&read).map_err(storage_error)?;
        let change_index = self.change_index_for_id(&rows, change_ref)?;
        let (links, _, changes) = self
            .snapshot_links(&read, &rows, true)
            .map_err(storage_error)?;
        let mut cursor = changes[change_index as usize].latest;
        while let Some(index) = cursor.checked_sub(1) {
            let (link, _) = &links[index as usize];
            if link.meta & 0x80 == 0 {
                return snapshot_id_at(&read, link.snapshot)
                    .map(Some)
                    .map_err(storage_error);
            }
            cursor = link.previous_change;
        }
        Ok(None)
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
                "change_id": binding.change_id, "worktree_name": nonempty(Some(&binding.worktree_name)),
                "line_name": nonempty(Some(&binding.line_name)), "author_mode": nonempty(Some(&binding.author_mode)),
                "model_name": nonempty(Some(&binding.model_name)), "created_at": link.created_at_s.to_string(),
                "ownership_source": "durable_snapshot_binding",
            }));
        }
        // A legacy accepted boundary is evidence for exactly that Snapshot,
        // never for every ancestor or an inferred Task-shaped Line name.
        for snapshot_id in snapshot_ids {
            if output
                .iter()
                .any(|row| row["snapshot_id"].as_str() == Some(snapshot_id))
            {
                continue;
            }
            let owners = rows
                .changes
                .values()
                .filter(|change| change["landed_snapshot_id"].as_str() == Some(snapshot_id))
                .collect::<Vec<_>>();
            if owners.len() == 1 {
                output.push(json!({
                    "snapshot_id": snapshot_id, "task_id": owners[0]["task_id"],
                    "change_id": owners[0]["change_id"],
                    "ownership_source": "derived_from_recorded_change",
                }));
            }
        }
        Ok(output)
    }
}

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
            worktree_name: "owned".to_string(),
            line_name: "main".to_string(),
            author_mode: "ai_with_human_review".to_string(),
            model_name: "模型".to_string(),
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
            4 + 2 * 40
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
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (base, accepted) = seed_line_and_snapshots(&store);
        let binding = owner(&store, &base);
        ChangeStore::land_change(
            &store,
            &format!("{}/C-01", binding.task_id),
            "main",
            &accepted,
            Some(&base),
        )
        .unwrap();
        let rows = store
            .snapshot_ownership_rows(&[base.clone(), accepted.clone()])
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["snapshot_id"], accepted);
        assert_eq!(rows[0]["ownership_source"], "derived_from_recorded_change");
        let other = owner(&store, &base);
        ChangeStore::land_change(
            &store,
            &format!("{}/C-01", other.task_id),
            "main",
            &accepted,
            Some(&base),
        )
        .unwrap();
        assert!(store
            .snapshot_ownership_rows(&[base, accepted])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn payload_bounds_and_reserved_bits_are_rejected() {
        let value = SnapshotWorkflowBinding {
            task_id: "LCT-0001".into(),
            change_id: "C-01".into(),
            worktree_name: "修復".into(),
            line_name: "feature/repair".into(),
            author_mode: "ai_with_human_review".into(),
            model_name: "模型".into(),
        };
        assert_eq!(
            SnapshotWorkflowBinding::decode(&value.encode().unwrap()).unwrap(),
            value
        );
        assert!(SnapshotWorkflowBinding::decode(&[0; 9]).is_err());
        let mut invalid = value.clone();
        invalid.model_name = "x".repeat(65_536);
        assert!(invalid.encode().is_err());
        let mut raw = vec![0; 40];
        raw[0] = 0x40;
        assert!(Link::decode(&raw).is_err());
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
        let rows = store.read_rows().unwrap();
        let read = BinaryDbReadTxn::new(store.db());
        let (links, tasks, _) = store.snapshot_links(&read, &rows, true).unwrap();
        assert_eq!(links.last().unwrap().0.ordinal, 255);
        assert_eq!(tasks[0].count, 256);
        assert_eq!(links.len(), 256);
    }
}
