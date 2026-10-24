use std::path::Path;

#[path = "../../../test_support.rs"]
mod workspace_test_support;

#[test]
fn runtime_workspace_locator_resolves_the_active_worktree() {
    let workspace = workspace_test_support::rust_workspace_root();

    assert!(workspace.join("Cargo.toml").is_file());
    assert!(workspace.join("crates/ait-core/Cargo.toml").is_file());
    assert!(workspace.join("crates/ait-cli/Cargo.toml").is_file());
    assert_eq!(
        workspace_test_support::crate_root("ait-cli"),
        workspace.join("crates/ait-cli")
    );
}

#[test]
fn cargo_binary_locator_ignores_a_missing_compile_time_candidate() {
    let stale_candidate = Path::new("/ait-test-missing-worktree/debug/ait-cli");
    assert!(!stale_candidate.exists());

    let binary = workspace_test_support::cargo_binary(
        "ait-cli",
        Some(stale_candidate.to_str().expect("static path must be UTF-8")),
    );

    assert!(
        binary.is_file(),
        "resolved binary missing: {}",
        binary.display()
    );
    assert_ne!(binary, stale_candidate);
    assert_eq!(
        binary.file_name(),
        Some(std::ffi::OsStr::new(if cfg!(windows) {
            "ait-cli.exe"
        } else {
            "ait-cli"
        }))
    );
}
