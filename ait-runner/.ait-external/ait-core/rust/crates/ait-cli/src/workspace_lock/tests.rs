use super::{
    can_borrow_legacy_process_lock, workspace_command_lock_path, workspace_root,
    WorkspaceCommandLock, LOCK_TOKEN_ENV,
};
use crate::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonMap, JsonValue};
use fs2::FileExt;
use std::fs::OpenOptions;
use std::sync::Mutex;
use tempfile::TempDir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn test_repo(temp: &TempDir) -> RepoRuntime {
    RepoRuntime {
        root: temp.path().to_path_buf(),
        ait_dir: temp.path().join(".ait"),
        config: JsonMap::<String, JsonValue>::new(),
        worktree_config_path: None,
    }
}

#[test]
fn workspace_command_lock_path_matches_hash_contract_shape() {
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);
    let path = workspace_command_lock_path(&repo);
    assert_eq!(
        path.extension().and_then(|value| value.to_str()),
        Some("lock")
    );
    assert!(path.to_string_lossy().contains(".ait/workspace/locks/"));
}

#[test]
fn workspace_command_lock_records_metadata_after_blocking_acquire() {
    let _env_lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);

    let lock = WorkspaceCommandLock::acquire(&repo, "ait-cli task abandon").unwrap();

    let path = workspace_command_lock_path(&repo);
    let metadata: JsonValue =
        crate::json_support::parse_value(&std::fs::read_to_string(path).unwrap(), "Invalid JSON")
            .unwrap();
    assert_eq!(
        metadata.get("command").and_then(JsonValue::as_str),
        Some("ait-cli task abandon")
    );
    assert!(metadata
        .get("owner_token")
        .and_then(JsonValue::as_str)
        .is_some());
    drop(lock);
}

#[test]
fn nested_rust_workspace_command_borrows_outer_lock() {
    let _env_lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);

    let outer = WorkspaceCommandLock::acquire(&repo, "outer command").unwrap();
    let inner = WorkspaceCommandLock::acquire(&repo, "inner command").unwrap();

    assert!(inner.is_borrowed());
    let metadata: JsonValue = crate::json_support::parse_value(
        &std::fs::read_to_string(workspace_command_lock_path(&repo)).unwrap(),
        "Invalid JSON",
    )
    .unwrap();
    assert_eq!(
        metadata.get("command").and_then(JsonValue::as_str),
        Some("outer command")
    );
    drop(inner);
    drop(outer);
}

#[test]
fn workspace_lock_token_is_restored_after_owned_lock_drop() {
    let _env_lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);
    std::env::set_var(LOCK_TOKEN_ENV, "previous-token");

    {
        let _lock = WorkspaceCommandLock::acquire(&repo, "outer command").unwrap();
        assert_ne!(std::env::var(LOCK_TOKEN_ENV).unwrap(), "previous-token");
    }

    assert_eq!(std::env::var(LOCK_TOKEN_ENV).unwrap(), "previous-token");
    std::env::remove_var(LOCK_TOKEN_ENV);
}

#[test]
fn legacy_direct_child_lock_borrow_requires_actual_held_lock() {
    let _env_lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);
    let lock_path = workspace_command_lock_path(&repo);
    std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let metadata = json!({
        "pid": super::parent_pid(),
        "workspace_root": workspace_root(&repo),
    });
    std::fs::write(
        &lock_path,
        crate::json_support::encode_value_to_vec_error_string(&metadata).unwrap(),
    )
    .unwrap();

    assert!(!can_borrow_legacy_process_lock(
        &metadata,
        &lock_path,
        &workspace_root(&repo)
    ));

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    file.lock_exclusive().unwrap();
    assert!(can_borrow_legacy_process_lock(
        &metadata,
        &lock_path,
        &workspace_root(&repo)
    ));
    file.unlock().unwrap();
}

#[test]
fn legacy_same_process_lock_borrow_requires_actual_held_lock() {
    let _env_lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);
    let lock_path = workspace_command_lock_path(&repo);
    std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let metadata = json!({
        "pid": std::process::id(),
        "workspace_root": workspace_root(&repo),
    });
    std::fs::write(
        &lock_path,
        crate::json_support::encode_value_to_vec_error_string(&metadata).unwrap(),
    )
    .unwrap();

    assert!(!can_borrow_legacy_process_lock(
        &metadata,
        &lock_path,
        &workspace_root(&repo)
    ));

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    file.lock_exclusive().unwrap();
    assert!(can_borrow_legacy_process_lock(
        &metadata,
        &lock_path,
        &workspace_root(&repo)
    ));
    file.unlock().unwrap();
}
