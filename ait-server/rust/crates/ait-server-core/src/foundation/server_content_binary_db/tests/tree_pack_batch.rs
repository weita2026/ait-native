use super::*;
use crate::foundation::pack_substrate::{build_tree_pack_members, write_tree_pack_archive};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn id(name: &str) -> String {
    format!(
        "TRE-{}",
        &format!("{:X}", Sha256::digest(name.as_bytes()))[..20]
    )
}
fn child(name: &str, target: &str) -> Value {
    json!({"entry_name":name,"entry_type":"tree","target_id":id(target),"mode":"tree","size_bytes":null})
}

fn pack(db: &TestDb, number: u64, trees: &[(&str, Vec<Value>)]) -> (Value, Vec<Value>) {
    let content = ServerBinaryRepositoryContentStore::new(db.clone());
    let pack_id = format!("TPK-{number:012X}");
    let declarations = trees
        .iter()
        .map(|(name, rows)| json!({"tree_id":id(name),"entry_count":rows.len()}))
        .collect::<Vec<_>>();
    let rows = trees
        .iter()
        .flat_map(|(name, rows)| {
            rows.iter().map(move |row| {
                let mut row = row.clone();
                row["tree_id"] = json!(id(name));
                row
            })
        })
        .collect::<Vec<_>>();
    let members = build_tree_pack_members(&json!(declarations), &json!(rows)).unwrap();
    let path = content.tree_pack_path(&pack_id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut metadata = write_tree_pack_archive(
        path.to_str().unwrap(),
        &pack_id,
        "2026-01-01T00:00:00Z",
        &members,
    )
    .unwrap();
    metadata["pack_id"] = json!(pack_id);
    let locators = metadata["pack_index"]["trees"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tree| {
            json!({
                "tree_id":tree["tree_id"],"entry_count":tree["entry_count"],"tree_pack_id":pack_id,
                "tree_pack_checksum":tree["checksum"],"created_at":"2026-01-01T00:00:00Z"
            })
        })
        .collect();
    (metadata, locators)
}

fn append(db: &TestDb, packs: &[(Value, Vec<Value>)]) -> StoreResult<()> {
    let content = ServerBinaryRepositoryContentStore::new(db.clone());
    let mut tx = BinaryDbWriteTxn::begin_with_fsync_policy(
        db,
        BinaryDbCommandScope::ServerRemoteSyncCommit,
        BinaryDbNoopFsyncPolicy,
    )?;
    content.prepare_remote_sync_write_set(&mut tx, false, true, false, None)?;
    let batch = packs
        .iter()
        .map(|(pack, locators)| (pack, locators.as_slice()))
        .collect::<Vec<_>>();
    match content.append_tree_packs_in_tx(&mut tx, &batch) {
        Ok(()) => tx.commit().map(|_| ()),
        Err(error) => {
            tx.abort()?;
            Err(error)
        }
    }
}

fn targets(db: &TestDb, tree: &str) -> StoreResult<Vec<String>> {
    let content = ServerBinaryRepositoryContentStore::new(db.clone());
    let read = BinaryDbReadTxn::new(db);
    let view = content.tree(tree)?.unwrap();
    Ok(content
        .tree_entries_for_tree_with_read(&read, &view)?
        .into_iter()
        .map(|row| row.target_id)
        .collect())
}

#[test]
fn later_nested_tree_packs_preserve_pack_order_and_ranges() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let packs = [
        pack(&db, 1, &[("root", vec![child("nested", "middle")])]),
        pack(&db, 2, &[("middle", vec![child("leaf", "leaf")])]),
        pack(&db, 3, &[("leaf", vec![])]),
    ];
    append(&db, &packs)?;
    assert_eq!(targets(&db, &id("root"))?, vec![id("middle")]);
    assert_eq!(targets(&db, &id("middle"))?, vec![id("leaf")]);
    let content = ServerBinaryRepositoryContentStore::new(db.clone());
    assert_eq!(content.tree(&id("root"))?.unwrap().tree_index, 0);
    assert_eq!(content.tree(&id("leaf"))?.unwrap().tree_index, 2);
    fs::remove_dir_all(root).unwrap();
    Ok(())
}

#[test]
fn mutually_dependent_pack_groups_are_a_valid_tree_dag() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let packs = [
        pack(
            &db,
            1,
            &[("root", vec![child("later", "middle")]), ("leaf", vec![])],
        ),
        pack(&db, 2, &[("middle", vec![child("back", "leaf")])]),
    ];
    append(&db, &packs)?;
    assert_eq!(targets(&db, &id("root"))?, vec![id("middle")]);
    assert_eq!(targets(&db, &id("middle"))?, vec![id("leaf")]);
    fs::remove_dir_all(root).unwrap();
    Ok(())
}

