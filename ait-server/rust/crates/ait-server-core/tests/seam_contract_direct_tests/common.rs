use ait_server_core::foundation::scheduler::SchedulerPolicy;
use serde_json::{json, Value as JsonValue};
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn run_seam(args: &[&str]) -> Output {
    Command::new(seam_binary_path())
        .args(args)
        .output()
        .expect("seam binary should run")
}

#[cfg(feature = "legacy-postgres-runtime")]
fn run_seam_without_postgres_dsn(args: &[&str]) -> Output {
    Command::new(seam_binary_path())
        .args(args)
        .env_remove("AIT_NATIVE_SERVER_POSTGRES_DSN")
        .output()
        .expect("seam binary should run")
}

fn run_seam_with_stdin(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(seam_binary_path())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("seam binary should spawn");
    child
        .stdin
        .take()
        .expect("stdin should be piped")
        .write_all(stdin.as_bytes())
        .expect("stdin payload should be written");
    child.wait_with_output().expect("seam binary should run")
}

fn stdout_json(output: &Output) -> JsonValue {
    assert!(
        output.status.success(),
        "expected seam command to succeed, stderr: {}",
        stderr_text(output)
    );
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be utf-8");
    serde_json::from_str(stdout.trim()).expect("stdout should be JSON")
}

fn assert_failed_with(output: &Output, expected_stderr: &str) {
    assert!(
        !output.status.success(),
        "expected seam command to fail, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "failed seam commands should not emit stdout"
    );
    assert!(
        stderr_text(output).contains(expected_stderr),
        "stderr did not contain {expected_stderr:?}: {}",
        stderr_text(output)
    );
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr should be utf-8")
}

fn temp_payload_file(name: &str, payload: &str) -> PathBuf {
    let path = env::temp_dir().join(format!(
        "ait-server-core-seam-{}-{name}.json",
        std::process::id()
    ));
    fs::write(&path, payload).expect("payload file should be written");
    path
}

fn seam_binary_path() -> PathBuf {
    let cargo_bin = PathBuf::from(env!("CARGO_BIN_EXE_ait-server-core-seam"));
    if cargo_bin.is_file() {
        return cargo_bin;
    }

    if let Ok(current_exe) = env::current_exe() {
        if let Some(target_profile_dir) = current_exe.parent().and_then(|deps| deps.parent()) {
            let profile_candidate = target_profile_dir.join("ait-server-core-seam");
            if profile_candidate.is_file() {
                return profile_candidate;
            }
        }
    }

    cargo_bin
}
