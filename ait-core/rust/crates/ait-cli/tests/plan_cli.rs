use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use fs2::FileExt;
use predicates::prelude::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn write_file(path: &Path, content: &str) {
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    let mut handle = fs::File::create(path).unwrap();
    handle.write_all(content.as_bytes()).unwrap();
}

fn init_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let output = Command::cargo_bin("ait-cli")
        .unwrap()
        .current_dir(root)
        .args(["init", "--name", "fixture-ait", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "init stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    write_file(
        &root.join(".ait/config.json"),
        r#"{
  "repo_name": "fixture-ait",
  "default_line": "main",
  "default_remote": "origin",
  "remotes": {
    "origin": {
      "remote_id": 1,
      "url": "http://127.0.0.1:1",
      "repo_name": "fixture-ait",
      "created_at": "2026-06-08T00:00:00Z"
    }
  },
  "user_email": "tester@example.com"
}"#,
    );
    write_file(
        &root.join("docs/sprints/example.md"),
        r#"# Example

## Ship the first standalone shell [plan-ref: example/root]

- [ ] Build the standalone shell. [ref: example/build-shell]
"#,
    );
    temp
}

fn cargo_bin() -> Command {
    Command::cargo_bin("ait-cli").unwrap()
}

fn workspace_lock_path(root: &Path) -> std::path::PathBuf {
    let workspace_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(workspace_root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    workspace_root
        .join(".ait")
        .join("workspace")
        .join("locks")
        .join(format!("{}.lock", &hex[..16]))
}

fn json_output(root: &Path, args: &[&str]) -> JsonValue {
    let output = cargo_bin().current_dir(root).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Invalid CLI JSON").unwrap()
}

#[test]
fn local_plan_commands_work_end_to_end() {
    let temp = init_repo();
    let root = temp.path();

    let sync = json_output(root, &["plan", "sync", "docs/sprints/example.md", "--json"]);
    assert_eq!(sync.get("status").and_then(JsonValue::as_str), Some("ok"));
    let plan_id = sync["results"][0]["plan_id"].as_str().unwrap().to_string();

    let list = json_output(root, &["plan", "list", "--json"]);
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| {
        row["head_artifact_path"].as_str() == Some("AGENTS.md")
            && row["head_artifact_heading"].as_str() == Some("AGENTS")
    }));
    assert!(rows
        .iter()
        .any(|row| row["plan_id"].as_str() == Some(plan_id.as_str())));

    let show = json_output(root, &["plan", "show", &plan_id, "--json"]);
    assert_eq!(show["plan_id"].as_str(), Some(plan_id.as_str()));

    let items = json_output(root, &["plan", "items", &plan_id, "--json"]);
    assert_eq!(
        items["items"][0]["plan_item_ref"].as_str(),
        Some("example/build-shell")
    );

    let inspect = json_output(root, &["plan", "inspect", &plan_id, "--json"]);
    assert_eq!(inspect["plan"]["taskable_item_count"].as_i64(), Some(1));

    let candidates = json_output(root, &["plan", "candidates", "--json"]);
    assert_eq!(
        candidates["summary"]["taskable_item_count"].as_i64(),
        Some(1)
    );
}

#[test]
fn text_list_output_smoke_contains_title() {
    let temp = init_repo();
    let root = temp.path();
    let _ = json_output(root, &["plan", "sync", "docs/sprints/example.md", "--json"]);
    cargo_bin()
        .current_dir(root)
        .args(["plan", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Ship the first standalone shell"));
}

#[test]
fn plan_sync_waits_for_workspace_lock_release() {
    let temp = init_repo();
    let root = temp.path();
    let lock_path = workspace_lock_path(root);
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let mut handle = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    handle.try_lock_exclusive().unwrap();
    handle.set_len(0).unwrap();
    handle
        .write_all(
            br#"{"command":"python test holder","pid":42,"started_at":"2026-06-08T00:00:00+00:00"}"#,
        )
        .unwrap();
    handle.flush().unwrap();

    let release = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        drop(handle);
    });
    let started = Instant::now();

    cargo_bin()
        .current_dir(root)
        .args(["plan", "sync", "docs/sprints/example.md", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("plan_id"));
    release.join().unwrap();
    assert!(started.elapsed() >= Duration::from_millis(80));
}
