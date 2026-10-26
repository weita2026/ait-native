//! Retained runtime-root and ref-name SDK helpers.
use ait_core::{json_support::JsonValue, ref_names, runtime_roots};
use napi::{Error, Result};
use napi_derive::napi;

#[napi]
pub fn runtime_data_env_value() -> JsonValue {
    runtime_roots::runtime_data_env_value()
        .map(JsonValue::String)
        .unwrap_or(JsonValue::Null)
}
#[napi]
pub fn runtime_data_env_name() -> JsonValue {
    runtime_roots::runtime_data_env_name()
        .map(|value| JsonValue::String(value.to_string()))
        .unwrap_or(JsonValue::Null)
}
#[napi]
pub fn resolve_runtime_data_root_with_source(path: Option<String>) -> Result<Vec<String>> {
    let (root, source) = runtime_roots::resolve_runtime_data_root_with_source(path.as_deref())
        .map_err(Error::from_reason)?;
    Ok(vec![root.to_string_lossy().to_string(), source.to_string()])
}
#[napi]
pub fn resolve_runtime_data_root(path: Option<String>) -> Result<String> {
    runtime_roots::resolve_runtime_data_root(path.as_deref())
        .map(|root| root.to_string_lossy().to_string())
        .map_err(Error::from_reason)
}
#[napi]
pub fn resolve_server_runtime_root_with_source(path: Option<String>) -> Result<Vec<String>> {
    let (root, source) = runtime_roots::resolve_server_runtime_root_with_source(path.as_deref())
        .map_err(Error::from_reason)?;
    Ok(vec![root.to_string_lossy().to_string(), source.to_string()])
}
#[napi]
pub fn resolve_server_runtime_root(path: Option<String>) -> Result<String> {
    runtime_roots::resolve_server_runtime_root(path.as_deref())
        .map(|root| root.to_string_lossy().to_string())
        .map_err(Error::from_reason)
}
#[napi]
pub fn encode_ref_name(name: String) -> String {
    ref_names::encode_ref_name(&name)
}
#[napi]
pub fn decode_ref_name(name: String) -> String {
    ref_names::decode_ref_name(&name)
}
