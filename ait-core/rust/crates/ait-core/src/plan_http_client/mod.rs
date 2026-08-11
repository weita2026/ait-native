#[cfg(test)]
use crate::json_support::JsonMap as Map;
use crate::json_support::JsonValue as Value;
#[cfg(test)]
use crate::remote_sync_backend::{
    ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE, ZSTD_BULK_TREE_PACK_MEDIA_TYPE,
};
#[cfg(test)]
use crate::repository_pack_json::ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_NAME;
use crate::repository_pack_json::{
    ZstdBulkCommitRequest, ZstdBulkCommitRequestJson, ZstdBulkCommitResponse,
    ZstdBulkCommitResponseJson, ZstdBulkPlanRequest, ZstdBulkPlanRequestJson, ZstdBulkPlanResponse,
    ZstdBulkPlanResponseJson, ZstdImportManifestJson, ZstdImportManifestPayload,
    ZstdPackUploadResponse, ZstdPackUploadResponseJson, ZstdPullManifestJson,
    ZstdPullManifestPayload, ZstdPullManifestRequest,
};

pub(crate) fn encode_path_segment(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        let ch = byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | '~') {
            output.push(ch);
        } else {
            output.push('%');
            output.push_str(&format!("{byte:02X}"));
        }
    }
    output
}

pub(crate) fn configured_repository_authority_path_segment(
    config: &PlanHttpClientConfig,
) -> PlanHttpClientResult<String> {
    config
        .repository_index
        .map(|repository_index| repository_index.to_string())
        .ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "Plan HTTP repository_index is required for repository-authority operations."
                    .to_string(),
            )
        })
}

mod request_specs;
mod response;
mod transport;
mod transport_ports;

pub use self::request_specs::*;
use self::response::{
    parse_any_payload, parse_json_bytes_payload, parse_list_payload, parse_object_payload,
};
pub(crate) use self::transport::build_request_spec as build_plan_http_request_spec;
pub use self::transport::{
    close_with_plan_http_client_lifecycle, execute_bytes_with_plan_http_transport,
    execute_json_with_plan_http_transport, inspect_with_plan_http_client_lifecycle,
    PlanHttpBytesRequestSpec, PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientLifecycle,
    PlanHttpClientManager, PlanHttpClientResult, PlanHttpClientStats, PlanHttpRequestSpec,
    PlanHttpTransport,
};

mod auth_endpoints;
mod ci_jobs;
mod metrics_readiness;
mod plan_endpoints;
mod planning_sessions;
mod repository_admin;
mod server_operational_endpoints;
mod task_endpoints;

pub use self::auth_endpoints::*;
pub use self::ci_jobs::*;
pub use self::metrics_readiness::*;
pub use self::plan_endpoints::*;
pub use self::planning_sessions::*;
pub use self::repository_admin::*;
pub use self::task_endpoints::*;

#[cfg(test)]
mod tests;
