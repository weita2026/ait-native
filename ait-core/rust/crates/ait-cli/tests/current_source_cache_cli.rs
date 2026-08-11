use ait_core::current_source_cache::current_core_source_identity;
use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use predicates::prelude::PredicateBooleanExt;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;
use tempfile::TempDir;

fn cargo_bin() -> Command {
    Command::cargo_bin("ait-cli").unwrap()
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn output_json(command: &mut Command) -> JsonValue {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Invalid CLI JSON").unwrap()
}

fn seed_core_repo(root: &Path) {
    write_file(&root.join("rust/Cargo.toml"), "[workspace]\n");
    write_file(&root.join("rust/crates/ait-cli/Cargo.toml"), "[package]\n");
    write_file(&root.join("rust/crates/ait-core/Cargo.toml"), "[package]\n");
    write_file(&root.join("rust/crates/ait-py/Cargo.toml"), "[package]\n");
    write_file(
        &root.join("rust/crates/ait-cli/src/main.rs"),
        "fn main() {}\n",
    );
    write_file(
        &root.join("rust/crates/ait-core/src/lib.rs"),
        "pub fn core() {}\n",
    );
    write_file(
        &root.join("rust/crates/ait-py/src/lib.rs"),
        "pub fn py() {}\n",
    );
}

fn cli_bootstrap_metadata(temp: &TempDir) -> (PathBuf, PathBuf) {
    let core_root = temp.path().join("ait-core");
    let metadata_path = temp.path().join(".current-source-build.json");
    seed_core_repo(&core_root);
    let identity = current_core_source_identity(&core_root).unwrap();
    let command = cargo_bin();
    let executable_path = PathBuf::from(command.get_program());
    let executable_sha256 = format!("{:x}", Sha256::digest(fs::read(&executable_path).unwrap()));
    let executable_mtime_ns = u64::try_from(
        fs::metadata(&executable_path)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    )
    .unwrap();
    write_file(
        &metadata_path,
        &format!(
            concat!(
                "{{\n",
                "  \"ait_cli_mtime_ns\": {},\n",
                "  \"ait_cli_profile\": \"release\",\n",
                "  \"ait_cli_sha256\": \"{}\",\n",
                "  \"core_source_fingerprint\": \"{}\",\n",
                "  \"core_source_mtime_ns\": {}\n",
                "}}\n"
            ),
            executable_mtime_ns,
            executable_sha256,
            identity.source_fingerprint,
            identity.source_mtime_ns
        ),
    );
    (core_root, metadata_path)
}

#[test]
fn current_source_run_cli_validates_then_reparses_in_the_same_process() {
    let temp = TempDir::new().unwrap();
    let (core_root, metadata_path) = cli_bootstrap_metadata(&temp);

    cargo_bin()
        .current_dir(temp.path())
        .args([
            "current-source-cache",
            "run-cli",
            "--metadata-path",
            metadata_path.to_str().unwrap(),
            "--core-repo-root",
            core_root.to_str().unwrap(),
            "--",
            "init",
            "--help",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("Usage: ait init"));
}

#[test]
fn current_source_run_cli_fails_closed_before_forwarded_dispatch_when_stale() {
    let temp = TempDir::new().unwrap();
    let (core_root, metadata_path) = cli_bootstrap_metadata(&temp);
    write_file(
        &core_root.join("rust/crates/ait-core/src/lib.rs"),
        "pub fn changed_after_build() {}\n",
    );

    cargo_bin()
        .current_dir(temp.path())
        .args([
            "current-source-cache",
            "run-cli",
            "--metadata-path",
            metadata_path.to_str().unwrap(),
            "--core-repo-root",
            core_root.to_str().unwrap(),
            "--",
            "init",
            "--help",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("Current-source ait-cli is stale"))
        .stdout(predicates::str::is_empty());
}

#[test]
fn current_source_run_cli_is_hidden_and_rejects_recursive_dispatch() {
    let temp = TempDir::new().unwrap();
    let (core_root, metadata_path) = cli_bootstrap_metadata(&temp);

    cargo_bin()
        .arg("current-source-cache")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("run-cli").not());

    cargo_bin()
        .current_dir(temp.path())
        .args([
            "current-source-cache",
            "run-cli",
            "--metadata-path",
            metadata_path.to_str().unwrap(),
            "--core-repo-root",
            core_root.to_str().unwrap(),
            "--",
            "current-source-cache",
            "run-cli",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "refuses recursive internal command dispatch",
        ));
}

#[test]
fn current_source_cache_contract_runs_without_repo_and_isolates_fingerprints() {
    let temp = TempDir::new().unwrap();
    let namespace_root = temp.path().join("AIT_RAM/.ait-temp/repo-scope");
    let core_root = temp.path().join("ait-core");
    seed_core_repo(&core_root);

    let base = output_json(cargo_bin().current_dir(temp.path()).args([
        "current-source-cache",
        "contract",
        "--namespace-root",
        namespace_root.to_str().unwrap(),
        "--core-repo-root",
        core_root.to_str().unwrap(),
        "--core-source-fingerprint",
        "core-a",
        "--ext-suffix",
        ".cpython-314-darwin.so",
        "--worker-id",
        "shared",
        "--json",
    ]));
    let changed = output_json(cargo_bin().current_dir(temp.path()).args([
        "current-source-cache",
        "contract",
        "--namespace-root",
        namespace_root.to_str().unwrap(),
        "--core-repo-root",
        core_root.to_str().unwrap(),
        "--core-source-fingerprint",
        "core-b",
        "--ext-suffix",
        ".cpython-314-darwin.so",
        "--worker-id",
        "shared",
        "--json",
    ]));

    assert_eq!(base["cache_schema_version"], "v3-source-fingerprint");
    assert_eq!(base["namespace"], "current-source-native");
    assert_eq!(base["core_source_fingerprint"], "core-a");
    assert_eq!(base["worker_id"], "shared");
    assert_ne!(base["build_key"], changed["build_key"]);
    assert!(base["cache_root"]
        .as_str()
        .unwrap()
        .starts_with(namespace_root.to_str().unwrap()));
    assert!(base["target_dir"]
        .as_str()
        .unwrap()
        .ends_with("/cargo-target"));
}

#[test]
fn current_source_cache_core_fingerprint_runs_without_repo() {
    let temp = TempDir::new().unwrap();
    let core_root = temp.path().join("ait-core");
    seed_core_repo(&core_root);

    let first = output_json(cargo_bin().current_dir(temp.path()).args([
        "current-source-cache",
        "core-fingerprint",
        core_root.to_str().unwrap(),
        "--json",
    ]));
    write_file(&core_root.join("README.md"), "ignored\n");
    let second = output_json(cargo_bin().current_dir(temp.path()).args([
        "current-source-cache",
        "core-fingerprint",
        core_root.to_str().unwrap(),
        "--json",
    ]));

    assert_eq!(first["kind"], "core");
    assert_eq!(first["fingerprint"].as_str().unwrap().len(), 64);
    assert_eq!(first["fingerprint"], second["fingerprint"]);
}

#[test]
fn current_source_cache_lifecycle_commands_activate_release_and_prune() {
    let temp = TempDir::new().unwrap();
    let namespace_root = temp.path().join("AIT_RAM/.ait-temp/repo-scope");
    let core_root = temp.path().join("ait-core");
    seed_core_repo(&core_root);

    let contract = output_json(cargo_bin().current_dir(temp.path()).args([
        "current-source-cache",
        "contract",
        "--namespace-root",
        namespace_root.to_str().unwrap(),
        "--core-repo-root",
        core_root.to_str().unwrap(),
        "--core-source-fingerprint",
        "core-a",
        "--ext-suffix",
        ".cpython-314-darwin.so",
        "--worker-id",
        "gw1",
        "--json",
    ]));
    let cache_root = Path::new(contract["cache_root"].as_str().unwrap()).to_path_buf();
    write_file(
        &cache_root.join("runtime-extensions/ait_py/payload.bin"),
        "payload",
    );

    let building = output_json(cargo_bin().current_dir(temp.path()).args([
        "current-source-cache",
        "mark-building",
        "--namespace-root",
        namespace_root.to_str().unwrap(),
        "--core-repo-root",
        core_root.to_str().unwrap(),
        "--core-source-fingerprint",
        "core-a",
        "--ext-suffix",
        ".cpython-314-darwin.so",
        "--worker-id",
        "gw1",
        "--source-mtime-ns",
        "1",
        "--extra-json",
        "{\"core_repo_root\":\"/native/core\"}",
        "--json",
    ]));
    assert_eq!(building["state"], "building");
    assert!(cache_root.join("manifest.json").exists());

    let stale = output_json(cargo_bin().current_dir(temp.path()).args([
        "current-source-cache",
        "contract",
        "--namespace-root",
        namespace_root.to_str().unwrap(),
        "--core-repo-root",
        core_root.to_str().unwrap(),
        "--core-source-fingerprint",
        "core-stale",
        "--ext-suffix",
        ".cpython-314-darwin.so",
        "--worker-id",
        "shared",
        "--json",
    ]));
    let stale_root = Path::new(stale["cache_root"].as_str().unwrap()).to_path_buf();
    let stale_build_key = stale["build_key"].as_str().unwrap().to_string();
    write_file(
        &stale_root.join("runtime-extensions/ait_py/payload.bin"),
        "stale payload",
    );
    write_file(
        &stale_root.join("manifest.json"),
        &format!(
            "{{\"state\":\"ready\",\"build_key\":\"{stale_build_key}\",\"last_used_at\":1,\"size_bytes\":13}}\n"
        ),
    );

    let owner_pid = std::process::id().to_string();
    let ready = output_json(cargo_bin().current_dir(temp.path()).args([
        "current-source-cache",
        "activate",
        "--namespace-root",
        namespace_root.to_str().unwrap(),
        "--core-repo-root",
        core_root.to_str().unwrap(),
        "--core-source-fingerprint",
        "core-a",
        "--ext-suffix",
        ".cpython-314-darwin.so",
        "--worker-id",
        "gw1",
        "--source-mtime-ns",
        "1",
        "--register-lease",
        "--owner-pid",
        owner_pid.as_str(),
        "--json",
    ]));
    assert_eq!(ready["state"], "ready");
    assert_eq!(ready["manifest"]["state"], "ready");
    assert_eq!(
        ready["prune"]["removed_idle"],
        JsonValue::Array(vec![JsonValue::String(stale_build_key),])
    );
    assert!(!stale_root.exists());
    let lease_path = ready["lease"]["lease_path"].as_str().unwrap();
    assert!(Path::new(lease_path).exists());
    let lease_payload =
        JsonCodec::parse_slice_with_error_prefix(&fs::read(lease_path).unwrap(), "Invalid lease")
            .unwrap();
    assert_eq!(lease_payload["pid"], std::process::id());

    output_json(cargo_bin().current_dir(temp.path()).args([
        "current-source-cache",
        "release-lease",
        "--lease-path",
        lease_path,
        "--namespace-root",
        namespace_root.to_str().unwrap(),
        "--remove-unleased-ready",
        "--json",
    ]));

    assert!(!cache_root.exists());
}
