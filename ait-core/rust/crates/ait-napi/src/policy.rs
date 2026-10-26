//! Retained policy and provenance SDK functions, backed by the same Rust APIs as PyO3.
use crate::JsonInput;
use ait_core::{json_support::JsonValue, policy};
use napi::{Error, Result};
use napi_derive::napi;

#[napi]
pub fn policy_profile_names() -> Vec<String> {
    policy::policy_profile_names()
}
#[napi]
pub fn author_mode_values() -> Vec<String> {
    policy::author_mode_values()
}
#[napi]
pub fn policy_content_class_values() -> Vec<String> {
    policy::policy_content_class_values()
}
#[napi]
pub fn policy_author_class_values() -> Vec<String> {
    policy::policy_author_class_values()
}
#[napi]
pub fn normalize_author_mode(value: String) -> Result<String> {
    policy::normalize_author_mode(&value).map_err(Error::from_reason)
}

#[napi]
pub fn missing_code_review_summary_sections(value: Option<String>) -> Vec<String> {
    policy::missing_code_review_summary_sections(value.as_deref())
}
#[napi]
pub fn is_structured_code_review_summary(value: Option<String>) -> bool {
    policy::is_structured_code_review_summary(value.as_deref())
}
#[napi]
pub fn render_code_review_summary_template(style: Option<String>) -> Result<String> {
    policy::render_code_review_summary_template(Some(style.as_deref().unwrap_or("inline")))
        .map(str::to_string)
        .map_err(Error::from_reason)
}
#[napi]
pub fn code_review_summary_requirement_text(value: Option<String>) -> String {
    policy::code_review_summary_requirement_text(value.as_deref())
}
#[napi]
pub fn derive_policy_content_class(changed_paths: Option<JsonInput>) -> String {
    let null = JsonValue::Null;
    policy::derive_policy_content_class(Some(changed_paths.as_deref().unwrap_or(&null)))
}
#[napi]
pub fn derive_policy_author_class(author_mode: Option<String>) -> JsonValue {
    policy::derive_policy_author_class(author_mode.as_deref())
        .map(JsonValue::String)
        .unwrap_or(JsonValue::Null)
}
#[napi]
pub fn build_minimum_provenance(
    author_mode: String,
    model_name: Option<String>,
) -> Result<JsonValue> {
    let (summary, detail) = policy::build_minimum_provenance(&author_mode, model_name.as_deref())
        .map_err(Error::from_reason)?;
    Ok(JsonValue::Array(vec![summary, detail]))
}
#[napi]
pub fn policy_profile(name: String) -> Result<JsonValue> {
    policy::policy_profile(&name).map_err(Error::from_reason)
}

#[napi]
pub fn normalize_policy(
    policy_value: Option<JsonInput>,
    fallback_profile: Option<String>,
) -> Result<JsonValue> {
    policy::normalize_policy(
        policy_value.as_deref(),
        fallback_profile.as_deref().unwrap_or("prototype"),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct EffectivePolicyOptions {
    pub content_class: Option<String>,
    pub author_class: Option<String>,
    pub fallback_profile: Option<String>,
}
#[napi]
pub fn resolve_effective_policy(
    policy_value: Option<JsonInput>,
    options: Option<EffectivePolicyOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    policy::resolve_effective_policy(
        policy_value.as_deref(),
        options.content_class.as_deref(),
        options.author_class.as_deref(),
        options.fallback_profile.as_deref().unwrap_or("prototype"),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn policy_to_yaml(
    policy_value: Option<JsonInput>,
    fallback_profile: Option<String>,
) -> Result<String> {
    policy::policy_to_yaml(
        policy_value.as_deref(),
        fallback_profile.as_deref().unwrap_or("prototype"),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn parse_policy_yaml(text: String, fallback_profile: Option<String>) -> Result<JsonValue> {
    policy::parse_policy_yaml(&text, fallback_profile.as_deref().unwrap_or("prototype"))
        .map_err(Error::from_reason)
}
