use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;

use tempfile::{tempdir, TempDir};

use super::*;

const PUBLIC_KEY: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
const SIGNATURE_TIMESTAMP: &str = "1714990000";
const RAW_INTERACTION: &str = r#"{"id":"112233445566778899","type":2,"token":"discord-token-1","application_id":"123456789012345678","channel_id":"998877665544332211","guild_id":"556677889900112233","data":{"id":"887766554433221100","name":"ask","type":1,"options":[{"name":"text","type":3,"value":"Hello from Discord"}]},"member":{"user":{"id":"U-discord-1","username":"weita","global_name":"WeiTa"}}}"#;
const VALID_SIGNATURE: &str = "cdd61b985c0507f54f261a6fb4415dd5db603c78387c7176ea311202c61578f67cbdf9e596dfc0c039c7e80f2ced117804740076680db3075ffd55818605ce00";

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

impl DiscordInteractionOnceJobExecutor for StubJob {
    fn execute_interaction_job(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.borrow_mut().push(request.clone());
        self.result.clone()
    }
}

fn fixture(remote: bool, public_key: Option<&str>) -> TempDir {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
    let config = if remote {
        json!({
            "repo_name": "discord-fixture",
            "workflow_mode": "solo_remote",
            "default_remote": "origin",
            "remotes": {"origin": {"url": "http://127.0.0.1:8088"}},
        })
    } else {
        json!({
            "repo_name": "discord-fixture",
            "workflow_mode": "solo_local",
        })
    };
    fs::write(temp.path().join(".ait/config.json"), config.to_string()).expect("repo config");
    let mut worker = json!({
        "kind": "discord",
        "name": "main",
        "application_id": "123456789012345678",
    });
    if let Some(public_key) = public_key {
        worker["public_key"] = JsonValue::String(public_key.to_string());
    }
    fs::write(
        temp.path().join(".ait/agent-workers.json"),
        json!({
            "version": 1,
            "workers": {"discord/main": worker},
        })
        .to_string(),
    )
    .expect("worker manifest");
    temp
}

fn signed_request(temp: &TempDir) -> DiscordInteractionOnceRequest {
    DiscordInteractionOnceRequest {
        path_inputs: WorkerPathInputs {
            current_dir: temp.path().to_path_buf(),
            repo_root_override: None,
            manifest_path_override: None,
        },
        worker_name: "main".to_string(),
        process_env: BTreeMap::new(),
        raw_payload: RAW_INTERACTION.to_string(),
        signature: Some(VALID_SIGNATURE.to_string()),
        signature_timestamp: Some(SIGNATURE_TIMESTAMP.to_string()),
    }
}

fn processed_job() -> JsonValue {
    json!({
        "contract": "ait_agent_core.event_loop.DiscordInteractionJob.v1",
        "migration_stage": "rust_agent_discord_interaction_job_transaction",
        "interaction_job_state": "processed",
        "ok": true,
        "processed": true,
        "duplicate": false,
        "conversation_key": "discord:998877665544332211",
        "binding_created": true,
        "turn_ok": true,
        "recorded": true,
        "sequence": 8,
        "response": {"type": 4, "data": {"content": "Rust Discord reply"}},
        "delivery_request": {
            "reply_mode": "interaction",
            "operations": [{
                "kind": "send_followup_attachment",
                "interaction_token": "discord-token-1",
                "attachment": {
                    "local_path": "artifacts/private-report.md",
                    "file_name": "report.md",
                },
            }],
        },
        "recovery_request": {
            "conversation_key": "discord:998877665544332211",
            "delivery_request": null,
            "pending_reply": {"event_id": "112233445566778899"},
        },
        "error_kind": null,
        "error": "backend internals must be dropped",
        "debug": "port internals must be dropped",
    })
}

