//! Team operations and exact native presentation, without a CLI dispatcher.
use crate::{agent_teams, binding_services::WorkflowOperationResult, runtime::RepoRuntime};
use ait_core::json_support::JsonValue;

fn result(payload: JsonValue, expected_status: &str) -> WorkflowOperationResult {
    let exit_code = if payload["status"].as_str() == Some(expected_status) {
        0
    } else {
        1
    };
    WorkflowOperationResult { payload, exit_code }
}
pub fn team_contacts(repo: &RepoRuntime) -> WorkflowOperationResult {
    result(agent_teams::contacts(repo), "ok")
}
pub fn acknowledge_team_session(repo: &RepoRuntime, activation: &str) -> WorkflowOperationResult {
    result(agent_teams::acknowledge(repo, activation), "ready")
}
pub fn ensure_team_session(
    repo: &RepoRuntime,
    agent: u32,
    role: Option<u32>,
) -> WorkflowOperationResult {
    result(agent_teams::ensure(repo, agent, role), "ready")
}
pub fn format_team_contacts(payload: &JsonValue) -> String {
    agent_teams::render_contacts(payload)
}
pub fn format_team_session_acknowledge(payload: &JsonValue) -> String {
    format!(
        "Team activation: {}\nSession ID: {}\n{}",
        payload["status"],
        payload["session_id"].as_str().unwrap_or("unbound"),
        payload["detail"].as_str().unwrap_or_default()
    )
}
pub fn format_team_session_ensure(payload: &JsonValue, agent: u32) -> String {
    format!(
        "Team session: {}\nAgent: {}\nSession ID: {}\nAction: {}\n{}",
        payload["status"],
        agent,
        payload["session_id"].as_str().unwrap_or("unbound"),
        payload["action"].as_str().unwrap_or("none"),
        payload["detail"].as_str().unwrap_or_default()
    )
}
