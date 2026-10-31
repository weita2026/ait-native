//! Disposable native fixtures and process barriers for workflow acceptance.
#![allow(dead_code)]
use crate::{cli_binary, native_home};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const COMMAND_LIMIT: Duration = Duration::from_secs(30);
const PEER_LIMIT: Duration = Duration::from_secs(60);
const WORKER_ADDRESS: &str = "AIT_WORKFLOW_ACCEPTANCE_ADDRESS";
const GITLESS_STAGE: &str = "AIT_WORKFLOW_GITLESS_STAGE";
const GITLESS_ROOT: &str = "AIT_WORKFLOW_GITLESS_ROOT";
const GITLESS_COMPLETE: &str = "isolated Git NotFound and native lifecycle completed";
const GITLESS_PARENT_COMPLETE: &str = "Git-present parent and isolation controls completed";
const GIT_SENTINEL: &str = "Git-present negative control executed";
const GITLESS_FAILURE: &str = "intentional isolation worker failure";

fn reject_git_beside_cli(binary: &Path) -> Result<(), String> {
    let git = binary
        .parent()
        .expect("absolute CLI directory")
        .join("git.exe");
    match fs::symlink_metadata(&git) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Cannot exclude Git at {}: {error}", git.display())),
        Ok(_) => Err(format!(
            "Git beside selected CLI cannot be excluded: {}",
            git.display()
        )),
    }
}

fn gitless_command(test: &str, stage: &str, root: &Path, path: &Path) -> Command {
    let harness = std::env::current_exe().unwrap();
    assert!(harness.is_absolute());
    let mut command = Command::new(harness);
    command
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(GITLESS_STAGE, stage)
        .env(GITLESS_ROOT, root)
        .env("PATH", path)
        .current_dir(root);
    command
}

fn gitless_result(output: &Output, marker: &str) -> Result<(), String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() || stdout.lines().filter(|line| *line == marker).count() != 1 {
        return Err(format!(
            "Isolation worker {:?}\n{stdout}\n{stderr}",
            output.status
        ));
    }
    Ok(())
}

/// Run the existing case with an isolated *parent* PATH, without changing the
/// shared test runner. Windows still searches the parent's PATH after a child
/// PATH override. A copied harness is Git only in negative controls; the real
/// lifecycle must observe NotFound and uses the original selected CLI.
pub fn gitless_worker(test: &str) -> bool {
    let stage = std::env::var(GITLESS_STAGE).ok();
    match stage.as_deref() {
        Some("isolated") => {
            let root = PathBuf::from(std::env::var_os(GITLESS_ROOT).unwrap());
            let empty = root.join("empty-path");
            assert_eq!(std::env::var_os("PATH").as_deref(), Some(empty.as_os_str()));
            assert_eq!(fs::read_dir(empty).unwrap().count(), 0);
            false
        }
        Some("sentinel") => {
            println!("\n{GIT_SENTINEL}");
            true
        }
        Some("stdout-failure") => {
            println!("{GITLESS_FAILURE}");
            std::io::stdout().flush().unwrap();
            std::process::exit(23);
        }
        Some("failure") => {
            let root = PathBuf::from(std::env::var_os(GITLESS_ROOT).unwrap());
            let output = diagnostic_output(gitless_command(
                test,
                "stdout-failure",
                &root,
                &root.join("empty-path"),
            ));
            assert_eq!(output.status.code(), Some(23));
            assert!(output.stderr.is_empty());
            parsed(&output);
            panic!("non-successful command was accepted");
        }
        Some("parent") => {
            let root = PathBuf::from(std::env::var_os(GITLESS_ROOT).unwrap());
            let empty = root.join("empty-path");
            let present = root.join("git-present");
            assert_eq!(
                std::env::var_os("PATH").as_deref(),
                Some(present.as_os_str())
            );
            let probe = |path: &Path| {
                Command::new("git")
                    .args(["--exact", test, "--nocapture", "--test-threads=1"])
                    .env(GITLESS_STAGE, "sentinel")
                    .env("PATH", path)
                    .current_dir(&root)
                    .output()
            };
            gitless_result(&probe(&present).unwrap(), GIT_SENTINEL).unwrap();
            // Exercise the original Windows lookup trap with a known executable.
            let child_path_only = probe(&empty);
            #[cfg(windows)]
            gitless_result(&child_path_only.unwrap(), GIT_SENTINEL).unwrap();
            #[cfg(not(windows))]
            assert_eq!(
                child_path_only.unwrap_err().kind(),
                std::io::ErrorKind::NotFound
            );

            let blocked = root.join("blocked-cli");
            fs::create_dir(&blocked).unwrap();
            fs::copy(
                present.join(if cfg!(windows) { "git.exe" } else { "git" }),
                blocked.join("git.exe"),
            )
            .unwrap();
            let rejected = reject_git_beside_cli(&blocked.join("ait-cli.exe")).unwrap_err();
            assert!(rejected.contains("Git beside selected CLI"), "{rejected}");
            #[cfg(windows)]
            {
                // Only this negative control selects a copy. The real lifecycle
                // below retains the original installed CLI and must see NotFound.
                let blocked_cli = blocked.join("ait-cli.exe");
                fs::copy(
                    cli_binary::selected(env!("CARGO_BIN_EXE_ait-cli")),
                    &blocked_cli,
                )
                .unwrap();
                let mut command = gitless_command(test, "isolated", &root, &empty);
                command.env("AIT_TEST_CLI_BIN", &blocked_cli);
                let output = bounded_output(command);
                let rejected = gitless_result(&output, GITLESS_COMPLETE).unwrap_err();
                assert!(rejected.contains("Git beside selected CLI"), "{rejected}");
            }

            let failure = bounded_output(gitless_command(test, "failure", &root, &empty));
            let propagated = gitless_result(&failure, GITLESS_COMPLETE).unwrap_err();
            assert!(propagated.contains(GITLESS_FAILURE), "{propagated}");
            for expected in [
                format!("executable: {:?}", std::env::current_exe().unwrap()),
                format!(
                    "argv: {:?}",
                    ["--exact", test, "--nocapture", "--test-threads=1"]
                ),
                format!("cwd: {:?}", Some(root.as_path())),
                "raw status:".to_string(),
                "exit code: Some(23)".to_string(),
                "stdout (".to_string(),
                "stderr (0 bytes):".to_string(),
                "CLI command returned non-success".to_string(),
            ] {
                assert!(
                    propagated.contains(&expected),
                    "missing {expected:?}: {propagated}"
                );
            }
            let isolated = bounded_output_with_limit(
                gitless_command(test, "isolated", &root, &empty),
                PEER_LIMIT * 2,
            );
            gitless_result(&isolated, GITLESS_COMPLETE)
                .unwrap_or_else(|failure| panic!("{failure}"));
            eprint!("{}", String::from_utf8_lossy(&isolated.stderr));
            println!("\n{GITLESS_PARENT_COMPLETE}");
            true
        }
        None => {
            let temp = TempDir::new().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let empty = root.join("empty-path");
            let present = root.join("git-present");
            fs::create_dir(&empty).unwrap();
            fs::create_dir(&present).unwrap();
            let git = present.join(if cfg!(windows) { "git.exe" } else { "git" });
            fs::copy(std::env::current_exe().unwrap(), git).unwrap();
            let output = bounded_output_with_limit(
                gitless_command(test, "parent", &root, &present),
                PEER_LIMIT * 4,
            );
            gitless_result(&output, GITLESS_PARENT_COMPLETE)
                .unwrap_or_else(|failure| panic!("{failure}"));
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            // Nested harness summaries stay captured: installed-case accounting
            // must still report exactly one execution of the existing selector.
            eprintln!("Git-free worker verified NotFound, lifecycle and isolation controls");
            true
        }
        Some(other) => panic!("Unknown private Git-free worker stage: {other}"),
    }
}

