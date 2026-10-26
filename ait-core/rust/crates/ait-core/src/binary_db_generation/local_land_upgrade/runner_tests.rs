use super::*;
use crate::binary_db::{BinaryDbCommandScope, BinaryDbWriteTxn};
use crate::content_binary_db::LocalContentBinaryDb;
use crate::line_binary_db::BinaryDbLineStore;
use crate::line_store::LineStore;
use crate::local_snapshot::{
    LocalSnapshotBlobReadStore, LocalSnapshotTreeReadStore, SnapshotAuthoringOptions,
};
use crate::task_store::TaskStore;
use crate::workflow_binary_db::{
    LocalChangeRecord, LocalLandRecord, LocalTaskRecord, SnapshotLinkRecord,
};
use tempfile::TempDir;

struct Fixture {
    temp: TempDir,
    authority: PathBuf,
    receipt_base: String,
    receipt_revision: String,
}

fn family(root: &Path, name: &str, layout: u32, rows: &[u8]) {
    let mut bytes = layout.to_le_bytes().to_vec();
    bytes.extend_from_slice(rows);
    fs::write(root.join(name), bytes).unwrap();
}

fn fixture() -> Fixture {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let authority = root.join(".ait/binary-db");
    fs::create_dir_all(&authority).unwrap();
    fs::write(
        root.join(".ait/config.json"),
        br#"{"repo_name":"ait-runner","id_namespace_prefix":"R"}"#,
    )
    .unwrap();
    let content = LocalContentBinaryDb::<1>::new(
        authority.clone(),
        root,
        AuthorityId::new("local:runner-fixture"),
        LocalStateScope::Repository,
    );
    let lines = BinaryDbLineStore::<_, 1>::new(content.db().clone());
    lines
        .create_line("main", None, "2026-10-20T00:00:00Z")
        .unwrap();
    let mut parents = Vec::new();
    let mut receipt_base = String::new();
    let mut receipt_revision = String::new();
    for number in 0..131 {
        fs::write(root.join("fixture.txt"), format!("snapshot {number}")).unwrap();
        let snapshot = content
            .create_snapshot_content_with_workflow_binding(
                "ait-runner",
                "main",
                &parents,
                Some("fixture snapshot"),
                false,
                SnapshotAuthoringOptions::default(),
                None,
            )
            .unwrap();
        let id = snapshot["snapshot_id"].as_str().unwrap().to_string();
        if number == 119 {
            receipt_base = id.clone();
        }
        if number == 122 {
            receipt_revision = id.clone();
        }
        parents = vec![id];
    }
    let last = parents.remove(0);
    lines
        .set_line_head("main", Some(&last), "2026-10-20T00:00:00Z")
        .unwrap();
    let workflow =
        BinaryDbWorkflowStore::<_, 1>::new_with_namespace(content.db().clone(), "ait-runner", "R");
    let title = "Prepare the coherent local candidate";
    workflow
        .create_task(
            "ait-runner",
            title,
            "Keep the distinct original intent",
            Some("R"),
            None,
            None,
            None,
        )
        .unwrap();
    let mut write = BinaryDbWriteTxn::begin(content.db(), BinaryDbCommandScope::General).unwrap();
    let file = BinaryDbWorkflowStore::<crate::binary_db::LocalBinaryDbFs, 1>::task_record_file();
    let mut task = LocalTaskRecord::decode(&write.read_record(file.clone(), 0).unwrap()).unwrap();
    let time = task.created_at_s;
    task.task_meta |= 0x40;
    task.local_meta = 1;
    task.published_remote_task_index = 63;
    task.published_at_s = time + 1;
    task.updated_at_s = time + 2;
    task.closed_at_s = time + 2;
    write
        .overwrite_record(file, 0, &task.encode().unwrap())
        .unwrap();
    write.commit().unwrap();
    let change = LocalChangeRecord {
        change_meta: 2,
        local_meta: 1,
        payload_len: title.len() as u16,
        change_ordinal: 0,
        change_state: 0,
        reserved1: 0,
        payload_offset: 4,
        task_index: 0,
        previous_change_index_plus1: 0,
        fork_snapshot_index_plus1: 1,
        published_remote_change_ordinal_plus1: 1,
        reserved2: 0,
        reserved3: 0,
        created_at_s: time,
        updated_at_s: time + 2,
        published_at_s: time + 1,
        base_line_index_plus1: 1,
        archived_at_s: 0,
    };
    family(&authority, "change.bin", 1, &change.encode().unwrap());
    family(&authority, "change_payload.bin", 1, title.as_bytes());
    family(
        &authority,
        "task_change_index.bin",
        1,
        &[1, 0, 0, 0, 1, 0, 1, 0],
    );
    let receipt = LocalLandRecord {
        land_meta: 2 | 8 | 16,
        land_ordinal: 0,
        change_ordinal_plus1: 0,
        failure_kind: 0,
        task_index: 0,
        previous_task_land_index_plus1: 0,
        previous_change_land_index_plus1: 0,
        pre_land_target_snapshot_index_plus1: 120,
        landed_snapshot_index_plus1: 123,
        submitted_at_s: time + 2,
        updated_at_s: time + 2,
        target_line_index_plus1: 1,
    };
    family(&authority, "land.bin", 1, &receipt.encode().unwrap());
    family(
        &authority,
        "task_land_index.bin",
        1,
        &[1, 0, 0, 0, 1, 0, 0, 0],
    );
    family(
        &authority,
        "change_land_index.bin",
        1,
        &[1, 0, 0, 0, 1, 0, 1, 0],
    );
    let fields = [
        "feature/lrt-0001",
        "main",
        "LRT-0001",
        "C-01",
        "ai_with_human_review",
    ];
    let mut payload = Vec::new();
    for field in fields {
        payload.extend_from_slice(&(field.len() as u16).to_le_bytes());
    }
    for field in fields {
        payload.extend_from_slice(field.as_bytes());
    }
    let mut link = vec![1 | 8 | 16 | 32, 0];
    link.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    link.extend_from_slice(&4u64.to_le_bytes());
    for number in [0u32, 1, 122, 0, 0] {
        link.extend_from_slice(&number.to_le_bytes());
    }
    link.extend_from_slice(&(time + 1).to_le_bytes());
    family(&authority, "snapshot_link.bin", 1, &link);
    family(&authority, "snapshot_link_payload.bin", 1, &payload);
    family(
        &authority,
        "task_snapshot_index.bin",
        1,
        &[1, 0, 0, 0, 1, 0, 1, 0],
    );
    family(
        &authority,
        "change_snapshot_index.bin",
        1,
        &[1, 0, 0, 0, 1, 0, 0, 0],
    );
    Fixture {
        temp,
        authority,
        receipt_base,
        receipt_revision,
    }
}

