use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, BTreeSet};

use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbFsyncPolicy, BinaryDbIndexAppender,
    BinaryDbWriteTxn, StoreResult,
};
use crate::content_binary_db::{
    blob_id_from_sha256, blob_id_index_key, object_pack_hash48_from_id, object_pack_id_from_hash48,
    object_pack_id_index_key, object_pack_relative_path, snapshot_hash48_from_id,
    snapshot_id_from_hash48, snapshot_id_index_key, tree_id_from_hash80, tree_id_index_key,
    tree_pack_hash48_from_id, tree_pack_id_from_hash48, tree_pack_id_index_key,
    tree_pack_relative_path, BinaryBlobCodec, BinaryBlobRecord, BinaryDbBlobStore,
    BinaryDbObjectPackStore, BinaryDbSnapshotStore, BinaryDbTreePackStore, BinaryDbTreeStore,
    BinaryObjectPackCodec, BinaryObjectPackCompressionKind, BinaryObjectPackMemberCodec,
    BinaryObjectPackMemberKind, BinaryObjectPackMemberRecord, BinaryObjectPackRecord,
    BinarySnapshotCodec, BinarySnapshotPayload, BinarySnapshotRecord, BinaryTreeCodec,
    BinaryTreePackCodec, BinaryTreePackRecord, BinaryTreeRecord,
};
use crate::line_binary_db::binary_line_index_by_name_in_write;
use crate::pack_substrate::{PackFormatKind, TreePackFormatKind};
use crate::snapshot_store::{compatibility_parent_projections, validate_snapshot_parent_set};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryDbObjectPackMemberWriteInput {
    pub blob_id: String,
    pub sha256: String,
    pub size_bytes: i64,
    pub pack_entry_type: String,
    pub pack_base_blob_id: Option<String>,
    pub pack_chain_depth: i64,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryDbObjectPackWriteInput {
    pub pack_id: String,
    pub pack_rel_path: String,
    pub pack_format: String,
    pub member_count: i64,
    pub total_bytes: i64,
    pub created_at: String,
    pub members: Vec<BinaryDbObjectPackMemberWriteInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryDbTreePackTreeWriteInput {
    pub tree_id: String,
    pub entry_count: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryDbTreeEntryWriteInput {
    pub tree_id: String,
    pub entry_name: String,
    pub entry_type: String,
    pub target_id: String,
    pub mode: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryDbTreePackWriteInput {
    pub pack_id: String,
    pub pack_rel_path: String,
    pub pack_format: String,
    pub tree_count: i64,
    pub total_bytes: i64,
    pub created_at: String,
    pub trees: Vec<BinaryDbTreePackTreeWriteInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryDbSnapshotWriteInput {
    pub snapshot_id: String,
    pub parent_snapshot_ids: Vec<String>,
    pub root_tree_pack_id: String,
    pub root_entry_ordinal: i64,
    pub manifest_hash: String,
    pub message: Option<String>,
    pub line_name: String,
    pub snapshot_kind: String,
    pub file_count: i64,
    pub total_bytes: i64,
    pub created_at: String,
}

pub struct BinaryDbContentWriteCoordinator<'a, B, const WRITE_LAYOUT: u32>
where
    B: BinaryDb,
{
    blobs: &'a BinaryDbBlobStore<B, WRITE_LAYOUT>,
    object_packs: &'a BinaryDbObjectPackStore<B, WRITE_LAYOUT>,
    tree_packs: &'a BinaryDbTreePackStore<B, WRITE_LAYOUT>,
    trees: &'a BinaryDbTreeStore<B, WRITE_LAYOUT>,
    snapshots: &'a BinaryDbSnapshotStore<B, WRITE_LAYOUT>,
}

impl<'a, B, const WRITE_LAYOUT: u32> BinaryDbContentWriteCoordinator<'a, B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn new(
        blobs: &'a BinaryDbBlobStore<B, WRITE_LAYOUT>,
        object_packs: &'a BinaryDbObjectPackStore<B, WRITE_LAYOUT>,
        tree_packs: &'a BinaryDbTreePackStore<B, WRITE_LAYOUT>,
        trees: &'a BinaryDbTreeStore<B, WRITE_LAYOUT>,
        snapshots: &'a BinaryDbSnapshotStore<B, WRITE_LAYOUT>,
    ) -> Self {
        Self {
            blobs,
            object_packs,
            tree_packs,
            trees,
            snapshots,
        }
    }
}

impl<'a, B, const WRITE_LAYOUT: u32> BinaryDbContentWriteCoordinator<'a, B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender,
{
    pub fn record_object_pack_metadata(
        &self,
        scope: BinaryDbCommandScope,
        input: &BinaryDbObjectPackWriteInput,
    ) -> StoreResult<()> {
        self.record_object_pack_metadata_batch(scope, std::slice::from_ref(input))
    }

    pub fn record_object_pack_metadata_batch(
        &self,
        scope: BinaryDbCommandScope,
        inputs: &[BinaryDbObjectPackWriteInput],
    ) -> StoreResult<()> {
        ensure_content_write_scope(scope)?;
        let mut tx = self.blobs.begin_write_txn(scope)?;
        let mut wrote = false;
        for input in inputs {
            wrote |= self.record_object_pack_metadata_in_write(&mut tx, input)?;
        }
        if wrote {
            tx.commit().map(|_| ())
        } else {
            tx.abort()
        }
    }

    fn record_object_pack_metadata_in_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, B, F>,
        input: &BinaryDbObjectPackWriteInput,
    ) -> StoreResult<bool>
    where
        F: BinaryDbFsyncPolicy,
    {
        validate_non_empty(&input.pack_id, "pack_id")?;
        if input.member_count
            != i64::try_from(input.members.len())
                .map_err(|_| "object pack member count overflows i64".to_string())?
        {
            return Err(format!(
                "Binary DB object pack {} member_count {} does not match {} members.",
                input.pack_id,
                input.member_count,
                input.members.len()
            )
            .into());
        }
        let pack_format_kind = PackFormatKind::from_persisted(&input.pack_format)?;
        let canonical_pack_format = pack_format_kind.persisted_name();
        let expected_path = object_pack_relative_path(&input.pack_id, canonical_pack_format)?;
        if input.pack_rel_path != expected_path {
            return Err(format!(
                "Binary DB object pack {} path mismatch: expected {}, got {}",
                input.pack_id, expected_path, input.pack_rel_path
            )
            .into());
        }
        let pack_hash = object_pack_hash48_from_id(&input.pack_id)?;
        let (pack_hash_hi16, pack_hash_lo32) = split_hash48(pack_hash);
        let created_at_s = parse_created_at_s(&input.created_at)?;
        let member_count = u32_from_i64(input.member_count, "member_count")?;
        let total_bytes = u64_from_i64(input.total_bytes, "total_bytes")?;
        let mut current_blob_ids = std::collections::BTreeSet::new();
        let mut member_sha256 = Vec::with_capacity(input.members.len());
        for member in &input.members {
            validate_non_empty(&member.blob_id, "blob_id")?;
            let sha256 = parse_hex_array::<32>(&member.sha256, "sha256")?;
            let expected_blob_id = blob_id_from_sha256(&sha256);
            if !expected_blob_id.eq_ignore_ascii_case(&member.blob_id) {
                return Err(format!(
                    "Binary DB blob id {} does not match sha256-derived id {}.",
                    member.blob_id, expected_blob_id
                )
                .into());
            }
            u64_from_i64(member.size_bytes, "size_bytes")?;
            u8_from_i64(member.pack_chain_depth, "pack_chain_depth")?;
            parse_created_at_s(&member.created_at)?;
            if !current_blob_ids.insert(member.blob_id.to_ascii_lowercase()) {
                return Err(format!(
                    "Binary DB object pack {} has duplicate blob member {}.",
                    input.pack_id, member.blob_id
                )
                .into());
            }
            member_sha256.push(sha256);
        }

        if object_pack_index_in_write::<B, _, WRITE_LAYOUT>(tx, &input.pack_id)?.is_some() {
            return Ok(false);
        }
        let first_member_index =
            tx.record_count(BinaryDbObjectPackStore::<B, WRITE_LAYOUT>::object_pack_member_file())?;
        let mut external_base_blob_indices = std::collections::BTreeMap::new();
        for member in &input.members {
            let Some(base_blob_id) = member
                .pack_base_blob_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let normalized_base_blob_id = base_blob_id.to_ascii_lowercase();
            if current_blob_ids.contains(&normalized_base_blob_id)
                || external_base_blob_indices.contains_key(&normalized_base_blob_id)
            {
                continue;
            }
            let base_index = blob_index_in_write::<B, _, WRITE_LAYOUT>(tx, base_blob_id)?
                .ok_or_else(|| {
                    format!(
                        "Binary DB object pack {} base blob {} is missing.",
                        input.pack_id, base_blob_id
                    )
                })?;
            external_base_blob_indices.insert(normalized_base_blob_id, base_index);
        }

        let expected_pack_index =
            tx.record_count(BinaryDbObjectPackStore::<B, WRITE_LAYOUT>::object_pack_file())?;
        let (pack_index, _) = self.object_packs.append_object_pack_with_id_index(
            tx,
            &BinaryObjectPackRecord {
                pack_meta: BinaryObjectPackRecord::META_READY,
                pack_format_kind: object_pack_format_kind_byte(pack_format_kind),
                pack_hash_hi16,
                pack_hash_lo32,
                first_member_index,
                member_count,
                total_bytes,
                created_at_s,
            },
        )?;
        ensure_dense_index("object pack", expected_pack_index, pack_index)?;

        let mut blob_indices = std::collections::BTreeMap::new();
        for (offset, member) in input.members.iter().enumerate() {
            let offset = u32::try_from(offset)
                .map_err(|_| format!("object pack member offset overflows u32: {offset}"))?;
            let member_index = first_member_index
                .checked_add(offset)
                .ok_or_else(|| "object pack member index overflow".to_string())?;
            let size_bytes = u64_from_i64(member.size_bytes, "size_bytes")?;
            let sha256 = member_sha256[offset as usize];
            let blob_index = match blob_index_in_write::<B, _, WRITE_LAYOUT>(tx, &member.blob_id)? {
                Some(existing_index) => {
                    let raw = tx.read_record(
                        BinaryBlobCodec::<WRITE_LAYOUT>::record_file(),
                        existing_index,
                    )?;
                    let existing = BinaryBlobCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
                    if existing.blob_meta != BinaryBlobRecord::META_HAS_PACK_MEMBER
                        || existing.hash_kind != 0
                        || existing.reserved0 != 0
                        || existing.size_bytes != size_bytes
                        || existing.pruned_at_s != 0
                        || existing.sha256 != sha256
                    {
                        return Err(BinaryDbError::corruption(format!(
                            "Existing Binary DB blob {} does not match repeated object-pack member metadata.",
                            member.blob_id
                        )));
                    }
                    let selected_member_index = existing.pack_member_index().ok_or_else(|| {
                        BinaryDbError::corruption(format!(
                            "Existing Binary DB blob {} has no selected object-pack member.",
                            member.blob_id
                        ))
                    })?;
                    let selected_raw = tx.read_record(
                        BinaryObjectPackMemberCodec::<WRITE_LAYOUT>::record_file(),
                        selected_member_index,
                    )?;
                    let selected =
                        BinaryObjectPackMemberCodec::<WRITE_LAYOUT>::decode_record(&selected_raw)?;
                    if selected.is_tombstone() || selected.blob_index != existing_index {
                        return Err(BinaryDbError::corruption(format!(
                            "Existing Binary DB blob {} selected object-pack member is inconsistent.",
                            member.blob_id
                        )));
                    }
                    existing_index
                }
                None => {
                    let expected_blob_index =
                        tx.record_count(BinaryDbBlobStore::<B, WRITE_LAYOUT>::blob_file())?;
                    let (blob_index, _) = self.blobs.append_blob_with_id_index(
                        tx,
                        &BinaryBlobRecord {
                            blob_meta: BinaryBlobRecord::META_HAS_PACK_MEMBER,
                            hash_kind: 0,
                            reserved0: 0,
                            size_bytes,
                            pack_member_index_plus1: member_index
                                .checked_add(1)
                                .ok_or_else(|| "pack member index overflow".to_string())?,
                            created_at_s: parse_created_at_s(&member.created_at)?,
                            pruned_at_s: 0,
                            sha256,
                        },
                    )?;
                    ensure_dense_index("blob", expected_blob_index, blob_index)?;
                    blob_index
                }
            };
            blob_indices.insert(member.blob_id.to_ascii_lowercase(), blob_index);
        }

        for (offset, member) in input.members.iter().enumerate() {
            let normalized_blob_id = member.blob_id.to_ascii_lowercase();
            let blob_index = blob_indices
                .get(&normalized_blob_id)
                .copied()
                .ok_or_else(|| format!("Missing Binary DB blob index for {}", member.blob_id))?;
            let base_blob_index_plus1 = match member.pack_base_blob_id.as_deref() {
                Some(base_blob_id) if !base_blob_id.trim().is_empty() => {
                    let normalized_base_blob_id = base_blob_id.to_ascii_lowercase();
                    let base_index = match blob_indices.get(&normalized_base_blob_id).copied() {
                        Some(index) => index,
                        None => *external_base_blob_indices
                            .get(&normalized_base_blob_id)
                            .ok_or_else(|| {
                                format!(
                                    "Missing Binary DB locked base blob index for {}",
                                    base_blob_id
                                )
                            })?,
                    };
                    base_index
                        .checked_add(1)
                        .ok_or_else(|| "base blob index overflow".to_string())?
                }
                _ => 0,
            };
            let expected_member_index =
                first_member_index
                    .checked_add(u32::try_from(offset).map_err(|_| {
                        format!("object pack member offset overflows u32: {offset}")
                    })?)
                    .ok_or_else(|| "object pack member index overflow".to_string())?;
            let member_index = self.object_packs.append_object_pack_member_record(
                tx,
                &BinaryObjectPackMemberRecord {
                    member_meta: object_pack_member_meta(&member.pack_entry_type),
                    delta_chain_depth: u8_from_i64(member.pack_chain_depth, "pack_chain_depth")?,
                    reserved0: 0,
                    pack_index,
                    blob_index,
                    base_blob_index_plus1,
                },
            )?;
            ensure_dense_index("object pack member", expected_member_index, member_index)?;
        }
        Ok(true)
    }

    pub fn record_tree_pack_metadata(
        &self,
        scope: BinaryDbCommandScope,
        input: &BinaryDbTreePackWriteInput,
    ) -> StoreResult<()> {
        self.record_tree_pack_metadata_with_ordinals(scope, input, None)
    }

    pub(crate) fn record_tree_pack_metadata_with_ordinals(
        &self,
        scope: BinaryDbCommandScope,
        input: &BinaryDbTreePackWriteInput,
        pack_entry_ordinals: Option<&BTreeMap<String, u32>>,
    ) -> StoreResult<()> {
        ensure_content_write_scope(scope)?;
        let mut tx = self.trees.begin_write_txn(scope)?;
        let wrote = self.record_tree_pack_metadata_with_ordinals_in_write(
            &mut tx,
            input,
            pack_entry_ordinals,
        )?;
        if wrote {
            tx.commit().map(|_| ())
        } else {
            tx.abort()
        }
    }

    fn record_tree_pack_metadata_with_ordinals_in_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, B, F>,
        input: &BinaryDbTreePackWriteInput,
        pack_entry_ordinals: Option<&BTreeMap<String, u32>>,
    ) -> StoreResult<bool>
    where
        F: BinaryDbFsyncPolicy,
    {
        validate_non_empty(&input.pack_id, "pack_id")?;
        if input.tree_count
            != i64::try_from(input.trees.len())
                .map_err(|_| "tree pack tree count overflows i64".to_string())?
        {
            return Err(format!(
                "Binary DB tree pack {} tree_count {} does not match {} tree locators.",
                input.pack_id,
                input.tree_count,
                input.trees.len()
            )
            .into());
        }
        let mut ordered_trees = input.trees.iter().collect::<Vec<_>>();
        ordered_trees.sort_by(|left, right| left.tree_id.cmp(&right.tree_id));
        let mut input_tree_ids = std::collections::BTreeSet::new();
        for tree in &ordered_trees {
            tree_id_index_key(&tree.tree_id)?;
            u32_from_i64(tree.entry_count, "entry_count")?;
            if !input_tree_ids.insert(tree.tree_id.to_ascii_lowercase()) {
                return Err(format!(
                    "Binary DB tree pack {} has duplicate tree {}.",
                    input.pack_id, tree.tree_id
                )
                .into());
            }
        }
        let pack_format_kind = TreePackFormatKind::from_persisted(&input.pack_format)?;
        let canonical_pack_format = pack_format_kind.persisted_name();
        let expected_path = tree_pack_relative_path(&input.pack_id, canonical_pack_format)?;
        if input.pack_rel_path != expected_path {
            return Err(format!(
                "Binary DB tree pack {} path mismatch: expected {}, got {}",
                input.pack_id, expected_path, input.pack_rel_path
            )
            .into());
        }
        let pack_hash = tree_pack_hash48_from_id(&input.pack_id)?;
        let (pack_hash_hi16, pack_hash_lo32) = split_hash48(pack_hash);
        let tree_count = u32_from_i64(input.tree_count, "tree_count")?;
        let total_bytes = u64_from_i64(input.total_bytes, "total_bytes")?;
        let created_at_s = parse_created_at_s(&input.created_at)?;

        if let Some(existing_pack_index) =
            tree_pack_index_in_write::<B, _, WRITE_LAYOUT>(tx, &input.pack_id)?
        {
            validate_existing_tree_pack_metadata::<B, _, WRITE_LAYOUT>(
                tx,
                existing_pack_index,
                input,
                &ordered_trees,
                tree_pack_format_kind_byte(pack_format_kind),
                tree_count,
                total_bytes,
                created_at_s,
                pack_entry_ordinals,
            )?;
            return Ok(false);
        }
        let first_tree_index = tx.record_count(BinaryDbTreeStore::<B, WRITE_LAYOUT>::tree_file())?;
        for (offset, tree) in ordered_trees.into_iter().enumerate() {
            let default_pack_entry_ordinal = u32::try_from(offset)
                .map_err(|_| format!("Binary DB tree offset overflows u32 for {}", tree.tree_id))?;
            let pack_entry_ordinal = pack_entry_ordinals
                .map(|ordinals| {
                    ordinals
                        .get(&tree.tree_id.to_ascii_lowercase())
                        .copied()
                        .ok_or_else(|| {
                            format!(
                                "Binary DB tree pack {} is missing the physical ordinal for tree {}.",
                                input.pack_id, tree.tree_id
                            )
                        })
                })
                .transpose()?
                .unwrap_or(default_pack_entry_ordinal);
            let expected_tree_index = first_tree_index
                .checked_add(default_pack_entry_ordinal)
                .ok_or_else(|| "Binary DB tree index overflow".to_string())?;
            let tree_record = BinaryTreeRecord {
                tree_meta: 0,
                reserved0: 0,
                pack_entry_ordinal,
                entry_count: u32_from_i64(tree.entry_count, "entry_count")?,
                tree_hash80: tree_id_index_key(&tree.tree_id)?,
            };
            let tree_index = match tree_index_in_write::<B, _, WRITE_LAYOUT>(tx, &tree.tree_id)? {
                Some(existing_index) => {
                    let existing_raw = tx.read_record(
                        BinaryDbTreeStore::<B, WRITE_LAYOUT>::tree_file(),
                        existing_index,
                    )?;
                    let existing = BinaryTreeCodec::<WRITE_LAYOUT>::decode_record(&existing_raw)?;
                    if existing.is_tombstone()
                        || existing.tree_hash80 != tree_record.tree_hash80
                        || existing.entry_count != tree_record.entry_count
                    {
                        return Err(BinaryDbError::corruption(format!(
                            "Existing Binary DB tree {} does not match repeated tree-pack member metadata.",
                            tree.tree_id
                        )));
                    }
                    self.trees.append_tree_record(tx, &tree_record)?
                }
                None => self.trees.append_tree_with_id_index(tx, &tree_record)?.0,
            };
            ensure_dense_index("tree", expected_tree_index, tree_index)?;
        }
        let expected_pack_index =
            tx.record_count(BinaryDbTreePackStore::<B, WRITE_LAYOUT>::tree_pack_file())?;
        let (pack_index, _) = self.tree_packs.append_tree_pack_with_id_index(
            tx,
            &BinaryTreePackRecord {
                pack_meta: BinaryTreePackRecord::META_READY
                    | if pack_entry_ordinals.is_some() {
                        BinaryTreePackRecord::META_SPARSE_PHYSICAL_ORDINALS
                    } else {
                        0
                    },
                pack_format_kind: tree_pack_format_kind_byte(pack_format_kind),
                pack_hash_hi16,
                pack_hash_lo32,
                first_tree_index,
                tree_count,
                total_bytes,
                created_at_s,
            },
        )?;
        ensure_dense_index("tree pack", expected_pack_index, pack_index)?;
        Ok(true)
    }

    pub fn record_tree_pack_metadata_batch_with_entries(
        &self,
        scope: BinaryDbCommandScope,
        inputs: &[(BinaryDbTreePackWriteInput, Vec<BinaryDbTreeEntryWriteInput>)],
    ) -> StoreResult<()> {
        ensure_content_write_scope(scope)?;
        let mut tx = self.trees.begin_write_txn(scope)?;
        let mut wrote = false;
        for (input, entries) in inputs {
            self.validate_complete_tree_pack_entries_in_write(&tx, input, entries)?;
            wrote |= self.record_tree_pack_metadata_with_ordinals_in_write(&mut tx, input, None)?;
        }
        if wrote {
            tx.commit().map(|_| ())
        } else {
            tx.abort()
        }
    }

    fn validate_complete_tree_pack_entries_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, B, F>,
        input: &BinaryDbTreePackWriteInput,
        entries: &[BinaryDbTreeEntryWriteInput],
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let mut entry_counts_by_tree_id = BTreeMap::<String, usize>::new();
        for entry in entries {
            validate_non_empty(&entry.tree_id, "tree_id")?;
            validate_non_empty(&entry.entry_name, "entry_name")?;
            validate_non_empty(&entry.entry_type, "entry_type")?;
            validate_non_empty(&entry.target_id, "target_id")?;
            match tree_entry_kind_byte(&entry.entry_type)? {
                0 => {
                    blob_id_index_key(&entry.target_id)?;
                }
                1 => {
                    tree_id_index_key(&entry.target_id)?;
                }
                _ => unreachable!("tree_entry_kind_byte only returns blob or tree"),
            }
            tree_entry_mode_bits(&entry.entry_type, &entry.mode)?;
            *entry_counts_by_tree_id
                .entry(entry.tree_id.to_ascii_lowercase())
                .or_default() += 1;
        }

        let mut input_tree_ids = BTreeSet::new();
        for tree in &input.trees {
            tree_id_index_key(&tree.tree_id)?;
            let normalized_tree_id = tree.tree_id.to_ascii_lowercase();
            if !input_tree_ids.insert(normalized_tree_id.clone()) {
                return Err(format!(
                    "Binary DB tree pack {} has duplicate tree {}.",
                    input.pack_id, tree.tree_id
                )
                .into());
            }
            let actual_count = entry_counts_by_tree_id
                .get(&normalized_tree_id)
                .copied()
                .unwrap_or_default();
            let expected_count = u32_from_i64(tree.entry_count, "entry_count")?;
            if expected_count
                != u32::try_from(actual_count).map_err(|_| {
                    format!("Binary DB tree {} entry count overflows u32", tree.tree_id)
                })?
            {
                return Err(format!(
                    "Binary DB tree {} entry_count {} does not match {} tree entries.",
                    tree.tree_id, tree.entry_count, actual_count
                )
                .into());
            }
        }
        for tree_id in entry_counts_by_tree_id.keys() {
            if !input_tree_ids.contains(tree_id) {
                return Err(format!(
                    "Binary DB tree pack {} has entries for unknown tree {}.",
                    input.pack_id, tree_id
                )
                .into());
            }
        }

        for entry in entries {
            match tree_entry_kind_byte(&entry.entry_type)? {
                0 if blob_index_in_write::<B, _, WRITE_LAYOUT>(tx, &entry.target_id)?.is_none() => {
                    return Err(format!(
                        "Binary DB blob {} is missing for tree entry {}.",
                        entry.target_id, entry.entry_name
                    )
                    .into());
                }
                1 if !input_tree_ids.contains(&entry.target_id.to_ascii_lowercase())
                    && tree_index_in_write::<B, _, WRITE_LAYOUT>(tx, &entry.target_id)?
                        .is_none() =>
                {
                    return Err(format!(
                        "Binary DB tree {} is missing for tree entry {}.",
                        entry.target_id, entry.entry_name
                    )
                    .into());
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn mark_tree_pack_sparse_physical_ordinals(
        &self,
        scope: BinaryDbCommandScope,
        pack_id: &str,
    ) -> StoreResult<()> {
        ensure_content_write_scope(scope)?;
        validate_non_empty(pack_id, "pack_id")?;
        let mut tx = self.tree_packs.begin_write_txn(scope)?;
        let pack_index = tree_pack_index_in_write::<B, _, WRITE_LAYOUT>(&tx, pack_id)?
            .ok_or_else(|| format!("Binary DB tree pack {pack_id} is missing."))?;
        let record_file = BinaryDbTreePackStore::<B, WRITE_LAYOUT>::tree_pack_file();
        let raw = tx.read_record(record_file.clone(), pack_index)?;
        let mut record = BinaryTreePackCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
        if record.has_sparse_physical_ordinals() {
            tx.abort()?;
            return Ok(());
        }
        record.pack_meta |= BinaryTreePackRecord::META_SPARSE_PHYSICAL_ORDINALS;
        tx.overwrite_record(
            record_file,
            pack_index,
            &BinaryTreePackCodec::<WRITE_LAYOUT>::encode_record(&record)?,
        )?;
        tx.commit().map(|_| ())
    }

    pub fn record_tree_pack_metadata_with_entries(
        &self,
        scope: BinaryDbCommandScope,
        input: &BinaryDbTreePackWriteInput,
        entries: &[BinaryDbTreeEntryWriteInput],
    ) -> StoreResult<()> {
        self.record_tree_pack_metadata_with_entry_scope(scope, input, entries, None, false)
    }

    pub fn record_tree_pack_metadata_with_reachable_entries(
        &self,
        scope: BinaryDbCommandScope,
        input: &BinaryDbTreePackWriteInput,
        entries: &[BinaryDbTreeEntryWriteInput],
        reachable_tree_ids: &BTreeSet<String>,
    ) -> StoreResult<()> {
        self.record_tree_pack_metadata_with_entry_scope(
            scope,
            input,
            entries,
            Some(reachable_tree_ids),
            true,
        )
    }

    fn record_tree_pack_metadata_with_entry_scope(
        &self,
        scope: BinaryDbCommandScope,
        input: &BinaryDbTreePackWriteInput,
        entries: &[BinaryDbTreeEntryWriteInput],
        reachable_tree_ids: Option<&BTreeSet<String>>,
        reuse_existing_trees: bool,
    ) -> StoreResult<()> {
        ensure_content_write_scope(scope)?;
        validate_non_empty(&input.pack_id, "pack_id")?;
        if input.tree_count
            != i64::try_from(input.trees.len())
                .map_err(|_| "tree pack tree count overflows i64".to_string())?
        {
            return Err(format!(
                "Binary DB tree pack {} tree_count {} does not match {} tree locators.",
                input.pack_id,
                input.tree_count,
                input.trees.len()
            )
            .into());
        }

        let pack_format_kind = TreePackFormatKind::from_persisted(&input.pack_format)?;
        let canonical_pack_format = pack_format_kind.persisted_name();
        let expected_path = tree_pack_relative_path(&input.pack_id, canonical_pack_format)?;
        if input.pack_rel_path != expected_path {
            return Err(format!(
                "Binary DB tree pack {} path mismatch: expected {}, got {}",
                input.pack_id, expected_path, input.pack_rel_path
            )
            .into());
        }

        let mut entries_by_tree_id = std::collections::BTreeMap::<String, Vec<_>>::new();
        for entry in entries {
            validate_non_empty(&entry.tree_id, "tree_id")?;
            validate_non_empty(&entry.entry_name, "entry_name")?;
            validate_non_empty(&entry.entry_type, "entry_type")?;
            validate_non_empty(&entry.target_id, "target_id")?;
            match tree_entry_kind_byte(&entry.entry_type)? {
                0 => {
                    blob_id_index_key(&entry.target_id)?;
                }
                1 => {
                    tree_id_index_key(&entry.target_id)?;
                }
                _ => unreachable!("tree_entry_kind_byte only returns blob or tree"),
            }
            tree_entry_mode_bits(&entry.entry_type, &entry.mode)?;
            entries_by_tree_id
                .entry(entry.tree_id.to_ascii_lowercase())
                .or_default()
                .push(entry);
        }
        for entries in entries_by_tree_id.values_mut() {
            entries.sort_by(|left, right| left.entry_name.cmp(&right.entry_name));
        }

        let mut input_tree_ids = std::collections::BTreeSet::new();
        for tree in &input.trees {
            tree_id_index_key(&tree.tree_id)?;
            let normalized_tree_id = tree.tree_id.to_ascii_lowercase();
            if !input_tree_ids.insert(normalized_tree_id.clone()) {
                return Err(format!(
                    "Binary DB tree pack {} has duplicate tree {}.",
                    input.pack_id, tree.tree_id
                )
                .into());
            }
            let tree_entries = entries_by_tree_id
                .get(&normalized_tree_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let expected_count = u32_from_i64(tree.entry_count, "entry_count")?;
            if expected_count
                != u32::try_from(tree_entries.len()).map_err(|_| {
                    format!("Binary DB tree {} entry count overflows u32", tree.tree_id)
                })?
            {
                return Err(format!(
                    "Binary DB tree {} entry_count {} does not match {} tree entries.",
                    tree.tree_id,
                    tree.entry_count,
                    tree_entries.len()
                )
                .into());
            }
        }
        for tree_id in entries_by_tree_id.keys() {
            if !input_tree_ids.contains(tree_id) {
                return Err(format!(
                    "Binary DB tree pack {} has entries for unknown tree {}.",
                    input.pack_id, tree_id
                )
                .into());
            }
        }

        let read = self.trees.begin_read_txn();
        for entry in entries {
            if reachable_tree_ids
                .is_some_and(|required| !required.contains(&entry.tree_id.to_ascii_lowercase()))
            {
                continue;
            }
            match tree_entry_kind_byte(&entry.entry_type)? {
                0 if self.blobs.get_blob_view(&read, &entry.target_id)?.is_none() => {
                    return Err(format!(
                        "Binary DB blob {} is missing for tree entry {}.",
                        entry.target_id, entry.entry_name
                    )
                    .into())
                }
                1 if !input_tree_ids.contains(&entry.target_id.to_ascii_lowercase())
                    && self.trees.get_tree_view(&read, &entry.target_id)?.is_none() =>
                {
                    return Err(format!(
                        "Binary DB tree {} is missing for tree entry {}.",
                        entry.target_id, entry.entry_name
                    )
                    .into())
                }
                _ => {}
            }
        }
        let existing_tree_ids = if reuse_existing_trees {
            input
                .trees
                .iter()
                .map(|tree| {
                    self.trees
                        .get_tree_view(&read, &tree.tree_id)
                        .map(|existing| existing.map(|_| tree.tree_id.to_ascii_lowercase()))
                })
                .collect::<StoreResult<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<BTreeSet<_>>()
        } else {
            BTreeSet::new()
        };
        drop(read);
        if !reuse_existing_trees || existing_tree_ids.is_empty() {
            return self.record_tree_pack_metadata(scope, input);
        }

        let mut retained_input = input.clone();
        let mut pack_entry_ordinals = BTreeMap::new();
        retained_input.trees = input
            .trees
            .iter()
            .enumerate()
            .filter(|(_, tree)| !existing_tree_ids.contains(&tree.tree_id.to_ascii_lowercase()))
            .map(|(ordinal, tree)| {
                let normalized_tree_id = tree.tree_id.to_ascii_lowercase();
                pack_entry_ordinals.insert(
                    normalized_tree_id,
                    u32::try_from(ordinal)
                        .map_err(|_| "tree pack physical ordinal overflows u32".to_string())?,
                );
                Ok(tree.clone())
            })
            .collect::<Result<Vec<_>, String>>()?;
        retained_input.tree_count = i64::try_from(retained_input.trees.len())
            .map_err(|_| "tree pack retained tree count overflows i64".to_string())?;
        if retained_input.trees.is_empty() {
            return Ok(());
        }
        self.record_tree_pack_metadata_with_ordinals(
            scope,
            &retained_input,
            Some(&pack_entry_ordinals),
        )
    }

    pub fn record_snapshot(
        &self,
        scope: BinaryDbCommandScope,
        input: &BinaryDbSnapshotWriteInput,
    ) -> StoreResult<bool> {
        self.record_snapshots(scope, std::slice::from_ref(input))
            .map(|mut results| results.pop().unwrap_or(false))
    }

    pub fn record_snapshots(
        &self,
        scope: BinaryDbCommandScope,
        inputs: &[BinaryDbSnapshotWriteInput],
    ) -> StoreResult<Vec<bool>> {
        ensure_content_write_scope(scope)?;
        let mut tx = self.snapshots.begin_write_txn(scope)?;
        let mut results = Vec::with_capacity(inputs.len());
        for input in inputs {
            results
                .push(self.record_snapshot_with_history_boundary_in_write(&mut tx, input, false)?);
        }
        if results.iter().any(|wrote| *wrote) {
            tx.commit()?;
        } else {
            tx.abort()?;
        }
        Ok(results)
    }

    pub fn record_snapshot_at_remote_head_history_boundary(
        &self,
        scope: BinaryDbCommandScope,
        input: &BinaryDbSnapshotWriteInput,
    ) -> StoreResult<bool> {
        self.record_snapshot_with_history_boundary(scope, input, true)
    }

    fn record_snapshot_with_history_boundary(
        &self,
        scope: BinaryDbCommandScope,
        input: &BinaryDbSnapshotWriteInput,
        remote_head_history_boundary: bool,
    ) -> StoreResult<bool> {
        ensure_content_write_scope(scope)?;
        let mut tx = self.snapshots.begin_write_txn(scope)?;
        let wrote = self.record_snapshot_with_history_boundary_in_write(
            &mut tx,
            input,
            remote_head_history_boundary,
        )?;
        if wrote {
            tx.commit()?;
        } else {
            tx.abort()?;
        }
        Ok(wrote)
    }

    fn record_snapshot_with_history_boundary_in_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, B, F>,
        input: &BinaryDbSnapshotWriteInput,
        remote_head_history_boundary: bool,
    ) -> StoreResult<bool>
    where
        F: BinaryDbFsyncPolicy,
    {
        validate_non_empty(&input.snapshot_id, "snapshot_id")?;
        let snapshot_hash = snapshot_hash48_from_id(&input.snapshot_id)?;
        let snapshot_meta =
            snapshot_kind_byte(&input.snapshot_kind)? | BinarySnapshotRecord::META_HAS_ROOT_LOCATOR;
        let root_entry_ordinal = u32_from_i64(input.root_entry_ordinal, "root_entry_ordinal")?;
        let manifest_hash = parse_hex_array::<32>(&input.manifest_hash, "manifest_hash")?;
        let file_count = u32_from_i64(input.file_count, "file_count")?;
        let total_bytes = u64_from_i64(input.total_bytes, "total_bytes")?;
        let created_at_s = parse_created_at_s(&input.created_at)?;
        let parent_snapshot_ids = input.parent_snapshot_ids.clone();
        let (primary_parent_snapshot_id, parent_snapshot_id) =
            compatibility_parent_projections(&parent_snapshot_ids);
        validate_snapshot_parent_set(
            Some(&input.snapshot_id),
            &parent_snapshot_ids,
            primary_parent_snapshot_id.as_deref(),
            parent_snapshot_id.as_deref(),
        )?;

        if snapshot_index_in_write::<B, _, WRITE_LAYOUT>(tx, &input.snapshot_id)?.is_some() {
            return Ok(false);
        }
        let source_has_parent = !parent_snapshot_ids.is_empty();
        let parent_snapshot_indices = if remote_head_history_boundary {
            Vec::new()
        } else {
            parent_snapshot_ids
                .iter()
                .map(|parent_id| {
                    snapshot_index_in_write::<B, _, WRITE_LAYOUT>(tx, parent_id)?.ok_or_else(|| {
                        BinaryDbError::missing_data(format!(
                            "Binary DB parent snapshot {parent_id} is missing."
                        ))
                    })
                })
                .collect::<StoreResult<Vec<_>>>()?
        };
        let parent_snapshot_index_plus1 = parent_snapshot_indices
            .first()
            .copied()
            .map(|index| {
                index
                    .checked_add(1)
                    .ok_or_else(|| "parent snapshot index overflow".to_string())
            })
            .transpose()?
            .unwrap_or(0);
        let root_tree_pack_index_plus1 =
            tree_pack_index_in_write::<B, _, WRITE_LAYOUT>(tx, &input.root_tree_pack_id)?
                .ok_or_else(|| {
                    format!(
                        "Binary DB root tree pack {} is missing for snapshot {}.",
                        input.root_tree_pack_id, input.snapshot_id
                    )
                })?
                .checked_add(1)
                .ok_or_else(|| "root tree pack index overflow".to_string())?;
        let line_index_plus1 = binary_line_index_by_name_in_write(tx, &input.line_name)?
            .map(|index| {
                index
                    .checked_add(1)
                    .ok_or_else(|| "line index overflow".to_string())
            })
            .transpose()?
            .unwrap_or(0);
        let expected_snapshot_index =
            tx.record_count(BinaryDbSnapshotStore::<B, WRITE_LAYOUT>::snapshot_file())?;
        let (snapshot_index, _, _) = self.snapshots.append_snapshot_with_id_index(
            tx,
            BinarySnapshotRecord {
                snapshot_meta,
                history_flags: if remote_head_history_boundary && source_has_parent {
                    BinarySnapshotRecord::FLAG_REMOTE_HEAD_HISTORY_BOUNDARY
                } else {
                    0
                },
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: snapshot_hash,
                parent_snapshot_index_plus1,
                root_tree_pack_index_plus1,
                root_entry_ordinal,
                line_index_plus1,
                manifest_hash,
                file_count,
                total_bytes,
                created_at_s,
            },
            &BinarySnapshotPayload {
                line_name: input.line_name.clone(),
                message: input.message.clone(),
                additional_parent_snapshot_indices: parent_snapshot_indices
                    .iter()
                    .skip(1)
                    .copied()
                    .collect(),
            },
        )?;
        ensure_dense_index("snapshot", expected_snapshot_index, snapshot_index)?;
        Ok(true)
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_existing_tree_pack_metadata<B, F, const WRITE_LAYOUT: u32>(
    tx: &BinaryDbWriteTxn<'_, B, F>,
    pack_index: u32,
    input: &BinaryDbTreePackWriteInput,
    ordered_trees: &[&BinaryDbTreePackTreeWriteInput],
    pack_format_kind: u8,
    tree_count: u32,
    total_bytes: u64,
    created_at_s: u64,
    pack_entry_ordinals: Option<&BTreeMap<String, u32>>,
) -> StoreResult<()>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    let raw_pack = tx.read_record(
        BinaryTreePackCodec::<WRITE_LAYOUT>::record_file(),
        pack_index,
    )?;
    let pack = BinaryTreePackCodec::<WRITE_LAYOUT>::decode_record(&raw_pack)?;
    let expected_pack_meta = BinaryTreePackRecord::META_READY
        | if pack_entry_ordinals.is_some() {
            BinaryTreePackRecord::META_SPARSE_PHYSICAL_ORDINALS
        } else {
            0
        };
    if pack.pack_meta != expected_pack_meta
        || pack.pack_format_kind != pack_format_kind
        || pack.tree_count != tree_count
        || pack.total_bytes != total_bytes
        || pack.created_at_s != created_at_s
    {
        return Err(BinaryDbError::corruption(format!(
            "Existing Binary DB tree pack {} metadata does not match the verified import.",
            input.pack_id
        )));
    }

    let tree_end = pack
        .first_tree_index
        .checked_add(tree_count)
        .ok_or_else(|| BinaryDbError::corruption("Binary DB tree index overflow"))?;
    if tree_end > tx.record_count(BinaryTreeCodec::<WRITE_LAYOUT>::record_file())? {
        return Err(BinaryDbError::corruption(format!(
            "Existing Binary DB tree pack {} points beyond tree.bin.",
            input.pack_id
        )));
    }

    for (ordinal, expected_tree) in ordered_trees.iter().enumerate() {
        let expected_local_ordinal = u32::try_from(ordinal).map_err(|_| {
            BinaryDbError::invalid_domain_data(format!(
                "Binary DB tree ordinal overflows u32: {ordinal}"
            ))
        })?;
        let expected_pack_entry_ordinal = pack_entry_ordinals
            .map(|ordinals| {
                ordinals
                    .get(&expected_tree.tree_id.to_ascii_lowercase())
                    .copied()
                    .ok_or_else(|| {
                        BinaryDbError::invalid_domain_data(format!(
                            "Binary DB tree pack {} is missing the physical ordinal for tree {}.",
                            input.pack_id, expected_tree.tree_id
                        ))
                    })
            })
            .transpose()?
            .unwrap_or(expected_local_ordinal);
        let tree_index = pack
            .first_tree_index
            .checked_add(expected_local_ordinal)
            .ok_or_else(|| BinaryDbError::corruption("Binary DB tree index overflow"))?;
        let raw_tree =
            tx.read_record(BinaryTreeCodec::<WRITE_LAYOUT>::record_file(), tree_index)?;
        let tree = BinaryTreeCodec::<WRITE_LAYOUT>::decode_record(&raw_tree)?;
        let expected_entry_count = u32_from_i64(expected_tree.entry_count, "entry_count")?;
        if tree.tree_meta != 0
            || tree.reserved0 != 0
            || tree.pack_entry_ordinal != expected_pack_entry_ordinal
            || tree.entry_count != expected_entry_count
            || !tree_id_from_hash80(&tree.tree_hash80).eq_ignore_ascii_case(&expected_tree.tree_id)
        {
            return Err(BinaryDbError::corruption(format!(
                "Existing Binary DB tree pack {} ordinal {} tree locator does not match verified tree {}.",
                input.pack_id, ordinal, expected_tree.tree_id
            )));
        }
    }
    Ok(())
}

fn ensure_content_write_scope(scope: BinaryDbCommandScope) -> StoreResult<()> {
    if scope.lock_file_names().contains(&"content.write.lock") {
        return Ok(());
    }
    Err(BinaryDbError::invalid_domain_data(format!(
        "Binary DB content coordinator scope {scope:?} does not own content.write.lock"
    )))
}

fn newest_unique_candidates(mut candidates: Vec<u32>) -> Vec<u32> {
    candidates.sort_unstable();
    candidates.dedup();
    candidates.reverse();
    candidates
}

fn blob_index_in_write<B, F, const WRITE_LAYOUT: u32>(
    tx: &BinaryDbWriteTxn<'_, B, F>,
    blob_id: &str,
) -> StoreResult<Option<u32>>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    let key = blob_id_index_key(blob_id)?;
    for index in newest_unique_candidates(
        tx.lookup_index(BinaryBlobCodec::<WRITE_LAYOUT>::id_index(), &key)?,
    ) {
        let raw = tx.read_record(BinaryBlobCodec::<WRITE_LAYOUT>::record_file(), index)?;
        let record = BinaryBlobCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
        if !record.is_tombstone()
            && blob_id_from_sha256(&record.sha256).eq_ignore_ascii_case(blob_id)
        {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn object_pack_index_in_write<B, F, const WRITE_LAYOUT: u32>(
    tx: &BinaryDbWriteTxn<'_, B, F>,
    pack_id: &str,
) -> StoreResult<Option<u32>>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    let key = object_pack_id_index_key(pack_id)?;
    for index in newest_unique_candidates(
        tx.lookup_index(BinaryObjectPackCodec::<WRITE_LAYOUT>::id_index(), &key)?,
    ) {
        let raw = tx.read_record(BinaryObjectPackCodec::<WRITE_LAYOUT>::record_file(), index)?;
        let record = BinaryObjectPackCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
        if !record.is_tombstone()
            && object_pack_id_from_hash48(record.pack_hash48()).eq_ignore_ascii_case(pack_id)
        {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn tree_pack_index_in_write<B, F, const WRITE_LAYOUT: u32>(
    tx: &BinaryDbWriteTxn<'_, B, F>,
    pack_id: &str,
) -> StoreResult<Option<u32>>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    let key = tree_pack_id_index_key(pack_id)?;
    for index in newest_unique_candidates(
        tx.lookup_index(BinaryTreePackCodec::<WRITE_LAYOUT>::id_index(), &key)?,
    ) {
        let raw = tx.read_record(BinaryTreePackCodec::<WRITE_LAYOUT>::record_file(), index)?;
        let record = BinaryTreePackCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
        if !record.is_tombstone()
            && tree_pack_id_from_hash48(record.pack_hash48()).eq_ignore_ascii_case(pack_id)
        {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn tree_index_in_write<B, F, const WRITE_LAYOUT: u32>(
    tx: &BinaryDbWriteTxn<'_, B, F>,
    tree_id: &str,
) -> StoreResult<Option<u32>>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    let key = tree_id_index_key(tree_id)?;
    for index in newest_unique_candidates(
        tx.lookup_index(BinaryTreeCodec::<WRITE_LAYOUT>::id_index(), &key)?,
    ) {
        let raw = tx.read_record(BinaryTreeCodec::<WRITE_LAYOUT>::record_file(), index)?;
        let record = BinaryTreeCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
        if !record.is_tombstone()
            && tree_id_from_hash80(&record.tree_hash80).eq_ignore_ascii_case(tree_id)
        {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn snapshot_index_in_write<B, F, const WRITE_LAYOUT: u32>(
    tx: &BinaryDbWriteTxn<'_, B, F>,
    snapshot_id: &str,
) -> StoreResult<Option<u32>>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    let key = snapshot_id_index_key(snapshot_id)?;
    for index in newest_unique_candidates(
        tx.lookup_index(BinarySnapshotCodec::<WRITE_LAYOUT>::id_index(), &key)?,
    ) {
        let raw = tx.read_record(BinarySnapshotCodec::<WRITE_LAYOUT>::record_file(), index)?;
        let record = BinarySnapshotCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
        if !record.is_tombstone()
            && snapshot_id_from_hash48(record.snapshot_hash48()).eq_ignore_ascii_case(snapshot_id)
        {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn ensure_dense_index(label: &str, expected: u32, actual: u32) -> StoreResult<()> {
    if actual == expected {
        return Ok(());
    }
    Err(BinaryDbError::corruption(format!(
        "Binary DB {label} append returned dense index {actual}, expected {expected}"
    )))
}

fn object_pack_format_kind_byte(kind: PackFormatKind) -> u8 {
    match kind {
        PackFormatKind::ZstdChunkedV1 => 1,
    }
}

fn tree_pack_format_kind_byte(kind: TreePackFormatKind) -> u8 {
    match kind {
        TreePackFormatKind::ZstdChunkedTreeV1 => 1,
    }
}

fn tree_entry_kind_byte(value: &str) -> StoreResult<u8> {
    match value.trim() {
        "blob" => Ok(0),
        "tree" => Ok(1),
        other => Err(format!("unsupported Binary DB tree entry kind: {other}").into()),
    }
}

fn tree_entry_mode_bits(entry_type: &str, mode: &str) -> StoreResult<u16> {
    if entry_type.trim() == "tree" {
        return Ok(0);
    }
    let trimmed = mode.trim();
    let octal = trimmed
        .strip_prefix("0o")
        .or_else(|| trimmed.strip_prefix("0O"))
        .unwrap_or(trimmed);
    u16::from_str_radix(octal, 8)
        .map_err(|err| format!("invalid Binary DB tree entry mode `{mode}`: {err}").into())
}

fn object_pack_member_meta(pack_entry_type: &str) -> u8 {
    let kind = match pack_entry_type.trim() {
        "delta" => BinaryObjectPackMemberKind::Delta,
        _ => BinaryObjectPackMemberKind::Full,
    };
    let compression = BinaryObjectPackCompressionKind::Zstd;
    let kind_bits = match kind {
        BinaryObjectPackMemberKind::Full => 0,
        BinaryObjectPackMemberKind::Delta => 1,
        BinaryObjectPackMemberKind::Reserved(value) => value & 0b0000_0011,
    };
    let compression_bits = match compression {
        BinaryObjectPackCompressionKind::None => 0,
        BinaryObjectPackCompressionKind::Zstd => 2,
        BinaryObjectPackCompressionKind::Reserved(value) => value & 0b0000_0011,
    };
    kind_bits | (compression_bits << 2)
}

fn snapshot_kind_byte(value: &str) -> StoreResult<u8> {
    match value.trim() {
        "" | "line" => Ok(0),
        "stash" => Ok(1),
        other => Err(format!("unsupported Binary DB snapshot kind: {other}").into()),
    }
}

fn split_hash48(value: u64) -> (u16, u32) {
    ((value >> 32) as u16, value as u32)
}

fn parse_created_at_s(value: &str) -> StoreResult<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    if let Ok(value) = trimmed.parse::<u64>() {
        return Ok(value);
    }
    let normalized = normalize_legacy_transport_timestamp(trimmed);
    let parsed = DateTime::parse_from_rfc3339(&normalized)
        .map_err(|err| format!("invalid Binary DB created_at timestamp `{trimmed}`: {err}"))?
        .with_timezone(&Utc);
    Ok(u64::try_from(parsed.timestamp()).map_err(|_| {
        format!("Binary DB created_at timestamp `{trimmed}` is before the Unix epoch")
    })?)
}

/// Archived relational exports used a space separator and occasionally emitted a
/// whole-hour UTC offset without minutes. Accept those two bounded transport
/// variants at the import boundary and persist only epoch seconds.
fn normalize_legacy_transport_timestamp(value: &str) -> String {
    let mut normalized = value.to_string();
    if normalized.as_bytes().get(10) == Some(&b' ') {
        normalized.replace_range(10..11, "T");
    }
    let bytes = normalized.as_bytes();
    if bytes.len() >= 3 {
        let sign = bytes.len() - 3;
        if matches!(bytes[sign], b'+' | b'-') && bytes[sign + 1..].iter().all(u8::is_ascii_digit) {
            normalized.push_str(":00");
            return normalized;
        }
    }
    let bytes = normalized.as_bytes();
    if bytes.len() >= 5 {
        let sign = bytes.len() - 5;
        if matches!(bytes[sign], b'+' | b'-') && bytes[sign + 1..].iter().all(u8::is_ascii_digit) {
            normalized.insert(normalized.len() - 2, ':');
        }
    }
    normalized
}

fn parse_hex_array<const N: usize>(value: &str, label: &str) -> StoreResult<[u8; N]> {
    let trimmed = value.trim();
    if trimmed.len() != N * 2 {
        return Err(format!(
            "{label} must have {} hex chars, got {}",
            N * 2,
            trimmed.len()
        )
        .into());
    }
    let mut out = [0_u8; N];
    for (index, chunk) in trimmed.as_bytes().chunks_exact(2).enumerate() {
        let hi = hex_nibble(chunk[0]).ok_or_else(|| format!("{label} contains invalid hex"))?;
        let lo = hex_nibble(chunk[1]).ok_or_else(|| format!("{label} contains invalid hex"))?;
        out[index] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn u8_from_i64(value: i64, label: &str) -> StoreResult<u8> {
    Ok(u8::try_from(value).map_err(|_| format!("{label} is outside u8 range: {value}"))?)
}

fn u32_from_i64(value: i64, label: &str) -> StoreResult<u32> {
    Ok(u32::try_from(value).map_err(|_| format!("{label} is outside u32 range: {value}"))?)
}

fn u64_from_i64(value: i64, label: &str) -> StoreResult<u64> {
    Ok(u64::try_from(value)
        .map_err(|_| format!("{label} is negative or overflows u64: {value}"))?)
}

fn validate_non_empty(value: &str, label: &str) -> StoreResult<()> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty").into());
    }
    Ok(())
}

#[cfg(test)]
mod created_at_timestamp_tests {
    use super::parse_created_at_s;

    #[test]
    fn accepts_historical_transport_timestamp_with_whole_hour_offset() {
        assert_eq!(
            parse_created_at_s("2026-07-05 21:55:30+08").unwrap(),
            parse_created_at_s("2026-07-05T21:55:30+08:00").unwrap()
        );
    }

    #[test]
    fn accepts_historical_compact_numeric_offset() {
        assert_eq!(
            parse_created_at_s("2026-07-05 21:55:30+0800").unwrap(),
            parse_created_at_s("2026-07-05T21:55:30+08:00").unwrap()
        );
    }
}
