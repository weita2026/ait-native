use super::generation_content_indexes::validate_content_identity_indexes;
use super::{GenerationResult, Path};
use crate::binary_db::{
    AuthorityId, LocalBinaryDbFs, LocalStateScope, REPOSITORY_BINARY_DB_BIN_PATHS,
    REPOSITORY_BINARY_DB_INDEX_PATHS,
};
use crate::content_binary_db::{
    blob_id_from_sha256, tree_id_from_hash80, BinaryBlobCodec, BinaryDbTreeReadCache,
    BinaryObjectPackCodec, BinaryObjectPackMemberCodec, BinarySnapshotCodec, BinarySnapshotRecord,
    BinaryTreeCodec, BinaryTreePackCodec, BinaryTreePackRecord, LocalContentBinaryDb, BLOB_BIN,
    BLOB_RECORD_SIZE, OBJECT_PACK_BIN, OBJECT_PACK_MEMBER_BIN, OBJECT_PACK_MEMBER_RECORD_SIZE,
    OBJECT_PACK_RECORD_SIZE, SNAPSHOT_BIN, SNAPSHOT_PAYLOAD_BIN, SNAPSHOT_RECORD_SIZE, TREE_BIN,
    TREE_PACK_BIN, TREE_PACK_RECORD_SIZE, TREE_RECORD_SIZE,
};
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};

const FILE_HEADER_BYTES: u64 = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RepositoryContentClosureSummary {
    pub(super) blob_count: u32,
    pub(super) snapshot_count: u32,
    pub(super) object_pack_count: u32,
    pub(super) object_pack_member_count: u32,
    pub(super) tree_pack_count: u32,
    pub(super) tree_count: u32,
    pub(super) tree_entry_count: u64,
}

