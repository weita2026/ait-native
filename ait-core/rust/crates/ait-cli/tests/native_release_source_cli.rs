use ait_cli::release_surface::create_workflow_release_explicit;
use ait_cli::runtime::RepoRuntime;
use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn run_success(repo: &Path, args: &[&str]) {
    let mut command = Command::cargo_bin("ait-cli").expect("ait-cli build artifact");
    command.current_dir(repo).args(args).assert().success();
}

#[test]
fn native_source_local_setup_fails_closed_without_workflow_release_authority() {
    let temp = TempDir::new().unwrap();
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root).unwrap();
    run_success(
        &repo_root,
        &["init", "--name", "native-source-fixture", "--json"],
    );
    let repo = RepoRuntime::discover_from_path(&repo_root).unwrap();

    let error = create_workflow_release_explicit(
        &repo,
        "REL-NATIVE-SOURCE",
        "native-source-fixture",
        env!("CARGO_PKG_VERSION"),
        "main",
        "SNP-NATIVE-SOURCE",
        "manifest-native-source",
        "local-cli",
        Some("ait-native"),
        Some(env!("CARGO_PKG_VERSION")),
        Some(">=3.11"),
        Some("candidate"),
        "[]",
        "[]",
        "{}",
        "{\"native_distribution\":{\"command_profile\":\"cli\"}}",
    )
    .unwrap_err();

    assert_eq!(
        error,
        ait_core::agent_local_workflow_backend::LOCAL_WORKFLOW_AUTHORITY_ERROR
    );
    assert!(!repo_root
        .join(".ait/binary-db/workflow_record.bin")
        .exists());
    assert!(!repo_root
        .join(".ait/binary-db/workflow_record_payload.bin")
        .exists());
}
