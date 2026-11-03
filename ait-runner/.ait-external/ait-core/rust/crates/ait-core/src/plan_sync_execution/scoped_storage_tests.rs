use super::*;
use crate::binary_db::{
    BinaryDb, BinaryDbCommandLockSet, BinaryDbCommandScope, BinaryFileId, BinaryIndexId,
    BinaryPayloadFileId, LocalStateScope,
};
use tempfile::TempDir;

fn fixture() -> (TempDir, PlanSyncStorageRequest) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let authority = root.join(".ait/binary-db");
    fs::create_dir_all(&authority).unwrap();
    fs::create_dir_all(root.join(".ait/objects")).unwrap();
    fs::write(root.join(".ait/config.json"), r#"{"repo_name":"fixture"}"#).unwrap();
    let request = PlanSyncStorageRequest {
        write_layout: Some(1),
        authority_root: Some(authority.to_string_lossy().into_owned()),
        activation_pointer: Some(authority.to_string_lossy().into_owned()),
        repo_root: Some(root.to_string_lossy().into_owned()),
        pack_root: Some(root.to_string_lossy().into_owned()),
        local_authority_id: Some("local:fixture".into()),
        current_line_state_scope: Some(LocalStateScope::Repository),
    };
    (temp, request)
}

#[test]
fn scoped_backend_rejects_workflow_records_payloads_and_indexes_before_file_access() {
    let (_temp, request) = fixture();
    let authority = Path::new(request.authority_root.as_ref().unwrap());
    // A directory cannot be read as a fixed record, payload or index. Opening
    // must neither inspect these entries nor grant access through a store handle.
    for leaf in [
        "task.bin",
        "change.bin",
        "snapshot.bin",
        "snapshot_link.bin",
        "land.bin",
        "worker_job.bin",
        "task_payload.bin",
        "snapshot_payload.bin",
        "snapshot_id.idx",
        "agents.bin",
        "agents_payload.bin",
    ] {
        fs::create_dir(authority.join(leaf)).unwrap();
    }
    let (db, _) = request.binary_db("fixture", true).unwrap();
    for leaf in [
        "task.bin",
        "change.bin",
        "snapshot.bin",
        "snapshot_link.bin",
        "land.bin",
        "worker_job.bin",
        "agents.bin",
    ] {
        assert!(db
            .record_count(BinaryFileId::new(leaf, 1, 4))
            .unwrap_err()
            .to_string()
            .contains("no local schema declaration"));
    }
    assert!(db
        .read_payload(BinaryPayloadFileId::new("snapshot_payload.bin", 1), 4, 1)
        .unwrap_err()
        .to_string()
        .contains("no local schema declaration"));
    assert!(db
        .read_payload(BinaryPayloadFileId::new("agents_payload.bin", 1), 4, 1)
        .unwrap_err()
        .to_string()
        .contains("no local schema declaration"));
    assert!(db
        .lookup_index(BinaryIndexId::new("snapshot_id.idx", 1), b"unused")
        .unwrap_err()
        .to_string()
        .contains("no local schema declaration"));
    assert!(!authority.join("plan.bin").exists());
}

#[test]
fn scoped_open_preserves_plan_recovery_of_uncommitted_dependencies() {
    let (_temp, request) = fixture();
    let authority = Path::new(request.authority_root.as_ref().unwrap());
    fs::write(authority.join("plan_payload.bin"), []).unwrap();
    fs::write(authority.join("plan_revision.bin"), [1, 0, 0, 0, 1]).unwrap();
    fs::write(authority.join("plan_item_payload.bin"), []).unwrap();
    request.binary_db("fixture", true).unwrap();
    assert!(!crate::plan_binary_db::plan_binary_db_recovery_required(authority).unwrap());
    assert!(!authority.join("plan_payload.bin").exists());
}

#[test]
fn scoped_formats_and_roots_fail_closed() {
    for leaf in BIN_PATHS.iter().chain(INDEX_PATHS) {
        let (_temp, request) = fixture();
        fs::write(
            Path::new(request.authority_root.as_ref().unwrap()).join(leaf),
            2u32.to_le_bytes(),
        )
        .unwrap();
        assert!(
            request
                .binary_db("fixture", true)
                .unwrap_err()
                .contains("Unsupported Plan/content layout"),
            "{leaf}"
        );
    }
    let (_temp, mut request) = fixture();
    let authority = Path::new(request.authority_root.as_ref().unwrap());
    fs::write(authority.join("blob.bin"), [1, 0, 0, 0, 1]).unwrap();
    assert!(request
        .binary_db("fixture", true)
        .unwrap_err()
        .contains("record length"));
    request.pack_root = Some("/unselected-pack-root".into());
    assert!(request
        .binary_db("fixture", true)
        .unwrap_err()
        .contains("canonical repository"));
}

#[test]
fn scoped_reader_holds_activation_coordination_and_preserves_general_admission() {
    let (_temp, request) = fixture();
    let root = Path::new(request.repo_root.as_ref().unwrap());
    let authority = Path::new(request.authority_root.as_ref().unwrap());
    fs::write(authority.join("land.bin"), [1, 0, 0, 0, 0]).unwrap();
    fs::write(authority.join("snapshot_link.bin"), 1u32.to_le_bytes()).unwrap();
    let (db, _) = request.binary_db("fixture", true).unwrap();
    assert!(BinaryDbCommandLockSet::try_acquire(
        &binary_db_activation_lock_root(root),
        BinaryDbCommandScope::General
    )
    .unwrap()
    .is_none());
    drop(db);
    let guard = BinaryDbCommandLockSet::try_acquire(
        &binary_db_activation_lock_root(root),
        BinaryDbCommandScope::General,
    )
    .unwrap()
    .unwrap();
    drop(guard);
    // The one-byte receipt tail is not a complete legacy Land44 record.
    assert_eq!(
        request.binary_db("fixture", false).unwrap_err(),
        "Unsupported legacy workflow representation"
    );
    assert_eq!(
        fs::read(authority.join("land.bin")).unwrap(),
        [1, 0, 0, 0, 0]
    );
    assert_eq!(
        fs::read(authority.join("snapshot_link.bin")).unwrap(),
        1u32.to_le_bytes()
    );
}

#[test]
fn scoped_open_waits_for_plan_content_writer() {
    let (_temp, request) = fixture();
    let lock = BinaryDbCommandLockSet::acquire(
        &StorePath::from(PathBuf::from(request.authority_root.as_ref().unwrap())),
        BinaryDbCommandScope::PlanSyncLocal,
    )
    .unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sender
            .send(request.binary_db("fixture", true).map(|_| ()))
            .unwrap();
    });
    assert!(receiver
        .recv_timeout(std::time::Duration::from_millis(100))
        .is_err());
    drop(lock);
    receiver
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
}
