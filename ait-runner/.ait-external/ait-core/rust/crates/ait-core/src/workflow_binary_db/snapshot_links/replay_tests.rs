use super::super::tests::{fixture, seed_line_and_snapshots as seed_workflow_snapshots};
use super::*;
use crate::binary_db::LocalBinaryDbFs;
use crate::content_binary_db::*;
use tempfile::TempDir;

type Workflow = BinaryDbWorkflowStore<LocalBinaryDbFs, 1>;

fn seed_line_and_snapshots(store: &Workflow) -> (String, String) {
    let ids = seed_workflow_snapshots(store);
    let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
    for (index, id) in [&ids.0, &ids.1].into_iter().enumerate() {
        tx.append_index_candidate(
            BinarySnapshotCodec::<1>::id_index(),
            &snapshot_id_index_key(id).unwrap(),
            index as u32,
        )
        .unwrap();
    }
    tx.commit().unwrap();
    ids
}

fn source_owner(
    store: &Workflow,
    base: &str,
    source: &str,
    change: bool,
) -> SnapshotWorkflowBinding {
    let task = TaskStore::create_task(
        store,
        "fixture",
        "Replay",
        "Preserve completed ownership",
        Some("C"),
        None,
        None,
        None,
    )
    .unwrap();
    let task_id = task["task_id"].as_str().unwrap().to_string();
    if change {
        ChangeStore::create_change(
            store,
            &task_id,
            "fixture",
            "Revision",
            "main",
            Some("C"),
            Some(base),
        )
        .unwrap();
    }
    let binding = SnapshotWorkflowBinding {
        task_id,
        change_id: if change { "C-01".into() } else { String::new() },
        line_name: "main".into(),
    };
    let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
    store
        .record_snapshot_link_in_write(&mut tx, &binding, source, 1)
        .unwrap();
    tx.commit().unwrap();
    binding
}

fn finish(store: &Workflow, owner: &SnapshotWorkflowBinding, source: &str, base: &str) {
    let change =
        (!owner.change_id.is_empty()).then(|| format!("{}/{}", owner.task_id, owner.change_id));
    store
        .finish_local_task(
            &owner.task_id,
            source,
            "main",
            Some(base),
            change.as_deref(),
            None,
        )
        .unwrap();
}

fn append_checkpoint(store: &Workflow, id: &str, parent: &str) {
    let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
    let file = BinarySnapshotCodec::<1>::record_file();
    let mut record =
        BinarySnapshotCodec::<1>::decode_record(&tx.read_record(file.clone(), 1).unwrap()).unwrap();
    record.snapshot_hash48 = snapshot_hash48_from_id(id).unwrap();
    record.parent_snapshot_index_plus1 = snapshot_index_by_id(&tx, parent).unwrap().unwrap() + 1;
    let index = tx
        .append_record(
            file,
            &BinarySnapshotCodec::<1>::encode_record(&record).unwrap(),
        )
        .unwrap();
    tx.append_index_candidate(
        BinarySnapshotCodec::<1>::id_index(),
        &snapshot_id_index_key(id).unwrap(),
        index,
    )
    .unwrap();
    tx.commit().unwrap();
}

fn input(source: &str, id: &str, parent: &str) -> BinaryDbSnapshotReplayWriteInput {
    BinaryDbSnapshotReplayWriteInput {
        source_snapshot_id: source.into(),
        snapshot: BinaryDbSnapshotWriteInput {
            snapshot_id: id.into(),
            parent_snapshot_ids: vec![parent.into()],
            root_tree_pack_id: tree_pack_id_from_hash48(1),
            root_entry_ordinal: 0,
            manifest_hash: "01".repeat(32),
            message: Some("Rebased history".into()),
            line_name: "main".into(),
            snapshot_kind: "line".into(),
            file_count: 0,
            total_bytes: 0,
            created_at: "2026-10-20T00:00:00Z".into(),
        },
    }
}