pub fn gitless_worker_complete() {
    println!("\n{GITLESS_COMPLETE}");
}

pub struct Fixture {
    pub temp: TempDir,
    pub root: PathBuf,
    pub search_path: Option<PathBuf>,
}

impl Fixture {
    pub fn new(gitless: bool) -> Self {
        let temp = TempDir::new().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("repo");
        fs::create_dir(&root).unwrap();
        let search_path = gitless.then(|| base.join("empty-path"));
        if let Some(path) = &search_path {
            fs::create_dir(path).unwrap();
        }
        parsed_cli(&root, &["init", "--json"], search_path.as_deref());
        let path = root.join(".ait/config.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        // Keep managed worktrees and seed caches beside the disposable fixture,
        // rather than using a host's automatically detected RAM volume.
        config["task_worktree"] = json!({"ephemeral_root": base.join("managed")});
        fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
        parsed_cli(
            &root,
            &[
                "config",
                "set",
                "--user-name",
                "workflow-acceptance",
                "--json",
            ],
            search_path.as_deref(),
        );
        Self {
            temp,
            root,
            search_path,
        }
    }

    pub fn edit(&self, name: &str) -> PathBuf {
        self.temp.path().canonicalize().unwrap().join(name)
    }

    pub fn json(&self, root: &Path, args: &[&str]) -> Value {
        parsed_cli(root, args, self.search_path.as_deref())
    }
}

pub fn cli(root: &Path, args: &[&str], search_path: Option<&Path>) -> Output {
    bounded_output(cli_command(root, args, search_path))
}

fn parsed_cli(root: &Path, args: &[&str], search_path: Option<&Path>) -> Value {
    parsed(&diagnostic_output(cli_command(root, args, search_path)))
}

pub fn checked_cli_json<T>(
    root: &Path,
    args: &[&str],
    search_path: Option<&Path>,
    check: impl FnOnce(&Output, &Value) -> Result<T, String>,
) -> (Value, T) {
    let command = cli_command(root, args, search_path);
    let identity = command_identity(&command);
    let output = bounded_output(command);
    let checked = (|| {
        let value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("Invalid CLI JSON: {error}"))?;
        let accepted = check(&output, &value)?;
        Ok::<_, String>((value, accepted))
    })();
    checked.unwrap_or_else(|error| panic!("{error}\n{}", command_output(&identity, &output)))
}

