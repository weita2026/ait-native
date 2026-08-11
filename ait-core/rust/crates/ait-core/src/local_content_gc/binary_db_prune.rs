use super::*;
use crate::binary_db::{
    BinaryDbCommandScope, BinaryDbFsyncPolicy, BinaryDbWriteTxn, LocalBinaryDbFs,
};
use crate::content_binary_db::{
    absolute_repo_path, object_pack_format_name, object_pack_id_from_hash48,
    object_pack_relative_path, BinaryBlobCodec, BinaryBlobRecord, BinaryDbBlobStore,
    BinaryDbObjectPackStore, BinaryObjectPackCodec, BinaryObjectPackMemberCodec,
    BinaryObjectPackMemberRecord, BinaryObjectPackRecord,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;

#[derive(Clone, Debug, Eq, PartialEq)]
struct BinaryDbOrphanObjectPack {
    pack_index: u32,
    pack_id: String,
    pack_path: String,
    record: BinaryObjectPackRecord,
    members: Vec<(u32, BinaryObjectPackMemberRecord)>,
    blob_tombstones: Vec<(u32, BinaryBlobRecord, u32)>,
    verified_fallback_blob_indices: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BinaryDbBaseBlobPointerRewrite {
    member_index: u32,
    record: BinaryObjectPackMemberRecord,
    fallback_blob_index: u32,
}

pub(super) fn binary_db_prune_orphan_packs<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
) -> Result<JsonValue, String> {
    // Phase one takes a stable catalog plan under content.write.lock, then
    // releases it before reading fallback bytes through the normal Blob API.
    let mut planning_write = content
        .object_packs()
        .begin_write_txn(BinaryDbCommandScope::ContentWrite)
        .map_err(|error| error.to_string())?;
    let candidates = binary_db_orphan_object_pack_candidates::<WRITE_LAYOUT, _>(&planning_write)?;
    let base_pointer_rewrites =
        binary_db_base_blob_pointer_rewrites::<WRITE_LAYOUT, _>(&planning_write, &candidates)?;
    planning_write.abort().map_err(|error| error.to_string())?;
    if candidates.is_empty() && base_pointer_rewrites.is_empty() {
        return Ok(binary_db_prune_summary(
            Vec::new(),
            0,
            0,
            0,
            0,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ));
    }

    binary_db_verify_fallback_blob_content(content, &candidates, &base_pointer_rewrites)?;

    // Phase three reacquires only content.write.lock and recomputes the plan.
    // Any concurrent catalog change makes the command fail before mutation.
    let mut write = content
        .object_packs()
        .begin_write_txn(BinaryDbCommandScope::ContentWrite)
        .map_err(|error| error.to_string())?;
    let revalidated = binary_db_orphan_object_pack_candidates::<WRITE_LAYOUT, _>(&write)?;
    let revalidated_base_pointer_rewrites =
        binary_db_base_blob_pointer_rewrites::<WRITE_LAYOUT, _>(&write, &revalidated)?;
    if candidates != revalidated || base_pointer_rewrites != revalidated_base_pointer_rewrites {
        write.abort().map_err(|error| error.to_string())?;
        return Err(
            "Binary DB content catalog changed while orphan-pack fallbacks were being verified; rerun gc prune."
                .to_string(),
        );
    }

    let mut removed_member_count = 0_usize;
    let mut removed_duplicate_blob_count = 0_usize;
    let mut tombstoned_blob_indices = BTreeSet::new();
    for rewrite in &base_pointer_rewrites {
        let mut record = rewrite.record.clone();
        record.base_blob_index_plus1 =
            rewrite.fallback_blob_index.checked_add(1).ok_or_else(|| {
                format!(
                    "Binary DB fallback blob index {} cannot be encoded as a member pointer.",
                    rewrite.fallback_blob_index
                )
            })?;
        let bytes = BinaryObjectPackMemberCodec::<WRITE_LAYOUT>::encode_record(&record)
            .map_err(|error| error.to_string())?;
        write
            .overwrite_record(
                BinaryDbObjectPackStore::<LocalBinaryDbFs, WRITE_LAYOUT>::object_pack_member_file(),
                rewrite.member_index,
                &bytes,
            )
            .map_err(|error| error.to_string())?;
    }
    for candidate in &candidates {
        for (blob_index, blob, _) in &candidate.blob_tombstones {
            if !tombstoned_blob_indices.insert(*blob_index) {
                return Err(format!(
                    "Binary DB orphan-pack plan selected duplicate blob tombstone {blob_index}."
                ));
            }
            let mut tombstone = blob.clone();
            tombstone.blob_meta |= BinaryBlobRecord::META_TOMBSTONE;
            let bytes = BinaryBlobCodec::<WRITE_LAYOUT>::encode_record(&tombstone)
                .map_err(|error| error.to_string())?;
            write
                .overwrite_record(
                    BinaryDbBlobStore::<LocalBinaryDbFs, WRITE_LAYOUT>::blob_file(),
                    *blob_index,
                    &bytes,
                )
                .map_err(|error| error.to_string())?;
            removed_duplicate_blob_count += 1;
        }
        for (member_index, member) in &candidate.members {
            let mut tombstone = member.clone();
            tombstone.member_meta |= BinaryObjectPackMemberRecord::META_TOMBSTONE;
            let bytes = BinaryObjectPackMemberCodec::<WRITE_LAYOUT>::encode_record(&tombstone)
                .map_err(|error| error.to_string())?;
            write
                .overwrite_record(
                    BinaryDbObjectPackStore::<LocalBinaryDbFs, WRITE_LAYOUT>::object_pack_member_file(),
                    *member_index,
                    &bytes,
                )
                .map_err(|error| error.to_string())?;
            removed_member_count += 1;
        }
        let mut tombstone = candidate.record.clone();
        tombstone.pack_meta |= BinaryObjectPackRecord::META_TOMBSTONE;
        let bytes = BinaryObjectPackCodec::<WRITE_LAYOUT>::encode_record(&tombstone)
            .map_err(|error| error.to_string())?;
        write
            .overwrite_record(
                BinaryDbObjectPackStore::<LocalBinaryDbFs, WRITE_LAYOUT>::object_pack_file(),
                candidate.pack_index,
                &bytes,
            )
            .map_err(|error| error.to_string())?;
    }

    let commit = write.commit().map_err(|error| error.to_string())?;
    let mut cleanup_warnings = commit
        .into_lock_cleanup_warning()
        .map(|warning| {
            vec![format!(
                "Binary DB catalog committed, but lock cleanup warned: {warning}"
            )]
        })
        .unwrap_or_default();

    // Archive deletion is intentionally after the durable catalog commit. A
    // failed unlink cannot make readers resolve a deleted archive through live
    // metadata; it is reported as cleanup debt instead of a retryable catalog
    // failure.
    let mut removed_pack_paths = Vec::new();
    let mut already_missing_pack_paths = Vec::new();
    for candidate in &candidates {
        let absolute = absolute_repo_path(content.pack_root(), &candidate.pack_path)
            .map_err(|error| error.to_string())?;
        match fs::remove_file(&absolute) {
            Ok(()) => removed_pack_paths.push(candidate.pack_path.clone()),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                already_missing_pack_paths.push(candidate.pack_path.clone())
            }
            Err(error) => cleanup_warnings.push(format!(
                "Binary DB catalog no longer references {}, but archive cleanup failed at {}: {error}",
                candidate.pack_id,
                absolute.display()
            )),
        }
    }

    Ok(binary_db_prune_summary(
        candidates
            .iter()
            .map(|candidate| candidate.pack_id.clone())
            .collect(),
        removed_member_count,
        removed_duplicate_blob_count,
        candidates
            .iter()
            .flat_map(|candidate| candidate.verified_fallback_blob_indices.iter().copied())
            .chain(
                base_pointer_rewrites
                    .iter()
                    .map(|rewrite| rewrite.fallback_blob_index),
            )
            .collect::<BTreeSet<_>>()
            .len(),
        base_pointer_rewrites.len(),
        removed_pack_paths,
        already_missing_pack_paths,
        cleanup_warnings,
    ))
}

