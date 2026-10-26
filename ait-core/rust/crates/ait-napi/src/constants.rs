//! Retained public SDK values; arrays use direct value functions.
use ait_core::{policy, runtime_roots, workflow_primitives as workflow};
use napi_derive::napi;

#[napi]
pub const CODE_REVIEW_SUMMARY_TEMPLATE: &str = policy::CODE_REVIEW_SUMMARY_TEMPLATE;
#[napi]
pub const CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE: &str =
    policy::CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE;
#[napi]
pub const CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND: &str =
    policy::CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND;
#[napi]
pub const DEFAULT_ID_NAMESPACE_PREFIX: &str = workflow::DEFAULT_ID_NAMESPACE_PREFIX;
#[napi]
pub const LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX: &str = workflow::LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX;
#[napi]
pub const REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX: &str = workflow::REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX;
#[napi]
pub const RUNTIME_DATA_ENV: &str = runtime_roots::RUNTIME_DATA_ENV;
#[napi]
pub const LEGACY_SERVER_DATA_ENV: &str = runtime_roots::LEGACY_SERVER_DATA_ENV;
#[napi]
pub const CONTRACT_VERSION: &str = "task-close-v1";
#[napi]
pub const PLAN_CONTRACT_VERSION: &str = "plan-foundation-v7";
#[napi]
pub const TARGET_TRIPLE: &str = match option_env!("TARGET") {
    Some(target) => target,
    None => "unknown-target",
};
#[napi]
pub fn workflow_id_families() -> Vec<String> {
    workflow::WORKFLOW_ID_FAMILIES
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}
#[napi]
pub fn workflow_task_change_origin_namespace_prefixes() -> Vec<String> {
    workflow::WORKFLOW_TASK_CHANGE_ORIGIN_NAMESPACE_PREFIXES
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}
#[napi]
pub fn policy_requirement_flags() -> Vec<String> {
    policy::POLICY_REQUIREMENT_FLAGS
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}
