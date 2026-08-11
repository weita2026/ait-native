use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "ait-server-installed-lifecycle-{name}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create isolated lifecycle test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove isolated lifecycle test directory");
    }
}

fn run_server(arguments: &[&str]) -> Output {
    let mut command = clean_server_command();
    command
        .args(arguments)
        .output()
        .expect("execute ait-server")
}

fn clean_server_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ait-server"));
    for name in [
        "AIT_NATIVE_SERVER_DATA",
        "AIT_RUNTIME_DATA",
        "AIT_NATIVE_SERVER_BINARY_ACTIVATION",
        "AIT_NATIVE_SERVER_BINARY_REGISTRY",
        "AIT_NATIVE_SERVER_FRESH_BOOTSTRAP",
        "AIT_NATIVE_SERVER_CI_RAM_ROOT",
        "AIT_CI_RAM_ROOT",
        "AIT_NATIVE_SERVER_CI_STARTUP_ADMISSION",
        "AIT_SERVER_STARTUP_PROBE_ONLY",
    ] {
        command.env_remove(name);
    }
    command
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn init_is_idempotent_and_creates_only_an_empty_runtime_root() {
    let directory = TestDirectory::new("init");
    let root = directory.path().join("server-data");
    let root_text = root.to_str().unwrap();

    let first = run_server(&["init", "--data", root_text]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("initialization created"));
    assert!(root.join("binary-v0/active.json").is_file());

    let second = run_server(&["init", "--data", root_text]);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(String::from_utf8_lossy(&second.stdout).contains("initialization existing"));
}

#[test]
fn init_refuses_a_nonempty_unactivated_runtime_root() {
    let directory = TestDirectory::new("nonempty");
    fs::write(directory.path().join("existing-data"), b"must survive\n").unwrap();
    let root_text = directory.path().to_str().unwrap();

    let output = run_server(&["init", "--data", root_text]);

    assert_eq!(output.status.code(), Some(78));
    assert!(String::from_utf8_lossy(&output.stderr).contains("non-empty"));
    assert_eq!(
        fs::read(directory.path().join("existing-data")).unwrap(),
        b"must survive\n"
    );
    assert!(!directory.path().join("binary-v0/active.json").exists());
}

#[test]
fn deferred_probe_checks_durable_storage_without_requiring_a_ram_mount() {
    let directory = TestDirectory::new("probe");
    let root = directory.path().join("server-data");
    let root_text = root.to_str().unwrap();

    let output = run_server(&["probe", "--data", root_text, "--defer-ci-admission"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("startup probe ok"));
    assert!(stdout.contains("ci_ram_runtime_root=deferred"));
    assert!(!root.join("binary-v0/active.json").exists());
}

#[test]
fn installed_run_initializes_then_serves_from_an_explicit_root() {
    let directory = TestDirectory::new("run");
    let root = directory.path().join("server-data");
    let root_text = root.to_str().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);

    let child = clean_server_command()
        .args([
            "run",
            "--data",
            root_text,
            "--init-if-missing",
            "--defer-ci-admission",
        ])
        .env("AITSERVER_LISTEN", address.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start installed server smoke");
    let mut child = ChildGuard(child);

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut ready = false;
    while Instant::now() < deadline {
        if let Some(status) = child.0.try_wait().unwrap() {
            panic!("installed server exited before readiness: {status}");
        }
        if health_is_ready(address) {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(ready, "installed server did not become ready at {address}");
    assert!(root.join("binary-v0/active.json").is_file());
}

fn health_is_ready(address: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    if stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut response = Vec::new();
    if stream.read_to_end(&mut response).is_err() {
        return false;
    }
    response.starts_with(b"HTTP/1.1 200")
}
