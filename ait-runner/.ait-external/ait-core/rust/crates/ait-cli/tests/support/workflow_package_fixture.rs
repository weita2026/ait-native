//! Isolated native/HTTP package fixture shared by the CLI acceptance targets.
#![allow(dead_code)]
use crate::{cli_binary, native_home};

use ait_cli::init_surface::{init_repo, InitRequest};
use ait_cli::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonValue};
use ait_core::line_store::LineStore;
use ait_core::local_snapshot::SnapshotAuthoringOptions;
use ait_core::remote_store::{RemoteAddRecord, RemoteStore};
use ait_core::snapshot_store::{SnapshotRecord, SnapshotStore};
use ait_core::task_store::TaskStore;
use ait_core::workflow_package::inspect_workflow_package_manifest;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub const MODE: &str = "ai_only_experimental";
pub const REVIEW: &str = "Reviewed files: frozen package source; Findings: none after inspection; Risks: low; Tests: isolated native CLI checks passed; Recommendation: pass";

pub fn fixture_phase(phase: &str) {
    use std::io::Write;
    // Direct stderr remains observable even while libtest captures test output.
    writeln!(std::io::stderr().lock(), "ait.fixture.phase={phase}")
        .expect("write fixture phase observation");
}

pub fn command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(cli_binary::selected(env!("CARGO_BIN_EXE_ait-cli")));
    command
        .current_dir(root)
        .env("CODEX_HOME", native_home::path())
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CODEX_SESSION_ID")
        .args(args);
    command
}

pub fn run(root: &Path, args: &[&str]) -> Output {
    command(root, args).output().unwrap()
}

