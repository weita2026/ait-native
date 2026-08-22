use super::*;

pub fn build_get_server_operational_capabilities_request_spec(
    config: &PlanHttpClientConfig,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        "/v1/native/capabilities",
        Vec::new(),
        None,
    )
}

pub fn build_get_repository_by_index_request_spec(
    config: &PlanHttpClientConfig,
    repository_index: RepositoryIndex,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &repository_authority_path(repository_index),
        Vec::new(),
        None,
    )
}

pub fn build_begin_repository_retirement_request_spec(
    config: &PlanHttpClientConfig,
    repository_index: RepositoryIndex,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &repository_retirement_path(repository_index),
        Vec::new(),
        Some(Value::Object(Map::new())),
    )
}

pub fn build_abort_repository_retirement_request_spec(
    config: &PlanHttpClientConfig,
    repository_index: RepositoryIndex,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &repository_retirement_abort_path(repository_index),
        Vec::new(),
        Some(Value::Object(Map::new())),
    )
}

pub fn build_get_repository_retirement_request_spec(
    config: &PlanHttpClientConfig,
    repository_index: RepositoryIndex,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &repository_retirement_path(repository_index),
        Vec::new(),
        None,
    )
}

pub fn build_get_repository_retirement_file_request_spec(
    config: &PlanHttpClientConfig,
    repository_index: RepositoryIndex,
    file_path: &str,
) -> PlanHttpClientResult<PlanHttpBytesRequestSpec> {
    let encoded_path = encoded_remote_authority_path(file_path)?;
    let path = format!(
        "{}/files/{encoded_path}",
        repository_retirement_path(repository_index)
    );
    let mut spec = plan_http_transport::build_bytes_request_spec(
        config,
        Method::GET,
        &path,
        Vec::new(),
        None,
        REMOTE_AUTHORITY_FILE_MEDIA_TYPE,
        None,
    )?;
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_purge_repository_retirement_request_spec(
    config: &PlanHttpClientConfig,
    repository_index: RepositoryIndex,
    manifest: &RemoteExportManifest,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let body = manifest.to_json().map_err(PlanHttpClientError::Invalid)?;
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &repository_retirement_purge_path(repository_index),
        Vec::new(),
        Some(body),
    )
}

pub fn build_begin_repository_restore_request_spec(
    config: &PlanHttpClientConfig,
    manifest: &RemoteExportManifest,
    policy_flags: u8,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let manifest = manifest.to_json().map_err(PlanHttpClientError::Invalid)?;
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        repository_restores_path(),
        Vec::new(),
        Some(Value::Object(Map::from_iter([
            ("manifest".to_string(), manifest),
            (
                "policy_flags".to_string(),
                Value::Number(u64::from(policy_flags).into()),
            ),
        ]))),
    )
}

pub fn build_upload_repository_restore_file_request_spec(
    config: &PlanHttpClientConfig,
    restore_token: &str,
    file_path: &str,
    bytes: Vec<u8>,
) -> PlanHttpClientResult<PlanHttpBytesRequestSpec> {
    let token = validated_restore_token(restore_token)?;
    let encoded_path = encoded_remote_authority_path(file_path)?;
    let path = format!(
        "{}/{token}/files/{encoded_path}",
        repository_restores_path()
    );
    let mut spec = plan_http_transport::build_bytes_request_spec(
        config,
        Method::PUT,
        &path,
        Vec::new(),
        Some(bytes),
        "application/json",
        Some(REMOTE_AUTHORITY_FILE_MEDIA_TYPE),
    )?;
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_commit_repository_restore_request_spec(
    config: &PlanHttpClientConfig,
    restore_token: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let token = validated_restore_token(restore_token)?;
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!("{}/{token}/commit", repository_restores_path()),
        Vec::new(),
        None,
    )
}

pub fn build_list_worker_jobs_request_spec(
    config: &PlanHttpClientConfig,
    repository_index: RepositoryIndex,
    state_kind: Option<u8>,
    limit: u32,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    crate::server_operational::validate_worker_job_list_limit(limit)
        .map_err(PlanHttpClientError::Invalid)?;
    let mut query_pairs = vec![("limit".to_string(), limit.to_string())];
    if let Some(state_kind) = state_kind {
        query_pairs.push(("state_kind".to_string(), state_kind.to_string()));
    }
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &repository_worker_jobs_path(repository_index),
        query_pairs,
        None,
    )
}

