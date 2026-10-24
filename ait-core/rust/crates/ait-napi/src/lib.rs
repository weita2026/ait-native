use ait_agent_core::language_binding_info_json;
use ait_agent_core::language_bindings::CHAT_INTEGRATIONS_RETIRED_MESSAGE;
use ait_core::json_support::{JsonCodec, JsonEncodeOptions, JsonValue};
use napi::{Error, Result};
use napi_derive::napi;
use std::ffi::OsString;

#[napi(js_name = "runCli")]
pub fn run_cli(args: Vec<String>) -> u32 {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(OsString::from("ait"));
    argv.extend(args.into_iter().map(OsString::from));
    u32::from(ait_cli::entry_with_args(argv))
}

#[napi(js_name = "bindingInfoJson")]
pub fn binding_info_json() -> Result<String> {
    encode(language_binding_info_json())
}

#[napi(js_name = "agentWorkerCapabilitiesJson")]
pub fn agent_worker_capabilities_json() -> Result<String> {
    Err(Error::from_reason(CHAT_INTEGRATIONS_RETIRED_MESSAGE))
}

#[napi(js_name = "agentManagementJson")]
pub fn agent_management_json(request_json: String) -> Result<String> {
    let _ = request_json;
    Err(Error::from_reason(CHAT_INTEGRATIONS_RETIRED_MESSAGE))
}

#[napi(js_name = "agentWorkerTransactionJson")]
pub fn agent_worker_transaction_json(request_json: String) -> Result<String> {
    let _ = request_json;
    Err(Error::from_reason(CHAT_INTEGRATIONS_RETIRED_MESSAGE))
}

fn encode(value: JsonValue) -> Result<String> {
    JsonCodec::encode_value(&value, JsonEncodeOptions::compact())
        .map_err(|error| Error::from_reason(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_info_is_direct_and_versioned() {
        let payload = binding_info_json().expect("binding info");

        assert!(payload.contains("\"contract\":\"ait.language.binding.v1\""));
        assert!(payload.contains("\"node_binding\":\"napi\""));
        assert!(payload.contains("\"process_transport_allowed\":false"));
        assert!(payload.contains(&format!("\"version\":\"{}\"", env!("CARGO_PKG_VERSION"))));
    }

    #[test]
    fn chat_exports_fail_before_request_parsing_or_runtime_execution() {
        assert_eq!(
            agent_worker_capabilities_json()
                .expect_err("retired capabilities")
                .reason,
            CHAT_INTEGRATIONS_RETIRED_MESSAGE
        );
        for request in [
            "{",
            r#"{"operation":"start","name":"retired","repo_root":"/unavailable/retired-chat","manifest_path":"/unavailable/retired-chat/workers.json","worker_binary":"/unavailable/retired-chat/worker"}"#,
        ] {
            for result in [
                agent_management_json(request.to_string()),
                agent_worker_transaction_json(request.to_string()),
            ] {
                assert_eq!(
                    result.expect_err("retired chat call").reason,
                    CHAT_INTEGRATIONS_RETIRED_MESSAGE
                );
            }
        }
    }

    #[test]
    fn embedded_cli_returns_status_without_terminating_the_host() {
        assert_eq!(run_cli(vec!["--help".to_string()]), 0);
        assert_eq!(run_cli(vec!["not-a-command".to_string()]), 2);
    }
}
