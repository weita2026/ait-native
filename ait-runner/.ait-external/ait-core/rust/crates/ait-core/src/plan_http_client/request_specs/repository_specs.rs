use super::*;

pub fn build_ensure_repository_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    default_line: &str,
    policy: Option<&Value>,
    id_namespace_prefix: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repo_name = require_non_empty_text(repo_name, "repo_name")?;
    if require_non_empty_text(default_line, "default_line")? != "main" {
        return Err(PlanHttpClientError::Invalid(
            "Binary Repository registration requires logical default Line main.".to_string(),
        ));
    }
    let namespace =
        validate_repository_registration_namespace(id_namespace_prefix.unwrap_or_default())?;
    let policy_flags = repository_registration_policy_flags(policy)?;
    let mut body = Map::new();
    body.insert("repository_name".to_string(), Value::String(repo_name));
    body.insert("namespace".to_string(), Value::String(namespace));
    body.insert(
        "policy_flags".to_string(),
        Value::Number(u64::from(policy_flags).into()),
    );
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        "/v1/native/repository-authorities",
        Vec::new(),
        Some(Value::Object(body)),
    )
}

fn validate_repository_registration_namespace(value: &str) -> PlanHttpClientResult<String> {
    if value.len() > 2
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
    {
        return Err(PlanHttpClientError::Invalid(
            "Binary Repository namespace must contain zero, one, or two ASCII alphanumeric, underscore, or hyphen bytes."
                .to_string(),
        ));
    }
    Ok(value.to_string())
}

pub fn repository_registration_policy_flags(policy: Option<&Value>) -> PlanHttpClientResult<u8> {
    let Some(policy) = policy else {
        return Ok(0b1000_0011);
    };
    let object = policy.as_object().ok_or_else(|| {
        PlanHttpClientError::Invalid(
            "Binary Repository prototype policy must be a JSON object.".to_string(),
        )
    })?;
    reject_repository_policy_extra_keys(
        object,
        &["policy_id", "version", "defaults", "class_overrides"],
        "policy",
    )?;
    if object.get("policy_id").and_then(Value::as_str) != Some("prototype") {
        return Err(PlanHttpClientError::Invalid(
            "Binary Repository policy_id must be exact prototype.".to_string(),
        ));
    }
    if object
        .get("version")
        .is_some_and(|version| version.as_u64() != Some(1))
    {
        return Err(PlanHttpClientError::Invalid(
            "Binary Repository prototype policy version must be exact integer 1.".to_string(),
        ));
    }
    let defaults = object
        .get("defaults")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "Binary Repository prototype policy requires defaults object.".to_string(),
            )
        })?;
    let names = [
        "require_attestation",
        "require_tests",
        "require_lint",
        "require_security_scan",
        "require_license_scan",
        "require_ai_provenance",
        "require_code_review_summary",
    ];
    reject_repository_policy_extra_keys(defaults, &names, "policy.defaults")?;
    let mut flags = 0_u8;
    for (index, name) in names.iter().enumerate() {
        let enabled = match defaults.get(*name) {
            None => index < 2,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(PlanHttpClientError::Invalid(format!(
                    "Binary Repository prototype policy defaults.{name} must be boolean."
                )))
            }
        };
        if enabled {
            flags |= 1 << index;
        }
    }
    let docs_override = match object.get("class_overrides") {
        None => true,
        Some(Value::Array(values)) if values.is_empty() => false,
        Some(Value::Array(values)) if values.len() == 1 => {
            validate_repository_docs_override(&values[0])?;
            true
        }
        Some(_) => {
            return Err(PlanHttpClientError::Invalid(
                "Binary Repository prototype policy class_overrides must be absent, empty, or the exact docs-only override."
                    .to_string(),
            ))
        }
    };
    if docs_override {
        flags |= 1 << 7;
    }
    Ok(flags)
}

fn validate_repository_docs_override(value: &Value) -> PlanHttpClientResult<()> {
    let object = value.as_object().ok_or_else(|| {
        PlanHttpClientError::Invalid(
            "Binary Repository docs-only override must be an object.".to_string(),
        )
    })?;
    reject_repository_policy_extra_keys(object, &["when", "set"], "class_overrides[0]")?;
    let when = object
        .get("when")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "Binary Repository docs-only override requires when object.".to_string(),
            )
        })?;
    if when.len() != 1 || when.get("content_class").and_then(Value::as_str) != Some("docs_only") {
        return Err(PlanHttpClientError::Invalid(
            "Binary Repository docs-only override when object is not exact.".to_string(),
        ));
    }
    let set = object
        .get("set")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "Binary Repository docs-only override requires set object.".to_string(),
            )
        })?;
    let required = [
        "require_tests",
        "require_lint",
        "require_security_scan",
        "require_license_scan",
    ];
    reject_repository_policy_extra_keys(set, &required, "class_overrides[0].set")?;
    if set.len() != required.len()
        || required
            .iter()
            .any(|name| set.get(*name) != Some(&Value::Bool(false)))
    {
        return Err(PlanHttpClientError::Invalid(
            "Binary Repository docs-only override set object is not exact.".to_string(),
        ));
    }
    Ok(())
}

