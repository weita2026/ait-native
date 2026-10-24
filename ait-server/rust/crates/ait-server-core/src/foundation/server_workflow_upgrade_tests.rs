use super::*;
use crate::foundation::remote_binary_db::{BinaryDbCommandScope, BinaryDbWriteTxn};
use crate::foundation::server_content_binary_db::{
    server_snapshot_hash48_from_id, ServerBinaryDbLineStore, ServerBinaryDbSnapshotStore,
    ServerBinarySnapshotPayload, ServerBinarySnapshotRecord,
};
use crate::foundation::server_workflow_store::*;
use crate::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store;
use serde_json::json;

#[path = "server_workflow_upgrade/historical_tests.rs"]
pub(super) mod historical_disposition;

#[test]
fn later_old_revisions_continue_from_new_c01_after_first_change_is_skipped() {
    let (root, mut request, mut files) = fixture("all-later-revisions");
    let mut changes = records(&files, "change.bin", 68).unwrap();
    for (ordinal, title) in [
        (1u8, "first retained revision"),
        (2u8, "second retained revision"),
    ] {
        let mut row = changes[0].clone();
        row[4] = ordinal;
        set_u32(&mut row, 20, if ordinal == 1 { 1 } else { 3 });
        let bytes = files.get_mut("change_payload.bin").unwrap();
        set_u64(&mut row, 8, bytes.len() as u64);
        row[2..4].copy_from_slice(&(title.len() as u16).to_le_bytes());
        bytes.extend_from_slice(title.as_bytes());
        changes.push(row);
    }
    files.insert("change.bin".into(), record_file(1, &changes));
    files.insert(
        "task_change_index.bin".into(),
        index_file(
            2,
            &[(0, 0, 0, 0), (1, 1, 0, 0), (2, 0, 1, 1), (3, 0, 2, 3)],
            true,
        )
        .unwrap(),
    );
    for name in [
        "change_patchset_index.bin",
        "change_land_index.bin",
        "change_snapshot_index.bin",
    ] {
        files.insert(
            name.into(),
            index_file(4, &[(0, 1, 0, 0)], name != "change_snapshot_index.bin").unwrap(),
        );
    }
    for (name, bytes) in &files {
        fs::write(request.source_repository.join(name), bytes).unwrap();
    }
    request.expected_source_files = fingerprints(&files);
    request.initial_change_indices = BTreeSet::from([0, 1]);
    request.incoming_change_indices.clear();
    stage_repository_workflow(request.clone()).unwrap();
    let target = read_files(&request.staged_repository).unwrap();
    let retained = records(&target, "change.bin", 72).unwrap();
    assert_eq!(retained.len(), 2);
    for (ordinal, row) in retained.iter().enumerate() {
        assert_eq!(u32_at(row, 16), 0);
        assert_eq!(row[4] as usize, ordinal);
        assert_eq!(u32_at(row, 20), ordinal as u32);
        let title = payload(
            &target,
            "change_payload.bin",
            u64_at(row, 8),
            u16_at(row, 2) as usize,
        )
        .unwrap();
        assert_eq!(
            std::str::from_utf8(title).unwrap(),
            ["first retained revision", "second retained revision"][ordinal]
        );
    }
    fs::remove_dir_all(root).unwrap();
}

