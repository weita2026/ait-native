use super::{
    can_borrow_legacy_process_lock, workspace_command_lock_path, workspace_root,
    WorkspaceCommandLock, LOCK_TOKEN_ENV,
};
use crate::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonMap, JsonValue};
use fs2::FileExt;
use std::fs::OpenOptions;
use std::process::Command;
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
    let directory = workspace_root(&repo)
        .join(".ait")
        .join("workspace")
        .join("locks");
    assert_eq!(path.parent(), Some(directory.as_path()));
    let hash = path.file_stem().unwrap().to_str().unwrap();
    assert_eq!(hash.len(), 16);
    assert!(hash
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
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
fn workspace_lock_delegation_preserves_physical_exclusion() {
    const CHILD_ROOT: &str = "AIT_WORKSPACE_LOCK_TEST_ROOT";
    const CHILD_EXPECT: &str = "AIT_WORKSPACE_LOCK_TEST_EXPECT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = std::path::PathBuf::from(root);
        let repo = RepoRuntime {
            ait_dir: root.join(".ait"),
            root,
            config: JsonMap::<String, JsonValue>::new(),
            worktree_config_path: None,
        };
        let result = WorkspaceCommandLock::try_acquire(&repo, "delegated child");
        match std::env::var(CHILD_EXPECT).unwrap().as_str() {
            "borrowed" => {
                let lock = result.unwrap();
                assert!(lock.is_borrowed());
                assert_eq!(lock.metadata()["command"], "delegating parent");
            }
            "busy" => assert!(result.is_err()),
            "owned" => assert!(!result.unwrap().is_borrowed()),
            unexpected => panic!("Unexpected child expectation: {unexpected}"),
        }
        return;
    }

    let _env_lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);
    let outer = WorkspaceCommandLock::acquire(&repo, "delegating parent").unwrap();
    let metadata = std::fs::read(workspace_command_lock_path(&repo)).unwrap();
    let token = std::env::var(LOCK_TOKEN_ENV).unwrap();
    let child = |expected: &str, owner_token: Option<&str>| {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "workspace_lock::tests::workspace_lock_delegation_preserves_physical_exclusion",
                "--exact",
                "--test-threads=1",
            ])
            .env(CHILD_ROOT, &repo.root)
            .env(CHILD_EXPECT, expected);
        match owner_token {
            Some(token) => command.env(LOCK_TOKEN_ENV, token),
            None => command.env_remove(LOCK_TOKEN_ENV),
        };
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "child {expected}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };

    child("borrowed", Some(&token));
    child("busy", None);
    child("busy", Some("unrelated-owner-token"));
    assert_eq!(
        std::fs::read(workspace_command_lock_path(&repo)).unwrap(),
        metadata,
    );
    drop(outer);
    child("owned", Some(&token));
}

#[test]
fn workspace_lock_conflicts_with_existing_whole_file_locks() {
    let _env_lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);
    let outer = WorkspaceCommandLock::acquire(&repo, "physical owner").unwrap();
    let path = workspace_command_lock_path(&repo);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let error = file.try_lock_exclusive().unwrap_err();
    assert!(super::is_lock_would_block(&error));
    drop(outer);

    file.try_lock_exclusive().unwrap();
    assert!(WorkspaceCommandLock::try_acquire(&repo, "contender").is_err());
    FileExt::unlock(&file).unwrap();
    let owner = WorkspaceCommandLock::try_acquire(&repo, "next owner").unwrap();
    assert!(!owner.is_borrowed());
}

#[cfg(windows)]
#[test]
fn workspace_lock_metadata_cannot_overlap_the_lock_byte() {
    use super::windows_lock::{check_metadata_length, LOCK_OFFSET};
    assert!(check_metadata_length(LOCK_OFFSET).is_ok());
    assert!(check_metadata_length(LOCK_OFFSET + 1).is_err());
    assert!(check_metadata_length(u64::MAX).is_err());
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
    assert_eq!(
        can_borrow_legacy_process_lock(&metadata, &lock_path, &workspace_root(&repo)),
        cfg!(unix), // Only Unix currently supplies a parent PID.
    );
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
