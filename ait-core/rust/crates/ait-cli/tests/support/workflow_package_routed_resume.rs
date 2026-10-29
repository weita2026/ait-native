//! Explicit pairing gate for reviewed native Server and Runner artifacts.
use super::*;
use ait_core::plan_http_client::{PlanHttpClientConfig, PlanHttpClientManager};
use ait_core::server_operational::{RepositoryIndex, WorkerJobIndex, WorkerJobKey};
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct ChildProcess(Option<Child>);

pub fn config_publication_preserves_admission_identity() {
    use ait_cli::runtime::RepoRuntime;
    use ait_core::binary_db_generation::admit_activated_binary_db_generation_for_runtime;
    use ait_core::repo_state_json::RootConfigLease;
    use ait_core::task_store::TaskStore;
    use std::io::{Seek, Write};
    use std::sync::mpsc;

    let fixture = Fixture::new();
    let (task, _edit) = fixture.start("config-publication-identity");
    let expected_task = fixture.task(&task);
    let before = RepoRuntime::discover_from_path(&fixture.root).unwrap();
    let expected_name = before.repo_name();
    let expected_namespace = before.id_namespace_prefix();
    let path = fixture.root.join(".ait/config.json");
    let mut config: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["workflow_ready_poll_seconds"] = json!(43);
    let complete = serde_json::to_vec_pretty(&config).unwrap();
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (cli_tx, cli_rx) = mpsc::channel();
    let (sdk_tx, sdk_rx) = mpsc::channel();
    let readers_started = std::sync::Barrier::new(3);
    std::thread::scope(|scope| {
        let path = &path;
        let writer = scope.spawn(move || {
            let lease = RootConfigLease::try_acquire(path, true, true).unwrap();
            // Hold the actual empty-file interval until both admission paths
            // are running. Neither may observe EOF or a default owner.
            lease.with_file(|file| file.set_len(0)).unwrap();
            held_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            lease
                .with_file(|file| {
                    file.rewind()?;
                    file.write_all(&complete)?;
                    file.set_len(complete.len() as u64)?;
                    file.flush()
                })
                .unwrap();
        });
        held_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let root = &fixture.root;
        let task = &task;
        let readers_started = &readers_started;
        let cli_reader = scope.spawn(move || {
            readers_started.wait();
            let result = (|| -> Result<_, String> {
                let repo = RepoRuntime::discover_from_path(root)?;
                let task = TaskStore::get_task(&repo.task_store()?, task)
                    .map_err(|error| error.to_string())?;
                Ok((
                    repo.repo_name(),
                    repo.id_namespace_prefix(),
                    task,
                    repo.config["workflow_ready_poll_seconds"].clone(),
                ))
            })();
            cli_tx.send(result).unwrap();
        });
        let expected_name = &expected_name;
        let sdk_reader = scope.spawn(move || {
            readers_started.wait();
            let result = admit_activated_binary_db_generation_for_runtime(
                root,
                &root.join(".ait/binary-db"),
                expected_name,
            )
            .map(|_| ());
            sdk_tx.send(result).unwrap();
        });
        readers_started.wait();
        for pending in [
            cli_rx.recv_timeout(Duration::from_millis(50)).map(|_| ()),
            sdk_rx.recv_timeout(Duration::from_millis(50)).map(|_| ()),
        ] {
            assert!(
                matches!(pending, Err(mpsc::RecvTimeoutError::Timeout)),
                "admission completed inside an unpublished empty-config interval"
            );
        }
        release_tx.send(()).unwrap();
        writer.join().unwrap();
        let (name, namespace, actual_task, hint) = cli_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        sdk_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        cli_reader.join().unwrap();
        sdk_reader.join().unwrap();
        assert_eq!(&name, expected_name);
        assert_eq!(namespace, expected_namespace);
        assert_eq!(actual_task, expected_task);
        assert_eq!(hint, 43);
    });
    assert_eq!(
        serde_json::from_slice::<JsonValue>(&fs::read(&path).unwrap()).unwrap(),
        config
    );
}

impl ChildProcess {
    fn output(mut self) -> Output {
        self.0.take().unwrap().wait_with_output().unwrap()
    }