fn cli_command(root: &Path, args: &[&str], search_path: Option<&Path>) -> Command {
    let binary = cli_binary::selected(env!("CARGO_BIN_EXE_ait-cli"));
    assert!(binary.is_absolute(), "selected CLI must remain absolute");
    #[cfg(windows)]
    if search_path.is_some() {
        // The worker probe cannot certify this different executable directory.
        reject_git_beside_cli(&binary).unwrap();
    }
    let mut command = Command::new(binary);
    command
        .env("CODEX_HOME", native_home::path())
        .current_dir(root)
        .args(args);
    if let Some(path) = search_path {
        command.env("PATH", path);
    }
    command
}

fn command_identity(command: &Command) -> String {
    format!(
        "executable: {:?}\nargv: {:?}\ncwd: {:?}",
        command.get_program(),
        command.get_args().collect::<Vec<_>>(),
        command.get_current_dir(),
    )
}

fn command_output(identity: &str, output: &Output) -> String {
    // Keep the original streams before the identity for tail-based reporters.
    format!(
            "Workflow CLI failure\nstdout ({} bytes):\n{}\nstderr ({} bytes):\n{}\n{identity}\nraw status: {:?}\nexit code: {:?}",
            output.stdout.len(),
            String::from_utf8_lossy(&output.stdout),
            output.stderr.len(),
            String::from_utf8_lossy(&output.stderr),
            output.status,
            output.status.code(),
    )
}

fn diagnostic_output(command: Command) -> Output {
    let identity = command_identity(&command);
    let output = bounded_output(command);
    if !output.status.success() {
        eprintln!("{}", command_output(&identity, &output));
    }
    output
}

fn bounded_output(command: Command) -> Output {
    bounded_output_with_limit(command, COMMAND_LIMIT)
}

fn bounded_output_with_limit(mut command: Command, limit: Duration) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let err = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native command exceeded {limit:?}: {command:?}");
        }
        // Poll completion; coordination uses acknowledged socket barriers.
        thread::sleep(Duration::from_millis(10));
    };
    Output {
        status,
        stdout: out.join().unwrap(),
        stderr: err.join().unwrap(),
    }
}

pub fn parsed(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "CLI command returned non-success: {:?}",
        output.status,
    );
    serde_json::from_slice(&output.stdout).expect("Successful CLI output was not valid JSON")
}

pub fn cli_json(root: &Path, args: &[&str]) -> Value {
    parsed_cli(root, args, None)
}

pub fn request(root: &Path, args: &[&str]) -> Value {
    json!({"operation": "cli", "root": root, "args": args})
}

pub fn write_request(path: &Path, text: &str, append: bool) -> Value {
    json!({"operation": "write", "path": path, "text": text, "append": append})
}

fn send(peer: &mut BufReader<TcpStream>, value: &Value) {
    writeln!(peer.get_mut(), "{value}").unwrap();
}

fn receive(peer: &mut BufReader<TcpStream>) -> Value {
    let mut line = String::new();
    assert!(
        peer.read_line(&mut line).unwrap() > 0,
        "worker disconnected"
    );
    serde_json::from_str(&line).unwrap()
}

fn peer(stream: TcpStream) -> BufReader<TcpStream> {
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(PEER_LIMIT)).unwrap();
    stream.set_write_timeout(Some(PEER_LIMIT)).unwrap();
    stream.set_nodelay(true).unwrap();
    BufReader::new(stream)
}