fn replay(store: &Workflow, inputs: &[BinaryDbSnapshotReplayWriteInput]) -> StoreResult<()> {
    let root = std::path::Path::new(".");
    let blobs = BinaryDbBlobStore::<_, 1>::new(store.db().clone(), root);
    let objects = BinaryDbObjectPackStore::<_, 1>::new(store.db().clone(), root);
    let packs = BinaryDbTreePackStore::<_, 1>::new(store.db().clone(), root);
    let trees = BinaryDbTreeStore::<_, 1>::new(store.db().clone(), root);
    let snapshots = BinaryDbSnapshotStore::<_, 1>::new(store.db().clone(), root);
    let writer = BinaryDbContentWriteCoordinator::new(&blobs, &objects, &packs, &trees, &snapshots);
    let pack_id = tree_pack_id_from_hash48(1);
    writer.record_tree_pack_metadata(
        BinaryDbCommandScope::ContentWrite,
        &BinaryDbTreePackWriteInput {
            pack_rel_path: tree_pack_relative_path(
                &pack_id,
                crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
            )?,
            pack_id,
            pack_format: crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.into(),
            tree_count: 1,
            total_bytes: 0,
            created_at: "2026-10-20T00:00:00Z".into(),
            trees: vec![BinaryDbTreePackTreeWriteInput {
                tree_id: "TRE-00000000000000000001".into(),
                entry_count: 0,
            }],
        },
    )?;
    writer.record_replayed_snapshots(BinaryDbCommandScope::SnapshotWrite, inputs, "fixture", "C")
}

