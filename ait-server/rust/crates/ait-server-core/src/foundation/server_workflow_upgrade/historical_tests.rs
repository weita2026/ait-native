use super::*;
use crate::foundation::server_workflow_store::ServerWorkflowLandStore;

fn write_source(request: &mut WorkflowUpgradeRequest, files: &Files) {
    for (name, bytes) in files {
        fs::write(request.source_repository.join(name), bytes).unwrap();
    }
    request.expected_source_files = fingerprints(files);
}

pub(in crate::foundation::server_workflow_upgrade) fn pseudo_fixture(
    label: &str,
) -> (PathBuf, WorkflowUpgradeRequest, Files) {
    let (root, mut request, _) = fixture(label);
    let db = FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("repo"),
        RepoName::new("repo"),
        StorePath::new(request.source_repository.clone()),
        StoreGeneration::new(1),
    );
    for name in ["plan.bin", "plan_revision.bin"] {
        assert_eq!(
            fs::read(request.source_repository.join(name))
                .unwrap()
                .len(),
            4
        );
        fs::write(request.source_repository.join(name), 2u32.to_le_bytes()).unwrap();
    }
    let actor = BinaryDbServerWorkflowV0Store::new_frozen(db.clone())
        .resolve_user_actor(
            &crate::foundation::workflow_binary_v0_adapter::UserActorIdentity::new(
                "fixture",
                "fixture@example.test",
            )
            .unwrap(),
        )
        .unwrap();
    let plans = crate::foundation::server_plan_binary_db::BinaryDbServerPlanService::new(db);
    for name in ["first", "second"] {
        plans.create_plan("repo",&json!({"actor_key_index":actor,"title":name,"status":"draft","summary":"Source Plan binding",
            "artifact_path":format!("docs/sprints/{name}.md"),"artifact_heading":"Source",
            "items":[{"plan_item_ref":name,"text":name,"checkbox_state":"open","heading_path":["Source"],"line_number":1}]})).unwrap();
    }
    let mut files = read_files(&request.source_repository).unwrap();
    // Model the historical layout explicitly; that source carried no author tail.
    for (name, width, legacy_width) in [("plan.bin", 52, 48), ("plan_revision.bin", 60, 56)] {
        let legacy = records(&files, name, width)
            .unwrap()
            .into_iter()
            .map(|row| row[..legacy_width].to_vec())
            .collect::<Vec<_>>();
        files.insert(name.into(), record_file(1, &legacy));
    }
    let mut tasks = records(&files, "task.bin", 60).unwrap();
    tasks[1][0] |= TASK_META_PLANNED;
    set_u32(&mut tasks[1], 12, 1);
    set_u32(&mut tasks[1], 16, 1);
    let linked_at = u64_at(&tasks[1], 20);
    set_u64(&mut tasks[1], 36, linked_at);
    files.insert("task.bin".into(), record_file(1, &tasks));
    let land = records(&files, "land.bin", 48).unwrap().remove(0);
    let mut patchsets = records(&files, "patchset.bin", 65).unwrap();
    let p = &mut patchsets[0];
    set_u64(p, 24, u64_at(&land, 28));
    p[32..51].fill(0);
    p[0] |= PATCHSET_EVALUATION_PENDING;
    let manifest = json!({"contract":"ait-local-land-receipt/v1","source_kind":"imported_local_land_receipt",
        "task_id":"T-0002","change_ref":"T-0002/C-01","local_task_id":"LCT-0002",
        "local_change_id":"C-01","local_change_ref":"LCT-0002/C-01","target_line":"main",
        "pre_land_target_snapshot_id":"SNP-0000000000A1","landed_snapshot_id":"SNP-0000000000A2",
        "landed_at_s":u64_at(&land,28)});
    let summary = format!("ait-local-land-receipt/v1 {manifest}");
    let payload = files.get_mut("patchset_summary_payload.bin").unwrap();
    set_u64(p, 51, payload.len() as u64);
    p[59..61].copy_from_slice(&(summary.len() as u16).to_le_bytes());
    payload.extend_from_slice(summary.as_bytes());
    files.insert("patchset.bin".into(), record_file(1, &patchsets));
    request.initial_change_indices = BTreeSet::from([0, 1]);
    request.incoming_change_indices.clear();
    request.historical_disposition = WorkflowHistoricalDisposition {
        legacy_history_patchset_indices: BTreeSet::from([0]),
        retired_pseudo_land_indices: BTreeSet::from([0]),
        retained_terminal_task_indices: BTreeSet::from([1]),
        ..Default::default()
    };
    write_source(&mut request, &files);
    (root, request, files)
}