pub fn output_json(output: &Output) -> JsonValue {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

pub fn cli(root: &Path, args: &[&str]) -> JsonValue {
    let output = run(root, args);
    assert!(
        output.status.success(),
        "{args:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output_json(&output)
}

#[derive(Clone)]
pub struct Batch {
    pub manifest: JsonValue,
    pub bytes: Vec<u8>,
    pub identity: JsonValue,
    pub evidence: BTreeMap<String, JsonValue>,
    pub accepted: Option<JsonValue>,
}

pub struct RemoteState {
    pub head: String,
    pub batches: Vec<Batch>,
    pub mappings: BTreeMap<String, String>,
    pub requests: Vec<(String, String)>,
    pub finishes: Vec<JsonValue>,
    pub uploads: Vec<JsonValue>,
    pub archive_missing: bool,
    pub fail_formal_reads: bool,
    pub omit_membership: bool,
    pub corrupt_key: bool,
    pub incomplete_response: bool,
    pub accepted_reply: Option<(u16, JsonValue)>,
    pub lose_response: bool,
    pub cleanup_error: Option<&'static str>,
    pub policy_requires_human: bool,
    pub conflicting_snapshot_metadata: bool,
    pub repository_override: Option<JsonValue>,
    pub snapshot_authority_override: Option<JsonValue>,
    pub package_execution_override: Option<JsonValue>,
    pub job_override: Option<JsonValue>,
    pub pending_execution_reads: usize,
    pub job_read_status: Option<u16>,
    pub reset_jobs: BTreeSet<usize>,
    pub reset_mode_available: bool,
    pub replacements: BTreeMap<usize, usize>,
    pub reprepare_requests: Vec<usize>,
    pub lose_reprepare_response: bool,
    pub reprepare_response_override: Option<JsonValue>,
}

fn response(
    method: &str,
    path: &str,
    bytes: &[u8],
    hash: Option<&str>,
    actor: Option<&str>,
    selected_job: Option<&str>,
    state: &mut RemoteState,
) -> (u16, JsonValue) {
    state.requests.push((method.into(), path.into()));
    if method == "GET" && path == "/healthz" {
        return (
            200,
            json!({"ready":true,"phase":null,"processed":null,"total":null,"last_progress_at":null}),
        );
    }
    if method == "GET" && path == "/v1/native/repository-authorities/0/changes" {
        return (200, json!([]));
    }
    if method == "GET" && path.ends_with("/lines/main") {
        return (
            200,
            json!({"line_name":"main","head_snapshot_id":state.head}),
        );
    }
    if method == "GET" && path == "/v1/native/repository-authorities/0" {
        return (
            200,
            json!({"contract":"ait.server.repository-authority.v1",
                "repository":state.repository_override.clone().unwrap_or_else(|| json!({"repository_index":0,"repository_name":"fixture","namespace":"","policy_flags":0,"tombstoned":false})),
                "pack_storage":{}}),
        );
    }
    if method == "POST" && path == "/v1/native/repository-authorities" {
        return (200, json!({"repository_index":0,"actor_key_index":0}));
    }
    if method == "POST" && path.ends_with("/remote-sync/zstd-bulk/commit") {
        assert_eq!(actor, Some("0"));
        if bytes.is_empty() {
            let old = selected_job.unwrap().parse::<usize>().unwrap();
            assert_eq!(selected_job.unwrap(), old.to_string());
            state.reprepare_requests.push(old);
            if hash != state.batches[old].identity["package_hash"].as_str() {
                return (
                    409,
                    json!({"detail":"Retained selector belongs to another package hash"}),
                );
            }
            if !state.reset_jobs.contains(&old) || state.archive_missing {
                return (
                    409,
                    json!({"detail":"Retained source or confirmed reset is unavailable"}),
                );
            }
            if state.batches.iter().any(|batch| batch.accepted.is_some()) {
                return (
                    409,
                    json!({"detail":"Formal acceptance requires exact readback"}),
                );
            }
            let reused = state.replacements.contains_key(&old);
            let new = if let Some(new) = state.replacements.get(&old) {
                *new
            } else {
                let new = state.batches.len();
                let mut batch = state.batches[old].clone();
                batch.identity["worker_job_index"] = json!(new);
                batch.evidence.clear();
                state.batches.push(batch);
                state.replacements.insert(old, new);
                new
            };
            if std::mem::take(&mut state.lose_reprepare_response) {
                return (408, json!({"detail":"Retained reprepare ACK was lost"}));
            }
            return (200, state.reprepare_response_override.clone().unwrap_or_else(|| json!({"expected_identity":state.batches[new].identity,"ci_state":"passed","reused":reused})));
        }
        let manifest = inspect_workflow_package_manifest(bytes).unwrap();
        let actual_hash = format!("{:x}", Sha256::digest(bytes));
        assert_eq!(hash, Some(actual_hash.as_str()));
        assert_eq!(manifest["base_snapshot_id"], state.head);
        state.uploads.push(manifest.clone());
        let identity = json!({"repository_index":0,"package_hash":actual_hash,"worker_job_index":state.batches.len(),"base_snapshot_id":manifest["base_snapshot_id"],"revision_snapshot_id":manifest["revision_snapshot_id"]});
        state.batches.push(Batch {
            manifest,
            bytes: bytes.to_vec(),
            identity: identity.clone(),
            evidence: BTreeMap::new(),
            accepted: None,
        });
        return (
            200,
            json!({"expected_identity":identity,"ci_state":"passed","reused":false}),
        );
    }
    if let Some((_, job)) = path.split_once("/worker-jobs/") {
        let index = job.parse::<usize>().unwrap();
        let Some(batch) = state.batches.get_mut(index) else {
            return (404, json!({"detail":"Original Job is unavailable"}));
        };
        if method == "GET" {
            if let Some(status) = state.job_read_status {
                return (
                    status,
                    json!({"detail":"Original Job mode read is unavailable"}),
                );
            }
            let ci_state = if state.pending_execution_reads > 0 {
                state.pending_execution_reads -= 1;
                "queued"
            } else {
                "passed"
            };
            let execution = if state.reset_jobs.contains(&index) && state.reset_mode_available {
                json!({"expected_identity":batch.identity,"ci_state":"interrupted","availability":"available","recoverable":false,"author_mode":batch.manifest["author_mode"]})
            } else if state.archive_missing || state.reset_jobs.contains(&index) {
                JsonValue::Null
            } else {
                json!({"expected_identity":batch.identity,"ci_state":ci_state,"availability":"available","recoverable":batch.accepted.is_none(),"author_mode":batch.manifest["author_mode"]})
            };
            return (
                200,
                json!({"job":state.job_override.clone().unwrap_or_else(|| json!({"repository_index":0,"worker_job_index":index,"tombstoned":false,"patchset_index":if batch.accepted.is_some(){json!(index)}else{JsonValue::Null}})),
                    "package_execution":state.package_execution_override.clone().unwrap_or(execution)}),
            );
        }
        assert_eq!(method, "POST");
        let body: JsonValue = serde_json::from_slice(bytes).unwrap();
        assert_eq!(body["expected_identity"], batch.identity);
        assert_eq!(body["actor_key_index"], 0);
        let operation = body["operation"].as_str().unwrap();
        let payload = &body["payload"];
        if operation == "attestation" {
            assert_eq!(payload["author_mode"], batch.manifest["author_mode"]);
            assert_eq!(payload["require_tests_pass"], true);
            assert_eq!(
                payload["require_human_review"],
                batch.manifest["author_mode"] == "ai_with_human_review"
            );
        } else {
            assert_eq!(operation, "review");
            assert_eq!(payload["reviewer"], "Faraday");
            assert_eq!(payload["reviewer_kind"], "bot");
            assert!(matches!(
                payload["action"].as_str(),
                Some("code_review_summary" | "task_approve")
            ));
        }
        let key = body["idempotency_key"].as_str().unwrap().to_string();
        if let Some(previous) = batch.evidence.insert(key, body.clone()) {
            assert_eq!(previous, body);
        }
        return (
            200,
            json!({"expected_identity":batch.identity,"operation":operation,"accepted":true,"recorded_at_s":123}),
        );
    }
    if method == "POST" && path.ends_with("/task-land") {
        let body: JsonValue = serde_json::from_slice(bytes).unwrap();
        state.finishes.push(body.clone());
        let index = body["expected_identity"]["worker_job_index"]
            .as_u64()
            .unwrap() as usize;
        let batch = &state.batches[index];
        assert_eq!(body["expected_identity"], batch.identity);
        assert_eq!(body["actor_key_index"], 0);
        assert_eq!(body["target_line"], "main");
        if let Some(accepted) = &batch.accepted {
            return state
                .accepted_reply
                .clone()
                .unwrap_or((200, accepted.clone()));
        }
        if state.head != batch.identity["base_snapshot_id"] {
            return (
                409,
                json!({"detail":"BASE_STALE: reconcile the Remote target explicitly; retain the original package and CI identity"}),
            );
        }
        assert!(batch
            .evidence
            .values()
            .any(|row| row["operation"] == "attestation"));
        for action in ["code_review_summary", "task_approve"] {
            assert!(batch
                .evidence
                .values()
                .any(|row| row["payload"]["action"] == action));
        }
        if state.policy_requires_human {
            return (
                409,
                json!({"detail":"PACKAGE_HUMAN_REVIEW_REQUIRED: mandatory Policy requires actual human approval"}),
            );
        }
        let mut entries = Vec::new();
        for source in batch.manifest["history_entries"].as_array().unwrap() {
            let local = source["local_task_id"].as_str().unwrap().to_string();
            let next = format!("RT-{:04}", state.mappings.len() + 1);
            let remote = state.mappings.entry(local.clone()).or_insert(next).clone();
            let changes = source["changes"].as_array().unwrap().iter().enumerate().map(|(ordinal, change)| {
                json!({"local_change_ref":change["local_change_ref"],"remote_change_ref":format!("{remote}/C-{:02}", ordinal+1)})
            }).collect::<Vec<_>>();
            entries.push(json!({"local_task_id":local,"remote_task_id":remote,"changes":changes}));
        }
        let final_task = entries.last().unwrap()["remote_task_id"].as_str().unwrap();
        let accepted = json!({"outcome":"confirmed","expected_identity":batch.identity,"patchset_id":format!("{final_task}/P-01"),"submission_id":format!("{final_task}/L-01"),"entries":entries});
        state.head = batch.identity["revision_snapshot_id"]
            .as_str()
            .unwrap()
            .into();
        state.batches[index].accepted = Some(accepted.clone());
        if let Some(code) = state.cleanup_error {
            return (
                if code.ends_with("PENDING") { 503 } else { 409 },
                json!({"detail":format!("{code}: original publication is accepted; cleanup is unresolved")}),
            );
        }
        if std::mem::take(&mut state.lose_response) {
            return (408, json!({"detail":"Request timed out after commit"}));
        }
        if state.incomplete_response {
            return (
                200,
                json!({"outcome":"incomplete","expected_identity":accepted["expected_identity"],"patchset_id":accepted["patchset_id"],"submission_id":accepted["submission_id"],"entries":[]}),
            );
        }
        return (200, accepted);
    }
    if method == "GET" {
        if let Some(result) = formal_read(path, state) {
            return result;
        }
    }
    panic!("Unexpected frozen-candidate request {method} {path}");
}

fn formal_receipt(batch: &Batch) -> JsonValue {
    let accepted = batch.accepted.as_ref().unwrap();
    let task = accepted["entries"].as_array().unwrap().last().unwrap()["remote_task_id"]
        .as_str()
        .unwrap();
    json!({"repository_index":0,"task_id":task,"change_id":null,"change_ref":null,
        "submission_id":accepted["submission_id"],"patchset_id":accepted["patchset_id"],
        "status":"succeeded","target_line":"main","target_line_index":0,"mode":"direct",
        "base_snapshot_id":batch.identity["base_snapshot_id"],"revision_snapshot_id":batch.identity["revision_snapshot_id"],
        "landed_snapshot_id":batch.identity["revision_snapshot_id"],"pre_land_target_snapshot_id":batch.identity["base_snapshot_id"],
        "created_at":"2026-10-23T00:00:00Z","updated_at":"2026-10-23T00:00:00Z"})
}

fn formal_read(path: &str, state: &RemoteState) -> Option<(u16, JsonValue)> {
    let path = path.split('?').next().unwrap();
    let (kind, id) = ["/snapshots/", "/tasks/", "/patchsets/", "/lands/"]
        .into_iter()
        .find_map(|kind| path.split_once(kind).map(|(_, id)| (kind, id)))?;
    let id = id.replace("%2F", "/").replace("%2f", "/");
    if state.fail_formal_reads {
        return Some((503, json!({"detail":"Required formal read is unavailable"})));
    }
    for (index, batch) in state.batches.iter().enumerate() {
        let Some(accepted) = &batch.accepted else {
            continue;
        };
        let receipt = formal_receipt(batch);
        if kind == "/lands/" && receipt["submission_id"] == id {
            return Some((200, receipt));
        }
        if kind == "/patchsets/" && accepted["patchset_id"] == id {
            let entries = if state.omit_membership {
                Vec::new()
            } else {
                batch.manifest["history_entries"].as_array().unwrap().iter().map(|source| {
                    json!({"local_task_id":source["local_task_id"],"task_id":state.mappings[source["local_task_id"].as_str().unwrap()],
                        "pre_land_target_snapshot_id":source["pre_land_target_snapshot_id"],"landed_snapshot_id":source["landed_snapshot_id"],"landed_at_s":source["landed_at_s"]})
                }).collect()
            };
            let key = format!(
                "workflow-package:{}:{}",
                if state.corrupt_key {
                    "00".repeat(32)
                } else {
                    batch.identity["package_hash"].as_str().unwrap().to_string()
                },
                index
            );
            let manifest = json!({"contract":"ait-history-promotion/v3","idempotency_key":key,"target_line":"main",
                "base_snapshot_id":batch.identity["base_snapshot_id"],"revision_snapshot_id":batch.identity["revision_snapshot_id"],
                "entries":entries,"aggregate":{"task_id":receipt["task_id"],"patchset_id":id,"patchset_index":index}});
            return Some((
                200,
                json!({"repository_index":0,"task_id":receipt["task_id"],"patchset_id":id,"patchset_index":index,
                "ci_worker_job_index":index,"base_snapshot_id":batch.identity["base_snapshot_id"],"revision_snapshot_id":batch.identity["revision_snapshot_id"],
                "author_mode":batch.manifest["author_mode"],"source_kind":"history_promotion_aggregate","summary":format!("ait-history-promotion/v1 {manifest}")}),
            ));
        }
        for source in batch.manifest["history_entries"].as_array().unwrap() {
            let remote = &state.mappings[source["local_task_id"].as_str().unwrap()];
            if kind == "/tasks/" && remote == &id {
                return Some((
                    200,
                    json!({"repository_index":0,"task_id":id,"status":"completed",
                    "current_patchset_id":format!("{id}/P-01"),"successful_land":if receipt["task_id"] == id {receipt.clone()}else{JsonValue::Null}}),
                ));
            }
            if kind == "/snapshots/" {
                for snapshot in source["snapshots"].as_array().unwrap() {
                    if snapshot["snapshot_id"] == id {
                        let change = snapshot["local_change_ref"]
                            .as_str()
                            .map(|local| {
                                accepted["entries"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .find(|entry| entry["local_task_id"] == source["local_task_id"])
                                    .unwrap()["changes"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .find(|change| change["local_change_ref"] == local)
                                    .unwrap()["remote_change_ref"]
                                    .clone()
                            })
                            .unwrap_or(JsonValue::Null);
                        let metadata = batch.manifest["content"]["snapshots"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|row| row["snapshot_id"] == id)
                            .unwrap();
                        // The native HTTP route projects repo_id to repository_index
                        // before appending the governed workflow owner.
                        let mut value = json!({"repo_name":"fixture","repository_index":0,"snapshot_id":id,
                            "parent_snapshot_ids":metadata["parent_snapshot_ids"],
                            "manifest_hash":metadata["manifest_hash"],"line_name":metadata["line_name"],
                            "capture_binding":null,
                            "workflow_owner":{"task_id":remote,"change_ref":change}});
                        if let Some(overrides) = &state.snapshot_authority_override {
                            for (field, replacement) in overrides.as_object().unwrap() {
                                value[field] = replacement.clone();
                            }
                        }
                        if state.conflicting_snapshot_metadata {
                            value["manifest_hash"] = json!("conflicting immutable Snapshot");
                        }
                        return Some((200, value));
                    }
                }
            }
        }
    }
    Some((
        404,
        json!({"detail":if kind == "/snapshots/" {format!("Unknown snapshot {id} for repository fixture")}else{format!("Unknown formal resource {id}")}}),
    ))
}

pub struct Fixture {
    _temp: tempfile::TempDir,
    pub root: PathBuf,
    pub scratch: PathBuf,
    pub base: String,
    pub state: Arc<Mutex<RemoteState>>,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    pub fn new() -> Self {
        Self::with_origin("fixture", None)
    }

    pub fn with_server(url: &str) -> Self {
        Self::with_origin("ait-core", Some(url))
    }

    fn with_origin(repo_name: &str, origin: Option<&str>) -> Self {
        // Validate a supplied binary before this fixture's in-process init and
        // baseline Snapshot authoring, not only before its first CLI command.
        let _ = cli_binary::selected(env!("CARGO_BIN_EXE_ait-cli"));
        let temp = tempfile::tempdir().unwrap();
        let scratch = temp.path().canonicalize().unwrap();
        let root = scratch.join("repo");
        fs::create_dir(&root).unwrap();
        init_repo(&InitRequest {
            root: root.clone(),
            name: Some(repo_name.into()),
            default_line: "main".into(),
            policy_profile: "prototype".into(),
            default_author_mode: MODE.into(),
            default_model: None,
            repair_existing: false,
        })
        .unwrap();
        let path = root.join(".ait/config.json");
        let mut config: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for (key, value) in [
            ("repository_index", json!(0)),
            ("user_name", json!("Fixture initiator")),
            ("user_email", json!("fixture@example.test")),
            ("agent_teams", json!(false)),
            ("sprint", json!("off")),
            ("plan_task_binding", json!({"mode":"off"})),
            ("task_review", json!(false)),
        ] {
            config[key] = value;
        }
        fs::write(path, config.to_string()).unwrap();
        let repo = RepoRuntime::discover_from_path(&root).unwrap();
        fs::write(root.join("base.txt"), "base\n").unwrap();
        let base = repo
            .local_content_maintenance_store::<1>()
            .unwrap()
            .create_snapshot_content_with_workflow_binding(
                repo_name,
                "main",
                &[],
                Some("Fixture base"),
                false,
                SnapshotAuthoringOptions::default(),
                None,
            )
            .unwrap()["snapshot_id"]
            .as_str()
            .unwrap()
            .to_string();
        repo.line_store()
            .unwrap()
            .set_line_head("main", Some(&base), "2026-10-23T00:00:00Z")
            .unwrap();
        let server = origin
            .is_none()
            .then(|| tiny_http::Server::http("127.0.0.1:0").unwrap());
        let url = origin
            .map(str::to_string)
            .unwrap_or_else(|| format!("http://{}", server.as_ref().unwrap().server_addr()));
        repo.remote_store()
            .unwrap()
            .add_remote(&RemoteAddRecord {
                name: "origin".into(),
                url,
                repo_name: Some(repo_name.into()),
                make_default: true,
                created_at: "2026-10-23T00:00:00Z".into(),
            })
            .unwrap();
        let state = Arc::new(Mutex::new(RemoteState {
            head: base.clone(),
            batches: Vec::new(),
            mappings: BTreeMap::new(),
            requests: Vec::new(),
            finishes: Vec::new(),
            uploads: Vec::new(),
            archive_missing: false,
            fail_formal_reads: false,
            omit_membership: false,
            corrupt_key: false,
            incomplete_response: false,
            accepted_reply: None,
            lose_response: false,
            cleanup_error: None,
            policy_requires_human: false,
            conflicting_snapshot_metadata: false,
            repository_override: None,
            snapshot_authority_override: None,
            package_execution_override: None,
            job_override: None,
            pending_execution_reads: 0,
            job_read_status: None,
            reset_jobs: BTreeSet::new(),
            reset_mode_available: false,
            replacements: BTreeMap::new(),
            reprepare_requests: Vec::new(),
            lose_reprepare_response: false,
            reprepare_response_override: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let shared = state.clone();
        let stopped = stop.clone();
        let handle = server.map(|server| {
            thread::spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    let Some(mut request) = server.recv_timeout(Duration::from_millis(25)).unwrap()
                    else {
                        continue;
                    };
                    let method = request.method().as_str().to_string();
                    let path = request.url().to_string();
                    let header = |name: &str| {
                        request
                            .headers()
                            .iter()
                            .find(|header| {
                                header.field.as_str().as_str().eq_ignore_ascii_case(name)
                            })
                            .map(|header| header.value.as_str().to_string())
                    };
                    let hash = header("X-Ait-Package-Sha256");
                    let actor = header("X-Ait-Actor-Key-Index");
                    let selected_job = header("X-Ait-Worker-Job-Index");
                    let mut bytes = Vec::new();
                    request.as_reader().read_to_end(&mut bytes).unwrap();
                    let (status, body) = response(
                        &method,
                        &path,
                        &bytes,
                        hash.as_deref(),
                        actor.as_deref(),
                        selected_job.as_deref(),
                        &mut shared.lock().unwrap(),
                    );
                    request
                        .respond(
                            tiny_http::Response::from_string(body.to_string())
                                .with_status_code(status),
                        )
                        .unwrap();
                }
            })
        });
        Self {
            _temp: temp,
            root,
            scratch,
            base,
            state,
            stop,
            server: handle,
        }
    }

    pub fn repo(&self) -> RepoRuntime {
        RepoRuntime::discover_from_path(&self.root).unwrap()
    }
    pub fn snapshots(&self) -> Vec<SnapshotRecord> {
        self.repo()
            .local_snapshot_operation_store::<1>(&self.root)
            .unwrap()
            .list_line_snapshots()
            .unwrap()
    }
    pub fn task(&self, id: &str) -> JsonValue {
        TaskStore::get_task(&self.repo().task_store().unwrap(), id).unwrap()
    }
    pub fn head(&self) -> String {
        self.repo()
            .line_store()
            .unwrap()
            .line_by_name("main")
            .unwrap()
            .unwrap()
            .head_snapshot_id
            .unwrap()
    }
    pub fn start(&self, label: &str) -> (String, PathBuf) {
        let edit = self.scratch.join(label);
        let started = cli(
            &self.root,
            &[
                "task",
                "start",
                "--local",
                "--title",
                label,
                "--intent",
                label,
                "--edit-root",
                edit.to_str().unwrap(),
                "--json",
            ],
        );
        (started["task_id"].as_str().unwrap().into(), edit)
    }
    pub fn finish_local(&self, id: &str, edit: &Path) -> String {
        fixture_phase("local_finish.initial_cli.begin");
        let output = run(
            edit,
            &[
                "task",
                "finish",
                id,
                "--message",
                "Fixture Local result",
                "--local",
                "--json",
                "--full",
            ],
        );
        fixture_phase("local_finish.initial_cli.returned");
        #[cfg(windows)]
        let output = self.resume_empty_local_closeout(id, edit, output);
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let finished = output_json(&output);
        assert_eq!(finished["apply_status"], "done");
        assert!(matches!(
            finished["closeout_status"].as_str(),
            Some("complete" | "complete_unbound")
        ));
        assert_eq!(
            finished["bound_worktree_cleanup"]["status"], "removed",
            "{finished}"
        );
        assert!(!edit.exists());
        fixture_phase("local_finish.complete");
        finished["landed_snapshot_id"].as_str().unwrap().into()
    }

    #[cfg(windows)]
    fn resume_empty_local_closeout(&self, id: &str, edit: &Path, output: Output) -> Output {
        let first = output_json(&output);
        let expected_command = format!("ait task finish {id} --local");
        if output.status.success()
            || first["apply_status"] != "done"
            || first["task_status"] != "completed"
            || first["local_finish"]["status"] != "succeeded"
            || first["bound_worktree_cleanup"]["status"] != "failed"
            || first["closeout_status"] != "partial"
            || first["closeout_recovery"]["code"] != "resume_task_land_closeout"
            || first["closeout_recovery"]["idempotent"] != true
            || first["closeout_recovery"]["command"] != expected_command
        {
            return output;
        }
        let empty_or_absent = match fs::read_dir(edit) {
            Ok(mut entries) => entries.next().is_none(),
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        };
        if !empty_or_absent {
            return output;
        }
        // The first CLI has exited, releasing its cwd handle. Only an empty or
        // absent root may use the existing receipt-authorized closeout path.
        fixture_phase("local_finish.closeout_resume.begin");
        let resumed = run(
            &self.root,
            &["task", "finish", id, "--local", "--json", "--full"],
        );
        fixture_phase("local_finish.closeout_resume.returned");
        let finished = output_json(&resumed);
        assert_eq!(finished["local_finish"], first["local_finish"]);
        assert!(finished["auto_snapshot"].is_null());
        fixture_phase("local_finish.closeout_resume.verified");
        resumed
    }
    pub fn initial_batch(&self) -> (Vec<String>, String) {
        let (a, edit) = self.start("first");
        fs::write(edit.join("first.txt"), "first\n").unwrap();
        self.finish_local(&a, &edit);
        let (b, edit) = self.start("second");
        fs::write(edit.join("second.txt"), "second\n").unwrap();
        let fork = cli(
            &edit,
            &[
                "snapshot",
                "create",
                &b,
                "--message",
                "Reviewed initial fixture work",
                "--json",
            ],
        )["snapshot_id"]
            .as_str()
            .unwrap()
            .to_string();
        cli(
            &edit,
            &[
                "change",
                "create",
                &b,
                "--title",
                "Fixture reviewed correction",
                "--fork-snapshot",
                &fork,
                "--local",
                "--json",
            ],
        );
        fs::write(edit.join("revision.txt"), "revision\n").unwrap();
        let final_id = self.finish_local(&b, &edit);
        (vec![a, b], final_id)
    }
    pub fn ready(&self, id: &str, identity: Option<&Path>) -> JsonValue {
        let mut args = vec![
            "workflow",
            "ready",
            id,
            "--apply",
            "--remote",
            "origin",
            "--author-mode",
            MODE,
            "--json",
            "--full",
        ];
        if let Some(path) = identity {
            args.extend(["--expected-identity", path.to_str().unwrap()]);
        }
        cli(&self.root, &args)
    }
    pub fn identity(&self, value: &JsonValue, name: &str) -> PathBuf {
        let path = self.scratch.join(name);
        fs::write(&path, value.to_string()).unwrap();
        path
    }
    pub fn finish_output(&self, id: &str, identity: &Path) -> Output {
        run(
            &self.root,
            &[
                "workflow",
                "finish",
                id,
                "--apply",
                "--remote",
                "origin",
                "--expected-identity",
                identity.to_str().unwrap(),
                "--reviewer",
                "Faraday",
                "--review-message",
                REVIEW,
                "--json",
                "--full",
            ],
        )
    }
    pub fn finish(&self, id: &str, identity: &Path) -> JsonValue {
        let output = self.finish_output(id, identity);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        output_json(&output)
    }
    pub fn foreign(&self) -> String {
        self.repo()
            .local_content_maintenance_store::<1>()
            .unwrap()
            .create_snapshot_content_with_workflow_binding(
                "fixture",
                "main",
                std::slice::from_ref(&self.base),
                Some("Competing Remote target"),
                false,
                SnapshotAuthoringOptions::default(),
                None,
            )
            .unwrap()["snapshot_id"]
            .as_str()
            .unwrap()
            .into()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(server) = self.server.take() {
            server.join().unwrap();
        }
    }
}
