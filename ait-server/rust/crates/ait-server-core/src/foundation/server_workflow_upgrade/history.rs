//! Read-only source-era checks and derivation of uniquely proven membership.
//! Historical policy is separate; quoted strings never become lookup aliases.
use super::*;
use crate::foundation::server_content_binary_db::{
    ServerBinarySnapshotCodec, ServerBinarySnapshotParentEdgeCodec, ServerBinarySnapshotRecord,
};
use serde_json::Value;

pub(super) struct SourceHistory {
    local: BTreeMap<u32, Value>,
}

/// Derive facts from source-era native identities. This does not approve the
/// bootstrap boundary, duplicate choice, incoming scope or terminal policy.
pub(super) fn derive_proven_disposition(
    source: &Files,
) -> Result<WorkflowHistoricalDisposition, String> {
    let mut disposition = WorkflowHistoricalDisposition::default();
    for (index, row) in records(source, "patchset.bin", 65)?.iter().enumerate() {
        let text = summary(source, row)?;
        if [
            "ait-local-land-receipt/v1 ",
            "ait-history-promotion/v1 ",
            "ait-history-promotion-stage/v1 ",
        ]
        .iter()
        .any(|prefix| text.starts_with(prefix))
        {
            disposition
                .legacy_history_patchset_indices
                .insert(index as u32);
        } else if [
            "ait-local-land-receipt/",
            "ait-history-promotion/",
            "ait-history-promotion-stage/",
        ]
        .iter()
        .any(|prefix| text.starts_with(prefix))
        {
            return Err("Unsupported source-era history contract".into());
        }
    }
    let history = validate_source_history(source, &disposition)?;
    let changes = records(source, "change.bin", 68)?;
    let patchsets = records(source, "patchset.bin", 65)?;
    let lands = records(source, "land.bin", 48)?;
    let mut actual = BTreeSet::new();
    for (index, row) in lands.iter().enumerate() {
        if row[0] & (LAND_TOMBSTONE | LAND_STATUS_MASK) != LAND_STATUS_SUCCEEDED {
            continue;
        }
        let change = changes
            .get(u32_at(row, 4) as usize)
            .ok_or("Source Land Change is out of range")?;
        let patchset = patchsets
            .get(u32_at(row, 8) as usize)
            .ok_or("Source Land Patchset is out of range")?;
        if u32_at(patchset, 4) != u32_at(row, 4) || patchset[2] != row[2] {
            return Err("Source Land/Patchset numeric owner differs".into());
        }
        if history.local.contains_key(&u32_at(row, 8)) {
            disposition.retired_pseudo_land_indices.insert(index as u32);
        } else {
            actual.insert(u32_at(change, 16));
        }
    }
    validate_land_disposition(source, &disposition, &history)?;
    for (index, task) in records(source, "task.bin", 60)?.iter().enumerate() {
        if task[0] & TASK_META_COMPLETED != 0 && !actual.contains(&(index as u32)) {
            disposition
                .retained_terminal_task_indices
                .insert(index as u32);
        }
    }
    Ok(disposition)
}

/// Apply the owner-approved earliest exact duplicate rule to proven legacy
/// receipt membership. Different actual acceptances remain unresolved.
pub(super) fn derive_approved_disposition(
    source: &Files,
) -> Result<WorkflowHistoricalDisposition, String> {
    let mut disposition = derive_proven_disposition(source)?;
    let changes = records(source, "change.bin", 68)?;
    let lands = records(source, "land.bin", 48)?;
    let mut actual = BTreeMap::<u32, Vec<usize>>::new();
    for (index, row) in lands.iter().enumerate() {
        if row[0] & (LAND_TOMBSTONE | LAND_STATUS_MASK) == LAND_STATUS_SUCCEEDED
            && !disposition
                .retired_pseudo_land_indices
                .contains(&(index as u32))
        {
            let change = changes
                .get(u32_at(row, 4) as usize)
                .ok_or("Source Land Change is out of range")?;
            actual.entry(u32_at(change, 16)).or_default().push(index);
        }
    }
    for (task, rows) in actual {
        if rows.len() <= 1 {
            continue;
        }
        let first = &lands[rows[0]];
        let later = &lands[rows[1]];
        if rows.len() != 2
            || first[0] != later[0]
            || first[2..12] != later[2..12]
            || first[20..28] != later[20..28]
            || first[44..48] != later[44..48]
            || u32_at(first, 20) != 0
            || first[1] >= later[1]
            || u32_at(later, 12) != rows[0] as u32 + 1
            || u32_at(later, 16) != rows[0] as u32 + 1
            || u64_at(first, 28) > u64_at(later, 28)
            || u64_at(first, 36) > u64_at(later, 36)
        {
            return Err(format!("UNRESOLVED_HISTORICAL_RECEIPTS: Task index {task} has multiple distinct or unsupported actual receipts"));
        }
        disposition
            .retired_duplicate_land_indices
            .insert(rows[1] as u32);
    }
    Ok(disposition)
}

