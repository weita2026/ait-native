use super::*;
use crate::workflow_expected_identity::PackageWorkflowExpectedIdentity;
use crate::workflow_package::{
    EncodedWorkflowPackage, WorkflowPackageManifest, WORKFLOW_PACKAGE_MEDIA_TYPE,
};
use sha2::{Digest, Sha256};

fn package_authority(
    config: &PlanHttpClientConfig,
    expected: &PackageWorkflowExpectedIdentity,
) -> PlanHttpClientResult<String> {
    expected.validate().map_err(PlanHttpClientError::Invalid)?;
    if config.repository_index.map(|index| index.get()) != Some(expected.repository_index) {
        return Err(PlanHttpClientError::Invalid(
            "Package identity disagrees with HTTP Repository authority".into(),
        ));
    }
    Ok(format!(
        "/v1/native/repository-authorities/{}",
        expected.repository_index
    ))
}

pub fn build_prepare_workflow_package_request_spec(
    config: &PlanHttpClientConfig,
    manifest: &WorkflowPackageManifest,
    package: &EncodedWorkflowPackage,
    actor_key_index: u32,
    recovery: Option<&PackageWorkflowExpectedIdentity>,
) -> PlanHttpClientResult<PlanHttpBytesRequestSpec> {
    if config.repository_index.map(|index| index.get()) != Some(manifest.repository_index) {
        return Err(PlanHttpClientError::Invalid(
            "Prepared package Repository differs from HTTP authority".into(),
        ));
    }
    if let Some(expected) = recovery {
        expected.validate().map_err(PlanHttpClientError::Invalid)?;
    }
    let path = format!(
        "/v1/native/repository-authorities/{}",
        manifest.repository_index
    );
    if recovery.is_some_and(|expected| {
        expected.repository_index != manifest.repository_index
            || expected.package_hash != package.package_hash
            || expected.base_snapshot_id != manifest.base_snapshot_id
            || expected.revision_snapshot_id != manifest.revision_snapshot_id
    }) || format!("{:x}", Sha256::digest(&package.bytes)) != package.package_hash
    {
        return Err(PlanHttpClientError::Invalid(
            "Prepared bytes disagree with exact package recovery identity".into(),
        ));
    }
    super::super::actor_registration::validate_actor_key_index(actor_key_index)?;
    let mut spec = plan_http_transport::build_bytes_request_spec(
        config,
        Method::POST,
        &format!("{path}/remote-sync/zstd-bulk/commit"),
        Vec::new(),
        Some(package.bytes.clone()),
        "application/json",
        Some(WORKFLOW_PACKAGE_MEDIA_TYPE),
    )?;
    spec.headers
        .insert("X-Ait-Package-Sha256".into(), package.package_hash.clone());
    spec.headers
        .insert("X-Ait-Actor-Key-Index".into(), actor_key_index.to_string());
    if let Some(identity) = recovery {
        spec.headers.insert(
            "X-Ait-Worker-Job-Index".into(),
            identity.worker_job_index.to_string(),
        );
    }
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_reprepare_workflow_package_request_spec(
    config: &PlanHttpClientConfig,
    identity: &PackageWorkflowExpectedIdentity,
    actor_key_index: u32,
) -> PlanHttpClientResult<PlanHttpBytesRequestSpec> {
    let path = package_authority(config, identity)?;
    super::super::actor_registration::validate_actor_key_index(actor_key_index)?;
    let mut spec = plan_http_transport::build_bytes_request_spec(
        config,
        Method::POST,
        &format!("{path}/remote-sync/zstd-bulk/commit"),
        Vec::new(),
        Some(Vec::new()),
        "application/json",
        Some(WORKFLOW_PACKAGE_MEDIA_TYPE),
    )?;
    spec.headers
        .insert("X-Ait-Package-Sha256".into(), identity.package_hash.clone());
    spec.headers
        .insert("X-Ait-Actor-Key-Index".into(), actor_key_index.to_string());
    spec.headers.insert(
        "X-Ait-Worker-Job-Index".into(),
        identity.worker_job_index.to_string(),
    );
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_finish_workflow_package_request_spec(
    config: &PlanHttpClientConfig,
    identity: &PackageWorkflowExpectedIdentity,
    target_line: &str,
    mode: &str,
    actor_key_index: u32,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let path = package_authority(config, identity)?;
    super::super::actor_registration::validate_actor_key_index(actor_key_index)?;
    if target_line.is_empty()
        || target_line.trim() != target_line
        || !matches!(mode, "direct" | "merge" | "ff-only")
    {
        return Err(PlanHttpClientError::Invalid(
            "Package finish requires exact target Line and existing finish mode".into(),
        ));
    }
    let mut spec = plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!("{path}/task-land"),
        Vec::new(),
        Some(serde_json::json!({
            "contract":"task-land-package-guarded/v1",
            "idempotency_key":format!("workflow-package:{}:{}",identity.package_hash,identity.worker_job_index),
            "actor_key_index":actor_key_index,"target_line":target_line,"mode":mode,"expected_identity":identity,
        })),
    )?;
    spec.timeout_ms = zstd_bulk_timeout_ms(config);
    Ok(spec)
}

pub fn build_workflow_package_evidence_request_spec(
    config: &PlanHttpClientConfig,
    identity: &PackageWorkflowExpectedIdentity,
    operation: &str,
    idempotency_key: &str,
    actor_key_index: u32,
    payload: Value,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let path = package_authority(config, identity)?;
    super::super::actor_registration::validate_actor_key_index(actor_key_index)?;
    if !matches!(operation, "review" | "attestation")
        || idempotency_key.is_empty()
        || idempotency_key.trim() != idempotency_key
    {
        return Err(PlanHttpClientError::Invalid(
            "Package evidence requires an existing operation and exact idempotency key".into(),
        ));
    }
    let object = payload.as_object().ok_or_else(|| {
        PlanHttpClientError::Invalid("Package evidence payload must be an object".into())
    })?;
    for key in [
        "patchset_id",
        "review_id",
        "group_request_id",
        "created_at",
        "updated_at",
        "recorded_at_s",
        "patchset_ci",
    ] {
        if object.contains_key(key) {
            return Err(PlanHttpClientError::Invalid(format!(
                "Package evidence cannot supply {key}"
            )));
        }
    }
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!("{path}/worker-jobs/{}", identity.worker_job_index),
        Vec::new(),
        Some(serde_json::json!({
            "contract":"workflow-package-evidence/v1","operation":operation,"idempotency_key":idempotency_key,
            "actor_key_index":actor_key_index,"expected_identity":identity,"payload":payload,
        })),
    )
}

pub fn build_abandon_workflow_package_request_spec(
    config: &PlanHttpClientConfig,
    identity: &PackageWorkflowExpectedIdentity,
    actor_key_index: u32,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let path = package_authority(config, identity)?;
    super::super::actor_registration::validate_actor_key_index(actor_key_index)?;
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!("{path}/worker-jobs/{}", identity.worker_job_index),
        Vec::new(),
        Some(serde_json::json!({
            "contract":"workflow-package-abandon/v1","actor_key_index":actor_key_index,"expected_identity":identity,
            "idempotency_key":format!("workflow-package-abandon:{}:{}",identity.package_hash,identity.worker_job_index),
        })),
    )
}
