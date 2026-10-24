use super::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn blob_id(name: &str) -> String {
    format!(
        "BLB-{}",
        &format!("{:x}", Sha256::digest(name.as_bytes()))[..20]
    )
}

fn pack(number: u64, members: &[(&str, Option<&str>)]) -> (Value, Vec<Value>) {
    let mut entries = Vec::new();
    let mut locators = Vec::new();
    let id = format!("PCK-{number:012X}");
    for &(name, base) in members {
        let blob = blob_id(name);
        let checksum = format!("{:x}", Sha256::digest(name.as_bytes()));
        let kind = if base.is_some() { "delta" } else { "full" };
        let base = base.map(blob_id);
        entries.push(json!({"blob_id":blob,"checksum":checksum,"entry_type":kind,
            "base_blob_id":base,"chain_depth":u8::from(base.is_some()),
            "uncompressed_byte_length":name.len()}));
        locators.push(
            json!({"blob_id":blob,"sha256":checksum,"size_bytes":name.len(),
            "pack_id":id,"pack_entry_type":kind,"pack_base_blob_id":base,
            "pack_chain_depth":u8::from(base.is_some()),"created_at":"2026-01-01T00:00:00Z"}),
        );
    }
    (
        json!({"pack_id":id,"pack_format":"ait-pack-v3-zstd-chunked",
        "total_bytes":0,"created_at":"2026-01-01T00:00:00Z","pack_index":{"entries":entries}}),
        locators,
    )
}

fn append(db: &TestDb, packs: &[(Value, Vec<Value>)]) -> StoreResult<()> {
    let content = ServerBinaryRepositoryContentStore::new(db.clone());
    let mut tx = BinaryDbWriteTxn::begin_with_fsync_policy(
        db,
        BinaryDbCommandScope::ServerRemoteSyncCommit,
        BinaryDbNoopFsyncPolicy,
    )?;
    content.prepare_remote_sync_write_set(&mut tx, true, false, false, None)?;
    let batch = packs
        .iter()
        .map(|(pack, locators)| (pack, locators.as_slice()))
        .collect::<Vec<_>>();
    match content.append_object_packs_in_tx(&mut tx, &batch) {
        Ok(()) => tx.commit().map(|_| ()),
        Err(error) => {
            tx.abort()?;
            Err(error)
        }
    }
}

#[test]
fn later_and_chained_cross_pack_bases_keep_input_pack_order() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    append(
        &db,
        &[
            pack(1, &[("a", Some("b"))]),
            pack(2, &[("b", Some("c"))]),
            pack(3, &[("c", None)]),
        ],
    )?;
    let content = ServerBinaryRepositoryContentStore::new(db);
    let a = content.blob(&blob_id("a"))?.unwrap();
    let b = content.blob(&blob_id("b"))?.unwrap();
    assert_eq!(a.base_blob_id, Some(blob_id("b")));
    assert_eq!(b.base_blob_id, Some(blob_id("c")));
    assert_eq!(a.member.pack_index, 0);
    assert_eq!(b.member.pack_index, 1);
    assert_eq!(content.blob(&blob_id("c"))?.unwrap().member.pack_index, 2);
    fs::remove_dir_all(root).unwrap();
    Ok(())
}

#[test]
fn same_pack_and_existing_store_bases_remain_valid() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let existing = pack(1, &[("old", None)]);
    append(&db, std::slice::from_ref(&existing))?;
    append(
        &db,
        &[
            existing,
            pack(2, &[("a", Some("b")), ("b", None), ("c", Some("old"))]),
        ],
    )?;
    let content = ServerBinaryRepositoryContentStore::new(db);
    assert_eq!(
        content.blob(&blob_id("a"))?.unwrap().base_blob_id,
        Some(blob_id("b"))
    );
    assert_eq!(
        content.blob(&blob_id("c"))?.unwrap().base_blob_id,
        Some(blob_id("old"))
    );
    assert_eq!(content.blob(&blob_id("c"))?.unwrap().member.pack_index, 1);
    fs::remove_dir_all(root).unwrap();
    Ok(())
}

