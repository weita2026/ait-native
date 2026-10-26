//! Typed native Git pins for content validation and garbage-collection reads.
use super::*;
use crate::binary_db::{BinaryDbReadTxn, BinaryFileId, LocalBinaryDbFs};
use crate::content_binary_db::{BinaryDbTreeReadCache, BinaryTreePackCodec, LocalContentBinaryDb};
use crate::plan_binary_db::BinaryDbPlanStore;
use std::collections::BTreeSet;

#[derive(Default)]
pub struct GitContentPins {
    pub blob_ids: BTreeSet<String>,
    pub tree_ids: BTreeSet<String>,
}
pub fn content_gc_pins<const W: u32>(
    interop: &LocalBinaryDbFs,
    content: &LocalContentBinaryDb<W>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    cache: &mut BinaryDbTreeReadCache,
) -> Result<GitContentPins, String> {
    let git = BinaryDbReadTxn::new(interop);
    let mut pins = GitContentPins::default();
    let objects = BinaryFileId::new("git_object.bin", 1, 32);
    for i in 0..git.record_count(objects.clone())? {
        let object = GitObjectRecord::decode(&git.read_record(objects.clone(), i)?)?;
        pins.blob_ids.insert(
            content
                .blobs()
                .blob_view_at(read, object.content_blob_index)?
                .blob_id,
        );
    }
    let mappings = BinaryFileId::new("git_commit_mapping.bin", 1, 100);
    for i in 0..git.record_count(mappings.clone())? {
        let mapping = GitCommitMappingRecord::decode(&git.read_record(mappings.clone(), i)?)?;
        content
            .snapshots()
            .snapshot_view_at(read, mapping.snapshot_index)?;
        pins.tree_ids.insert(
            content
                .trees()
                .tree_view_at_with_cache(read, mapping.document_set_tree_index, cache)?
                .tree_id,
        );
    }
    let plans = BinaryDbPlanStore::<_, W>::new(content.db().clone());
    for i in 0..read.record_count(BinaryDbPlanStore::<LocalBinaryDbFs, W>::plan_revision_file())? {
        let (revision, _) = plans.read_plan_revision(read, i)?;
        if !revision.is_raw_git_artifact() {
            continue;
        }
        if revision.is_tombstone() {
            return Err("raw Git PlanRevision slots must remain pinned".into());
        }
        let pack_index = revision
            .root_tree_pack_index()
            .ok_or("raw Git revision root absent")?;
        let pack = BinaryTreePackCodec::<W>::decode_record(
            &read.read_record(BinaryTreePackCodec::<W>::record_file(), pack_index)?,
        )?;
        if !pack.is_ready() || pack.is_tombstone() || revision.root_entry_ordinal >= pack.tree_count
        {
            return Err("raw Git PlanRevision root invalid".into());
        }
        let tree = pack
            .first_tree_index
            .checked_add(revision.root_entry_ordinal)
            .ok_or("raw root ordinal overflow")?;
        pins.tree_ids.insert(
            content
                .trees()
                .tree_view_at_with_cache(read, tree, cache)?
                .tree_id,
        );
    }
    Ok(pins)
}