pub(super) fn validate_repository_content_closure(
    authority_root: &Path,
    pack_root: &Path,
) -> GenerationResult<RepositoryContentClosureSummary> {
    let blob_count = record_count(authority_root, BLOB_BIN, BLOB_RECORD_SIZE)?;
    let snapshot_count = record_count(authority_root, SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE)?;
    let object_pack_count = record_count(authority_root, OBJECT_PACK_BIN, OBJECT_PACK_RECORD_SIZE)?;
    let object_pack_member_count = record_count(
        authority_root,
        OBJECT_PACK_MEMBER_BIN,
        OBJECT_PACK_MEMBER_RECORD_SIZE,
    )?;
    let tree_pack_count = record_count(authority_root, TREE_PACK_BIN, TREE_PACK_RECORD_SIZE)?;
    let tree_count = record_count(authority_root, TREE_BIN, TREE_RECORD_SIZE)?;
    let snapshot_payload_bytes = payload_bytes(authority_root, SNAPSHOT_PAYLOAD_BIN)?;

    let mut active_blob_ids = BTreeSet::new();
    let mut blob_member_pointers = Vec::with_capacity(blob_count as usize);
    let mut blob_records = Vec::with_capacity(blob_count as usize);
    validate_records(
        authority_root,
        BLOB_BIN,
        BLOB_RECORD_SIZE,
        blob_count,
        |index, raw| {
            let record = BinaryBlobCodec::<1>::decode_record(raw).map_err(|error| {
                format!("Binary DB content closure cannot decode blob {index}: {error}")
            })?;
            if record.is_tombstone() || record.is_pruned() {
                return Err(format!(
                    "Binary DB canonical generation contains unavailable blob {index}."
                ));
            }
            let blob_id = blob_id_from_sha256(&record.sha256).to_ascii_lowercase();
            // Blob indexes are append-only candidate lists. Multiple live records may
            // therefore carry the same content-addressed ID; closure validation below
            // still checks every record's pack-member pointers independently.
            active_blob_ids.insert(blob_id);
            let member_index = record.pack_member_index();
            if let Some(member_index) = member_index {
                require_index(
                    "blob",
                    index,
                    "object pack member",
                    member_index,
                    object_pack_member_count,
                )?;
            }
            blob_member_pointers.push(member_index);
            blob_records.push(record);
            Ok(())
        },
    )?;

    let mut object_pack_records = Vec::with_capacity(object_pack_count as usize);
    let mut expected_member_start = 0_u32;
    validate_records(
        authority_root,
        OBJECT_PACK_BIN,
        OBJECT_PACK_RECORD_SIZE,
        object_pack_count,
        |index, raw| {
            let record = BinaryObjectPackCodec::<1>::decode_record(raw).map_err(|error| {
                format!("Binary DB content closure cannot decode object pack {index}: {error}")
            })?;
            if record.is_tombstone() || !record.is_ready() {
                return Err(format!(
                    "Binary DB canonical generation contains unavailable object pack {index}."
                ));
            }
            require_dense_range(
                "object pack",
                index,
                OBJECT_PACK_MEMBER_BIN,
                record.first_member_index,
                record.member_count,
                object_pack_member_count,
                &mut expected_member_start,
            )?;
            object_pack_records.push(record);
            Ok(())
        },
    )?;
    require_dense_coverage(
        "object pack",
        OBJECT_PACK_MEMBER_BIN,
        expected_member_start,
        object_pack_member_count,
    )?;

    let mut object_member_records = Vec::with_capacity(object_pack_member_count as usize);
    validate_records(
        authority_root,
        OBJECT_PACK_MEMBER_BIN,
        OBJECT_PACK_MEMBER_RECORD_SIZE,
        object_pack_member_count,
        |index, raw| {
            let record = BinaryObjectPackMemberCodec::<1>::decode_record(raw).map_err(|error| {
                format!(
                    "Binary DB content closure cannot decode object pack member {index}: {error}"
                )
            })?;
            if record.is_tombstone() {
                return Err(format!(
                    "Binary DB canonical generation contains tombstoned object pack member {index}."
                ));
            }
            require_index(
                "object pack member",
                index,
                "object pack",
                record.pack_index,
                object_pack_count,
            )?;
            require_index(
                "object pack member",
                index,
                "blob",
                record.blob_index,
                blob_count,
            )?;
            if let Some(base_blob_index) = record.base_blob_index() {
                require_index(
                    "object pack member",
                    index,
                    "base blob",
                    base_blob_index,
                    blob_count,
                )?;
            }
            object_member_records.push(record);
            Ok(())
        },
    )?;
    validate_object_member_backreferences(
        &object_pack_records,
        &object_member_records,
        &blob_records,
        &blob_member_pointers,
    )?;

    let mut tree_pack_records = Vec::with_capacity(tree_pack_count as usize);
    let mut expected_tree_start = 0_u32;
    validate_records(
        authority_root,
        TREE_PACK_BIN,
        TREE_PACK_RECORD_SIZE,
        tree_pack_count,
        |index, raw| {
            let record = BinaryTreePackCodec::<1>::decode_record(raw).map_err(|error| {
                format!("Binary DB content closure cannot decode tree pack {index}: {error}")
            })?;
            if record.is_tombstone() || !record.is_ready() {
                return Err(format!(
                    "Binary DB canonical generation contains unavailable tree pack {index}."
                ));
            }
            require_dense_range(
                "tree pack",
                index,
                TREE_BIN,
                record.first_tree_index,
                record.tree_count,
                tree_count,
                &mut expected_tree_start,
            )?;
            tree_pack_records.push(record);
            Ok(())
        },
    )?;
    require_dense_coverage("tree pack", TREE_BIN, expected_tree_start, tree_count)?;

    let mut active_tree_ids = BTreeSet::new();
    let mut tree_records = Vec::with_capacity(tree_count as usize);
    let mut pack_cursor = 0_usize;
    validate_records(
        authority_root,
        TREE_BIN,
        TREE_RECORD_SIZE,
        tree_count,
        |index, raw| {
            while pack_cursor < tree_pack_records.len() {
                let pack = &tree_pack_records[pack_cursor];
                let end = checked_range_end(
                    "tree pack",
                    pack_cursor as u32,
                    pack.first_tree_index,
                    pack.tree_count,
                )?;
                if index < end {
                    break;
                }
                pack_cursor += 1;
            }
            let pack = tree_pack_records.get(pack_cursor).ok_or_else(|| {
                format!(
                    "Binary DB content closure tree {index} has no authoritative tree-pack range."
                )
            })?;
            let expected_ordinal = index.checked_sub(pack.first_tree_index).ok_or_else(|| {
                format!("Binary DB content closure tree {index} precedes tree pack {pack_cursor}.")
            })?;
            let record = BinaryTreeCodec::<1>::decode_record(raw).map_err(|error| {
                format!("Binary DB content closure cannot decode tree {index}: {error}")
            })?;
            if record.is_tombstone() {
                return Err(format!(
                    "Binary DB canonical generation contains tombstoned tree {index}."
                ));
            }
            if pack.is_tombstone() && !record.is_tombstone() {
                return Err(format!(
                    "Binary DB content closure tombstoned tree pack {pack_cursor} contains live tree {index}."
                ));
            }
            if !pack.has_sparse_physical_ordinals() && record.pack_entry_ordinal != expected_ordinal
            {
                return Err(format!(
                    "Binary DB content closure tree {index} has compact pack ordinal {}, expected {expected_ordinal}.",
                    record.pack_entry_ordinal
                ));
            }
            let tree_id = tree_id_from_hash80(&record.tree_hash80).to_ascii_lowercase();
            if !record.is_tombstone() {
                active_tree_ids.insert(tree_id);
            }
            tree_records.push(record);
            Ok(())
        },
    )?;

    let tree_entry_count = validate_tree_pack_payloads(
        authority_root,
        pack_root,
        &tree_pack_records,
        &tree_records,
        &active_blob_ids,
        &active_tree_ids,
    )?;

    let snapshot_payload_path = authority_root.join(SNAPSHOT_PAYLOAD_BIN);
    let mut snapshot_payload_file = if snapshot_count == 0 {
        None
    } else {
        Some(fs::File::open(&snapshot_payload_path).map_err(|error| {
            format!(
                "failed to open Snapshot payload authority {}: {error}",
                snapshot_payload_path.display()
            )
        })?)
    };
    let mut snapshot_records = Vec::with_capacity(snapshot_count as usize);
    let mut additional_parents_by_snapshot = Vec::with_capacity(snapshot_count as usize);
    validate_records(
        authority_root,
        SNAPSHOT_BIN,
        SNAPSHOT_RECORD_SIZE,
        snapshot_count,
        |index, raw| {
            let record = BinarySnapshotCodec::<1>::decode_record(raw).map_err(|error| {
                format!("Binary DB content closure cannot decode snapshot {index}: {error}")
            })?;
            if record.history_flags & !crate::content_binary_db::BinarySnapshotRecord::KNOWN_FLAGS
                != 0
            {
                return Err(format!(
                    "Binary DB content closure snapshot {index} has unknown history flags {:#04x}.",
                    record.history_flags
                ));
            }
            if let Some(parent_index) = record.parent_snapshot_index() {
                require_index(
                    "snapshot",
                    index,
                    "parent snapshot",
                    parent_index,
                    snapshot_count,
                )?;
            }
            if record.payload_len > 0 {
                require_payload_range(
                    "snapshot",
                    index,
                    SNAPSHOT_PAYLOAD_BIN,
                    record.payload_offset,
                    u64::from(record.payload_len),
                    snapshot_payload_bytes,
                )?;
            }
            let additional_parent_snapshot_indices = if record.is_tombstone()
                && record.payload_len == 0
            {
                Vec::new()
            } else {
                let payload_file = snapshot_payload_file.as_mut().ok_or_else(|| {
                    format!(
                        "Binary DB content closure snapshot {index} has no Snapshot payload authority."
                    )
                })?;
                let raw_payload = read_payload_slice(
                    payload_file,
                    record.payload_offset,
                    record.payload_len,
                    index,
                )?;
                let payload = BinarySnapshotCodec::<1>::decode_payload(
                    &raw_payload,
                    record.has_line_name_payload(),
                    record.has_additional_parents(),
                )
                .map_err(|error| {
                    format!(
                        "Binary DB content closure cannot decode snapshot payload {index}: {error}"
                    )
                })?;
                if record.has_message() != payload.message.is_some() {
                    return Err(format!(
                        "Binary DB content closure snapshot {index} message flag disagrees with its payload."
                    ));
                }
                payload.additional_parent_snapshot_indices
            };
            if !record.is_tombstone() {
                if let Some(tree_pack_index) = record.root_tree_pack_index() {
                    require_index(
                        "snapshot",
                        index,
                        "root tree pack",
                        tree_pack_index,
                        tree_pack_count,
                    )?;
                    let pack = &tree_pack_records[tree_pack_index as usize];
                    if pack.is_tombstone() {
                        return Err(format!(
                        "Binary DB content closure snapshot {index} references tombstoned root tree pack {tree_pack_index}."
                    ));
                    }
                    if record.root_entry_ordinal >= pack.tree_count {
                        return Err(format!(
                        "Binary DB content closure snapshot {index} root ordinal {} is outside tree pack {tree_pack_index} count {}.",
                        record.root_entry_ordinal, pack.tree_count
                    ));
                    }
                    let tree_index = pack
                    .first_tree_index
                    .checked_add(record.root_entry_ordinal)
                    .ok_or_else(|| {
                        format!(
                            "Binary DB content closure snapshot {index} root tree index overflows u32."
                        )
                    })?;
                    let tree = tree_records.get(tree_index as usize).ok_or_else(|| {
                    format!(
                        "Binary DB content closure snapshot {index} root tree {tree_index} is missing."
                    )
                })?;
                    if tree.is_tombstone() {
                        return Err(format!(
                        "Binary DB content closure snapshot {index} references tombstoned root tree {tree_index}."
                    ));
                    }
                }
            }
            snapshot_records.push(record);
            additional_parents_by_snapshot.push(additional_parent_snapshot_indices);
            Ok(())
        },
    )?;
    validate_snapshot_parent_authority(&snapshot_records, &additional_parents_by_snapshot)?;
    validate_content_identity_indexes(authority_root)?;

    Ok(RepositoryContentClosureSummary {
        blob_count,
        snapshot_count,
        object_pack_count,
        object_pack_member_count,
        tree_pack_count,
        tree_count,
        tree_entry_count,
    })
}