#[test]
fn signed_interaction_dispatches_typed_job_and_returns_minimal_redacted_output() {
    let temp = fixture(true, Some(PUBLIC_KEY));
    let executor = StubJob::returning(processed_job());

    let output =
        execute_discord_interaction_once_with_job_executor(&executor, &signed_request(&temp))
            .expect("interaction once");

    assert_eq!(output["contract"], DISCORD_INTERACTION_ONCE_CONTRACT);
    assert_eq!(output["ok"], true);
    assert_eq!(output["response"]["type"], 4);
    assert_eq!(output["response"]["data"]["content"], "Rust Discord reply");
    assert_eq!(
        output["interaction_job"]["interaction_job_state"],
        "processed"
    );
    assert_eq!(
        output["interaction_job"]["conversation_key"],
        "discord:998877665544332211"
    );
    assert!(output["interaction_job"].get("session_id").is_none());
    assert!(output["interaction_job"].get("error").is_none());
    assert!(output["interaction_job"].get("debug").is_none());
    assert!(output["interaction_job"].get("delivery_request").is_none());
    assert!(output["interaction_job"].get("recovery_request").is_none());
    assert_eq!(output["python_worker_execution_allowed"], false);

    let requests = executor.requests.borrow();
    assert_eq!(requests.len(), 1);
    let job = &requests[0];
    assert_eq!(job["runtime_target"]["mode"], "remote");
    assert_eq!(job["runtime_target"]["repo_name"], "discord-fixture");
    assert_eq!(
        job["interaction_payload"]["channel_id"],
        "998877665544332211"
    );
    assert!(job["state_path"]
        .as_str()
        .is_some_and(|path| path.ends_with(".ait/agent-runtime/discord-main-sync.json")));
    assert!(!job.to_string().contains(PUBLIC_KEY));
    assert!(!job.to_string().contains(VALID_SIGNATURE));

    let public = output.to_string();
    for forbidden in [
        PUBLIC_KEY,
        VALID_SIGNATURE,
        "discord-token-1",
        "artifacts/private-report.md",
        RAW_INTERACTION,
        "backend internals",
        "port internals",
    ] {
        assert!(
            !public.contains(forbidden),
            "public result leaked {forbidden}"
        );
    }
}

#[test]
fn duplicate_job_response_is_preserved_without_private_job_fields() {
    let temp = fixture(true, Some(PUBLIC_KEY));
    let mut duplicate = processed_job();
    duplicate["interaction_job_state"] = json!("duplicate_ignored");
    duplicate["processed"] = json!(false);
    duplicate["duplicate"] = json!(true);
    duplicate["turn_ok"] = JsonValue::Null;
    duplicate["sequence"] = JsonValue::Null;
    duplicate["response"] =
        json!({"type": 4, "data": {"content": "Duplicate Discord interaction ignored."}});
    let executor = StubJob::returning(duplicate);

    let output =
        execute_discord_interaction_once_with_job_executor(&executor, &signed_request(&temp))
            .expect("duplicate");

    assert_eq!(output["interaction_job"]["duplicate"], true);
    assert_eq!(
        output["response"]["data"]["content"],
        "Duplicate Discord interaction ignored."
    );
}

#[test]
fn invalid_signature_and_missing_public_key_fail_before_job() {
    let temp = fixture(true, Some(PUBLIC_KEY));
    let executor = StubJob::returning(processed_job());
    let mut invalid = signed_request(&temp);
    invalid.signature = Some("invalid".to_string());

    let error = execute_discord_interaction_once_with_job_executor(&executor, &invalid)
        .expect_err("invalid signature");
    assert_eq!(error.code, "discord_interaction_signature_invalid");
    assert_eq!(error.exit_code, EXIT_INVALID_REQUEST);
    assert!(executor.requests.borrow().is_empty());
    assert!(!error.render_json().contains(PUBLIC_KEY));
    assert!(!error.render_json().contains("discord-token-1"));

    let missing = fixture(true, None);
    let error =
        execute_discord_interaction_once_with_job_executor(&executor, &signed_request(&missing))
            .expect_err("missing key");
    assert_eq!(error.code, "discord_public_key_missing");
    assert!(executor.requests.borrow().is_empty());
}

#[test]
fn local_mode_dispatches_repo_root_to_the_rust_interaction_job() {
    let temp = fixture(false, Some(PUBLIC_KEY));
    let executor = StubJob::returning(processed_job());

    let output =
        execute_discord_interaction_once_with_job_executor(&executor, &signed_request(&temp))
            .expect("local interaction result");

    assert_eq!(output["ok"], true);
    assert_eq!(
        output["interaction_job"]["interaction_job_state"],
        "processed"
    );
    assert_eq!(output["response"]["data"]["content"], "Rust Discord reply");
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
        .join(".ait/agent-runtime/discord-main-sync.json")
        .exists());
}

#[test]
fn executor_errors_and_invalid_contracts_are_stable_and_secret_safe() {
    let temp = fixture(true, Some(PUBLIC_KEY));
    let failing = StubJob::failing("backend leaked discord-token-1 and public key material");
    let error =
        execute_discord_interaction_once_with_job_executor(&failing, &signed_request(&temp))
            .expect_err("executor failure");
    assert_eq!(error.code, "discord_interaction_job_failed");
    assert!(!error.render_json().contains("discord-token-1"));

    let invalid = StubJob::returning(json!({
        "ok": true,
        "backend_debug": "must not escape",
    }));
    let error =
        execute_discord_interaction_once_with_job_executor(&invalid, &signed_request(&temp))
            .expect_err("invalid contract");
    assert_eq!(error.code, "discord_interaction_job_contract_invalid");
    assert!(!error.render_json().contains("must not escape"));
}
