use super::{
    agent_telegram_command_trigger_execute_operation_json,
    execute_with_telegram_command_trigger_operation_executor,
    DefaultTelegramCommandTriggerOperationExecutor, TelegramCommandTriggerOperationExecutor,
};
use ait_core::json_support::{json, JsonValue};

struct SubstituteCommandTriggerOperationExecutor;

impl TelegramCommandTriggerOperationExecutor for SubstituteCommandTriggerOperationExecutor {
    fn execute_operation_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        Ok(json!({
            "kind": request.get("kind").cloned().unwrap_or(JsonValue::Null),
            "ok": true,
            "substitute": true,
        }))
    }
}

#[test]
#[cfg(unix)]
fn command_trigger_execute_operation_runs_handler_in_rust() {
    let planned = agent_telegram_command_trigger_execute_operation_json(&json!({
        "kind": "run_handler",
        "handler_command": [
            "/bin/sh",
            "-c",
            r#"cat >/dev/null; printf '%s' '{"reply":{"text":"done"}}'"#
        ],
        "stdin_json": {"message": "done"},
    }))
    .unwrap();

    assert_eq!(planned["kind"], "run_handler");
    assert_eq!(planned["method"], "std::process::Command");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["returncode"], 0);
    assert!(planned["stdout"].as_str().unwrap().contains("\"done\""));
    assert_eq!(planned["stderr"], "");
}

#[test]
#[cfg(unix)]
fn command_trigger_execute_operation_reports_nonzero_exit() {
    let planned = agent_telegram_command_trigger_execute_operation_json(&json!({
        "kind": "run_handler",
        "handler_command": ["/bin/sh", "-c", "cat >/dev/null; printf 'boom\\n' >&2; exit 7"],
        "stdin_json": {},
    }))
    .unwrap();

    assert_eq!(planned["ok"], false);
    assert_eq!(planned["returncode"], 7);
    assert_eq!(planned["error"], "boom");
    assert_eq!(planned["stderr"], "boom\n");
}

#[test]
fn command_trigger_execute_operation_fails_closed_for_empty_command() {
    let planned = agent_telegram_command_trigger_execute_operation_json(&json!({
        "kind": "run_handler",
        "handler_command": [],
    }))
    .unwrap();

    assert_eq!(planned["ok"], false);
    assert_eq!(planned["returncode"], -1);
    assert_eq!(
        planned["error"],
        "Operational trigger handler command is empty."
    );
}

#[test]
#[cfg(unix)]
fn command_trigger_execute_operation_applies_nested_operation_env_contract() {
    let planned = agent_telegram_command_trigger_execute_operation_json(&json!({
        "operation": {
            "kind": "run_handler",
            "repo_root": "/tmp/ait-repo",
            "env_overrides": {
                "EXTRA_FLAG": true,
                "RETRY_COUNT": 3,
            },
            "handler_command": [
                "/bin/sh",
                "-c",
                r#"cat >/dev/null; printf '{"repo_root":"%s","extra_flag":"%s","retry_count":"%s","pythonpath":"%s"}' "$AIT_REPO_ROOT" "$EXTRA_FLAG" "$RETRY_COUNT" "$PYTHONPATH""#
            ],
            "stdin_json": {"message": "done"},
        }
    }))
    .unwrap();

    assert_eq!(planned["kind"], "run_handler");
    assert_eq!(planned["ok"], true);
    let stdout: ait_core::json_support::JsonValue = crate::json_support::parse_value(
        planned["stdout"].as_str().unwrap(),
        "Invalid handler stdout JSON",
    )
    .expect("handler stdout must be JSON");
    assert_eq!(stdout["repo_root"], "/tmp/ait-repo");
    assert_eq!(stdout["extra_flag"], "True");
    assert_eq!(stdout["retry_count"], "3");
    assert!(stdout["pythonpath"]
        .as_str()
        .unwrap()
        .starts_with("/tmp/ait-repo/src"));
}

#[test]
fn command_trigger_execute_operation_error_contract_is_stable() {
    let unsupported = agent_telegram_command_trigger_execute_operation_json(&json!({
        "kind": "unknown",
    }))
    .unwrap();
    assert_eq!(unsupported["kind"], "unknown");
    assert_eq!(unsupported["ok"], false);
    assert_eq!(unsupported["returncode"], -1);
    assert_eq!(
        unsupported["error"],
        "Unsupported Telegram command trigger operation: unknown."
    );

    let missing = agent_telegram_command_trigger_execute_operation_json(&json!({})).unwrap();
    assert_eq!(missing["kind"], "");
    assert_eq!(missing["ok"], false);
    assert_eq!(
        missing["error"],
        "Unsupported Telegram command trigger operation: <missing>."
    );

    let invalid = agent_telegram_command_trigger_execute_operation_json(&json!("bad"));
    assert_eq!(
        invalid.unwrap_err(),
        "Telegram command trigger operation request must be a JSON object."
    );
}

#[test]
fn command_trigger_default_executor_satisfies_trait_entrypoint() {
    let executor: &dyn TelegramCommandTriggerOperationExecutor =
        &DefaultTelegramCommandTriggerOperationExecutor;
    let planned = executor
        .execute_operation_json(&json!({
            "kind": "unknown",
        }))
        .unwrap();

    assert_eq!(planned["kind"], "unknown");
    assert_eq!(planned["ok"], false);
    assert_eq!(
        planned["error"],
        "Unsupported Telegram command trigger operation: unknown."
    );
}

#[test]
fn command_trigger_bound_entrypoint_accepts_substitute_executor() {
    let executor = SubstituteCommandTriggerOperationExecutor;
    let planned = execute_with_telegram_command_trigger_operation_executor(
        &executor,
        &json!({"kind": "run_handler"}),
    )
    .unwrap();
    assert_eq!(planned["kind"], "run_handler");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["substitute"], true);
}
