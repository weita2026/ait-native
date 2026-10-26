#[path = "support/native_home.rs"]
mod native_home;

use ait_cli::init_surface::{init_repo, InitRequest};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use tiny_http::{Response, Server};

type RecordedRequests = Arc<Mutex<Vec<(String, String, Value)>>>;

struct Fixture {
    url: String,
    requests: RecordedRequests,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new(invalid_actor: bool) -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(20)).unwrap()
                else {
                    continue;
                };
                let method = request.method().as_str().to_string();
                let path = request.url().split('?').next().unwrap().to_string();
                let mut bytes = Vec::new();
                request.as_reader().read_to_end(&mut bytes).unwrap();
                let body: Value = if bytes.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes).unwrap()
                };
                recorded
                    .lock()
                    .unwrap()
                    .push((method.clone(), path.clone(), body.clone()));
                let repository = json!({"repository_index":7,
                    "repository_name":"Exact remote authority", "namespace":"R",
                    "policy_flags":17, "tombstoned":false});
                let (status, payload) = match (method.as_str(), path.as_str()) {
                    ("GET", "/v1/native/repository-authorities/7") => {
                        (200, json!({"repository":repository}))
                    }
                    ("POST", "/v1/native/repository-authorities") => {
                        let actor = if invalid_actor {
                            u32::MAX
                        } else if body["user_name"] == "Bob" {
                            9
                        } else {
                            0
                        };
                        (
                            200,
                            json!({"repository":repository, "actor_key_index":actor}),
                        )
                    }
                    ("POST", "/v1/native/repository-authorities/7/changes/RT-1%2FC-01:close") => (
                        200,
                        json!({"task_id":"RT-1", "change_id":"RT-1/C-01",
                            "status":"archived", "actor_key_index":body["actor_key_index"]}),
                    ),
                    ("GET", "/v1/native/repository-authorities/7/changes/RT-1%2FC-01") => (
                        200,
                        json!({"task_id":"RT-1", "change_id":"RT-1/C-01",
                            "status":"active", "actor_key_index":null}),
                    ),
                    _ => (404, json!({"error":"unexpected fixture route"})),
                };
                request
                    .respond(Response::from_string(payload.to_string()).with_status_code(status))
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
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn repository(url: &str, identity: Option<(&str, &str)>) -> TempDir {
    let temp = TempDir::new().unwrap();
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("local display name".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let mut config = json!({"repo_name":"local display name", "repository_index":7,
        "id_namespace_prefix":"R", "default_line":"main", "default_remote":"origin",
        "sprint":"off", "plan_task_binding":{"mode":"off"},
        "remotes":{"origin":{"remote_id":1,"url":url,"repo_name":"configured display name"}}});
    if let Some((name, email)) = identity {
        config["user_name"] = json!(name);
        config["user_email"] = json!(email);
    }
    fs::write(temp.path().join(".ait/config.json"), config.to_string()).unwrap();
    temp
}

fn run(root: &Path, action: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ait-cli"))
        .env("CODEX_HOME", native_home::path())
        .current_dir(root)
        .args([
            "change",
            action,
            "RT-1/C-01",
            "--remote",
            "origin",
            "--json",
        ])
        .output()
        .unwrap()
}

#[test]
fn cli_resolves_each_configured_actor_and_preserves_change_readback() {
    let server = Fixture::new(false);
    for (name, email, actor) in [(" Alice ", " A@Example ", 0), ("Bob", "B@Example", 9)] {
        let repo = repository(&server.url, Some((name, email)));
        let before = fs::read(repo.path().join(".ait/config.json")).unwrap();
        let result = run(repo.path(), "close");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let payload: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(payload["actor_key_index"], actor);
        assert_eq!(payload["change_ref"], "RT-1/C-01");
        assert_eq!(
            fs::read(repo.path().join(".ait/config.json")).unwrap(),
            before
        );
    }
    let recorded = server.requests.lock().unwrap();
    let requests = recorded
        .iter()
        .filter(|(method, path, _)| {
            method == "POST"
                && (path == "/v1/native/repository-authorities" || path.ends_with(":close"))
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 4);
    for (i, actor) in [0, 9].into_iter().enumerate() {
        let registration = &requests[i * 2].2;
        assert_eq!(requests[i * 2].1, "/v1/native/repository-authorities");
        assert_eq!(registration["repository_index"], 7);
        assert_eq!(registration["repository_name"], "Exact remote authority");
        assert_eq!(registration["namespace"], "R");
        assert_eq!(registration["policy_flags"], 17);
        assert_eq!(
            requests[i * 2 + 1].2,
            json!({"status":"archived","actor_key_index":actor})
        );
    }
    assert_eq!(requests[0].2["user_name"], "Alice");
    assert_eq!(requests[0].2["user_email"], "A@Example");
}

#[test]
fn cli_missing_identity_keeps_reads_available_and_blocks_mutation_before_http() {
    let server = Fixture::new(false);
    let repo = repository(&server.url, None);
    let read = run(repo.path(), "show");
    assert!(
        read.status.success(),
        "{}",
        String::from_utf8_lossy(&read.stderr)
    );
    assert!(serde_json::from_slice::<Value>(&read.stdout).unwrap()["actor_key_index"].is_null());
    let result = run(repo.path(), "close");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("user_name"));
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "GET");
}

#[test]
fn cli_rejects_registration_actor_overflow_before_domain_mutation() {
    let server = Fixture::new(true);
    let repo = repository(&server.url, Some(("Alice", "A@Example")));
    let result = run(repo.path(), "close");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("plus-one bounds"));
    assert_eq!(server.requests.lock().unwrap().len(), 2);
}
