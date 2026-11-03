use super::*;
use ait_core::change_store::ChangeStore;
use ait_core::plan_http_client::{
    build_get_land_request_spec, build_get_line_request_spec, build_get_patchset_request_spec,
    build_get_remote_snapshot_request_spec, build_get_task_request_spec,
    build_list_patchsets_request_spec, PlanHttpRequestSpec,
};
use ait_core::server_operational::{RepositoryIndex, WorkerJobIndex, WorkerJobKey};
use ait_core::workflow_package::{PackageCompletionOutcome, PreparedPackageExecution};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[path = "../../../../../tests/support/cli_binary.rs"]
mod cli_binary;
#[path = "../../../../../tests/support/native_home.rs"]
mod native_home;

struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn path(spec: PlanHttpRequestSpec) -> String {
    let (_, address) = spec.url.split_once("://").unwrap();
    address[address.find('/').unwrap()..].into()
}

fn completed(
    fixture: &Fixture,
    name: &str,
    parent: &str,
    file: &str,
    with_change: bool,
    checkpoint: bool,
) -> (String, String) {
    let tasks = fixture.repo.task_store().unwrap();
    let namespace = fixture.repo.id_namespace_prefix();
    let task = TaskStore::create_task(
        &tasks,
        &fixture.repo.repo_name(),
        name,
        "Preserve source cutoff",
        Some(&namespace),
        None,
        None,
        None,
    )
    .unwrap();
    let id = required_string_field(&task, "task_id").unwrap();
    let line = format!("feature/{name}");
    fixture
        .repo
        .line_store()
        .unwrap()
        .create_line(&line, Some(parent), "100")
        .unwrap();
    fs::write(fixture.temp.path().join(file), name).unwrap();
    if with_change {
        ChangeStore::create_change(
            &fixture.repo.change_store().unwrap(),
            &id,
            &fixture.repo.repo_name(),
            "Original revision",
            &line,
            Some(&namespace),
            Some(parent),
        )
        .unwrap();
    }
    let change = with_change.then(|| format!("{id}/C-01"));
    let owner = SnapshotWorkflowBinding {
        task_id: id.clone(),
        change_id: if with_change {
            "C-01".into()
        } else {
            String::new()
        },
        line_name: line.clone(),
    };
    let content = fixture
        .repo
        .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .unwrap();
    let checkpoint_parent = if checkpoint {
        let row = content
            .create_snapshot_content_with_workflow_binding(
                &fixture.repo.repo_name(),
                &line,
                &[parent.into()],
                Some("Original checkpoint"),
                true,
                SnapshotAuthoringOptions::default(),
                Some((&owner, &namespace)),
            )
            .unwrap();
        required_string_field(&row, "snapshot_id").unwrap()
    } else {
        parent.into()
    };
    if checkpoint {
        fs::write(fixture.temp.path().join(file), format!("{name} result")).unwrap();
    }
    let snapshot = content
        .create_snapshot_content_with_workflow_binding(
            &fixture.repo.repo_name(),
            &line,
            &[checkpoint_parent],
            Some(name),
            true,
            SnapshotAuthoringOptions::default(),
            Some((&owner, &namespace)),
        )
        .unwrap();
    let snapshot = required_string_field(&snapshot, "snapshot_id").unwrap();
    tasks
        .finish_local_task(
            &id,
            &snapshot,
            "main",
            Some(parent),
            change.as_deref(),
            None,
        )
        .unwrap();
    (id, snapshot)
}

struct ReplayFixture {
    local: Fixture,
    prefix: Vec<String>,
    pending: String,
    original: String,
    rows: BTreeMap<String, JsonValue>,
    receipt_paths: Vec<String>,
    checkpoint_path: String,
    task_path: String,
    server: Option<tiny_http::Server>,
    remote: RemoteRow,
}

impl ReplayFixture {
    fn new(conflict: bool) -> Self {
        Self::with_local(fixture(), conflict, None)
    }