#[test]
fn source_history_preserves_shared_content_and_rejects_duplicate_owner_tuples() {
    let (root, mut request, mut files) = pseudo_fixture("shared-content-owner-tuples");
    let mut links = records(&files, "snapshot_link.bin", 40).unwrap();
    let mut other = links[0].clone();
    set_u32(&mut other, 12, 0);
    set_u32(&mut other, 16, 1);
    let retired = payload(
        &files,
        "snapshot_link_payload.bin",
        u64_at(&other, 4),
        u16_at(&other, 2) as usize,
    )
    .unwrap()
    .to_vec();
    let bytes = files.get_mut("snapshot_link_payload.bin").unwrap();
    set_u64(&mut other, 4, bytes.len() as u64);
    bytes.extend_from_slice(&retired);
    links.push(other);
    files.insert("snapshot_link.bin".into(), record_file(1, &links));
    files.insert(
        "task_snapshot_index.bin".into(),
        index_file(2, &[(0, 1, 0, 0), (1, 0, 0, 0)], true).unwrap(),
    );
    files.insert(
        "change_snapshot_index.bin".into(),
        index_file(2, &[(0, 1, 0, 0), (1, 0, 0, 0)], false).unwrap(),
    );
    let proven = history::derive_proven_disposition(&files).unwrap();
    assert_eq!(proven.retired_pseudo_land_indices, BTreeSet::from([0]));
    write_source(&mut request, &files);
    let source = read_files(&request.source_repository).unwrap();
    stage_repository_workflow(request.clone()).unwrap();
    let target = read_files(&request.staged_repository).unwrap();
    let converted = records(&target, "snapshot_link.bin", 22).unwrap();
    assert_eq!(converted.len(), 2);
    for (before, after) in links.iter().zip(&converted) {
        let after = WorkflowBinaryV0Codec::decode_snapshot_link(after).unwrap();
        assert_eq!(after.task_index, u32_at(before, 12));
        assert_eq!(after.snapshot_ordinal, before[1]);
        assert_eq!(after.content_snapshot_index, u32_at(before, 20));
        assert_eq!(after.created_at_s, u64_at(before, 32));
    }
    let db = FilesystemServerRemoteBinaryDb::serving_authority(
        RepoId::new("0"),
        RepoName::new("repo"),
        StorePath::new(request.staged_repository.clone()),
        StoreGeneration::new(1),
    );
    let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), "CT")
        .unwrap()
        .with_repository_index(0);
    let read = crate::foundation::remote_binary_db::BinaryDbReadTxn::new(&db);
    let snapshot = u32_at(&links[0], 20);
    assert!(workflow
        .capture_binding_with_read(&read, snapshot)
        .unwrap_err()
        .contains("duplicate or retired numeric ownership"));
    drop(read);
    assert_eq!(read_files(&request.staged_repository).unwrap(), target);
    assert_eq!(read_files(&request.source_repository).unwrap(), source);
    let mut duplicate = links[0].clone();
    duplicate[1] = 1;
    links.push(duplicate);
    files.insert("snapshot_link.bin".into(), record_file(1, &links));
    assert!(history::derive_proven_disposition(&files)
        .unwrap_err()
        .contains("duplicate matching Snapshot Links"));
    links.pop();
    let time = u64_at(&links[0], 32);
    set_u64(&mut links[0], 32, time + 1);
    files.insert("snapshot_link.bin".into(), record_file(1, &links));
    assert!(history::derive_proven_disposition(&files)
        .unwrap_err()
        .contains("inconsistent Snapshot Links"));
    links.remove(0);
    files.insert("snapshot_link.bin".into(), record_file(1, &links));
    assert!(history::derive_proven_disposition(&files)
        .unwrap_err()
        .contains("Snapshot Link is missing"));
    let mut duplicate_target = converted.clone();
    let mut third = converted[0].clone();
    third[1] = 1;
    duplicate_target.push(third);
    fs::write(
        request.staged_repository.join("snapshot_link.bin"),
        record_file(2, &duplicate_target),
    )
    .unwrap();
    fs::write(
        request.staged_repository.join("task_snapshot_index.bin"),
        index_file(2, &[(0, 1, 0, 0), (1, 0, 0, 0), (2, 1, 1, 1)], true).unwrap(),
    )
    .unwrap();
    assert!(validate_frozen_server_workflow_v0(&db)
        .unwrap_err()
        .contains("duplicate Task/content ownership"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn exact_pseudo_retirement_preserves_terminal_provenance_and_allocated_history() {
    let (root, request, source) = pseudo_fixture("explicit-pseudo-disposition");
    let proven = history::derive_proven_disposition(&source).unwrap();
    assert_eq!(
        proven.legacy_history_patchset_indices,
        request
            .historical_disposition
            .legacy_history_patchset_indices
    );
    assert_eq!(
        proven.retired_pseudo_land_indices,
        request.historical_disposition.retired_pseudo_land_indices
    );
    assert_eq!(
        proven.retained_terminal_task_indices,
        request
            .historical_disposition
            .retained_terminal_task_indices
    );
    assert!(proven.retired_duplicate_land_indices.is_empty());
    stage_repository_workflow(request.clone()).unwrap();
    let target = read_files(&request.staged_repository).unwrap();
    for file in [
        "task.bin",
        "task_payload.bin",
        "patchset_summary_payload.bin",
        "task_land_index.bin",
    ] {
        assert_legacy_prefix_unchanged(file, &source[file], &target[file]);
    }
    let lands = records(&target, "land.bin", 52).unwrap();
    assert_eq!(lands.len(), 1);
    let row = WorkflowBinaryV0Codec::decode_land(&lands[0]).unwrap();
    assert_ne!(row.land_meta & LAND_TOMBSTONE, 0);
    assert_eq!(row.land_ordinal, 0);
    assert_eq!(row.task_index, 1);
    assert_eq!(row.change_ordinal_plus1, 0);
    let db = FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("repo"),
        RepoName::new("repo"),
        StorePath::new(request.staged_repository.clone()),
        StoreGeneration::new(1),
    );
    validate_frozen_server_workflow_v0(&db).unwrap();
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db);
    let task = store.get_task(Some("repo"), "T-0002").unwrap();
    assert_eq!(task["status"], "completed");
    assert!(task["successful_land"].is_null());
    assert_eq!(
        store.read_task_audit("repo", "T-0002", "main").unwrap()["verdict"]["code"],
        "completion_receipt_missing"
    );
    assert!(store.get_land(Some("repo"), "T-0002/L-01").is_err());
    assert_eq!(read_files(&request.source_repository).unwrap(), source);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn historical_decisions_are_exact_and_inconsistent_source_never_stages() {
    for case in 0..5 {
        let (root, mut request, mut source) = pseudo_fixture(&format!("reject-historical-{case}"));
        match case {
            0 => request
                .historical_disposition
                .legacy_history_patchset_indices
                .clear(),
            1 => request
                .historical_disposition
                .retired_pseudo_land_indices
                .clear(),
            2 => request
                .historical_disposition
                .retained_terminal_task_indices
                .clear(),
            3 => {
                request
                    .historical_disposition
                    .retained_terminal_task_indices
                    .insert(0);
            }
            _ => {
                let mut links = records(&source, "snapshot_link.bin", 40).unwrap();
                set_u32(&mut links[0], 12, 0);
                source.insert("snapshot_link.bin".into(), record_file(1, &links));
                write_source(&mut request, &source);
            }
        }
        assert!(stage_repository_workflow(request.clone()).is_err());
        assert!(!request.staged_repository.exists());
        assert_eq!(read_files(&request.source_repository).unwrap(), source);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_plan_binding_requires_its_exact_revision_item_and_readable_owner() {
    for case in 0..4 {
        let (root, mut request, mut source) =
            pseudo_fixture(&format!("reject-plan-relationship-{case}"));
        match case {
            0 => {
                let mut tasks = records(&source, "task.bin", 60).unwrap();
                set_u32(&mut tasks[1], 16, 2);
                source.insert("task.bin".into(), record_file(1, &tasks));
            }
            1 => {
                let mut tasks = records(&source, "task.bin", 60).unwrap();
                set_u32(&mut tasks[1], 12, 2);
                source.insert("task.bin".into(), record_file(1, &tasks));
            }
            2 => {
                let mut revisions = records(&source, "plan_revision.bin", 56).unwrap();
                set_u32(&mut revisions[0], 16, 99);
                source.insert("plan_revision.bin".into(), record_file(1, &revisions));
            }
            _ => {
                source.insert("plan_item_payload.bin".into(), 1u32.to_le_bytes().to_vec());
            }
        }
        write_source(&mut request, &source);
        let error = stage_repository_workflow(request.clone()).unwrap_err();
        assert!(error.contains("Plan binding"), "{error}");
        if case == 0 {
            assert!(
                error.contains("outside its bound revision range"),
                "{error}"
            );
        }
        assert!(!request.staged_repository.exists());
        assert_eq!(read_files(&request.source_repository).unwrap(), source);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn explicit_duplicate_choice_keeps_rct1036_shape_without_ordinal_reuse() {
    for retired in [0, 1] {
        let (root, mut request, mut source) =
            fixture(&format!("explicit-duplicate-choice-{retired}"));
        let mut lands = records(&source, "land.bin", 48).unwrap();
        lands[0][0] &= !LAND_HAS_PRE_TARGET;
        set_u32(&mut lands[0], 20, 0);
        let mut later = lands[0].clone();
        later[1] = 1;
        set_u32(&mut later, 12, 1);
        set_u32(&mut later, 16, 1);
        let time = u64_at(&later, 28) + 12;
        set_u64(&mut later, 28, time);
        set_u64(&mut later, 36, time);
        lands.push(later);
        source.insert("land.bin".into(), record_file(1, &lands));
        let mut tasks = records(&source, "task_land_index.bin", 8).unwrap();
        tasks[1] = WorkflowBinaryV0Codec::encode_inventory_index(V0InventoryIndexRecord {
            latest_index_plus1: 2,
            count: 2,
            reserved0: 0,
        })
        .unwrap();
        source.insert("task_land_index.bin".into(), record_file(1, &tasks));
        let mut changes = records(&source, "change_land_index.bin", 8).unwrap();
        changes[1] = WorkflowBinaryV0Codec::encode_ordinal_index(V0OrdinalIndexRecord {
            latest_index_plus1: 2,
            count: 2,
            next_ordinal: 2,
            reserved0: 0,
        })
        .unwrap();
        source.insert("change_land_index.bin".into(), record_file(1, &changes));
        let approved = history::derive_approved_disposition(&source).unwrap();
        assert_eq!(approved.retired_duplicate_land_indices, BTreeSet::from([1]));
        for offset in [20, 44] {
            let mut unknown = source.clone();
            let mut distinct = lands.clone();
            let value = u32_at(&distinct[1], offset);
            set_u32(&mut distinct[1], offset, value + 1);
            unknown.insert("land.bin".into(), record_file(1, &distinct));
            assert!(history::derive_approved_disposition(&unknown)
                .unwrap_err()
                .contains("UNRESOLVED_HISTORICAL_RECEIPTS"));
        }
        request
            .historical_disposition
            .retired_duplicate_land_indices
            .insert(retired);
        write_source(&mut request, &source);
        stage_repository_workflow(request.clone()).unwrap();
        let target = read_files(&request.staged_repository).unwrap();
        assert_eq!(source["task_land_index.bin"], target["task_land_index.bin"]);
        let rows = records(&target, "land.bin", 52).unwrap();
        assert_eq!(rows.len(), 2);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row[1], index as u8);
            assert_eq!(row[0] & LAND_TOMBSTONE != 0, index == retired as usize);
        }
        let db = FilesystemServerRemoteBinaryDb::test_fixture(
            RepoId::new("repo"),
            RepoName::new("repo"),
            StorePath::new(request.staged_repository.clone()),
            StoreGeneration::new(1),
        );
        validate_frozen_server_workflow_v0(&db).unwrap();
        let store = BinaryDbServerWorkflowV0Store::new_frozen(db);
        let kept = if retired == 1 { 1 } else { 2 };
        assert_eq!(
            store.get_task(Some("repo"), "T-0002").unwrap()["successful_land"]["submission_id"],
            format!("T-0002/L-{kept:02}")
        );
        assert!(
            store.get_task(Some("repo"), "T-0002").unwrap()["successful_land"]
                ["pre_land_target_snapshot_id"]
                .is_null()
        );
        assert!(store
            .get_land(Some("repo"), &format!("T-0002/L-{:02}", retired + 1))
            .is_err());
        assert_eq!(read_files(&request.source_repository).unwrap(), source);
        fs::remove_dir_all(root).unwrap();
    }
}

fn replace_summary(files: &mut Files, index: usize, value: &serde_json::Value, prefix: &str) {
    let mut rows = records(files, "patchset.bin", 65).unwrap();
    let summary = format!("{prefix} {value}");
    let bytes = files.get_mut("patchset_summary_payload.bin").unwrap();
    set_u64(&mut rows[index], 51, bytes.len() as u64);
    rows[index][59..61].copy_from_slice(&(summary.len() as u16).to_le_bytes());
    bytes.extend_from_slice(summary.as_bytes());
    files.insert("patchset.bin".into(), record_file(1, &rows));
}

#[test]
fn source_era_aggregate_and_correction_validate_exact_numeric_receipts_before_removal() {
    use crate::foundation::server_content_binary_db::ServerBinarySnapshotCodec;
    let (root, mut request, mut files) = pseudo_fixture("typed-source-validation");
    let mut rows = records(&files, "patchset.bin", 65).unwrap();
    let raw = payload(
        &files,
        "patchset_summary_payload.bin",
        u64_at(&rows[0], 51),
        u16_at(&rows[0], 59) as usize,
    )
    .unwrap();
    let local: serde_json::Value = serde_json::from_str(
        std::str::from_utf8(raw)
            .unwrap()
            .strip_prefix("ait-local-land-receipt/v1 ")
            .unwrap(),
    )
    .unwrap();
    let mut entry = local.clone();
    entry["task_index"] = json!(1);
    entry["change_index"] = json!(1);
    entry["receipt_patchset_index"] = json!(0);
    entry["receipt_patchset_id"] = json!("T-0002/C-01/P-01");
    entry["snapshot_link_count"] = json!(1);
    let aggregate = json!({"contract":"ait-history-promotion/v1","target_line":"main",
        "base_snapshot_id":"SNP-0000000000A1","revision_snapshot_id":"SNP-0000000000A2",
        "entries":[entry],"aggregate":{"task_index":1,"task_id":"T-0002","change_index":1,
            "change_ref":"T-0002/C-01","patchset_index":1,"patchset_id":"T-0002/C-01/P-02"}});
    let mut p = rows[0].clone();
    p[1] = 1;
    rows.push(p.clone());
    p[1] = 2;
    set_u32(&mut p, 20, 2);
    rows.push(p);
    files.insert("patchset.bin".into(), record_file(1, &rows));
    replace_summary(&mut files, 1, &aggregate, "ait-history-promotion/v1");
    let mut snapshots = records(&files, "snapshot.bin", 88).unwrap();
    let mut tail = ServerBinarySnapshotCodec::<1>::decode_record(&snapshots[1]).unwrap();
    tail.snapshot_hash48 = server_snapshot_hash48_from_id("SNP-0000000000A3").unwrap();
    tail.parent_snapshot_index_plus1 = 2;
    tail.snapshot_meta &= !ServerBinarySnapshotRecord::META_PARENT_EDGES_AUTHORITY;
    tail.history_flags = 0;
    tail.created_at_s = 4;
    snapshots.push(ServerBinarySnapshotCodec::<1>::encode_record(&tail).unwrap());
    files.insert("snapshot.bin".into(), record_file(1, &snapshots));
    let mut links = records(&files, "snapshot_link.bin", 40).unwrap();
    let mut link = links[0].clone();
    link[1] = 1;
    set_u32(&mut link, 20, 2);
    set_u64(&mut link, 32, 4);
    links.push(link);
    files.insert("snapshot_link.bin".into(), record_file(1, &links));
    let mut correction = aggregate.clone();
    correction["contract"] = json!("ait-history-promotion-correction/v1");
    correction["revision_snapshot_id"] = json!("SNP-0000000000A3");
    correction["aggregate"]["patchset_index"] = json!(2);
    correction["aggregate"]["patchset_id"] = json!("T-0002/C-01/P-03");
    correction["correction"] = json!({"original_patchset_id":"T-0002/C-01/P-02","prior_patchset_id":"T-0002/C-01/P-02",
        "receipt_revision_snapshot_id":"SNP-0000000000A2","snapshots":[{"snapshot_id":"SNP-0000000000A3","created_at_s":4}]});
    replace_summary(&mut files, 2, &correction, "ait-history-promotion/v1");
    request
        .historical_disposition
        .legacy_history_patchset_indices = BTreeSet::from([0, 1, 2]);
    history::validate_source_history(&files, &request.historical_disposition).unwrap();
    for case in 0..7 {
        let mut bad = files.clone();
        let mut v = correction.clone();
        match case {
            0 => v["entries"][0]["receipt_patchset_index"] = json!(1),
            1 => v["entries"][0]["change_index"] = json!(0),
            2 => v["entries"][0]["snapshot_link_count"] = json!(2),
            3 => v["contract"] = json!("ait-history-promotion-stage/v1"),
            4 => v["correction"]["snapshots"][0]["created_at_s"] = json!(5),
            5 => v["correction"]["prior_patchset_id"] = json!("T-0002/C-01/P-03"),
            _ => {
                let mut tasks = records(&bad, "task.bin", 60).unwrap();
                set_u32(&mut tasks[1], 12, 100);
                bad.insert("task.bin".into(), record_file(1, &tasks));
            }
        }
        replace_summary(&mut bad, 2, &v, "ait-history-promotion/v1");
        assert!(
            history::validate_source_history(&bad, &request.historical_disposition).is_err(),
            "case {case}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}