    fn stop(&mut self) -> std::io::Result<()> {
        if let Some(child) = self.0.as_mut() {
            if child.try_wait()?.is_none() {
                if let Err(error) = child.kill() {
                    if child.try_wait()?.is_none() {
                        return Err(error);
                    }
                }
            }
            child.wait()?;
        }
        self.0 = None;
        Ok(())
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub fn run_pair() {
    run(false);
}

pub fn run_reset_pair() {
    run(true);
}

fn wait_ready(server: &mut ChildProcess, client: &mut PlanHttpClientManager) {
    for _ in 0..200 {
        assert!(
            server.0.as_mut().unwrap().try_wait().unwrap().is_none(),
            "isolated Server exited"
        );
        if client
            .get_server_health()
            .is_ok_and(|health| health["ready"] == true)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("isolated reviewed Server did not become ready");
}

fn write_ci_entry(edit: &Path) {
    // Runner executes repository CI from the materialized workspace root.
    // Use that same contract on both platforms; PowerShell's Join-Path provider
    // cannot resolve the Windows verbatim script root used by native Runner.
    #[cfg(windows)]
    fs::write(
        edit.join("ci/run.ps1"),
        r#"$ErrorActionPreference = 'Stop'
if ([System.IO.File]::ReadAllText('proof.txt') -cne "original sealed source`n") {
    throw 'The executed source differs from the original sealed content.'
}
"#,
    )
    .unwrap();

    #[cfg(not(windows))]
    {
        let ci = edit.join("ci/run.sh");
        fs::write(
            &ci,
            "#!/bin/sh\nset -eu\ntest \"$(cat proof.txt)\" = \"original sealed source\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        fs::set_permissions(&ci, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(windows)]
#[test]
fn windows_ci_entry_checks_proof_from_normal_and_verbatim_workspaces() {
    use std::path::{Component, PathBuf, Prefix};

    let temp = tempfile::tempdir().unwrap();
    let verbatim = temp.path().canonicalize().unwrap().join("workspace");
    let Some(Component::Prefix(prefix)) = verbatim.components().next() else {
        panic!("canonical Windows path must have a prefix: {verbatim:?}");
    };
    let Prefix::VerbatimDisk(drive) = prefix.kind() else {
        panic!("test requires a drive-qualified temp directory: {verbatim:?}");
    };
    let mut normal = PathBuf::from(format!("{}:\\", char::from(drive)));
    normal.extend(verbatim.components().skip(2).map(|part| part.as_os_str()));
    fs::create_dir_all(verbatim.join("ci")).unwrap();
    write_ci_entry(&verbatim);
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").expect("Windows SystemRoot"))
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");

    for proof in ["original sealed source\n", "altered sealed source\n"] {
        fs::write(verbatim.join("proof.txt"), proof).unwrap();
        for root in [&normal, &verbatim] {
            let output = Command::new(&powershell)
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(root.join("ci/run.ps1"))
                .current_dir(root)
                .output()
                .unwrap();
            assert_eq!(
                output.status.success(),
                proof == "original sealed source\n",
                "root={root:?}\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            if proof != "original sealed source\n" {
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("The executed source differs")
                );
            }
        }
    }
}

fn assert_ci_delivery(output: &Output, expected: WorkerJobKey) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let delivery = output_json(output);
    assert_eq!(delivery["contract"], "ait.runner.delivery.v1", "{delivery}");
    assert_eq!(delivery["status"], "delivered", "{delivery}");
    assert_eq!(
        delivery["repository_index"].as_u64(),
        Some(u64::from(expected.repository_index.get()))
    );
    assert_eq!(
        delivery["worker_job_index"].as_u64(),
        Some(u64::from(expected.worker_job_index.get()))
    );
    assert!(delivery["attempt_count"]
        .as_u64()
        .is_some_and(|count| count > 0));
    assert_eq!(delivery["terminal_operation"], "complete", "{delivery}");
    assert_eq!(delivery["result"]["status"], "succeeded", "{delivery}");
    assert_eq!(delivery["result"]["tests_status"], "pass", "{delivery}");
    assert!(delivery["result"]["suite_result_count"]
        .as_u64()
        .is_some_and(|count| count > 0));
    assert_eq!(
        delivery["result"]["cleanup"]["attempt_root_removed"], true,
        "{delivery}"
    );
    assert_eq!(
        delivery["result"]["cleanup"]["remaining_owned_paths"], 0,
        "{delivery}"
    );
}

fn run(reset: bool) {
    let server_bin =
        std::env::var_os("TEST_REVIEWED_SERVER_BIN").expect("set the exact reviewed Server binary");
    let runner_bin = std::env::var_os("TEST_NATIVE_RUNNER_BIN")
        .expect("set the native Runner binary used for real CI");
    let ram_root =
        std::env::var_os("TEST_SERVER_CI_RAM_ROOT").expect("set an existing memory-backed CI root");
    let data = tempfile::tempdir().unwrap();
    let ram = tempfile::tempdir_in(ram_root).unwrap();
    let initialized = Command::new(&server_bin)
        .args(["init", "--data"])
        .arg(data.path())
        .output()
        .unwrap();
    assert!(
        initialized.status.success(),
        "{}",
        String::from_utf8_lossy(&initialized.stderr)
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let spawn_server = || {
        ChildProcess(Some(
            Command::new(&server_bin)
                .args(["--data"])
                .arg(data.path())
                .args(["--listen", &address.to_string(), "--defer-ci-admission"])
                .env("AIT_NATIVE_SERVER_CI_RAM_ROOT", ram.path())
                .env("AIT_NATIVE_SERVER_CI_RAM_MIN_AVAILABLE_BYTES", "0")
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        ))
    };
    let mut server = spawn_server();
    let url = format!("http://{address}");
    let mut client = PlanHttpClientManager::new(PlanHttpClientConfig {
        base_url: url.clone(),
        repository_index: Some(RepositoryIndex::new(0)),
        default_timeout_ms: 1000,
        ..Default::default()
    })
    .unwrap();
    wait_ready(&mut server, &mut client);

    let fixture = Fixture::with_server(&url);
    let authority = client
        .get_repository_by_index(RepositoryIndex::new(0))
        .unwrap();
    let config_path = fixture.root.join(".ait/config.json");
    let mut config: JsonValue = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["id_namespace_prefix"] = authority["repository"]["namespace"].clone();
    fs::write(&config_path, config.to_string()).unwrap();
    let (task, edit) = fixture.start("routed-sealed-mode");
    fs::create_dir(edit.join("ci")).unwrap();
    fs::write(edit.join("proof.txt"), "original sealed source\n").unwrap();
    write_ci_entry(&edit);
    fixture.finish_local(&task, &edit);
    defaults(&fixture, true, "ai_with_human_review");
    assert_eq!(
        client
            .list_worker_jobs(RepositoryIndex::new(0), None, 100)
            .unwrap()["count"],
        0
    );
    let mut fresh_args = vec![
        "workflow", "ready", &task, "--apply", "--remote", "origin", "--json", "--full",
    ];
    if reset {
        fresh_args.extend(["--summary", "original sealed summary"]);
    }
    let mut fresh_process = ChildProcess(Some(
        command(&fixture.root, &fresh_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let mut key = WorkerJobKey::new(RepositoryIndex::new(0), WorkerJobIndex::new(0));
    let mut pending = None;
    for _ in 0..600 {
        if fresh_process
            .0
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_some()
        {
            let failed = fresh_process.output();
            panic!(
                "fresh CLI exited before queued publication: {}\n{}",
                String::from_utf8_lossy(&failed.stdout),
                String::from_utf8_lossy(&failed.stderr)
            );
        }
        if let Ok(job) = client.get_worker_job(key) {
            if job["package_execution"]["author_mode"].as_str().is_some() {
                pending = Some(job);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let pending = pending.expect("fresh CLI did not select its native package Job");
    assert_eq!(pending["package_execution"]["ci_state"], "queued");
    let mut identity = pending["package_execution"]["expected_identity"].clone();
    let mut pin = fixture.identity(&identity, "routed.identity.json");
    assert_eq!(
        pending["package_execution"]["author_mode"], MODE,
        "{pending}"
    );
    assert_eq!(pending["package_execution"]["expected_identity"], identity);
    let mut original_snapshots = fixture.snapshots();
    let original_task = fixture.task(&task);

    let ci_result = Command::new(&runner_bin)
        .args([
            "serve",
            "--server",
            &url,
            "--worker-id",
            "sealed-mode-pair",
            "--repository-index",
            "0",
            "--once",
            "--attempt-root",
        ])
        .arg(data.path().join("runner-attempt"))
        .output()
        .unwrap();
    assert_ci_delivery(&ci_result, key);
    let done = client.get_worker_job(key).unwrap();
    assert_eq!(done["package_execution"]["ci_state"], "passed", "{done}");
    assert_eq!(done["package_execution"]["author_mode"], MODE);
    let fresh = output_json(&fresh_process.output());
    assert_eq!(fresh["next_action"]["code"], "done", "{fresh}");
    assert_eq!(fresh["expected_identity"], identity);
    defaults(&fixture, false, "human_only");
    let conflict = ready(&fixture, &task, Some(&pin), Some("human_only"));
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("sealed package"));
    assert_eq!(client.get_worker_job(key).unwrap(), done);
    let resumed = output_json(&ready(&fixture, &task, Some(&pin), None));
    assert_eq!(resumed["next_action"]["code"], "done", "{resumed}");
    assert_eq!(resumed["expected_identity"], identity);
    assert_eq!(resumed["package_execution"]["author_mode"], MODE);
    assert_eq!(fixture.snapshots(), original_snapshots);
    assert_eq!(fixture.task(&task), original_task);
    let jobs = client
        .list_worker_jobs(RepositoryIndex::new(0), None, 100)
        .unwrap();
    assert_eq!(jobs["count"], 1, "{jobs}");
    assert!(client.get_worker_job(key).unwrap()["job"]["patchset_index"].is_null());

    let retired_pin = if reset {
        let old_identity = identity.clone();
        let old_pin = pin.clone();
        let (later, edit) = fixture.start("later-after-original-ci");
        fs::write(edit.join("later.txt"), "must remain Local after reset\n").unwrap();
        fixture.finish_local(&later, &edit);
        original_snapshots = fixture.snapshots();
        server
            .stop()
            .expect("stop and reap the original Server process before restart");
        server = spawn_server();
        wait_ready(&mut server, &mut client);
        let old_finish = output_json(&finish(&fixture, &task, Some(&old_pin)));
        assert_ne!(old_finish["next_action"]["code"], "done");
        let readonly = output_json(&super::run(
            &fixture.root,
            &[
                "workflow",
                "ready",
                &task,
                "--remote",
                "origin",
                "--expected-identity",
                old_pin.to_str().unwrap(),
                "--json",
                "--full",
            ],
        ));
        assert_ne!(readonly["next_action"]["code"], "done");
        let override_result = ready(&fixture, &task, Some(&old_pin), Some("human_only"));
        assert!(
            !override_result.status.success()
                || output_json(&override_result)["next_action"]["code"] != "done"
        );
        assert_eq!(
            client
                .list_worker_jobs(RepositoryIndex::new(0), None, 100)
                .unwrap()["count"],
            1
        );
        let mut replacement_process = ChildProcess(Some(
            command(
                &fixture.root,
                &[
                    "workflow",
                    "ready",
                    &task,
                    "--apply",
                    "--remote",
                    "origin",
                    "--expected-identity",
                    old_pin.to_str().unwrap(),
                    "--json",
                    "--full",
                ],
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        ));
        key = WorkerJobKey::new(RepositoryIndex::new(0), WorkerJobIndex::new(1));
        let mut queued = None;
        for _ in 0..600 {
            if replacement_process
                .0
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_some()
            {
                let failed = replacement_process.output();
                panic!(
                    "retained ready exited before replacement CI: {}\n{}",
                    String::from_utf8_lossy(&failed.stdout),
                    String::from_utf8_lossy(&failed.stderr)
                );
            }
            if let Ok(job) = client.get_worker_job(key) {
                if job["package_execution"]["author_mode"].as_str().is_some() {
                    queued = Some(job);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let queued = queued.expect("retained ready did not allocate its replacement");
        let new_identity = queued["package_execution"]["expected_identity"].clone();
        assert_eq!(queued["package_execution"]["author_mode"], MODE);
        for field in [
            "repository_index",
            "package_hash",
            "base_snapshot_id",
            "revision_snapshot_id",
        ] {
            assert_eq!(new_identity[field], old_identity[field]);
        }
        let ci_result = Command::new(&runner_bin)
            .args([
                "serve",
                "--server",
                &url,
                "--worker-id",
                "sealed-mode-after-reset",
                "--repository-index",
                "0",
                "--once",
                "--attempt-root",
            ])
            .arg(data.path().join("replacement-runner-attempt"))
            .output()
            .unwrap();
        assert_ci_delivery(&ci_result, key);
        assert_eq!(
            client.get_worker_job(key).unwrap()["package_execution"]["ci_state"],
            "passed"
        );
        let replaced = output_json(&replacement_process.output());
        assert_eq!(replaced["next_action"]["code"], "done", "{replaced}");
        assert_eq!(replaced["expected_identity"], new_identity);
        let replay = output_json(&ready(&fixture, &task, Some(&old_pin), None));
        assert_eq!(replay["expected_identity"], new_identity, "{replay}");
        identity = new_identity;
        pin = fixture.identity(&identity, "replacement.identity.json");
        assert_eq!(fixture.snapshots(), original_snapshots);
        assert_eq!(fixture.task(&task), original_task);
        Some(old_pin)
    } else {
        None
    };

    let published = fixture.finish(&task, &pin);
    let cleaned = client.get_worker_job(key).unwrap();
    assert!(
        cleaned["package_execution"].is_null()
            || cleaned["package_execution"]["availability"] == "unavailable",
        "{cleaned}"
    );
    assert!(cleaned["package_execution"]["author_mode"].is_null());
    let recovered = accepted(&ready(&fixture, &task, Some(&pin), None), &identity);
    for field in ["patchset_id", "submission_id"] {
        assert_eq!(
            recovered["completion"][field],
            published["completion"][field]
        );
    }
    let conflict = ready(&fixture, &task, Some(&pin), Some("human_only"));
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("sealed mode"));
    assert_eq!(
        client
            .list_worker_jobs(RepositoryIndex::new(0), None, 100)
            .unwrap()["count"],
        if reset { 2 } else { 1 }
    );
    if let Some(old_pin) = retired_pin {
        let held = output_json(&finish(&fixture, &task, Some(&old_pin)));
        assert_ne!(held["next_action"]["code"], "done");
    }
}