fn binary_db_orphan_object_pack_candidates<const WRITE_LAYOUT: u32, F>(
    write: &BinaryDbWriteTxn<'_, LocalBinaryDbFs, F>,
) -> Result<Vec<BinaryDbOrphanObjectPack>, String>
where
    F: BinaryDbFsyncPolicy,
{
    let blob_file = BinaryDbBlobStore::<LocalBinaryDbFs, WRITE_LAYOUT>::blob_file();
    let blob_count = write
        .record_count(blob_file.clone())
        .map_err(|error| error.to_string())?;
    let mut live_member_indices = BTreeSet::new();
    let mut blobs = Vec::with_capacity(blob_count as usize);
    for blob_index in 0..blob_count {
        let raw = write
            .read_record(blob_file.clone(), blob_index)
            .map_err(|error| error.to_string())?;
        let blob = BinaryBlobCodec::<WRITE_LAYOUT>::decode_record(&raw)
            .map_err(|error| error.to_string())?;
        if !blob.is_tombstone() && !blob.is_pruned() {
            if let Some(member_index) = blob.pack_member_index() {
                live_member_indices.insert(member_index);
            }
        }
        blobs.push(blob);
    }

    let pack_file = BinaryDbObjectPackStore::<LocalBinaryDbFs, WRITE_LAYOUT>::object_pack_file();
    let member_file =
        BinaryDbObjectPackStore::<LocalBinaryDbFs, WRITE_LAYOUT>::object_pack_member_file();
    let pack_count = write
        .record_count(pack_file.clone())
        .map_err(|error| error.to_string())?;
    let member_count = write
        .record_count(member_file.clone())
        .map_err(|error| error.to_string())?;
    let mut candidates = Vec::new();

    for pack_index in 0..pack_count {
        let raw = write
            .read_record(pack_file.clone(), pack_index)
            .map_err(|error| error.to_string())?;
        let record = BinaryObjectPackCodec::<WRITE_LAYOUT>::decode_record(&raw)
            .map_err(|error| error.to_string())?;
        if record.is_tombstone() {
            continue;
        }
        let pack_id = object_pack_id_from_hash48(record.pack_hash48());
        let end = record
            .first_member_index
            .checked_add(record.member_count)
            .ok_or_else(|| format!("Object pack {pack_id} member range overflows u32."))?;
        if end > member_count {
            return Err(format!(
                "Object pack {pack_id} requires member records [{}..{end}), but only {member_count} exist.",
                record.first_member_index
            ));
        }

        let mut referenced_member_count = 0_usize;
        let mut members = Vec::with_capacity(record.member_count as usize);
        for member_index in record.first_member_index..end {
            let raw = write
                .read_record(member_file.clone(), member_index)
                .map_err(|error| error.to_string())?;
            let member = BinaryObjectPackMemberCodec::<WRITE_LAYOUT>::decode_record(&raw)
                .map_err(|error| error.to_string())?;
            if member.is_tombstone() {
                return Err(format!(
                    "Live object pack {pack_id} contains tombstoned member record {member_index}."
                ));
            }
            if member.pack_index != pack_index {
                return Err(format!(
                    "Object pack {pack_id} member record {member_index} points to pack {}, expected {pack_index}.",
                    member.pack_index
                ));
            }
            if live_member_indices.contains(&member_index) {
                referenced_member_count += 1;
            }
            members.push((member_index, member));
        }

        let recovery = binary_db_pack_recovery_plan::<WRITE_LAYOUT, _>(
            write,
            &pack_id,
            &members,
            &blobs,
            pack_count,
            member_count,
            &pack_file,
            &member_file,
        )?;
        if let Some((blob_tombstones, verified_fallback_blob_indices)) = recovery {
            let pack_format =
                object_pack_format_name(record.format_kind()).map_err(|error| error.to_string())?;
            let pack_path = object_pack_relative_path(&pack_id, pack_format)
                .map_err(|error| error.to_string())?;
            candidates.push(BinaryDbOrphanObjectPack {
                pack_index,
                pack_id,
                pack_path,
                record,
                members,
                blob_tombstones,
                verified_fallback_blob_indices,
            });
        } else if referenced_member_count == 0 {
            return Err(format!(
                "Object pack {pack_id} has no live member locator, but its content does not have a verified authoritative fallback; refusing orphan-pack pruning."
            ));
        } else if referenced_member_count != record.member_count as usize {
            return Err(format!(
                "Object pack {pack_id} is only partially referenced: {referenced_member_count} of {} members have live blob locators; refusing orphan-pack pruning.",
                record.member_count
            ));
        }
    }
    Ok(candidates)
}

