use ait_cli::init_surface::{init_repo as initialize_repo, InitRequest};
use ait_core::json_support::{json, JsonCodec, JsonValue};
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
use tiny_http::{Header, Response, Server};

fn write_file(path: &Path, content: &str) {
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    let mut handle = fs::File::create(path).unwrap();
    handle.write_all(content.as_bytes()).unwrap();
}

fn write_workflow_config(root: &Path, mode: &str, scope: &str, remote_url: &str) {
    write_file(
        &root.join(".ait/config.json"),
        &format!(
            r#"{{
  "repo_name": "fixture-ait",
  "repository_index": 7,
  "default_line": "main",
  "workflow_mode": "{mode}",
  "workflow_default_scope": "{scope}",
  "task_default_scope": "{scope}",
  "change_default_scope": "{scope}",
  "default_remote": "origin",
  "remotes": {{
    "origin": {{
      "remote_id": 1,
      "url": "{remote_url}",
      "repo_name": "fixture-ait",
      "created_at": "2026-06-08T00:00:00Z"
    }}
  }},
  "user_email": "tester@example.com"
}}"#
        ),
    );
}

fn init_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    initialize_repo(&InitRequest {
        root: root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    write_workflow_config(root, "solo_local", "local", "http://127.0.0.1:1");
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

fn json_response(status: u16, payload: &JsonValue) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(payload.to_string())
        .with_status_code(status)
        .with_header(Header::from_bytes(b"Content-Type", b"application/json").unwrap())
}

fn zstd_bulk_ids(payload: &JsonValue, field: &str, id_field: &str) -> Vec<JsonValue> {
    payload
        .get(field)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get(id_field).and_then(JsonValue::as_str))
        .map(|value| JsonValue::String(value.to_string()))
        .collect()
}

fn spawn_plan_sync_server(
    fail_create: bool,
) -> (String, thread::JoinHandle<Vec<(String, String)>>) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let address = server.server_addr();
    let handle = thread::spawn(move || {
        let mut observed = Vec::new();
        let mut received_request = false;
        loop {
            let timeout = if received_request { 1 } else { 10 };
            let Some(mut request) = server.recv_timeout(Duration::from_secs(timeout)).unwrap()
            else {
                break;
            };
            received_request = true;
            let method = request.method().as_str().to_string();
            let url = request.url().to_string();
            let mut body_bytes = Vec::new();
            request.as_reader().read_to_end(&mut body_bytes).unwrap();
            let body = String::from_utf8_lossy(&body_bytes);
            observed.push((method.clone(), url.clone()));

            let response = if method == "GET"
                && (url == "/v1/native/repository-authorities/7/sprints"
                    || url.starts_with("/v1/native/repository-authorities/7/sprints?"))
            {
                json_response(200, &json!([]))
            } else if method == "GET"
                && url.starts_with("/v1/native/repository-authorities/7/sprints/")
            {
                json_response(
                    400,
                    &json!({"detail": "record index out of bounds for file 'plan.bin'"}),
                )
            } else if method == "GET"
                && (url.starts_with(
                    "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/object-packs/",
                ) || url.starts_with(
                    "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/tree-packs/",
                ))
            {
                json_response(404, &json!({"detail": "repository pack is absent"}))
            } else if method == "PUT"
                && (url.starts_with(
                    "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/object-packs/",
                ) || url.starts_with(
                    "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/tree-packs/",
                ))
            {
                let pack_id = url.rsplit('/').next().unwrap_or_default();
                json_response(
                    200,
                    &json!({
                        "repo_name": "fixture-ait",
                        "pack_id": pack_id,
                        "stored": true,
                        "pack_bytes": body_bytes.len(),
                        "raw_binary_upload": true,
                    }),
                )
            } else if method == "POST"
                && url == "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/commit"
            {
                let committed =
                    JsonCodec::parse_value_with_error_prefix(&body, "Invalid zstd commit").unwrap();
                let committed_snapshot_ids = zstd_bulk_ids(&committed, "snapshots", "snapshot_id");
                let committed_object_pack_ids =
                    zstd_bulk_ids(&committed, "object_packs", "pack_id");
                let committed_tree_pack_ids = zstd_bulk_ids(&committed, "tree_packs", "pack_id");
                json_response(
                    200,
                    &json!({
                        "repo_name": "fixture-ait",
                        "committed_snapshot_ids": committed_snapshot_ids,
                        "committed_object_pack_ids": committed_object_pack_ids,
                        "committed_tree_pack_ids": committed_tree_pack_ids,
                        "upserted_snapshots": committed_snapshot_ids.len(),
                        "remote_line": JsonValue::Null,
                        "line_update": JsonValue::Null,
                    }),
                )
            } else if method == "POST" && url == "/v1/native/repository-authorities/7/sprints" {
                if fail_create {
                    json_response(503, &json!({"detail": "injected Plan publish failure"}))
                } else {
                    let submitted =
                        JsonCodec::parse_value_with_error_prefix(&body, "Invalid submitted Plan")
                            .unwrap();
                    json_response(
                        200,
                        &json!({
                            "repo_name": "fixture-ait",
                            "plan_id": "PR-42",
                            "plan_revision_id": "plan-revision:42",
                            "head_revision_id": "plan-revision:42",
                            "title": submitted.get("title").cloned().unwrap_or(JsonValue::Null),
                            "status": submitted.get("status").cloned().unwrap_or(JsonValue::Null),
                            "summary": submitted.get("summary").cloned().unwrap_or(JsonValue::Null),
                            "artifact_path": submitted.get("artifact_path").cloned().unwrap_or(JsonValue::Null),
                            "artifact_selector": submitted.get("artifact_selector").cloned().unwrap_or(JsonValue::Null),
                            "artifact_heading": submitted.get("artifact_heading").cloned().unwrap_or(JsonValue::Null),
                            "items": submitted.get("items").cloned().unwrap_or_else(|| json!([])),
                            "source_kind": submitted.get("source_kind").cloned().unwrap_or(JsonValue::Null),
                        }),
                    )
                }
            } else {
                json_response(
                    404,
                    &json!({"detail": format!("unexpected request {method} {url}")}),
                )
            };
            request.respond(response).unwrap();
        }
        observed
    });
    (format!("http://{address}"), handle)
}