fn fixture(label: &str) -> (PathBuf, WorkflowUpgradeRequest, Files) {
    let root = std::env::temp_dir().join(format!(
        "ait-workflow-upgrade-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    for schema in SERVER_BINARY_DB_BIN_SCHEMAS {
        fs::write(source.join(schema.path), schema.layout_id.to_le_bytes()).unwrap();
    }
    for schema in SERVER_BINARY_DB_INDEX_SCHEMAS {
        fs::write(source.join(schema.path), schema.layout_id.to_le_bytes()).unwrap();
    }
    let db = FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("repo"),
        RepoName::new("repo"),
        StorePath::new(source.clone()),
        StoreGeneration::new(1),
    );
    let lines = ServerBinaryDbLineStore::<_, 1>::new(db.clone());
    let main = lines.create_line("main", 0, 1).unwrap();
    let snapshots = ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone());
    let append = |id: &str, parent: u32, line: u32, name: &str, time: u64| {
        snapshots
            .append_snapshot(
                id,
                ServerBinarySnapshotRecord {
                    actor_index_plus1: 0,
                    snapshot_meta: 0,
                    history_flags: 0,
                    payload_len: 0,
                    payload_offset: 0,
                    snapshot_hash48: server_snapshot_hash48_from_id(id).unwrap(),
                    parent_snapshot_index_plus1: parent,
                    root_tree_pack_index_plus1: 0,
                    root_entry_ordinal: 0,
                    line_index_plus1: line + 1,
                    manifest_hash: [0; 32],
                    file_count: 0,
                    total_bytes: 0,
                    created_at_s: time,
                },
                &ServerBinarySnapshotPayload {
                    line_name: name.into(),
                    message: None,
                },
            )
            .unwrap()
    };
    append("SNP-0000000000A1", 0, main, "main", 1);
    lines.set_line_head("main", 1, 2).unwrap();
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    store
        .resolve_user_actor(
            &crate::foundation::workflow_binary_v0_adapter::UserActorIdentity::new(
                "fixture",
                "fixture@example.test",
            )
            .unwrap(),
        )
        .unwrap();
    for title in ["admitted bootstrap", "retained revision"] {
        let task = store
            .create_task(
                "repo",
                &json!({"actor_key_index":0,"title":title,"intent":"retain immutable evidence"}),
            )
            .unwrap();
        store
            .create_change(
                "repo",
                &json!({"actor_key_index":0,"task_id":task["task_id"],"title":title,"base_line":"main"}),
            )
            .unwrap();
    }
    let feature = lines.create_line("feature/t-0002", 1, 2).unwrap();
    append("SNP-0000000000A2", 1, feature, "feature/t-0002", 3);
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerSnapshotCapture).unwrap();
    tx.append_record(
        WorkflowBinaryV0Codec::snapshot_link_file(),
        &WorkflowBinaryV0Codec::encode_snapshot_link(V0SnapshotLinkRecord {
            link_meta: 1,
            snapshot_ordinal: 0,
            task_index: 1,
            change_index_plus1: 2,
            content_snapshot_index: 1,
            created_at_s: 3,
        })
        .unwrap(),
    )
    .unwrap();
    tx.overwrite_record(
        WorkflowBinaryV0Codec::chain_index_file("task_snapshot_index.bin"),
        1,
        &WorkflowBinaryV0Codec::encode_task_snapshot_index(V0OrdinalIndexRecord {
            latest_index_plus1: 1,
            count: 1,
            next_ordinal: 1,
            reserved0: 0,
        })
        .unwrap(),
    )
    .unwrap();
    tx.overwrite_record(
        WorkflowBinaryV0Codec::chain_index_file("change_snapshot_index.bin"),
        1,
        &WorkflowBinaryV0Codec::encode_inventory_index(V0InventoryIndexRecord {
            latest_index_plus1: 1,
            count: 1,
            reserved0: 0,
        })
        .unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    drop(tx);
    let candidate=store.publish_patchset("T-0002/C-01",&json!({"actor_key_index":0,"base_snapshot_id":"SNP-0000000000A1","revision_snapshot_id":"SNP-0000000000A2","summary":"retained actual remote candidate","author_mode":"ai_with_human_review"})).unwrap();
    let id = candidate["patchset_id"].as_str().unwrap();
    store
        .record_review(
            "T-0002",
            &json!({"patchset_id":id,"action":"approve","reviewer":"owner"}),
        )
        .unwrap();
    let run = store
        .run_patchset_ci(id, &json!({"trigger":"workflow_ready"}))
        .unwrap();
    store.complete_patchset_ci(id,&json!({"patchset_id":id,"ci_run_seq":run["ci_run_seq"],"selected_suite_count":1,"suite_result_count":1,"blocking_failure_count":0,"overall_status":"pass","tests_status":"pass","lint_status":"pass"})).unwrap();
    store.put_attestation(id,&json!({"verification_state":"pass","require_human_review":true,"require_tests_pass":true})).unwrap();
    assert_eq!(store.evaluate_policy(id).unwrap()["decision"], "pass");
    store.submit_task_land("T-0002",&json!({"actor_key_index":0,"contract":"task-land-atomic/v2","idempotency_key":"retained-actual-receipt","task_or_change_ref":"T-0002","target_line":"main","mode":"direct"})).unwrap();
    validate_frozen_server_workflow_v0(&db).unwrap();
    let mut files = read_files(&source).unwrap();
    // Reconstruct the exact bounded predecessor, including its removed payload.
    for family in crate::foundation::remote_actor_metadata::RemoteWorkflowFile::ALL {
        let bytes = &files[family.name()];
        let rows = bytes[4..]
            .chunks_exact(family.target_width())
            .map(|row| row[..family.source_width()].to_vec())
            .collect::<Vec<_>>();
        files.insert(family.name().into(), record_file(1, &rows));
    }
    for schema in SERVER_BINARY_DB_BIN_SCHEMAS {
        if schema.layout_id == 2 {
            files.get_mut(schema.path).unwrap()[..4].copy_from_slice(&1u32.to_le_bytes());
        }
    }
    let mut changes = records(&files, "change.bin", 68).unwrap();
    set_u32(&mut changes[1], 24, 1);
    let task_rows = records(&files, "task.bin", 60).unwrap();
    for change in &mut changes {
        let task = u32_at(change, 16) as usize;
        set_u64(change, 32, u64_at(&task_rows[task], 20));
    }
    files.insert("change.bin".into(), record_file(1, &changes));
    for (name, width, cache) in [("patchset.bin", 65, 2), ("land.bin", 48, 2)] {
        let mut rows = records(&files, name, width).unwrap();
        for row in &mut rows {
            row[cache] -= 1;
            set_u32(row, 4, 1);
        }
        files.insert(name.into(), record_file(1, &rows));
    }
    for (name, width) in [
        ("attest.bin", 24),
        ("review.bin", 40),
        ("policy.bin", 32),
        ("waiver.bin", 44),
    ] {
        let mut rows = records(&files, name, width).unwrap();
        for row in &mut rows {
            row[3] -= 1;
        }
        files.insert(name.into(), record_file(1, &rows));
    }
    let text_fields = [
        "legacy-worktree",
        "feature/t-0002",
        "T-0002",
        "C-01",
        "ai_with_human_review",
    ];
    let mut retired = Vec::new();
    for text in text_fields {
        retired.extend_from_slice(&(text.len() as u16).to_le_bytes());
    }
    for text in text_fields {
        retired.extend_from_slice(text.as_bytes());
    }
    retired.extend_from_slice(b"legacy-model");
    let mut link = vec![0; 40];
    link[0] = 0x39;
    link[2..4].copy_from_slice(&(retired.len() as u16).to_le_bytes());
    set_u64(&mut link, 4, 4);
    set_u32(&mut link, 12, 1);
    set_u32(&mut link, 16, 2);
    set_u32(&mut link, 20, 1);
    set_u64(&mut link, 32, 3);
    files.insert("snapshot_link.bin".into(), record_file(1, &[link]));
    let mut payload = 1u32.to_le_bytes().to_vec();
    payload.extend_from_slice(&retired);
    files.insert("snapshot_link_payload.bin".into(), payload);
    for (name, bytes) in &files {
        fs::write(source.join(name), bytes).unwrap();
    }
    let request = WorkflowUpgradeRequest {
        source_repository: source,
        staged_repository: root.join("inactive"),
        expected_source_files: fingerprints(&files),
        initial_change_indices: BTreeSet::from([0]),
        incoming_change_indices: BTreeSet::from([1]),
        historical_disposition: WorkflowHistoricalDisposition::default(),
    };
    (root, request, files)
}