#[test]
fn replay_preserves_terminal_task_change_receipt_and_exact_owner_after_reopen() {
    for with_change in [false, true] {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (base, accepted) = seed_line_and_snapshots(&store);
        let owner = source_owner(&store, &base, &accepted, with_change);
        finish(&store, &owner, &accepted, &base);
        let task_before = TaskStore::get_task(&store, &owner.task_id).unwrap();
        let receipt_before = store.local_task_receipt(&owner.task_id).unwrap();
        let protected_paths = [
            "task.bin",
            "change.bin",
            "land.bin",
            "task_land_index.bin",
            "change_land_index.bin",
        ];
        let before = protected_paths.map(|path| {
            std::fs::read(temp.path().join("binary-db").join(path)).unwrap_or_default()
        });
        assert!(store.validate_snapshot_binding(&owner).is_err());
        let first = "SNP-000000000003";
        let second = "SNP-000000000004";
        replay(&store, &[input(&accepted, first, &base)]).unwrap();
        // Another ready/rebase can replay the previous result with the same
        // original successful receipt and no new source discriminator.
        replay(&store, &[input(first, second, first)]).unwrap();
        let reopened = fixture(&temp);
        assert_eq!(
            TaskStore::get_task(&reopened, &owner.task_id).unwrap(),
            task_before
        );
        assert_eq!(
            reopened.local_task_receipt(&owner.task_id).unwrap(),
            receipt_before
        );
        for (path, bytes) in protected_paths.into_iter().zip(before) {
            assert_eq!(
                std::fs::read(temp.path().join("binary-db").join(path)).unwrap_or_default(),
                bytes,
                "{path}"
            );
        }
        let links = reopened
            .snapshot_ownership_rows(&[accepted, first.into(), second.into()])
            .unwrap();
        assert_eq!(links.len(), 3);
        assert!(links.iter().all(|link| link["task_id"] == owner.task_id
            && link["change_id"].as_str()
                == (!owner.change_id.is_empty()).then_some(owner.change_id.as_str())));
        assert_eq!(
            reopened
                .latest_task_snapshot_id(&owner.task_id)
                .unwrap()
                .as_deref(),
            Some(second)
        );
        assert!(reopened.validate_snapshot_binding(&owner).is_err());
        let mut capture =
            BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
        assert!(store
            .record_snapshot_link_in_write(&mut capture, &owner, second, 1)
            .unwrap_err()
            .contains("active Task"));
        capture.abort().unwrap();
        let read = BinaryDbReadTxn::new(reopened.db());
        for index in 0..3 {
            let link = Link::decode(
                &read
                    .read_record(Workflow::snapshot_link_file(), index)
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(link.task, 0);
            assert_eq!(link.ordinal, index as u8);
            assert_eq!(link.change_plus1, u32::from(with_change));
        }
        drop(read);
        reopened.validate_detached_authority().unwrap();
    }
}

#[test]
fn replay_rejects_active_or_receiptless_owners_and_unowned_sources() {
    for close_without_receipt in [false, true] {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (base, accepted) = seed_line_and_snapshots(&store);
        let owner = source_owner(&store, &base, &accepted, false);
        if close_without_receipt {
            TaskStore::close_task(&store, &owner.task_id, "completed").unwrap();
        }
        let before = store
            .db()
            .record_count(BinarySnapshotCodec::<1>::record_file())
            .unwrap();
        assert!(
            replay(&store, &[input(&accepted, "SNP-000000000003", &base)])
                .unwrap_err()
                .contains("successful receipt")
        );
        assert_eq!(
            store
                .db()
                .record_count(BinarySnapshotCodec::<1>::record_file())
                .unwrap(),
            before
        );
        assert!(replay(&store, &[input(&base, "SNP-000000000003", &base)])
            .unwrap_err()
            .contains("exact live source"));
    }
}

#[test]
fn failed_replay_batch_rolls_back_content_links_and_owner_indexes() {
    let temp = TempDir::new().unwrap();
    let store = fixture(&temp);
    let (base, accepted) = seed_line_and_snapshots(&store);
    let owner = source_owner(&store, &base, &accepted, false);
    finish(&store, &owner, &accepted, &base);
    let before = std::fs::read(temp.path().join("binary-db/task_snapshot_index.bin")).unwrap();
    let first = input(&accepted, "SNP-000000000003", &base);
    let invalid = input(&base, "SNP-000000000004", "SNP-000000000003");
    assert!(replay(&store, &[first.clone(), invalid]).is_err());
    assert_eq!(
        store
            .db()
            .record_count(BinarySnapshotCodec::<1>::record_file())
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .db()
            .record_count(Workflow::snapshot_link_file())
            .unwrap(),
        1
    );
    assert_eq!(
        std::fs::read(temp.path().join("binary-db/task_snapshot_index.bin")).unwrap(),
        before
    );
    let mut wrong_line = first.clone();
    wrong_line.snapshot.line_name = "foreign-line".into();
    assert!(replay(&store, &[wrong_line])
        .unwrap_err()
        .contains("Line differs"));
    assert_eq!(
        store
            .db()
            .record_count(BinarySnapshotCodec::<1>::record_file())
            .unwrap(),
        2
    );
    // Existing immutable content, whether linked or not, is never rebound.
    assert!(replay(&store, &[input(&accepted, &base, &accepted)])
        .unwrap_err()
        .contains("new Snapshot"));
    replay(&store, std::slice::from_ref(&first)).unwrap();
    assert!(replay(&store, &[first])
        .unwrap_err()
        .contains("new Snapshot"));
    assert_eq!(
        store
            .db()
            .record_count(Workflow::snapshot_link_file())
            .unwrap(),
        2
    );
}

#[test]
fn five_completed_tasks_replay_atomically_on_one_new_chain_without_replacing_receipts_or_maps() {
    use crate::snapshot_rebase::{
        plan_snapshot_history_rebase, SnapshotFileMap, SnapshotHistoryBoundary,
        SnapshotReplayParent,
    };
    let temp = TempDir::new().unwrap();
    let store = fixture(&temp);
    let (base, first) = seed_line_and_snapshots(&store);
    let mut previous = base.clone();
    let mut boundaries = Vec::new();
    for index in 0..5 {
        let source = if index == 0 {
            first.clone()
        } else {
            format!("SNP-{:012X}", index + 2)
        };
        if index != 0 {
            append_checkpoint(&store, &source, &previous);
        }
        let owner = source_owner(&store, &previous, &source, index % 2 == 0);
        finish(&store, &owner, &source, &previous);
        if index == 1 {
            TaskStore::mark_task_published(
                &store,
                &owner.task_id,
                Some("origin"),
                Some("RCT-0082"),
            )
            .unwrap();
        }
        boundaries.push(SnapshotHistoryBoundary {
            base_snapshot_id: previous,
            snapshot_id: source.clone(),
            metadata: TaskStore::get_task(&store, &owner.task_id).unwrap(),
            owner,
            base_files: SnapshotFileMap::new(),
            files: SnapshotFileMap::new(),
        });
        previous = source;
    }
    let target = "SNP-000000000020";
    append_checkpoint(&store, target, &base);
    let steps = plan_snapshot_history_rebase(target, &SnapshotFileMap::new(), &boundaries).unwrap();
    let new_ids = (0..5)
        .map(|index| format!("SNP-{:012X}", 100 + index))
        .collect::<Vec<_>>();
    let inputs = steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            assert!(step.reuse_snapshot_id.is_none());
            let parent = match &step.parent {
                SnapshotReplayParent::Existing(id) => id,
                SnapshotReplayParent::Replayed(previous) => &new_ids[*previous],
            };
            input(&step.source.snapshot_id, &new_ids[index], parent)
        })
        .collect::<Vec<_>>();
    replay(&store, &inputs).unwrap();
    let reopened = fixture(&temp);
    let rows = reopened.snapshot_ownership_rows(&new_ids).unwrap();
    assert_eq!(rows.len(), 5);
    for (index, (source, output)) in boundaries.iter().zip(&inputs).enumerate() {
        assert_eq!(
            TaskStore::get_task(&reopened, &source.owner.task_id).unwrap(),
            source.metadata
        );
        assert_eq!(rows[index]["task_id"], source.owner.task_id);
        let read = BinaryDbReadTxn::new(reopened.db());
        let snapshot = snapshot_index_by_id(&read, &output.snapshot.snapshot_id)
            .unwrap()
            .unwrap();
        let record = BinarySnapshotCodec::<1>::decode_record(
            &read
                .read_record(BinarySnapshotCodec::<1>::record_file(), snapshot)
                .unwrap(),
        )
        .unwrap();
        let parent = snapshot_index_by_id(&read, &output.snapshot.parent_snapshot_ids[0])
            .unwrap()
            .unwrap();
        assert_eq!(record.parent_snapshot_index_plus1, parent + 1);
        let link = Link::decode(
            &read
                .read_record(Workflow::snapshot_link_file(), 5 + index as u32)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(link.task, index as u32);
        assert_eq!(link.ordinal, 1);
        assert_eq!(link.change_plus1 != 0, index % 2 == 0);
    }
    assert_eq!(
        reopened
            .db()
            .record_count(Workflow::land_record_file())
            .unwrap(),
        5
    );
    assert_eq!(boundaries[1].metadata["published_task_id"], "RCT-0082");
}

