use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::binary_db::{
    AuthorityId, BinaryDb, BinaryDbCommandLockSet, BinaryDbCommandScope, LocalBinaryDbFs,
    LocalStateScope, StorePath, BIN_FILE_HEADER_BYTES,
};
use crate::file_io::{FileIoByteStore, FileIoDurabilityStore, FilesystemFileIoStore};

use super::{
    PlanCodec, PlanItemCodec, PlanItemRecord, PlanRecord, PlanRevisionCodec, PlanRevisionRecord,
    PLAN_BIN, PLAN_ITEM_BIN, PLAN_ITEM_PAYLOAD_BIN, PLAN_ITEM_RECORD_SIZE, PLAN_LAYOUT_ID,
    PLAN_PAYLOAD_BIN, PLAN_RECORD_SIZE, PLAN_REVISION_BIN, PLAN_REVISION_PAYLOAD_BIN,
    PLAN_REVISION_RECORD_SIZE,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanBinaryDbRecoveryState {
    Clean,
    Repairable,
    Repaired,
    Blocked,
}

impl PlanBinaryDbRecoveryState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Repairable => "repairable",
            Self::Repaired => "repaired",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanBinaryDbRecoveryReport {
    pub state: PlanBinaryDbRecoveryState,
    pub authority_root: PathBuf,
    pub committed_plan_count: u32,
    pub repair_candidates: Vec<String>,
    pub repairs: Vec<String>,
    pub issues: Vec<String>,
}

impl PlanBinaryDbRecoveryReport {
    pub fn is_ready(&self) -> bool {
        matches!(
            self.state,
            PlanBinaryDbRecoveryState::Clean | PlanBinaryDbRecoveryState::Repaired
        )
    }
}

#[derive(Clone, Debug)]
enum RepairMutation {
    Remove,
    Replace(Vec<u8>),
}

#[derive(Clone, Debug)]
struct RepairAction {
    order: u8,
    path: PathBuf,
    description: String,
    mutation: RepairMutation,
}

#[derive(Clone, Debug)]
struct RecoveryAnalysis {
    authority_root: PathBuf,
    committed_plan_count: u32,
    actions: Vec<RepairAction>,
}

#[derive(Clone, Debug)]
struct FixedDependency {
    path: PathBuf,
    name: &'static str,
    record_size: usize,
    bytes: Vec<u8>,
    complete_count: u32,
}

/// Complete read-only pre-admission check for both malformed files and
/// structurally valid dependency tails that no committed Plan root reaches.
pub fn plan_binary_db_recovery_required(authority_root: &Path) -> Result<bool, String> {
    analyze_plan_authority(authority_root).map(|analysis| !analysis.actions.is_empty())
}

/// Read-only diagnosis of Plan authority. This never repairs or synthesizes
/// bytes and reports referenced damage as blocked.
pub fn inspect_plan_binary_db_authority(authority_root: &Path) -> PlanBinaryDbRecoveryReport {
    match analyze_plan_authority(authority_root) {
        Ok(analysis) => PlanBinaryDbRecoveryReport {
            state: if analysis.actions.is_empty() {
                PlanBinaryDbRecoveryState::Clean
            } else {
                PlanBinaryDbRecoveryState::Repairable
            },
            authority_root: analysis.authority_root,
            committed_plan_count: analysis.committed_plan_count,
            repair_candidates: analysis
                .actions
                .iter()
                .map(|action| action.description.clone())
                .collect(),
            repairs: Vec::new(),
            issues: Vec::new(),
        },
        Err(issue) => PlanBinaryDbRecoveryReport {
            state: PlanBinaryDbRecoveryState::Blocked,
            authority_root: authority_root.to_path_buf(),
            committed_plan_count: 0,
            repair_candidates: Vec::new(),
            repairs: Vec::new(),
            issues: vec![issue],
        },
    }
}

/// Repairs only bytes that are proven unreachable from every complete
/// `plan.bin` root. Referenced damage fails closed with a restore instruction.
pub fn repair_plan_binary_db_authority(
    authority_root: &Path,
) -> Result<PlanBinaryDbRecoveryReport, String> {
    let authority = StorePath::from(authority_root.to_path_buf());
    let mut lock =
        BinaryDbCommandLockSet::acquire(&authority, BinaryDbCommandScope::PlanSyncLocalPlan)
            .map_err(|error| format!("failed to acquire Plan recovery lock: {error}"))?;
    let mut analysis = analyze_plan_authority(authority_root).map_err(blocked_recovery_error)?;
    if analysis.actions.is_empty() {
        lock.release()
            .map_err(|error| format!("failed to release Plan recovery lock: {error}"))?;
        return Ok(PlanBinaryDbRecoveryReport {
            state: PlanBinaryDbRecoveryState::Clean,
            authority_root: analysis.authority_root,
            committed_plan_count: analysis.committed_plan_count,
            repair_candidates: Vec::new(),
            repairs: Vec::new(),
            issues: Vec::new(),
        });
    }

    analysis.actions.sort_by_key(|action| action.order);
    let repairs = analysis
        .actions
        .iter()
        .map(|action| action.description.clone())
        .collect::<Vec<_>>();
    let repo_root = authority_root
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| authority_root.parent().unwrap_or(authority_root));
    let db = LocalBinaryDbFs::new(
        authority_root.to_path_buf(),
        repo_root.to_path_buf(),
        AuthorityId::new("plan-recovery"),
        LocalStateScope::Repository,
    );
    let files = FilesystemFileIoStore;
    let mut staging_directories = BTreeSet::new();
    for action in &analysis.actions {
        match &action.mutation {
            RepairMutation::Remove => files
                .remove_file_if_exists(&action.path)
                .map_err(|error| format!("failed to apply {}: {error}", action.description))?,
            RepairMutation::Replace(bytes) => {
                let staging = db
                    .replace_file_atomically(&action.path, bytes, "Plan recovery prefix")
                    .map_err(|error| format!("failed to apply {}: {error}", action.description))?;
                staging_directories.insert(staging);
            }
        }
    }
    files
        .sync_dir(authority_root)
        .map_err(|error| format!("failed to sync repaired Plan authority: {error}"))?;
    for staging in staging_directories {
        let _ = files.sync_dir(&staging);
    }

    let verified = analyze_plan_authority(authority_root).map_err(blocked_recovery_error)?;
    if !verified.actions.is_empty() {
        return Err(format!(
            "Plan Binary DB recovery did not converge: {}",
            verified
                .actions
                .iter()
                .map(|action| action.description.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    lock.release()
        .map_err(|error| format!("failed to release Plan recovery lock: {error}"))?;
    Ok(PlanBinaryDbRecoveryReport {
        state: PlanBinaryDbRecoveryState::Repaired,
        authority_root: verified.authority_root,
        committed_plan_count: verified.committed_plan_count,
        repair_candidates: Vec::new(),
        repairs,
        issues: Vec::new(),
    })
}

pub fn repair_plan_binary_db_authority_if_needed(
    authority_root: &Path,
) -> Result<Option<PlanBinaryDbRecoveryReport>, String> {
    let analysis = analyze_plan_authority(authority_root).map_err(blocked_recovery_error)?;
    if analysis.actions.is_empty() {
        return Ok(None);
    }
    repair_plan_binary_db_authority(authority_root).map(Some)
}

fn blocked_recovery_error(issue: String) -> String {
    format!(
        "Plan Binary DB recovery is blocked because committed authority references missing or malformed data: {issue} Restore a known-good .ait/binary-db generation or backup before retrying."
    )
}

fn analyze_plan_authority(authority_root: &Path) -> Result<RecoveryAnalysis, String> {
    if !authority_root.is_dir() {
        return Ok(RecoveryAnalysis {
            authority_root: authority_root.to_path_buf(),
            committed_plan_count: 0,
            actions: Vec::new(),
        });
    }
    let mut actions = Vec::new();
    let (plans, root_action) = load_plan_roots(authority_root)?;
    if let Some(action) = root_action {
        actions.push(action);
    }

    let plan_payload_ranges = plans
        .iter()
        .enumerate()
        .map(|(index, record)| {
            (
                record.payload_offset,
                u32::from(record.payload_len),
                format!("plan.bin[{index}].payload"),
            )
        })
        .collect::<Vec<_>>();
    if let Some(action) =
        analyze_payload_file(authority_root, PLAN_PAYLOAD_BIN, &plan_payload_ranges, 20)?
    {
        actions.push(action);
    }

    let head_revisions = plans
        .iter()
        .enumerate()
        .filter_map(|(plan_index, plan)| {
            plan.latest_revision_index()
                .map(|revision_index| (plan_index as u32, revision_index))
        })
        .collect::<Vec<_>>();
    let revisions = load_fixed_dependency(
        authority_root,
        PLAN_REVISION_BIN,
        PLAN_REVISION_RECORD_SIZE,
        !head_revisions.is_empty(),
    )?;
    let mut reachable_revisions = BTreeSet::new();
    let mut revision_records = Vec::<(u32, PlanRevisionRecord)>::new();
    for (plan_index, head_index) in head_revisions {
        let mut next = Some(head_index);
        let mut chain = BTreeSet::new();
        while let Some(revision_index) = next {
            if !chain.insert(revision_index) {
                return Err(format!(
                    "plan.bin[{plan_index}] revision chain contains a cycle at plan_revision.bin[{revision_index}]"
                ));
            }
            let raw = fixed_record(&revisions, revision_index)?;
            let record =
                PlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_record(raw).map_err(|error| {
                    format!("cannot decode plan_revision.bin[{revision_index}]: {error}")
                })?;
            if record.plan_index != plan_index {
                return Err(format!(
                    "plan_revision.bin[{revision_index}] belongs to plan {}, but plan.bin[{plan_index}] references it",
                    record.plan_index
                ));
            }
            if reachable_revisions.insert(revision_index) {
                revision_records.push((revision_index, record.clone()));
            }
            next = record.previous_revision_index();
        }
    }
    if let Some(action) = prefix_action_for_dependency(
        &revisions,
        reachable_revisions
            .iter()
            .next_back()
            .map(|index| index + 1),
        60,
    )? {
        actions.push(action);
    }

    let revision_payload_ranges = revision_records
        .iter()
        .map(|(index, record)| {
            (
                record.payload_offset,
                u32::from(record.payload_len),
                format!("plan_revision.bin[{index}].payload"),
            )
        })
        .collect::<Vec<_>>();
    if let Some(action) = analyze_payload_file(
        authority_root,
        PLAN_REVISION_PAYLOAD_BIN,
        &revision_payload_ranges,
        40,
    )? {
        actions.push(action);
    }

    let mut reachable_items = BTreeSet::new();
    for (revision_index, revision) in &revision_records {
        for offset in 0..u32::from(revision.item_count) {
            let item_index = revision
                .item_start_index
                .checked_add(offset)
                .ok_or_else(|| {
                    format!("plan_revision.bin[{revision_index}] item range overflows u32")
                })?;
            reachable_items.insert(item_index);
        }
    }
    let items = load_fixed_dependency(
        authority_root,
        PLAN_ITEM_BIN,
        PLAN_ITEM_RECORD_SIZE,
        !reachable_items.is_empty(),
    )?;
    let mut item_records = Vec::<(u32, PlanItemRecord)>::new();
    for item_index in &reachable_items {
        let raw = fixed_record(&items, *item_index)?;
        let record = PlanItemCodec::<PLAN_LAYOUT_ID>::decode_record(raw)
            .map_err(|error| format!("cannot decode plan_item.bin[{item_index}]: {error}"))?;
        item_records.push((*item_index, record));
    }
    if let Some(action) = prefix_action_for_dependency(
        &items,
        reachable_items.iter().next_back().map(|index| index + 1),
        50,
    )? {
        actions.push(action);
    }

    let item_payload_ranges = item_records
        .iter()
        .map(|(index, record)| {
            (
                record.payload_offset,
                u32::from(record.payload_len),
                format!("plan_item.bin[{index}].payload"),
            )
        })
        .collect::<Vec<_>>();
    if let Some(action) = analyze_payload_file(
        authority_root,
        PLAN_ITEM_PAYLOAD_BIN,
        &item_payload_ranges,
        30,
    )? {
        actions.push(action);
    }

    Ok(RecoveryAnalysis {
        authority_root: authority_root.to_path_buf(),
        committed_plan_count: u32::try_from(plans.len())
            .map_err(|_| "plan.bin record count exceeds u32::MAX".to_string())?,
        actions,
    })
}

fn load_plan_roots(
    authority_root: &Path,
) -> Result<(Vec<PlanRecord>, Option<RepairAction>), String> {
    let path = authority_root.join(PLAN_BIN);
    let Some(bytes) = read_optional(&path)? else {
        return Ok((Vec::new(), None));
    };
    let header_len = usize::try_from(BIN_FILE_HEADER_BYTES).unwrap_or(4);
    let record_size = usize::try_from(PLAN_RECORD_SIZE)
        .map_err(|_| "plan.bin record size overflows usize".to_string())?;
    if bytes.len() < header_len
        || bytes.get(..header_len) != Some(PLAN_LAYOUT_ID.to_le_bytes().as_slice())
    {
        if bytes.len() < header_len + record_size {
            return Ok((
                Vec::new(),
                Some(remove_action(
                    90,
                    path,
                    "remove incomplete uncommitted plan.bin root",
                )),
            ));
        }
        return Err(
            "plan.bin has a malformed layout header while still containing possible committed roots"
                .to_string(),
        );
    }
    let complete_count = (bytes.len() - header_len) / record_size;
    let complete_len = header_len + complete_count * record_size;
    let mut plans = Vec::with_capacity(complete_count);
    for (index, raw) in bytes[header_len..complete_len]
        .chunks_exact(record_size)
        .enumerate()
    {
        plans.push(
            PlanCodec::<PLAN_LAYOUT_ID>::decode_record(raw)
                .map_err(|error| format!("cannot decode plan.bin[{index}]: {error}"))?,
        );
    }
    let action = (complete_count == 0 || complete_len != bytes.len()).then(|| RepairAction {
        order: 90,
        path,
        description: if complete_count == 0 {
            "remove incomplete uncommitted plan.bin root".to_string()
        } else {
            format!(
                "truncate uncommitted partial plan.bin tail from {} to {complete_len} bytes",
                bytes.len()
            )
        },
        mutation: if complete_count == 0 {
            RepairMutation::Remove
        } else {
            RepairMutation::Replace(bytes[..complete_len].to_vec())
        },
    });
    Ok((plans, action))
}

fn load_fixed_dependency(
    authority_root: &Path,
    name: &'static str,
    record_size: u32,
    required: bool,
) -> Result<FixedDependency, String> {
    let path = authority_root.join(name);
    let Some(bytes) = read_optional(&path)? else {
        if required {
            return Err(format!("referenced {name} is missing"));
        }
        return Ok(FixedDependency {
            path,
            name,
            record_size: usize::try_from(record_size)
                .map_err(|_| format!("{name} record size overflows usize"))?,
            bytes: Vec::new(),
            complete_count: 0,
        });
    };
    let header_len = usize::try_from(BIN_FILE_HEADER_BYTES).unwrap_or(4);
    let record_size =
        usize::try_from(record_size).map_err(|_| format!("{name} record size overflows usize"))?;
    if !required {
        return Ok(FixedDependency {
            path,
            name,
            record_size,
            bytes,
            complete_count: 0,
        });
    }
    if bytes.len() < header_len
        || bytes.get(..header_len) != Some(PLAN_LAYOUT_ID.to_le_bytes().as_slice())
    {
        return Err(format!(
            "referenced {name} has a missing or malformed layout header"
        ));
    }
    let complete_count = u32::try_from((bytes.len() - header_len) / record_size)
        .map_err(|_| format!("{name} record count exceeds u32::MAX"))?;
    Ok(FixedDependency {
        path,
        name,
        record_size,
        bytes,
        complete_count,
    })
}

fn fixed_record(file: &FixedDependency, index: u32) -> Result<&[u8], String> {
    if index >= file.complete_count {
        return Err(format!(
            "committed Plan root references {}[{index}], but only {} complete records exist",
            file.name, file.complete_count
        ));
    }
    let header_len = usize::try_from(BIN_FILE_HEADER_BYTES).unwrap_or(4);
    let offset = header_len
        + usize::try_from(index)
            .map_err(|_| format!("{} index overflows usize: {index}", file.name))?
            * file.record_size;
    Ok(&file.bytes[offset..offset + file.record_size])
}

fn prefix_action_for_dependency(
    file: &FixedDependency,
    desired_count: Option<u32>,
    order: u8,
) -> Result<Option<RepairAction>, String> {
    let Some(desired_count) = desired_count else {
        return Ok((!file.bytes.is_empty()).then(|| {
            remove_action(
                order,
                file.path.clone(),
                format!("remove unreferenced {}", file.name),
            )
        }));
    };
    if desired_count > file.complete_count {
        return Err(format!(
            "committed Plan roots require {desired_count} {} records, but only {} complete records exist",
            file.name, file.complete_count
        ));
    }
    let header_len = usize::try_from(BIN_FILE_HEADER_BYTES).unwrap_or(4);
    let desired_len = header_len
        .checked_add(
            usize::try_from(desired_count)
                .map_err(|_| format!("{} desired count overflows usize", file.name))?
                .checked_mul(file.record_size)
                .ok_or_else(|| format!("{} desired prefix overflows usize", file.name))?,
        )
        .ok_or_else(|| format!("{} desired prefix overflows usize", file.name))?;
    Ok((file.bytes.len() != desired_len).then(|| RepairAction {
        order,
        path: file.path.clone(),
        description: format!(
            "truncate unreferenced {} tail from {} to {desired_len} bytes",
            file.name,
            file.bytes.len()
        ),
        mutation: RepairMutation::Replace(file.bytes[..desired_len].to_vec()),
    }))
}

fn analyze_payload_file(
    authority_root: &Path,
    name: &'static str,
    ranges: &[(u64, u32, String)],
    order: u8,
) -> Result<Option<RepairAction>, String> {
    let path = authority_root.join(name);
    let referenced = ranges
        .iter()
        .filter(|(_, len, _)| *len != 0)
        .collect::<Vec<_>>();
    let Some(metadata) = read_optional_metadata(&path)? else {
        if let Some((_, _, label)) = referenced.first() {
            return Err(format!("{label} references missing {name}"));
        }
        return Ok(None);
    };
    if referenced.is_empty() {
        return Ok(Some(remove_action(
            order,
            path,
            format!("remove unreferenced {name}"),
        )));
    }
    let file_len = metadata.len();
    let header_len = u64::from(BIN_FILE_HEADER_BYTES);
    if file_len < header_len || read_layout_header(&path)? != PLAN_LAYOUT_ID.to_le_bytes() {
        return Err(format!(
            "referenced {name} has a missing or malformed layout header"
        ));
    }
    let mut required_end = header_len;
    for (offset, len, label) in referenced {
        if *offset < header_len {
            return Err(format!(
                "{label} starts before the {name} layout header at offset {offset}"
            ));
        }
        let end = offset
            .checked_add(u64::from(*len))
            .ok_or_else(|| format!("{label} range overflows u64"))?;
        if end > file_len {
            return Err(format!(
                "{label} ends at {end}, but {name} contains only {file_len} bytes"
            ));
        }
        required_end = required_end.max(end);
    }
    let required_len = usize::try_from(required_end)
        .map_err(|_| format!("referenced {name} length exceeds address space"))?;
    if file_len == required_end {
        return Ok(None);
    }
    let prefix = read_prefix(&path, required_len)?;
    Ok(Some(RepairAction {
        order,
        path,
        description: format!(
            "truncate unreferenced {name} tail from {file_len} to {required_len} bytes"
        ),
        mutation: RepairMutation::Replace(prefix),
    }))
}

fn remove_action(order: u8, path: PathBuf, description: impl Into<String>) -> RepairAction {
    RepairAction {
        order,
        path,
        description: description.into(),
        mutation: RepairMutation::Remove,
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if read_optional_metadata(path)?.is_none() {
        return Ok(None);
    }
    fs::read(path)
        .map(Some)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))
}

fn read_optional_metadata(path: &Path) -> Result<Option<fs::Metadata>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to inspect {}: {error}", path.display())),
    };
    if !metadata.file_type().is_file() {
        return Err(format!(
            "Plan authority path must be a regular file: {}",
            path.display()
        ));
    }
    Ok(Some(metadata))
}

fn read_prefix(path: &Path, len: usize) -> Result<Vec<u8>, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let mut bytes = vec![0_u8; len];
    file.read_exact(&mut bytes)
        .map_err(|error| format!("failed to read {} prefix: {error}", path.display()))?;
    Ok(bytes)
}

