//! Read-only inspection must preserve publication identity after Task closeout.
#[path = "support/native_home.rs"]
mod native_home;

use ait_cli::runtime::RepoRuntime;
use ait_core::task_store::TaskStore;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tempfile::TempDir;
use tiny_http::{Header, Response, Server};

struct InspectionRemote {
    url: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl InspectionRemote {
    fn new(changes: Vec<Value>) -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Some(request) = server.recv_timeout(Duration::from_millis(50)).unwrap() else {
                    continue;
                };
                let method = request.method().as_str().to_string();
                let url = request.url().to_string();
                observed.lock().unwrap().push((method.clone(), url.clone()));
                let root = "/v1/native/repository-authorities/19";
                let (status, payload) = if method != "GET" {
                    (405, json!({"error":"inspection attempted a mutation"}))
                } else if url == format!("{root}/tasks/RCT-0001") {
                    (
                        200,
                        json!({"task_id":"RCT-0001","status":"abandoned","successful_land":null,"current_patchset_id":null}),
                    )
                } else if url == format!("{root}/changes") {
                    (200, json!(changes))
                } else if let Some(change) = changes.iter().find(|row| {
                    url == format!(
                        "{root}/changes/{}",
                        row["change_ref"].as_str().unwrap().replace('/', "%2F")
                    )
                }) {
                    (200, change.clone())
                } else {
                    (404, json!({"error":"unexpected inspection route"}))
                };
                request
                    .respond(
                        Response::from_string(payload.to_string())
                            .with_status_code(status)
                            .with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            ),
                    )
                    .unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn observed(&self) -> Vec<(String, String)> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for InspectionRemote {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ait-cli"))
        .env("CODEX_HOME", native_home::path())
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn fixture(remote: &InspectionRemote, status: &str) -> TempDir {
    let temp = TempDir::new().unwrap();
    let init = run(temp.path(), &["init", "--json"]);
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    let path = temp.path().join(".ait/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["id_namespace_prefix"] = json!("C");
    config["repository_index"] = json!(19);
    config["default_remote"] = json!("origin");
    let row = json!({"remote_id":1,"url":remote.url,"repo_name":"inspection-fixture","created_at":"2026-10-18T00:00:00Z"});
    config["remotes"] = json!({"origin":row,"other":row});
    fs::write(path, config.to_string()).unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let store = repo.task_store().unwrap();
    let task = store
        .create_task(
            &repo.repo_name(),
            "Published terminal Task",
            "Inspect retained remote history",
            Some("C"),
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(task["task_id"], "LCT-0001");
    store
        .mark_task_published("LCT-0001", Some("origin"), Some("RCT-0001"))
        .unwrap();
    store.close_task("LCT-0001", status).unwrap();
    temp
}

fn authority_hash(root: &Path) -> Vec<u8> {
    let dir = root.join(".ait/binary-db");
    let mut paths = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.is_file())
        .collect::<Vec<_>>();
    paths.sort();
    let mut hash = Sha256::new();
    for path in paths {
        hash.update(path.file_name().unwrap().as_encoded_bytes());
        hash.update(fs::read(path).unwrap());
    }
    hash.finalize().to_vec()
}

fn revision(ordinal: u8, status: &str) -> Value {
    json!({"task_id":"RCT-0001","change_id":format!("C-{ordinal:02}"),"change_ref":format!("RCT-0001/C-{ordinal:02}"),
        "title":"Retained real revision","status":status,"base_line":"main","fork_snapshot_id":"SNP-FORK"})
}

#[test]
fn terminal_published_task_inspection_preserves_exact_remote_mapping_and_workflow_guards() {
    for (task_status, change_status) in [("canceled", "canceled"), ("abandoned", "archived")] {
        let remote = InspectionRemote::new(vec![revision(1, change_status)]);
        let temp = fixture(&remote, task_status);
        let before = authority_hash(temp.path());
        for reference in ["LCT-0001", "RCT-0001", "RCT-0001/C-01"] {
            let output = run(
                temp.path(),
                &["change", "show", reference, "--remote", "origin", "--json"],
            );
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let row: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(row["task_id"], "RCT-0001");
            assert_eq!(row["change_ref"], "RCT-0001/C-01");
            assert_eq!(row["status"], change_status);
        }
        let reads = remote.observed();
        assert_eq!(
            reads
                .iter()
                .filter(|(_, path)| path.ends_with("/tasks/RCT-0001"))
                .count(),
            2
        );
        assert_eq!(
            reads
                .iter()
                .filter(|(_, path)| path.ends_with("/changes"))
                .count(),
            2
        );
        assert_eq!(
            reads
                .iter()
                .filter(|(_, path)| path.ends_with("/changes/RCT-0001%2FC-01"))
                .count(),
            3
        );
        assert!(reads.iter().all(|(method, _)| method == "GET"));
        let wrong = run(
            temp.path(),
            &["change", "show", "LCT-0001", "--remote", "other", "--json"],
        );
        assert!(!wrong.status.success());
        assert!(String::from_utf8_lossy(&wrong.stderr).contains("selected remote other"));
        let ready = run(
            temp.path(),
            &[
                "workflow", "ready", "LCT-0001", "--apply", "--remote", "origin", "--json",
            ],
        );
        assert!(!ready.status.success());
        assert!(String::from_utf8_lossy(&ready.stderr).contains("not active or completed"));
        assert_eq!(remote.observed(), reads);
        assert_eq!(authority_hash(temp.path()), before);
    }
}

#[test]
fn terminal_published_task_inspection_reports_zero_and_multiple_without_guessing() {
    for changes in [
        Vec::new(),
        vec![revision(1, "canceled"), revision(2, "archived")],
    ] {
        let multiple = !changes.is_empty();
        let remote = InspectionRemote::new(changes);
        let temp = fixture(&remote, "abandoned");
        let before = authority_hash(temp.path());
        let output = run(
            temp.path(),
            &["change", "show", "LCT-0001", "--remote", "origin", "--json"],
        );
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(if multiple {
                "multiple Changes"
            } else {
                "has no Change"
            })
        );
        let reads = remote.observed();
        assert_eq!(reads.len(), 2);
        assert!(reads
            .iter()
            .all(|(method, path)| method == "GET" && !path.contains("/changes/")));
        assert_eq!(authority_hash(temp.path()), before);
    }
}
