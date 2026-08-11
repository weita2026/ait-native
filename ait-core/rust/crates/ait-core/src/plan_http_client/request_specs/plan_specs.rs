use super::*;

pub fn build_list_plans_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    artifact_path: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let normalized_artifact_path = normalize_optional_text(artifact_path);
    let mut query_pairs = Vec::new();
    if let Some(path) = normalized_artifact_path {
        query_pairs.push(("artifact_path".to_string(), path));
    }
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &plan_collection_path(config, repo_name)?,
        query_pairs,
        None,
    )
}

pub fn build_get_plan_request_spec(
    config: &PlanHttpClientConfig,
    plan_id: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &plan_item_path(config, plan_id, "")?,
        Vec::new(),
        None,
    )
}

pub fn build_list_plan_revisions_request_spec(
    config: &PlanHttpClientConfig,
    plan_id: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &plan_item_path(config, plan_id, "/revisions")?,
        Vec::new(),
        None,
    )
}

pub fn build_resolve_task_plan_linkage_request_spec(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    let mut body = Map::new();
    body.insert("plan_id".to_string(), optional_json_string(plan_id));
    body.insert(
        "origin_plan_revision_id".to_string(),
        optional_json_string(origin_plan_revision_id),
    );
    body.insert(
        "plan_item_ref".to_string(),
        optional_json_string(plan_item_ref),
    );
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!(
            "/v1/native/repository-authorities/{repository_index}/sprint-task-linkage/resolve"
        ),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_list_plan_ids_matching_contains_request_spec(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
    contains_terms: &[String],
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    let terms = contains_terms
        .iter()
        .filter_map(|entry| normalize_optional_text(Some(entry.as_str())))
        .map(Value::String)
        .collect::<Vec<_>>();
    let mut body = Map::new();
    body.insert("contains_terms".to_string(), Value::Array(terms));
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!(
            "/v1/native/repository-authorities/{repository_index}/sprint-plan-ids/by-contains"
        ),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_read_plan_candidate_inputs_request_spec(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
    contains_terms: &[String],
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let mut query_pairs = Vec::new();
    let normalized_terms: Vec<String> = contains_terms
        .iter()
        .filter_map(|entry| normalize_optional_text(Some(entry.as_str())))
        .collect();
    if !normalized_terms.is_empty() {
        query_pairs.push(("contains".to_string(), normalized_terms.join(",")));
    }
    let repository_index = configured_repository_authority_path_segment(config)?;
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &format!(
            "/v1/native/repository-authorities/{repository_index}/read/plans/candidate-inputs"
        ),
        query_pairs,
        None,
    )
}

pub fn build_get_plan_revision_request_spec(
    config: &PlanHttpClientConfig,
    plan_id: &str,
    plan_revision_id: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &plan_item_path(
            config,
            plan_id,
            &format!("/revisions/{}", encode_path_segment(plan_revision_id)),
        )?,
        Vec::new(),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_create_plan_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    title: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items: &[Value],
    summary: Option<&str>,
    status: &str,
    _plan_id: Option<&str>,
    source_kind: &str,
    artifact_body: Option<&str>,
    packed_artifact: Option<&Value>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let mut body = Map::new();
    body.insert("title".to_string(), Value::String(title.to_string()));
    body.insert(
        "artifact_path".to_string(),
        Value::String(artifact_path.to_string()),
    );
    body.insert(
        "artifact_selector".to_string(),
        optional_json_string(artifact_selector),
    );
    body.insert(
        "artifact_heading".to_string(),
        Value::String(artifact_heading.to_string()),
    );
    body.insert("items".to_string(), Value::Array(items.to_vec()));
    body.insert("status".to_string(), Value::String(status.to_string()));
    body.insert(
        "source_kind".to_string(),
        Value::String(source_kind.to_string()),
    );
    insert_optional_string(&mut body, "summary", summary);
    insert_optional_exact_string(&mut body, "artifact_body", artifact_body);
    insert_optional_packed_artifact(&mut body, packed_artifact);
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &plan_collection_path(config, repo_name)?,
        Vec::new(),
        Some(Value::Object(body)),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_revise_plan_request_spec(
    config: &PlanHttpClientConfig,
    plan_id: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items: &[Value],
    title: Option<&str>,
    summary: Option<&str>,
    source_kind: &str,
    artifact_body: Option<&str>,
    expected_head_revision_id: Option<&str>,
    packed_artifact: Option<&Value>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let mut body = Map::new();
    body.insert(
        "artifact_path".to_string(),
        Value::String(artifact_path.to_string()),
    );
    body.insert(
        "artifact_selector".to_string(),
        optional_json_string(artifact_selector),
    );
    body.insert(
        "artifact_heading".to_string(),
        Value::String(artifact_heading.to_string()),
    );
    body.insert("items".to_string(), Value::Array(items.to_vec()));
    body.insert(
        "source_kind".to_string(),
        Value::String(source_kind.to_string()),
    );
    insert_optional_string(&mut body, "title", title);
    insert_optional_string(&mut body, "summary", summary);
    insert_optional_exact_string(&mut body, "artifact_body", artifact_body);
    insert_optional_packed_artifact(&mut body, packed_artifact);
    insert_optional_string(
        &mut body,
        "expected_head_revision_id",
        expected_head_revision_id,
    );
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &plan_item_path(config, plan_id, "/revisions")?,
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_update_plan_status_request_spec(
    config: &PlanHttpClientConfig,
    plan_id: &str,
    status: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let mut body = Map::new();
    body.insert("status".to_string(), Value::String(status.to_string()));
    plan_http_transport::build_request_spec(
        config,
        Method::PATCH,
        &plan_item_path(config, plan_id, "")?,
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_put_plan_revision_artifacts_request_spec(
    config: &PlanHttpClientConfig,
    plan_id: &str,
    plan_revision_id: &str,
    artifacts: &[Value],
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let mut body = Map::new();
    body.insert("artifacts".to_string(), Value::Array(artifacts.to_vec()));
    plan_http_transport::build_request_spec(
        config,
        Method::PUT,
        &plan_item_path(
            config,
            plan_id,
            &format!(
                "/revisions/{}/artifacts",
                encode_path_segment(plan_revision_id)
            ),
        )?,
        Vec::new(),
        Some(Value::Object(body)),
    )
}