fn encoded_remote_authority_path(file_path: &str) -> PlanHttpClientResult<String> {
    validate_remote_authority_relative_path(file_path).map_err(PlanHttpClientError::Invalid)?;
    Ok(file_path
        .split('/')
        .map(encode_path_segment)
        .collect::<Vec<_>>()
        .join("/"))
}

fn validated_restore_token(value: &str) -> PlanHttpClientResult<String> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PlanHttpClientError::Invalid(
            "Repository restore token must be 32 lowercase hexadecimal characters.".to_string(),
        ));
    }
    Ok(value.to_string())
}

pub fn build_get_worker_job_request_spec(
    config: &PlanHttpClientConfig,
    key: WorkerJobKey,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &worker_job_path(key),
        Vec::new(),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_support::json;
    use crate::server_operational::WorkerJobIndex;

    fn config() -> PlanHttpClientConfig {
        PlanHttpClientConfig {
            base_url: "https://example.test".to_string(),
            ..PlanHttpClientConfig::default()
        }
    }

    #[test]
    fn numeric_specs_never_emit_legacy_identity_fields() {
        let key = WorkerJobKey::new(RepositoryIndex::new(4), WorkerJobIndex::new(8));
        let get = build_get_worker_job_request_spec(&config(), key).expect("get job spec");
        assert_eq!(
            get.path,
            "/v1/native/repository-authorities/4/worker-jobs/8"
        );
    }

    #[test]
    fn retirement_and_restore_specs_use_numeric_routes_and_raw_file_media_type() {
        let repository_index = RepositoryIndex::new(7);
        let manifest = RemoteExportManifest {
            schema: crate::server_repo_retire::REMOTE_EXPORT_SCHEMA.to_string(),
            state: crate::server_repo_retire::REMOTE_EXPORT_STATE_COMPLETE.to_string(),
            repo_name: "duplicate-name".to_string(),
            namespace: "R".to_string(),
            exported_at_s: 1_786_000_000,
            files: vec![crate::server_repo_retire::RemoteExportFile {
                path: "nested/worker job.bin".to_string(),
                size: 4,
                sha256: "0".repeat(64),
            }],
        };
        let begin =
            build_begin_repository_retirement_request_spec(&config(), repository_index).unwrap();
        assert_eq!(begin.path, "/v1/native/repository-authorities/7/retirement");
        let abort =
            build_abort_repository_retirement_request_spec(&config(), repository_index).unwrap();
        assert_eq!(
            abort.path,
            "/v1/native/repository-authorities/7/retirement/abort"
        );
        assert_eq!(abort.method, "POST");
        assert_eq!(abort.body, Some(json!({})));
        let download = build_get_repository_retirement_file_request_spec(
            &config(),
            repository_index,
            &manifest.files[0].path,
        )
        .unwrap();
        assert_eq!(
            download.path,
            "/v1/native/repository-authorities/7/retirement/files/nested/worker%20job.bin"
        );
        assert_eq!(download.headers["Accept"], REMOTE_AUTHORITY_FILE_MEDIA_TYPE);
        let purge =
            build_purge_repository_retirement_request_spec(&config(), repository_index, &manifest)
                .unwrap();
        assert_eq!(
            purge.path,
            "/v1/native/repository-authorities/7/retirement/purge"
        );
        assert!(purge.body.unwrap().get("old_repository_index").is_none());

        let token = "0123456789abcdef0123456789abcdef";
        let restore =
            build_begin_repository_restore_request_spec(&config(), &manifest, 0b1000_0011).unwrap();
        assert_eq!(restore.path, "/v1/native/repository-restores");
        let restore_body = restore.body.unwrap();
        assert_eq!(restore_body["policy_flags"], json!(0b1000_0011));
        assert_eq!(restore_body["manifest"], manifest.to_json().unwrap());
        assert!(restore_body.get("old_repository_index").is_none());
        assert!(restore_body.get("server_instance_id").is_none());
        let upload = build_upload_repository_restore_file_request_spec(
            &config(),
            token,
            &manifest.files[0].path,
            vec![1, 2, 3, 4],
        )
        .unwrap();
        assert_eq!(
            upload.path,
            "/v1/native/repository-restores/0123456789abcdef0123456789abcdef/files/nested/worker%20job.bin"
        );
        assert_eq!(
            upload.headers["Content-Type"],
            REMOTE_AUTHORITY_FILE_MEDIA_TYPE
        );
        let commit = build_commit_repository_restore_request_spec(&config(), token).unwrap();
        assert_eq!(
            commit.path,
            "/v1/native/repository-restores/0123456789abcdef0123456789abcdef/commit"
        );
    }
}
