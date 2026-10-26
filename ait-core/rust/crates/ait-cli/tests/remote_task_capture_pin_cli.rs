//! Real-server regression; supply the exact reviewed component artifacts explicitly.
use ait_core::plan_http_client::PlanHttpClientConfig;
use ait_core::server_operational::RepositoryIndex;
use ait_core::task_workflow_http_adapter::HttpTaskRemote;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    url: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn isolated(command: &mut Command, home: &Path) {
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("CODEX_HOME", home.join("codex"))
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CODEX_SESSION_ID");
}

fn server(binary: &Path, root: &Path, home: &Path) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut command = Command::new(binary);
    isolated(&mut command, home);
    let child = command
        .args([
            "--data",
            root.to_str().unwrap(),
            "--listen",
            &address.to_string(),
            "--init-if-missing",
            "--defer-ci-admission",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut result = Server {
        child,
        url: format!("http://{address}"),
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            result.child.try_wait().unwrap().is_none(),
            "isolated Server exited before health check"
        );
        if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            stream
                .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut response = String::new();
            let _ = stream.read_to_string(&mut response);
            if response.starts_with("HTTP/1.1 200") {
                return result;
            }
        }
        assert!(
            Instant::now() < deadline,
            "isolated Server health check timed out"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn run(binary: &Path, root: &Path, home: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(binary);
    isolated(&mut command, home);
    command.current_dir(root).args(args).output().unwrap()
}

fn json(binary: &Path, root: &Path, home: &Path, args: &[&str]) -> Value {
    let output = run(binary, root, home, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn repository(binary: &Path, parent: &Path, home: &Path) -> PathBuf {
    let root = parent.join("repo");
    fs::create_dir_all(&root).unwrap();
    json(binary, &root, home, &["init", "--json"]);
    json(
        binary,
        &root,
        home,
        &[
            "config",
            "set",
            "--sprint",
            "on",
            "--user-name",
            "Capture pin fixture",
            "--user-email",
            "capture-pin@example.invalid",
            "--json",
        ],
    );
    fs::write(root.join("code.rs"), "fn baseline() {}\n").unwrap();
    fs::write(root.join("docs/sprints/start.md"),
        "# Remote capture pin [plan-ref: test/capture-pin]\n\n- [ ] Establish the original capture authority [ref: capture-pin/start]\n").unwrap();
    root
}

fn remote(binary: &Path, root: &Path, home: &Path, name: &str, url: &str, default: bool) {
    let mut args = vec!["remote", "add", name, url, "--json"];
    if default {
        args.push("--default");
    }
    json(binary, root, home, &args);
}

fn inject_configured_url(root: &Path, url: &str) {
    // Fault injection changes only this isolated configuration, never a capture pin.
    let path = root.join(".ait/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["remotes"]["b"]["url"] = Value::String(url.into());
    fs::write(path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
}

fn start(
    binary: &Path,
    root: &Path,
    home: &Path,
    selected: &str,
    edit: &Path,
    intent: &str,
) -> Output {
    run(
        binary,
        root,
        home,
        &[
            "task",
            "start",
            "--from",
            "docs/sprints/start.md#capture-pin/start",
            "--intent",
            intent,
            "--remote",
            selected,
            "--edit-root",
            edit.to_str().unwrap(),
            "--json",
            "--full",
        ],
    )
}

fn successful_start(
    binary: &Path,
    root: &Path,
    home: &Path,
    selected: &str,
    edit: &Path,
    intent: &str,
) -> Value {
    let output = start(binary, root, home, selected, edit, intent);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        result["change"].is_null(),
        "initial Task unexpectedly created a Change"
    );
    result
}

fn snapshot_remote(url: &str, index: u32) -> HttpTaskRemote {
    HttpTaskRemote::new(PlanHttpClientConfig {
        base_url: url.into(),
        repository_index: Some(RepositoryIndex::new(index)),
        default_timeout_ms: 3000,
        ..PlanHttpClientConfig::default()
    })
    .unwrap()
}

#[cfg(unix)]
struct RestorePermissions(PathBuf, fs::Permissions);

#[cfg(unix)]
impl Drop for RestorePermissions {
    fn drop(&mut self) {
        fs::set_permissions(&self.0, self.1.clone()).unwrap();
    }
}

#[test]
#[ignore = "requires AIT_TEST_SERVER_BIN and AIT_TEST_BASELINE_CLI exact reviewed artifacts"]
fn explicit_remote_start_pins_before_first_capture_with_same_numbered_server_authorities() {
    let server_binary =
        PathBuf::from(std::env::var_os("AIT_TEST_SERVER_BIN").expect("reviewed Server executable"));
    let baseline =
        PathBuf::from(std::env::var_os("AIT_TEST_BASELINE_CLI").expect("reviewed pre-fix CLI"));
    let candidate = Path::new(env!("CARGO_BIN_EXE_ait-cli"));
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    let a = server(&server_binary, &root.join("server-a"), &home);
    let b = server(&server_binary, &root.join("server-b"), &home);
    let control = repository(&baseline, &root.join("control"), &home);
    let selected = repository(candidate, &root.join("selected"), &home);
    // Establish each independent authority before the local Repository index is pinned.
    remote(candidate, &control, &home, "a", &a.url, true);
    remote(candidate, &selected, &home, "b", &b.url, true);
    remote(candidate, &control, &home, "b", &b.url, false);
    remote(candidate, &selected, &home, "a", &a.url, true);
    let control_edit = root.join("control-edit");
    let control_task = successful_start(
        &baseline,
        &control,
        &home,
        "a",
        &control_edit,
        "Remote A control",
    );
    let control_proof = json(
        &baseline,
        &control_edit,
        &home,
        &["worktree", "show", "--json"],
    );
    assert_eq!(
        control_proof["capture_authority"],
        Value::Null,
        "baseline no longer reproduces missing pin"
    );
    let selected_edit = root.join("selected-edit");
    let selected_task = successful_start(
        candidate,
        &selected,
        &home,
        "b",
        &selected_edit,
        "Remote B candidate",
    );
    assert_eq!(selected_task["task_id"], control_task["task_id"]);
    let task = selected_task["task_id"].as_str().unwrap();
    let proof = json(
        candidate,
        &selected_edit,
        &home,
        &["worktree", "show", "--json"],
    );
    let index = proof["capture_authority"]["repository_index"]
        .as_u64()
        .unwrap() as u32;
    assert_eq!(
        proof["capture_authority"],
        serde_json::json!({"remote_name": "b", "repository_index": index})
    );
    let named = json(
        candidate,
        &selected,
        &home,
        &[
            "worktree",
            "show",
            proof["name"].as_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(named["capture_authority"], proof["capture_authority"]);
    let config_a = json(candidate, &control, &home, &["config", "show", "--json"]);
    assert_eq!(
        config_a["repository_index"].as_u64(),
        Some(u64::from(index))
    );
    let tasks_a_before = json(
        candidate,
        &selected,
        &home,
        &["task", "list", "--remote", "a", "--all", "--json"],
    );
    fs::write(
        selected_edit.join("code.rs"),
        "fn baseline() {}\nfn first_remote_capture() {}\n",
    )
    .unwrap();
    let captured = json(
        candidate,
        &selected_edit,
        &home,
        &[
            "snapshot",
            "create",
            task,
            "--message",
            "first Task capture",
            "--json",
            "--full",
        ],
    );
    assert_eq!(captured["task_id"].as_str(), Some(task));
    assert!(captured["change_id"].is_null());
    let id = captured["snapshot_id"].as_str().unwrap();
    let stored = snapshot_remote(&b.url, index)
        .get_remote_snapshot("repo", id, false, None)
        .unwrap();
    assert_eq!(stored["capture_binding"]["task_id"].as_str(), Some(task));
    assert!(stored["capture_binding"]["change_ref"].is_null());
    let absent = snapshot_remote(&a.url, index)
        .get_remote_snapshot("repo", id, false, None)
        .unwrap_err();
    assert_eq!(absent.remote_status(), Some(404));
    assert_eq!(
        json(
            candidate,
            &selected,
            &home,
            &["task", "list", "--remote", "a", "--all", "--json"]
        ),
        tasks_a_before
    );
    let retried = successful_start(
        candidate,
        &selected,
        &home,
        "b",
        &selected_edit,
        "Remote B candidate",
    );
    assert_eq!(retried["task_id"].as_str(), Some(task));
    assert_eq!(retried["worktree_reused"], true);

    fs::remove_dir_all(&selected_edit).unwrap();
    json(
        candidate,
        &selected,
        &home,
        &[
            "worktree",
            "recreate",
            proof["name"].as_str().unwrap(),
            "--json",
        ],
    );
    let recreated = json(
        candidate,
        &selected_edit,
        &home,
        &["worktree", "show", "--json"],
    );
    assert_eq!(recreated["capture_authority"], proof["capture_authority"]);
    assert_eq!(recreated["head_snapshot_id"].as_str(), Some(id));

    let snapshots_before = json(
        candidate,
        &selected_edit,
        &home,
        &["snapshot", "list", "--all", "--json"],
    );
    inject_configured_url(&selected, &a.url);
    let wrong = run(
        candidate,
        &selected_edit,
        &home,
        &[
            "snapshot",
            "create",
            task,
            "--message",
            "must reject changed endpoint",
            "--json",
        ],
    );
    assert!(!wrong.status.success());
    assert_eq!(
        json(
            candidate,
            &selected_edit,
            &home,
            &["snapshot", "list", "--all", "--json"]
        ),
        snapshots_before
    );
    inject_configured_url(&selected, &b.url);

    let unbound = selected.clone();
    json(
        candidate,
        &unbound,
        &home,
        &["config", "set", "--sprint", "off", "--json"],
    );
    let unbound_edit = root.join("unbound-edit");
    let unbound_task = json(
        candidate,
        &unbound,
        &home,
        &[
            "task",
            "start",
            "--title",
            "Unbound Remote capture pin",
            "--intent",
            "Bind the exact Remote for unbound start",
            "--remote",
            "b",
            "--edit-root",
            unbound_edit.to_str().unwrap(),
            "--json",
            "--full",
        ],
    );
    assert!(unbound_task["change"].is_null());
    assert_eq!(
        json(
            candidate,
            &unbound_edit,
            &home,
            &["worktree", "show", "--json"]
        )["capture_authority"],
        proof["capture_authority"]
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let failed_repo = repository(candidate, &root.join("failed-setup"), &home);
        remote(candidate, &failed_repo, &home, "b", &b.url, true);
        fs::write(failed_repo.join("docs/sprints/start.md"),
            "# Failed setup [plan-ref: test/capture-failure]\n\n- [ ] Preserve acknowledged Task identity [ref: capture-failure/start]\n").unwrap();
        let before = json(
            candidate,
            &failed_repo,
            &home,
            &["task", "list", "--remote", "b", "--all", "--json"],
        );
        let registry = failed_repo.join(".ait/worktrees");
        fs::create_dir_all(&registry).unwrap();
        let restore = RestorePermissions(
            registry.clone(),
            fs::metadata(&registry).unwrap().permissions(),
        );
        fs::set_permissions(&registry, fs::Permissions::from_mode(0o555)).unwrap();
        let failed_edit = root.join("failed-edit");
        let args = [
            "task",
            "start",
            "--from",
            "docs/sprints/start.md#capture-failure/start",
            "--intent",
            "Preserve the acknowledged Task after setup failure",
            "--remote",
            "b",
            "--edit-root",
            failed_edit.to_str().unwrap(),
            "--json",
        ];
        let failed = run(candidate, &failed_repo, &home, &args);
        drop(restore);
        assert!(!failed.status.success());
        let after = json(
            candidate,
            &failed_repo,
            &home,
            &["task", "list", "--remote", "b", "--all", "--json"],
        );
        assert_eq!(
            after.as_array().unwrap().len(),
            before.as_array().unwrap().len() + 1
        );
        let created = after
            .as_array()
            .unwrap()
            .iter()
            .find(|row| {
                !before
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|old| old["task_id"] == row["task_id"])
            })
            .unwrap()["task_id"]
            .as_str()
            .unwrap();
        assert!(String::from_utf8_lossy(&failed.stderr).contains(created));
        assert!(failed.stdout.is_empty());
        let retry = run(candidate, &failed_repo, &home, &args);
        assert!(
            !retry.status.success(),
            "unproven partial worktree must remain on hold"
        );
        assert_eq!(
            json(
                candidate,
                &failed_repo,
                &home,
                &["task", "list", "--remote", "b", "--all", "--json"]
            ),
            after
        );
    }

    let local = repository(candidate, &root.join("local"), &home);
    let local_edit = root.join("local-edit");
    json(
        candidate,
        &local,
        &home,
        &[
            "task",
            "start",
            "--from",
            "docs/sprints/start.md#capture-pin/start",
            "--intent",
            "Local remains unpinned",
            "--local",
            "--edit-root",
            local_edit.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(
        json(
            candidate,
            &local_edit,
            &home,
            &["worktree", "show", "--json"]
        )["capture_authority"],
        Value::Null
    );
}