fn reject_repository_policy_extra_keys(
    object: &Map<String, Value>,
    allowed: &[&str],
    label: &str,
) -> PlanHttpClientResult<()> {
    if let Some(extra) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(PlanHttpClientError::Invalid(format!(
            "Binary Repository prototype {label} contains unknown field {extra}."
        )));
    }
    Ok(())
}

pub fn build_get_server_handshake_request_spec(
    config: &PlanHttpClientConfig,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(config, Method::GET, "/v1/handshake", Vec::new(), None)
}

pub fn build_get_server_health_request_spec(
    config: &PlanHttpClientConfig,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(config, Method::GET, "/healthz", Vec::new(), None)
}

pub fn build_get_repository_storage_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repo_name = require_non_empty_text(repo_name, "repo_name")?;
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &format!(
            "/v1/native/admin/repositories/{}/storage",
            encode_path_segment(&repo_name)
        ),
        Vec::new(),
        None,
    )
}

pub fn build_pack_repo_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    repack: bool,
    max_members: Option<i64>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repo_name = require_non_empty_text(repo_name, "repo_name")?;
    let mut body = Map::new();
    body.insert("repack".to_string(), Value::Bool(repack));
    body.insert(
        "max_members".to_string(),
        max_members
            .map(|value| Value::Number(value.into()))
            .unwrap_or(Value::Null),
    );
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!(
            "/v1/native/admin/repositories/{}:pack",
            encode_path_segment(&repo_name)
        ),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_optimize_repo_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    repair: bool,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repo_name = require_non_empty_text(repo_name, "repo_name")?;
    let mut body = Map::new();
    body.insert("repair".to_string(), Value::Bool(repair));
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!(
            "/v1/native/admin/repositories/{}:optimize",
            encode_path_segment(&repo_name)
        ),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_gc_repo_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    prune_unreferenced: bool,
    prune_orphan_packs: bool,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repo_name = require_non_empty_text(repo_name, "repo_name")?;
    let mut body = Map::new();
    body.insert(
        "prune_unreferenced".to_string(),
        Value::Bool(prune_unreferenced),
    );
    body.insert(
        "prune_orphan_packs".to_string(),
        Value::Bool(prune_orphan_packs),
    );
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!(
            "/v1/native/admin/repositories/{}:gc",
            encode_path_segment(&repo_name)
        ),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_retire_repo_request_spec(
    _config: &PlanHttpClientConfig,
    _repo_name: &str,
    _expected_repository_identity: &str,
    _require_verified_export: bool,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    Err(PlanHttpClientError::Invalid(
        "Repository retirement is excluded from the active Binary DB v0 server contract."
            .to_string(),
    ))
}

pub fn build_list_repo_jobs_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    state: Option<&str>,
    limit: i64,
    diagnostics: bool,
    stale_after_seconds: i64,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repo_name = require_non_empty_text(repo_name, "repo_name")?;
    let mut query_pairs = vec![
        ("limit".to_string(), limit.to_string()),
        ("diagnostics".to_string(), diagnostics.to_string()),
        (
            "stale_after_seconds".to_string(),
            stale_after_seconds.to_string(),
        ),
    ];
    if let Some(state) = normalize_optional_text(state) {
        query_pairs.push(("state".to_string(), state));
    }
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &format!(
            "/v1/native/admin/repositories/{}/jobs",
            encode_path_segment(&repo_name)
        ),
        query_pairs,
        None,
    )
}

pub fn build_get_repo_job_request_spec(
    config: &PlanHttpClientConfig,
    job_id: i64,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &format!("/v1/native/admin/jobs/{job_id}"),
        Vec::new(),
        None,
    )
}

pub fn build_get_server_metrics_request_spec(
    config: &PlanHttpClientConfig,
    recent_jobs_limit: i64,
    stale_after_seconds: i64,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        "/v1/native/admin/metrics",
        vec![
            (
                "recent_jobs_limit".to_string(),
                recent_jobs_limit.to_string(),
            ),
            (
                "stale_after_seconds".to_string(),
                stale_after_seconds.to_string(),
            ),
        ],
        None,
    )
}

