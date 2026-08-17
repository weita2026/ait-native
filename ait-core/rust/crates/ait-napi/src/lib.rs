use ait_agent_core::language_binding_info_json;
use ait_agent_worker::{
    agent_worker_capabilities_binding_json, agent_worker_transaction_binding_json,
};
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
    agent_worker_capabilities_binding_json()
        .map_err(Error::from_reason)
        .and_then(encode)
}

#[napi(js_name = "agentWorkerTransactionJson")]
pub fn agent_worker_transaction_json(request_json: String) -> Result<String> {
    let request = parse_request(&request_json)?;
    agent_worker_transaction_binding_json(&request)
        .map_err(Error::from_reason)
        .and_then(encode)
}

fn parse_request(text: &str) -> Result<JsonValue> {
    JsonCodec::parse_value_with_error_prefix(text, "invalid AIT N-API request JSON")
        .map_err(|error| Error::from_reason(error.to_string()))
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
    fn malformed_request_fails_at_the_addon_boundary() {
        let error = agent_worker_transaction_json("{".to_string()).expect_err("invalid request");

        assert!(error.reason.contains("invalid AIT N-API request JSON"));
    }

    #[test]
    fn embedded_cli_returns_status_without_terminating_the_host() {
        assert_eq!(run_cli(vec!["--help".to_string()]), 0);
        assert_eq!(run_cli(vec!["not-a-command".to_string()]), 2);
    }
}