#[test]
fn runner_cohort_admission_preserves_task_receipt_content_and_reopens_without_conversion() {
    let fixture = fixture();
    // Bootstrap identity is numeric, independent of Task/Change title equality.
    let title_path = fixture.authority.join("change_payload.bin");
    let mut title = fs::read(&title_path).unwrap();
    title[4] = b'X';
    fs::write(title_path, title).unwrap();
    let root = fixture.temp.path();
    let before_task = fs::read(fixture.authority.join("task.bin")).unwrap();
    let before_snapshots = fs::read(fixture.authority.join("snapshot.bin")).unwrap();
    let before_receipt = fs::read(fixture.authority.join("land.bin")).unwrap();
    let (_, guard) =
        super::super::generation_activation::admit_activated_binary_db_generation_for_runtime(
            root,
            &fixture.authority,
            "ait-runner",
        )
        .unwrap();
    drop(guard);
    assert_eq!(
        header(&fixture.authority.join("land.bin")).unwrap(),
        Some(2)
    );
    assert_eq!(
        header(&fixture.authority.join("snapshot_link.bin")).unwrap(),
        Some(2)
    );
    assert!(!fixture.authority.join("change.bin").exists());
    assert!(!fixture.authority.join("change_payload.bin").exists());
    assert!(!fixture.authority.join("snapshot_link_payload.bin").exists());
    assert_eq!(
        fs::read(fixture.authority.join("task.bin")).unwrap(),
        before_task
    );
    assert_eq!(
        fs::read(fixture.authority.join("snapshot.bin")).unwrap(),
        before_snapshots
    );
    let receipt = fs::read(fixture.authority.join("land.bin")).unwrap();
    assert_eq!(&receipt[4..], &before_receipt[4..]); // Initial Change index and Task index are both zero.
    let link = SnapshotLinkRecord::decode(
        &fs::read(fixture.authority.join("snapshot_link.bin")).unwrap()[4..],
    )
    .unwrap();
    assert_eq!(
        (link.task, link.change_plus1, link.snapshot, link.ordinal),
        (0, 0, 122, 0)
    );
    let workflow = BinaryDbWorkflowStore::<_, 1>::new_with_namespace(
        database(&fixture.authority, root),
        "ait-runner",
        "R",
    );
    let task = workflow.get_task("LRT-0001").unwrap();
    assert_eq!(task["status"], "completed");
    assert_eq!(task["published_task_id"], "RRT-0064");
    assert_eq!(task["intent"], "Keep the distinct original intent");
    assert_eq!(
        task["local_finish"]["pre_land_target_snapshot_id"],
        fixture.receipt_base
    );
    assert_eq!(
        task["local_finish"]["landed_snapshot_id"],
        fixture.receipt_revision
    );
    assert!(task["local_finish"]["change_ref"].is_null());
    let owners = workflow
        .snapshot_ownership_rows(std::slice::from_ref(&fixture.receipt_revision))
        .unwrap();
    assert_eq!(owners[0]["task_id"], "LRT-0001");
    assert!(owners[0]["change_id"].is_null());
    assert!(workflow.changes_for_task("LRT-0001").unwrap().is_empty());
    let content = LocalContentBinaryDb::<1>::new(
        fixture.authority.clone(),
        root,
        AuthorityId::new("local:runner-fixture"),
        LocalStateScope::Repository,
    );
    for (id, expected) in [
        (&fixture.receipt_base, b"snapshot 119".as_slice()),
        (&fixture.receipt_revision, b"snapshot 122".as_slice()),
    ] {
        let rows = content.snapshot_tree_file_rows(Some(id)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(content.read_blob_bytes(&rows[0].blob_id).unwrap(), expected);
    }
    let fingerprint = fingerprint_direct_authority_contents(&fixture.authority).unwrap();
    let (_, guard) =
        super::super::generation_activation::admit_activated_binary_db_generation_for_runtime(
            root,
            &fixture.authority,
            "ait-runner",
        )
        .unwrap();
    drop(guard);
    assert_eq!(
        fingerprint_direct_authority_contents(&fixture.authority).unwrap(),
        fingerprint
    );
}

#[test]
fn runner_revision_namespace_and_owner_drift_are_rejected_before_publication() {
    for fault in [
        "revision",
        "namespace",
        "owner",
        "publication",
        "snapshot_count",
        "receipt_snapshot",
        "snapshot_link_out_of_bounds",
        "receipt_base_out_of_bounds",
    ] {
        let fixture = fixture();
        let root = fixture.temp.path();
        match fault {
            "revision" => {
                let path = fixture.authority.join("change.bin");
                let mut bytes = fs::read(&path).unwrap();
                bytes[4 + 4] = 1;
                fs::write(path, bytes).unwrap();
            }
            "namespace" => fs::write(
                root.join(".ait/config.json"),
                br#"{"repo_name":"ait-runner","id_namespace_prefix":"C"}"#,
            )
            .unwrap(),
            "owner" => {
                let path = fixture.authority.join("snapshot_link_payload.bin");
                let mut bytes = fs::read(&path).unwrap();
                let at = bytes
                    .windows(8)
                    .position(|window| window == b"LRT-0001")
                    .unwrap();
                bytes[at + 1] = b'C';
                fs::write(path, bytes).unwrap();
            }
            "publication" => {
                let path = fixture.authority.join("task.bin");
                let mut bytes = fs::read(&path).unwrap();
                bytes[4 + 20..4 + 24].copy_from_slice(&62u32.to_le_bytes());
                fs::write(path, bytes).unwrap();
            }
            "snapshot_count" => {
                let path = fixture.authority.join("snapshot.bin");
                let mut bytes = fs::read(&path).unwrap();
                bytes.truncate(bytes.len() - 88);
                fs::write(path, bytes).unwrap();
            }
            "receipt_snapshot" => {
                let path = fixture.authority.join("land.bin");
                let mut bytes = fs::read(&path).unwrap();
                bytes[4 + 20..4 + 24].copy_from_slice(&130u32.to_le_bytes());
                fs::write(path, bytes).unwrap();
            }
            "snapshot_link_out_of_bounds" => {
                let path = fixture.authority.join("snapshot_link.bin");
                let mut bytes = fs::read(&path).unwrap();
                bytes[4 + 20..4 + 24].copy_from_slice(&131u32.to_le_bytes());
                fs::write(path, bytes).unwrap();
                let path = fixture.authority.join("land.bin");
                let mut bytes = fs::read(&path).unwrap();
                bytes[4 + 20..4 + 24].copy_from_slice(&132u32.to_le_bytes());
                fs::write(path, bytes).unwrap();
            }
            "receipt_base_out_of_bounds" => {
                let path = fixture.authority.join("land.bin");
                let mut bytes = fs::read(&path).unwrap();
                bytes[4 + 16..4 + 20].copy_from_slice(&132u32.to_le_bytes());
                fs::write(path, bytes).unwrap();
            }
            _ => unreachable!(),
        }
        let before = fingerprint_direct_authority_contents(&fixture.authority).unwrap();
        assert!(
            admit_local_receipt_upgrade(root, &fixture.authority, "ait-runner").is_err(),
            "{fault}"
        );
        assert_eq!(
            fingerprint_direct_authority_contents(&fixture.authority).unwrap(),
            before,
            "{fault}"
        );
        assert_eq!(
            header(&fixture.authority.join("land.bin")).unwrap(),
            Some(1)
        );
    }
}