/// Read the native relationship/payload semantics under retained source locks.
/// A legacy Plan gets only an ephemeral metadata view through the canonical
/// converter; normal serving descriptors and write guards remain target-only.
pub(super) fn validate_source_plan_bindings(root: &Path, source: &Files) -> Result<(), String> {
    use crate::foundation::remote_binary_db::{BinaryDbReadScope, BinaryDbReadTxn};
    use crate::foundation::server_plan_binary_db::remote_actor_profile::{
        classify_remote_plan_metadata, convert_remote_plan_metadata, RemotePlanMetadataProfile,
    };
    use crate::foundation::server_plan_binary_db::BinaryDbServerPlanService;
    let mut has_binding = false;
    for raw in records(source, "task.bin", 60)? {
        let task =
            WorkflowBinaryV0Codec::decode_task_prefix(&raw).map_err(|error| error.to_string())?;
        has_binding |= task.remote_meta & 1 == 0 && task.origin_plan_revision_index_plus1 != 0;
    }
    if !has_binding {
        return Ok(());
    }
    let view = if classify_remote_plan_metadata(&source["plan.bin"], &source["plan_revision.bin"])
        .map_err(|error| error.to_string())?
        == RemotePlanMetadataProfile::Legacy
    {
        let view = super::batch::MetadataOutputs::new(&std::env::temp_dir())?;
        let (plans, revisions) =
            convert_remote_plan_metadata(&source["plan.bin"], &source["plan_revision.bin"])
                .map_err(|error| error.to_string())?;
        for name in [
            "plan.bin",
            "plan_revision.bin",
            "plan_payload.bin",
            "plan_revision_payload.bin",
            "plan_item.bin",
            "plan_item_payload.bin",
        ] {
            let bytes = match name {
                "plan.bin" => &plans,
                "plan_revision.bin" => &revisions,
                _ => source.get(name).ok_or("missing source Plan metadata")?,
            };
            fs::write(view.0.join(name), bytes).map_err(|error| error.to_string())?;
        }
        Some(view)
    } else {
        None
    };
    let root = view.as_ref().map(|view| view.0.as_path()).unwrap_or(root);
    let db = FilesystemServerRemoteBinaryDb::serving_authority(
        RepoId::new("workflow-conversion"),
        RepoName::new("workflow-conversion"),
        StorePath::new(root.to_path_buf()),
        StoreGeneration::new(1),
    );
    let read = BinaryDbReadTxn::new_for_scope(&db, BinaryDbReadScope::PLAN);
    let plans = BinaryDbServerPlanService::new(db.clone());
    for (index, raw) in records(source, "task.bin", 60)?.iter().enumerate() {
        let task = WorkflowBinaryV0Codec::decode_task_prefix(raw).map_err(|e| e.to_string())?;
        if task.remote_meta & 1 != 0 {
            continue;
        }
        if let Some(revision) = task.origin_plan_revision_index_plus1.checked_sub(1) {
            let item = task
                .plan_item_index_plus1
                .checked_sub(1)
                .ok_or("Legacy Task Plan binding is incomplete")?;
            plans
                .task_binding_projection_with_read(&read, revision, item)
                .map_err(|e| format!("Legacy Task index {index} Plan binding: {e}"))?;
        }
    }
    Ok(())
}

fn summary(source: &Files, row: &[u8]) -> Result<String, String> {
    let raw = payload(
        source,
        "patchset_summary_payload.bin",
        u64_at(row, 51),
        u16_at(row, 59) as usize,
    )?;
    Ok(
        WorkflowBinaryV0Codec::decode_single_text_payload(raw, "legacy history summary")
            .map_err(|e| e.to_string())?
            .to_owned(),
    )
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Legacy history {key} is missing"))
}