    fn with_local(mut local: Fixture, conflict: bool, native: Option<&RemoteRow>) -> Self {
        fs::remove_file(local.temp.path().join("remote.txt")).unwrap();
        fs::write(local.temp.path().join("feature.txt"), "feature").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("feature.txt", local.temp.path().join("feature-link")).unwrap();
        let first = local.id.clone();
        if native.is_some() {
            write_native_ci(local.temp.path());
        }
        let (second, second_snapshot) =
            completed(&local, "L2", &local.original, "l2.txt", false, true);
        let (pending, original) = completed(&local, "L3", &second_snapshot, "l3.txt", true, false);
        set_local_line_head(&local.repo, "main", Some(&original)).unwrap();
        let tasks = local.repo.task_store().unwrap();
        let namespace = ait_core::workflow_primitives::workflow_origin_namespace_prefix(
            "R",
            Some(&local.repo.id_namespace_prefix()),
        )
        .unwrap();
        let prefix = vec![first, second];
        let mappings = prefix
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let mapped = format!("{namespace}T-{:04}", 1001 + index);
                if native.is_none() {
                    TaskStore::mark_task_published(&tasks, id, Some("origin"), Some(&mapped))
                        .unwrap();
                }
                mapped
            })
            .collect::<Vec<_>>();
        fs::remove_file(local.temp.path().join("l3.txt")).unwrap();
        fs::write(local.temp.path().join("bugfix.txt"), "reviewed correction").unwrap();
        if conflict {
            fs::write(
                local.temp.path().join("l3.txt"),
                "conflicting Remote result",
            )
            .unwrap();
        }
        let content = local
            .repo
            .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .unwrap();
        let target = content
            .create_snapshot_content_with_workflow_binding(
                &local.repo.repo_name(),
                "main",
                std::slice::from_ref(&local.base),
                Some("Accepted correction R"),
                false,
                SnapshotAuthoringOptions::default(),
                None,
            )
            .unwrap();
        local.target = required_string_field(&target, "snapshot_id").unwrap();
        local.id = pending.clone();
        local.entry =
            workflow_completed_local_entry_if_present(&local.repo, &pending, Some("origin"))
                .unwrap()
                .unwrap();
        if let Some(remote) = native {
            return Self {
                local,
                prefix,
                pending,
                original,
                remote: remote.clone(),
                rows: BTreeMap::new(),
                receipt_paths: Vec::new(),
                checkpoint_path: String::new(),
                task_path: String::new(),
                server: None,
            };
        }
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let remote = RemoteRow {
            url: format!("http://{}", server.server_addr()),
            ..local.remote.clone()
        };
        let config = http_config(&local.repo, &remote);
        let mut rows = BTreeMap::new();
        let mut receipt_paths = Vec::new();
        let mut checkpoint_path = String::new();
        let mut task_path = String::new();
        for (id, mapped) in prefix.iter().zip(&mappings) {
            let entry = workflow_completed_local_entry_if_present(&local.repo, id, Some("origin"))
                .unwrap()
                .unwrap();
            let receipt = &entry["state"]["local_finish"];
            let endpoint = required_string_field(receipt, "landed_snapshot_id").unwrap();
            let patchset = format!("{mapped}/P-01");
            let summary = json!({"contract":"ait-local-land-receipt/v2","local_task_id":id,
                "local_change_id":null,"local_change_ref":null,"task_id":mapped,"change_ref":null,
                "target_line":"main","pre_land_target_snapshot_id":receipt["pre_land_target_snapshot_id"],
                "landed_snapshot_id":endpoint,"landed_at_s":snapshot_seconds(receipt["updated_at"].as_str().unwrap()).unwrap(),
                "source_kind":"imported_local_land_receipt"});
            let p = path(
                build_get_patchset_request_spec(&config, &patchset, Some("fixture"), None).unwrap(),
            );
            rows.insert(p.clone(), json!({"repository_index":0,"task_id":mapped,"patchset_id":patchset,"change_ref":null,
                "source_kind":"imported_local_land_receipt","summary":format!("ait-local-land-receipt/v1 {summary}"),
                "base_snapshot_id":receipt["pre_land_target_snapshot_id"],"revision_snapshot_id":endpoint,
                "created_at":receipt["updated_at"],"ci_run_seq":0}));
            receipt_paths.push(p);
            rows.insert(
                path(build_list_patchsets_request_spec(&config, mapped, Some("fixture")).unwrap()),
                json!([{"patchset_id":patchset}]),
            );
            for (source, _) in tasks.completed_task_source_snapshots(id).unwrap() {
                let mut snapshot = snapshot_show(&local.repo, &source).unwrap();
                snapshot["repository_index"] = json!(0);
                snapshot["workflow_owner"] = json!({"task_id":mapped,"change_ref":null});
                let p = path(
                    build_get_remote_snapshot_request_spec(
                        &config, "fixture", &source, false, None,
                    )
                    .unwrap(),
                );
                if source != endpoint {
                    checkpoint_path = p.clone();
                }
                rows.insert(p, snapshot);
            }
            let p = path(build_get_task_request_spec(&config, mapped, Some("fixture")).unwrap());
            rows.insert(
                p.clone(),
                json!({"repository_index":0,"task_id":mapped,"status":"completed"}),
            );
            task_path = p;
        }
        let mapped = mappings.last().unwrap();
        let submission = format!("{mapped}/L-01");
        let patchset = format!("{mapped}/P-09");
        let accepted = json!({"repository_index":0,"task_id":mapped,"submission_id":submission,"patchset_id":patchset,
            "status":"succeeded","target_line":"main","base_snapshot_id":local.base,
            "revision_snapshot_id":local.target,"landed_snapshot_id":local.target,
            "pre_land_target_snapshot_id":local.base,"mode":"direct","change_id":null,"change_ref":null});
        rows.get_mut(&task_path).unwrap()["successful_land"] = accepted.clone();
        rows.insert(
            path(build_get_land_request_spec(&config, &submission, Some("fixture")).unwrap()),
            accepted,
        );
        rows.insert(path(build_get_patchset_request_spec(&config, &patchset, Some("fixture"), None).unwrap()),
            json!({"repository_index":0,"task_id":mapped,"patchset_id":patchset,"base_snapshot_id":local.base,"revision_snapshot_id":local.target}));
        rows.insert(
            path(build_get_line_request_spec(&config, "fixture", "main").unwrap()),
            json!({"line_name":"main","head_snapshot_id":local.target}),
        );
        Self {
            local,
            prefix,
            pending,
            original,
            rows,
            receipt_paths,
            checkpoint_path,
            task_path,
            server: Some(server),
            remote,
        }
    }

    fn run(&self, work: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                while !stopped.load(Ordering::SeqCst) {
                    let Some(request) = self
                        .server
                        .as_ref()
                        .unwrap()
                        .recv_timeout(Duration::from_millis(20))
                        .unwrap()
                    else {
                        continue;
                    };
                    assert_eq!(request.method().as_str(), "GET", "preflight cannot publish");
                    let response = self
                        .rows
                        .get(request.url())
                        .unwrap_or_else(|| panic!("Unexpected read {}", request.url()));
                    request
                        .respond(tiny_http::Response::from_string(response.to_string()))
                        .unwrap();
                }
            });
            let guard = StopOnDrop(stop);
            let result = work();
            drop(guard);
            handle.join().unwrap();
            result
        })
    }

    fn prepare(&self) -> Result<PreparedSource, String> {
        prepare_source_history_at_base(
            &self.local.repo,
            &self.remote,
            &self.local.entry,
            &self.local.target,
            Some("ai_only_experimental"),
            Some("fresh suffix"),
        )
    }

    /// Reuse the original receipt projection for fixture setup only. R is the
    /// controlled Remote correction; the suffix below is prepared by the CLI.
    fn corrected_prefix(&self) -> PreparedSource {
        let repo = &self.local.repo;
        let entry = workflow_completed_local_entry_if_present(
            repo,
            self.prefix.last().unwrap(),
            Some("origin"),
        )
        .unwrap()
        .unwrap();
        let mut prepared = prepare_source_history_at_base(
            repo,
            &self.remote,
            &entry,
            &self.local.base,
            Some(NATIVE_MODE),
            Some("Controlled accepted correction seed"),
        )
        .unwrap();
        let snapshots = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
                &repo.workspace_root(),
            )
            .unwrap();
        let correction = snapshot_by_id_with_snapshot_store(&snapshots, &self.local.target)
            .unwrap()
            .unwrap();
        prepared.manifest.revision_snapshot_id = correction.snapshot_id.clone();
        prepared.manifest.history_entries.last_mut().unwrap()["snapshots"]
            .as_array_mut().unwrap().push(json!({"snapshot_id":correction.snapshot_id,
                "created_at_s":snapshot_seconds(&correction.created_at).unwrap(),"local_change_ref":null}));
        prepared
            .mappings
            .last_mut()
            .unwrap()
            .snapshots
            .push(PackageSnapshotOwnerExpectation {
                snapshot_id: correction.snapshot_id.clone(),
                local_change_ref: None,
            });
        let mut ids = prepared
            .manifest
            .content
            .snapshots
            .iter()
            .map(|row| row.snapshot_id.clone())
            .collect::<Vec<_>>();
        ids.push(correction.snapshot_id);
        let plan = build_zstd_bulk_local_plan(repo, &ids, &BTreeSet::new()).unwrap();
        prepared.manifest.content = ZstdBulkCommitRequest::from_json_rows(
            None,
            None,
            ordered_object_pack_metadata(&plan).unwrap(),
            ordered_tree_pack_metadata(&plan).unwrap(),
            plan.blob_locators.values().cloned().collect(),
            plan.tree_locators.values().cloned().collect(),
            plan.snapshots.values().cloned().collect(),
            None,
        )
        .unwrap();
        let objects = plan
            .object_packs
            .iter()
            .map(|(id, pack)| (id.clone(), fs::read(&pack.pack_abs_path).unwrap()))
            .collect();
        let trees = plan
            .tree_packs
            .iter()
            .map(|(id, pack)| (id.clone(), fs::read(&pack.pack_abs_path).unwrap()))
            .collect();
        prepared.package = prepared.manifest.encode(&objects, &trees).unwrap();
        prepared
    }
}