#[test]
fn combined_staging_remaps_numeric_owners_and_preserves_actual_receipt_and_gates() {
    let (root, request, source) = fixture("combined");
    let report = stage_repository_workflow(request.clone()).unwrap();
    assert_eq!(report.change_index_map, vec![None, Some(0)]);
    assert_eq!(read_files(&request.source_repository).unwrap(), source);
    let target = read_files(&request.staged_repository).unwrap();
    assert!(!target.contains_key("snapshot_link_payload.bin"));
    let link = WorkflowBinaryV0Codec::decode_snapshot_link(
        &records(&target, "snapshot_link.bin", 22).unwrap()[0],
    )
    .unwrap();
    assert_eq!(
        (
            link.task_index,
            link.change_index_plus1,
            link.content_snapshot_index,
            link.link_meta
        ),
        (1, 1, 1, 1)
    );
    let patch = records(&target, "patchset.bin", 65).unwrap();
    let old = records(&source, "patchset.bin", 65).unwrap();
    assert_eq!(&patch[0][8..], &old[0][8..]);
    assert!(!String::from_utf8_lossy(&target["change_payload.bin"]).contains("admitted bootstrap"));
    for name in [
        "task.bin",
        "snapshot.bin",
        "snapshot_payload.bin",
        "policy_check.bin",
        "actor.bin",
        "actor_payload.bin",
        "line.bin",
    ] {
        assert_legacy_prefix_unchanged(name, &source[name], &target[name]);
    }
    let (plans,revisions)=crate::foundation::server_plan_binary_db::remote_actor_profile::convert_remote_plan_metadata(&source["plan.bin"],&source["plan_revision.bin"]).unwrap();
    assert_eq!(target["plan.bin"], plans);
    assert_eq!(target["plan_revision.bin"], revisions);
    let db = FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("repo"),
        RepoName::new("repo"),
        StorePath::new(request.staged_repository.clone()),
        StoreGeneration::new(1),
    );
    let store = BinaryDbServerWorkflowV0Store::new(db);
    let task = store.get_task(Some("repo"), "T-0002").unwrap();
    assert_eq!(task["successful_land"]["patchset_id"], "T-0002/C-01/P-01");
    assert_eq!(task["status"], "completed");
    assert_eq!(
        store
            .list_changes("repo")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(stage_repository_workflow(request)
        .unwrap_err()
        .contains("already exists"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn conversion_stops_incoming_refs_mixed_sources_and_failed_validation_without_output() {
    let (root, mut request, source) = fixture("reject");
    request.incoming_change_indices.insert(0);
    assert!(stage_repository_workflow(request.clone())
        .unwrap_err()
        .contains("INCOMING_PUBLICATION_REFERENCE_CONFLICT"));
    assert!(!request.staged_repository.exists());
    request.incoming_change_indices.remove(&0);
    request
        .expected_source_files
        .get_mut("task.bin")
        .unwrap()
        .push('0');
    assert!(stage_repository_workflow(request.clone())
        .unwrap_err()
        .contains("fingerprint"));
    request.expected_source_files = fingerprints(&source);
    let mut mixed = source.clone();
    mixed.get_mut("change.bin").unwrap()[0] = 2;
    assert!(convert(
        &mixed,
        &request.initial_change_indices,
        &request.incoming_change_indices
    )
    .unwrap_err()
    .contains("Mixed"));
    let mut corrupt = source.clone();
    corrupt.get_mut("policy_check.bin").unwrap().push(0);
    for (name, bytes) in &corrupt {
        fs::write(request.source_repository.join(name), bytes).unwrap();
    }
    request.expected_source_files = fingerprints(&corrupt);
    assert!(stage_repository_workflow(request.clone()).is_err());
    assert!(!request.staged_repository.exists());
    for (name, bytes) in &source {
        fs::write(request.source_repository.join(name), bytes).unwrap();
    }
    request.expected_source_files = fingerprints(&source);
    stage_repository_workflow(request).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn duplicate_and_imported_successes_require_historical_disposition() {
    let (root, request, mut source) = fixture("disposition");
    let mut lands = records(&source, "land.bin", 48).unwrap();
    let mut second = lands[0].clone();
    second[1] = 1;
    set_u32(&mut second, 12, 1);
    set_u32(&mut second, 16, 1);
    lands.push(second);
    source.insert("land.bin".into(), record_file(1, &lands));
    let error = convert(
        &source,
        &request.initial_change_indices,
        &request.incoming_change_indices,
    )
    .unwrap_err();
    assert!(
        error.contains("multiple succeeded Land rows [0, 1]"),
        "{error}"
    );
    source.insert("land.bin".into(), record_file(1, &lands[..1]));
    let summary = WorkflowBinaryV0Codec::encode_single_text_payload(
        "ait-local-land-receipt/v1 {}",
        "summary",
    )
    .unwrap();
    let mut bytes = 1u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&summary);
    source.insert("patchset_summary_payload.bin".into(), bytes);
    let mut rows = records(&source, "patchset.bin", 65).unwrap();
    set_u64(&mut rows[0], 51, 4);
    rows[0][59..61].copy_from_slice(&(summary.len() as u16).to_le_bytes());
    source.insert("patchset.bin".into(), record_file(1, &rows));
    assert!(convert(
        &source,
        &request.initial_change_indices,
        &request.incoming_change_indices
    )
    .unwrap_err()
    .contains("typed history requires exact source Patchset membership"));
    assert!(!request.staged_repository.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn admitted_initial_associations_clear_together_without_erasing_success_or_ci() {
    let (root, mut request, source) = fixture("task-only");
    request.initial_change_indices.insert(1);
    request.incoming_change_indices.clear();
    let report = stage_repository_workflow(request.clone()).unwrap();
    assert_eq!(report.change_index_map, vec![None, None]);
    let target = read_files(&request.staged_repository).unwrap();
    assert_eq!(target["change.bin"].len(), 4);
    let patch = WorkflowBinaryV0Codec::decode_frozen_patchset(
        &records(&target, "patchset.bin", 65).unwrap()[0],
    )
    .unwrap();
    assert_eq!(
        (
            patch.task_index,
            patch.change_ordinal_plus1,
            patch.previous_change_patchset_index_plus1
        ),
        (1, 0, 0)
    );
    assert_eq!(patch.ci_status_bits, 0b01_01_01);
    let land =
        WorkflowBinaryV0Codec::decode_land(&records(&target, "land.bin", 52).unwrap()[0]).unwrap();
    assert_eq!(
        (
            land.task_index,
            land.change_ordinal_plus1,
            land.previous_change_land_index_plus1
        ),
        (1, 0, 0)
    );
    assert_eq!(land.land_meta & LAND_STATUS_MASK, LAND_STATUS_SUCCEEDED);
    let link = WorkflowBinaryV0Codec::decode_snapshot_link(
        &records(&target, "snapshot_link.bin", 22).unwrap()[0],
    )
    .unwrap();
    assert_eq!(
        (link.task_index, link.change_index_plus1, link.link_meta),
        (1, 0, 0)
    );
    for name in ["attest.bin", "review.bin", "policy.bin"] {
        assert_eq!(target[name][7], 0);
    }
    for name in [
        "task.bin",
        "policy_check.bin",
        "actor.bin",
        "snapshot.bin",
        "line.bin",
    ] {
        assert_legacy_prefix_unchanged(name, &source[name], &target[name]);
    }
    let db = FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("repo"),
        RepoName::new("repo"),
        StorePath::new(request.staged_repository),
        StoreGeneration::new(1),
    );
    let store = BinaryDbServerWorkflowV0Store::new(db);
    let task = store.get_task(Some("repo"), "T-0002").unwrap();
    assert_eq!(task["successful_land"]["patchset_id"], "T-0002/P-01");
    assert!(task["successful_land"]["change_ref"].is_null());
    assert!(store
        .get_patchset(Some("repo"), "T-0002/C-01/P-01")
        .is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn admitted_retained_c02_becomes_exact_c01_with_terminal_receipt_and_gates() {
    let (root, mut request, mut source) = fixture("retained-c02");
    let mut changes = records(&source, "change.bin", 68).unwrap();
    let mut retained = changes[1].clone();
    retained[4] = 1;
    set_u32(&mut retained, 20, 2);
    let title = WorkflowBinaryV0Codec::encode_single_text_payload(
        "deleted initial Task-2 content",
        "Change title",
    )
    .unwrap();
    let payload = source.get_mut("change_payload.bin").unwrap();
    set_u64(&mut changes[1], 8, payload.len() as u64);
    changes[1][2..4].copy_from_slice(&(title.len() as u16).to_le_bytes());
    changes[1][0] = CHANGE_LIFECYCLE_ARCHIVED | CHANGE_META_SUPERSEDED;
    set_u32(&mut changes[1], 24, 0);
    payload.extend_from_slice(&title);
    changes.push(retained.clone());
    source.insert("change.bin".into(), record_file(1, &changes));
    source.insert(
        "task_change_index.bin".into(),
        index_file(2, &[(0, 0, 0, 0), (1, 1, 0, 0), (2, 1, 1, 2)], true).unwrap(),
    );
    for (name, width) in [("patchset.bin", 65), ("land.bin", 48)] {
        let mut rows = records(&source, name, width).unwrap();
        rows[0][2] = 1;
        set_u32(&mut rows[0], 4, 2);
        source.insert(name.into(), record_file(1, &rows));
    }
    for (name, width) in [
        ("attest.bin", 24),
        ("review.bin", 40),
        ("policy.bin", 32),
        ("waiver.bin", 44),
    ] {
        let mut rows = records(&source, name, width).unwrap();
        for row in &mut rows {
            row[3] = 1;
        }
        source.insert(name.into(), record_file(1, &rows));
    }
    let mut links = records(&source, "snapshot_link.bin", 40).unwrap();
    set_u32(&mut links[0], 16, 3);
    source.insert("snapshot_link.bin".into(), record_file(1, &links));
    for name in [
        "change_patchset_index.bin",
        "change_land_index.bin",
        "change_snapshot_index.bin",
    ] {
        source.insert(
            name.into(),
            index_file(3, &[(0, 2, 0, 0)], name != "change_snapshot_index.bin").unwrap(),
        );
    }
    for (name, bytes) in &source {
        fs::write(request.source_repository.join(name), bytes).unwrap();
    }
    request.expected_source_files = fingerprints(&source);
    request.initial_change_indices.insert(1);
    request.incoming_change_indices = BTreeSet::from([2]);
    assert!(stage_repository_workflow(request.clone())
        .unwrap_err()
        .contains("INCOMING_PUBLICATION_REFERENCE_CONFLICT"));
    assert!(!request.staged_repository.exists());
    request.incoming_change_indices.clear();
    let report = stage_repository_workflow(request.clone()).unwrap();
    assert_eq!(report.change_index_map, vec![None, None, Some(0)]);
    assert_eq!(read_files(&request.source_repository).unwrap(), source);
    let target = read_files(&request.staged_repository).unwrap();
    let change = records(&target, "change.bin", 72).unwrap();
    assert_eq!(change.len(), 1);
    assert_eq!(change[0][4], 0);
    assert_eq!(&change[0][28..68], &retained[28..]);
    assert!(!String::from_utf8_lossy(&target["change_payload.bin"])
        .contains("deleted initial Task-2 content"));
    assert_eq!(target["patchset.bin"][6], 1);
    assert_eq!(target["land.bin"][6], 1);
    for name in ["attest.bin", "review.bin", "policy.bin"] {
        assert_eq!(target[name][7], 1);
    }
    for _ in 0..2 {
        let db = FilesystemServerRemoteBinaryDb::test_fixture(
            RepoId::new("repo"),
            RepoName::new("repo"),
            StorePath::new(request.staged_repository.clone()),
            StoreGeneration::new(1),
        );
        let store = BinaryDbServerWorkflowV0Store::new(db);
        let change = store.get_change(Some("repo"), "T-0002/C-01").unwrap();
        assert_eq!(change["title"], "retained revision");
        assert_eq!(change["status"], "landed");
        assert_eq!(change["fork_snapshot_id"], "SNP-0000000000A1");
        assert_eq!(change["current_patchset_id"], "T-0002/C-01/P-01");
        assert!(store.get_change(Some("repo"), "T-0002/C-02").is_err());
        let task = store.get_task(Some("repo"), "T-0002").unwrap();
        assert_eq!(task["successful_land"]["change_ref"], "T-0002/C-01");
        assert_eq!(task["successful_land"]["patchset_id"], "T-0002/C-01/P-01");
        assert_eq!(
            task["successful_land"]["landed_snapshot_id"],
            "SNP-0000000000A2"
        );
    }
    assert_eq!(read_files(&request.staged_repository).unwrap(), target);
    fs::remove_dir_all(root).unwrap();
}

fn assert_legacy_prefix_unchanged(name: &str, before: &[u8], after: &[u8]) {
    let width = match name {
        "task.bin" => 60,
        "snapshot.bin" => 88,
        _ => {
            assert_eq!(before, after, "{name}");
            return;
        }
    };
    assert_eq!(&after[..4], &2u32.to_le_bytes());
    assert_eq!((before.len() - 4) / width, (after.len() - 4) / (width + 4));
    for (old, new) in before[4..]
        .chunks_exact(width)
        .zip(after[4..].chunks_exact(width + 4))
    {
        assert_eq!(old, &new[..width]);
        assert_eq!(&new[width..], &[0; 4]);
    }
}
