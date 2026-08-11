use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;

use hmac::{Hmac, Mac};
use sha2::Sha256;
use tempfile::{tempdir, TempDir};

use super::*;

const SIGNING_SECRET: &str = "test-signing-secret";
const SIGNATURE_TIMESTAMP: &str = "1714990000";
const RAW_COMMAND: &str = "team_id=T1&channel_id=C1&channel_name=ops&user_id=U1&user_name=alice&command=%2Fait&text=hello+world&response_url=https%3A%2F%2Fhooks.slack.test%2Fsecret-response&trigger_id=trig-1";

type HmacSha256 = Hmac<Sha256>;

struct StubJob {
    requests: RefCell<Vec<JsonValue>>,
    result: Result<JsonValue, String>,
}

impl StubJob {
    fn returning(result: JsonValue) -> Self {
        Self {
            requests: RefCell::new(Vec::new()),
            result: Ok(result),
        }
    }

    fn failing(error: &str) -> Self {
        Self {
            requests: RefCell::new(Vec::new()),
            result: Err(error.to_string()),
        }
    }
}

impl SlackCommandOnceJobExecutor for StubJob {
    fn execute_command_job(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.borrow_mut().push(request.clone());
        self.result.clone()
    }
}

fn fixture(remote: bool, signing_secret: Option<&str>) -> TempDir {
    let temp = tempdir().expect("tempdir");
    fs::create_dir(temp.path().join(".ait")).expect("ait dir");
    let config = if remote {
        json!({
            "repo_name": "slack-fixture",
            "workflow_mode": "solo_remote",
            "default_remote": "origin",
            "remotes": {"origin": {"url": "http://127.0.0.1:8088"}},
        })
    } else {
        json!({
            "repo_name": "slack-fixture",
            "workflow_mode": "solo_local",
        })
    };
    fs::write(temp.path().join(".ait/config.json"), config.to_string()).expect("repo config");
    let mut worker = json!({
        "kind": "slack",
        "name": "main",
        "ack_text": "queued by rust",
        "response_type": "in_channel",
    });
    if let Some(signing_secret) = signing_secret {
        worker["signing_secret"] = JsonValue::String(signing_secret.to_string());
    }
    fs::write(
        temp.path().join(".ait/agent-workers.json"),
        json!({
            "version": 1,
            "workers": {"slack/main": worker},
        })
        .to_string(),
    )
    .expect("worker manifest");
    fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
    fs::write(
        temp.path().join(".ait/agent-runtime/slack.env"),
        "AIT_SLACK_ACK_TEXT=queued by rust\nAIT_SLACK_RESPONSE_TYPE=in_channel\n",
    )
    .expect("Slack env");
    temp
}

fn signed_request(temp: &TempDir, raw_payload: &str) -> SlackCommandOnceRequest {
    SlackCommandOnceRequest {
        path_inputs: WorkerPathInputs {
            current_dir: temp.path().to_path_buf(),
            repo_root_override: None,
            manifest_path_override: None,
        },
        worker_name: "main".to_string(),
        process_env: BTreeMap::new(),
        raw_payload: raw_payload.to_string(),
        signature: Some(signature(raw_payload)),
        signature_timestamp: Some(SIGNATURE_TIMESTAMP.to_string()),
        now_unix_seconds: Some(1_714_990_000),
    }
}

fn signature(raw_payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(SIGNING_SECRET.as_bytes()).expect("HMAC");
    mac.update(format!("v0:{SIGNATURE_TIMESTAMP}:{raw_payload}").as_bytes());
    let digest = mac.finalize().into_bytes();
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("v0={hex}")
}

fn processed_job() -> JsonValue {
    json!({
        "contract": "ait_agent_core.event_loop.SlackCommandJob.v1",
        "migration_stage": "rust_agent_slack_command_job_transaction",
        "command_job_state": "processed",
        "ok": true,
        "processed": true,
        "duplicate": false,
        "conversation_key": "slack:C1:root",
        "binding_created": false,
        "turn_ok": true,
        "delivery_attempted": true,
        "delivered": true,
        "recorded": true,
        "sequence": 7,
        "error_kind": null,
        "error": "backend detail must be dropped",
        "debug": "port internals must be dropped",
    })
}

#[test]
fn signed_command_dispatches_typed_rust_job_and_returns_minimal_redacted_output() {
    let temp = fixture(true, Some(SIGNING_SECRET));
    let executor = StubJob::returning(processed_job());

    let output = execute_slack_command_once_with_job_executor(
        &executor,
        &signed_request(&temp, RAW_COMMAND),
    )
    .expect("command once");

    assert_eq!(output["contract"], SLACK_COMMAND_ONCE_CONTRACT);
    assert_eq!(output["ok"], true);
    assert_eq!(output["command_job_dispatched"], true);
    assert_eq!(output["response"]["text"], "queued by rust");
    assert_eq!(output["command_job"]["command_job_state"], "processed");
    assert_eq!(output["command_job"]["sequence"], 7);
    assert_eq!(output["command_job"]["conversation_key"], "slack:C1:root");
    assert!(output["command_job"].get("session_id").is_none());
    assert!(output["command_job"].get("error").is_none());
    assert!(output["command_job"].get("debug").is_none());
    assert_eq!(output["python_worker_execution_allowed"], false);

    let requests = executor.requests.borrow();
    assert_eq!(requests.len(), 1);
    let job = &requests[0];
    assert_eq!(job["runtime_target"]["mode"], "remote");
    assert_eq!(job["runtime_target"]["workflow_mode"], "solo_remote");
    assert_eq!(job["runtime_target"]["repo_name"], "slack-fixture");
    assert_eq!(job["runtime_target"]["server_url"], "http://127.0.0.1:8088");
    assert_eq!(job["command_payload"]["channel_id"], "C1");
    assert_eq!(job["command_payload"]["text"], "hello world");
    assert!(job["state_path"]
        .as_str()
        .is_some_and(|path| path.ends_with(".ait/agent-runtime/slack-main-sync.json")));
    assert!(!job.to_string().contains(SIGNING_SECRET));

    let public = output.to_string();
    for forbidden in [
        SIGNING_SECRET,
        "hooks.slack.test",
        RAW_COMMAND,
        "backend detail",
        "port internals",
    ] {
        assert!(
            !public.contains(forbidden),
            "public result leaked {forbidden}"
        );
    }
}