fn read_layout_header(path: &Path) -> Result<[u8; 4], String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let mut header = [0_u8; 4];
    file.read_exact(&mut header)
        .map_err(|error| format!("failed to read {} layout header: {error}", path.display()))?;
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_binary_db::{
        LocalPlanBinaryDb, PlanItemPayload, PlanPayload, PlanRevisionPayload,
    };
    use tempfile::tempdir;

    #[test]
    fn recovery_removes_only_unreferenced_empty_and_header_only_plan_files() {
        let temp = tempdir().expect("tempdir");
        let authority = temp.path().join(".ait/binary-db");
        fs::create_dir_all(&authority).expect("create authority");
        fs::write(authority.join(PLAN_PAYLOAD_BIN), []).expect("empty plan payload");
        fs::write(
            authority.join(PLAN_REVISION_PAYLOAD_BIN),
            PLAN_LAYOUT_ID.to_le_bytes(),
        )
        .expect("header-only revision payload");

        assert!(plan_binary_db_recovery_required(&authority).expect("preflight"));
        let diagnostic = inspect_plan_binary_db_authority(&authority);
        assert_eq!(diagnostic.state, PlanBinaryDbRecoveryState::Repairable);
        assert_eq!(diagnostic.committed_plan_count, 0);

        let repaired = repair_plan_binary_db_authority(&authority).expect("repair");
        assert_eq!(repaired.state, PlanBinaryDbRecoveryState::Repaired);
        assert!(!authority.join(PLAN_PAYLOAD_BIN).exists());
        assert!(!authority.join(PLAN_REVISION_PAYLOAD_BIN).exists());
        assert_eq!(
            inspect_plan_binary_db_authority(&authority).state,
            PlanBinaryDbRecoveryState::Clean
        );
    }

    #[test]
    fn recovery_fails_closed_when_a_committed_plan_references_missing_payload() {
        let temp = tempdir().expect("tempdir");
        let authority = temp.path().join(".ait/binary-db");
        fs::create_dir_all(&authority).expect("create authority");
        let record = PlanRecord {
            plan_meta: 0,
            reserved0: 0,
            payload_len: 5,
            payload_offset: 4,
            latest_revision_index_plus1: 0,
            published_plan_index_plus1: 0,
            published_latest_revision_index_plus1: 0,
            created_at_s: 1,
            updated_at_s: 1,
            published_at_s: 0,
        };
        write_fixed_file(
            &authority.join(PLAN_BIN),
            &[PlanCodec::<PLAN_LAYOUT_ID>::encode_record(&record).expect("encode plan")],
        );
        fs::write(authority.join(PLAN_PAYLOAD_BIN), []).expect("empty referenced payload");

        let diagnostic = inspect_plan_binary_db_authority(&authority);
        assert_eq!(diagnostic.state, PlanBinaryDbRecoveryState::Blocked);
        assert!(
            diagnostic.issues[0].contains("referenced plan_payload.bin"),
            "issues: {:?}",
            diagnostic.issues
        );
        let error = repair_plan_binary_db_authority(&authority)
            .expect_err("referenced damage must not be repaired");
        assert!(error.contains("recovery is blocked"));
        assert!(error.contains("Restore a known-good"));
        assert!(authority.join(PLAN_BIN).exists());
        assert_eq!(
            fs::metadata(authority.join(PLAN_PAYLOAD_BIN))
                .expect("payload remains")
                .len(),
            0
        );
    }

    #[test]
    fn recovery_trims_partial_dependency_and_payload_tails_without_changing_committed_view() {
        let temp = tempdir().expect("tempdir");
        let authority = temp.path().join(".ait/binary-db");
        fs::create_dir_all(&authority).expect("create authority");
        seed_complete_plan(&authority);

        append_bytes(&authority.join(PLAN_REVISION_BIN), &[0xaa, 0xbb, 0xcc]);
        append_bytes(&authority.join(PLAN_ITEM_BIN), &[0xdd, 0xee]);
        append_bytes(
            &authority.join(PLAN_REVISION_PAYLOAD_BIN),
            b"orphan-revision-tail",
        );
        append_bytes(&authority.join(PLAN_ITEM_PAYLOAD_BIN), b"orphan-item-tail");
        append_bytes(&authority.join(PLAN_PAYLOAD_BIN), b"orphan-title-tail");

        assert!(
            plan_binary_db_recovery_required(&authority).expect("tail preflight"),
            "structurally valid payload tails must still trigger reachability recovery"
        );
        let repaired = repair_plan_binary_db_authority_if_needed(&authority)
            .expect("repair tails")
            .expect("tail recovery report");
        assert_eq!(repaired.state, PlanBinaryDbRecoveryState::Repaired);
        assert!(repaired
            .repairs
            .iter()
            .any(|repair| repair.contains("plan_revision.bin tail")));

        let plans = LocalPlanBinaryDb::<PLAN_LAYOUT_ID>::new(
            authority.clone(),
            temp.path(),
            AuthorityId::new("recovery-test"),
            LocalStateScope::Repository,
        );
        let read = plans.begin_read_txn();
        let view = plans
            .get_plan(&read, 0, Some("fixture"))
            .expect("read committed plan after repair");
        assert_eq!(view.title_text().expect("title"), "Power-safe plan");
        assert_eq!(
            view.head_revision
                .as_ref()
                .expect("head")
                .record
                .revision_number,
            1
        );
        assert_eq!(view.head_revision.as_ref().expect("head").items.len(), 1);
        assert_eq!(
            inspect_plan_binary_db_authority(&authority).state,
            PlanBinaryDbRecoveryState::Clean
        );
    }

    #[test]
    fn recovery_preserves_complete_plan_root_prefix_after_torn_next_root() {
        let temp = tempdir().expect("tempdir");
        let authority = temp.path().join(".ait/binary-db");
        fs::create_dir_all(&authority).expect("create authority");
        seed_complete_plan(&authority);
        let committed_len = fs::metadata(authority.join(PLAN_BIN))
            .expect("plan metadata")
            .len();
        append_bytes(&authority.join(PLAN_BIN), &[1, 2, 3, 4, 5, 6, 7]);

        let repaired = repair_plan_binary_db_authority(&authority).expect("repair root tail");
        assert_eq!(repaired.committed_plan_count, 1);
        assert_eq!(
            fs::metadata(authority.join(PLAN_BIN))
                .expect("repaired plan metadata")
                .len(),
            committed_len
        );
        assert_eq!(
            inspect_plan_binary_db_authority(&authority).state,
            PlanBinaryDbRecoveryState::Clean
        );
    }

    #[test]
    fn recovery_does_not_treat_remote_publication_mapping_as_a_local_revision_locator() {
        let temp = tempdir().expect("tempdir");
        let authority = temp.path().join(".ait/binary-db");
        fs::create_dir_all(&authority).expect("create authority");
        seed_complete_plan(&authority);

        let bytes = fs::read(authority.join(PLAN_BIN)).expect("read plan root");
        let mut plan =
            PlanCodec::<PLAN_LAYOUT_ID>::decode_record(&bytes[4..]).expect("decode plan root");
        plan.published_plan_index_plus1 = 42;
        plan.published_latest_revision_index_plus1 = 43;
        write_fixed_file(
            &authority.join(PLAN_BIN),
            &[
                PlanCodec::<PLAN_LAYOUT_ID>::encode_record(&plan)
                    .expect("encode published mapping"),
            ],
        );

        let report = inspect_plan_binary_db_authority(&authority);
        assert_eq!(report.state, PlanBinaryDbRecoveryState::Clean);
        assert!(!plan_binary_db_recovery_required(&authority).expect("mapping preflight"));
    }

    fn seed_complete_plan(authority: &Path) {
        let plan_payload = PlanCodec::<PLAN_LAYOUT_ID>::encode_payload(&PlanPayload {
            title_bytes: b"Power-safe plan".to_vec(),
        })
        .expect("encode plan payload");
        write_payload_file(&authority.join(PLAN_PAYLOAD_BIN), &plan_payload);

        let item_payload = PlanItemCodec::<PLAN_LAYOUT_ID>::encode_payload(&PlanItemPayload {
            plan_item_ref_bytes: b"power-safe/item".to_vec(),
            text_bytes: b"Keep the old or new state".to_vec(),
            heading_path: vec!["Work".to_string()],
        })
        .expect("encode item payload");
        write_payload_file(&authority.join(PLAN_ITEM_PAYLOAD_BIN), &item_payload);
        let item = PlanItemRecord {
            item_meta: 1 | 0b0000_0100,
            reserved0: 0,
            payload_len: u16::try_from(item_payload.len()).expect("item payload length"),
            payload_offset: 4,
            line_number: 12,
        };
        write_fixed_file(
            &authority.join(PLAN_ITEM_BIN),
            &[PlanItemCodec::<PLAN_LAYOUT_ID>::encode_record(&item).expect("encode item")],
        );

        let revision_payload =
            PlanRevisionCodec::<PLAN_LAYOUT_ID>::encode_payload(&PlanRevisionPayload {
                title_snapshot_bytes: b"Power-safe plan".to_vec(),
                summary_bytes: b"atomic".to_vec(),
                artifact_path_bytes: b"docs/sprints/power-safe.md".to_vec(),
                artifact_selector_bytes: Vec::new(),
                artifact_heading_bytes: b"Power safe".to_vec(),
                artifact_blob_id_bytes: Vec::new(),
            })
            .expect("encode revision payload");
        write_payload_file(
            &authority.join(PLAN_REVISION_PAYLOAD_BIN),
            &revision_payload,
        );
        let revision = PlanRevisionRecord {
            revision_meta: 0,
            reserved0: 0,
            payload_len: u16::try_from(revision_payload.len()).expect("revision payload length"),
            revision_number: 1,
            item_count: 1,
            payload_offset: 4,
            plan_index: 0,
            previous_revision_index_plus1: 0,
            item_start_index: 0,
            published_revision_index_plus1: 0,
            root_tree_pack_index_plus1: 0,
            root_entry_ordinal: 0,
            created_at_s: 1,
            published_at_s: 0,
        };
        write_fixed_file(
            &authority.join(PLAN_REVISION_BIN),
            &[
                PlanRevisionCodec::<PLAN_LAYOUT_ID>::encode_record(&revision)
                    .expect("encode revision"),
            ],
        );

        let plan = PlanRecord {
            plan_meta: 0,
            reserved0: 0,
            payload_len: u16::try_from(plan_payload.len()).expect("plan payload length"),
            payload_offset: 4,
            latest_revision_index_plus1: 1,
            published_plan_index_plus1: 0,
            published_latest_revision_index_plus1: 0,
            created_at_s: 1,
            updated_at_s: 1,
            published_at_s: 0,
        };
        write_fixed_file(
            &authority.join(PLAN_BIN),
            &[PlanCodec::<PLAN_LAYOUT_ID>::encode_record(&plan).expect("encode plan")],
        );
    }

    fn write_payload_file(path: &Path, payload: &[u8]) {
        let mut bytes = PLAN_LAYOUT_ID.to_le_bytes().to_vec();
        bytes.extend_from_slice(payload);
        fs::write(path, bytes).expect("write payload file");
    }

    fn write_fixed_file(path: &Path, records: &[Vec<u8>]) {
        let mut bytes = PLAN_LAYOUT_ID.to_le_bytes().to_vec();
        for record in records {
            bytes.extend_from_slice(record);
        }
        fs::write(path, bytes).expect("write fixed file");
    }

    fn append_bytes(path: &Path, suffix: &[u8]) {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(path)
            .expect("open append target");
        file.write_all(suffix).expect("append bytes");
    }
}