pub fn build_get_server_readiness_request_spec(
    config: &PlanHttpClientConfig,
    recent_jobs_limit: i64,
    stale_after_seconds: i64,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        "/v1/native/admin/readiness",
        vec![
            (
                "recent_jobs_limit".to_string(),
                recent_jobs_limit.to_string(),
            ),
            (
                "stale_after_seconds".to_string(),
                stale_after_seconds.to_string(),
            ),
        ],
        None,
    )
}

pub fn build_reconcile_repo_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    repair: bool,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repo_name = require_non_empty_text(repo_name, "repo_name")?;
    let mut body = Map::new();
    body.insert("repair".to_string(), Value::Bool(repair));
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!(
            "/v1/native/admin/repositories/{}:reconcile",
            encode_path_segment(&repo_name)
        ),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_get_line_request_spec(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
    line_name: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    let line_name = require_non_empty_text(line_name, "line_name")?;
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &format!(
            "/v1/native/repository-authorities/{repository_index}/lines/{}",
            encode_path_segment(&line_name)
        ),
        Vec::new(),
        None,
    )
}

pub fn build_list_lines_request_spec(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &format!("/v1/native/repository-authorities/{repository_index}/lines"),
        Vec::new(),
        None,
    )
}

pub fn build_update_remote_line_request_spec(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
    line_name: &str,
    head_snapshot_id: Option<&str>,
    expected_head_snapshot_id: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    let line_name = require_non_empty_text(line_name, "line_name")?;
    let mut body = Map::new();
    body.insert(
        "head_snapshot_id".to_string(),
        optional_json_string(head_snapshot_id),
    );
    if expected_head_snapshot_id.is_some() {
        body.insert(
            "expected_head_snapshot_id".to_string(),
            optional_json_string(expected_head_snapshot_id),
        );
    }
    plan_http_transport::build_request_spec(
        config,
        Method::PUT,
        &format!(
            "/v1/native/repository-authorities/{repository_index}/lines/{}",
            encode_path_segment(&line_name)
        ),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_rename_remote_line_request_spec(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
    old_line_name: &str,
    new_line_name: &str,
    expected_line_id: &str,
    expected_head_snapshot_id: Option<&str>,
    idempotency_key: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    let old_line_name = require_non_empty_text(old_line_name, "old_line_name")?;
    let new_line_name = require_non_empty_text(new_line_name, "new_line_name")?;
    let expected_line_id = require_non_empty_text(expected_line_id, "expected_line_id")?;
    let idempotency_key = require_non_empty_text(idempotency_key, "idempotency_key")?;
    let body = Value::Object(Map::from_iter([
        (
            "contract".to_string(),
            Value::String("line-lifecycle/v1".to_string()),
        ),
        ("new_line_name".to_string(), Value::String(new_line_name)),
        (
            "expected_line_id".to_string(),
            Value::String(expected_line_id),
        ),
        (
            "expected_head_snapshot_id".to_string(),
            optional_json_string(expected_head_snapshot_id),
        ),
        (
            "idempotency_key".to_string(),
            Value::String(idempotency_key),
        ),
    ]));
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!(
            "/v1/native/repository-authorities/{repository_index}/lines/{}:rename",
            encode_path_segment(&old_line_name)
        ),
        Vec::new(),
        Some(body),
    )
}

pub fn build_delete_remote_line_request_spec(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
    line_name: &str,
    expected_line_id: &str,
    expected_head_snapshot_id: Option<&str>,
    idempotency_key: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    let line_name = require_non_empty_text(line_name, "line_name")?;
    let expected_line_id = require_non_empty_text(expected_line_id, "expected_line_id")?;
    let idempotency_key = require_non_empty_text(idempotency_key, "idempotency_key")?;
    let body = Value::Object(Map::from_iter([
        (
            "contract".to_string(),
            Value::String("line-lifecycle/v1".to_string()),
        ),
        (
            "expected_line_id".to_string(),
            Value::String(expected_line_id),
        ),
        (
            "expected_head_snapshot_id".to_string(),
            optional_json_string(expected_head_snapshot_id),
        ),
        (
            "idempotency_key".to_string(),
            Value::String(idempotency_key),
        ),
    ]));
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!(
            "/v1/native/repository-authorities/{repository_index}/lines/{}:delete",
            encode_path_segment(&line_name)
        ),
        Vec::new(),
        Some(body),
    )
}

pub fn build_get_remote_snapshot_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    snapshot_id: &str,
    include_content: bool,
    path: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    SnapshotJson::stateless().build_get_remote_snapshot_request_spec(
        config,
        repo_name,
        snapshot_id,
        include_content,
        path,
    )
}