/// The same named test runs in a child process, avoiding an extra CI test target.
pub fn run_worker() -> bool {
    let Ok(address) = std::env::var(WORKER_ADDRESS) else {
        return false;
    };
    let mut peer = peer(TcpStream::connect(address).unwrap());
    let index: usize = std::env::var("AIT_WORKFLOW_ACCEPTANCE_INDEX")
        .unwrap()
        .parse()
        .unwrap();
    send(
        &mut peer,
        &json!({"index": index, "pid": std::process::id()}),
    );
    loop {
        let request = receive(&mut peer);
        if request["operation"] == "stop" {
            break;
        }
        send(&mut peer, &json!({"armed": std::process::id()}));
        assert_eq!(receive(&mut peer), "run");
        let reply = match request["operation"].as_str().unwrap() {
            "cli" => {
                let args = request["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| arg.as_str().unwrap())
                    .collect::<Vec<_>>();
                let output = cli(Path::new(request["root"].as_str().unwrap()), &args, None);
                json!({"ok": output.status.success(),
                    "stdout": String::from_utf8_lossy(&output.stdout),
                    "stderr": String::from_utf8_lossy(&output.stderr)})
            }
            "write" => {
                let path = Path::new(request["path"].as_str().unwrap());
                let text = request["text"].as_str().unwrap().as_bytes();
                if request["append"] == true {
                    fs::OpenOptions::new()
                        .append(true)
                        .open(path)
                        .unwrap()
                        .write_all(text)
                        .unwrap();
                } else {
                    fs::write(path, text).unwrap();
                }
                json!({"ok": true})
            }
            operation => panic!("unexpected worker operation {operation}"),
        };
        send(&mut peer, &reply);
    }
    true
}

pub fn reply_json(reply: &Value) -> Value {
    assert_eq!(reply["ok"], true, "{reply}");
    serde_json::from_str(reply["stdout"].as_str().unwrap()).unwrap()
}

pub struct Workers {
    children: Vec<Child>,
    peers: Vec<BufReader<TcpStream>>,
}

impl Workers {
    pub fn spawn(test: &str, count: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let mut workers = Self {
            children: Vec::new(),
            peers: Vec::new(),
        };
        for index in 0..count {
            workers.children.push(
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", test, "--nocapture", "--test-threads=1"])
                    .env(WORKER_ADDRESS, address.to_string())
                    .env("AIT_WORKFLOW_ACCEPTANCE_INDEX", index.to_string())
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit())
                    .spawn()
                    .unwrap(),
            );
        }
        let deadline = Instant::now() + PEER_LIMIT;
        let mut indexed = (0..count).map(|_| None).collect::<Vec<_>>();
        while indexed.iter().any(Option::is_none) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let mut connection = peer(stream);
                    let ready = receive(&mut connection);
                    let index = ready["index"].as_u64().unwrap() as usize;
                    assert_eq!(ready["pid"], workers.children[index].id());
                    assert!(indexed[index].replace(connection).is_none());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    workers.assert_live();
                    assert!(
                        Instant::now() < deadline,
                        "workers did not reach the barrier"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("worker accept: {error}"),
            }
        }
        assert_eq!(
            workers
                .children
                .iter()
                .map(Child::id)
                .collect::<BTreeSet<_>>()
                .len(),
            count
        );
        workers.peers = indexed.into_iter().map(Option::unwrap).collect();
        workers
    }

    fn assert_live(&mut self) {
        for child in &mut self.children {
            assert!(
                child.try_wait().unwrap().is_none(),
                "worker {} exited early",
                child.id()
            );
        }
    }

    /// Every requested worker acknowledges readiness before any gets the run signal.
    pub fn together(&mut self, requests: Vec<(usize, Value)>) -> Vec<(usize, Value)> {
        self.assert_live();
        assert_eq!(
            requests
                .iter()
                .map(|(index, _)| index)
                .collect::<BTreeSet<_>>()
                .len(),
            requests.len()
        );
        for (index, request) in &requests {
            send(&mut self.peers[*index], request);
        }
        for (index, _) in &requests {
            assert_eq!(
                receive(&mut self.peers[*index])["armed"],
                self.children[*index].id()
            );
        }
        for (index, _) in &requests {
            send(&mut self.peers[*index], &json!("run"));
        }
        requests
            .into_iter()
            .map(|(index, _)| (index, receive(&mut self.peers[index])))
            .collect()
    }

    pub fn one(&mut self, index: usize, request: Value) -> Value {
        self.together(vec![(index, request)]).remove(0).1
    }

    pub fn finish(mut self) {
        for connection in &mut self.peers {
            send(connection, &json!({"operation": "stop"}));
        }
        for child in &mut self.children {
            let deadline = Instant::now() + COMMAND_LIMIT;
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "worker {} failed: {status}", child.id());
                    break;
                }
                assert!(Instant::now() < deadline, "worker did not stop");
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        for connection in &mut self.peers {
            let _ = connection.get_mut().shutdown(Shutdown::Both);
        }
        for child in &mut self.children {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

pub struct FinishContext<'a> {
    pub task: &'a str,
    pub root: &'a Path,
    pub edit: &'a Path,
    pub baseline: &'a Value,
    pub checkpoint: &'a Value,
    pub dirty: bool,
}

pub struct FinishIdentity {
    snapshot: Value,
    receipt: Value,
}

fn finish_fields(value: &Value, fields: &[(&str, Value)]) -> Result<(), String> {
    for (field, expected) in fields {
        if value.pointer(field) != Some(expected) {
            return Err(format!(
                "finish {field}: expected {expected}, got {:?}",
                value.pointer(field)
            ));
        }
    }
    Ok(())
}

