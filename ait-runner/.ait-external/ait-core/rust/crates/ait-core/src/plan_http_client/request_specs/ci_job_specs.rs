use super::*;

pub fn build_publish_patchset_request_spec(
    config: &PlanHttpClientConfig,
    change_id: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    summary: &str,
    author_mode: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    PatchsetJson::stateless().build_publish_patchset_request_spec(
        config,
        change_id,
        base_snapshot_id,
        revision_snapshot_id,
        summary,
        author_mode,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_publish_release_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    release_id: &str,
    version: &str,
    line: &str,
    snapshot_id: &str,
    manifest_hash: &str,
    profile: &str,
    package: Value,
    checks: Value,
    artifacts: Value,
    formula: Value,
    metadata: Value,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repo_name = require_non_empty_text(repo_name, "repo_name")?;
    let release_id = require_non_empty_text(release_id, "release_id")?;
    let version = require_non_empty_text(version, "version")?;
    let line = require_non_empty_text(line, "line")?;
    let snapshot_id = require_non_empty_text(snapshot_id, "snapshot_id")?;
    let manifest_hash = require_non_empty_text(manifest_hash, "manifest_hash")?;
    let profile = require_non_empty_text(profile, "profile")?;
    let mut body = Map::new();
    body.insert("release_id".to_string(), Value::String(release_id));
    body.insert("version".to_string(), Value::String(version));
    body.insert("line".to_string(), Value::String(line));
    body.insert("snapshot_id".to_string(), Value::String(snapshot_id));
    body.insert("manifest_hash".to_string(), Value::String(manifest_hash));
    body.insert("profile".to_string(), Value::String(profile));
    body.insert("package".to_string(), object_or_empty(package, "package")?);
    body.insert("checks".to_string(), array_or_empty(checks, "checks")?);
    body.insert(
        "artifacts".to_string(),
        array_or_empty(artifacts, "artifacts")?,
    );
    body.insert("formula".to_string(), object_or_empty(formula, "formula")?);
    body.insert(
        "metadata".to_string(),
        object_or_empty(metadata, "metadata")?,
    );
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!("/v1/native/repositories/{repo_name}/releases"),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_get_release_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    release_ref: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repo_name = require_non_empty_text(repo_name, "repo_name")?;
    let release_ref = require_non_empty_text(release_ref, "release_ref")?;
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &format!(
            "/v1/native/repositories/{repo_name}/releases/{}",
            encode_path_segment(&release_ref)
        ),
        Vec::new(),
        None,
    )
}

pub fn build_list_patchsets_request_spec(
    config: &PlanHttpClientConfig,
    change_id: &str,
    repo_name: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    PatchsetJson::stateless().build_list_patchsets_request_spec(config, change_id, repo_name)
}

pub fn build_get_patchset_request_spec(
    config: &PlanHttpClientConfig,
    patchset_id: &str,
    repo_name: Option<&str>,
    change_ref: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    PatchsetJson::stateless().build_get_patchset_request_spec(
        config,
        patchset_id,
        repo_name,
        change_ref,
    )
}

pub fn build_select_patchset_request_spec(
    config: &PlanHttpClientConfig,
    change_id: &str,
    patchset_id: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    PatchsetJson::stateless().build_select_patchset_request_spec(config, change_id, patchset_id)
}

pub fn build_run_patchset_ci_request_spec(
    config: &PlanHttpClientConfig,
    patchset_id: &str,
    trigger: &str,
    execution_profile: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    PatchsetJson::stateless().build_run_patchset_ci_request_spec(
        config,
        patchset_id,
        trigger,
        execution_profile,
    )
}