#[test]
fn replay_rejects_ordinal_overflow_without_reusing_or_wrapping() {
    let temp = TempDir::new().unwrap();
    let store = fixture(&temp);
    let (base, accepted) = seed_line_and_snapshots(&store);
    let owner = source_owner(&store, &base, &accepted, false);
    let file = BinarySnapshotCodec::<1>::record_file();
    let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
    let mut record =
        BinarySnapshotCodec::<1>::decode_record(&tx.read_record(file.clone(), 1).unwrap()).unwrap();
    let mut last = accepted;
    for number in 3..=256 {
        let id = format!("SNP-{number:012X}");
        record.snapshot_hash48 = snapshot_hash48_from_id(&id).unwrap();
        record.parent_snapshot_index_plus1 = number - 1;
        tx.append_record(
            file.clone(),
            &BinarySnapshotCodec::<1>::encode_record(&record).unwrap(),
        )
        .unwrap();
        tx.append_index_candidate(
            BinarySnapshotCodec::<1>::id_index(),
            &snapshot_id_index_key(&id).unwrap(),
            number - 1,
        )
        .unwrap();
        store
            .record_snapshot_link_in_write(&mut tx, &owner, &id, 1)
            .unwrap();
        last = id;
    }
    tx.commit().unwrap();
    finish(&store, &owner, &last, &base);
    // The final representable ordinal remains usable by restricted replay.
    replay(&store, &[input(&last, "SNP-000000000101", &base)]).unwrap();
    let last_link = Link::decode(
        &store
            .db()
            .read_record(Workflow::snapshot_link_file(), 255)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(last_link.ordinal, 255);
    let before = store.db().record_count(file.clone()).unwrap();
    assert!(replay(&store, &[input(&last, "SNP-000000000102", &base)])
        .unwrap_err()
        .contains("256 Snapshot links"));
    assert_eq!(store.db().record_count(file).unwrap(), before);
    assert_eq!(
        store
            .db()
            .record_count(Workflow::snapshot_link_file())
            .unwrap(),
        256
    );
}

#[test]
fn replay_never_reuses_a_tombstoned_content_identity() {
    let temp = TempDir::new().unwrap();
    let store = fixture(&temp);
    let (base, accepted) = seed_line_and_snapshots(&store);
    let owner = source_owner(&store, &base, &accepted, false);
    finish(&store, &owner, &accepted, &base);
    let deleted = "SNP-000000000003";
    append_checkpoint(&store, deleted, &base);
    let file = BinarySnapshotCodec::<1>::record_file();
    let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
    let mut raw = tx.read_record(file.clone(), 2).unwrap();
    raw[0] |= 0x80;
    tx.overwrite_record(file.clone(), 2, &raw).unwrap();
    tx.commit().unwrap();
    assert!(replay(&store, &[input(&accepted, deleted, &base)])
        .unwrap_err()
        .contains("new Snapshot"));
    assert_eq!(store.db().record_count(file).unwrap(), 3);
    assert_eq!(
        store
            .db()
            .record_count(Workflow::snapshot_link_file())
            .unwrap(),
        1
    );
}
