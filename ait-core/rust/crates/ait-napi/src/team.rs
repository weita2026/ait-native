use crate::{repository::repository, workflows::WorkflowOperationResult, JsonInput};
use ait_cli::binding_team as service;
use napi::{Error, Result};
use napi_derive::napi;

fn index(value: &JsonInput, field: &str) -> Result<u32> {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| Error::from_reason(format!("{field} must be an integer in the u32 range.")))
}
#[napi]
pub fn team_contacts(root: String) -> Result<WorkflowOperationResult> {
    Ok(service::team_contacts(&repository(&root)?).into())
}
#[napi]
pub fn team_session_acknowledge(
    root: String,
    activation: String,
) -> Result<WorkflowOperationResult> {
    Ok(service::acknowledge_team_session(&repository(&root)?, &activation).into())
}
#[napi]
pub fn team_session_ensure(
    root: String,
    agent: JsonInput,
    role: Option<JsonInput>,
) -> Result<WorkflowOperationResult> {
    let agent = index(&agent, "agent")?;
    let role = role
        .as_ref()
        .map(|value| index(value, "role"))
        .transpose()?;
    Ok(service::ensure_team_session(&repository(&root)?, agent, role).into())
}
#[napi]
pub fn format_team_contacts(payload: JsonInput) -> String {
    service::format_team_contacts(&payload)
}
#[napi]
pub fn format_team_session_acknowledge(payload: JsonInput) -> String {
    service::format_team_session_acknowledge(&payload)
}
#[napi]
pub fn format_team_session_ensure(payload: JsonInput, agent: JsonInput) -> Result<String> {
    Ok(service::format_team_session_ensure(
        &payload,
        index(&agent, "agent")?,
    ))
}