const NATIVE_MODE: &str = "ai_only_experimental";
const NATIVE_REVIEW: &str = "Reviewed files: generated native fresh-flow fixture; Findings: controlled fixture checks passed; Risks: fixture evidence only, no actual-agent review; Tests: exact native Runner CI passed; Recommendation: pass";

fn write_native_ci(root: &Path) {
    fs::create_dir(root.join("ci")).unwrap();
    #[cfg(not(windows))]
    {
        let script = root.join("ci/run.sh");
        fs::write(&script, "#!/bin/sh\nset -eu\ntest \"$(cat bugfix.txt)\" = 'reviewed correction'\nif test -f l3.txt; then test \"$(cat l3.txt)\" = L3; fi\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(script, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    #[cfg(windows)]
    fs::write(root.join("ci/run.ps1"), r#"$ErrorActionPreference = 'Stop'
if ([System.IO.File]::ReadAllText('bugfix.txt') -cne 'reviewed correction') { throw 'Missing accepted correction' }
if ((Test-Path 'l3.txt') -and ([System.IO.File]::ReadAllText('l3.txt') -cne 'L3')) { throw 'Wrong L3 content' }
"#).unwrap();
}

fn native_command(root: &Path, args: &[&str]) -> Command {
    std::env::var_os("AIT_TEST_CLI_BIN")
        .expect("set the frozen CLI input for this ignored native case");
    let mut command = Command::new(cli_binary::selected(""));
    command
        .current_dir(root)
        .args(args)
        .env("CODEX_HOME", native_home::path())
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CODEX_SESSION_ID");
    command
}

fn native_json(output: &Output) -> JsonValue {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
    })
}

struct NativeProcess(Option<Child>);

impl NativeProcess {
    fn output(mut self, phase: &str) -> Output {
        let deadline = std::time::Instant::now() + Duration::from_secs(90);
        while self.0.as_mut().unwrap().try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                self.0.as_mut().unwrap().kill().unwrap();
                let output = self.0.take().unwrap().wait_with_output().unwrap();
                panic!(
                    "{phase} timed out\n{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.0.take().unwrap().wait_with_output().unwrap()
    }

    fn assert_running(&mut self, phase: &str) {
        if self.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            let output = self.0.take().unwrap().wait_with_output().unwrap();
            panic!(
                "{phase} exited before admission\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

impl Drop for NativeProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

/// The same explicit pairing inputs and native entrypoints as routed-resume.
/// Every lifecycle mutation below targets this disposable authority only.
struct NativePair {
    server: NativeProcess,
    client: PlanHttpClientManager,
    url: String,
    runner: PathBuf,
    data: tempfile::TempDir,
    _ram: tempfile::TempDir,
}

impl NativePair {
    fn new() -> Self {
        use ait_core::plan_http_client::PlanHttpClientConfig;
        let server =
            std::env::var_os("TEST_REVIEWED_SERVER_BIN").expect("set the reviewed native Server");
        let runner =
            std::env::var_os("TEST_NATIVE_RUNNER_BIN").expect("set the verified native Runner");
        let ram_root =
            std::env::var_os("TEST_SERVER_CI_RAM_ROOT").expect("set the native CI RAM root");
        let data = tempfile::tempdir().unwrap();
        let ram = tempfile::tempdir_in(ram_root).unwrap();
        let initialized = Command::new(&server)
            .args(["init", "--data"])
            .arg(data.path())
            .output()
            .unwrap();
        assert!(
            initialized.status.success(),
            "{}",
            String::from_utf8_lossy(&initialized.stderr)
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let server = NativeProcess(Some(
            Command::new(&server)
                .args(["--data"])
                .arg(data.path())
                .args(["--listen", &address.to_string(), "--defer-ci-admission"])
                .env("AIT_NATIVE_SERVER_CI_RAM_ROOT", ram.path())
                .env("AIT_NATIVE_SERVER_CI_RAM_MIN_AVAILABLE_BYTES", "0")
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        ));
        let url = format!("http://{address}");
        let client = PlanHttpClientManager::new(PlanHttpClientConfig {
            base_url: url.clone(),
            repository_index: Some(RepositoryIndex::new(0)),
            default_timeout_ms: 1000,
            ..Default::default()
        })
        .unwrap();
        let mut pair = Self {
            server,
            client,
            url,
            runner: runner.into(),
            data,
            _ram: ram,
        };
        for _ in 0..200 {
            pair.server.assert_running("isolated Server");
            if pair
                .client
                .get_server_health()
                .is_ok_and(|row| row["ready"] == true)
            {
                return pair;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("isolated native Server did not become ready");
    }

    fn key(index: u32) -> WorkerJobKey {
        WorkerJobKey::new(RepositoryIndex::new(0), WorkerJobIndex::new(index))
    }

    fn queued(
        &mut self,
        index: u32,
        process: &mut NativeProcess,
    ) -> PackageWorkflowExpectedIdentity {
        for _ in 0..600 {
            process.assert_running("fresh ready");
            if let Ok(row) = self.client.get_worker_job(Self::key(index)) {
                if row["package_execution"]["expected_identity"].is_object() {
                    assert_eq!(row["package_execution"]["ci_state"], "queued", "{row}");
                    return PackageWorkflowExpectedIdentity::from_value(
                        &row["package_execution"]["expected_identity"],
                    )
                    .unwrap();
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("fresh ready did not queue the expected native Job");
    }

    fn ci(&mut self, index: u32, pass: bool) {
        let output = NativeProcess(Some(
            Command::new(&self.runner)
                .args([
                    "serve",
                    "--server",
                    &self.url,
                    "--worker-id",
                    "fresh-suffix-fixture",
                    "--repository-index",
                    "0",
                    "--once",
                    "--attempt-root",
                ])
                .arg(self.data.path().join(format!("runner-{index}")))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ))
        .output("native Runner");
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let delivery = native_json(&output);
        assert_eq!(delivery["contract"], "ait.runner.delivery.v1", "{delivery}");
        assert_eq!(delivery["status"], "delivered", "{delivery}");
        assert_eq!(delivery["repository_index"], 0);
        assert_eq!(delivery["worker_job_index"], index);
        assert_eq!(delivery["terminal_operation"], "complete", "{delivery}");
        assert_eq!(
            delivery["result"]["tests_status"],
            if pass { "pass" } else { "fail" },
            "{delivery}"
        );
        assert!(delivery["result"]["suite_result_count"]
            .as_u64()
            .is_some_and(|count| count > 0));
        assert_eq!(delivery["result"]["cleanup"]["attempt_root_removed"], true);
        assert_eq!(delivery["result"]["cleanup"]["remaining_owned_paths"], 0);
        let job = self.client.get_worker_job(Self::key(index)).unwrap();
        assert_eq!(
            job["package_execution"]["ci_state"],
            if pass { "passed" } else { "failed" },
            "{job}"
        );
    }

    fn accept_correction(&mut self, fixture: &ReplayFixture) -> PackageCompletion {
        use ait_core::attest_json::{
            AttestJson, RemoteAttestationStatement, RemoteVerificationState,
        };
        let seed = fixture.corrected_prefix();
        let mut client = package_client(&fixture.local.repo, &fixture.remote, "ait-core").unwrap();
        let prepared = client
            .prepare_workflow_package(&seed.manifest, &seed.package, None)
            .unwrap();
        let identity = PreparedPackageExecution::from_value(&prepared)
            .unwrap()
            .expected_identity;
        assert_eq!(identity.worker_job_index, 1);
        self.ci(1, true);
        let statement = RemoteAttestationStatement {
            verification_state: RemoteVerificationState::Pass,
            ..Default::default()
        };
        let payload = AttestJson::stateless()
            .build_put_attestation_body(NATIVE_MODE, &statement)
            .unwrap();
        client
            .record_workflow_package_evidence(
                &identity,
                "attestation",
                "fixture-correction-attest",
                payload,
            )
            .unwrap();
        for action in ["code_review_summary", "task_approve"] {
            client.record_workflow_package_evidence(&identity, "review", &format!("fixture-correction-{action}"),
                json!({"reviewer":"Faraday","reviewer_kind":"bot","action":action,"comment":NATIVE_REVIEW,"blocking":false})).unwrap();
        }
        let accepted = client
            .finish_workflow_package(&identity, "main", "direct")
            .unwrap();
        let result = PackageCompletion::from_value(&accepted, &identity, &seed.mappings).unwrap();
        assert_eq!(result.outcome, PackageCompletionOutcome::Confirmed);
        let mut imported = seed.mappings;
        // R is controlled Remote-authored fixture setup. Publish only the
        // original Local fixed owners through the existing guarded writer.
        for entry in &mut imported {
            entry
                .snapshots
                .retain(|row| row.snapshot_id != fixture.local.target);
        }
        fixture
            .local
            .repo
            .task_store()
            .unwrap()
            .record_package_publication(&result, &imported)
            .unwrap();
        result
    }
}

fn start_native_ready(fixture: &ReplayFixture, task: &str) -> NativeProcess {
    NativeProcess(Some(
        native_command(
            fixture.local.temp.path(),
            &[
                "workflow",
                "ready",
                task,
                "--apply",
                "--remote",
                "origin",
                "--author-mode",
                NATIVE_MODE,
                "--json",
                "--full",
            ],
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap(),
    ))
}

#[test]
#[ignore = "requires the updated frozen CLI, reviewed native Server, native Runner and CI RAM root"]
fn accepted_correction_fresh_cli_runs_new_native_ci_and_finishes_only_the_suffix() {
    use ait_core::remote_store::{RemoteAddRecord, RemoteStore};
    // Validate the existing frozen-input selector before creating authority.
    std::env::var_os("AIT_TEST_CLI_BIN").expect("set the updated frozen CLI input");
    let _ = cli_binary::selected("");
    let mut pair = NativePair::new();
    let authority = pair
        .client
        .get_repository_by_index(RepositoryIndex::new(0))
        .unwrap();
    let namespace = authority["repository"]["namespace"].as_str().unwrap();
    let mut local = fixture_with_authority(false, "ait-core", Some(namespace));
    let path = local.temp.path().join(".ait/config.json");
    let mut config: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (key, value) in [
        ("repository_index", json!(0)),
        ("id_namespace_prefix", json!(namespace)),
        ("user_name", json!("Fixture initiator")),
        ("user_email", json!("fixture@example.test")),
        ("agent_teams", json!(false)),
        ("sprint", json!("off")),
        ("plan_task_binding", json!({"mode":"off"})),
        ("task_review", json!(false)),
        ("default_author_mode", json!(NATIVE_MODE)),
    ] {
        config[key] = value;
    }
    fs::write(path, config.to_string()).unwrap();
    local.repo = RepoRuntime::discover_from_path(local.temp.path()).unwrap();
    let remote = RemoteRow {
        name: "origin".into(),
        url: pair.url.clone(),
        repo_name: Some("ait-core".into()),
    };
    local
        .repo
        .remote_store()
        .unwrap()
        .add_remote(&RemoteAddRecord {
            name: remote.name.clone(),
            url: remote.url.clone(),
            repo_name: remote.repo_name.clone(),
            make_default: true,
            created_at: "2026-10-23T00:00:00Z".into(),
        })
        .unwrap();
    let fixture = ReplayFixture::with_local(local, false, Some(&remote));
    let repo = &fixture.local.repo;
    let tasks = repo.task_store().unwrap();
    let local_receipts = tasks.list_completed_tasks_with_landed_changes().unwrap();
    let original_sources = tasks
        .completed_task_source_snapshots(&fixture.pending)
        .unwrap();
    let mut task_remote = http_task_remote(repo, &fixture.remote).unwrap();
    assert!(
        workflow_remote_line_head(&pair.client.get_line("ait-core", "main").unwrap(), "main",)
            .unwrap()
            .is_none()
    );
    workflow_initialize_null_remote_base_with_task_remote(
        repo,
        &fixture.remote,
        &mut task_remote,
        "ait-core",
        "main",
        &fixture.local.base,
    )
    .unwrap();
    assert_eq!(
        local_line_head_snapshot_id(repo, "main")
            .unwrap()
            .as_deref(),
        Some(fixture.original.as_str())
    );

    // The old prefix really fails native CI while L3 is already completed.
    let mut failed_ready = start_native_ready(&fixture, fixture.prefix.last().unwrap());
    let failed_identity = pair.queued(0, &mut failed_ready);
    pair.ci(0, false);
    let failed = failed_ready.output("failed prefix ready");
    let failed = native_json(&failed);
    assert_eq!(failed["next_action"]["code"], "ci_failed", "{failed}");
    assert_eq!(failed["apply_status"], "stopped", "{failed}");
    assert_eq!(
        PackageWorkflowExpectedIdentity::from_value(&failed["expected_identity"]).unwrap(),
        failed_identity
    );
    assert_eq!(failed_identity.base_snapshot_id, fixture.local.base);
    let (_, failed_sources) = recover_source_history(
        repo,
        fixture.prefix.last().unwrap(),
        "origin",
        &failed_identity,
    )
    .unwrap();
    assert_eq!(
        failed_sources
            .iter()
            .map(|row| row.local_task_id.clone())
            .collect::<Vec<_>>(),
        fixture.prefix
    );
    assert_eq!(
        workflow_remote_line_head(&pair.client.get_line("ait-core", "main").unwrap(), "main")
            .unwrap()
            .as_deref(),
        Some(fixture.local.base.as_str())
    );
    for id in &fixture.prefix {
        assert!(TaskStore::get_task(&tasks, id).unwrap()["published_task_id"].is_null());
    }
    let failed_job = pair.client.get_worker_job(NativePair::key(0)).unwrap();

    // This existing-format seed is fixture setup, with its own native CI and
    // controlled governance. It never changes the old failed sealed identity.
    let prefix = pair.accept_correction(&fixture);
    assert_eq!(
        workflow_remote_line_head(&pair.client.get_line("ait-core", "main").unwrap(), "main")
            .unwrap()
            .as_deref(),
        Some(fixture.local.target.as_str())
    );
    let prefix_local = fixture
        .prefix
        .iter()
        .map(|id| TaskStore::get_task(&tasks, id).unwrap())
        .collect::<Vec<_>>();
    let prefix_remote = prefix
        .entries
        .iter()
        .map(|row| {
            pair.client
                .get_task(&row.remote_task_id, Some("ait-core"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    for original in &local_receipts {
        let current = TaskStore::get_task(&tasks, original["task_id"].as_str().unwrap()).unwrap();
        assert_eq!(current["local_finish"], original["local_finish"]);
    }

    // Restore the known Local bytes, then invoke the normal fresh CLI path.
    fs::remove_file(fixture.local.temp.path().join("bugfix.txt")).unwrap();
    fs::write(fixture.local.temp.path().join("l3.txt"), "L3").unwrap();
    let mut fresh = start_native_ready(&fixture, &fixture.pending);
    let identity = pair.queued(2, &mut fresh);
    assert_eq!(identity.base_snapshot_id, fixture.local.target);
    assert_ne!(identity.revision_snapshot_id, fixture.original);
    assert_ne!(identity.package_hash, failed_identity.package_hash);
    let (target, selected) =
        recover_source_history(repo, &fixture.pending, "origin", &identity).unwrap();
    assert_eq!(target, "main");
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].local_task_id, fixture.pending);
    assert_eq!(selected[0].snapshots.len(), 2);
    assert_eq!(selected[0].changes.len(), 1);
    let effective = source_files(repo, &identity.revision_snapshot_id).unwrap();
    for path in ["bugfix.txt", "l3.txt", "l2.txt", "feature.txt"] {
        assert!(effective.contains_key(path), "missing {path}");
    }
    assert!(!effective.contains_key("l4.txt"));

    // L4 finishes after the exact native Job pins L3's source cutoff.
    let (later, later_snapshot) = completed(
        &fixture.local,
        "L4",
        &fixture.original,
        "l4.txt",
        false,
        false,
    );
    set_local_line_head(repo, "main", Some(&later_snapshot)).unwrap();
    let later_before = TaskStore::get_task(&tasks, &later).unwrap();
    let files_before = fs::read(fixture.local.temp.path().join("l4.txt")).unwrap();
    pair.ci(2, true);
    let ready_output = fresh.output("fresh suffix ready");
    assert!(
        ready_output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&ready_output.stdout),
        String::from_utf8_lossy(&ready_output.stderr)
    );
    let ready = native_json(&ready_output);
    assert_eq!(ready["next_action"]["code"], "done", "{ready}");
    assert_eq!(
        PackageWorkflowExpectedIdentity::from_value(&ready["expected_identity"]).unwrap(),
        identity
    );
    let pin = tempfile::NamedTempFile::new().unwrap();
    fs::write(pin.path(), serde_json::to_vec(&identity).unwrap()).unwrap();
    let output = NativeProcess(Some(
        native_command(
            fixture.local.temp.path(),
            &[
                "workflow",
                "finish",
                &fixture.pending,
                "--apply",
                "--remote",
                "origin",
                "--expected-identity",
                pin.path().to_str().unwrap(),
                "--reviewer",
                "Faraday",
                "--review-message",
                NATIVE_REVIEW,
                "--json",
                "--full",
            ],
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap(),
    ))
    .output("fresh suffix finish");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let finished = native_json(&output);
    assert_eq!(finished["next_action"]["code"], "done", "{finished}");
    assert_eq!(finished["closeout_status"], "complete", "{finished}");
    let accepted =
        PackageCompletion::from_value(&finished["completion"], &identity, &selected).unwrap();
    assert_eq!(accepted.outcome, PackageCompletionOutcome::Confirmed);
    assert_eq!(accepted.entries.len(), 1);
    assert_eq!(accepted.entries[0].local_task_id, fixture.pending);
    let mapping = &accepted.entries[0];
    for snapshot in &selected[0].snapshots {
        let response = pair
            .client
            .get_remote_snapshot("ait-core", &snapshot.snapshot_id, false, None)
            .unwrap();
        let row = ait_core::workflow_package::package_response_data(&response).unwrap();
        assert_eq!(row["workflow_owner"]["task_id"], mapping.remote_task_id);
        assert_eq!(
            row["workflow_owner"]["change_ref"],
            mapping.changes[0].remote_change_ref
        );
    }
    assert_eq!(
        TaskStore::get_task(&tasks, &fixture.pending).unwrap()["published_task_id"],
        mapping.remote_task_id
    );
    assert_eq!(
        ChangeStore::get_change(
            &repo.change_store().unwrap(),
            &format!("{}/C-01", fixture.pending)
        )
        .unwrap()["published_change_id"],
        mapping.changes[0].remote_change_ref
    );
    assert_eq!(
        workflow_remote_line_head(&pair.client.get_line("ait-core", "main").unwrap(), "main")
            .unwrap()
            .as_deref(),
        Some(identity.revision_snapshot_id.as_str())
    );
    assert_eq!(
        tasks
            .completed_task_source_snapshots(&fixture.pending)
            .unwrap(),
        original_sources
    );
    assert_eq!(TaskStore::get_task(&tasks, &later).unwrap(), later_before);
    assert_eq!(
        local_line_head_snapshot_id(repo, "main").unwrap(),
        Some(later_snapshot)
    );
    assert_eq!(
        fs::read(fixture.local.temp.path().join("l4.txt")).unwrap(),
        files_before
    );
    for (id, before) in fixture.prefix.iter().zip(prefix_local) {
        assert_eq!(TaskStore::get_task(&tasks, id).unwrap(), before);
    }
    for (mapping, before) in prefix.entries.iter().zip(prefix_remote) {
        let after = pair
            .client
            .get_task(&mapping.remote_task_id, Some("ait-core"))
            .unwrap();
        for field in [
            "task_id",
            "title",
            "intent",
            "status",
            "current_patchset_id",
            "successful_land",
        ] {
            assert_eq!(
                after[field], before[field],
                "published prefix changed {field}"
            );
        }
    }
    let after_failed = pair.client.get_worker_job(NativePair::key(0)).unwrap();
    assert_eq!(after_failed["job"], failed_job["job"]);
    assert_eq!(
        after_failed["package_execution"]["expected_identity"],
        failed_job["package_execution"]["expected_identity"]
    );
    assert_eq!(after_failed["package_execution"]["ci_state"], "failed");
    assert_eq!(
        pair.client
            .list_worker_jobs(RepositoryIndex::new(0), None, 100)
            .unwrap()["count"],
        3
    );
    assert_eq!(
        tasks
            .list_completed_tasks_with_landed_changes()
            .unwrap()
            .len(),
        4,
        "no integration Task"
    );
}

#[test]
fn accepted_correction_replays_only_frozen_suffix_and_recovers_exact_retry() {
    let fixture = ReplayFixture::new(false);
    let repo = &fixture.local.repo;
    let tasks = repo.task_store().unwrap();
    let before = tasks.list_completed_tasks_with_landed_changes().unwrap();
    let original_sources = tasks
        .completed_task_source_snapshots(&fixture.pending)
        .unwrap();
    // L4 finishes after the selected L3 cutoff, before preparation reads history.
    fs::remove_file(fixture.local.temp.path().join("bugfix.txt")).unwrap();
    fs::write(fixture.local.temp.path().join("l3.txt"), "L3").unwrap();
    let (later, later_snapshot) = completed(
        &fixture.local,
        "L4",
        &fixture.original,
        "l4.txt",
        false,
        false,
    );
    set_local_line_head(repo, "main", Some(&later_snapshot)).unwrap();
    let before_later = TaskStore::get_task(&tasks, &later).unwrap();
    let before_files = fs::read(fixture.local.temp.path().join("l4.txt")).unwrap();
    fixture
        .run(|| {
            let prepared = prepare_source_history(
                repo,
                &fixture.remote,
                &fixture.pending,
                Some("ai_only_experimental"),
                Some("fresh suffix"),
            )?;
            assert_eq!(prepared.manifest.base_snapshot_id, fixture.local.target);
            assert_ne!(prepared.manifest.revision_snapshot_id, fixture.original);
            assert_eq!(prepared.manifest.history_entries.len(), 1);
            assert_eq!(prepared.mappings[0].local_task_id, fixture.pending);
            assert!(prepared.mappings[0].remote_task_id.is_none());
            assert_eq!(prepared.mappings[0].snapshots.len(), 2);
            assert_eq!(prepared.mappings[0].changes.len(), 1);
            let change = format!("{}/C-01", fixture.pending);
            assert_eq!(prepared.mappings[0].changes[0].local_change_ref, change);
            assert!(prepared.mappings[0]
                .snapshots
                .iter()
                .all(|row| row.local_change_ref.as_deref() == Some(change.as_str())));
            let entry = &prepared.manifest.history_entries[0];
            assert_eq!(entry["landed_snapshot_id"], fixture.original);
            assert_eq!(
                entry["pre_land_target_snapshot_id"],
                fixture.local.entry["state"]["local_finish"]["pre_land_target_snapshot_id"]
            );
            let effective = source_files(repo, &prepared.manifest.revision_snapshot_id)?;
            for file in ["feature.txt", "l2.txt", "l3.txt", "bugfix.txt"] {
                assert!(effective.contains_key(file), "missing {file}");
            }
            assert!(!effective.contains_key("l4.txt"));
            let owners = tasks
                .snapshot_ownership_rows(std::slice::from_ref(
                    &prepared.manifest.revision_snapshot_id,
                ))
                .unwrap();
            assert_eq!(owners[0]["task_id"], fixture.pending);
            assert_eq!(owners[0]["change_id"], "C-01");
            let identity = PackageWorkflowExpectedIdentity {
                repository_index: 0,
                worker_job_index: 0,
                package_hash: prepared.package.package_hash.clone(),
                base_snapshot_id: fixture.local.target.clone(),
                revision_snapshot_id: prepared.manifest.revision_snapshot_id.clone(),
            };
            assert_eq!(
                recover_source_history(repo, &fixture.pending, "origin", &identity)?.1,
                prepared.mappings
            );
            let count = repo
                .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
                    &repo.workspace_root(),
                )?
                .list_line_snapshots()?
                .len();
            let repeated = prepare_source_history(
                repo,
                &fixture.remote,
                &fixture.pending,
                Some("ai_only_experimental"),
                Some("fresh suffix"),
            )?;
            assert_eq!(prepared.package, repeated.package);
            assert_eq!(
                repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
                    &repo.workspace_root()
                )?
                .list_line_snapshots()?
                .len(),
                count
            );
            // A previous sealed original chain remains read-only after R acceptance.
            let old = PackageWorkflowExpectedIdentity {
                revision_snapshot_id: fixture.local.entry["state"]["local_finish"]
                    ["pre_land_target_snapshot_id"]
                    .as_str()
                    .unwrap()
                    .into(),
                base_snapshot_id: fixture.local.base.clone(),
                ..identity
            };
            recover_source_history(repo, fixture.prefix.last().unwrap(), "origin", &old)?;
            assert_eq!(
                repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
                    &repo.workspace_root()
                )?
                .list_line_snapshots()?
                .len(),
                count
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        tasks
            .completed_task_source_snapshots(&fixture.pending)
            .unwrap(),
        original_sources
    );
    for original in before {
        assert_eq!(
            TaskStore::get_task(&tasks, original["task_id"].as_str().unwrap()).unwrap(),
            original
        );
    }
    assert_eq!(TaskStore::get_task(&tasks, &later).unwrap(), before_later);
    assert_eq!(
        local_line_head_snapshot_id(repo, "main").unwrap(),
        Some(later_snapshot)
    );
    assert_eq!(
        fs::read(fixture.local.temp.path().join("l4.txt")).unwrap(),
        before_files
    );
}

#[test]
fn conflict_missing_foreign_and_unaccepted_prefix_hold_without_snapshot_writes() {
    for case in [
        "conflict",
        "missing",
        "missing_checkpoint",
        "foreign",
        "unaccepted",
        "contradictory",
        "advanced",
    ] {
        let mut fixture = ReplayFixture::new(case == "conflict");
        match case {
            "missing" => {
                fixture.rows.get_mut(&fixture.receipt_paths[0]).unwrap()["summary"] =
                    json!("No imported receipt");
                fixture.rows.get_mut(&fixture.receipt_paths[0]).unwrap()["source_kind"] =
                    json!("manual");
            }
            "foreign" => {
                fixture.rows.get_mut(&fixture.receipt_paths[0]).unwrap()["task_id"] =
                    json!("RT-9999");
            }
            "missing_checkpoint" => {
                fixture.rows.get_mut(&fixture.checkpoint_path).unwrap()["workflow_owner"] =
                    JsonValue::Null;
            }
            "unaccepted" => {
                fixture.rows.get_mut(&fixture.task_path).unwrap()["status"] = json!("active");
            }
            "contradictory" => {
                fixture.rows.get_mut(&fixture.task_path).unwrap()["successful_land"]
                    ["revision_snapshot_id"] = json!(fixture.original);
            }
            "advanced" => {
                let line = fixture
                    .rows
                    .values_mut()
                    .find(|row| {
                        row["line_name"].as_str() == Some("main")
                            && row.get("head_snapshot_id").is_some()
                    })
                    .unwrap();
                line["head_snapshot_id"] = json!(fixture.original);
            }
            _ => {}
        }
        let repo = &fixture.local.repo;
        let snapshots = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
                &repo.workspace_root(),
            )
            .unwrap();
        let before = snapshots.list_line_snapshots().unwrap();
        let tasks = repo.task_store().unwrap();
        let receipts = tasks.list_completed_tasks_with_landed_changes().unwrap();
        let owners = tasks
            .snapshot_ownership_rows(
                &before
                    .iter()
                    .map(|row| row.snapshot_id.clone())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        fixture
            .run(|| {
                let error = fixture
                    .prepare()
                    .err()
                    .ok_or("invalid proof unexpectedly prepared")?;
                let expected = match case {
                    "conflict" => "Fresh history replay held",
                    "missing" => "lacks its exact original Remote receipt",
                    "missing_checkpoint" => "Snapshot mapping is incomplete",
                    "foreign" => "foreign owner",
                    "unaccepted" => "no exact accepted Remote finish",
                    "contradictory" => "finish projections conflict",
                    "advanced" => "Remote target advanced",
                    _ => unreachable!(),
                };
                assert!(error.contains(expected), "{case}: {error}");
                Ok(())
            })
            .unwrap();
        assert_eq!(snapshots.list_line_snapshots().unwrap(), before, "{case}");
        assert_eq!(
            tasks.list_completed_tasks_with_landed_changes().unwrap(),
            receipts,
            "{case}"
        );
        assert_eq!(
            tasks
                .snapshot_ownership_rows(
                    &before
                        .iter()
                        .map(|row| row.snapshot_id.clone())
                        .collect::<Vec<_>>()
                )
                .unwrap(),
            owners,
            "{case}"
        );
    }
}

/// Explicit cross-owner input generation; these are actual native package
/// inputs, not a passing Server result or a replacement acceptance receipt.
#[test]
#[ignore = "Retains package inputs for the assigned native Server check"]
fn native_server_package_inputs() {
    use std::io::Read;
    let fixture = ReplayFixture::new(false);
    fixture.run(|| {
        let prepared = fixture.prepare()?;
        let repo = &fixture.local.repo;
        let tasks = repo.task_store()?;
        let snapshots = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
            &repo.workspace_root(),
        )?;
        let last = workflow_completed_local_entry_if_present(
            repo, fixture.prefix.last().unwrap(), Some("origin"),
        )?.ok_or("Missing original prefix source")?;
        let final_original = required_string_field(&last["state"]["local_finish"], "landed_snapshot_id")?;
        let (entries, _) = local_completion::workflow_local_history_entries(
            repo, fixture.prefix.last().unwrap(), "main", &fixture.local.base, &final_original,
        )?;
        let mut prefix = prepared.manifest.clone();
        prefix.base_snapshot_id = fixture.local.base.clone();
        prefix.revision_snapshot_id = fixture.local.target.clone();
        prefix.summary = Some("Accepted-prefix seed for native suffix admission".into());
        let mut selected = BTreeSet::from([fixture.local.base.clone(), fixture.local.target.clone()]);
        prefix.history_entries = entries.iter().map(|entry| {
            let id = required_string_field(entry, "local_task_id")?;
            let mut owned = Vec::new();
            for (snapshot, owner) in tasks.completed_task_source_snapshots(&id).map_err(|error| error.to_string())? {
                assert!(owner.change_id.is_empty());
                let row = snapshot_by_id_with_snapshot_store(&snapshots, &snapshot)?.ok_or("Missing prefix Snapshot")?;
                selected.insert(snapshot.clone());
                owned.push(json!({"snapshot_id":snapshot,"created_at_s":snapshot_seconds(&row.created_at)?,"local_change_ref":null}));
            }
            Ok(json!({"local_task_id":id,"expected_remote_task_id":null,
                "task":{"title":entry["task"]["title"],"intent":entry["task"]["intent"]},"changes":[],
                "pre_land_target_snapshot_id":entry["local_finish"]["pre_land_target_snapshot_id"],
                "landed_snapshot_id":entry["landed_snapshot_id"],"landed_at_s":entry["landed_at_s"],"snapshots":owned}))
        }).collect::<Result<Vec<_>, String>>()?;
        let correction = snapshot_by_id_with_snapshot_store(&snapshots, &fixture.local.target)?
            .ok_or("Missing accepted correction seed")?;
        prefix.history_entries.last_mut().unwrap()["snapshots"].as_array_mut().unwrap().push(
            json!({"snapshot_id":correction.snapshot_id,"created_at_s":snapshot_seconds(&correction.created_at)?,"local_change_ref":null}),
        );
        prefix.content.snapshots.retain(|row| selected.contains(&row.snapshot_id));
        let decoder = zstd::stream::read::Decoder::new(prepared.package.bytes.as_slice()).map_err(|error| error.to_string())?;
        let mut archive = tar::Archive::new(decoder);
        let mut objects = BTreeMap::new();
        let mut trees = BTreeMap::new();
        for entry in archive.entries().map_err(|error| error.to_string())? {
            let mut entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path().map_err(|error| error.to_string())?.to_string_lossy().into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
            if let Some(id) = path.strip_prefix("objects/") { objects.insert(id.to_string(), bytes); }
            else if let Some(id) = path.strip_prefix("trees/") { trees.insert(id.to_string(), bytes); }
        }
        let seed = prefix.encode(&objects, &trees)?;
        let output = tempfile::Builder::new().prefix("ait-core-native-suffix-").tempdir().map_err(|error| error.to_string())?.keep();
        fs::write(output.join("accepted-prefix.zst"), seed.bytes).map_err(|error| error.to_string())?;
        fs::write(output.join("fresh-suffix.zst"), prepared.package.bytes).map_err(|error| error.to_string())?;
        println!("Native Server package inputs: {}", output.display());
        Ok(())
    }).unwrap();
}
