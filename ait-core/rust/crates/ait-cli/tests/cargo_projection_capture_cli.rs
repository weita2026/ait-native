#[path = "support/native_home.rs"]
mod native_home;

use ait_cli::runtime::RepoRuntime;
use ait_core::local_snapshot::{LocalSnapshotBlobReadStore, LocalSnapshotTreeReadStore};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

const POLICY: &str = "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/canonical\"\n\n[alias]\nfixture-build = [\"test\", \"--profile\", \"ait-ci\"]\n";

fn json(root: &Path, args: &[&str]) -> Value {
    let result = Command::new(env!("CARGO_BIN_EXE_ait-cli"))
        .env("CODEX_HOME", native_home::path())
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{args:?}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

fn start(root: &Path, edit: &Path, name: &str) -> (String, PathBuf) {
    let card = format!("docs/sprints/{name}.md");
    fs::create_dir_all(root.join("docs/sprints")).unwrap();
    fs::write(root.join(&card), format!("# Cargo capture [plan-ref: fixture/{name}]\n\n- [ ] Verify Cargo capture [ref: fixture/{name}/run]\n")).unwrap();
    let result = json(
        root,
        &[
            "task",
            "start",
            "--from",
            &format!("{card}#fixture/{name}/run"),
            "--intent",
            "Verify Cargo capture",
            "--edit-root",
            edit.to_str().unwrap(),
            "--local",
            "--json",
            "--full",
        ],
    );
    (
        result["task_id"].as_str().unwrap().into(),
        PathBuf::from(result["worktree"]["path"].as_str().unwrap()),
    )
}

fn captured_config(root: &Path, snapshot: &str) -> Vec<u8> {
    let repo = RepoRuntime::discover_from_path(root).unwrap();
    let store = repo
        .local_snapshot_operation_store::<1>(&repo.workspace_root())
        .unwrap();
    let row = store
        .snapshot_tree_path_row(snapshot, ".cargo/config.toml")
        .unwrap()
        .unwrap();
    store
        .read_blob_bytes(row["blob_id"].as_str().unwrap())
        .unwrap()
}

fn capture(edit: &Path, task: &str, marker: &str) -> String {
    fs::write(edit.join("capture.txt"), marker).unwrap();
    let result = json(
        edit,
        &[
            "snapshot",
            "create",
            task,
            "--message",
            "Verify Cargo source bytes",
            "--json",
        ],
    );
    result["snapshot_id"].as_str().unwrap().into()
}

#[test]
fn native_task_capture_preserves_source_policy_for_ram_and_ssd_projections() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = base.join("repo");
    fs::create_dir(&root).unwrap();
    json(&root, &["init", "--json"]);
    json(
        &root,
        &[
            "config",
            "set",
            "--user-name",
            "cargo-capture-fixture",
            "--json",
        ],
    );
    let (seed_task, seed) = start(&root, &base.join("seed"), "cargo-policy-seed");
    fs::create_dir_all(seed.join(".cargo")).unwrap();
    fs::write(seed.join(".cargo/config.toml"), POLICY).unwrap();
    let seeded = capture(&seed, &seed_task, "seed");
    assert_eq!(captured_config(&seed, &seeded), POLICY.as_bytes());
    json(&root, &["task", "finish", &seed_task, "--local", "--json"]);

    let (task, edit) = start(&root, &base.join("capture"), "cargo-projection-capture");
    let marker: Value =
        serde_json::from_slice(&fs::read(edit.join(".ait-worktree.json")).unwrap()).unwrap();
    let name = marker["worktree_name"].as_str().unwrap();
    let shared = fs::canonicalize(edit.join(".ait")).unwrap();
    let target = shared.join("cargo-target/task-workspaces").join(name);
    let primary = shared.join("cargo-build");
    let physical = fs::canonicalize(&primary).unwrap_or_else(|_| primary.clone());
    for (index, build_root) in [primary, physical, shared.join("cargo-build-ssd")]
        .into_iter()
        .enumerate()
    {
        let projection = format!("# Managed by ait: workspace-isolated final artifacts and intermediates.\n[build]\ntarget-dir = {}\nbuild-dir = {}\n\n[alias]\nfixture-build = [\"test\", \"--profile\", \"ait-ci\"]\n", serde_json::to_string(&target).unwrap(), serde_json::to_string(&build_root.join("task-workspaces").join(name)).unwrap());
        fs::write(edit.join(".cargo/config.toml"), &projection).unwrap();
        let snapshot = capture(&edit, &task, &format!("placement {index}"));
        let bytes = captured_config(&edit, &snapshot);
        assert_eq!(bytes, POLICY.as_bytes(), "placement {index}");
        assert!(!String::from_utf8(bytes)
            .unwrap()
            .contains("task-workspaces"));
    }

    let authored = "# Managed by ait: workspace-isolated final artifacts and intermediates.\n[build]\ntarget-dir = \"foreign-target\"\nbuild-dir = \"foreign-build\"\n\n[alias]\nkeep-user-alias = \"test\"\n";
    fs::write(edit.join(".cargo/config.toml"), authored).unwrap();
    let snapshot = capture(&edit, &task, "authored foreign paths");
    assert_eq!(captured_config(&edit, &snapshot), authored.as_bytes());
    let revised = POLICY.replace("fixture-build", "authored-alias");
    fs::write(edit.join(".cargo/config.toml"), &revised).unwrap();
    let snapshot = capture(&edit, &task, "authored source policy");
    assert_eq!(captured_config(&edit, &snapshot), revised.as_bytes());
    json(&root, &["task", "finish", &task, "--local", "--json"]);
}
