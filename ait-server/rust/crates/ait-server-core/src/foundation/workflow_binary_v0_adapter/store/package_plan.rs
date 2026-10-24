//! Required conversion, shared by pre-CI reads and the locked publication.
//! Future numeric keys are symbolic allocations, never reservations or records.
use super::*;
use crate::foundation::server_content_binary_db::{
    server_line_name_hash64, server_snapshot_hash48_from_id,
};
use crate::foundation::workflow_package::projected_package_line;

pub(super) struct ResolvedChange {
    pub existing: Option<WorkflowOwner>,
    pub base: String,
    pub fork: String,
}
pub(super) struct ResolvedEntry {
    pub task_index: u32,
    pub receipt: Option<(WorkflowOwner, u32, String, JsonValue)>,
    pub changes: Vec<ResolvedChange>,
}
pub(super) struct PackageConversion {
    pub entries: Vec<ResolvedEntry>,
    pub owners: BTreeMap<String, WorkflowOwner>,
    pub attestation_slots: usize,
}

#[derive(Clone)]
struct Snapshot {
    index: Option<u32>,
    parents: Vec<String>,
    created_at_s: u64,
}
struct Snapshots<'a, A> {
    read: &'a A,
    incoming: BTreeMap<String, &'a JsonValue>,
    loaded: BTreeMap<String, Snapshot>,
    edges: Option<BTreeMap<u32, Vec<(u16, u32)>>>,
}
impl<'a, A: ReadV0> Snapshots<'a, A> {
    fn get(&mut self, id: &str) -> Result<Snapshot, String> {
        if let Some(value) = self.loaded.get(id) {
            return Ok(value.clone());
        }
        let key = server_snapshot_hash48_from_id(id)
            .map_err(|e| e.to_string())?
            .to_le_bytes();
        // The existing ID index uses the eight-byte little-endian hash key.
        let mut stored = None;
        for index in self
            .read
            .lookup_index_v0(ServerBinarySnapshotCodec::<2>::id_index(), &key)
            .map_err(|e| e.to_string())?
        {
            let record = ServerBinarySnapshotCodec::<2>::decode_record(
                &self
                    .read
                    .read_record_v0(ServerBinarySnapshotCodec::<2>::record_file(), index)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if server_snapshot_id_from_hash48(record.snapshot_hash48) != id {
                continue;
            }
            if record.is_tombstone() {
                return Err(format!("Snapshot {id} is retired"));
            }
            if stored.replace((index, record)).is_some() {
                return Err(format!("Snapshot {id} is ambiguous"));
            }
        }
        let parents = if let Some((index, record)) = &stored {
            if record.has_parent_edges_authority() {
                if self.edges.is_none() {
                    let file = ServerBinarySnapshotParentEdgeCodec::<1>::record_file();
                    let mut edges = BTreeMap::<u32, Vec<(u16, u32)>>::new();
                    for i in 0..self
                        .read
                        .record_count_v0(file.clone())
                        .map_err(|e| e.to_string())?
                    {
                        let edge = ServerBinarySnapshotParentEdgeCodec::<1>::decode_record(
                            &self
                                .read
                                .read_record_v0(file.clone(), i)
                                .map_err(|e| e.to_string())?,
                        )
                        .map_err(|e| e.to_string())?;
                        edges
                            .entry(edge.child_snapshot_index)
                            .or_default()
                            .push((edge.parent_ordinal, edge.parent_snapshot_index));
                    }
                    self.edges = Some(edges);
                }
                let mut edges = self
                    .edges
                    .as_ref()
                    .expect("loaded edges")
                    .get(index)
                    .cloned()
                    .unwrap_or_default();
                edges.sort_by_key(|(ordinal, _)| *ordinal);
                if edges
                    .iter()
                    .enumerate()
                    .any(|(i, (ordinal, _))| i != usize::from(*ordinal))
                    || edges.first().map(|(_, p)| *p)
                        != record.parent_snapshot_index_plus1.checked_sub(1)
                {
                    return Err(format!("Snapshot {id} ordered parents conflict"));
                }
                edges
                    .into_iter()
                    .map(|(_, p)| snapshot_id(self.read, p))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                record
                    .parent_snapshot_index_plus1
                    .checked_sub(1)
                    .map(|p| snapshot_id(self.read, p))
                    .transpose()?
                    .into_iter()
                    .collect()
            }
        } else {
            Vec::new()
        };
        let incoming = self.incoming.get(id);
        let (parents, created_at_s) = if let Some(row) = incoming {
            let incoming_parents = json_parents(row)?;
            let time = row["created_at"]
                .as_str()
                .ok_or("Source Snapshot time missing")?;
            let time = chrono::DateTime::parse_from_rfc3339(time)
                .map_err(|e| e.to_string())?
                .timestamp();
            let time = u64::try_from(time).map_err(|_| "Source Snapshot time is negative")?;
            if stored.as_ref().is_some_and(|(_, r)| r.created_at_s != time)
                || stored.is_some() && parents != incoming_parents
            {
                return Err(format!("Snapshot {id} immutable time/parents differ"));
            }
            (incoming_parents, time)
        } else if let Some((_, record)) = &stored {
            (parents, record.created_at_s)
        } else {
            return Err(format!("Required Snapshot {id} is missing"));
        };
        let value = Snapshot {
            index: stored.map(|(i, _)| i),
            parents,
            created_at_s,
        };
        self.loaded.insert(id.into(), value.clone());
        Ok(value)
    }
    fn ancestors(&mut self, id: &str) -> Result<BTreeSet<String>, String> {
        let mut result = BTreeSet::new();
        let mut active = BTreeSet::new();
        let mut pending = vec![(id.to_string(), false)];
        while let Some((id, exiting)) = pending.pop() {
            if exiting {
                active.remove(&id);
                result.insert(id);
                continue;
            }
            if result.contains(&id) {
                continue;
            }
            if !active.insert(id.clone()) {
                return Err("Package Snapshot ancestry is cyclic".into());
            }
            let snapshot = self.get(&id)?;
            pending.push((id, true));
            for parent in snapshot.parents.into_iter().rev() {
                pending.push((parent, false));
            }
        }
        Ok(result)
    }
}
fn snapshot_id(read: &impl ReadV0, index: u32) -> Result<String, String> {
    let record = ServerBinarySnapshotCodec::<2>::decode_record(
        &read
            .read_record_v0(ServerBinarySnapshotCodec::<2>::record_file(), index)
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if record.is_tombstone() {
        return Err("Required parent Snapshot is retired".into());
    }
    Ok(server_snapshot_id_from_hash48(record.snapshot_hash48))
}
fn json_parents(row: &JsonValue) -> Result<Vec<String>, String> {
    if let Some(parents) = row.get("parent_snapshot_ids") {
        return parents
            .as_array()
            .ok_or("Snapshot parents must be an array")?
            .iter()
            .map(|p| {
                p.as_str()
                    .map(str::to_string)
                    .ok_or("Snapshot parent must be text".into())
            })
            .collect();
    }
    Ok(row
        .get("parent_snapshot_id")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .into_iter()
        .collect())
}
fn line(
    read: &impl ReadV0,
    name: &str,
) -> Result<
    Option<(
        u32,
        crate::foundation::server_content_binary_db::ServerBinaryLineRecord,
    )>,
    String,
> {
    let mut found = None;
    for i in read
        .lookup_index_v0(
            ServerBinaryLineCodec::<1>::name_index(),
            &server_line_name_hash64(name.as_bytes()).to_le_bytes(),
        )
        .map_err(|e| e.to_string())?
    {
        let r = ServerBinaryLineCodec::<1>::decode_record(
            &read
                .read_record_v0(ServerBinaryLineCodec::<1>::record_file(), i)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let bytes = read
            .read_payload_v0(
                ServerBinaryLineCodec::<1>::payload_file(),
                r.line_name_offset,
                u32::from(r.line_name_len),
            )
            .map_err(|e| e.to_string())?;
        if bytes != name.as_bytes() {
            continue;
        }
        if r.is_tombstone() || found.replace((i, r)).is_some() {
            return Err(format!("Projected Line {name} is retired or ambiguous"));
        }
    }
    Ok(found)
}
fn ordinal(
    read: &impl ReadV0,
    task: u32,
    path: &'static str,
) -> Result<V0OrdinalIndexRecord, String> {
    let bytes = read
        .read_record_v0(WorkflowBinaryV0Codec::chain_index_file(path), task)
        .map_err(|e| e.to_string())?;
    if path == "task_snapshot_index.bin" {
        WorkflowBinaryV0Codec::decode_task_snapshot_index(&bytes).map_err(|e| e.to_string())
    } else {
        WorkflowBinaryV0Codec::decode_ordinal_index(&bytes).map_err(|e| e.to_string())
    }
}
fn same_owner(a: WorkflowOwner, b: WorkflowOwner) -> bool {
    a.task_index == b.task_index && a.change.map(|(i, _)| i) == b.change.map(|(i, _)| i)
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub fn preflight_workflow_package_history(
        &self,
        manifest: &WorkflowPackageManifest,
        identity: &PackageExpectedIdentity,
    ) -> Result<usize, String> {
        let prepared = self.prepare_workflow_package_history(manifest, identity)?;
        Ok(self
            .convert_package(&BinaryDbReadTxn::new(&self.db), &prepared)?
            .attestation_slots)
    }

    pub(super) fn convert_package<A: ReadV0>(
        &self,
        read: &A,
        prepared: &PreparedWorkflowPackageHistory,
    ) -> Result<PackageConversion, String> {
        let package = &prepared.manifest;
        if package.base_snapshot_id == package.revision_snapshot_id {
            return Err("Final no-op requires exact existing formal readback; new CI cannot publish an existing head".into());
        }
        let mut snapshots = Snapshots {
            read,
            incoming: package
                .content
                .snapshots
                .iter()
                .map(|row| {
                    Ok((
                        row["snapshot_id"]
                            .as_str()
                            .ok_or("Incoming Snapshot ID missing")?
                            .to_string(),
                        row,
                    ))
                })
                .collect::<Result<_, String>>()?,
            loaded: BTreeMap::new(),
            edges: None,
        };
        let mut errors = prepared.binding_errors.clone();
        let mut selected = BTreeMap::new();
        // Decode only required source content; unrelated Snapshot records are not loaded.
        for entry in &package.history_entries {
            for snapshot in &entry.snapshots {
                match snapshots.get(&snapshot.snapshot_id) {
                    Ok(value) if value.created_at_s == snapshot.created_at_s => {
                        if let Some(index) = value.index {
                            selected.insert(index, snapshot.snapshot_id.clone());
                        }
                    }
                    Ok(_) => errors.push(format!(
                        "{}: immutable Snapshot time differs",
                        entry.local_task_id
                    )),
                    Err(error) => errors.push(error),
                }
            }
        }
        if let Ok(snapshot) = snapshots.get(&package.revision_snapshot_id) {
            if let Some(index) = snapshot.index {
                selected.insert(index, package.revision_snapshot_id.clone());
            }
        }
        // There is no content-owner secondary index in v0. One required family
        // traversal resolves all selected owners, rather than a proof per Snapshot.
        let mut owners = BTreeMap::new();
        let mut selected_links = BTreeMap::new();
        let mut owner_counts = BTreeMap::<u32, usize>::new();
        for i in 0..read
            .record_count_v0(WorkflowBinaryV0Codec::snapshot_link_file())
            .map_err(|e| e.to_string())?
        {
            let link = WorkflowBinaryV0Codec::decode_snapshot_link(
                &read
                    .read_record_v0(WorkflowBinaryV0Codec::snapshot_link_file(), i)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            *owner_counts.entry(link.task_index).or_default() += 1;
            let Some(id) = selected.get(&link.content_snapshot_index) else {
                continue;
            };
            selected_links.insert(link.content_snapshot_index, link);
            let owner = if let Some(change) = link.change_index_plus1.checked_sub(1) {
                let record = self.read_change(read, change).map_err(|e| e.to_string())?;
                if record.task_index != link.task_index || record.remote_meta & 1 != 0 {
                    return Err("Source numeric Change owner is retired or inconsistent".into());
                }
                WorkflowOwner {
                    task_index: link.task_index,
                    change: Some((change, record)),
                }
            } else {
                self.workflow_owner(read, &self.task_id(link.task_index))?
            };
            if link.link_meta & SNAPSHOT_LINK_TOMBSTONE != 0
                || owners.insert(id.clone(), owner).is_some()
                || snapshots.get(id)?.created_at_s != link.created_at_s
            {
                return Err(
                    "Source numeric Snapshot owner is duplicate, retired or inconsistent".into(),
                );
            }
        }
        let mut next_task = read
            .record_count_v0(WorkflowBinaryV0Codec::task_file())
            .map_err(|e| e.to_string())?;
        let longest_task = format!("{}T-{}", self.task_prefix, u32::MAX);
        let mut entries = Vec::new();
        let mut plan_owners = BTreeMap::new();
        let mut task_owners = BTreeSet::new();
        let mut previous_receipt = None;
        let mut attestation_slots = 64;
        let mut next_patchset = read
            .record_count_v0(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|e| e.to_string())?;
        let mut receipt_rows = Vec::new();
        for (position, (entry, &binding)) in package
            .history_entries
            .iter()
            .zip(&prepared.bindings)
            .enumerate()
        {
            let result = (|| -> Result<ResolvedEntry, String> {
                let task_payload = V0TaskPayload {
                    title: Self::required_text(&entry.task, "title")?,
                    intent: Self::required_text(&entry.task, "intent")?,
                };
                u16::try_from(
                    WorkflowBinaryV0Codec::encode_task_payload(&task_payload)
                        .map_err(|e| e.to_string())?
                        .len(),
                )
                .map_err(|_| "Task payload exceeds u16 byte capacity")?;
                let mut known = entry
                    .expected_remote_task_id
                    .as_deref()
                    .map(|id| self.task_index_for_id(read, id))
                    .transpose()?;
                for snapshot in &entry.snapshots {
                    if let Some(owner) = owners.get(&snapshot.snapshot_id) {
                        if known.is_some_and(|task| task != owner.task_index) {
                            return Err("Source Task exact numeric owners conflict".into());
                        }
                        known = Some(owner.task_index);
                    }
                }
                if known.is_none() && entry.snapshots.is_empty() {
                    return Err("Source Task has no Snapshot anchor or exact mapping".into());
                }
                let rows = entry.snapshots.iter().map(|s| Ok(json!({"snapshot_id":s.snapshot_id,"parent_snapshot_ids":snapshots.get(&s.snapshot_id)?.parents}))).collect::<Result<Vec<_>, String>>()?;
                crate::foundation::workflow_package::validate_snapshot_order(&rows)?;
                let original_base = snapshots.get(&entry.pre_land_target_snapshot_id)?;
                let original_revision = snapshots.get(&entry.landed_snapshot_id)?;
                let original_scope = snapshots.ancestors(&entry.landed_snapshot_id)?;
                if !original_scope.contains(&entry.pre_land_target_snapshot_id) {
                    return Err("Original receipt endpoints do not form a content chain".into());
                }
                let receipt = known
                    .map(|task| {
                        self.package_reusable_receipt(
                            read,
                            entry,
                            binding,
                            &package.target_line,
                            (
                                original_base
                                    .index
                                    .ok_or("Published original base is missing")?,
                                original_revision
                                    .index
                                    .ok_or("Published original revision is missing")?,
                            ),
                            task,
                            &selected_links,
                        )
                    })
                    .transpose()?
                    .flatten();
                if known.is_some() && receipt.is_none() {
                    return Err("Known Remote Task has no exact original Local receipt".into());
                }
                let task_index = if let Some(task) = known {
                    task
                } else {
                    let task = next_task;
                    next_task = next_task
                        .checked_add(1)
                        .ok_or("Task u32 capacity exhausted")?;
                    task
                };
                if !task_owners.insert(task_index) {
                    return Err("Distinct source Tasks alias one Remote Task".into());
                }
                if binding.0 != 0
                    && (plan_owners
                        .insert((binding.0, binding.1), task_index)
                        .is_some_and(|old| old != task_index)
                        || self
                            .task_for_plan_binding_in_write(read, binding.0, binding.1)?
                            .is_some_and(|old| old != task_index))
                {
                    return Err("Source Plan binding already owns another Task".into());
                }
                let final_entry = position + 1 == package.history_entries.len();
                let terminal = known
                    .map(|task| {
                        self.read_task(read, task)
                            .map(|r| r.is_terminal())
                            .map_err(|e| e.to_string())
                    })
                    .transpose()?
                    .unwrap_or(false);
                if final_entry && terminal {
                    return Err("Final Remote Task is terminal; whole published batch requires exact readback, not new CI/finish".into());
                }
                let mut changes = Vec::new();
                let mut used_changes = BTreeSet::new();
                let mut proven_lines = BTreeSet::new();
                for change in &entry.changes {
                    let title = Self::required_text(&change.change, "title")?;
                    u16::try_from(
                        WorkflowBinaryV0Codec::encode_single_text_payload(&title, "Change title")
                            .map_err(|e| e.to_string())?
                            .len(),
                    )
                    .map_err(|_| "Change payload exceeds u16 byte capacity")?;
                    let source_base = Self::required_text(&change.change, "base_line")?;
                    projected_package_line(&longest_task, &source_base)?;
                    let base = projected_package_line(&self.task_id(task_index), &source_base)?;
                    let fork = Self::required_text(&change.change, "fork_snapshot_id")?;
                    let fork_snapshot = snapshots.get(&fork)?;
                    let mut owner = change
                        .expected_remote_change_ref
                        .as_deref()
                        .map(|id| self.workflow_owner(read, id))
                        .transpose()?;
                    let mut anchored = false;
                    // Native Local rebase preserves Change provenance and
                    // its creation fork while moving Snapshot ancestry.
                    for snapshot in entry
                        .snapshots
                        .iter()
                        .filter(|s| s.local_change_ref.as_deref() == Some(&change.local_change_ref))
                    {
                        anchored = true;
                        if let Some(actual) = owners.get(&snapshot.snapshot_id) {
                            if owner.is_some_and(|old| !same_owner(old, *actual)) {
                                return Err("Source Change exact Snapshot owners conflict".into());
                            }
                            owner = Some(*actual);
                        }
                    }
                    if let Some(owner) = owner {
                        if owner.task_index != task_index || owner.change.is_none() {
                            return Err("Source Change mapped outside its exact Task/Change".into());
                        }
                        let (index, record) = owner.change.expect("checked owner");
                        let stored_title = WorkflowBinaryV0Codec::decode_single_text_payload(
                            &read
                                .read_payload_v0(
                                    WorkflowBinaryV0Codec::change_payload_file(),
                                    record.payload_offset,
                                    u32::from(record.payload_len),
                                )
                                .map_err(|e| e.to_string())?,
                            "Change title",
                        )
                        .map_err(|e| e.to_string())?
                        .to_string();
                        let projected = line(read, &base)?.ok_or("Mapped Change has no proven projected base; legacy association is unknown")?;
                        if stored_title != title
                            || record.base_line_index_plus1 != projected.0 + 1
                            || Some(record.fork_snapshot_index_plus1)
                                != fork_snapshot.index.and_then(|i| i.checked_add(1))
                        {
                            return Err("Mapped Change immutable title/base/fork differs; legacy association is unknown".into());
                        }
                        if !used_changes.insert(index) {
                            return Err("Distinct source Changes alias one Remote Change".into());
                        }
                        proven_lines.insert(base.clone());
                    } else if !anchored {
                        return Err("New source Change has no owned Snapshot anchor".into());
                    }
                    changes.push(ResolvedChange {
                        existing: owner,
                        base,
                        fork,
                    });
                }
                let mapped_changes = entry
                    .changes
                    .iter()
                    .zip(&changes)
                    .filter_map(|(source, resolved)| {
                        Some((
                            source.local_change_ref.as_str(),
                            resolved.existing?.change?.0,
                        ))
                    })
                    .collect::<BTreeMap<_, _>>();
                for snapshot in &entry.snapshots {
                    let Some(actual) = owners.get(&snapshot.snapshot_id) else {
                        continue;
                    };
                    let expected_change = snapshot
                        .local_change_ref
                        .as_deref()
                        .map(|reference| {
                            mapped_changes
                                .get(reference)
                                .copied()
                                .ok_or("Stored Snapshot requires its exact mapped Change owner")
                        })
                        .transpose()?;
                    if actual.task_index != task_index
                        || actual.change.map(|(index, _)| index) != expected_change
                    {
                        return Err("Stored Snapshot exact Task/Change owner differs".into());
                    }
                }
                let new_changes = changes.iter().filter(|c| c.existing.is_none()).count();
                if terminal && new_changes != 0 {
                    return Err("Terminal Remote Task cannot accept new Changes".into());
                }
                for change in &changes {
                    if line(read, &change.base)?.is_some() && !proven_lines.contains(&change.base) {
                        return Err(
                            "Projected Line name collision lacks exact mapped Change proof".into(),
                        );
                    }
                }
                let allocated_changes = known
                    .map(|t| {
                        ordinal(read, t, "task_change_index.bin")
                            .map(|r| usize::from(r.next_ordinal))
                    })
                    .transpose()?
                    .unwrap_or(0);
                if allocated_changes + new_changes > 64 {
                    return Err("Change64/Task capacity exhausted".into());
                }
                let allocated_patchsets = known
                    .map(|t| {
                        self.allocated_task_patchset_ordinals(read, t)
                            .map(usize::from)
                    })
                    .transpose()?
                    .unwrap_or(0);
                let new_patchsets = usize::from(receipt.is_none()) + usize::from(final_entry);
                if allocated_patchsets + new_patchsets > 64 {
                    return Err("Patchset64/Task capacity exhausted".into());
                }
                let new_links = entry
                    .snapshots
                    .iter()
                    .filter(|s| !owners.contains_key(&s.snapshot_id))
                    .count();
                let allocated_links = known
                    .map(|t| {
                        ordinal(read, t, "task_snapshot_index.bin")
                            .map(|r| usize::from(r.next_ordinal))
                    })
                    .transpose()?
                    .unwrap_or(0);
                if allocated_links.max(*owner_counts.get(&task_index).unwrap_or(&0)) + new_links
                    > 256
                {
                    return Err("Snapshot-owner256/Task capacity exhausted".into());
                }
                if final_entry {
                    let lands = known
                        .map(|t| {
                            self.task_inventory(read, "task_land_index.bin", t)
                                .map(|r| usize::from(r.count))
                        })
                        .transpose()?
                        .unwrap_or(0);
                    if lands >= 64 {
                        return Err("Land64/Task capacity exhausted".into());
                    }
                    let attests = known
                        .map(|t| {
                            ordinal(read, t, "task_attest_index.bin")
                                .map(|r| usize::from(r.next_ordinal))
                        })
                        .transpose()?
                        .unwrap_or(0);
                    attestation_slots = 64usize
                        .checked_sub(attests)
                        .ok_or("Attestation64/Task capacity exhausted")?;
                    if attestation_slots == 0 {
                        return Err("Attestation64/Task capacity exhausted".into());
                    }
                    if !entry
                        .snapshots
                        .iter()
                        .any(|s| s.snapshot_id == package.revision_snapshot_id)
                        && owners
                            .get(&package.revision_snapshot_id)
                            .is_none_or(|o| o.task_index != task_index)
                    {
                        return Err("Final aggregate Snapshot has no exact final source Task ownership; no-op form is unsupported".into());
                    }
                }
                let task_id = self.task_id(task_index);
                let local = json!({"contract":"ait-local-land-receipt/v2","local_task_id":entry.local_task_id,"local_change_id":null,"local_change_ref":null,"task_id":task_id,"change_ref":null,"target_line":package.target_line,"pre_land_target_snapshot_id":entry.pre_land_target_snapshot_id,"landed_snapshot_id":entry.landed_snapshot_id,"landed_at_s":entry.landed_at_s,"source_kind":"imported_local_land_receipt"});
                let (receipt_owner, receipt_index, receipt_id, local) =
                    if let Some((owner, index, id, local)) = &receipt {
                        (*owner, *index, id.clone(), local.clone())
                    } else {
                        u16::try_from(
                            WorkflowBinaryV0Codec::encode_single_text_payload(
                                &format!(
                                    "{LOCAL_LAND_RECEIPT_SUMMARY_PREFIX}{}",
                                    serde_json::to_string(&local).map_err(|e| e.to_string())?
                                ),
                                "Patchset summary",
                            )
                            .map_err(|e| e.to_string())?
                            .len(),
                        )
                        .map_err(|_| "Source receipt summary exceeds u16 byte capacity")?;
                        let index = next_patchset;
                        next_patchset = next_patchset
                            .checked_add(1)
                            .ok_or("Patchset physical u32 capacity exhausted")?;
                        (
                            WorkflowOwner {
                                task_index,
                                change: None,
                            },
                            index,
                            format!("{task_id}/P-01"),
                            local,
                        )
                    };
                receipt_rows.push(json!({"local_task_id":entry.local_task_id,"local_change_id":local["local_change_id"],"local_change_ref":local["local_change_ref"],"task_index":task_index,"task_id":task_id,"change_index":receipt_owner.change.map(|(i,_)|i),"change_ref":receipt_owner.change.map(|(_,r)|self.change_ref(task_index,r.change_ordinal)),"receipt_patchset_index":receipt_index,"receipt_patchset_id":receipt_id,"pre_land_target_snapshot_id":entry.pre_land_target_snapshot_id,"landed_snapshot_id":entry.landed_snapshot_id,"landed_at_s":entry.landed_at_s,"snapshot_link_count":entry.snapshots.len()}));
                Ok(ResolvedEntry {
                    task_index,
                    receipt,
                    changes,
                })
            })();
            if previous_receipt
                .is_some_and(|previous: &String| previous != &entry.pre_land_target_snapshot_id)
            {
                errors.push("Original Local receipts are discontinuous".into());
            }
            previous_receipt = Some(&entry.landed_snapshot_id);
            match result {
                Ok(entry) => entries.push(entry),
                Err(e) => errors.push(format!("{}: {e}", entry.local_task_id)),
            }
        }
        // Replay is a single-parent chain. Original metadata above may have
        // several ordered parents, without flattening or changing authoring data.
        let replay = (|| -> Result<(), String> {
            let base = snapshots.ancestors(&package.base_snapshot_id)?;
            if !snapshots
                .ancestors(&package.revision_snapshot_id)?
                .contains(&package.base_snapshot_id)
            {
                return Err("Tested revision does not descend from package base".into());
            }
            let source = package
                .history_entries
                .iter()
                .flat_map(|e| e.snapshots.iter().map(|s| s.snapshot_id.as_str()))
                .collect::<BTreeSet<_>>();
            let mut cursor = package.revision_snapshot_id.clone();
            let mut seen = BTreeSet::new();
            while !base.contains(&cursor) {
                if !seen.insert(cursor.clone())
                    || (!source.contains(cursor.as_str())
                        && !owners
                            .get(&cursor)
                            .is_some_and(|owner| task_owners.contains(&owner.task_index)))
                {
                    return Err("Tested replay lacks explicit source ownership".into());
                }
                let parents = snapshots.get(&cursor)?.parents;
                if parents.len() != 1 {
                    return Err("Tested replay requires a single-parent chain".into());
                }
                cursor = parents[0].clone();
            }
            let target =
                line(read, package.target_line.trim())?.ok_or("Package target Line is missing")?;
            if target.1.is_archived()
                || target
                    .1
                    .head_snapshot_index()
                    .map(|i| snapshot_id(read, i))
                    .transpose()?
                    .as_deref()
                    != Some(&package.base_snapshot_id)
            {
                return Err(
                    "STALE_TARGET_LINE: tested package base moved or target archived".into(),
                );
            }
            Ok(())
        })();
        if let Err(e) = replay {
            errors.push(e);
        }
        let new_changes = entries
            .iter()
            .flat_map(|entry| &entry.changes)
            .filter(|change| change.existing.is_none())
            .count();
        let new_links = package
            .history_entries
            .iter()
            .flat_map(|entry| &entry.snapshots)
            .filter(|snapshot| !owners.contains_key(&snapshot.snapshot_id))
            .count();
        let new_lines = entries
            .iter()
            .flat_map(|entry| &entry.changes)
            .filter(|change| change.existing.is_none())
            .map(|change| change.base.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .try_fold(0u32, |count, name| {
                Ok::<_, String>(count + u32::from(line(read, name)?.is_none()))
            })?;
        for (file, added, name) in [
            (WorkflowBinaryV0Codec::change_file(), new_changes, "Change"),
            (
                WorkflowBinaryV0Codec::snapshot_link_file(),
                new_links,
                "Snapshot owner",
            ),
            (
                ServerBinaryLineCodec::<1>::record_file(),
                new_lines as usize,
                "Line",
            ),
        ] {
            if read
                .record_count_v0(file)
                .map_err(|e| e.to_string())?
                .checked_add(
                    u32::try_from(added)
                        .map_err(|_| "Package physical record count exceeds u32")?,
                )
                .is_none()
            {
                errors.push(format!("{name} physical u32 capacity exhausted"));
            }
        }
        if next_patchset.checked_add(1).is_none() {
            errors.push("Final Patchset physical u32 capacity exhausted".into());
        }
        if let Some(owner) = entries
            .last()
            .filter(|_| entries.len() == package.history_entries.len())
        {
            let ordinal = if owner.receipt.is_some() {
                self.next_task_patchset_ordinal(read, owner.task_index)?
            } else {
                1
            };
            let task_id = self.task_id(owner.task_index);
            let patchset_id = format!("{task_id}/P-{:02}", ordinal + 1);
            let manifest = json!({"contract":"ait-history-promotion/v3","idempotency_key":format!("workflow-package:{}:{}", "0".repeat(64), u32::MAX),"request_sha256":prepared.fingerprint,"target_line":package.target_line,"base_snapshot_id":package.base_snapshot_id,"revision_snapshot_id":package.revision_snapshot_id,"display_summary":package.summary,"entries":receipt_rows,"aggregate":{"task_index":owner.task_index,"task_id":task_id,"change_index":null,"change_ref":null,"patchset_index":next_patchset,"patchset_id":patchset_id}});
            let summary = format!(
                "{HISTORY_PROMOTION_SUMMARY_PREFIX}{}",
                serde_json::to_string(&manifest).map_err(|e| e.to_string())?
            );
            if WorkflowBinaryV0Codec::encode_single_text_payload(&summary, "Patchset summary")
                .map_err(|e| e.to_string())?
                .len()
                > usize::from(u16::MAX)
            {
                errors.push("Aggregate summary exceeds u16 byte capacity".into());
            }
        }
        if !errors.is_empty() {
            errors.truncate(16);
            for error in &mut errors {
                if error.len() > 1024 {
                    let mut end = 1024;
                    while !error.is_char_boundary(end) {
                        end -= 1;
                    }
                    error.truncate(end);
                    error.push_str("...");
                }
            }
            return Err(format!(
                "Package conversion rejected: {}",
                errors.join("; ")
            ));
        }
        Ok(PackageConversion {
            entries,
            owners,
            attestation_slots,
        })
    }
}

#[cfg(test)]
#[path = "package_plan_tests.rs"]
mod tests;