pub fn build_plan_remote_zstd_bulk_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    request: &Value,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let typed_request = ZstdBulkPlanRequestJson::stateless()
        .decode_value(request.clone())
        .map_err(PlanHttpClientError::Invalid)?;
    build_plan_remote_zstd_bulk_typed_request_spec(config, repo_name, &typed_request)
}

pub fn build_plan_remote_zstd_bulk_typed_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    request: &ZstdBulkPlanRequest,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let body = ZstdBulkPlanRequestJson::stateless()
        .encode_value(request)
        .map_err(PlanHttpClientError::Invalid)?;
    let mut spec = plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &remote_sync_path(config, repo_name, "plan")?,
        Vec::new(),
        Some(body),
    )?;
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_put_remote_zstd_object_pack_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    pack_id: &str,
    pack_bytes: &[u8],
) -> PlanHttpClientResult<PlanHttpBytesRequestSpec> {
    let pack_id = require_non_empty_text(pack_id, "pack_id")?;
    let mut spec = plan_http_transport::build_bytes_request_spec(
        config,
        Method::PUT,
        &remote_sync_path(
            config,
            repo_name,
            &format!("object-packs/{}", encode_path_segment(&pack_id)),
        )?,
        Vec::new(),
        Some(pack_bytes.to_vec()),
        "application/json",
        Some(ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE),
    )?;
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_put_remote_zstd_tree_pack_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    pack_id: &str,
    pack_bytes: &[u8],
) -> PlanHttpClientResult<PlanHttpBytesRequestSpec> {
    let pack_id = require_non_empty_text(pack_id, "pack_id")?;
    let mut spec = plan_http_transport::build_bytes_request_spec(
        config,
        Method::PUT,
        &remote_sync_path(
            config,
            repo_name,
            &format!("tree-packs/{}", encode_path_segment(&pack_id)),
        )?,
        Vec::new(),
        Some(pack_bytes.to_vec()),
        "application/json",
        Some(ZSTD_BULK_TREE_PACK_MEDIA_TYPE),
    )?;
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_commit_remote_zstd_bulk_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    request: &Value,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let typed_request = ZstdBulkCommitRequestJson::stateless()
        .decode_value(request.clone())
        .map_err(PlanHttpClientError::Invalid)?;
    build_commit_remote_zstd_bulk_typed_request_spec(config, repo_name, &typed_request)
}

pub fn build_commit_remote_zstd_bulk_typed_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    request: &ZstdBulkCommitRequest,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let body = ZstdBulkCommitRequestJson::stateless()
        .encode_value(request)
        .map_err(PlanHttpClientError::Invalid)?;
    let mut spec = plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &remote_sync_path(config, repo_name, "commit")?,
        Vec::new(),
        Some(body),
    )?;
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_get_remote_zstd_object_pack_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    pack_id: &str,
) -> PlanHttpClientResult<PlanHttpBytesRequestSpec> {
    let pack_id = require_non_empty_text(pack_id, "pack_id")?;
    let mut spec = plan_http_transport::build_bytes_request_spec(
        config,
        Method::GET,
        &remote_sync_path(
            config,
            repo_name,
            &format!("object-packs/{}", encode_path_segment(&pack_id)),
        )?,
        Vec::new(),
        None,
        ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE,
        None,
    )?;
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_get_remote_zstd_tree_pack_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    pack_id: &str,
) -> PlanHttpClientResult<PlanHttpBytesRequestSpec> {
    let pack_id = require_non_empty_text(pack_id, "pack_id")?;
    let mut spec = plan_http_transport::build_bytes_request_spec(
        config,
        Method::GET,
        &remote_sync_path(
            config,
            repo_name,
            &format!("tree-packs/{}", encode_path_segment(&pack_id)),
        )?,
        Vec::new(),
        None,
        ZSTD_BULK_TREE_PACK_MEDIA_TYPE,
        None,
    )?;
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_get_remote_zstd_import_manifest_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    snapshot_id: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let snapshot_id = require_non_empty_text(snapshot_id, "snapshot_id")?;
    let mut spec = plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &remote_sync_path(
            config,
            repo_name,
            &format!("import-manifests/{}", encode_path_segment(&snapshot_id)),
        )?,
        Vec::new(),
        None,
    )?;
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_get_remote_zstd_pull_manifest_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    request: &ZstdPullManifestRequest,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let body = ZstdPullManifestRequestJson::stateless()
        .encode_value(request)
        .map_err(PlanHttpClientError::Invalid)?;
    let mut spec = plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &remote_sync_path(config, repo_name, "pull-manifests")?,
        Vec::new(),
        Some(body),
    )?;
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_get_remote_snapshots_existence_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    snapshot_ids: &[String],
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    SnapshotJson::stateless().build_get_remote_snapshots_existence_request_spec(
        config,
        repo_name,
        snapshot_ids,
    )
}