fn validate_object_member_backreferences(
    packs: &[crate::content_binary_db::BinaryObjectPackRecord],
    members: &[crate::content_binary_db::BinaryObjectPackMemberRecord],
    blobs: &[crate::content_binary_db::BinaryBlobRecord],
    blob_member_pointers: &[Option<u32>],
) -> GenerationResult<()> {
    let mut pack_cursor = 0_usize;
    for (member_index, member) in members.iter().enumerate() {
        while pack_cursor < packs.len() {
            let pack = &packs[pack_cursor];
            let end = checked_range_end(
                "object pack",
                pack_cursor as u32,
                pack.first_member_index,
                pack.member_count,
            )? as usize;
            if member_index < end {
                break;
            }
            pack_cursor += 1;
        }
        if member.pack_index as usize != pack_cursor {
            return Err(format!(
                "Binary DB content closure object pack member {member_index} points to pack {}, expected {pack_cursor}.",
                member.pack_index
            ));
        }
        let pack = &packs[pack_cursor];
        if pack.is_tombstone() && !member.is_tombstone() {
            return Err(format!(
                "Binary DB content closure tombstoned object pack {pack_cursor} contains live member {member_index}."
            ));
        }
        if member.is_tombstone() {
            if !pack.is_tombstone() {
                return Err(format!(
                    "Binary DB content closure active object pack {pack_cursor} contains tombstoned member {member_index}."
                ));
            }
            continue;
        }
        let blob = &blobs[member.blob_index as usize];
        if blob.is_tombstone() || blob.is_pruned() {
            return Err(format!(
                "Binary DB content closure live object pack member {member_index} references unavailable blob {}.",
                member.blob_index
            ));
        }
        if let Some(base_blob_index) = member.base_blob_index() {
            let base_blob = &blobs[base_blob_index as usize];
            if base_blob.is_tombstone() || base_blob.is_pruned() {
                return Err(format!(
                    "Binary DB content closure live object pack member {member_index} references unavailable base blob {base_blob_index}."
                ));
            }
        }
        if blob_member_pointers[member.blob_index as usize].is_none() {
            return Err(format!(
                "Binary DB content closure live object pack member {member_index} references blob {} without a selected object pack member.",
                member.blob_index
            ));
        }
    }
    for (blob_index, blob) in blobs.iter().enumerate() {
        if blob.is_tombstone() {
            continue;
        }
        let Some(member_index) = blob_member_pointers[blob_index] else {
            continue;
        };
        let member = members.get(member_index as usize).ok_or_else(|| {
            format!(
                "Binary DB content closure blob {blob_index} points to missing member {member_index}."
            )
        })?;
        if member.is_tombstone() || member.blob_index as usize != blob_index {
            return Err(format!(
                "Binary DB content closure live blob {blob_index} points to unavailable member {member_index}."
            ));
        }
    }
    Ok(())
}

