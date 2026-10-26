use super::*;
use crate::binary_db::{AuthorityId, BinaryDb, BinaryDbCommandLockSet, LocalStateScope, StorePath};
use std::{fs, path::Path, sync::mpsc, thread, time::Duration};
use tempfile::TempDir;

fn fixture() -> (TempDir, LocalBinaryDbFs) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let authority = root.join(".ait/binary-db");
    fs::create_dir_all(&authority).unwrap();
    fs::write(root.join(".ait/config.json"), r#"{"repo_name":"fixture"}"#).unwrap();
    let db = LocalBinaryDbFs::new(
        authority,
        root,
        AuthorityId::new("local:fixture"),
        LocalStateScope::Repository,
    );
    (temp, db)
}

#[test]
fn role_prune_reads_only_agent_bindings_without_rewriting_them() {
    let (temp, db) = fixture();
    ensure_pool(&db).unwrap();
    let agents = read(&db).unwrap();
    assign(&db, 2, &agents[2], 1, "bound-developer").unwrap();
    let authority = db.authority_root().as_path();
    let before = [
        fs::read(authority.join("agents.bin")).unwrap(),
        fs::read(authority.join("agents_payload.bin")).unwrap(),
    ];
    for leaf in [
        "task.bin",
        "change.bin",
        "snapshot.bin",
        "snapshot_link.bin",
        "land.bin",
        "worker_job.bin",
        "snapshot_id.idx",
        "agent_roles.bin",
        "agent_teams.bin",
        "agent_team_membership.bin",
        "plan.bin",
    ] {
        fs::create_dir(authority.join(leaf)).unwrap();
    }
    let result = read_for_role_prune(temp.path(), "fixture").unwrap();
    assert_eq!(result[2].session, "bound-developer");
    assert_eq!(result[2].role, Some(1));
    assert_eq!(fs::read(authority.join("agents.bin")).unwrap(), before[0]);
    assert_eq!(
        fs::read(authority.join("agents_payload.bin")).unwrap(),
        before[1]
    );
}

#[test]
fn role_prune_preserves_missing_and_malformed_agent_errors() {
    let (temp, db) = fixture();
    let authority = db.authority_root().as_path();
    assert!(read_for_role_prune(temp.path(), "fixture")
        .unwrap()
        .is_empty());
    assert!(!authority.join("agents.bin").exists());
    assert!(!authority.join("agents_payload.bin").exists());
    ensure_pool(&db).unwrap();
    fs::write(authority.join("agents_payload.bin"), []).unwrap();
    assert!(read_for_role_prune(temp.path(), "fixture").is_err());
    fs::remove_file(authority.join("agents_payload.bin")).unwrap();
    assert!(read_for_role_prune(temp.path(), "fixture").is_err());
    fs::write(authority.join("agents.bin"), 2u32.to_le_bytes()).unwrap();
    assert!(read_for_role_prune(temp.path(), "fixture")
        .unwrap_err()
        .contains("layout"));
}

fn wait_for_writer(root: &Path, lock_root: StorePath) {
    let lock = BinaryDbCommandLockSet::acquire(&lock_root, BinaryDbCommandScope::General).unwrap();
    let root = root.to_path_buf();
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || sender.send(read_for_role_prune(&root, "fixture")).unwrap());
    assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
    drop(lock);
    assert!(receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap()
        .is_empty());
    worker.join().unwrap();
}

#[test]
fn role_prune_coordinates_with_activation_and_general_writers() {
    let (temp, db) = fixture();
    wait_for_writer(
        temp.path(),
        crate::binary_db_generation::binary_db_activation_lock_root(temp.path()),
    );
    wait_for_writer(temp.path(), db.authority_root().clone());
}
