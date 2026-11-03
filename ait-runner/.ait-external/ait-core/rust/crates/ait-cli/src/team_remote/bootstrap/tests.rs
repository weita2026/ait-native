use super::*;
use tempfile::TempDir;

#[test]
fn shared_operation_lock_excludes_parallel_config_and_ensure() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".ait")).unwrap();
    let (_, lock) = operation_lock(temp.path()).unwrap();
    assert!(operation_lock(temp.path()).is_err());
    drop(lock);
    assert!(operation_lock(temp.path()).is_ok());
}
