use super::*;
use crate::binary_db::{
    AuthorityId, BinaryDbCommandScope, BinaryDbNoopFsyncPolicy, LocalStateScope,
};
use crate::content_binary_db::{
    blob_id_from_sha256, object_pack_id_from_hash48, BinaryBlobRecord,
    BinaryObjectPackMemberRecord, BinaryObjectPackRecord,
};
use crate::pack_substrate::{
    default_object_pack_relative_path, write_pack_archive_with_format, PACK_FORMAT_ZSTD_CHUNKED_V1,
};
use sha2::{Digest, Sha256};
use std::fs;
use tempfile::TempDir;

const TEST_LAYOUT: u32 = 1;

fn split_hash48(value: u64) -> (u16, u32) {
    ((value >> 32) as u16, value as u32)
}

fn local_content(temp: &TempDir) -> LocalContentBinaryDb<TEST_LAYOUT> {
    LocalContentBinaryDb::new(
        temp.path().join(".ait/binary-db"),
        temp.path(),
        AuthorityId::new("local-content-gc-test"),
        LocalStateScope::Repository,
    )
}

fn write_pack_placeholder(temp: &TempDir, pack_id: &str) -> String {
    let relative = default_object_pack_relative_path(pack_id);
    let absolute = temp.path().join(&relative);
    fs::create_dir_all(absolute.parent().unwrap()).unwrap();
    fs::write(&absolute, b"test pack placeholder").unwrap();
    relative
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

#[test]
fn binary_db_default_stats_output_is_bounded_across_pack_inventory_rows() {
    let temp = TempDir::new().unwrap();
    let content = local_content(&temp);
    let mut write = content
        .blobs()
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .unwrap();
    for ordinal in 0..128_u64 {
        let pack_hash = 0x0100_0000_0000 + ordinal;
        let pack_id = object_pack_id_from_hash48(pack_hash);
        write_pack_placeholder(&temp, &pack_id);
        let (pack_hash_hi16, pack_hash_lo32) = split_hash48(pack_hash);
        content
            .object_packs()
            .append_object_pack_with_id_index(
                &mut write,
                &BinaryObjectPackRecord {
                    pack_meta: BinaryObjectPackRecord::META_READY,
                    pack_format_kind: 1,
                    pack_hash_hi16,
                    pack_hash_lo32,
                    first_member_index: 0,
                    member_count: 0,
                    total_bytes: 0,
                    created_at_s: ordinal,
                },
            )
            .unwrap();
    }
    write.commit().unwrap();

    let stats = content.storage_stats().unwrap();
    assert_eq!(stats["pack_count"], 128);
    assert!(stats.get("inventory_included").is_none());
    assert!(stats.get("packs").is_none());
    assert!(stats.get("tree_packs").is_none());
    assert!(stats.to_string().len() < 10_000);
}

fn write_readable_full_pack(temp: &TempDir, pack_id: &str, blob_id: &str, data: &[u8]) -> String {
    let relative = default_object_pack_relative_path(pack_id);
    let absolute = temp.path().join(&relative);
    fs::create_dir_all(absolute.parent().unwrap()).unwrap();
    write_pack_archive_with_format(
        absolute.to_string_lossy().as_ref(),
        pack_id,
        "2026-07-15T00:00:00Z",
        &json!([{
            "entry_name": format!("blobs/{blob_id}"),
            "blob_id": blob_id,
            "data": data,
        }]),
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    relative
}

#[test]
fn binary_db_prune_revalidation_rejects_catalog_change_before_mutation() {
    let temp = TempDir::new().unwrap();
    let content = local_content(&temp);
    let plan = binary_db_verified_orphan_pack_prune_plan(&content).unwrap();

    let pack_hash = 0x7172_7374_7576;
    let pack_id = object_pack_id_from_hash48(pack_hash);
    let pack_path = write_pack_placeholder(&temp, &pack_id);
    let (pack_hi, pack_lo) = split_hash48(pack_hash);
    let mut mutation = content
        .object_packs()
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .unwrap();
    content
        .object_packs()
        .append_object_pack_with_id_index(
            &mut mutation,
            &BinaryObjectPackRecord {
                pack_meta: BinaryObjectPackRecord::META_READY,
                pack_format_kind: 1,
                pack_hash_hi16: pack_hi,
                pack_hash_lo32: pack_lo,
                first_member_index: 0,
                member_count: 0,
                total_bytes: 0,
                created_at_s: 1,
            },
        )
        .unwrap();
    mutation.commit().unwrap();

    let mut revalidation = content
        .object_packs()
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .unwrap();
    let error =
        binary_db_require_unchanged_orphan_pack_prune_plan::<TEST_LAYOUT, _>(&revalidation, &plan)
            .unwrap_err();
    revalidation.abort().unwrap();
    assert!(error.contains("content catalog changed"));
    assert!(temp.path().join(pack_path).is_file());
    let read = content.object_packs().begin_read_txn();
    assert!(!content
        .object_packs()
        .read_object_pack_record(&read, 0)
        .unwrap()
        .is_tombstone());
}

#[test]
fn binary_db_prune_removes_only_fully_unreferenced_object_pack() {
    let temp = TempDir::new().unwrap();
    let content = local_content(&temp);
    let valid_hash = 0x0102_0304_0506;
    let orphan_hash = 0x1112_1314_1516;
    let valid_id = object_pack_id_from_hash48(valid_hash);
    let orphan_id = object_pack_id_from_hash48(orphan_hash);
    let blob_bytes = b"canonical fallback blob\n";
    let blob_sha = sha256(blob_bytes);
    let blob_id = blob_id_from_sha256(&blob_sha);
    let valid_path = write_readable_full_pack(&temp, &valid_id, &blob_id, blob_bytes);
    let orphan_path = write_pack_placeholder(&temp, &orphan_id);
    let (valid_hi, valid_lo) = split_hash48(valid_hash);
    let (orphan_hi, orphan_lo) = split_hash48(orphan_hash);

    let mut write = content
        .blobs()
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .unwrap();
    content
        .object_packs()
        .append_object_pack_with_id_index(
            &mut write,
            &BinaryObjectPackRecord {
                pack_meta: BinaryObjectPackRecord::META_READY,
                pack_format_kind: 1,
                pack_hash_hi16: valid_hi,
                pack_hash_lo32: valid_lo,
                first_member_index: 0,
                member_count: 1,
                total_bytes: blob_bytes.len() as u64,
                created_at_s: 1,
            },
        )
        .unwrap();
    content
        .object_packs()
        .append_object_pack_with_id_index(
            &mut write,
            &BinaryObjectPackRecord {
                pack_meta: BinaryObjectPackRecord::META_READY,
                pack_format_kind: 1,
                pack_hash_hi16: orphan_hi,
                pack_hash_lo32: orphan_lo,
                first_member_index: 1,
                member_count: 1,
                total_bytes: blob_bytes.len() as u64,
                created_at_s: 1,
            },
        )
        .unwrap();
    content
        .blobs()
        .append_blob_with_id_index(
            &mut write,
            &BinaryBlobRecord {
                blob_meta: BinaryBlobRecord::META_HAS_PACK_MEMBER,
                hash_kind: 1,
                reserved0: 0,
                size_bytes: blob_bytes.len() as u64,
                pack_member_index_plus1: 1,
                created_at_s: 1,
                pruned_at_s: 0,
                sha256: blob_sha,
            },
        )
        .unwrap();
    content
        .object_packs()
        .append_object_pack_member_record(
            &mut write,
            &BinaryObjectPackMemberRecord {
                member_meta: 0,
                delta_chain_depth: 0,
                reserved0: 0,
                pack_index: 0,
                blob_index: 0,
                base_blob_index_plus1: 0,
            },
        )
        .unwrap();
    content
        .object_packs()
        .append_object_pack_member_record(
            &mut write,
            &BinaryObjectPackMemberRecord {
                member_meta: 0,
                delta_chain_depth: 0,
                reserved0: 0,
                pack_index: 1,
                blob_index: 0,
                base_blob_index_plus1: 0,
            },
        )
        .unwrap();
    write.commit().unwrap();

    let preview = content.preview_orphan_pack_prune().unwrap();
    assert_eq!(preview["mode"], "preview");
    assert_eq!(preview["applied"], false);
    assert_eq!(preview["candidate_orphan_pack_count"], 1);
    assert_eq!(preview["candidate_orphan_pack_member_count"], 1);
    assert_eq!(preview["candidate_orphan_pack_ids"][0], orphan_id);
    assert_eq!(preview["candidate_orphan_pack_paths"][0], orphan_path);
    assert!(temp.path().join(&valid_path).is_file());
    assert!(temp.path().join(&orphan_path).is_file());
    let preview_read = content.object_packs().begin_read_txn();
    assert!(!content
        .object_packs()
        .read_object_pack_record(&preview_read, 1)
        .unwrap()
        .is_tombstone());
    assert!(!content
        .object_packs()
        .read_object_pack_member_record(&preview_read, 1)
        .unwrap()
        .is_tombstone());
    drop(preview_read);

    let result = content.prune_orphan_packs().unwrap();
    assert_eq!(result["mode"], "apply");
    assert_eq!(result["applied"], true);
    assert_eq!(
        result["candidate_orphan_pack_ids"],
        preview["candidate_orphan_pack_ids"]
    );
    assert_eq!(
        result["candidate_orphan_pack_paths"],
        preview["candidate_orphan_pack_paths"]
    );
    assert_eq!(result["removed_orphan_pack_count"], 1);
    assert_eq!(result["removed_orphan_pack_member_count"], 1);
    assert_eq!(result["removed_orphan_pack_ids"][0], orphan_id);
    assert!(temp.path().join(valid_path).is_file());
    assert!(!temp.path().join(orphan_path).exists());

    let read = content.object_packs().begin_read_txn();
    assert!(content
        .object_packs()
        .get_object_pack_view(&read, &valid_id)
        .unwrap()
        .is_some());
    assert!(content
        .object_packs()
        .get_object_pack_view(&read, &orphan_id)
        .unwrap()
        .is_none());
    assert!(content
        .object_packs()
        .read_object_pack_record(&read, 1)
        .unwrap()
        .is_tombstone());
    assert!(content
        .object_packs()
        .read_object_pack_member_record(&read, 1)
        .unwrap()
        .is_tombstone());
    drop(read);

    let second = content.prune_orphan_packs().unwrap();
    assert_eq!(second["mode"], "apply");
    assert_eq!(second["applied"], true);
    assert_eq!(second["removed_orphan_pack_count"], 0);
}

#[test]
fn binary_db_prune_restores_older_blob_before_removing_duplicate_repair_pack() {
    let temp = TempDir::new().unwrap();
    let content = local_content(&temp);
    let original_hash = 0x4142_4344_4546;
    let repair_hash = 0x5152_5354_5556;
    let dependent_hash = 0x6162_6364_6566;
    let original_id = object_pack_id_from_hash48(original_hash);
    let repair_id = object_pack_id_from_hash48(repair_hash);
    let dependent_id = object_pack_id_from_hash48(dependent_hash);
    let blob_bytes = b"historical blame content\n";
    let blob_sha = sha256(blob_bytes);
    let blob_id = blob_id_from_sha256(&blob_sha);
    let dependent_bytes = b"live delta depending on historical blame content\n";
    let dependent_sha = sha256(dependent_bytes);
    let original_path = write_readable_full_pack(&temp, &original_id, &blob_id, blob_bytes);
    let repair_path = write_pack_placeholder(&temp, &repair_id);
    let dependent_path = write_pack_placeholder(&temp, &dependent_id);
    let (original_hi, original_lo) = split_hash48(original_hash);
    let (repair_hi, repair_lo) = split_hash48(repair_hash);
    let (dependent_hi, dependent_lo) = split_hash48(dependent_hash);

    let mut write = content
        .blobs()
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .unwrap();
    for (pack_hash_hi16, pack_hash_lo32, first_member_index, total_bytes) in [
        (original_hi, original_lo, 0, blob_bytes.len() as u64),
        (repair_hi, repair_lo, 1, blob_bytes.len() as u64),
        (dependent_hi, dependent_lo, 2, dependent_bytes.len() as u64),
    ] {
        content
            .object_packs()
            .append_object_pack_with_id_index(
                &mut write,
                &BinaryObjectPackRecord {
                    pack_meta: BinaryObjectPackRecord::META_READY,
                    pack_format_kind: 1,
                    pack_hash_hi16,
                    pack_hash_lo32,
                    first_member_index,
                    member_count: 1,
                    total_bytes,
                    created_at_s: 1,
                },
            )
            .unwrap();
    }
    for pack_member_index_plus1 in [1, 2] {
        content
            .blobs()
            .append_blob_with_id_index(
                &mut write,
                &BinaryBlobRecord {
                    blob_meta: BinaryBlobRecord::META_HAS_PACK_MEMBER,
                    hash_kind: 1,
                    reserved0: 0,
                    size_bytes: blob_bytes.len() as u64,
                    pack_member_index_plus1,
                    created_at_s: 1,
                    pruned_at_s: 0,
                    sha256: blob_sha,
                },
            )
            .unwrap();
    }
    content
        .blobs()
        .append_blob_with_id_index(
            &mut write,
            &BinaryBlobRecord {
                blob_meta: BinaryBlobRecord::META_HAS_PACK_MEMBER,
                hash_kind: 1,
                reserved0: 0,
                size_bytes: dependent_bytes.len() as u64,
                pack_member_index_plus1: 3,
                created_at_s: 1,
                pruned_at_s: 0,
                sha256: dependent_sha,
            },
        )
        .unwrap();
    for (pack_index, blob_index, member_meta, delta_chain_depth, base_blob_index_plus1) in [
        (0, 0, 0, 0, 0),
        (1, 1, 0, 0, 0),
        // This live member models metadata captured while the duplicate repair
        // blob was authoritative. Prune must retarget the pointer before
        // tombstoning that duplicate record.
        (2, 2, 1, 1, 2),
    ] {
        content
            .object_packs()
            .append_object_pack_member_record(
                &mut write,
                &BinaryObjectPackMemberRecord {
                    member_meta,
                    delta_chain_depth,
                    reserved0: 0,
                    pack_index,
                    blob_index,
                    base_blob_index_plus1,
                },
            )
            .unwrap();
    }
    write.commit().unwrap();

    let result = content.prune_orphan_packs().unwrap();
    assert_eq!(result["removed_orphan_pack_count"], 1);
    assert_eq!(result["removed_duplicate_blob_count"], 1);
    assert_eq!(result["verified_fallback_blob_count"], 1);
    assert_eq!(result["rewritten_base_blob_pointer_count"], 1);
    assert_eq!(result["removed_orphan_pack_ids"][0], repair_id);
    assert!(temp.path().join(original_path).is_file());
    assert!(!temp.path().join(repair_path).exists());
    assert!(temp.path().join(dependent_path).is_file());

    let read = content.blobs().begin_read_txn();
    let restored = content
        .blobs()
        .get_blob_view(&read, &blob_id)
        .unwrap()
        .unwrap();
    assert_eq!(restored.blob_index, 0);
    assert_eq!(
        content
            .blobs()
            .read_blob_bytes_for_id(&read, &blob_id)
            .unwrap(),
        Some(blob_bytes.to_vec())
    );
    assert!(content
        .blobs()
        .blob_view_at(&read, 1)
        .unwrap()
        .record
        .is_tombstone());
    assert!(content
        .object_packs()
        .read_object_pack_record(&read, 1)
        .unwrap()
        .is_tombstone());
    assert!(content
        .object_packs()
        .read_object_pack_member_record(&read, 1)
        .unwrap()
        .is_tombstone());
    let dependent_member = content
        .object_packs()
        .read_object_pack_member_record(&read, 2)
        .unwrap();
    assert!(!dependent_member.is_tombstone());
    assert_eq!(dependent_member.base_blob_index(), Some(0));
    drop(read);

    let second = content.prune_orphan_packs().unwrap();
    assert_eq!(second["removed_orphan_pack_count"], 0);
    assert_eq!(second["rewritten_base_blob_pointer_count"], 0);
}

#[test]
fn binary_db_prune_rejects_partially_referenced_pack_without_mutation() {
    let temp = TempDir::new().unwrap();
    let content = local_content(&temp);
    let pack_hash = 0x2122_2324_2526;
    let pack_id = object_pack_id_from_hash48(pack_hash);
    let pack_path = write_pack_placeholder(&temp, &pack_id);
    let (pack_hi, pack_lo) = split_hash48(pack_hash);

    let mut write = content
        .blobs()
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .unwrap();
    content
        .object_packs()
        .append_object_pack_with_id_index(
            &mut write,
            &BinaryObjectPackRecord {
                pack_meta: BinaryObjectPackRecord::META_READY,
                pack_format_kind: 1,
                pack_hash_hi16: pack_hi,
                pack_hash_lo32: pack_lo,
                first_member_index: 0,
                member_count: 2,
                total_bytes: 2,
                created_at_s: 1,
            },
        )
        .unwrap();
    content
        .blobs()
        .append_blob_with_id_index(
            &mut write,
            &BinaryBlobRecord {
                blob_meta: BinaryBlobRecord::META_HAS_PACK_MEMBER,
                hash_kind: 1,
                reserved0: 0,
                size_bytes: 1,
                pack_member_index_plus1: 1,
                created_at_s: 1,
                pruned_at_s: 0,
                sha256: [0x24; 32],
            },
        )
        .unwrap();
    for _ in 0..2 {
        content
            .object_packs()
            .append_object_pack_member_record(
                &mut write,
                &BinaryObjectPackMemberRecord {
                    member_meta: 0,
                    delta_chain_depth: 0,
                    reserved0: 0,
                    pack_index: 0,
                    blob_index: 0,
                    base_blob_index_plus1: 0,
                },
            )
            .unwrap();
    }
    write.commit().unwrap();

    let error = content.prune_orphan_packs().unwrap_err();
    assert!(error.contains("only partially referenced: 1 of 2"));
    assert!(temp.path().join(pack_path).is_file());
    let read = content.object_packs().begin_read_txn();
    assert!(!content
        .object_packs()
        .read_object_pack_record(&read, 0)
        .unwrap()
        .is_tombstone());
    assert!(!content
        .object_packs()
        .read_object_pack_member_record(&read, 0)
        .unwrap()
        .is_tombstone());
    assert!(!content
        .object_packs()
        .read_object_pack_member_record(&read, 1)
        .unwrap()
        .is_tombstone());
}

#[test]
fn binary_db_prune_preserves_locatorless_pack_when_it_is_the_only_live_copy() {
    let temp = TempDir::new().unwrap();
    let content = local_content(&temp);
    let pack_hash = 0x3132_3334_3536;
    let pack_id = object_pack_id_from_hash48(pack_hash);
    let pack_path = write_pack_placeholder(&temp, &pack_id);
    let (pack_hi, pack_lo) = split_hash48(pack_hash);

    let mut write = content
        .blobs()
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .unwrap();
    content
        .object_packs()
        .append_object_pack_with_id_index(
            &mut write,
            &BinaryObjectPackRecord {
                pack_meta: BinaryObjectPackRecord::META_READY,
                pack_format_kind: 1,
                pack_hash_hi16: pack_hi,
                pack_hash_lo32: pack_lo,
                first_member_index: 0,
                member_count: 1,
                total_bytes: 1,
                created_at_s: 1,
            },
        )
        .unwrap();
    content
        .blobs()
        .append_blob_with_id_index(
            &mut write,
            &BinaryBlobRecord {
                blob_meta: 0,
                hash_kind: 1,
                reserved0: 0,
                size_bytes: 1,
                pack_member_index_plus1: 0,
                created_at_s: 1,
                pruned_at_s: 0,
                sha256: [0x35; 32],
            },
        )
        .unwrap();
    content
        .object_packs()
        .append_object_pack_member_record(
            &mut write,
            &BinaryObjectPackMemberRecord {
                member_meta: 0,
                delta_chain_depth: 0,
                reserved0: 0,
                pack_index: 0,
                blob_index: 0,
                base_blob_index_plus1: 0,
            },
        )
        .unwrap();
    write.commit().unwrap();

    let error = content.prune_orphan_packs().unwrap_err();
    assert!(error.contains("refusing to delete recoverable history"));
    assert!(temp.path().join(pack_path).is_file());
    let read = content.object_packs().begin_read_txn();
    assert!(!content
        .object_packs()
        .read_object_pack_record(&read, 0)
        .unwrap()
        .is_tombstone());
    assert!(!content
        .object_packs()
        .read_object_pack_member_record(&read, 0)
        .unwrap()
        .is_tombstone());
}
