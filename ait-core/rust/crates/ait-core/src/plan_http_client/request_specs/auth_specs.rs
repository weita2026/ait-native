use super::*;

pub fn build_auth_whoami_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let query = normalize_optional_text(repo_name)
        .map(|repo_name| vec![("repo_name".to_string(), repo_name)])
        .unwrap_or_default();
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        "/v1/native/auth/whoami",
        query,
        None,
    )
}

pub fn build_grant_role_bindings_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    actor_identity: &str,
    roles: &[String],
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repo_name = require_non_empty_text(repo_name, "repo_name")?;
    let actor_identity = require_non_empty_text(actor_identity, "actor_identity")?;
    let roles = roles
        .iter()
        .filter_map(|role| normalize_optional_text(Some(role.as_str())))
        .map(Value::String)
        .collect::<Vec<_>>();
    if roles.is_empty() {
        return Err(PlanHttpClientError::Invalid(
            "Plan HTTP roles must not be empty.".to_string(),
        ));
    }
    let mut body = Map::new();
    body.insert("actor_identity".to_string(), Value::String(actor_identity));
    body.insert("roles".to_string(), Value::Array(roles));
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!(
            "/v1/native/admin/repositories/{}/bindings",
            encode_path_segment(&repo_name)
        ),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_list_role_bindings_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repo_name = require_non_empty_text(repo_name, "repo_name")?;
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &format!(
            "/v1/native/admin/repositories/{}/bindings",
            encode_path_segment(&repo_name)
        ),
        Vec::new(),
        None,
    )
}
