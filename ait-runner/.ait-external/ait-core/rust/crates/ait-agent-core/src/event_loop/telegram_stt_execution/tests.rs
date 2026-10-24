use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ait_core::json_support::json;
use tempfile::{tempdir, TempDir};

use super::*;

struct Fixture {
    _temp: TempDir,
    program: PathBuf,
    audio: PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempdir().unwrap();
    let source = temp.path().join("native_stt_fixture.rs");
    let program = temp.path().join("native-stt-fixture");
    let audio = temp.path().join("private-audio.ogg");
    fs::write(&audio, b"fixture audio bytes").unwrap();
    fs::write(
        &source,
        r###"
use std::env;
use std::io::{self, Read};
use std::thread;
use std::time::Duration;

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    if input.contains("\"model\":\"timeout\"") {
        thread::sleep(Duration::from_secs(5));
    }
    if input.contains("\"model\":\"exit\"") {
        std::process::exit(7);
    }
    if input.contains("\"model\":\"malformed\"") {
        print!("{{}}");
        return;
    }
    if input.contains("\"model\":\"oversized\"") {
        print!("{{\"contract\":\"ait.agent.telegram_stt_response.v1\",\"ok\":true,\"transcript\":\"");
        print!("{}", "x".repeat(1_100_000));
        print!("\"}}");
        return;
    }
    if input.contains("\"model\":\"failed\"") {
        print!(r#"{{"contract":"ait.agent.telegram_stt_response.v1","ok":false,"error_kind":"fixture-secret"}}"#);
        return;
    }
    let wire_valid = input.contains("\"contract\":\"ait.agent.telegram_stt_request.v1\"")
        && input.contains("\"operation\":\"transcribe\"")
        && input.contains("\"python_runtime_allowed\":false")
        && input.contains("\"audio_path\":");
    let environment_cleared = env::vars().next().is_none();
    let transcript = if input.contains("\"model\":\"empty\"") {
        ""
    } else if wire_valid && environment_cleared {
        "environment-cleared native transcript"
    } else {
        "wire-or-environment-invalid"
    };
    print!(
        r#"{{"contract":"ait.agent.telegram_stt_response.v1","ok":true,"transcript":"{}","language":"en"}}"#,
        transcript
    );
}
"###,
    )
    .unwrap();
    let output = Command::new("rustc")
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&program)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture compilation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Fixture {
        _temp: temp,
        program,
        audio,
    }
}

fn request(audio: &Path, model: &str) -> JsonValue {
    json!({
        "operation": "transcribe",
        "local_path": audio,
        "model": model,
        "device": "cpu",
        "compute_type": "float32",
        "language": "en",
    })
}

#[test]
fn native_program_round_trip_clears_environment_and_returns_safe_execution_contract() {
    let fixture = fixture();
    let executor =
        ExternalProgramTelegramSttExecutor::new(fixture.program.clone(), Duration::from_secs(2))
            .unwrap();

    let result = executor
        .execute_stt(&request(&fixture.audio, "fixture-model"))
        .unwrap();

    assert_eq!(result["contract"], TELEGRAM_STT_EXECUTION_CONTRACT);
    assert_eq!(
        result["transcript"],
        "environment-cleared native transcript"
    );
    assert_eq!(result["language"], "en");
    assert_eq!(result["python_stt_allowed"], false);
    assert_eq!(result["inherited_environment_allowed"], false);
    let debug = format!("{executor:?}");
    assert!(!debug.contains(fixture.program.to_string_lossy().as_ref()));
    assert!(!debug.contains("private-audio"));
}

#[test]
fn timeout_kills_the_child_and_process_dispositions_are_typed_and_secret_safe() {
    let fixture = fixture();
    let timeout_executor =
        ExternalProgramTelegramSttExecutor::new(fixture.program.clone(), Duration::from_millis(50))
            .unwrap();
    let started = Instant::now();
    assert_eq!(
        timeout_executor
            .execute_stt(&request(&fixture.audio, "timeout"))
            .unwrap_err()
            .kind(),
        TelegramSttExecutionErrorKind::Timeout
    );
    assert!(started.elapsed() < Duration::from_secs(2));

    let executor =
        ExternalProgramTelegramSttExecutor::new(fixture.program.clone(), Duration::from_secs(2))
            .unwrap();
    for (model, expected) in [
        ("empty", TelegramSttExecutionErrorKind::Empty),
        ("failed", TelegramSttExecutionErrorKind::Transcription),
        ("malformed", TelegramSttExecutionErrorKind::Contract),
        ("oversized", TelegramSttExecutionErrorKind::OutputLimit),
        ("exit", TelegramSttExecutionErrorKind::Exit),
    ] {
        let failure = executor
            .execute_stt(&request(&fixture.audio, model))
            .unwrap_err();
        assert_eq!(failure.kind(), expected);
        let public = format!("{failure:?} {failure}");
        assert!(!public.contains("fixture-secret"));
        assert!(!public.contains("private-audio"));
    }
}

#[test]
fn forbidden_missing_and_invalid_program_or_request_paths_fail_closed() {
    for program in [
        PathBuf::from("python3"),
        PathBuf::from("/usr/bin/python3"),
        PathBuf::from("/tmp/private.py"),
        PathBuf::from("/bin/sh"),
    ] {
        assert_eq!(
            ExternalProgramTelegramSttExecutor::new(program, Duration::from_secs(1))
                .unwrap_err()
                .kind(),
            TelegramSttExecutionErrorKind::Configuration
        );
    }

    let fixture = fixture();
    let unavailable = ExternalProgramTelegramSttExecutor::new(
        fixture._temp.path().join("missing-native-stt"),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(
        unavailable
            .execute_stt(&request(&fixture.audio, "fixture-model"))
            .unwrap_err()
            .kind(),
        TelegramSttExecutionErrorKind::Unavailable
    );

    let executor =
        ExternalProgramTelegramSttExecutor::new(fixture.program.clone(), Duration::from_secs(1))
            .unwrap();
    let mut invalid = request(&fixture.audio, "fixture-model");
    invalid["private_extension"] = json!("secret");
    assert_eq!(
        executor.execute_stt(&invalid).unwrap_err().kind(),
        TelegramSttExecutionErrorKind::InvalidRequest
    );
    fs::remove_file(&fixture.audio).unwrap();
    assert_eq!(
        executor
            .execute_stt(&request(&fixture.audio, "fixture-model"))
            .unwrap_err()
            .kind(),
        TelegramSttExecutionErrorKind::InvalidRequest
    );
}