type BinaryDbPackRecoveryPlan = (Vec<(u32, BinaryBlobRecord, u32)>, Vec<u32>);

#[allow(clippy::too_many_arguments)]
fn binary_db_pack_recovery_plan<const WRITE_LAYOUT: u32, F>(
    write: &BinaryDbWriteTxn<'_, LocalBinaryDbFs, F>,
    candidate_pack_id: &str,
    candidate_members: &[(u32, BinaryObjectPackMemberRecord)],
    blobs: &[BinaryBlobRecord],
    pack_count: u32,
    member_count: u32,
    pack_file: &crate::binary_db::BinaryFileId,
    member_file: &crate::binary_db::BinaryFileId,
) -> Result<Option<BinaryDbPackRecoveryPlan>, String>
where
    F: BinaryDbFsyncPolicy,
{
    let mut blob_tombstones = BTreeMap::new();
    let mut fallback_blob_indices = BTreeSet::new();
    for (candidate_member_index, candidate_member) in candidate_members {
        let blob = blobs
            .get(candidate_member.blob_index as usize)
            .ok_or_else(|| {
                format!(
                    "Object pack {candidate_pack_id} member {candidate_member_index} references missing blob record {}.",
                    candidate_member.blob_index
                )
            })?;
        if blob.is_tombstone() || blob.is_pruned() {
            continue;
        }
        match blob.pack_member_index() {
            Some(authoritative_member_index)
                if authoritative_member_index != *candidate_member_index =>
            {
                binary_db_validate_authoritative_member::<WRITE_LAYOUT, _>(
                    write,
                    authoritative_member_index,
                    candidate_member.blob_index,
                    pack_count,
                    member_count,
                    pack_file,
                    member_file,
                )?;
                fallback_blob_indices.insert(candidate_member.blob_index);
            }
            current_pointer => {
                let fallback_blob_index = binary_db_find_earlier_blob_fallback::<WRITE_LAYOUT, _>(
                    write,
                    candidate_member.blob_index,
                    blob,
                    blobs,
                    pack_count,
                    member_count,
                    pack_file,
                    member_file,
                )?;
                let Some(fallback_blob_index) = fallback_blob_index else {
                    if current_pointer.is_none() {
                        return Err(format!(
                            "Object pack {candidate_pack_id} member {candidate_member_index} is the only catalog copy for live blob record {}, but that blob has no authoritative member locator; refusing to delete recoverable history.",
                            candidate_member.blob_index
                        ));
                    }
                    return Ok(None);
                };
                blob_tombstones.insert(
                    candidate_member.blob_index,
                    (blob.clone(), fallback_blob_index),
                );
                fallback_blob_indices.insert(fallback_blob_index);
            }
        }
    }
    Ok(Some((
        blob_tombstones
            .into_iter()
            .map(|(blob_index, (blob, fallback_blob_index))| {
                (blob_index, blob, fallback_blob_index)
            })
            .collect(),
        fallback_blob_indices.into_iter().collect(),
    )))
}