fn number(value: &Value, key: &str) -> Result<u32, String> {
    value[key]
        .as_u64()
        .and_then(|n| n.try_into().ok())
        .ok_or_else(|| format!("Legacy history {key} is invalid"))
}

fn task_id(value: &Value, task: u32) -> Result<String, String> {
    let id = text(value, "task_id")?;
    let (prefix, _) = id
        .rsplit_once('-')
        .ok_or("Legacy history Task identity is invalid")?;
    if prefix.is_empty() || id != format!("{prefix}-{:04}", task + 1) {
        return Err("Legacy history Task identity differs from numeric owner".into());
    }
    Ok(id.to_owned())
}

struct SourceContent {
    snapshots: Vec<ServerBinarySnapshotRecord>,
    parents: Vec<Vec<u32>>,
    links: BTreeMap<u32, Vec<Vec<u8>>>,
}

impl SourceContent {
    fn load(source: &Files) -> Result<Self, String> {
        let snapshots = records(source, "snapshot.bin", 88)?
            .iter()
            .map(|raw| {
                ServerBinarySnapshotCodec::<1>::decode_record(raw).map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut edges = BTreeMap::<u32, Vec<(u16, u32)>>::new();
        for raw in records(source, "snapshot_parent_edge.bin", 12)? {
            let e = ServerBinarySnapshotParentEdgeCodec::<1>::decode_record(&raw)
                .map_err(|e| e.to_string())?;
            if e.child_snapshot_index as usize >= snapshots.len()
                || e.parent_snapshot_index as usize >= snapshots.len()
            {
                return Err("Legacy history Snapshot parent is out of range".into());
            }
            edges
                .entry(e.child_snapshot_index)
                .or_default()
                .push((e.parent_ordinal, e.parent_snapshot_index));
        }
        let mut parents = Vec::with_capacity(snapshots.len());
        for (index, row) in snapshots.iter().enumerate() {
            let mut ordered = edges.remove(&(index as u32)).unwrap_or_default();
            ordered.sort_unstable();
            let linked = if row.has_parent_edges_authority() {
                if ordered
                    .iter()
                    .enumerate()
                    .any(|(n, (o, _))| n != usize::from(*o))
                {
                    return Err("Legacy history parent ordinals disagree".into());
                }
                ordered.into_iter().map(|(_, p)| p).collect::<Vec<_>>()
            } else {
                if !ordered.is_empty() {
                    return Err("Legacy history has undeclared parent edges".into());
                }
                row.parent_snapshot_index_plus1
                    .checked_sub(1)
                    .into_iter()
                    .collect()
            };
            if linked.first().copied() != row.parent_snapshot_index_plus1.checked_sub(1)
                || linked.iter().any(|&p| p as usize >= snapshots.len())
                || row.is_remote_head_history_boundary() && !linked.is_empty()
            {
                return Err("Legacy history Snapshot parent authority disagrees".into());
            }
            parents.push(linked);
        }
        let mut links = BTreeMap::<u32, Vec<Vec<u8>>>::new();
        for row in records(source, "snapshot_link.bin", 40)? {
            links.entry(u32_at(&row, 20)).or_default().push(row);
        }
        Ok(Self {
            snapshots,
            parents,
            links,
        })
    }

    fn snapshot(&self, index: u32) -> Result<&ServerBinarySnapshotRecord, String> {
        let row = self
            .snapshots
            .get(index as usize)
            .ok_or("Legacy history Snapshot is out of range")?;
        if row.is_tombstone() {
            return Err("Legacy history Snapshot DAG is tombstoned".into());
        }
        Ok(row)
    }

    fn snapshot_id(&self, index: u32) -> Result<String, String> {
        Ok(format!(
            "SNP-{:012X}",
            self.snapshot(index)?.snapshot_hash48
        ))
    }

    fn closure(&self, start: u32) -> Result<BTreeSet<u32>, String> {
        let mut done = BTreeSet::new();
        let mut active = BTreeSet::new();
        let mut pending = vec![(start, false)];
        while let Some((index, exit)) = pending.pop() {
            if exit {
                active.remove(&index);
                done.insert(index);
                continue;
            }
            if done.contains(&index) {
                continue;
            }
            if !active.insert(index) {
                return Err("Legacy history Snapshot DAG has a cycle".into());
            }
            self.snapshot(index)?;
            pending.push((index, true));
            pending.extend(self.parents[index as usize].iter().map(|&p| (p, false)));
        }
        Ok(done)
    }
}

fn validate_links(
    content: &SourceContent,
    task: u32,
    change: u32,
    base: u32,
    revision: u32,
) -> Result<usize, String> {
    let base_scope = content.closure(base)?;
    let revision_scope = content.closure(revision)?;
    if !revision_scope.contains(&base) {
        return Err("Legacy history range does not descend from base".into());
    }
    let wanted = revision_scope
        .difference(&base_scope)
        .copied()
        .collect::<BTreeSet<_>>();
    if wanted.is_empty() {
        return Err("Legacy history Snapshot range is empty".into());
    }
    for &snapshot in &wanted {
        let snapshot_id = content.snapshot_id(snapshot)?;
        // A historical association is scoped to this receipt's exact owner.
        // Other Tasks may quote the same immutable content; this check grants
        // no current capture, correction or finish authority.
        let mut rows = content
            .links
            .get(&snapshot)
            .into_iter()
            .flatten()
            .filter(|row| u32_at(row, 12) == task && u32_at(row, 16) == change + 1);
        let link = rows.next()
            .ok_or_else(|| format!(
                "Legacy history Snapshot Link is missing: Task index {task}, Change index {change}, {snapshot_id}",
            ))?;
        if rows.next().is_some() {
            return Err(format!("Legacy history has duplicate matching Snapshot Links: Task index {task}, Change index {change}, {snapshot_id}"));
        }
        if link[0] & SNAPSHOT_LINK_TOMBSTONE != 0
            || u32_at(link, 12) != task
            || u32_at(link, 16) != change + 1
            || u64_at(link, 32) != content.snapshot(snapshot)?.created_at_s
        {
            return Err(
                "Legacy history has foreign, duplicate or inconsistent Snapshot Links".into(),
            );
        }
    }
    Ok(wanted.len())
}

fn validate_owner(
    source: &Files,
    value: &Value,
    ps_index: u32,
    patchsets: &[Vec<u8>],
    changes: &[Vec<u8>],
) -> Result<(u32, u32), String> {
    let ps = patchsets
        .get(ps_index as usize)
        .ok_or("Legacy history Patchset is out of range")?;
    let change = u32_at(ps, 4);
    let c = changes
        .get(change as usize)
        .ok_or("Legacy history Change is out of range")?;
    let task = u32_at(c, 16);
    let tasks = records(source, "task.bin", 60)?;
    let t = tasks
        .get(task as usize)
        .ok_or("Legacy history Task is out of range")?;
    WorkflowBinaryV0Codec::decode_task_prefix(t).map_err(|e| e.to_string())?;
    for (key, offset) in [("plan_revision.bin", 12), ("plan_item.bin", 16)] {
        if let Some(index) = u32_at(t, offset).checked_sub(1) {
            let width = if key == "plan_revision.bin" { 56 } else { 16 };
            if index as usize >= records(source, key, width as usize)?.len() {
                return Err("Legacy Task Plan binding is out of range".into());
            }
        }
    }
    let id = task_id(value, task)?;
    if text(value, "change_ref")? != format!("{id}/C-{:02}", c[4] + 1) {
        return Err("Legacy history Change text differs from source numeric owner".into());
    }
    Ok((task, change))
}

pub(super) fn validate_source_namespace(source: &Files, namespace: &[u8; 2]) -> Result<(), String> {
    let namespace = std::str::from_utf8(namespace)
        .map_err(|e| e.to_string())?
        .trim_end_matches('\0');
    let task_prefix = format!("R{namespace}T-");
    for ps in records(source, "patchset.bin", 65)? {
        let s = summary(source, &ps)?;
        let prefix = [
            "ait-local-land-receipt/v1 ",
            "ait-history-promotion/v1 ",
            "ait-history-promotion-stage/v1 ",
        ]
        .into_iter()
        .find(|p| s.starts_with(p));
        let Some(prefix) = prefix else {
            continue;
        };
        let v: Value = serde_json::from_str(&s[prefix.len()..]).map_err(|e| e.to_string())?;
        let owner = v.get("aggregate").unwrap_or(&v);
        if let Some(task) = owner["task_id"].as_str() {
            if !task.starts_with(&task_prefix) {
                return Err(
                    "Legacy history Task namespace differs from Repository authority".into(),
                );
            }
        }
    }
    Ok(())
}

pub(super) fn validate_source_history(
    source: &Files,
    disposition: &WorkflowHistoricalDisposition,
) -> Result<SourceHistory, String> {
    let patchsets = records(source, "patchset.bin", 65)?;
    let changes = records(source, "change.bin", 68)?;
    let mut all = BTreeMap::new();
    let mut local = BTreeMap::new();
    for (index, ps) in patchsets.iter().enumerate() {
        let s = summary(source, ps)?;
        let prefix = [
            "ait-local-land-receipt/v1 ",
            "ait-history-promotion/v1 ",
            "ait-history-promotion-stage/v1 ",
        ]
        .into_iter()
        .find(|p| s.starts_with(p));
        let Some(prefix) = prefix else {
            continue;
        };
        let v: Value = serde_json::from_str(&s[prefix.len()..])
            .map_err(|e| format!("Invalid legacy history: {e}"))?;
        all.insert(index as u32, (prefix, v));
    }
    if all.keys().copied().collect::<BTreeSet<_>>() != disposition.legacy_history_patchset_indices {
        return Err("UNRESOLVED_HISTORICAL_DISPOSITION: typed history requires exact source Patchset membership".into());
    }
    let content = SourceContent::load(source)?;
    for (&index, (prefix, v)) in &all {
        if *prefix != "ait-local-land-receipt/v1 " {
            continue;
        }
        if text(v, "contract")? != "ait-local-land-receipt/v1"
            || text(v, "source_kind")? != "imported_local_land_receipt"
        {
            return Err("Unsupported legacy local receipt contract".into());
        }
        let (task, change) = validate_owner(source, v, index, &patchsets, &changes)?;
        let p = &patchsets[index as usize];
        let lines = records(source, "line.bin", 40)?;
        let line = u32_at(&changes[change as usize], 56)
            .checked_sub(1)
            .and_then(|i| lines.get(i as usize))
            .ok_or("Legacy receipt target Line is missing")?;
        let line_name = std::str::from_utf8(payload(
            source,
            "line_name_payload.bin",
            u64_at(line, 4),
            u16_at(line, 2) as usize,
        )?)
        .map_err(|e| e.to_string())?;
        if text(v, "pre_land_target_snapshot_id")? != content.snapshot_id(u32_at(p, 16))?
            || text(v, "landed_snapshot_id")? != content.snapshot_id(u32_at(p, 20))?
            || u64_at(p, 24) == 0
            || v["landed_at_s"].as_u64() != Some(u64_at(p, 24))
            || u32_at(p, 40) != 0
            || u16_at(p, 44) != 0
            || u16_at(p, 46) != 0
            || text(v, "target_line")? != line_name
            || text(v, "local_change_ref")?
                != format!(
                    "{}/{}",
                    text(v, "local_task_id")?,
                    text(v, "local_change_id")?
                )
        {
            return Err("Legacy local receipt differs from exact source candidate".into());
        }
        validate_links(&content, task, change, u32_at(p, 16), u32_at(p, 20))
            .map_err(|e| format!("Legacy receipt Patchset index {index}: {e}"))?;
        local.insert(index, v.clone());
    }
    for (&index, (prefix, v)) in &all {
        if *prefix == "ait-local-land-receipt/v1 " {
            continue;
        }
        if *prefix != "ait-history-promotion/v1 "
            || !v["stage_ordinal"].is_null()
            || !v["promotion_id"].is_null()
            || !matches!(
                text(v, "contract")?,
                "ait-history-promotion/v1" | "ait-history-promotion-correction/v1"
            )
        {
            return Err("Unsupported legacy staged/history contract".into());
        }
        let owner = &v["aggregate"];
        let (task, change) = validate_owner(source, owner, index, &patchsets, &changes)?;
        let p = &patchsets[index as usize];
        let id = task_id(owner, task)?;
        if number(owner, "task_index")? != task
            || number(owner, "change_index")? != change
            || number(owner, "patchset_index")? != index
            || text(owner, "patchset_id")?
                != format!(
                    "{id}/C-{:02}/P-{:02}",
                    changes[change as usize][4] + 1,
                    p[1] + 1
                )
            || text(v, "base_snapshot_id")? != content.snapshot_id(u32_at(p, 16))?
            || text(v, "revision_snapshot_id")? != content.snapshot_id(u32_at(p, 20))?
        {
            return Err("Legacy aggregate source owner/candidate differs".into());
        }
        let entries = v["entries"]
            .as_array()
            .filter(|a| !a.is_empty())
            .ok_or("Legacy aggregate entries are missing")?;
        let mut expected = text(v, "base_snapshot_id")?.to_owned();
        let mut seen = BTreeSet::new();
        let mut previous_time = 0;
        for e in entries {
            let receipt = number(e, "receipt_patchset_index")?;
            let original = local
                .get(&receipt)
                .ok_or("Legacy aggregate receipt source is missing")?;
            let (owner_task, owner_change) =
                validate_owner(source, e, receipt, &patchsets, &changes)?;
            let row = &patchsets[receipt as usize];
            if !seen.insert(owner_task)
                || e["landed_at_s"].as_u64().is_none_or(|t| t < previous_time)
                || original["target_line"] != v["target_line"]
                || number(e, "task_index")? != owner_task
                || number(e, "change_index")? != owner_change
                || text(e, "receipt_patchset_id")?
                    != format!("{}/P-{:02}", text(e, "change_ref")?, row[1] + 1)
                || text(e, "pre_land_target_snapshot_id")? != expected
            {
                return Err("Legacy aggregate receipt chain differs".into());
            }
            for key in [
                "task_id",
                "change_ref",
                "local_task_id",
                "local_change_id",
                "local_change_ref",
                "pre_land_target_snapshot_id",
                "landed_snapshot_id",
                "landed_at_s",
            ] {
                if e[key] != original[key] {
                    return Err(format!("Legacy aggregate receipt {key} differs"));
                }
            }
            if e["snapshot_link_count"].as_u64()
                != Some(validate_links(
                    &content,
                    owner_task,
                    owner_change,
                    u32_at(row, 16),
                    u32_at(row, 20),
                )? as u64)
            {
                return Err("Legacy aggregate Snapshot inventory differs".into());
            }
            expected = text(e, "landed_snapshot_id")?.to_owned();
            previous_time = e["landed_at_s"]
                .as_u64()
                .ok_or("Legacy aggregate event time is missing")?;
        }
        if entries.last().unwrap()["task_index"].as_u64() != Some(u64::from(task)) {
            return Err("Legacy aggregate does not belong to its final receipt Task".into());
        }
        if v["contract"] == "ait-history-promotion/v1"
            && (expected != text(v, "revision_snapshot_id")?
                || entries.last().unwrap()["change_index"].as_u64() != Some(u64::from(change)))
        {
            return Err("Legacy aggregate terminal differs".into());
        }
        if v["contract"] == "ait-history-promotion-correction/v1" {
            let correction = &v["correction"];
            let original_id = text(correction, "original_patchset_id")?;
            let original = all
                .values()
                .map(|(_, v)| v)
                .find(|x| x["aggregate"]["patchset_id"] == original_id)
                .ok_or("Legacy correction original is missing")?;
            if original["contract"] != "ait-history-promotion/v1"
                || original["entries"] != v["entries"]
                || original["base_snapshot_id"] != v["base_snapshot_id"]
                || original["revision_snapshot_id"] != expected
                || correction["receipt_revision_snapshot_id"] != expected
                || original["aggregate"]["task_index"].as_u64() != Some(u64::from(task))
                || original["aggregate"]["change_index"].as_u64() != Some(u64::from(change))
            {
                return Err("Legacy correction rewrites original receipt authority".into());
            }
            let original_index = number(&original["aggregate"], "patchset_index")?;
            let prior = all
                .values()
                .map(|(_, v)| v)
                .find(|x| x["aggregate"]["patchset_id"] == correction["prior_patchset_id"])
                .ok_or("Legacy correction prior candidate is missing")?;
            if number(&prior["aggregate"], "patchset_index")? >= index
                || prior["aggregate"]["task_index"] != original["aggregate"]["task_index"]
            {
                return Err("Legacy correction prior candidate differs".into());
            }
            let original_row = patchsets
                .get(original_index as usize)
                .ok_or("Legacy correction original candidate is missing")?;
            let base = u32_at(original_row, 20);
            let count = validate_links(&content, task, change, base, u32_at(p, 20))?;
            let named = correction["snapshots"]
                .as_array()
                .ok_or("Legacy correction Snapshot inventory is missing")?;
            let expected = content
                .closure(u32_at(p, 20))?
                .difference(&content.closure(base)?)
                .copied()
                .collect::<BTreeSet<_>>();
            let mut named_ids = BTreeSet::new();
            for value in named {
                let id = text(value, "snapshot_id")?;
                let snapshot = expected
                    .iter()
                    .copied()
                    .find(|&i| content.snapshot_id(i).as_deref() == Ok(id))
                    .ok_or("Legacy correction names an unexpected Snapshot")?;
                if !named_ids.insert(snapshot)
                    || value["created_at_s"].as_u64()
                        != Some(content.snapshot(snapshot)?.created_at_s)
                {
                    return Err("Legacy correction Snapshot identity/time differs".into());
                }
            }
            if named_ids != expected || named.len() != count {
                return Err("Legacy correction Snapshot inventory is incomplete".into());
            }
        }
    }
    Ok(SourceHistory { local })
}

pub(super) fn validate_land_disposition(
    source: &Files,
    disposition: &WorkflowHistoricalDisposition,
    history: &SourceHistory,
) -> Result<(), String> {
    let lands = records(source, "land.bin", 48)?;
    let ps = records(source, "patchset.bin", 65)?;
    if !disposition
        .retired_pseudo_land_indices
        .is_disjoint(&disposition.retired_duplicate_land_indices)
    {
        return Err("Historical Land decisions overlap".into());
    }
    for &index in disposition
        .retired_pseudo_land_indices
        .iter()
        .chain(&disposition.retired_duplicate_land_indices)
    {
        let row = lands
            .get(index as usize)
            .ok_or("Historical Land decision is out of range")?;
        if row[0] & (LAND_TOMBSTONE | LAND_STATUS_MASK) != LAND_STATUS_SUCCEEDED {
            return Err("Historical retirement requires an original live success".into());
        }
        let patch = u32_at(row, 8);
        let p = ps
            .get(patch as usize)
            .ok_or("Historical Land candidate is missing")?;
        if disposition.retired_pseudo_land_indices.contains(&index) {
            let v = history
                .local
                .get(&patch)
                .ok_or("Only proven imported-local pseudo-success may be retired as provenance")?;
            let line = u32_at(row, 44)
                .checked_sub(1)
                .ok_or("Pseudo-success target Line is missing")?;
            let lines = records(source, "line.bin", 40)?;
            let line = lines
                .get(line as usize)
                .ok_or("Pseudo-success target Line is out of range")?;
            let name = std::str::from_utf8(payload(
                source,
                "line_name_payload.bin",
                u64_at(line, 4),
                u16_at(line, 2) as usize,
            )?)
            .map_err(|e| e.to_string())?;
            if u32_at(row, 20) != u32_at(p, 16) + 1
                || u32_at(row, 24) != u32_at(p, 20) + 1
                || v["landed_at_s"].as_u64() != Some(u64_at(row, 28))
                || u64_at(row, 28) != u64_at(row, 36)
                || text(v, "target_line")? != name
            {
                return Err("Pseudo-success differs from its exact source receipt".into());
            }
        } else {
            if history.local.contains_key(&patch) {
                return Err("Pseudo-success is not a real duplicate receipt".into());
            }
            let kept = lands.iter().enumerate().any(|(other, r)| {
                other != index as usize
                    && !disposition
                        .retired_duplicate_land_indices
                        .contains(&(other as u32))
                    && !disposition
                        .retired_pseudo_land_indices
                        .contains(&(other as u32))
                    && r[0] & (LAND_TOMBSTONE | LAND_STATUS_MASK) == LAND_STATUS_SUCCEEDED
                    && r[0] & 0x7f == row[0] & 0x7f
                    && r[2..12] == row[2..12]
                    && r[20..28] == row[20..28]
                    && r[44..48] == row[44..48]
            });
            if !kept {
                return Err("Selected duplicate has no exact retained actual receipt".into());
            }
        }
    }
    Ok(())
}