#[test]
fn same_pack_and_committed_tree_children_remain_valid() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let stored = pack(&db, 1, &[("stored", vec![])]);
    append(&db, std::slice::from_ref(&stored))?;
    let next = pack(
        &db,
        2,
        &[
            (
                "root",
                vec![child("local", "local"), child("stored", "stored")],
            ),
            ("local", vec![]),
        ],
    );
    append(&db, &[stored, next])?;
    assert_eq!(targets(&db, &id("root"))?, vec![id("local"), id("stored")]);
    fs::remove_dir_all(root).unwrap();
    Ok(())
}

#[test]
fn duplicate_tree_payloads_do_not_read_partial_forward_references() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let rows = vec![child("later", "leaf")];
    let packs = [
        pack(&db, 1, &[("root", rows.clone())]),
        pack(&db, 2, &[("root", rows)]),
        pack(&db, 3, &[("leaf", vec![])]),
    ];
    append(&db, &packs)?;
    assert_eq!(targets(&db, &id("root"))?, vec![id("leaf")]);
    fs::remove_dir_all(root).unwrap();
    Ok(())
}

#[test]
fn sparse_physical_tree_ordinals_are_not_rewritten() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let (mut metadata, locators) = pack(
        &db,
        1,
        &[
            ("unused", vec![]),
            ("leaf", vec![]),
            ("root", vec![child("leaf", "leaf")]),
        ],
    );
    let physical = metadata["pack_index"]["trees"].as_array().unwrap().clone();
    let root_entry = physical
        .iter()
        .find(|row| row["tree_id"] == id("root"))
        .unwrap()
        .clone();
    let leaf_entry = physical
        .iter()
        .find(|row| row["tree_id"] == id("leaf"))
        .unwrap()
        .clone();
    metadata["pack_index"]["trees"] = json!([root_entry, leaf_entry]);
    let locators = locators
        .into_iter()
        .filter(|row| row["tree_id"] != id("unused"))
        .collect();
    append(&db, &[(metadata, locators)])?;
    let content = ServerBinaryRepositoryContentStore::new(db.clone());
    assert_eq!(
        content
            .tree(&id("root"))?
            .unwrap()
            .record
            .pack_entry_ordinal as u64,
        root_entry["entry_ordinal"].as_u64().unwrap()
    );
    assert_eq!(targets(&db, &id("root"))?, vec![id("leaf")]);
    fs::remove_dir_all(root).unwrap();
    Ok(())
}

#[test]
fn missing_and_cyclic_tree_references_publish_nothing() -> StoreResult<()> {
    for cycle in [false, true] {
        let root = temporary_root();
        let db = test_db(&root);
        let mut packs = vec![pack(&db, 1, &[("root", vec![child("child", "child")])])];
        if cycle {
            packs.push(pack(&db, 2, &[("child", vec![child("back", "root")])]))
        }
        let error = append(&db, &packs).unwrap_err().to_string();
        assert!(
            error.contains(if cycle {
                "cyclic Tree"
            } else {
                "is missing for tree entry"
            }),
            "{error}"
        );
        let content = ServerBinaryRepositoryContentStore::new(db.clone());
        assert!(content.tree(&id("root"))?.is_none());
        assert!(content.tree_pack("TPK-000000000001")?.is_none());
        fs::remove_dir_all(root).unwrap();
    }
    Ok(())
}

#[test]
fn conflicting_incoming_or_stored_tree_content_is_rejected() -> StoreResult<()> {
    for stored in [false, true] {
        let root = temporary_root();
        let db = test_db(&root);
        let first = pack(&db, 1, &[("root", vec![])]);
        if stored {
            append(&db, std::slice::from_ref(&first))?;
        }
        let conflict = pack(
            &db,
            2,
            &[("root", vec![child("child", "leaf")]), ("leaf", vec![])],
        );
        let packs = if stored {
            vec![conflict]
        } else {
            vec![first, conflict]
        };
        let error = append(&db, &packs).unwrap_err().to_string();
        assert!(error.contains("already exists"), "{error}");
        let content = ServerBinaryRepositoryContentStore::new(db.clone());
        assert!(content.tree_pack("TPK-000000000002")?.is_none());
        if stored {
            assert!(targets(&db, &id("root"))?.is_empty());
        } else {
            assert!(content.tree(&id("root"))?.is_none());
        }
        fs::remove_dir_all(root).unwrap();
    }
    Ok(())
}