#[allow(clippy::too_many_arguments)]
fn binary_db_find_earlier_blob_fallback<const WRITE_LAYOUT: u32, F>(
    write: &BinaryDbWriteTxn<'_, LocalBinaryDbFs, F>,
    candidate_blob_index: u32,
    candidate_blob: &BinaryBlobRecord,
    blobs: &[BinaryBlobRecord],
    pack_count: u32,
    member_count: u32,
    pack_file: &crate::binary_db::BinaryFileId,
    member_file: &crate::binary_db::BinaryFileId,
) -> Result<Option<u32>, String>
where
    F: BinaryDbFsyncPolicy,
{
    let mut invalid_fallback = None;
    for fallback_blob_index in 0..candidate_blob_index {
        let fallback = &blobs[fallback_blob_index as usize];
        if fallback.is_tombstone()
            || fallback.is_pruned()
            || fallback.sha256 != candidate_blob.sha256
            || fallback.size_bytes != candidate_blob.size_bytes
            || fallback.hash_kind != candidate_blob.hash_kind
        {
            continue;
        }
        let Some(fallback_member_index) = fallback.pack_member_index() else {
            invalid_fallback = Some(format!(
                "Earlier duplicate blob record {fallback_blob_index} has no member locator."
            ));
            continue;
        };
        match binary_db_validate_authoritative_member::<WRITE_LAYOUT, _>(
            write,
            fallback_member_index,
            fallback_blob_index,
            pack_count,
            member_count,
            pack_file,
            member_file,
        ) {
            Ok(()) => return Ok(Some(fallback_blob_index)),
            Err(error) => invalid_fallback = Some(error),
        }
    }
    if let Some(error) = invalid_fallback {
        return Err(format!(
            "Blob record {candidate_blob_index} has an earlier duplicate, but it is not a valid fallback: {error}"
        ));
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
fn binary_db_validate_authoritative_member<const WRITE_LAYOUT: u32, F>(
    write: &BinaryDbWriteTxn<'_, LocalBinaryDbFs, F>,
    member_index: u32,
    expected_blob_index: u32,
    pack_count: u32,
    member_count: u32,
    pack_file: &crate::binary_db::BinaryFileId,
    member_file: &crate::binary_db::BinaryFileId,
) -> Result<(), String>
where
    F: BinaryDbFsyncPolicy,
{
    if member_index >= member_count {
        return Err(format!(
            "Live blob record {expected_blob_index} points to missing object-pack member {member_index}."
        ));
    }
    let member_raw = write
        .read_record(member_file.clone(), member_index)
        .map_err(|error| error.to_string())?;
    let member = BinaryObjectPackMemberCodec::<WRITE_LAYOUT>::decode_record(&member_raw)
        .map_err(|error| error.to_string())?;
    if member.is_tombstone() || member.blob_index != expected_blob_index {
        return Err(format!(
            "Object-pack member {member_index} is not a live backreference for blob record {expected_blob_index}."
        ));
    }
    if member.pack_index >= pack_count {
        return Err(format!(
            "Authoritative object-pack member {member_index} points to missing pack {}.",
            member.pack_index
        ));
    }
    let pack_raw = write
        .read_record(pack_file.clone(), member.pack_index)
        .map_err(|error| error.to_string())?;
    let pack = BinaryObjectPackCodec::<WRITE_LAYOUT>::decode_record(&pack_raw)
        .map_err(|error| error.to_string())?;
    let end = pack
        .first_member_index
        .checked_add(pack.member_count)
        .ok_or_else(|| {
            format!(
                "Authoritative object pack {} member range overflows u32.",
                member.pack_index
            )
        })?;
    if pack.is_tombstone() || member_index < pack.first_member_index || member_index >= end {
        return Err(format!(
            "Authoritative object-pack member {member_index} is not owned by a live pack range."
        ));
    }
    Ok(())
}

fn binary_db_base_blob_pointer_rewrites<const WRITE_LAYOUT: u32, F>(
    write: &BinaryDbWriteTxn<'_, LocalBinaryDbFs, F>,
    candidates: &[BinaryDbOrphanObjectPack],
) -> Result<Vec<BinaryDbBaseBlobPointerRewrite>, String>
where
    F: BinaryDbFsyncPolicy,
{
    let blob_file = BinaryDbBlobStore::<LocalBinaryDbFs, WRITE_LAYOUT>::blob_file();
    let member_file =
        BinaryDbObjectPackStore::<LocalBinaryDbFs, WRITE_LAYOUT>::object_pack_member_file();
    let pack_file = BinaryDbObjectPackStore::<LocalBinaryDbFs, WRITE_LAYOUT>::object_pack_file();
    let blob_count = write
        .record_count(blob_file.clone())
        .map_err(|error| error.to_string())?;
    let member_count = write
        .record_count(member_file.clone())
        .map_err(|error| error.to_string())?;
    let pack_count = write
        .record_count(pack_file.clone())
        .map_err(|error| error.to_string())?;

    let mut blobs = Vec::with_capacity(blob_count as usize);
    for blob_index in 0..blob_count {
        let raw = write
            .read_record(blob_file.clone(), blob_index)
            .map_err(|error| error.to_string())?;
        blobs.push(
            BinaryBlobCodec::<WRITE_LAYOUT>::decode_record(&raw)
                .map_err(|error| error.to_string())?,
        );
    }

    let candidate_member_indices = candidates
        .iter()
        .flat_map(|candidate| {
            candidate
                .members
                .iter()
                .map(|(member_index, _)| *member_index)
        })
        .collect::<BTreeSet<_>>();
    let planned_fallback_by_blob_index = candidates
        .iter()
        .flat_map(|candidate| {
            candidate
                .blob_tombstones
                .iter()
                .map(|(blob_index, _, fallback_blob_index)| (*blob_index, *fallback_blob_index))
        })
        .collect::<BTreeMap<_, _>>();
    let planned_blob_tombstones = planned_fallback_by_blob_index
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();

    let mut active_by_identity = BTreeMap::<(u8, u64, [u8; 32]), Vec<u32>>::new();
    for (blob_index, blob) in blobs.iter().enumerate() {
        let blob_index = u32::try_from(blob_index)
            .map_err(|_| "Binary DB blob index overflows u32.".to_string())?;
        if blob.is_tombstone() || blob.is_pruned() || planned_blob_tombstones.contains(&blob_index)
        {
            continue;
        }
        active_by_identity
            .entry((blob.hash_kind, blob.size_bytes, blob.sha256))
            .or_default()
            .push(blob_index);
    }

    let mut rewrites = Vec::new();
    for member_index in 0..member_count {
        if candidate_member_indices.contains(&member_index) {
            continue;
        }
        let raw = write
            .read_record(member_file.clone(), member_index)
            .map_err(|error| error.to_string())?;
        let member = BinaryObjectPackMemberCodec::<WRITE_LAYOUT>::decode_record(&raw)
            .map_err(|error| error.to_string())?;
        if member.is_tombstone() {
            continue;
        }
        let Some(base_blob_index) = member.base_blob_index() else {
            continue;
        };
        let base_blob = blobs.get(base_blob_index as usize).ok_or_else(|| {
            format!(
                "Live object-pack member {member_index} references missing base blob record {base_blob_index}."
            )
        })?;
        if !base_blob.is_tombstone()
            && !base_blob.is_pruned()
            && !planned_blob_tombstones.contains(&base_blob_index)
        {
            continue;
        }

        let identity = (base_blob.hash_kind, base_blob.size_bytes, base_blob.sha256);
        let fallback_blob_index = if let Some(fallback_blob_index) = planned_fallback_by_blob_index
            .get(&base_blob_index)
            .copied()
        {
            fallback_blob_index
        } else {
            let fallback_candidates = active_by_identity.get(&identity).ok_or_else(|| {
                format!(
                    "Live object-pack member {member_index} references unavailable base blob record {base_blob_index}, and no active blob with the same identity exists."
                )
            })?;
            let mut fallback = None;
            let mut last_error = None;
            for fallback_blob_index in fallback_candidates.iter().rev().copied() {
                let Some(fallback_member_index) =
                    blobs[fallback_blob_index as usize].pack_member_index()
                else {
                    last_error = Some(format!(
                        "Active fallback blob record {fallback_blob_index} has no member locator."
                    ));
                    continue;
                };
                match binary_db_validate_authoritative_member::<WRITE_LAYOUT, _>(
                    write,
                    fallback_member_index,
                    fallback_blob_index,
                    pack_count,
                    member_count,
                    &pack_file,
                    &member_file,
                ) {
                    Ok(()) => {
                        fallback = Some(fallback_blob_index);
                        break;
                    }
                    Err(error) => last_error = Some(error),
                }
            }
            fallback.ok_or_else(|| {
                format!(
                    "Live object-pack member {member_index} has no valid active fallback for base blob record {base_blob_index}: {}",
                    last_error.unwrap_or_else(|| "no fallback candidate".to_string())
                )
            })?
        };

        let fallback_blob = &blobs[fallback_blob_index as usize];
        if (
            fallback_blob.hash_kind,
            fallback_blob.size_bytes,
            fallback_blob.sha256,
        ) != identity
        {
            return Err(format!(
                "Base-pointer rewrite for member {member_index} changes blob identity from record {base_blob_index} to {fallback_blob_index}."
            ));
        }
        let fallback_member_index = fallback_blob.pack_member_index().ok_or_else(|| {
            format!("Fallback blob record {fallback_blob_index} has no member locator.")
        })?;
        binary_db_validate_authoritative_member::<WRITE_LAYOUT, _>(
            write,
            fallback_member_index,
            fallback_blob_index,
            pack_count,
            member_count,
            &pack_file,
            &member_file,
        )?;
        rewrites.push(BinaryDbBaseBlobPointerRewrite {
            member_index,
            record: member,
            fallback_blob_index,
        });
    }
    Ok(rewrites)
}

fn binary_db_verify_fallback_blob_content<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
    candidates: &[BinaryDbOrphanObjectPack],
    base_pointer_rewrites: &[BinaryDbBaseBlobPointerRewrite],
) -> Result<(), String> {
    let mut fallback_indices = candidates
        .iter()
        .flat_map(|candidate| candidate.verified_fallback_blob_indices.iter().copied())
        .collect::<BTreeSet<_>>();
    fallback_indices.extend(
        base_pointer_rewrites
            .iter()
            .map(|rewrite| rewrite.fallback_blob_index),
    );
    let read = content.blobs().begin_read_txn();
    for blob_index in fallback_indices {
        let view = content
            .blobs()
            .blob_view_at(&read, blob_index)
            .map_err(|error| error.to_string())?;
        if view.record.is_tombstone() || view.record.is_pruned() {
            return Err(format!(
                "Verified fallback blob {} became unavailable before content validation.",
                view.blob_id
            ));
        }
        content
            .blobs()
            .read_blob_bytes_for_view(&read, &view, 0)
            .map_err(|error| {
                format!(
                    "Refusing orphan-pack cleanup because fallback blob {} failed a normal Binary DB read: {error}",
                    view.blob_id
                )
            })?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "summary inputs map one-to-one to the stable public JSON fields"
)]
fn binary_db_prune_summary(
    removed_pack_ids: Vec<String>,
    removed_member_count: usize,
    removed_duplicate_blob_count: usize,
    verified_fallback_blob_count: usize,
    rewritten_base_blob_pointer_count: usize,
    removed_pack_paths: Vec<String>,
    already_missing_pack_paths: Vec<String>,
    cleanup_warnings: Vec<String>,
) -> JsonValue {
    json!({
        "storage_backend": "binary_db",
        "removed_orphan_pack_count": removed_pack_ids.len(),
        "removed_orphan_pack_member_count": removed_member_count,
        "removed_duplicate_blob_count": removed_duplicate_blob_count,
        "verified_fallback_blob_count": verified_fallback_blob_count,
        "rewritten_base_blob_pointer_count": rewritten_base_blob_pointer_count,
        "removed_orphan_pack_ids": removed_pack_ids,
        "removed_orphan_pack_paths": removed_pack_paths,
        "already_missing_orphan_pack_paths": already_missing_pack_paths,
        "cleanup_warnings": cleanup_warnings,
    })
}
