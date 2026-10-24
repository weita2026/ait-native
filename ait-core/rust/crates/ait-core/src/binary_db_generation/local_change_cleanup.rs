//! Supported Local format conversion with isolated historical dispositions.
use super::{GenerationResult, Path};
use crate::workflow_binary_db::legacy_snapshot_links::{
    LegacySnapshotLink, LegacySnapshotLinkPayload,
};
use crate::workflow_binary_db::{
    LocalChangeRecord, LocalLandRecord, LocalTaskRecord, SnapshotLinkRecord,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

mod known_local;

pub(super) const CHANGED_FILES: &[&str] = &[
    "change.bin",
    "change_payload.bin",
    "task_change_index.bin",
    "land.bin",
    "task_land_index.bin",
    "change_land_index.bin",
    "snapshot_link.bin",
    "snapshot_link_payload.bin",
    "task_snapshot_index.bin",
    "change_snapshot_index.bin",
];
pub(super) struct Conversion {
    pub files: BTreeMap<&'static str, Vec<u8>>,
    pub changes: Vec<LocalChangeRecord>,
    pub mapping: Vec<Option<u32>>,
    pub task_count: usize,
    pub sources: BTreeMap<&'static str, Vec<u8>>,
    pub core_history: bool,
}
fn read(root: &Path, name: &str) -> GenerationResult<Vec<u8>> {
    match fs::read(root.join(name)) {
        Ok(bytes) if bytes.len() >= 4 => Ok(bytes),
        Ok(_) => Err(format!("Truncated {name}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(1u32.to_le_bytes().to_vec()),
        Err(e) => Err(e.to_string()),
    }
}
fn u32_at(row: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(row[at..at + 4].try_into().unwrap())
}
fn put(row: &mut [u8], at: usize, value: u32) {
    row[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn published_task_to(tasks: &[u8], task: usize, remote: u32) -> bool {
    tasks
        .get(4 + task * 64..4 + (task + 1) * 64)
        .is_some_and(|row| row[1] & 1 != 0 && u32_at(row, 20) == remote)
}
fn index_bytes(rows: &[[u8; 8]]) -> Vec<u8> {
    let mut out = 1u32.to_le_bytes().to_vec();
    for row in rows {
        out.extend_from_slice(row);
    }
    out
}
fn payload<'a>(bytes: &'a [u8], row: &[u8], offset_at: usize) -> GenerationResult<&'a [u8]> {
    let offset = usize::try_from(u64::from_le_bytes(
        row[offset_at..offset_at + 8].try_into().unwrap(),
    ))
    .map_err(|_| "Payload offset overflow")?;
    let len = usize::from(u16::from_le_bytes(row[2..4].try_into().unwrap()));
    if offset < 4 {
        return Err("Payload overlaps layout header".into());
    }
    bytes
        .get(offset..offset.checked_add(len).ok_or("Payload end overflow")?)
        .ok_or_else(|| "Missing typed payload".into())
}
impl Conversion {
    pub fn build(root: &Path, name: &str) -> GenerationResult<Self> {
        let namespace = match name {
            "ait-core" => "C",
            "ait-runner" => "R",
            _ => {
                return Err(
                    "Historical offline conversion requires its exact source disposition".into(),
                )
            }
        };
        let conversion = Self::build_with_namespace(root, name, namespace)?;
        if name == "ait-core" && !conversion.core_history {
            return Err("Offline generation requires the original Core historical disposition; use ordinary admission for supported Local formats".into());
        }
        if name == "ait-runner" {
            validate_runner_cohort(root, &conversion.sources["task.bin"], &conversion.changes)?;
        }
        Ok(conversion)
    }

    pub fn build_with_namespace(
        root: &Path,
        name: &str,
        namespace: &str,
    ) -> GenerationResult<Self> {
        let local_prefix =
            crate::workflow_primitives::workflow_origin_namespace_prefix("L", Some(namespace))?;
        let mut sources = BTreeMap::new();
        let mut read_source = |name| -> GenerationResult<Vec<u8>> {
            let bytes = read(root, name)?;
            sources.insert(name, bytes.clone());
            Ok(bytes)
        };
        let task_bytes = read_source("task.bin")?;
        require_rows(&task_bytes, 64)?;
        let task_count = (task_bytes.len() - 4) / 64;
        let change_bytes = read_source("change.bin")?;
        require_rows(&change_bytes, 68)?;
        let change_payload = read_source("change_payload.bin")?;
        let mut changes = Vec::new();
        for row in change_bytes[4..].chunks_exact(68) {
            let change = LocalChangeRecord::decode(row).map_err(|e| e.to_string())?;
            if change.task_index as usize >= task_count {
                return Err("Legacy Change Task reference is missing".into());
            }
            changes.push(change);
        }
        let profile = known_local::validate(root, &task_bytes, &changes)?;
        // Names alone do not identify a historical source. These existing
        // publication relationships select its bounded disposition; malformed
        // variants still fail that disposition rather than a generic fallback.
        let core = name == "ait-core"
            && (published_task_to(&task_bytes, 2, 981) || published_task_to(&task_bytes, 53, 787));
        let runner = name == "ait-runner"
            && (published_task_to(&task_bytes, 0, 63)
                || (profile.snapshot_count() == 131
                    && task_bytes.get(5).is_some_and(|meta| meta & 1 != 0)));
        let historical = core || runner;
        if (core && namespace != "C") || (runner && namespace != "R") {
            return Err("Historical Local source namespace differs from its disposition".into());
        }
        if runner {
            validate_runner_cohort(root, &task_bytes, &changes)?;
        } else if core && (changes.is_empty() || task_count < 1113) {
            return Err("Local receipt upgrade coordination required: source is outside the approved ait-core legacy Change cohort".into());
        }
        let retained = [(2u32, 3usize), (53, 55), (620, 621)];
        let mut seen = BTreeSet::new();
        let mut mapping = vec![None; changes.len()];
        let mut target_changes = 1u32.to_le_bytes().to_vec();
        let mut target_payload = 1u32.to_le_bytes().to_vec();
        let mut task_changes = vec![[0; 8]; task_count];
        let mut kept = 0u32;
        for (old_index, row) in changes.iter().enumerate() {
            std::str::from_utf8(payload(
                &change_payload,
                &change_bytes[4 + old_index * 68..][..68],
                8,
            )?)
            .map_err(|_| "Local Change title is not UTF-8")?;
            if row.change_ordinal == 0 {
                if row.previous_change_index_plus1 != 0 || !seen.insert(row.task_index) {
                    return Err(
                        "Legacy initial Change ownership/chain changed; coordination required"
                            .into(),
                    );
                }
                continue;
            }
            if historical
                && (row.change_ordinal != 1 || !retained.contains(&(row.task_index, old_index)))
            {
                return Err("Unexpected legacy revision Change; coordination required".into());
            }
            if !historical {
                // The admitted source starts with C-01 bootstrap work. Every
                // later row is a real revision; preserve its remaining bytes.
            } else if row.task_index == 620 {
                if row.change_meta & 3 != 2
                    || row.change_state != 0
                    || row.local_meta != 0
                    || row.published_remote_change_ordinal_plus1 != 0
                {
                    return Err("Retained LCT-0621 revision state/publication changed".into());
                }
            } else {
                let task = &task_bytes[4 + row.task_index as usize * 64..][..64];
                let remote = if row.task_index == 2 { 981 } else { 787 };
                if row.change_meta & 3 != 3
                    || row.change_state != 1
                    || row.local_meta != 1
                    || row.published_remote_change_ordinal_plus1 != 1
                    || u32_at(task, 20) != remote
                {
                    return Err("Retained canceled revision Remote mapping changed".into());
                }
            }
            let source = &change_bytes[4 + old_index * 68..][..68];
            let title = payload(&change_payload, source, 8)?;
            let mut target = source.to_vec();
            target[4] = row.change_ordinal - 1;
            target[8..16].copy_from_slice(&(target_payload.len() as u64).to_le_bytes());
            let previous = row
                .previous_change_index_plus1
                .checked_sub(1)
                .and_then(|old| mapping[old as usize])
                .map_or(0, |new| new + 1);
            put(&mut target, 20, previous);
            target_payload.extend_from_slice(title);
            target_changes.extend_from_slice(&target);
            mapping[old_index] = Some(kept);
            let owner = &mut task_changes[row.task_index as usize];
            owner[..4].copy_from_slice(&(kept + 1).to_le_bytes());
            owner[4] = row.change_ordinal;
            owner[6] = row.change_ordinal;
            kept += 1;
        }
        if !runner
            && historical
            && !changes.is_empty()
            && (0..1113u32).any(|task| seen.contains(&task) == [362, 376].contains(&task))
        {
            return Err("Legacy initial Change cohort changed; coordination required".into());
        }
        if core && !changes.is_empty() && kept != 3 {
            return Err(
                "Approved three legacy revisions are not all present; coordination required".into(),
            );
        }
        let mut files = BTreeMap::new();
        files.insert("change.bin", target_changes);
        files.insert("change_payload.bin", target_payload);
        files.insert("task_change_index.bin", index_bytes(&task_changes));
        let mut lands = read_source("land.bin")?;
        require_rows(&lands, 44)?;
        u32::try_from((lands.len() - 4) / 44).map_err(|_| "Local receipt count overflow")?;
        lands[..4].copy_from_slice(&2u32.to_le_bytes());
        let mut task_lands = vec![[0; 8]; task_count];
        let mut change_lands = vec![[0; 8]; kept as usize];
        for (index, row) in lands[4..].chunks_exact_mut(44).enumerate() {
            let old = u32_at(row, 4) as usize;
            let change = changes
                .get(old)
                .ok_or("Receipt references missing legacy Change")?;
            if row[2] != change.change_ordinal {
                return Err("Receipt legacy Change ordinal differs".into());
            }
            put(row, 4, change.task_index);
            row[2] = if mapping[old].is_some() {
                change.change_ordinal
            } else {
                0
            };
            profile.validate_receipt(row)?;
            put(row, 12, 0);
            LocalLandRecord::decode(row).map_err(|e| e.to_string())?;
            let owner = task_lands
                .get_mut(change.task_index as usize)
                .ok_or("Receipt Task missing")?;
            if owner[4] != 0 {
                return Err("Duplicate Task finish receipts".into());
            }
            owner[..4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
            owner[4] = 1;
            if let Some(new) = mapping[old] {
                let projected = &mut change_lands[new as usize];
                projected[..4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
                projected[4] = 1;
                projected[6] = row[1] + 1;
            }
        }
        files.insert("land.bin", lands);
        files.insert("task_land_index.bin", index_bytes(&task_lands));
        files.insert("change_land_index.bin", index_bytes(&change_lands));
        let source_links = read_source("snapshot_link.bin")?;
        require_rows(&source_links, 40)?;
        u32::try_from((source_links.len() - 4) / 40)
            .map_err(|_| "Local Snapshot Link count overflow")?;
        let source_payload = read_source("snapshot_link_payload.bin")?;
        let mut links = 2u32.to_le_bytes().to_vec();
        let mut task_links = vec![[0; 8]; task_count];
        let mut change_links = vec![[0; 8]; kept as usize];
        let mut source_change_links = vec![0u32; changes.len()];
        let mut live_snapshots = BTreeSet::new();
        for (index, raw) in source_links[4..].chunks_exact(40).enumerate() {
            let old = LegacySnapshotLink::decode(raw).map_err(|e| e.to_string())?;
            if old.meta & 0x80 == 0 && !live_snapshots.insert(old.snapshot) {
                return Err("Local Snapshot has multiple live ownership links".into());
            }
            let binding = LegacySnapshotLinkPayload::decode(payload(&source_payload, raw, 4)?)
                .map_err(|e| e.to_string())?;
            if binding.task_id != format!("{local_prefix}T-{:04}", old.task + 1) {
                return Err(
                    "Legacy Snapshot Link textual Task differs from its numeric owner".into(),
                );
            }
            let new = if let Some(source) = old.change_plus1.checked_sub(1) {
                let change = changes
                    .get(source as usize)
                    .ok_or("Snapshot Link references missing legacy Change")?;
                if change.task_index != old.task
                    || binding.change_id != format!("C-{:02}", change.change_ordinal + 1)
                {
                    return Err("Legacy Snapshot Link fixed/textual Change differs".into());
                }
                mapping[source as usize]
            } else {
                if !binding.change_id.is_empty() {
                    return Err("Legacy Snapshot Link has text without a Change".into());
                }
                None
            };
            let task = task_links
                .get_mut(old.task as usize)
                .ok_or("Snapshot Link Task is missing")?;
            let count = u16::from_le_bytes(task[4..6].try_into().unwrap());
            profile.validate_link(&old, u32_at(task, 0), &source_change_links)?;
            if count >= 256 || count != u16::from(old.ordinal) {
                return Err("Legacy Snapshot Link Task ordinal is inconsistent".into());
            }
            let row = SnapshotLinkRecord {
                meta: (old.meta & !0x39) | u8::from(new.is_some()),
                ordinal: old.ordinal,
                task: old.task,
                change_plus1: new.map_or(0, |index| index + 1),
                snapshot: old.snapshot,
                created_at_s: old.created_at_s,
            };
            links.extend_from_slice(&row.encode().map_err(|e| e.to_string())?);
            put(task, 0, index as u32 + 1);
            if let Some(old_change) = old.change_plus1.checked_sub(1) {
                source_change_links[old_change as usize] = index as u32 + 1;
            }
            task[4..6].copy_from_slice(&(count + 1).to_le_bytes());
            task[6] = (count + 1).min(255) as u8;
            if let Some(new) = new {
                let owner = &mut change_links[new as usize];
                put(owner, 0, index as u32 + 1);
                let count = u16::from_le_bytes(owner[4..6].try_into().unwrap()) + 1;
                owner[4..6].copy_from_slice(&count.to_le_bytes());
            }
        }
        files.insert("snapshot_link.bin", links);
        // Retire the obsolete typed payload with the prepared metadata batch.
        files.insert("snapshot_link_payload.bin", 1u32.to_le_bytes().to_vec());
        files.insert("task_snapshot_index.bin", index_bytes(&task_links));
        files.insert("change_snapshot_index.bin", index_bytes(&change_links));
        Ok(Self {
            files,
            changes,
            mapping,
            task_count,
            sources,
            core_history: core,
        })
    }

    pub fn retain_changed_files(&mut self, root: &Path) -> GenerationResult<()> {
        // Compare affected inputs already decoded above. Index inputs are
        // read only to avoid replacing an unchanged rebuilt projection.
        let mut unchanged = Vec::new();
        for (name, target) in &self.files {
            let source = match self.sources.get(name) {
                Some(bytes) => bytes.clone(),
                None => read(root, name)?,
            };
            if name.ends_with("_index.bin") {
                require_rows(&source, 8)?;
            }
            let absent_target =
                target.len() == 4 && !["land.bin", "snapshot_link.bin"].contains(name);
            if (!absent_target && source == *target)
                || (absent_target && !root.join(name).try_exists().map_err(|e| e.to_string())?)
            {
                unchanged.push(*name);
            }
        }
        for name in unchanged {
            self.files.remove(name);
        }
        Ok(())
    }
    pub fn write(&self, root: &Path) -> GenerationResult<()> {
        use std::io::Write;
        for (name, bytes) in &self.files {
            let path = root.join(name);
            // Empty families are absent except the two complete-target sentinels.
            if bytes.len() == 4 && !["land.bin", "snapshot_link.bin"].contains(name) {
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
                continue;
            }
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .map_err(|e| e.to_string())?;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

// This is the separately authorized Runner source, not a generic initial-row
// filter. Its numeric and publication relationships must still match under the
// existing admission lock; additional Tasks/revisions require coordination.
fn validate_runner_cohort(
    root: &Path,
    tasks: &[u8],
    changes: &[LocalChangeRecord],
) -> GenerationResult<()> {
    let exact_count = |name: &str, width: u64, count: u64| -> GenerationResult<()> {
        let size = fs::metadata(root.join(name))
            .map_err(|error| error.to_string())?
            .len();
        if size != 4 + width * count {
            return Err(format!(
                "Unsupported Runner initial-only cohort: {name} count changed"
            ));
        }
        Ok(())
    };
    if tasks.len() != 4 + 64 || changes.len() != 1 {
        return Err("Unsupported Runner initial-only Task/Change cohort".into());
    }
    exact_count("land.bin", 44, 1)?;
    exact_count("snapshot_link.bin", 40, 1)?;
    exact_count("snapshot.bin", 88, 131)?;
    let task = LocalTaskRecord::decode(&tasks[4..]).map_err(|error| error.to_string())?;
    let change = &changes[0];
    if task.task_meta & 0xc0 != 0x40
        || task.local_meta != 1
        || task.published_remote_task_index != 63
        || change.task_index != 0
        || change.change_ordinal != 0
        || change.previous_change_index_plus1 != 0
        || change.change_meta & 3 != 2
        || change.change_state != 0
        || change.local_meta != 1
        || change.published_remote_change_ordinal_plus1 != 1
    {
        return Err(
            "Unsupported Runner initial-only ownership, lifecycle or publication mapping".into(),
        );
    }
    let lands = read(root, "land.bin")?;
    let links = read(root, "snapshot_link.bin")?;
    require_rows(&lands, 44)?;
    require_rows(&links, 40)?;
    let receipt = &lands[4..];
    let link = LegacySnapshotLink::decode(&links[4..]).map_err(|error| error.to_string())?;
    if receipt[0] & 0x87 != 2
        || receipt[1] != 0
        || receipt[2] != 0
        || u32_at(receipt, 4) != 0
        || link.task != 0
        || link.change_plus1 != 1
        || link.ordinal != 0
        || link.previous_task != 0
        || link.previous_change != 0
        || link.snapshot >= 131
        || u32_at(receipt, 16) > 131
        || u32_at(receipt, 20) != link.snapshot + 1
    {
        return Err("Unsupported Runner initial-only receipt/Snapshot Link ownership".into());
    }
    Ok(())
}

fn require_rows(bytes: &[u8], width: usize) -> GenerationResult<()> {
    if bytes[..4] != 1u32.to_le_bytes() || !(bytes.len() - 4).is_multiple_of(width) {
        return Err("Unsupported legacy workflow representation".into());
    }
    Ok(())
}
