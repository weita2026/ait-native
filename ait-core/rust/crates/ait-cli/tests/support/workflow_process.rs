//! Disposable native fixtures and process barriers for workflow acceptance.
#![allow(dead_code)]

use crate::native_home;
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
        parsed(&cli(&root, &["init", "--json"], search_path.as_deref()));
        let path = root.join(".ait/config.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        // Keep managed worktrees and seed caches beside the disposable fixture,
        // rather than using a host's automatically detected RAM volume.
        config["task_worktree"] = json!({"ephemeral_root": base.join("managed")});
        fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
        parsed(&cli(
            &root,
            &[
                "config",
                "set",
                "--user-name",
                "workflow-acceptance",
                "--json",
            ],
            search_path.as_deref(),
        ));
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
        parsed(&cli(root, args, self.search_path.as_deref()))
    }
}

pub fn cli(root: &Path, args: &[&str], search_path: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ait-cli"));
    command
        .env("CODEX_HOME", native_home::path())
        .current_dir(root)
        .args(args);
    if let Some(path) = search_path {
        command.env("PATH", path);
    }
    bounded_output(command)
}

fn bounded_output(mut command: Command) -> Output {
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
    let deadline = Instant::now() + COMMAND_LIMIT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native command exceeded {COMMAND_LIMIT:?}: {command:?}");
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
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

pub fn cli_json(root: &Path, args: &[&str]) -> Value {
    parsed(&cli(root, args, None))
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
