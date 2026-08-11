#![allow(dead_code)]

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

const RUST_WORKSPACE_ROOT_OVERRIDE: &str = "AIT_TEST_RUST_WORKSPACE_ROOT";

pub fn rust_workspace_root() -> PathBuf {
    if let Some(override_root) = env::var_os(RUST_WORKSPACE_ROOT_OVERRIDE) {
        let candidate = PathBuf::from(override_root);
        return validated_rust_workspace_root(&candidate).unwrap_or_else(|| {
            panic!(
                "{RUST_WORKSPACE_ROOT_OVERRIDE} points at `{}`, which is not an AIT Rust workspace",
                candidate.display()
            )
        });
    }

    let mut starts = Vec::new();
    if let Ok(current_dir) = env::current_dir() {
        starts.push(("current_dir", current_dir));
    }
    if let Ok(current_exe) = env::current_exe() {
        starts.push(("current_exe", current_exe));
    }
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        starts.push(("compiled_manifest_dir", PathBuf::from(manifest_dir)));
    }

    let mut checked = BTreeSet::new();
    let mut start_descriptions = Vec::new();
    for (source, start) in starts {
        start_descriptions.push(format!("{source}={}", start.display()));
        for ancestor in start.ancestors() {
            for candidate in [ancestor.to_path_buf(), ancestor.join("rust")] {
                if !checked.insert(candidate.clone()) {
                    continue;
                }
                if let Some(root) = validated_rust_workspace_root(&candidate) {
                    return root;
                }
            }
        }
    }

    panic!(
        "could not locate the active AIT Rust workspace at runtime; set {RUST_WORKSPACE_ROOT_OVERRIDE} explicitly; starts: {}; checked: {}",
        start_descriptions.join(", "),
        checked
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub fn repository_root() -> PathBuf {
    rust_workspace_root()
        .parent()
        .expect("AIT Rust workspace must have a repository parent")
        .to_path_buf()
}

pub fn crate_root(crate_name: &str) -> PathBuf {
    let root = rust_workspace_root().join("crates").join(crate_name);
    assert!(
        root.join("Cargo.toml").is_file(),
        "AIT test crate `{crate_name}` is missing at {}",
        root.display()
    );
    root
}

pub fn cargo_binary(binary_name: &str, compiled_candidate: Option<&str>) -> PathBuf {
    let mut checked = Vec::new();
    if let Ok(current_exe) = env::current_exe() {
        if let Some(profile_dir) = cargo_profile_dir_for_test_executable(&current_exe) {
            let candidate = profile_dir.join(executable_name(binary_name));
            if candidate.is_file() {
                return candidate;
            }
            checked.push(candidate);
        }
    }

    let runtime_key = format!("CARGO_BIN_EXE_{binary_name}");
    if let Some(candidate) = env::var_os(&runtime_key).map(PathBuf::from) {
        if candidate.is_file() {
            return candidate;
        }
        checked.push(candidate);
    }

    if let Some(candidate) = compiled_candidate.map(PathBuf::from) {
        if candidate.is_file() {
            return candidate;
        }
        checked.push(candidate);
    }

    panic!(
        "could not locate current Cargo binary `{binary_name}`; stale compile-time paths are not executable; checked: {}",
        checked
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub fn request_worker_shutdown(
    repository_root: &Path,
    transport: &str,
    worker_name: &str,
    pid: u32,
) {
    let runtime_root = repository_root.join(".ait/agent-runtime");
    fs::create_dir_all(&runtime_root).expect("create worker runtime directory");
    let path = runtime_root.join(format!(
        "{}-{}-termination.json",
        transport.trim().to_ascii_lowercase(),
        worker_name.trim().to_ascii_lowercase()
    ));
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(
        &temporary,
        format!(
            "{{\"pid\":{pid},\"reason\":\"integration_test_stop\",\"worker_name\":\"{worker_name}\"}}\n"
        ),
    )
    .expect("write worker termination context");
    fs::rename(&temporary, &path).expect("publish worker termination context");
}

pub fn wait_for_child_exit(child: &mut Child, label: &str, timeout: Duration) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{label} did not stop after receiving its termination context");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn validated_rust_workspace_root(candidate: &Path) -> Option<PathBuf> {
    if !candidate.join("Cargo.toml").is_file()
        || !candidate.join("crates/ait-core/Cargo.toml").is_file()
        || !candidate.join("crates/ait-cli/Cargo.toml").is_file()
    {
        return None;
    }
    candidate
        .canonicalize()
        .ok()
        .or_else(|| Some(candidate.to_path_buf()))
}

fn cargo_profile_dir_for_test_executable(current_exe: &Path) -> Option<PathBuf> {
    let parent = current_exe.parent()?;
    if parent.file_name().is_some_and(|name| name == "deps") {
        parent.parent().map(Path::to_path_buf)
    } else {
        Some(parent.to_path_buf())
    }
}

fn executable_name(binary_name: &str) -> String {
    format!("{binary_name}{}", env::consts::EXE_SUFFIX)
}
