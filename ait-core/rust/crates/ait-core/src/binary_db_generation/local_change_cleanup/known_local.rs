//! Complete known Local layout-1 input, independent of Repository identity.
use super::*;
use std::io::Read;

pub(super) struct Profile {
    snapshots: u32,
    lines: u32,
}

// Only unchanged header/width sentinels needed by converted references are read.
fn reference_count(root: &Path, leaf: &str, width: u64) -> GenerationResult<u32> {
    let path = root.join(leaf);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.file_type().is_file() {
        return Err(format!(
            "Local source reference is not a direct file: {leaf}"
        ));
    }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut header = [0; 4];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if header != 1u32.to_le_bytes() || size < 4 || !(size - 4).is_multiple_of(width) {
        return Err(format!(
            "Unsupported Local source reference profile: {leaf}"
        ));
    }
    u32::try_from((size - 4) / width).map_err(|_| "Local reference count overflow".into())
}

pub(super) fn validate(
    root: &Path,
    tasks: &[u8],
    changes: &[LocalChangeRecord],
) -> GenerationResult<Profile> {
    let task_count = (tasks.len() - 4) / 64;
    u32::try_from(task_count).map_err(|_| "Local Task count overflow")?;
    u32::try_from(changes.len()).map_err(|_| "Local Change count overflow")?;
    let profile = Profile {
        snapshots: reference_count(root, "snapshot.bin", 88)?,
        lines: reference_count(root, "line.bin", 40)?,
    };
    let tasks = tasks[4..]
        .chunks_exact(64)
        .map(LocalTaskRecord::decode)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut previous = vec![None; task_count];
    for (index, change) in changes.iter().enumerate() {
        let task = &tasks[change.task_index as usize];
        let expected = previous[change.task_index as usize];
        let expected_ordinal = expected.map_or(0, |old: usize| changes[old].change_ordinal + 1);
        if change.change_ordinal != expected_ordinal
            || change.previous_change_index_plus1 != expected.map_or(0, |old| old as u32 + 1)
            || change.fork_snapshot_index_plus1 > profile.snapshots
            || change.base_line_index_plus1 > profile.lines
            || (change.local_meta & 1 != 0 && task.local_meta & 1 == 0)
        {
            return Err("Unsupported Local Change ownership, fork or publication reference".into());
        }
        previous[change.task_index as usize] = Some(index);
    }
    Ok(profile)
}

impl Profile {
    pub(super) fn snapshot_count(&self) -> u32 {
        self.snapshots
    }

    pub(super) fn validate_receipt(&self, row: &[u8]) -> GenerationResult<()> {
        if row[0] & 0x87 != 2
            || u32_at(row, 8) != 0
            || u32_at(row, 12) != 0
            || u32_at(row, 16) > self.snapshots
            || u32_at(row, 20) > self.snapshots
            || u32_at(row, 40) == 0
            || u32_at(row, 40) > self.lines
        {
            return Err("Unsupported Local successful receipt/reference profile".into());
        }
        Ok(())
    }

    pub(super) fn validate_link(
        &self,
        link: &LegacySnapshotLink,
        previous_task: u32,
        previous_changes: &[u32],
    ) -> GenerationResult<()> {
        let previous_change = match link.change_plus1.checked_sub(1) {
            Some(index) => *previous_changes
                .get(index as usize)
                .ok_or("Local Snapshot Link Change is missing")?,
            None => 0,
        };
        if link.snapshot >= self.snapshots
            || link.previous_task != previous_task
            || link.previous_change != previous_change
        {
            return Err("Unsupported Local Snapshot Link predecessor/content reference".into());
        }
        Ok(())
    }
}