#[test]
fn local_plan_commands_work_end_to_end() {
    let temp = init_repo();
    let root = temp.path();

    let sync = json_output(root, &["plan", "sync", "docs/sprints/example.md", "--json"]);
    assert_eq!(sync.get("status").and_then(JsonValue::as_str), Some("ok"));
    let plan_id = sync["results"][0]["plan_id"].as_str().unwrap().to_string();

    let repeated = json_output(root, &["plan", "sync", "docs/sprints/example.md", "--json"]);
    assert_eq!(repeated["results"][0]["action"].as_str(), Some("unchanged"));
    let revisions = json_output(root, &["plan", "revisions", &plan_id, "--json"]);
    assert_eq!(revisions.as_array().unwrap().len(), 1);

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
fn native_plan_scope_defaults_and_cross_mode_overrides_are_compatible() {
    let temp = init_repo();
    let root = temp.path();

    write_workflow_config(root, "solo_remote", "remote", "http://127.0.0.1:1");
    cargo_bin()
        .current_dir(root)
        .args(["plan", "list", "--json"])
        .assert()
        .failure();
    cargo_bin()
        .current_dir(root)
        .args(["plan", "list", "--local", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("AGENTS.md"));

    write_workflow_config(root, "solo_local", "local", "http://127.0.0.1:1");
    cargo_bin()
        .current_dir(root)
        .args(["plan", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("AGENTS.md"));
    cargo_bin()
        .current_dir(root)
        .args(["plan", "list", "--remote", "origin", "--json"])
        .assert()
        .failure();
}

#[test]
fn remote_plan_sync_failure_retains_local_lineage_and_retry_is_idempotent() {
    let temp = init_repo();
    let root = temp.path();
    let (failing_url, failing_server) = spawn_plan_sync_server(true);
    write_workflow_config(root, "solo_remote", "remote", &failing_url);

    let failed_output = cargo_bin()
        .current_dir(root)
        .args(["plan", "sync", "docs/sprints/example.md", "--json"])
        .output()
        .unwrap();
    assert!(!failed_output.status.success());
    let failed = JsonCodec::parse_slice_with_error_prefix(
        &failed_output.stdout,
        "Invalid failed Plan sync JSON",
    )
    .unwrap();
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["mode"], "local_publish");
    assert_eq!(failed["results"].as_array().unwrap().len(), 1);
    assert!(String::from_utf8_lossy(&failed_output.stderr)
        .contains("failed after retaining local Plan lineage"));
    let first_requests = failing_server.join().unwrap();
    assert!(
        first_requests.iter().any(|(method, url)| {
            method == "POST" && url == "/v1/native/repository-authorities/7/sprints"
        }),
        "{first_requests:?}"
    );

    let local_plans = json_output(root, &["plan", "list", "--local", "--json"]);
    let plan_id = local_plans
        .as_array()
        .unwrap()
        .iter()
        .find(|plan| plan["head_artifact_path"] == "docs/sprints/example.md")
        .and_then(|plan| plan["plan_id"].as_str())
        .unwrap()
        .to_string();
    let revisions_before = json_output(root, &["plan", "revisions", &plan_id, "--local", "--json"]);
    assert_eq!(revisions_before.as_array().unwrap().len(), 1);

    let (healthy_url, healthy_server) = spawn_plan_sync_server(false);
    write_workflow_config(root, "solo_remote", "remote", &healthy_url);
    let retried = json_output(root, &["plan", "sync", "docs/sprints/example.md", "--json"]);
    assert_eq!(retried["status"], "ok");
    assert_eq!(retried["mode"], "local_publish");
    assert_eq!(retried["results"][0]["action"], "unchanged");
    assert_eq!(retried["publish_results"].as_array().unwrap().len(), 1);
    let retry_requests = healthy_server.join().unwrap();
    assert!(
        retry_requests.iter().any(|(method, url)| {
            method == "POST" && url == "/v1/native/repository-authorities/7/sprints"
        }),
        "{retry_requests:?}"
    );

    let revisions_after = json_output(root, &["plan", "revisions", &plan_id, "--local", "--json"]);
    assert_eq!(revisions_after.as_array().unwrap().len(), 1);
}

#[test]
fn structured_plan_sync_failure_is_json_and_exits_nonzero() {
    let temp = init_repo();
    let output = cargo_bin()
        .current_dir(temp.path())
        .args([
            "plan",
            "sync",
            "docs/sprints/missing.md",
            "--local",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload =
        JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Invalid failure JSON").unwrap();
    assert_eq!(payload["status"], "failed");
    assert!(String::from_utf8_lossy(&output.stderr).contains("Plan sync failed"));
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
