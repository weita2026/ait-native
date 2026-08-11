use crate::attest_json::AttestJson;
use crate::change_json::ChangeJson;
use crate::json_support::{JsonMap as Map, JsonValue as Value};
use crate::land_json::LandJson;
use crate::patchset_json::PatchsetJson;
use crate::policy_json::PolicyJson;
use crate::remote_sync_backend::{
    ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE, ZSTD_BULK_TREE_PACK_MEDIA_TYPE,
};
use crate::repository_pack_json::{
    ZstdBulkCommitRequest, ZstdBulkCommitRequestJson, ZstdBulkPlanRequest, ZstdBulkPlanRequestJson,
    ZstdPullManifestRequest, ZstdPullManifestRequestJson,
};
use crate::server_operational::{
    claim_next_worker_job_path, repository_authority_path, repository_restores_path,
    repository_retirement_abort_path, repository_retirement_path, repository_retirement_purge_path,
    repository_worker_jobs_path, worker_job_operation_path, worker_job_path, RepositoryIndex,
    WorkerJobKey, WorkerLeaseProof,
};
use crate::server_repo_retire::{
    validate_remote_authority_relative_path, RemoteExportManifest, REMOTE_AUTHORITY_FILE_MEDIA_TYPE,
};
use crate::snapshot_json::SnapshotJson;
use crate::task_json::TaskJson;
use reqwest::Method;

use super::transport as plan_http_transport;
use super::{
    configured_repository_authority_path_segment, encode_path_segment, PlanHttpBytesRequestSpec,
    PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientResult, PlanHttpRequestSpec,
};

const ZSTD_BULK_TIMEOUT_MS: u64 = 900_000;

fn zstd_bulk_timeout_ms(config: &PlanHttpClientConfig) -> u64 {
    config.default_timeout_ms.max(ZSTD_BULK_TIMEOUT_MS)
}

fn require_non_empty_text(value: &str, field: &str) -> PlanHttpClientResult<String> {
    normalize_optional_text(Some(value)).ok_or_else(|| {
        PlanHttpClientError::Invalid(format!("Plan HTTP {field} must not be empty."))
    })
}

fn plan_collection_path(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
) -> PlanHttpClientResult<String> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    Ok(format!(
        "/v1/native/repository-authorities/{repository_index}/sprints"
    ))
}

fn plan_item_path(
    config: &PlanHttpClientConfig,
    plan_id: &str,
    suffix: &str,
) -> PlanHttpClientResult<String> {
    let plan_id = require_non_empty_text(plan_id, "plan_id")?;
    let plan_id = encode_path_segment(&plan_id);
    let repository_index = configured_repository_authority_path_segment(config)?;
    Ok(format!(
        "/v1/native/repository-authorities/{repository_index}/sprints/{plan_id}{suffix}"
    ))
}

fn remote_sync_path(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
    suffix: &str,
) -> PlanHttpClientResult<String> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    Ok(format!(
        "/v1/native/repository-authorities/{repository_index}/remote-sync/zstd-bulk/{suffix}"
    ))
}

pub(super) fn optional_json_string(value: Option<&str>) -> Value {
    match normalize_optional_text(value) {
        Some(value) => Value::String(value),
        None => Value::Null,
    }
}

pub(super) fn insert_optional_string(
    body: &mut Map<String, Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(text) = normalize_optional_text(value) {
        body.insert(key.to_string(), Value::String(text));
    }
}

pub(super) fn insert_optional_exact_string(
    body: &mut Map<String, Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(text) = value {
        body.insert(key.to_string(), Value::String(text.to_string()));
    }
}

pub(super) fn insert_optional_packed_artifact(
    body: &mut Map<String, Value>,
    value: Option<&Value>,
) {
    let Some(Value::Object(object)) = value else {
        return;
    };
    if let Some(Value::String(blob_id)) = object.get("artifact_blob_id") {
        if !blob_id.trim().is_empty() {
            body.insert(
                "artifact_blob_id".to_string(),
                Value::String(blob_id.clone()),
            );
        }
    }
    body.insert("packed_artifact".to_string(), Value::Object(object.clone()));
}

pub(super) fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    let text = value?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

pub(super) fn object_or_empty(value: Value, field: &str) -> PlanHttpClientResult<Value> {
    match value {
        Value::Null => Ok(Value::Object(Map::new())),
        Value::Object(_) => Ok(value),
        _ => Err(PlanHttpClientError::Invalid(format!(
            "Plan HTTP {field} payload must be an object."
        ))),
    }
}

pub(super) fn array_or_empty(value: Value, field: &str) -> PlanHttpClientResult<Value> {
    match value {
        Value::Null => Ok(Value::Array(Vec::new())),
        Value::Array(_) => Ok(value),
        _ => Err(PlanHttpClientError::Invalid(format!(
            "Plan HTTP {field} payload must be a list."
        ))),
    }
}

mod auth_specs;
mod ci_job_specs;
mod plan_specs;
mod planning_session_specs;
mod repository_specs;
mod server_operational_specs;
mod task_specs;

pub use self::auth_specs::*;
pub use self::ci_job_specs::*;
pub use self::plan_specs::*;
pub use self::planning_session_specs::*;
pub use self::repository_specs::*;
pub use self::server_operational_specs::*;
pub use self::task_specs::*;
