use ait_agent_core::language_binding_info_json;
use ait_agent_core::language_bindings::CHAT_INTEGRATIONS_RETIRED_MESSAGE;
use ait_core::json_support::{JsonCodec, JsonEncodeOptions, JsonValue};
use napi::{Error, Result};
use napi_derive::napi;

pub mod agent_sdk;
pub mod artifacts;
pub mod auth;
pub mod binary_db;
pub mod changes;
pub mod constants;
pub mod external;
pub mod filesystem;
pub mod git;
pub mod governance;
pub mod identity;
pub mod inspection;
pub mod json_requests;
mod json_value;
pub mod lines;
pub mod maintenance;
pub mod recovery;
pub mod release;
pub mod review;
pub mod stash;
pub mod sync;
pub mod tags;
pub mod task_sdk;
pub mod team;
pub use json_value::JsonInput;
pub mod operations;
pub mod output;
pub mod plan_dispatch;
pub mod plan_http;
pub mod plan_sdk;
pub mod plans;
pub mod policy;
pub mod remotes;
pub mod repositories;
pub mod repository;
pub mod runtime;
pub mod snapshots;
pub mod source_cache;
pub mod storage;
pub mod workflow_sdk;
pub mod workflows;
pub mod workspace;
pub mod worktree_operations;
pub mod worktrees;

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
    fn direct_binding_sources_have_no_cli_transport_or_repository_dispatcher() {
        let sources = [
            include_str!("changes.rs"),
            include_str!("auth.rs"),
            include_str!("artifacts.rs"),
            include_str!("agent_sdk.rs"),
            include_str!("binary_db.rs"),
            include_str!("constants.rs"),
            include_str!("recovery.rs"),
            include_str!("filesystem.rs"),
            include_str!("storage.rs"),
            include_str!("../../ait-core/src/plan_storage_sdk.rs"),
            include_str!("stash.rs"),
            include_str!("tags.rs"),
            include_str!("task_sdk.rs"),
            include_str!("review.rs"),
            include_str!("release.rs"),
            include_str!("external.rs"),
            include_str!("team.rs"),
            include_str!("governance.rs"),
            include_str!("git.rs"),
            include_str!("inspection.rs"),
            include_str!("identity.rs"),
            include_str!("maintenance.rs"),
            include_str!("sync.rs"),
            include_str!("workspace.rs"),
            include_str!("worktrees.rs"),
            include_str!("worktree_operations.rs"),
            include_str!("repository.rs"),
            include_str!("repositories.rs"),
            include_str!("remotes.rs"),
            include_str!("snapshots.rs"),
            include_str!("source_cache.rs"),
            include_str!("operations.rs"),
            include_str!("output.rs"),
            include_str!("plans.rs"),
            include_str!("plan_sdk.rs"),
            include_str!("plan_dispatch.rs"),
            include_str!("plan_http.rs"),
            include_str!("../../ait-core/src/plan_http_sdk.rs"),
            include_str!("../../ait-core/src/plan_application/sdk.rs"),
            include_str!("policy.rs"),
            include_str!("runtime.rs"),
            include_str!("workflows.rs"),
            include_str!("workflow_sdk.rs"),
            include_str!("../../ait-core/src/workflow_context_sdk.rs"),
            include_str!("json_requests.rs"),
            include_str!("lines.rs"),
            include_str!("json_value.rs"),
            include_str!("../../ait-cli/src/binding_services.rs"),
            include_str!("../../ait-cli/src/binding_release.rs"),
            include_str!("../../ait-cli/src/binding_external.rs"),
            include_str!("../../ait-cli/src/binding_binary_db.rs"),
            include_str!("../../ait-cli/src/binding_team.rs"),
            include_str!("../../ait-cli/src/binding_source_cache.rs"),
            include_str!("../../ait-cli/src/binding_worktrees.rs"),
            include_str!("../../ait-cli/src/binding_changes.rs"),
            include_str!("../../ait-cli/src/binding_git.rs"),
            include_str!("../../ait-cli/src/binding_maintenance.rs"),
            include_str!("../../ait-cli/src/binding_review.rs"),
            include_str!("../../ait-cli/src/main_app/plan_services.rs"),
            include_str!("../../ait-cli/src/main_app/workflow_services.rs"),
        ];
        for source in sources {
            for forbidden in [
                "entry_with_args",
                "run_cli(",
                "runCli",
                "repo_command",
                "Command::new",
                "std::process::",
                "parse_from(",
                "try_parse_from(",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "Binding source contains forbidden transport {forbidden}"
                );
            }
        }
    }

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
}
