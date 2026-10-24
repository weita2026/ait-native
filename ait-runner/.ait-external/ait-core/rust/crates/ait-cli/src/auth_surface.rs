use crate::runtime::{RemoteRow, RepoRuntime};
use ait_core::json_support::{json, JsonValue};
use ait_core::plan_http_client::{
    auth_whoami as http_auth_whoami, grant_role_bindings as http_grant_role_bindings,
    list_role_bindings as http_list_role_bindings, PlanHttpClientConfig,
};

#[derive(Clone, Debug, Default)]
pub struct AuthRemoteRequest {
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct AuthGrantRequest {
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
    pub actor_identity: String,
    pub roles: Vec<String>,
}

pub fn auth_whoami(repo: &RepoRuntime, request: &AuthRemoteRequest) -> Result<JsonValue, String> {
    match remote_context(
        repo,
        request.remote_name.as_deref(),
        request.repo_name.as_deref(),
    )
    .and_then(|(remote, repo_name)| {
        http_auth_whoami(http_config(repo, &remote), Some(repo_name.as_str()))
            .map_err(|err| err.to_string())
    }) {
        Ok(payload) => Ok(payload),
        Err(_) => Ok(local_auth_snapshot(repo, request.repo_name.as_deref())),
    }
}

pub fn auth_grant(repo: &RepoRuntime, request: &AuthGrantRequest) -> Result<JsonValue, String> {
    let actor_identity = normalize_required_text(&request.actor_identity, "actor identity")?;
    let roles = request
        .roles
        .iter()
        .filter_map(|role| normalize_optional_text(Some(role.as_str())))
        .collect::<Vec<_>>();
    if roles.is_empty() {
        return Err("At least one --role value is required.".to_string());
    }
    let (remote, repo_name) = remote_context(
        repo,
        request.remote_name.as_deref(),
        request.repo_name.as_deref(),
    )?;
    http_grant_role_bindings(
        http_config(repo, &remote),
        &repo_name,
        &actor_identity,
        &roles,
    )
    .map_err(|err| err.to_string())
}

pub fn auth_bindings(repo: &RepoRuntime, request: &AuthRemoteRequest) -> Result<JsonValue, String> {
    let (remote, repo_name) = remote_context(
        repo,
        request.remote_name.as_deref(),
        request.repo_name.as_deref(),
    )?;
    http_list_role_bindings(http_config(repo, &remote), &repo_name)
        .map(JsonValue::Array)
        .map_err(|err| err.to_string())
}

fn remote_context(
    repo: &RepoRuntime,
    requested_remote: Option<&str>,
    requested_repo: Option<&str>,
) -> Result<(RemoteRow, String), String> {
    let remote = repo.remote_row(requested_remote)?;
    let repo_name = normalize_optional_text(requested_repo)
        .or_else(|| remote.repo_name.clone())
        .unwrap_or_else(|| repo.repo_name());
    Ok((remote, repo_name))
}

fn http_config(repo: &RepoRuntime, remote: &RemoteRow) -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: remote.url.clone(),
        repository_index: repo.repository_index(),
        headers: repo.auth_headers(),
        ..PlanHttpClientConfig::default()
    }
}

fn local_auth_snapshot(repo: &RepoRuntime, requested_repo: Option<&str>) -> JsonValue {
    let actor = repo
        .actor_identity()
        .unwrap_or_else(|| "anonymous".to_string());
    let mut payload = json!({
        "identity": actor,
        "mode": "open",
    });
    if let Some(repo_name) = normalize_optional_text(requested_repo) {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("repo_name".to_string(), JsonValue::String(repo_name));
        }
    }
    payload
}

fn normalize_required_text(value: &str, field: &str) -> Result<String, String> {
    normalize_optional_text(Some(value)).ok_or_else(|| format!("{field} is required."))
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}