pub fn build_request_review_request_spec(
    config: &PlanHttpClientConfig,
    change_id: &str,
    patchset_id: &str,
    reviewer_groups: &[String],
    note: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let change_id = require_non_empty_text(change_id, "change_id")?;
    let patchset_id = require_non_empty_text(patchset_id, "patchset_id")?;
    let reviewer_groups = reviewer_groups
        .iter()
        .map(|group| require_non_empty_text(group, "reviewer_group").map(Value::String))
        .collect::<PlanHttpClientResult<Vec<_>>>()?;
    let mut body = Map::new();
    body.insert("patchset_id".to_string(), Value::String(patchset_id));
    body.insert("reviewer_groups".to_string(), Value::Array(reviewer_groups));
    body.insert("note".to_string(), optional_json_string(note));
    let change_id = encode_path_segment(&change_id);
    let repository_index = configured_repository_authority_path_segment(config)?;
    let path = format!(
        "/v1/native/repository-authorities/{repository_index}/changes/{change_id}:requestReview"
    );
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &path,
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_read_patchset_ci_status_request_spec(
    config: &PlanHttpClientConfig,
    patchset_id: &str,
    recent_limit: i64,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    PatchsetJson::stateless().build_read_patchset_ci_status_request_spec(
        config,
        patchset_id,
        recent_limit,
    )
}

pub fn build_read_patchset_ci_readiness_request_spec(
    config: &PlanHttpClientConfig,
    patchset_id: &str,
    recent_limit: i64,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    PatchsetJson::stateless().build_read_patchset_ci_readiness_request_spec(
        config,
        patchset_id,
        recent_limit,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_record_review_request_spec(
    config: &PlanHttpClientConfig,
    change_id: &str,
    patchset_id: &str,
    reviewer: &str,
    action: &str,
    comment: Option<&str>,
    blocking: bool,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let change_id = require_non_empty_text(change_id, "change_id")?;
    let patchset_id = require_non_empty_text(patchset_id, "patchset_id")?;
    let reviewer = require_non_empty_text(reviewer, "reviewer")?;
    let action = require_non_empty_text(action, "action")?;
    let mut body = Map::new();
    body.insert("patchset_id".to_string(), Value::String(patchset_id));
    body.insert("reviewer".to_string(), Value::String(reviewer));
    body.insert("action".to_string(), Value::String(action));
    body.insert("blocking".to_string(), Value::Bool(blocking));
    body.insert("comment".to_string(), optional_json_string(comment));
    let change_id = encode_path_segment(&change_id);
    let repository_index = configured_repository_authority_path_segment(config)?;
    let path =
        format!("/v1/native/repository-authorities/{repository_index}/changes/{change_id}/reviews");
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &path,
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_list_reviews_request_spec(
    config: &PlanHttpClientConfig,
    change_id: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let change_id = require_non_empty_text(change_id, "change_id")?;
    let change_id = encode_path_segment(&change_id);
    let repository_index = configured_repository_authority_path_segment(config)?;
    let path =
        format!("/v1/native/repository-authorities/{repository_index}/changes/{change_id}/reviews");
    plan_http_transport::build_request_spec(config, Method::GET, &path, Vec::new(), None)
}

pub fn build_put_attestation_request_spec(
    config: &PlanHttpClientConfig,
    patchset_id: &str,
    author_mode: &str,
    evaluation_summary: &Value,
    provenance_summary: &Value,
    detail: &Value,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    AttestJson::stateless().build_put_attestation_request_spec(
        config,
        patchset_id,
        author_mode,
        evaluation_summary,
        provenance_summary,
        detail,
    )
}

pub fn build_get_attestation_request_spec(
    config: &PlanHttpClientConfig,
    patchset_id: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    AttestJson::stateless().build_get_attestation_request_spec(config, patchset_id)
}

pub fn build_evaluate_policy_request_spec(
    config: &PlanHttpClientConfig,
    patchset_id: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    PolicyJson::stateless().build_evaluate_policy_request_spec(config, patchset_id)
}

pub fn build_get_policy_request_spec(
    config: &PlanHttpClientConfig,
    patchset_id: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    PolicyJson::stateless().build_get_policy_request_spec(config, patchset_id)
}

pub fn build_submit_land_request_spec(
    config: &PlanHttpClientConfig,
    change_id: &str,
    patchset_id: Option<&str>,
    target_line: &str,
    mode: &str,
    repo_name: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    LandJson::stateless().build_submit_land_request_spec(
        config,
        change_id,
        patchset_id,
        target_line,
        mode,
        repo_name,
    )
}

pub fn build_submit_task_land_request_spec(
    config: &PlanHttpClientConfig,
    task_or_change_ref: &str,
    target_line: Option<&str>,
    mode: &str,
    idempotency_key: &str,
    repo_name: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    LandJson::stateless().build_submit_task_land_request_spec(
        config,
        task_or_change_ref,
        target_line,
        mode,
        idempotency_key,
        repo_name,
    )
}

pub fn build_get_land_request_spec(
    config: &PlanHttpClientConfig,
    submission_id: &str,
    repo_name: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    LandJson::stateless().build_get_land_request_spec(config, submission_id, repo_name)
}

pub fn build_create_waiver_request_spec(
    config: &PlanHttpClientConfig,
    patchset_id: &str,
    rule_name: &str,
    reason: &str,
    expires_at: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    PolicyJson::stateless().build_create_waiver_request_spec(
        config,
        patchset_id,
        rule_name,
        reason,
        expires_at,
    )
}

pub fn build_retry_land_request_spec(
    config: &PlanHttpClientConfig,
    submission_id: &str,
    reason: Option<&str>,
    repo_name: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    LandJson::stateless().build_retry_land_request_spec(config, submission_id, reason, repo_name)
}