fn finish_common(
    context: &FinishContext<'_>,
    identity: &FinishIdentity,
    value: &Value,
    recovery: bool,
) -> Result<(), String> {
    finish_fields(
        value,
        &[
            ("/task_id", json!(context.task)),
            ("/task/task_id", json!(context.task)),
            ("/task_status", json!("completed")),
            ("/task/status", json!("completed")),
            ("/apply_status", json!("done")),
            (
                "/execution_status",
                json!(if recovery {
                    "already_landed"
                } else {
                    "applied"
                }),
            ),
            ("/mode", json!("local")),
            ("/target_line", json!("main")),
            ("/current_line", json!("main")),
            ("/line_name", json!("main")),
            ("/change", Value::Null),
            ("/change_id", Value::Null),
            ("/change_ref", Value::Null),
            ("/change_status", Value::Null),
            ("/landed_snapshot_id", identity.snapshot.clone()),
            ("/local_finish", identity.receipt.clone()),
            ("/task/local_finish", identity.receipt.clone()),
            ("/local_finish/status", json!("succeeded")),
            ("/local_finish/task_id", json!(context.task)),
            ("/local_finish/target_line", json!("main")),
            ("/local_finish/change_ref", Value::Null),
            (
                "/local_finish/landed_snapshot_id",
                identity.snapshot.clone(),
            ),
            (
                "/local_finish/pre_land_target_snapshot_id",
                context.baseline.clone(),
            ),
            (
                "/previous_target_head_snapshot_id",
                context.baseline.clone(),
            ),
            ("/workspace_action", json!("restored")),
            ("/repo_root_restore/status", json!("restored")),
            ("/repo_root_restore/applied", json!(true)),
            ("/repo_root_restore/workspace_root", json!(context.root)),
            (
                "/repo_root_restore/target_snapshot_id",
                identity.snapshot.clone(),
            ),
            ("/repo_root_restore/dirty_outside_paths", json!([])),
            ("/plan_checklist_closeout/status", json!("synced")),
            ("/plan_markdown_materialization/status", json!("in_sync")),
            ("/plan_markdown_materialization/drift", json!([])),
            ("/task_land_contract/scope", json!("local")),
            (
                "/task_land_contract/version",
                json!("task-land-plan-closeout/v1"),
            ),
            (
                "/task_land_contract/recovery_policy",
                json!("idempotent_phase_resume"),
            ),
        ],
    )?;
    if identity.receipt["land_ordinal"].as_u64().is_none() {
        return Err("finish receipt has no valid ordinal".into());
    }
    if !recovery || value.get("post_finish_hooks").is_some() {
        finish_fields(
            value,
            &[
                ("/post_finish_hooks/status", json!("skipped")),
                ("/post_finish_hooks/reason", json!("no_hooks_configured")),
            ],
        )?;
    }
    if recovery {
        finish_fields(
            value,
            &[
                ("/auto_snapshot", Value::Null),
                ("/repo_root_restore/plan/write_count", json!(0)),
                ("/repo_root_restore/plan/remove_count", json!(0)),
            ],
        )
    } else if !context.dirty {
        finish_fields(value, &[("/auto_snapshot", Value::Null)])?;
        if identity.snapshot != *context.checkpoint {
            return Err("clean finish changed the saved checkpoint Snapshot".into());
        }
        Ok(())
    } else {
        finish_fields(
            value,
            &[
                ("/auto_snapshot/snapshot_id", identity.snapshot.clone()),
                ("/auto_snapshot/task_id", json!(context.task)),
                ("/auto_snapshot/change_id", Value::Null),
                ("/auto_snapshot/newly_created", json!(true)),
                (
                    "/auto_snapshot/parent_snapshot_id",
                    context.checkpoint.clone(),
                ),
            ],
        )
    }
}

fn finish_other_phases(value: &Value, allowed: &[&str]) -> Result<(), String> {
    for (phase, component) in value.as_object().ok_or("finish is not an object")? {
        if !allowed.contains(&phase.as_str())
            && matches!(
                component["status"].as_str(),
                Some("failed" | "blocked" | "partial" | "pending" | "deferred")
            )
        {
            return Err(format!("unrelated finish failure: {phase}"));
        }
    }
    Ok(())
}

fn finish_windows_partial(
    context: &FinishContext<'_>,
    identity: &FinishIdentity,
    value: &Value,
) -> Result<(), String> {
    finish_common(context, identity, value, false)?;
    finish_fields(
        value,
        &[
            ("/closeout_status", json!("partial")),
            ("/bound_worktree_cleanup/status", json!("failed")),
            ("/bound_worktree_cleanup/task_id", json!(context.task)),
            ("/bound_line_closeout/status", json!("failed")),
            (
                "/bound_line_closeout/reason",
                json!("feature_line_closeout_failed"),
            ),
            (
                "/bound_line_closeout/error",
                json!("Bound worktree cleanup failed before Line closeout."),
            ),
            ("/bound_line_closeout/task_id", json!(context.task)),
            ("/bound_line_closeout/task_status", json!("completed")),
            ("/main_seed_sync/status", json!("pending")),
            (
                "/main_seed_sync/reason",
                json!("task_closeout_recovery_pending"),
            ),
            ("/main_seed_sync/line_name", json!("main")),
            ("/automatic_reconciliation/status", json!("skipped")),
            (
                "/automatic_reconciliation/reason",
                json!("task_closeout_recovery_pending"),
            ),
            (
                "/closeout_recovery/code",
                json!("resume_task_land_closeout"),
            ),
            (
                "/closeout_recovery/command",
                json!(format!("ait task finish {} --local", context.task)),
            ),
            ("/closeout_recovery/idempotent", json!(true)),
        ],
    )?;
    if !value["bound_worktree_cleanup"]["error"]
        .as_str()
        .is_some_and(|error| error.ends_with("(os error 32)"))
    {
        return Err("finish cleanup is not exactly Windows sharing violation32".into());
    }
    finish_other_phases(
        value,
        &[
            "bound_worktree_cleanup",
            "bound_line_closeout",
            "main_seed_sync",
        ],
    )
}