fn validate_tree_pack_payloads(
    authority_root: &Path,
    pack_root: &Path,
    pack_records: &[BinaryTreePackRecord],
    tree_records: &[crate::content_binary_db::BinaryTreeRecord],
    active_blob_ids: &BTreeSet<String>,
    active_tree_ids: &BTreeSet<String>,
) -> GenerationResult<u64> {
    if tree_records.is_empty() {
        return Ok(0);
    }
    let db = LocalBinaryDbFs::new(
        authority_root.to_path_buf(),
        pack_root.to_path_buf(),
        AuthorityId::new("generation-content-closure"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS)
    .for_detached_generation_without_locks();
    let content = LocalContentBinaryDb::<1>::from_db_with_roots(
        db,
        pack_root.to_path_buf(),
        pack_root.to_path_buf(),
    );
    let read = content.trees().begin_read_txn();
    let packs = content
        .tree_packs()
        .list_tree_pack_views(&read)
        .map_err(|error| format!("Binary DB content closure cannot list tree packs: {error}"))?;
    let expected_active_pack_count = pack_records
        .iter()
        .filter(|record| !record.is_tombstone())
        .count();
    if packs.len() != expected_active_pack_count {
        return Err(format!(
            "Binary DB content closure read {} tree packs, expected {}.",
            packs.len(),
            expected_active_pack_count
        ));
    }

    let mut cache = BinaryDbTreeReadCache::default();
    let mut entry_total = 0_u64;
    for pack in &packs {
        let pack_index = pack.tree_pack_index as usize;
        let expected = pack_records.get(pack_index).ok_or_else(|| {
            format!("Binary DB content closure read out-of-range tree pack {pack_index}.")
        })?;
        if pack.record != *expected {
            return Err(format!(
                "Binary DB content closure tree pack {pack_index} changed while validating."
            ));
        }
        for ordinal in 0..pack.record.tree_count {
            let tree_index = pack
                .record
                .first_tree_index
                .checked_add(ordinal)
                .ok_or_else(|| {
                    format!("Binary DB content closure tree pack {pack_index} range overflows u32.")
                })?;
            let tree = tree_records.get(tree_index as usize).ok_or_else(|| {
                format!(
                    "Binary DB content closure tree pack {pack_index} references missing tree {tree_index}."
                )
            })?;
            if tree.is_tombstone() {
                continue;
            }
            let entries = content
                .trees()
                .list_tree_entry_views_for_record_in_pack_with_cache(
                    &read,
                    tree_index,
                    tree,
                    pack,
                    &mut cache,
                )
                .map_err(|error| {
                    format!(
                        "Binary DB content closure cannot read tree {tree_index} from pack {pack_index}: {error}"
                    )
                })?;
            let mut names = BTreeSet::new();
            for entry in &entries {
                if !names.insert(entry.entry_name.as_str()) {
                    return Err(format!(
                        "Binary DB content closure tree {tree_index} contains duplicate entry name {:?}.",
                        entry.entry_name
                    ));
                }
                let target = entry.target_id.to_ascii_lowercase();
                let target_exists = match entry.entry_type.as_str() {
                    "blob" => active_blob_ids.contains(&target),
                    "tree" => active_tree_ids.contains(&target),
                    other => {
                        return Err(format!(
                            "Binary DB content closure tree {tree_index} contains unsupported entry type {other:?}."
                        ));
                    }
                };
                if !target_exists {
                    return Err(format!(
                        "Binary DB content closure tree {tree_index} entry {:?} references missing active {} {}.",
                        entry.entry_name, entry.entry_type, entry.target_id
                    ));
                }
            }
            entry_total = entry_total
                .checked_add(entries.len() as u64)
                .ok_or_else(|| {
                    "Binary DB content closure tree entry count overflows u64.".to_string()
                })?;
        }
    }
    Ok(entry_total)
}

fn validate_snapshot_parent_authority(
    snapshots: &[BinarySnapshotRecord],
    additional_parents_by_snapshot: &[Vec<u32>],
) -> GenerationResult<()> {
    if snapshots.len() != additional_parents_by_snapshot.len() {
        return Err(
            "Binary DB content closure lost Snapshot parent payload alignment.".to_string(),
        );
    }
    let mut parents = vec![Vec::new(); snapshots.len()];
    for (snapshot_index, snapshot) in snapshots.iter().enumerate() {
        let additional_parents = &additional_parents_by_snapshot[snapshot_index];
        if snapshot.is_tombstone() {
            if !additional_parents.is_empty() {
                return Err(format!(
                    "Binary DB content closure tombstoned snapshot {snapshot_index} has additional parents."
                ));
            }
            continue;
        }
        if snapshot.has_additional_parents() == additional_parents.is_empty() {
            return Err(format!(
                "Binary DB content closure snapshot {snapshot_index} additional-parent flag disagrees with its payload."
            ));
        }
        if snapshot.is_remote_head_history_boundary()
            && (snapshot.parent_snapshot_index().is_some() || !additional_parents.is_empty())
        {
            return Err(format!(
                "Binary DB content closure snapshot {snapshot_index} is a remote-head history boundary but also has local parents."
            ));
        }
        if let Some(parent) = snapshot.parent_snapshot_index() {
            parents[snapshot_index].push(parent);
            parents[snapshot_index].extend_from_slice(additional_parents);
        } else if !additional_parents.is_empty() {
            return Err(format!(
                "Binary DB content closure snapshot {snapshot_index} has additional parents without ordinal zero."
            ));
        }
        let mut seen = BTreeSet::new();
        for (ordinal, parent) in parents[snapshot_index].iter().copied().enumerate() {
            let parent_snapshot = snapshots.get(parent as usize).ok_or_else(|| {
                format!(
                    "Binary DB content closure snapshot {snapshot_index} references missing parent snapshot {parent}."
                )
            })?;
            if parent == snapshot_index as u32 {
                return Err(format!(
                    "Binary DB content closure snapshot {snapshot_index} points at itself."
                ));
            }
            if parent >= snapshot_index as u32 {
                return Err(format!(
                    "Binary DB content closure snapshot {snapshot_index} parent ordinal {ordinal} must reference an earlier record, got {parent}."
                ));
            }
            if !seen.insert(parent) {
                return Err(format!(
                    "Binary DB content closure snapshot {snapshot_index} contains duplicate parent snapshot {parent}."
                ));
            }
            if parent_snapshot.is_tombstone() {
                return Err(format!(
                    "Binary DB content closure snapshot {snapshot_index} references tombstoned parent snapshot {parent}."
                ));
            }
        }
    }
    validate_snapshot_parent_graph(&parents)
}

fn read_payload_slice(
    file: &mut fs::File,
    offset: u64,
    length: u16,
    snapshot_index: u32,
) -> GenerationResult<Vec<u8>> {
    file.seek(SeekFrom::Start(offset)).map_err(|error| {
        format!("failed to seek Snapshot payload for record {snapshot_index} to {offset}: {error}")
    })?;
    let mut raw = vec![0_u8; usize::from(length)];
    file.read_exact(&mut raw).map_err(|error| {
        format!("failed to read Snapshot payload for record {snapshot_index}: {error}")
    })?;
    Ok(raw)
}

fn validate_snapshot_parent_graph(parents: &[Vec<u32>]) -> GenerationResult<()> {
    let mut states = vec![0_u8; parents.len()];
    for start in 0..parents.len() {
        if states[start] == 2 {
            continue;
        }
        visit_snapshot_parent_graph(start, parents, &mut states)?;
    }
    Ok(())
}

fn visit_snapshot_parent_graph(
    index: usize,
    parents: &[Vec<u32>],
    states: &mut [u8],
) -> GenerationResult<()> {
    match states[index] {
        2 => return Ok(()),
        1 => {
            return Err(format!(
                "Binary DB content closure snapshot parent graph contains a cycle at snapshot {index}."
            ));
        }
        _ => {}
    }
    states[index] = 1;
    for parent in &parents[index] {
        visit_snapshot_parent_graph(*parent as usize, parents, states)?;
    }
    states[index] = 2;
    Ok(())
}

fn record_count(authority_root: &Path, name: &str, record_size: u32) -> GenerationResult<u32> {
    let path = authority_root.join(name);
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("failed to inspect {}: {error}", path.display())),
    };
    let body_bytes = metadata
        .len()
        .checked_sub(FILE_HEADER_BYTES)
        .ok_or_else(|| {
            format!(
                "Binary DB content closure file {} is shorter than its header.",
                path.display()
            )
        })?;
    let record_size = u64::from(record_size);
    if body_bytes % record_size != 0 {
        return Err(format!(
            "Binary DB content closure file {} is not aligned to {record_size}-byte records.",
            path.display()
        ));
    }
    u32::try_from(body_bytes / record_size).map_err(|_| {
        format!(
            "Binary DB content closure record count overflows u32: {}",
            path.display()
        )
    })
}