#[test]
fn persistent_duplicate_replaces_ack_with_ephemeral_duplicate_response() {
    let temp = fixture(true, Some(SIGNING_SECRET));
    let mut duplicate = processed_job();
    duplicate["command_job_state"] = json!("duplicate");
    duplicate["ok"] = json!(true);
    duplicate["processed"] = json!(false);
    duplicate["duplicate"] = json!(true);
    let executor = StubJob::returning(duplicate);

    let output = execute_slack_command_once_with_job_executor(
        &executor,
        &signed_request(&temp, RAW_COMMAND),
    )
    .expect("duplicate");

    assert_eq!(output["response"]["response_type"], "ephemeral");
    assert_eq!(
        output["response"]["text"],
        "Duplicate Slack command ignored."
    );
    assert_eq!(output["command_job"]["duplicate"], true);
}

#[test]
fn ingress_ignored_command_returns_without_job_side_effects() {
    let temp = fixture(true, Some(SIGNING_SECRET));
    let raw_payload = "team_id=T1&channel_id=C1&user_id=U1&command=%2Fait&text=+++&response_url=https%3A%2F%2Fhooks.slack.test%2Fsecret-response&trigger_id=trig-empty";
    let executor = StubJob::returning(processed_job());

    let output = execute_slack_command_once_with_job_executor(
        &executor,
        &signed_request(&temp, raw_payload),
    )
    .expect("ignored command");

    assert_eq!(output["command_job_dispatched"], false);
    assert_eq!(output["command_job"], JsonValue::Null);
    assert_eq!(output["response"]["response_type"], "ephemeral");
    assert_eq!(
        output["response"]["text"],
        "Slack command must include text content."
    );
    assert!(executor.requests.borrow().is_empty());
}

#[test]
fn invalid_signature_and_missing_secret_fail_before_command_job() {
    let temp = fixture(true, Some(SIGNING_SECRET));
    let executor = StubJob::returning(processed_job());
    let mut invalid = signed_request(&temp, RAW_COMMAND);
    invalid.signature = Some("v0=invalid".to_string());

    let error = execute_slack_command_once_with_job_executor(&executor, &invalid)
        .expect_err("invalid signature");

    assert_eq!(error.code, "slack_command_signature_invalid");
    assert_eq!(error.exit_code, EXIT_INVALID_REQUEST);
    assert!(executor.requests.borrow().is_empty());
    assert!(!error.render_json().contains(SIGNING_SECRET));
    assert!(!error.render_json().contains("hooks.slack.test"));

    let missing = fixture(true, None);
    let error = execute_slack_command_once_with_job_executor(
        &executor,
        &signed_request(&missing, RAW_COMMAND),
    )
    .expect_err("missing secret");
    assert_eq!(error.code, "slack_signing_secret_missing");
    assert!(executor.requests.borrow().is_empty());
}

#[test]
fn local_mode_dispatches_repo_root_to_the_rust_command_job() {
    let temp = fixture(false, Some(SIGNING_SECRET));
    let executor = StubJob::returning(processed_job());

    let output = execute_slack_command_once_with_job_executor(
        &executor,
        &signed_request(&temp, RAW_COMMAND),
    )
    .expect("local command result");

    assert_eq!(output["ok"], true);
    assert_eq!(output["command_job"]["command_job_state"], "processed");
    assert_eq!(output["response"]["text"], "queued by rust");
    let requests = executor.requests.borrow();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["runtime_target"]["mode"], "local");
    assert_eq!(requests[0]["runtime_target"]["workflow_mode"], "solo_local");
    assert_eq!(
        requests[0]["runtime_target"]["repo_root"],
        temp.path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert!(!temp
        .path()
        .join(".ait/agent-runtime/slack-main-sync.json")
        .exists());
}

#[test]
fn executor_errors_and_invalid_contracts_are_stable_and_secret_safe() {
    let temp = fixture(true, Some(SIGNING_SECRET));
    let failing = StubJob::failing(
        "backend leaked test-signing-secret and https://hooks.slack.test/secret-response",
    );

    let error =
        execute_slack_command_once_with_job_executor(&failing, &signed_request(&temp, RAW_COMMAND))
            .expect_err("executor failure");

    assert_eq!(error.code, "slack_command_job_failed");
    assert!(!error.render_json().contains(SIGNING_SECRET));
    assert!(!error.render_json().contains("hooks.slack.test"));

    let invalid = StubJob::returning(json!({
        "ok": true,
        "duplicate": false,
        "backend_debug": "must not escape",
    }));
    let error =
        execute_slack_command_once_with_job_executor(&invalid, &signed_request(&temp, RAW_COMMAND))
            .expect_err("invalid contract");
    assert_eq!(error.code, "slack_command_job_contract_invalid");
    assert!(!error.render_json().contains("must not escape"));
}