fn finish_complete(
    context: &FinishContext<'_>,
    identity: &FinishIdentity,
    value: &Value,
    recovery: bool,
) -> Result<(), String> {
    finish_common(context, identity, value, recovery)?;
    finish_fields(
        value,
        &[
            ("/closeout_status", json!("complete")),
            ("/bound_worktree_cleanup/status", json!("removed")),
            ("/bound_worktree_cleanup/task_id", json!(context.task)),
            ("/bound_worktree_cleanup/worktree/removed", json!(true)),
            ("/bound_worktree_cleanup/worktree/deleted_path", json!(true)),
            ("/bound_worktree_cleanup/worktree/path", json!(context.edit)),
            ("/bound_line_closeout/status", json!("archived")),
            ("/bound_line_closeout/task_id", json!(context.task)),
            ("/bound_line_closeout/task_status", json!("completed")),
            ("/bound_line_closeout/target_line", json!("main")),
            (
                "/bound_line_closeout/expected_revision_snapshot_id",
                identity.snapshot.clone(),
            ),
            ("/bound_line_closeout/local/line/status", json!("archived")),
            (
                "/bound_line_closeout/local/line/head_snapshot_id",
                identity.snapshot.clone(),
            ),
            ("/automatic_reconciliation/status", json!("completed")),
        ],
    )?;
    if !value.get("closeout_recovery").is_none_or(Value::is_null)
        || value["main_seed_sync"]["status"].as_str().is_none()
    {
        return Err("finish still requires closeout recovery".into());
    }
    finish_other_phases(value, &[])
}

fn finish_identity(context: &FinishContext<'_>, value: &Value) -> Result<FinishIdentity, String> {
    let snapshot = if context.dirty {
        value["auto_snapshot"]["snapshot_id"].clone()
    } else {
        context.checkpoint.clone()
    };
    if !snapshot.as_str().is_some_and(|id| {
        id.len() == 16
            && id.starts_with("SNP-")
            && id.as_bytes()[4..]
                .iter()
                .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(b))
    }) {
        return Err("finish did not select the exact Task Snapshot".into());
    }
    Ok(FinishIdentity {
        snapshot,
        receipt: value["local_finish"].clone(),
    })
}

pub fn finish_initial(
    context: &FinishContext<'_>,
    value: &Value,
    code: Option<i32>,
    windows: bool,
) -> Result<FinishIdentity, String> {
    let identity = finish_identity(context, value)?;
    match code {
        Some(0) => finish_complete(context, &identity, value, false)?,
        Some(2) if windows => finish_windows_partial(context, &identity, value)?,
        _ => return Err(format!("unexpected first finish exit {code:?}")),
    }
    Ok(identity)
}