#[test]
fn mutually_dependent_packs_with_full_roots_are_not_blob_cycles() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    append(
        &db,
        &[
            pack(1, &[("a", None), ("c", Some("b"))]),
            pack(2, &[("b", None), ("d", Some("a"))]),
        ],
    )?;
    let content = ServerBinaryRepositoryContentStore::new(db);
    assert_eq!(
        content.blob(&blob_id("c"))?.unwrap().base_blob_id,
        Some(blob_id("b"))
    );
    assert_eq!(
        content.blob(&blob_id("d"))?.unwrap().base_blob_id,
        Some(blob_id("a"))
    );
    fs::remove_dir_all(root).unwrap();
    Ok(())
}

#[test]
fn a_later_full_copy_does_not_hide_a_cyclic_canonical_blob() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let error = append(
        &db,
        &[
            pack(1, &[("a", Some("b"))]),
            pack(2, &[("b", Some("a"))]),
            pack(3, &[("a", None)]),
        ],
    )
    .unwrap_err();
    assert!(error.to_string().contains("cyclic delta base"));
    let content = ServerBinaryRepositoryContentStore::new(db);
    assert!(content.blob(&blob_id("a"))?.is_none());
    fs::remove_dir_all(root).unwrap();
    Ok(())
}

#[test]
fn duplicate_delta_content_does_not_read_unstaged_forward_bases() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    append(
        &db,
        &[
            pack(1, &[("a", Some("b"))]),
            pack(2, &[("a", None)]),
            pack(3, &[("b", None)]),
        ],
    )?;
    let content = ServerBinaryRepositoryContentStore::new(db);
    assert_eq!(
        content.blob(&blob_id("a"))?.unwrap().base_blob_id,
        Some(blob_id("b"))
    );
    fs::remove_dir_all(root).unwrap();
    Ok(())
}

#[test]
fn absent_bases_and_real_blob_cycles_leave_no_content() -> StoreResult<()> {
    for (packs, message) in [
        (
            vec![
                pack(1, &[("root", None)]),
                pack(2, &[("a", Some("missing"))]),
            ],
            "delta base blob",
        ),
        (
            vec![pack(1, &[("a", Some("b"))]), pack(2, &[("b", Some("a"))])],
            "cyclic delta base",
        ),
        (
            vec![pack(1, &[("a", Some("b")), ("b", Some("a"))])],
            "cyclic delta base",
        ),
    ] {
        let root = temporary_root();
        let db = test_db(&root);
        assert!(append(&db, &packs)
            .unwrap_err()
            .to_string()
            .contains(message));
        let content = ServerBinaryRepositoryContentStore::new(db);
        assert!(content.object_pack("PCK-000000000001")?.is_none());
        assert!(content.blob(&blob_id("root"))?.is_none());
        assert!(content.blob(&blob_id("a"))?.is_none());
        fs::remove_dir_all(root).unwrap();
    }
    Ok(())
}

#[test]
fn batch_preserves_existing_blob_immutable_content_checks() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    append(&db, &[pack(1, &[("a", None)])])?;
    let (mut altered, mut locators) = pack(2, &[("a", None)]);
    let mut checksum = locators[0]["sha256"].as_str().unwrap().to_string();
    let last = checksum.pop().unwrap();
    checksum.push(if last == '0' { '1' } else { '0' });
    locators[0]["sha256"] = json!(checksum);
    altered["pack_index"]["entries"][0]["checksum"] = json!(checksum);
    assert!(append(&db, &[(altered, locators)])
        .unwrap_err()
        .to_string()
        .contains("different content"));
    let content = ServerBinaryRepositoryContentStore::new(db);
    assert!(content.object_pack("PCK-000000000002")?.is_none());
    assert_eq!(content.blob(&blob_id("a"))?.unwrap().member.pack_index, 0);
    fs::remove_dir_all(root).unwrap();
    Ok(())
}