fn payload_bytes(authority_root: &Path, name: &str) -> GenerationResult<u64> {
    let path = authority_root.join(name);
    match fs::metadata(&path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

fn validate_records(
    authority_root: &Path,
    name: &str,
    record_size: u32,
    count: u32,
    mut validate: impl FnMut(u32, &[u8]) -> GenerationResult<()>,
) -> GenerationResult<()> {
    if count == 0 {
        return Ok(());
    }
    let path = authority_root.join(name);
    let mut file = fs::File::open(&path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    file.seek(SeekFrom::Start(FILE_HEADER_BYTES))
        .map_err(|error| format!("failed to seek {}: {error}", path.display()))?;
    let mut raw = vec![0_u8; record_size as usize];
    for index in 0..count {
        file.read_exact(&mut raw).map_err(|error| {
            format!(
                "failed to read Binary DB content closure record {index} from {}: {error}",
                path.display()
            )
        })?;
        validate(index, &raw)?;
    }
    Ok(())
}

fn require_index(
    owner_label: &str,
    owner_index: u32,
    target_label: &str,
    target_index: u32,
    target_count: u32,
) -> GenerationResult<()> {
    if target_index < target_count {
        return Ok(());
    }
    Err(format!(
        "Binary DB content closure {owner_label} {owner_index} references {target_label} index {target_index}, but only {target_count} records exist."
    ))
}

fn checked_range_end(
    owner_label: &str,
    owner_index: u32,
    first: u32,
    count: u32,
) -> GenerationResult<u32> {
    first.checked_add(count).ok_or_else(|| {
        format!("Binary DB content closure {owner_label} {owner_index} range overflows u32.")
    })
}

fn require_dense_range(
    owner_label: &str,
    owner_index: u32,
    target_file: &str,
    first: u32,
    count: u32,
    target_count: u32,
    expected_start: &mut u32,
) -> GenerationResult<()> {
    if first != *expected_start {
        return Err(format!(
            "Binary DB content closure {owner_label} {owner_index} starts {target_file} at {first}, expected dense offset {expected_start}."
        ));
    }
    let end = checked_range_end(owner_label, owner_index, first, count)?;
    if end > target_count {
        return Err(format!(
            "Binary DB content closure {owner_label} {owner_index} requires {target_file} records [{first}..{end}), but the file contains {target_count} records."
        ));
    }
    *expected_start = end;
    Ok(())
}

fn require_dense_coverage(
    owner_label: &str,
    target_file: &str,
    covered: u32,
    target_count: u32,
) -> GenerationResult<()> {
    if covered == target_count {
        return Ok(());
    }
    Err(format!(
        "Binary DB content closure {owner_label} ranges cover {covered} {target_file} records, but the file contains {target_count}."
    ))
}

fn require_payload_range(
    owner_label: &str,
    owner_index: u32,
    target_file: &str,
    offset: u64,
    length: u64,
    target_bytes: u64,
) -> GenerationResult<()> {
    let end = offset.checked_add(length).ok_or_else(|| {
        format!(
            "Binary DB content closure {owner_label} {owner_index} payload range overflows u64."
        )
    })?;
    if offset >= FILE_HEADER_BYTES && end <= target_bytes {
        return Ok(());
    }
    Err(format!(
        "Binary DB content closure {owner_label} {owner_index} requires {target_file} bytes [{offset}..{end}), but the file contains {target_bytes} bytes."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_binary_db::{
        BinaryBlobRecord, BinaryObjectPackMemberRecord, BinaryObjectPackRecord,
        BinaryTreePackRecord, BinaryTreeRecord,
    };
    use std::io::Write;
    use tempfile::TempDir;

    fn write_record_file(path: &Path, records: &[Vec<u8>]) {
        let mut file = fs::File::create(path).unwrap();
        file.write_all(&1_u32.to_le_bytes()).unwrap();
        for record in records {
            file.write_all(record).unwrap();
        }
        file.sync_all().unwrap();
    }

    fn snapshot_fixture_record(hash: u64, primary_parent: Option<u32>) -> BinarySnapshotRecord {
        BinarySnapshotRecord {
            snapshot_meta: 0,
            history_flags: 0,
            payload_len: 0,
            payload_offset: 0,
            snapshot_hash48: hash,
            parent_snapshot_index_plus1: primary_parent.map_or(0, |index| index + 1),
            root_tree_pack_index_plus1: 0,
            root_entry_ordinal: 0,
            line_index_plus1: 0,
            manifest_hash: [0; 32],
            file_count: 0,
            total_bytes: 0,
            created_at_s: hash,
        }
    }

    #[test]
    fn compact_tree_locator_must_match_dense_pack_ordinal() {
        let temp = TempDir::new().unwrap();
        let authority = temp.path().join("binary-db");
        fs::create_dir_all(&authority).unwrap();
        let pack = BinaryTreePackRecord {
            pack_meta: BinaryTreePackRecord::META_READY,
            pack_format_kind: 1,
            pack_hash_hi16: 0,
            pack_hash_lo32: 0,
            first_tree_index: 0,
            tree_count: 1,
            total_bytes: 1,
            created_at_s: 1,
        };
        let tree = BinaryTreeRecord {
            tree_meta: 0,
            reserved0: 0,
            pack_entry_ordinal: 7,
            entry_count: 0,
            tree_hash80: [1_u8; 10],
        };
        write_record_file(
            &authority.join(TREE_PACK_BIN),
            &[BinaryTreePackCodec::<1>::encode_record(&pack).unwrap()],
        );
        write_record_file(
            &authority.join(TREE_BIN),
            &[BinaryTreeCodec::<1>::encode_record(&tree).unwrap()],
        );

        let error = validate_repository_content_closure(&authority, temp.path()).unwrap_err();
        assert!(error.contains("compact pack ordinal 7, expected 0"));
    }

    #[test]
    fn snapshot_parent_graph_allows_reordered_indices_and_rejects_cycles() {
        validate_snapshot_parent_graph(&[vec![1], Vec::new()]).unwrap();
        let error = validate_snapshot_parent_graph(&[vec![1], vec![0]]).unwrap_err();
        assert!(error.contains("parent graph contains a cycle"));
    }

    #[test]
    fn canonical_snapshot_parent_closure_accepts_ordered_merge_payload_and_rejects_bad_graphs() {
        let valid = vec![
            snapshot_fixture_record(1, None),
            snapshot_fixture_record(2, Some(0)),
            snapshot_fixture_record(3, Some(0)),
            snapshot_fixture_record(4, Some(1)),
        ];
        let additional = vec![Vec::new(), Vec::new(), Vec::new(), vec![2]];
        let mut valid = valid;
        valid[3].snapshot_meta |= BinarySnapshotRecord::META_HAS_ADDITIONAL_PARENTS;
        validate_snapshot_parent_authority(&valid, &additional)
            .expect("ordered merge parent payload is canonical");

        let mut missing = valid.clone();
        missing[3].parent_snapshot_index_plus1 = 100;
        let error = validate_snapshot_parent_authority(&missing, &additional).unwrap_err();
        assert!(error.contains("missing parent snapshot 99"));

        let mut self_parent = valid.clone();
        self_parent[1].parent_snapshot_index_plus1 = 2;
        let error = validate_snapshot_parent_authority(&self_parent, &additional).unwrap_err();
        assert!(error.contains("points at itself"));

        let mut forward = valid.clone();
        forward[0].parent_snapshot_index_plus1 = 3;
        let error = validate_snapshot_parent_authority(&forward, &additional).unwrap_err();
        assert!(error.contains("must reference an earlier record"));

        let duplicate = vec![Vec::new(), Vec::new(), Vec::new(), vec![1]];
        let error = validate_snapshot_parent_authority(&valid, &duplicate).unwrap_err();
        assert!(error.contains("duplicate parent snapshot 1"));

        let mut boundary = valid;
        boundary[3].history_flags = BinarySnapshotRecord::FLAG_REMOTE_HEAD_HISTORY_BOUNDARY;
        let error = validate_snapshot_parent_authority(&boundary, &additional).unwrap_err();
        assert!(error.contains("remote-head history boundary"));
    }

    #[test]
    fn canonical_generation_rejects_tombstoned_member_in_active_object_pack() {
        let live_pack = BinaryObjectPackRecord {
            pack_meta: BinaryObjectPackRecord::META_READY,
            pack_format_kind: 1,
            pack_hash_hi16: 0,
            pack_hash_lo32: 1,
            first_member_index: 0,
            member_count: 2,
            total_bytes: 1,
            created_at_s: 1,
        };
        let live_member = BinaryObjectPackMemberRecord {
            member_meta: 0,
            delta_chain_depth: 0,
            reserved0: 0,
            pack_index: 0,
            blob_index: 0,
            base_blob_index_plus1: 0,
        };
        let mut tombstoned_member = live_member.clone();
        tombstoned_member.member_meta |= BinaryObjectPackMemberRecord::META_TOMBSTONE;
        let live_blob = BinaryBlobRecord {
            blob_meta: BinaryBlobRecord::META_HAS_PACK_MEMBER,
            hash_kind: 1,
            reserved0: 0,
            size_bytes: 1,
            pack_member_index_plus1: 1,
            created_at_s: 1,
            pruned_at_s: 0,
            sha256: [0x22; 32],
        };

        let error = validate_object_member_backreferences(
            &[live_pack],
            &[live_member, tombstoned_member],
            &[live_blob],
            &[Some(0)],
        )
        .unwrap_err();
        assert!(error.contains("active object pack 0 contains tombstoned member 1"));
    }

    #[test]
    fn overlapping_object_packs_may_share_a_blob_with_one_selected_member() {
        let first_pack = BinaryObjectPackRecord {
            pack_meta: BinaryObjectPackRecord::META_READY,
            pack_format_kind: 1,
            pack_hash_hi16: 0,
            pack_hash_lo32: 1,
            first_member_index: 0,
            member_count: 1,
            total_bytes: 1,
            created_at_s: 1,
        };
        let second_pack = BinaryObjectPackRecord {
            pack_hash_lo32: 2,
            first_member_index: 1,
            ..first_pack.clone()
        };
        let selected_member = BinaryObjectPackMemberRecord {
            member_meta: 0,
            delta_chain_depth: 0,
            reserved0: 0,
            pack_index: 0,
            blob_index: 0,
            base_blob_index_plus1: 0,
        };
        let duplicate_member = BinaryObjectPackMemberRecord {
            pack_index: 1,
            ..selected_member.clone()
        };
        let blob = BinaryBlobRecord {
            blob_meta: BinaryBlobRecord::META_HAS_PACK_MEMBER,
            hash_kind: 1,
            reserved0: 0,
            size_bytes: 1,
            pack_member_index_plus1: 1,
            created_at_s: 1,
            pruned_at_s: 0,
            sha256: [0x23; 32],
        };

        validate_object_member_backreferences(
            &[first_pack, second_pack],
            &[selected_member, duplicate_member],
            &[blob],
            &[Some(0)],
        )
        .expect("overlapping immutable packs may retain a non-selected physical copy");
    }

    #[test]
    fn live_object_pack_member_requires_its_blob_to_select_a_member() {
        let pack = BinaryObjectPackRecord {
            pack_meta: BinaryObjectPackRecord::META_READY,
            pack_format_kind: 1,
            pack_hash_hi16: 0,
            pack_hash_lo32: 1,
            first_member_index: 0,
            member_count: 1,
            total_bytes: 1,
            created_at_s: 1,
        };
        let member = BinaryObjectPackMemberRecord {
            member_meta: 0,
            delta_chain_depth: 0,
            reserved0: 0,
            pack_index: 0,
            blob_index: 0,
            base_blob_index_plus1: 0,
        };
        let blob = BinaryBlobRecord {
            blob_meta: 0,
            hash_kind: 1,
            reserved0: 0,
            size_bytes: 1,
            pack_member_index_plus1: 0,
            created_at_s: 1,
            pruned_at_s: 0,
            sha256: [0x24; 32],
        };

        let error = validate_object_member_backreferences(&[pack], &[member], &[blob], &[None])
            .unwrap_err();
        assert!(error.contains("without a selected object pack member"));
    }

    #[test]
    fn live_object_pack_member_rejects_tombstoned_base_blob_pointer() {
        let pack = BinaryObjectPackRecord {
            pack_meta: BinaryObjectPackRecord::META_READY,
            pack_format_kind: 1,
            pack_hash_hi16: 0,
            pack_hash_lo32: 1,
            first_member_index: 0,
            member_count: 1,
            total_bytes: 1,
            created_at_s: 1,
        };
        let member = BinaryObjectPackMemberRecord {
            member_meta: 1,
            delta_chain_depth: 1,
            reserved0: 0,
            pack_index: 0,
            blob_index: 0,
            base_blob_index_plus1: 2,
        };
        let blob = BinaryBlobRecord {
            blob_meta: BinaryBlobRecord::META_HAS_PACK_MEMBER,
            hash_kind: 1,
            reserved0: 0,
            size_bytes: 1,
            pack_member_index_plus1: 1,
            created_at_s: 1,
            pruned_at_s: 0,
            sha256: [0x33; 32],
        };
        let mut tombstoned_base = blob.clone();
        tombstoned_base.blob_meta = BinaryBlobRecord::META_TOMBSTONE;
        tombstoned_base.pack_member_index_plus1 = 0;
        tombstoned_base.sha256 = [0x44; 32];

        let error = validate_object_member_backreferences(
            &[pack],
            &[member],
            &[blob, tombstoned_base],
            &[Some(0), None],
        )
        .unwrap_err();
        assert!(error.contains("references unavailable base blob 1"));
    }

    #[test]
    fn canonical_generation_rejects_blob_tombstones_but_allows_live_duplicate_candidates() {
        let temp = TempDir::new().unwrap();
        let authority = temp.path().join("binary-db");
        fs::create_dir_all(&authority).unwrap();
        let original = BinaryBlobRecord {
            blob_meta: 0,
            hash_kind: 1,
            reserved0: 0,
            size_bytes: 1,
            pack_member_index_plus1: 0,
            created_at_s: 1,
            pruned_at_s: 0,
            sha256: [0x44; 32],
        };
        let mut duplicate = original.clone();
        duplicate.blob_meta |= BinaryBlobRecord::META_TOMBSTONE;
        write_record_file(
            &authority.join(BLOB_BIN),
            &[
                BinaryBlobCodec::<1>::encode_record(&original).unwrap(),
                BinaryBlobCodec::<1>::encode_record(&duplicate).unwrap(),
            ],
        );
        let error = validate_repository_content_closure(&authority, temp.path()).unwrap_err();
        assert!(error.contains("canonical generation contains unavailable blob 1"));

        duplicate.blob_meta &= !BinaryBlobRecord::META_TOMBSTONE;
        write_record_file(
            &authority.join(BLOB_BIN),
            &[
                BinaryBlobCodec::<1>::encode_record(&original).unwrap(),
                BinaryBlobCodec::<1>::encode_record(&duplicate).unwrap(),
            ],
        );
        super::super::generation_content_indexes::rebuild_content_identity_indexes(&authority)
            .unwrap();
        validate_repository_content_closure(&authority, temp.path()).unwrap();
    }
}