fn worker_finish_initial(
    context: &FinishContext<'_>,
    reply: &Value,
    windows: bool,
) -> Result<Option<(Value, FinishIdentity)>, String> {
    let ok = reply["ok"]
        .as_bool()
        .ok_or("worker omitted success status")?;
    let stdout = reply["stdout"].as_str().ok_or("worker omitted stdout")?;
    let stderr = reply["stderr"].as_str().ok_or("worker omitted stderr")?;
    if !ok && stdout.is_empty() && stderr.contains("ait worktree rebase --onto main") {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(stdout)
        .map_err(|error| format!("Invalid worker finish JSON: {error}"))?;
    let identity = finish_identity(context, &value)?;
    if ok {
        finish_complete(context, &identity, &value, false)?;
    } else if windows && stderr.is_empty() {
        finish_windows_partial(context, &identity, &value)?;
    } else {
        return Err("unrecognized non-successful worker finish".into());
    }
    Ok(Some((value, identity)))
}

/// Classify only after together() has collected the whole acknowledged batch.
/// The existing worker envelope has no numeric CLI exit status.
pub fn finish_reply(context: &FinishContext<'_>, reply: &Value) -> Option<(Value, FinishIdentity)> {
    let command = cli_command(
        context.edit,
        &[
            "task",
            "finish",
            context.task,
            "--local",
            "--json",
            "--full",
        ],
        None,
    );
    let diagnostic = || {
        format!(
            "{}\nraw status: unavailable in worker envelope\nreply: {reply}",
            command_identity(&command)
        )
    };
    let result = worker_finish_initial(context, reply, cfg!(windows))
        .unwrap_or_else(|error| panic!("{error}\n{}", diagnostic()));
    if result.is_none() {
        let task = cli_json(
            context.root,
            &["task", "show", context.task, "--local", "--json"],
        );
        let head = cli_json(context.root, &["line", "show", "main", "--json"]);
        assert!(
            task["task_id"] == context.task
                && task["status"].as_str().is_some()
                && task["status"] != "completed"
                && task["local_finish"].is_null()
                && head["head_snapshot_id"] != *context.baseline,
            "stale rejection must precede apply: {task}\n{}",
            diagnostic()
        );
    }
    result
}

/// Resume only a strictly verified applied partial, once, from the root.
/// Historical completed-receipt retries never enter this fresh-apply path.
pub fn finish_closeout(
    context: &FinishContext<'_>,
    first: Value,
    identity: &FinishIdentity,
    search_path: Option<&Path>,
) -> Value {
    if first["closeout_status"] != "partial" {
        return first;
    }
    let receipts = fs::read(context.root.join(".ait/binary-db/land.bin")).unwrap();
    let count = parsed_cli(context.root, &["status", "--json", "--full"], search_path)
        ["snapshot_count"]
        .clone();
    let (recovered, ()) = checked_cli_json(
        context.root,
        &[
            "task",
            "finish",
            context.task,
            "--local",
            "--json",
            "--full",
        ],
        search_path,
        |output, value| {
            finish_recovered(context, identity, value, output.status.code())?;
            let after = fs::read(context.root.join(".ait/binary-db/land.bin"))
                .map_err(|error| error.to_string())?;
            let state = parsed_cli(context.root, &["status", "--json", "--full"], search_path);
            finish_no_reapply(&receipts, &after, &count, &state["snapshot_count"])?;
            if state["head_snapshot_id"] != identity.snapshot || context.edit.exists() {
                return Err("root recovery changed target or left the worktree".into());
            }
            Ok(())
        },
    );
    recovered
}

pub fn finish_from_edit(context: &FinishContext<'_>, args: &[&str]) -> Value {
    let (first, identity) = checked_cli_json(context.edit, args, None, |output, value| {
        finish_initial(context, value, output.status.code(), cfg!(windows))
    });
    finish_guard_controls(context, &identity, &first);
    finish_closeout(context, first, &identity, None)
}

fn finish_no_reapply(
    before: &[u8],
    after: &[u8],
    count: &Value,
    recovered_count: &Value,
) -> Result<(), String> {
    if before != after || count.as_u64().is_none() || count != recovered_count {
        Err("finish recovery repeated apply or created an extra Snapshot".into())
    } else {
        Ok(())
    }
}

fn finish_recovered(
    context: &FinishContext<'_>,
    identity: &FinishIdentity,
    value: &Value,
    code: Option<i32>,
) -> Result<(), String> {
    if code != Some(0) {
        return Err(format!("root finish recovery failed with {code:?}"));
    }
    finish_complete(context, identity, value, true)
}

pub fn finish_guard_controls(
    context: &FinishContext<'_>,
    identity: &FinishIdentity,
    first: &Value,
) {
    // Exercise response guards on every host; only a real Windows run can
    // prove the actual partial-closeout/root-resume path below.
    let mut partial = first.clone();
    partial["closeout_status"] = json!("partial");
    partial["bound_worktree_cleanup"] = json!({
        "status":"failed", "task_id":context.task,
        "error":"The process cannot access the file because it is being used by another process. (os error 32)"
    });
    partial["bound_line_closeout"] = json!({
        "status":"failed", "reason":"feature_line_closeout_failed",
        "error":"Bound worktree cleanup failed before Line closeout.",
        "task_id":context.task, "task_status":"completed"
    });
    partial["main_seed_sync"] = json!({
        "status":"pending", "reason":"task_closeout_recovery_pending", "line_name":"main"
    });
    partial["automatic_reconciliation"] = json!({
        "status":"skipped", "reason":"task_closeout_recovery_pending"
    });
    partial["closeout_recovery"] = json!({
        "code":"resume_task_land_closeout", "idempotent":true,
        "command":format!("ait task finish {} --local", context.task)
    });
    finish_initial(context, &partial, Some(2), true).unwrap();
    let worker =
        |ok: bool, value: &Value| json!({"ok":ok, "stdout":value.to_string(), "stderr":""});
    assert!(
        worker_finish_initial(context, &worker(false, &partial), true)
            .unwrap()
            .is_some()
    );
    assert!(worker_finish_initial(context, &worker(false, &partial), false).is_err());
    assert!(worker_finish_initial(context, &worker(true, &partial), true).is_err());
    let stale = json!({"ok":false,"stdout":"","stderr":"Error: stale candidate; ait worktree rebase --onto main"});
    assert!(worker_finish_initial(context, &stale, true)
        .unwrap()
        .is_none());
    let mut unrelated_stderr = worker(false, &partial);
    unrelated_stderr["stderr"] = json!("unrelated failure");
    assert!(worker_finish_initial(context, &unrelated_stderr, true).is_err());
    let mut false_stale = stale.clone();
    false_stale["stdout"] = json!(partial.to_string());
    assert!(worker_finish_initial(context, &false_stale, false).is_err());
    false_stale["stdout"] = json!("unrelated output");
    assert!(worker_finish_initial(context, &false_stale, true).is_err());
    false_stale["stdout"] = json!("");
    false_stale["stderr"] = json!("unrelated failure");
    assert!(worker_finish_initial(context, &false_stale, true).is_err());
    let mut wrong_selection = partial.clone();
    wrong_selection["auto_snapshot"] = if context.dirty {
        Value::Null
    } else {
        json!({"snapshot_id":context.checkpoint,"newly_created":true})
    };
    assert!(worker_finish_initial(context, &worker(false, &wrong_selection), true).is_err());
    assert!(finish_initial(context, &partial, Some(2), false).is_err());
    assert!(finish_initial(context, &partial, Some(1), true).is_err());
    assert!(finish_initial(context, &partial, Some(0), true).is_err());
    let common_drift = [
        ("/task_id", json!("LT-9999")),
        ("/landed_snapshot_id", json!("wrong-snapshot")),
        (
            "/local_finish/land_ordinal",
            json!(identity.receipt["land_ordinal"].as_u64().unwrap() + 1),
        ),
        ("/task/local_finish/target_line", json!("other")),
        ("/target_line", json!("other")),
        ("/apply_status", json!("failed")),
        ("/task_status", json!("active")),
        ("/task/status", json!("active")),
        ("/plan_checklist_closeout/status", json!("failed")),
        ("/plan_markdown_materialization/status", json!("failed")),
        ("/repo_root_restore/status", json!("failed")),
    ];
    for (field, replacement) in &common_drift {
        let mut invalid = partial.clone();
        *invalid.pointer_mut(field).unwrap() = replacement.clone();
        assert!(
            finish_windows_partial(context, identity, &invalid).is_err(),
            "accepted {field}"
        );
    }
    for (field, replacement) in [
        (
            "/bound_worktree_cleanup/error",
            json!("sharing violation (os error 320)"),
        ),
        (
            "/bound_worktree_cleanup/error",
            json!("Permission denied (os error 13)"),
        ),
        ("/bound_line_closeout/reason", json!("unrelated")),
        ("/main_seed_sync/status", json!("failed")),
        ("/automatic_reconciliation/reason", json!("unrelated")),
        ("/post_finish_hooks/status", json!("failed")),
        (
            "/closeout_recovery/command",
            json!("ait task finish LT-9999 --local"),
        ),
        ("/closeout_recovery/idempotent", json!(false)),
    ] {
        let mut invalid = partial.clone();
        *invalid.pointer_mut(field).unwrap() = replacement;
        assert!(
            finish_windows_partial(context, identity, &invalid).is_err(),
            "accepted {field}"
        );
    }
    let mut unrelated = partial.clone();
    unrelated["other_mandatory_step"] = json!({"status":"failed"});
    assert!(finish_windows_partial(context, identity, &unrelated).is_err());

    let mut recovered = first.clone();
    recovered["execution_status"] = json!("already_landed");
    recovered["auto_snapshot"] = Value::Null;
    recovered["closeout_status"] = json!("complete");
    recovered["closeout_recovery"] = Value::Null;
    recovered["bound_worktree_cleanup"] = json!({
        "status":"removed", "task_id":context.task,
        "worktree":{"removed":true, "deleted_path":true, "path":context.edit}
    });
    recovered["bound_line_closeout"] = json!({
        "status":"archived", "task_id":context.task, "task_status":"completed",
        "target_line":"main", "expected_revision_snapshot_id":identity.snapshot,
        "local":{"line":{"status":"archived", "head_snapshot_id":identity.snapshot}}
    });
    recovered["main_seed_sync"] = json!({"status":"refreshed"});
    recovered["automatic_reconciliation"] = json!({"status":"completed"});
    recovered["repo_root_restore"]["plan"]["write_count"] = json!(0);
    recovered["repo_root_restore"]["plan"]["remove_count"] = json!(0);
    finish_recovered(context, identity, &recovered, Some(0)).unwrap();
    assert!(finish_recovered(context, identity, &recovered, Some(2)).is_err());
    for (field, replacement) in common_drift.iter().cloned().chain([
        ("/execution_status", json!("applied")),
        ("/auto_snapshot", json!({"newly_created":true})),
        ("/closeout_status", json!("partial")),
        ("/bound_worktree_cleanup/status", json!("failed")),
        ("/bound_worktree_cleanup/worktree/removed", json!(false)),
        (
            "/bound_worktree_cleanup/worktree/path",
            json!("other-worktree"),
        ),
        ("/bound_line_closeout/status", json!("active")),
        ("/main_seed_sync/status", json!("pending")),
        ("/repo_root_restore/plan/write_count", json!(1)),
        ("/repo_root_restore/plan/remove_count", json!(1)),
    ]) {
        let mut invalid = recovered.clone();
        *invalid.pointer_mut(field).unwrap() = replacement;
        assert!(
            finish_recovered(context, identity, &invalid, Some(0)).is_err(),
            "accepted {field}"
        );
    }
    assert!(finish_no_reapply(b"receipt", b"receipt", &json!(3), &json!(3)).is_ok());
    assert!(finish_no_reapply(b"receipt", b"duplicate", &json!(3), &json!(3)).is_err());
    assert!(finish_no_reapply(b"receipt", b"receipt", &json!(3), &json!(4)).is_err());
}
